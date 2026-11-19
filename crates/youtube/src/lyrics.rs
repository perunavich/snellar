use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::Value;

use snellar_core::error::Code;
use snellar_core::model::{LyricLine, Lyrics, Track};
use snellar_core::product::USER_AGENT;

use crate::net::{self, Http};
use crate::ytm::Ytm;

const NOISE: [&str; 14] =
    ["official", "video", "audio", "lyric", "visualizer", "remaster", "live", "feat", "ft.", "version", "mono", "stereo", "hd", "4k"];
const TIMEOUT: Duration = Duration::from_secs(6);
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";
const TAIL_NOISE: [&str; 7] = ["remaster", "live", "version", "mono", "stereo", "edit", "single"];
const COMPARABLE_PAIRS: usize = 8;
const SHARED_PAIRS: f64 = 0.3;
const MAIN_SCRIPT: f64 = 0.2;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Entry {
    track_name: String,
    artist_name: String,
    duration: f64,
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

pub struct LyricsSource {
    http: Http,
    ytm: Ytm,
}

pub(crate) fn norm(s: &str) -> String {
    let lower: String = s.chars().flat_map(char::to_lowercase).map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    lower.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn title_forms(title: &str) -> Vec<String> {
    let cleaned = clean_title(title);
    let mut outside = String::new();
    let mut forms = Vec::new();
    let mut rest = cleaned.as_str();
    while let Some(start) = rest.find(['(', '[']) {
        let close = if rest[start..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[start..].find(close) else { break };
        outside.push_str(&rest[..start]);
        let inner = &rest[start + 1..start + len];
        if inner.chars().any(|c| c.is_alphabetic() && !c.is_ascii()) {
            forms.push(norm(inner));
        }
        rest = &rest[start + len + 1..];
    }
    outside.push_str(rest);
    forms.push(norm(&outside));
    forms.push(norm(&cleaned));
    forms.retain(|f| !f.is_empty());
    forms
}

fn strict_match(track: &Track, title: &str, artists: &[&str]) -> bool {
    let found = title_forms(title);
    if !title_forms(&track.title).iter().any(|w| found.contains(w)) {
        return false;
    }
    let names: Vec<String> = artists.iter().flat_map(|a| a.split(['、', ',', '&', '/'])).map(norm).collect();
    track.artists.iter().any(|a| names.contains(&norm(a.name.trim_end_matches(" - Topic"))))
}

const GENERIC_TITLES: [&str; 9] = ["intro", "outro", "interlude", "skit", "prelude", "prologue", "epilogue", "untitled", "instrumental"];

fn is_generic(title: &str) -> bool {
    let t = norm(&clean_title(title));
    GENERIC_TITLES.contains(&t.as_str()) || t.strip_prefix("track ").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
}

fn near(track: &Track, seconds: f64) -> bool {
    let known = track.duration > 0.0 && seconds > 0.0;
    if is_generic(&track.title) {
        return known && (track.duration - seconds).abs() <= 2.0;
    }
    !known || (track.duration - seconds).abs() <= 4.0
}

fn confirmed(track: &Track, title: &str, artists: &[&str], seconds: f64) -> bool {
    near(track, seconds) && strict_match(track, title, artists)
}

fn verdict(track: &Track, e: &Entry) -> Option<Outcome> {
    if !confirmed(track, &e.track_name, &[&e.artist_name], e.duration) {
        return None;
    }
    if e.instrumental {
        return Some(Outcome::Instrumental(Vec::new()));
    }
    lyrics_of(e).map(|l| Outcome::Found(vec![l]))
}

enum Outcome {
    Found(Vec<Lyrics>),
    Instrumental(Vec<Lyrics>),
    Missing,
}

impl From<Option<Lyrics>> for Outcome {
    fn from(found: Option<Lyrics>) -> Outcome {
        found.map_or(Outcome::Missing, |l| Outcome::Found(vec![l]))
    }
}

async fn limited(source: impl Future<Output = Result<Outcome>>) -> Result<Outcome> {
    tokio::time::timeout(TIMEOUT, source).await.unwrap_or_else(|_| Err(Code::NetTimeout.err("timed out")))
}

fn credits_removed(lines: Vec<LyricLine>) -> Vec<LyricLine> {
    lines
        .into_iter()
        .filter(|l| l.time > 30.0 || !(l.text.contains(" : ") || l.text.contains('：') || (l.time <= 1.0 && l.text.contains(" - "))))
        .collect()
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find(['(', '[']) {
        let close = if rest[start..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[start..].find(close) else { break };
        out.push_str(&rest[..start]);
        let inner = rest[start..start + len].to_lowercase();
        if !NOISE.iter().any(|n| inner.contains(n)) {
            out.push_str(&rest[start..=start + len]);
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn clean_title(title: &str) -> String {
    let mut t = strip_brackets(title);
    if let Some((head, tail)) = t.rsplit_once(" - ")
        && TAIL_NOISE.iter().any(|n| tail.to_lowercase().contains(n))
    {
        t = head.into();
    }
    let lower = t.to_ascii_lowercase();
    if let Some(i) = [" feat.", " feat ", " ft.", " ft ", " featuring "].iter().filter_map(|m| lower.find(m)).min() {
        t.truncate(i);
    }
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_artist(track: &Track) -> String {
    let name = track.artists.first().map_or("", |a| a.name.as_str());
    name.trim_end_matches(" - Topic").trim().into()
}

fn parse_lrc(lrc: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for line in lrc.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(tag) = rest.strip_prefix('[') {
            let Some((stamp, after)) = tag.split_once(']') else { break };
            let Some((m, s)) = stamp.split_once(':') else { break };
            let (Ok(m), Ok(s)) = (m.parse::<f64>(), s.parse::<f64>()) else { break };
            times.push(m * 60.0 + s);
            rest = after;
        }
        lines.extend(times.into_iter().map(|time| LyricLine { time, text: rest.trim().into() }));
    }
    lines.sort_by(|a, b| a.time.total_cmp(&b.time));
    lines
}

fn synced_from(lrc: &str, source: &str) -> Option<Lyrics> {
    let lines = credits_removed(parse_lrc(lrc));
    if !lines.is_empty() {
        return Some(Lyrics::timed(lines, source));
    }
    (!lrc.trim().is_empty()).then(|| Lyrics::text(lrc, source))
}

fn lyrics_of(e: &Entry) -> Option<Lyrics> {
    let synced = e.synced_lyrics.as_deref().map(|lrc| credits_removed(parse_lrc(lrc))).filter(|l| !l.is_empty());
    if let Some(lines) = synced {
        return Some(Lyrics::timed(lines, "lrclib"));
    }
    let plain = e.plain_lyrics.as_deref().filter(|p| !p.trim().is_empty())?;
    Some(Lyrics::text(plain, "lrclib"))
}

impl LyricsSource {
    pub fn new(http: impl Into<Http>) -> Self {
        let http = http.into();
        Self { ytm: Ytm::new(http.clone()), http }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str, params: &[(&str, &str)], user_agent: &str) -> Result<Option<T>> {
        let headers = [("User-Agent", user_agent), ("Referer", "https://music.163.com")];
        let resp = net::get(&self.http, url, params, &headers, TIMEOUT).await.context("lyrics request failed")?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(net::json(resp).await.context("lyrics service returned invalid JSON")?))
    }

    async fn lrclib(&self, track: &Track) -> Result<Outcome> {
        let title = clean_title(&track.title);
        let artist = clean_artist(track);
        let album = track.album.as_ref().map_or("", |a| a.name.as_str());
        let duration = (track.duration.round() as u64).to_string();

        let mut params = vec![("track_name", title.as_str()), ("artist_name", artist.as_str())];
        if !album.is_empty() {
            params.push(("album_name", album));
        }
        if track.duration > 0.0 {
            params.push(("duration", duration.as_str()));
        }
        let search = [("track_name", title.as_str()), ("artist_name", artist.as_str())];
        let (exact, searched) = tokio::join!(
            self.get::<Entry>("https://lrclib.net/api/get", &params, USER_AGENT),
            self.get::<Vec<Entry>>("https://lrclib.net/api/search", &search, USER_AGENT),
        );
        lrclib_outcome(track, exact, searched)
    }

    async fn netease(&self, track: &Track) -> Result<Option<Lyrics>> {
        let query = format!("{} {}", clean_title(&track.title), clean_artist(track));
        let found: Option<Value> =
            self.get("http://music.163.com/api/search/get", &[("s", &query), ("type", "1"), ("limit", "5")], BROWSER_UA).await?;
        let Some(id) = found.as_ref().and_then(|f| netease_song(track, f)) else { return Ok(None) };
        let lyric: Option<Value> = self
            .get("http://music.163.com/api/song/lyric", &[("id", &id.to_string()), ("lv", "1"), ("kv", "1"), ("tv", "-1")], BROWSER_UA)
            .await?;
        Ok(lyric.as_ref().and_then(netease_lyrics))
    }

    async fn kugou(&self, track: &Track) -> Result<Option<Lyrics>> {
        let keyword = format!("{} - {}", clean_artist(track), clean_title(&track.title));
        let duration = ((track.duration * 1000.0) as u64).to_string();
        let found: Option<Value> = self
            .get(
                "http://lyrics.kugou.com/search",
                &[("ver", "1"), ("man", "yes"), ("client", "pc"), ("keyword", &keyword), ("duration", &duration), ("hash", "")],
                BROWSER_UA,
            )
            .await?;
        let candidates = found.as_ref().and_then(|f| f["candidates"].as_array()).map_or(&[][..], Vec::as_slice);
        let pick = candidates.iter().find(|c| {
            let seconds = c["duration"].as_f64().unwrap_or(0.0) / 1000.0;
            confirmed(track, c["song"].as_str().unwrap_or(""), &[c["singer"].as_str().unwrap_or("")], seconds)
        });
        let (Some(id), Some(key)) = (pick.and_then(|c| c["id"].as_str()), pick.and_then(|c| c["accesskey"].as_str())) else {
            return Ok(None);
        };
        let download: Option<Value> = self
            .get(
                "http://lyrics.kugou.com/download",
                &[("ver", "1"), ("client", "pc"), ("id", id), ("accesskey", key), ("fmt", "lrc"), ("charset", "utf8")],
                BROWSER_UA,
            )
            .await?;
        let Some(content) = download.as_ref().and_then(|d| d["content"].as_str()) else { return Ok(None) };
        let lrc = String::from_utf8(STANDARD.decode(content).context("kugou lyrics are not base64")?)?;
        Ok(synced_from(&lrc, "kugou"))
    }

    async fn youtube(&self, track: &Track) -> Result<Option<Lyrics>> {
        let plain = self.ytm.lyrics(&track.id).await?;
        Ok(plain.as_deref().map(youtube_lyrics))
    }

    pub async fn fetch(&self, track: &Track) -> Result<Option<Lyrics>> {
        let (lrclib, netease, kugou, youtube) = tokio::join!(
            limited(self.lrclib(track)),
            limited(async { Ok(self.netease(track).await?.into()) }),
            limited(async { Ok(self.kugou(track).await?.into()) }),
            limited(async { Ok(self.youtube(track).await?.into()) }),
        );
        choose([lrclib, netease, kugou, youtube])
    }
}

fn youtube_lyrics(plain: &str) -> Lyrics {
    Lyrics::text(plain, "youtube music")
}

fn netease_song(track: &Track, found: &Value) -> Option<u64> {
    let songs = found["result"]["songs"].as_array().map_or(&[][..], Vec::as_slice);
    let song = songs.iter().find(|s| {
        let artists: Vec<&str> = s["artists"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).collect();
        let seconds = s["duration"].as_f64().unwrap_or(0.0) / 1000.0;
        confirmed(track, s["name"].as_str().unwrap_or(""), &artists, seconds)
    });
    song.and_then(|s| s["id"].as_u64())
}

fn netease_lyrics(lyric: &Value) -> Option<Lyrics> {
    synced_from(lyric["lrc"]["lyric"].as_str()?, "netease")
}

fn lrclib_outcome(track: &Track, exact: Result<Option<Entry>>, searched: Result<Option<Vec<Entry>>>) -> Result<Outcome> {
    let mut found = Vec::new();
    let mut instrumental = false;
    if let Ok(Some(e)) = &exact
        && let Some(outcome) = verdict(track, e)
    {
        match outcome {
            Outcome::Found(lyrics) => found.extend(lyrics),
            Outcome::Instrumental(_) => instrumental = true,
            Outcome::Missing => {}
        }
    }
    let searched = match searched {
        Ok(list) => list.unwrap_or_default(),
        Err(e) if exact.is_err() => return Err(e),
        Err(_) => Vec::new(),
    };
    let mut candidates: Vec<&Entry> =
        searched.iter().filter(|e| !e.instrumental && confirmed(track, &e.track_name, &[&e.artist_name], e.duration)).collect();
    candidates.sort_by_key(|e| e.synced_lyrics.is_none());
    found.extend(candidates.into_iter().filter_map(lyrics_of));
    Ok(match (instrumental, found.is_empty()) {
        (true, _) => Outcome::Instrumental(found),
        (false, true) => Outcome::Missing,
        (false, false) => Outcome::Found(found),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Script {
    Latin,
    Greek,
    Cyrillic,
    Hebrew,
    Arabic,
    Devanagari,
    Thai,
    Hangul,
    Han,
    Other,
}

fn script(c: char) -> Script {
    match c {
        'a'..='z' | 'A'..='Z' | '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}' => Script::Latin,
        '\u{0370}'..='\u{03FF}' => Script::Greek,
        '\u{0400}'..='\u{052F}' => Script::Cyrillic,
        '\u{0590}'..='\u{05FF}' => Script::Hebrew,
        '\u{0600}'..='\u{06FF}' | '\u{0750}'..='\u{077F}' => Script::Arabic,
        '\u{0900}'..='\u{097F}' => Script::Devanagari,
        '\u{0E00}'..='\u{0E7F}' => Script::Thai,
        '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{AC00}'..='\u{D7AF}' => Script::Hangul,
        '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' => {
            Script::Han
        }
        _ => Script::Other,
    }
}

fn unspaced(c: char) -> bool {
    matches!(script(c), Script::Han | Script::Thai)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Judged {
    Same,
    OtherScript,
    TooShort,
    Differ,
}

struct Text<'a> {
    lyrics: &'a Lyrics,
    pairs: HashSet<String>,
    scripts: BTreeSet<Script>,
}

impl<'a> Text<'a> {
    fn of(lyrics: &'a Lyrics) -> Text<'a> {
        let lines: Vec<&str> =
            if lyrics.synced { lyrics.lines.iter().map(|l| l.text.as_str()).collect() } else { lyrics.plain.lines().collect() };
        let mut pairs = HashSet::new();
        let mut letters: BTreeMap<Script, usize> = BTreeMap::new();
        for line in lines {
            let mut units: Vec<String> = Vec::new();
            for word in norm(line).split_whitespace() {
                if word.chars().any(unspaced) {
                    units.extend(word.chars().map(String::from));
                } else {
                    units.push(word.into());
                }
            }
            pairs.extend(units.windows(2).map(|w| w.join(" ")));
            for c in line.chars().filter(|c| c.is_alphabetic()) {
                *letters.entry(script(c)).or_default() += 1;
            }
        }
        let total: usize = letters.values().sum();
        let scripts = letters.into_iter().filter(|(_, n)| *n as f64 >= MAIN_SCRIPT * total as f64).map(|(s, _)| s).collect();
        Text { lyrics, pairs, scripts }
    }

    fn solid(&self) -> bool {
        self.pairs.len() >= COMPARABLE_PAIRS
    }

    fn judge(&self, other: &Text) -> Judged {
        if !self.solid() || !other.solid() {
            return Judged::TooShort;
        }
        if self.scripts != other.scripts {
            return Judged::OtherScript;
        }
        let fewer = self.pairs.len().min(other.pairs.len());
        if self.pairs.intersection(&other.pairs).count() as f64 >= SHARED_PAIRS * fewer as f64 { Judged::Same } else { Judged::Differ }
    }
}

fn backers(texts: &[Text], pool: &[usize], i: usize) -> usize {
    let sources: HashSet<&str> =
        pool.iter().filter(|&&j| j == i || texts[i].judge(&texts[j]) == Judged::Same).map(|&j| texts[j].lyrics.source.as_str()).collect();
    sources.len()
}

fn consensus(texts: &[Text], pool: &[usize]) -> Option<usize> {
    let backing: Vec<usize> = pool.iter().map(|&i| backers(texts, pool, i)).collect();
    let most = *backing.iter().max()?;
    let leaders: Vec<usize> = pool.iter().zip(&backing).filter(|(_, b)| **b == most).map(|(&i, _)| i).collect();
    let contested = leaders.iter().any(|&a| leaders.iter().any(|&b| texts[a].judge(&texts[b]) == Judged::Differ));
    (!contested).then(|| leaders[0])
}

fn ranked(texts: &[Text], video: Option<usize>) -> Vec<usize> {
    let all = 0..texts.len();
    if let Some(video) = video {
        let judged = |i: usize| if i == video { Judged::Same } else { texts[i].judge(&texts[video]) };
        let same: Vec<usize> = all.clone().filter(|&i| judged(i) == Judged::Same).collect();
        let foreign: Vec<usize> = all.filter(|&i| judged(i) == Judged::OtherScript && texts[i].lyrics.synced).collect();
        let synced = same.iter().copied().filter(|&i| texts[i].lyrics.synced);
        let plain = same.iter().copied().filter(|&i| !texts[i].lyrics.synced);
        return synced.chain(consensus(texts, &foreign)).chain(plain).collect();
    }
    let solid: Vec<usize> = all.clone().filter(|&i| texts[i].solid()).collect();
    let pool = if solid.is_empty() { all.collect() } else { solid };
    let Some(reference) = consensus(texts, &pool) else { return Vec::new() };
    let mut fits: Vec<(Judged, usize)> = pool
        .iter()
        .map(|&i| (if i == reference { Judged::Same } else { texts[i].judge(&texts[reference]) }, i))
        .filter(|(judged, _)| *judged != Judged::Differ)
        .collect();
    fits.sort_by_key(|&(judged, i)| (judged, !texts[i].lyrics.synced));
    fits.into_iter().map(|(_, i)| i).collect()
}

fn choose(results: [Result<Outcome>; 4]) -> Result<Option<Lyrics>> {
    let texts: Vec<Text> = results
        .iter()
        .filter_map(|r| if let Ok(Outcome::Found(l) | Outcome::Instrumental(l)) = r { Some(l) } else { None })
        .flatten()
        .map(Text::of)
        .collect();
    let own = match &results[3] {
        Ok(Outcome::Found(l)) => l.first(),
        _ => None,
    };
    let video = texts.iter().position(|t| own.is_some_and(|v| std::ptr::eq(t.lyrics, v)) && t.solid());
    if video.is_none() && results.iter().any(|r| matches!(r, Ok(Outcome::Instrumental(_)))) {
        return Ok(own.cloned());
    }
    if let Some(&best) = ranked(&texts, video).first() {
        return Ok(Some(texts[best].lyrics.clone()));
    }
    if results.iter().all(Result::is_err) {
        return results.into_iter().next().map_or(Ok(None), |r| r.map(|_| None));
    }
    Ok(None)
}
