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
use snellar_core::sync::Lock;
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
        if !self.seen.locked().insert(format!("{sender}\0{}\0{}", source.artist, source.title)) {
            return Ok(());
        }
        let Some(track) = resolve(backend, &source).await else { return Ok(()) };
        if self.declined.locked().contains(&track.id) || self.playing.load(Ordering::Relaxed) {
            return Ok(());
        }
        let id = format!("handoff-{}", self.next.fetch_add(1, Ordering::Relaxed));
        *self.pending.locked() = Some(Pending { id: id.clone(), source: sender.to_string(), track: track.clone() });
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
            let mut pending = pending.locked();
            if pending.as_ref().is_some_and(|p| p.id == id) {
                *pending = None;
            }
        });
        Ok(())
    }

    pub async fn respond(&self, id: &str, action: &str) -> Result<Option<(Track, f64)>> {
        let offered = {
            let mut pending = self.pending.locked();
            if pending.as_ref().is_some_and(|p| p.id == id) { pending.take() } else { None }
        };
        let Some(offered) = offered else { return Ok(None) };
        if action == "continue" && self.playing.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if action != "continue" {
            self.declined.locked().insert(offered.track.id);
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
