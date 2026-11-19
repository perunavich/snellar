use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::ConfigVerifierExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use snellar_core::error::Code;
use snellar_core::product;
use snellar_youtube::net::cause_code;

const STEP: Duration = Duration::from_secs(5);

const HOSTS: [(&str, &str); 9] = [
    ("music.youtube.com", "search, pages, radio"),
    ("www.youtube.com", "stream addresses"),
    ("redirector.googlevideo.com", "audio"),
    ("i.ytimg.com", "covers"),
    ("lh3.googleusercontent.com", "covers and artist photos"),
    ("lrclib.net", "lyrics"),
    ("api.deezer.com", "recommendations"),
    ("musicbrainz.org", "recommendations"),
    ("labs.api.listenbrainz.org", "recommendations"),
];

type Failure = (Code, String);

#[derive(Debug)]
pub struct Step {
    pub name: &'static str,
    pub ms: u128,
    pub result: Result<String, Failure>,
}

async fn timed<T>(step: impl Future<Output = Result<T, Failure>>) -> (u128, Result<T, Failure>) {
    let start = Instant::now();
    let result =
        tokio::time::timeout(STEP, step).await.unwrap_or_else(|_| Err((Code::NetTimeout, format!("no answer in {} s", STEP.as_secs()))));
    (start.elapsed().as_millis(), result)
}

fn failure(e: &io::Error, otherwise: Code) -> Failure {
    (cause_code(e).unwrap_or(otherwise), e.to_string())
}

async fn connect(addrs: &[SocketAddr]) -> Result<TcpStream, Failure> {
    let mut last = (Code::NetOffline, "no address".to_string());
    for addr in addrs {
        match TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(e) => last = failure(&e, Code::NetOffline),
        }
    }
    Err(last)
}

pub async fn head(mut stream: impl AsyncRead + AsyncWrite + Unpin, host: &str) -> Result<String, Failure> {
    let request = format!("HEAD / HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {}\r\nAccept: */*\r\nConnection: close\r\n\r\n", product::NAME);
    stream.write_all(request.as_bytes()).await.map_err(|e| failure(&e, Code::NetOffline))?;
    let mut head = Vec::new();
    let mut buf = [0u8; 512];
    while !head.windows(2).any(|w| w == b"\r\n") && head.len() < 4096 {
        let n = stream.read(&mut buf).await.map_err(|e| failure(&e, Code::NetOffline))?;
        if n == 0 {
            return Err((Code::NetConnect, "the connection closed before an answer".into()));
        }
        head.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&head);
    let line = text.lines().next().unwrap_or_default().trim().to_string();
    let status: u16 =
        line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| (Code::YtParse, format!("not HTTP: {line}")))?;
    match status {
        403 | 451 => Err((Code::NetBlocked, line)),
        429 => Err((Code::NetLimited, line)),
        500..=599 => Err((Code::NetServer, line)),
        _ => Ok(line),
    }
}

pub async fn probe(host: &str, addrs: impl Future<Output = io::Result<Vec<SocketAddr>>>, tls: &TlsConnector) -> Vec<Step> {
    let mut steps = Vec::new();
    let (ms, found) = timed(async {
        let addrs = addrs.await.map_err(|e| (Code::NetDns, e.to_string()))?;
        if addrs.is_empty() { Err((Code::NetDns, "no address".to_string())) } else { Ok(addrs) }
    })
    .await;
    let shown = found.as_ref().map(|a| format!("{} ({} in all)", a[0].ip(), a.len())).map_err(Clone::clone);
    steps.push(Step { name: "dns", ms, result: shown });
    let Ok(addrs) = found else { return steps };

    let (ms, tcp) = timed(connect(&addrs)).await;
    steps.push(Step {
        name: "tcp",
        ms,
        result: tcp.as_ref().map(|s| s.peer_addr().map_or_else(|_| String::new(), |a| a.to_string())).map_err(Clone::clone),
    });
    let Ok(tcp) = tcp else { return steps };

    let name = ServerName::try_from(host.to_string()).map_err(|e| (Code::ApiArgs, e.to_string()));
    let (ms, secured) = timed(async { tls.connect(name?, tcp).await.map_err(|e| failure(&e, Code::NetTls)) }).await;
    let version = secured
        .as_ref()
        .map(|s| s.get_ref().1.protocol_version().map_or_else(String::new, |v| format!("{v:?}").replace("TLSv1_", "TLS 1.")));
    steps.push(Step { name: "tls", ms, result: version.map_err(Clone::clone) });
    let Ok(secured) = secured else { return steps };

    let (ms, answer) = timed(head(secured, host)).await;
    steps.push(Step { name: "http", ms, result: answer });
    steps
}

pub fn render(results: &[(&str, &str, Vec<Step>)]) -> String {
    let mut out = vec!["Direct connections, without a proxy; each host gets one HEAD request and nothing else.".to_string()];
    let mut failed = Vec::new();
    for (host, why, steps) in results {
        out.push(format!("\n{host}  ({why})"));
        for step in steps {
            let (status, detail) = match &step.result {
                Ok(detail) => ("ok".to_string(), detail.clone()),
                Err((code, detail)) => (code.to_string(), detail.clone()),
            };
            out.push(format!("  {:<5} {:<12} {:>5} ms  {detail}", step.name, status, step.ms));
        }
        if steps.iter().any(|s| s.result.is_err()) {
            failed.push(*host);
        }
    }
    out.push(String::new());
    out.push(if failed.is_empty() {
        format!("All {} hosts answered.", results.len())
    } else {
        format!("{} of {} hosts failed: {}", failed.len(), results.len(), failed.join(", "))
    });
    out.join("\n")
}

pub async fn report() -> String {
    let tls = match ClientConfig::with_platform_verifier() {
        Ok(config) => TlsConnector::from(Arc::new(config)),
        Err(e) => return format!("cannot load the system's certificates: {e}"),
    };
    let checks = HOSTS.iter().map(|&(host, why)| {
        let tls = tls.clone();
        async move {
            let addrs = async move { tokio::net::lookup_host((host, 443)).await.map(Iterator::collect) };
            (host, why, probe(host, addrs, &tls).await)
        }
    });
    render(&join_all(checks).await)
}
