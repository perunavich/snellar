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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_eleven_character_ids_are_accepted() {
        assert!(valid_id("dQw4w9WgXcQ") && valid_id("a-b_c-d_e-f"));
        for bad in ["", "short", "dQw4w9WgXcQx", "https://x.y/z", "dQw4w9 gXcQ", "dQw4w9WgXc\u{e9}", "yt:dQw4w9Wg"] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn a_bare_video_id_gains_the_prefix_and_a_prefixed_one_is_kept() {
        assert_eq!(TrackId::parse("dQw4w9WgXcQ"), "yt:dQw4w9WgXcQ");
        assert_eq!(TrackId::parse("yt:dQw4w9WgXcQ"), "yt:dQw4w9WgXcQ");
        assert_eq!(TrackId::parse("yt:dQw4w9WgXcQ").video_id(), Some("dQw4w9WgXcQ"));
        assert_eq!(TrackId::youtube("dQw4w9WgXcQ").to_string(), "yt:dQw4w9WgXcQ");
    }

    #[test]
    fn anything_else_is_kept_as_given_and_has_no_video_id() {
        for raw in ["", "short", "yt:short", "sc:dQw4w9WgXcQ", "YT:dQw4w9WgXcQ", "https://x.y/dQw4w9WgXcQ"] {
            let id = TrackId::parse(raw);
            assert_eq!(id, raw);
            assert_eq!(id.video_id(), None, "{raw}");
        }
    }

    #[test]
    fn json_reads_both_forms_and_writes_the_prefixed_one() {
        let read: Vec<TrackId> = serde_json::from_str(r#"["dQw4w9WgXcQ", "yt:dQw4w9WgXcQ", "odd"]"#).unwrap();
        assert_eq!(read, [TrackId::youtube("dQw4w9WgXcQ"), TrackId::youtube("dQw4w9WgXcQ"), TrackId::parse("odd")]);
        assert_eq!(serde_json::to_string(&read[0]).unwrap(), r#""yt:dQw4w9WgXcQ""#);
    }
}
