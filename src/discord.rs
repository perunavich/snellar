use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{Activity, ActivityType, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use serde_json::Value;

use snellar_core::library::Library;
use snellar_core::model::Track;
use snellar_core::product;
use snellar_core::settings;

const APP_ID: &str = "1566534028306424001";
const RETRY: Duration = Duration::from_secs(5);

pub enum Msg {
    Track(Option<Track>),
    Duration(f64),
    Playing(bool),
    Position(f64),
    Seeked(f64),
}

#[derive(Default)]
struct Shown {
    track: Option<Track>,
    playing: bool,
    position: f64,
    stamped: Option<Instant>,
}

impl Shown {
    fn apply(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Track(t) => {
                self.track = t;
                self.position = 0.0;
                self.stamped = Some(Instant::now());
                true
            }
            Msg::Duration(d) => {
                let known = self.track.as_mut().filter(|t| t.duration != d);
                let changed = known.is_some();
                if let Some(track) = known {
                    track.duration = d;
                }
                changed
            }
            Msg::Playing(on) => std::mem::replace(&mut self.playing, on) != on,
            Msg::Position(p) => {
                self.position = p;
                self.stamped = Some(Instant::now());
                false
            }
            Msg::Seeked(p) => {
                self.position = p;
                self.stamped = Some(Instant::now());
                true
            }
        }
    }

    fn elapsed(&self) -> f64 {
        self.position + self.stamped.map_or(0.0, |s| s.elapsed().as_secs_f64())
    }
}

fn activity(track: &Track, elapsed: f64) -> Activity<'_> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let start = now - elapsed as i64;
    let mut times = Timestamps::new().start(start);
    if track.duration > 0.0 {
        times = times.end(start + track.duration as i64);
    }
    let mut assets = Assets::new().large_text(track.album.as_ref().map_or(track.title.as_str(), |a| a.name.as_str()));
    if !track.thumb.is_empty() {
        assets = assets.large_image(track.thumb.as_str());
    }
    Activity::new()
        .activity_type(ActivityType::Listening)
        .details(track.title.as_str())
        .state(track.artist_line())
        .assets(assets)
        .timestamps(times)
}

fn app_id(library: &Library) -> Option<String> {
    if !settings::DISCORD_RPC.or_default::<bool>(library) {
        return None;
    }
    let custom = match settings::DISCORD_APP_ID.get::<Value>(library) {
        Some(Value::String(s)) => s,
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    [custom.trim(), APP_ID].into_iter().find(|id| !id.is_empty()).map(str::to_string)
}

trait Sink {
    fn show(&mut self, track: &Track, elapsed: f64) -> bool;
    fn clear(&mut self) -> bool;
    fn close(&mut self);
}

impl Sink for DiscordIpcClient {
    fn show(&mut self, track: &Track, elapsed: f64) -> bool {
        self.set_activity(activity(track, elapsed)).is_ok()
    }

    fn clear(&mut self) -> bool {
        self.clear_activity().is_ok()
    }

    fn close(&mut self) {
        self.clear_activity().ok();
        DiscordIpc::close(self).ok();
    }
}

#[derive(Clone, Copy)]
struct Pace {
    settle: Duration,
    window: Duration,
    burst: usize,
}

const PACE: Pace = Pace { settle: Duration::from_millis(600), window: Duration::from_secs(20), burst: 5 };

fn run<S: Sink>(rx: Receiver<Msg>, app_id: impl Fn() -> Option<String>, mut connect: impl FnMut(&str) -> Option<S>, pace: Pace) {
    let mut shown = Shown::default();
    let mut client: Option<(String, S)> = None;
    let mut active = false;
    let mut dirty = false;
    let mut changed = Instant::now();
    let mut hold = Instant::now();
    let mut sent: VecDeque<Instant> = VecDeque::new();
    loop {
        let wait = if dirty && shown.playing && shown.track.is_some() {
            let free = if sent.len() >= pace.burst { sent[sent.len() - pace.burst] + pace.window } else { hold };
            (changed + pace.settle).max(free).max(hold).saturating_duration_since(Instant::now())
        } else if dirty {
            Duration::ZERO
        } else {
            RETRY
        };
        match rx.recv_timeout(wait) {
            Ok(msg) => {
                if shown.apply(msg) {
                    dirty = true;
                    changed = Instant::now();
                }
                while let Ok(msg) = rx.try_recv() {
                    if shown.apply(msg) {
                        dirty = true;
                        changed = Instant::now();
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let Some(id) = app_id() else {
            if let Some((_, mut c)) = client.take() {
                c.close();
            }
            active = false;
            dirty = false;
            continue;
        };
        if client.as_ref().is_some_and(|(current, _)| *current != id) {
            client = None;
            active = false;
        }
        let Some(track) = shown.track.as_ref().filter(|_| shown.playing) else {
            if let (true, Some((_, c))) = (active, client.as_mut())
                && !c.clear()
            {
                client = None;
            }
            active = false;
            dirty = false;
            continue;
        };
        if client.is_none() {
            let Some(c) = connect(&id) else {
                hold = Instant::now() + RETRY;
                continue;
            };
            client = Some((id, c));
            dirty = true;
        }
        if !dirty && active {
            continue;
        }
        let now = Instant::now();
        while sent.front().is_some_and(|t| now.duration_since(*t) >= pace.window) {
            sent.pop_front();
        }
        if sent.len() >= pace.burst || now < changed + pace.settle {
            continue;
        }
        let (_, c) = client.as_mut().unwrap();
        if !c.show(track, shown.elapsed()) {
            client = None;
            hold = Instant::now() + RETRY;
            continue;
        }
        sent.push_back(now);
        active = true;
        dirty = false;
    }
}

pub fn spawn(library: Arc<Library>) -> Sender<Msg> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name(format!("{}-discord", product::ID))
        .spawn(move || {
            let connect = |id: &str| {
                let mut c = DiscordIpcClient::new(id);
                c.connect().ok().map(|_| c)
            };
            run(rx, || app_id(&library), connect, PACE);
        })
        .expect("discord thread");
    tx
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc as Shared, Mutex};

    use super::*;

    type Log = Shared<Mutex<Vec<(Instant, Option<String>)>>>;

    struct Fake(Log);

    impl Sink for Fake {
        fn show(&mut self, track: &Track, _: f64) -> bool {
            self.0.lock().unwrap().push((Instant::now(), Some(track.title.clone())));
            true
        }

        fn clear(&mut self) -> bool {
            self.0.lock().unwrap().push((Instant::now(), None));
            true
        }

        fn close(&mut self) {}
    }

    const TEST_PACE: Pace = Pace { settle: Duration::from_millis(60), window: Duration::from_millis(500), burst: 5 };

    fn track(n: usize) -> Msg {
        Msg::Track(Some(Track { id: format!("t{n}").as_str().into(), title: format!("t{n}"), ..Track::default() }))
    }

    fn session() -> (Sender<Msg>, Log) {
        let (tx, rx) = mpsc::channel();
        let log = Shared::new(Mutex::new(Vec::new()));
        let sink = log.clone();
        std::thread::spawn(move || run(rx, || Some("1".into()), move |_| Some(Fake(sink.clone())), TEST_PACE));
        tx.send(Msg::Playing(true)).unwrap();
        (tx, log)
    }

    fn titles(log: &Log) -> Vec<String> {
        log.lock().unwrap().iter().filter_map(|(_, t)| t.clone()).collect()
    }

    #[test]
    fn a_length_learned_later_keeps_the_position() {
        let mut shown = Shown::default();
        shown.apply(Msg::Track(Some(Track { id: "a".into(), ..Track::default() })));
        shown.apply(Msg::Position(40.0));
        assert!(shown.apply(Msg::Duration(180.0)), "the card gets its timeline");
        assert_eq!((shown.position, shown.track.as_ref().unwrap().duration), (40.0, 180.0));
        assert!(!shown.apply(Msg::Duration(180.0)));
    }

    #[test]
    fn fast_skipping_shows_only_the_last_track() {
        let (tx, log) = session();
        for n in 0..20 {
            tx.send(track(n)).unwrap();
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(titles(&log), vec!["t19".to_string()]);
    }

    #[test]
    fn a_long_skip_run_stays_within_the_rate_limit_and_ends_on_the_last_track() {
        let (tx, log) = session();
        let start = Instant::now();
        let mut n = 0;
        while start.elapsed() < Duration::from_millis(1500) {
            tx.send(track(n)).unwrap();
            n += 1;
            std::thread::sleep(Duration::from_millis(90));
        }
        tx.send(track(1000)).unwrap();
        std::thread::sleep(Duration::from_millis(1200));
        let sent = log.lock().unwrap().clone();
        assert_eq!(sent.last().unwrap().1.as_deref(), Some("t1000"));
        for (i, (at, _)) in sent.iter().enumerate() {
            let in_window = sent[i..].iter().filter(|(t, _)| t.duration_since(*at) < TEST_PACE.window).count();
            assert!(in_window <= TEST_PACE.burst, "{in_window} updates inside one window");
        }
    }

    #[test]
    fn an_exhausted_budget_waits_and_then_shows_the_newest_track() {
        let (tx, log) = session();
        for n in 0..5 {
            tx.send(track(n)).unwrap();
            std::thread::sleep(Duration::from_millis(90));
        }
        assert_eq!(titles(&log).len(), 5);
        tx.send(track(5)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        tx.send(track(6)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(titles(&log).len(), 5, "the sixth update has to wait for the window");
        std::thread::sleep(Duration::from_millis(500));
        let shown = titles(&log);
        assert_eq!(shown.len(), 6);
        assert_eq!(shown[5], "t6");
    }

    #[test]
    fn pausing_clears_at_once_even_with_the_budget_spent() {
        let (tx, log) = session();
        for n in 0..5 {
            tx.send(track(n)).unwrap();
            std::thread::sleep(Duration::from_millis(90));
        }
        tx.send(Msg::Playing(false)).unwrap();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(log.lock().unwrap().last().unwrap().1, None);
    }

    #[test]
    #[ignore]
    fn silent_without_discord() {
        let library = Arc::new(Library::open(Path::new(":memory:")).unwrap());
        library.set_setting("discordRpc", "true");
        library.set_setting("discordAppId", "\"123456789012345678\"");
        let tx = spawn(library);
        let start = Instant::now();
        tx.send(Msg::Track(Some(Track { id: "x".into(), title: "T".into(), ..Track::default() }))).unwrap();
        tx.send(Msg::Playing(true)).unwrap();
        tx.send(Msg::Seeked(10.0)).unwrap();
        std::thread::sleep(Duration::from_secs(7));
        assert!(start.elapsed() < Duration::from_secs(8));
        assert!(tx.send(Msg::Playing(false)).is_ok());
    }

    #[test]
    fn the_public_app_id_is_used_unless_overridden() {
        let library = Library::open(Path::new(":memory:")).unwrap();
        assert_eq!(app_id(&library), None);
        library.set_setting("discordRpc", "true");
        assert_eq!(app_id(&library).as_deref(), Some("1566534028306424001"));
        library.set_setting("discordAppId", "\"\"");
        assert_eq!(app_id(&library).as_deref(), Some("1566534028306424001"));
        library.set_setting("discordAppId", "\"42\"");
        assert_eq!(app_id(&library).as_deref(), Some("42"));
        library.set_setting("discordRpc", "false");
        assert_eq!(app_id(&library), None);
    }
}
