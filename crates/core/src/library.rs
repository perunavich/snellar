use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Params, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::clock;
use crate::error::Code;
use crate::ids::TrackId;
use crate::model::{AlbumCard, ArtistCard, Dated, Folder, Pin, Playlist, Track};
use crate::ui::Toast;

mod migrations;
mod recent;

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
    let mut last = last.lock().unwrap_or_else(PoisonError::into_inner);
    if last.is_some_and(|at| at.elapsed() < FAILURE_GAP) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
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
        *self.db.lock().unwrap_or_else(PoisonError::into_inner) = db;
        Ok(())
    }

    pub fn failed_reads(&self) -> u64 {
        self.failed_reads.load(Ordering::Relaxed)
    }

    pub fn read_failure(&self) -> Code {
        *self.read_failure.lock().unwrap_or_else(PoisonError::into_inner)
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
        let result = f(&self.db.lock().unwrap_or_else(PoisonError::into_inner));
        result.unwrap_or_else(|e| {
            self.log_failure(&e);
            *self.read_failure.lock().unwrap_or_else(PoisonError::into_inner) = Code::sqlite(&e);
            self.failed_reads.fetch_add(1, Ordering::Relaxed);
            T::default()
        })
    }

    fn write<T: Default>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> T {
        let result = f(&self.db.lock().unwrap_or_else(PoisonError::into_inner));
        result.unwrap_or_else(|e| {
            self.log_failure(&e);
            if let Some(sink) = self.sink.get().filter(|_| due(&self.told)) {
                sink(Toast { detail: e.to_string(), ..Toast::new(Code::sqlite(&e)) });
            }
            T::default()
        })
    }

    fn attempt<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> anyhow::Result<T> {
        let result = f(&self.db.lock().unwrap_or_else(PoisonError::into_inner));
        result.map_err(|e| {
            self.log_failure(&e);
            let code = Code::sqlite(&e);
            anyhow::Error::new(e).context(code)
        })
    }

    pub fn track(&self, id: &TrackId) -> Option<Track> {
        self.run(|db| db.query_row("SELECT json FROM tracks WHERE id = ?1", [id], |r| r.get::<_, String>(0)).optional())
            .and_then(|json| serde_json::from_str(&json).ok())
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

    pub fn fill_lengths(&self, tracks: &mut [Track]) {
        if tracks.iter().all(|t| t.duration > 0.0) {
            return;
        }
        self.run(|db| {
            let mut known = db.prepare_cached("SELECT json_extract(json, '$.duration') FROM tracks WHERE id = ?1 AND json_valid(json)")?;
            for track in tracks.iter_mut().filter(|t| t.duration <= 0.0) {
                let found: Option<f64> = known.query_row(params![track.id], |row| row.get(0)).optional()?.flatten();
                track.duration = found.unwrap_or(track.duration);
            }
            Ok(())
        })
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

    pub fn set_download(&self, id: &TrackId, path: &Path, loudness_db: Option<f64>) {
        self.write(|db| {
            db.execute(
                "INSERT OR REPLACE INTO downloads (track_id, path, loudness, at) VALUES (?1, ?2, ?3, ?4)",
                params![id, path.to_string_lossy(), loudness_db, now()],
            )
            .map(drop)
        })
    }

    pub fn download(&self, id: &TrackId) -> Option<(PathBuf, Option<f64>)> {
        self.run(|db| {
            db.query_row("SELECT path, loudness FROM downloads WHERE track_id = ?1", [id], |r| {
                Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?))
            })
            .optional()
        })
    }

    pub fn downloaded(&self) -> Vec<TrackId> {
        self.run(|db| {
            let mut stmt = db.prepare_cached("SELECT track_id, path FROM downloads")?;
            let rows =
                stmt.query_map([], |r| Ok((TrackId::from(r.get::<_, String>(0)?.as_str()), PathBuf::from(r.get::<_, String>(1)?))))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
        .into_iter()
        .filter(|(_, path)| path.is_file())
        .map(|(id, _)| id)
        .collect()
    }

    pub fn forget_download(&self, id: &TrackId) {
        self.write(|db| db.execute("DELETE FROM downloads WHERE track_id = ?1", [id]).map(drop))
    }

    pub fn clear_history(&self) {
        self.write(|db| db.execute_batch("DELETE FROM history; DELETE FROM listens;"))
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

    pub fn remove_from_playlist(&self, id: i64, track_id: &TrackId) {
        self.write(|db| db.execute("DELETE FROM playlist_items WHERE playlist_id = ?1 AND track_id = ?2", params![id, track_id]).map(drop))
    }

    pub fn saved_albums(&self) -> Vec<AlbumCard> {
        self.run(|db| rows(db, "SELECT json FROM albums ORDER BY added_at DESC, rowid DESC", []))
    }

    pub fn toggle_album(&self, a: &AlbumCard) -> bool {
        self.write(|db| toggle_saved(db, "albums", &a.id, a))
    }

    pub fn saved_playlists(&self) -> Vec<AlbumCard> {
        self.run(|db| rows(db, "SELECT json FROM remote_playlists ORDER BY added_at DESC, rowid DESC", []))
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

    pub fn set_setting(&self, key: &str, json: &str) {
        self.write(|db| {
            db.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, json],
            )
            .map(drop)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str) -> Track {
        Track { id: id.into(), title: format!("title {id}"), thumb: format!("thumb-{id}"), ..Track::default() }
    }

    fn lib() -> Library {
        Library::open(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn a_failed_read_is_counted_with_its_code() {
        let lib = Library::open(Path::new(":memory:")).unwrap();
        assert_eq!(lib.failed_reads(), 0);
        assert!(lib.liked().is_empty());
        assert_eq!(lib.failed_reads(), 0, "an empty list is not a failure");
        lib.db.lock().unwrap().execute_batch("ALTER TABLE likes RENAME TO likes_gone").unwrap();
        assert!(lib.liked().is_empty());
        assert_eq!(lib.failed_reads(), 1);
        assert_eq!(lib.read_failure(), Code::LibraryFailed);
    }

    #[test]
    fn a_damaged_file_or_a_newer_one_is_refused_and_left_as_it_was() {
        let dir = std::env::temp_dir().join(format!("snellar-open-refused-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let damaged = dir.join("damaged.db");
        std::fs::write(&damaged, vec![7u8; 8192]).unwrap();
        assert_eq!(Library::open(&damaged).err().unwrap().code(), Code::LibraryCorrupt);
        assert_eq!(std::fs::read(&damaged).unwrap(), vec![7u8; 8192]);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "nothing is created beside it");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_local_cover_is_stored_by_name_and_follows_the_data_directory() {
        let base = std::env::temp_dir().join(format!("snellar-library-cover-{}", std::process::id()));
        std::fs::remove_dir_all(&base).ok();
        let (one, two) = (base.join("one"), base.join("two"));
        std::fs::create_dir_all(&one).unwrap();
        let url = |dir: &Path, name: &str| format!("file://{}/covers/{name}", dir.display());
        let stored = |dir: &Path, id: i64| -> String {
            Connection::open(dir.join("library.db"))
                .unwrap()
                .query_row("SELECT cover FROM playlists WHERE id = ?1", [id], |r| r.get(0))
                .unwrap()
        };

        let l = Library::open(&one.join("library.db")).unwrap();
        let id = l.create_playlist("road").unwrap();
        l.set_playlist_cover(id, &url(&one, "a.jpg"));
        assert_eq!(stored(&one, id), "covers/a.jpg");
        let p = l.playlist(id).unwrap();
        assert_eq!((p.cover.as_str(), p.thumb.as_str()), (url(&one, "a.jpg").as_str(), url(&one, "a.jpg").as_str()));
        assert_eq!(l.playlists()[0].thumb, url(&one, "a.jpg"));
        let gone = l.delete_playlist(id).unwrap().unwrap();
        assert_eq!(l.restore_playlist(&gone).unwrap(), id);
        assert_eq!(stored(&one, id), "covers/a.jpg", "undo keeps the portable name");
        drop(l);

        std::fs::rename(&one, &two).unwrap();
        let l = Library::open(&two.join("library.db")).unwrap();
        assert_eq!(l.playlist(id).unwrap().thumb, url(&two, "a.jpg"));
        assert_eq!(l.playlists()[0].cover, url(&two, "a.jpg"));

        Connection::open(two.join("library.db"))
            .unwrap()
            .execute("UPDATE playlists SET cover = 'file:///home/old/.local/share/old-name/covers/b.png' WHERE id = ?1", [id])
            .unwrap();
        assert_eq!(l.playlist(id).unwrap().thumb, url(&two, "b.png"), "a row written before keeps working");

        l.set_playlist_cover(id, "https://i.ytimg.com/covers/x.jpg");
        assert_eq!(stored(&two, id), "https://i.ytimg.com/covers/x.jpg");
        assert_eq!(l.playlist(id).unwrap().thumb, "https://i.ytimg.com/covers/x.jpg");
        l.set_playlist_cover(id, "");
        assert_eq!(l.playlist(id).unwrap().cover, "");
        drop(l);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn like_toggles_and_lists_newest_first() {
        let l = lib();
        assert!(l.toggle_like(&track("a")));
        assert!(l.toggle_like(&track("b")));
        assert!(l.is_liked(&"a".into()));
        assert_eq!(l.favorites().iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert!(!l.toggle_like(&track("a")));
        assert!(!l.is_liked(&"a".into()));
        assert_eq!(l.favorites().len(), 1);
    }

    #[test]
    fn history_is_distinct_and_recent_first() {
        let l = lib();
        for id in ["a", "b", "a", "c", "b"] {
            l.push_history(&track(id));
        }
        assert_eq!(l.history(10).iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
        assert_eq!(l.history(2).len(), 2);
    }

    #[test]
    fn playlists_crud() {
        let l = lib();
        let id = l.create_playlist("road").unwrap();
        assert_eq!(l.add_to_playlist(id, &[track("a")]).unwrap(), 1);
        assert_eq!(l.add_to_playlist(id, &[track("b"), track("a")]).unwrap(), 1);
        assert_eq!(l.add_to_playlist(id, &[track("a")]).unwrap(), 0);
        let all = l.playlists();
        assert_eq!((all[0].name.as_str(), all[0].count, all[0].thumb.as_str()), ("road", 2, "thumb-a"));
        assert!(all[0].tracks.is_empty());
        let p = l.playlist(id).unwrap();
        assert_eq!(p.tracks.iter().map(|t| t.item.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        l.remove_from_playlist(id, &"a".into());
        l.rename_playlist(id, "home");
        let p = l.playlist(id).unwrap();
        assert_eq!((p.name.as_str(), p.count, p.thumb.as_str()), ("home", 1, "thumb-b"));
        l.delete_playlist(id).unwrap();
        assert!(l.playlist(id).is_none());
        assert!(l.playlists().is_empty());
    }

    #[test]
    fn recent_playlists_put_the_last_one_added_to_first() {
        let l = lib();
        let (a, b, c) = (l.create_playlist("a").unwrap(), l.create_playlist("b").unwrap(), l.create_playlist("c").unwrap());
        l.write(|db| db.execute_batch("UPDATE playlists SET created_at = 100"));
        l.add_to_playlist(a, &[track("x")]).unwrap();
        l.add_to_playlist(b, &[track("y")]).unwrap();
        l.write(|db| db.execute("UPDATE playlist_items SET added_at = 300 WHERE playlist_id = ?1", [a]).map(drop));
        l.write(|db| db.execute("UPDATE playlist_items SET added_at = 200 WHERE playlist_id = ?1", [b]).map(drop));
        assert_eq!(l.recent_playlists().iter().map(|p| p.id).collect::<Vec<_>>(), [a, b, c]);
        assert_eq!(l.playlists().iter().map(|p| p.id).collect::<Vec<_>>(), [a, b, c]);
        l.write(|db| db.execute("UPDATE playlist_items SET added_at = 400 WHERE playlist_id = ?1", [b]).map(drop));
        assert_eq!(l.recent_playlists().iter().map(|p| p.id).collect::<Vec<_>>(), [b, a, c]);
    }

    #[test]
    fn a_download_counts_only_while_its_file_exists() {
        let dir = std::env::temp_dir().join(format!("snellar-downloaded-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("song.opus");
        std::fs::write(&file, b"x").unwrap();
        let l = lib();
        let id: TrackId = "a".into();
        l.set_download(&id, &file, None);
        assert_eq!(l.downloaded(), std::slice::from_ref(&id));
        std::fs::remove_file(&file).unwrap();
        assert!(l.downloaded().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_deleted_playlist_comes_back_whole() {
        let l = lib();
        let first = l.create_playlist("first").unwrap();
        let id = l.create_playlist("road").unwrap();
        l.add_to_playlist(id, &[track("c"), track("a"), track("b")]).unwrap();
        let gone = l.delete_playlist(id).unwrap().unwrap();
        assert!(l.playlist(id).is_none() && l.delete_playlist(id).unwrap().is_none());
        assert_eq!(l.restore_playlist(&gone).unwrap(), id);
        let back = l.playlist(id).unwrap();
        assert_eq!((back.name.as_str(), back.tracks.iter().map(|t| t.item.id.as_str()).collect::<Vec<_>>()), ("road", vec!["c", "a", "b"]));
        assert_eq!(l.playlists().iter().map(|p| p.id).collect::<Vec<_>>(), [first, id], "back in its place");
        l.delete_playlist(first).unwrap();
        let taken = Playlist { id, ..gone };
        let other = l.restore_playlist(&taken).unwrap();
        assert_ne!(other, id, "an id in use gets a new one");
        assert_eq!(l.playlist(other).unwrap().count, 3);
    }

    #[test]
    fn dates_come_with_likes_history_and_playlist_items_and_survive_an_undo() {
        let l = lib();
        let before = now();
        l.toggle_like(&track("a"));
        l.push_history(&track("a"));
        let id = l.create_playlist("p").unwrap();
        l.add_to_playlist(id, &[track("a"), track("b")]).unwrap();
        assert!(l.liked()[0].at >= before && l.played(5)[0].at >= before);
        let json = serde_json::to_value(&l.liked()[0]).unwrap();
        assert_eq!((json["id"].as_str(), json["at"].as_i64()), (Some("a"), Some(l.liked()[0].at)));

        let gone: Playlist = serde_json::from_value(serde_json::to_value(l.delete_playlist(id).unwrap().unwrap()).unwrap()).unwrap();
        let at: Vec<i64> = gone.tracks.iter().map(|t| t.at).collect();
        assert!(at.iter().all(|&a| a >= before));
        std::thread::sleep(std::time::Duration::from_millis(5));
        l.restore_playlist(&gone).unwrap();
        assert_eq!(l.playlist(id).unwrap().tracks.iter().map(|t| t.at).collect::<Vec<_>>(), at, "an undo keeps when each was added");
    }

    #[test]
    fn a_track_moves_inside_its_playlist_and_stays_there() {
        let l = lib();
        let id = l.create_playlist("p").unwrap();
        l.add_to_playlist(id, &[track("a"), track("b"), track("c"), track("d")]).unwrap();
        l.remove_from_playlist(id, &"b".into());
        assert!(l.move_in_playlist(id, &"d".into(), 0));
        assert_eq!(listed(&l.playlist(id).unwrap()), ["d", "a", "c"]);
        assert!(l.move_in_playlist(id, &"d".into(), 9));
        assert_eq!(listed(&l.playlist(id).unwrap()), ["a", "c", "d"]);
        assert!(l.move_in_playlist(id, &"a".into(), 1));
        assert_eq!(listed(&l.playlist(id).unwrap()), ["c", "a", "d"]);
        assert!(!l.move_in_playlist(id, &"zz".into(), 0));
        l.add_to_playlist(id, &[track("e")]).unwrap();
        assert_eq!(listed(&l.playlist(id).unwrap()), ["c", "a", "d", "e"], "a new track still goes last");
        assert_eq!(l.playlists()[0].thumb, "thumb-c", "the cover follows the first track");
    }

    #[test]
    fn a_playlist_keeps_its_own_cover_and_description_through_an_undo() {
        let l = lib();
        let id = l.create_playlist("p").unwrap();
        l.add_to_playlist(id, &[track("a"), track("b")]).unwrap();
        l.set_playlist_cover(id, "file:///covers/p.png");
        l.describe_playlist(id, "for the road");
        let p = l.playlist(id).unwrap();
        assert_eq!(
            (p.thumb.as_str(), p.cover.as_str(), p.description.as_str()),
            ("file:///covers/p.png", "file:///covers/p.png", "for the road")
        );
        assert_eq!(l.playlists()[0].thumb, "file:///covers/p.png");
        let gone = l.delete_playlist(id).unwrap().unwrap();
        l.restore_playlist(&gone).unwrap();
        let back = l.playlist(id).unwrap();
        assert_eq!((back.cover.as_str(), back.description.as_str()), ("file:///covers/p.png", "for the road"));
        l.set_playlist_cover(id, "");
        assert_eq!(l.playlists()[0].thumb, "thumb-a", "no own cover: the first track's");
    }

    #[test]
    fn pins_toggle_and_keep_the_order_they_were_pinned_in() {
        let l = lib();
        let pin = |kind: &str, id: &str| Pin {
            kind: kind.into(),
            id: id.into(),
            item: serde_json::json!({ "id": id, "title": format!("t {id}") }),
        };
        assert!(l.toggle_pin(&pin("album", "MPREb_x")));
        assert!(l.toggle_pin(&pin("playlist", "3")));
        assert!(l.toggle_pin(&pin("artist", "3")));
        let found: Vec<(String, String)> = l.pins().into_iter().map(|p| (p.kind, p.id)).collect();
        assert_eq!(found, [("album".into(), "MPREb_x".into()), ("playlist".into(), "3".into()), ("artist".into(), "3".into())]);
        assert_eq!(l.pins()[0].item["title"], "t MPREb_x");
        assert!(!l.toggle_pin(&pin("playlist", "3")));
        assert_eq!(l.pins().len(), 2);
    }

    #[test]
    fn playlists_go_into_folders_and_out_again() {
        let l = lib();
        let road = l.create_playlist("road").unwrap();
        let gym = l.create_playlist("gym").unwrap();
        let mood = l.create_folder("moods").unwrap();
        assert!(l.move_to_folder(road, Some(mood)) && l.move_to_folder(gym, Some(mood)));
        assert!(!l.move_to_folder(road, Some(999)), "no such folder");
        l.rename_folder(mood, "Moods");
        l.open_folder(mood, false);
        assert_eq!(l.folders(), [Folder { id: mood, name: "Moods".into(), open: false }]);
        assert_eq!(l.playlists().iter().map(|p| p.folder).collect::<Vec<_>>(), [Some(mood), Some(mood)]);
        let gone = l.delete_playlist(gym).unwrap().unwrap();
        assert_eq!(l.playlist(l.restore_playlist(&gone).unwrap()).unwrap().folder, Some(mood), "an undo puts it back in its folder");
        assert!(l.move_to_folder(road, None));
        let gone = l.delete_playlist(gym).unwrap().unwrap();
        l.delete_folder(mood);
        assert!(l.folders().is_empty());
        assert_eq!(l.playlist(l.restore_playlist(&gone).unwrap()).unwrap().folder, None, "its folder was deleted meanwhile");
        assert_eq!(l.playlists().len(), 2, "deleting a folder keeps its playlists");
    }

    #[test]
    fn the_whole_library_comes_newest_first_with_its_kind() {
        let l = lib();
        let road = l.create_playlist("road").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        l.toggle_album(&AlbumCard { id: "MPREb_x".into(), title: "X".into(), ..AlbumCard::default() });
        std::thread::sleep(std::time::Duration::from_millis(3));
        l.toggle_follow(&ArtistCard { id: "UCq".into(), name: "Queen".into(), ..ArtistCard::default() });
        std::thread::sleep(std::time::Duration::from_millis(3));
        l.toggle_playlist(&AlbumCard { id: "PLx".into(), title: "Mix".into(), kind: "Playlist".into(), ..AlbumCard::default() });
        let all = l.saved();
        let kinds: Vec<(&str, &str)> = all.iter().map(|d| (d.item.kind.as_str(), d.item.id.as_str())).collect();
        assert_eq!(kinds, [("remote", "PLx"), ("artist", "UCq"), ("album", "MPREb_x"), ("playlist", road.to_string().as_str())]);
        assert_eq!(all[3].item.item["name"], "road");
        assert!(all.windows(2).all(|w| w[0].at >= w[1].at));
    }

    #[test]
    fn many_tracks_are_liked_or_unliked_at_once() {
        let l = lib();
        l.toggle_like(&track("a"));
        l.dislike(&track("c"));
        assert_eq!(l.set_liked(&[track("a"), track("b"), track("c")], true), 2, "a was already liked");
        assert_eq!(l.favorites().len(), 3);
        assert!(l.disliked().is_empty(), "a like takes the dislike back");
        assert_eq!(l.set_liked(&[track("a"), track("d")], false), 1);
        assert_eq!(l.favorites().len(), 2);
    }

    #[test]
    fn albums_and_settings() {
        let l = lib();
        let album = AlbumCard { id: "x".into(), title: "X".into(), ..AlbumCard::default() };
        assert!(l.toggle_album(&album));
        assert_eq!(l.saved_albums()[0].title, "X");
        assert!(!l.toggle_album(&album));
        assert!(l.saved_albums().is_empty());
        assert_eq!(l.setting("volume"), None);
        l.set_setting("volume", "40");
        l.set_setting("volume", "50");
        assert_eq!(l.setting("volume").as_deref(), Some("50"));
    }

    #[test]
    fn follow_toggles_and_lists_newest_first() {
        let l = lib();
        let card = |id: &str| ArtistCard { id: id.into(), name: format!("name {id}"), ..ArtistCard::default() };
        assert!(l.toggle_follow(&card("UCa")) && l.toggle_follow(&card("UCb")));
        assert_eq!(l.followed().iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["UCb", "UCa"]);
        assert!(!l.toggle_follow(&card("UCb")));
        assert_eq!(l.followed().iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["name UCa"]);
    }

    #[test]
    fn youtube_playlists_are_saved_apart_from_albums() {
        let l = lib();
        let list = AlbumCard { id: "PLx".into(), title: "Mix".into(), kind: "Playlist".into(), ..AlbumCard::default() };
        assert!(l.toggle_playlist(&list));
        assert_eq!(l.saved_playlists()[0].title, "Mix");
        assert!(l.saved_albums().is_empty());
        assert!(!l.toggle_playlist(&list) && l.saved_playlists().is_empty());
    }

    #[test]
    fn track_edits_are_upserted() {
        let l = lib();
        l.toggle_like(&track("a"));
        l.push_history(&Track { title: "new".into(), ..track("a") });
        assert_eq!(l.favorites()[0].title, "new");
        l.record_listen(&"a".into(), 12.0, true);
    }

    #[test]
    fn a_row_without_a_length_never_erases_a_known_one() {
        let l = lib();
        l.remember_track(&Track { duration: 182.5, ..track("a") });
        let shelf_row = Track { duration: 0.0, title: "new title".into(), ..track("a") };
        l.push_played(&shelf_row, None);
        l.toggle_like(&shelf_row);
        l.remember_track(&shelf_row);
        let kept = l.track(&"a".into()).unwrap();
        assert_eq!((kept.duration, kept.title.as_str()), (182.5, "new title"), "the rest of the row still updates");
        assert_eq!(l.history(1)[0].duration, 182.5);
        assert_eq!(l.favorites()[0].duration, 182.5);
        l.remember_track(&Track { duration: 200.0, ..track("a") });
        assert_eq!(l.track(&"a".into()).unwrap().duration, 200.0, "a new real length replaces it");
    }

    #[test]
    fn rows_without_a_length_take_the_one_the_library_knows() {
        let l = lib();
        l.remember_track(&Track { duration: 182.5, ..track("a") });
        l.remember_track(&Track { duration: 0.0, ..track("b") });
        let mut rows = vec![
            Track { duration: 0.0, ..track("a") },
            Track { duration: 0.0, ..track("b") },
            Track { duration: 0.0, ..track("c") },
            Track { duration: 9.0, ..track("a") },
        ];
        l.fill_lengths(&mut rows);
        assert_eq!(rows.iter().map(|t| t.duration).collect::<Vec<_>>(), [182.5, 0.0, 0.0, 9.0]);
    }

    #[test]
    fn clear_history_keeps_likes() {
        let l = lib();
        l.toggle_like(&track("a"));
        l.push_history(&track("a"));
        l.clear_history();
        assert!(l.history(10).is_empty());
        assert_eq!(l.favorites().len(), 1);
    }

    #[test]
    fn clear_history_also_clears_the_listen_log() {
        let l = lib();
        l.toggle_like(&track("a"));
        l.push_history(&track("a"));
        l.record_listen(&"a".into(), 120.0, false);
        l.dislike(&track("b"));
        l.clear_history();
        assert!(l.listens(10).is_empty());
        assert_eq!((l.favorites().len(), l.disliked().len()), (1, 1));
    }

    #[test]
    fn a_panic_inside_one_query_does_not_lock_everyone_out() {
        let l = lib();
        l.set_setting("k", "1");
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l.run(|_| -> rusqlite::Result<()> { panic!("boom") })));
        assert!(crashed.is_err());
        assert_eq!(l.setting("k").as_deref(), Some("1"));
    }

    fn refusing(name: &str, table: &str) -> (Library, PathBuf) {
        let dir = scratch(name);
        let path = dir.join("library.db");
        let l = Library::open(&path).unwrap();
        Connection::open(&path)
            .unwrap()
            .execute_batch(&format!("CREATE TRIGGER refuse BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'disk full'); END;"))
            .unwrap();
        (l, dir)
    }

    #[test]
    fn failed_writes_reach_the_person_once_and_again_only_after_a_while() {
        let (l, dir) = refusing("toast", "settings");
        let toasts = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = toasts.clone();
        l.on_write_failure(move |t| sink.lock().unwrap().push(t));
        for _ in 0..50 {
            l.set_setting("position", "1");
        }
        l.toggle_album(&AlbumCard { id: "x".into(), ..AlbumCard::default() });
        {
            let seen = toasts.lock().unwrap();
            assert_eq!(seen.len(), 1, "no toast storm: {seen:?}");
            assert_eq!(seen[0].code, Code::LibraryFailed);
            assert!(seen[0].detail.contains("disk full"));
        }
        *l.told.lock().unwrap() = Instant::now().checked_sub(FAILURE_GAP);
        l.set_setting("position", "2");
        assert_eq!(toasts.lock().unwrap().len(), 2, "a failure that goes on is told again");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_library_on_disk_uses_wal_with_normal_sync_and_history_is_one_transaction() {
        let (l, dir) = refusing("sync", "history");
        let db = l.db.lock().unwrap();
        assert_eq!(db.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(db.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0)).unwrap(), "wal");
        drop(db);
        l.push_history(&track("a"));
        assert!(l.track(&"a".into()).is_none(), "a history row that failed leaves no half of it behind");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writes_that_answer_with_an_id_fail_instead_of_answering_zero() {
        let (l, dir) = refusing("ids", "playlists");
        assert_eq!(Code::of(&l.create_playlist("road").unwrap_err()), Code::LibraryFailed);
        let (other, other_dir) = refusing("ids-items", "playlist_items");
        let id = other.create_playlist("road").unwrap();
        assert_eq!(Code::of(&other.add_to_playlist(id, &[track("a")]).unwrap_err()), Code::LibraryFailed);
        assert_eq!(other.playlist(id).unwrap().count, 0, "the transaction rolled back");
        assert_eq!(Code::of(&other.create_filled_playlist("mix", &[track("a")]).unwrap_err()), Code::LibraryFailed);
        assert_eq!(other.playlists().len(), 1, "a list that could not be filled is not left behind empty");
        let (folders, folders_dir) = refusing("ids-folders", "folders");
        assert_eq!(Code::of(&folders.create_folder("f").unwrap_err()), Code::LibraryFailed);
        for d in [dir, other_dir, folders_dir] {
            std::fs::remove_dir_all(&d).ok();
        }
    }

    #[test]
    fn a_delete_that_fails_keeps_the_playlist_and_says_so() {
        let dir = scratch("delete");
        let path = dir.join("library.db");
        let l = Library::open(&path).unwrap();
        let id = l.create_playlist("road").unwrap();
        l.add_to_playlist(id, &[track("a"), track("b")]).unwrap();
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TRIGGER refuse BEFORE DELETE ON playlists BEGIN SELECT RAISE(ABORT, 'disk full'); END;")
            .unwrap();
        assert_eq!(Code::of(&l.delete_playlist(id).unwrap_err()), Code::LibraryFailed, "no undo snapshot for a delete that did not happen");
        assert_eq!(l.playlist(id).unwrap().count, 2, "its items were not removed without it");
        std::fs::remove_dir_all(&dir).ok();
    }

    const MAIN_SCHEMA: &str = "
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS tracks (id TEXT PRIMARY KEY, json TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS likes (track_id TEXT PRIMARY KEY, added_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS history (track_id TEXT NOT NULL, played_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS listens (track_id TEXT NOT NULL, seconds REAL NOT NULL, skipped INTEGER NOT NULL, at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS dislikes (track_id TEXT PRIMARY KEY, at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS playlists (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS playlist_items (
    playlist_id INTEGER NOT NULL, track_id TEXT NOT NULL, position INTEGER NOT NULL, added_at INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, track_id)
);
CREATE TABLE IF NOT EXISTS albums (id TEXT PRIMARY KEY, json TEXT NOT NULL, added_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS history_track ON history (track_id);
";

    const TABLES: [&str; 9] = ["tracks", "likes", "history", "listens", "dislikes", "playlists", "playlist_items", "albums", "settings"];

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-migrate-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn unversioned(path: &Path, schema: &str) {
        let db = Connection::open(path).unwrap();
        db.execute_batch(schema).unwrap();
        let a = serde_json::to_string(&track("a")).unwrap();
        let b = serde_json::to_string(&track("b")).unwrap();
        db.execute("INSERT INTO tracks (id, json) VALUES ('a', ?1), ('b', ?2)", [a, b]).unwrap();
        db.execute_batch(
            "INSERT INTO likes VALUES ('a', 1), ('b', 2);
             INSERT INTO history VALUES ('a', 10), ('b', 11), ('a', 12);
             INSERT INTO listens VALUES ('a', 120.5, 0, 10), ('b', 3.0, 1, 11);
             INSERT INTO playlists (name, created_at) VALUES ('road', 5);
             INSERT INTO playlist_items VALUES (1, 'b', 0, 6), (1, 'a', 1, 7);
             INSERT INTO albums VALUES ('x', '{\"id\":\"x\",\"title\":\"X\"}', 8);
             INSERT INTO settings VALUES ('volume', '40'), ('handoff', 'true'), ('queue', '{\"tracks\":[],\"cursor\":0}');",
        )
        .unwrap();
        if schema.contains("dislikes") {
            db.execute("INSERT INTO dislikes VALUES ('b', 9)", []).unwrap();
        }
    }

    fn counts(path: &Path) -> Vec<i64> {
        let db = Connection::open(path).unwrap();
        TABLES.iter().map(|t| db.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0)).unwrap()).collect()
    }

    fn user_version(path: &Path) -> i32 {
        migrations::version(&Connection::open(path).unwrap()).unwrap()
    }

    fn backups(dir: &Path) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_str().unwrap().starts_with("library.db.backup-"))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn an_unversioned_library_is_backed_up_and_migrated_without_loss() {
        let dir = scratch("unversioned");
        let path = dir.join("library.db");
        unversioned(&path, MAIN_SCHEMA);
        let before = counts(&path);
        assert_eq!(before, [2, 2, 3, 2, 1, 1, 2, 1, 3]);

        let l = Library::open(&path).unwrap();
        assert_eq!(l.favorites().iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(l.history(10).len(), 2);
        assert_eq!(l.listens(10).len(), 2);
        assert_eq!(l.disliked()[0].id, "b");
        assert_eq!(l.playlist(1).unwrap().tracks.iter().map(|t| t.item.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(l.saved_albums()[0].title, "X");
        assert_eq!(l.setting("handoff").as_deref(), Some("true"));
        drop(l);

        assert_eq!(counts(&path), before);
        assert_eq!(user_version(&path), migrations::LATEST);
        let saved = backups(&dir);
        assert_eq!(saved.len(), 1);
        assert!(saved[0].to_str().unwrap().contains(".backup-v0-"));
        assert_eq!(counts(&saved[0]), before);
        assert_eq!(user_version(&saved[0]), 0);

        Library::open(&path).unwrap();
        assert_eq!(backups(&dir), saved, "a current library is not backed up again");
        assert_eq!(counts(&path), before);
        assert_eq!(user_version(&path), migrations::LATEST);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_dislike_can_be_taken_back() {
        let l = lib();
        l.toggle_like(&track("a"));
        l.dislike(&track("a"));
        l.dislike(&track("b"));
        assert_eq!((l.favorites().len(), l.disliked().len()), (0, 2));
        l.undislike(&"a".into());
        assert_eq!(l.disliked().iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert!(l.favorites().is_empty(), "taking a dislike back does not like it");
    }

    #[test]
    fn a_library_from_before_dislikes_gains_the_table() {
        let dir = scratch("pre-dislikes");
        let path = dir.join("library.db");
        let old = MAIN_SCHEMA.replace("CREATE TABLE IF NOT EXISTS dislikes (track_id TEXT PRIMARY KEY, at INTEGER NOT NULL);", "");
        unversioned(&path, &old);
        let l = Library::open(&path).unwrap();
        assert!(l.disliked().is_empty());
        l.dislike(&track("a"));
        assert_eq!(l.disliked().len(), 1);
        assert_eq!(l.favorites().len(), 1);
        assert_eq!(backups(&dir).len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_v2_library_gains_followed_artists_and_saved_playlists_without_losing_rows() {
        let dir = scratch("v2");
        let path = dir.join("library.db");
        unversioned(&path, MAIN_SCHEMA);
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 2).unwrap();
        let before = counts(&path);

        let l = Library::open(&path).unwrap();
        assert!(l.followed().is_empty() && l.saved_playlists().is_empty());
        assert!(l.toggle_playlist(&AlbumCard { id: "PLq".into(), ..AlbumCard::default() }));
        assert!(l.toggle_follow(&ArtistCard { id: "UCq".into(), name: "Queen".into(), ..ArtistCard::default() }));
        assert_eq!(l.favorites().len(), 2);
        drop(l);

        assert_eq!(counts(&path), before);
        assert_eq!(user_version(&path), migrations::LATEST);
        let saved = backups(&dir);
        assert_eq!(saved.len(), 1);
        assert!(saved[0].to_str().unwrap().contains(".backup-v2-"));
        assert_eq!((counts(&saved[0]), user_version(&saved[0])), (before, 2));
        assert_eq!(Library::open(&path).unwrap().followed()[0].name, "Queen");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_v4_library_gains_playlist_details_pins_and_folders_without_losing_rows() {
        let dir = scratch("v4");
        let path = dir.join("library.db");
        let v4 = format!(
            "{MAIN_SCHEMA}CREATE TABLE artists (id TEXT PRIMARY KEY, json TEXT NOT NULL, added_at INTEGER NOT NULL);
             CREATE TABLE remote_playlists (id TEXT PRIMARY KEY, json TEXT NOT NULL, added_at INTEGER NOT NULL);"
        );
        unversioned(&path, &v4);
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 4).unwrap();
        let before = counts(&path);

        let l = Library::open(&path).unwrap();
        let p = l.playlist(1).unwrap();
        assert_eq!((p.name.as_str(), p.description.as_str(), p.cover.as_str(), p.thumb.as_str()), ("road", "", "", "thumb-b"));
        l.describe_playlist(1, "old songs");
        assert!(l.pins().is_empty());
        assert!(l.folders().is_empty() && l.playlist(1).unwrap().folder.is_none());
        l.toggle_pin(&Pin { kind: "playlist".into(), id: "1".into(), item: serde_json::json!({ "id": 1 }) });
        drop(l);

        assert_eq!(counts(&path), before);
        assert_eq!(user_version(&path), migrations::LATEST);
        let saved = backups(&dir);
        assert_eq!(saved.len(), 1);
        assert!(saved[0].to_str().unwrap().contains(".backup-v4-"));
        let again = Library::open(&path).unwrap();
        assert_eq!((again.playlist(1).unwrap().description.as_str(), again.pins().len()), ("old songs", 1));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_v7_library_gains_downloads_without_losing_rows() {
        let dir = scratch("v7");
        let path = dir.join("library.db");
        let v7 = format!(
            "{MAIN_SCHEMA}CREATE TABLE artists (id TEXT PRIMARY KEY, json TEXT NOT NULL, added_at INTEGER NOT NULL);
             CREATE TABLE remote_playlists (id TEXT PRIMARY KEY, json TEXT NOT NULL, added_at INTEGER NOT NULL);
             ALTER TABLE playlists ADD COLUMN description TEXT NOT NULL DEFAULT '';
             ALTER TABLE playlists ADD COLUMN cover TEXT NOT NULL DEFAULT '';
             CREATE TABLE pins (kind TEXT NOT NULL, id TEXT NOT NULL, json TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (kind, id));
             CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, open INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL);
             ALTER TABLE playlists ADD COLUMN folder INTEGER;"
        );
        unversioned(&path, &v7);
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 7).unwrap();
        let before = counts(&path);

        let l = Library::open(&path).unwrap();
        let id = TrackId::youtube("dQw4w9WgXcQ");
        assert_eq!(l.download(&id), None);
        l.set_download(&id, Path::new("/music/Rick Astley - Never.opus"), Some(0.98));
        assert_eq!(l.download(&id), Some((PathBuf::from("/music/Rick Astley - Never.opus"), Some(0.98))));
        l.set_download(&id, Path::new("/music/again.opus"), None);
        assert_eq!(l.download(&id), Some((PathBuf::from("/music/again.opus"), None)), "a second download replaces the first");
        l.forget_download(&id);
        assert_eq!(l.download(&id), None);
        drop(l);

        assert_eq!(counts(&path), before);
        assert_eq!(user_version(&path), migrations::LATEST);
        assert!(backups(&dir)[0].to_str().unwrap().contains(".backup-v7-"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_v8_library_gains_play_contexts_and_old_plays_still_fill_the_recent_shelf() {
        let dir = scratch("v8");
        let path = dir.join("library.db");
        let v8 = format!(
            "{MAIN_SCHEMA}CREATE TABLE artists (id TEXT PRIMARY KEY, json TEXT NOT NULL, added_at INTEGER NOT NULL);
             CREATE TABLE remote_playlists (id TEXT PRIMARY KEY, json TEXT NOT NULL, added_at INTEGER NOT NULL);
             ALTER TABLE playlists ADD COLUMN description TEXT NOT NULL DEFAULT '';
             ALTER TABLE playlists ADD COLUMN cover TEXT NOT NULL DEFAULT '';
             CREATE TABLE pins (kind TEXT NOT NULL, id TEXT NOT NULL, json TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (kind, id));
             CREATE TABLE folders (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, open INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL);
             ALTER TABLE playlists ADD COLUMN folder INTEGER;
             CREATE TABLE downloads (track_id TEXT PRIMARY KEY, path TEXT NOT NULL, loudness REAL, at INTEGER NOT NULL);"
        );
        unversioned(&path, &v8);
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 8).unwrap();
        let before = counts(&path);

        let l = Library::open(&path).unwrap();
        assert_eq!(counts(&path), before, "no row is lost");
        assert_eq!(l.history(10).iter().map(|t| t.id.to_string()).collect::<Vec<_>>(), ["a", "b"], "the plays are kept in order");
        assert_eq!(l.recent(10).iter().map(|p| (p.kind.as_str(), p.id.as_str())).collect::<Vec<_>>(), [("track", "a"), ("track", "b")]);

        let album = Pin { kind: "album".into(), id: "al".into(), item: serde_json::json!({ "id": "al", "title": "Al" }) };
        l.push_played(&track("c"), Some(&album));
        l.push_played(&track("d"), Some(&album));
        l.push_history(&track("e"));
        let shown = l.recent(10);
        assert_eq!(shown.iter().map(|p| (p.kind.as_str(), p.id.as_str())).collect::<Vec<_>>()[0], ("album", "al"));
        assert_eq!(shown.iter().filter(|p| p.kind == "album").count(), 1, "one card, not one per play");
        l.clear_history();
        assert!(l.recent(10).is_empty(), "clearing the history clears the shelf");
        drop(l);

        assert_eq!(user_version(&path), migrations::LATEST);
        assert!(backups(&dir)[0].to_str().unwrap().contains(".backup-v8-"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_recent_shelf_reads_a_long_history_quickly() {
        let l = Library::open(Path::new(":memory:")).unwrap();
        for i in 0..5000 {
            let t = Track {
                album: Some(crate::model::Ref { id: format!("al{}", i % 50), name: "Album".into() }),
                ..track(&format!("t{}", i % 800))
            };
            l.push_history(&t);
        }
        let started = Instant::now();
        let shown = l.recent(20);
        assert!(started.elapsed() < Duration::from_millis(100), "{:?}", started.elapsed());
        assert_eq!(shown.len(), 20);
    }

    #[test]
    fn tracks_in_several_playlists_count_once_newest_first() {
        let l = Library::open(Path::new(":memory:")).unwrap();
        let first = l.create_filled_playlist("first", &[track("a"), track("b")]).unwrap();
        l.add_to_playlist(first, &[]).unwrap();
        assert_eq!(l.playlisted(10).iter().map(|t| t.id.to_string()).collect::<Vec<_>>(), ["a", "b"]);
        l.create_filled_playlist("second", &[track("b")]).unwrap();
        assert_eq!(l.playlisted(10).len(), 2);
        assert_eq!(l.playlisted(1).len(), 1);
    }

    #[test]
    fn a_new_library_starts_current_without_a_backup() {
        let dir = scratch("fresh");
        let path = dir.join("library.db");
        Library::open(&path).unwrap().set_setting("volume", "40");
        assert_eq!(user_version(&path), migrations::LATEST);
        assert!(backups(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_newer_library_is_refused_and_left_untouched() {
        let dir = scratch("newer");
        let path = dir.join("library.db");
        unversioned(&path, MAIN_SCHEMA);
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "user_version", migrations::LATEST + 1).unwrap();
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        drop(db);
        let bytes = std::fs::read(&path).unwrap();

        let err = Library::open(&path).err().unwrap();
        assert!(matches!(err, OpenError::Newer { found, known } if found == migrations::LATEST + 1 && known == migrations::LATEST));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(backups(&dir).is_empty());
        assert_eq!(user_version(&path), migrations::LATEST + 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_bare_stub_never_overwrites_real_metadata() {
        let l = lib();
        l.toggle_like(&track("a"));
        l.push_history(&Track { id: "a".into(), title: "a".into(), ..Track::default() });
        l.add_to_playlist(l.create_playlist("p").unwrap(), &[Track { id: "a".into(), title: "a".into(), ..Track::default() }]).unwrap();
        assert_eq!(l.track(&"a".into()).unwrap().title, "title a");
        assert_eq!(l.favorites()[0].thumb, "thumb-a");
        assert!(l.track(&"missing".into()).is_none());
    }

    const V1_FIXTURE: &str = include_str!("../../../tests/fixtures/library-v1.sql");

    fn ids(tracks: &[Track]) -> Vec<&str> {
        tracks.iter().map(|t| t.id.as_str()).collect()
    }

    fn listed(p: &Playlist) -> Vec<&str> {
        p.tracks.iter().map(|t| t.item.id.as_str()).collect()
    }

    #[test]
    fn a_v1_library_gets_prefixed_track_ids_without_losing_rows() {
        let dir = scratch("v1");
        let path = dir.join("library.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(V1_FIXTURE).unwrap();
        db.execute("INSERT INTO tracks VALUES ('aaaaaaaaaaa', 'not json')", []).unwrap();
        drop(db);
        let before = counts(&path);
        assert_eq!(before, [7, 4, 7, 5, 1, 2, 5, 1, 5]);

        let l = Library::open(&path).unwrap();
        assert_eq!(ids(&l.favorites()), ["legacy-row", "yt:-tJYN-eG1zk", "yt:5NV6Rdv1a3I", "yt:dQw4w9WgXcQ"]);
        assert!(l.is_liked(&TrackId::youtube("dQw4w9WgXcQ")) && l.is_liked(&"dQw4w9WgXcQ".into()));
        assert_eq!(ids(&l.history(10)), ["yt:fHI8X4OXluQ", "yt:-tJYN-eG1zk", "yt:sElE_BfQ67s", "yt:dQw4w9WgXcQ", "yt:5NV6Rdv1a3I"]);
        assert_eq!(l.listens(10).len(), 5);
        assert_eq!(ids(&l.disliked()), ["yt:sElE_BfQ67s"]);
        assert_eq!(listed(&l.playlist(1).unwrap()), ["yt:fHI8X4OXluQ", "yt:dQw4w9WgXcQ", "yt:-tJYN-eG1zk"]);
        assert_eq!(listed(&l.playlist(2).unwrap()), ["yt:5NV6Rdv1a3I", "legacy-row"]);
        assert_eq!(l.track(&TrackId::youtube("-tJYN-eG1zk")).unwrap().title, "We Will Rock You");
        assert_eq!(l.saved_albums()[0].id, "MPREb_ram");
        let queue: serde_json::Value = serde_json::from_str(&l.setting("queue").unwrap()).unwrap();
        let queued: Vec<&str> = queue["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!((queued, queue["cursor"].as_i64()), (vec!["yt:fHI8X4OXluQ", "yt:dQw4w9WgXcQ", "yt:5NV6Rdv1a3I"], Some(1)));
        assert_eq!(l.setting("position").as_deref(), Some("42.5"));
        drop(l);

        assert_eq!(counts(&path), before);
        assert_eq!(user_version(&path), migrations::LATEST);
        let db = Connection::open(&path).unwrap();
        for (table, column) in [
            ("tracks", "id"),
            ("likes", "track_id"),
            ("dislikes", "track_id"),
            ("history", "track_id"),
            ("listens", "track_id"),
            ("playlist_items", "track_id"),
        ] {
            let bare: i64 = db
                .query_row(&format!("SELECT COUNT(*) FROM {table} WHERE {column} NOT LIKE 'yt:%' AND {column} != 'legacy-row'"), [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(bare, 0, "{table}");
        }
        let mismatched: i64 = db
            .query_row("SELECT COUNT(*) FROM tracks WHERE json_valid(json) AND json_extract(json, '$.id') != id", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mismatched, 0);
        let kept: String = db.query_row("SELECT json FROM tracks WHERE id = 'yt:aaaaaaaaaaa'", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, "not json");

        let saved = backups(&dir);
        assert_eq!(saved.len(), 1);
        assert!(saved[0].to_str().unwrap().contains(".backup-v1-"));
        assert_eq!((counts(&saved[0]), user_version(&saved[0])), (before, 1));
        let old: i64 = Connection::open(&saved[0])
            .unwrap()
            .query_row("SELECT COUNT(*) FROM likes WHERE track_id LIKE 'yt:%'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(old, 0, "the backup keeps the bare ids");
        std::fs::remove_dir_all(&dir).ok();
    }
}
