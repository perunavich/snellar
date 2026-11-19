use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

fn since_1970() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()
}

pub fn unix_secs() -> u64 {
    since_1970().as_secs()
}

pub fn unix_millis() -> u64 {
    since_1970().as_millis() as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub hour: u32,
    pub weekday: u32,
    pub day: i64,
}

pub fn local(millis: i64) -> Local {
    let secs = millis.div_euclid(1000) as libc::time_t;
    // SAFETY: an all-zero `tm` is a valid value; localtime_r only writes into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to live locals for the duration of the call.
    unsafe { libc::localtime_r(&secs, &mut tm) };
    Local { hour: tm.tm_hour as u32, weekday: tm.tm_wday as u32, day: i64::from(tm.tm_year) * 400 + i64::from(tm.tm_yday) }
}

#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> i64 + Send + Sync>);

impl Clock {
    pub fn system() -> Clock {
        Clock(Arc::new(|| unix_millis() as i64))
    }

    pub fn from_fn(now: impl Fn() -> i64 + Send + Sync + 'static) -> Clock {
        Clock(Arc::new(now))
    }

    pub fn millis(&self) -> i64 {
        (self.0)()
    }

    pub fn secs(&self) -> u64 {
        self.millis().max(0) as u64 / 1000
    }
}

impl Default for Clock {
    fn default() -> Clock {
        Clock::system()
    }
}
