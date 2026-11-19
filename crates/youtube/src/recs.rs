use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;

use snellar_core::cache::{Bounded, write_atomic};
use snellar_core::clock::{Clock, unix_millis, unix_secs};
use snellar_core::error::Code;
use snellar_core::ids::TrackId;
use snellar_core::journal;
use snellar_core::library::Library;
use snellar_core::model::{Artist, ArtistCard, DayPick, Part, Pick, Shelf, Track, Via, Why};
use snellar_core::product::USER_AGENT;
use snellar_core::settings;
use snellar_core::signals::daypart;
use snellar_core::sync::Lock;
use snellar_core::tasks::TaskSet;

use crate::lyrics::{clean_title, norm};
use crate::net::{self, Http};
use crate::ytm::{Filter, Ytm};

mod sieve;

use sieve::Versions;

const TIMEOUT: Duration = Duration::from_secs(6);
const GATHER_TTL: Duration = Duration::from_secs(1800);
const HOME_TTL: u64 = 300;
const PAID_SEARCHES: usize = 14;
const AUTOPLAY_DEADLINE: Duration = Duration::from_millis(3000);
const WAVE_DEADLINE: Duration = Duration::from_secs(4);
const HOME_DEADLINE: Duration = Duration::from_secs(8);
const MOOD_PLAYLISTS: usize = 3;
const WAVE_TTL: Duration = Duration::from_secs(1800);
const WAVE_DEFAULT: usize = 25;
const SKIP_HALF_LIFE_DAYS: f64 = 45.0;
const LISTEN_HALF_LIFE_DAYS: f64 = 60.0;
const PLAYS: usize = 400;
const HEARD: usize = 5000;
const WAVE_SEEDS: usize = 4;
const FOLLOWED_WEIGHT: f64 = 0.5;
const FAMILIAR_MAX: usize = 40;
const RESTED: usize = 200;
const REST_MS: i64 = 7 * 86_400_000;
const DAYPART_PLAYS: usize = 3000;
const SESSION_PLAYS: usize = 300;
const RADIO_PLAYS: usize = 60;
const RADIO_KEPT: usize = 2;
const RADIO_FIRST: f64 = 0.8;
const RADIO_KEPT_WEIGHT: f64 = 0.6;
pub const VERSION: u32 = 6;
const READY_TTL: Duration = Duration::from_secs(600);
const HOME_BUDGET: Duration = Duration::from_secs(3);
const REFRESH_BUDGET: Duration = Duration::from_secs(8);
const WAVE_PREVIEW: usize = 10;
const LISTED_TRACKS: usize = 500;
const LISTED_WEIGHT: f64 = 0.25;
const LISTED_ARTIST_CAP: f64 = 0.5;
const LISTED_ANCHORS: usize = 5;
const LISTED_ANCHOR_WEIGHT: f64 = 0.4;
const SCENE_ARTISTS: usize = 8;
const SCENE_SONGS: usize = 4;
const SCENE_MIN: usize = 8;
const OUTER_WEIGHT: f64 = 0.6;
const PAGE_WAIT: Duration = Duration::from_secs(4);
const GRACE: Duration = Duration::from_millis(800);
const CORE_WAIT: Duration = Duration::from_millis(3500);
const WARM_GAP: Duration = Duration::from_secs(15);
const BRAINZ_GAP: Duration = Duration::from_millis(1100);
const MEMO_CAP: usize = 4000;
const MEMO_TTL: Duration = Duration::from_secs(30 * 86_400);
const GATHERED_CAP: usize = 32;
const PAGES_CAP: usize = 256;
const WAVES_CAP: usize = 16;
const READY_CAP: usize = 16;
const IDS_CAP: usize = 2000;
const IDS_TTL: Duration = Duration::from_secs(7 * 86_400);
const SIMILAR_ALGORITHM: &str = "session_based_days_9000_session_300_contribution_5_threshold_15_limit_50_skip_30";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Src {
    Radio,
    Scene,
    Outer,
    Other,
    Familiar,
}

#[derive(Clone)]
struct Cand {
    artist: String,
    title: String,
    score: f64,
    pop: f64,
    duration: f64,
    track: Option<Track>,
    fallback: Option<Track>,
    why: Option<Why>,
    src: Src,
}

impl Cand {
    fn of(t: Track, score: f64, src: Src) -> Cand {
        Cand {
            artist: t.artists.first().map_or_else(String::new, |a| a.name.clone()),
            title: t.title.clone(),
            score,
            pop: 0.0,
            duration: t.duration,
            track: Some(t),
            fallback: None,
            why: None,
            src,
        }
    }
}

struct Pacer {
    gap: Duration,
    next: tokio::sync::Mutex<tokio::time::Instant>,
}

impl Pacer {
    fn new(gap: Duration) -> Pacer {
        Pacer { gap, next: tokio::sync::Mutex::new(tokio::time::Instant::now()) }
    }

    async fn wait(&self) {
        let mut next = self.next.lock().await;
        tokio::time::sleep_until(*next).await;
        *next = tokio::time::Instant::now() + self.gap;
    }
}

fn brainz_query(name: &str) -> String {
    format!("artist:\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Memo {
    matches: Vec<(String, u64, Track)>,
    cards: Vec<(String, u64, ArtistCard)>,
    related: Vec<(String, u64, Vec<String>)>,
    farms: Vec<(String, u64)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WaveKnobs {
    pub mood: Option<String>,
    pub language: Option<String>,
    pub novelty: Option<f64>,
    pub limit: Option<usize>,
    pub reshuffle: bool,
}

struct Flavor {
    explore: f64,
    exact: bool,
    language: Option<Lang>,
    mood: Option<String>,
    versions: Versions,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Lang {
    Uk,
    Ru,
    En,
    Ko,
    Ja,
    Zh,
}

impl Lang {
    fn parse(code: &str) -> Option<Lang> {
        Some(match code {
            "uk" => Lang::Uk,
            "ru" => Lang::Ru,
            "en" => Lang::En,
            "ko" => Lang::Ko,
            "ja" => Lang::Ja,
            "zh" => Lang::Zh,
            _ => return None,
        })
    }

    fn fit(self, text: &str) -> f64 {
        let has = |f: fn(char) -> bool| text.chars().any(f);
        let cyrillic = has(|c| ('\u{400}'..='\u{4ff}').contains(&c));
        let uk_marker = has(|c| "іїєґІЇЄҐ".contains(c));
        let ru_marker = has(|c| "ыэъёЫЭЪЁ".contains(c));
        let hangul = has(|c| ('\u{ac00}'..='\u{d7af}').contains(&c) || ('\u{1100}'..='\u{11ff}').contains(&c));
        let kana = has(|c| ('\u{3040}'..='\u{30ff}').contains(&c));
        let han = has(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
        let latin_only = !(cyrillic || hangul || kana || han);
        match self {
            Lang::Uk if uk_marker => 2.0,
            Lang::Uk if ru_marker => 0.15,
            Lang::Uk if cyrillic => 1.2,
            Lang::Ru if ru_marker => 2.0,
            Lang::Ru if uk_marker => 0.15,
            Lang::Ru if cyrillic => 1.2,
            Lang::En if latin_only => 1.3,
            Lang::Ko if hangul => 2.0,
            Lang::Ja if kana => 2.0,
            Lang::Ja if han => 0.6,
            Lang::Zh if han && !kana => 2.0,
            _ => {
                if self == Lang::En {
                    0.2
                } else {
                    0.1
                }
            }
        }
    }
}

struct Gather {
    cands: Mutex<Vec<Cand>>,
    core: watch::Sender<bool>,
    scene: watch::Sender<bool>,
    done: watch::Sender<bool>,
}

#[derive(Clone)]
struct HomeFailure {
    code: Code,
    after: Option<u64>,
    detail: String,
}

impl HomeFailure {
    fn of(e: &anyhow::Error) -> HomeFailure {
        HomeFailure { code: Code::of(e), after: Code::retry_after(e), detail: format!("{e:#}") }
    }

    fn error(&self) -> anyhow::Error {
        net::coded(anyhow::anyhow!(self.detail.clone()), self.code, self.after)
    }
}

struct Build {
    failure: Mutex<Option<HomeFailure>>,
    fresh: bool,
    shelves: Mutex<Vec<(usize, Shelf)>>,
    done: watch::Sender<bool>,
}

#[derive(Default)]
struct Shown {
    tracks: Vec<Track>,
    keys: HashSet<String>,
    artists: HashSet<String>,
}

fn shown_in(shelves: &[Shelf]) -> Shown {
    let tracks: Vec<Track> = shelves.iter().flat_map(|s| s.tracks.iter().cloned()).collect();
    let artists = shelves.iter().flat_map(|s| s.artists.iter().map(|a| norm(&a.name))).collect();
    Shown { keys: tracks.iter().map(track_key).collect(), artists, tracks }
}

fn unseen_first(names: Vec<String>, shown: &HashSet<String>) -> Vec<String> {
    let (new, old): (Vec<_>, Vec<_>) = names.into_iter().partition(|n| !shown.contains(&norm(n)));
    new.into_iter().chain(old).collect()
}

impl Build {
    fn push(&self, rank: usize, shelf: Shelf) {
        if !shelf.is_empty() {
            self.shelves.locked().push((rank, shelf));
        }
    }

    fn snapshot(&self) -> Vec<Shelf> {
        let mut shelves = self.shelves.locked().clone();
        shelves.sort_by_key(|(rank, _)| *rank);
        shelves.into_iter().map(|(_, s)| s).collect()
    }
}

#[derive(Default)]
struct Taste {
    liked: HashMap<String, f64>,
    listed: HashMap<String, f64>,
    known_artists: HashSet<String>,
    skipped_artists: HashMap<String, f64>,
    disliked_artists: HashSet<String>,
    skipped: HashSet<String>,
    disliked: HashSet<String>,
    heard: HashSet<String>,
    unplayable: HashSet<TrackId>,
    rested: Vec<Track>,
    rested_keys: HashSet<String>,
    anchors: Vec<(Track, f64)>,
    hide_spam: bool,
    farms: HashSet<String>,
}

pub struct Recs {
    http: Http,
    ytm: Arc<Ytm>,
    library: Arc<Library>,
    gate: Semaphore,
    gathered: Mutex<Bounded<Arc<Vec<Cand>>>>,
    gathering: Mutex<HashMap<TrackId, Arc<Gather>>>,
    moods: Mutex<Option<Vec<(String, String)>>>,
    waves: Mutex<Bounded<Vec<Track>>>,
    pages: Mutex<Bounded<Arc<Artist>>>,
    warmed: Mutex<Option<Instant>>,
    matches: Mutex<Bounded<Option<Track>>>,
    related: Mutex<Bounded<Vec<String>>>,
    farms: Mutex<Bounded<()>>,
    cards: Mutex<Bounded<Option<ArtistCard>>>,
    flushing: tokio::sync::Mutex<()>,
    cache_dir: PathBuf,
    build: Mutex<Option<Arc<Build>>>,
    deezer_ids: Mutex<Bounded<Option<u64>>>,
    mbids: Mutex<Bounded<Option<String>>>,
    brainz: Pacer,
    home: Mutex<Option<(u64, Vec<Shelf>)>>,
    home_failure: Mutex<Option<HomeFailure>>,
    ready: Mutex<Bounded<Vec<Track>>>,
    tasks: TaskSet,
    clock: Clock,
    card_versions: Mutex<Option<Versions>>,
    picking: tokio::sync::Mutex<()>,
}

fn rnd() -> f64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
    let mut x = nanos ^ COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
}

fn canonical_title(title: &str, artist: &str) -> String {
    norm(&sieve::bare_title(title, artist))
}

fn key(artist: &str, title: &str) -> String {
    format!("{}|{}", norm(artist), canonical_title(title, artist))
}

fn match_key(c: &Cand, versions: Versions) -> String {
    let on = [versions.live, versions.cover, versions.remix, versions.speed].map(|on| if on { '1' } else { '0' });
    format!("{}|{}", key(&c.artist, &c.title), on.iter().collect::<String>())
}

fn track_key(t: &Track) -> String {
    key(t.artists.first().map_or("", |a| a.name.as_str()), &t.title)
}

fn stamp(mut tracks: Vec<Track>, via: Via) -> Vec<Track> {
    for (i, t) in tracks.iter_mut().enumerate() {
        let why = t.pick.take().and_then(|p| p.why);
        t.pick = Some(Box::new(Pick { via, position: i as u32, version: VERSION, why }));
    }
    tracks
}

fn unwanted(t: &Track, taste: &Taste) -> bool {
    taste.disliked.contains(&track_key(t)) || sieve::blocked_track(t, taste)
}

fn still_liked(kept: Option<Vec<Track>>, dislikes: &Taste) -> Option<Vec<Track>> {
    let mut tracks = kept?;
    let before = tracks.len();
    tracks.retain(|t| !unwanted(t, dislikes));
    (!tracks.is_empty() && tracks.len() * 2 >= before).then_some(tracks)
}

fn without_dislikes(shelf: &mut Shelf, taste: &Taste) {
    let named = |name: &str| taste.disliked_artists.contains(&norm(name));
    if shelf.kind == "becauseLiked" && named(&shelf.arg) {
        shelf.tracks.clear();
    }
    if let Some(pick) = shelf.pick.as_mut() {
        pick.artists.retain(|a| !named(a));
    }
    shelf.tracks.retain(|t| !unwanted(t, taste));
    shelf.albums.retain(|a| !a.artists.iter().any(|r| named(&r.name)));
    shelf.artists.retain(|a| !named(&a.name));
}

fn sift_shelf(shelf: &mut Shelf, taste: &Taste, versions: Versions) {
    without_dislikes(shelf, taste);
    shelf.tracks.retain(|t| !sieve::spam_track(t, taste) && !versions.bars(t));
    shelf.albums.retain(|a| !a.artists.iter().any(|r| sieve::spam_artist(&r.name, taste)));
    shelf.artists.retain(|a| !sieve::spam_artist(&a.name, taste));
}

fn primary_artist(t: &Track) -> String {
    t.artists.first().map_or_else(String::new, |a| norm(&a.name))
}

async fn all<F: Future + Send + 'static>(jobs: impl IntoIterator<Item = F>) -> Vec<F::Output>
where
    F::Output: Send + 'static,
{
    let mut set = JoinSet::new();
    jobs.into_iter().for_each(|j| {
        set.spawn(j);
    });
    let mut out = Vec::new();
    while let Some(Ok(done)) = set.join_next().await {
        out.push(done);
    }
    out
}

async fn soft(source: impl Future<Output = Result<Vec<Cand>>>) -> Vec<Cand> {
    tokio::time::timeout(TIMEOUT * 2, source).await.ok().and_then(Result::ok).unwrap_or_default()
}

fn dz_cand(item: &Value, score: f64) -> Option<Cand> {
    Some(Cand {
        artist: item["artist"]["name"].as_str()?.into(),
        title: item["title_short"].as_str().or_else(|| item["title"].as_str())?.into(),
        score,
        pop: (item["rank"].as_f64().unwrap_or(0.0) / 1_000_000.0).min(1.0),
        duration: item["duration"].as_f64().unwrap_or(0.0),
        track: None,
        fallback: None,
        why: None,
        src: Src::Outer,
    })
}

fn deezer_pick(found: &Value, name: &str) -> Option<u64> {
    found["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| norm(a["name"].as_str().unwrap_or("")) == norm(name))
        .max_by_key(|a| a["nb_fan"].as_u64().unwrap_or(0))
        .and_then(|a| a["id"].as_u64())
}

fn lastfm_cands(v: &Value) -> Vec<Cand> {
    let number = |x: &Value| x.as_f64().or_else(|| x.as_str()?.parse().ok());
    v["similartracks"]["track"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(Cand {
                artist: t["artist"]["name"].as_str()?.into(),
                title: t["name"].as_str()?.into(),
                score: number(&t["match"]).unwrap_or(0.5),
                pop: 0.0,
                duration: number(&t["duration"]).unwrap_or(0.0),
                track: None,
                fallback: None,
                why: None,
                src: Src::Outer,
            })
        })
        .collect()
}

fn pick(c: &Cand, results: Vec<Track>, versions: Versions) -> Option<Track> {
    let artist = norm(&c.artist);
    let title = norm(&clean_title(&c.title));
    let wanted = sieve::version_in(&c.title, &c.artist);
    let fit = |t: &Track| (sieve::version_of(t) == wanted, sieve::grade(t));
    let found = results.into_iter().filter(|t| {
        let same_artist = t.artists.iter().any(|a| {
            let n = norm(&a.name);
            n == artist || (n.len() > 3 && artist.contains(&n)) || (artist.len() > 3 && n.contains(&artist))
        });
        let found = norm(&clean_title(&t.title));
        let same_title =
            found == title || (found.len() >= 4 && title.len() >= 4 && (found.starts_with(&title) || title.starts_with(&found)));
        let same_length = c.duration <= 0.0 || t.duration <= 0.0 || (c.duration - t.duration).abs() <= 3.0;
        let version = sieve::version_of(t);
        let asked = version.is_some() && version == wanted;
        same_artist && same_title && same_length && !versions.bars(t) && (versions.allows_all(t) || asked)
    });
    found.fold(None, |best: Option<Track>, t| match best {
        Some(b) if fit(&b) >= fit(&t) => Some(b),
        _ => Some(t),
    })
}

fn rank(cands: Vec<Cand>, taste: &Taste, exclude: &HashSet<String>, discover: bool, versions: Versions) -> Vec<Cand> {
    let scene: HashSet<String> = cands.iter().filter(|c| matches!(c.src, Src::Radio | Src::Scene)).map(|c| norm(&c.artist)).collect();
    let gated = scene.len() >= SCENE_MIN;
    let mut groups: HashMap<String, (Cand, usize)> = HashMap::new();
    for mut c in cands {
        if c.src == Src::Outer && gated {
            if !scene.contains(&norm(&c.artist)) {
                continue;
            }
            c.score *= OUTER_WEIGHT;
        }
        if sieve::blocked(&c, taste) || sieve::spam(&c, taste) || c.track.as_ref().is_some_and(sieve::unwanted) {
            continue;
        }
        if let Some(t) = c.track.take_if(|t| sieve::wants_the_original(t, versions)) {
            c.title = sieve::bare_title(&t.title, &c.artist);
            if sieve::version_of(&t).is_some() {
                c.duration = 0.0;
            }
            c.fallback = (!versions.bars(&t)).then_some(t);
        } else if c.track.is_none() && versions.bars_title(&c.title, &c.artist) {
            c.title = sieve::bare_title(&c.title, &c.artist);
            c.duration = 0.0;
        }
        let k = key(&c.artist, &c.title);
        let heard = taste.heard.contains(&k) && !(c.src == Src::Familiar && taste.rested_keys.contains(&k));
        if exclude.contains(&k) || heard || taste.skipped.contains(&k) || taste.disliked.contains(&k) {
            continue;
        }
        let known = taste.known_artists.contains(&norm(&c.artist));
        if discover && known {
            continue;
        }
        match groups.get_mut(&k) {
            Some((best, n)) => {
                *n += 1;
                best.score += c.score;
                best.pop = best.pop.max(c.pop);
                best.track = sieve::better(best.track.take(), c.track);
                best.fallback = sieve::better(best.fallback.take(), c.fallback);
            }
            None => {
                groups.insert(k, (c, 1));
            }
        }
    }
    let mut ranked: Vec<Cand> = groups
        .into_values()
        .map(|(mut c, n)| {
            let artist = norm(&c.artist);
            let listed = taste.listed.get(&artist).copied().unwrap_or(0.0);
            let liked = (taste.liked.get(&artist).copied().unwrap_or(0.0) + listed).min(1.5);
            let skips = taste.skipped_artists.get(&artist).copied().unwrap_or(0.0).min(3.0);
            let version = if c.track.is_none() && c.fallback.is_some() { sieve::ONLY_A_VERSION } else { 1.0 };
            c.score *= (1.0 + 0.2 * (n as f64).ln()) * (1.0 - 0.35 * c.pop.powi(2)) * (1.0 + 0.3 * liked) * (1.0 - 0.25 * skips) * version;
            c.score *= 0.9 + 0.2 * rnd();
            c
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    ranked
}

fn by_seed(groups: Vec<Vec<Cand>>, weights: &[f64]) -> Vec<Cand> {
    let mut owner: HashMap<String, (usize, f64)> = HashMap::new();
    for (g, group) in groups.iter().enumerate() {
        for c in group {
            let best = owner.entry(norm(&c.artist)).or_insert((g, c.score));
            if c.score > best.1 {
                *best = (g, c.score);
            }
        }
    }
    let mut taken = HashSet::new();
    let mut queues: Vec<std::collections::VecDeque<Cand>> = groups
        .into_iter()
        .enumerate()
        .map(|(g, group)| group.into_iter().filter(|c| owner.get(&norm(&c.artist)).is_some_and(|(o, _)| *o == g)).collect())
        .collect();
    let weights: Vec<f64> = (0..queues.len()).map(|g| weights.get(g).copied().unwrap_or(1.0).max(0.01)).collect();
    let total: f64 = weights.iter().sum();
    let mut credit = vec![0.0; queues.len()];
    let mut out = Vec::new();
    while queues.iter().any(|q| !q.is_empty()) {
        for (g, q) in queues.iter().enumerate() {
            if !q.is_empty() {
                credit[g] += weights[g];
            }
        }
        let Some(g) = (0..queues.len()).filter(|g| !queues[*g].is_empty()).max_by(|a, b| credit[*a].total_cmp(&credit[*b])) else { break };
        credit[g] -= total;
        if let Some(c) = queues[g].pop_front()
            && taken.insert(key(&c.artist, &c.title))
        {
            out.push(c);
        }
    }
    out
}

fn balance(tracks: Vec<Track>, known: &HashSet<String>, (explore, exact): (f64, bool), n: usize) -> Vec<Track> {
    let is_fresh = |t: &Track| !known.contains(&primary_artist(t));
    let target = (explore * n as f64).round() as usize;
    let mut head: Vec<Track> = tracks.iter().take(n).cloned().collect();
    let mut fresh_spare: Vec<Track> = tracks.iter().skip(n).filter(|t| is_fresh(t)).cloned().collect();
    let mut familiar_spare: Vec<Track> = tracks.into_iter().skip(n).filter(|t| !is_fresh(t)).collect();
    loop {
        let fresh = head.iter().filter(|t| is_fresh(t)).count();
        let (incoming, drop_fresh) = match fresh.cmp(&target) {
            std::cmp::Ordering::Less if !fresh_spare.is_empty() => (fresh_spare.remove(0), false),
            std::cmp::Ordering::Greater if exact && !familiar_spare.is_empty() => (familiar_spare.remove(0), true),
            _ => break,
        };
        let Some(at) = head.iter().rposition(|t| is_fresh(t) == drop_fresh) else { break };
        head[at] = incoming;
    }
    head
}

fn diversify(tracks: Vec<Track>, n: usize) -> Vec<Track> {
    let mut picked: Vec<Track> = Vec::new();
    let mut deferred = Vec::new();
    let mut titles = HashSet::new();
    let mut per_artist: HashMap<String, usize> = HashMap::new();
    for t in tracks {
        let artist = primary_artist(&t);
        let title = canonical_title(&t.title, &artist);
        if titles.contains(&title) || per_artist.get(&artist).copied().unwrap_or(0) >= 2 {
            continue;
        }
        if picked.last().is_some_and(|p| primary_artist(p) == artist) {
            deferred.push(t);
            continue;
        }
        titles.insert(title);
        *per_artist.entry(artist).or_default() += 1;
        picked.push(t);
    }
    for t in deferred {
        let artist = primary_artist(&t);
        if picked.len() < n && per_artist.get(&artist).copied().unwrap_or(0) < 2 {
            *per_artist.entry(artist).or_default() += 1;
            picked.push(t);
        }
    }
    picked.truncate(n);
    picked
}

impl Recs {
    pub fn new(http: impl Into<Http>, ytm: Arc<Ytm>, library: Arc<Library>, cache_dir: PathBuf) -> Arc<Self> {
        Recs::with_clock(http, ytm, library, cache_dir, Clock::system())
    }

    pub fn with_clock(http: impl Into<Http>, ytm: Arc<Ytm>, library: Arc<Library>, cache_dir: PathBuf, clock: Clock) -> Arc<Self> {
        let memo: Memo =
            std::fs::read(cache_dir.join("recs-memo.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let age = |at: u64| Duration::from_secs(unix_secs().saturating_sub(at));
        let mut matches = Bounded::new(MEMO_CAP, MEMO_TTL);
        memo.matches.into_iter().for_each(|(k, at, t)| matches.insert_aged(k, age(at), Some(t)));
        let mut cards = Bounded::new(MEMO_CAP, MEMO_TTL);
        memo.cards.into_iter().for_each(|(k, at, c)| cards.insert_aged(k, age(at), Some(c)));
        let mut related = Bounded::new(MEMO_CAP, MEMO_TTL);
        memo.related.into_iter().for_each(|(k, at, names)| related.insert_aged(k, age(at), names));
        let mut farms = Bounded::new(MEMO_CAP, MEMO_TTL);
        memo.farms.into_iter().for_each(|(k, at)| farms.insert_aged(k, age(at), ()));
        let home = std::fs::read(cache_dir.join("home.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| Some((v["at"].as_u64()?, serde_json::from_value(v["shelves"].clone()).ok()?)));
        Arc::new(Self {
            http: http.into(),
            ytm,
            library,
            gate: Semaphore::new(2),
            gathered: Mutex::new(Bounded::new(GATHERED_CAP, GATHER_TTL)),
            gathering: Mutex::default(),
            moods: Mutex::default(),
            waves: Mutex::new(Bounded::new(WAVES_CAP, WAVE_TTL)),
            pages: Mutex::new(Bounded::new(PAGES_CAP, GATHER_TTL)),
            warmed: Mutex::default(),
            matches: Mutex::new(matches),
            cards: Mutex::new(cards),
            related: Mutex::new(related),
            farms: Mutex::new(farms),
            flushing: tokio::sync::Mutex::new(()),
            cache_dir,
            build: Mutex::default(),
            deezer_ids: Mutex::new(Bounded::new(IDS_CAP, IDS_TTL)),
            mbids: Mutex::new(Bounded::new(IDS_CAP, IDS_TTL)),
            brainz: Pacer::new(BRAINZ_GAP),
            home: Mutex::new(home),
            home_failure: Mutex::default(),
            ready: Mutex::new(Bounded::new(READY_CAP, READY_TTL)),
            tasks: TaskSet::default(),
            clock,
            card_versions: Mutex::default(),
            picking: tokio::sync::Mutex::new(()),
        })
    }

    async fn flush_memo(&self) {
        let _one_at_a_time = self.flushing.lock().await;
        let now = unix_secs();
        let memo = Memo {
            matches: self
                .matches
                .locked()
                .aged()
                .filter_map(|(k, age, t)| Some((k.clone(), now.saturating_sub(age.as_secs()), t.clone()?)))
                .collect(),
            cards: self
                .cards
                .locked()
                .aged()
                .filter_map(|(k, age, c)| Some((k.clone(), now.saturating_sub(age.as_secs()), c.clone()?)))
                .collect(),
            related: self
                .related
                .locked()
                .aged()
                .map(|(k, age, names)| (k.clone(), now.saturating_sub(age.as_secs()), names.clone()))
                .collect(),
            farms: self.farms.locked().aged().map(|(k, age, ())| (k.clone(), now.saturating_sub(age.as_secs()))).collect(),
        };
        if let Ok(bytes) = serde_json::to_vec(&memo) {
            let _ = write_atomic(&self.cache_dir.join("recs-memo.json"), bytes).await;
        }
    }

    async fn get(&self, url: &str, params: &[(&str, &str)]) -> Result<Value> {
        net::json(net::get(&self.http, url, params, &[("User-Agent", USER_AGENT)], TIMEOUT).await?).await
    }

    fn lastfm_key(&self) -> Option<String> {
        let key = settings::LASTFM_KEY.get::<String>(&self.library)?;
        (!key.trim().is_empty()).then(|| key.trim().to_string())
    }

    fn dislikes(&self) -> Taste {
        let mut taste = Taste::default();
        for t in self.library.disliked() {
            taste.disliked.insert(track_key(&t));
            taste.disliked_artists.insert(primary_artist(&t));
        }
        taste
    }

    fn taste(&self) -> Taste {
        let likes = self.library.favorites();
        let played = self.library.played(HEARD);
        let last_played: HashMap<String, i64> = played.iter().map(|d| (track_key(&d.item), d.at)).collect();
        let history: Vec<Track> = played.into_iter().map(|d| d.item).collect();
        let mut taste = self.dislikes();
        for (i, t) in likes.iter().enumerate() {
            for a in &t.artists {
                *taste.liked.entry(norm(&a.name)).or_default() += 1.0 / (1.0 + i as f64 / 20.0);
            }
        }
        for t in history.iter().chain(&likes) {
            taste.heard.insert(track_key(t));
            taste.known_artists.extend(t.artists.iter().map(|a| norm(&a.name)));
        }
        let now = unix_millis() as i64;
        let decay = |at: i64, half_life_days: f64| 0.5f64.powf((now - at).max(0) as f64 / 86_400_000.0 / half_life_days);
        let mut seen = HashSet::new();
        let mut kept = Vec::new();
        for play in self.library.plays(PLAYS).into_iter().filter(|p| p.learn) {
            let artist = primary_artist(&play.track);
            let strength = match play.heard {
                _ if !play.early_skip() && !play.late_skip() => 0.0,
                s if s < 10.0 && play.early_skip() => 1.0,
                _ if play.early_skip() => 0.5,
                _ => 0.25,
            };
            if strength > 0.0 {
                *taste.skipped_artists.entry(artist).or_default() += strength * decay(play.at, SKIP_HALF_LIFE_DAYS);
                if strength >= 0.5 {
                    taste.skipped.insert(track_key(&play.track));
                }
            } else if play.finished() {
                let weight = decay(play.at, LISTEN_HALF_LIFE_DAYS) * play.presence_weight();
                *taste.liked.entry(artist).or_default() += 0.3 * weight;
                if seen.insert(play.track.id.clone()) {
                    kept.push(play.track.clone());
                    taste.anchors.push((play.track, 0.6 * weight));
                }
            }
        }
        taste.heard.extend(taste.disliked.iter().cloned());
        let mut listed_anchors = 0;
        for (i, t) in self.library.playlisted(LISTED_TRACKS).into_iter().enumerate() {
            let (k, artist) = (track_key(&t), primary_artist(&t));
            taste.heard.insert(k.clone());
            taste.known_artists.extend(t.artists.iter().map(|a| norm(&a.name)));
            if taste.skipped.contains(&k) || taste.disliked.contains(&k) || taste.disliked_artists.contains(&artist) {
                continue;
            }
            let weight = taste.listed.entry(artist).or_default();
            *weight = (*weight + LISTED_WEIGHT / (1.0 + i as f64 / 20.0)).min(LISTED_ARTIST_CAP);
            if listed_anchors < LISTED_ANCHORS && !taste.anchors.iter().any(|(a, _)| a.id == t.id) {
                taste.anchors.push((t, LISTED_ANCHOR_WEIGHT));
                listed_anchors += 1;
            }
        }
        let rested_at = |t: &Track| last_played.get(&track_key(t)).is_none_or(|at| now - at > REST_MS);
        let mut rested_seen = HashSet::new();
        taste.rested =
            likes.iter().chain(&kept).filter(|t| rested_at(t) && rested_seen.insert(track_key(t))).take(RESTED).cloned().collect();
        for t in likes.into_iter().take(30) {
            taste.anchors.retain(|(a, _)| a.id != t.id);
            taste.anchors.push((t, 1.0));
        }
        for a in self.library.followed() {
            let artist = norm(&a.name);
            let weight = taste.liked.entry(artist.clone()).or_default();
            *weight = weight.max(FOLLOWED_WEIGHT);
            taste.known_artists.insert(artist);
        }
        taste.unplayable = self.library.unplayable().into_iter().collect();
        taste.hide_spam = self.hides_spam();
        if taste.hide_spam {
            taste.farms = self.farms.locked().aged().map(|(k, _, ())| k.clone()).collect();
        }
        let blocked = taste.disliked_artists.clone();
        let anchors = std::mem::take(&mut taste.anchors);
        taste.anchors = anchors.into_iter().filter(|(t, _)| !sieve::blocked_track(t, &taste)).collect();
        taste.liked.retain(|artist, _| !blocked.contains(artist));
        taste.listed.retain(|artist, _| !blocked.contains(artist));
        let rested = std::mem::take(&mut taste.rested);
        taste.rested = rested.into_iter().filter(|t| !sieve::blocked_track(t, &taste) && !taste.skipped.contains(&track_key(t))).collect();
        taste.rested_keys = taste.rested.iter().map(track_key).collect();
        taste
    }

    fn anchors(&self, taste: &Taste, count: usize, avoid: &str) -> Vec<(Track, f64)> {
        pick_anchors(taste.anchors.iter().filter(|(t, _)| primary_artist(t) != avoid).cloned().collect(), count)
    }
}

fn pick_anchors(mut pool: Vec<(Track, f64)>, count: usize) -> Vec<(Track, f64)> {
    let mut out: Vec<(Track, f64)> = Vec::new();
    while out.len() < count && !pool.is_empty() {
        let total: f64 = pool.iter().map(|(_, w)| w).sum();
        let mut r = rnd() * total;
        let i = pool.iter().position(|(_, w)| {
            r -= w;
            r <= 0.0
        });
        let (t, w) = pool.swap_remove(i.unwrap_or(0));
        pool.retain(|(o, _)| primary_artist(o) != primary_artist(&t));
        out.push((t, w));
    }
    out
}

impl Recs {
    async fn ytm_radio(&self, seed: &Track) -> Result<Vec<Cand>> {
        let tracks = self.ytm.radio(&seed.id).await?;
        let len = tracks.len() as f64;
        Ok(tracks.into_iter().enumerate().map(|(i, t)| Cand::of(t, 1.0 - 0.5 * i as f64 / len, Src::Radio)).collect())
    }

    async fn page(&self, id: &str) -> Option<Arc<Artist>> {
        if let Some(page) = self.pages.locked().get(id) {
            return Some(page.clone());
        }
        let page = Arc::new(self.ytm.artist_quiet(id).await.ok()?);
        self.pages.locked().insert(id.into(), page.clone());
        Some(page)
    }

    async fn neighbours(self: &Arc<Self>, seed: &Track) -> Result<Vec<ArtistCard>> {
        let name = seed.artists.first().map_or("", |a| a.name.as_str());
        let hide_spam = self.hides_spam();
        let related: Vec<String> = self.related_names(name).await.into_iter().filter(|n| !(hide_spam && self.is_farm(n))).collect();
        if !related.is_empty() {
            let cards = self.artist_cards(related.into_iter().take(SCENE_ARTISTS + 2).collect()).await;
            if !cards.is_empty() {
                return Ok(cards);
            }
        }
        let id = match seed.artists.first().filter(|a| !a.id.is_empty()) {
            Some(a) => a.id.clone(),
            None => self.artist_card(&primary_artist(seed)).await.context("seed artist has no page")?.id,
        };
        let page = self.page(&id).await.context("artist page unavailable")?;
        Ok(page.related.clone())
    }

    async fn scene_songs(self: &Arc<Self>, seed: &Track) -> Result<Vec<Cand>> {
        let cards = self.neighbours(seed).await?;
        let jobs = cards.into_iter().take(SCENE_ARTISTS).enumerate().map(|(i, card)| {
            let this = self.clone();
            async move {
                let top = tokio::time::timeout(PAGE_WAIT, this.page(&card.id)).await.ok()??;
                let weight = 0.85 * (1.0 - 0.5 * i as f64 / SCENE_ARTISTS as f64);
                Some(
                    top.top
                        .iter()
                        .take(SCENE_SONGS)
                        .enumerate()
                        .map(|(p, t)| Cand::of(t.clone(), weight * (1.0 - 0.08 * p as f64), Src::Scene))
                        .collect::<Vec<_>>(),
                )
            }
        });
        Ok(all(jobs).await.into_iter().flatten().flatten().collect())
    }

    async fn deezer_artist(&self, name: &str) -> Result<Option<u64>> {
        if let Some(hit) = self.deezer_ids.locked().get(name) {
            return Ok(*hit);
        }
        let found = self.get("https://api.deezer.com/search/artist", &[("q", name), ("limit", "8")]).await?;
        let id = deezer_pick(&found, name);
        self.deezer_ids.locked().insert(name.into(), id);
        Ok(id)
    }

    async fn related_names(&self, artist: &str) -> Vec<String> {
        let k = norm(artist);
        if k.is_empty() {
            return Vec::new();
        }
        if let Some(hit) = self.related.locked().get(&k) {
            return hit.clone();
        }
        let Ok(Some(id)) = self.deezer_artist(artist).await else { return Vec::new() };
        let Ok(reply) = self.get(&format!("https://api.deezer.com/artist/{id}/related"), &[("limit", "20")]).await else {
            return Vec::new();
        };
        let artists = reply["data"].as_array().cloned().unwrap_or_default();
        let names: Vec<String> = artists.iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
        self.note_farms(&artists);
        if !names.is_empty() {
            self.related.locked().insert(k, names.clone());
        }
        names
    }

    /// Remembers the artists of a Deezer list that look like release farms (`sieve::farm`), for 30 days like the list.
    fn note_farms(&self, artists: &[Value]) {
        let mut farms = self.farms.locked();
        for a in artists {
            let (Some(name), Some(releases), Some(fans)) = (a["name"].as_str(), a["nb_album"].as_u64(), a["nb_fan"].as_u64()) else {
                continue;
            };
            let k = norm(name);
            if sieve::farm(releases, fans) && !farms.contains_key(&k) {
                journal::info("recs", format!("{name} looks like a release farm ({releases} releases, {fans} fans on Deezer)"));
                farms.insert(k, ());
            }
        }
    }

    fn is_farm(&self, name: &str) -> bool {
        self.farms.locked().contains_key(&norm(name))
    }

    fn hides_spam(&self) -> bool {
        settings::RECS_SPAM.or_default::<bool>(&self.library)
    }

    async fn mbid(&self, name: &str) -> Result<Option<String>> {
        if let Some(hit) = self.mbids.locked().get(name) {
            return Ok(hit.clone());
        }
        let query = brainz_query(name);
        let params = [("query", query.as_str()), ("fmt", "json"), ("limit", "3")];
        self.brainz.wait().await;
        let mut found = self.get("https://musicbrainz.org/ws/2/artist", &params).await;
        for _ in 0..2 {
            if found.is_ok() {
                break;
            }
            self.brainz.wait().await;
            found = self.get("https://musicbrainz.org/ws/2/artist", &params).await;
        }
        let found = found?;
        let id = found["artists"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| norm(a["name"].as_str().unwrap_or("")) == norm(name))
            .and_then(|a| a["id"].as_str().map(String::from));
        self.mbids.locked().insert(name.into(), id.clone());
        Ok(id)
    }

    async fn listenbrainz(self: &Arc<Self>, artist: &str) -> Result<Vec<Cand>> {
        let mbid = self.mbid(artist).await?.context("artist not on musicbrainz")?;
        let similar = self
            .get("https://labs.api.listenbrainz.org/similar-artists/json", &[("artist_mbids", &mbid), ("algorithm", SIMILAR_ALGORITHM)])
            .await?;
        let similar = similar.as_array().context("unexpected similar-artists reply")?;
        let top = similar.first().and_then(|a| a["score"].as_f64()).unwrap_or(1.0);
        let jobs = similar.iter().filter(|a| a["artist_mbid"] != mbid.as_str()).take(6).filter_map(|a| {
            let this = self.clone();
            let id = a["artist_mbid"].as_str()?.to_string();
            let weight = 0.9 * a["score"].as_f64().unwrap_or(0.0) / top;
            Some(async move {
                let rec = this.get(&format!("https://api.listenbrainz.org/1/popularity/top-recordings-for-artist/{id}"), &[]).await.ok()?;
                Some(
                    rec.as_array()?
                        .iter()
                        .take(8)
                        .enumerate()
                        .filter_map(|(p, r)| {
                            Some(Cand {
                                artist: r["artist_name"].as_str()?.into(),
                                title: r["recording_name"].as_str()?.into(),
                                score: weight * (1.0 - 0.04 * p as f64),
                                pop: (1.0 + r["total_listen_count"].as_f64().unwrap_or(0.0)).ln() / 17.0,
                                duration: r["length"].as_f64().unwrap_or(0.0) / 1000.0,
                                track: None,
                                fallback: None,
                                why: None,
                                src: Src::Outer,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
            })
        });
        Ok(all(jobs).await.into_iter().flatten().flatten().collect())
    }

    async fn lastfm(&self, seed: &Track) -> Result<Vec<Cand>> {
        let Some(api_key) = self.lastfm_key() else { return Ok(Vec::new()) };
        let title = clean_title(&seed.title);
        let artist = seed.artists.first().map_or("", |a| a.name.as_str());
        let reply = self
            .get(
                "https://ws.audioscrobbler.com/2.0/",
                &[
                    ("method", "track.getsimilar"),
                    ("artist", artist),
                    ("track", &title),
                    ("api_key", &api_key),
                    ("format", "json"),
                    ("limit", "40"),
                    ("autocorrect", "1"),
                ],
            )
            .await?;
        Ok(lastfm_cands(&reply))
    }

    async fn gather(self: &Arc<Self>, seed: &Track, budget: Duration) -> Arc<Vec<Cand>> {
        if let Some(hit) = self.gathered.locked().get(seed.id.as_str()) {
            return hit.clone();
        }
        let running = self.gathering.locked().get(&seed.id).cloned();
        let gather = match running {
            Some(g) => g,
            None => self.start_gather(seed),
        };
        let (mut core, mut scene) = (gather.core.subscribe(), gather.scene.subscribe());
        let _ = tokio::time::timeout(budget.max(CORE_WAIT), core.wait_for(|d| *d)).await;
        let _ = tokio::time::timeout(GRACE, scene.wait_for(|d| *d)).await;
        let ready = gather.cands.locked().clone();
        Arc::new(ready)
    }

    fn start_gather(self: &Arc<Self>, seed: &Track) -> Arc<Gather> {
        let gather = Arc::new(Gather {
            cands: Mutex::default(),
            core: watch::channel(false).0,
            scene: watch::channel(false).0,
            done: watch::channel(false).0,
        });
        self.gathering.locked().insert(seed.id.clone(), gather.clone());
        let (this, task, seed) = (self.clone(), gather.clone(), seed.clone());
        self.tasks.spawn("recs gather", async move {
            let artist = seed.artists.first().map_or_else(String::new, |a| a.name.clone());
            let feed = |found: Vec<Cand>| task.cands.locked().extend(found);
            tokio::join!(
                async {
                    feed(soft(this.ytm_radio(&seed)).await);
                    task.core.send_replace(true);
                },
                async {
                    feed(soft(this.scene_songs(&seed)).await);
                    task.scene.send_replace(true);
                },
                async { feed(soft(this.listenbrainz(&artist)).await) },
                async { feed(soft(this.lastfm(&seed)).await) },
            );
            let all = task.cands.locked().clone();
            if all.iter().any(|c| c.src == Src::Radio) {
                this.gathered.locked().insert(seed.id.to_string(), Arc::new(all));
            }
            this.gathering.locked().remove(&seed.id);
            task.done.send_replace(true);
        });
        gather
    }

    async fn pool(self: &Arc<Self>, seeds: &[(Track, f64)], budget: Duration) -> Vec<Cand> {
        self.pool_by_seed(seeds, budget).await.concat()
    }

    async fn pool_by_seed(self: &Arc<Self>, seeds: &[(Track, f64)], budget: Duration) -> Vec<Vec<Cand>> {
        let jobs = seeds.iter().enumerate().map(|(i, (seed, weight))| {
            let this = self.clone();
            let (seed, weight) = (seed.clone(), *weight);
            async move {
                let mut cands = this.gather(&seed, budget).await.to_vec();
                let artist = seed.artists.first().map(|a| a.name.clone()).unwrap_or_default();
                let liked = this.library.is_liked(&seed.id);
                for c in &mut cands {
                    c.score *= weight;
                    c.why = Some(match (c.src, liked) {
                        (Src::Radio, true) => Why::Liked { artist: artist.clone() },
                        (Src::Radio, false) => Why::Played { artist: artist.clone() },
                        _ => Why::Similar { artist: artist.clone() },
                    });
                }
                (i, cands)
            }
        });
        let mut found = all(jobs).await;
        found.sort_by_key(|(i, _)| *i);
        found.into_iter().map(|(_, cands)| cands).collect()
    }

    async fn find_song(&self, c: &Cand, versions: Versions) -> Option<Track> {
        let mut song = self.look_up(c, versions).await?;
        if let Some(why) = &c.why {
            song.pick = Some(Box::new(Pick { why: Some(why.clone()), ..Pick::default() }));
        }
        Some(song)
    }

    async fn look_up(&self, c: &Cand, versions: Versions) -> Option<Track> {
        if c.track.is_some() {
            return c.track.clone();
        }
        let k = match_key(c, versions);
        let kept = self.matches.locked().get(&k).cloned();
        if let Some(hit) = kept {
            return hit.or_else(|| c.fallback.clone());
        }
        let found = async {
            let _permit = self.gate.acquire().await.ok()?;
            let query = format!("{} {}", c.artist, clean_title(&c.title));
            self.ytm.search_quiet(&query, Filter::Songs).await.ok()
        };
        let Some(found) = found.await else { return c.fallback.clone() };
        let song = pick(c, found.tracks, versions);
        self.matches.locked().insert(k, song.clone());
        song.or_else(|| c.fallback.clone())
    }

    async fn resolve(self: &Arc<Self>, ranked: Vec<Cand>, versions: Versions, deadline: Duration, want: usize) -> Vec<Track> {
        let total = ranked.len();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for (i, c) in ranked.into_iter().enumerate() {
            let (this, tx) = (self.clone(), tx.clone());
            self.tasks.spawn("recs song match", async move {
                let _ = tx.send((i, this.find_song(&c, versions).await));
            });
        }
        drop(tx);
        let stop = tokio::time::Instant::now() + deadline;
        let mut slots: Vec<Option<Option<Track>>> = vec![None; total];
        while let Ok(Some((i, song))) = tokio::time::timeout_at(stop, rx.recv()).await {
            slots[i] = Some(song);
            let mut found = 0;
            for slot in &slots {
                match slot {
                    None => break,
                    Some(song) => found += song.is_some() as usize,
                }
                if found >= want {
                    return slots.into_iter().flatten().flatten().collect();
                }
            }
        }
        slots.into_iter().flatten().flatten().collect()
    }

    async fn recommend(
        self: &Arc<Self>,
        seeds: &[(Track, f64)],
        taste: &Taste,
        exclude: &HashSet<String>,
        flavor: &Flavor,
        n: usize,
        deadline: Duration,
    ) -> Vec<Track> {
        let versions = flavor.versions;
        let mut groups = self.pool_by_seed(seeds, deadline / 2).await;
        let (language, mood, familiar) = (
            async {
                match flavor.language {
                    Some(lang) => self.language_cands(lang).await,
                    None => Vec::new(),
                }
            },
            async {
                match &flavor.mood {
                    Some(mood) => self.mood_cands(mood).await,
                    None => Vec::new(),
                }
            },
            async {
                if flavor.exact && flavor.explore < 0.5 && flavor.mood.is_none() {
                    self.familiar_cands(&taste.rested).await
                } else {
                    Vec::new()
                }
            },
        );
        let (language, mood, familiar) = tokio::join!(language, mood, familiar);
        let mut weights: Vec<f64> = seeds.iter().map(|(_, w)| *w).collect();
        let extra: Vec<Cand> = language.into_iter().chain(mood).chain(familiar).collect();
        if !extra.is_empty() {
            groups.push(extra);
            weights.push(1.0);
        }
        if let Some(lang) = flavor.language {
            groups.iter_mut().flatten().for_each(|c| c.score *= lang.fit(&format!("{} {}", c.title, c.artist)));
        }
        let ranked: Vec<Vec<Cand>> = groups.into_iter().map(|g| rank(g, taste, exclude, false, versions)).collect();
        for ((seed, _), group) in seeds.iter().zip(&ranked) {
            if group.is_empty() {
                journal::info("recs", format!("no candidates came from the seed {} ({})", seed.id, seed.artist_line()));
            }
        }
        let ranked = by_seed(ranked, &weights);
        self.songs(ranked, n, versions, taste, (flavor.explore, flavor.exact), deadline / 2).await
    }

    fn flavor(&self, knobs: &WaveKnobs) -> Flavor {
        Flavor {
            exact: knobs.novelty.is_some(),
            explore: knobs.novelty.unwrap_or_else(|| settings::RECS_EXPLORE.or_default(&self.library)).clamp(0.0, 1.0),
            language: knobs.language.as_deref().and_then(Lang::parse),
            mood: knobs.mood.clone(),
            versions: self.versions(),
        }
    }

    fn versions(&self) -> Versions {
        let on = |s: &settings::Setting| s.or_default::<bool>(&self.library);
        Versions {
            live: on(&settings::RECS_LIVE),
            cover: on(&settings::RECS_COVERS),
            remix: on(&settings::RECS_REMIXES),
            speed: on(&settings::RECS_SPEED),
        }
    }

    pub fn knobs(&self) -> WaveKnobs {
        settings::WAVE.or_default(&self.library)
    }

    async fn songs(
        self: &Arc<Self>,
        ranked: Vec<Cand>,
        n: usize,
        versions: Versions,
        taste: &Taste,
        explore: (f64, bool),
        deadline: Duration,
    ) -> Vec<Track> {
        let is_free = |c: &Cand| c.track.is_some() || self.matches.locked().contains_key(&match_key(c, versions));
        let enough = ranked.iter().filter(|c| is_free(c)).count() >= 2 * n;
        let mut paid = 0;
        let pool: Vec<Cand> = ranked
            .into_iter()
            .filter_map(|mut c| {
                let free = is_free(&c);
                paid += !free as usize;
                if free || (!enough && paid <= PAID_SEARCHES) {
                    return Some(c);
                }
                c.track = c.fallback.take();
                c.track.is_some().then_some(c)
            })
            .collect();
        let mut found = self.resolve(pool, versions, deadline, 3 * n).await;
        found.retain(|t| !sieve::blocked_track(t, taste) && !sieve::unwanted(t) && !versions.bars(t));
        let spread = diversify(found.clone(), found.len());
        diversify(balance(spread, &taste.known_artists, explore, n), n)
    }

    async fn mood_cands(self: &Arc<Self>, mood: &str) -> Vec<Cand> {
        let cached = self.moods.locked().clone();
        let moods = match cached {
            Some(m) => m,
            None => {
                let Ok(found) = self.ytm.moods().await else { return Vec::new() };
                *self.moods.locked() = Some(found.clone());
                found
            }
        };
        let Some((_, params)) = moods.iter().find(|(name, _)| name.eq_ignore_ascii_case(mood)) else { return Vec::new() };
        let Ok(mut lists) = self.ytm.mood_playlists(params).await else { return Vec::new() };
        let mut chosen = Vec::new();
        while chosen.len() < MOOD_PLAYLISTS && !lists.is_empty() {
            chosen.push(lists.swap_remove((rnd() * lists.len() as f64) as usize % lists.len()));
        }
        let jobs = chosen.into_iter().map(|id| {
            let this = self.clone();
            async move { this.ytm.playlist_first_page(&id).await.map(|p| p.tracks).unwrap_or_default() }
        });
        let why = Why::Mood { name: mood.to_string() };
        all(jobs)
            .await
            .into_iter()
            .flatten()
            .map(|t| Cand { why: Some(why.clone()), ..Cand::of(t, 1.5 - 0.45 * rnd(), Src::Other) })
            .collect()
    }

    async fn language_cands(self: &Arc<Self>, lang: Lang) -> Vec<Cand> {
        let category = match lang {
            Lang::Ko => Some("K-Pop"),
            Lang::Ja => Some("J-Pop"),
            Lang::Zh => Some("Mandopop & cantopop"),
            _ => None,
        };
        if let Some(category) = category {
            return self.mood_cands(category).await;
        }
        let query = match lang {
            Lang::Uk => "популярні українські пісні",
            Lang::Ru => "популярные русские песни",
            _ => return Vec::new(),
        };
        let Ok(found) = self.ytm.search(query, Filter::Songs).await else { return Vec::new() };
        found.tracks.into_iter().filter(|t| t.duration < 720.0).map(|t| Cand::of(t, 1.2, Src::Other)).collect()
    }

    async fn familiar_cands(self: &Arc<Self>, rested: &[Track]) -> Vec<Cand> {
        let mut seen = HashSet::new();
        let artists: Vec<_> = self
            .library
            .favorites()
            .into_iter()
            .filter_map(|t| t.artists.into_iter().next())
            .filter(|a| !a.id.is_empty() && seen.insert(a.id.clone()))
            .take(3)
            .collect();
        let jobs = artists.into_iter().map(|a| {
            let this = self.clone();
            async move {
                let why = Why::Liked { artist: a.name.clone() };
                let top = this.ytm.artist(&a.id).await.map(|p| p.top).unwrap_or_default();
                top.into_iter().map(|t| Cand { why: Some(why.clone()), ..Cand::of(t, 0.9, Src::Other) }).collect::<Vec<_>>()
            }
        });
        let mut found: Vec<Cand> = all(jobs).await.into_iter().flatten().collect();
        let mut rested = rested.to_vec();
        while found.len() < FAMILIAR_MAX && !rested.is_empty() {
            let t = rested.swap_remove((rnd() * rested.len() as f64) as usize % rested.len());
            found.push(Cand { why: Some(Why::Favourite), ..Cand::of(t, 1.0, Src::Familiar) });
        }
        found
    }

    pub async fn wave(self: &Arc<Self>, knobs: &WaveKnobs, recent: &[Track]) -> Result<Vec<Track>> {
        let n = knobs.limit.unwrap_or(WAVE_DEFAULT).clamp(1, 100);
        let cache_key = format!("{:?}:{}", self.versions(), serde_json::to_string(&WaveKnobs { reshuffle: false, ..knobs.clone() })?);
        if !knobs.reshuffle {
            let kept = self.waves.locked().get(&cache_key).cloned();
            if let Some(tracks) = still_liked(kept, &self.dislikes()) {
                return Ok(tracks);
            }
        }
        let mut avoid = recent.to_vec();
        if knobs.reshuffle {
            avoid.extend(self.waves.locked().get(&cache_key).cloned().unwrap_or_default());
        }
        let tracks = stamp(self.build_wave(knobs, &avoid, n).await?, Via::Wave);
        if !tracks.is_empty() {
            self.waves.locked().insert(cache_key, tracks.clone());
        }
        Ok(tracks)
    }

    async fn build_wave(self: &Arc<Self>, knobs: &WaveKnobs, recent: &[Track], n: usize) -> Result<Vec<Track>> {
        let taste = self.taste();
        let weight = if knobs.mood.is_some() { 0.25 } else { 1.0 };
        let seeds: Vec<_> = self.anchors(&taste, WAVE_SEEDS, "").into_iter().map(|(t, _)| (t, weight)).collect();
        let exclude: HashSet<String> = recent.iter().map(track_key).collect();
        let flavor = self.flavor(knobs);
        let versions = flavor.versions;
        let mut tracks = self.recommend(&seeds, &taste, &exclude, &flavor, n, WAVE_DEADLINE).await;
        if tracks.is_empty() {
            let chart = self.get("https://api.deezer.com/chart/0/tracks", &[("limit", "40")]).await?;
            let cands: Vec<Cand> = chart["data"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(i, t)| dz_cand(t, 1.0 - 0.5 * i as f64 / 40.0))
                .collect();
            let ranked = rank(cands, &taste, &exclude, false, versions);
            tracks = self.songs(ranked, n, versions, &taste, (flavor.explore, flavor.exact), HOME_DEADLINE).await;
        }
        self.flush_memo().await;
        Ok(tracks)
    }

    pub async fn next_tracks(self: &Arc<Self>, seed: &Track, recent: &[Track], n: usize) -> Result<Vec<Track>> {
        let prepared = self.ready.locked().remove(seed.id.as_str());
        if let Some(mut tracks) = prepared {
            let (versions, dislikes) = (self.versions(), self.dislikes());
            tracks.retain(|t| !recent.iter().any(|r| r.id == t.id) && !versions.bars(t) && !unwanted(t, &dislikes));
            if tracks.len() >= n / 2 {
                tracks.truncate(n);
                return Ok(stamp(tracks, Via::Radio));
            }
        }
        Ok(stamp(self.compute_next(seed, recent, n).await?, Via::Radio))
    }

    pub fn forget(&self) {
        self.waves.locked().clear();
        self.gathered.locked().clear();
        self.ready.locked().clear();
        *self.home.locked() = None;
        *self.home_failure.locked() = None;
        let _ = std::fs::remove_file(self.cache_dir.join("home.json"));
    }

    pub fn warm(self: &Arc<Self>, seed: &Track) {
        {
            let mut last = self.warmed.locked();
            let ready = self.ready.locked().contains_key(seed.id.as_str());
            if ready || last.is_some_and(|at| at.elapsed() < WARM_GAP) {
                return;
            }
            *last = Some(Instant::now());
        }
        let (this, seed) = (self.clone(), seed.clone());
        self.tasks.spawn("recs warm", async move {
            if let Ok(tracks) = this.compute_next(&seed, &[], 25).await {
                this.ready.locked().insert(seed.id.to_string(), tracks);
            }
        });
    }

    async fn compute_next(self: &Arc<Self>, seed: &Track, recent: &[Track], n: usize) -> Result<Vec<Track>> {
        let taste = self.taste();
        let seeds = self.radio_seeds(seed, recent, &taste);
        let mut exclude: HashSet<String> = recent.iter().map(track_key).collect();
        exclude.insert(track_key(seed));
        let flavor = self.flavor(&WaveKnobs::default());
        let mut tracks = self.recommend(&seeds, &taste, &exclude, &flavor, n, AUTOPLAY_DEADLINE).await;
        tracks.retain(|t| !recent.iter().any(|r| r.id == t.id) && t.id != seed.id);
        self.flush_memo().await;
        if tracks.is_empty() {
            return self.plain_radio(seed).await;
        }
        Ok(tracks)
    }

    pub async fn plain_radio(&self, seed: &Track) -> Result<Vec<Track>> {
        let versions = self.versions();
        let mut radio = self.ytm.radio(&seed.id).await?;
        let dislikes = self.dislikes();
        radio.retain(|t| !versions.bars(t) && !unwanted(t, &dislikes));
        Ok(radio)
    }

    fn radio_seeds(&self, seed: &Track, recent: &[Track], taste: &Taste) -> Vec<(Track, f64)> {
        let unwanted = |t: &Track| {
            let k = track_key(t);
            taste.skipped.contains(&k) || taste.disliked.contains(&k) || sieve::blocked_track(t, taste)
        };
        let queued: HashSet<&TrackId> = recent.iter().map(|t| &t.id).collect();
        let kept = self.library.plays(RADIO_PLAYS).into_iter().filter(|p| p.learn && p.finished() && queued.contains(&p.track.id));
        let mut seeds: Vec<(Track, f64)> = Vec::new();
        let candidates = std::iter::once((seed.clone(), 1.0))
            .chain(recent.first().map(|first| (first.clone(), RADIO_FIRST)))
            .chain(kept.take(RADIO_KEPT).map(|p| (p.track, RADIO_KEPT_WEIGHT)));
        for (t, w) in candidates {
            if !unwanted(&t) && !seeds.iter().any(|(s, _)| s.id == t.id) {
                seeds.push((t, w));
            }
        }
        if seeds.is_empty() {
            seeds.push((seed.clone(), 1.0));
        }
        seeds
    }

    async fn artist_card(&self, name: &str) -> Option<ArtistCard> {
        let k = norm(name);
        if let Some(hit) = self.cards.locked().get(&k) {
            return hit.clone();
        }
        let _permit = self.gate.acquire().await.ok()?;
        let found = self.ytm.search_quiet(name, Filter::Artists).await.ok()?;
        let card = found.artists.into_iter().find(|a| norm(&a.name) == k);
        self.cards.locked().insert(k, card.clone());
        card
    }

    async fn artist_cards(self: &Arc<Self>, names: Vec<String>) -> Vec<ArtistCard> {
        let jobs = names.into_iter().enumerate().map(|(i, name)| {
            let this = self.clone();
            async move { (i, this.artist_card(&name).await) }
        });
        let mut found = all(jobs).await;
        found.sort_by_key(|(i, _)| *i);
        found.into_iter().filter_map(|(_, c)| c).collect()
    }

    async fn discover(self: &Arc<Self>, seeds: &[(Track, f64)], taste: &Taste, shown: &HashSet<String>) -> Shelf {
        let ranked = rank(self.pool(seeds, HOME_DEADLINE / 2).await, taste, &HashSet::new(), true, Versions::default());
        let mut names: Vec<String> = Vec::new();
        for c in ranked.iter().take(60) {
            if !names.iter().any(|n| norm(n) == norm(&c.artist)) {
                names.push(c.artist.clone());
            }
        }
        let mut names = unseen_first(names, shown);
        names.truncate(10);
        Shelf { title: "Discover".into(), kind: "discover".into(), artists: self.artist_cards(names).await, ..Default::default() }
    }

    async fn because(self: &Arc<Self>, seed: (Track, f64), taste: &Taste, shown: &HashSet<String>) -> Shelf {
        let n = 12;
        let artist = seed.0.artists.first().map_or(String::new(), |a| a.name.clone());
        let title = format!("Because you liked {artist}");
        let flavor = self.flavor(&WaveKnobs::default());
        let tracks = self.recommend(&[seed], taste, shown, &flavor, n, HOME_DEADLINE).await;
        Shelf { title, kind: "becauseLiked".into(), arg: artist, tracks: stamp(tracks, Via::Because), ..Default::default() }
    }

    async fn trending_tracks(self: &Arc<Self>, build: &Build) {
        let Ok(chart) = self.get("https://api.deezer.com/chart/0/tracks", &[("limit", "40")]).await else { return };
        let cands: Vec<Cand> = chart["data"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, t)| dz_cand(t, 1.0 - 0.5 * i as f64 / 40.0))
            .collect();
        let tracks = self.songs(cands, 15, self.versions(), &Taste::default(), (0.0, false), HOME_DEADLINE).await;
        build.push(
            0,
            Shelf { title: "Trending now".into(), kind: "trending".into(), tracks: stamp(tracks, Via::Shelf), ..Default::default() },
        );
    }

    async fn popular_artists(self: &Arc<Self>, build: &Build) {
        let Ok(chart) = self.get("https://api.deezer.com/chart/0/artists", &[("limit", "12")]).await else { return };
        let names = chart["data"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str().map(String::from)).collect();
        let artists = self.artist_cards(names).await;
        build.push(1, Shelf { title: "Popular artists".into(), kind: "popularArtists".into(), artists, ..Default::default() });
    }

    pub async fn daypart(self: &Arc<Self>, part: Option<Part>, reshuffle: bool, shown: &[Track]) -> Shelf {
        let _one_at_a_time = self.picking.lock().await;
        let plays = self.library.plays(DAYPART_PLAYS);
        let now = self.clock.millis();
        let part = part.unwrap_or_else(|| daypart::part_at(&plays, now));
        let habits: Vec<Part> = Part::ALL.into_iter().filter(|p| daypart::habit(&plays, *p, now).is_some()).collect();
        self.day_pick(part, habits, daypart::habit(&plays, part, now), reshuffle, shown).await
    }

    async fn day_pick(
        self: &Arc<Self>,
        part: Part,
        habits: Vec<Part>,
        habit: Option<daypart::Habit>,
        reshuffle: bool,
        shown: &[Track],
    ) -> Shelf {
        let Some(habit) = habit else { return self.wave_pick(part, habits, reshuffle, shown).await };
        let knobs = self.knobs();
        let versions = self.versions();
        let cache_key = format!("daypart:{part:?}:{:?}:{versions:?}:{}", habit.weekend, serde_json::to_string(&knobs).unwrap_or_default());
        let kept = self.waves.locked().get(&cache_key).cloned();
        let dislikes = self.dislikes();
        let liked = if reshuffle { None } else { still_liked(kept.clone(), &dislikes) };
        let tracks = match liked {
            Some(tracks) => tracks,
            None => {
                let before = if reshuffle { kept } else { None };
                let taste = self.taste();
                let pool: Vec<(Track, f64)> = habit.anchors.iter().filter(|(t, _)| !sieve::blocked_track(t, &taste)).cloned().collect();
                if pool.is_empty() {
                    return self.wave_pick(part, habits, reshuffle, shown).await;
                }
                let seeds: Vec<(Track, f64)> = pick_anchors(pool, WAVE_SEEDS).into_iter().map(|(t, _)| (t, 1.0)).collect();
                let exclude: HashSet<String> = shown.iter().chain(before.iter().flatten()).map(track_key).collect();
                let found = self.recommend(&seeds, &taste, &exclude, &self.flavor(&knobs), WAVE_DEFAULT, WAVE_DEADLINE).await;
                self.flush_memo().await;
                let tracks = stamp(found, Via::Daypart);
                if !tracks.is_empty() {
                    self.waves.locked().insert(cache_key, tracks.clone());
                }
                tracks
            }
        };
        let pick =
            DayPick { part, habits, habit: true, weekend: habit.weekend, artists: habit.artists, plays: habit.plays, days: habit.days };
        let mut shelf = Shelf { kind: "daypart".into(), tracks, pick: Some(pick), ..Default::default() };
        without_dislikes(&mut shelf, &dislikes);
        shelf
    }

    async fn wave_pick(self: &Arc<Self>, part: Part, habits: Vec<Part>, reshuffle: bool, shown: &[Track]) -> Shelf {
        let tracks = self.wave(&WaveKnobs { reshuffle, ..self.knobs() }, shown).await.unwrap_or_default();
        let pick = DayPick { part, habits, ..DayPick::default() };
        Shelf { kind: "daypart".into(), tracks, pick: Some(pick), ..Default::default() }
    }

    async fn personal(self: &Arc<Self>, build: &Arc<Build>, fresh: bool) {
        let taste = Arc::new(self.taste());
        if taste.anchors.is_empty() {
            tokio::join!(self.trending_tracks(build), self.popular_artists(build));
            return;
        }
        let shown = Arc::new(match self.home.locked().as_ref() {
            Some((_, shelves)) if fresh => shown_in(shelves),
            _ => Shown::default(),
        });
        let mix_seeds = self.anchors(&taste, 3, "");
        let mut liked: Vec<_> = taste.liked.iter().collect();
        liked.sort_by(|a, b| b.1.total_cmp(a.1));
        let mut because_seeds = Vec::new();
        for (artist, _) in liked.into_iter().take(4) {
            if let Some(seed) = taste.anchors.iter().find(|(t, w)| *w == 1.0 && primary_artist(t) == *artist) {
                because_seeds.push(seed.clone());
            }
        }
        because_seeds.truncate(2);
        let mix = async {
            *self.card_versions.locked() = Some(self.versions());
            let mut pick = self.daypart(None, fresh, &shown.tracks).await;
            if pick.tracks.len() < WAVE_PREVIEW && !shown.tracks.is_empty() {
                pick = self.daypart(None, fresh, &[]).await;
            }
            build.push(0, pick);
        };
        let discover = async { build.push(3, self.discover(&mix_seeds, &taste, &shown.artists).await) };
        let because = all(because_seeds.into_iter().enumerate().map(|(i, seed)| {
            let (this, taste, build, shown) = (self.clone(), taste.clone(), build.clone(), shown.clone());
            async move { build.push(1 + i, this.because(seed, &taste, &shown.keys).await) }
        }));
        tokio::join!(mix, discover, because);
    }

    async fn build_home(self: &Arc<Self>, build: &Arc<Build>, fresh: bool) {
        let ytm = async {
            match self.ytm.home().await {
                Ok(shelves) => {
                    let taste = self.taste();
                    for (i, mut shelf) in shelves.into_iter().enumerate() {
                        sift_shelf(&mut shelf, &taste, self.versions());
                        shelf.tracks = stamp(shelf.tracks, Via::Shelf);
                        build.push(100 + i, shelf);
                    }
                }
                Err(e) => *build.failure.locked() = Some(HomeFailure::of(&e)),
            }
        };
        tokio::join!(self.personal(build, fresh), ytm);
        let shelves = build.snapshot();
        let failure = build.failure.locked().clone();
        let current = self.build.locked().as_ref().is_some_and(|b| Arc::ptr_eq(b, build));
        if current {
            *self.home_failure.locked() = failure.clone();
        }
        if !shelves.is_empty() && failure.is_none() {
            let at = self.clock.secs();
            if current {
                if let Ok(bytes) = serde_json::to_vec(&json!({"at": at, "shelves": shelves})) {
                    let _ = tokio::fs::write(self.cache_dir.join("home.json"), bytes).await;
                }
                *self.home.locked() = Some((at, shelves));
            }
        }
        self.flush_memo().await;
        build.done.send_replace(true);
    }

    fn start_build(self: &Arc<Self>, fresh: bool) -> Arc<Build> {
        let mut slot = self.build.locked();
        if let Some(running) = slot.as_ref().filter(|b| !*b.done.borrow() && (b.fresh || !fresh)) {
            return running.clone();
        }
        let build = Arc::new(Build { failure: Mutex::default(), fresh, shelves: Mutex::default(), done: watch::channel(false).0 });
        *slot = Some(build.clone());
        let (this, task) = (self.clone(), build.clone());
        self.tasks.spawn("home build", async move { this.build_home(&task, fresh).await });
        build
    }

    pub fn part_now(&self) -> Part {
        daypart::part_at(&self.library.plays(SESSION_PLAYS), self.clock.millis())
    }

    async fn home_with(self: &Arc<Self>, force: bool) -> Result<Vec<Shelf>> {
        let shelves = self.kept_or_built(force).await?;
        Ok(self.as_of_now(shelves).await)
    }

    async fn as_of_now(self: &Arc<Self>, mut shelves: Vec<Shelf>) -> Vec<Shelf> {
        let (versions, part) = (self.versions(), self.part_now());
        let built_for = *self.card_versions.locked();
        let dislikes = self.dislikes();
        let stale = |s: &Shelf| {
            s.pick.as_ref().is_some_and(|p| p.part != part)
                || built_for.is_some_and(|v| v != versions)
                || s.tracks.iter().any(|t| versions.bars(t))
                || (!s.tracks.is_empty() && still_liked(Some(s.tracks.clone()), &dislikes).is_none())
        };
        if shelves.iter().any(|s| s.kind == "daypart" && stale(s)) {
            let fresh = self.daypart(Some(part), false, &[]).await;
            if fresh.tracks.is_empty() {
                shelves.retain(|s| s.kind != "daypart");
            } else {
                *self.card_versions.locked() = Some(versions);
                if let Some((_, kept)) = self.home.locked().as_mut() {
                    kept.iter_mut().filter(|s| s.kind == "daypart").for_each(|s| *s = fresh.clone());
                }
                shelves.iter_mut().filter(|s| s.kind == "daypart").for_each(|s| *s = fresh.clone());
            }
        }
        shelves.retain_mut(|shelf| {
            let had = !shelf.is_empty();
            shelf.tracks.retain(|t| !versions.bars(t));
            without_dislikes(shelf, &dislikes);
            !had || !shelf.is_empty()
        });
        shelves
    }

    async fn kept_or_built(self: &Arc<Self>, force: bool) -> Result<Vec<Shelf>> {
        let cached = self.home.locked().clone();
        if let Some((at, shelves)) = &cached
            && !force
        {
            if self.clock.secs().saturating_sub(*at) > HOME_TTL {
                self.start_build(false);
            }
            return Ok(shelves.clone());
        }
        let build = self.start_build(force);
        let mut done = build.done.subscribe();
        let budget = if force { REFRESH_BUDGET } else { HOME_BUDGET };
        let finished = tokio::time::timeout(budget, done.wait_for(|d| *d)).await.is_ok();
        let ready = build.snapshot();
        let failure = build.failure.locked().clone();
        if failure.is_some() {
            *self.home_failure.locked() = failure.clone();
        }
        match (cached, ready.is_empty(), failure) {
            (Some((_, stale)), _, Some(_)) | (Some((_, stale)), true, None) => Ok(stale),
            (None, true, Some(failure)) => Err(failure.error()),
            (None, true, None) if !finished => Err(Code::NetTimeout.err("no home shelves came in time")),
            _ => Ok(ready),
        }
    }

    pub async fn home(self: &Arc<Self>) -> Result<Vec<Shelf>> {
        self.home_with(false).await
    }

    pub async fn refresh_home(self: &Arc<Self>) -> Result<Vec<Shelf>> {
        self.home_with(true).await
    }

    pub fn home_problem(&self) -> Option<(Code, Option<u64>)> {
        self.home_failure.locked().as_ref().map(|f| (f.code, f.after))
    }
}
