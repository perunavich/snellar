use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use zbus::message::Type;
use zbus::{Connection, MatchRule, MessageStream, OwnedVariant};

use snellar_core::ids::{TrackId, valid_id};
use snellar_core::model::Track;
use snellar_core::product;
use snellar_core::ui::UiEvent;

const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const PATH: &str = "/org/mpris/MediaPlayer2";
const LENGTH_TOLERANCE: f64 = 3.0;
const SHORTEST: f64 = 60.0;
const LONGEST: f64 = 15.0 * 60.0;
const METHOD_TIMEOUT: Duration = Duration::from_secs(2);

pub trait Host: Send + Sync + 'static {
    fn handoff(&self) -> &Handoff;
    fn handoff_enabled(&self) -> bool;
    fn emit(&self, event: UiEvent);
    fn track(&self, id: &TrackId) -> impl Future<Output = Result<Track>> + Send;
    fn songs(&self, query: &str) -> impl Future<Output = Result<Vec<Track>>> + Send;
}

struct Pending {
    id: String,
    source: String,
    track: Track,
}

struct Source {
    title: String,
    artist: String,
    length: f64,
    url: Option<String>,
}

pub struct Handoff {
    playing: Arc<AtomicBool>,
    timeout: Duration,
    conn: OnceLock<Connection>,
    pending: Arc<Mutex<Option<Pending>>>,
    seen: Mutex<HashSet<String>>,
    declined: Mutex<HashSet<TrackId>>,
    next: AtomicU64,
}

fn normalize(text: &str) -> String {
    let mut depth = 0;
    let kept: String = text
        .chars()
        .filter(|c| match c {
            '(' | '[' => {
                depth += 1;
                false
            }
            ')' | ']' => {
                depth = (depth - 1).max(0);
                false
            }
            _ => depth == 0,
        })
        .map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or(c) } else { ' ' })
        .collect();
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn length_fits(track: &Track, length: f64) -> bool {
    length <= 0.0 || track.duration <= 0.0 || (track.duration - length).abs() <= LENGTH_TOLERANCE
}

fn worth_offering(source: &Source) -> bool {
    (SHORTEST..=LONGEST).contains(&source.length) && !source.artist.trim().is_empty() && !source.title.trim().is_empty()
}

fn matches(track: &Track, source: &Source) -> bool {
    let wanted = normalize(&source.title);
    let have = normalize(&track.title);
    let said = normalize(&format!("{} {}", source.artist, source.title));
    !have.is_empty()
        && wanted.contains(&have)
        && length_fits(track, source.length)
        && track.artists.iter().any(|a| {
            let name = normalize(&a.name);
            !name.is_empty() && said.contains(&name)
        })
}

fn music_id(url: &str) -> Option<TrackId> {
    let url = reqwest::Url::parse(url).ok()?;
    if url.host_str() != Some("music.youtube.com") || url.path() != "/watch" {
        return None;
    }
    url.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v).filter(|v| valid_id(v)).map(|v| TrackId::youtube(&v))
}

fn text(value: &OwnedVariant) -> Option<String> {
    String::try_from(value.try_clone().ok()?).ok()
}

fn micros(value: &OwnedVariant) -> Option<i64> {
    let copy = value.try_clone().ok()?;
    i64::try_from(copy.try_clone().ok()?).ok().or_else(|| u64::try_from(copy).ok().map(|n| n as i64))
}

fn source_of(state: &HashMap<String, OwnedVariant>) -> Option<Source> {
    let metadata = HashMap::<String, OwnedVariant>::try_from(state.get("Metadata")?.try_clone().ok()?).ok()?;
    let title = text(metadata.get("xesam:title")?)?;
    let artist = metadata
        .get("xesam:artist")
        .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
        .map(|names| names.join(", "))
        .unwrap_or_default();
    Some(Source {
        title,
        artist,
        length: metadata.get("mpris:length").and_then(micros).map_or(0.0, |us| us as f64 / 1e6),
        url: metadata.get("xesam:url").and_then(text),
    })
}

async fn resolve(backend: &impl Host, source: &Source) -> Option<Track> {
    if let Some(id) = source.url.as_deref().and_then(music_id) {
        return backend.track(&id).await.ok().filter(|t| length_fits(t, source.length));
    }
    let found = backend.songs(&format!("{} {}", source.artist, source.title)).await.ok()?;
    found.into_iter().find(|t| matches(t, source))
}

impl Handoff {
    pub fn new(playing: Arc<AtomicBool>, timeout: Duration) -> Handoff {
        Handoff {
            playing,
            timeout,
            conn: OnceLock::new(),
            pending: Arc::default(),
            seen: Mutex::default(),
            declined: Mutex::default(),
            next: AtomicU64::new(1),
        }
    }

    async fn consider(&self, backend: &impl Host, conn: &Connection, sender: &str) -> Result<()> {
        if !backend.handoff_enabled() || self.playing.load(Ordering::Relaxed) {
            return Ok(());
        }
        let own: Option<String> = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(product::MPRIS_BUS,),
            )
            .await
            .ok()
            .and_then(|reply| reply.body().deserialize().ok());
        if own.as_deref() == Some(sender) {
            return Ok(());
        }
        let state: HashMap<String, OwnedVariant> = conn
            .call_method(Some(sender), PATH, Some("org.freedesktop.DBus.Properties"), "GetAll", &(PLAYER,))
            .await?
            .body()
            .deserialize()?;
        if state.get("PlaybackStatus").and_then(text).as_deref() != Some("Playing") {
            return Ok(());
        }
        let Some(source) = source_of(&state).filter(worth_offering) else { return Ok(()) };
        if !self.seen.lock().unwrap().insert(format!("{sender}\0{}\0{}", source.artist, source.title)) {
            return Ok(());
        }
        let Some(track) = resolve(backend, &source).await else { return Ok(()) };
        if self.declined.lock().unwrap().contains(&track.id) || self.playing.load(Ordering::Relaxed) {
            return Ok(());
        }
        let id = format!("handoff-{}", self.next.fetch_add(1, Ordering::Relaxed));
        *self.pending.lock().unwrap() = Some(Pending { id: id.clone(), source: sender.to_string(), track: track.clone() });
        backend.emit(UiEvent::Nudge(
            serde_json::json!({
                "id": id,
                "kind": "handoff",
                "track": track.title,
                "text": track.artist_line(),
                "cover": track.thumb,
                "timeout": self.timeout.as_secs(),
                "actions": [{ "id": "continue", "primary": true }, { "id": "ignore" }],
            })
            .to_string(),
        ));
        let (pending, timeout) = (self.pending.clone(), self.timeout);
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            let mut pending = pending.lock().unwrap();
            if pending.as_ref().is_some_and(|p| p.id == id) {
                *pending = None;
            }
        });
        Ok(())
    }

    pub async fn respond(&self, id: &str, action: &str) -> Result<Option<(Track, f64)>> {
        let offered = {
            let mut pending = self.pending.lock().unwrap();
            if pending.as_ref().is_some_and(|p| p.id == id) { pending.take() } else { None }
        };
        let Some(offered) = offered else { return Ok(None) };
        if action == "continue" && self.playing.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if action != "continue" {
            self.declined.lock().unwrap().insert(offered.track.id);
            return Ok(None);
        }
        let conn = self.conn.get().context("handoff is not watching the session bus")?;
        let source = offered.source.as_str();
        let position =
            match conn.call_method(Some(source), PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(PLAYER, "Position")).await {
                Ok(reply) => reply.body().deserialize::<OwnedVariant>().ok().and_then(|v| micros(&v)),
                Err(_) => None,
            };
        conn.call_method(Some(source), PATH, Some(PLAYER), "Pause", &()).await.ok();
        Ok(Some((offered.track, position.unwrap_or(0).max(0) as f64 / 1e6)))
    }
}

pub fn watch(backend: Arc<impl Host>) {
    std::thread::Builder::new()
        .name(format!("{}-handoff", product::ID))
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("handoff runtime");
            if let Err(e) = runtime.block_on(run(backend, None)) {
                snellar_core::journal::error("handoff", format!("{e:#}"));
            }
        })
        .expect("handoff thread");
}

pub async fn run(backend: Arc<impl Host>, address: Option<String>) -> Result<()> {
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address.as_str()),
        None => zbus::connection::Builder::session(),
    };
    let conn = builder.method_timeout(METHOD_TIMEOUT).build().await?;
    let handoff = backend.handoff();
    handoff.conn.set(conn.clone()).ok();
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.DBus.Properties")
        .member("PropertiesChanged")
        .path(PATH)
        .build()?;
    let mut signals = MessageStream::for_match_rule(rule, &conn, None).await?;
    while let Some(Ok(message)) = signals.next().await {
        let Some(sender) = message.header().sender().map(|s| s.to_string()) else { continue };
        let Ok((interface, changed, _)) = message.body().deserialize::<(String, HashMap<String, OwnedVariant>, Vec<String>)>() else {
            continue;
        };
        if interface == PLAYER && (changed.contains_key("PlaybackStatus") || changed.contains_key("Metadata")) {
            let _ = handoff.consider(&*backend, &conn, &sender).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use zbus::Variant;

    use crate::api::Backend;
    use crate::testkit::Bus;
    use snellar_core::ui::PlayState;

    use super::*;

    fn source(title: &str, artist: &str, length: f64) -> Source {
        Source { title: title.into(), artist: artist.into(), length, url: None }
    }

    fn song(title: &str, artist: &str, duration: f64) -> Track {
        Track {
            id: TrackId::youtube("dQw4w9WgXcQ"),
            title: title.into(),
            artists: vec![snellar_core::model::Ref { id: "a".into(), name: artist.into() }],
            duration,
            thumb: "https://i.ytimg.com/x".into(),
            ..Track::default()
        }
    }

    #[test]
    fn titles_are_compared_without_noise() {
        assert_eq!(normalize("Never Gonna Give You Up (Official Video) [4K]"), "never gonna give you up");
        assert_eq!(normalize("  AC/DC -  Back In Black! "), "ac dc back in black");
    }

    #[test]
    fn a_match_needs_title_artist_and_length() {
        let track = song("Never Gonna Give You Up", "Rick Astley", 213.0);
        assert!(matches(&track, &source("Rick Astley - Never Gonna Give You Up (Official Video)", "Rick Astley", 214.5)));
        assert!(matches(&track, &source("Never Gonna Give You Up", "Rick Astley - Topic", 0.0)));
        assert!(!matches(&track, &source("Never Gonna Give You Up", "Rick Astley", 260.0)));
        assert!(!matches(&track, &source("How to cook rice", "Rick Astley", 213.0)));
        assert!(!matches(&track, &source("Never Gonna Give You Up", "Somebody Else", 213.0)));
        assert!(!matches(&song("Intro", "The xx", 127.0), &source("Intro to quantum computing", "Physics Channel", 127.0)));
    }

    #[test]
    fn only_songs_of_one_to_fifteen_minutes_with_an_artist_are_offered() {
        for (length, artist, offered) in [
            (213.0, "Rick Astley", true),
            (60.0, "Rick Astley", true),
            (900.0, "Rick Astley", true),
            (59.9, "Rick Astley", false),
            (900.1, "Rick Astley", false),
            (0.0, "Rick Astley", false),
            (3600.0, "Lofi Girl", false),
            (213.0, "", false),
            (213.0, "  ", false),
        ] {
            assert_eq!(worth_offering(&source("Never Gonna Give You Up", artist, length)), offered, "{length} s by {artist:?}");
        }
        assert!(!worth_offering(&source(" ", "Rick Astley", 213.0)));
    }

    #[test]
    fn only_music_urls_carry_an_id() {
        assert_eq!(music_id("https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=x"), Some(TrackId::youtube("dQw4w9WgXcQ")));
        assert_eq!(music_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), None);
        assert_eq!(music_id("https://music.youtube.com/watch?v=not-valid"), None);
        assert_eq!(music_id("https://evil.example/watch?v=dQw4w9WgXcQ"), None);
    }

    #[derive(Default)]
    struct Fake {
        paused: bool,
        title: String,
        artist: String,
        url: String,
        length: i64,
        hang: bool,
    }

    struct FakePlayer(Arc<Mutex<Fake>>);

    #[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
    impl FakePlayer {
        fn pause(&self) {
            self.0.lock().unwrap().paused = true;
        }

        #[zbus(property)]
        async fn position(&self) -> i64 {
            let hang = self.0.lock().unwrap().hang;
            if hang {
                std::future::pending::<()>().await;
            }
            42_000_000
        }

        #[zbus(property)]
        fn playback_status(&self) -> String {
            if self.0.lock().unwrap().paused { "Paused" } else { "Playing" }.into()
        }

        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, Variant<'static>> {
            let fake = self.0.lock().unwrap();
            HashMap::from([
                ("xesam:title".to_string(), Variant::from(fake.title.clone())),
                ("xesam:artist".to_string(), Variant::from(vec![fake.artist.clone()])),
                ("xesam:url".to_string(), Variant::from(fake.url.clone())),
                ("mpris:length".to_string(), Variant::from(fake.length)),
            ])
        }
    }

    struct Remote {
        conn: Connection,
        state: Arc<Mutex<Fake>>,
    }

    async fn fake_player(bus: &Bus, name: &str) -> Remote {
        let state = Arc::new(Mutex::new(Fake::default()));
        let conn = zbus::connection::Builder::address(bus.address.as_str())
            .name(name.to_string())
            .serve_at(PATH, FakePlayer(state.clone()))
            .build()
            .await
            .unwrap();
        Remote { conn, state }
    }

    async fn play(remote: &Remote, title: &str, artist: &str, url: &str, length: i64) {
        {
            let mut fake = remote.state.lock().unwrap();
            (fake.paused, fake.title, fake.artist, fake.url, fake.length) = (false, title.into(), artist.into(), url.into(), length);
        }
        let conn = &remote.conn;
        let mut metadata: HashMap<&str, Variant> = HashMap::new();
        metadata.insert("xesam:title", Variant::from(title));
        metadata.insert("xesam:artist", Variant::from(vec![artist]));
        metadata.insert("xesam:url", Variant::from(url));
        metadata.insert("mpris:length", Variant::from(length));
        let mut changed: HashMap<&str, Variant> = HashMap::new();
        changed.insert("PlaybackStatus", Variant::from("Playing"));
        changed.insert("Metadata", Variant::from(metadata));
        conn.emit_signal(
            None::<&str>,
            PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(PLAYER, changed, Vec::<String>::new()),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(700)).await;
    }

    const MUSIC: &str = "https://music.youtube.com/watch?v=dQw4w9WgXcQ";
    const LENGTH: i64 = 213_000_000;

    struct Rig {
        bus: Bus,
        backend: Arc<Backend>,
        nudges: Arc<Mutex<Vec<serde_json::Value>>>,
        events: Arc<Mutex<Vec<UiEvent>>>,
    }

    impl Rig {
        async fn new(name: &str) -> Rig {
            let rig = Rig::unset(name).await;
            rig.backend.dispatch("setSetting", &serde_json::json!({ "key": "handoff", "value": true })).await.unwrap();
            rig
        }

        async fn unset(name: &str) -> Rig {
            let bus = Bus::start();
            let dir = std::env::temp_dir().join(format!("snellar-handoff-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            let nudges = Arc::new(Mutex::new(Vec::new()));
            let events = Arc::new(Mutex::new(Vec::new()));
            let (cards, log) = (nudges.clone(), events.clone());
            let backend = Backend::offline(
                &dir,
                Arc::new(move |event| {
                    if let UiEvent::Nudge(json) = &event {
                        cards.lock().unwrap().push(serde_json::from_str(json).unwrap());
                    }
                    log.lock().unwrap().push(event);
                }),
            );
            let known = serde_json::json!({ "id": "dQw4w9WgXcQ", "title": "Never Gonna Give You Up", "artists": [{ "id": "a", "name": "Rick Astley" }], "duration": 213.0, "thumb": "https://i.ytimg.com/x" });
            backend.dispatch("like", &serde_json::json!({ "track": known })).await.unwrap();
            backend.dispatch("setSetting", &serde_json::json!({ "key": "volume", "value": 0 })).await.unwrap();
            tokio::spawn(run(backend.clone(), Some(bus.address.clone())));
            for _ in 0..50 {
                if backend.handoff().conn.get().is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Rig { bus, backend, nudges, events }
        }

        fn cards(&self) -> usize {
            self.nudges.lock().unwrap().len()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_card_is_offered_once_and_accepting_pauses_the_source_and_resumes_here() {
        let rig = Rig::new("accept").await;
        let fake = fake_player(&rig.bus, "org.mpris.MediaPlayer2.fake").await;

        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 1);
        let offer = rig.nudges.lock().unwrap()[0].clone();
        assert_eq!((offer["kind"].as_str(), offer["track"].as_str()), (Some("handoff"), Some("Never Gonna Give You Up")));
        assert!(offer.get("title").is_none() && offer["actions"][0].get("label").is_none(), "the window words the card");
        assert_eq!(
            (offer["text"].as_str(), offer["cover"].as_str(), offer["timeout"].as_u64()),
            (Some("Rick Astley"), Some("https://i.ytimg.com/x"), Some(15))
        );
        assert_eq!(offer["actions"][0]["id"], "continue");
        assert_eq!(offer["actions"][0]["primary"], true);

        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 1, "the same track must not be offered twice");

        let id = offer["id"].as_str().unwrap();
        rig.backend.dispatch("nudgeAnswer", &serde_json::json!({ "id": "unknown", "action": "continue" })).await.unwrap();
        assert!(!fake.state.lock().unwrap().paused);
        rig.backend.dispatch("nudgeAnswer", &serde_json::json!({ "id": id, "action": "continue" })).await.ok();
        assert!(fake.state.lock().unwrap().paused, "the other player gets Pause");
        let (current, position) = {
            let events = rig.events.lock().unwrap();
            let current = events.iter().find_map(|e| if let UiEvent::Current(Some(t)) = e { Some(t.id.clone()) } else { None });
            let position = events.iter().find_map(|e| if let UiEvent::Position(p) = e { Some(*p) } else { None });
            (current, position)
        };
        assert_eq!(current, Some(TrackId::youtube("dQw4w9WgXcQ")));
        assert!((position.unwrap() - 42.0).abs() <= 2.0, "resumes at {position:?}");
        rig.backend.dispatch("nudgeAnswer", &serde_json::json!({ "id": id, "action": "continue" })).await.unwrap();
        rig.backend.dispatch("clearQueue", &serde_json::json!({})).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nothing_is_looked_up_or_offered_until_the_setting_is_true() {
        let rig = Rig::unset("unset").await;
        let fake = fake_player(&rig.bus, "org.mpris.MediaPlayer2.fake").await;
        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        play(&fake, "Some Song", "Some Artist", "https://www.youtube.com/watch?v=aaaaaaaaaaa", LENGTH).await;
        assert_eq!(rig.cards(), 0);
        assert!(rig.backend.handoff().seen.lock().unwrap().is_empty(), "no source may be looked at while the setting is unset");

        rig.backend.dispatch("setSetting", &serde_json::json!({ "key": "handoff", "value": true })).await.unwrap();
        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 1, "turning it on needs no restart");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_declined_track_is_never_offered_again_from_any_source() {
        let rig = Rig::new("decline").await;
        let first = fake_player(&rig.bus, "org.mpris.MediaPlayer2.first").await;
        let second = fake_player(&rig.bus, "org.mpris.MediaPlayer2.second").await;

        play(&first, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 1);
        let id = rig.nudges.lock().unwrap()[0]["id"].as_str().unwrap().to_string();
        rig.backend.dispatch("nudgeAnswer", &serde_json::json!({ "id": id, "action": "ignore" })).await.unwrap();
        assert!(!first.state.lock().unwrap().paused, "declining leaves the source alone");

        play(&first, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        play(&second, "Never Gonna Give You Up (Official Video)", "Rick Astley", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nothing_is_offered_for_our_own_mpris_while_playing_for_unknown_tracks_or_when_off() {
        let rig = Rig::new("guards").await;
        let own = fake_player(&rig.bus, product::MPRIS_BUS).await;
        play(&own, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 0, "our own MPRIS is not another player");

        let fake = fake_player(&rig.bus, "org.mpris.MediaPlayer2.fake").await;
        play(&fake, "How to cook rice", "Some Channel", "https://www.youtube.com/watch?v=aaaaaaaaaaa", 600_000_000).await;
        assert_eq!(rig.cards(), 0, "a video that is not music stays silent");
        play(&fake, "Unknown Song", "Nobody", "https://music.youtube.com/watch?v=bbbbbbbbbbb", LENGTH).await;
        assert_eq!(rig.cards(), 0, "a track that cannot be found stays silent");
        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, 45_000_000).await;
        assert_eq!(rig.cards(), 0, "a clip under a minute is not offered");
        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, 1_200_000_000).await;
        assert_eq!(rig.cards(), 0, "twenty minutes is a mix or a talk, not a song");
        play(&fake, "Never Gonna Give You Up", "", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 0, "a source without an artist is not offered");

        rig.backend.emit(UiEvent::State(PlayState { playing: true, ..PlayState::default() }));
        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 0, "nothing while Snellar is playing");
        rig.backend.emit(UiEvent::State(PlayState::default()));

        rig.backend.dispatch("setSetting", &serde_json::json!({ "key": "handoff", "value": false })).await.unwrap();
        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 0, "the setting switches it off");

        rig.backend.dispatch("setSetting", &serde_json::json!({ "key": "handoff", "value": true })).await.unwrap();
        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        assert_eq!(rig.cards(), 1, "and back on");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_client_that_never_answers_does_not_stop_the_watch() {
        let rig = Rig::new("hung").await;
        let hung = fake_player(&rig.bus, "org.mpris.MediaPlayer2.hung").await;
        let good = fake_player(&rig.bus, "org.mpris.MediaPlayer2.good").await;
        hung.state.lock().unwrap().hang = true;
        play(&hung, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        play(&good, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        for _ in 0..40 {
            if rig.cards() > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(rig.cards(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn accepting_survives_a_source_that_hangs_or_is_gone() {
        let rig = Rig::new("respond").await;
        let hung = fake_player(&rig.bus, "org.mpris.MediaPlayer2.hung").await;
        hung.state.lock().unwrap().hang = true;
        for (source, id) in [("org.mpris.MediaPlayer2.hung", "1"), ("org.mpris.MediaPlayer2.gone", "2")] {
            *rig.backend.handoff().pending.lock().unwrap() =
                Some(Pending { id: id.into(), source: source.into(), track: song("t", "a", 1.0) });
            let answer = tokio::time::timeout(Duration::from_secs(8), rig.backend.handoff().respond(id, "continue")).await;
            let (track, position) = answer.unwrap_or_else(|_| panic!("{source}: no answer")).unwrap().expect(source);
            assert_eq!((track.title.as_str(), position), ("t", 0.0), "{source}");
        }
    }

    struct Flipping {
        handoff: Handoff,
        playing: Arc<AtomicBool>,
        cards: Mutex<usize>,
    }

    impl Host for Flipping {
        fn handoff(&self) -> &Handoff {
            &self.handoff
        }
        fn handoff_enabled(&self) -> bool {
            true
        }
        fn emit(&self, event: UiEvent) {
            if matches!(event, UiEvent::Nudge(_)) {
                *self.cards.lock().unwrap() += 1;
            }
        }
        async fn track(&self, _id: &TrackId) -> Result<Track> {
            self.playing.store(true, Ordering::Relaxed);
            Ok(song("Never Gonna Give You Up", "Rick Astley", 213.0))
        }
        async fn songs(&self, _query: &str) -> Result<Vec<Track>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_card_when_snellar_started_playing_during_the_lookup() {
        let bus = Bus::start();
        let playing = Arc::new(AtomicBool::new(false));
        let host = Arc::new(Flipping { handoff: Handoff::new(playing.clone(), Duration::from_secs(5)), playing, cards: Mutex::new(0) });
        tokio::spawn(run(host.clone(), Some(bus.address.clone())));
        for _ in 0..50 {
            if host.handoff.conn.get().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let fake = fake_player(&bus, "org.mpris.MediaPlayer2.fake").await;
        play(&fake, "Never Gonna Give You Up", "Rick Astley", MUSIC, LENGTH).await;
        assert!(host.playing.load(Ordering::Relaxed), "the lookup ran");
        assert_eq!(*host.cards.lock().unwrap(), 0);
        assert!(host.handoff.pending.lock().unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_offer_answered_after_snellar_started_playing_is_dropped() {
        let handoff = Handoff::new(Arc::new(AtomicBool::new(true)), Duration::from_millis(100));
        *handoff.pending.lock().unwrap() = Some(Pending { id: "1".into(), source: ":1.99".into(), track: song("t", "a", 1.0) });
        assert!(handoff.respond("1", "continue").await.unwrap().is_none(), "what plays here is not replaced");
        assert!(handoff.pending.lock().unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dismissed_or_expired_offer_leaves_the_source_alone() {
        let handoff = Handoff::new(Arc::new(AtomicBool::new(false)), Duration::from_millis(100));
        *handoff.pending.lock().unwrap() = Some(Pending { id: "1".into(), source: ":1.99".into(), track: song("t", "a", 1.0) });
        assert!(handoff.respond("1", "ignore").await.unwrap().is_none());
        assert!(handoff.pending.lock().unwrap().is_none());
    }
}
