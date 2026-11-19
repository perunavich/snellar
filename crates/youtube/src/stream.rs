use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use snellar_core::cache::{Flights, write_atomic};
use snellar_core::clock::unix_secs;
use snellar_core::error::Code;
use snellar_core::model::AudioFormat;
use snellar_core::sync::Lock;
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
const REFUSAL_TTL: u64 = 900;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    #[default]
    Auto,
    High,
    DataSaver,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Auto, Quality::High, Quality::DataSaver];

    pub fn name(self) -> &'static str {
        match self {
            Quality::Auto => "auto",
            Quality::High => "high",
            Quality::DataSaver => "dataSaver",
        }
    }

    pub fn named(s: &str) -> Option<Quality> {
        Quality::ALL.into_iter().find(|q| q.name() == s)
    }

    pub fn parse(s: &str) -> Quality {
        Quality::named(s).unwrap_or_default()
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

struct Refusal {
    at: u64,
    code: Code,
    proxy: Option<String>,
}

pub struct Resolver {
    http: Http,
    cache_dir: PathBuf,
    cache: Mutex<HashMap<String, Cached>>,
    flights: Flights,
    visitor: tokio::sync::Mutex<Option<(u64, String)>>,
    refused: Mutex<HashMap<String, Refusal>>,
    saving: tokio::sync::Mutex<()>,
    ytdlp_enabled: AtomicBool,
    ytdlp_runs: AtomicU64,
    quality: AtomicU8,
}

fn expiry(url: &str) -> u64 {
    let expire = url
        .split(['?', '&', '/'])
        .find_map(|p| p.strip_prefix("expire=").or_else(|| p.strip_prefix("expire/")))
        .and_then(|v| v.parse::<u64>().ok());
    expire.map_or(unix_secs() + FALLBACK_TTL, |e| e.saturating_sub(SAFETY_MARGIN))
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
            .filter(|(_, c)| c.expires > unix_secs())
            .collect();
        Self {
            http: Http::new(route, |b| b.connect_timeout(CONNECT_LIMIT)),
            cache_dir,
            cache: Mutex::new(cache),
            flights: Flights::default(),
            visitor: tokio::sync::Mutex::default(),
            refused: Mutex::default(),
            saving: tokio::sync::Mutex::default(),
            ytdlp_enabled: AtomicBool::new(true),
            ytdlp_runs: AtomicU64::new(0),
            quality: AtomicU8::new(Quality::Auto as u8),
        }
    }

    fn refusal(&self, id: &str) -> Option<anyhow::Error> {
        let proxy = self.http.route().proxy();
        let mut refused = self.refused.locked();
        match refused.get(id) {
            Some(r) if r.proxy == proxy && r.at + REFUSAL_TTL > unix_secs() => {
                Some(r.code.err(format!("YouTube already refused {id} in this session")))
            }
            Some(_) => {
                refused.remove(id);
                None
            }
            None => None,
        }
    }

    fn refuse(&self, id: &str, e: &anyhow::Error) {
        let code = Code::of(e);
        if code.refused_for_good() {
            let refusal = Refusal { at: unix_secs(), code, proxy: self.http.route().proxy() };
            self.refused.locked().insert(id.into(), refusal);
        }
    }

    fn cached(&self, id: &str) -> Option<Resolved> {
        let cache = self.cache.locked();
        cache.get(id).filter(|c| c.expires > unix_secs()).map(|c| (c.url.clone(), c.user_agent.clone()))
    }

    pub fn invalidate(&self, video_id: &str) {
        self.cache.locked().remove(video_id);
        self.refused.locked().remove(video_id);
    }

    pub fn format(&self, video_id: &str) -> Option<AudioFormat> {
        self.cache.locked().get(video_id).and_then(|c| c.format.clone())
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
            self.cache.locked().clear();
        }
    }

    pub async fn resolve(&self, video_id: &str) -> Result<String> {
        Ok(self.resolve_with_ua(video_id).await?.0)
    }

    pub async fn resolve_with_ua(&self, video_id: &str) -> Result<Resolved> {
        if let Some(hit) = self.cached(video_id) {
            return Ok(hit);
        }
        if let Some(refused) = self.refusal(video_id) {
            return Err(refused);
        }
        let _flight = self.flights.hold(video_id).await;
        if let Some(hit) = self.cached(video_id) {
            return Ok(hit);
        }
        if let Some(refused) = self.refusal(video_id) {
            return Err(refused);
        }
        let ((url, user_agent), format) = match self.direct_resolve(video_id).await {
            Ok(hit) => hit,
            Err(direct) if Code::of(&direct).nothing_gets_through() => return Err(direct),
            Err(direct) if Code::of(&direct).refused_for_good() => {
                self.refuse(video_id, &direct);
                return Err(direct);
            }
            Err(direct) => match self.fallback(video_id).await {
                Ok(url) => ((url, None), None),
                Err(other) => {
                    let told = truer(direct, other);
                    self.refuse(video_id, &told);
                    return Err(told);
                }
            },
        };
        {
            let entry = Cached { url: url.clone(), expires: expiry(&url), user_agent: user_agent.clone(), format };
            self.cache.locked().insert(video_id.into(), entry);
        }
        self.save().await?;
        Ok((url, user_agent))
    }

    async fn save(&self) -> Result<()> {
        let _saving = self.saving.lock().await;
        let snapshot = {
            let mut cache = self.cache.locked();
            let now = unix_secs();
            cache.retain(|_, c| c.expires > now);
            serde_json::to_vec(&*cache)?
        };
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
            let age = unix_secs().saturating_sub(*at);
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
        *slot = Some((unix_secs(), visitor.clone()));
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
    (code.network() && code != Code::NetTimeout) || code.refused_for_good()
}

fn truer(direct: anyhow::Error, ytdlp: anyhow::Error) -> anyhow::Error {
    let (d, y) = (Code::of(&direct), Code::of(&ytdlp));
    if d.refused_for_good() {
        direct
    } else if y.refused_for_good() {
        ytdlp
    } else if d == Code::YtUnavailable || d.network() {
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

fn refusal_code(resp: &Value, status: &str) -> Code {
    let playability = &resp["playabilityStatus"];
    let reasons = [
        playability["reason"].as_str(),
        playability["errorScreen"]["playerErrorMessageRenderer"]["reason"]["simpleText"].as_str(),
        playability["errorScreen"]["playerErrorMessageRenderer"]["subreason"]["simpleText"].as_str(),
    ];
    let said = |words: &[&str]| reasons.iter().flatten().any(|r| words.iter().any(|w| r.to_lowercase().contains(w)));
    if playability.get("desktopLegacyAgeGateReason").is_some()
        || matches!(status, "AGE_VERIFICATION_REQUIRED" | "AGE_CHECK_REQUIRED")
        || said(&["confirm your age", "age-restricted", "inappropriate for some users"])
    {
        Code::YtAgeRestricted
    } else if said(&["in your country"]) {
        Code::YtRegion
    } else if said(&["private video", "this video is private", "has been removed", "no longer available"]) {
        Code::YtRemoved
    } else {
        Code::YtUnavailable
    }
}

fn pick_audio<'a>(resp: &'a Value, client: &str, quality: Quality) -> Result<Pick<'a>> {
    let status = resp["playabilityStatus"]["status"].as_str().unwrap_or("UNKNOWN");
    if status != "OK" {
        return Err(refusal_code(resp, status).err(format!("{client} playability: {status}")));
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
