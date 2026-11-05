use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use snellar_core::error::Code;
use snellar_core::journal;
use snellar_core::product;
use snellar_core::ui::UiEvent;

use crate::api::Backend;

const MAX_BODY: usize = 4 * 1024 * 1024;

pub async fn serve(backend: Arc<Backend>, events: broadcast::Sender<UiEvent>) {
    let Some(path) = product::socket_path() else { return };
    if let Err(e) = listen(&path, backend, events).await {
        journal::error("api", format!("{e:#}"));
    }
}

fn claim(path: &Path) -> Result<UnixListener> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                bail!("another instance already serves {}", path.display());
            }
            std::fs::remove_file(path)?;
        }
        Ok(_) => bail!("{} exists and is not a socket, refusing to replace it", path.display()),
        Err(_) => {}
    }
    let listener = UnixListener::bind(path).with_context(|| format!("cannot bind {}", path.display()))?;
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(listener)
}

async fn listen(path: &Path, backend: Arc<Backend>, events: broadcast::Sender<UiEvent>) -> Result<()> {
    let listener = claim(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let (backend, events) = (backend.clone(), events.subscribe());
        tokio::spawn(async move {
            let _ = handle(stream, &backend, events).await;
        });
    }
}

fn event_json(event: &UiEvent) -> Option<(&'static str, Value)> {
    Some(match event {
        UiEvent::Reply { .. } | UiEvent::Capture(_) | UiEvent::TrayReady(_) | UiEvent::TrayAction(_) => return None,
        UiEvent::State(s) => (
            "state",
            json!({ "playing": s.playing, "loading": s.loading, "position": s.position, "duration": s.duration, "volume": s.volume }),
        ),
        UiEvent::Position(p) => ("position", json!(p)),
        UiEvent::Current(t) => ("current", json!(t)),
        UiEvent::Queue { tracks, cursor } => ("queue", json!({ "tracks": tracks, "cursor": cursor })),
        UiEvent::Liked(v) => ("liked", json!(v)),
        UiEvent::History => ("history", Value::Null),
        UiEvent::Tools => ("tools", Value::Null),
        UiEvent::Modes { shuffle, repeat } => ("modes", json!({ "shuffle": shuffle, "repeat": repeat })),
        UiEvent::Toast(t) => ("toast", json!(t)),
        UiEvent::PlayFailure(f) => ("failure", json!(f)),
        UiEvent::Library(l) => ("library", json!(l)),
        UiEvent::Nudge(n) => ("nudge", serde_json::from_str(n).unwrap_or(Value::Null)),
        UiEvent::Import(p) => ("import", json!(p)),
        UiEvent::Stream(s) => ("stream", json!(s)),
        UiEvent::Desktop(d) => ("desktop", serde_json::from_str(d).unwrap_or(Value::Null)),
    })
}

async fn reply(stream: &mut UnixStream, status: &str, body: &Value) -> Result<()> {
    let body = body.to_string();
    let head =
        format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    Ok(())
}

async fn handle(stream: UnixStream, backend: &Backend, events: broadcast::Receiver<UiEvent>) -> Result<()> {
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    reader.read_line(&mut request).await?;
    let mut parts = request.split_whitespace();
    let (verb, path) = (parts.next().context("empty request")?.to_string(), parts.next().context("no path")?.to_string());
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    if length > MAX_BODY {
        return reply(
            &mut reader.into_inner(),
            "413 Payload Too Large",
            &json!({ "error": format!("request body is limited to {MAX_BODY} bytes"), "code": Code::ApiArgs }),
        )
        .await;
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    let mut stream = reader.into_inner();

    match (verb.as_str(), path.as_str()) {
        ("GET", "/events") => stream_events(stream, backend, events).await,
        ("GET", "/methods") => call(&mut stream, backend, "methods", Value::Null).await,
        ("GET", "/now") => call(&mut stream, backend, "now", Value::Null).await,
        ("POST", p) if p.starts_with("/call/") => match parse_body(&body) {
            Ok(args) => call(&mut stream, backend, &p["/call/".len()..], args).await,
            Err(e) => reply(&mut stream, "400 Bad Request", &json!({ "error": format!("{e:#}"), "code": Code::ApiArgs })).await,
        },
        _ => reply(
            &mut stream,
            "404 Not Found",
            &json!({ "error": "no such route; use POST /call/<method>, GET /methods, GET /now or GET /events", "code": Code::ApiMethod }),
        )
        .await,
    }
}

fn parse_body(body: &[u8]) -> Result<Value> {
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(body).context("the request body is not valid JSON")
}

async fn call(stream: &mut UnixStream, backend: &Backend, method: &str, args: Value) -> Result<()> {
    if !crate::api::METHODS.iter().any(|m| m.name == method) {
        return reply(
            stream,
            "404 Not Found",
            &json!({ "error": format!("unknown method `{method}`, see GET /methods"), "code": Code::ApiMethod }),
        )
        .await;
    }
    let started = std::time::Instant::now();
    let outcome = backend.dispatch(method, &args).await;
    journal::call("ctl", method, &args, started.elapsed(), outcome.as_ref().err());
    match outcome {
        Ok(value) => reply(stream, "200 OK", &value).await,
        Err(e) => reply(stream, "400 Bad Request", &json!({ "error": format!("{e:#}"), "code": Code::of(&e) })).await,
    }
}

async fn stream_events(mut stream: UnixStream, backend: &Backend, mut events: broadcast::Receiver<UiEvent>) -> Result<()> {
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n").await?;
    let mut send = async |name: &str, data: &Value| -> Result<()> {
        stream.write_all(format!("event: {name}\ndata: {data}\n\n").as_bytes()).await?;
        Ok(())
    };
    send(
        "snapshot",
        &json!({ "now": backend.dispatch("now", &Value::Null).await?, "queue": backend.dispatch("queue", &Value::Null).await? }),
    )
    .await?;
    loop {
        match events.recv().await {
            Ok(event) => {
                if let Some((name, data)) = event_json(&event) {
                    send(name, &data).await?;
                }
            }
            Err(RecvError::Lagged(_)) => {
                send("snapshot", &json!({ "now": backend.dispatch("now", &Value::Null).await?, "queue": backend.dispatch("queue", &Value::Null).await? })).await?;
            }
            Err(RecvError::Closed) => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-server-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_regular_file_at_the_socket_path_is_left_alone() {
        let dir = dir("file");
        let path = dir.join("notes.txt");
        std::fs::write(&path, "keep me").unwrap();
        assert!(claim(&path).unwrap_err().to_string().contains("not a socket"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
    }

    #[tokio::test]
    async fn a_stale_socket_is_replaced_and_a_live_one_is_respected() {
        let dir = dir("socket");
        let path = dir.join("snellar.sock");
        let first = claim(&path).unwrap();
        assert!(claim(&path).unwrap_err().to_string().contains("another instance"));
        drop(first);
        let second = claim(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        drop(second);
    }

    #[test]
    fn bad_json_is_an_error_not_null() {
        assert_eq!(parse_body(b"").unwrap(), Value::Null);
        assert_eq!(parse_body(br#"{"a":1}"#).unwrap()["a"], 1);
        assert!(parse_body(b"{oops").is_err());
    }
}
