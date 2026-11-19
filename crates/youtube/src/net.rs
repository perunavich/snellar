use std::error::Error as _;
use std::io::{self, ErrorKind};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use reqwest::{ClientBuilder, NoProxy, Response, StatusCode};
use snellar_core::error::{Code, RetryAfter};
use snellar_core::journal;
use snellar_core::sync::{Lock, Share};

use crate::reach::{self, Armed, Reach, Targets, Verdict};

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

pub fn coded(e: anyhow::Error, code: Code, after: Option<u64>) -> anyhow::Error {
    match after {
        Some(seconds) => e.context(RetryAfter(seconds)).context(code),
        None => e.context(code),
    }
}

pub fn checked(response: Response) -> anyhow::Result<Response> {
    let after = response.headers().get(RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
    response.error_for_status().map_err(|e| {
        let code = code(&e);
        coded(anyhow::Error::new(e), code, after)
    })
}

pub async fn get(http: &Http, url: &str, params: &[(&str, &str)], headers: &[(&str, &str)], timeout: Duration) -> anyhow::Result<Response> {
    let url = if params.is_empty() { reqwest::Url::parse(url)? } else { reqwest::Url::parse_with_params(url, params)? };
    let mut request = http.client().get(url).timeout(timeout);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().await.map_err(net)
}

pub async fn json<T: serde::de::DeserializeOwned>(response: Response) -> anyhow::Result<T> {
    checked(response)?.json().await.map_err(net)
}

pub fn ytdlp_cause(stderr: &str) -> Option<Code> {
    const SAID: &[(&str, Code)] = &[
        ("confirm your age", Code::YtAgeRestricted),
        ("age-restricted", Code::YtAgeRestricted),
        ("inappropriate for some users", Code::YtAgeRestricted),
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
        ("in your country", Code::YtRegion),
        ("private video", Code::YtRemoved),
        ("this video is private", Code::YtRemoved),
        ("this video has been removed", Code::YtRemoved),
        ("no longer available", Code::YtRemoved),
        ("account associated with this video has been terminated", Code::YtRemoved),
        // bare: yt-dlp prints this for geo, bot checks and more, so it proves nothing lasting
        ("video unavailable", Code::YtUnavailable),
    ];
    let stderr = stderr.to_lowercase();
    SAID.iter().find(|(words, _)| stderr.contains(words)).map(|(_, code)| *code)
}

#[derive(Clone, Default)]
pub struct Route {
    way: Arc<RwLock<(u64, Option<String>)>>,
    reach: Arc<Reach>,
}

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
        let mut state = self.way.writing();
        if state.1 != proxy {
            *state = (state.0 + 1, proxy);
        }
    }

    pub fn proxy(&self) -> Option<String> {
        self.way.reading().1.clone()
    }

    pub fn in_use(&self) -> bool {
        self.proxy().is_some()
            || ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
                .iter()
                .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
    }

    pub fn blame(&self, code: Code) -> Code {
        if !self.in_use() {
            return if self.reach.verdict() == Verdict::Blocked && reach::suspicious(code) { Code::NetYoutubeBlocked } else { code };
        }
        match code {
            Code::NetDns | Code::NetOffline | Code::NetConnect => Code::ProxyDown,
            Code::NetTimeout => Code::ProxyTimeout,
            Code::NetBlocked => Code::ProxyBlocked,
            other => other,
        }
    }

    fn current(&self) -> (u64, Option<String>) {
        self.way.reading().clone()
    }

    pub fn reach(&self) -> &Reach {
        &self.reach
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
        let mut held = self.0.client.locked();
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

impl Route {
    pub fn watch(&self, targets: Targets, tell: impl Fn(Option<Code>) + Send + Sync + 'static) {
        self.reach().arm(targets, code, tell);
    }

    pub fn verdict(&self) -> Verdict {
        if self.in_use() { Verdict::Online } else { self.reach().verdict() }
    }

    // After a request failed in a way that may mean YouTube is out of reach, look once (not more
    // often than every 10 s) before the failure is named, so the reply and the banner agree.
    pub async fn settle(&self, code: Code) {
        let Some(armed) = self.reach().armed() else { return };
        if !reach::suspicious(code) || self.in_use() {
            return;
        }
        let _flight = self.reach().flight.lock().await;
        if self.reach().fresh() {
            return;
        }
        let seen = Reach::look(&armed, true).await;
        if self.reach().record(seen, &armed) {
            self.poll(armed);
        }
    }

    fn poll(&self, armed: Arc<Armed>) {
        let route = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(armed.every()).await;
                let reach = route.reach();
                let _flight = reach.flight.lock().await;
                let seen = if route.in_use() { Verdict::Online } else { Reach::look(&armed, reach.verdict() != Verdict::Blocked).await };
                reach.record(seen, &armed);
                if seen == Verdict::Online {
                    return;
                }
            }
        });
    }
}
