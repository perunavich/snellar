use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use snellar_core::cache::write_atomic;
use snellar_core::error::Code;
use snellar_core::model::AudioFormat;
use tokio::process::Command;
use tokio::task::JoinSet;

use crate::net::{Http, Route, checked, net, ytdlp_cause};

const SAFETY_MARGIN: u64 = 300;
const FALLBACK_TTL: u64 = 5 * 3600;
const TIMEOUT: Duration = Duration::from_secs(20);
const PLAYER_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_LIMIT: Duration = Duration::from_secs(4);
const HEDGE: Duration = Duration::from_millis(700);
const VISITOR_TTL: u64 = 1800;
const ATTEMPTS: usize = 3;
const WATCH_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36";
const SIGNATURE_TIMESTAMP: u32 = 20753;
const PROBE_BYTES: usize = 16384;
const FIRST_RANGE: u64 = 128 * 1024;
const MID_RANGE: u64 = 512 * 1024;
const AUTO_CEILING: u64 = 160_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    #[default]
    Auto,
    High,
    DataSaver,
}

impl Quality {
    pub fn parse(s: &str) -> Quality {
        match s {
            "high" => Quality::High,
            "dataSaver" => Quality::DataSaver,
            _ => Quality::Auto,
        }
    }

    fn ytdlp(self) -> &'static str {
        match self {
            Quality::DataSaver => "worstaudio/worst",
            _ => "bestaudio/best",
        }
    }
}

struct Client {
    name: &'static str,
    version: &'static str,
    id: &'static str,
    user_agent: &'static str,
    fields: &'static [(&'static str, &'static str)],
}

const VISION_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

const VISIONOS: Client = Client {
    name: "VISIONOS",
    version: "1.02",
    id: "101",
    user_agent: VISION_UA,
    fields: &[
        ("deviceMake", "Apple"),
        ("deviceModel", "RealityDevice17,1"),
        ("osName", "visionOS"),
        ("osVersion", "26.5.23O471"),
        ("userAgent", VISION_UA),
    ],
};

#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    url: String,
    expires: u64,
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    format: Option<AudioFormat>,
}

type Resolved = (String, Option<String>);

pub struct Resolver {
    http: Http,
    cache_dir: PathBuf,
    cache: Mutex<HashMap<String, Cached>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    visitor: tokio::sync::Mutex<Option<(u64, String)>>,
    saving: tokio::sync::Mutex<()>,
    ytdlp_enabled: AtomicBool,
    ytdlp_runs: AtomicU64,
    quality: AtomicU8,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn expiry(url: &str) -> u64 {
    let expire = url
        .split(['?', '&', '/'])
        .find_map(|p| p.strip_prefix("expire=").or_else(|| p.strip_prefix("expire/")))
        .and_then(|v| v.parse::<u64>().ok());
    expire.map_or(now() + FALLBACK_TTL, |e| e.saturating_sub(SAFETY_MARGIN))
}

impl Resolver {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self::with_route(cache_dir, &Route::default())
    }

    pub fn with_route(cache_dir: PathBuf, route: &Route) -> Self {
        let cache = std::fs::read(cache_dir.join("streams.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<HashMap<String, Cached>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, c)| c.expires > now())
            .collect();
        Self {
            http: Http::new(route, |b| b.connect_timeout(CONNECT_LIMIT)),
            cache_dir,
            cache: Mutex::new(cache),
            locks: Mutex::default(),
            visitor: tokio::sync::Mutex::default(),
            saving: tokio::sync::Mutex::default(),
            ytdlp_enabled: AtomicBool::new(true),
            ytdlp_runs: AtomicU64::new(0),
            quality: AtomicU8::new(Quality::Auto as u8),
        }
    }

    fn cached(&self, id: &str) -> Option<Resolved> {
        let cache = self.cache.lock().unwrap();
        cache.get(id).filter(|c| c.expires > now()).map(|c| (c.url.clone(), c.user_agent.clone()))
    }

    pub fn invalidate(&self, video_id: &str) {
        self.cache.lock().unwrap().remove(video_id);
    }

    pub fn format(&self, video_id: &str) -> Option<AudioFormat> {
        self.cache.lock().unwrap().get(video_id).and_then(|c| c.format.clone())
    }

    pub fn quality(&self) -> Quality {
        match self.quality.load(Ordering::Relaxed) {
            1 => Quality::High,
            2 => Quality::DataSaver,
            _ => Quality::Auto,
        }
    }

    pub fn set_quality(&self, quality: Quality) {
        if self.quality.swap(quality as u8, Ordering::Relaxed) != quality as u8 {
            self.cache.lock().unwrap().clear();
        }
    }

    pub async fn resolve(&self, video_id: &str) -> Result<String> {
        Ok(self.resolve_with_ua(video_id).await?.0)
    }

    pub async fn resolve_with_ua(&self, video_id: &str) -> Result<Resolved> {
        if let Some(hit) = self.cached(video_id) {
            return Ok(hit);
        }
        let lock = self.locks.lock().unwrap().entry(video_id.into()).or_default().clone();
        let _guard = lock.lock().await;
        if let Some(hit) = self.cached(video_id) {
            return Ok(hit);
        }
        let ((url, user_agent), format) = match self.direct_resolve(video_id).await {
            Ok(hit) => hit,
            Err(direct) if Code::of(&direct).nothing_gets_through() => return Err(direct),
            Err(direct) => match self.fallback(video_id).await {
                Ok(url) => ((url, None), None),
                Err(other) => return Err(truer(direct, other)),
            },
        };
        {
            let entry = Cached { url: url.clone(), expires: expiry(&url), user_agent: user_agent.clone(), format };
            self.cache.lock().unwrap().insert(video_id.into(), entry);
        }
        self.save().await?;
        Ok((url, user_agent))
    }

    async fn save(&self) -> Result<()> {
        let _saving = self.saving.lock().await;
        let snapshot = serde_json::to_vec(&*self.cache.lock().unwrap())?;
        let _ = write_atomic(&self.cache_dir.join("streams.json"), snapshot).await;
        Ok(())
    }

    pub fn set_ytdlp(&self, enabled: bool) {
        self.ytdlp_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn route(&self) -> &Route {
        self.http.route()
    }

    async fn visitor(&self, video_id: &str, fresh: bool) -> Result<String> {
        let mut slot = self.visitor.lock().await;
        if let Some((at, visitor)) = slot.as_ref() {
            let age = now().saturating_sub(*at);
            if (!fresh && age < VISITOR_TTL) || age < 2 {
                return Ok(visitor.clone());
            }
        }
        let page = self
            .http
            .client()
            .get(format!("https://www.youtube.com/watch?v={video_id}&bpctr=9999999999&has_verified=1"))
            .header("User-Agent", WATCH_UA)
            .header("Cookie", "PREF=hl=en&tz=UTC; SOCS=CAI")
            .timeout(Duration::from_secs(6))
            .send()
            .await
            .map_err(net)?;
        let page = checked(page)?.text().await.map_err(net)?;
        let visitor = page
            .split_once("\"visitorData\":\"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(v, _)| v.to_string())
            .ok_or_else(|| Code::YtParse.err("watch page has no visitor data"))?;
        *slot = Some((now(), visitor.clone()));
        Ok(visitor)
    }

    async fn direct_resolve(&self, video_id: &str) -> Result<(Resolved, Option<AudioFormat>)> {
        let quality = self.quality();
        let mut failure = None;
        for attempt in 0..ATTEMPTS {
            let visitor = self.visitor(video_id, attempt > 0).await?;
            let mut set = JoinSet::new();
            for delay in [Duration::ZERO, HEDGE] {
                let (http, id, visitor) = (self.http.client(), video_id.to_string(), visitor.clone());
                set.spawn(async move {
                    tokio::time::sleep(delay).await;
                    direct(&http, &VISIONOS, &id, &visitor, quality).await
                });
            }
            while let Some(done) = set.join_next().await {
                match done {
                    Ok(Ok((hit, format))) => return Ok((hit, Some(format))),
                    Ok(Err(e)) if answer_is_final(Code::of(&e)) => return Err(e),
                    Ok(Err(e)) => failure = Some(e),
                    Err(_) => {}
                }
            }
        }
        Err(failure.unwrap_or_else(|| Code::Internal.err(format!("no direct stream for {video_id}"))))
    }

    async fn fallback(&self, video_id: &str) -> Result<String> {
        if !self.ytdlp_enabled.load(Ordering::Relaxed) {
            return Err(Code::YtUnavailable.err(format!("no direct stream for {video_id} and the yt-dlp fallback is disabled")));
        }
        let runs = self.ytdlp_runs.fetch_add(1, Ordering::Relaxed) + 1;
        snellar_core::journal::info("resolver", format!("resolving {video_id} with yt-dlp (fallback use {runs})"));
        self.run(video_id).await
    }

    async fn run(&self, video_id: &str) -> Result<String> {
        let bin = snellar_core::product::ytdlp_bin();
        let child = Command::new(&bin)
            .env_remove("LD_LIBRARY_PATH")
            .args(["-f", self.quality().ytdlp(), "-g", "--no-playlist", "--no-warnings"])
            .args(self.http.route().proxy().map(|p| ["--proxy".to_string(), p]).into_iter().flatten())
            .arg(format!("https://music.youtube.com/watch?v={video_id}"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("cannot start {}", bin.display()))
            .context(Code::YtdlpMissing)?;
        let out = tokio::time::timeout(TIMEOUT, child.wait_with_output())
            .await
            .context("yt-dlp timed out after 20s")
            .context(Code::NetTimeout)?
            .context(Code::YtdlpFailed)?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        match stdout.lines().next().filter(|l| l.starts_with("http")) {
            Some(url) if out.status.success() => Ok(url.into()),
            _ => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let tail: Vec<_> = stderr.lines().rev().take(3).collect();
                Err(ytdlp_cause(&stderr)
                    .unwrap_or(Code::YtdlpFailed)
                    .err(format!("yt-dlp failed for {video_id}: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" "))))
            }
        }
    }

    pub fn prefetch(self: &Arc<Self>, video_id: &str) {
        let this = self.clone();
        let id = video_id.to_string();
        tokio::spawn(async move {
            let _ = this.resolve(&id).await;
        });
    }
}

fn answer_is_final(code: Code) -> bool {
    code.network() && code != Code::NetTimeout
}

fn truer(direct: anyhow::Error, ytdlp: anyhow::Error) -> anyhow::Error {
    let (d, y) = (Code::of(&direct), Code::of(&ytdlp));
    if d == Code::YtUnavailable || d.network() {
        direct
    } else if y == Code::YtUnavailable || y.network() || d == Code::Internal {
        ytdlp
    } else {
        direct
    }
}

async fn direct(
    http: &reqwest::Client,
    client: &Client,
    video_id: &str,
    visitor: &str,
    quality: Quality,
) -> Result<(Resolved, AudioFormat)> {
    let mut context =
        json!({"clientName": client.name, "clientVersion": client.version, "hl": "en", "timeZone": "UTC", "utcOffsetMinutes": 0});
    for (k, v) in client.fields {
        context[*k] = json!(v);
    }
    let body = json!({
        "videoId": video_id,
        "context": {"client": context},
        "playbackContext": {"contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS", "signatureTimestamp": SIGNATURE_TIMESTAMP}},
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    let sent = http
        .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
        .header("User-Agent", client.user_agent)
        .header("Origin", "https://www.youtube.com")
        .header("X-Youtube-Client-Name", client.id)
        .header("X-Youtube-Client-Version", client.version)
        .header("X-Goog-Visitor-Id", visitor)
        .timeout(PLAYER_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(net)?;
    let resp: Value = checked(sent)?.json().await.map_err(net)?;
    let pick = pick_audio(&resp, client.name, quality)?;
    probe(http, pick.url, client.user_agent, pick.length).await?;
    Ok(((pick.url.into(), Some(client.user_agent.into())), pick.format))
}

struct Pick<'a> {
    url: &'a str,
    length: u64,
    format: AudioFormat,
}

fn codec(mime: &str) -> String {
    let named = mime.split_once("codecs=\"").and_then(|(_, rest)| rest.split(['"', '.']).next()).unwrap_or_default();
    match named {
        "mp4a" => "aac".into(),
        "" => mime.split(['/', ';']).nth(1).unwrap_or_default().into(),
        other => other.into(),
    }
}

fn loudness(resp: &Value, format: &Value) -> Option<f64> {
    let config = &resp["playerConfig"]["audioConfig"];
    let absolute = || {
        let target = config["loudnessTargetLkfs"].as_f64().unwrap_or(-14.0);
        format["trackAbsoluteLoudnessLkfs"].as_f64().or_else(|| config["trackAbsoluteLoudnessLkfs"].as_f64()).map(|lkfs| lkfs - target)
    };
    format["loudnessDb"].as_f64().or_else(|| config["loudnessDb"].as_f64()).or_else(absolute).filter(|db| db.is_finite())
}

fn pick_audio<'a>(resp: &'a Value, client: &str, quality: Quality) -> Result<Pick<'a>> {
    let status = resp["playabilityStatus"]["status"].as_str().unwrap_or("UNKNOWN");
    if status != "OK" {
        return Err(Code::YtUnavailable.err(format!("{client} playability: {status}")));
    }
    let formats = ["adaptiveFormats", "formats"].iter().flat_map(|k| resp["streamingData"][k].as_array().into_iter().flatten());
    let audio: Vec<Offer> = formats
        .filter_map(|f| {
            let mime = f["mimeType"].as_str().filter(|m| m.starts_with("audio/"))?;
            let url = f["url"].as_str()?;
            let length = f["contentLength"].as_str()?.parse::<u64>().ok()?;
            Some(Offer { raw: f, url, bitrate: f["bitrate"].as_u64().unwrap_or(0), length, codec: codec(mime) })
        })
        .collect();
    let capped = |o: &&Offer| o.bitrate <= AUTO_CEILING;
    let chosen = match quality {
        Quality::High => audio.iter().max_by_key(|o| o.bitrate),
        Quality::DataSaver => audio.iter().min_by_key(|o| o.bitrate),
        Quality::Auto => audio
            .iter()
            .filter(capped)
            .filter(|o| o.codec == "opus")
            .max_by_key(|o| o.bitrate)
            .or_else(|| audio.iter().filter(capped).max_by_key(|o| o.bitrate))
            .or_else(|| audio.iter().min_by_key(|o| o.bitrate)),
    };
    let offer = chosen.with_context(|| format!("{client} returned no direct audio url"))?;
    let sample_rate = offer.raw["audioSampleRate"].as_str().and_then(|r| r.parse().ok()).unwrap_or(0);
    let format = AudioFormat {
        codec: offer.codec.clone(),
        bitrate: (offer.bitrate / 1000) as u32,
        sample_rate,
        loudness_db: loudness(resp, offer.raw),
    };
    Ok(Pick { url: offer.url, length: offer.length, format })
}

struct Offer<'a> {
    raw: &'a Value,
    url: &'a str,
    bitrate: u64,
    length: u64,
    codec: String,
}

async fn probe_range(http: &reqwest::Client, url: &str, user_agent: &str, from: u64, to: u64) -> Result<()> {
    let resp = http
        .get(url)
        .header("User-Agent", user_agent)
        .header("Range", format!("bytes={from}-{to}"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(net)?;
    let mut resp = checked(resp)?;
    if resp.status().as_u16() != 206 {
        bail!("range {from}-{to} answered {}", resp.status());
    }
    let mut got = 0;
    while got < PROBE_BYTES {
        match resp.chunk().await.map_err(net)? {
            Some(chunk) => got += chunk.len(),
            None => break,
        }
    }
    if got == 0 {
        bail!("range {from}-{to} returned no data");
    }
    Ok(())
}

async fn probe(http: &reqwest::Client, url: &str, user_agent: &str, length: u64) -> Result<()> {
    let middle = length / 2;
    tokio::try_join!(
        probe_range(http, url, user_agent, 0, FIRST_RANGE - 1),
        probe_range(http, url, user_agent, middle, (middle + MID_RANGE - 1).min(length.saturating_sub(1))),
    )
    .map(drop)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn saves_at_the_same_time_leave_one_whole_streams_file() {
        let dir = std::env::temp_dir().join(format!("snellar-streams-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let resolver = Arc::new(Resolver::new(dir.clone()));
        {
            let mut cache = resolver.cache.lock().unwrap();
            for i in 0..2000 {
                cache.insert(format!("id{i}"), Cached { url: "u".repeat(100), expires: now() + 100, user_agent: None, format: None });
            }
        }
        let saves: Vec<_> = (0..16)
            .map(|_| {
                let resolver = resolver.clone();
                tokio::spawn(async move { resolver.save().await })
            })
            .collect();
        for save in saves {
            save.await.unwrap().unwrap();
        }
        let saved: HashMap<String, Cached> = serde_json::from_slice(&std::fs::read(dir.join("streams.json")).unwrap()).unwrap();
        assert_eq!(saved.len(), 2000);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temp file stays");
        std::fs::remove_dir_all(&dir).ok();
    }

    async fn fake_proxy(reply: &'static str) -> Route {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut conn, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = tokio::io::AsyncReadExt::read(&mut conn, &mut buf).await;
                let _ = tokio::io::AsyncWriteExt::write_all(&mut conn, reply.as_bytes()).await;
            }
        });
        let route = Route::default();
        route.set(Some(format!("http://{addr}")));
        route
    }

    fn resolver_at(name: &str, route: &Route) -> Resolver {
        let dir = std::env::temp_dir().join(format!("snellar-stream-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        Resolver::with_route(dir, route)
    }

    #[tokio::test]
    async fn the_network_cause_comes_back_instead_of_the_tool_that_ran_last() {
        let route = fake_proxy("HTTP/1.1 407 Proxy Authentication Required\r\ncontent-length: 0\r\n\r\n").await;
        let resolver = resolver_at("proxy-auth", &route);
        let e = resolver.resolve_with_ua("aaaaaaaaaaa").await.unwrap_err();
        assert_eq!(Code::of(&e), Code::ProxyAuth, "{e:#}");
    }

    #[test]
    fn the_truer_of_two_failures_is_told() {
        let pick = |d: Code, y: Code| Code::of(&truer(d.err("direct"), y.err("ytdlp")));
        assert_eq!(pick(Code::NetBlocked, Code::YtdlpMissing), Code::NetBlocked);
        assert_eq!(pick(Code::YtUnavailable, Code::YtdlpFailed), Code::YtUnavailable);
        assert_eq!(pick(Code::Internal, Code::YtdlpMissing), Code::YtdlpMissing);
        assert_eq!(pick(Code::YtParse, Code::NetLimited), Code::NetLimited);
        assert_eq!(pick(Code::YtParse, Code::YtdlpFailed), Code::YtParse);
    }

    #[test]
    fn a_track_youtube_refuses_is_unavailable() {
        let resp = json!({"playabilityStatus": {"status": "ERROR", "reason": "Video unavailable"}});
        let Err(e) = pick_audio(&resp, "TEST", Quality::Auto) else { panic!("no format in this answer") };
        assert_eq!(Code::of(&e), Code::YtUnavailable);
    }

    #[test]
    fn expiry_from_url() {
        let url = "https://rr1.googlevideo.com/videoplayback?expire=2000000000&ei=x";
        assert_eq!(expiry(url), 2000000000 - SAFETY_MARGIN);
        assert!(expiry("https://x/y") > now());
    }

    async fn full_ranges(http: &reqwest::Client, url: &str, ua: &str, length: u64) -> [bool; 4] {
        let get = |range: String| async move {
            let resp = http.get(url).header("User-Agent", ua).header("Range", range).send().await;
            resp.map(|r| r.status().as_u16() == 206 || r.status().as_u16() == 200).unwrap_or(false)
        };
        [
            get("bytes=0-131071".into()).await,
            get(format!("bytes={}-{}", length / 2, length / 2 + 524287)).await,
            get(format!("bytes={}-{}", length.saturating_sub(100_000), length - 1)).await,
            get("bytes=0-".into()).await,
        ]
    }

    #[tokio::test]
    #[ignore]
    async fn live_bench() {
        let http = reqwest::Client::new();
        let ytm = crate::ytm::Ytm::new(http.clone());
        let dir = std::env::temp_dir().join(format!("snellar-bench-{}", now()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = Resolver::new(dir);
        let queries = [
            "taylor swift anti-hero",
            "metallica one",
            "miles davis so what",
            "beethoven moonlight sonata",
            "kendrick lamar humble",
            "bad bunny titi me pregunto",
            "okean elzy bez boyu",
            "kalush stefania",
            "yoasobi idol",
            "psy gangnam style",
            "blackpink ddu-du ddu-du",
            "arijit singh tum hi ho",
            "tarkan simarik",
            "fairuz kifak inta",
            "anitta envolver",
            "rammstein du hast",
            "stromae alors on danse",
            "daft punk get lucky",
            "avicii wake me up",
            "johnny cash hurt",
            "abba dancing queen",
            "kent musik",
            "adele hello",
            "billie eilish bad guy",
        ];
        let (mut ok, mut full, mut total, mut times) = (0, 0, 0, Vec::new());
        for q in queries {
            let Some(t) = ytm.search(q, crate::ytm::Filter::Songs).await.ok().and_then(|r| r.tracks.into_iter().next()) else { continue };
            total += 1;
            let start = std::time::Instant::now();
            let hit = r.direct_resolve(t.id.video_id().unwrap()).await;
            let took = start.elapsed();
            let Ok(((url, ua), _)) = hit else {
                println!("{q:<30} {} DIRECT FAILED {took:?}", t.id);
                continue;
            };
            ok += 1;
            times.push(took);
            let length: u64 = url.split("clen=").nth(1).and_then(|v| v.split('&').next()).and_then(|v| v.parse().ok()).unwrap_or(0);
            let ranges = full_ranges(&http, &url, ua.as_deref().unwrap_or_default(), length).await;
            full += ranges.iter().all(|r| *r) as usize;
            println!(
                "{q:<30} {} direct {took:?} client={} ranges(start,mid,end,open)={ranges:?}",
                t.id,
                url.split("&c=").nth(1).and_then(|v| v.split('&').next()).unwrap_or("?")
            );
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        times.sort();
        println!("direct ok {ok}/{total}, all four ranges {full}/{total}, median {:?}, max {:?}", times.get(times.len() / 2), times.last());
    }

    #[tokio::test]
    #[ignore]
    async fn live_ytdlp_bench() {
        let dir = std::env::temp_dir().join(format!("snellar-bench-{}", now()));
        std::fs::create_dir_all(&dir).unwrap();
        let r = Resolver::new(dir);
        let ytm = crate::ytm::Ytm::new(reqwest::Client::new());
        let http = reqwest::Client::new();
        let (mut ok, mut full, mut total, mut times) = (0, 0, 0, Vec::new());
        for q in [
            "taylor swift anti-hero",
            "metallica one",
            "kendrick lamar humble",
            "okean elzy bez boyu",
            "yoasobi idol",
            "psy gangnam style",
            "rammstein du hast",
            "adele hello",
        ] {
            let Some(t) = ytm.search(q, crate::ytm::Filter::Songs).await.ok().and_then(|r| r.tracks.into_iter().next()) else { continue };
            total += 1;
            let start = std::time::Instant::now();
            let url = r.run(t.id.video_id().unwrap()).await;
            let took = start.elapsed();
            let Ok(url) = url else {
                println!("{q:<30} yt-dlp FAILED {took:?}");
                continue;
            };
            ok += 1;
            times.push(took);
            let length: u64 = url.split("clen=").nth(1).and_then(|v| v.split('&').next()).and_then(|v| v.parse().ok()).unwrap_or(0);
            let ranges = full_ranges(&http, &url, "", length).await;
            full += ranges.iter().all(|r| *r) as usize;
            println!("{q:<30} yt-dlp {took:?} ranges={ranges:?}");
        }
        times.sort();
        println!("yt-dlp ok {ok}/{total}, all four ranges {full}/{total}, median {:?}", times.get(times.len() / 2));
    }

    #[tokio::test]
    #[ignore]
    async fn live_resolve_with_ua() {
        let dir = std::env::temp_dir().join("snellar-stream-test3");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = Resolver::new(dir);
        let t = Instant::now();
        let (url, ua) = r.resolve_with_ua("dQw4w9WgXcQ").await.unwrap();
        println!("cold {:?} ua={ua:?}", t.elapsed());
        let ua = ua.expect("direct client expected");
        let resp = reqwest::Client::new().get(&url).header("User-Agent", &ua).header("Range", "bytes=0-65535").send().await.unwrap();
        println!("ranged status {} len {:?}", resp.status(), resp.bytes().await.map(|b| b.len()));
        let wrong = reqwest::Client::new().get(&url).header("Range", "bytes=0-65535").send().await.unwrap();
        println!("default-UA status {}", wrong.status());
        println!("{url}");
    }

    #[tokio::test]
    #[ignore]
    async fn live_resolve() {
        let dir = std::env::temp_dir().join("snellar-stream-test");
        std::fs::create_dir_all(&dir).unwrap();
        let r = Arc::new(Resolver::new(dir));
        let t = Instant::now();
        let (a, b) = tokio::join!(r.resolve("dQw4w9WgXcQ"), r.resolve("dQw4w9WgXcQ"));
        println!("cold (2 concurrent) {:?}", t.elapsed());
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a, b);
        println!("{:.120}", a);
        let t = Instant::now();
        r.resolve("dQw4w9WgXcQ").await.unwrap();
        println!("cached {:?}", t.elapsed());
        let err = r.resolve("xxxxxxxxxxx").await.unwrap_err();
        println!("error: {err}");
        r.prefetch("9bZkp7q19f0");
        tokio::time::sleep(Duration::from_secs(12)).await;
        println!("prefetched: {}", r.cached("9bZkp7q19f0").is_some());
    }

    #[tokio::test]
    #[ignore]
    async fn live_stages() {
        let r = Resolver::new(std::env::temp_dir().join("snellar-stages"));
        let http = reqwest::Client::new();
        for i in 0..8 {
            let t = std::time::Instant::now();
            let visitor = r.visitor("dQw4w9WgXcQ", true).await;
            let fetch = t.elapsed();
            let Ok(visitor) = visitor else {
                println!("{i} visitor failed {fetch:?}");
                continue;
            };
            let mut line = format!("{i} visitor {fetch:?}");
            for client in [&VISIONOS] {
                let t = std::time::Instant::now();
                let res = direct(&http, client, "dQw4w9WgXcQ", &visitor, Quality::Auto).await;
                line +=
                    &format!(" | {} {:?} {}", client.name, t.elapsed(), res.map(|_| "ok".to_string()).unwrap_or_else(|e| format!("{e:#}")));
            }
            println!("{line}");
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_slow_ids() {
        let r = Resolver::new(std::env::temp_dir().join("snellar-stages"));
        let http = reqwest::Client::new();
        let visitor = r.visitor("dQw4w9WgXcQ", true).await.unwrap();
        for id in ["3YgtjHZyCIQ", "m9SMT5ipbxk", "JJPlzNajdUs", "RYUsdVmMxxc", "NbEfWg8VBho"] {
            let mut line = id.to_string();
            for client in [&VISIONOS] {
                let t = std::time::Instant::now();
                let res = direct(&http, client, id, &visitor, Quality::Auto).await;
                line +=
                    &format!(" | {} {:?} {}", client.name, t.elapsed(), res.map(|_| "ok".to_string()).unwrap_or_else(|e| format!("{e:#}")));
            }
            println!("{line}");
        }
    }
}

#[cfg(test)]
mod fixtures {
    use super::*;

    const PLAYER: &str = include_str!("../../../tests/fixtures/youtube/player-dqw4w9wgxcq-2026-11-03.json");
    const LOUDNESS: &str = include_str!("../../../tests/fixtures/youtube/player-loudness-handmade-2026-11-03.json");

    #[test]
    fn the_best_audio_format_is_picked_from_a_player_answer() {
        let resp: Value = serde_json::from_str(PLAYER).unwrap();
        let pick = pick_audio(&resp, VISIONOS.name, Quality::High).unwrap();
        assert_eq!((pick.url, pick.length), ("https://rr.googlevideo.example/videoplayback?scrubbed", 3433755));
        assert_eq!(pick.format, AudioFormat { codec: "opus".into(), bitrate: 136, sample_rate: 48000, loudness_db: Some(0.97999954) });
        let blocked = json!({"playabilityStatus": {"status": "LOGIN_REQUIRED"}, "streamingData": resp["streamingData"]});
        assert!(format!("{:#}", pick_audio(&blocked, "VISIONOS", Quality::Auto).err().unwrap()).contains("LOGIN_REQUIRED"));
        let silent = json!({"playabilityStatus": {"status": "OK"}, "streamingData": {"adaptiveFormats": [{"mimeType": "video/mp4", "url": "x", "contentLength": "1"}]}});
        assert!(pick_audio(&silent, "VISIONOS", Quality::Auto).is_err());
    }

    #[test]
    fn the_quality_setting_chooses_the_format() {
        let resp: Value = serde_json::from_str(PLAYER).unwrap();
        let chosen = |q| {
            let f = pick_audio(&resp, "VISIONOS", q).unwrap().format;
            (f.codec, f.bitrate)
        };
        assert_eq!(chosen(Quality::Auto), ("opus".into(), 136));
        assert_eq!(chosen(Quality::High), ("opus".into(), 136));
        assert_eq!(chosen(Quality::DataSaver), ("opus".into(), 49));
        let all: Value = serde_json::from_str(LOUDNESS).unwrap();
        let premium = |q| pick_audio(&all["premium"], "VISIONOS", q).unwrap().url;
        assert_eq!(premium(Quality::High), "https://rr.googlevideo.example/774");
        assert_eq!(premium(Quality::Auto), "https://rr.googlevideo.example/251");
        assert_eq!(premium(Quality::DataSaver), "https://rr.googlevideo.example/140");
        assert_eq!(pick_audio(&all["config_only"], "VISIONOS", Quality::Auto).unwrap().format.codec, "aac");
        assert_eq!(
            [Quality::parse("high"), Quality::parse("dataSaver"), Quality::parse("auto"), Quality::parse("x")],
            [Quality::High, Quality::DataSaver, Quality::Auto, Quality::Auto]
        );
    }

    #[test]
    fn the_loudness_comes_from_the_format_then_the_player_config() {
        let all: Value = serde_json::from_str(LOUDNESS).unwrap();
        let loud = |case: &str| pick_audio(&all[case], "VISIONOS", Quality::Auto).unwrap().format.loudness_db;
        assert_eq!(loud("quiet"), Some(-6.4));
        assert_eq!(loud("config_only"), Some(4.25));
        assert_eq!(loud("absolute_only"), Some(5.5));
        assert_eq!(loud("none"), None);
    }

    #[test]
    fn a_cache_written_before_formats_were_kept_still_reads() {
        let old: HashMap<String, Cached> =
            serde_json::from_str(r#"{"dQw4w9WgXcQ": {"url": "https://x", "expires": 1, "user_agent": null}}"#).unwrap();
        assert_eq!(old["dQw4w9WgXcQ"].format, None);
    }
}
