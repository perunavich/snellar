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

use crate::net::Http;
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
            if shared.warmed.lock().unwrap().find(&id, &url).is_some() {
                return;
            }
            let Ok(chunk) = shared.chunk(&url, ua.as_deref(), 0, FIRST_CHUNK - 1).await else { return };
            shared.warmed.lock().unwrap().store(id, url, Arc::new(chunk));
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
                let mut resp = self.http.client().get(parsed).timeout(Duration::from_secs(10)).send().await?.error_for_status()?;
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
        let warm = self.warmed.lock().unwrap().find(id, &url).filter(|_| range.is_none_or(|(start, _)| start == 0));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_stable() {
        assert_eq!(key("https://a/b"), key("https://a/b"));
        assert_ne!(key("https://a/b"), key("https://a/c"));
    }

    #[test]
    fn sniffs_images() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF]), "image/jpeg");
        assert_eq!(sniff(b"\x89PNG"), "image/png");
        assert_eq!(sniff(b"RIFFxxxxWEBP"), "image/webp");
    }

    #[test]
    fn broken_cache_files_are_not_images() {
        assert!(is_image(&[0xFF, 0xD8, 0xFF]));
        assert!(!is_image(&[]));
        assert!(!is_image(&[0; 64]));
    }

    #[test]
    fn prune_keeps_newest() {
        let dir = std::env::temp_dir().join("snellar-prune-test");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), vec![0u8; 10]).unwrap();
        prune(&dir);
        assert!(dir.join("a").exists());
    }

    struct Running {
        base: String,
        port: u16,
        token: String,
    }

    async fn start(dir: &std::path::Path) -> Running {
        std::fs::remove_dir_all(dir).ok();
        std::fs::create_dir_all(dir).unwrap();
        let bound = bind().unwrap();
        let base = image_base(&bound);
        let (port, token) = (bound.listener.local_addr().unwrap().port(), bound.token.clone());
        Proxy::serve(bound, Arc::new(Resolver::new(dir.to_path_buf())), dir.to_path_buf()).unwrap();
        Running { base, port, token }
    }

    async fn raw(port: u16, request: &str) -> String {
        use tokio::io::AsyncReadExt;
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut out = String::new();
        stream.read_to_string(&mut out).await.unwrap();
        out
    }

    fn url(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }

    #[test]
    fn only_google_and_youtube_hosts_pass() {
        for ok in ["https://i.ytimg.com/a.jpg", "https://lh3.googleusercontent.com/x", "https://ytimg.com/a", "https://yt3.ggpht.com/a"] {
            assert!(host_allowed(&url(ok), &IMAGE_HOSTS), "{ok}");
        }
        for bad in [
            "http://i.ytimg.com/a.jpg",
            "https://evilytimg.com/a",
            "https://ytimg.com.evil.com/a",
            "https://i.ytimg.com@evil.com/a",
            "https://i.ytimg.com:8443/a",
            "https://127.0.0.1/a",
            "https://localhost/a",
            "https://[::1]/a",
            "https://169.254.169.254/a",
        ] {
            assert!(!host_allowed(&url(bad), &IMAGE_HOSTS), "{bad}");
        }
        assert!(host_allowed(&url("https://rr1---sn-abc.googlevideo.com/videoplayback"), &MEDIA_HOSTS));
        assert!(!host_allowed(&url("https://i.ytimg.com/a"), &MEDIA_HOSTS));
    }

    #[test]
    fn redirects_may_only_stay_inside_the_allowlist() {
        assert!(redirect_allowed(&url("https://rr2---sn-x.googlevideo.com/v")));
        assert!(!redirect_allowed(&url("http://127.0.0.1:631/")));
        assert!(!redirect_allowed(&url("https://evil.example/r")));
    }

    #[test]
    fn tokens_differ_between_runs() {
        assert_ne!(random_token(), random_token());
        assert_eq!(random_token().len(), 32);
    }

    #[tokio::test]
    async fn rejects_foreign_hosts_tokens_methods_and_odd_ids() {
        let proxy = start(&std::env::temp_dir().join("snellar-proxy-guard")).await;
        let (port, token) = (proxy.port, &proxy.token);
        let good = format!("GET /{token}/img?u=https%3A%2F%2F127.0.0.1%2Fa HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(raw(port, &good).await.starts_with("HTTP/1.1 403"));
        let rebound = format!("GET /{token}/dQw4w9WgXcQ HTTP/1.1\r\nHost: evil.example:{port}\r\n\r\n");
        assert!(raw(port, &rebound).await.starts_with("HTTP/1.1 403"));
        let no_token = format!("GET /dQw4w9WgXcQ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(raw(port, &no_token).await.starts_with("HTTP/1.1 404"));
        let post = format!("POST /{token}/dQw4w9WgXcQ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(raw(port, &post).await.starts_with("HTTP/1.1 405"));
        let odd = format!("GET /{token}/not-a-video-id-at-all HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(raw(port, &odd).await.starts_with("HTTP/1.1 404"));
    }

    async fn silent_server(headers: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((mut socket, _)) = listener.accept().await {
                if !headers.is_empty() {
                    let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut [0u8; 1024]).await;
                    let _ = socket.write_all(headers.as_bytes()).await;
                }
                held.push(socket);
            }
        });
        port
    }

    #[tokio::test]
    async fn an_upstream_that_goes_quiet_fails_the_request_instead_of_hanging() {
        let http = shape(reqwest::Client::builder(), Duration::from_millis(300)).build().unwrap();
        let wait = Duration::from_secs(3);

        let port = silent_server("").await;
        let started = tokio::time::timeout(wait, http.get(format!("http://127.0.0.1:{port}/")).send()).await;
        assert!(matches!(started, Ok(Err(_))), "no answer at all must end in an error: {started:?}");

        let port = silent_server("HTTP/1.1 206 Partial Content\r\nContent-Length: 1000\r\n\r\nabc").await;
        let resp = http.get(format!("http://127.0.0.1:{port}/")).send().await.unwrap();
        let body = tokio::time::timeout(wait, resp.bytes()).await;
        assert!(matches!(body, Ok(Err(_))), "a body that stops half way must end in an error: {body:?}");
    }

    struct Fake {
        client: reqwest::Client,
        url: String,
        asked: Arc<AtomicUsize>,
    }

    impl Upstream for Fake {
        async fn next(&mut self, from: u64, to: u64) -> Result<Arc<Chunk>> {
            fetch(&self.client, &self.url, None, from, to).await.map(Arc::new)
        }
    }

    async fn fake_upstream(answer: fn(u64, u64) -> (String, Vec<u8>)) -> Fake {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://127.0.0.1:{}/videoplayback", listener.local_addr().unwrap().port());
        let asked = Arc::new(AtomicUsize::new(0));
        let count = asked.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let (read, mut write) = socket.into_split();
                let mut lines = BufReader::new(read).lines();
                let mut range = (0, u64::MAX);
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.is_empty() {
                        break;
                    }
                    if let Some((from, Some(to))) = line.to_ascii_lowercase().strip_prefix("range:").and_then(|v| parse_range(v.trim())) {
                        range = (from, to);
                    }
                }
                count.fetch_add(1, Ordering::SeqCst);
                let (head, body) = answer(range.0, range.1);
                let head = format!("{head}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = write.write_all(head.as_bytes()).await;
                let _ = write.write_all(&body).await;
            }
        });
        Fake { client: reqwest::Client::new(), url, asked }
    }

    async fn relayed(upstream: &mut Fake, range: Option<(u64, Option<u64>)>) -> (Result<()>, String) {
        let mut out = Vec::new();
        let done = tokio::time::timeout(Duration::from_secs(5), relay(&mut out, range, upstream)).await;
        let done =
            done.unwrap_or_else(|_| panic!("the relay never ended after {} upstream requests", upstream.asked.load(Ordering::SeqCst)));
        (done, String::from_utf8_lossy(&out).into_owned())
    }

    const SONG: u64 = 300 * 1024;

    fn song(from: u64, to: u64) -> (String, Vec<u8>) {
        let to = to.min(SONG - 1);
        let head = format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{to}/{SONG}\r\nContent-Type: audio/webm\r\n");
        (head, (from..=to).map(|i| (i % 251) as u8).collect())
    }

    #[tokio::test]
    async fn a_range_is_relayed_in_chunks_from_the_upstream() {
        let mut upstream = fake_upstream(song).await;
        let mut out = Vec::new();
        relay(&mut out, Some((1000, None)), &mut upstream).await.unwrap();
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let head = String::from_utf8_lossy(&out[..split]);
        assert!(head.starts_with("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1000-307199/307200\r\n"), "{head}");
        assert!(head.contains(&format!("Content-Length: {}\r\n", SONG - 1000)), "{head}");
        assert_eq!(out[split..], song(1000, SONG - 1).1);
        assert_eq!(upstream.asked.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_warmed_chunk_is_used_only_for_the_url_it_came_from() {
        let chunk = |total| Arc::new(Chunk { body: Bytes::from_static(b"x"), total, kind: String::new() });
        let mut warmed = Warmed::default();
        warmed.store("a".into(), "https://r1/high".into(), chunk(100));
        assert_eq!(warmed.find("a", "https://r1/high").unwrap().total, 100);
        assert!(warmed.find("a", "https://r1/low").is_none(), "another quality or a resolved-again URL is another file");
        assert!(warmed.find("b", "https://r1/high").is_none());
        warmed.store("a".into(), "https://r1/low".into(), chunk(50));
        assert!(warmed.find("a", "https://r1/high").is_none(), "the old file is replaced");
        assert_eq!(warmed.find("a", "https://r1/low").unwrap().total, 50);
        for i in 0..WARM_SLOTS {
            warmed.store(format!("x{i}"), "u".into(), chunk(1));
        }
        assert_eq!(warmed.0.len(), WARM_SLOTS, "the slots stay bounded");
        assert!(warmed.find("a", "https://r1/low").is_none());
    }

    #[tokio::test]
    async fn an_upstream_whose_length_changes_mid_song_fails_instead_of_splicing_two_files() {
        let mut upstream = fake_upstream(|from, to| {
            if from == 0 {
                song(from, to)
            } else {
                (
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{to}/{}\r\nContent-Type: audio/webm\r\n",
                        SONG + 1
                    ),
                    vec![7; 16],
                )
            }
        })
        .await;
        let (done, out) = relayed(&mut upstream, None).await;
        assert_eq!(Code::of(&done.unwrap_err()), Code::NetServer);
        assert!(out.starts_with("HTTP/1.1 200 OK\r\n"), "{out}");
        assert_eq!(upstream.asked.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_upstream_that_sends_empty_bodies_ends_the_stream_after_a_few_tries() {
        let mut upstream =
            fake_upstream(|_, _| ("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/1000\r\n".into(), Vec::new())).await;
        let (done, out) = relayed(&mut upstream, Some((0, None))).await;
        assert_eq!(Code::of(&done.unwrap_err()), Code::NetServer);
        assert!(out.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{out}");
        assert_eq!(upstream.asked.load(Ordering::SeqCst), 1 + RESUME_TRIES);
    }

    #[tokio::test]
    async fn an_upstream_that_stops_sending_mid_song_ends_the_stream() {
        let mut upstream = fake_upstream(|from, to| if from == 0 { song(from, to) } else { (song(from, to).0, Vec::new()) }).await;
        let (done, out) = relayed(&mut upstream, None).await;
        assert_eq!(Code::of(&done.unwrap_err()), Code::NetServer);
        assert!(out.starts_with("HTTP/1.1 200 OK\r\n"), "{out}");
        assert_eq!(upstream.asked.load(Ordering::SeqCst), 2 + RESUME_TRIES);
    }

    #[tokio::test]
    async fn a_zero_length_upstream_ends_with_an_error_instead_of_a_panic() {
        let mut upstream =
            fake_upstream(|_, _| ("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes */0\r\n".into(), b"abc".to_vec())).await;
        for range in [None, Some((0, None))] {
            let (done, out) = relayed(&mut upstream, range).await;
            assert_eq!(Code::of(&done.unwrap_err()), Code::NetServer);
            assert!(out.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{out}");
        }
    }

    #[tokio::test]
    async fn a_range_past_the_end_is_answered_416() {
        let mut upstream = fake_upstream(|_, _| song(0, SONG - 1)).await;
        let (done, out) = relayed(&mut upstream, Some((SONG + 50, None))).await;
        done.unwrap();
        assert!(out.starts_with(&format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{SONG}\r\n")), "{out}");
        assert_eq!(upstream.asked.load(Ordering::SeqCst), 1);

        let (done, out) = relayed(&mut upstream, Some((500, Some(100)))).await;
        done.unwrap();
        assert!(out.starts_with("HTTP/1.1 416 Range Not Satisfiable\r\n"), "{out}");
        assert_eq!(upstream.asked.load(Ordering::SeqCst), 1, "an inverted range is refused without asking upstream");

        let (done, out) = relayed(&mut upstream, Some((u64::MAX - 1, None))).await;
        done.unwrap();
        assert!(out.starts_with("HTTP/1.1 416 Range Not Satisfiable\r\n"), "{out}");
    }

    #[tokio::test]
    async fn refuses_local_and_plain_http_images() {
        let proxy = start(&std::env::temp_dir().join("snellar-img-refuse")).await;
        for target in ["http://i.ytimg.com/a.jpg", "https://127.0.0.1/a.jpg", "https://localhost/a.jpg", "https://evil.example/a.jpg"] {
            let resp = reqwest::get(format!("{}{}", proxy.base, target.replace(':', "%3A").replace('/', "%2F"))).await.unwrap();
            assert_eq!(resp.status().as_u16(), 403, "{target}");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_image_cache() {
        let dir = std::env::temp_dir().join("snellar-img-live");
        let base = start(&dir).await.base;
        let url = format!("{base}https%3A%2F%2Fi.ytimg.com%2Fvi%2FdQw4w9WgXcQ%2Fhqdefault.jpg");
        let client = reqwest::Client::new();
        let t = std::time::Instant::now();
        let first = client.get(&url).send().await.unwrap();
        assert_eq!(first.headers()["content-type"], "image/jpeg");
        let size = first.bytes().await.unwrap().len();
        let miss = t.elapsed();
        let t = std::time::Instant::now();
        assert_eq!(client.get(&url).send().await.unwrap().bytes().await.unwrap().len(), size);
        println!("miss {miss:?} hit {:?} ({size} bytes)", t.elapsed());
        assert_eq!(std::fs::read_dir(&dir).unwrap().flatten().filter(|e| e.file_name().len() == 16).count(), 1);
    }
}
