use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use rusqlite::{Connection, OptionalExtension, Params, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::clock;
use crate::error::Code;
use crate::ids::TrackId;
use crate::model::{AlbumCard, ArtistCard, Dated, Folder, Pin, Playlist, Track};
use crate::sync::Lock;
use crate::ui::Toast;

mod migrations;
mod recent;
pub mod signals;

#[derive(Debug)]
pub enum OpenError {
    Newer { found: i32, known: i32 },
    Backup { path: PathBuf, source: rusqlite::Error },
    Sqlite(rusqlite::Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Newer { found, known } => {
                write!(f, "the library has schema version {found}, this build knows up to {known}; it was left untouched")
            }
            OpenError::Backup { path, source } => write!(f, "no migration ran: cannot back up to {}: {source}", path.display()),
            OpenError::Sqlite(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            OpenError::Newer { .. } => None,
            OpenError::Backup { source, .. } => Some(source),
            OpenError::Sqlite(e) => Some(e),
        }
    }
}

impl OpenError {
    pub fn code(&self) -> Code {
        match self {
            OpenError::Newer { .. } => Code::LibraryNewer,
            OpenError::Backup { .. } => Code::LibraryFailed,
            OpenError::Sqlite(e) => Code::sqlite(e),
        }
    }
}

impl From<rusqlite::Error> for OpenError {
    fn from(e: rusqlite::Error) -> Self {
        OpenError::Sqlite(e)
    }
}

const COVERS: &str = "covers";
const FAILURE_GAP: Duration = Duration::from_secs(60);
const RECENT_WINDOW: i64 = 400;

type Sink = Box<dyn Fn(Toast) + Send + Sync>;

pub struct Library {
    db: Mutex<Connection>,
    logged: Mutex<Option<Instant>>,
    told: Mutex<Option<Instant>>,
    sink: OnceLock<Sink>,
    data: Option<PathBuf>,
    failed_reads: AtomicU64,
    read_failure: Mutex<Code>,
}

fn due(last: &Mutex<Option<Instant>>) -> bool {
    let mut last = last.locked();
    if last.is_some_and(|at| at.elapsed() < FAILURE_GAP) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

fn now() -> i64 {
    clock::unix_millis() as i64
}

fn rows<T: DeserializeOwned>(db: &Connection, sql: &str, params: impl Params) -> rusqlite::Result<Vec<T>> {
    let mut stmt = db.prepare_cached(sql)?;
    let mut found = Vec::new();
    let mut unreadable = 0;
    for json in stmt.query_map(params, |r| r.get::<_, String>(0))? {
        match serde_json::from_str(&json?) {
            Ok(row) => found.push(row),
            Err(_) => unreadable += 1,
        }
    }
    if unreadable > 0 {
        crate::journal::warn("library", format!("skipped {unreadable} rows with unreadable JSON"));
    }
    Ok(found)
}

fn dated(db: &Connection, sql: &str, params: impl Params) -> rusqlite::Result<Vec<Dated<Track>>> {
    let mut stmt = db.prepare_cached(sql)?;
    let found = stmt.query_map(params, |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = Vec::new();
    let mut unreadable = 0;
    for row in found {
        let (json, at) = row?;
        match serde_json::from_str(&json) {
            Ok(item) => out.push(Dated { item, at }),
            Err(_) => unreadable += 1,
        }
    }
    if unreadable > 0 {
        crate::journal::warn("library", format!("skipped {unreadable} rows with unreadable JSON"));
    }
    Ok(out)
}

fn backup(db: &Connection, path: &Path, from: i32) -> Result<(), OpenError> {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".backup-v{from}-{}", clock::file_stamp(SystemTime::now())));
    let to = PathBuf::from(name);
    let Some(target) = to.to_str() else {
        return Err(OpenError::Backup { source: rusqlite::Error::InvalidPath(to.clone()), path: to });
    };
    match db.execute("VACUUM INTO ?1", [target]) {
        Ok(_) => Ok(()),
        Err(source) => Err(OpenError::Backup { path: to, source }),
    }
}

fn append<'a>(db: &Connection, id: i64, items: impl IntoIterator<Item = (&'a Track, i64)>) -> rusqlite::Result<usize> {
    let mut added = 0;
    for (t, at) in items {
        upsert_track(db, t)?;
        added += db.execute(
            "INSERT OR IGNORE INTO playlist_items (playlist_id, track_id, position, added_at)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM playlist_items WHERE playlist_id = ?1), ?3)",
            params![id, t.id, at],
        )?;
    }
    Ok(added)
}

fn is_saved(db: &Connection, table: &str, id: &str) -> rusqlite::Result<bool> {
    Ok(db.query_row(&format!("SELECT 1 FROM {table} WHERE id = ?1"), [id], |_| Ok(())).optional()?.is_some())
}

fn toggle_saved(db: &Connection, table: &str, id: &str, item: &impl Serialize) -> rusqlite::Result<bool> {
    if db.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])? > 0 {
        return Ok(false);
    }
    db.execute(
        &format!("INSERT INTO {table} (id, json, added_at) VALUES (?1, ?2, ?3)"),
        params![id, serde_json::to_string(item).unwrap_or_default(), now()],
    )?;
    Ok(true)
}

const KEEP_KNOWN_LENGTH: &str = "DO UPDATE SET json = CASE
    WHEN json_extract(excluded.json, '$.duration') > 0 OR NOT json_valid(tracks.json) THEN excluded.json
    ELSE json_set(excluded.json, '$.duration', COALESCE(json_extract(tracks.json, '$.duration'), 0))
END";

fn upsert_track(db: &Connection, t: &Track) -> rusqlite::Result<()> {
    let stored;
    let t = if t.pick.is_some() {
        stored = Track { pick: None, ..t.clone() };
        &stored
    } else {
        t
    };
    let stub = t.artists.is_empty() && t.thumb.is_empty();
    let conflict = if stub { "DO NOTHING" } else { KEEP_KNOWN_LENGTH };
    db.execute(
        &format!("INSERT INTO tracks (id, json) VALUES (?1, ?2) ON CONFLICT (id) {conflict}"),
        params![t.id, serde_json::to_string(t).unwrap_or_default()],
    )
    .map(drop)
}

fn local_cover(cover: &str) -> Option<&std::ffi::OsStr> {
    if cover.contains("://") && !cover.starts_with("file://") {
        return None;
    }
    let path = Path::new(cover.strip_prefix("file://").unwrap_or(cover));
    (path.parent()?.file_name()? == COVERS).then_some(path.file_name()?)
}

fn shown_cover(data: Option<&Path>, cover: &str) -> String {
    match (data, local_cover(cover)) {
        (Some(data), Some(name)) => format!("file://{}", data.join(COVERS).join(name).display()),
        _ => cover.to_string(),
    }
}

fn stored_cover(data: Option<&Path>, cover: &str) -> String {
    match (data, local_cover(cover)) {
        (Some(_), Some(name)) => Path::new(COVERS).join(name).display().to_string(),
        _ => cover.to_string(),
    }
}

fn read_playlist(db: &Connection, data: Option<&Path>, id: i64) -> rusqlite::Result<Option<Playlist>> {
    let head = db
        .query_row("SELECT name, description, cover, folder FROM playlists WHERE id = ?1", [id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<i64>>(3)?))
        })
        .optional()?;
    let Some((name, description, cover, folder)) = head else { return Ok(None) };
    let cover = shown_cover(data, &cover);
    let tracks = dated(
        db,
        "SELECT t.json, i.added_at FROM playlist_items i JOIN tracks t ON t.id = i.track_id WHERE i.playlist_id = ?1 ORDER BY i.position",
        [id],
    )?;
    let thumb = if cover.is_empty() { tracks.first().map(|t| t.item.thumb.clone()).unwrap_or_default() } else { cover.clone() };
    Ok(Some(Playlist { id, name, description, cover, folder, count: tracks.len(), thumb, tracks }))
}

fn connect(path: &Path) -> Result<Connection, OpenError> {
    let mut db = Connection::open(path)?;
    let found = migrations::version(&db)?;
    if found > migrations::LATEST {
        return Err(OpenError::Newer { found, known: migrations::LATEST });
    }
    if found < migrations::LATEST && db.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))? > 0 {
        backup(&db, path, found)?;
    }
    db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
    migrations::apply(&mut db)?;
    signals::attach(&db, path);
    Ok(db)
}

impl Library {
    pub fn open(path: &Path) -> Result<Library, OpenError> {
        let db = connect(path)?;
        let data = path.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf);
        Ok(Library {
            db: Mutex::new(db),
            logged: Mutex::new(None),
            told: Mutex::new(None),
            sink: OnceLock::new(),
            data,
            failed_reads: AtomicU64::new(0),
            read_failure: Mutex::new(Code::LibraryFailed),
        })
    }

    pub fn reopen(&self, path: &Path) -> Result<(), OpenError> {
        let db = connect(path)?;
        *self.db.locked() = db;
        Ok(())
    }

    pub fn failed_reads(&self) -> u64 {
        self.failed_reads.load(Ordering::Relaxed)
    }

    pub fn read_failure(&self) -> Code {
        *self.read_failure.locked()
    }

    pub fn on_write_failure(&self, sink: impl Fn(Toast) + Send + Sync + 'static) {
        self.sink.set(Box::new(sink)).ok();
    }

    fn log_failure(&self, e: &rusqlite::Error) {
        if due(&self.logged) {
            crate::journal::error("library", e.to_string());
        }
    }

    fn run<T: Default>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> T {
        self.read(f).unwrap_or_default()
    }

    /// A read whose failure the caller can tell from an empty answer; it still counts as a failed read, so a
    /// library-read method that used it answers with the error instead of an empty list.
    fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> anyhow::Result<T> {
        let result = f(&self.db.locked());
        result.map_err(|e| {
            self.log_failure(&e);
            let code = Code::sqlite(&e);
            *self.read_failure.locked() = code;
            self.failed_reads.fetch_add(1, Ordering::Relaxed);
            anyhow::Error::new(e).context(code)
        })
    }

    fn write<T: Default>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> T {
        let result = f(&self.db.locked());
        result.unwrap_or_else(|e| {
            self.log_failure(&e);
            if let Some(sink) = self.sink.get().filter(|_| due(&self.told)) {
                sink(Toast { detail: e.to_string(), ..Toast::new(Code::sqlite(&e)) });
            }
            T::default()
        })
    }

    fn attempt<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> anyhow::Result<T> {
        let result = f(&self.db.locked());
        result.map_err(|e| {
            self.log_failure(&e);
            let code = Code::sqlite(&e);
            anyhow::Error::new(e).context(code)
        })
    }

    pub fn track(&self, id: &TrackId) -> Option<Track> {
        self.track_read(id).ok().flatten()
    }

    /// The song's details; an error, not None, when the library cannot be read.
    pub fn track_read(&self, id: &TrackId) -> anyhow::Result<Option<Track>> {
        let json = self.read(|db| db.query_row("SELECT json FROM tracks WHERE id = ?1", [id], |r| r.get::<_, String>(0)).optional())?;
        Ok(json.and_then(|json| serde_json::from_str(&json).ok()))
    }

    pub fn is_liked(&self, id: &TrackId) -> bool {
        self.run(|db| db.query_row("SELECT 1 FROM likes WHERE track_id = ?1", [id], |_| Ok(true)).optional().map(|v| v.unwrap_or(false)))
    }

    pub fn toggle_like(&self, t: &Track) -> bool {
        self.write(|db| {
            if db.execute("DELETE FROM likes WHERE track_id = ?1", [&t.id])? > 0 {
                return Ok(false);
            }
            upsert_track(db, t)?;
            db.execute("DELETE FROM dislikes WHERE track_id = ?1", [&t.id])?;
            db.execute("INSERT INTO likes (track_id, added_at) VALUES (?1, ?2)", params![t.id, now()])?;
            Ok(true)
        })
    }

    pub fn set_liked(&self, tracks: &[Track], on: bool) -> usize {
        self.write(|db| {
            let tx = db.unchecked_transaction()?;
            let mut changed = 0;
            let at = now();
            for t in tracks {
                if on {
                    upsert_track(&tx, t)?;
                    tx.execute("DELETE FROM dislikes WHERE track_id = ?1", [&t.id])?;
                    changed += tx.execute("INSERT OR IGNORE INTO likes (track_id, added_at) VALUES (?1, ?2)", params![t.id, at])?;
                } else {
                    changed += tx.execute("DELETE FROM likes WHERE track_id = ?1", [&t.id])?;
                }
            }
            tx.commit()?;
            Ok(changed)
        })
    }

    pub fn dislike(&self, t: &Track) {
        self.write(|db| {
            upsert_track(db, t)?;
            db.execute("DELETE FROM likes WHERE track_id = ?1", [&t.id])?;
            db.execute("INSERT OR REPLACE INTO dislikes (track_id, at) VALUES (?1, ?2)", params![t.id, now()])?;
            Ok(())
        })
    }

    pub fn undislike(&self, id: &TrackId) {
        self.write(|db| db.execute("DELETE FROM dislikes WHERE track_id = ?1", [id]).map(drop))
    }

    pub fn disliked(&self) -> Vec<Track> {
        self.run(|db| rows(db, "SELECT t.json FROM dislikes d JOIN tracks t ON t.id = d.track_id ORDER BY d.at DESC", []))
    }

    pub fn favorites(&self) -> Vec<Track> {
        self.liked().into_iter().map(|d| d.item).collect()
    }

    pub fn liked(&self) -> Vec<Dated<Track>> {
        self.run(|db| {
            dated(
                db,
                "SELECT t.json, l.added_at FROM likes l JOIN tracks t ON t.id = l.track_id ORDER BY l.added_at DESC, l.rowid DESC",
                [],
            )
        })
    }

    pub fn push_history(&self, t: &Track) {
        self.write(|db| {
            let tx = db.unchecked_transaction()?;
            upsert_track(&tx, t)?;
            tx.execute("INSERT INTO history (track_id, played_at) VALUES (?1, ?2)", params![t.id, now()])?;
            tx.commit()
        })
    }

    pub fn push_played(&self, t: &Track, context: Option<&Pin>) {
        let context = context.and_then(|c| serde_json::to_string(c).ok());
        self.write(|db| {
            let tx = db.unchecked_transaction()?;
            upsert_track(&tx, t)?;
            tx.execute("INSERT INTO history (track_id, played_at, context) VALUES (?1, ?2, ?3)", params![t.id, now(), context])?;
            tx.commit()
        })
    }

    pub fn recent(&self, limit: usize) -> Vec<Pin> {
        let plays: Vec<recent::Play> = self.run(|db| {
            let mut stmt = db.prepare_cached(
                "SELECT t.json, h.context FROM history h JOIN tracks t ON t.id = h.track_id ORDER BY h.rowid DESC LIMIT ?1",
            )?;
            let found = stmt.query_map([RECENT_WINDOW], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
            Ok(found
                .filter_map(|row| {
                    let (track, context) = row.ok()?;
                    let context = context.and_then(|c| serde_json::from_str(&c).ok());
                    Some(recent::Play { track: serde_json::from_str(&track).ok()?, context })
                })
                .collect())
        });
        recent::shelf(&plays, &self.playlists(), &self.followed(), limit)
    }

    pub fn remember_track(&self, t: &Track) {
        self.write(|db| upsert_track(db, t));
    }

    pub fn remember_tracks(&self, tracks: &[Track]) {
        self.write(|db| {
            let tx = db.unchecked_transaction()?;
            tracks.iter().try_for_each(|t| upsert_track(&tx, t))?;
            tx.commit()
        })
    }

    pub fn fill_lengths(&self, tracks: &mut [Track]) {
        if tracks.iter().all(|t| t.duration > 0.0) {
            return;
        }
        let wanted: Vec<&str> = tracks.iter().filter(|t| t.duration <= 0.0).map(|t| t.id.as_str()).collect();
        let ids = serde_json::to_string(&wanted).unwrap_or_default();
        let known: HashMap<String, f64> = self.run(|db| {
            let mut found = db.prepare_cached(
                "SELECT id, json_extract(json, '$.duration') FROM tracks
                 WHERE id IN (SELECT value FROM json_each(?1)) AND json_valid(json)",
            )?;
            let rows = found.query_map(params![ids], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<f64>>(1)?)))?;
            rows.filter_map(|r| r.map(|(id, d)| d.map(|d| (id, d))).transpose()).collect()
        });
        for track in tracks.iter_mut().filter(|t| t.duration <= 0.0) {
            if let Some(&d) = known.get(track.id.as_str()) {
                track.duration = d;
            }
        }
    }

    pub fn playlisted(&self, limit: usize) -> Vec<Track> {
        self.run(|db| {
            rows(
                db,
                "SELECT t.json FROM playlist_items i JOIN tracks t ON t.id = i.track_id
                 GROUP BY i.track_id ORDER BY MAX(i.added_at) DESC, MIN(i.position) LIMIT ?1",
                [limit as i64],
            )
        })
    }

    pub fn history(&self, limit: usize) -> Vec<Track> {
        self.played(limit).into_iter().map(|d| d.item).collect()
    }

    pub fn played(&self, limit: usize) -> Vec<Dated<Track>> {
        self.run(|db| {
            dated(
                db,
                "SELECT t.json, MAX(h.played_at) FROM history h JOIN tracks t ON t.id = h.track_id
                 GROUP BY h.track_id ORDER BY MAX(h.rowid) DESC LIMIT ?1",
                [limit as i64],
            )
        })
    }

    pub fn listens(&self, limit: usize) -> Vec<(Track, f64, bool, i64)> {
        self.run(|db| {
            let mut stmt = db.prepare_cached(
                "SELECT t.json, l.seconds, l.skipped, l.at FROM listens l JOIN tracks t ON t.id = l.track_id ORDER BY l.rowid DESC LIMIT ?1",
            )?;
            let found = stmt.query_map([limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            Ok(found
                .filter_map(|r| {
                    let (json, seconds, skipped, at) = r.ok()?;
                    Some((serde_json::from_str(&json).ok()?, seconds, skipped, at))
                })
                .collect())
        })
    }

    /// False when the row could not be written (the person was told).
    pub fn set_download(&self, id: &TrackId, path: &Path, loudness_db: Option<f64>) -> bool {
        self.write(|db| {
            db.execute(
                "INSERT OR REPLACE INTO downloads (track_id, path, loudness, at) VALUES (?1, ?2, ?3, ?4)",
                params![id, path.to_string_lossy(), loudness_db, now()],
            )
            .map(|_| true)
        })
    }

    pub fn download(&self, id: &TrackId) -> Option<(PathBuf, Option<f64>)> {
        self.download_row(id).ok().flatten()
    }

    /// The file and loudness of a downloaded song; an error, not None, when the library cannot be read.
    pub fn download_row(&self, id: &TrackId) -> anyhow::Result<Option<(PathBuf, Option<f64>)>> {
        self.read(|db| {
            db.query_row("SELECT path, loudness FROM downloads WHERE track_id = ?1", [id], |r| {
                Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?))
            })
            .optional()
        })
    }

    pub fn download_paths(&self) -> anyhow::Result<Vec<(TrackId, PathBuf)>> {
        self.read(|db| {
            let mut stmt = db.prepare_cached("SELECT track_id, path FROM downloads")?;
            let rows =
                stmt.query_map([], |r| Ok((TrackId::from(r.get::<_, String>(0)?.as_str()), PathBuf::from(r.get::<_, String>(1)?))))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    /// False when the row could not be written (the person was told).
    pub fn move_download(&self, id: &TrackId, path: &Path) -> bool {
        self.write(|db| db.execute("UPDATE downloads SET path = ?2 WHERE track_id = ?1", params![id, path.to_string_lossy()]).map(|_| true))
    }

    pub fn downloaded(&self) -> Vec<TrackId> {
        self.download_paths().unwrap_or_default().into_iter().filter(|(_, path)| path.is_file()).map(|(id, _)| id).collect()
    }

    pub fn downloaded_songs(&self) -> Vec<Dated<Track>> {
        let found: Vec<(String, String, Option<String>, i64)> = self.run(|db| {
            let mut stmt = db.prepare_cached(
                "SELECT d.track_id, d.path, t.json, d.at FROM downloads d LEFT JOIN tracks t ON t.id = d.track_id ORDER BY d.at DESC, d.rowid DESC",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect()
        });
        found
            .into_iter()
            .filter(|(_, path, _, _)| Path::new(path).is_file())
            .map(|(id, path, json, at)| {
                let known = json.and_then(|j| serde_json::from_str(&j).ok());
                let item = known.unwrap_or_else(|| Track {
                    id: id.as_str().into(),
                    title: Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                    ..Track::default()
                });
                Dated { item, at }
            })
            .collect()
    }

    /// False when the row could not be removed (the person was told).
    pub fn forget_download(&self, id: &TrackId) -> bool {
        self.write(|db| db.execute("DELETE FROM downloads WHERE track_id = ?1", [id]).map(|_| true))
    }

    pub fn clear_history(&self) {
        self.write(|db| db.execute_batch("DELETE FROM history; DELETE FROM listens; DELETE FROM sig.plays;"))
    }

    pub fn record_listen(&self, id: &TrackId, seconds: f64, skipped: bool) {
        self.write(|db| {
            db.execute("INSERT INTO listens (track_id, seconds, skipped, at) VALUES (?1, ?2, ?3, ?4)", params![id, seconds, skipped, now()])
                .map(drop)
        })
    }

    pub fn playlists(&self) -> Vec<Playlist> {
        self.playlists_by("p.id")
    }

    pub fn recent_playlists(&self) -> Vec<Playlist> {
        self.playlists_by("COALESCE((SELECT MAX(added_at) FROM playlist_items WHERE playlist_id = p.id), p.created_at) DESC, p.id DESC")
    }

    fn playlists_by(&self, order: &str) -> Vec<Playlist> {
        self.run(|db| {
            let mut stmt = db.prepare_cached(&format!(
                "SELECT p.id, p.name, p.description, p.cover, p.folder,
                    (SELECT COUNT(*) FROM playlist_items WHERE playlist_id = p.id),
                    (SELECT t.json FROM playlist_items i JOIN tracks t ON t.id = i.track_id
                     WHERE i.playlist_id = p.id ORDER BY i.position LIMIT 1)
                 FROM playlists p ORDER BY {order}"
            ))?;
            let lists = stmt.query_map([], |r| {
                let first: Option<Track> = r.get::<_, Option<String>>(6)?.and_then(|j| serde_json::from_str(&j).ok());
                let cover = shown_cover(self.data.as_deref(), &r.get::<_, String>(3)?);
                Ok(Playlist {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    folder: r.get(4)?,
                    count: r.get::<_, i64>(5)? as usize,
                    thumb: if cover.is_empty() { first.map(|t| t.thumb).unwrap_or_default() } else { cover.clone() },
                    cover,
                    tracks: Vec::new(),
                })
            })?;
            lists.collect()
        })
    }

    pub fn playlist(&self, id: i64) -> Option<Playlist> {
        self.run(|db| read_playlist(db, self.data.as_deref(), id))
    }

    /// None when there is no such playlist; an error when the library cannot be read.
    pub fn playlist_ids(&self, id: i64) -> anyhow::Result<Option<Vec<TrackId>>> {
        self.read(|db| {
            if db.query_row("SELECT 1 FROM playlists WHERE id = ?1", [id], |_| Ok(())).optional()?.is_none() {
                return Ok(None);
            }
            let mut stmt = db.prepare_cached("SELECT track_id FROM playlist_items WHERE playlist_id = ?1 ORDER BY position")?;
            let ids = stmt.query_map([id], |r| Ok(TrackId::from(r.get::<_, String>(0)?.as_str())))?;
            ids.collect::<rusqlite::Result<Vec<_>>>().map(Some)
        })
    }

    pub fn create_playlist(&self, name: &str) -> anyhow::Result<i64> {
        self.attempt(|db| {
            db.execute("INSERT INTO playlists (name, created_at) VALUES (?1, ?2)", params![name, now()])?;
            Ok(db.last_insert_rowid())
        })
    }

    pub fn create_filled_playlist(&self, name: &str, tracks: &[Track]) -> anyhow::Result<i64> {
        self.attempt(|db| {
            let tx = db.unchecked_transaction()?;
            let at = now();
            tx.execute("INSERT INTO playlists (name, created_at) VALUES (?1, ?2)", params![name, at])?;
            let id = tx.last_insert_rowid();
            append(&tx, id, tracks.iter().map(|t| (t, at)))?;
            tx.commit()?;
            Ok(id)
        })
    }

    pub fn rename_playlist(&self, id: i64, name: &str) {
        self.write(|db| db.execute("UPDATE playlists SET name = ?1 WHERE id = ?2", params![name, id]).map(drop))
    }

    pub fn describe_playlist(&self, id: i64, description: &str) {
        self.write(|db| db.execute("UPDATE playlists SET description = ?1 WHERE id = ?2", params![description, id]).map(drop))
    }

    pub fn set_playlist_cover(&self, id: i64, cover: &str) {
        let cover = stored_cover(self.data.as_deref(), cover);
        self.write(|db| db.execute("UPDATE playlists SET cover = ?1 WHERE id = ?2", params![cover, id]).map(drop))
    }

    pub fn playlist_covers(&self) -> anyhow::Result<Vec<(i64, String)>> {
        self.attempt(|db| {
            let mut stmt = db.prepare("SELECT id, cover FROM playlists")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, shown_cover(self.data.as_deref(), &r.get::<_, String>(1)?))))?;
            rows.collect()
        })
    }

    pub fn delete_playlist(&self, id: i64) -> anyhow::Result<Option<Playlist>> {
        self.attempt(|db| {
            let tx = db.unchecked_transaction()?;
            let Some(gone) = read_playlist(&tx, self.data.as_deref(), id)? else { return Ok(None) };
            tx.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", [id])?;
            tx.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
            tx.commit()?;
            Ok(Some(gone))
        })
    }

    pub fn restore_playlist(&self, p: &Playlist) -> anyhow::Result<i64> {
        let cover = stored_cover(self.data.as_deref(), &p.cover);
        self.attempt(|db| {
            let tx = db.unchecked_transaction()?;
            let free = tx.execute(
                "INSERT OR IGNORE INTO playlists (id, name, description, cover, folder, created_at)
                 VALUES (?1, ?2, ?3, ?4, (SELECT id FROM folders WHERE id = ?5), ?6)",
                params![p.id, p.name, p.description, cover, p.folder, now()],
            )? > 0;
            if !free {
                tx.execute(
                    "INSERT INTO playlists (name, description, cover, folder, created_at)
                     VALUES (?1, ?2, ?3, (SELECT id FROM folders WHERE id = ?4), ?5)",
                    params![p.name, p.description, cover, p.folder, now()],
                )?;
            }
            let id = if free { p.id } else { tx.last_insert_rowid() };
            append(&tx, id, p.tracks.iter().map(|t| (&t.item, if t.at > 0 { t.at } else { now() })))?;
            tx.commit()?;
            Ok(id)
        })
    }

    pub fn add_to_playlist(&self, id: i64, tracks: &[Track]) -> anyhow::Result<usize> {
        self.attempt(|db| {
            let tx = db.unchecked_transaction()?;
            let at = now();
            let added = append(&tx, id, tracks.iter().map(|t| (t, at)))?;
            tx.commit()?;
            Ok(added)
        })
    }

    pub fn move_in_playlist(&self, id: i64, track: &TrackId, to: usize) -> bool {
        self.write(|db| {
            let tx = db.unchecked_transaction()?;
            let mut order: Vec<String> = tx
                .prepare("SELECT track_id FROM playlist_items WHERE playlist_id = ?1 ORDER BY position")?
                .query_map([id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            let Some(from) = order.iter().position(|t| track == t.as_str()) else { return Ok(false) };
            let moved = order.remove(from);
            order.insert(to.min(order.len()), moved);
            {
                let mut set = tx.prepare("UPDATE playlist_items SET position = ?3 WHERE playlist_id = ?1 AND track_id = ?2")?;
                for (position, t) in order.iter().enumerate() {
                    set.execute(params![id, t, position as i64])?;
                }
            }
            tx.commit()?;
            Ok(true)
        })
    }

    pub fn playlists_with(&self, track_id: &TrackId) -> Vec<i64> {
        self.run(|db| {
            let mut stmt = db.prepare_cached("SELECT playlist_id FROM playlist_items WHERE track_id = ?1 ORDER BY playlist_id")?;
            let found = stmt.query_map([track_id], |r| r.get(0))?;
            found.collect()
        })
    }

    pub fn remove_from_playlist(&self, id: i64, track_id: &TrackId) {
        self.write(|db| db.execute("DELETE FROM playlist_items WHERE playlist_id = ?1 AND track_id = ?2", params![id, track_id]).map(drop))
    }

    pub fn saved_albums(&self) -> Vec<AlbumCard> {
        self.run(|db| rows(db, "SELECT json FROM albums ORDER BY added_at DESC, rowid DESC", []))
    }

    /// Whether the album is saved; an error when the library cannot be read.
    pub fn album_saved(&self, id: &str) -> anyhow::Result<bool> {
        self.read(|db| is_saved(db, "albums", id))
    }

    pub fn toggle_album(&self, a: &AlbumCard) -> bool {
        self.write(|db| toggle_saved(db, "albums", &a.id, a))
    }

    pub fn saved_playlists(&self) -> Vec<AlbumCard> {
        self.run(|db| rows(db, "SELECT json FROM remote_playlists ORDER BY added_at DESC, rowid DESC", []))
    }

    /// Whether the YouTube playlist is saved; an error when the library cannot be read.
    pub fn playlist_saved(&self, id: &str) -> anyhow::Result<bool> {
        self.read(|db| is_saved(db, "remote_playlists", id))
    }

    pub fn toggle_playlist(&self, p: &AlbumCard) -> bool {
        self.write(|db| toggle_saved(db, "remote_playlists", &p.id, p))
    }

    pub fn followed(&self) -> Vec<ArtistCard> {
        self.run(|db| rows(db, "SELECT json FROM artists ORDER BY added_at DESC, rowid DESC", []))
    }

    pub fn toggle_follow(&self, a: &ArtistCard) -> bool {
        self.write(|db| toggle_saved(db, "artists", &a.id, a))
    }

    pub fn folders(&self) -> Vec<Folder> {
        self.run(|db| {
            let mut stmt = db.prepare_cached("SELECT id, name, open FROM folders ORDER BY id")?;
            let found = stmt.query_map([], |r| Ok(Folder { id: r.get(0)?, name: r.get(1)?, open: r.get(2)? }))?;
            found.collect()
        })
    }

    pub fn create_folder(&self, name: &str) -> anyhow::Result<i64> {
        self.attempt(|db| {
            db.execute("INSERT INTO folders (name, created_at) VALUES (?1, ?2)", params![name, now()])?;
            Ok(db.last_insert_rowid())
        })
    }

    pub fn rename_folder(&self, id: i64, name: &str) {
        self.write(|db| db.execute("UPDATE folders SET name = ?1 WHERE id = ?2", params![name, id]).map(drop))
    }

    pub fn open_folder(&self, id: i64, open: bool) {
        self.write(|db| db.execute("UPDATE folders SET open = ?1 WHERE id = ?2", params![open, id]).map(drop))
    }

    pub fn delete_folder(&self, id: i64) {
        self.write(|db| {
            let tx = db.unchecked_transaction()?;
            tx.execute("UPDATE playlists SET folder = NULL WHERE folder = ?1", [id])?;
            tx.execute("DELETE FROM folders WHERE id = ?1", [id])?;
            tx.commit()
        })
    }

    pub fn move_to_folder(&self, playlist: i64, folder: Option<i64>) -> bool {
        self.write(|db| {
            db.execute(
                "UPDATE playlists SET folder = ?2 WHERE id = ?1 AND (?2 IS NULL OR EXISTS (SELECT 1 FROM folders WHERE id = ?2))",
                params![playlist, folder],
            )
            .map(|n| n > 0)
        })
    }

    pub fn saved(&self) -> Vec<Dated<Pin>> {
        let mut out: Vec<Dated<Pin>> = Vec::new();
        let created: Vec<(i64, i64)> = self.run(|db| {
            let mut stmt = db.prepare_cached("SELECT id, created_at FROM playlists")?;
            let found = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            found.collect()
        });
        for p in self.playlists() {
            let at = created.iter().find(|(id, _)| *id == p.id).map_or(0, |(_, at)| *at);
            out.push(Dated {
                item: Pin { kind: "playlist".into(), id: p.id.to_string(), item: serde_json::to_value(&p).unwrap_or_default() },
                at,
            });
        }
        for (kind, table) in [("remote", "remote_playlists"), ("album", "albums"), ("artist", "artists")] {
            let rows: Vec<(String, String, i64)> = self.run(|db| {
                let mut stmt = db.prepare(&format!("SELECT id, json, added_at FROM {table}"))?;
                let found = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                found.collect()
            });
            for (id, json, at) in rows {
                if let Ok(item) = serde_json::from_str(&json) {
                    out.push(Dated { item: Pin { kind: kind.into(), id, item }, at });
                }
            }
        }
        out.sort_by_key(|d| std::cmp::Reverse(d.at));
        out
    }

    pub fn pins(&self) -> Vec<Pin> {
        self.run(|db| {
            let mut stmt = db.prepare_cached("SELECT kind, id, json FROM pins ORDER BY at, rowid")?;
            let found = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            Ok(found
                .filter_map(|row| {
                    let (kind, id, json) = row.ok()?;
                    Some(Pin { kind, id, item: serde_json::from_str(&json).ok()? })
                })
                .collect())
        })
    }

    pub fn toggle_pin(&self, pin: &Pin) -> bool {
        self.write(|db| {
            if db.execute("DELETE FROM pins WHERE kind = ?1 AND id = ?2", params![pin.kind, pin.id])? > 0 {
                return Ok(false);
            }
            db.execute(
                "INSERT INTO pins (kind, id, json, at) VALUES (?1, ?2, ?3, ?4)",
                params![pin.kind, pin.id, pin.item.to_string(), now()],
            )?;
            Ok(true)
        })
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.run(|db| db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional())
    }

    pub fn setting_read(&self, key: &str) -> anyhow::Result<Option<String>> {
        self.read(|db| db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional())
    }

    /// False when the setting could not be written (the person was told).
    pub fn set_setting(&self, key: &str, json: &str) -> bool {
        self.write(|db| {
            db.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, json],
            )
            .map(|_| true)
        })
    }
}
