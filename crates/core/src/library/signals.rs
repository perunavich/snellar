use std::path::Path;

use rusqlite::{Connection, OptionalExtension, ToSql, params};

use super::{Library, now};
use crate::error::{self, Code};
use crate::ids::TrackId;
use crate::model::{Track, Via};
use crate::signals::{Ended, Heard, Play};

type Step = fn(&Connection) -> rusqlite::Result<()>;

const STEPS: &[Step] = &[plays, loudness];

pub const LATEST: i32 = STEPS.len() as i32;

fn plays(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "
CREATE TABLE IF NOT EXISTS sig.plays (
    id INTEGER PRIMARY KEY,
    track_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    start REAL NOT NULL DEFAULT 0,
    heard REAL NOT NULL DEFAULT 0,
    furthest REAL NOT NULL DEFAULT 0,
    duration REAL NOT NULL DEFAULT 0,
    ended TEXT,
    via TEXT NOT NULL,
    position INTEGER,
    version INTEGER,
    chosen INTEGER NOT NULL DEFAULT 0,
    idle REAL NOT NULL DEFAULT 0,
    locked REAL NOT NULL DEFAULT 0,
    learn INTEGER NOT NULL DEFAULT 1,
    listen INTEGER
);
CREATE INDEX IF NOT EXISTS sig.plays_track ON plays (track_id);
CREATE INDEX IF NOT EXISTS sig.plays_listen ON plays (listen);
CREATE TABLE IF NOT EXISTS sig.unplayable (track_id TEXT PRIMARY KEY, code TEXT NOT NULL, at INTEGER NOT NULL);
",
    )
}

fn loudness(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS sig.loudness (track_id TEXT PRIMARY KEY, lufs REAL NOT NULL, seconds REAL NOT NULL, at INTEGER NOT NULL);")
}

fn version(db: &Connection) -> rusqlite::Result<i32> {
    db.pragma_query_value(Some("sig"), "user_version", |r| r.get(0))
}

fn migrate(db: &Connection) -> rusqlite::Result<()> {
    let found = version(db)?;
    if found > LATEST {
        return Err(rusqlite::Error::InvalidParameterName(format!("signals schema {found}, this build knows up to {LATEST}")));
    }
    for (target, step) in (1..).zip(STEPS) {
        if found < target {
            let tx = db.unchecked_transaction()?;
            step(&tx)?;
            tx.pragma_update(Some("sig"), "user_version", target)?;
            tx.commit()?;
        }
    }
    Ok(())
}

pub fn file_for(library: &Path) -> Option<std::path::PathBuf> {
    let real = library.file_name().is_some_and(|n| n != ":memory:");
    real.then(|| library.with_extension("signals.db"))
}

pub fn attach(db: &Connection, library: &Path) {
    let file = file_for(library);
    let on_disk = file.as_ref().and_then(|f| f.to_str()).map(|f| (f.to_string(), open(db, f)));
    match on_disk {
        Some((_, Ok(()))) => {}
        Some((file, Err(e))) => {
            crate::journal::warn("library", format!("{file}: play signals are kept in memory for this run: {e}"));
            let _ = db.execute("DETACH DATABASE sig", []);
            let _ = open(db, ":memory:");
        }
        None => {
            let _ = open(db, ":memory:");
        }
    }
}

fn open(db: &Connection, file: &str) -> rusqlite::Result<()> {
    db.execute("ATTACH DATABASE ?1 AS sig", [file])?;
    if file != ":memory:" {
        db.pragma_update(Some("sig"), "journal_mode", "WAL")?;
    }
    migrate(db)
}

pub struct Started<'a> {
    pub track: &'a Track,
    pub start: f64,
    pub chosen: bool,
}

const PLAY_COLUMNS: &str = "t.json, p.id, p.at, p.start, p.heard, p.furthest, p.duration, p.ended, p.via, p.position, p.version, p.chosen, p.idle, p.locked, p.learn";

fn read_play(r: &rusqlite::Row) -> rusqlite::Result<Option<Play>> {
    let json: String = r.get(0)?;
    let Ok(track) = serde_json::from_str::<Track>(&json) else { return Ok(None) };
    Ok(Some(Play {
        id: r.get(1)?,
        track,
        at: r.get(2)?,
        start: r.get(3)?,
        heard: r.get(4)?,
        furthest: r.get(5)?,
        duration: r.get(6)?,
        ended: r.get::<_, Option<String>>(7)?.map_or(Ended::Unknown, |e| Ended::parse(&e)),
        via: Via::parse(&r.get::<_, String>(8)?),
        position: r.get(9)?,
        version: r.get(10)?,
        chosen: r.get(11)?,
        idle: r.get(12)?,
        locked: r.get(13)?,
        learn: r.get(14)?,
        flagged: false,
    }))
}

impl Library {
    pub fn start_play(&self, s: &Started) -> Option<i64> {
        let pick = s.track.pick.clone().unwrap_or_default();
        self.write(|db| {
            db.execute(
                "INSERT INTO sig.plays (track_id, at, start, furthest, duration, via, position, version, chosen) VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    s.track.id,
                    now(),
                    s.start,
                    s.track.duration,
                    pick.via.name(),
                    s.track.pick.as_ref().map(|p| p.position),
                    s.track.pick.as_ref().map(|p| p.version),
                    s.chosen
                ],
            )?;
            Ok(Some(db.last_insert_rowid()))
        })
    }

    pub fn end_play(&self, id: Option<i64>, track: &TrackId, heard: &Heard, duration: f64, ended: Ended) {
        let skipped = ended == Ended::Skipped && heard.heard < crate::signals::EARLY_SKIP;
        let counted = ended != Ended::Failed;
        self.write(|db| {
            let tx = db.unchecked_transaction()?;
            let listen = if counted {
                tx.execute(
                    "INSERT INTO listens (track_id, seconds, skipped, at) VALUES (?1, ?2, ?3, ?4)",
                    params![track, heard.furthest, skipped, now()],
                )?;
                Some(tx.last_insert_rowid())
            } else {
                None
            };
            if let Some(id) = id {
                tx.execute(
                    "UPDATE sig.plays SET heard = ?2, furthest = ?3, duration = CASE WHEN ?4 > 0 THEN ?4 ELSE duration END,
                     ended = ?5, idle = ?6, locked = ?7, listen = ?8 WHERE id = ?1",
                    params![id, heard.heard, heard.furthest, duration, ended.name(), heard.idle, heard.locked, listen],
                )?;
            }
            tx.commit()
        })
    }

    pub fn plays(&self, limit: usize) -> Vec<Play> {
        self.run(|db| {
            let mut found = Vec::new();
            let mut stmt = db.prepare_cached(&format!(
                "SELECT {PLAY_COLUMNS} FROM sig.plays p JOIN tracks t ON t.id = p.track_id WHERE p.ended IS NOT NULL ORDER BY p.at DESC, p.id DESC LIMIT ?1"
            ))?;
            for play in stmt.query_map([limit as i64], read_play)? {
                found.extend(play?);
            }
            let mut stmt = db.prepare_cached(
                "SELECT t.json, l.seconds, l.skipped, l.at FROM listens l JOIN tracks t ON t.id = l.track_id
                 WHERE NOT EXISTS (SELECT 1 FROM sig.plays p WHERE p.listen = l.rowid) ORDER BY l.rowid DESC LIMIT ?1",
            )?;
            let legacy = stmt.query_map([limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            for row in legacy {
                let (json, seconds, skipped, at) = row?;
                if let Ok(track) = serde_json::from_str(&json) {
                    found.push(Play::legacy(track, seconds, skipped, at));
                }
            }
            found.sort_by_key(|p| std::cmp::Reverse(p.at));
            found.truncate(limit);
            Ok(found)
        })
    }

    /// Removes every play of a track: its history rows, its listens and its play signals, so nothing the recommendations
    /// read remembers it. The track row, likes and playlists stay.
    pub fn forget_track(&self, id: &TrackId) {
        self.write(|db| {
            let tx = db.unchecked_transaction()?;
            tx.execute("DELETE FROM history WHERE track_id = ?1", [id])?;
            tx.execute("DELETE FROM listens WHERE track_id = ?1", [id])?;
            tx.execute("DELETE FROM sig.plays WHERE track_id = ?1", [id])?;
            tx.commit()
        })
    }

    pub fn mark_unplayable(&self, id: &TrackId, code: Code) {
        self.write(|db| {
            db.execute("INSERT OR REPLACE INTO sig.unplayable (track_id, code, at) VALUES (?1, ?2, ?3)", params![id, code.as_str(), now()])
                .map(drop)
        })
    }

    /// Keeps the integrated loudness the player measured for a track, and how many seconds of it the measure covers.
    /// A measure that covers more of the track replaces a shorter one, whichever is written first.
    pub fn remember_loudness(&self, id: &TrackId, lufs: f64, seconds: f64) {
        self.write(|db| {
            db.execute(
                "INSERT INTO sig.loudness (track_id, lufs, seconds, at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (track_id) DO UPDATE SET lufs = excluded.lufs, seconds = excluded.seconds, at = excluded.at
                 WHERE excluded.seconds > loudness.seconds",
                params![id, lufs, seconds, now()],
            )
            .map(drop)
        })
    }

    /// The integrated loudness the player measured for a track, in LUFS.
    pub fn measured_loudness(&self, id: &TrackId) -> Option<f64> {
        self.run(|db| db.query_row("SELECT lufs FROM sig.loudness WHERE track_id = ?1", [id], |r| r.get(0)).optional())
    }

    pub fn unplayable(&self) -> Vec<TrackId> {
        let now = now();
        let rules: Vec<(&str, i64)> = error::ALL
            .iter()
            .filter_map(|(code, _)| code.refusal_lasts().map(|lasts| (code.as_str(), now.saturating_sub(lasts))))
            .collect();
        let sql = format!("SELECT track_id FROM sig.unplayable WHERE {}", vec!["(code = ? AND at > ?)"; rules.len()].join(" OR "));
        self.run(|db| {
            let mut stmt = db.prepare_cached(&sql)?;
            let values: Vec<&dyn ToSql> = rules.iter().flat_map(|(code, since)| [code as &dyn ToSql, since as &dyn ToSql]).collect();
            let ids = stmt.query_map(values.as_slice(), |r| Ok(TrackId::from(r.get::<_, String>(0)?.as_str())))?;
            ids.collect()
        })
    }
}
