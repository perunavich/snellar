//! Loading a track and keeping it playing: the per-load state, the phases, the start, the retries when the stream
//! breaks, and what a failure does next.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::oneshot;

use snellar_core::error::{Code, Told};
use snellar_core::ids::TrackId;
use snellar_core::model::Track;
use snellar_core::settings;
use snellar_core::signals::{Ended, Heard};
use snellar_core::source::Stream;
use snellar_core::ui::{PlayFailure, Toast, UiEvent};

use crate::fx;

use super::effects::{Measure, remember_loudness};
use super::gapless::Preload;
use super::{AUTO_RETRIES, AUTO_RETRY_BASE, EOF_SLACK, Link, MAX_FAILURES, MAX_REFUSED, Msg, Owner, Parked, Reply, STALL_AFTER, Shared};

/// Names one load. A job brings its result back with the id of the load it was for, and the owner drops the result
/// unless that load is still the current one: a later move has replaced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LoadId(u64);

/// Where the player is with the current track. Every phase but `Idle` has a load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    /// No load: an empty or restored queue, a queue that ran out, a cleared or cancelled start, a failure that stays,
    /// or mpv gone.
    Idle,
    /// The stream is resolved, or the file is on its way to mpv.
    Resolving,
    /// mpv has the file and has not told a position in it yet.
    Opening,
    /// mpv tells positions in the file: playing, paused or waiting for its cache.
    Playing,
    /// The stream broke while it played; a fresh one is fetched and handed to mpv, which has told no position in it yet.
    Retrying,
    /// The start failed; it is tried again by itself after a wait.
    Waiting,
    /// The file played to its end and nothing started after it.
    Ended,
}

/// What broke the stream of a load that is retried, and so what giving up means.
pub(super) enum Broke {
    /// It ended long before its length: given up, it counts as finished.
    CutShort,
    /// mpv said it failed, with mpv's words.
    Error(Option<String>),
    /// It waited for its cache too long.
    Stalled,
}

/// Everything that belongs to one attempt at one track, from the move that starts it until the next load replaces
/// it. A new load starts empty, so a pending seek, a pause or a count of retries never reaches another track.
pub(super) struct Load {
    pub(super) id: LoadId,
    pub(super) track: Track,
    pub(super) phase: Phase,
    /// Whether the person wants it paused. The owner mirrors it into `Output::pause` for whoever sends `pause`.
    pub(super) pause: bool,
    /// A position asked for that mpv has not confirmed: the start when the file is handed, sent to mpv at the first
    /// position in the file, or on its way to mpv while it plays. Until mpv's mark says it landed, the bar keeps it.
    pub(super) seek: Option<f64>,
    /// The mark of the latest seek sent to mpv for the file it plays; only its mark lands the seek.
    pub(super) seek_mark: Option<u64>,
    /// What a pending seek moved, put back if mpv refuses it.
    pub(super) before_seek: Option<Before>,
    /// Where its file is, as mpv last confirmed (a position it told, or a seek it took). Whether an end came early
    /// is judged from it; the bar, a retry, a park and a hand use the wanted position (`Owner::wanted_position`).
    pub(super) heard: f64,
    /// Where its file starts, or restarts for a retry.
    pub(super) start: f64,
    /// Picked by the person from a list (a play, a click in the queue), for the play history.
    pub(super) chosen: bool,
    pub(super) attempts: u8,
    /// Where the stream broke last; playing well past it forgets the attempts.
    pub(super) retry_point: Option<f64>,
    /// Why the fallback plays instead of the stream; told if the fallback fails too.
    pub(super) cause: Option<anyhow::Error>,
    pub(super) broke: Option<Broke>,
    /// Numbers its hands: only the latest hand job may hand its file, and only its failure counts.
    pub(super) hand: u64,
    /// What the latest hand loads, kept to hand it again to a new mpv when mpv dies under the hand.
    pub(super) url: Option<String>,
    /// The mark of the hand that went to mpv for it; mpv's `Loaded` with this mark says mpv took the file.
    pub(super) handing: Option<u64>,
    /// The file of it mpv took; `None` before mpv took it, while a retry fetches a new one, and after its end.
    pub(super) file: Option<u64>,
    /// mpv told a position in one of its files: a retry after that is no longer a start (no spinner).
    pub(super) played: bool,
    /// Its stream is known and described (`Owner::stream`), even if mpv does not have it yet.
    pub(super) described: bool,
    /// Numbers its waits for the cache: a stall watch acts only on the wait it was started for.
    pub(super) stall: u64,
    pub(super) preload: Preload,
    /// What the loudness meter read of it while it was measured.
    pub(super) measure: Measure,
    /// Whoever asked for this load and waits for it to start, fail or be replaced.
    pub(super) waiters: Vec<Reply>,
}

impl Load {
    /// The pending seek is settled (taken by a hand, or landed in mpv): it is no longer pending, and nothing of it is
    /// put back.
    fn settle_seek(&mut self) -> Option<f64> {
        self.seek_mark = None;
        self.before_seek = None;
        self.seek.take()
    }
}

/// What a seek moved before mpv took it.
pub(super) struct Before {
    retry_point: Option<f64>,
    heard: Heard,
}

/// How a move ends the play that was on, whether the person picked the new track, and how it starts.
#[derive(Clone, Copy)]
pub(super) struct Move {
    ended: Ended,
    chosen: bool,
    pause: Pause,
    start: Start,
}

/// An explicit move plays; an automatic one (a skip past a failure, a retry, the next track at the end of one) keeps
/// the pause the person wanted.
#[derive(Clone, Copy, PartialEq)]
enum Pause {
    Play,
    Keep,
}

#[derive(Clone, Copy)]
enum Start {
    Top,
    At(f64),
    /// Where the track was parked, if the parked position is for it.
    Resume,
}

impl Move {
    pub(super) fn new(ended: Ended, chosen: bool) -> Move {
        Move { ended, chosen, pause: Pause::Play, start: Start::Top }
    }

    pub(super) fn auto() -> Move {
        Move { ended: Ended::Replaced, chosen: false, pause: Pause::Keep, start: Start::Top }
    }

    pub(super) fn at(self, position: f64) -> Move {
        Move { start: Start::At(position), ..self }
    }

    pub(super) fn resume(self) -> Move {
        Move { start: Start::Resume, ..self }
    }
}

/// What the owner answers a job that is about to hand a file to mpv: where it starts, the mark mpv sends back once it
/// has taken it, and the gain as the loudness is now.
pub(super) struct Hand {
    start: f64,
    file: u64,
    gain: f64,
}

pub(super) struct Found {
    stream: Stream,
    local: bool,
    cause: Option<anyhow::Error>,
}

pub(super) fn cut_short(position: f64, duration: f64) -> bool {
    duration > 0.0 && position + EOF_SLACK < duration
}

pub(super) fn worth_a_fallback(code: Code) -> bool {
    !code.network() && !code.refused_for_good() && !matches!(code, Code::YtdlpMissing | Code::YtUnavailable)
}

#[derive(Debug, PartialEq)]
pub(super) enum AfterFailure {
    Skip,
    Wait(Duration),
    Stay,
}

/// Failures across loads: how many in a row, the automatic retries of the failing track, and the banner.
#[derive(Default)]
pub(super) struct Failures {
    pub(super) count: u8,
    pub(super) auto_retries: u8,
    pub(super) failed: Option<(TrackId, Code)>,
    pub(super) marks: BTreeMap<String, Code>,
    pub(super) stopped: bool,
}

impl Failures {
    pub(super) fn forget(&mut self) {
        self.count = 0;
        self.auto_retries = 0;
        self.failed = None;
        self.stopped = false;
    }

    /// Counts a failure of `id` and says what comes next; `more` is whether a track follows it.
    pub(super) fn note(&mut self, id: TrackId, e: &anyhow::Error, more: bool) -> AfterFailure {
        let code = Code::of(e);
        self.count = self.count.saturating_add(1);
        self.marks.insert(id.to_string(), code);
        self.failed = Some((id, code));
        if code.passes_by_itself() && self.auto_retries < AUTO_RETRIES {
            let wait = AUTO_RETRY_BASE * 2u32.pow(u32::from(self.auto_retries));
            self.auto_retries += 1;
            AfterFailure::Wait(Code::retry_after(e).map_or(wait, |after| wait.max(Duration::from_secs(after))))
        } else if code.network() || !more {
            AfterFailure::Stay
        } else if self.count < if code.refused_for_good() { MAX_REFUSED } else { MAX_FAILURES } {
            AfterFailure::Skip
        } else {
            self.stopped = true;
            AfterFailure::Stay
        }
    }

    pub(super) fn report(&self, retrying: bool) -> Option<PlayFailure> {
        self.failed.as_ref().map(|(id, code)| PlayFailure {
            id: id.clone(),
            code: *code,
            tracks: self.marks.clone(),
            stopped: self.stopped,
            retrying,
        })
    }
}

impl Owner {
    pub(super) fn phase(&self) -> Phase {
        self.load.as_ref().map_or(Phase::Idle, |l| l.phase)
    }

    /// A first start is under way: the spinner shows. A start retried before it told a position is still one.
    pub(super) fn starting(&self) -> bool {
        self.load.as_ref().is_some_and(|l| match l.phase {
            Phase::Resolving | Phase::Opening => true,
            Phase::Retrying => !l.played,
            _ => false,
        })
    }

    /// The current load's file is the one mpv plays, so what mpv says of its file is about this track.
    pub(super) fn in_mpv(&self) -> bool {
        self.load.as_ref().is_some_and(|l| l.file.is_some() && l.file == self.file)
    }

    pub(super) fn current_id(&self) -> Option<LoadId> {
        self.load.as_ref().map(|l| l.id)
    }

    pub(super) fn is_current(&self, id: LoadId) -> bool {
        self.current_id() == Some(id)
    }

    /// The one place the phase changes. Whoever waits on a load is answered as soon as it leaves `Resolving`, however
    /// it leaves (a failed start answers them first with its error), so no caller waits forever. `Idle` drops the
    /// load and clears a track queued in mpv after it; dropping it counts as a change of load, so a radio asked
    /// for the dropped one does not play.
    pub(super) fn enter(&mut self, phase: Phase) {
        let waiters = match phase {
            Phase::Idle => match self.load.take() {
                Some(mut load) => {
                    self.loads += 1;
                    self.forget_preload(&mut load);
                    remember_loudness(&self.shared, &load.track.id, &mut load.measure);
                    load.waiters
                }
                None => Vec::new(),
            },
            _ => match &mut self.load {
                Some(load) => {
                    load.phase = phase;
                    if phase == Phase::Resolving { Vec::new() } else { std::mem::take(&mut load.waiters) }
                }
                None => Vec::new(),
            },
        };
        for reply in waiters {
            self.later(reply, Ok(()));
        }
    }

    fn wanted_pause(&self) -> bool {
        self.shared.out.pause.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn mirror_pause(&self, pause: bool) {
        self.shared.out.pause.store(pause, std::sync::atomic::Ordering::SeqCst);
    }

    /// Starts the track at `index`: the play that was on ends, a new load replaces the old one, and its stream is
    /// resolved by a job.
    pub(super) fn begin(&mut self, index: usize, mv: Move, reply: Option<Reply>) {
        let Some(track) = self.queue.tracks.get(index).cloned() else {
            return self.answer(reply, Err(Code::ApiNotFound.err(format!("no queue item {index}"))));
        };
        if !self.shared.source.owns(&track.id) {
            let e = Code::SourceUnsupported.err(format!("`{}` is not a {} track", track.id, self.shared.source.name()));
            return self.answer(reply, Err(e));
        }
        let parked = self.parked.take();
        let start = match mv.start {
            Start::Top => 0.0,
            Start::At(position) => position,
            Start::Resume => parked.filter(|p| p.id == track.id && self.queue.cursor == Some(index)).map_or(0.0, |p| p.at),
        };
        let load = self.take_cursor(index, &track, start, mv, Phase::Resolving);
        if let (Some(reply), Some(current)) = (reply, &mut self.load) {
            current.waiters.push(reply);
        }
        self.announce(&track, start);
        let (shared, link) = (self.shared.clone(), self.link.clone());
        self.shared.tasks.spawn("resolve a stream", resolve(shared, link, load, track.id));
    }

    /// Puts a new load for the track at `index` in place of the current one.
    pub(super) fn take_cursor(&mut self, index: usize, track: &Track, start: f64, mv: Move, phase: Phase) -> LoadId {
        self.finish_listen(mv.ended);
        let pause = mv.pause == Pause::Keep && self.wanted_pause();
        self.enter(Phase::Idle);
        self.queue.cursor = Some(index);
        if self.failures.failed.as_ref().is_some_and(|(id, _)| *id != track.id) {
            self.failures.failed = None;
            self.failures.stopped = false;
            self.failures.auto_retries = 0;
            self.shared.emit(UiEvent::PlayFailure(self.failure()));
        }
        self.loads += 1;
        let id = LoadId(self.loads);
        self.load = Some(Load {
            id,
            track: track.clone(),
            phase,
            pause,
            seek: None,
            seek_mark: None,
            before_seek: None,
            heard: start,
            start,
            chosen: mv.chosen,
            attempts: 0,
            retry_point: None,
            cause: None,
            broke: None,
            hand: 0,
            url: None,
            handing: None,
            file: None,
            played: false,
            described: false,
            stall: 0,
            preload: Preload::Not,
            measure: Measure::default(),
            waiters: Vec::new(),
        });
        self.mirror_pause(pause);
        self.play.position = start;
        self.play.duration = track.duration;
        settings::POSITION.set(&self.shared.library, &start);
        self.publish_state();
        self.publish_queue();
        id
    }

    pub(super) fn announce(&self, track: &Track, start: f64) {
        self.shared.emit(UiEvent::Current(Some(track.clone())));
        self.shared.emit(UiEvent::Position(start));
        self.shared.emit(UiEvent::Liked(self.shared.library.is_liked(&track.id)));
    }

    pub(super) fn resolved(&mut self, id: LoadId, found: Result<Found>) {
        if !self.is_current(id) {
            return;
        }
        let found = match found {
            Ok(found) => found,
            Err(e) => return self.start_failed(e),
        };
        if let Some(load) = &mut self.load {
            load.cause = found.cause;
        }
        self.describe(found.stream.format.as_ref(), found.local);
        self.hand_soon(found.stream.url);
    }

    /// Starts a new hand of the current load's file; any hand before it no longer counts.
    fn hand_soon(&mut self, url: String) {
        let Some(load) = &mut self.load else { return };
        load.hand += 1;
        load.handing = None;
        load.url = Some(url.clone());
        let (shared, link, id, number) = (self.shared.clone(), self.link.clone(), load.id, load.hand);
        self.shared.tasks.spawn("hand a file to mpv", hand(shared, link, id, number, url));
    }

    /// A job holds the turn and is about to hand the load's file to mpv: it may, if it is the load's latest hand and
    /// the load has no file in mpv yet. A seek asked for meanwhile becomes the start; the gain is the one for the
    /// loudness as it is now.
    pub(super) fn answer_hand(&mut self, id: LoadId, hand: u64, reply: oneshot::Sender<Option<Hand>>) {
        let file = self.files + 1;
        let gain = self.gain();
        let answer = match &mut self.load {
            Some(load)
                if load.id == id
                    && load.hand == hand
                    && load.handing.is_none()
                    && load.file.is_none()
                    && matches!(load.phase, Phase::Resolving | Phase::Retrying) =>
            {
                let start = load.settle_seek().unwrap_or(load.start);
                load.start = start;
                load.heard = start;
                load.handing = Some(file);
                self.files = file;
                Some(Hand { start, file, gain })
            }
            _ => None,
        };
        self.later(reply, answer);
    }

    /// mpv took the file with this mark (its answer to the load, in order with what it says of its files). From here
    /// on, what mpv says is about this file; if it is the current load's, the load is in mpv: a start opens (its
    /// waiters are answered), a retry waits for its first position.
    pub(super) fn took(&mut self, file: u64) {
        self.file = Some(file);
        self.heard_duration = None;
        self.mpv.cache = false;
        let Some(load) = self.load.as_mut().filter(|l| l.handing == Some(file)) else { return };
        load.handing = None;
        load.file = Some(file);
        if load.phase != Phase::Resolving {
            return;
        }
        load.attempts = if load.cause.is_some() { 2 } else { 0 };
        load.retry_point = load.cause.is_some().then_some(load.seek.unwrap_or(load.start));
        self.enter(Phase::Opening);
        self.started();
    }

    /// The latest hand of the current load failed. If mpv went away under it, its end is on its way and the file is
    /// handed again to a new mpv then (`died`); otherwise the start fails, or the retry gives up.
    pub(super) fn hand_failed(&mut self, id: LoadId, hand: u64, error: anyhow::Error, gone: bool) {
        let Some(load) = self.load.as_mut().filter(|l| l.id == id && l.hand == hand) else { return };
        load.handing = None;
        if gone {
            load.hand += 1;
            return;
        }
        match load.phase {
            Phase::Resolving => self.start_failed(error),
            Phase::Retrying => self.gave_up(Err(error)),
            _ => {}
        }
    }

    /// Where the person wants the current load to be: a seek not yet landed, else the clock. The bar shows it (a seek
    /// moves `play.position` at once, and the clock does not move the bar until the seek landed), and a retry, a park
    /// and a hand start there. Whether an end came early is judged from where mpv is instead (`Load::heard`).
    fn wanted_position(&self) -> f64 {
        self.load.as_ref().and_then(|l| l.seek).unwrap_or(self.play.position)
    }

    /// Keeps the current track's wanted position for the next time it is played from idle.
    fn park(&mut self) {
        let Some(id) = self.queue.current().map(|t| t.id.clone()) else { return };
        let at = self.wanted_position();
        self.parked = (at > 0.0).then_some(Parked { id, at });
    }

    /// The start of the current load failed: the error goes to whoever waits on it, and the failure decides whether
    /// the queue skips on, waits to try again, or stays.
    pub(super) fn start_failed(&mut self, e: anyhow::Error) {
        let Some(load) = &mut self.load else { return };
        let waiters = std::mem::take(&mut load.waiters);
        let id = load.id;
        self.mpv.cache = false;
        self.park();
        let first = self.failures.auto_retries == 0;
        self.finish_listen(Ended::Failed);
        let next = self.note_failure(&e);
        self.enter(if matches!(next, AfterFailure::Wait(_)) { Phase::Waiting } else { Phase::Idle });
        self.publish_state();
        self.stop_unless_skipping(&next);
        self.shared.emit(UiEvent::PlayFailure(self.failure()));
        if first {
            self.shared.emit(UiEvent::Toast(Toast::failed(Code::PlayFailed, &e)));
        }
        let copies: Vec<anyhow::Error> = waiters.iter().skip(1).map(|_| Code::of(&e).err(format!("{e:#}")).context(Told)).collect();
        for (reply, e) in waiters.into_iter().zip(std::iter::once(e.context(Told)).chain(copies)) {
            self.later(reply, Err(e));
        }
        self.after_failure(next, id);
    }

    /// The current track failed after it had started.
    pub(super) fn failed(&mut self, e: anyhow::Error) {
        let Some(id) = self.current_id() else { return };
        self.mpv.cache = false;
        self.finish_listen(Ended::Failed);
        self.park();
        let next = self.note_failure(&e);
        self.enter(if matches!(next, AfterFailure::Wait(_)) { Phase::Waiting } else { Phase::Idle });
        self.publish_state();
        self.stop_unless_skipping(&next);
        self.shared.emit(UiEvent::PlayFailure(self.failure()));
        self.shared.emit(UiEvent::Toast(Toast::failed(Code::PlayFailed, &e)));
        self.after_failure(next, id);
    }

    /// After a failure that does not move on, nothing of an older track plays on under the failed one (a start that
    /// failed leaves the track before it in mpv).
    fn stop_unless_skipping(&self, next: &AfterFailure) {
        if !matches!(next, AfterFailure::Skip) {
            self.stop_audio(None);
        }
    }

    fn note_failure(&mut self, e: &anyhow::Error) -> AfterFailure {
        let code = Code::of(e);
        let Some(id) = self.queue.current().map(|t| t.id.clone()) else { return AfterFailure::Stay };
        if code.refused_for_good() {
            self.shared.library.mark_unplayable(&id, code);
        }
        let more = self.queue.next_index().is_some();
        self.failures.note(id, e, more)
    }

    fn after_failure(&mut self, next: AfterFailure, id: LoadId) {
        match next {
            AfterFailure::Skip => self.advance(true, Move::auto(), None),
            AfterFailure::Wait(wait) => {
                let link = self.link.clone();
                self.shared.tasks.spawn("retry a failed start", async move {
                    tokio::time::sleep(wait).await;
                    link.send(Msg::RetryDue(id));
                });
            }
            AfterFailure::Stay => {}
        }
    }

    /// The wait after a failed start is over: the track at the cursor is tried again, wherever the queue moved it,
    /// from where it was.
    pub(super) fn retry_due(&mut self, id: LoadId) {
        if !self.is_current(id) || self.phase() != Phase::Waiting {
            return;
        }
        if let Some(cursor) = self.queue.cursor {
            self.begin(cursor, Move::auto().resume(), None);
        }
    }

    pub(super) fn failure(&self) -> Option<PlayFailure> {
        self.failures.report(self.phase() == Phase::Waiting)
    }

    /// mpv ended a file. `file` is the file mpv played as far as the owner knows; an end of any file but the current
    /// load's (an older one, or the one a retry is replacing) is not about the current track.
    pub(super) fn ended(&mut self, file: Option<u64>, reason: &str, error: Option<String>) {
        if file.is_none() || file != self.file || !self.in_mpv() {
            return;
        }
        let heard = self.load.as_ref().map_or(0.0, |l| l.heard);
        match reason {
            "eof" if !self.stream.local && cut_short(heard, self.play.duration) => self.retry(Broke::CutShort),
            "eof" => self.finished(),
            "error" => self.retry(Broke::Error(error)),
            _ => {}
        }
    }

    /// The current track played to its end: the track queued in mpv takes over, or the queue moves on.
    fn finished(&mut self) {
        if let Some(load) = &mut self.load {
            remember_loudness(&self.shared, &load.track.id, &mut load.measure);
        }
        self.finish_listen(Ended::Finished);
        if let Some(load) = &mut self.load {
            load.file = None;
        }
        self.enter(Phase::Ended);
        match self.hand_over() {
            Some(regain) => {
                let gain = regain.then(|| self.gain());
                self.shared.tasks.spawn("reassert the effects", super::gapless::after_hand_over(self.shared.clone(), gain));
            }
            None => self.advance(true, Move::auto(), None),
        }
    }

    /// The stream broke: the first retry asks for a fresh stream, the second for the fallback, then it gives up. A
    /// track queued in mpv after it is taken out, or mpv would play it in place of the retry.
    pub(super) fn retry(&mut self, broke: Broke) {
        let position = self.wanted_position();
        let owned = self.queue.current().map(|t| t.id.clone()).filter(|id| self.shared.source.owns(id));
        let Some(load) = &mut self.load else { return };
        load.attempts += 1;
        load.retry_point = Some(position);
        load.start = position;
        load.heard = position;
        load.seek_mark = None;
        load.before_seek = None;
        let (id, attempt) = (load.id, load.attempts);
        let Some(track) = owned.filter(|_| attempt <= 2) else { return self.give_up(broke, Ok(())) };
        load.file = None;
        load.handing = None;
        load.url = None;
        load.hand += 1;
        load.broke = Some(broke);
        self.mpv.cache = false;
        self.enter(Phase::Retrying);
        self.forget_current_preload();
        self.publish_state();
        let (shared, link) = (self.shared.clone(), self.link.clone());
        self.shared.tasks.spawn("fetch the stream again", async move {
            let found = async {
                shared.audio().await?;
                let found = match attempt {
                    1 => Some(shared.source.stream(&track, true).await?),
                    _ => shared.source.fallback(&track).map(Stream::url),
                };
                Ok(found.map(|stream| shared.remembered(&track, stream)))
            }
            .await;
            link.send(Msg::Refetched { load: id, found });
        });
    }

    pub(super) fn refetched(&mut self, id: LoadId, found: Result<Option<Stream>>) {
        if !self.is_current(id) || self.phase() != Phase::Retrying {
            return;
        }
        match found {
            Err(e) => self.gave_up(Err(e)),
            Ok(None) => self.gave_up(Ok(())),
            Ok(Some(stream)) => {
                self.forget_current_preload();
                if let Some(load) = &mut self.load {
                    load.preload = Preload::Tried;
                }
                self.describe(stream.format.as_ref(), false);
                self.hand_soon(stream.url);
            }
        }
    }

    /// A retry in progress ran out of streams (`Ok`) or failed.
    fn gave_up(&mut self, result: Result<()>) {
        let broke = self.load.as_mut().and_then(|l| l.broke.take()).unwrap_or(Broke::Error(None));
        self.give_up(broke, result);
    }

    fn give_up(&mut self, broke: Broke, result: Result<()>) {
        match (broke, result) {
            (Broke::CutShort, _) => self.finished(),
            (Broke::Error(said), Ok(())) => {
                let cause = self.load.as_mut().and_then(|l| l.cause.take());
                self.failed(cause.unwrap_or_else(|| Code::AudioFailed.err(said.unwrap_or_else(|| "playback failed".into()))));
            }
            (Broke::Stalled, Ok(())) => self.failed(Code::NetTimeout.err("the stream stalled")),
            (_, Err(e)) => self.failed(e),
        }
    }

    /// mpv waits for its cache in the current load's file: if the same wait still goes on after a while, the stream
    /// is fetched again. What counts is that mpv has this load's file, whatever the phase (a retried file that stalls
    /// before its first position is watched too); a wait in an older file, or while a retry fetches, is not.
    pub(super) fn stalled(&mut self) {
        if !self.in_mpv() {
            return;
        }
        let Some(load) = self.load.as_mut() else { return };
        load.stall += 1;
        let (id, stall) = (load.id, load.stall);
        let link = self.link.clone();
        self.shared.tasks.spawn("stall watch", async move {
            tokio::time::sleep(STALL_AFTER).await;
            link.send(Msg::StallExpired { load: id, stall });
        });
    }

    pub(super) fn stall_expired(&mut self, id: LoadId, stall: u64) {
        let Some(load) = &self.load else { return };
        if load.id != id || load.stall != stall || !self.in_mpv() || !self.mpv.cache {
            return;
        }
        self.retry(Broke::Stalled);
    }

    /// mpv is gone. A start or a retry whose file mpv had not taken goes on: a file on its way is handed again (to a
    /// new mpv), one still resolving is handed when it is ready, and nothing is told, since nothing heard stops. A
    /// wait to try again goes on too. A file mpv had is lost, and the track is parked where it was; after the end of
    /// the queue there is nothing to keep.
    pub(super) fn died(&mut self) {
        let mut heard = self.play.playing || self.starting();
        self.mpv.idle = true;
        self.mpv.cache = false;
        self.mpv.seeking = false;
        self.file = None;
        match self.load.as_ref().map(|l| (l.phase, l.file.is_some())) {
            Some((Phase::Resolving | Phase::Retrying, false)) => {
                heard = false;
                self.hand_again();
            }
            Some((Phase::Waiting, _)) => {}
            Some((Phase::Opening | Phase::Playing | Phase::Retrying, _)) => {
                self.park();
                self.enter(Phase::Idle);
            }
            Some((Phase::Ended | Phase::Idle | Phase::Resolving, _)) => self.enter(Phase::Idle),
            None => {}
        }
        self.publish_state();
        if heard {
            self.shared.emit(UiEvent::Toast(Toast::new(Code::AudioRestart)));
        }
    }

    /// Hands the current load's file again after mpv died under its hand. A load still resolving its stream has no
    /// file yet; its job hands it when it is ready.
    fn hand_again(&mut self) {
        if let Some(url) = self.load.as_ref().and_then(|l| l.url.clone()) {
            self.hand_soon(url);
        }
    }

    /// While a track starts or is fetched again, a toggle flips the pause the load wants (its hand sends it); while
    /// it plays, it flips mpv's. With nothing of the track in mpv (idle, a failure waiting to try again, the end of
    /// the queue) it plays the track at the cursor, from where it was parked.
    pub(super) fn toggle(&mut self, reply: Reply) {
        match self.phase() {
            Phase::Resolving | Phase::Opening | Phase::Retrying => {
                let pause = self.load.as_ref().is_some_and(|l| !l.pause);
                self.want_pause(pause, reply)
            }
            Phase::Playing if !self.mpv.idle => self.want_pause(!self.mpv.paused, reply),
            _ => match self.queue.cursor {
                Some(cursor) => self.begin(cursor, Move::new(Ended::Replaced, false).resume(), Some(reply)),
                None => self.want_pause(!self.mpv.paused, reply),
            },
        }
    }

    fn want_pause(&mut self, pause: bool, reply: Reply) {
        if let Some(load) = &mut self.load {
            load.pause = pause;
        }
        self.mirror_pause(pause);
        let (shared, link) = (self.shared.clone(), self.link.clone());
        self.shared.tasks.spawn("pause", async move {
            let sent = async {
                let audio = shared.audio().await?;
                shared.out.send_pause(&*audio).await
            }
            .await;
            link.send(Msg::Answer(reply, sent));
        });
    }

    /// A seek is the load's until mpv confirms it, and the bar shows it at once: while the track plays it goes to mpv
    /// at once; while the track starts or is fetched again it waits (the hand takes it as the start, or it is sent at
    /// the first position). A retry counts its attempts from where play resumes, so the retry point moves with it.
    /// With nothing of the track in mpv it parks the track there.
    pub(super) fn seek(&mut self, position: f64, reply: Reply) {
        match self.phase() {
            Phase::Resolving | Phase::Opening | Phase::Retrying | Phase::Playing => {
                let playing = self.phase() == Phase::Playing && self.in_mpv();
                let heard = self.shared.listening.heard();
                if let Some(load) = &mut self.load {
                    load.before_seek.get_or_insert(Before { retry_point: load.retry_point, heard });
                    load.seek = Some(position);
                    if let Some(point) = &mut load.retry_point {
                        *point = position;
                    }
                }
                self.seeked(position);
                if playing { self.seek_in_mpv(position, Some(reply)) } else { self.later(reply, Ok(())) }
            }
            Phase::Idle | Phase::Waiting | Phase::Ended => {
                if let Some(id) = self.queue.current().map(|t| t.id.clone()) {
                    self.parked = Some(Parked { id, at: position });
                    self.seeked(position);
                }
                self.later(reply, Ok(()));
            }
        }
    }

    /// Seeks in the current load's file with a fresh mark. The job checks under the turn that this is still the
    /// latest seek and the file still the one mpv plays, so a seek meant for one file never lands in another and an
    /// older seek never lands after a newer one; it always reports back. That mpv took it comes in mpv's own order,
    /// as `Event::Seeked` with the mark, after everything mpv said from before it.
    pub(super) fn seek_in_mpv(&mut self, position: f64, reply: Option<Reply>) {
        let mark = self.marks + 1;
        let in_mpv = self.in_mpv();
        let Some(load) = self.load.as_mut().filter(|_| in_mpv) else { return self.answer(reply, Ok(())) };
        self.marks = mark;
        load.seek_mark = Some(mark);
        let (shared, link, load) = (self.shared.clone(), self.link.clone(), load.id);
        self.shared.tasks.spawn("seek", async move {
            let mut used = None;
            let result = async {
                let audio = shared.audio().await?;
                used = Some(audio.clone());
                let mut turn = shared.out.turn.lock().await;
                if link.ask(|reply| Msg::LatestSeek { load, mark, reply }).await != Some(true) {
                    return Ok(());
                }
                shared.out.send_fx(&mut turn, &*audio, fx::Mode::Settle).await?;
                audio.seek(position, mark).await
            }
            .await;
            let gone = used.is_some_and(|audio: Arc<dyn crate::audio::AudioOut>| audio.gone());
            link.send(Msg::SeekSent { load, mark, result, gone, reply });
        });
    }

    pub(super) fn latest_seek(&self, id: LoadId, mark: u64) -> bool {
        self.is_current(id) && self.in_mpv() && self.load.as_ref().is_some_and(|l| l.seek_mark == Some(mark))
    }

    /// mpv took the seek with this mark. If it is the latest one for the file mpv plays, the seek has landed: the file
    /// is there, and what mpv tells from now on is taken. An older seek's mark is overtaken by the newer one.
    pub(super) fn landed(&mut self, mark: u64) {
        if !self.in_mpv() {
            return;
        }
        let Some(load) = self.load.as_mut().filter(|l| l.seek_mark == Some(mark)) else { return };
        if let Some(position) = load.settle_seek() {
            load.heard = position;
        }
    }

    /// A seek job came back. Only the latest seek, refused by an mpv that still plays the load's file, is undone: the
    /// bar, the retry point and the play go back to where they were. A seek that mpv died under, that went to a file
    /// that is gone, or that a newer one overtook stays the load's wanted position (the next hand or park takes it),
    /// and its caller hears it went fine.
    pub(super) fn seek_sent(&mut self, id: LoadId, mark: u64, result: Result<()>, gone: bool, reply: Option<Reply>) {
        let answer = match result {
            Err(e) if !gone && self.latest_seek(id, mark) => {
                self.seek_refused();
                Err(e)
            }
            _ => Ok(()),
        };
        self.answer(reply, answer);
    }

    fn seek_refused(&mut self) {
        let Some(load) = &mut self.load else { return };
        let before = load.before_seek.take();
        load.seek = None;
        load.seek_mark = None;
        let heard = load.heard;
        if let Some(before) = before {
            load.retry_point = before.retry_point;
            self.shared.listening.restore(before.heard);
        }
        self.seeked(heard);
    }

    fn seeked(&mut self, position: f64) {
        self.play.position = position;
        self.shared.listening.jumped(position);
        self.shared.emit(UiEvent::Position(position));
        self.shared.mpris.send(crate::mpris::Update::Seeked(position)).ok();
        self.shared.discord.send(crate::discord::Msg::Seeked(position)).ok();
    }

    pub(super) fn cancel_start(&mut self, reply: Reply) {
        if !self.starting() && self.phase() != Phase::Waiting {
            return self.later(reply, Ok(()));
        }
        self.failures.auto_retries = 0;
        self.failures.failed = None;
        self.enter(Phase::Idle);
        self.publish_state();
        self.shared.emit(UiEvent::PlayFailure(None));
        self.stop_audio(Some(reply));
    }

    /// Stops everything: no load, no failures, an empty clock; mpv stops. It counts as a change of load even when
    /// none ran, so a radio asked before it does not play.
    pub(super) fn halt(&mut self, reply: Option<Reply>) {
        if self.load.is_none() {
            self.loads += 1;
        }
        self.enter(Phase::Idle);
        self.parked = None;
        self.failures.forget();
        self.failures.marks.clear();
        self.shared.emit(UiEvent::PlayFailure(None));
        self.finish_listen(Ended::Halted);
        self.play.position = 0.0;
        self.play.duration = 0.0;
        self.stream = Default::default();
        self.publish_state();
        self.publish_queue();
        self.shared.emit(UiEvent::Stream(Default::default()));
        self.shared.emit(UiEvent::Position(0.0));
        self.shared.emit(UiEvent::Current(None));
        self.stop_audio(reply);
    }

    /// Stops mpv, under the turn and only if no load started since, so a stop never lands after a newer file.
    fn stop_audio(&self, reply: Option<Reply>) {
        let (shared, link, load) = (self.shared.clone(), self.link.clone(), self.current_id());
        self.shared.tasks.spawn("stop mpv", async move {
            if let Some(audio) = shared.out.current().await {
                let _turn = shared.out.turn.lock().await;
                if link.ask(|reply| Msg::Current { load, reply }).await == Some(true) {
                    audio.stop().await.ok();
                }
            }
            if let Some(reply) = reply {
                link.send(Msg::Answer(reply, Ok(())));
            }
        });
    }
}

async fn resolve(shared: Arc<Shared>, link: Link, load: LoadId, id: TrackId) {
    let found = match shared.open(&id, false).await {
        Ok((stream, local)) => Ok(Found { stream, local, cause: None }),
        Err(e) => match worth_a_fallback(Code::of(&e)).then(|| shared.source.fallback(&id)).flatten() {
            Some(url) => Ok(Found { stream: shared.remembered(&id, Stream::url(url)), local: false, cause: Some(e) }),
            None => Err(e),
        },
    };
    link.send(Msg::Resolved { load, found });
}

/// Hands a file to mpv: the effects for it, the pause the person wants, then, holding the turn so no other file or
/// seek goes between, the owner's go-ahead and the load itself. Success is not reported: mpv's mark for the file
/// tells the owner, in order with what mpv says of its files. A failure is, with whether mpv went away under it.
async fn hand(shared: Arc<Shared>, link: Link, load: LoadId, hand: u64, url: String) {
    let mut used = None;
    let handed = async {
        let audio = shared.audio().await?;
        used = Some(audio.clone());
        shared.out.apply_fx(&*audio, fx::Mode::Live).await?;
        shared.out.send_pause(&*audio).await.ok();
        let mut turn = shared.out.turn.lock().await;
        let Some(Hand { start, file, gain }) = link.ask(|reply| Msg::Hand { load, hand, reply }).await.flatten() else { return Ok(()) };
        shared.out.send_fx(&mut turn, &*audio, fx::Mode::Settle).await?;
        audio.load(&url, start, Some(gain), file).await
    }
    .await;
    if let Err(error) = handed {
        let gone = used.is_some_and(|audio: Arc<dyn crate::audio::AudioOut>| audio.gone());
        link.send(Msg::HandFailed { load, hand, error, gone });
    }
}
