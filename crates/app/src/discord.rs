use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use discord_rich_presence::activity::{Activity, ActivityType, Assets, Timestamps};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use serde_json::Value;

use snellar_core::clock;
use snellar_core::journal;
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
    Settings,
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
            Msg::Settings => false,
        }
    }

    fn elapsed(&self) -> f64 {
        self.position + self.stamped.map_or(0.0, |s| s.elapsed().as_secs_f64())
    }
}

fn activity(track: &Track, elapsed: f64) -> Activity<'_> {
    let now = clock::unix_secs() as i64;
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
    let mut missing = false;
    let mut recheck = false;
    loop {
        let wait = if recheck {
            Duration::ZERO
        } else if dirty && shown.playing && shown.track.is_some() {
            let free = if sent.len() >= pace.burst { sent[sent.len() - pace.burst] + pace.window } else { hold };
            (changed + pace.settle).max(free).max(hold).saturating_duration_since(Instant::now())
        } else if dirty {
            Duration::ZERO
        } else {
            RETRY
        };
        match rx.recv_timeout(wait) {
            Ok(msg) => {
                for msg in std::iter::once(msg).chain(std::iter::from_fn(|| rx.try_recv().ok())) {
                    if matches!(msg, Msg::Settings) {
                        recheck = true;
                    } else if shown.apply(msg) {
                        dirty = true;
                        changed = Instant::now();
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        recheck = false;
        let Some(id) = app_id() else {
            if let Some((_, mut c)) = client.take() {
                c.close();
            }
            active = false;
            dirty = false;
            continue;
        };
        if let Some((_, mut old)) = client.take_if(|(current, _)| *current != id) {
            old.close();
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
                if !std::mem::replace(&mut missing, true) {
                    journal::info("discord", format!("Discord is not running, trying again every {} s", RETRY.as_secs()));
                }
                hold = Instant::now() + RETRY;
                dirty = true;
                continue;
            };
            if std::mem::take(&mut missing) {
                journal::info("discord", "connected to Discord");
            }
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
