use serde::{Deserialize, Serialize};

pub mod daypart;
mod report;

pub use report::{Line, report};

use crate::model::{Track, Via};

pub const EARLY_SKIP: f64 = 30.0;
pub const COMPLETE: f64 = 0.8;
pub const IDLE_WEIGHT: f64 = 0.5;
pub const LOCKED_WEIGHT: f64 = 0.1;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Ended {
    #[default]
    Unknown,
    Finished,
    Skipped,
    Replaced,
    Failed,
    Quit,
    Halted,
    Legacy,
}

impl Ended {
    const ALL: [Ended; 8] =
        [Ended::Unknown, Ended::Finished, Ended::Skipped, Ended::Replaced, Ended::Failed, Ended::Quit, Ended::Halted, Ended::Legacy];

    pub fn name(self) -> &'static str {
        match self {
            Ended::Unknown => "unknown",
            Ended::Finished => "finished",
            Ended::Skipped => "skipped",
            Ended::Replaced => "replaced",
            Ended::Failed => "failed",
            Ended::Quit => "quit",
            Ended::Halted => "halted",
            Ended::Legacy => "legacy",
        }
    }

    pub fn parse(name: &str) -> Ended {
        Ended::ALL.into_iter().find(|e| e.name() == name).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presence {
    #[default]
    Unknown,
    Present,
    Idle,
    Locked,
}

impl Presence {
    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn of(code: u8) -> Presence {
        match code {
            1 => Presence::Present,
            2 => Presence::Idle,
            3 => Presence::Locked,
            _ => Presence::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Heard {
    pub start: f64,
    pub heard: f64,
    pub furthest: f64,
    pub idle: f64,
    pub locked: f64,
    last: Option<f64>,
}

const STEP: f64 = 2.0;

impl Heard {
    pub fn from(start: f64) -> Heard {
        Heard { start, furthest: start, last: Some(start), ..Heard::default() }
    }

    pub fn tick(&mut self, position: f64, presence: Presence) {
        let step = self.last.map(|last| position - last).filter(|d| *d > 0.0 && *d <= STEP);
        if let Some(d) = step {
            self.heard += d;
            match presence {
                Presence::Idle => self.idle += d,
                Presence::Locked => self.locked += d,
                Presence::Present | Presence::Unknown => {}
            }
        }
        self.last = Some(position);
        self.furthest = self.furthest.max(position);
    }

    pub fn jumped(&mut self, position: f64) {
        self.last = Some(position);
        self.furthest = self.furthest.max(position);
    }
}

#[derive(Clone, Debug, Default)]
pub struct Play {
    pub id: Option<i64>,
    pub track: Track,
    pub at: i64,
    pub start: f64,
    pub heard: f64,
    pub furthest: f64,
    pub duration: f64,
    pub ended: Ended,
    pub via: Via,
    pub position: Option<u32>,
    pub version: Option<u32>,
    pub chosen: bool,
    pub idle: f64,
    pub locked: f64,
    pub learn: bool,
    pub flagged: bool,
}

impl Play {
    pub fn length(&self) -> f64 {
        if self.duration > 0.0 { self.duration } else { self.track.duration }
    }

    pub fn completion(&self) -> Option<f64> {
        let left = self.length() - self.start;
        (left > 0.0).then(|| (self.heard / left).min(1.0))
    }

    pub fn skipped(&self) -> bool {
        match self.ended {
            Ended::Skipped => true,
            Ended::Legacy => self.flagged,
            _ => false,
        }
    }

    pub fn early_skip(&self) -> bool {
        self.skipped() && self.heard < EARLY_SKIP
    }

    pub fn finished(&self) -> bool {
        if self.ended == Ended::Legacy && self.flagged {
            return false;
        }
        match self.completion() {
            Some(c) => c >= COMPLETE,
            None => matches!(self.ended, Ended::Finished | Ended::Legacy),
        }
    }

    pub fn late_skip(&self) -> bool {
        let completion = self.completion().unwrap_or(if self.skipped() { 0.0 } else { 1.0 });
        matches!(self.ended, Ended::Skipped | Ended::Legacy) && !self.early_skip() && completion < 0.4
    }

    pub fn presence_weight(&self) -> f64 {
        if self.heard <= 0.0 {
            return 1.0;
        }
        let (idle, locked) = (self.idle.min(self.heard), self.locked.min(self.heard));
        let present = (self.heard - idle - locked).max(0.0);
        (present + idle * IDLE_WEIGHT + locked * LOCKED_WEIGHT) / self.heard
    }

    pub fn legacy(track: Track, seconds: f64, skipped: bool, at: i64) -> Play {
        Play { track, at, heard: seconds, furthest: seconds, ended: Ended::Legacy, flagged: skipped, learn: true, ..Play::default() }
    }
}
