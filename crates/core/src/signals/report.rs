use std::collections::HashMap;

use serde::Serialize;

use super::{Ended, Play};
use crate::ids::TrackId;
use crate::model::Via;

const DAY: i64 = 86_400_000;
const WEEK: i64 = 7 * DAY;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub via: String,
    pub started: String,
    pub plays: usize,
    pub early_skips: f64,
    pub completed: f64,
    pub liked: f64,
    pub repeated: f64,
    pub failed: usize,
    pub away: f64,
}

#[derive(Default)]
struct Tally {
    plays: usize,
    early: usize,
    complete: usize,
    liked: usize,
    repeated: usize,
    failed: usize,
    away: f64,
    heard: f64,
}

fn rate(n: usize, of: usize) -> f64 {
    if of == 0 { 0.0 } else { (n as f64 / of as f64 * 1000.0).round() / 1000.0 }
}

fn started(play: &Play) -> &'static str {
    match (play.ended, play.chosen) {
        (Ended::Legacy, _) => "unknown",
        (_, true) => "chosen",
        (_, false) => "itself",
    }
}

fn bucket(play: &Play) -> String {
    if play.ended == Ended::Legacy { "before signals".into() } else { play.via.name().into() }
}

pub fn report(plays: &[Play], likes: &HashMap<TrackId, i64>, since: i64) -> Vec<Line> {
    let mut by_track: HashMap<&TrackId, Vec<i64>> = HashMap::new();
    for p in plays {
        by_track.entry(&p.track.id).or_default().push(p.at);
    }
    let mut tallies: HashMap<(String, &'static str), Tally> = HashMap::new();
    for p in plays.iter().filter(|p| p.at >= since && p.learn) {
        let t = tallies.entry((bucket(p), started(p))).or_default();
        if p.ended == Ended::Failed {
            t.failed += 1;
            continue;
        }
        if matches!(p.ended, Ended::Unknown | Ended::Quit | Ended::Halted) {
            continue;
        }
        t.plays += 1;
        t.early += p.early_skip() as usize;
        t.complete += p.finished() as usize;
        t.liked += likes.get(&p.track.id).is_some_and(|at| *at >= p.at && *at < p.at + WEEK) as usize;
        t.repeated += by_track.get(&p.track.id).is_some_and(|ats| ats.iter().any(|at| *at > p.at && *at < p.at + WEEK)) as usize;
        t.heard += p.heard;
        t.away += p.heard * (1.0 - p.presence_weight());
    }
    let mut lines: Vec<Line> = tallies
        .into_iter()
        .map(|((via, started), t)| Line {
            via,
            started: started.into(),
            plays: t.plays,
            early_skips: rate(t.early, t.plays),
            completed: rate(t.complete, t.plays),
            liked: rate(t.liked, t.plays),
            repeated: rate(t.repeated, t.plays),
            failed: t.failed,
            away: if t.heard > 0.0 { (t.away / t.heard * 1000.0).round() / 1000.0 } else { 0.0 },
        })
        .collect();
    let order = |via: &str| Via::ALL.iter().position(|v| v.name() == via).unwrap_or(Via::ALL.len());
    lines.sort_by(|a, b| order(&a.via).cmp(&order(&b.via)).then(a.started.cmp(&b.started)));
    lines
}
