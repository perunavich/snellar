use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

use snellar_core::error::Code;
use snellar_core::product;
use snellar_core::source::BoxFuture;
use snellar_core::sync::Lock;
use snellar_youtube::net::Route;

use crate::audio::{AudioOut, Event, Launch, Opened};

const OBSERVED: [&str; 9] =
    ["time-pos", "duration", "pause", "idle-active", "paused-for-cache", "seeking", "volume", "audio-codec-name", "audio-device-list"];

static SPAWNS: AtomicU64 = AtomicU64::new(0);

/// Requests waiting for mpv's answer, each with what is said in order with mpv's events once it succeeded.
type Pending = Arc<Mutex<HashMap<u64, (oneshot::Sender<Result<Value, String>>, Option<Event>)>>>;

pub struct Mpv {
    _child: Child,
    socket: PathBuf,
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    pending: Pending,
    ids: AtomicU64,
    route: Route,
    gone: Arc<AtomicBool>,
}

pub fn launch(route: Route) -> Launch {
    Arc::new(move || {
        let route = route.clone();
        Box::pin(async move {
            let (mut mpv, events) = Mpv::spawn().await.context(Code::AudioFailed)?;
            mpv.route = route;
            Ok::<Opened, anyhow::Error>((Arc::new(mpv), events))
        })
    })
}

impl AudioOut for Mpv {
    fn set<'a>(&'a self, name: &'a str, value: Value) -> BoxFuture<'a, Result<()>> {
        Box::pin(Mpv::set(self, name, value))
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Value>> {
        Box::pin(self.command(json!(["get_property", name])))
    }

    fn af_command<'a>(&'a self, label: &'a str, target: &'a str, option: &'a str, value: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { self.command(json!(["af-command", label, option, value, target])).await.map(drop) })
    }

    fn load<'a>(&'a self, url: &'a str, start: f64, gain: Option<f64>, mark: u64) -> BoxFuture<'a, Result<()>> {
        Box::pin(Mpv::load(self, url, "replace", start, gain, Some(Event::Loaded(mark))))
    }

    fn append<'a>(&'a self, url: &'a str, gain: Option<f64>, mark: u64) -> BoxFuture<'a, Result<()>> {
        Box::pin(Mpv::load(self, url, "append", 0.0, gain, Some(Event::Appended(mark))))
    }

    fn clear_next(&self, mark: u64) -> BoxFuture<'_, Result<()>> {
        Box::pin(
            async move { self.request(json!(["playlist-clear"]), Some(Event::Cleared(mark))).await.context(Code::AudioFailed).map(drop) },
        )
    }

    fn seek(&self, position: f64, mark: u64) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.request(json!(["seek", position, "absolute"]), Some(Event::Seeked(mark))).await.context(Code::AudioFailed).map(drop)
        })
    }

    fn stop(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { self.command(json!(["stop"])).await.map(drop) })
    }

    fn gone(&self) -> bool {
        self.gone.load(Ordering::SeqCst)
    }
}

impl Mpv {
    pub async fn spawn() -> Result<(Mpv, mpsc::UnboundedReceiver<Event>)> {
        Mpv::spawn_with(&[]).await
    }

    async fn spawn_with(extra: &[&str]) -> Result<(Mpv, mpsc::UnboundedReceiver<Event>)> {
        let dir = product::runtime_dir();
        let socket =
            dir.join(format!("{}{}-{}.sock", product::MPV_SOCKET_PREFIX, std::process::id(), SPAWNS.fetch_add(1, Ordering::Relaxed)));
        remove_stale_sockets(&dir);
        std::fs::remove_file(&socket).ok();

        let mut cmd = Command::new(product::mpv_bin());
        cmd.env_remove("LD_LIBRARY_PATH");
        cmd.args([
            "--idle=yes",
            "--no-video",
            "--no-terminal",
            "--config=no",
            "--audio-display=no",
            "--volume-max=150",
            "--gapless-audio=weak",
            "--cache=yes",
            "--ytdl-format=bestaudio",
        ])
        .args(extra)
        .arg(format!("--input-ipc-server={}", socket.display()));
        cmd.arg(format!("--script-opts=ytdl_hook-ytdl_path={}", ytdl_paths()));
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let mut child =
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).spawn().context("cannot start mpv")?;

        let stream = connect(&socket, &mut child).await?;
        let (read, write) = stream.into_split();
        let pending = Pending::default();
        let (events, rx) = mpsc::unbounded_channel();
        let gone = Arc::new(AtomicBool::new(false));
        tokio::spawn(read_loop(BufReader::new(read), pending.clone(), events, gone.clone()));

        let mpv = Mpv {
            _child: child,
            socket,
            writer: tokio::sync::Mutex::new(write),
            pending,
            ids: AtomicU64::new(1),
            route: Route::default(),
            gone,
        };
        for (i, name) in OBSERVED.iter().enumerate() {
            mpv.command(json!(["observe_property", i + 1, name])).await?;
        }
        Ok((mpv, rx))
    }

    pub async fn command(&self, args: Value) -> Result<Value> {
        self.request(args, None).await.context(Code::AudioFailed)
    }

    async fn request(&self, args: Value, mark: Option<Event>) -> Result<Value> {
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.locked().insert(id, (tx, mark));
        let line = format!("{}\n", json!({ "command": args, "request_id": id }));
        if let Err(e) = self.writer.lock().await.write_all(line.as_bytes()).await {
            self.pending.locked().remove(&id);
            self.gone.store(true, Ordering::SeqCst);
            return Err(e).context("mpv is gone");
        }
        match tokio::time::timeout(Duration::from_secs(10), rx).await {
            Ok(Ok(reply)) => reply.map_err(|e| anyhow!("mpv: {e}")),
            Ok(Err(_)) => bail!("mpv exited"),
            Err(_) => {
                self.pending.locked().remove(&id);
                bail!("mpv did not answer")
            }
        }
    }

    pub async fn set(&self, name: &str, value: impl Into<Value>) -> Result<()> {
        self.command(json!(["set_property", name, value.into()])).await.map(drop)
    }

    pub async fn load(&self, url: &str, mode: &str, start: f64, gain: Option<f64>, mark: Option<Event>) -> Result<()> {
        let options = load_options(url, start, gain, self.route.proxy().as_deref());
        let loadfile = json!({ "name": "loadfile", "url": url, "flags": mode, "options": options });
        self.request(loadfile, mark).await.context(Code::AudioFailed).map(drop)
    }
}

fn ytdl_paths() -> String {
    let own = product::own_ytdlp().unwrap_or_else(|| product::YTDLP.into());
    format!("{}:{}", product::managed_ytdlp().display(), own.display())
}

fn quoted(value: &str) -> String {
    format!("%{}%{value}", value.len())
}

fn load_options(url: &str, start: f64, gain: Option<f64>, proxy: Option<&str>) -> String {
    let mut options = Vec::new();
    if url.starts_with("ytdl://") {
        if let Some(proxy) = proxy {
            options.push(format!("http-proxy={}", quoted(proxy)));
            options.push(format!("ytdl-raw-options={}", quoted(&format!("proxy={}", quoted(proxy)))));
        }
    } else {
        options.push("ytdl=no".to_string());
    }
    if start > 0.0 {
        options.push(format!("start={start}"));
    }
    if let Some(gain) = gain {
        options.push(format!("replaygain-fallback={gain:.2}"));
    }
    options.join(",")
}

impl Drop for Mpv {
    fn drop(&mut self) {
        std::fs::remove_file(&self.socket).ok();
    }
}

fn remove_stale_sockets(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let pid = name
            .to_str()
            .and_then(|n| n.strip_prefix(product::MPV_SOCKET_PREFIX)?.strip_suffix(".sock")?.split_once('-')?.0.parse::<u32>().ok());
        if pid.is_some_and(|p| !Path::new(&format!("/proc/{p}")).exists()) {
            std::fs::remove_file(entry.path()).ok();
        }
    }
}

async fn connect(socket: &PathBuf, child: &mut Child) -> Result<UnixStream> {
    for _ in 0..100 {
        if let Ok(stream) = UnixStream::connect(socket).await {
            return Ok(stream);
        }
        if let Some(status) = child.try_wait()? {
            bail!("mpv exited at startup ({status})");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("mpv ipc socket did not appear")
}

async fn read_loop(
    read: BufReader<tokio::net::unix::OwnedReadHalf>,
    pending: Pending,
    events: mpsc::UnboundedSender<Event>,
    gone: Arc<AtomicBool>,
) {
    let mut lines = read.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(id) = msg["request_id"].as_u64() {
            if let Some((tx, mark)) = pending.locked().remove(&id) {
                let reply = match msg["error"].as_str() {
                    Some("success") => Ok(msg["data"].clone()),
                    other => Err(other.unwrap_or("unknown error").to_string()),
                };
                if let (Ok(_), Some(mark)) = (&reply, mark) {
                    events.send(mark).ok();
                }
                tx.send(reply).ok();
            }
            continue;
        }
        let event = match msg["event"].as_str() {
            Some("property-change") => Event::Property(msg["name"].as_str().unwrap_or_default().into(), msg["data"].clone()),
            Some("end-file") => Event::EndFile {
                reason: msg["reason"].as_str().unwrap_or_default().into(),
                error: msg["file_error"].as_str().map(Into::into),
            },
            _ => continue,
        };
        if events.send(event).is_err() {
            return;
        }
    }
    gone.store(true, Ordering::SeqCst);
    pending.locked().clear();
    events.send(Event::Died).ok();
}
