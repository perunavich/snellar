//! The queue and its order: shuffle, repeat, what plays next, the radio that continues it, and the copy kept in
//! the settings.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use snellar_core::error::Code;
use snellar_core::ids::TrackId;
use snellar_core::model::{Pin, Repeat, Track, Via};
use snellar_core::settings;
use snellar_core::ui::{Toast, UiEvent};

use super::load::Move;
use super::{Msg, Owner, Parked, QUEUE_SAVE_DELAY, Reply, TOP_UP_AT};

pub(super) fn stamped(mut tracks: Vec<Track>, via: Via) -> Vec<Track> {
    for (i, t) in tracks.iter_mut().enumerate() {
        let pick = t.pick.get_or_insert_with(|| Box::new(snellar_core::model::Pick { position: i as u32, ..Default::default() }));
        pick.via = via;
    }
    tracks
}

pub(super) struct Queue {
    pub(super) tracks: Vec<Track>,
    pub(super) cursor: Option<usize>,
    pub(super) shuffle: bool,
    pub(super) original: Vec<Track>,
    pub(super) epoch: u64,
    rng: u64,
    dismissed: HashSet<TrackId>,
}

impl Queue {
    pub(super) fn new() -> Queue {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Queue { tracks: Vec::new(), cursor: None, shuffle: false, original: Vec::new(), epoch: 0, rng: seed | 1, dismissed: HashSet::new() }
    }

    pub(super) fn current(&self) -> Option<&Track> {
        self.cursor.and_then(|i| self.tracks.get(i))
    }

    pub(super) fn next_index(&self) -> Option<usize> {
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

    pub(super) fn replace(&mut self, mut tracks: Vec<Track>, index: usize) {
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

    pub(super) fn insert(&mut self, track: Track, next: bool) -> usize {
        let at = if next { self.cursor.map_or(0, |c| c + 1) } else { self.tracks.len() };
        if self.shuffle {
            let after = self.current().and_then(|c| self.original.iter().position(|t| t.id == c.id));
            self.original.insert(if next { after.map_or(self.original.len(), |i| i + 1) } else { self.original.len() }, track.clone());
        }
        self.tracks.insert(at, track);
        at
    }

    pub(super) fn insert_all(&mut self, tracks: Vec<Track>, next: bool) -> Option<usize> {
        let first = if next { self.cursor.map_or(0, |c| c + 1) } else { self.tracks.len() };
        let added = !tracks.is_empty();
        let ordered: Vec<Track> = if next { tracks.into_iter().rev().collect() } else { tracks };
        for t in ordered {
            self.insert(t, next);
        }
        added.then_some(first)
    }

    pub(super) fn append_new(&mut self, tracks: Vec<Track>) -> Option<usize> {
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

    pub(super) fn append_if_current(&mut self, epoch: u64, tracks: Vec<Track>) -> Option<usize> {
        if self.epoch == epoch { self.append_new(tracks) } else { None }
    }

    pub(super) fn settle_radio(&mut self, still_current: bool, epoch: u64, related: Vec<Track>) -> Option<usize> {
        self.append_if_current(epoch, related);
        if still_current && self.epoch == epoch { self.next_index() } else { None }
    }

    pub(super) fn locate(&self, index: usize, id: Option<&TrackId>) -> Option<usize> {
        let Some(id) = id else { return (index < self.tracks.len()).then_some(index) };
        if self.tracks.get(index).is_some_and(|t| t.id == *id) {
            return Some(index);
        }
        (0..self.tracks.len()).filter(|i| self.tracks[*i].id == *id).min_by_key(|i| i.abs_diff(index))
    }

    pub(super) fn remove(&mut self, index: usize) -> Option<bool> {
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

    pub(super) fn move_item(&mut self, from: usize, to: usize) -> bool {
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

    pub(super) fn clear(&mut self) {
        self.epoch += 1;
        self.dismissed.clear();
        self.tracks.clear();
        self.original.clear();
        self.cursor = None;
    }

    pub(super) fn keep_current(&mut self) -> bool {
        let Some(current) = self.current().cloned() else { return false };
        self.epoch += 1;
        self.dismissed.extend(self.tracks.iter().filter(|t| t.id != current.id).map(|t| t.id.clone()));
        self.original = if self.shuffle { vec![current.clone()] } else { Vec::new() };
        self.tracks = vec![current];
        self.cursor = Some(0);
        true
    }

    pub(super) fn learn_lengths(&mut self, found: impl Fn(&TrackId) -> Option<f64>) -> bool {
        let mut changed = false;
        for track in self.tracks.iter_mut().chain(self.original.iter_mut()).filter(|t| t.duration <= 0.0) {
            if let Some(seconds) = found(&track.id) {
                track.duration = seconds;
                changed = true;
            }
        }
        changed
    }

    pub(super) fn set_shuffle(&mut self, on: bool) {
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

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Origin {
    pub(super) card: Pin,
    pub(super) tracks: HashSet<TrackId>,
}

/// The queue as the `queue` setting keeps it.
#[derive(Serialize, Deserialize, Default)]
pub(super) struct Saved {
    pub(super) tracks: Vec<Track>,
    pub(super) cursor: Option<usize>,
    #[serde(default)]
    pub(super) origin: Option<Origin>,
    #[serde(default)]
    pub(super) shuffle: bool,
    #[serde(default)]
    pub(super) original: Vec<Track>,
    #[serde(default)]
    pub(super) repeat: Repeat,
}

impl Saved {
    pub(super) fn of(o: &Owner) -> Saved {
        Saved {
            tracks: o.queue.tracks.clone(),
            cursor: o.queue.cursor,
            origin: o.origin.clone(),
            shuffle: o.queue.shuffle,
            original: o.queue.original.clone(),
            repeat: o.repeat,
        }
    }

    pub(super) fn current(&self) -> Option<&Track> {
        self.cursor.and_then(|c| self.tracks.get(c))
    }
}

impl Owner {
    /// The track that plays when the current one ends by itself.
    pub(super) fn following(&self) -> Option<usize> {
        match (self.queue.cursor, self.queue.next_index()) {
            (Some(c), _) if self.repeat == Repeat::One => Some(c),
            (_, Some(n)) => Some(n),
            (_, None) if self.repeat == Repeat::All && !self.queue.tracks.is_empty() => Some(0),
            _ => None,
        }
    }

    pub(super) fn following_id(&self) -> Option<&TrackId> {
        self.following().map(|i| &self.queue.tracks[i].id)
    }

    /// Moves on from the current track: `auto` when it ended or failed by itself (repeat-one plays it again then),
    /// not when the person pressed next. At the end of the queue autoplay asks for a radio and plays its first track.
    pub(super) fn advance(&mut self, auto: bool, mv: Move, reply: Option<Reply>) {
        let q = &self.queue;
        let step = match (q.cursor, q.next_index(), q.tracks.last()) {
            (Some(c), _, _) if auto && self.repeat == Repeat::One => Some(c),
            (_, Some(n), _) => Some(n),
            (_, None, Some(_)) if self.repeat == Repeat::All => Some(0),
            (_, None, Some(last)) if self.shared.autoplay() && self.shared.has_radio() => {
                let (seed, queue, epoch, asked) = (last.clone(), q.tracks.clone(), q.epoch, self.loads);
                let (shared, link) = (self.shared.clone(), self.link.clone());
                self.shared.tasks.spawn("continue with the radio", async move {
                    let related = shared.source.radio(&seed, &queue).await.map(|tracks| shared.timed(tracks));
                    link.send(Msg::Continued { asked, epoch, mv, related, reply });
                });
                return;
            }
            _ => None,
        };
        match step {
            Some(i) => self.begin(i, mv, reply),
            None => self.answer(reply, Ok(())),
        }
    }

    /// The radio that continues the queue came back. Its first track plays only if nothing else started meanwhile.
    pub(super) fn continued(&mut self, asked: u64, epoch: u64, mv: Move, related: Result<Vec<Track>>, reply: Option<Reply>) {
        match related {
            Ok(related) => {
                let related = stamped(related, self.continuing);
                let still_current = self.loads == asked;
                let first = self.queue.settle_radio(still_current, epoch, related);
                self.publish_queue();
                match first {
                    Some(i) => self.begin(i, mv, reply),
                    None => self.answer(reply, Ok(())),
                }
            }
            Err(e) => {
                self.shared.emit(UiEvent::Toast(Toast::failed(Code::AutoplayFailed, &e)));
                self.answer(reply, Err(e));
            }
        }
    }

    pub(super) fn top_up(&mut self) {
        let upcoming = self.queue.tracks.len() - self.queue.cursor.map_or(0, |c| c + 1);
        if self.topping == Some(self.queue.epoch)
            || upcoming > TOP_UP_AT
            || self.repeat == Repeat::All
            || !self.shared.autoplay()
            || !self.shared.has_radio()
        {
            return;
        }
        let Some(seed) = self.queue.tracks.last().cloned() else { return };
        let (queue, epoch) = (self.queue.tracks.clone(), self.queue.epoch);
        self.topping = Some(epoch);
        let (shared, link) = (self.shared.clone(), self.link.clone());
        self.shared.tasks.spawn("radio top-up", async move {
            let related = shared.source.radio(&seed, &queue).await.map(|tracks| shared.timed(tracks));
            link.send(Msg::ToppedUp { epoch, related });
        });
    }

    /// A top-up came back; a queue replaced meanwhile does not take it (`epoch`).
    pub(super) fn topped_up(&mut self, epoch: u64, related: Result<Vec<Track>>) {
        if self.topping == Some(epoch) {
            self.topping = None;
        }
        if let Ok(tracks) = related {
            let tracks = stamped(tracks, self.continuing);
            self.queue.append_if_current(epoch, tracks);
            self.publish_queue();
        }
    }

    pub(super) fn replace_queue(&mut self, tracks: Vec<Track>, index: usize) -> u64 {
        self.queue.replace(tracks, index);
        self.ask_lengths();
        self.parked = None;
        self.origin = None;
        self.queue.epoch
    }

    /// Asks the source once for the lengths of the rows that have none. The answer comes back as a message, so it
    /// is applied to the rows it is for (asking before the rows were in the queue lost a fast answer).
    pub(super) fn ask_lengths(&mut self) {
        let (asked, source) = (&mut self.asked, &self.shared.source);
        let unknown: Vec<TrackId> = self
            .queue
            .tracks
            .iter()
            .filter(|t| t.duration <= 0.0 && source.owns(&t.id))
            .filter(|t| asked.insert(t.id.clone()))
            .map(|t| t.id.clone())
            .collect();
        if unknown.is_empty() {
            return;
        }
        let (shared, link) = (self.shared.clone(), self.link.clone());
        self.shared.tasks.spawn("learn the lengths of queued tracks", async move {
            let found = shared.source.lengths(&unknown).await;
            link.send(Msg::Lengths { asked: unknown, found });
        });
    }

    /// Lengths the source told; rows it could not be asked about are asked again with the next queue.
    pub(super) fn lengths(&mut self, asked: Vec<TrackId>, found: Result<HashMap<TrackId, f64>>) {
        match found {
            Ok(found) => self.apply_lengths(&found),
            Err(_) => self.asked.retain(|id| !asked.contains(id)),
        }
    }

    pub(super) fn apply_lengths(&mut self, found: &HashMap<TrackId, f64>) {
        if self.queue.learn_lengths(|id| found.get(id).copied()) {
            self.publish_queue();
        }
    }

    pub(super) fn publish_queue(&mut self) {
        self.recheck_preload();
        self.ask_lengths();
        let cursor = self.queue.cursor.map_or(-1, |c| c as i32);
        self.shared.emit(UiEvent::Queue { tracks: self.queue.tracks.clone(), cursor });
        self.save_queue_soon();
        self.view_due = true;
    }

    pub(super) fn publish_modes(&mut self) {
        self.recheck_preload();
        self.shared.emit(UiEvent::Modes { shuffle: self.queue.shuffle, repeat: self.repeat });
        self.view_due = true;
    }

    /// Saves the queue half a second after it changed, once for a burst of changes; quitting saves at once.
    pub(super) fn save_queue_soon(&mut self) {
        if self.queue_due {
            return;
        }
        self.queue_due = true;
        let link = self.link.clone();
        self.shared.tasks.spawn("save the queue", async move {
            tokio::time::sleep(QUEUE_SAVE_DELAY).await;
            link.send(Msg::SaveQueue);
        });
    }

    pub(super) fn save_queue(&mut self) {
        self.queue_due = false;
        settings::QUEUE.set(&self.shared.library, &Saved::of(self));
    }

    pub(super) fn restore(&mut self) {
        let library = self.shared.library.clone();
        let Some(saved) = settings::QUEUE.get::<Saved>(&library) else { return };
        let position = settings::POSITION.or_default(&library);
        self.queue.cursor = saved.cursor.filter(|c| *c < saved.tracks.len());
        self.queue.tracks = self.shared.timed(saved.tracks);
        self.queue.shuffle = saved.shuffle && !saved.original.is_empty();
        self.queue.original = if self.queue.shuffle { self.shared.timed(saved.original) } else { Vec::new() };
        self.repeat = saved.repeat;
        self.origin = saved.origin;
        self.publish_queue();
        self.publish_modes();
        let Some(track) = self.queue.current().cloned() else { return };
        self.parked = Some(Parked { id: track.id.clone(), at: position });
        self.play.position = position;
        self.play.duration = track.duration;
        self.announce(&track, position);
    }
}
