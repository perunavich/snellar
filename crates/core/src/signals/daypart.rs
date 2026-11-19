use std::collections::{HashMap, HashSet};

use super::Play;
use crate::clock;
use crate::ids::TrackId;
use crate::model::{Part, Track};

const WINDOW: i64 = 8 * 7 * 86_400_000;
const HALF_LIFE_DAYS: f64 = 28.0;
pub const ENOUGH_PLAYS: usize = 12;
pub const ENOUGH_DAYS: usize = 3;
pub const ENOUGH_ARTISTS: usize = 4;
const PRESENT_ENOUGH: f64 = 0.5;
const NAMED: usize = 3;

pub fn at(millis: i64) -> Part {
    Part::of_hour(clock::local(millis).hour)
}

pub const BREAK: i64 = 4 * 3_600_000;

fn awake(p: &Play) -> bool {
    p.presence_weight() >= PRESENT_ENOUGH
}

fn stretched(clock: Part, session: i64) -> Part {
    if clock == Part::Morning && at(session) == Part::Night { Part::Night } else { clock }
}

fn walk(plays: &[Play], until: i64, mut each: impl FnMut(usize, Part)) -> Option<(i64, i64)> {
    let mut order: Vec<usize> = (0..plays.len()).filter(|&i| plays[i].at <= until).collect();
    order.sort_by_key(|&i| plays[i].at);
    let mut session: Option<(i64, i64)> = None;
    for i in order {
        let p = &plays[i];
        let open = session.filter(|(_, end)| p.at - end < BREAK);
        if awake(p) {
            let (start, end) = open.unwrap_or((p.at, p.at));
            session = Some((start, end.max(p.at + (p.heard * 1000.0) as i64)));
        }
        let current = if awake(p) { session } else { open };
        each(i, current.map_or(at(p.at), |(start, _)| stretched(at(p.at), start)));
    }
    session
}

fn parts(plays: &[Play]) -> Vec<Part> {
    let mut out = vec![Part::Day; plays.len()];
    walk(plays, i64::MAX, |i, part| out[i] = part);
    out
}

pub fn part_at(plays: &[Play], now: i64) -> Part {
    match walk(plays, now, |_, _| {}) {
        Some((start, end)) if now - end < BREAK => stretched(at(now), start),
        _ => at(now),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Habit {
    pub part: Part,
    pub weekend: Option<bool>,
    pub plays: usize,
    pub days: usize,
    pub artists: Vec<String>,
    pub anchors: Vec<(Track, f64)>,
}

struct Kept<'a> {
    play: &'a Play,
    weekend: bool,
    day: i64,
    weight: f64,
}

fn kept(plays: &[Play], part: Part, now: i64) -> Vec<Kept<'_>> {
    plays
        .iter()
        .zip(parts(plays))
        .filter(|(p, _)| p.learn && p.finished() && now - p.at < WINDOW && awake(p))
        .filter_map(|(p, of)| {
            let at = clock::local(p.at);
            (of == part).then(|| Kept {
                play: p,
                weekend: at.weekday == 0 || at.weekday == 6,
                day: at.day,
                weight: p.presence_weight() * 0.5f64.powf((now - p.at).max(0) as f64 / 86_400_000.0 / HALF_LIFE_DAYS),
            })
        })
        .collect()
}

fn artist(t: &Track) -> String {
    t.artists.first().map(|a| a.name.clone()).unwrap_or_default()
}

fn gather(found: &[&Kept], part: Part, weekend: Option<bool>) -> Option<Habit> {
    let days: HashSet<i64> = found.iter().map(|k| k.day).collect();
    let mut by_artist: HashMap<String, f64> = HashMap::new();
    let mut by_track: HashMap<&TrackId, (&Track, f64)> = HashMap::new();
    for k in found {
        *by_artist.entry(artist(&k.play.track)).or_default() += k.weight;
        by_track.entry(&k.play.track.id).or_insert((&k.play.track, 0.0)).1 += k.weight;
    }
    by_artist.remove("");
    if found.len() < ENOUGH_PLAYS || days.len() < ENOUGH_DAYS || by_artist.len() < ENOUGH_ARTISTS {
        return None;
    }
    let mut artists: Vec<(String, f64)> = by_artist.into_iter().collect();
    artists.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut anchors: Vec<(Track, f64)> = by_track.into_values().map(|(t, w)| (t.clone(), w)).collect();
    anchors.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.id.as_str().cmp(b.0.id.as_str())));
    Some(Habit {
        part,
        weekend,
        plays: found.len(),
        days: days.len(),
        artists: artists.into_iter().take(NAMED).map(|(a, _)| a).collect(),
        anchors,
    })
}

pub fn habit(plays: &[Play], part: Part, now: i64) -> Option<Habit> {
    let found = kept(plays, part, now);
    let today = clock::local(now);
    let weekend = today.weekday == 0 || today.weekday == 6;
    let same: Vec<&Kept> = found.iter().filter(|k| k.weekend == weekend).collect();
    gather(&same, part, Some(weekend)).or_else(|| gather(&found.iter().collect::<Vec<_>>(), part, None))
}
