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
use snellar_core::error::{Code, RetryAfter};
use snellar_core::ids::TrackId;
use snellar_core::library::Library;
use snellar_core::model::{Artist, ArtistCard, Shelf, Track};
use snellar_core::product::USER_AGENT;
use snellar_core::settings;
use snellar_core::tasks::TaskSet;

use crate::lyrics::{clean_title, norm};
use crate::net::{Http, net};
use crate::ytm::{Filter, Ytm};

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
const READY_TTL: Duration = Duration::from_secs(600);
const HOME_BUDGET: Duration = Duration::from_secs(3);
const REFRESH_BUDGET: Duration = Duration::from_secs(8);
const WAVE_PREVIEW: usize = 10;
const LISTED_TRACKS: usize = 500;
const LISTED_WEIGHT: f64 = 0.25;
const LISTED_ARTIST_CAP: f64 = 0.5;
const LISTED_ANCHORS: usize = 5;
const LISTED_ANCHOR_WEIGHT: f64 = 0.4;
const VARIANTS: [&str; 12] =
    ["live", "cover", "remix", "sped up", "slowed", "karaoke", "instrumental", "nightcore", "reverb", "8d", "tribute", "acapella"];
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
}

#[derive(Clone)]
struct Cand {
    artist: String,
    title: String,
    score: f64,
    pop: f64,
    duration: f64,
    track: Option<Track>,
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
struct Memo {
    matches: Vec<(String, u64, Track)>,
    cards: Vec<(String, u64, ArtistCard)>,
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
        let e = anyhow::anyhow!(self.detail.clone());
        match self.after {
            Some(after) => e.context(RetryAfter(after)).context(self.code),
            None => e.context(self.code),
        }
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
        if !(shelf.tracks.is_empty() && shelf.albums.is_empty() && shelf.artists.is_empty()) {
            self.shelves.lock().unwrap().push((rank, shelf));
        }
    }

    fn snapshot(&self) -> Vec<Shelf> {
        let mut shelves = self.shelves.lock().unwrap().clone();
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
    anchors: Vec<(Track, f64)>,
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
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
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

fn canonical_title(title: &str) -> String {
    let mut bare = String::new();
    let mut depth = 0;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => bare.push(c),
            _ => {}
        }
    }
    let head = bare.split(" - ").next().unwrap_or_default();
    let bare = norm(head);
    if bare.is_empty() { norm(title) } else { bare }
}

fn key(artist: &str, title: &str) -> String {
    format!("{}|{}", norm(artist), canonical_title(title))
}

fn track_key(t: &Track) -> String {
    key(t.artists.first().map_or("", |a| a.name.as_str()), &t.title)
}

fn is_variant(title: &str) -> bool {
    let t = norm(title);
    VARIANTS.iter().any(|v| t.split_whitespace().collect::<Vec<_>>().join(" ").contains(v))
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
        src: Src::Outer,
    })
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
                src: Src::Outer,
            })
        })
        .collect()
}

fn pick(c: &Cand, results: Vec<Track>, allow_variants: bool) -> Option<Track> {
    let artist = norm(&c.artist);
    let title = norm(&clean_title(&c.title));
    results.into_iter().find(|t| {
        let same_artist = t.artists.iter().any(|a| {
            let n = norm(&a.name);
            n == artist || (n.len() > 3 && artist.contains(&n)) || (artist.len() > 3 && n.contains(&artist))
        });
        let found = norm(&clean_title(&t.title));
        let same_title =
            found == title || (found.len() >= 4 && title.len() >= 4 && (found.starts_with(&title) || title.starts_with(&found)));
        let same_length = c.duration <= 0.0 || t.duration <= 0.0 || (c.duration - t.duration).abs() <= 3.0;
        same_artist && same_title && same_length && (allow_variants || !is_variant(&t.title) || is_variant(&c.title))
    })
}

fn rank(cands: Vec<Cand>, taste: &Taste, exclude: &HashSet<String>, discover: bool) -> Vec<Cand> {
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
        let k = key(&c.artist, &c.title);
        if exclude.contains(&k) || taste.heard.contains(&k) || taste.skipped.contains(&k) || taste.disliked.contains(&k) {
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
                if best.track.is_none() {
                    best.track = c.track;
                }
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
            let disliked = if taste.disliked_artists.contains(&artist) { 0.25 } else { 1.0 };
            c.score *= (1.0 + 0.2 * (n as f64).ln()) * (1.0 - 0.35 * c.pop.powi(2)) * (1.0 + 0.3 * liked) * (1.0 - 0.25 * skips) * disliked;
            c.score *= 0.9 + 0.2 * rnd();
            c
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    ranked
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
        let title = canonical_title(&t.title);
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
        let memo: Memo =
            std::fs::read(cache_dir.join("recs-memo.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let age = |at: u64| Duration::from_secs(unix_now().saturating_sub(at));
        let mut matches = Bounded::new(MEMO_CAP, MEMO_TTL);
        memo.matches.into_iter().for_each(|(k, at, t)| matches.insert_aged(k, age(at), Some(t)));
        let mut cards = Bounded::new(MEMO_CAP, MEMO_TTL);
        memo.cards.into_iter().for_each(|(k, at, c)| cards.insert_aged(k, age(at), Some(c)));
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
        })
    }

    async fn flush_memo(&self) {
        let _one_at_a_time = self.flushing.lock().await;
        let now = unix_now();
        let memo = Memo {
            matches: self
                .matches
                .lock()
                .unwrap()
                .aged()
                .filter_map(|(k, age, t)| Some((k.clone(), now.saturating_sub(age.as_secs()), t.clone()?)))
                .collect(),
            cards: self
                .cards
                .lock()
                .unwrap()
                .aged()
                .filter_map(|(k, age, c)| Some((k.clone(), now.saturating_sub(age.as_secs()), c.clone()?)))
                .collect(),
        };
        if let Ok(bytes) = serde_json::to_vec(&memo) {
            let _ = write_atomic(&self.cache_dir.join("recs-memo.json"), bytes).await;
        }
    }

    async fn get(&self, url: &str, params: &[(&str, &str)]) -> Result<Value> {
        let url = if params.is_empty() { reqwest::Url::parse(url)? } else { reqwest::Url::parse_with_params(url, params)? };
        let resp = self.http.client().get(url).header("User-Agent", USER_AGENT).timeout(TIMEOUT).send().await.map_err(net)?;
        resp.error_for_status().map_err(net)?.json().await.map_err(net)
    }

    fn lastfm_key(&self) -> Option<String> {
        let raw = self.library.setting(settings::LASTFM_KEY.key)?;
        let key = serde_json::from_str::<String>(&raw).unwrap_or(raw);
        (!key.trim().is_empty()).then(|| key.trim().to_string())
    }

    fn taste(&self) -> Taste {
        let likes = self.library.favorites();
        let history = self.library.history(150);
        let mut taste = Taste::default();
        for (i, t) in likes.iter().enumerate() {
            for a in &t.artists {
                *taste.liked.entry(norm(&a.name)).or_default() += 1.0 / (1.0 + i as f64 / 20.0);
            }
        }
        for t in history.iter().chain(&likes) {
            taste.heard.insert(track_key(t));
            taste.known_artists.extend(t.artists.iter().map(|a| norm(&a.name)));
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        let decay = |at: i64, half_life_days: f64| 0.5f64.powf((now - at).max(0) as f64 / 86_400_000.0 / half_life_days);
        let mut seen = HashSet::new();
        for (t, seconds, skipped, at) in self.library.listens(400) {
            let artist = primary_artist(&t);
            let completion = if t.duration > 0.0 {
                (seconds / t.duration).min(1.0)
            } else if skipped {
                0.0
            } else {
                1.0
            };
            let strength = match seconds {
                s if s < 10.0 && skipped => 1.0,
                s if s < 30.0 && skipped => 0.5,
                _ if completion < 0.4 => 0.25,
                _ => 0.0,
            };
            if strength > 0.0 {
                *taste.skipped_artists.entry(artist).or_default() += strength * decay(at, SKIP_HALF_LIFE_DAYS);
                if strength >= 0.5 {
                    taste.skipped.insert(track_key(&t));
                }
            } else if completion >= 0.8 {
                *taste.liked.entry(artist).or_default() += 0.3 * decay(at, LISTEN_HALF_LIFE_DAYS);
                if seen.insert(t.id.clone()) {
                    taste.anchors.push((t, 0.6 * decay(at, LISTEN_HALF_LIFE_DAYS)));
                }
            }
        }
        for t in self.library.disliked() {
            taste.disliked.insert(track_key(&t));
            taste.disliked_artists.insert(primary_artist(&t));
            taste.heard.insert(track_key(&t));
        }
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
        for t in likes.into_iter().take(30) {
            taste.anchors.retain(|(a, _)| a.id != t.id);
            taste.anchors.push((t, 1.0));
        }
        taste
    }

    fn anchors(&self, taste: &Taste, count: usize, avoid: &str) -> Vec<(Track, f64)> {
        let mut pool: Vec<_> = taste.anchors.iter().filter(|(t, _)| primary_artist(t) != avoid).cloned().collect();
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

    async fn ytm_radio(&self, seed: &Track) -> Result<Vec<Cand>> {
        let tracks = self.ytm.radio(&seed.id).await?;
        let len = tracks.len() as f64;
        Ok(tracks.into_iter().enumerate().map(|(i, t)| Cand::of(t, 1.0 - 0.5 * i as f64 / len, Src::Radio)).collect())
    }

    async fn page(&self, id: &str) -> Option<Arc<Artist>> {
        if let Some(page) = self.pages.lock().unwrap().get(id) {
            return Some(page.clone());
        }
        let page = Arc::new(self.ytm.artist_quiet(id).await.ok()?);
        self.pages.lock().unwrap().insert(id.into(), page.clone());
        Some(page)
    }

    async fn scene_songs(self: &Arc<Self>, seed: &Track) -> Result<Vec<Cand>> {
        let id = match seed.artists.first().filter(|a| !a.id.is_empty()) {
            Some(a) => a.id.clone(),
            None => self.artist_card(&primary_artist(seed)).await.context("seed artist has no page")?.id,
        };
        let page = self.page(&id).await.context("artist page unavailable")?;
        let jobs = page.related.iter().take(SCENE_ARTISTS).enumerate().map(|(i, card)| {
            let (this, id) = (self.clone(), card.id.clone());
            async move {
                let top = tokio::time::timeout(PAGE_WAIT, this.page(&id)).await.ok()??;
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
        if let Some(hit) = self.deezer_ids.lock().unwrap().get(name) {
            return Ok(*hit);
        }
        let found = self.get("https://api.deezer.com/search/artist", &[("q", name), ("limit", "3")]).await?;
        let id = found["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| norm(a["name"].as_str().unwrap_or("")) == norm(name))
            .and_then(|a| a["id"].as_u64());
        self.deezer_ids.lock().unwrap().insert(name.into(), id);
        Ok(id)
    }

    async fn deezer_related(self: &Arc<Self>, artist: &str) -> Result<Vec<Cand>> {
        let id = self.deezer_artist(artist).await?.context("artist not on deezer")?;
        let (related_url, radio_url) =
            (format!("https://api.deezer.com/artist/{id}/related"), format!("https://api.deezer.com/artist/{id}/radio"));
        let (related, radio) = tokio::try_join!(self.get(&related_url, &[("limit", "12")]), self.get(&radio_url, &[("limit", "25")]))?;
        let mut out: Vec<Cand> = radio["data"].as_array().into_iter().flatten().filter_map(|t| dz_cand(t, 0.5)).collect();
        let tops = related["data"].as_array().into_iter().flatten().take(6).enumerate().filter_map(|(i, a)| {
            let this = self.clone();
            let rid = a["id"].as_u64()?;
            Some(async move {
                let top = this.get(&format!("https://api.deezer.com/artist/{rid}/top"), &[("limit", "8")]).await.ok()?;
                let weight = 0.8 * (1.0 - 0.6 * i as f64 / 12.0);
                let tracks = top["data"].as_array()?;
                Some(tracks.iter().enumerate().filter_map(|(p, t)| dz_cand(t, weight * (1.0 - 0.04 * p as f64))).collect::<Vec<_>>())
            })
        });
        out.extend(all(tops).await.into_iter().flatten().flatten());
        Ok(out)
    }

    async fn mbid(&self, name: &str) -> Result<Option<String>> {
        if let Some(hit) = self.mbids.lock().unwrap().get(name) {
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
        self.mbids.lock().unwrap().insert(name.into(), id.clone());
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
        if let Some(hit) = self.gathered.lock().unwrap().get(seed.id.as_str()) {
            return hit.clone();
        }
        let running = self.gathering.lock().unwrap().get(&seed.id).cloned();
        let gather = match running {
            Some(g) => g,
            None => self.start_gather(seed),
        };
        let (mut core, mut scene) = (gather.core.subscribe(), gather.scene.subscribe());
        let _ = tokio::time::timeout(budget.max(CORE_WAIT), core.wait_for(|d| *d)).await;
        let _ = tokio::time::timeout(GRACE, scene.wait_for(|d| *d)).await;
        let ready = gather.cands.lock().unwrap().clone();
        Arc::new(ready)
    }

    fn start_gather(self: &Arc<Self>, seed: &Track) -> Arc<Gather> {
        let gather = Arc::new(Gather {
            cands: Mutex::default(),
            core: watch::channel(false).0,
            scene: watch::channel(false).0,
            done: watch::channel(false).0,
        });
        self.gathering.lock().unwrap().insert(seed.id.clone(), gather.clone());
        let (this, task, seed) = (self.clone(), gather.clone(), seed.clone());
        self.tasks.spawn("recs gather", async move {
            let artist = seed.artists.first().map_or_else(String::new, |a| a.name.clone());
            let feed = |found: Vec<Cand>| task.cands.lock().unwrap().extend(found);
            tokio::join!(
                async {
                    feed(soft(this.ytm_radio(&seed)).await);
                    task.core.send_replace(true);
                },
                async {
                    feed(soft(this.scene_songs(&seed)).await);
                    task.scene.send_replace(true);
                },
                async { feed(soft(this.deezer_related(&artist)).await) },
                async { feed(soft(this.listenbrainz(&artist)).await) },
                async { feed(soft(this.lastfm(&seed)).await) },
            );
            let all = task.cands.lock().unwrap().clone();
            if all.iter().any(|c| c.src == Src::Radio) {
                this.gathered.lock().unwrap().insert(seed.id.to_string(), Arc::new(all));
            }
            this.gathering.lock().unwrap().remove(&seed.id);
            task.done.send_replace(true);
        });
        gather
    }

    async fn pool(self: &Arc<Self>, seeds: &[(Track, f64)], budget: Duration) -> Vec<Cand> {
        let jobs = seeds.iter().map(|(seed, weight)| {
            let this = self.clone();
            let (seed, weight) = (seed.clone(), *weight);
            async move {
                let mut cands = this.gather(&seed, budget).await.to_vec();
                cands.iter_mut().for_each(|c| c.score *= weight);
                cands
            }
        });
        all(jobs).await.concat()
    }

    async fn find_song(&self, c: &Cand, allow_variants: bool) -> Option<Track> {
        if c.track.is_some() {
            return c.track.clone();
        }
        let k = key(&c.artist, &c.title);
        if let Some(hit) = self.matches.lock().unwrap().get(&k) {
            return hit.clone();
        }
        let _permit = self.gate.acquire().await.ok()?;
        let query = format!("{} {}", c.artist, clean_title(&c.title));
        let found = self.ytm.search_quiet(&query, Filter::Songs).await.ok()?;
        let song = pick(c, found.tracks, allow_variants);
        self.matches.lock().unwrap().insert(k, song.clone());
        song
    }

    async fn resolve(self: &Arc<Self>, ranked: Vec<Cand>, allow_variants: bool, deadline: Duration, want: usize) -> Vec<Track> {
        let total = ranked.len();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        for (i, c) in ranked.into_iter().enumerate() {
            let (this, tx) = (self.clone(), tx.clone());
            self.tasks.spawn("recs song match", async move {
                let _ = tx.send((i, this.find_song(&c, allow_variants).await));
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
        let mut cands = self.pool(seeds, deadline / 2).await;
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
            async { if flavor.exact && flavor.explore < 0.5 && flavor.mood.is_none() { self.familiar_cands().await } else { Vec::new() } },
        );
        let (language, mood, familiar) = tokio::join!(language, mood, familiar);
        cands.extend(language.into_iter().chain(mood).chain(familiar));
        if let Some(lang) = flavor.language {
            cands.iter_mut().for_each(|c| c.score *= lang.fit(&format!("{} {}", c.title, c.artist)));
        }
        let ranked = rank(cands, taste, exclude, false);
        let allow = seeds.iter().any(|(s, _)| is_variant(&s.title));
        self.songs(ranked, n, allow, taste, (flavor.explore, flavor.exact), deadline / 2).await
    }

    fn flavor(&self, knobs: &WaveKnobs) -> Flavor {
        Flavor {
            exact: knobs.novelty.is_some(),
            explore: knobs.novelty.unwrap_or_else(|| settings::RECS_EXPLORE.or_default(&self.library)).clamp(0.0, 1.0),
            language: knobs.language.as_deref().and_then(Lang::parse),
            mood: knobs.mood.clone(),
        }
    }

    pub fn knobs(&self) -> WaveKnobs {
        settings::WAVE.or_default(&self.library)
    }

    async fn songs(
        self: &Arc<Self>,
        ranked: Vec<Cand>,
        n: usize,
        allow_variants: bool,
        taste: &Taste,
        explore: (f64, bool),
        deadline: Duration,
    ) -> Vec<Track> {
        let is_free = |c: &Cand| c.track.is_some() || self.matches.lock().unwrap().contains_key(&key(&c.artist, &c.title));
        let enough = ranked.iter().filter(|c| is_free(c)).count() >= 2 * n;
        let mut paid = 0;
        let pool: Vec<Cand> = ranked
            .into_iter()
            .filter(|c| {
                let free = is_free(c);
                paid += !free as usize;
                free || (!enough && paid <= PAID_SEARCHES)
            })
            .collect();
        let found = self.resolve(pool, allow_variants, deadline, 3 * n).await;
        let spread = diversify(found.clone(), found.len());
        diversify(balance(spread, &taste.known_artists, explore, n), n)
    }

    async fn mood_cands(self: &Arc<Self>, mood: &str) -> Vec<Cand> {
        let cached = self.moods.lock().unwrap().clone();
        let moods = match cached {
            Some(m) => m,
            None => {
                let Ok(found) = self.ytm.moods().await else { return Vec::new() };
                *self.moods.lock().unwrap() = Some(found.clone());
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
        all(jobs).await.into_iter().flatten().map(|t| Cand::of(t, 1.5 - 0.45 * rnd(), Src::Other)).collect()
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

    async fn familiar_cands(self: &Arc<Self>) -> Vec<Cand> {
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
            async move { this.ytm.artist(&a.id).await.map(|p| p.top).unwrap_or_default() }
        });
        all(jobs).await.into_iter().flatten().map(|t| Cand::of(t, 0.9, Src::Other)).collect()
    }

    pub async fn wave(self: &Arc<Self>, knobs: &WaveKnobs, recent: &[Track]) -> Result<Vec<Track>> {
        let n = knobs.limit.unwrap_or(WAVE_DEFAULT).clamp(1, 100);
        let cache_key = serde_json::to_string(&WaveKnobs { reshuffle: false, ..knobs.clone() })?;
        if !knobs.reshuffle {
            let kept = self.waves.lock().unwrap().get(&cache_key).cloned();
            if let Some(tracks) = kept {
                return Ok(tracks);
            }
        }
        let tracks = self.build_wave(knobs, recent, n).await?;
        self.waves.lock().unwrap().insert(cache_key, tracks.clone());
        Ok(tracks)
    }

    async fn build_wave(self: &Arc<Self>, knobs: &WaveKnobs, recent: &[Track], n: usize) -> Result<Vec<Track>> {
        let taste = self.taste();
        let weight = if knobs.mood.is_some() { 0.25 } else { 1.0 };
        let seeds: Vec<_> = self.anchors(&taste, 3, "").into_iter().map(|(t, w)| (t, w * weight)).collect();
        let exclude: HashSet<String> = recent.iter().map(track_key).collect();
        let flavor = self.flavor(knobs);
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
            tracks =
                self.songs(rank(cands, &taste, &exclude, false), n, false, &taste, (flavor.explore, flavor.exact), HOME_DEADLINE).await;
        }
        self.flush_memo().await;
        Ok(tracks)
    }

    pub async fn next_tracks(self: &Arc<Self>, seed: &Track, recent: &[Track], n: usize) -> Result<Vec<Track>> {
        let prepared = self.ready.lock().unwrap().remove(seed.id.as_str());
        if let Some(mut tracks) = prepared {
            tracks.retain(|t| !recent.iter().any(|r| r.id == t.id));
            if tracks.len() >= n / 2 {
                tracks.truncate(n);
                return Ok(tracks);
            }
        }
        self.compute_next(seed, recent, n).await
    }

    pub fn forget(&self) {
        self.waves.lock().unwrap().clear();
        self.ready.lock().unwrap().clear();
        *self.home.lock().unwrap() = None;
        *self.home_failure.lock().unwrap() = None;
        let _ = std::fs::remove_file(self.cache_dir.join("home.json"));
    }

    pub fn warm(self: &Arc<Self>, seed: &Track) {
        {
            let mut last = self.warmed.lock().unwrap();
            let ready = self.ready.lock().unwrap().contains_key(seed.id.as_str());
            if ready || last.is_some_and(|at| at.elapsed() < WARM_GAP) {
                return;
            }
            *last = Some(Instant::now());
        }
        let (this, seed) = (self.clone(), seed.clone());
        self.tasks.spawn("recs warm", async move {
            if let Ok(tracks) = this.compute_next(&seed, &[], 25).await {
                this.ready.lock().unwrap().insert(seed.id.to_string(), tracks);
            }
        });
    }

    async fn compute_next(self: &Arc<Self>, seed: &Track, recent: &[Track], n: usize) -> Result<Vec<Track>> {
        let taste = self.taste();
        let mut seeds = vec![(seed.clone(), 1.0)];
        seeds.extend(self.anchors(&taste, 2, &primary_artist(seed)).into_iter().map(|(t, w)| (t, w * 0.35)));
        let mut exclude: HashSet<String> = recent.iter().map(track_key).collect();
        exclude.insert(track_key(seed));
        let flavor = self.flavor(&self.knobs());
        let mut tracks = self.recommend(&seeds, &taste, &exclude, &flavor, n, AUTOPLAY_DEADLINE).await;
        tracks.retain(|t| !recent.iter().any(|r| r.id == t.id) && t.id != seed.id);
        self.flush_memo().await;
        if tracks.is_empty() {
            return self.ytm.radio(&seed.id).await;
        }
        Ok(tracks)
    }

    async fn artist_card(&self, name: &str) -> Option<ArtistCard> {
        let k = norm(name);
        if let Some(hit) = self.cards.lock().unwrap().get(&k) {
            return hit.clone();
        }
        let _permit = self.gate.acquire().await.ok()?;
        let found = self.ytm.search_quiet(name, Filter::Artists).await.ok()?;
        let card = found.artists.into_iter().find(|a| norm(&a.name) == k);
        self.cards.lock().unwrap().insert(k, card.clone());
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
        let ranked = rank(self.pool(seeds, HOME_DEADLINE / 2).await, taste, &HashSet::new(), true);
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
        let tracks = self.recommend(&[seed], taste, shown, &self.flavor(&WaveKnobs::default()), n, HOME_DEADLINE).await;
        Shelf { title, kind: "becauseLiked".into(), arg: artist, tracks, ..Default::default() }
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
        let tracks = self.songs(cands, 15, false, &Taste::default(), (0.0, false), HOME_DEADLINE).await;
        build.push(0, Shelf { title: "Trending now".into(), kind: "trending".into(), tracks, ..Default::default() });
    }

    async fn popular_artists(self: &Arc<Self>, build: &Build) {
        let Ok(chart) = self.get("https://api.deezer.com/chart/0/artists", &[("limit", "12")]).await else { return };
        let names = chart["data"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str().map(String::from)).collect();
        let artists = self.artist_cards(names).await;
        build.push(1, Shelf { title: "Popular artists".into(), kind: "popularArtists".into(), artists, ..Default::default() });
    }

    async fn personal(self: &Arc<Self>, build: &Arc<Build>, fresh: bool) {
        let taste = Arc::new(self.taste());
        if taste.anchors.is_empty() {
            tokio::join!(self.trending_tracks(build), self.popular_artists(build));
            return;
        }
        let shown = Arc::new(match self.home.lock().unwrap().as_ref() {
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
            let knobs = WaveKnobs { reshuffle: fresh, ..self.knobs() };
            let mut tracks = self.wave(&knobs, &shown.tracks).await.unwrap_or_default();
            if tracks.len() < WAVE_PREVIEW && !shown.tracks.is_empty() {
                tracks = self.wave(&knobs, &[]).await.unwrap_or_default();
            }
            build.push(0, Shelf { title: "For you".into(), kind: "forYou".into(), tracks, ..Default::default() });
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
                    for (i, shelf) in shelves.into_iter().enumerate() {
                        build.push(100 + i, shelf);
                    }
                }
                Err(e) => *build.failure.lock().unwrap() = Some(HomeFailure::of(&e)),
            }
        };
        tokio::join!(self.personal(build, fresh), ytm);
        let shelves = build.snapshot();
        let failure = build.failure.lock().unwrap().clone();
        let current = self.build.lock().unwrap().as_ref().is_some_and(|b| Arc::ptr_eq(b, build));
        if current {
            *self.home_failure.lock().unwrap() = failure.clone();
        }
        if !shelves.is_empty() && failure.is_none() {
            let at = unix_now();
            if current {
                if let Ok(bytes) = serde_json::to_vec(&json!({"at": at, "shelves": shelves})) {
                    let _ = tokio::fs::write(self.cache_dir.join("home.json"), bytes).await;
                }
                *self.home.lock().unwrap() = Some((at, shelves));
            }
        }
        self.flush_memo().await;
        build.done.send_replace(true);
    }

    fn start_build(self: &Arc<Self>, fresh: bool) -> Arc<Build> {
        let mut slot = self.build.lock().unwrap();
        if let Some(running) = slot.as_ref().filter(|b| !*b.done.borrow() && (b.fresh || !fresh)) {
            return running.clone();
        }
        let build = Arc::new(Build { failure: Mutex::default(), fresh, shelves: Mutex::default(), done: watch::channel(false).0 });
        *slot = Some(build.clone());
        let (this, task) = (self.clone(), build.clone());
        self.tasks.spawn("home build", async move { this.build_home(&task, fresh).await });
        build
    }

    async fn home_with(self: &Arc<Self>, force: bool) -> Result<Vec<Shelf>> {
        let cached = self.home.lock().unwrap().clone();
        if let Some((at, shelves)) = &cached
            && !force
        {
            if unix_now().saturating_sub(*at) > HOME_TTL {
                self.start_build(false);
            }
            return Ok(shelves.clone());
        }
        let build = self.start_build(force);
        let mut done = build.done.subscribe();
        let budget = if force { REFRESH_BUDGET } else { HOME_BUDGET };
        let finished = tokio::time::timeout(budget, done.wait_for(|d| *d)).await.is_ok();
        let ready = build.snapshot();
        let failure = build.failure.lock().unwrap().clone();
        if failure.is_some() {
            *self.home_failure.lock().unwrap() = failure.clone();
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
        self.home_failure.lock().unwrap().as_ref().map(|f| (f.code, f.after))
    }
}

#[cfg(test)]
mod eval;
#[cfg(test)]
mod latency;
#[cfg(test)]
mod scene;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fake_music;
    use snellar_core::model::Ref;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_mood_reads_only_the_first_page_of_each_playlist() {
        let long: Value = serde_json::from_str(include_str!("../../../tests/fixtures/youtube/playlist-long-2026-11-03.json")).unwrap();
        let next: Value = serde_json::from_str(include_str!("../../../tests/fixtures/youtube/playlist-long-next-2026-11-03.json")).unwrap();
        let moods = json!({"grid": [{"musicNavigationButtonRenderer": {
            "buttonText": {"runs": [{"text": "Chill"}]}, "clickCommand": {"browseEndpoint": {"params": "chill-params"}}
        }}]});
        let cards = ["PLone", "PLtwo", "PLthree"]
            .map(|id| json!({"musicTwoRowItemRenderer": {"navigationEndpoint": {"browseEndpoint": {"browseId": format!("VL{id}")}}}}));
        let lists = json!({ "grid": cards });
        let music = fake_music::serve(
            move |body| match body["browseId"].as_str() {
                Some("FEmusic_moods_and_genres") => moods.clone(),
                Some("FEmusic_moods_and_genres_category") => lists.clone(),
                Some(_) => long.clone(),
                None => next.clone(),
            },
            None,
        )
        .await;
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        let dir = std::env::temp_dir().join(format!("snellar-recs-mood-{}", std::process::id()));
        let recs = Recs::new(reqwest::Client::new(), music.ytm.clone(), library, dir);
        let found = recs.mood_cands("Chill").await;
        let seen = music.seen.lock().unwrap();
        assert_eq!(seen.iter().filter(|b| b.get("continuation").is_some()).count(), 0, "a top-up does not page through whole playlists");
        assert_eq!(found.len(), 300, "three first pages of 100");
    }

    fn track(id: &str, title: &str, artist: &str, duration: f64) -> Track {
        Track {
            id: id.into(),
            title: title.into(),
            artists: vec![Ref { id: String::new(), name: artist.into() }],
            duration,
            ..Default::default()
        }
    }

    fn cand(artist: &str, title: &str, score: f64) -> Cand {
        Cand { artist: artist.into(), title: title.into(), score, pop: 0.0, duration: 0.0, track: None, src: Src::Outer }
    }

    #[test]
    fn matching_rejects_variants_and_wrong_length() {
        let c = Cand { duration: 238.0, ..cand("Radiohead", "Creep", 1.0) };
        let results = vec![
            track("a", "Creep (Live)", "Radiohead", 240.0),
            track("b", "Creep", "Radiohead", 260.0),
            track("c", "Creep - Remix", "Radiohead", 238.0),
            track("d", "Creep", "Radiohead", 239.0),
        ];
        assert_eq!(pick(&c, results.clone(), false).unwrap().id, "d");
        assert_eq!(pick(&c, results, true).unwrap().id, "a");
    }

    #[test]
    fn ranking_uses_taste_and_agreement() {
        let mut taste = Taste::default();
        taste.skipped_artists.insert("bad".into(), 3.0);
        taste.heard.insert(key("Heard", "Song"));
        let ranked = rank(
            vec![cand("Heard", "Song", 9.0), cand("Bad", "One", 1.0), cand("Good", "Two", 0.8), cand("Good", "Two", 0.8)],
            &taste,
            &HashSet::new(),
            false,
        );
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].artist, "Good");
    }

    fn radio_cand(artist: &str, title: &str) -> Cand {
        Cand { src: Src::Radio, ..cand(artist, title, 1.0) }
    }

    #[test]
    fn outside_sources_stay_inside_the_scene() {
        let mut cands: Vec<Cand> = (0..SCENE_MIN).map(|i| radio_cand(&format!("scene{i}"), "song")).collect();
        cands.push(cand("Scene3", "other song", 1.0));
        cands.push(cand("Mainstream Star", "hit", 1.0));
        let ranked = rank(cands, &Taste::default(), &HashSet::new(), false);
        assert!(ranked.iter().any(|c| c.title == "other song"));
        assert!(!ranked.iter().any(|c| c.artist == "Mainstream Star"));
        let outer = ranked.iter().find(|c| c.title == "other song").unwrap();
        assert!(outer.score < 1.0);
    }

    #[test]
    fn without_a_radio_outside_sources_are_kept() {
        let cands = vec![cand("A", "x", 1.0), cand("B", "y", 1.0)];
        assert_eq!(rank(cands, &Taste::default(), &HashSet::new(), false).len(), 2);
    }

    #[test]
    fn diversity_limits_artists_and_titles() {
        let tracks = vec![
            track("1", "A", "x", 1.0),
            track("2", "B", "x", 1.0),
            track("3", "C", "x", 1.0),
            track("4", "D", "y", 1.0),
            track("5", "D (Live)", "z", 1.0),
        ];
        let out = diversify(tracks, 10);
        assert_eq!(out.iter().filter(|t| primary_artist(t) == "x").count(), 2);
        assert!(out.windows(2).all(|w| primary_artist(&w[0]) != primary_artist(&w[1])) || out.len() < 4);
        assert!(!out.iter().any(|t| t.id == "5"));
    }

    #[test]
    fn versions_share_a_key() {
        assert_eq!(key("Cure", "Boys Don't Cry (86 Mix / 2026 Remaster)"), key("The Cure", "Boys Don't Cry").replace("the cure", "cure"));
        assert_eq!(canonical_title("Song - Radio Edit"), canonical_title("Song (Live)"));
        assert_eq!(canonical_title("(Untitled)"), "untitled");
    }

    #[test]
    fn explore_guarantees_a_minimum_of_new_artists() {
        let known: HashSet<String> = ["old".to_string()].into();
        let tracks: Vec<Track> =
            (0..20).map(|i| track(&i.to_string(), "t", &if i < 12 { "old".to_string() } else { format!("new{i}") }, 1.0)).collect();
        let fresh = |list: &[Track]| list.iter().take(10).filter(|t| !known.contains(&primary_artist(t))).count();
        assert_eq!(fresh(&balance(tracks.clone(), &known, (0.0, true), 10)), 0);
        assert_eq!(fresh(&balance(tracks, &known, (0.4, true), 10)), 4);
        let all_fresh: Vec<Track> = (0..10).map(|i| track(&i.to_string(), "t", &format!("n{i}"), 1.0)).collect();
        assert_eq!(balance(all_fresh, &known, (0.9, true), 10).len(), 10);
    }

    #[test]
    fn language_fit_separates_scripts() {
        assert!(Lang::Uk.fit("Без бою Океан Ельзи їжак") > Lang::Uk.fit("Кино Группа крови ы"));
        assert!(Lang::Uk.fit("Kalush Stefania") < 0.2);
        assert!(Lang::En.fit("Daft Punk") > Lang::En.fit("Океан Ельзи"));
        assert!(Lang::Ko.fit("강남스타일 PSY") > Lang::Ko.fit("Gangnam Style"));
        assert!(Lang::Ja.fit("アイドル YOASOBI") > Lang::Zh.fit("アイドル YOASOBI"));
    }

    #[test]
    fn dislike_and_skips_shape_taste() {
        let (recs, _, library) = {
            let http = reqwest::Client::new();
            let ytm = Arc::new(Ytm::new(http.clone()));
            let dir = std::env::temp_dir().join(format!("snellar-taste-{}", rnd().to_bits()));
            std::fs::create_dir_all(&dir).unwrap();
            let library = Arc::new(Library::open(&dir.join("library.db")).unwrap());
            (Recs::new(http, ytm.clone(), library.clone(), dir), ytm, library)
        };
        let early = track("early", "Skipped Early", "A", 200.0);
        let late = track("late", "Skipped Late", "B", 200.0);
        let full = track("full", "Played Fully", "C", 200.0);
        let hated = track("hated", "Not Interested", "D", 200.0);
        for t in [&early, &late, &full, &hated] {
            library.push_history(t);
        }
        library.record_listen(&"early".into(), 4.0, true);
        library.record_listen(&"late".into(), 20.0, true);
        library.record_listen(&"full".into(), 190.0, false);
        library.dislike(&hated);
        let taste = recs.taste();
        assert!(taste.skipped_artists["a"] > taste.skipped_artists["b"]);
        assert!(taste.skipped.contains(&track_key(&early)) && taste.skipped.contains(&track_key(&late)));
        assert!(taste.liked["c"] > 0.0 && taste.anchors.iter().any(|(t, _)| t.id == "full"));
        assert!(taste.disliked.contains(&track_key(&hated)) && taste.disliked_artists.contains("d"));
        let ranked = rank(
            vec![cand("D", "Not Interested", 1.0), cand("D", "Other Song", 1.0), cand("E", "Fine", 1.0)],
            &taste,
            &HashSet::new(),
            false,
        );
        assert_eq!(ranked.iter().map(|c| c.title.as_str()).collect::<Vec<_>>().first(), Some(&"Fine"));
        assert!(!ranked.iter().any(|c| c.title == "Not Interested"));
    }

    #[test]
    fn tracks_in_playlists_shape_taste_below_a_like() {
        let (recs, _, library) = setup();
        let liked = track("liked", "Liked Song", "Loved", 200.0);
        library.toggle_like(&liked);
        let (kept, skipped, hated) = (
            track("kept", "Kept Song", "Kept", 200.0),
            track("skip", "Skipped Song", "Passed", 200.0),
            track("hate", "Hated Song", "Hated", 200.0),
        );
        library.push_history(&skipped);
        library.record_listen(&skipped.id, 3.0, true);
        library.dislike(&hated);
        library.create_filled_playlist("mine", &[kept.clone(), skipped.clone(), hated.clone()]).unwrap();

        let taste = recs.taste();
        assert!(taste.listed["kept"] > 0.0 && taste.listed["kept"] < taste.liked["loved"]);
        assert!(!taste.liked.contains_key("kept"), "the shelves named after likes keep reading likes only");
        let weight = |id: &str| taste.anchors.iter().find(|(t, _)| t.id == id).map(|(_, w)| *w);
        assert!(weight("kept").unwrap() < weight("liked").unwrap());
        assert_eq!((weight("skip"), weight("hate")), (None, None), "a skip or a dislike outranks being in a playlist");
        assert!(!taste.listed.contains_key("hated") && !taste.listed.contains_key("passed"));
        assert!(taste.heard.contains(&track_key(&kept)) && taste.known_artists.contains("kept"));
    }

    #[test]
    fn a_long_playlist_cannot_outweigh_the_likes() {
        let (recs, _, library) = setup();
        let songs: Vec<Track> = (0..200).map(|i| track(&format!("t{i}"), &format!("Song {i}"), "Prolific", 200.0)).collect();
        library.create_filled_playlist("import", &songs).unwrap();
        library.toggle_like(&track("liked", "Liked Song", "Loved", 200.0));
        let taste = recs.taste();
        assert!(taste.listed["prolific"] <= LISTED_ARTIST_CAP);
        assert!(taste.anchors.iter().filter(|(t, _)| t.id != "liked").count() <= LISTED_ANCHORS);
        assert!(taste.heard.contains(&key("Prolific", "Song 150")), "all of it counts as already known");
    }

    #[test]
    fn the_playlist_signal_raises_a_candidate_less_than_a_like() {
        let mut taste = Taste::default();
        taste.liked.insert("loved".into(), 1.0);
        taste.listed.insert("listed".into(), LISTED_ARTIST_CAP);
        let score = |artist: &str| {
            let best = (0..40).map(|_| rank(vec![cand(artist, "Song", 1.0)], &taste, &HashSet::new(), false)[0].score);
            best.fold(f64::MAX, f64::min)
        };
        assert!(score("listed") > score("nobody"));
        assert!(score("loved") > score("listed"));
    }

    #[test]
    fn a_refresh_remembers_what_was_on_screen() {
        let shelf = Shelf {
            title: "For you".into(),
            tracks: vec![track("a", "Song A", "Artist One", 100.0), track("b", "Song B", "Artist Two", 100.0)],
            artists: vec![ArtistCard { id: "x".into(), name: "The Band".into(), ..Default::default() }],
            ..Default::default()
        };
        let shown = shown_in(&[shelf]);
        assert_eq!(shown.tracks.len(), 2);
        assert!(shown.keys.contains(&key("Artist One", "Song A")) && shown.keys.contains(&key("artist two", "song b")));
        assert!(shown.artists.contains(&norm("the band")));
    }

    #[test]
    fn artists_not_shown_before_come_first_and_the_rest_fill_the_shelf() {
        let shown: HashSet<String> = ["old one", "old two"].iter().map(|n| norm(n)).collect();
        let names = ["Old One", "Fresh A", "Old Two", "Fresh B"].map(String::from).to_vec();
        assert_eq!(unseen_first(names.clone(), &shown), ["Fresh A", "Fresh B", "Old One", "Old Two"]);
        assert_eq!(unseen_first(names.clone(), &HashSet::new()), names);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_home_is_not_kept_and_the_next_visit_asks_again() {
        let full: Value = serde_json::from_str(include_str!("../../../tests/fixtures/youtube/home-home-2026-11-03.json")).unwrap();
        let empty = json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": []}}}}]}}});
        let answer_full = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = answer_full.clone();
        let music = fake_music::serve(move |_| if flag.load(Ordering::Relaxed) { full.clone() } else { empty.clone() }, None).await;
        let dead = reqwest::Client::builder().proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap()).build().unwrap();
        let dir = memo_dir("empty-home");
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        let recs = Recs::new(dead, music.ytm.clone(), library, dir.clone());

        assert!(recs.home().await.unwrap().is_empty());
        assert!(recs.home.lock().unwrap().is_none(), "nothing is held in memory");
        assert!(!dir.join("home.json").exists(), "nothing is written to disk");

        answer_full.store(true, Ordering::Relaxed);
        assert!(!recs.home().await.unwrap().is_empty(), "the next visit asks again");
        assert!(recs.home.lock().unwrap().is_some() && dir.join("home.json").exists(), "a real answer is kept");
    }

    fn failing_music() -> Arc<Ytm> {
        Arc::new(Ytm::at(reqwest::Client::builder().no_proxy().build().unwrap(), "http://127.0.0.1:9".into()))
    }

    fn home_recs(name: &str, ytm: Arc<Ytm>) -> (Arc<Recs>, std::path::PathBuf) {
        let dead = reqwest::Client::builder().proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap()).build().unwrap();
        let dir = memo_dir(name);
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        (Recs::new(dead, ytm, library, dir.clone()), dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_home_with_nothing_to_show_says_why_and_is_not_kept() {
        let (recs, dir) = home_recs("home-fails", failing_music());
        let e = recs.refresh_home().await.unwrap_err();
        assert!(Code::of(&e).network(), "{e:#}");
        assert!(recs.home.lock().unwrap().is_none(), "the failure is not held");
        assert!(!dir.join("home.json").exists(), "the failure is not written");
        assert_eq!(recs.home_problem().map(|p| p.0), Some(Code::of(&e)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_home_keeps_what_was_there_and_tells_it_is_old() {
        let (recs, dir) = home_recs("home-stale", failing_music());
        let kept = Shelf { title: "Kept".into(), ..Default::default() };
        *recs.home.lock().unwrap() = Some((unix_now() - 3600, vec![kept]));
        let shelves = recs.refresh_home().await.unwrap();
        assert_eq!(shelves[0].title, "Kept");
        assert!(unix_now() - recs.home.lock().unwrap().as_ref().unwrap().0 >= 3600, "the old answer stays old");
        assert!(!dir.join("home.json").exists(), "nothing new is written");
        assert!(recs.home_problem().is_some(), "the note is there");
    }

    #[test]
    fn forgetting_drops_what_was_built_from_the_history() {
        let (recs, _, _) = setup();
        recs.waves.lock().unwrap().insert("k".into(), vec![track("a", "A", "x", 1.0)]);
        recs.ready.lock().unwrap().insert("a".into(), vec![track("b", "B", "y", 1.0)]);
        *recs.home.lock().unwrap() = Some((unix_now(), vec![Shelf::default()]));
        let file = recs.cache_dir.join("home.json");
        std::fs::write(&file, "{}").unwrap();
        recs.forget();
        assert!(recs.waves.lock().unwrap().is_empty() && recs.ready.lock().unwrap().is_empty());
        assert!(recs.home.lock().unwrap().is_none() && !file.exists());
    }

    #[tokio::test]
    async fn musicbrainz_is_asked_at_most_once_per_gap_even_from_parallel_seeds() {
        let pacer = Arc::new(Pacer::new(Duration::from_millis(150)));
        let started = Instant::now();
        let waits: Vec<_> = (0..3)
            .map(|_| {
                tokio::spawn({
                    let pacer = pacer.clone();
                    async move { pacer.wait().await }
                })
            })
            .collect();
        for w in waits {
            w.await.unwrap();
        }
        assert!(started.elapsed() >= Duration::from_millis(300), "{:?}", started.elapsed());
    }

    #[test]
    fn artist_names_with_quotes_stay_one_search_term() {
        assert_eq!(brainz_query("Say \"Hi\" \\ Co"), "artist:\"Say \\\"Hi\\\" \\\\ Co\"");
        assert_eq!(brainz_query("Radiohead"), "artist:\"Radiohead\"");
    }

    #[test]
    fn requests_name_this_project_not_another() {
        assert!(!USER_AGENT.contains("rootscripts") && USER_AGENT.starts_with(snellar_core::product::NAME));
    }

    fn memo_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-memo-{name}-{}", rnd().to_bits()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn recs_in(dir: &std::path::Path) -> Arc<Recs> {
        let http = reqwest::Client::new();
        let library = Arc::new(Library::open(std::path::Path::new(":memory:")).unwrap());
        Recs::new(http.clone(), Arc::new(Ytm::new(http)), library, dir.to_path_buf())
    }

    #[tokio::test]
    async fn the_memo_file_expires_old_entries_stays_within_its_cap_and_is_written_whole() {
        let dir = memo_dir("file");
        let day = 86_400;
        let now = unix_now();
        let hit = |n: usize, age: u64| json!([format!("k{n}"), now - age, track("id", "T", "A", 1.0)]);
        let mut matches = vec![hit(0, 40 * day), hit(1, day)];
        matches.extend((2..MEMO_CAP + 100).map(|n| hit(n, 60)));
        std::fs::write(dir.join("recs-memo.json"), json!({ "matches": matches, "cards": [] }).to_string()).unwrap();
        let recs = recs_in(&dir);
        {
            let kept = recs.matches.lock().unwrap();
            assert!(kept.get("k0").is_none(), "an entry older than the memo ttl was loaded");
            assert!(kept.len() <= MEMO_CAP, "{} entries", kept.len());
        }
        recs.matches.lock().unwrap().insert("fresh".into(), Some(track("id2", "T2", "A2", 1.0)));
        recs.flush_memo().await;
        let written: Value = serde_json::from_slice(&std::fs::read(dir.join("recs-memo.json")).unwrap()).unwrap();
        assert!(written["matches"].as_array().unwrap().len() <= MEMO_CAP);
        assert!(!dir.join("recs-memo.json.tmp").exists());
    }

    #[test]
    fn lastfm_parse() {
        let v = json!({"similartracks": {"track": [{"name": "Karma Police", "match": "0.8", "duration": 264, "artist": {"name": "Radiohead"}}]}});
        let c = lastfm_cands(&v);
        assert_eq!((c[0].title.as_str(), c[0].score, c[0].duration), ("Karma Police", 0.8, 264.0));
    }

    fn setup() -> (Arc<Recs>, Arc<Ytm>, Arc<Library>) {
        let dir = std::env::temp_dir().join(format!("snellar-recs-cache-{}", rnd().to_bits()));
        std::fs::create_dir_all(&dir).unwrap();
        let http = reqwest::Client::new();
        let ytm = Arc::new(Ytm::new(http.clone()));
        let path = std::env::temp_dir().join(format!("snellar-recs-{}.db", rnd().to_bits()));
        let library = Arc::new(Library::open(&path).unwrap());
        (Recs::new(http, ytm.clone(), library.clone(), dir), ytm, library)
    }

    async fn seed(ytm: &Ytm, q: &str) -> Track {
        ytm.search(q, Filter::Songs).await.unwrap().tracks.remove(0)
    }

    fn show(t: &Track) -> String {
        format!("{} - {}", t.title, t.artist_line())
    }

    #[tokio::test]
    #[ignore]
    async fn live_sources() {
        let (recs, ytm, _) = setup();
        let s = seed(&ytm, "radiohead creep").await;
        let t = Instant::now();
        let dz = recs.deezer_related("Radiohead").await.unwrap();
        println!(
            "deezer {:?}: {} e.g. {:?}",
            t.elapsed(),
            dz.len(),
            dz.iter().take(3).map(|c| format!("{} - {}", c.title, c.artist)).collect::<Vec<_>>()
        );
        let t = Instant::now();
        let lb = recs.listenbrainz("Radiohead").await.unwrap();
        println!(
            "listenbrainz {:?}: {} e.g. {:?}",
            t.elapsed(),
            lb.len(),
            lb.iter().take(3).map(|c| format!("{} - {}", c.title, c.artist)).collect::<Vec<_>>()
        );
        let t = Instant::now();
        let radio = recs.ytm_radio(&s).await.unwrap();
        println!("ytm radio {:?}: {}", t.elapsed(), radio.len());
        assert!(!dz.is_empty() && !lb.is_empty() && !radio.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn live_next_tracks() {
        let (recs, ytm, _) = setup();
        for q in [
            "radiohead creep",
            "kendrick lamar humble",
            "daft punk get lucky",
            "rammstein du hast",
            "miles davis so what",
            "bad bunny titi me pregunto",
            "ЛСП Монетка",
        ] {
            let s = seed(&ytm, q).await;
            let t = Instant::now();
            let ours = recs.next_tracks(&s, &[], 20).await.unwrap();
            let took = t.elapsed();
            let plain = ytm.radio(&s.id).await.unwrap();
            println!("\n=== seed: {} ({:?}, {} picks)", show(&s), took, ours.len());
            for (i, t) in ours.iter().enumerate() {
                let yt = plain.get(i).map_or(String::new(), show);
                println!("{:>2}. {:<60.60} | ytm: {}", i + 1, show(t), yt);
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_home_cold_and_warm() {
        let (recs, ytm, lib) = setup();
        let dir = recs.cache_dir.clone();
        let t = Instant::now();
        let first = recs.home().await.unwrap();
        println!(
            "\nfirst open, no cache: {:?}, {} shelves ready: {:?}",
            t.elapsed(),
            first.len(),
            first.iter().map(|s| s.title.as_str()).collect::<Vec<_>>()
        );
        let mut done = recs.start_build(false).done.subscribe();
        done.wait_for(|d| *d).await.unwrap();
        println!("background build finished {:?} after start", t.elapsed());
        let t = Instant::now();
        let second = recs.home().await.unwrap();
        println!(
            "second open (memory): {:?}, {} shelves: {:?}",
            t.elapsed(),
            second.len(),
            second.iter().map(|s| s.title.as_str()).collect::<Vec<_>>()
        );
        let restarted = Recs::new(reqwest::Client::new(), ytm.clone(), lib.clone(), dir);
        let t = Instant::now();
        let third = restarted.home().await.unwrap();
        println!("after restart (disk): {:?}, {} shelves", t.elapsed(), third.len());
        assert!(t.elapsed() < Duration::from_millis(100) && !third.is_empty());

        for q in ["radiohead creep", "portishead glory box", "massive attack teardrop", "thom yorke black swan"] {
            let s = seed(&ytm, q).await;
            lib.push_history(&s);
            lib.toggle_like(&s);
        }
        let t = Instant::now();
        let refreshed = restarted.refresh_home().await.unwrap();
        println!("\nforced refresh with likes: {:?}", t.elapsed());
        for s in &refreshed {
            println!("  {} | tracks={} artists={}", s.title, s.tracks.len(), s.artists.len());
            s.tracks.iter().take(4).for_each(|t| println!("      {}", show(t)));
        }
        assert!(!refreshed.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn live_home_timings() {
        let (recs, ytm, lib) = setup();
        let stage = |name: &str, t: Instant| println!("{name:<34} {:?}", t.elapsed());
        let t = Instant::now();
        let dz = recs.get("https://api.deezer.com/chart/0/tracks", &[("limit", "40")]).await.unwrap();
        stage("deezer chart tracks", t);
        let cands: Vec<Cand> =
            dz["data"].as_array().unwrap().iter().enumerate().filter_map(|(i, t)| dz_cand(t, 1.0 - 0.5 * i as f64 / 40.0)).collect();
        let t = Instant::now();
        let songs = recs.songs(cands, 15, false, &Taste::default(), (0.0, false), HOME_DEADLINE).await;
        stage(&format!("match chart candidates until 15 ({})", songs.len()), t);
        let t = Instant::now();
        let names = recs.get("https://api.deezer.com/chart/0/artists", &[("limit", "12")]).await.unwrap();
        let names: Vec<String> = names["data"].as_array().unwrap().iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
        stage("deezer chart artists", t);
        let t = Instant::now();
        let cards = recs.artist_cards(names).await;
        stage(&format!("12 artist cards ({})", cards.len()), t);
        let t = Instant::now();
        let h = ytm.home().await.unwrap();
        stage(&format!("ytm.home ({} shelves)", h.len()), t);

        for q in ["radiohead creep", "portishead glory box", "massive attack teardrop"] {
            let s = seed(&ytm, q).await;
            lib.push_history(&s);
            lib.toggle_like(&s);
        }
        let t = Instant::now();
        let taste = recs.taste();
        stage("taste()", t);
        let seeds = recs.anchors(&taste, 3, "");
        for (s, _) in &seeds {
            let t = Instant::now();
            let radio = recs.ytm_radio(s).await.map(|c| c.len());
            stage(&format!("  ytm radio {:?}", radio.ok()), t);
            let t = Instant::now();
            let dz = recs.deezer_related(&s.artists[0].name).await.map(|c| c.len());
            stage(&format!("  deezer related {:?}", dz.ok()), t);
            let t = Instant::now();
            let lb = recs.listenbrainz(&s.artists[0].name).await.map(|c| c.len());
            stage(&format!("  listenbrainz {:?}", lb.ok()), t);
        }
        let t = Instant::now();
        let pool = recs.pool(&seeds, Duration::from_secs(6)).await;
        stage(&format!("pool of 3 seeds ({} cands)", pool.len()), t);
        let t = Instant::now();
        let ranked = rank(pool, &taste, &HashSet::new(), false);
        stage("rank", t);
        let t = Instant::now();
        let tracks = recs.songs(ranked, 20, false, &taste, (0.3, false), HOME_DEADLINE).await;
        stage(&format!("match until 20 ({})", tracks.len()), t);
        let (fresh, _, _) = setup();
        let t = Instant::now();
        let build = fresh.start_build(false);
        build.done.subscribe().wait_for(|d| *d).await.unwrap();
        stage(&format!("full build, cold memo ({} shelves)", build.snapshot().len()), t);
    }
}
