//! The player. One owner task holds the queue, the load in progress and what mpv said, and handles one message at a
//! time: the `Player` methods, mpv's events and the results of its own jobs all arrive as messages. Slow work (a
//! stream to resolve, a radio to ask for, a file to hand to mpv) runs in jobs that report back with the id of the
//! load they were for, so a result a later move made stale is dropped by identity. Decision 0100.

mod devices;
mod effects;
mod gapless;
mod listen;
mod load;
mod queue;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, WeakUnboundedSender};
use tokio::sync::oneshot;

use snellar_core::error::Code;
use snellar_core::ids::TrackId;
use snellar_core::library::Library;
use snellar_core::model::{AudioFormat, Fx, Pin, Repeat, StreamInfo, Track, Via};
use snellar_core::settings;
use snellar_core::signals::{Ended, Presence};
use snellar_core::source::{Source, Stream};
use snellar_core::sync::Lock;
use snellar_core::tasks::TaskSet;
use snellar_core::ui::{PlayFailure, PlayState, Ui, UiEvent};

use crate::audio::{Event, Launch};
use crate::discord;
use crate::fx;
use crate::mpris::{self, Command, Update};

use effects::{Output, Wanted};
use gapless::Edit;
use listen::Listening;
use load::{Failures, Found, Hand, Load, LoadId, Move, Phase};
use queue::{Origin, Queue, Saved};

const POSITION_INTERVAL: Duration = Duration::from_millis(200);
const SAVE_INTERVAL: Duration = Duration::from_secs(5);
const QUEUE_SAVE_DELAY: Duration = Duration::from_millis(500);
const STALL_AFTER: Duration = Duration::from_secs(8);
const PREFETCH_AHEAD: usize = 2;
const TOP_UP_AT: usize = 2;
const EOF_SLACK: f64 = 5.0;
const MAX_FAILURES: u8 = 3;
const MAX_REFUSED: u8 = 10;
const AUTO_RETRIES: u8 = 3;
const AUTO_RETRY_BASE: Duration = Duration::from_secs(5);
const PLAYED_ENOUGH: f64 = 3.0;
const PRELOAD_LEAD: f64 = 10.0;
/// How often, in seconds of the track, the loudness meter of a measured track is read.
const MEASURE_EVERY: f64 = 5.0;
/// A measure is kept once it covers at least this many seconds and this share of the track.
const MEASURED_ENOUGH: f64 = 30.0;
const MEASURED_SHARE: f64 = 0.5;

type Reply = oneshot::Sender<Result<()>>;

/// A job's line back to the owner task. It is weak: once the player is dropped, what a job brings back is dropped.
#[derive(Clone)]
struct Link(WeakUnboundedSender<Msg>);

impl Link {
    fn send(&self, msg: Msg) -> bool {
        self.0.upgrade().is_some_and(|tx| tx.send(msg).is_ok())
    }

    /// Asks the owner task and waits for the answer; `None` once the player is gone.
    async fn ask<T>(&self, msg: impl FnOnce(oneshot::Sender<T>) -> Msg) -> Option<T> {
        let (tx, rx) = oneshot::channel();
        if !self.send(msg(tx)) {
            return None;
        }
        rx.await.ok()
    }
}

enum Msg {
    Play {
        tracks: Vec<Track>,
        index: usize,
        card: Option<Pin>,
        reply: Reply,
    },
    Radio {
        seed: Track,
        position: f64,
        reply: Reply,
    },
    Enqueue {
        tracks: Vec<Track>,
        next: bool,
        reply: Reply,
    },
    Toggle(Reply),
    Next(Reply),
    Prev(Reply),
    Seek(f64, Reply),
    Jump {
        index: usize,
        id: Option<TrackId>,
        reply: Reply,
    },
    Remove {
        index: usize,
        id: Option<TrackId>,
        reply: Reply,
    },
    MoveItem {
        from: usize,
        to: usize,
        id: Option<TrackId>,
        reply: Reply,
    },
    ClearOthers(oneshot::Sender<bool>),
    Clear(Reply),
    Shuffle(bool, Reply),
    Repeat(Repeat, Reply),
    CancelStart(Reply),
    Volume(f64, Reply),
    Loudness(bool, Reply),
    Audio(Event),
    Resolved {
        load: LoadId,
        found: Result<Found>,
    },
    Hand {
        load: LoadId,
        hand: u64,
        reply: oneshot::Sender<Option<Hand>>,
    },
    /// A hand failed. That a hand succeeded is never told this way: mpv's mark says it, in order with its events.
    HandFailed {
        load: LoadId,
        hand: u64,
        error: anyhow::Error,
        /// mpv went away under it; its end is on its way.
        gone: bool,
    },
    Refetched {
        load: LoadId,
        found: Result<Option<Stream>>,
    },
    Current {
        load: Option<LoadId>,
        reply: oneshot::Sender<bool>,
    },
    /// A seek job holds the turn: may it send seek `mark`?
    LatestSeek {
        load: LoadId,
        mark: u64,
        reply: oneshot::Sender<bool>,
    },
    /// A seek job came back. That mpv took the seek is not told this way: mpv's mark says it, in order with its events.
    SeekSent {
        load: LoadId,
        mark: u64,
        result: Result<()>,
        /// mpv went away under it; its end is on its way.
        gone: bool,
        reply: Option<Reply>,
    },
    /// A job's answer: sent from the owner once the view holds what the message that spawned the job changed.
    Answer(Reply, Result<()>),
    Continued {
        /// The load count when the radio was asked: any load started since means something else plays.
        asked: u64,
        epoch: u64,
        mv: Move,
        related: Result<Vec<Track>>,
        reply: Option<Reply>,
    },
    RadioArrived {
        epoch: u64,
        related: Result<Vec<Track>>,
        loaded: Result<()>,
        reply: Reply,
    },
    ToppedUp {
        epoch: u64,
        related: Result<Vec<Track>>,
    },
    Lengths {
        asked: Vec<TrackId>,
        found: Result<HashMap<TrackId, f64>>,
    },
    Preloaded {
        load: LoadId,
        id: TrackId,
        found: Result<(Stream, bool)>,
    },
    StallExpired {
        load: LoadId,
        stall: u64,
    },
    /// The loudness meter's reading for the load's file.
    Measured {
        load: LoadId,
        file: u64,
        lufs: f64,
        seconds: f64,
    },
    RetryDue(LoadId),
    SaveQueue,
}

/// What the `Player` methods that answer at once read: the owner task writes it after each message, before it
/// answers anyone.
#[derive(Default)]
struct View {
    saved: Arc<Saved>,
    play: PlayState,
    stream: StreamInfo,
    failure: Option<PlayFailure>,
}

/// What the owner task, its jobs and the `Player` handle share. Jobs hold it strongly; dropping the `Player` aborts
/// them, so nothing keeps it (and mpv) alive after the player is gone. Nothing here is player state but the open play
/// (`Listening`) and the copy for readers (`View`), each behind a lock held for a few lines.
struct Shared {
    ui: Ui,
    source: Arc<dyn Source>,
    library: Arc<Library>,
    out: Output,
    mpris: UnboundedSender<Update>,
    discord: Sender<discord::Msg>,
    tasks: TaskSet,
    presence: OnceLock<Arc<AtomicU8>>,
    listening: Listening,
    view: Mutex<View>,
    link: Link,
    me: Weak<Shared>,
}

impl Shared {
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

    fn prefetch(&self, id: &TrackId) {
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
            Some(stream) if !fresh => Ok((self.remembered(id, stream), true)),
            _ => Ok((self.remembered(id, self.source.stream(id, fresh).await?), false)),
        }
    }

    /// A stream without a loudness value gets the one measured when the track last played, if any; a value from the
    /// source is never replaced.
    fn remembered(&self, id: &TrackId, mut stream: Stream) -> Stream {
        if stream.format.as_ref().is_none_or(|f| f.loudness_db.is_none())
            && let Some(lufs) = self.library.measured_loudness(id)
        {
            stream.format.get_or_insert_default().loudness_db = Some(fx::loudness_db(lufs));
        }
        stream
    }

    fn presence(&self) -> Presence {
        self.presence.get().map_or(Presence::Unknown, |p| Presence::of(p.load(Ordering::Relaxed)))
    }

    fn autoplay(&self) -> bool {
        settings::AUTOPLAY.or_default(&self.library)
    }

    fn has_radio(&self) -> bool {
        self.source.capabilities().radio
    }

    fn timed(&self, mut tracks: Vec<Track>) -> Vec<Track> {
        self.library.fill_lengths(&mut tracks);
        tracks
    }
}

fn mpris_control(me: Weak<Player>, rt: Handle) -> mpris::Control {
    Arc::new(move |command| {
        if let Some(p) = me.upgrade() {
            let _rt = rt.enter();
            let player = p.clone();
            p.shared.tasks.spawn("mpris command", async move { player.obey(command).await });
        }
    })
}

pub struct Player {
    shared: Arc<Shared>,
    tx: UnboundedSender<Msg>,
    /// Holds the owner task; dropping the player stops it.
    owner: TaskSet,
}

impl Drop for Player {
    /// The owner first, so it starts no job after the jobs are aborted; a turn it is in the middle of finds the jobs'
    /// set closed.
    fn drop(&mut self) {
        self.owner.abort_all();
        self.shared.tasks.abort_all();
    }
}

impl Player {
    pub async fn start(ui: Ui, source: Arc<dyn Source>, launch: Launch, library: Arc<Library>) -> Result<Arc<Player>> {
        let volume = settings::VOLUME.or_default(&library);
        let fx = fx::clamp(&settings::FX.or_default(&library));
        let loudness = settings::LOUDNESS.or_default(&library);
        let device = settings::AUDIO_DEVICE.or_default(&library);
        let rt = Handle::current();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let link = Link(tx.downgrade());
        let player = Arc::new_cyclic(|me: &Weak<Player>| {
            let shared = Arc::new_cyclic(|this| Shared {
                ui,
                source,
                library: library.clone(),
                out: Output::new(launch, Wanted { fx, measuring: false, volume, device }),
                mpris: mpris::spawn(mpris_control(me.clone(), rt)),
                discord: discord::spawn(library),
                tasks: TaskSet::default(),
                presence: OnceLock::new(),
                listening: Listening::default(),
                view: Mutex::default(),
                link,
                me: this.clone(),
            });
            Player { shared, tx, owner: TaskSet::default() }
        });
        let (edits, list) = tokio::sync::mpsc::unbounded_channel();
        let mut owner = Owner::new(player.shared.clone(), edits, volume, loudness);
        owner.restore();
        if let Some(track) = owner.queue.current() {
            player.shared.prefetch(&track.id);
        }
        owner.publish_state();
        owner.sync_view();
        player.owner.spawn("player", owner.run(rx));
        player.owner.spawn("mpv's list", gapless::edit_list(Arc::downgrade(&player.shared), list));
        player.shared.audio().await?;
        Ok(player)
    }

    fn send(&self, msg: Msg) {
        self.tx.send(msg).ok();
    }

    /// Sends a message and waits for its answer. An answer the owner dropped (a load it replaced) is a quiet `Ok`.
    async fn ask(&self, msg: impl FnOnce(Reply) -> Msg) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.send(msg(tx));
        rx.await.unwrap_or(Ok(()))
    }

    pub fn snapshot(&self) -> Value {
        let view = self.shared.view.locked();
        json!({
            "current": view.saved.current(),
            "playing": view.play.playing,
            "loading": view.play.loading,
            "position": view.play.position,
            "duration": view.play.duration,
            "volume": view.play.volume,
            "shuffle": view.saved.shuffle,
            "repeat": view.saved.repeat,
            "stream": view.stream,
            "failure": view.failure,
        })
    }

    pub fn queue(&self) -> Value {
        let view = self.shared.view.locked();
        json!({ "tracks": view.saved.tracks, "cursor": view.saved.cursor.map_or(-1, |c| c as i64) })
    }

    pub fn current(&self) -> Option<Track> {
        self.shared.view.locked().saved.current().cloned()
    }

    /// On quit: ends the open play, keeps the position and saves the queue, without waiting for the owner task.
    pub fn save_now(&self) {
        let (duration, position, current, saved) = {
            let view = self.shared.view.locked();
            (view.play.duration, view.play.position, view.saved.current().is_some(), view.saved.clone())
        };
        self.shared.listening.finish(&self.shared.library, duration, Ended::Quit);
        if current {
            settings::POSITION.set(&self.shared.library, &position);
        }
        settings::QUEUE.set(&self.shared.library, &*saved);
    }

    pub async fn play(&self, tracks: Vec<Track>, index: usize, card: Option<Pin>) -> Result<()> {
        if tracks.is_empty() {
            return Ok(());
        }
        let tracks = self.shared.timed(tracks);
        self.ask(|reply| Msg::Play { tracks, index, card, reply }).await
    }

    pub async fn radio(&self, seed: Track) -> Result<()> {
        self.radio_from(seed, 0.0).await
    }

    pub async fn radio_from(&self, seed: Track, position: f64) -> Result<()> {
        let seed = self.shared.timed(vec![seed]).remove(0);
        self.ask(|reply| Msg::Radio { seed, position, reply }).await
    }

    pub async fn enqueue(&self, tracks: Vec<Track>, next: bool) -> Result<()> {
        let tracks = self.shared.timed(tracks);
        self.ask(|reply| Msg::Enqueue { tracks, next, reply }).await
    }

    pub fn is_near(&self, id: &TrackId) -> bool {
        let view = self.shared.view.locked();
        let from = view.saved.cursor.unwrap_or(0);
        view.saved.tracks.iter().skip(from).take(2).any(|t| t.id == *id)
    }

    pub fn prefetch(&self, id: &TrackId) {
        self.shared.prefetch(id);
    }

    async fn obey(&self, command: Command) {
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

    pub async fn toggle(&self) -> Result<()> {
        self.ask(Msg::Toggle).await
    }

    pub async fn next(&self) -> Result<()> {
        self.ask(Msg::Next).await
    }

    pub async fn prev(&self) -> Result<()> {
        self.ask(Msg::Prev).await
    }

    pub async fn seek(&self, pos: f64) -> Result<()> {
        self.ask(|reply| Msg::Seek(pos.max(0.0), reply)).await
    }

    pub async fn set_volume(&self, v: f64) -> Result<()> {
        let v = v.clamp(0.0, 100.0);
        self.shared.audio().await?.set("volume", json!(v)).await?;
        settings::VOLUME.set(&self.shared.library, &v);
        self.shared.out.wanted.locked().volume = v;
        self.ask(|reply| Msg::Volume(v, reply)).await
    }

    pub async fn jump(&self, index: usize, id: Option<&TrackId>) -> Result<()> {
        let id = id.cloned();
        self.ask(|reply| Msg::Jump { index, id, reply }).await
    }

    pub async fn remove(&self, index: usize, id: Option<&TrackId>) -> Result<()> {
        let id = id.cloned();
        self.ask(|reply| Msg::Remove { index, id, reply }).await
    }

    pub async fn move_item(&self, from: usize, to: usize, id: Option<&TrackId>) -> Result<()> {
        let id = id.cloned();
        self.ask(|reply| Msg::MoveItem { from, to, id, reply }).await
    }

    pub async fn clear_others(&self) -> bool {
        let (tx, rx) = oneshot::channel();
        self.send(Msg::ClearOthers(tx));
        rx.await.unwrap_or(false)
    }

    pub async fn clear(&self) {
        self.ask(Msg::Clear).await.ok();
    }

    pub async fn set_shuffle(&self, on: bool) {
        self.ask(|reply| Msg::Shuffle(on, reply)).await.ok();
    }

    pub async fn set_repeat(&self, mode: Repeat) {
        self.ask(|reply| Msg::Repeat(mode, reply)).await.ok();
    }

    pub async fn set_fx(&self, fx: Fx) -> Result<()> {
        let audio = self.shared.audio().await?;
        self.shared.out.set_fx(&*audio, fx::clamp(&fx), &self.shared.library).await
    }

    pub async fn devices(&self) -> Result<Value> {
        self.shared.devices().await
    }

    pub async fn set_device(&self, name: &str) -> Result<()> {
        self.shared.set_device(name).await
    }

    pub async fn set_loudness(&self, on: bool) -> Result<()> {
        self.ask(|reply| Msg::Loudness(on, reply)).await
    }

    pub async fn cancel_start(&self) {
        self.ask(Msg::CancelStart).await.ok();
    }

    /// Removes every play of a track from the history and from what recommendations learn. When it is the song that
    /// plays, its open play is dropped too, so its end is never written; the song plays on.
    pub fn remove_from_history(&self, id: &TrackId) {
        self.shared.listening.forget(id);
        self.shared.library.forget_track(id);
        self.shared.emit(UiEvent::History);
    }

    pub fn discord_changed(&self) {
        self.shared.discord.send(discord::Msg::Settings).ok();
    }

    pub fn sense(&self, presence: Arc<AtomicU8>) {
        self.shared.presence.set(presence).ok();
    }
}

/// What mpv last said about itself.
struct Mpv {
    paused: bool,
    idle: bool,
    cache: bool,
    seeking: bool,
}

/// Where the current track starts the next time it is played from idle: the restored position, where mpv died,
/// where a failure stopped it, or a seek while nothing played. Only a resume of that same track takes it.
struct Parked {
    id: TrackId,
    at: f64,
}

/// The owner task's state. Only the task touches it.
struct Owner {
    shared: Arc<Shared>,
    link: Link,
    queue: Queue,
    repeat: Repeat,
    origin: Option<Origin>,
    /// How autoplay marks the tracks it adds after the current list.
    continuing: Via,
    play: PlayState,
    mpv: Mpv,
    load: Option<Load>,
    /// Numbers loads and files: a source of fresh ids, compared only for equality. `loads` also moves when a load
    /// is dropped without a new one, so it changes whenever the current load does.
    loads: u64,
    files: u64,
    /// Numbers the seeks and the clears sent to mpv, for mpv's mark that it took one.
    marks: u64,
    /// The last file a clear sent to mpv's list after it takes out (appends and clears go to mpv in order, `Edit`).
    cleared: u64,
    edits: UnboundedSender<Edit>,
    /// The file mpv plays, as its ordered events tell: set by its mark for a load (`Event::Loaded`) or by the hand-over
    /// to the file queued after it, cleared when mpv dies. What mpv says is about this file.
    file: Option<u64>,
    parked: Option<Parked>,
    failures: Failures,
    heard_duration: Option<f64>,
    stream: StreamInfo,
    loudness: bool,
    measuring: bool,
    /// The queue (epoch) a radio top-up is running for: a new queue does not wait for the old one's answer.
    topping: Option<u64>,
    asked: HashSet<TrackId>,
    saved: Instant,
    told: Instant,
    queue_due: bool,
    view_due: bool,
    /// Answers held until the view is written, so whoever is answered reads the state the answer is about.
    later: Vec<Box<dyn FnOnce() + Send>>,
}

impl Owner {
    fn new(shared: Arc<Shared>, edits: UnboundedSender<Edit>, volume: f64, loudness: bool) -> Owner {
        Owner {
            link: shared.link.clone(),
            shared,
            queue: Queue::new(),
            repeat: Repeat::Off,
            origin: None,
            continuing: Via::Autoplay,
            play: PlayState { volume, ..PlayState::default() },
            mpv: Mpv { paused: false, idle: true, cache: false, seeking: false },
            load: None,
            loads: 0,
            files: 0,
            marks: 0,
            cleared: 0,
            edits,
            file: None,
            parked: None,
            failures: Failures::default(),
            heard_duration: None,
            stream: StreamInfo::default(),
            loudness,
            measuring: false,
            topping: None,
            asked: HashSet::new(),
            saved: Instant::now(),
            told: Instant::now(),
            queue_due: false,
            view_due: true,
            later: Vec::new(),
        }
    }

    async fn run(mut self, mut rx: UnboundedReceiver<Msg>) {
        while let Some(msg) = rx.recv().await {
            self.handle(msg);
            self.sync_view();
            for answer in std::mem::take(&mut self.later) {
                answer();
            }
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Play { tracks, index, card, reply } => self.play(tracks, index, card, reply),
            Msg::Radio { seed, position, reply } => self.radio(seed, position, reply),
            Msg::Enqueue { tracks, next, reply } => self.enqueue(tracks, next, reply),
            Msg::Toggle(reply) => self.toggle(reply),
            Msg::Next(reply) => {
                self.chose_to_move();
                self.advance(false, Move::new(Ended::Skipped, false), Some(reply));
            }
            Msg::Prev(reply) => self.prev(reply),
            Msg::Seek(position, reply) => self.seek(position, reply),
            Msg::Jump { index, id, reply } => self.jump(index, id, reply),
            Msg::Remove { index, id, reply } => self.remove(index, id, reply),
            Msg::MoveItem { from, to, id, reply } => {
                let moved = self.move_item(from, to, id);
                self.later(reply, moved);
            }
            Msg::ClearOthers(reply) => {
                let kept = self.queue.keep_current();
                if kept {
                    self.publish_queue();
                }
                self.later(reply, kept);
            }
            Msg::Clear(reply) => {
                self.queue.clear();
                self.origin = None;
                self.halt(Some(reply));
            }
            Msg::Shuffle(on, reply) => {
                self.queue.set_shuffle(on);
                self.publish_queue();
                self.publish_modes();
                self.later(reply, Ok(()));
            }
            Msg::Repeat(mode, reply) => {
                self.repeat = mode;
                self.publish_modes();
                self.save_queue_soon();
                self.later(reply, Ok(()));
            }
            Msg::CancelStart(reply) => self.cancel_start(reply),
            Msg::Volume(v, reply) => {
                self.play.volume = v;
                self.publish_state();
                self.later(reply, Ok(()));
            }
            Msg::Loudness(on, reply) => self.set_loudness(on, reply),
            Msg::Audio(event) => self.heard(event),
            Msg::Resolved { load, found } => self.resolved(load, found),
            Msg::Hand { load, hand, reply } => self.answer_hand(load, hand, reply),
            Msg::HandFailed { load, hand, error, gone } => self.hand_failed(load, hand, error, gone),
            Msg::Refetched { load, found } => self.refetched(load, found),
            Msg::Current { load, reply } => {
                let current = self.current_id() == load;
                self.later(reply, current);
            }
            Msg::LatestSeek { load, mark, reply } => {
                let latest = self.latest_seek(load, mark);
                self.later(reply, latest);
            }
            Msg::SeekSent { load, mark, result, gone, reply } => self.seek_sent(load, mark, result, gone, reply),
            Msg::Answer(reply, result) => self.later(reply, result),
            Msg::Continued { asked, epoch, mv, related, reply } => self.continued(asked, epoch, mv, related, reply),
            Msg::RadioArrived { epoch, related, loaded, reply } => {
                let arrived = loaded.and(related).map(|related| {
                    self.queue.append_if_current(epoch, queue::stamped(related, Via::Radio));
                    self.publish_queue();
                });
                self.later(reply, arrived);
            }
            Msg::ToppedUp { epoch, related } => self.topped_up(epoch, related),
            Msg::Lengths { asked, found } => self.lengths(asked, found),
            Msg::Preloaded { load, id, found } => self.preloaded(load, id, found),
            Msg::StallExpired { load, stall } => self.stall_expired(load, stall),
            Msg::Measured { load, file, lufs, seconds } => self.measured(load, file, lufs, seconds),
            Msg::RetryDue(load) => self.retry_due(load),
            Msg::SaveQueue => self.save_queue(),
        }
    }

    /// Holds an answer until the view is written.
    fn later<T: Send + 'static>(&mut self, reply: oneshot::Sender<T>, value: T) {
        self.later.push(Box::new(move || {
            reply.send(value).ok();
        }));
    }

    fn answer(&mut self, reply: Option<Reply>, result: Result<()>) {
        if let Some(reply) = reply {
            self.later(reply, result);
        }
    }

    fn sync_view(&mut self) {
        let saved = self.view_due.then(|| Arc::new(Saved::of(self)));
        self.view_due = false;
        let failure = self.failure();
        let mut view = self.shared.view.locked();
        if let Some(saved) = saved {
            view.saved = saved;
        }
        view.play = self.play.clone();
        view.stream = self.stream.clone();
        view.failure = failure;
    }

    fn publish_state(&mut self) {
        self.play.playing = !self.mpv.paused && !self.mpv.idle;
        self.play.loading = self.starting() || self.mpv.cache || self.mpv.seeking;
        self.shared.emit(UiEvent::State(self.play.clone()));
    }

    /// A move the person made: the failures before it are forgotten.
    fn chose_to_move(&mut self) {
        self.failures.forget();
        self.shared.emit(UiEvent::PlayFailure(None));
    }

    fn play(&mut self, tracks: Vec<Track>, index: usize, card: Option<Pin>, reply: Reply) {
        let origin = card.map(|card| Origin { card, tracks: tracks.iter().map(|t| t.id.clone()).collect() });
        let same = index < tracks.len()
            && self.queue.tracks.len() == tracks.len()
            && self.queue.tracks.iter().zip(&tracks).all(|(a, b)| a.id == b.id);
        let continuing = match tracks.get(index).map(Track::via) {
            Some(via @ (Via::Wave | Via::Daypart)) => via,
            _ => Via::Autoplay,
        };
        if !same {
            self.replace_queue(tracks, index);
        }
        (self.origin, self.continuing) = (origin, continuing);
        match if same { Some(index) } else { self.queue.cursor } {
            Some(at) => self.begin(at, Move::new(Ended::Replaced, true), Some(reply)),
            None => self.later(reply, Ok(())),
        }
    }

    fn radio(&mut self, seed: Track, position: f64, reply: Reply) {
        let epoch = self.replace_queue(vec![seed.clone()], 0);
        self.continuing = Via::Radio;
        let mv = Move::new(Ended::Replaced, true).at(position);
        if !self.shared.has_radio() {
            return self.begin(0, mv, Some(reply));
        }
        let (started, loaded) = oneshot::channel();
        self.begin(0, mv, Some(started));
        let (shared, link) = (self.shared.clone(), self.link.clone());
        self.shared.tasks.spawn("radio", async move {
            let (loaded, related) =
                tokio::join!(async { loaded.await.unwrap_or(Ok(())) }, shared.source.radio(&seed, std::slice::from_ref(&seed)));
            let related = related.map(|tracks| shared.timed(tracks));
            link.send(Msg::RadioArrived { epoch, related, loaded, reply });
        });
    }

    fn enqueue(&mut self, tracks: Vec<Track>, next: bool, reply: Reply) {
        let Some(at) = self.queue.insert_all(tracks, next) else { return self.later(reply, Ok(())) };
        self.publish_queue();
        match self.queue.cursor {
            None => self.begin(at, Move::new(Ended::Replaced, false), Some(reply)),
            Some(_) => self.later(reply, Ok(())),
        }
    }

    fn prev(&mut self, reply: Reply) {
        match self.queue.cursor {
            Some(c) if c > 0 && self.play.position <= 3.0 => self.begin(c - 1, Move::new(Ended::Skipped, false), Some(reply)),
            _ => self.seek(0.0, reply),
        }
    }

    fn jump(&mut self, index: usize, id: Option<TrackId>, reply: Reply) {
        self.chose_to_move();
        match self.queue.locate(index, id.as_ref()) {
            Some(index) => self.begin(index, Move::new(Ended::Skipped, true), Some(reply)),
            None => self.later(reply, Err(Code::ApiNotFound.err("that track is no longer in the queue"))),
        }
    }

    fn remove(&mut self, index: usize, id: Option<TrackId>, reply: Reply) {
        let Some(index) = self.queue.locate(index, id.as_ref()) else {
            return self.later(reply, Err(Code::ApiNotFound.err("that track is no longer in the queue")));
        };
        let Some(was_current) = self.queue.remove(index) else {
            return self.later(reply, Err(Code::ApiNotFound.err(format!("no queue item {index}"))));
        };
        self.publish_queue();
        match (was_current, self.queue.cursor) {
            (false, _) => self.later(reply, Ok(())),
            (true, Some(next)) => self.begin(next, Move::new(Ended::Skipped, false), Some(reply)),
            (true, None) => self.halt(Some(reply)),
        }
    }

    fn move_item(&mut self, from: usize, to: usize, id: Option<TrackId>) -> Result<()> {
        let from = self.queue.locate(from, id.as_ref()).ok_or_else(|| Code::ApiNotFound.err("that track is no longer in the queue"))?;
        if !self.queue.move_item(from, to) {
            return Err(Code::ApiArgs.err(format!("queue has {} items, cannot move {from} to {to}", self.queue.tracks.len())));
        }
        self.publish_queue();
        Ok(())
    }

    fn heard(&mut self, event: Event) {
        match event {
            Event::Property(name, value) => self.property(&name, value),
            Event::Loaded(file) => self.took(file),
            Event::Appended(file) => self.appended(file),
            Event::Seeked(mark) => self.landed(mark),
            Event::Cleared(mark) => self.list_cleared(mark),
            Event::EndFile { reason, error } => {
                self.heard_duration = None;
                self.ended(self.file, &reason, error);
            }
            Event::Died => self.died(),
        }
    }

    fn property(&mut self, name: &str, value: Value) {
        match (name, &value) {
            ("time-pos", Value::Number(n)) => {
                self.clock(n.as_f64().unwrap_or_default());
                return;
            }
            ("duration", Value::Number(n)) => {
                if !self.in_mpv() {
                    return;
                }
                self.heard_duration = n.as_f64();
                self.learn_duration();
            }
            ("duration", Value::Null) => {
                self.heard_duration = None;
                return;
            }
            ("pause", Value::Bool(b)) => self.mpv.paused = *b,
            ("idle-active", Value::Bool(b)) => self.mpv.idle = *b,
            ("paused-for-cache", Value::Bool(b)) => {
                self.mpv.cache = *b;
                if *b {
                    self.stalled();
                }
            }
            ("seeking", Value::Bool(b)) => self.mpv.seeking = *b,
            ("volume", Value::Number(n)) => self.play.volume = n.as_f64().unwrap_or_default(),
            ("audio-device-list", list) => {
                self.shared.devices_changed(list);
                return;
            }
            ("audio-codec-name", Value::String(codec)) => {
                if self.in_mpv() && self.stream.codec.is_empty() {
                    self.stream.codec = codec.clone();
                    self.shared.emit(UiEvent::Stream(self.stream.clone()));
                }
                return;
            }
            _ => return,
        }
        self.publish_state();
    }

    /// mpv told a position in the current load's file: it is where the file is (`heard`). The first one starts the
    /// play (and sends a seek asked for before); while a seek has not landed, the bar and the play keep the seek.
    fn clock(&mut self, t: f64) {
        if !self.in_mpv() {
            return;
        }
        self.learn_duration();
        let Some(load) = &mut self.load else { return };
        load.heard = t;
        if matches!(load.phase, Phase::Opening | Phase::Retrying) {
            load.played = true;
            let seek = load.seek;
            self.enter(Phase::Playing);
            self.publish_state();
            if let Some(position) = seek {
                self.seek_in_mpv(position, None);
            }
        }
        let Some(load) = self.load.as_mut().filter(|l| l.seek.is_none()) else { return };
        if load.retry_point.is_some_and(|point| t >= point + PLAYED_ENOUGH) {
            load.retry_point = None;
            load.attempts = 0;
        }
        self.play.position = t;
        self.shared.listening.tick(t, self.shared.presence());
        if t >= PLAYED_ENOUGH {
            self.failures.count = 0;
        }
        self.preload_soon(t);
        self.measure_soon(t);
        if self.saved.elapsed() >= SAVE_INTERVAL {
            self.saved = Instant::now();
            settings::POSITION.set(&self.shared.library, &t);
        }
        if self.told.elapsed() >= POSITION_INTERVAL {
            self.told = Instant::now();
            self.shared.emit(UiEvent::Position(t));
        }
    }

    fn learn_duration(&mut self) {
        let Some(duration) = self.heard_duration.filter(|d| *d > 0.0 && self.in_mpv()) else { return };
        self.play.duration = duration;
        let Some(id) = self.queue.current().filter(|t| t.duration <= 0.0).map(|t| t.id.clone()) else { return };
        self.queue.learn_lengths(|t| (*t == id).then_some(duration));
        let Some(track) = self.queue.current().cloned() else { return };
        self.shared.library.remember_track(&track);
        self.shared.emit(UiEvent::History);
        self.publish_queue();
        (self.shared.ui)(UiEvent::Current(Some(track.clone())));
        self.shared.mpris.send(Update::Track(Some(track))).ok();
        self.shared.discord.send(discord::Msg::Duration(duration)).ok();
    }
}
