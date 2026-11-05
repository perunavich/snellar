use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::mpsc::Sender;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use snellar_core::error::{Code, Told};
use snellar_core::ids::TrackId;
use snellar_core::library::Library;
use snellar_core::model::{AudioDevice, AudioFormat, Fx, Leveling, Pin, Repeat, StreamInfo, Track};
use snellar_core::settings;
use snellar_core::source::{Source, Stream};
use snellar_core::tasks::TaskSet;
use snellar_core::ui::{PlayFailure, PlayState, Toast, Ui, UiEvent};

use crate::audio::{AudioOut, Event, Launch};
use crate::discord;
use crate::fx;
use crate::mpris::{self, Command, Update};

const POSITION_INTERVAL: Duration = Duration::from_millis(200);
const SAVE_INTERVAL: Duration = Duration::from_secs(5);
const STALL_AFTER: Duration = Duration::from_secs(8);
const PREFETCH_AHEAD: usize = 2;
const TOP_UP_AT: usize = 2;
const EOF_SLACK: f64 = 5.0;
const MAX_FAILURES: u8 = 3;
const AUTO_RETRIES: u8 = 3;
const AUTO_RETRY_BASE: Duration = Duration::from_secs(5);
const PLAYED_ENOUGH: f64 = 3.0;
const PRELOAD_LEAD: f64 = 10.0;

fn cut_short(position: f64, duration: f64) -> bool {
    duration > 0.0 && position + EOF_SLACK < duration
}

fn mpris_control(me: Weak<Player>, rt: Handle) -> mpris::Control {
    Arc::new(move |command| {
        if let Some(p) = me.upgrade() {
            let _rt = rt.enter();
            let player = p.clone();
            p.tasks.spawn("mpris command", async move { player.obey(command).await });
        }
    })
}

struct Queue {
    tracks: Vec<Track>,
    cursor: Option<usize>,
    shuffle: bool,
    original: Vec<Track>,
    epoch: u64,
    rng: u64,
    dismissed: HashSet<TrackId>,
}

impl Queue {
    fn new() -> Queue {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Queue { tracks: Vec::new(), cursor: None, shuffle: false, original: Vec::new(), epoch: 0, rng: seed | 1, dismissed: HashSet::new() }
    }

    fn current(&self) -> Option<&Track> {
        self.cursor.and_then(|i| self.tracks.get(i))
    }

    fn next_index(&self) -> Option<usize> {
        let next = self.cursor.map_or(0, |c| c + 1);
        (next < self.tracks.len()).then_some(next)
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn shuffle_from(&mut self, start: usize) {
        for i in (start + 1..self.tracks.len()).rev() {
            let j = start + (self.random() % (i - start + 1) as u64) as usize;
            self.tracks.swap(i, j);
        }
    }

    fn replace(&mut self, mut tracks: Vec<Track>, index: usize) {
        self.epoch += 1;
        self.dismissed.clear();
        let index = index.min(tracks.len().saturating_sub(1));
        if self.shuffle && !tracks.is_empty() {
            self.original = tracks.clone();
            let first = tracks.remove(index);
            tracks.insert(0, first);
            self.tracks = tracks;
            self.cursor = Some(0);
            self.shuffle_from(1);
        } else {
            self.cursor = (!tracks.is_empty()).then_some(index);
            self.tracks = tracks;
        }
    }

    fn insert(&mut self, track: Track, next: bool) -> usize {
        let at = if next { self.cursor.map_or(0, |c| c + 1) } else { self.tracks.len() };
        if self.shuffle {
            let after = self.current().and_then(|c| self.original.iter().position(|t| t.id == c.id));
            self.original.insert(if next { after.map_or(self.original.len(), |i| i + 1) } else { self.original.len() }, track.clone());
        }
        self.tracks.insert(at, track);
        at
    }

    fn insert_all(&mut self, tracks: Vec<Track>, next: bool) -> Option<usize> {
        let first = if next { self.cursor.map_or(0, |c| c + 1) } else { self.tracks.len() };
        let added = !tracks.is_empty();
        let ordered: Vec<Track> = if next { tracks.into_iter().rev().collect() } else { tracks };
        for t in ordered {
            self.insert(t, next);
        }
        added.then_some(first)
    }

    fn append_new(&mut self, tracks: Vec<Track>) -> Option<usize> {
        let first = self.tracks.len();
        for t in tracks {
            if !self.dismissed.contains(&t.id) && !self.tracks.iter().any(|q| q.id == t.id) {
                if self.shuffle {
                    self.original.push(t.clone());
                }
                self.tracks.push(t);
            }
        }
        (self.tracks.len() > first).then_some(first)
    }

    fn append_if_current(&mut self, epoch: u64, tracks: Vec<Track>) -> Option<usize> {
        if self.epoch == epoch { self.append_new(tracks) } else { None }
    }

    fn settle_radio(&mut self, still_current: bool, epoch: u64, related: Vec<Track>) -> Option<usize> {
        self.append_if_current(epoch, related);
        if still_current && self.epoch == epoch { self.next_index() } else { None }
    }

    fn locate(&self, index: usize, id: Option<&TrackId>) -> Option<usize> {
        let Some(id) = id else { return (index < self.tracks.len()).then_some(index) };
        if self.tracks.get(index).is_some_and(|t| t.id == *id) {
            return Some(index);
        }
        (0..self.tracks.len()).filter(|i| self.tracks[*i].id == *id).min_by_key(|i| i.abs_diff(index))
    }

    fn remove(&mut self, index: usize) -> Option<bool> {
        if index >= self.tracks.len() {
            return None;
        }
        let track = self.tracks.remove(index);
        self.dismissed.insert(track.id.clone());
        if self.shuffle
            && let Some(i) = self.original.iter().position(|t| t.id == track.id)
        {
            self.original.remove(i);
        }
        let was_current = self.cursor == Some(index);
        match self.cursor {
            Some(c) if index < c => self.cursor = Some(c - 1),
            Some(c) if was_current => self.cursor = (c < self.tracks.len()).then_some(c),
            _ => {}
        }
        Some(was_current)
    }

    fn move_item(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tracks.len() || to >= self.tracks.len() {
            return false;
        }
        let t = self.tracks.remove(from);
        self.tracks.insert(to, t);
        self.cursor = self.cursor.map(|c| match c {
            c if c == from => to,
            c if from < c && c <= to => c - 1,
            c if to <= c && c < from => c + 1,
            c => c,
        });
        true
    }

    fn clear(&mut self) {
        self.epoch += 1;
        self.dismissed.clear();
        self.tracks.clear();
        self.original.clear();
        self.cursor = None;
    }

    fn keep_current(&mut self) -> bool {
        let Some(current) = self.current().cloned() else { return false };
        self.epoch += 1;
        self.dismissed.extend(self.tracks.iter().filter(|t| t.id != current.id).map(|t| t.id.clone()));
        self.original = if self.shuffle { vec![current.clone()] } else { Vec::new() };
        self.tracks = vec![current];
        self.cursor = Some(0);
        true
    }

    fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        if on {
            self.original = self.tracks.clone();
            self.shuffle_from(self.cursor.map_or(0, |c| c + 1));
        } else {
            let current = self.current().map(|t| t.id.clone());
            self.tracks = std::mem::take(&mut self.original);
            self.cursor = current.and_then(|id| self.tracks.iter().position(|t| t.id == id));
        }
    }
}

struct State {
    queue: Queue,
    repeat: Repeat,
    play: PlayState,
    paused: bool,
    idle: bool,
    starting: bool,
    cache: bool,
    seeking: bool,
    attempts: u8,
    retry_point: Option<f64>,
    heard_duration: Option<f64>,
    failures: u8,
    auto_retries: u8,
    failed: Option<(TrackId, Code)>,
    marks: BTreeMap<String, Code>,
    stopped: bool,
    retrying: bool,
    fallback_cause: Option<anyhow::Error>,
    generation: u64,
    handed: u64,
    stall: u64,
    listening: Option<TrackId>,
    listened: f64,
    fx: Fx,
    topping: bool,
    resume: Option<f64>,
    saved: Instant,
    stream: StreamInfo,
    loudness: bool,
    measuring: bool,
    preload: Option<Preload>,
    preloading: Option<u64>,
    device: String,
    devices: Vec<AudioDevice>,
    origin: Option<Origin>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Origin {
    card: Pin,
    tracks: HashSet<TrackId>,
}

struct Preload {
    id: TrackId,
    generation: u64,
    format: Option<AudioFormat>,
    local: bool,
}

impl State {
    fn following(&self) -> Option<usize> {
        match (self.queue.cursor, self.queue.next_index()) {
            (Some(c), _) if self.repeat == Repeat::One => Some(c),
            (_, Some(n)) => Some(n),
            (_, None) if self.repeat == Repeat::All && !self.queue.tracks.is_empty() => Some(0),
            _ => None,
        }
    }

    fn following_id(&self) -> Option<&TrackId> {
        self.following().map(|i| &self.queue.tracks[i].id)
    }

    fn gain(&self, format: Option<&AudioFormat>) -> Option<f64> {
        format.and_then(|f| f.loudness_db).map(fx::gain)
    }

    fn failure(&self) -> Option<PlayFailure> {
        self.failed.as_ref().map(|(id, code)| PlayFailure {
            id: id.clone(),
            code: *code,
            tracks: self.marks.clone(),
            stopped: self.stopped,
            retrying: self.retrying,
        })
    }

    fn forget_failures(&mut self) {
        self.failures = 0;
        self.auto_retries = 0;
        self.failed = None;
        self.stopped = false;
        self.retrying = false;
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    tracks: Vec<Track>,
    cursor: Option<usize>,
    #[serde(default)]
    origin: Option<Origin>,
}

impl Saved {
    fn of(st: &State) -> Saved {
        Saved { tracks: st.queue.tracks.clone(), cursor: st.queue.cursor, origin: st.origin.clone() }
    }
}

pub struct Player {
    ui: Ui,
    source: Arc<dyn Source>,
    library: Arc<Library>,
    st: Mutex<State>,
    launch: Launch,
    audio: tokio::sync::Mutex<Option<Arc<dyn AudioOut>>>,
    mpris: UnboundedSender<Update>,
    discord: Sender<discord::Msg>,
    tasks: TaskSet,
    me: Weak<Player>,
}

impl Player {
    pub async fn start(ui: Ui, source: Arc<dyn Source>, launch: Launch, library: Arc<Library>) -> Result<Arc<Player>> {
        let volume = settings::VOLUME.or_default(&library);
        let fx = settings::FX.or_default(&library);
        let loudness = settings::LOUDNESS.or_default(&library);
        let device = settings::AUDIO_DEVICE.or_default(&library);
        let rt = tokio::runtime::Handle::current();
        let player = Arc::new_cyclic(|me| Player {
            ui,
            source,
            library: library.clone(),
            st: Mutex::new(State {
                queue: Queue::new(),
                repeat: Repeat::Off,
                play: PlayState { volume, ..PlayState::default() },
                paused: false,
                idle: true,
                starting: false,
                cache: false,
                seeking: false,
                attempts: 0,
                retry_point: None,
                heard_duration: None,
                failures: 0,
                auto_retries: 0,
                failed: None,
                marks: BTreeMap::new(),
                stopped: false,
                retrying: false,
                fallback_cause: None,
                generation: 0,
                handed: 0,
                stall: 0,
                listening: None,
                listened: 0.0,
                fx,
                topping: false,
                resume: None,
                saved: Instant::now(),
                stream: StreamInfo::default(),
                loudness,
                measuring: false,
                preload: None,
                preloading: None,
                device,
                devices: Vec::new(),
                origin: None,
            }),
            launch,
            audio: tokio::sync::Mutex::new(None),
            mpris: mpris::spawn(mpris_control(me.clone(), rt)),
            discord: discord::spawn(library),
            tasks: TaskSet::default(),
            me: me.clone(),
        });
        player.restore();
        if let Some(track) = player.current() {
            player.prefetch(&track.id);
        }
        player.audio().await?;
        player.publish_state(&mut player.st.lock().unwrap());
        Ok(player)
    }

    pub fn snapshot(&self) -> Value {
        let st = self.st.lock().unwrap();
        json!({
            "current": st.queue.current(),
            "playing": st.play.playing,
            "loading": st.play.loading,
            "position": st.play.position,
            "duration": st.play.duration,
            "volume": st.play.volume,
            "shuffle": st.queue.shuffle,
            "repeat": st.repeat,
            "stream": st.stream,
            "failure": st.failure(),
        })
    }

    pub fn queue(&self) -> Value {
        let st = self.st.lock().unwrap();
        json!({ "tracks": st.queue.tracks, "cursor": st.queue.cursor.map_or(-1, |c| c as i64) })
    }

    pub fn current(&self) -> Option<Track> {
        self.st.lock().unwrap().queue.current().cloned()
    }

    pub fn save_now(&self) {
        let mut st = self.st.lock().unwrap();
        self.finish_listen(&mut st, false);
        if st.queue.current().is_some() {
            settings::POSITION.set(&self.library, &st.play.position);
        }
        settings::QUEUE.set(&self.library, &Saved::of(&st));
    }

    pub async fn play(self: &Arc<Self>, tracks: Vec<Track>, index: usize, card: Option<Pin>) -> Result<()> {
        if tracks.is_empty() {
            return Ok(());
        }
        let origin = card.map(|card| Origin { card, tracks: tracks.iter().map(|t| t.id.clone()).collect() });
        let same = {
            let st = self.st.lock().unwrap();
            index < tracks.len() && st.queue.tracks.len() == tracks.len() && st.queue.tracks.iter().zip(&tracks).all(|(a, b)| a.id == b.id)
        };
        if same {
            self.st.lock().unwrap().origin = origin;
            return self.load(index).await;
        }
        self.replace_queue(self.timed(tracks), index);
        self.st.lock().unwrap().origin = origin;
        self.load_cursor().await
    }

    pub async fn radio(self: &Arc<Self>, seed: Track) -> Result<()> {
        self.radio_from(seed, 0.0).await
    }

    pub async fn radio_from(self: &Arc<Self>, seed: Track, position: f64) -> Result<()> {
        let seed = self.timed(vec![seed]).remove(0);
        let epoch = self.replace_queue(vec![seed.clone()], 0);
        self.st.lock().unwrap().resume = Some(position).filter(|p| *p > 0.0);
        if !self.has_radio() {
            return self.load_cursor().await;
        }
        let (loaded, related) = tokio::join!(self.load_cursor(), self.related(&seed, std::slice::from_ref(&seed)));
        loaded?;
        let related = self.timed(related?);
        let mut st = self.st.lock().unwrap();
        st.queue.append_if_current(epoch, related);
        self.publish_queue(&mut st);
        Ok(())
    }

    pub async fn enqueue(self: &Arc<Self>, tracks: Vec<Track>, next: bool) -> Result<()> {
        let tracks = self.timed(tracks);
        let idle = {
            let mut st = self.st.lock().unwrap();
            let Some(at) = st.queue.insert_all(tracks, next) else { return Ok(()) };
            self.publish_queue(&mut st);
            st.queue.cursor.is_none().then_some(at)
        };
        match idle {
            Some(at) => self.load(at).await,
            None => Ok(()),
        }
    }

    pub fn is_near(&self, id: &TrackId) -> bool {
        let st = self.st.lock().unwrap();
        let from = st.queue.cursor.unwrap_or(0);
        st.queue.tracks.iter().skip(from).take(2).any(|t| t.id == *id)
    }

    pub fn prefetch(&self, id: &TrackId) {
        if self.downloaded(id).is_none() {
            self.source.prefetch(id);
        }
    }

    fn downloaded(&self, id: &TrackId) -> Option<Stream> {
        let (path, loudness_db) = self.library.download(id)?;
        if !path.exists() {
            snellar_core::journal::warn("player", format!("{id}: the downloaded file is not there, playing from the source"));
            return None;
        }
        Some(Stream { url: path.to_string_lossy().into_owned(), format: Some(AudioFormat { loudness_db, ..AudioFormat::default() }) })
    }

    async fn open(&self, id: &TrackId, fresh: bool) -> Result<(Stream, bool)> {
        match self.downloaded(id) {
            Some(stream) if !fresh => Ok((stream, true)),
            _ => Ok((self.source.stream(id, fresh).await?, false)),
        }
    }

    async fn obey(self: &Arc<Self>, command: Command) {
        match command {
            Command::Toggle => {
                self.toggle().await.ok();
            }
            Command::Next => {
                self.next().await.ok();
            }
            Command::Prev => {
                self.prev().await.ok();
            }
            Command::Seek(target) => {
                self.seek(target).await.ok();
            }
            Command::Volume(v) => {
                self.set_volume(v).await.ok();
            }
            Command::Shuffle(on) => self.set_shuffle(on).await,
            Command::Repeat(mode) => self.set_repeat(mode).await,
        }
    }

    pub async fn toggle(self: &Arc<Self>) -> Result<()> {
        let (idle, paused) = {
            let st = self.st.lock().unwrap();
            (st.idle && st.queue.cursor.is_some(), st.paused)
        };
        if idle {
            return self.load_cursor().await;
        }
        self.audio().await?.set("pause", json!(!paused)).await
    }

    pub async fn next(self: &Arc<Self>) -> Result<()> {
        self.chose_to_move();
        self.advance(None).await
    }

    fn chose_to_move(&self) {
        let mut st = self.st.lock().unwrap();
        st.forget_failures();
        self.emit(UiEvent::PlayFailure(None));
    }

    pub async fn cancel_start(&self) {
        {
            let mut st = self.st.lock().unwrap();
            if !st.starting && !st.retrying {
                return;
            }
            st.generation += 1;
            st.starting = false;
            st.retrying = false;
            st.auto_retries = 0;
            st.failed = None;
            self.publish_state(&mut st);
            self.emit(UiEvent::PlayFailure(None));
        }
        if let Some(audio) = self.audio.lock().await.clone() {
            audio.stop().await.ok();
        }
    }

    pub async fn prev(self: &Arc<Self>) -> Result<()> {
        let (position, cursor) = {
            let st = self.st.lock().unwrap();
            (st.play.position, st.queue.cursor)
        };
        match cursor {
            Some(c) if c > 0 && position <= 3.0 => self.load(c - 1).await,
            _ => self.seek(0.0).await,
        }
    }

    pub async fn seek(&self, pos: f64) -> Result<()> {
        let pos = pos.max(0.0);
        let parked = {
            let mut st = self.st.lock().unwrap();
            let parked = st.idle && !st.starting;
            if parked {
                if st.queue.current().is_none() {
                    return Ok(());
                }
                st.resume = Some(pos);
            }
            parked
        };
        if !parked {
            self.audio().await?.seek(pos).await?;
        }
        let mut st = self.st.lock().unwrap();
        st.play.position = pos;
        st.listened = st.listened.max(pos);
        self.emit(UiEvent::Position(pos));
        self.mpris.send(Update::Seeked(pos)).ok();
        self.discord.send(discord::Msg::Seeked(pos)).ok();
        Ok(())
    }

    pub async fn set_volume(&self, v: f64) -> Result<()> {
        let v = v.clamp(0.0, 100.0);
        self.audio().await?.set("volume", json!(v)).await?;
        settings::VOLUME.set(&self.library, &v);
        let mut st = self.st.lock().unwrap();
        st.play.volume = v;
        self.publish_state(&mut st);
        Ok(())
    }

    fn position(&self, index: usize, id: Option<&TrackId>) -> Result<usize> {
        let st = self.st.lock().unwrap();
        st.queue.locate(index, id).ok_or_else(|| Code::ApiNotFound.err("that track is no longer in the queue"))
    }

    pub async fn jump(self: &Arc<Self>, index: usize, id: Option<&TrackId>) -> Result<()> {
        self.chose_to_move();
        let index = self.position(index, id)?;
        self.load(index).await
    }

    pub async fn remove(self: &Arc<Self>, index: usize, id: Option<&TrackId>) -> Result<()> {
        let index = self.position(index, id)?;
        let current = {
            let mut st = self.st.lock().unwrap();
            let Some(was_current) = st.queue.remove(index) else { return Err(Code::ApiNotFound.err(format!("no queue item {index}"))) };
            self.publish_queue(&mut st);
            was_current.then_some(st.queue.cursor)
        };
        match current {
            Some(Some(next)) => self.load(next).await,
            Some(None) => {
                self.halt().await;
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub async fn move_item(&self, from: usize, to: usize, id: Option<&TrackId>) -> Result<()> {
        let from = self.position(from, id)?;
        let mut st = self.st.lock().unwrap();
        if !st.queue.move_item(from, to) {
            return Err(Code::ApiArgs.err(format!("queue has {} items, cannot move {from} to {to}", st.queue.tracks.len())));
        }
        self.publish_queue(&mut st);
        Ok(())
    }

    pub fn clear_others(&self) -> bool {
        let mut st = self.st.lock().unwrap();
        let kept = st.queue.keep_current();
        if kept {
            self.publish_queue(&mut st);
        }
        kept
    }

    pub async fn clear(&self) {
        {
            let mut st = self.st.lock().unwrap();
            st.queue.clear();
            st.resume = None;
            st.origin = None;
        }
        self.halt().await;
    }

    pub async fn set_shuffle(&self, on: bool) {
        let mut st = self.st.lock().unwrap();
        st.queue.set_shuffle(on);
        self.publish_queue(&mut st);
        self.publish_modes(&mut st);
    }

    pub async fn set_repeat(&self, mode: Repeat) {
        let mut st = self.st.lock().unwrap();
        st.repeat = mode;
        self.publish_modes(&mut st);
    }

    pub async fn set_fx(&self, fx: Fx) -> Result<()> {
        let fx = fx::clamp(&fx);
        let measuring = self.st.lock().unwrap().measuring;
        apply_fx(&*self.audio().await?, &fx, measuring).await?;
        self.st.lock().unwrap().fx = fx;
        Ok(())
    }

    pub async fn devices(&self) -> Result<Value> {
        let audio = self.audio().await?;
        let devices = read_devices(&audio.get("audio-device-list").await?);
        let active = audio.get("audio-device").await?;
        let mut st = self.st.lock().unwrap();
        st.devices = devices;
        Ok(json!({ "devices": st.devices, "wanted": st.device, "active": active }))
    }

    pub async fn set_device(&self, name: &str) -> Result<()> {
        let audio = self.audio().await?;
        self.st.lock().unwrap().device = name.to_owned();
        let devices = read_devices(&audio.get("audio-device-list").await?);
        self.route(&*audio, devices).await
    }

    async fn route(&self, audio: &dyn AudioOut, devices: Vec<AudioDevice>) -> Result<()> {
        let target = {
            let mut st = self.st.lock().unwrap();
            st.devices = devices;
            let present = st.devices.iter().any(|d| d.name == st.device);
            if present { st.device.clone() } else { "auto".to_owned() }
        };
        if audio.get("audio-device").await?.as_str() != Some(target.as_str()) {
            audio.set("audio-device", json!(target)).await?;
        }
        Ok(())
    }

    fn devices_changed(&self, list: &Value) {
        let devices = read_devices(list);
        let Some(this) = self.me.upgrade() else { return };
        self.tasks.spawn("follow the output devices", async move {
            if let Ok(audio) = this.audio().await
                && let Err(e) = this.route(&*audio, devices).await
            {
                snellar_core::journal::warn("audio", format!("cannot switch the output device: {e:#}"));
            }
        });
    }

    pub async fn set_loudness(&self, on: bool) -> Result<()> {
        let audio = self.audio().await?;
        let (change, gain) = {
            let mut st = self.st.lock().unwrap();
            st.loudness = on;
            let playing = st.handed == st.generation && st.queue.current().is_some();
            if playing {
                st.stream.leveling = leveling(on, st.stream.gain);
                self.emit(UiEvent::Stream(st.stream.clone()));
            }
            let measure = playing && st.stream.leveling == Leveling::Measured;
            let change = (measure != st.measuring).then(|| (st.fx.clone(), measure));
            st.measuring = measure;
            (change, if on { st.stream.gain.unwrap_or(0.0) } else { 0.0 })
        };
        audio.set("replaygain-fallback", json!(gain)).await?;
        if let Some((fx, measure)) = change {
            apply_fx(&*audio, &fx, measure).await?;
        }
        Ok(())
    }

    fn emit(&self, event: UiEvent) {
        match &event {
            UiEvent::State(s) => {
                self.mpris.send(Update::Playing(s.playing)).ok();
                self.discord.send(discord::Msg::Playing(s.playing)).ok();
                self.mpris.send(Update::Volume(s.volume / 100.0)).ok();
            }
            UiEvent::Position(p) => {
                self.mpris.send(Update::Position(*p)).ok();
                self.discord.send(discord::Msg::Position(*p)).ok();
            }
            UiEvent::Current(t) => {
                self.mpris.send(Update::Track(t.clone())).ok();
                self.discord.send(discord::Msg::Track(t.clone())).ok();
            }
            UiEvent::Modes { shuffle, repeat } => {
                self.mpris.send(Update::Modes { shuffle: *shuffle, repeat: *repeat }).ok();
            }
            _ => {}
        }
        (self.ui)(event);
    }

    fn publish_state(&self, st: &mut State) {
        st.play.playing = !st.paused && !st.idle;
        st.play.loading = st.starting || st.cache || st.seeking;
        self.emit(UiEvent::State(st.play.clone()));
    }

    fn publish_queue(&self, st: &mut State) {
        self.recheck_preload(st);
        let cursor = st.queue.cursor.map_or(-1, |c| c as i32);
        self.emit(UiEvent::Queue { tracks: st.queue.tracks.clone(), cursor });
        settings::QUEUE.set(&self.library, &Saved::of(st));
    }

    fn timed(&self, mut tracks: Vec<Track>) -> Vec<Track> {
        self.library.fill_lengths(&mut tracks);
        tracks
    }

    fn restore(&self) {
        let Some(saved) = settings::QUEUE.get::<Saved>(&self.library) else { return };
        let position = settings::POSITION.or_default(&self.library);
        let mut st = self.st.lock().unwrap();
        st.queue.cursor = saved.cursor.filter(|c| *c < saved.tracks.len());
        st.queue.tracks = self.timed(saved.tracks);
        st.origin = saved.origin;
        let Some(track) = st.queue.current().cloned() else { return };
        st.resume = Some(position);
        st.play.position = position;
        st.play.duration = track.duration;
        self.publish_queue(&mut st);
        self.emit(UiEvent::Current(Some(track.clone())));
        self.emit(UiEvent::Position(position));
        self.emit(UiEvent::Liked(self.library.is_liked(&track.id)));
    }

    fn publish_modes(&self, st: &mut State) {
        self.recheck_preload(st);
        self.emit(UiEvent::Modes { shuffle: st.queue.shuffle, repeat: st.repeat });
    }

    fn replace_queue(&self, tracks: Vec<Track>, index: usize) -> u64 {
        let mut st = self.st.lock().unwrap();
        st.queue.replace(tracks, index);
        st.resume = None;
        st.origin = None;
        st.queue.epoch
    }

    fn finish_listen(&self, st: &mut State, ended: bool) {
        if let Some(id) = st.listening.take() {
            self.library.record_listen(&id, st.listened, !ended && st.listened < 30.0);
        }
    }

    async fn audio(&self) -> Result<Arc<dyn AudioOut>> {
        let mut slot = self.audio.lock().await;
        if let Some(audio) = &*slot {
            return Ok(audio.clone());
        }
        let (audio, events) = (self.launch)().await?;
        self.watch(events);
        let (volume, fx, measuring) = {
            let st = self.st.lock().unwrap();
            (st.play.volume, st.fx.clone(), st.measuring)
        };
        audio.set("volume", json!(volume)).await?;
        let devices = read_devices(&audio.get("audio-device-list").await?);
        self.route(&*audio, devices).await?;
        if let Err(e) = apply_fx(&*audio, &fx, measuring).await {
            apply_fx(&*audio, &Fx::default(), measuring).await?;
            self.st.lock().unwrap().fx = Fx::default();
            settings::FX.set(&self.library, &Fx::default());
            self.emit(UiEvent::Toast(Toast::failed(Code::FxReset, &e)));
        }
        *slot = Some(audio.clone());
        Ok(audio)
    }

    fn watch(&self, events: UnboundedReceiver<Event>) {
        self.tasks.spawn("audio events", run_events(self.me.clone(), events));
    }

    async fn halt(&self) {
        {
            let mut st = self.st.lock().unwrap();
            st.generation += 1;
            st.starting = false;
            st.forget_failures();
            st.marks.clear();
            st.fallback_cause = None;
            self.emit(UiEvent::PlayFailure(None));
            self.finish_listen(&mut st, false);
            st.play.position = 0.0;
            st.play.duration = 0.0;
            st.stream = StreamInfo::default();
            self.publish_state(&mut st);
            self.publish_queue(&mut st);
            self.emit(UiEvent::Stream(StreamInfo::default()));
        }
        self.emit(UiEvent::Position(0.0));
        self.emit(UiEvent::Current(None));
        if let Some(audio) = self.audio.lock().await.clone() {
            audio.stop().await.ok();
        }
    }

    async fn load_cursor(self: &Arc<Self>) -> Result<()> {
        let cursor = self.st.lock().unwrap().queue.cursor;
        match cursor {
            Some(i) => self.load(i).await,
            None => Ok(()),
        }
    }

    fn superseded(&self, generation: u64) -> bool {
        self.st.lock().unwrap().generation != generation
    }

    fn take_cursor(&self, st: &mut State, index: usize, track: &Track, start: f64) -> u64 {
        self.finish_listen(st, false);
        self.forget_preload(st);
        st.queue.cursor = Some(index);
        st.generation += 1;
        st.stall += 1;
        st.starting = true;
        st.attempts = 0;
        st.retry_point = None;
        st.retrying = false;
        st.fallback_cause = None;
        if st.failed.as_ref().is_some_and(|(id, _)| *id != track.id) {
            st.failed = None;
            st.stopped = false;
            st.auto_retries = 0;
            self.emit(UiEvent::PlayFailure(st.failure()));
        }
        st.play.position = start;
        st.play.duration = track.duration;
        settings::POSITION.set(&self.library, &start);
        self.publish_state(st);
        self.publish_queue(st);
        st.generation
    }

    fn announce(&self, track: &Track, start: f64) {
        self.emit(UiEvent::Current(Some(track.clone())));
        self.emit(UiEvent::Position(start));
        self.emit(UiEvent::Liked(self.library.is_liked(&track.id)));
    }

    fn started(self: &Arc<Self>, mut st: std::sync::MutexGuard<'_, State>, index: usize, track: &Track) {
        let card = st.origin.as_ref().filter(|o| o.tracks.contains(&track.id)).map(|o| &o.card);
        self.library.push_played(track, card);
        self.emit(UiEvent::History);
        st.marks.remove(&track.id.to_string());
        st.failed = None;
        st.stopped = false;
        st.retrying = false;
        st.auto_retries = 0;
        self.emit(UiEvent::PlayFailure(None));
        st.listening = Some(track.id.clone());
        st.listened = 0.0;
        for next in st.queue.tracks.iter().skip(index + 1).take(PREFETCH_AHEAD) {
            self.prefetch(&next.id);
        }
        drop(st);
        self.top_up();
    }

    async fn load(self: &Arc<Self>, index: usize) -> Result<()> {
        let start;
        let (track, generation) = {
            let mut st = self.st.lock().unwrap();
            let Some(track) = st.queue.tracks.get(index).cloned() else {
                return Err(Code::ApiNotFound.err(format!("no queue item {index}")));
            };
            if !self.source.owns(&track.id) {
                return Err(Code::SourceUnsupported.err(format!("`{}` is not a {} track", track.id, self.source.name())));
            }
            start = st.resume.take().filter(|_| st.queue.cursor == Some(index)).unwrap_or(0.0);
            let generation = self.take_cursor(&mut st, index, &track, start);
            (track, generation)
        };
        self.announce(&track, start);

        let (stream, cause) = match self.open(&track.id, false).await {
            Ok(stream) => (Ok(stream), None),
            Err(e) => match worth_a_fallback(Code::of(&e)).then(|| self.source.fallback(&track.id)).flatten() {
                Some(url) => (Ok((Stream::url(url), false)), Some(e)),
                None => (Err(e), None),
            },
        };
        let audio = self.audio().await;
        let started = match (audio, stream) {
            (Ok(audio), Ok((stream, local))) => {
                if self.superseded(generation) {
                    return Ok(());
                }
                match self.prepare(&*audio, stream.format.as_ref(), local).await {
                    Ok(gain) => {
                        audio.set("pause", json!(false)).await.ok();
                        if self.superseded(generation) {
                            return Ok(());
                        }
                        audio.load(&stream.url, start, Some(gain)).await
                    }
                    Err(e) => Err(e),
                }
            }
            (Err(e), _) => Err(e),
            (Ok(_), Err(e)) => Err(e),
        };

        let mut st = self.st.lock().unwrap();
        if st.generation != generation {
            return Ok(());
        }
        match started {
            Ok(()) => {
                st.attempts = if cause.is_some() { 2 } else { 0 };
                st.retry_point = cause.is_some().then_some(start);
                st.fallback_cause = cause;
                st.handed = generation;
                self.started(st, index, &track);
                Ok(())
            }
            Err(e) => {
                st.starting = false;
                st.cache = false;
                let first = !st.retrying && st.auto_retries == 0;
                self.finish_listen(&mut st, false);
                self.publish_state(&mut st);
                let next = self.note_failure(&mut st, &e);
                drop(st);
                if first {
                    self.emit(UiEvent::Toast(Toast::failed(Code::PlayFailed, &e)));
                }
                match next {
                    AfterFailure::Skip => self.skip_failed(generation),
                    AfterFailure::Wait(wait) => self.retry_later(index, generation, wait),
                    AfterFailure::Stay => {}
                }
                Err(e.context(Told))
            }
        }
    }

    fn note_failure(&self, st: &mut State, e: &anyhow::Error) -> AfterFailure {
        let code = Code::of(e);
        let Some(id) = st.queue.current().map(|t| t.id.clone()) else { return AfterFailure::Stay };
        st.failures = st.failures.saturating_add(1);
        st.marks.insert(id.to_string(), code);
        st.failed = Some((id, code));
        st.retrying = false;
        let next = if code.passes_by_itself() && st.auto_retries < AUTO_RETRIES {
            let wait = AUTO_RETRY_BASE * 2u32.pow(u32::from(st.auto_retries));
            st.auto_retries += 1;
            st.retrying = true;
            AfterFailure::Wait(Code::retry_after(e).map_or(wait, |after| wait.max(Duration::from_secs(after))))
        } else if code.network() || st.queue.next_index().is_none() {
            AfterFailure::Stay
        } else if st.failures < MAX_FAILURES {
            AfterFailure::Skip
        } else {
            st.stopped = true;
            AfterFailure::Stay
        };
        self.emit(UiEvent::PlayFailure(st.failure()));
        next
    }

    fn skip_failed(self: &Arc<Self>, generation: u64) {
        let this = self.clone();
        self.tasks.spawn("skip a failed track", async move {
            this.advance(Some(generation)).await.ok();
        });
    }

    fn retry_later(self: &Arc<Self>, index: usize, generation: u64, wait: Duration) {
        let this = self.clone();
        self.tasks.spawn("retry a failed start", async move {
            tokio::time::sleep(wait).await;
            if !this.superseded(generation) {
                this.load(index).await.ok();
            }
        });
    }

    fn recheck_preload(&self, st: &mut State) {
        let Some(preload) = &st.preload else { return };
        if preload.generation == st.generation && st.following_id() == Some(&preload.id) {
            return;
        }
        self.forget_preload(st);
    }

    fn forget_preload(&self, st: &mut State) {
        if st.preload.take().is_none() {
            return;
        }
        let Some(this) = self.me.upgrade() else { return };
        self.tasks.spawn("forget the preloaded track", async move {
            if let Ok(audio) = this.audio().await {
                audio.clear_next().await.ok();
            }
        });
    }

    fn preload_soon(&self, st: &mut State, position: f64) {
        if st.starting || st.preload.is_some() || st.preloading == Some(st.generation) || st.play.duration <= 0.0 {
            return;
        }
        if position < st.play.duration - PRELOAD_LEAD {
            return;
        }
        let Some(id) = st.following_id().cloned() else { return };
        let Some(this) = self.me.upgrade() else { return };
        let generation = st.generation;
        st.preloading = Some(generation);
        self.tasks.spawn("preload the next track", async move { this.preload(id, generation).await });
    }

    async fn preload(self: &Arc<Self>, id: TrackId, generation: u64) {
        let fits = |st: &State| st.generation == generation && st.following_id() == Some(&id);
        let (Ok((stream, local)), Ok(audio)) = (self.open(&id, false).await, self.audio().await) else { return };
        let gain = {
            let st = self.st.lock().unwrap();
            let gain = st.gain(stream.format.as_ref());
            if !fits(&st) || (st.loudness && gain.is_none()) != st.measuring {
                return;
            }
            gain.filter(|_| st.loudness).unwrap_or(0.0)
        };
        if audio.append(&stream.url, Some(gain)).await.is_err() {
            return;
        }
        let kept = {
            let mut st = self.st.lock().unwrap();
            let kept = fits(&st);
            if kept {
                st.preload = Some(Preload { id, generation, format: stream.format, local });
            }
            kept
        };
        if !kept {
            audio.clear_next().await.ok();
        }
    }

    fn hand_over(self: &Arc<Self>) -> bool {
        let mut st = self.st.lock().unwrap();
        let Some(preload) = st.preload.take() else { return false };
        let Some(index) = st.following().filter(|i| preload.generation == st.generation && st.queue.tracks[*i].id == preload.id) else {
            return false;
        };
        let track = st.queue.tracks[index].clone();
        let generation = self.take_cursor(&mut st, index, &track, 0.0);
        st.handed = generation;
        self.describe(&mut st, preload.format.as_ref(), preload.local);
        drop(st);
        self.announce(&track, 0.0);
        self.started(self.st.lock().unwrap(), index, &track);
        true
    }

    async fn advance(self: &Arc<Self>, heard: Option<u64>) -> Result<()> {
        enum Step {
            Load(usize),
            Radio(Track, Vec<Track>, u64, u64),
            Stay,
        }
        let step = {
            let st = self.st.lock().unwrap();
            if heard.is_some_and(|generation| generation != st.generation) {
                return Ok(());
            }
            let q = &st.queue;
            match (q.cursor, q.next_index(), q.tracks.last()) {
                (Some(c), _, _) if heard.is_some() && st.repeat == Repeat::One => Step::Load(c),
                (_, Some(n), _) => Step::Load(n),
                (_, None, Some(_)) if st.repeat == Repeat::All => Step::Load(0),
                (_, None, Some(last)) if self.autoplay() && self.has_radio() => {
                    Step::Radio(last.clone(), q.tracks.clone(), q.epoch, st.generation)
                }
                _ => Step::Stay,
            }
        };
        match step {
            Step::Load(i) => self.load(i).await,
            Step::Stay => Ok(()),
            Step::Radio(seed, queue, epoch, generation) => match self.related(&seed, &queue).await {
                Ok(related) => {
                    let first = {
                        let mut st = self.st.lock().unwrap();
                        let still_current = st.generation == generation;
                        let first = st.queue.settle_radio(still_current, epoch, related);
                        self.publish_queue(&mut st);
                        first
                    };
                    match first {
                        Some(i) => self.load(i).await,
                        None => Ok(()),
                    }
                }
                Err(e) => {
                    self.emit(UiEvent::Toast(Toast::failed(Code::AutoplayFailed, &e)));
                    Err(e)
                }
            },
        }
    }

    async fn related(&self, seed: &Track, queue: &[Track]) -> Result<Vec<Track>> {
        self.source.radio(seed, queue).await
    }

    fn has_radio(&self) -> bool {
        self.source.capabilities().radio
    }

    fn top_up(self: &Arc<Self>) {
        let (seed, queue, epoch) = {
            let mut st = self.st.lock().unwrap();
            let upcoming = st.queue.tracks.len() - st.queue.cursor.map_or(0, |c| c + 1);
            if st.topping || upcoming > TOP_UP_AT || st.repeat == Repeat::All || !self.autoplay() || !self.has_radio() {
                return;
            }
            let Some(seed) = st.queue.tracks.last().cloned() else { return };
            st.topping = true;
            (seed, st.queue.tracks.clone(), st.queue.epoch)
        };
        let this = self.clone();
        self.tasks.spawn("radio top-up", async move {
            let related = this.related(&seed, &queue).await;
            let mut st = this.st.lock().unwrap();
            st.topping = false;
            if let Ok(tracks) = related {
                st.queue.append_if_current(epoch, tracks);
                this.publish_queue(&mut st);
            }
        });
    }

    fn describe(&self, st: &mut State, format: Option<&AudioFormat>, local: bool) -> Option<(Fx, bool)> {
        let gain = st.gain(format);
        let format = format.cloned().unwrap_or_default();
        st.stream = StreamInfo {
            codec: format.codec,
            bitrate: format.bitrate,
            sample_rate: format.sample_rate,
            gain,
            leveling: leveling(st.loudness, gain),
            local,
        };
        self.emit(UiEvent::Stream(st.stream.clone()));
        let measure = st.stream.leveling == Leveling::Measured;
        let change = (measure != st.measuring).then(|| (st.fx.clone(), measure));
        st.measuring = measure;
        change
    }

    async fn prepare(&self, audio: &dyn AudioOut, format: Option<&AudioFormat>, local: bool) -> Result<f64> {
        let (change, gain) = {
            let mut st = self.st.lock().unwrap();
            let change = self.describe(&mut st, format, local);
            (change, st.stream.gain.filter(|_| st.loudness).unwrap_or(0.0))
        };
        if let Some((fx, measure)) = change {
            apply_fx(audio, &fx, measure).await?;
        }
        Ok(gain)
    }

    fn autoplay(&self) -> bool {
        settings::AUTOPLAY.or_default(&self.library)
    }

    fn learn_duration(&self, st: &mut State) {
        let Some(duration) = st.heard_duration.filter(|d| *d > 0.0 && st.handed == st.generation) else { return };
        st.play.duration = duration;
        let Some(cursor) = st.queue.cursor else { return };
        let Some(track) = st.queue.tracks.get_mut(cursor).filter(|t| t.duration <= 0.0) else { return };
        track.duration = duration;
        let track = track.clone();
        self.library.remember_track(&track);
        self.emit(UiEvent::History);
        self.publish_queue(st);
        (self.ui)(UiEvent::Current(Some(track.clone())));
        self.mpris.send(Update::Track(Some(track))).ok();
        self.discord.send(discord::Msg::Duration(duration)).ok();
    }

    fn property(&self, name: &str, value: Value, last_position: &mut Instant) {
        let mut st = self.st.lock().unwrap();
        match (name, &value) {
            ("time-pos", Value::Number(n)) => {
                if st.handed != st.generation {
                    return;
                }
                let t = n.as_f64().unwrap_or_default();
                self.learn_duration(&mut st);
                st.play.position = t;
                st.listened = st.listened.max(t);
                if t >= PLAYED_ENOUGH {
                    st.failures = 0;
                }
                if st.retry_point.is_some_and(|point| t >= point + PLAYED_ENOUGH) {
                    st.retry_point = None;
                    st.attempts = 0;
                }
                if st.starting {
                    st.starting = false;
                    self.publish_state(&mut st);
                }
                self.preload_soon(&mut st, t);
                if st.saved.elapsed() >= SAVE_INTERVAL {
                    st.saved = Instant::now();
                    settings::POSITION.set(&self.library, &t);
                }
                if last_position.elapsed() >= POSITION_INTERVAL {
                    *last_position = Instant::now();
                    self.emit(UiEvent::Position(t));
                }
                return;
            }
            ("duration", Value::Number(n)) => {
                st.heard_duration = n.as_f64();
                self.learn_duration(&mut st);
            }
            ("duration", Value::Null) => {
                st.heard_duration = None;
                return;
            }
            ("pause", Value::Bool(b)) => st.paused = *b,
            ("idle-active", Value::Bool(b)) => st.idle = *b,
            ("paused-for-cache", Value::Bool(b)) => {
                st.cache = *b;
                if *b {
                    st.stall += 1;
                    self.watch_stall(st.stall);
                }
            }
            ("seeking", Value::Bool(b)) => st.seeking = *b,
            ("volume", Value::Number(n)) => st.play.volume = n.as_f64().unwrap_or_default(),
            ("audio-device-list", list) => {
                drop(st);
                self.devices_changed(list);
                return;
            }
            ("audio-codec-name", Value::String(codec)) => {
                if st.handed == st.generation && st.stream.codec.is_empty() {
                    st.stream.codec = codec.clone();
                    self.emit(UiEvent::Stream(st.stream.clone()));
                }
                return;
            }
            _ => return,
        }
        self.publish_state(&mut st);
    }

    async fn ended(self: &Arc<Self>, reason: &str, error: Option<String>, heard: u64) {
        if self.superseded(heard) {
            return;
        }
        match reason {
            "eof" => {
                let (position, duration, local) = {
                    let st = self.st.lock().unwrap();
                    (st.play.position, st.play.duration, st.stream.local)
                };
                if !local && cut_short(position, duration) && matches!(self.retry().await, Ok(true)) {
                    return;
                }
                if self.superseded(heard) {
                    return;
                }
                self.finish_listen(&mut self.st.lock().unwrap(), true);
                if !self.hand_over() {
                    self.advance(Some(heard)).await.ok();
                }
            }
            "error" => {
                let result = match self.retry().await {
                    Ok(true) => Ok(()),
                    Ok(false) => {
                        let cause = self.st.lock().unwrap().fallback_cause.take();
                        Err(cause.unwrap_or_else(|| Code::AudioFailed.err(error.unwrap_or_else(|| "playback failed".into()))))
                    }
                    Err(e) => Err(e),
                };
                if let Err(e) = result {
                    self.failed(e, heard).await;
                }
            }
            _ => {}
        }
    }

    async fn failed(self: &Arc<Self>, e: anyhow::Error, heard: u64) {
        let (next, index) = {
            let mut st = self.st.lock().unwrap();
            if st.generation != heard {
                return;
            }
            st.starting = false;
            st.cache = false;
            self.finish_listen(&mut st, false);
            self.publish_state(&mut st);
            (self.note_failure(&mut st, &e), st.queue.cursor)
        };
        self.emit(UiEvent::Toast(Toast::failed(Code::PlayFailed, &e)));
        match (next, index, self.me.upgrade()) {
            (AfterFailure::Skip, _, _) => {
                self.advance(Some(heard)).await.ok();
            }
            (AfterFailure::Wait(wait), Some(index), Some(this)) => this.retry_later(index, heard, wait),
            _ => {}
        }
    }

    fn watch_stall(&self, token: u64) {
        let Some(this) = self.me.upgrade() else { return };
        self.tasks.spawn("stall watch", async move {
            tokio::time::sleep(STALL_AFTER).await;
            this.stall_expired(token).await;
        });
    }

    fn still_stalled(&self, token: u64) -> bool {
        let st = self.st.lock().unwrap();
        st.cache && st.stall == token
    }

    async fn stall_expired(self: &Arc<Self>, token: u64) {
        if !self.still_stalled(token) {
            return;
        }
        let heard = self.st.lock().unwrap().generation;
        match self.retry().await {
            Ok(true) => {}
            Ok(false) => self.failed(Code::NetTimeout.err("the stream stalled"), heard).await,
            Err(e) => self.failed(e, heard).await,
        }
    }

    async fn retry(&self) -> Result<bool> {
        let (id, position, attempt, generation) = {
            let mut st = self.st.lock().unwrap();
            st.attempts += 1;
            st.retry_point = Some(st.play.position);
            let id = st.queue.current().map(|t| t.id.clone()).filter(|id| self.source.owns(id));
            (id, st.play.position, st.attempts, st.generation)
        };
        let Some(id) = id else { return Ok(false) };
        let found = async {
            let audio = self.audio().await?;
            let stream = match attempt {
                1 => Some(self.source.stream(&id, true).await?),
                2 => self.source.fallback(&id).map(Stream::url),
                _ => None,
            };
            Ok::<_, anyhow::Error>((audio, stream))
        };
        let found = found.await;
        if self.superseded(generation) {
            return Ok(true);
        }
        let (audio, Some(stream)) = found? else { return Ok(false) };
        self.st.lock().unwrap().preload = None;
        let gain = self.prepare(&*audio, stream.format.as_ref(), false).await?;
        if self.superseded(generation) {
            return Ok(true);
        }
        audio.load(&stream.url, position, Some(gain)).await?;
        Ok(true)
    }

    fn died(&self) {
        let mut st = self.st.lock().unwrap();
        st.idle = true;
        st.starting = false;
        st.cache = false;
        st.seeking = false;
        if st.queue.current().is_some() {
            st.resume = Some(st.play.position);
        }
        self.publish_state(&mut st);
        self.emit(UiEvent::Toast(Toast::new(Code::AudioRestart)));
    }
}

#[derive(Debug, PartialEq)]
enum AfterFailure {
    Skip,
    Wait(Duration),
    Stay,
}

fn worth_a_fallback(code: Code) -> bool {
    !code.network() && !matches!(code, Code::YtdlpMissing | Code::YtUnavailable)
}

fn read_devices(list: &Value) -> Vec<AudioDevice> {
    Vec::<AudioDevice>::deserialize(list).unwrap_or_default().into_iter().filter(|d| d.name != "auto").collect()
}

fn leveling(on: bool, gain: Option<f64>) -> Leveling {
    match (on, gain) {
        (false, _) => Leveling::Off,
        (true, Some(_)) => Leveling::Gain,
        (true, None) => Leveling::Measured,
    }
}

async fn apply_fx(audio: &dyn AudioOut, fx: &Fx, measure: bool) -> Result<()> {
    audio.set("af", json!(fx::chain(fx, measure))).await?;
    audio.set("audio-pitch-correction", json!(fx.pitch_lock)).await?;
    audio.set("speed", json!(if fx.enabled { fx.speed } else { 1.0 })).await
}

async fn run_events(player: Weak<Player>, mut events: UnboundedReceiver<Event>) {
    let mut last_position = Instant::now();
    let mut previous_end: Option<oneshot::Receiver<()>> = None;
    while let Some(event) = events.recv().await {
        let Some(p) = player.upgrade() else { return };
        match event {
            Event::Property(name, value) => p.property(&name, value, &mut last_position),
            Event::EndFile { reason, error } => {
                let heard = {
                    let mut st = p.st.lock().unwrap();
                    st.heard_duration = None;
                    st.handed
                };
                let (finished, turn) = oneshot::channel();
                let before = previous_end.replace(turn);
                let player = p.clone();
                p.tasks.spawn("end of file", async move {
                    if let Some(before) = before {
                        before.await.ok();
                    }
                    player.ended(&reason, error, heard).await;
                    finished.send(()).ok();
                });
            }
            Event::Died => {
                p.audio.lock().await.take();
                p.died();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use snellar_core::source::Capabilities;
    use snellar_youtube::lyrics::LyricsSource;
    use snellar_youtube::proxy::Proxy;
    use snellar_youtube::recs::Recs;
    use snellar_youtube::source::YouTube;
    use snellar_youtube::stream::Resolver;
    use snellar_youtube::ytm::Ytm;

    use crate::fakes::{self, FakeAudio, FakeSource, until};

    fn track(id: &str) -> Track {
        Track {
            id: id.into(),
            title: format!("title {id}"),
            artists: vec![snellar_core::model::Ref { id: "ar".into(), name: "Artist".into() }],
            duration: 213.0,
            ..Track::default()
        }
    }

    fn ids(q: &Queue) -> String {
        q.tracks.iter().map(|t| t.id.as_str()).collect()
    }

    fn queue(ids: &str, cursor: usize) -> Queue {
        let mut q = Queue::new();
        q.replace(ids.chars().map(|c| track(&c.to_string())).collect(), cursor);
        q
    }

    #[test]
    fn enqueue_next_and_append() {
        let mut q = queue("abc", 1);
        q.insert(track("x"), true);
        q.insert(track("y"), false);
        assert_eq!(ids(&q), "abxcy");
        assert_eq!(q.cursor, Some(1));
    }

    #[test]
    fn enqueue_several_keeps_their_order() {
        let mut q = queue("abc", 1);
        assert_eq!(q.insert_all(vec![track("x"), track("y")], true), Some(2));
        assert_eq!(q.insert_all(vec![track("u"), track("v")], false), Some(5));
        assert_eq!(q.insert_all(Vec::new(), true), None);
        assert_eq!((ids(&q).as_str(), q.cursor), ("abxycuv", Some(1)));
        q.set_shuffle(true);
        q.insert_all(vec![track("p"), track("q")], true);
        assert_eq!(&ids(&q)[..4], "abpq");
        q.set_shuffle(false);
        assert_eq!(ids(&q), "abpqxycuv");
    }

    #[test]
    fn clearing_the_others_keeps_the_current_track() {
        let mut q = queue("abcde", 2);
        let epoch = q.epoch;
        assert!(q.keep_current());
        assert_eq!((ids(&q).as_str(), q.cursor), ("c", Some(0)));
        assert!(q.epoch > epoch, "a late radio answer for the old queue is dropped");
        assert_eq!(q.append_new(vec![track("a"), track("f")]), Some(1));
        assert_eq!(ids(&q), "cf", "cleared tracks are not brought back by autoplay");
        let mut shuffled = queue("abcd", 1);
        shuffled.set_shuffle(true);
        shuffled.keep_current();
        shuffled.set_shuffle(false);
        assert_eq!((ids(&shuffled).as_str(), shuffled.cursor), ("b", Some(0)));
        let mut idle = Queue::new();
        assert!(!idle.keep_current());
    }

    #[test]
    fn remove_adjusts_cursor() {
        let mut q = queue("abcde", 2);
        assert_eq!(q.remove(0), Some(false));
        assert_eq!((ids(&q).as_str(), q.cursor), ("bcde", Some(1)));
        assert_eq!(q.remove(3), Some(false));
        assert_eq!(q.cursor, Some(1));
        assert_eq!(q.remove(1), Some(true));
        assert_eq!((ids(&q).as_str(), q.cursor), ("bd", Some(1)));
        assert_eq!(q.remove(1), Some(true));
        assert_eq!(q.cursor, None);
        assert_eq!(q.remove(5), None);
    }

    #[test]
    fn move_keeps_current_track() {
        let mut q = queue("abcde", 2);
        q.move_item(0, 4);
        assert_eq!((ids(&q).as_str(), q.cursor), ("bcdea", Some(1)));
        q.move_item(4, 0);
        assert_eq!((ids(&q).as_str(), q.cursor), ("abcde", Some(2)));
        q.move_item(2, 3);
        assert_eq!((ids(&q).as_str(), q.cursor), ("abdce", Some(3)));
        q.move_item(4, 1);
        assert_eq!((ids(&q).as_str(), q.cursor), ("aebdc", Some(4)));
    }

    #[test]
    fn shuffle_only_touches_upcoming_and_restores() {
        let mut q = queue("abcdefghij", 3);
        q.set_shuffle(true);
        assert_eq!(&ids(&q)[..4], "abcd");
        let mut rest: Vec<_> = ids(&q)[4..].chars().collect();
        rest.sort();
        assert_eq!(rest.into_iter().collect::<String>(), "efghij");
        q.insert(track("z"), false);
        q.set_shuffle(false);
        assert_eq!(ids(&q), "abcdefghijz");
        assert_eq!(q.cursor, Some(3));
    }

    #[test]
    fn shuffled_play_starts_with_chosen() {
        let mut q = Queue::new();
        q.set_shuffle(true);
        q.replace("abcdef".chars().map(|c| track(&c.to_string())).collect(), 3);
        assert_eq!((q.cursor, &ids(&q)[..1]), (Some(0), "d"));
        q.set_shuffle(false);
        assert_eq!((ids(&q).as_str(), q.cursor), ("abcdef", Some(3)));
    }

    #[test]
    fn append_new_skips_duplicates() {
        let mut q = queue("ab", 1);
        assert_eq!(q.append_new(vec![track("b"), track("c"), track("d")]), Some(2));
        assert_eq!(q.append_new(vec![track("c")]), None);
        assert_eq!(ids(&q), "abcd");
        assert_eq!(q.next_index(), Some(2));
    }

    async fn start(ui: Ui, library: Arc<Library>) -> Arc<Player> {
        rig(ui, library, Capabilities { radio: true }).await.0
    }

    async fn rig(ui: Ui, library: Arc<Library>, capabilities: Capabilities) -> (Arc<Player>, Arc<FakeSource>, Arc<FakeAudio>) {
        let (source, audio) = (FakeSource::new(capabilities), FakeAudio::new());
        let player = Player::start(ui, source.clone(), audio.launch(), library).await.unwrap();
        (player, source, audio)
    }

    async fn youtube(ui: Ui, library: Arc<Library>) -> Arc<Player> {
        let http = reqwest::Client::new();
        let ytm = Arc::new(Ytm::new(http.clone()));
        let recs = Recs::new(http.clone(), ytm.clone(), library.clone(), std::env::temp_dir());
        let resolver = Arc::new(Resolver::new(std::env::temp_dir()));
        let proxy = Proxy::serve(snellar_youtube::proxy::bind().unwrap(), resolver.clone(), std::env::temp_dir()).unwrap();
        let source = YouTube::new(ytm, recs, resolver, proxy, LyricsSource::new(http));
        Player::start(ui, source, crate::mpv::launch(snellar_youtube::net::Route::default()), library).await.unwrap()
    }

    #[test]
    fn locate_follows_the_track_not_the_stale_index() {
        let q = queue("abcdef", 0);
        assert_eq!(q.locate(2, Some(&"c".into())), Some(2));
        assert_eq!(q.locate(5, Some(&"c".into())), Some(2));
        assert_eq!(q.locate(0, Some(&"z".into())), None);
        assert_eq!(q.locate(9, None), None);
        assert_eq!(q.locate(3, None), Some(3));
        let mut twice = queue("abca", 0);
        assert_eq!(twice.locate(3, Some(&"a".into())), Some(3));
        assert_eq!(twice.locate(1, Some(&"a".into())), Some(0));
        twice.remove(0);
        assert_eq!(twice.locate(0, Some(&"a".into())), Some(2));
    }

    #[test]
    fn removing_then_jumping_then_a_late_top_up_keeps_the_users_queue() {
        let mut q = queue("abcdefg", 0);
        let epoch = q.epoch;
        for gone in ["e", "d", "c"] {
            let at = q.locate(0, Some(&gone.into())).unwrap();
            q.remove(at);
        }
        let last = q.locate(0, Some(&"g".into())).unwrap();
        q.cursor = Some(last);
        let late = vec![track("c"), track("x"), track("g"), track("y")];
        assert_eq!(q.append_if_current(epoch, late), Some(4));
        assert_eq!(ids(&q), "abfgxy");
        assert_eq!(q.current().unwrap().id, "g");
        assert_eq!(q.cursor, Some(3));
    }

    #[tokio::test]
    async fn the_end_of_the_previous_file_during_a_jump_does_not_move_the_cursor() {
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        let saved = Saved { tracks: "abcdefg".chars().map(|c| track(&c.to_string().repeat(11))).collect(), cursor: Some(0), origin: None };
        library.set_setting("queue", &serde_json::to_string(&saved).unwrap());
        let player = start(Arc::new(|_| {}), library).await;
        let last_id = "g".repeat(11);
        {
            let mut st = player.st.lock().unwrap();
            st.repeat = Repeat::All;
            st.queue.remove(2);
            st.queue.remove(2);
            st.queue.remove(2);
            let last = st.queue.locate(0, Some(&last_id.as_str().into())).unwrap();
            st.queue.cursor = Some(last);
            st.generation += 1;
            st.starting = true;
            st.play.position = 200.0;
            st.play.duration = 200.0;
        }
        let heard = player.st.lock().unwrap().handed;
        player.ended("eof", None, heard).await;
        let st = player.st.lock().unwrap();
        assert_eq!(st.queue.current().map(|t| t.id.as_str()), Some(format!("yt:{last_id}").as_str()));
    }

    #[tokio::test]
    async fn the_queue_saved_before_track_ids_had_a_prefix_restores() {
        let dir = std::env::temp_dir().join(format!("snellar-player-v1-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.db");
        rusqlite::Connection::open(&path).unwrap().execute_batch(include_str!("../tests/fixtures/library-v1.sql")).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let player = start(Arc::new(move |e| sink.lock().unwrap().push(e)), Arc::new(Library::open(&path).unwrap())).await;

        let queue = player.queue();
        let queued: Vec<&str> = queue["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!((queued, queue["cursor"].as_i64()), (vec!["yt:fHI8X4OXluQ", "yt:dQw4w9WgXcQ", "yt:5NV6Rdv1a3I"], Some(1)));
        assert_eq!(player.current().unwrap().id, "yt:dQw4w9WgXcQ");
        assert_eq!(player.snapshot()["position"], json!(42.5));
        assert!(events.lock().unwrap().iter().any(|e| matches!(e, UiEvent::Liked(true))), "the restored track is liked");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_late_radio_answer_leaves_a_track_the_user_picked_alone() {
        let mut q = queue("abc", 0);
        let epoch = q.epoch;
        assert_eq!(q.settle_radio(false, epoch, vec![track("x"), track("y")]), None);
        assert_eq!((ids(&q).as_str(), q.cursor), ("abcxy", Some(0)));
    }

    #[test]
    fn a_radio_answer_continues_with_the_next_queued_track_not_past_a_top_up() {
        let mut q = queue("abc", 2);
        let epoch = q.epoch;
        q.append_new(vec![track("p"), track("q")]);
        assert_eq!(q.settle_radio(true, epoch, vec![track("r"), track("p")]), Some(3));
        assert_eq!(ids(&q), "abcpqr");
    }

    #[test]
    fn a_radio_answer_of_only_duplicates_still_plays_the_next_track() {
        let mut q = queue("abcd", 2);
        let epoch = q.epoch;
        assert_eq!(q.settle_radio(true, epoch, vec![track("a"), track("b")]), Some(3));
        let mut end = queue("abc", 2);
        let epoch = end.epoch;
        assert_eq!(end.settle_radio(true, epoch, vec![track("x")]), Some(3));
        assert_eq!(end.settle_radio(true, epoch, vec![track("a")]), Some(3));
        let mut empty = queue("abc", 2);
        let epoch = empty.epoch;
        assert_eq!(empty.settle_radio(true, epoch, vec![track("a")]), None);
    }

    #[test]
    fn a_top_up_from_a_replaced_queue_is_dropped() {
        let mut q = queue("abc", 0);
        let stale = q.epoch;
        q.replace(vec![track("p"), track("q")], 1);
        assert_eq!(q.append_if_current(stale, vec![track("x")]), None);
        assert_eq!(ids(&q), "pq");
        let mut cleared = queue("abc", 0);
        let before = cleared.epoch;
        cleared.clear();
        assert_eq!(cleared.append_if_current(before, vec![track("x")]), None);
        assert!(cleared.tracks.is_empty());
    }

    #[test]
    fn dropped_streams_are_told_apart_from_real_ends() {
        assert!(cut_short(30.0, 120.0));
        assert!(!cut_short(116.0, 120.0));
        assert!(!cut_short(212.5, 213.0));
        assert!(!cut_short(5.0, 0.0));
    }

    #[test]
    fn move_out_of_range_changes_nothing() {
        let mut q = queue("abc", 1);
        assert!(!q.move_item(0, 5) && !q.move_item(7, 0));
        assert_eq!((ids(&q).as_str(), q.cursor), ("abc", Some(1)));
        assert!(q.remove(9).is_none());
    }

    #[tokio::test]
    async fn restores_queue_without_playing() {
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        let saved = Saved { tracks: vec![track("a"), track("b"), track("c")], cursor: Some(1), origin: None };
        library.set_setting("queue", &serde_json::to_string(&saved).unwrap());
        library.set_setting("position", "42.5");
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let player = start(Arc::new(move |e| sink.lock().unwrap().push(e)), library).await;
        assert_eq!(player.current().unwrap().id, "b");
        let events = events.lock().unwrap();
        assert!(events.iter().any(|e| matches!(e, UiEvent::Position(p) if *p == 42.5)));
        assert!(events.iter().all(|e| !matches!(e, UiEvent::State(s) if s.playing)));
    }

    async fn restored(position: &str) -> Arc<Player> {
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        let saved = Saved { tracks: vec![track("a"), track("b"), track("c")], cursor: Some(1), origin: None };
        library.set_setting("queue", &serde_json::to_string(&saved).unwrap());
        library.set_setting("position", position);
        start(Arc::new(|_| {}), library).await
    }

    #[tokio::test]
    async fn a_new_list_does_not_inherit_the_restored_position() {
        let player = restored("140.0").await;
        assert_eq!(player.st.lock().unwrap().resume, Some(140.0));
        player.replace_queue(vec![track("x"), track("y")], 1);
        assert_eq!(player.st.lock().unwrap().resume, None);
    }

    #[tokio::test]
    async fn clearing_while_a_track_is_starting_stops_the_spinner_and_the_clock() {
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        let saved = Saved { tracks: vec![track("a")], cursor: Some(0), origin: None };
        library.set_setting("queue", &serde_json::to_string(&saved).unwrap());
        library.set_setting("position", "42.5");
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let player = start(Arc::new(move |e| sink.lock().unwrap().push(e)), library).await;
        {
            let mut st = player.st.lock().unwrap();
            st.starting = true;
            player.publish_state(&mut st);
        }
        player.clear().await;
        let events = events.lock().unwrap();
        let state = events.iter().rev().find_map(|e| if let UiEvent::State(s) = e { Some(s.clone()) } else { None }).unwrap();
        let position = events.iter().rev().find_map(|e| if let UiEvent::Position(p) = e { Some(*p) } else { None }).unwrap();
        assert!(!state.loading && !player.st.lock().unwrap().play.loading, "the spinner was never switched off");
        assert_eq!((position, state.position, state.duration), (0.0, 0.0, 0.0));
    }

    #[tokio::test]
    async fn the_clock_of_the_previous_file_is_ignored_while_the_next_one_resolves() {
        let player = restored("0").await;
        {
            let mut st = player.st.lock().unwrap();
            st.generation = 2;
            st.handed = 1;
            st.starting = true;
            st.play.position = 0.0;
        }
        let mut last = Instant::now();
        player.property("time-pos", json!(55.0), &mut last);
        {
            let st = player.st.lock().unwrap();
            assert_eq!((st.play.position, st.listened, st.starting), (0.0, 0.0, true));
        }
        player.st.lock().unwrap().handed = 2;
        player.property("time-pos", json!(1.0), &mut last);
        let st = player.st.lock().unwrap();
        assert_eq!((st.play.position, st.starting), (1.0, false));
    }

    #[tokio::test]
    async fn only_the_latest_stall_is_acted_on() {
        let player = restored("0").await;
        let mut last = Instant::now();
        player.property("paused-for-cache", json!(true), &mut last);
        let first = player.st.lock().unwrap().stall;
        player.property("paused-for-cache", json!(false), &mut last);
        player.property("paused-for-cache", json!(true), &mut last);
        let second = player.st.lock().unwrap().stall;
        assert!(!player.still_stalled(first), "a stall that ended and came back has not lasted as long as its first watchdog");
        assert!(player.still_stalled(second));
        player.property("paused-for-cache", json!(false), &mut last);
        assert!(!player.still_stalled(second));
    }

    #[tokio::test]
    async fn running_out_of_attempts_during_a_stall_reports_and_stops_the_spinner() {
        let toasts = Arc::new(Mutex::new(Vec::new()));
        let sink = toasts.clone();
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        let saved = Saved { tracks: vec![track("a")], cursor: Some(0), origin: None };
        library.set_setting("queue", &serde_json::to_string(&saved).unwrap());
        let player = start(
            Arc::new(move |e| {
                if let UiEvent::Toast(t) = e {
                    sink.lock().unwrap().push(t)
                }
            }),
            library,
        )
        .await;
        let mut last = Instant::now();
        {
            let mut st = player.st.lock().unwrap();
            st.attempts = 2;
            st.starting = true;
        }
        player.property("paused-for-cache", json!(true), &mut last);
        let token = player.st.lock().unwrap().stall;
        player.stall_expired(token).await;
        let st = player.st.lock().unwrap();
        assert!(!st.play.loading && !st.starting && !st.cache, "the spinner stays on forever");
        assert!(
            toasts.lock().unwrap().iter().any(|t| t.code == Code::PlayFailed && t.cause == Some(Code::NetTimeout)),
            "{:?}",
            toasts.lock().unwrap()
        );
    }

    type Traces = Arc<Mutex<Vec<Option<PlayFailure>>>>;

    fn failures() -> (Ui, Traces, Arc<Mutex<Vec<Toast>>>) {
        let (failures, told) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
        let (f, t) = (failures.clone(), told.clone());
        let ui: Ui = Arc::new(move |e| match e {
            UiEvent::PlayFailure(failure) => f.lock().unwrap().push(failure),
            UiEvent::Toast(toast) => t.lock().unwrap().push(toast),
            _ => {}
        });
        (ui, failures, told)
    }

    #[tokio::test]
    async fn a_start_that_fails_names_its_cause_and_leaves_a_trace() {
        let (ui, failures, told) = failures();
        let (player, source, _) = rig(ui, memory(), Capabilities::default()).await;
        source.go_offline();
        assert!(player.play(tracks("abc"), 0, None).await.is_err());
        assert_eq!(told.lock().unwrap().iter().filter(|t| t.cause == Some(Code::NetOffline)).count(), 1, "{:?}", told.lock().unwrap());
        let last = failures.lock().unwrap().last().cloned().flatten().expect("a trace");
        assert_eq!(last.code, Code::NetOffline);
        assert!(!last.stopped, "a network failure does not walk the queue");
        assert_eq!(player.st.lock().unwrap().queue.cursor, Some(0));
        assert!(!source.calls().iter().any(|c| c.starts_with("fallback")), "{:?}", source.calls());
    }

    #[tokio::test]
    async fn the_cause_before_the_fallback_is_told_when_the_fallback_dies_too() {
        let (ui, failures, told) = failures();
        let (player, source, audio) = rig(ui, memory(), Capabilities::default()).await;
        let a: TrackId = "a".into();
        source.fail_stream(&a, Code::YtdlpFailed);
        player.play(tracks("a"), 0, None).await.unwrap();
        assert_eq!(audio.loads(), ["fallback://a"]);
        let heard = player.st.lock().unwrap().generation;
        player.ended("error", Some("loading failed".into()), heard).await;
        assert_eq!(told.lock().unwrap().last().unwrap().cause, Some(Code::YtdlpFailed), "not mpv");
        assert_eq!(failures.lock().unwrap().last().cloned().flatten().unwrap().code, Code::YtdlpFailed);
    }

    #[tokio::test]
    async fn only_a_real_mpv_failure_blames_mpv() {
        let (ui, _, told) = failures();
        let (player, _, _) = rig(ui, memory(), Capabilities::default()).await;
        player.play(tracks("a"), 0, None).await.unwrap();
        let heard = player.st.lock().unwrap().generation;
        player.st.lock().unwrap().attempts = 2;
        player.ended("error", Some("audio output failed".into()), heard).await;
        assert_eq!(told.lock().unwrap().last().unwrap().cause, Some(Code::AudioFailed));
    }

    async fn until_stopped(player: &Player) {
        for _ in 0..200 {
            if player.st.lock().unwrap().stopped {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the queue never stopped");
    }

    #[tokio::test]
    async fn three_bad_tracks_stop_the_queue_and_continue_goes_on() {
        let (ui, failures, _) = failures();
        let (player, source, audio) = rig(ui, memory(), Capabilities::default()).await;
        let list = tracks("abcde");
        for t in &list[..3] {
            source.break_stream(&t.id);
        }
        player.play(list, 0, None).await.ok();
        until_stopped(&player).await;
        let last = failures.lock().unwrap().last().cloned().flatten().unwrap();
        assert!(last.stopped);
        assert_eq!(last.tracks.len(), 3);
        assert_eq!(player.st.lock().unwrap().queue.cursor, Some(2));
        player.next().await.unwrap();
        assert_eq!(player.st.lock().unwrap().queue.cursor, Some(3));
        assert!(audio.loads().last().unwrap().contains('d'));
        assert_eq!(failures.lock().unwrap().last().cloned().flatten(), None, "playing again clears the banner");
    }

    #[tokio::test]
    async fn a_limit_waits_as_long_as_it_is_told_and_the_retries_end() {
        let (ui, _, _) = failures();
        let (player, _, _) = rig(ui, memory(), Capabilities::default()).await;
        player.replace_queue(tracks("ab"), 0);
        let limited = |after: u64| anyhow::anyhow!("429").context(snellar_core::error::RetryAfter(after)).context(Code::NetLimited);
        let mut st = player.st.lock().unwrap();
        assert_eq!(player.note_failure(&mut st, &limited(30)), AfterFailure::Wait(Duration::from_secs(30)));
        assert!(st.retrying);
        assert_eq!(player.note_failure(&mut st, &limited(1)), AfterFailure::Wait(AUTO_RETRY_BASE * 2));
        assert_eq!(player.note_failure(&mut st, &limited(1)), AfterFailure::Wait(AUTO_RETRY_BASE * 4));
        assert_eq!(player.note_failure(&mut st, &limited(1)), AfterFailure::Stay, "the retries end");
        assert!(!st.retrying);
    }

    #[tokio::test]
    async fn cancel_stops_the_spinner_and_a_late_answer_is_dropped() {
        let (ui, _, told) = failures();
        let (player, source, audio) = rig(ui, memory(), Capabilities::default()).await;
        let a: TrackId = "a".into();
        let gate = source.hold(&a);
        let task = {
            let player = player.clone();
            tokio::spawn(async move { player.play(tracks("a"), 0, None).await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while !player.st.lock().unwrap().play.loading {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        player.cancel_start().await;
        assert!(!player.st.lock().unwrap().play.loading);
        gate.add_permits(1);
        task.await.unwrap().ok();
        assert!(audio.loads().is_empty(), "the late stream does not start");
        assert!(told.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn only_the_current_and_the_next_track_count_as_near() {
        let player = restored("0").await;
        assert!(player.is_near(&"b".into()) && player.is_near(&"c".into()));
        assert!(!player.is_near(&"a".into()) && !player.is_near(&"zzz".into()));
    }

    #[tokio::test]
    async fn clearing_the_queue_forgets_the_restored_position() {
        let player = restored("140.0").await;
        player.clear().await;
        assert_eq!(player.st.lock().unwrap().resume, None);
    }

    fn memory() -> Arc<Library> {
        Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap())
    }

    fn tracks(ids: &str) -> Vec<Track> {
        ids.chars().map(|c| track(&c.to_string())).collect()
    }

    fn toasts() -> (Ui, Arc<Mutex<Vec<Toast>>>) {
        let toasts = Arc::new(Mutex::new(Vec::new()));
        let sink = toasts.clone();
        let ui: Ui = Arc::new(move |e| {
            if let UiEvent::Toast(t) = e {
                sink.lock().unwrap().push(t)
            }
        });
        (ui, toasts)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_playing_stream_is_described_by_the_source_or_by_mpv() {
        let streams = Arc::new(Mutex::new(Vec::new()));
        let sink = streams.clone();
        let ui: Ui = Arc::new(move |e| {
            if let UiEvent::Stream(s) = e {
                sink.lock().unwrap().push(s)
            }
        });
        let (player, source, audio) = rig(ui, memory(), Capabilities::default()).await;
        let opus = AudioFormat { codec: "opus".into(), bitrate: 136, sample_rate: 48000, loudness_db: None };
        source.answer_format(&"a".into(), opus);
        player.play(tracks("ab"), 0, None).await.unwrap();
        assert_eq!(
            player.snapshot()["stream"],
            json!({"codec": "opus", "bitrate": 136, "sampleRate": 48000, "gain": null, "leveling": "measured", "local": false})
        );
        audio.send(Event::Property("audio-codec-name".into(), json!("vorbis")));
        source.fail_stream(&"b".into(), Code::YtdlpFailed);
        player.next().await.unwrap();
        assert_eq!(player.snapshot()["stream"]["codec"], "", "a's codec is not carried over");
        audio.send(Event::Property("audio-codec-name".into(), json!("vorbis")));
        until("the fallback's codec from mpv", || streams.lock().unwrap().last().is_some_and(|s| s.codec == "vorbis")).await;
        assert_eq!(audio.loads().last().unwrap(), "fallback://b");
        assert_eq!(player.snapshot()["stream"]["bitrate"], 0);
        player.clear().await;
        assert_eq!(streams.lock().unwrap().last(), Some(&StreamInfo::default()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loudness_lowers_a_loud_track_and_measures_one_without_a_value() {
        let (player, source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        let loud = AudioFormat { codec: "opus".into(), bitrate: 136, sample_rate: 48000, loudness_db: Some(0.97999954) };
        let quiet = AudioFormat { loudness_db: Some(-6.4), ..loud.clone() };
        source.answer_format(&"a".into(), loud);
        source.answer_format(&"b".into(), quiet);
        player.play(tracks("abc"), 0, None).await.unwrap();
        assert_eq!(audio.log().last().unwrap(), "load fake://a 0 gain=-0.97999954");
        assert_eq!(player.snapshot()["stream"]["leveling"], "gain");
        player.next().await.unwrap();
        assert_eq!(audio.log().last().unwrap(), "load fake://b 0 gain=0", "a quiet track is never raised");
        let measured = format!("set af \"{}\"", fx::chain(&Fx::default(), true));
        assert!(!audio.log().contains(&measured));
        player.next().await.unwrap();
        let log = audio.log();
        let at = |line: &str| log.iter().rposition(|l| l == line).unwrap();
        assert!(at(&measured) < at("load fake://c 0 gain=0"), "measured before it starts");
        assert_eq!(player.snapshot()["stream"]["leveling"], "measured");

        player.set_loudness(false).await.unwrap();
        let log = audio.log();
        assert_eq!(log[log.len() - 4..log.len() - 2], ["set replaygain-fallback 0.0".to_string(), "set af \"\"".into()]);
        assert_eq!(player.snapshot()["stream"]["leveling"], "off");
        let before = audio.log().len();
        player.prev().await.unwrap();
        assert!(!audio.log()[before..].iter().any(|l| l.starts_with("set af")), "nothing to measure when off");
        assert_eq!(audio.log().last().unwrap(), "load fake://b 0 gain=0", "off passes no gain");
        player.jump(0, None).await.unwrap();
        assert_eq!(audio.log().last().unwrap(), "load fake://a 0 gain=0", "even for a loud track");
        player.set_loudness(true).await.unwrap();
        assert_eq!(audio.log().last().unwrap(), "set replaygain-fallback -0.97999954", "on again lowers the playing track at once");
        assert_eq!(player.snapshot()["stream"]["leveling"], "gain");
    }

    fn devices(names: &[&str]) -> Value {
        let mut list = vec![json!({"name": "auto", "description": "Autoselect device"})];
        list.extend(names.iter().map(|n| json!({"name": n, "description": format!("{n} speakers")})));
        json!(list)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_saved_output_device_is_used_when_present_and_auto_while_it_is_missing() {
        let library = memory();
        settings::AUDIO_DEVICE.set(&library, &"pulse/usb");
        let audio = FakeAudio::new();
        audio.answer("audio-device-list", devices(&["pulse/builtin"]));
        audio.answer("audio-device", json!("auto"));
        let player = Player::start(Arc::new(|_| {}), FakeSource::new(Capabilities::default()), audio.launch(), library).await.unwrap();
        assert!(!audio.log().iter().any(|l| l.starts_with("set audio-device")), "auto already, nothing to switch");
        let listed = player.devices().await.unwrap();
        assert_eq!(
            listed["devices"],
            json!([{"name": "pulse/builtin", "description": "pulse/builtin speakers"}]),
            "mpv's own auto is not a device"
        );
        assert_eq!((listed["wanted"].as_str(), listed["active"].as_str()), (Some("pulse/usb"), Some("auto")));

        audio.answer("audio-device-list", devices(&["pulse/builtin", "pulse/usb"]));
        audio.send(Event::Property("audio-device-list".into(), devices(&["pulse/builtin", "pulse/usb"])));
        until("plugged in, it is used", || audio.log().contains(&"set audio-device \"pulse/usb\"".to_string())).await;
        audio.answer("audio-device-list", devices(&["pulse/builtin"]));
        audio.send(Event::Property("audio-device-list".into(), devices(&["pulse/builtin"])));
        until("unplugged, back to auto", || audio.log().last() == Some(&"set audio-device \"auto\"".to_string())).await;

        player.set_device("pulse/builtin").await.unwrap();
        assert_eq!(audio.log().last().unwrap(), "set audio-device \"pulse/builtin\"");
        player.set_device("auto").await.unwrap();
        assert_eq!(audio.log().last().unwrap(), "set audio-device \"auto\"");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn downloaded_tracks_play_from_their_files_without_asking_the_source() {
        let dir = std::env::temp_dir().join(format!("snellar-offline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let library = memory();
        for id in ["a", "b", "c"] {
            let file = dir.join(format!("{id}.opus"));
            std::fs::write(&file, b"opus").unwrap();
            library.set_download(&id.into(), &file, (id != "a").then_some(3.0));
        }
        let (toasts_ui, toasts) = toasts();
        let (player, source, audio) = rig(toasts_ui, library.clone(), Capabilities { radio: true }).await;
        source.go_offline();
        player.play(tracks("abc"), 0, None).await.unwrap();
        let file = |id: &str| dir.join(format!("{id}.opus")).to_string_lossy().into_owned();
        assert_eq!(audio.loads(), [file("a")]);
        assert_eq!(player.snapshot()["stream"]["local"], true);
        assert_eq!(player.snapshot()["stream"]["leveling"], "measured", "no value kept for a: loudnorm");

        audio.send(Event::EndFile { reason: "eof".into(), error: None });
        until("b from its file", || audio.loads().last() == Some(&file("b"))).await;
        assert_eq!(audio.log().last().unwrap(), &format!("load {} 0 gain=-3", file("b")), "the kept loudness value still lowers it");
        near_the_end(&audio).await;
        until("c appended from its file", || audio.log().iter().any(|l| l.starts_with(&format!("append {}", file("c"))))).await;
        audio.send(Event::EndFile { reason: "eof".into(), error: None });
        until("c taken over", || player.queue()["cursor"] == 2).await;
        assert_eq!(player.snapshot()["stream"]["local"], true);
        let asked: Vec<String> = source.calls().into_iter().filter(|c| !c.starts_with("radio")).collect();
        assert!(asked.is_empty(), "nothing but the radio top-up asked the network: {asked:?}");
        assert!(toasts.lock().unwrap().iter().all(|t| t.code != Code::PlayFailed), "{:?}", toasts.lock().unwrap());

        std::fs::remove_file(dir.join("a.opus")).unwrap();
        assert!(player.jump(0, None).await.is_err(), "offline, a deleted file cannot play");
        assert!(source.calls().contains(&"stream a".to_string()), "a deleted file falls back to the source");
        assert_eq!(library.download(&"a".into()).map(|d| d.0), Some(dir.join("a.opus")), "the row stays: an unmounted disk comes back");
        until("offline: the failure is told", || toasts.lock().unwrap().iter().any(|t| t.code == Code::PlayFailed)).await;
        std::fs::write(dir.join("a.opus"), b"opus").unwrap();
        player.jump(0, None).await.unwrap();
        assert_eq!(audio.loads().last().unwrap(), &file("a"), "the file is back: it plays from it again");
        std::fs::remove_dir_all(&dir).ok();
    }

    async fn near_the_end(audio: &FakeAudio) {
        audio.send(Event::Property("duration".into(), json!(30.0)));
        audio.send(Event::Property("time-pos".into(), json!(25.0)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_next_track_is_queued_in_mpv_and_taken_over_without_a_load() {
        let currents = Arc::new(Mutex::new(Vec::new()));
        let sink = currents.clone();
        let ui: Ui = Arc::new(move |e| {
            if let UiEvent::Current(Some(t)) = e {
                sink.lock().unwrap().push((t.id, Instant::now()))
            }
        });
        let (player, source, audio) = rig(ui, memory(), Capabilities::default()).await;
        source.answer_format(&"a".into(), AudioFormat { loudness_db: Some(1.0), ..AudioFormat::default() });
        source.answer_format(&"b".into(), AudioFormat { codec: "opus".into(), bitrate: 136, sample_rate: 48000, loudness_db: Some(2.0) });
        source.answer_format(&"c".into(), AudioFormat { loudness_db: Some(-1.0), ..AudioFormat::default() });
        player.play(tracks("abc"), 0, None).await.unwrap();
        audio.send(Event::Property("time-pos".into(), json!(1.0)));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!audio.log().iter().any(|l| l.starts_with("append")), "not queued while far from the end");
        near_the_end(&audio).await;
        until("b appended with its gain", || audio.log().contains(&"append fake://b gain=-2".to_string())).await;
        let eof = Instant::now();
        audio.send(Event::EndFile { reason: "eof".into(), error: None });
        until("b is current", || currents.lock().unwrap().last().is_some_and(|(id, _)| id.as_str() == "b")).await;
        let took = currents.lock().unwrap().last().unwrap().1 - eof;
        println!("fake: end of a to b current {} us, no load", took.as_micros());
        assert_eq!(audio.loads(), [fakes::url(&"a".into())], "mpv already plays b; loading it again would cut it");
        assert_eq!(player.queue()["cursor"], 1);
        assert_eq!(player.snapshot()["stream"]["gain"], -2.0);
        audio.send(Event::Property("time-pos".into(), json!(0.5)));
        until("b's clock is taken", || player.snapshot()["position"] == 0.5).await;
        near_the_end(&audio).await;
        until("c appended next", || audio.log().iter().any(|l| l.starts_with("append fake://c"))).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_queue_change_drops_the_queued_track_and_the_end_loads_the_new_next() {
        let (player, _source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        player.play(tracks("abc"), 0, None).await.unwrap();
        near_the_end(&audio).await;
        until("b appended", || audio.log().iter().any(|l| l.starts_with("append fake://b"))).await;
        player.enqueue(tracks("x"), true).await.unwrap();
        until("the stale b is cleared from mpv", || audio.log().contains(&"clear next".to_string())).await;
        audio.send(Event::EndFile { reason: "eof".into(), error: None });
        until("x loaded", || audio.loads().last() == Some(&fakes::url(&"x".into()))).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_track_that_needs_measuring_after_one_with_a_value_is_loaded_at_the_end() {
        let (player, source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        source.answer_format(&"a".into(), AudioFormat { loudness_db: Some(1.0), ..AudioFormat::default() });
        player.play(tracks("ab"), 0, None).await.unwrap();
        near_the_end(&audio).await;
        until("b streamed for the preload", || source.calls().iter().filter(|c| c.as_str() == "stream b").count() == 1).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!audio.log().iter().any(|l| l.starts_with("append")), "loudnorm would change under a's tail");
        let eof = Instant::now();
        audio.send(Event::EndFile { reason: "eof".into(), error: None });
        until("b loaded", || audio.loads().last() == Some(&fakes::url(&"b".into()))).await;
        println!("fake: end of a to the load of b {} us", eof.elapsed().as_micros());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_retry_replaces_the_list_in_mpv_so_the_queued_track_is_forgotten() {
        let (player, _source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        player.play(tracks("ab"), 0, None).await.unwrap();
        near_the_end(&audio).await;
        until("b appended", || audio.log().iter().any(|l| l.starts_with("append fake://b"))).await;
        audio.send(Event::EndFile { reason: "error".into(), error: Some("reset".into()) });
        until("a reloaded", || audio.loads().len() == 2).await;
        audio.send(Event::EndFile { reason: "eof".into(), error: None });
        until("b loaded, not taken over from a list mpv no longer has", || audio.loads().last() == Some(&fakes::url(&"b".into()))).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_stream_for_an_earlier_pick_never_plays_over_a_later_one() {
        let (player, source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities { radio: true }).await;
        let (a, b) = (TrackId::from("a"), TrackId::from("b"));
        let gate = source.hold(&a);
        let first = tokio::spawn({
            let player = player.clone();
            async move { player.play(tracks("abc"), 0, None).await }
        });
        until("the stream of a to be asked for", || source.calls().contains(&"stream a".to_string())).await;
        player.jump(1, None).await.unwrap();
        gate.add_permits(1);
        first.await.unwrap().unwrap();
        assert_eq!(audio.loads(), [fakes::url(&b)], "a must not start after the user picked b");
        assert_eq!(player.current().unwrap().id, b);
        player.property("time-pos", json!(7.0), &mut Instant::now());
        assert_eq!(player.st.lock().unwrap().play.position, 7.0, "the late stream of a froze the clock of b");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_unpause_for_an_earlier_pick_never_loads_it_over_a_later_one() {
        let (player, _, audio) = rig(Arc::new(|_| {}), memory(), Capabilities { radio: true }).await;
        let b = TrackId::from("b");
        let gate = audio.hold("pause");
        let first = tokio::spawn({
            let player = player.clone();
            async move { player.play(tracks("abc"), 0, None).await }
        });
        until("the unpause before a", || audio.log().iter().any(|l| l == "set pause false")).await;
        player.jump(1, None).await.unwrap();
        gate.add_permits(1);
        first.await.unwrap().unwrap();
        assert_eq!(audio.loads(), [fakes::url(&b)], "a must not start after the user picked b");
        assert_eq!(player.current().unwrap().id, b);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_retry_that_fails_after_the_user_jumped_leaves_their_pick_playing() {
        let (ui, toasts) = toasts();
        let (player, source, audio) = rig(ui, memory(), Capabilities { radio: true }).await;
        let (a, c) = (TrackId::from("a"), TrackId::from("c"));
        player.play(tracks("abcd"), 0, None).await.unwrap();
        let gate = source.hold(&a);
        source.break_stream(&a);
        audio.send(Event::EndFile { reason: "error".into(), error: Some("http 403".into()) });
        until("the retry of a", || source.calls().contains(&"stream a fresh".to_string())).await;
        player.jump(2, None).await.unwrap();
        gate.add_permits(1);
        until("the end-of-file task to finish", || player.tasks.len() == 1).await;
        assert_eq!(player.current().unwrap().id, c, "the failed retry of a skipped past c");
        assert_eq!(audio.loads().last(), Some(&fakes::url(&c)));
        assert!(toasts.lock().unwrap().iter().all(|t| t.code != Code::PlayFailed), "{:?}", toasts.lock().unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_retry_still_setting_up_the_meter_never_loads_over_a_later_pick() {
        let (player, source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities { radio: true }).await;
        let c = TrackId::from("c");
        player.play(tracks("abcd"), 0, None).await.unwrap();
        source.answer_format(&"a".into(), AudioFormat { loudness_db: Some(-3.0), ..AudioFormat::default() });
        let gate = audio.hold("af");
        let before = audio.log().len();
        audio.send(Event::EndFile { reason: "error".into(), error: Some("http 403".into()) });
        until("the retry of a to switch the meter", || audio.log()[before..].iter().any(|l| l.starts_with("set af"))).await;
        player.jump(2, None).await.unwrap();
        gate.add_permits(1);
        until("the end-of-file task to finish", || player.tasks.len() == 1).await;
        assert_eq!(player.current().unwrap().id, c);
        assert_eq!(audio.loads().last(), Some(&fakes::url(&c)), "the retry of a started over the user's pick");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn autoplay_appends_the_radio_and_plays_on_at_the_end_of_the_queue() {
        let (player, source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities { radio: true }).await;
        source.answer_radio(tracks("xy"));
        player.play(tracks("a"), 0, None).await.unwrap();
        until("the top-up", || player.tasks.len() == 1).await;
        player.next().await.unwrap();
        assert_eq!(player.current().unwrap().id, "x");
        assert_eq!(audio.loads(), [fakes::url(&"a".into()), fakes::url(&"x".into())]);
        assert!(source.calls().iter().any(|c| c == "radio a"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_source_without_radio_stops_at_the_end_and_plays_a_radio_seed_alone() {
        let (ui, toasts) = toasts();
        let (player, source, audio) = rig(ui, memory(), Capabilities { radio: false }).await;
        player.play(tracks("a"), 0, None).await.unwrap();
        player.next().await.unwrap();
        assert_eq!(player.current().unwrap().id, "a");
        player.radio(track("s")).await.unwrap();
        let queue = player.queue();
        assert_eq!((queue["tracks"].as_array().unwrap().len(), player.current().unwrap().id.as_str()), (1, "s"));
        assert_eq!(audio.loads(), [fakes::url(&"a".into()), fakes::url(&"s".into())]);
        assert!(source.calls().iter().all(|c| !c.starts_with("radio")), "{:?}", source.calls());
        assert!(toasts.lock().unwrap().is_empty(), "{:?}", toasts.lock().unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_clock_and_the_end_of_the_old_track_do_not_count_while_the_new_one_is_handed_to_mpv() {
        let (player, _source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        player.play(tracks("abc"), 0, None).await.unwrap();
        audio.send(Event::Property("time-pos".into(), json!(1.0)));
        until("a's clock", || player.snapshot()["position"] == 1.0).await;
        let gate = audio.hold("pause");
        let pick = tokio::spawn({
            let player = player.clone();
            async move { player.jump(1, None).await }
        });
        until("b waits to reach mpv", || player.queue()["cursor"] == 1 && audio.log().last().is_some_and(|l| l == "set pause false")).await;
        audio.send(Event::Property("time-pos".into(), json!(55.0)));
        audio.send(Event::EndFile { reason: "eof".into(), error: None });
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(player.snapshot()["position"], 0.0, "the old clock is not b's");
        assert_eq!(player.queue()["cursor"], 1, "the old end is not b's");
        assert_eq!(audio.loads(), [fakes::url(&"a".into())]);
        gate.add_permits(1);
        pick.await.unwrap().unwrap();
        assert_eq!(audio.loads().last(), Some(&fakes::url(&"b".into())));
        audio.send(Event::Property("time-pos".into(), json!(2.0)));
        until("b's own clock counts", || player.snapshot()["position"] == 2.0).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_pick_drops_the_track_queued_in_mpv_at_once_even_while_its_stream_resolves() {
        let (player, source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        player.play(tracks("abc"), 0, None).await.unwrap();
        near_the_end(&audio).await;
        until("b appended", || audio.log().iter().any(|l| l.starts_with("append fake://b"))).await;
        let before = audio.log().len();
        let gate = source.hold(&"x".into());
        let pick = tokio::spawn({
            let player = player.clone();
            async move { player.play(tracks("xb"), 0, None).await }
        });
        until("the stale b is cleared, or mpv plays it when a ends", || audio.log()[before..].contains(&"clear next".to_string())).await;
        gate.add_permits(1);
        pick.await.unwrap().unwrap();
    }

    fn idle_now(audio: &FakeAudio, idle: bool) {
        audio.send(Event::Property("idle-active".into(), json!(idle)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn seeking_a_restored_track_moves_where_play_starts_and_prev_still_works() {
        let library = memory();
        let saved = Saved { tracks: tracks("abc"), cursor: Some(1), origin: None };
        library.set_setting("queue", &serde_json::to_string(&saved).unwrap());
        library.set_setting("position", "120.0");
        let audio = FakeAudio::new();
        let source = FakeSource::new(Capabilities::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let ui: Ui = Arc::new(move |e| {
            if let UiEvent::Position(p) = e {
                sink.lock().unwrap().push(p)
            }
        });
        let player = Player::start(ui, source, audio.launch(), library).await.unwrap();
        player.seek(30.0).await.unwrap();
        assert_eq!(player.snapshot()["position"], 30.0);
        assert_eq!(events.lock().unwrap().last(), Some(&30.0), "the slider follows");
        assert!(!audio.log().iter().any(|l| l.starts_with("seek")), "mpv has no file to seek in");
        player.prev().await.unwrap();
        assert_eq!(player.snapshot()["position"], 0.0, "more than 3 s in: back to the start");
        assert_eq!(player.current().unwrap().id, "b");
        player.seek(45.0).await.unwrap();
        player.toggle().await.unwrap();
        assert!(audio.log().last().unwrap().starts_with("load fake://b 45 "), "{:?}", audio.log());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn prev_on_a_restored_track_near_its_start_goes_to_the_previous_track_not_the_old_position() {
        let player = restored("120.0").await;
        player.seek(0.0).await.unwrap();
        player.prev().await.unwrap();
        assert_eq!(player.current().unwrap().id, "a");
        assert_eq!(player.snapshot()["position"], 0.0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn seeking_with_nothing_loaded_and_no_track_is_harmless() {
        let (player, _source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        player.seek(10.0).await.unwrap();
        assert_eq!(player.st.lock().unwrap().resume, None);
        assert!(audio.loads().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_after_an_mpv_crash_continues_where_it_was() {
        let (player, _source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        player.play(tracks("ab"), 0, None).await.unwrap();
        idle_now(&audio, false);
        audio.send(Event::Property("time-pos".into(), json!(50.0)));
        until("the clock", || player.snapshot()["position"] == 50.0).await;
        audio.send(Event::Died);
        until("mpv gone", || player.st.lock().unwrap().idle).await;
        player.toggle().await.unwrap();
        assert!(audio.log().last().unwrap().starts_with("load fake://a 50 "), "{:?}", audio.log());
        assert_eq!(player.queue()["cursor"], 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stalls_far_apart_in_a_long_mix_do_not_abandon_it() {
        let (ui, toasts) = toasts();
        let (player, source, audio) = rig(ui, memory(), Capabilities::default()).await;
        player.play(tracks("ab"), 0, None).await.unwrap();
        for (round, at) in [100.0, 2000.0, 4000.0, 6000.0].into_iter().enumerate() {
            audio.send(Event::Property("time-pos".into(), json!(at)));
            until("the clock", || player.snapshot()["position"] == at).await;
            audio.send(Event::EndFile { reason: "error".into(), error: Some("reset".into()) });
            until("the retry", || audio.loads().len() == round + 2).await;
            assert_eq!(audio.loads().last().unwrap(), &fakes::url(&"a".into()), "a fresh stream each time, not the fallback");
            audio.send(Event::Property("time-pos".into(), json!(at + 1.0)));
            audio.send(Event::Property("time-pos".into(), json!(at + PLAYED_ENOUGH + 1.0)));
            until("played past the retry", || player.snapshot()["position"] == at + PLAYED_ENOUGH + 1.0).await;
        }
        assert_eq!(player.queue()["cursor"], 0);
        assert_eq!(source.calls().iter().filter(|c| c.as_str() == "stream a fresh").count(), 4);
        assert!(toasts.lock().unwrap().is_empty(), "{:?}", toasts.lock().unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_stream_that_dies_again_at_the_same_point_is_still_given_up_on() {
        let (ui, toasts) = toasts();
        let (player, _source, audio) = rig(ui, memory(), Capabilities::default()).await;
        player.play(tracks("ab"), 0, None).await.unwrap();
        audio.send(Event::Property("time-pos".into(), json!(100.0)));
        until("the clock", || player.snapshot()["position"] == 100.0).await;
        for round in 0..2 {
            audio.send(Event::EndFile { reason: "error".into(), error: Some("reset".into()) });
            until("the retry", || audio.loads().len() == round + 2).await;
            audio.send(Event::Property("time-pos".into(), json!(101.0)));
            until("the clock", || player.snapshot()["position"] == 101.0).await;
        }
        assert_eq!(audio.loads().last().unwrap(), "fallback://a");
        audio.send(Event::EndFile { reason: "error".into(), error: Some("reset".into()) });
        until("given up, on to b", || player.queue()["cursor"] == 1).await;
        assert!(toasts.lock().unwrap().iter().any(|t| t.code == Code::PlayFailed));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_same_end_heard_twice_never_skips_the_track_taken_over() {
        let (player, _source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        player.play(tracks("abc"), 0, None).await.unwrap();
        near_the_end(&audio).await;
        until("b appended", || audio.log().iter().any(|l| l.starts_with("append fake://b"))).await;
        let heard = player.st.lock().unwrap().handed;
        player.ended("eof", None, heard).await;
        assert_eq!(player.queue()["cursor"], 1);
        player.ended("eof", None, heard).await;
        assert_eq!(player.queue()["cursor"], 1, "the second end moved past b");
        assert_eq!(audio.loads(), [fakes::url(&"a".into())]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ends_of_file_run_one_at_a_time_in_order() {
        let (player, source, audio) = rig(Arc::new(|_| {}), memory(), Capabilities::default()).await;
        player.play(tracks("abc"), 0, None).await.unwrap();
        let gate = source.hold(&"a".into());
        audio.send(Event::EndFile { reason: "error".into(), error: Some("reset".into()) });
        until("the retry of a", || source.calls().contains(&"stream a fresh".to_string())).await;
        audio.send(Event::EndFile { reason: "eof".into(), error: None });
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(audio.loads().len(), 1, "the later end ran beside the retry: {:?}", audio.loads());
        assert_eq!(player.queue()["cursor"], 0);
        gate.add_permits(1);
        until("the retry loads a", || audio.loads().len() >= 2).await;
        assert_eq!(audio.loads()[1], fakes::url(&"a".into()), "the first event was served first");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_end_waiting_for_its_turn_is_dropped_when_the_user_moved_on() {
        let (ui, toasts) = toasts();
        let (player, source, audio) = rig(ui, memory(), Capabilities::default()).await;
        player.play(tracks("abcd"), 0, None).await.unwrap();
        let gate = source.hold(&"a".into());
        audio.send(Event::EndFile { reason: "error".into(), error: Some("reset".into()) });
        until("the retry of a", || source.calls().contains(&"stream a fresh".to_string())).await;
        audio.send(Event::EndFile { reason: "eof".into(), error: None });
        player.jump(2, None).await.unwrap();
        gate.add_permits(1);
        until("both ends done", || player.tasks.len() == 1).await;
        assert_eq!(player.queue()["cursor"], 2, "the waiting end of a moved past the pick");
        assert_eq!(audio.loads().last(), Some(&fakes::url(&"c".into())));
        assert!(toasts.lock().unwrap().is_empty(), "{:?}", toasts.lock().unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn quitting_saves_the_position_the_queue_and_the_pending_listen() {
        let library = memory();
        let (player, _source, audio) = rig(Arc::new(|_| {}), library.clone(), Capabilities::default()).await;
        player.play(tracks("abc"), 1, None).await.unwrap();
        audio.send(Event::Property("time-pos".into(), json!(123.0)));
        until("the clock", || player.snapshot()["position"] == 123.0).await;
        assert_eq!(settings::POSITION.or_default::<f64>(&library), 0.0, "the last periodic save is older");
        assert!(library.listens(5).is_empty());
        player.save_now();
        assert_eq!(settings::POSITION.or_default::<f64>(&library), 123.0);
        let queue = settings::QUEUE.get::<Saved>(&library).unwrap();
        assert_eq!((queue.tracks.len(), queue.cursor), (3, Some(1)));
        let listens = library.listens(5);
        assert_eq!(listens.len(), 1);
        assert_eq!((listens[0].0.id.as_str(), listens[0].1), ("b", 123.0));
        player.save_now();
        assert_eq!(library.listens(5).len(), 1, "a second save does not count it twice");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn bench_track_change_on_a_real_disk() {
        let dir = std::env::current_dir().unwrap().join("target").join(format!("track-change-bench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let library = Arc::new(Library::open(&dir.join("library.db")).unwrap());
        let (player, _source, _audio) = rig(Arc::new(|_| {}), library, Capabilities::default()).await;
        let queue: Vec<Track> = (0..200).map(|i| track(&format!("t{i}"))).collect();
        player.play(queue, 0, None).await.unwrap();
        let mut took = Vec::new();
        for _ in 0..100 {
            let at = Instant::now();
            player.next().await.unwrap();
            took.push(at.elapsed().as_secs_f64() * 1000.0);
        }
        took.sort_by(f64::total_cmp);
        println!("track change, 200-track queue, 100 runs: median {:.2} ms, p95 {:.2} ms, max {:.2} ms", took[50], took[95], took[99]);
        drop(player);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn untimed(id: &str) -> Track {
        Track { duration: 0.0, ..track(id) }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_track_without_a_length_learns_it_from_mpv_everywhere() {
        let library = memory();
        let currents = Arc::new(Mutex::new(Vec::new()));
        let sink = currents.clone();
        let ui: Ui = Arc::new(move |e| {
            if let UiEvent::Current(Some(t)) = e {
                sink.lock().unwrap().push(t)
            }
        });
        let (player, _source, audio) = rig(ui, library.clone(), Capabilities::default()).await;
        player.play(vec![untimed("a"), untimed("b")], 0, None).await.unwrap();
        audio.send(Event::Property("duration".into(), json!(182.5)));
        until("the queue row shows the time", || player.queue()["tracks"][0]["duration"] == 182.5).await;
        assert_eq!(player.queue()["tracks"][1]["duration"], 0.0, "only the track that plays");
        assert_eq!(player.snapshot()["duration"], 182.5);
        assert_eq!(library.track(&"a".into()).unwrap().duration, 182.5, "the library keeps it");
        assert_eq!(currents.lock().unwrap().last().unwrap().duration, 182.5, "the window, MPRIS and Discord are told");
        let saved = settings::QUEUE.get::<Saved>(&library).unwrap();
        assert_eq!(saved.tracks[0].duration, 182.5, "and the saved queue");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_length_the_library_knows_is_there_before_mpv_says_anything() {
        let library = memory();
        library.remember_track(&Track { duration: 182.5, ..track("a") });
        library.remember_track(&Track { duration: 99.0, ..track("b") });
        let currents = Arc::new(Mutex::new(Vec::new()));
        let sink = currents.clone();
        let ui: Ui = Arc::new(move |e| {
            if let UiEvent::Current(Some(t)) = e {
                sink.lock().unwrap().push(t)
            }
        });
        let (player, _source, _audio) = rig(ui, library.clone(), Capabilities::default()).await;
        player.play(vec![untimed("a"), untimed("b"), untimed("c")], 0, None).await.unwrap();
        assert_eq!(currents.lock().unwrap()[0].duration, 182.5, "the first word to the window, MPRIS and Discord has it");
        assert_eq!(player.snapshot()["duration"], 182.5);
        assert_eq!(player.queue()["tracks"][1]["duration"], 99.0, "a row that has not played yet");
        assert_eq!(player.queue()["tracks"][2]["duration"], 0.0, "nobody knows this one yet");
        assert_eq!(library.track(&"a".into()).unwrap().duration, 182.5, "starting it from a shelf row did not erase it");
        player.enqueue(vec![untimed("b")], false).await.unwrap();
        assert_eq!(player.queue()["tracks"][3]["duration"], 99.0, "enqueued rows too");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn history_is_told_after_the_new_song_is_in_it() {
        let library = memory();
        let told = Arc::new(Mutex::new(Vec::new()));
        let (sink, reader) = (told.clone(), library.clone());
        let ui: Ui = Arc::new(move |e| {
            if matches!(e, UiEvent::History) {
                sink.lock().unwrap().push(reader.history(1).first().map(|t| (t.id.clone(), t.duration)));
            }
        });
        let (player, _source, audio) = rig(ui, library.clone(), Capabilities::default()).await;
        player.play(vec![untimed("a"), untimed("b")], 0, None).await.unwrap();
        assert_eq!(*told.lock().unwrap(), [Some(("a".into(), 0.0))], "the song is first when the window is told");
        audio.send(Event::Property("duration".into(), json!(182.5)));
        until("the row gets its time", || told.lock().unwrap().len() == 2).await;
        assert_eq!(told.lock().unwrap()[1], Some(("a".into(), 182.5)), "told again when the time is known");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_length_mpv_reports_never_replaces_the_one_metadata_has() {
        let library = memory();
        let (player, _source, audio) = rig(Arc::new(|_| {}), library.clone(), Capabilities::default()).await;
        player.play(tracks("a"), 0, None).await.unwrap();
        audio.send(Event::Property("duration".into(), json!(215.0)));
        until("the clock takes mpv's length", || player.snapshot()["duration"] == 215.0).await;
        assert_eq!(player.queue()["tracks"][0]["duration"], 213.0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_length_of_the_old_file_is_not_written_to_the_new_track() {
        let library = memory();
        let (player, _source, audio) = rig(Arc::new(|_| {}), library.clone(), Capabilities::default()).await;
        player.play(vec![track("a"), untimed("b")], 0, None).await.unwrap();
        audio.send(Event::Property("duration".into(), json!(213.0)));
        until("a's length", || player.snapshot()["duration"] == 213.0).await;
        let gate = audio.hold("pause");
        let pick = tokio::spawn({
            let player = player.clone();
            async move { player.jump(1, None).await }
        });
        until("b waits to reach mpv", || player.queue()["cursor"] == 1).await;
        audio.send(Event::Property("duration".into(), json!(213.0)));
        gate.add_permits(1);
        pick.await.unwrap().unwrap();
        audio.send(Event::EndFile { reason: "stop".into(), error: None });
        audio.send(Event::Property("time-pos".into(), json!(0.5)));
        until("b's clock", || player.snapshot()["position"] == 0.5).await;
        assert_eq!(player.queue()["tracks"][1]["duration"], 0.0);
        audio.send(Event::Property("duration".into(), json!(99.0)));
        until("b's own length", || player.queue()["tracks"][1]["duration"] == 99.0).await;
    }

    #[tokio::test]
    #[ignore]
    async fn live_playback_via_ytdl() {
        let position = Arc::new(Mutex::new(0.0));
        let toasts = Arc::new(Mutex::new(Vec::new()));
        let (p, t) = (position.clone(), toasts.clone());
        let ui: Ui = Arc::new(move |e| match e {
            UiEvent::Position(x) => *p.lock().unwrap() = x,
            UiEvent::Toast(s) => t.lock().unwrap().push(s),
            _ => {}
        });
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        let player = youtube(ui, library.clone()).await;
        player.set_volume(20.0).await.unwrap();
        player
            .play(vec![Track { id: "dQw4w9WgXcQ".into(), title: "Never Gonna".into(), duration: 213.0, ..track("x") }], 0, None)
            .await
            .unwrap();

        let wait = |min: f64| {
            let position = position.clone();
            async move {
                for _ in 0..120 {
                    if *position.lock().unwrap() >= min {
                        return true;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                false
            }
        };
        assert!(wait(3.0).await, "position did not advance: {:?}", toasts.lock().unwrap());
        assert_eq!(player.current().unwrap().id, "yt:dQw4w9WgXcQ");

        let meta = tokio::process::Command::new("playerctl").args(["-p", snellar_core::product::ID, "metadata"]).output().await.unwrap();
        let meta = String::from_utf8_lossy(&meta.stdout);
        assert!(meta.contains("Never Gonna"), "{meta}");

        player.seek(100.0).await.unwrap();
        assert!(wait(101.0).await, "seek did not take effect");
        player.set_fx(Fx { bass: 4.0, speed: 1.25, reverb: 0.4, normalize: true, ..Fx::default() }).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        player.set_fx(Fx::default()).await.unwrap();
        player.toggle().await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let status = tokio::process::Command::new("playerctl").args(["-p", snellar_core::product::ID, "status"]).output().await.unwrap();
        assert_eq!(String::from_utf8_lossy(&status.stdout).trim(), "Paused");
        assert_eq!(library.history(5).len(), 1);
    }

    fn album_card() -> Pin {
        Pin { kind: "album".into(), id: "al".into(), item: json!({ "id": "al", "title": "Album" }) }
    }

    #[tokio::test]
    async fn plays_remember_the_collection_they_came_from_and_only_its_own_tracks() {
        let library = memory();
        let (player, _, _) = rig(Arc::new(|_| {}), library.clone(), Capabilities::default()).await;
        player.play(tracks("ab"), 0, Some(album_card())).await.unwrap();
        player.enqueue(vec![track("x")], false).await.unwrap();
        player.next().await.unwrap();
        player.next().await.unwrap();
        let cards: Vec<(String, String)> = library.recent(10).into_iter().map(|p| (p.kind, p.id)).collect();
        assert_eq!(cards, [("album".to_string(), "al".to_string()), ("track".to_string(), "x".to_string())]);
    }

    #[tokio::test]
    async fn a_radio_forgets_the_collection_that_played_before_it() {
        let library = memory();
        let (player, _, _) = rig(Arc::new(|_| {}), library.clone(), Capabilities::default()).await;
        player.play(tracks("ab"), 0, Some(album_card())).await.unwrap();
        library.clear_history();
        player.radio(track("a")).await.unwrap();
        assert!(library.recent(10).iter().all(|p| p.kind == "track"), "{:?}", library.recent(10));
    }

    #[tokio::test]
    async fn the_collection_is_still_known_after_a_restart() {
        let library = memory();
        let origin = Origin { card: album_card(), tracks: tracks("ab").into_iter().map(|t| t.id).collect() };
        let saved = Saved { tracks: tracks("ab"), cursor: Some(0), origin: Some(origin) };
        library.set_setting("queue", &serde_json::to_string(&saved).unwrap());
        let (player, _, _) = rig(Arc::new(|_| {}), library.clone(), Capabilities::default()).await;
        player.next().await.unwrap();
        assert_eq!(library.recent(10).first().map(|p| p.kind.clone()), Some("album".to_string()));
    }
}
