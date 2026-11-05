use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
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
use snellar_youtube::net::Route;

use crate::audio::{AudioOut, Event, Launch, Opened};

const OBSERVED: [&str; 9] =
    ["time-pos", "duration", "pause", "idle-active", "paused-for-cache", "seeking", "volume", "audio-codec-name", "audio-device-list"];

static SPAWNS: AtomicU64 = AtomicU64::new(0);

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

pub struct Mpv {
    _child: Child,
    socket: PathBuf,
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    pending: Pending,
    ids: AtomicU64,
    route: Route,
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

    fn load<'a>(&'a self, url: &'a str, start: f64, gain: Option<f64>) -> BoxFuture<'a, Result<()>> {
        Box::pin(Mpv::load(self, url, "replace", start, gain))
    }

    fn append<'a>(&'a self, url: &'a str, gain: Option<f64>) -> BoxFuture<'a, Result<()>> {
        Box::pin(Mpv::load(self, url, "append", 0.0, gain))
    }

    fn clear_next(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { self.command(json!(["playlist-clear"])).await.map(drop) })
    }

    fn seek(&self, position: f64) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { self.command(json!(["seek", position, "absolute"])).await.map(drop) })
    }

    fn stop(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { self.command(json!(["stop"])).await.map(drop) })
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
        if let Some(ytdlp) = dirs::home_dir().map(|h| h.join(".local/bin/yt-dlp")).filter(|p| p.exists()) {
            cmd.arg(format!("--script-opts=ytdl_hook-ytdl_path={}", ytdlp.display()));
        }
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
        tokio::spawn(read_loop(BufReader::new(read), pending.clone(), events));

        let mpv =
            Mpv { _child: child, socket, writer: tokio::sync::Mutex::new(write), pending, ids: AtomicU64::new(1), route: Route::default() };
        for (i, name) in OBSERVED.iter().enumerate() {
            mpv.command(json!(["observe_property", i + 1, name])).await?;
        }
        Ok((mpv, rx))
    }

    pub async fn command(&self, args: Value) -> Result<Value> {
        self.request(args).await.context(Code::AudioFailed)
    }

    async fn request(&self, args: Value) -> Result<Value> {
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let line = format!("{}\n", json!({ "command": args, "request_id": id }));
        if let Err(e) = self.writer.lock().await.write_all(line.as_bytes()).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e).context("mpv is gone");
        }
        match tokio::time::timeout(Duration::from_secs(10), rx).await {
            Ok(Ok(reply)) => reply.map_err(|e| anyhow!("mpv: {e}")),
            Ok(Err(_)) => bail!("mpv exited"),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                bail!("mpv did not answer")
            }
        }
    }

    pub async fn set(&self, name: &str, value: impl Into<Value>) -> Result<()> {
        self.command(json!(["set_property", name, value.into()])).await.map(drop)
    }

    pub async fn load(&self, url: &str, mode: &str, start: f64, gain: Option<f64>) -> Result<()> {
        let options = load_options(url, start, gain, self.route.proxy().as_deref());
        self.command(json!({ "name": "loadfile", "url": url, "flags": mode, "options": options })).await.map(drop)
    }
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

async fn read_loop(read: BufReader<tokio::net::unix::OwnedReadHalf>, pending: Pending, events: mpsc::UnboundedSender<Event>) {
    let mut lines = read.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(id) = msg["request_id"].as_u64() {
            if let Some(tx) = pending.lock().unwrap().remove(&id) {
                let reply = match msg["error"].as_str() {
                    Some("success") => Ok(msg["data"].clone()),
                    other => Err(other.unwrap_or("unknown error").to_string()),
                };
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
    pending.lock().unwrap().clear();
    events.send(Event::Died).ok();
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use snellar_youtube::proxy::Proxy;
    use snellar_youtube::stream::Resolver;

    #[test]
    fn a_known_loudness_goes_with_the_file_as_its_gain() {
        assert_eq!(load_options("http://127.0.0.1:9/s", 0.0, Some(-0.97999954), None), "ytdl=no,replaygain-fallback=-0.98");
        assert_eq!(load_options("/music/a.opus", 3.0, Some(0.0), None), "ytdl=no,start=3,replaygain-fallback=0.00");
    }

    async fn peak_after(gain: Option<f64>) -> i16 {
        let wav = std::env::temp_dir().join(format!("snellar-gain-{}-{}.wav", std::process::id(), gain.unwrap_or(1.0)));
        let (mpv, mut rx) =
            Mpv::spawn_with(&["--ao=pcm", "--audio-format=s16", &format!("--ao-pcm-file={}", wav.display())]).await.unwrap();
        mpv.load("av://lavfi:sine=frequency=440:duration=0.5", "replace", 0.0, gain).await.unwrap();
        while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            if matches!(event, Event::EndFile { .. }) {
                break;
            }
        }
        drop(mpv);
        let bytes = tokio::fs::read(&wav).await.unwrap();
        std::fs::remove_file(&wav).ok();
        let data = bytes.windows(4).position(|w| w == b"data").unwrap() + 8;
        bytes[data..].as_chunks::<2>().0.iter().map(|s| i16::from_le_bytes(*s).saturating_abs()).max().unwrap()
    }

    #[tokio::test]
    async fn mpv_applies_the_gain_given_with_the_file() {
        let plain = peak_after(None).await;
        let lowered = peak_after(Some(-6.0)).await;
        let zero = peak_after(Some(0.0)).await;
        assert!(plain > 1000, "the sine is audible: {plain}");
        let ratio = f64::from(lowered) / f64::from(plain);
        assert!((ratio - 0.501).abs() < 0.02, "-6 dB is half the amplitude: {lowered} of {plain}");
        assert_eq!(zero, plain, "a zero gain leaves the file as it is");
    }

    #[tokio::test]
    async fn mpv_lists_devices_by_name_and_description_and_takes_one() {
        let (mpv, _rx) = Mpv::spawn_with(&["--ao=null"]).await.unwrap();
        let list = AudioOut::get(&mpv, "audio-device-list").await.unwrap();
        let first = &list.as_array().unwrap()[0];
        assert!(first["name"].is_string() && first["description"].is_string(), "{list}");
        AudioOut::set(&mpv, "audio-device", json!("auto")).await.unwrap();
        assert_eq!(AudioOut::get(&mpv, "audio-device").await.unwrap(), "auto");
    }

    #[test]
    fn only_the_ytdl_fallback_gets_the_proxy() {
        assert_eq!(load_options("http://127.0.0.1:9/s", 12.5, None, Some("http://p:3128")), "ytdl=no,start=12.5");
        assert_eq!(load_options("ytdl://dQw4w9WgXcQ", 0.0, None, None), "");
        assert_eq!(
            load_options("ytdl://dQw4w9WgXcQ", 0.0, None, Some("http://p:3128")),
            "http-proxy=%13%http://p:3128,ytdl-raw-options=%23%proxy=%13%http://p:3128"
        );
    }

    const IDS: [&str; 5] = ["dQw4w9WgXcQ", "9bZkp7q19f0", "kJQP7kiw5Fk", "JGwWNGJdvx8", "fJ9rUzIMcZQ"];

    async fn first_audio(rx: &mut mpsc::UnboundedReceiver<Event>, t0: Instant) -> (Option<Duration>, Option<Duration>) {
        let (mut opened, mut audio) = (None, None);
        while audio.is_none() {
            let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(20), rx.recv()).await else { break };
            match event {
                Event::Property(name, value) if name == "duration" && opened.is_none() && value.is_number() => opened = Some(t0.elapsed()),
                Event::Property(name, value) if name == "time-pos" && value.as_f64().is_some_and(|t| t > 0.01 && t < 2.0) => {
                    audio = Some(t0.elapsed())
                }
                Event::EndFile { reason, .. } if reason == "error" => break,
                _ => {}
            }
        }
        (opened, audio)
    }

    fn ms(d: Option<Duration>) -> String {
        d.map_or("-".into(), |d| format!("{:>5}", d.as_millis()))
    }

    #[tokio::test]
    #[ignore]
    async fn live_start_latency() {
        let variants: Vec<(&str, Vec<&str>)> =
            vec![("default", vec![]), ("fast", vec!["--cache-pause-initial=no", "--demuxer-readahead-secs=20", "--audio-buffer=0.1"])];
        let only = std::env::var("VARIANT").ok();
        for (name, extra) in variants.iter().filter(|(n, _)| only.as_deref().is_none_or(|o| o == *n)) {
            let dir = std::env::temp_dir().join(format!("snellar-latency-{name}"));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            let resolver = Arc::new(Resolver::new(dir));
            let proxy = Proxy::serve(snellar_youtube::proxy::bind().unwrap(), resolver.clone(), std::env::temp_dir()).unwrap();
            let (mpv, mut rx) = Mpv::spawn_with(extra).await.unwrap();
            mpv.set("volume", 0).await.unwrap();
            println!("variant {name}: id  resolve | load->open load->audio | prefetched open audio | preopened next->audio");
            for (i, id) in IDS.iter().enumerate() {
                if std::env::var_os("WARM").is_some() {
                    proxy.warm(id);
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                }
                let t = Instant::now();
                resolver.resolve_with_ua(id).await.unwrap();
                let url = proxy.url(id);
                let resolve = t.elapsed();
                let t = Instant::now();
                mpv.load(&url, "replace", 0.0, None).await.unwrap();
                let (cold_open, cold_audio) = first_audio(&mut rx, t).await;

                if cold_audio.is_none() {
                    let direct_failed = t.elapsed();
                    let t = Instant::now();
                    mpv.load(&format!("ytdl://{id}"), "replace", 0.0, None).await.unwrap();
                    let (_, audio) = first_audio(&mut rx, t).await;
                    println!("  {id} direct url failed after {} ms, ytdl:// fallback audio in {} ms", direct_failed.as_millis(), ms(audio));
                    continue;
                }
                mpv.command(json!(["stop"])).await.unwrap();
                tokio::time::sleep(Duration::from_millis(300)).await;
                let t = Instant::now();
                mpv.load(&url, "replace", 0.0, None).await.unwrap();
                let (warm_open, warm_audio) = first_audio(&mut rx, t).await;

                let next = IDS[(i + 1) % IDS.len()];
                resolver.resolve_with_ua(next).await.unwrap();
                let nurl = proxy.url(next);
                mpv.load(&nurl, "append", 0.0, None).await.unwrap();
                tokio::time::sleep(Duration::from_secs(4)).await;
                while rx.try_recv().is_ok() {}
                let t = Instant::now();
                mpv.command(json!(["playlist-next"])).await.unwrap();
                let (_, pre_audio) = first_audio(&mut rx, t).await;
                mpv.command(json!(["stop"])).await.unwrap();
                tokio::time::sleep(Duration::from_millis(300)).await;
                while rx.try_recv().is_ok() {}
                println!(
                    "  {id} {:>5} | {} {} | {} {} | {}",
                    resolve.as_millis(),
                    ms(cold_open),
                    ms(cold_audio),
                    ms(warm_open),
                    ms(warm_audio),
                    ms(pre_audio)
                );
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_seek_through_proxy() {
        let dir = std::env::temp_dir().join(format!("snellar-seek-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let resolver = Arc::new(Resolver::new(dir.clone()));
        let proxy = Proxy::serve(snellar_youtube::proxy::bind().unwrap(), resolver.clone(), dir).unwrap();
        let (mpv, mut rx) = Mpv::spawn().await.unwrap();
        mpv.set("volume", 0).await.unwrap();
        for id in ["3YgtjHZyCIQ", "H4RELGc9su8", "sx5oqwa-Dps", "m9SMT5ipbxk", "4AgDYTuZWOw"] {
            resolver.resolve_with_ua(id).await.unwrap();
            mpv.load(&proxy.url(id), "replace", 0.0, None).await.unwrap();
            let (mut duration, mut sought, mut reached, mut error) = (0.0, false, None, false);
            let start = std::time::Instant::now();
            while reached.is_none() && !error && start.elapsed() < Duration::from_secs(25) {
                let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await else { break };
                match event {
                    Event::Property(name, value) if name == "duration" => duration = value.as_f64().unwrap_or(duration),
                    Event::Property(name, value) if name == "time-pos" => {
                        let pos = value.as_f64().unwrap_or(0.0);
                        if !sought && pos > 0.05 && duration > 0.0 {
                            sought = true;
                            mpv.command(json!(["seek", "80", "absolute-percent"])).await.unwrap();
                        } else if sought && pos > duration * 0.8 + 1.5 {
                            reached = Some(start.elapsed());
                        }
                    }
                    Event::EndFile { reason, .. } if reason == "error" => error = true,
                    _ => {}
                }
            }
            println!("{id} duration {duration:.0}s seek-to-80% playing: {reached:?} error={error}");
            mpv.command(json!(["stop"])).await.unwrap();
        }
    }
}
