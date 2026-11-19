//! What reaches mpv: the output (started on first use, started again after mpv died), the effects chain with its
//! live commands (decision 0096), loudness and its memory (decision 0101), and the pause.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use anyhow::Result;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use snellar_core::error::Code;
use snellar_core::ids::TrackId;
use snellar_core::library::Library;
use snellar_core::model::{AudioFormat, Fx, Leveling, StreamInfo};
use snellar_core::settings;
use snellar_core::sync::Lock;
use snellar_core::ui::{Toast, UiEvent};

use crate::audio::{AudioOut, Event, Launch};
use crate::fx;

use super::devices::read_devices;
use super::load::{LoadId, Phase};
use super::{Link, MEASURE_EVERY, MEASURED_ENOUGH, MEASURED_SHARE, Msg, Owner, Reply, Shared};

/// How many times mpv is started for one use when each one dies before it is ready.
const STARTS: u32 = 3;

pub(super) struct Output {
    launch: Launch,
    slot: tokio::sync::Mutex<Slot>,
    /// The turn: whatever sends a plan of the effects holds it, and so do a load and a seek, which must not land
    /// between a filter rebuild and the commands after it. A fair lock, so calls go in the order they came. It
    /// guards what mpv was last sent of the effects.
    pub(super) turn: tokio::sync::Mutex<Option<fx::Applied>>,
    pub(super) wanted: Mutex<Wanted>,
    /// The current load's wanted pause, mirrored here by the owner task for whoever sends `pause`. With no load it
    /// keeps the last one, which an automatic move carries to the next load.
    pub(super) pause: AtomicBool,
}

/// The running mpv, if any, and a count of the mpvs started.
#[derive(Default)]
struct Slot {
    running: Option<Running>,
    starts: u64,
}

/// An mpv in use, with the number of its start. Its watcher passes on what it says only while it is the one in the
/// slot, and only whoever takes it out of the slot tells the owner it died.
struct Running {
    number: u64,
    audio: Arc<dyn AudioOut>,
}

/// What the loudness meter read of a load while its track was measured (decision 0101).
#[derive(Default)]
pub(super) struct Measure {
    /// Where in the track the meter was last asked.
    pub(super) asked: Option<f64>,
    /// The reading that covers the most of the track; mpv builds the meter again for each file and after each seek,
    /// so a later reading may cover less.
    pub(super) best: Option<Reading>,
    /// The seconds the reading kept last covers; 0 before one is kept.
    pub(super) kept: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Reading {
    pub(super) lufs: f64,
    pub(super) seconds: f64,
    /// The track's length when it was read.
    pub(super) length: f64,
}

impl Reading {
    /// Covers enough of the track to stand for all of it.
    pub(super) fn enough(&self) -> bool {
        self.seconds >= MEASURED_ENOUGH.max(self.length * MEASURED_SHARE)
    }
}

/// Keeps a load's best reading for its track once it covers enough of it, and again when a later one covers more. The
/// write is a job; the library keeps the measure that covers most, whichever lands first.
pub(super) fn remember_loudness(shared: &Arc<Shared>, id: &TrackId, measure: &mut Measure) {
    let Some(reading) = measure.best.filter(|r| r.enough() && r.seconds > measure.kept) else { return };
    measure.kept = reading.seconds;
    let (job, id) = (shared.clone(), id.clone());
    shared.tasks.spawn("remember the loudness", async move {
        job.library.remember_loudness(&id, reading.lufs, reading.seconds);
    });
}

/// What the person wants of the output. Read under the turn when a plan is made.
pub(super) struct Wanted {
    pub(super) fx: Fx,
    pub(super) measuring: bool,
    pub(super) volume: f64,
    pub(super) device: String,
}

impl Output {
    pub(super) fn new(launch: Launch, wanted: Wanted) -> Output {
        Output {
            launch,
            slot: tokio::sync::Mutex::new(Slot::default()),
            turn: tokio::sync::Mutex::new(None),
            wanted: Mutex::new(wanted),
            pause: AtomicBool::new(false),
        }
    }

    /// The running output, without starting one.
    pub(super) async fn current(&self) -> Option<Arc<dyn AudioOut>> {
        self.slot.lock().await.running.as_ref().map(|r| r.audio.clone())
    }

    pub(super) async fn apply_fx(&self, audio: &dyn AudioOut, mode: fx::Mode) -> Result<()> {
        let mut applied = self.turn.lock().await;
        self.send_fx(&mut applied, audio, mode).await
    }

    /// Runs `then` after the effects settled, within the turn.
    pub(super) async fn settled<T>(&self, audio: &dyn AudioOut, then: impl Future<Output = Result<T>>) -> Result<T> {
        let mut applied = self.turn.lock().await;
        self.send_fx(&mut applied, audio, fx::Mode::Settle).await?;
        then.await
    }

    /// Sends what differs between the effects mpv has and the wanted ones. The caller holds the turn.
    pub(super) async fn send_fx(&self, applied: &mut Option<fx::Applied>, audio: &dyn AudioOut, mut mode: fx::Mode) -> Result<()> {
        loop {
            let (plan, then) = {
                let wanted = self.wanted.locked();
                fx::plan(applied.as_ref(), &wanted.fx, wanted.measuring, mode)
            };
            if plan == fx::Plan::default() {
                return Ok(());
            }
            if commands(audio, &plan.commands).await.is_err() {
                *applied = None;
                continue;
            }
            match rebuild(audio, &plan).await {
                Ok(()) => {
                    *applied = Some(then);
                    mode = fx::Mode::Live;
                }
                Err(e) => {
                    *applied = None;
                    return Err(e);
                }
            }
        }
    }

    /// Holds the turn from writing the wanted effects to saving them, so a later move is never saved before an
    /// earlier one, and a refused move rolls back.
    pub(super) async fn set_fx(&self, audio: &dyn AudioOut, fx: Fx, library: &Library) -> Result<()> {
        let mut applied = self.turn.lock().await;
        let before = std::mem::replace(&mut self.wanted.locked().fx, fx.clone());
        match self.send_fx(&mut applied, audio, fx::Mode::Live).await {
            Ok(()) => {
                settings::FX.set(library, &fx);
                Ok(())
            }
            Err(e) => {
                self.wanted.locked().fx = before;
                Err(e)
            }
        }
    }

    /// Sends the wanted pause, and again whenever it changed while the last one was on its way. Every sender does
    /// this, so the value that lands last is the one wanted, whatever order the sends complete in. Says how the
    /// first send went.
    pub(super) async fn send_pause(&self, audio: &dyn AudioOut) -> Result<()> {
        let mut first = None;
        loop {
            let wanted = self.pause.load(Ordering::SeqCst);
            let sent = audio.set("pause", json!(wanted)).await;
            first.get_or_insert(sent);
            if self.pause.load(Ordering::SeqCst) == wanted {
                return first.unwrap_or(Ok(()));
            }
        }
    }
}

impl Shared {
    /// The output, started on first use with the volume, the effects, the device and the pause the person wants. An
    /// mpv that is gone but has not said so yet is not handed out: it is taken out (and the owner told it died) and a
    /// new one started. One that dies before it is ready, or as it starts (before its socket answers or while it is
    /// told what to observe), is started again, a few times, as a hand would after it.
    pub(super) async fn audio(&self) -> Result<Arc<dyn AudioOut>> {
        let mut slot = self.out.slot.lock().await;
        if let Some(running) = &slot.running {
            if !running.audio.gone() {
                return Ok(running.audio.clone());
            }
            slot.running = None;
            self.link.send(Msg::Audio(Event::Died));
        }
        let mut starts = 1;
        loop {
            slot.starts += 1;
            let number = slot.starts;
            let ready = match (self.out.launch)().await {
                Ok((audio, events)) => {
                    self.watch(number, events);
                    match self.ready(&*audio).await {
                        Ok(()) => Ok(audio),
                        Err(e) => Err((e, audio.gone())),
                    }
                }
                Err(e) => Err((e, true)),
            };
            match ready {
                Ok(audio) => {
                    slot.running = Some(Running { number, audio: audio.clone() });
                    return Ok(audio);
                }
                Err((_, true)) if starts < STARTS => starts += 1,
                Err((e, _)) => return Err(e),
            }
        }
    }

    /// Sends a new mpv the volume, the effects, the device and the pause. Effects mpv refuses are reset, unless mpv is
    /// gone, which is no fault of theirs.
    async fn ready(&self, audio: &dyn AudioOut) -> Result<()> {
        *self.out.turn.lock().await = None;
        let volume = self.out.wanted.locked().volume;
        audio.set("volume", json!(volume)).await?;
        if let Err(e) = self.out.apply_fx(audio, fx::Mode::Settle).await {
            if audio.gone() {
                return Err(e);
            }
            self.out.wanted.locked().fx = Fx::default();
            self.out.apply_fx(audio, fx::Mode::Settle).await?;
            settings::FX.set(&self.library, &Fx::default());
            self.emit(UiEvent::Toast(Toast::failed(Code::FxReset, &e)));
        }
        let devices = read_devices(&audio.get("audio-device-list").await?);
        self.route(audio, &devices).await?;
        if self.out.pause.load(Ordering::SeqCst) {
            self.out.send_pause(audio).await?;
        }
        Ok(())
    }

    fn watch(&self, number: u64, events: UnboundedReceiver<Event>) {
        self.tasks.spawn("audio events", forward(self.me.clone(), self.link.clone(), number, events));
    }
}

/// Passes what mpv start `number` says to the owner task, while it is the mpv in the slot: one that never got there
/// (it died before it was ready) or was taken out already is not the player's. When it dies it is taken out first,
/// under the slot's lock, so the next use starts a new one and the owner hears of the death before any of the new
/// one's files.
async fn forward(me: Weak<Shared>, link: Link, number: u64, mut events: UnboundedReceiver<Event>) {
    while let Some(event) = events.recv().await {
        let Some(shared) = me.upgrade() else { return };
        let mut slot = shared.out.slot.lock().await;
        if slot.running.as_ref().is_none_or(|r| r.number != number) {
            continue;
        }
        if matches!(event, Event::Died) {
            slot.running = None;
        }
        if !link.send(Msg::Audio(event)) {
            return;
        }
    }
}

impl Owner {
    /// Describes the current load's stream, which is about to play.
    pub(super) fn describe(&mut self, format: Option<&AudioFormat>, local: bool) {
        let gain = format.and_then(|f| f.loudness_db).map(fx::gain);
        let format = format.cloned().unwrap_or_default();
        self.stream = StreamInfo {
            codec: format.codec,
            bitrate: format.bitrate,
            sample_rate: format.sample_rate,
            gain,
            leveling: leveling(self.loudness, gain),
            local,
        };
        self.shared.emit(UiEvent::Stream(self.stream.clone()));
        self.set_measuring(self.stream.leveling == Leveling::Measured);
        if let Some(load) = &mut self.load {
            load.described = true;
            if gain.is_some() {
                load.measure.best = None;
            }
        }
    }

    /// While the current track is measured and plays, reads the meter every few seconds of the track. The reading comes
    /// back for the load's file; a job, so the owner never waits for mpv.
    pub(super) fn measure_soon(&mut self, t: f64) {
        if !self.measuring || !self.in_mpv() || self.phase() != Phase::Playing {
            return;
        }
        let Some(load) = &mut self.load else { return };
        let Some(file) = load.file else { return };
        if load.measure.asked.is_some_and(|at| (t - at).abs() < MEASURE_EVERY) {
            return;
        }
        load.measure.asked = Some(t);
        let (shared, link, id) = (self.shared.clone(), self.link.clone(), load.id);
        self.shared.tasks.spawn("read the loudness meter", async move {
            let Some(audio) = shared.out.current().await else { return };
            let (Ok(metadata), Ok(rate)) = (audio.get(fx::MEASURED).await, audio.get("audio-params/samplerate").await) else { return };
            if let Some((lufs, seconds)) = fx::measured(&metadata, &rate) {
                link.send(Msg::Measured { load: id, file, lufs, seconds });
            }
        });
    }

    /// The meter's reading for a file of the current load. One for a file mpv no longer plays is dropped; one taken as
    /// mpv moved on to the next file covers a second or two, never enough to be kept.
    pub(super) fn measured(&mut self, id: LoadId, file: u64, lufs: f64, seconds: f64) {
        if !self.is_current(id) || !self.in_mpv() || self.file != Some(file) {
            return;
        }
        let length = self.play.duration;
        let Some(load) = &mut self.load else { return };
        if load.measure.best.is_none_or(|best| seconds > best.seconds) {
            load.measure.best = Some(Reading { lufs, seconds, length });
        }
        if load.measure.kept == 0.0 {
            remember_loudness(&self.shared, &load.track.id, &mut load.measure);
        }
    }

    /// The gain to load the described stream with, for the loudness as it is now.
    pub(super) fn gain(&self) -> f64 {
        self.stream.gain.filter(|_| self.loudness).unwrap_or(0.0)
    }

    fn set_measuring(&mut self, on: bool) {
        self.measuring = on;
        self.shared.out.wanted.locked().measuring = on;
    }

    /// The current load's stream follows at once once it is described, also while its file is on its way: the hand
    /// reads the gain and the chain when it loads it, and the switch holds the turn, so it lands after that load. A
    /// next track already appended carries the gain it was appended with, so it is appended again.
    pub(super) fn set_loudness(&mut self, on: bool, reply: Reply) {
        let changed = on != self.loudness;
        self.loudness = on;
        if changed {
            self.preload_for_loudness();
        }
        let playing = self.load.as_ref().is_some_and(|l| l.described) && self.queue.current().is_some();
        if playing {
            self.stream.leveling = leveling(on, self.stream.gain);
            self.shared.emit(UiEvent::Stream(self.stream.clone()));
        }
        let measure = playing && self.stream.leveling == Leveling::Measured;
        let change = measure != self.measuring;
        self.set_measuring(measure);
        let gain = if on { self.stream.gain.unwrap_or(0.0) } else { 0.0 };
        let (shared, link) = (self.shared.clone(), self.link.clone());
        self.shared.tasks.spawn("switch loudness", async move {
            let switched = async {
                let audio = shared.audio().await?;
                let mut turn = shared.out.turn.lock().await;
                audio.set("replaygain-fallback", json!(gain)).await?;
                if change {
                    shared.out.send_fx(&mut turn, &*audio, fx::Mode::Live).await?;
                }
                Ok(())
            }
            .await;
            link.send(Msg::Answer(reply, switched));
        });
    }
}

pub(super) fn leveling(on: bool, gain: Option<f64>) -> Leveling {
    match (on, gain) {
        (false, _) => Leveling::Off,
        (true, Some(_)) => Leveling::Gain,
        (true, None) => Leveling::Measured,
    }
}

async fn commands(audio: &dyn AudioOut, knobs: &[fx::Knob]) -> Result<()> {
    for knob in knobs {
        audio.af_command(fx::LIVE, &knob.target, knob.option, &knob.value).await?;
    }
    Ok(())
}

async fn rebuild(audio: &dyn AudioOut, plan: &fx::Plan) -> Result<()> {
    if let Some(af) = &plan.af {
        audio.set("af", json!(af)).await?;
    }
    for (name, value) in &plan.properties {
        audio.set(name, value.clone()).await?;
    }
    Ok(())
}
