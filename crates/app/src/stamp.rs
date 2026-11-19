use std::sync::OnceLock;

pub struct Stamp {
    pub commit: &'static str,
    pub qt: String,
}

static STAMP: OnceLock<Stamp> = OnceLock::new();

pub fn set(stamp: Stamp) {
    STAMP.set(stamp).ok();
}

pub fn commit() -> &'static str {
    STAMP.get().map_or("unknown", |s| s.commit)
}

pub fn qt() -> &'static str {
    STAMP.get().map_or("", |s| s.qt.as_str())
}
