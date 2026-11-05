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

use crate::net::{Http, net};
use crate::ytm::Ytm;

const NOISE: [&str; 14] =
    ["official", "video", "audio", "lyric", "visualizer", "remaster", "live", "feat", "ft.", "version", "mono", "stereo", "hd", "4k"];
const TIMEOUT: Duration = Duration::from_secs(6);
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";
const TAIL_NOISE: [&str; 7] = ["remaster", "live", "version", "mono", "stereo", "edit", "single"];

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
        return Some(Outcome::Instrumental);
    }
    lyrics_of(e).map(Outcome::Found)
}

enum Outcome {
    Found(Lyrics),
    Instrumental,
    Missing,
}

impl From<Option<Lyrics>> for Outcome {
    fn from(found: Option<Lyrics>) -> Outcome {
        found.map_or(Outcome::Missing, Outcome::Found)
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
    let lower = t.to_lowercase();
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
        return Some(Lyrics { synced: true, lines, plain: String::new(), source: source.into() });
    }
    let plain = lrc.trim();
    (!plain.is_empty()).then(|| Lyrics { synced: false, lines: Vec::new(), plain: plain.into(), source: source.into() })
}

fn lyrics_of(e: &Entry) -> Option<Lyrics> {
    if let Some(lines) = e.synced_lyrics.as_deref().map(parse_lrc).filter(|l| !l.is_empty()) {
        return Some(Lyrics { synced: true, lines, plain: String::new(), source: "lrclib".into() });
    }
    let plain = e.plain_lyrics.as_deref().filter(|p| !p.trim().is_empty())?;
    Some(Lyrics { synced: false, lines: Vec::new(), plain: plain.trim().into(), source: "lrclib".into() })
}

impl LyricsSource {
    pub fn new(http: impl Into<Http>) -> Self {
        let http = http.into();
        Self { ytm: Ytm::new(http.clone()), http }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, url: &str, params: &[(&str, &str)], user_agent: &str) -> Result<Option<T>> {
        let url = reqwest::Url::parse_with_params(url, params)?;
        let resp = self
            .http
            .client()
            .get(url)
            .header("User-Agent", user_agent)
            .header("Referer", "https://music.163.com")
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(net)
            .context("lyrics request failed")?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(resp.error_for_status().map_err(net)?.json().await.map_err(net).context("lyrics service returned invalid JSON")?))
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
        let exact = self.get::<Entry>("https://lrclib.net/api/get", &params, USER_AGENT).await;
        if let Ok(Some(e)) = &exact
            && let Some(outcome) = verdict(track, e)
        {
            return Ok(outcome);
        }

        let found = self
            .get::<Vec<Entry>>("https://lrclib.net/api/search", &[("track_name", &title), ("artist_name", &artist)], USER_AGENT)
            .await
            .or_else(|e| exact.map(|_| None).map_err(|_| e))?
            .unwrap_or_default();
        let candidates: Vec<_> =
            found.iter().filter(|e| !e.instrumental && confirmed(track, &e.track_name, &[&e.artist_name], e.duration)).collect();
        let best = candidates.iter().find(|e| e.synced_lyrics.is_some()).or(candidates.first());
        Ok(best.and_then(|e| lyrics_of(e)).into())
    }

    async fn netease(&self, track: &Track) -> Result<Option<Lyrics>> {
        let query = format!("{} {}", clean_title(&track.title), clean_artist(track));
        let found: Option<Value> =
            self.get("http://music.163.com/api/search/get", &[("s", &query), ("type", "1"), ("limit", "5")], BROWSER_UA).await?;
        let songs = found.as_ref().and_then(|f| f["result"]["songs"].as_array()).map_or(&[][..], Vec::as_slice);
        let song = songs.iter().find(|s| {
            let artists: Vec<&str> = s["artists"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).collect();
            let seconds = s["duration"].as_f64().unwrap_or(0.0) / 1000.0;
            confirmed(track, s["name"].as_str().unwrap_or(""), &artists, seconds)
        });
        let Some(id) = song.and_then(|s| s["id"].as_u64()) else { return Ok(None) };
        let lyric: Option<Value> = self
            .get("http://music.163.com/api/song/lyric", &[("id", &id.to_string()), ("lv", "1"), ("kv", "1"), ("tv", "-1")], BROWSER_UA)
            .await?;
        let Some(lrc) = lyric.as_ref().and_then(|l| l["lrc"]["lyric"].as_str()) else { return Ok(None) };
        Ok(synced_from(lrc, "netease"))
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
        Ok(plain.map(|p| Lyrics { synced: false, lines: Vec::new(), plain: p.trim().into(), source: "youtube music".into() }))
    }

    pub async fn fetch(&self, track: &Track) -> Result<Option<Lyrics>> {
        let (lrclib, netease, kugou, youtube) = tokio::join!(
            limited(self.lrclib(track)),
            limited(async { Ok(self.netease(track).await?.into()) }),
            limited(async { Ok(self.kugou(track).await?.into()) }),
            limited(async { Ok(self.youtube(track).await?.into()) }),
        );
        let results = [lrclib, netease, kugou, youtube];
        if results.iter().any(|r| matches!(r, Ok(Outcome::Instrumental))) {
            return Ok(None);
        }
        let found: Vec<&Lyrics> = results.iter().filter_map(|r| if let Ok(Outcome::Found(l)) = r { Some(l) } else { None }).collect();
        if let Some(best) = found.iter().find(|l| l.synced).or(found.first()) {
            return Ok(Some((*best).clone()));
        }
        if results.iter().all(Result::is_err) {
            return results.into_iter().next().map_or(Ok(None), |r| r.map(|_| None));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use snellar_core::model::Ref;

    use super::*;

    fn track(title: &str, artist: &str, album: &str, duration: f64) -> Track {
        Track {
            title: title.into(),
            artists: vec![Ref { id: String::new(), name: artist.into() }],
            album: Some(Ref { id: String::new(), name: album.into() }),
            duration,
            ..Default::default()
        }
    }

    #[test]
    fn titles() {
        assert_eq!(clean_title("Hello (Official Video) [HD]"), "Hello");
        assert_eq!(clean_title("Song feat. Someone"), "Song");
        assert_eq!(clean_title("Song - Remastered 2011"), "Song");
        assert_eq!(clean_title("Song (Part 2)"), "Song (Part 2)");
    }

    #[test]
    fn strict() {
        let t = track("Gangnam Style (강남스타일)", "PSY", "", 219.0);
        assert!(strict_match(&t, "강남스타일", &["PSY"]));
        assert!(strict_match(&t, "Gangnam Style", &["Psy"]));
        assert!(!strict_match(&t, "Gangnam Style", &["Other"]));
        assert!(!strict_match(&t, "Gangnam Style (Remix)", &["Other"]));
        let remix = track("Song (Remix)", "A", "", 100.0);
        assert!(!strict_match(&remix, "Other (Remix)", &["A"]));
        assert!(strict_match(&track("Hello", "A", "", 0.0), "Hello", &["A、C"]));
    }

    #[test]
    fn generic_titles_are_detected() {
        assert!(is_generic("Intro") && is_generic("Outro (Live)") && is_generic("Track 07") && is_generic("Interlude"));
        assert!(!is_generic("Introduction") && !is_generic("Track Star"));
    }

    #[test]
    fn intro_needs_artist_and_duration() {
        let xx = track("Intro", "The xx", "xx", 128.0);
        assert!(confirmed(&xx, "Intro", &["The xx"], 127.0));
        assert!(!confirmed(&xx, "Intro", &["M83"], 128.0));
        assert!(!confirmed(&xx, "Intro", &["The xx"], 131.0));
        assert!(!confirmed(&xx, "Intro", &["The xx"], 0.0));
        assert!(!confirmed(&track("Intro", "The xx", "xx", 0.0), "Intro", &["The xx"], 128.0));
        assert!(!confirmed(&xx, "Intro", &[""], 128.0));
    }

    #[test]
    fn ordinary_titles_tolerate_missing_duration() {
        let islands = track("Islands", "The xx", "xx", 161.0);
        assert!(confirmed(&islands, "Islands", &["The xx"], 164.0));
        assert!(confirmed(&islands, "Islands", &["The xx"], 0.0));
        assert!(!confirmed(&islands, "Islands", &["The xx"], 170.0));
        assert!(!confirmed(&islands, "Islands", &["Other"], 161.0));
    }

    fn entry(title: &str, artist: &str, duration: f64) -> Entry {
        Entry {
            track_name: title.into(),
            artist_name: artist.into(),
            duration,
            plain_lyrics: Some("someone else's words".into()),
            ..Default::default()
        }
    }

    #[test]
    fn intro_lyrics_of_another_artist_are_not_shown() {
        let xx = track("Intro", "The xx", "xx", 128.0);
        assert!(verdict(&xx, &entry("Intro", "Some Other Band", 128.0)).is_none());
        assert!(verdict(&xx, &entry("Intro", "The xx", 214.0)).is_none());
        assert!(verdict(&xx, &entry("Intro", "The xx", 0.0)).is_none());
        let right = verdict(&xx, &entry("Intro", "The xx", 128.0));
        assert!(matches!(right, Some(Outcome::Found(l)) if l.plain == "someone else's words"));
    }

    #[test]
    fn instrumental_flag_counts_only_for_the_confirmed_track() {
        let xx = track("Intro", "The xx", "xx", 128.0);
        let instrumental = |artist: &str| Entry { instrumental: true, ..entry("Intro", artist, 128.0) };
        assert!(matches!(verdict(&xx, &instrumental("The xx")), Some(Outcome::Instrumental)));
        assert!(verdict(&xx, &instrumental("Another")).is_none());
    }

    #[test]
    fn kugou_header_line_is_dropped() {
        let lines = credits_removed(vec![
            LyricLine { time: 0.0, text: "Islands - The xx".into() },
            LyricLine { time: 12.0, text: "I don't have to leave".into() },
        ]);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn lrc() {
        let l = parse_lrc("[ar:x]\n[00:01.50]a\n[01:02.00][00:03.00]b\n");
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].time, 1.5);
        assert_eq!(l[1].text, "b");
        assert_eq!(l[2].time, 62.0);
    }

    #[tokio::test]
    #[ignore]
    async fn live_chain() {
        let http = reqwest::Client::new();
        let ytm = Ytm::new(http.clone());
        let src = LyricsSource::new(http);
        let queries = [
            "daft punk get lucky",
            "queen bohemian rhapsody",
            "kalush stefania",
            "океан ельзи без бою",
            "psy gangnam style",
            "yonezu kenshi lemon",
            "yoasobi idol",
            "zzqx nonexistent qqq",
        ];
        for q in queries {
            let Some(t) = ytm.search(q, crate::ytm::Filter::Songs).await.unwrap().tracks.into_iter().next() else { continue };
            let start = std::time::Instant::now();
            let r = src.fetch(&t).await;
            let took = start.elapsed();
            match r {
                Ok(Some(l)) => println!(
                    "{q:<28} {} - {} | {} synced={} lines={} {took:?}",
                    t.title,
                    t.artist_line(),
                    l.source,
                    l.synced,
                    l.lines.len()
                ),
                Ok(None) => println!("{q:<28} {} - {} | none {took:?}", t.title, t.artist_line()),
                Err(e) => println!("{q:<28} ERR {e:#} {took:?}"),
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_each_source() {
        let http = reqwest::Client::new();
        let ytm = Ytm::new(http.clone());
        let src = LyricsSource::new(http);
        for q in ["daft punk get lucky", "kalush stefania", "psy gangnam style", "yoasobi idol"] {
            let t = ytm.search(q, crate::ytm::Filter::Songs).await.unwrap().tracks.remove(0);
            let show = |r: Result<Option<Lyrics>>| match r {
                Ok(Some(l)) => format!("synced={} lines={} plain={}", l.synced, l.lines.len(), l.plain.len()),
                Ok(None) => "none".into(),
                Err(e) => format!("err {e:#}"),
            };
            println!(
                "{q}: lrclib[{}] netease[{}] kugou[{}] youtube[{}]",
                show(src.lrclib(&t).await.map(|o| match o {
                    Outcome::Found(l) => Some(l),
                    _ => None,
                })),
                show(src.netease(&t).await),
                show(src.kugou(&t).await),
                show(src.youtube(&t).await)
            );
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_intro_shows_no_foreign_lyrics() {
        let http = reqwest::Client::new();
        let ytm = Ytm::new(http.clone());
        let src = LyricsSource::new(http);
        let found = ytm.search("the xx intro", crate::ytm::Filter::Songs).await.unwrap().tracks;
        let intro = found.into_iter().find(|t| t.title == "Intro" && t.artist_line().contains("The xx")).expect("The xx - Intro in search");
        let shown = src.fetch(&intro).await.unwrap();
        println!("shown: {:?}", shown.as_ref().map(|l| (&l.source, l.lines.first().map(|x| &x.text), l.plain.lines().next())));
        assert!(shown.is_none(), "The xx - Intro is instrumental, anything shown is another song's text");
    }

    #[tokio::test]
    #[ignore]
    async fn live_short_names() {
        let http = reqwest::Client::new();
        let ytm = Ytm::new(http.clone());
        let src = LyricsSource::new(http);
        for q in [
            "the xx intro",
            "the xx islands",
            "mo pilgrim",
            "m83 midnight city",
            "air playground love",
            "low in the drugstore",
            "yes owner of a lonely heart",
            "m83 intro",
            "outro m83",
        ] {
            let Some(t) = ytm.search(q, crate::ytm::Filter::Songs).await.ok().and_then(|r| r.tracks.into_iter().next()) else { continue };
            let show = |r: Result<Option<Lyrics>>| match r {
                Ok(Some(l)) => format!(
                    "{} lines={} first={:?}",
                    if l.synced { "synced" } else { "plain" },
                    l.lines.len(),
                    l.lines.first().map(|x| x.text.as_str()).or(l.plain.lines().next())
                ),
                Ok(None) => "none".into(),
                Err(e) => format!("err {e:#}"),
            };
            println!(
                "{} - {} ({}s, album {:?})\n   lrclib {}\n   netease {}\n   kugou {}\n   youtube {}",
                t.title,
                t.artist_line(),
                t.duration,
                t.album.as_ref().map(|a| &a.name),
                show(src.lrclib(&t).await.map(|o| match o {
                    Outcome::Found(l) => Some(l),
                    _ => None,
                })),
                show(src.netease(&t).await),
                show(src.kugou(&t).await),
                show(src.youtube(&t).await)
            );
            println!("   FETCH {}", show(src.fetch(&t).await));
        }
    }
}
