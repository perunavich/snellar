use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::UnixStream;

use snellar_core::product::{self, socket_path};

pub async fn call(method: &str, args: &Value) -> Result<Value> {
    let path = socket_path().context("XDG_RUNTIME_DIR is not set")?;
    call_at(&path, method, args).await
}

pub async fn call_at(path: &Path, method: &str, args: &Value) -> Result<Value> {
    let mut stream = UnixStream::connect(path)
        .await
        .map_err(|_| anyhow!("{} is not running (nothing listens on {}); start the app first", product::ID, path.display()))?;
    let body = args.to_string();
    let request = format!(
        "POST /call/{method} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        product::ID,
        body.len()
    );
    stream.write_all(request.as_bytes()).await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text.split_once("\r\n\r\n").context("malformed response")?;
    let value: Value = serde_json::from_str(body).context("response was not JSON")?;
    if head.starts_with("HTTP/1.1 200") {
        return Ok(value);
    }
    bail!("{}", value["error"].as_str().unwrap_or("request failed"))
}

pub fn raise_running(path: &Path, patience: Duration) -> bool {
    let Ok(mut stream) = std::os::unix::net::UnixStream::connect(path) else { return false };
    if stream.set_read_timeout(Some(patience)).is_err() || stream.set_write_timeout(Some(patience)).is_err() {
        return false;
    }
    let request = format!("POST /call/raise HTTP/1.1\r\nHost: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", product::ID);
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0; 8];
    stream.read_exact(&mut head).is_ok() && &head == b"HTTP/1.1"
}
