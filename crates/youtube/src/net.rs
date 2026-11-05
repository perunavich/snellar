use std::error::Error as _;
use std::io::{self, ErrorKind};
use std::sync::{Arc, Mutex, RwLock};

use reqwest::header::RETRY_AFTER;
use reqwest::{ClientBuilder, NoProxy, Response, StatusCode};
use snellar_core::error::{Code, RetryAfter};
use snellar_core::journal;

pub fn cause_code(cause: &(dyn std::error::Error + 'static)) -> Option<Code> {
    if cause.is::<rustls::Error>() {
        return Some(Code::NetTls);
    }
    let text = cause.to_string();
    if text.starts_with("dns error") {
        return Some(Code::NetDns);
    }
    if let Some(tunnel) = text.strip_prefix("tunnel error: ") {
        match tunnel {
            "proxy authorization required" => return Some(Code::ProxyAuth),
            "unexpected end of file" => return Some(Code::ProxyDown),
            "unsuccessful" | "proxy response headers too long" => return Some(Code::ProxyFailed),
            _ => {}
        }
    }
    let io = cause.downcast_ref::<io::Error>()?;
    if let Some(inner) = io.get_ref().and_then(|e| cause_code(e)) {
        return Some(inner);
    }
    match io.kind() {
        ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe => {
            Some(Code::NetConnect)
        }
        ErrorKind::NetworkUnreachable | ErrorKind::HostUnreachable | ErrorKind::NetworkDown | ErrorKind::AddrNotAvailable => {
            Some(Code::NetOffline)
        }
        ErrorKind::TimedOut => Some(Code::NetTimeout),
        _ => None,
    }
}

pub fn code(e: &reqwest::Error) -> Code {
    if e.is_timeout() {
        return Code::NetTimeout;
    }
    let mut cause = e.source();
    while let Some(c) = cause {
        if let Some(code) = cause_code(c) {
            return code;
        }
        cause = c.source();
    }
    match e.status() {
        Some(StatusCode::PROXY_AUTHENTICATION_REQUIRED) => Code::ProxyAuth,
        Some(StatusCode::TOO_MANY_REQUESTS) => Code::NetLimited,
        Some(StatusCode::FORBIDDEN | StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS) => Code::NetBlocked,
        Some(s) if s.is_server_error() => Code::NetServer,
        Some(s) if s.is_client_error() => Code::NetRefused,
        _ if e.is_decode() => Code::YtParse,
        _ if e.is_connect() || e.is_request() || e.is_body() => Code::NetOffline,
        _ => Code::Internal,
    }
}

pub fn net(e: reqwest::Error) -> anyhow::Error {
    let code = code(&e);
    anyhow::Error::new(e).context(code)
}

pub fn checked(response: Response) -> anyhow::Result<Response> {
    let after = response.headers().get(RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
    response.error_for_status().map_err(|e| {
        let code = code(&e);
        let e = anyhow::Error::new(e);
        match after {
            Some(seconds) => e.context(RetryAfter(seconds)).context(code),
            None => e.context(code),
        }
    })
}

pub fn ytdlp_cause(stderr: &str) -> Option<Code> {
    const SAID: &[(&str, Code)] = &[
        ("tunnel connection failed: 407", Code::ProxyAuth),
        ("proxy authentication required", Code::ProxyAuth),
        ("tunnel connection failed", Code::ProxyFailed),
        ("proxyerror", Code::ProxyDown),
        ("cannot connect to proxy", Code::ProxyDown),
        ("name or service not known", Code::NetDns),
        ("temporary failure in name resolution", Code::NetDns),
        ("no address associated with hostname", Code::NetDns),
        ("network is unreachable", Code::NetOffline),
        ("no route to host", Code::NetOffline),
        ("certificate verify failed", Code::NetTls),
        ("certificate_verify_failed", Code::NetTls),
        ("connection refused", Code::NetConnect),
        ("connection reset", Code::NetConnect),
        ("remote end closed connection", Code::NetConnect),
        ("timed out", Code::NetTimeout),
        ("http error 429", Code::NetLimited),
        ("too many requests", Code::NetLimited),
        ("http error 403", Code::NetBlocked),
        ("http error 451", Code::NetBlocked),
        ("confirm you're not a bot", Code::NetBlocked),
        ("http error 500", Code::NetServer),
        ("http error 502", Code::NetServer),
        ("http error 503", Code::NetServer),
        ("http error 504", Code::NetServer),
        ("video unavailable", Code::YtUnavailable),
        ("private video", Code::YtUnavailable),
        ("not available in your country", Code::YtUnavailable),
        ("this video has been removed", Code::YtUnavailable),
    ];
    let stderr = stderr.to_lowercase();
    SAID.iter().find(|(words, _)| stderr.contains(words)).map(|(_, code)| *code)
}

#[derive(Clone, Default)]
pub struct Route(Arc<RwLock<(u64, Option<String>)>>);

impl Route {
    pub fn parse(text: &str) -> anyhow::Result<Option<String>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let url = if text.contains("://") { text.to_string() } else { format!("http://{text}") };
        let parsed = reqwest::Url::parse(&url).map_err(|e| Code::ProxyInvalid.err(format!("`{text}` is not a proxy address: {e}")))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none_or(str::is_empty) {
            return Err(Code::ProxyInvalid.err(format!("`{text}` is not an http:// or https:// proxy address")));
        }
        Ok(Some(url))
    }

    pub fn set(&self, proxy: Option<String>) {
        let mut state = self.0.write().unwrap();
        if state.1 != proxy {
            *state = (state.0 + 1, proxy);
        }
    }

    pub fn proxy(&self) -> Option<String> {
        self.0.read().unwrap().1.clone()
    }

    pub fn in_use(&self) -> bool {
        self.proxy().is_some()
            || ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
                .iter()
                .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
    }

    pub fn blame(&self, code: Code) -> Code {
        if !self.in_use() {
            return code;
        }
        match code {
            Code::NetDns | Code::NetOffline | Code::NetConnect => Code::ProxyDown,
            Code::NetTimeout => Code::ProxyTimeout,
            Code::NetBlocked => Code::ProxyBlocked,
            other => other,
        }
    }

    fn current(&self) -> (u64, Option<String>) {
        self.0.read().unwrap().clone()
    }
}

type Shape = Box<dyn Fn(ClientBuilder) -> ClientBuilder + Send + Sync>;

#[derive(Clone)]
pub struct Http(Arc<Held>);

struct Held {
    route: Route,
    shape: Option<Shape>,
    client: Mutex<(u64, reqwest::Client)>,
}

fn build(shape: &Shape, proxy: Option<&str>) -> reqwest::Result<reqwest::Client> {
    let mut builder = shape(reqwest::Client::builder());
    if let Some(url) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(url)?.no_proxy(NoProxy::from_string("localhost,127.0.0.1,::1")));
    }
    builder.build()
}

impl Http {
    pub fn new(route: &Route, shape: impl Fn(ClientBuilder) -> ClientBuilder + Send + Sync + 'static) -> Http {
        let shape: Shape = Box::new(shape);
        let (generation, proxy) = route.current();
        let client = build(&shape, proxy.as_deref()).expect("http client");
        Http(Arc::new(Held { route: route.clone(), shape: Some(shape), client: Mutex::new((generation, client)) }))
    }

    pub fn route(&self) -> &Route {
        &self.0.route
    }

    pub fn client(&self) -> reqwest::Client {
        let mut held = self.0.client.lock().unwrap();
        let Some(shape) = &self.0.shape else { return held.1.clone() };
        let (generation, proxy) = self.0.route.current();
        if generation != held.0 {
            held.0 = generation;
            match build(shape, proxy.as_deref()) {
                Ok(client) => held.1 = client,
                Err(e) => journal::warn("net", format!("cannot use the proxy {proxy:?}: {e}")),
            }
        }
        held.1.clone()
    }
}

impl From<reqwest::Client> for Http {
    fn from(client: reqwest::Client) -> Http {
        Http(Arc::new(Held { route: Route::default(), shape: None, client: Mutex::new((0, client)) }))
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;

    use reqwest::dns::{Name, Resolve, Resolving};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    fn http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    async fn serve_once<F, Fut>(reply: F) -> SocketAddr
    where
        F: FnOnce(TcpStream) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (conn, _) = listener.accept().await.unwrap();
            reply(conn).await;
        });
        addr
    }

    struct NoAnswer;

    impl Resolve for NoAnswer {
        fn resolve(&self, _: Name) -> Resolving {
            Box::pin(async { Err(io::Error::other("failed to lookup address information: Name or service not known").into()) })
        }
    }

    #[tokio::test]
    async fn a_failed_lookup_is_dns() {
        let http = reqwest::Client::builder().no_proxy().dns_resolver(Arc::new(NoAnswer)).build().unwrap();
        let e = http.get("https://music.youtube.com/").send().await.unwrap_err();
        assert_eq!(Code::of(&net(e)), Code::NetDns);
    }

    #[tokio::test]
    async fn a_refused_or_reset_connection_is_connect() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let refused = http().get(format!("http://127.0.0.1:{port}/")).send().await.unwrap_err();
        assert_eq!(Code::of(&net(refused)), Code::NetConnect);

        let addr = serve_once(|mut conn| async move {
            let mut buf = [0u8; 1024];
            let _ = conn.read(&mut buf).await;
            conn.set_zero_linger().unwrap();
        })
        .await;
        let reset = http().get(format!("http://{addr}/")).send().await.unwrap_err();
        assert_eq!(Code::of(&net(reset)), Code::NetConnect);
    }

    #[tokio::test]
    async fn a_host_that_does_not_speak_tls_is_tls() {
        let addr = serve_once(|mut conn| async move {
            let mut buf = [0u8; 1024];
            let _ = conn.read(&mut buf).await;
            conn.write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n").await.unwrap();
        })
        .await;
        let e = http().get(format!("https://{addr}/")).send().await.unwrap_err();
        assert_eq!(Code::of(&net(e)), Code::NetTls);
    }

    #[tokio::test]
    async fn a_silent_host_is_a_timeout() {
        let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", silent.local_addr().unwrap());
        let held = tokio::spawn(async move {
            let (_conn, _) = silent.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let slow = http().get(url).timeout(Duration::from_millis(200)).send().await.unwrap_err();
        assert_eq!(Code::of(&net(slow).context("YouTube Music search request failed")), Code::NetTimeout);
        held.abort();
    }

    #[tokio::test]
    async fn status_codes_map_to_blocked_refused_limited_and_server() {
        let cases =
            [(403, Code::NetBlocked), (451, Code::NetBlocked), (404, Code::NetRefused), (429, Code::NetLimited), (503, Code::NetServer)];
        for (status, want) in cases {
            let addr = serve_once(move |mut conn| async move {
                let mut buf = [0u8; 1024];
                let _ = conn.read(&mut buf).await;
                let reply = format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                conn.write_all(reply.as_bytes()).await.unwrap();
            })
            .await;
            let resp = http().get(format!("http://{addr}/")).send().await.unwrap();
            let e = resp.error_for_status().unwrap_err();
            assert_eq!(Code::of(&net(e)), want, "status {status}");
        }
    }

    async fn named_server(name: &'static str) -> (SocketAddr, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut conn, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let n = conn.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                log.lock().unwrap().push(head.lines().next().unwrap_or_default().to_string());
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{name}", name.len());
                conn.write_all(reply.as_bytes()).await.ok();
            }
        });
        (addr, seen)
    }

    async fn answer(http: &Http, url: &str) -> String {
        http.client().get(url).send().await.unwrap().text().await.unwrap()
    }

    #[tokio::test]
    async fn a_proxy_change_applies_to_the_next_request_and_loopback_stays_direct() {
        let (site, _) = named_server("site").await;
        let (first, first_seen) = named_server("first").await;
        let (second, _) = named_server("second").await;
        let route = Route::default();
        let http = Http::new(&route, move |b| b.no_proxy().resolve("music.test", site));
        let page = format!("http://music.test:{}/x", site.port());
        assert_eq!(answer(&http, &page).await, "site");

        route.set(Route::parse(&format!("127.0.0.1:{}", first.port())).unwrap());
        assert_eq!(answer(&http, &page).await, "first");
        assert_eq!(first_seen.lock().unwrap().as_slice(), [format!("GET {page} HTTP/1.1")]);
        assert_eq!(answer(&http, &format!("http://{site}/")).await, "site", "loopback never goes through the proxy");

        route.set(Some(format!("http://{second}")));
        assert_eq!(answer(&http, &page).await, "second");
        route.set(None);
        assert_eq!(answer(&http, &page).await, "site");
        assert_eq!(route.proxy(), None);
    }

    async fn proxy_answering(reply: &'static str) -> (Route, Http) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut conn, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = conn.read(&mut buf).await;
                conn.write_all(reply.as_bytes()).await.ok();
            }
        });
        let route = Route::default();
        route.set(Some(format!("http://{addr}")));
        let http = Http::new(&route, |b| b.no_proxy());
        (route, http)
    }

    #[tokio::test]
    async fn a_proxy_that_wants_a_login_or_answers_badly_is_named() {
        let cases = [
            ("HTTP/1.1 407 Proxy Authentication Required\r\ncontent-length: 0\r\n\r\n", Code::ProxyAuth),
            ("HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n", Code::ProxyFailed),
        ];
        for (reply, want) in cases {
            let (_, http) = proxy_answering(reply).await;
            let e = http.client().get("https://music.youtube.com/").send().await.unwrap_err();
            assert_eq!(Code::of(&net(e)), want, "{reply}");
        }
    }

    #[tokio::test]
    async fn a_dead_proxy_is_the_proxy_not_a_firewall() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let route = Route::default();
        route.set(Some(format!("http://127.0.0.1:{port}")));
        let http = Http::new(&route, |b| b.no_proxy());
        let e = http.client().get("https://music.youtube.com/").send().await.unwrap_err();
        assert_eq!(route.blame(Code::of(&net(e))), Code::ProxyDown);
    }

    #[test]
    fn without_a_proxy_nothing_is_blamed_on_one() {
        let route = Route::default();
        if route.in_use() {
            return;
        }
        for code in [Code::NetDns, Code::NetConnect, Code::NetTimeout, Code::NetBlocked] {
            assert_eq!(route.blame(code), code);
        }
        route.set(Some("http://127.0.0.1:3128".into()));
        assert_eq!(route.blame(Code::NetBlocked), Code::ProxyBlocked);
        assert_eq!(route.blame(Code::NetTimeout), Code::ProxyTimeout);
        assert_eq!(route.blame(Code::NetDns), Code::ProxyDown);
        assert_eq!(route.blame(Code::NetLimited), Code::NetLimited);
    }

    #[tokio::test]
    async fn the_waiting_time_of_a_refusal_is_kept() {
        let addr = serve_once(|mut conn| async move {
            let mut buf = [0u8; 1024];
            let _ = conn.read(&mut buf).await;
            conn.write_all(b"HTTP/1.1 429 X\r\nretry-after: 42\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap();
        })
        .await;
        let resp = http().get(format!("http://{addr}/")).send().await.unwrap();
        let e = checked(resp).unwrap_err().context("search was rejected");
        assert_eq!(Code::of(&e), Code::NetLimited);
        assert_eq!(Code::retry_after(&e), Some(42));
    }

    #[test]
    fn what_ytdlp_prints_names_the_real_cause() {
        let cases = [
            (
                "ERROR: [youtube] x: Unable to download API page: <urlopen error [Errno -3] Temporary failure in name resolution>",
                Some(Code::NetDns),
            ),
            ("ERROR: [youtube] x: Unable to download API page: HTTP Error 403: Forbidden", Some(Code::NetBlocked)),
            ("ERROR: unable to download video data: HTTP Error 429: Too Many Requests", Some(Code::NetLimited)),
            ("ERROR: [youtube] x: Video unavailable", Some(Code::YtUnavailable)),
            ("ERROR: Tunnel connection failed: 407 Proxy Authentication Required", Some(Code::ProxyAuth)),
            ("ERROR: Tunnel connection failed: 502 Bad Gateway", Some(Code::ProxyFailed)),
            ("ERROR: <urlopen error [SSL: CERTIFICATE_VERIFY_FAILED] certificate verify failed>", Some(Code::NetTls)),
            ("ERROR: something odd", None),
        ];
        for (said, want) in cases {
            assert_eq!(ytdlp_cause(said), want, "{said}");
        }
    }

    #[test]
    fn proxy_addresses_are_checked() {
        assert_eq!(Route::parse("  ").unwrap(), None);
        assert_eq!(Route::parse("proxy.lan:3128").unwrap().as_deref(), Some("http://proxy.lan:3128"));
        assert_eq!(Route::parse("https://u:p@proxy.lan:443").unwrap().as_deref(), Some("https://u:p@proxy.lan:443"));
        for bad in ["socks5://proxy.lan:1080", "http://", "not a url at all"] {
            assert_eq!(Code::of(&Route::parse(bad).unwrap_err()), Code::ProxyInvalid, "{bad}");
        }
    }
}
