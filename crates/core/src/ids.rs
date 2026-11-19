use std::fmt;

use rusqlite::types::{ToSql, ToSqlOutput};
use serde::{Deserialize, Deserializer, Serialize};

const YOUTUBE: &str = "yt:";

pub fn valid_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct TrackId(String);

impl TrackId {
    pub fn youtube(video_id: &str) -> TrackId {
        TrackId(format!("{YOUTUBE}{video_id}"))
    }

    pub fn parse(raw: &str) -> TrackId {
        if valid_id(raw) { TrackId::youtube(raw) } else { TrackId(raw.to_owned()) }
    }

    pub fn video_id(&self) -> Option<&str> {
        self.0.strip_prefix(YOUTUBE).filter(|v| valid_id(v))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for TrackId {
    fn from(raw: &str) -> TrackId {
        TrackId::parse(raw)
    }
}

impl<'de> Deserialize<'de> for TrackId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(|raw| TrackId::parse(&raw))
    }
}

impl PartialEq<str> for TrackId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for TrackId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl ToSql for TrackId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}
