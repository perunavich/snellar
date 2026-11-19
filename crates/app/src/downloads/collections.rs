use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use snellar_core::error::Code;
use snellar_core::ids::TrackId;
use snellar_core::journal;
use snellar_core::library::Library;
use snellar_core::model::{Dated, Track};
use snellar_core::settings;
use snellar_core::sync::Lock;

use super::Downloads;

pub const KINDS: [&str; 3] = ["playlist", "remote", "album"];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Kept {
    collections: Vec<Entry>,
    pulled: Vec<TrackId>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Entry {
    kind: String,
    id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tracks: Vec<TrackId>,
}

impl Entry {
    fn is(&self, kind: &str, id: &str) -> bool {
        self.kind == kind && self.id == id
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub kind: String,
    pub id: String,
    pub total: usize,
    pub done: usize,
    pub refused: usize,
    pub busy: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stalled: Option<Code>,
}

#[derive(Default)]
struct State {
    kept: Kept,
    /// Why the list as saved could not be read last time; nothing is saved over it meanwhile.
    unread: Option<Code>,
    /// Songs that finished while the list could not be read; settled at the next read that works.
    later: Vec<TrackId>,
    /// Songs downloaded on their own while the list could not be read; taken off `pulled` at the next read that works.
    claims: Vec<TrackId>,
    refused: HashMap<TrackId, Code>,
    stalled: Option<Code>,
}

pub struct Collections {
    library: Arc<Library>,
    downloads: Arc<Downloads>,
    state: Mutex<State>,
}

fn stops_the_queue(code: Code) -> bool {
    code.network()
        || matches!(code, Code::DownloadNoSpace | Code::YtdlpMissing | Code::LibraryFailed | Code::LibraryLocked | Code::LibraryCorrupt)
}

fn ids(tracks: &[Track]) -> Vec<TrackId> {
    tracks.iter().map(|t| t.id.clone()).collect()
}

impl Collections {
    pub fn new(library: Arc<Library>, downloads: Arc<Downloads>) -> Arc<Collections> {
        let this = Arc::new(Collections { library, downloads, state: Mutex::default() });
        let weak = Arc::downgrade(&this);
        this.downloads.on_finished(Box::new(move |track, done| {
            if let Some(this) = weak.upgrade() {
                this.finished(track, done);
            }
        }));
        this
    }

    /// The state with the list as saved. Whatever reads fail, nothing is deleted or forgotten: a change that
    /// needs the list (`set`) is refused with the error, a clean-up (`release`, `prune`, `sync`) waits for a later
    /// call, and a song that finished or was claimed meanwhile is settled at the next read that works.
    /// Lock order: this state, then the downloads' files, then their queue; none is held across an await.
    fn state(&self) -> MutexGuard<'_, State> {
        let mut s = self.state.locked();
        match settings::DOWNLOADED_COLLECTIONS.read::<Kept>(&self.library) {
            Ok(kept) => {
                (s.kept, s.unread) = (kept.unwrap_or_default(), None);
                let claims = std::mem::take(&mut s.claims);
                if !claims.is_empty() && self.claim(&mut s, &claims).is_err() {
                    s.claims = claims;
                }
                for id in std::mem::take(&mut s.later) {
                    self.settle(&mut s, id);
                }
            }
            Err(e) => {
                journal::warn("downloads", format!("the downloaded collections stay as last read: {e:#}"));
                s.unread = Some(Code::of(&e));
            }
        }
        s
    }

    fn readable(&self) -> Result<MutexGuard<'_, State>> {
        let s = self.state();
        if let Some(code) = s.unread {
            return Err(code.err("the downloaded collections cannot be read"));
        }
        Ok(s)
    }

    fn save(&self, s: &State) -> Result<()> {
        if let Some(code) = s.unread {
            return Err(code.err("the downloaded collections are not saved over a list that could not be read"));
        }
        if !settings::DOWNLOADED_COLLECTIONS.set(&self.library, &s.kept) {
            return Err(Code::LibraryFailed.err("the downloaded collections could not be saved"));
        }
        Ok(())
    }

    /// Puts the changed list in place and saves it; when it cannot be saved, the list stays as it was.
    fn put(&self, s: &mut State, kept: Kept) -> Result<()> {
        let before = std::mem::replace(&mut s.kept, kept);
        if s.kept == before {
            return Ok(());
        }
        self.save(s).inspect_err(|_| s.kept = before)
    }

    fn local(&self, id: &str) -> Result<Option<Vec<TrackId>>> {
        match id.parse() {
            Ok(id) => self.library.playlist_ids(id),
            Err(_) => Ok(None),
        }
    }

    fn members(&self, entry: &Entry) -> Result<Vec<TrackId>> {
        if entry.kind != "playlist" {
            return Ok(entry.tracks.clone());
        }
        Ok(self.local(&entry.id)?.unwrap_or_default())
    }

    fn have(&self) -> Result<HashSet<TrackId>> {
        Ok(self.library.download_paths()?.into_iter().filter(|(_, path)| path.is_file()).map(|(id, _)| id).collect())
    }

    fn busy(&self) -> HashSet<TrackId> {
        self.downloads.busy().into_iter().collect()
    }

    fn missing_ids(&self, s: &State) -> Result<Vec<TrackId>> {
        if let Some(code) = s.unread {
            return Err(code.err("the downloaded collections cannot be read"));
        }
        let have = self.have()?;
        Ok(self.wanted(&s.kept)?.into_iter().filter(|id| !have.contains(id) && !s.refused.contains_key(id)).collect())
    }

    fn wanted(&self, kept: &Kept) -> Result<Vec<TrackId>> {
        let mut seen = HashSet::new();
        let mut wanted = Vec::new();
        for entry in &kept.collections {
            wanted.extend(self.members(entry)?.into_iter().filter(|id| seen.insert(id.clone())));
        }
        Ok(wanted)
    }

    pub fn has(&self, kind: &str, id: &str) -> bool {
        self.state().kept.collections.iter().any(|e| e.is(kind, id))
    }

    /// Turns a collection on or off. The change and the removal of the files nothing wants any more happen
    /// under one hold of the state, and only once every kept list can be read: otherwise nothing changes.
    pub fn set(&self, kind: &str, id: &str, on: bool, tracks: &[Track]) -> Result<()> {
        if on && kind != "playlist" {
            self.library.remember_tracks(tracks);
        }
        let mut s = self.readable()?;
        let mut kept = s.kept.clone();
        let at = kept.collections.iter().position(|e| e.is(kind, id));
        let listed = if kind == "playlist" { Vec::new() } else { ids(tracks) };
        match (on, at) {
            (true, Some(at)) => kept.collections[at].tracks = listed,
            (true, None) => kept.collections.push(Entry { kind: kind.into(), id: id.into(), tracks: listed }),
            (false, Some(at)) => {
                kept.collections.remove(at);
            }
            (false, None) => return Ok(()),
        }
        let wanted = self.wanted(&kept)?.into_iter().collect();
        self.put(&mut s, kept)?;
        self.let_go(&mut s, &wanted);
        drop(s);
        self.downloads.tell();
        if on {
            self.sync();
        }
        Ok(())
    }

    pub fn playlist_changed(&self, id: i64) {
        if self.has("playlist", &id.to_string()) {
            self.sync();
        }
    }

    /// Queues the songs the kept collections still miss. A read that fails leaves the collections stalled
    /// with its code, so the downloads are tried again later.
    pub fn sync(&self) {
        let missing = {
            let mut s = self.state();
            let read = self.missing_ids(&s).and_then(|missing| {
                missing.iter().filter_map(|id| self.library.track_read(id).transpose()).collect::<Result<Vec<Track>>>()
            });
            match read {
                Ok(missing) => {
                    s.stalled = None;
                    missing
                }
                Err(e) => {
                    s.stalled = Some(Code::of(&e));
                    return journal::warn("downloads", format!("nothing more downloads this time: {e:#}"));
                }
            }
        };
        self.downloads.collect(missing);
        self.downloads.tell();
    }

    /// Removes the files of the collected songs no downloaded collection wants any more, and drops such songs
    /// still waiting in the queue. A song whose file could not be removed stays counted and goes at a later call.
    pub fn release(&self) {
        let mut s = self.state();
        let wanted = match self.wanted(&s.kept) {
            Ok(wanted) if s.unread.is_none() => wanted.into_iter().collect(),
            Ok(_) => return journal::warn("downloads", "no files are removed while the downloaded collections cannot be read"),
            Err(e) => return journal::warn("downloads", format!("no files are removed while a playlist cannot be read: {e:#}")),
        };
        self.let_go(&mut s, &wanted);
        drop(s);
        self.downloads.tell();
    }

    /// Called with the state held, so a collection turned on again meanwhile sees the files gone and the queue
    /// trimmed before it looks, and downloads them again.
    fn let_go(&self, s: &mut State, wanted: &HashSet<TrackId>) {
        let gone: Vec<TrackId> = s.kept.pulled.iter().filter(|id| !wanted.contains(*id)).cloned().collect();
        let forgotten: Vec<TrackId> = gone.into_iter().filter(|id| self.downloads.forget(id)).collect();
        if !forgotten.is_empty() {
            let mut kept = s.kept.clone();
            kept.pulled.retain(|id| !forgotten.contains(id));
            if let Err(e) = self.put(s, kept) {
                journal::warn("downloads", format!("the removed songs are counted until the list can be saved: {e:#}"));
            }
        }
        self.downloads.drop_collected(|id| wanted.contains(id));
    }

    /// Songs downloaded on their own from now on, so no collection removes them. While the list cannot be read the
    /// download goes on and only this note waits, in `claims`, for the next read that works.
    pub fn claimed(&self, singles: &[TrackId]) -> Result<()> {
        let mut s = self.state();
        if s.unread.is_some() {
            journal::warn("downloads", format!("{} songs count as single downloads once the collections list can be read", singles.len()));
            s.later.retain(|id| !singles.contains(id));
            for id in singles {
                if !s.claims.contains(id) {
                    s.claims.push(id.clone());
                }
            }
            return Ok(());
        }
        self.claim(&mut s, singles)?;
        s.later.retain(|id| !singles.contains(id));
        Ok(())
    }

    fn claim(&self, s: &mut State, singles: &[TrackId]) -> Result<()> {
        let mut kept = s.kept.clone();
        kept.pulled.retain(|id| !singles.contains(id));
        self.put(s, kept)
    }

    /// A collected song that finished downloading: counted as pulled, or removed if no collection wants it now.
    /// When that cannot be told, the song waits in `later` with its file kept.
    fn settle(&self, s: &mut State, id: TrackId) {
        let wanted = match self.wanted(&s.kept) {
            Ok(wanted) if s.unread.is_none() => wanted,
            _ => return s.later.push(id),
        };
        if !wanted.contains(&id) {
            if !self.downloads.forget(&id) {
                s.later.push(id);
            }
            return;
        }
        if !s.kept.pulled.contains(&id) {
            let mut kept = s.kept.clone();
            kept.pulled.push(id.clone());
            if self.put(s, kept).is_err() {
                s.later.push(id);
            }
        }
    }

    fn finished(&self, track: &Track, done: Result<(), Code>) {
        let mut s = self.state();
        match done {
            Ok(()) => {
                s.refused.remove(&track.id);
                self.settle(&mut s, track.id.clone());
            }
            Err(code) if stops_the_queue(code) => {
                s.stalled = Some(code);
                self.downloads.drop_collected(|_| false);
                drop(s);
                self.downloads.tell();
            }
            Err(code) => {
                s.refused.insert(track.id.clone(), code);
            }
        }
    }

    pub fn prune(&self) {
        {
            let mut s = self.state();
            let mut gone = HashSet::new();
            for e in s.kept.collections.iter().filter(|e| e.kind == "playlist") {
                match self.local(&e.id) {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        gone.insert(e.id.clone());
                    }
                    Err(e) => return journal::warn("downloads", format!("deleted playlists keep their files until a read works: {e:#}")),
                }
            }
            if s.unread.is_some() {
                return journal::warn("downloads", "deleted playlists keep their files until the downloaded collections can be read");
            }
            let mut kept = s.kept.clone();
            kept.collections.retain(|e| e.kind != "playlist" || !gone.contains(&e.id));
            if let Err(e) = self.put(&mut s, kept) {
                return journal::warn("downloads", format!("deleted playlists stay listed until the list can be saved: {e:#}"));
            }
        }
        self.release();
    }

    pub fn singles(&self, songs: Vec<Dated<Track>>) -> Vec<Dated<Track>> {
        let s = self.state();
        songs.into_iter().filter(|t| !s.kept.pulled.contains(&t.item.id)).collect()
    }

    pub fn remotes(&self) -> Vec<String> {
        self.state().kept.collections.iter().filter(|e| e.kind == "remote").map(|e| e.id.clone()).collect()
    }

    pub fn stalled(&self) -> Option<Code> {
        self.state().stalled
    }

    /// Whether a kept collection misses songs; an error when that cannot be told.
    pub fn missing(&self) -> Result<bool> {
        Ok(!self.missing_ids(&self.state())?.is_empty())
    }

    pub fn progress(&self) -> Vec<Progress> {
        let s = self.state();
        let (have, busy) = (self.have().unwrap_or_default(), self.busy());
        s.kept
            .collections
            .iter()
            .map(|e| {
                let members = self.members(e).unwrap_or_default();
                let done = members.iter().filter(|id| have.contains(id)).count();
                let refused = members.iter().filter(|id| !have.contains(id) && s.refused.contains_key(id)).count();
                Progress {
                    kind: e.kind.clone(),
                    id: e.id.clone(),
                    total: members.len(),
                    done,
                    refused,
                    busy: members.iter().filter(|id| busy.contains(id)).count(),
                    stalled: s.stalled.filter(|_| done + refused < members.len()),
                }
            })
            .collect()
    }
}
