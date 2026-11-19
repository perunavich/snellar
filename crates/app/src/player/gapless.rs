//! Gapless: near the end of a track the next one is appended to mpv's list, and at the end it takes over without a
//! load. Edits of mpv's list go to mpv one at a time, in the order the owner made them (`Edit`).

use std::sync::{Arc, Weak};

use anyhow::Result;
use tokio::sync::mpsc::UnboundedReceiver;

use snellar_core::ids::TrackId;
use snellar_core::model::{AudioFormat, Track};
use snellar_core::source::Stream;

use super::load::{Load, LoadId, Move, Phase};
use super::{Msg, Owner, PRELOAD_LEAD, Shared};
use crate::fx;

/// The next track as far as the current load has queued it in mpv.
pub(super) enum Preload {
    /// Not asked for yet.
    Not,
    /// Asked for: the stream of `id` is being resolved. Only this answer is taken, and only while the load plays.
    Resolving { id: TrackId },
    /// Refused, failed or dropped: not asked for again by this load.
    Tried,
    /// Sent to mpv with the mark `file`; mpv's `Appended` with that mark, in order with its ends, says it is queued.
    /// `stale` once the loudness changed: it carries the gain it was sent with, so it is taken out once queued.
    Appending { file: u64, next: Next, stale: bool },
    /// In mpv's list after the current file, as the file `file`.
    Queued { file: u64, next: Next },
    /// Queued with a gain the loudness no longer wants, and taken out by the clear with the mark `clear`. Until mpv's
    /// `Cleared` with that mark, mpv may have moved on to it already: an end before it still hands over to it.
    Clearing { file: u64, next: Next, clear: u64 },
}

/// A next track sent to mpv.
pub(super) struct Next {
    id: TrackId,
    format: Option<AudioFormat>,
    local: bool,
}

/// An edit of mpv's list after the file it plays.
pub(super) enum Edit {
    Append { url: String, gain: f64, file: u64 },
    Clear { mark: u64 },
}

/// Sends the owner's edits of mpv's list one at a time, in the order it made them, so a clear never lands after a
/// newer append, nor an append after a newer clear. With mpv gone there is no list, and no reason to start mpv for it.
pub(super) async fn edit_list(me: Weak<Shared>, mut edits: UnboundedReceiver<Edit>) {
    while let Some(edit) = edits.recv().await {
        let Some(shared) = me.upgrade() else { return };
        let Some(audio) = shared.out.current().await else { continue };
        match edit {
            Edit::Append { url, gain, file } => audio.append(&url, Some(gain), file).await.ok(),
            Edit::Clear { mark } => audio.clear_next(mark).await.ok(),
        };
    }
}

impl Owner {
    /// Near the end of the current track while it plays, resolves the next one and appends it to mpv's list.
    pub(super) fn preload_soon(&mut self, position: f64) {
        let duration = self.play.duration;
        let wanted = self.phase() == Phase::Playing && self.in_mpv() && duration > 0.0 && position >= duration - PRELOAD_LEAD;
        if !wanted || !self.load.as_ref().is_some_and(|l| matches!(l.preload, Preload::Not)) {
            return;
        }
        let Some(id) = self.following_id().cloned() else { return };
        let Some(load) = &mut self.load else { return };
        load.preload = Preload::Resolving { id: id.clone() };
        let (shared, link, load) = (self.shared.clone(), self.link.clone(), load.id);
        self.shared.tasks.spawn("preload the next track", async move {
            let found = async {
                let opened = shared.open(&id, false).await?;
                shared.audio().await?;
                Ok(opened)
            }
            .await;
            link.send(Msg::Preloaded { load, id, found });
        });
    }

    /// The next track's stream is known, or could not be. It is appended only if the load still waits for it (a
    /// retry or a change to the queue forgot it) and its file plays in mpv. A track that needs measuring after one
    /// that does not (or the other way round) is loaded at the end instead: the loudness stage would change under the
    /// current track's tail.
    pub(super) fn preloaded(&mut self, load: LoadId, id: TrackId, found: Result<(Stream, bool)>) {
        let asked =
            self.load.as_ref().is_some_and(|l| l.id == load && matches!(&l.preload, Preload::Resolving { id: asked } if *asked == id));
        if !asked {
            return;
        }
        let fits = |owner: &Owner, gain: Option<f64>| {
            owner.phase() == Phase::Playing
                && owner.in_mpv()
                && owner.following_id() == Some(&id)
                && (owner.loudness && gain.is_none()) == owner.measuring
        };
        let file = self.files + 1;
        let (stream, local) = match found {
            Ok((stream, local)) if fits(self, stream.format.as_ref().and_then(|f| f.loudness_db)) => (stream, local),
            _ => {
                if let Some(current) = &mut self.load {
                    current.preload = Preload::Tried;
                }
                return;
            }
        };
        let gain = stream.format.as_ref().and_then(|f| f.loudness_db).map(fx::gain).filter(|_| self.loudness).unwrap_or(0.0);
        let Some(current) = &mut self.load else { return };
        current.preload = Preload::Appending { file, next: Next { id, format: stream.format, local }, stale: false };
        self.files = file;
        self.edits.send(Edit::Append { url: stream.url, gain, file }).ok();
    }

    /// mpv queued an appended file. If the current load is waiting for it, it is queued (and taken out again if the
    /// loudness changed meanwhile). A file a clear sent after it covers is gone already; any other was for a load that
    /// is gone, and mpv's list after the current file is cleared.
    pub(super) fn appended(&mut self, file: u64) {
        if let Some(load) = &mut self.load
            && matches!(&load.preload, Preload::Appending { file: f, .. } if *f == file)
            && let Preload::Appending { file, next, stale } = std::mem::replace(&mut load.preload, Preload::Tried)
        {
            load.preload = Preload::Queued { file, next };
            if stale {
                self.take_out_queued();
            }
            return;
        }
        if file > self.cleared {
            self.clear_next();
        }
    }

    /// mpv took the clear with this mark. If it took out a next track queued with an old gain, nothing follows the
    /// current file now: the next track is asked for again, with the gain as it is now.
    pub(super) fn list_cleared(&mut self, mark: u64) {
        let Some(load) = &mut self.load else { return };
        if !matches!(load.preload, Preload::Clearing { clear, .. } if clear == mark) {
            return;
        }
        load.preload = Preload::Not;
        self.preload_soon(self.play.position);
    }

    /// After a change to the queue or the modes: a next track that no longer comes next is forgotten (and taken out of
    /// mpv), and the one that comes next now is asked for.
    pub(super) fn recheck_preload(&mut self) {
        let following = self.following_id().cloned();
        let Some(mut load) = self.load.take() else { return };
        let keeps = match &load.preload {
            Preload::Queued { next, .. } | Preload::Appending { next, .. } | Preload::Clearing { next, .. } => {
                following.as_ref() == Some(&next.id)
            }
            Preload::Resolving { id } => following.as_ref() == Some(id),
            Preload::Not | Preload::Tried => true,
        };
        if !keeps {
            self.forget_preload(&mut load);
            load.preload = Preload::Not;
        }
        self.load = Some(load);
        if !keeps {
            self.preload_soon(self.play.position);
        }
    }

    pub(super) fn forget_preload(&mut self, load: &mut Load) {
        match load.preload {
            Preload::Queued { .. } | Preload::Appending { .. } => {
                load.preload = Preload::Tried;
                self.clear_next();
            }
            Preload::Resolving { .. } | Preload::Clearing { .. } => load.preload = Preload::Tried,
            Preload::Not | Preload::Tried => {}
        }
    }

    /// The loudness changed. A next track sent to mpv carries the gain it was sent with: once mpv has it queued it is
    /// taken out and asked for again (`list_cleared`). One still resolving takes the gain when it is appended, and one
    /// refused may fit now.
    pub(super) fn preload_for_loudness(&mut self) {
        let Some(load) = &mut self.load else { return };
        match &mut load.preload {
            Preload::Tried => load.preload = Preload::Not,
            Preload::Appending { stale, .. } => *stale = true,
            Preload::Queued { .. } => self.take_out_queued(),
            Preload::Not | Preload::Resolving { .. } | Preload::Clearing { .. } => {}
        }
        self.preload_soon(self.play.position);
    }

    fn take_out_queued(&mut self) {
        let clear = self.clear_next();
        if let Some(load) = &mut self.load
            && let Preload::Queued { file, next } = std::mem::replace(&mut load.preload, Preload::Tried)
        {
            load.preload = Preload::Clearing { file, next, clear };
        }
    }

    pub(super) fn forget_current_preload(&mut self) {
        if let Some(mut load) = self.load.take() {
            self.forget_preload(&mut load);
            self.load = Some(load);
        }
    }

    /// Takes whatever follows the current file out of mpv's list, after every append sent before it.
    fn clear_next(&mut self) -> u64 {
        self.marks += 1;
        self.cleared = self.files;
        self.edits.send(Edit::Clear { mark: self.marks }).ok();
        self.marks
    }

    /// At the end of the current track, the track mpv already moved on to becomes the current one: one queued, or one
    /// being taken out whose clear mpv had not taken yet. One queued with an old gain gets the gain as it is now
    /// (`Some(true)`). With nothing queued, a next track on its way to mpv is taken out.
    pub(super) fn hand_over(&mut self) -> Option<bool> {
        let following = self.following().map(|i| (i, self.queue.tracks[i].clone()));
        let load = self.load.as_mut()?;
        let (file, next, regain) = match std::mem::replace(&mut load.preload, Preload::Tried) {
            Preload::Queued { file, next } => (file, next, false),
            Preload::Clearing { file, next, .. } => (file, next, true),
            other => {
                load.preload = other;
                self.forget_current_preload();
                return None;
            }
        };
        let (index, track) = following.filter(|(_, t): &(usize, Track)| t.id == next.id)?;
        self.take_cursor(index, &track, 0.0, Move::auto(), Phase::Opening);
        self.file = Some(file);
        if let Some(load) = &mut self.load {
            load.file = Some(file);
        }
        self.describe(next.format.as_ref(), next.local);
        self.announce(&track, 0.0);
        self.started();
        Some(regain)
    }
}

/// The gain the track that took over should play with, sent with the effects reasserted for it.
pub(super) async fn after_hand_over(shared: Arc<Shared>, gain: Option<f64>) {
    let Ok(audio) = shared.audio().await else { return };
    let mut turn = shared.out.turn.lock().await;
    if let Some(gain) = gain {
        audio.set("replaygain-fallback", serde_json::json!(gain)).await.ok();
    }
    shared.out.send_fx(&mut turn, &*audio, fx::Mode::Reassert).await.ok();
}
