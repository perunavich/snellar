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

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-raise-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("snellar.sock")
    }

    #[test]
    fn a_live_instance_is_asked_to_raise_its_window() {
        let path = scratch("live");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut got = [0; 16];
            std::io::Read::read_exact(&mut stream, &mut got).unwrap();
            std::io::Write::write_all(&mut stream, b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nnull").unwrap();
            String::from_utf8_lossy(&got).into_owned()
        });
        assert!(raise_running(&path, Duration::from_secs(2)));
        assert_eq!(server.join().unwrap(), "POST /call/raise");
    }

    #[test]
    fn a_dead_socket_or_no_socket_does_not_block_the_start() {
        let path = scratch("dead");
        assert!(!raise_running(&path, Duration::from_secs(2)), "no socket at all");
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists(), "the file of a dead instance stays behind");
        assert!(!raise_running(&path, Duration::from_secs(2)), "a stale socket file");
    }

    #[test]
    fn an_instance_that_never_answers_does_not_block_the_start() {
        let path = scratch("silent");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let started = std::time::Instant::now();
        assert!(!raise_running(&path, Duration::from_millis(50)));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn says_when_the_app_is_not_running() {
        let err = call_at(&std::env::temp_dir().join("snellar-no-such.sock"), "now", &Value::Null).await.unwrap_err();
        assert!(format!("{err:#}").contains("snellar is not running"));
    }
}
