use std::collections::VecDeque;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::net::TcpListener as StdListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use bytes::Bytes;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};

use snellar_core::error::Code;
use snellar_core::ids::valid_id;
use snellar_core::sync::Lock;

use crate::net::{self, Http};
use crate::stream::Resolver;

const FIRST_CHUNK: u64 = 128 * 1024;
const CHUNK: u64 = 512 * 1024;
const WARM_SLOTS: usize = 8;
const IMAGE_CAP: u64 = 300 * 1024 * 1024;
const IMAGE_MAX_BYTES: usize = 8 * 1024 * 1024;
const PRUNE_EVERY: usize = 64;
const RESUME_TRIES: usize = 2;
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const KEEP_ALIVE_PING: Duration = Duration::from_secs(15);
const KEEP_ALIVE_WAIT: Duration = Duration::from_secs(5);
const IMAGE_HOSTS: [&str; 5] = ["ytimg.com", "googleusercontent.com", "ggpht.com", "gstatic.com", "youtube.com"];
const MEDIA_HOSTS: [&str; 3] = ["googlevideo.com", "youtube.com", "googleusercontent.com"];

struct Chunk {
    body: Bytes,
    total: u64,
    kind: String,
}

struct Images {
    dir: PathBuf,
    writes: AtomicUsize,
}

struct Shared {
    port: u16,
    token: String,
    resolver: Arc<Resolver>,
    http: Http,
    images: Images,
    warmed: Mutex<Warmed>,
}

pub struct Proxy {
    shared: Arc<Shared>,
}

pub struct Bound {
    listener: StdListener,
    token: String,
}

fn host_allowed(url: &reqwest::Url, suffixes: &[&str]) -> bool {
    url.scheme() == "https"
        && url.port().is_none_or(|p| p == 443)
        && url
            .host_str()
            .is_some_and(|host| suffixes.iter().any(|s| host == *s || host.strip_suffix(s).is_some_and(|rest| rest.ends_with('.'))))
}

fn random_token() -> String {
    let state = RandomState::new();
    let mut out = String::new();
    for round in 0..2u8 {
        let mut hasher = state.build_hasher();
        round.hash(&mut hasher);
        SystemTime::now().hash(&mut hasher);
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

fn redirect_allowed(url: &reqwest::Url) -> bool {
    host_allowed(url, &IMAGE_HOSTS) || host_allowed(url, &MEDIA_HOSTS)
}

fn shape(builder: reqwest::ClientBuilder, read_timeout: Duration) -> reqwest::ClientBuilder {
    let redirects = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() < 5 && redirect_allowed(attempt.url()) { attempt.follow() } else { attempt.stop() }
    });
    builder
        .redirect(redirects)
        .tcp_nodelay(true)
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(read_timeout)
        .http2_keep_alive_interval(KEEP_ALIVE_PING)
        .http2_keep_alive_timeout(KEEP_ALIVE_WAIT)
        .http2_keep_alive_while_idle(true)
        .pool_idle_timeout(Duration::from_secs(120))
}

pub fn bind() -> Result<Bound> {
    let listener = StdListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    Ok(Bound { listener, token: random_token() })
}

pub fn image_base(bound: &Bound) -> String {
    format!("http://127.0.0.1:{}/{}/img?u=", bound.listener.local_addr().map_or(0, |a| a.port()), bound.token)
}

impl Proxy {
    pub fn serve(bound: Bound, resolver: Arc<Resolver>, image_dir: PathBuf) -> Result<Arc<Proxy>> {
        let port = bound.listener.local_addr()?.port();
        let listener = TcpListener::from_std(bound.listener)?;
        let http = Http::new(resolver.route(), |b| shape(b, READ_TIMEOUT));
        let shared = Arc::new(Shared {
            port,
            token: bound.token,
            resolver,
            http,
            images: Images { dir: image_dir, writes: AtomicUsize::new(0) },
            warmed: Mutex::default(),
        });
        let images = shared.images.dir.clone();
        tokio::task::spawn_blocking(move || prune(&images));
        for url in ["https://i.ytimg.com/", "https://lh3.googleusercontent.com/"] {
            let http = shared.http.client();
            tokio::spawn(async move {
                let _ = http.head(url).timeout(Duration::from_secs(5)).send().await;
            });
        }
        let accept = shared.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let shared = accept.clone();
                tokio::spawn(async move {
                    let _ = shared.handle(socket).await;
                });
            }
        });
        Ok(Arc::new(Proxy { shared }))
    }

    pub fn url(&self, video_id: &str) -> String {
        format!("http://127.0.0.1:{}/{}/{video_id}", self.shared.port, self.shared.token)
    }

    pub fn warm(&self, video_id: &str) {
        if !valid_id(video_id) {
            return;
        }
        let shared = self.shared.clone();
        let id = video_id.to_string();
        tokio::spawn(async move {
            let Ok((url, ua)) = shared.resolver.resolve_with_ua(&id).await else { return };
            if shared.warmed.locked().find(&id, &url).is_some() {
                return;
            }
            let Ok(chunk) = shared.chunk(&url, ua.as_deref(), 0, FIRST_CHUNK - 1).await else { return };
            shared.warmed.locked().store(id, url, Arc::new(chunk));
        });
    }
}

fn parse_range(header: &str) -> Option<(u64, Option<u64>)> {
    let (start, end) = header.strip_prefix("bytes=")?.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()))
}

fn sniff(bytes: &[u8]) -> &'static str {
    match bytes {
        [0xFF, 0xD8, ..] => "image/jpeg",
        [0x89, b'P', ..] => "image/png",
        [b'R', b'I', b'F', b'F', ..] => "image/webp",
        [b'G', b'I', b'F', ..] => "image/gif",
        _ => "application/octet-stream",
    }
}

fn is_image(bytes: &[u8]) -> bool {
    sniff(bytes) != "application/octet-stream"
}

fn key(url: &str) -> String {
    let hash = url.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{hash:016x}")
}

fn prune(dir: &PathBuf) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= IMAGE_CAP {
        return;
    }
    files.sort();
    for (_, size, path) in files {
        if total <= IMAGE_CAP / 10 * 9 {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= size;
        }
    }
}

async fn respond(write: &mut (impl AsyncWrite + Unpin), status: &str, body: &[u8]) -> Result<()> {
    let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    write.write_all(head.as_bytes()).await?;
    write.write_all(body).await?;
    Ok(())
}

impl Shared {
    async fn handle(&self, socket: TcpStream) -> Result<()> {
        let (read, mut write) = socket.into_split();
        let mut lines = BufReader::new(read).lines();
        let request = lines.next_line().await?.context("empty request")?;
        let mut parts = request.split_whitespace();
        let (method, target) = (parts.next().unwrap_or_default().to_string(), parts.next().unwrap_or_default().to_string());
        let (mut range, mut host) = (None, String::new());
        while let Some(line) = lines.next_line().await? {
            if line.is_empty() {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("range:") {
                range = parse_range(value.trim());
            } else if let Some(value) = lower.strip_prefix("host:") {
                host = value.trim().to_string();
            }
        }
        if method != "GET" {
            return respond(&mut write, "405 Method Not Allowed", &[]).await;
        }
        if host != format!("127.0.0.1:{}", self.port) && host != format!("localhost:{}", self.port) {
            return respond(&mut write, "403 Forbidden", &[]).await;
        }
        let Some(path) = target.strip_prefix('/').and_then(|t| t.strip_prefix(self.token.as_str())).and_then(|t| t.strip_prefix('/'))
        else {
            return respond(&mut write, "404 Not Found", &[]).await;
        };
        match path.strip_prefix("img?u=") {
            Some(encoded) => self.image(&mut write, encoded).await,
            None if valid_id(path) => self.audio(&mut write, path, range).await,
            None => respond(&mut write, "404 Not Found", &[]).await,
        }
    }

    async fn image(&self, write: &mut OwnedWriteHalf, encoded: &str) -> Result<()> {
        let url = reqwest::Url::parse(&format!("http://x/?u={encoded}"))?
            .query_pairs()
            .find_map(|(k, v)| (k == "u").then(|| v.into_owned()))
            .context("missing url")?;
        let parsed = reqwest::Url::parse(&url)?;
        if !host_allowed(&parsed, &IMAGE_HOSTS) {
            return respond(write, "403 Forbidden", &[]).await;
        }
        let path = self.images.dir.join(key(&url));
        let body = match tokio::fs::read(&path).await {
            Ok(body) if is_image(&body) => {
                if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&path) {
                    file.set_modified(SystemTime::now()).ok();
                }
                Bytes::from(body)
            }
            _ => {
                let sent = self.http.client().get(parsed).timeout(Duration::from_secs(10)).send().await;
                let mut resp = match sent {
                    Ok(resp) => resp.error_for_status()?,
                    Err(e) => {
                        let route = self.resolver.route().clone();
                        let code = net::code(&e);
                        tokio::spawn(async move { route.settle(code).await });
                        return Err(e.into());
                    }
                };
                if !resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|k| k.starts_with("image/")) {
                    bail!("not an image");
                }
                let mut body = Vec::new();
                while let Some(part) = resp.chunk().await? {
                    body.extend_from_slice(&part);
                    if body.len() > IMAGE_MAX_BYTES {
                        bail!("image too large");
                    }
                }
                if body.is_empty() {
                    bail!("empty image");
                }
                let tmp = path.with_extension(format!("{}.tmp", random_token()));
                tokio::fs::write(&tmp, &body).await?;
                tokio::fs::rename(&tmp, &path).await?;
                if self.images.writes.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY == PRUNE_EVERY - 1 {
                    let dir = self.images.dir.clone();
                    tokio::task::spawn_blocking(move || prune(&dir));
                }
                Bytes::from(body)
            }
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: max-age=31536000\r\nConnection: close\r\n\r\n",
            sniff(&body),
            body.len()
        );
        write.write_all(head.as_bytes()).await?;
        write.write_all(&body).await?;
        Ok(())
    }

    async fn chunk(&self, url: &str, user_agent: Option<&str>, from: u64, to: u64) -> Result<Chunk> {
        if !host_allowed(&reqwest::Url::parse(url)?, &MEDIA_HOSTS) {
            bail!("refusing to fetch from a host outside the media allowlist");
        }
        fetch(&self.http.client(), url, user_agent, from, to).await
    }

    async fn resilient_chunk(&self, id: &str, url: &mut String, user_agent: &mut Option<String>, from: u64, to: u64) -> Result<Chunk> {
        let mut failures = 0;
        loop {
            match self.chunk(url, user_agent.as_deref(), from, to).await {
                Ok(chunk) => return Ok(chunk),
                Err(e) if failures == RESUME_TRIES => return Err(e),
                Err(_) => {
                    failures += 1;
                    self.resolver.invalidate(id);
                    (*url, *user_agent) = self.resolver.resolve_with_ua(id).await?;
                }
            }
        }
    }

    async fn audio(&self, write: &mut OwnedWriteHalf, id: &str, range: Option<(u64, Option<u64>)>) -> Result<()> {
        let (url, user_agent) = self.resolver.resolve_with_ua(id).await?;
        let warm = self.warmed.locked().find(id, &url).filter(|_| range.is_none_or(|(start, _)| start == 0));
        relay(write, range, &mut Media { shared: self, id, url, user_agent, warm }).await
    }
}

trait Upstream {
    fn next(&mut self, from: u64, to: u64) -> impl Future<Output = Result<Arc<Chunk>>> + Send;
}

#[derive(Default)]
struct Warmed(VecDeque<(String, String, Arc<Chunk>)>);

impl Warmed {
    fn find(&self, id: &str, url: &str) -> Option<Arc<Chunk>> {
        self.0.iter().find(|(warm, from, _)| warm == id && from == url).map(|(_, _, chunk)| chunk.clone())
    }

    fn store(&mut self, id: String, url: String, chunk: Arc<Chunk>) {
        self.0.retain(|(warm, _, _)| *warm != id);
        if self.0.len() >= WARM_SLOTS {
            self.0.pop_front();
        }
        self.0.push_back((id, url, chunk));
    }
}

struct Media<'a> {
    shared: &'a Shared,
    id: &'a str,
    url: String,
    user_agent: Option<String>,
    warm: Option<Arc<Chunk>>,
}

impl Upstream for Media<'_> {
    async fn next(&mut self, from: u64, to: u64) -> Result<Arc<Chunk>> {
        match self.warm.take() {
            Some(chunk) => Ok(chunk),
            None => self.shared.resilient_chunk(self.id, &mut self.url, &mut self.user_agent, from, to).await.map(Arc::new),
        }
    }
}

async fn fetch(client: &reqwest::Client, url: &str, user_agent: Option<&str>, from: u64, to: u64) -> Result<Chunk> {
    let mut req = client.get(url).header("Range", format!("bytes={from}-{to}"));
    if let Some(ua) = user_agent {
        req = req.header("User-Agent", ua);
    }
    let resp = req.send().await?;
    if resp.status().as_u16() != 206 {
        bail!("upstream answered {}", resp.status());
    }
    let total = resp
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit_once('/'))
        .and_then(|(_, total)| total.parse().ok())
        .context("upstream sent no content range")?;
    let kind = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("audio/webm").to_string();
    Ok(Chunk { body: resp.bytes().await?, total, kind })
}

async fn relay(write: &mut (impl AsyncWrite + Unpin), range: Option<(u64, Option<u64>)>, upstream: &mut impl Upstream) -> Result<()> {
    let (start, wanted_end) = range.unwrap_or((0, None));
    if wanted_end.is_some_and(|end| end < start) {
        return respond(write, "416 Range Not Satisfiable", &[]).await;
    }
    let mut pos = start;
    let mut stop = None;
    let mut total = None;
    let mut empty = 0;
    while stop.is_none_or(|s| pos <= s) {
        let size = if pos == start { FIRST_CHUNK } else { CHUNK };
        let to = pos.saturating_add(size - 1).min(stop.or(wanted_end).unwrap_or(u64::MAX));
        let chunk = upstream.next(pos, to).await?;
        let Some(end) = chunk.total.checked_sub(1) else {
            return broken(write, stop.is_some(), "upstream reported a length of 0").await;
        };
        if total.is_some_and(|known| known != chunk.total) {
            return broken(write, stop.is_some(), "upstream changed the length of the file mid-stream").await;
        }
        total = Some(chunk.total);
        let last = stop.unwrap_or_else(|| wanted_end.unwrap_or(end).min(end));
        if start > last {
            let head = format!(
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                chunk.total
            );
            return Ok(write.write_all(head.as_bytes()).await?);
        }
        if chunk.body.is_empty() {
            if empty == RESUME_TRIES {
                return broken(write, stop.is_some(), "upstream kept sending empty bodies").await;
            }
            empty += 1;
            continue;
        }
        empty = 0;
        if stop.is_none() {
            let head = match range {
                Some(_) => format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{last}/{}\r\nContent-Length: {}\r\n",
                    chunk.total,
                    last - start + 1
                ),
                None => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n", chunk.total),
            };
            let head = format!("{head}Content-Type: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n", chunk.kind);
            write.write_all(head.as_bytes()).await?;
            stop = Some(last);
        }
        let take = chunk.body.len().min((last.min(to) - pos + 1) as usize);
        write.write_all(&chunk.body[..take]).await?;
        pos += take as u64;
    }
    Ok(())
}

async fn broken(write: &mut (impl AsyncWrite + Unpin), head_sent: bool, why: &str) -> Result<()> {
    if !head_sent {
        respond(write, "502 Bad Gateway", &[]).await?;
    }
    Err(Code::NetServer.err(why))
}
