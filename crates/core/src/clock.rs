use std::time::{SystemTime, UNIX_EPOCH};

pub fn stamp(at: SystemTime) -> String {
    let d = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        d.subsec_millis()
    )
}

pub fn file_stamp(at: SystemTime) -> String {
    let stamp: String = stamp(at)[..19].chars().filter(|c| c.is_ascii_digit() || *c == ' ').collect();
    stamp.replace(' ', "-")
}
