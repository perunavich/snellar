//! Listening and history: the open play of the current track, the time really heard, and what a started track
//! writes to the history.

use std::sync::Mutex;

use snellar_core::ids::TrackId;
use snellar_core::library::Library;
use snellar_core::library::signals::Started;
use snellar_core::model::Track;
use snellar_core::signals::{Ended, Heard};
use snellar_core::sync::Lock;
use snellar_core::ui::UiEvent;

use super::{Owner, PREFETCH_AHEAD};

/// The play of the current track that is not written to the history yet. The owner task opens, ticks and ends it;
/// quitting ends it at once and removing a track from the history drops it, both from outside the task, so it sits
/// behind its own lock, which is never held across an await.
#[derive(Default)]
pub(super) struct Listening(Mutex<Open>);

#[derive(Default)]
struct Open {
    id: Option<TrackId>,
    row: Option<i64>,
    heard: Heard,
}

impl Listening {
    pub(super) fn start(&self, library: &Library, track: &Track, position: f64, chosen: bool) {
        let mut open = self.0.locked();
        open.id = Some(track.id.clone());
        open.heard = Heard::from(position);
        open.row = library.start_play(&Started { track, start: position, chosen });
    }

    pub(super) fn tick(&self, position: f64, presence: snellar_core::signals::Presence) {
        self.0.locked().heard.tick(position, presence);
    }

    pub(super) fn jumped(&self, position: f64) {
        self.0.locked().heard.jumped(position);
    }

    pub(super) fn finish(&self, library: &Library, duration: f64, ended: Ended) {
        let mut open = self.0.locked();
        if let Some(id) = open.id.take() {
            let row = open.row.take();
            library.end_play(row, &id, &open.heard, duration, ended);
        }
    }

    /// Drops the open play if it is of `id`, so its end is never written; the track plays on.
    pub(super) fn forget(&self, id: &TrackId) {
        let mut open = self.0.locked();
        if open.id.as_ref() == Some(id) {
            open.id = None;
            open.row = None;
        }
    }

    pub(super) fn heard(&self) -> Heard {
        self.0.locked().heard.clone()
    }

    /// Puts back what was heard before a jump that did not happen.
    pub(super) fn restore(&self, heard: Heard) {
        self.0.locked().heard = heard;
    }
}

impl Owner {
    pub(super) fn finish_listen(&self, ended: Ended) {
        self.shared.listening.finish(&self.shared.library, self.play.duration, ended);
    }

    /// The current load's file is in mpv: the history and the recommendations learn of it, its failures are
    /// forgotten, its play opens, and the next tracks are warmed up.
    pub(super) fn started(&mut self) {
        let Some(load) = &self.load else { return };
        let (track, chosen) = (load.track.clone(), load.chosen);
        let card = self.origin.as_ref().filter(|o| o.tracks.contains(&track.id)).map(|o| &o.card);
        self.shared.library.push_played(&track, card);
        self.shared.emit(UiEvent::History);
        self.failures.marks.remove(&track.id.to_string());
        self.failures.failed = None;
        self.failures.stopped = false;
        self.failures.auto_retries = 0;
        self.shared.emit(UiEvent::PlayFailure(None));
        self.shared.listening.start(&self.shared.library, &track, self.play.position, chosen);
        let from = self.queue.cursor.map_or(0, |c| c + 1);
        for next in self.queue.tracks.iter().skip(from).take(PREFETCH_AHEAD) {
            self.shared.prefetch(&next.id);
        }
        self.top_up();
    }
}
