use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;

use crate::ids::TrackId;

type Step = fn(&Connection) -> rusqlite::Result<()>;

const MIGRATIONS: &[Step] =
    &[schema, prefix_track_ids, followed_artists, saved_remote_playlists, playlist_details, pins, folders, downloads, play_context];

fn schema(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "
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
",
    )
}

const TRACK_ID_TABLES: [&str; 5] = ["likes", "dislikes", "history", "listens", "playlist_items"];

fn prefix_track_ids(db: &Connection) -> rusqlite::Result<()> {
    for table in TRACK_ID_TABLES {
        let rows: Vec<(i64, String)> = db
            .prepare(&format!("SELECT rowid, track_id FROM {table}"))?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut update = db.prepare(&format!("UPDATE {table} SET track_id = ?2 WHERE rowid = ?1"))?;
        for (rowid, old) in rows {
            let new = TrackId::parse(&old);
            if new != *old {
                update.execute(params![rowid, new])?;
            }
        }
    }
    let rows: Vec<(i64, String, String)> = db
        .prepare("SELECT rowid, id, json FROM tracks")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut update = db.prepare("UPDATE tracks SET id = ?2, json = ?3 WHERE rowid = ?1")?;
    for (rowid, old, json) in rows {
        let id = TrackId::parse(&old);
        let mut track: Value = serde_json::from_str(&json).unwrap_or(Value::Null);
        let prefixed = prefix_in(&mut track);
        if prefixed || id != *old {
            update.execute(params![rowid, id, if prefixed { track.to_string() } else { json }])?;
        }
    }
    let queue: Option<String> = db.query_row("SELECT value FROM settings WHERE key = 'queue'", [], |r| r.get(0)).optional()?;
    if let Some(Ok(mut saved)) = queue.map(|q| serde_json::from_str::<Value>(&q)) {
        let mut changed = false;
        for track in saved.get_mut("tracks").and_then(Value::as_array_mut).into_iter().flatten() {
            changed |= prefix_in(track);
        }
        if changed {
            db.execute("UPDATE settings SET value = ?1 WHERE key = 'queue'", [saved.to_string()])?;
        }
    }
    Ok(())
}

fn prefix_in(track: &mut Value) -> bool {
    let Some(id) = track.get_mut("id") else { return false };
    let Some(old) = id.as_str() else { return false };
    let new = TrackId::parse(old);
    if new == *old {
        return false;
    }
    *id = Value::String(new.to_string());
    true
}

fn followed_artists(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS artists (id TEXT PRIMARY KEY, json TEXT NOT NULL, added_at INTEGER NOT NULL);")
}

fn saved_remote_playlists(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS remote_playlists (id TEXT PRIMARY KEY, json TEXT NOT NULL, added_at INTEGER NOT NULL);")
}

fn playlist_details(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "ALTER TABLE playlists ADD COLUMN description TEXT NOT NULL DEFAULT '';
         ALTER TABLE playlists ADD COLUMN cover TEXT NOT NULL DEFAULT '';",
    )
}

fn pins(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS pins (kind TEXT NOT NULL, id TEXT NOT NULL, json TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (kind, id));")
}

fn folders(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS folders (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, open INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL);
         ALTER TABLE playlists ADD COLUMN folder INTEGER;",
    )
}

fn downloads(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS downloads (track_id TEXT PRIMARY KEY, path TEXT NOT NULL, loudness REAL, at INTEGER NOT NULL);",
    )
}

fn play_context(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("ALTER TABLE history ADD COLUMN context TEXT;")
}

pub const LATEST: i32 = MIGRATIONS.len() as i32;

pub fn version(db: &Connection) -> rusqlite::Result<i32> {
    db.pragma_query_value(None, "user_version", |r| r.get(0))
}

pub fn apply(db: &mut Connection) -> rusqlite::Result<()> {
    for (target, step) in (1..).zip(MIGRATIONS) {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if version(&tx)? < target {
            step(&tx)?;
            tx.pragma_update(None, "user_version", target)?;
        }
        tx.commit()?;
    }
    Ok(())
}
