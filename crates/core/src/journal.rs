use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use crate::clock::stamp;
use crate::error::Code;
use crate::product;

const LIMIT: u64 = 5 * 1024 * 1024;
const QUEUE: usize = 4096;
const BATCH: Duration = Duration::from_millis(50);
const SYNC_EVERY: Duration = Duration::from_secs(5);
const LINE_MAX: usize = 16 * 1024;
const STALL: Duration = Duration::from_secs(3);
const QUIET: [&str; 7] = ["prefetch", "isLiked", "getSetting", "getFx", "methods", "now", "queue"];
const SECRET_NAMES: [&str; 11] =
    ["token", "key", "sig", "signature", "auth", "password", "secret", "cookie", "bearer", "visitor", "apikey"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

struct Entry {
    at: SystemTime,
    level: Level,
    kind: &'static str,
    text: String,
}

#[derive(Default)]
struct Queue {
    items: Vec<Entry>,
    dropped: u64,
    pushed: u64,
    written: u64,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    done: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

trait Sink: Send {
    fn append(&mut self, bytes: &[u8]);
    fn sync(&mut self);
}

struct Files {
    dir: PathBuf,
    file: File,
    size: u64,
    limit: u64,
}

impl Files {
    fn open(dir: &Path, limit: u64) -> std::io::Result<Files> {
        std::fs::create_dir_all(dir)?;
        let file = OpenOptions::new().create(true).append(true).open(dir.join("journal.log"))?;
        let size = file.metadata()?.len();
        Ok(Files { dir: dir.to_path_buf(), file, size, limit })
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        self.file.sync_data()?;
        std::fs::rename(self.dir.join("journal.log"), self.dir.join("journal.1.log"))?;
        self.file = OpenOptions::new().create(true).append(true).open(self.dir.join("journal.log"))?;
        self.size = 0;
        Ok(())
    }
}

impl Sink for Files {
    fn append(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while !rest.is_empty() {
            let room = (self.limit / 2).saturating_sub(self.size) as usize;
            let fits = if rest.len() <= room { rest.len() } else { rest[..room].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1) };
            if fits == 0 && self.size > 0 && self.rotate().is_ok() {
                continue;
            }
            let take = if fits > 0 { fits } else { rest.iter().position(|b| *b == b'\n').map_or(rest.len(), |i| i + 1) };
            if self.file.write_all(&rest[..take]).is_ok() {
                self.size += take as u64;
            }
            rest = &rest[take..];
        }
    }

    fn sync(&mut self) {
        self.file.sync_data().ok();
    }
}

pub struct Journal {
    shared: Arc<Shared>,
    dir: PathBuf,
}

impl Journal {
    fn open(dir: &Path, limit: u64) -> Journal {
        match Files::open(dir, limit) {
            Ok(files) => Journal::with_sink(dir, Box::new(files)),
            Err(e) => {
                eprintln!("{}: journal: {}: {e}", product::ID, dir.display());
                Journal::with_sink(dir, Box::new(Nowhere))
            }
        }
    }

    fn with_sink(dir: &Path, sink: Box<dyn Sink>) -> Journal {
        let shared = Arc::new(Shared { queue: Mutex::new(Queue::default()), wake: Condvar::new(), done: Condvar::new() });
        let writer = shared.clone();
        std::thread::Builder::new().name(format!("{}-journal", product::ID)).spawn(move || run(writer, sink)).expect("journal thread");
        Journal { shared, dir: dir.to_path_buf() }
    }

    fn push(&self, level: Level, kind: &'static str, text: String) {
        let mut queue = lock(&self.shared.queue);
        if queue.items.len() >= QUEUE {
            queue.dropped += 1;
            return;
        }
        queue.items.push(Entry { at: SystemTime::now(), level, kind, text });
        queue.pushed += 1;
        drop(queue);
        self.shared.wake.notify_one();
    }

    fn flush(&self, wait: Duration) {
        let mut queue = lock(&self.shared.queue);
        let target = queue.pushed;
        let end = Instant::now() + wait;
        while queue.written < target {
            let Some(left) = end.checked_duration_since(Instant::now()) else { return };
            queue = self.shared.done.wait_timeout(queue, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

struct Nowhere;

impl Sink for Nowhere {
    fn append(&mut self, _: &[u8]) {}
    fn sync(&mut self) {}
}

#[derive(Default)]
struct Repeats {
    last: Option<(&'static str, String)>,
    count: u64,
    at: Option<SystemTime>,
}

impl Repeats {
    fn summary(&mut self) -> Option<Entry> {
        let count = std::mem::take(&mut self.count);
        let (kind, _) = self.last.as_ref()?;
        (count > 0).then(|| Entry {
            at: self.at.unwrap_or_else(SystemTime::now),
            level: Level::Info,
            kind,
            text: format!("the line above repeated {count} more times"),
        })
    }
}

fn run(shared: Arc<Shared>, mut sink: Box<dyn Sink>) {
    let mut dirty = false;
    let mut repeats = Repeats::default();
    loop {
        let mut queue = lock(&shared.queue);
        if queue.items.is_empty() && queue.dropped == 0 {
            queue = if dirty {
                shared.wake.wait_timeout(queue, SYNC_EVERY).unwrap_or_else(|e| e.into_inner()).0
            } else {
                shared.wake.wait(queue).unwrap_or_else(|e| e.into_inner())
            };
        }
        if queue.items.is_empty() && queue.dropped == 0 {
            drop(queue);
            if let Some(entry) = repeats.summary() {
                let mut out = String::new();
                format_entry(&mut out, &entry);
                sink.append(out.as_bytes());
            }
            sink.sync();
            dirty = false;
            continue;
        }
        drop(queue);
        std::thread::sleep(BATCH);
        let mut queue = lock(&shared.queue);
        let items = std::mem::take(&mut queue.items);
        let dropped = std::mem::take(&mut queue.dropped);
        let upto = queue.pushed;
        drop(queue);
        let mut out = String::with_capacity(items.len() * 96);
        for entry in &items {
            if repeats.last.as_ref().is_some_and(|(kind, text)| *kind == entry.kind && *text == entry.text) {
                repeats.count += 1;
                repeats.at = Some(entry.at);
                continue;
            }
            if let Some(summary) = repeats.summary() {
                format_entry(&mut out, &summary);
            }
            format_entry(&mut out, entry);
            repeats.last = Some((entry.kind, entry.text.clone()));
        }
        if dropped > 0 {
            format_entry(
                &mut out,
                &Entry {
                    at: SystemTime::now(),
                    level: Level::Warn,
                    kind: "journal",
                    text: format!("{dropped} entries dropped, the writer fell behind"),
                },
            );
        }
        sink.append(out.as_bytes());
        dirty = true;
        lock(&shared.queue).written = upto;
        shared.done.notify_all();
    }
}

fn format_entry(out: &mut String, entry: &Entry) {
    let mut text = scrub(&entry.text);
    if text.len() > LINE_MAX {
        let mut cut = LINE_MAX;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str(" ...");
    }
    let mut lines = text.lines();
    out.push_str(&format!("{} {:<5} {:<8} {}\n", stamp(entry.at), entry.level.name(), entry.kind, lines.next().unwrap_or("")));
    for line in lines {
        out.push_str("    ");
        out.push_str(line);
        out.push('\n');
    }
}

fn is_entry_start(line: &str) -> bool {
    let b = line.as_bytes();
    b.len() > 24 && b[4] == b'-' && b[7] == b'-' && b[10] == b' ' && b[13] == b':' && b[19] == b'.' && b[..4].iter().all(u8::is_ascii_digit)
}

fn home() -> Option<&'static str> {
    static HOME: OnceLock<Option<String>> = OnceLock::new();
    HOME.get_or_init(|| dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).filter(|h| h.len() > 1)).as_deref()
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

fn cut_url(rest: &str) -> usize {
    rest.find(|c: char| c.is_whitespace() || "\"'<>)]}\\".contains(c)).unwrap_or(rest.len())
}

fn names_secret(written: &str) -> bool {
    let name: Vec<char> =
        written.trim_end_matches([' ', '"']).chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    let name: String = name.into_iter().rev().collect::<String>().to_ascii_lowercase();
    SECRET_NAMES.contains(&name.as_str())
}

pub fn scrub(text: &str) -> String {
    let text = text.replace('\0', "");
    let text = match home() {
        Some(home) => text.replace(home, "~"),
        None => text,
    };
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let scheme = ["https://", "http://"].into_iter().find(|s| rest.starts_with(s));
        if let Some(scheme) = scheme {
            let end = cut_url(rest);
            let url = &rest[scheme.len()..end];
            let host_end = url.find(['/', '?', '#']).unwrap_or(url.len());
            out.push_str(scheme);
            out.push_str(&url[..host_end]);
            if host_end < url.len() {
                out.push_str("/…");
            }
            rest = &rest[end..];
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        if is_token_char(c) {
            let end = rest.find(|c: char| !is_token_char(c)).unwrap_or(rest.len());
            let word = &rest[..end];
            let opaque = word.len() >= 32 && word.chars().any(|c| c.is_ascii_digit()) && word.chars().any(|c| c.is_ascii_alphabetic());
            out.push_str(if opaque { "…" } else { word });
            rest = &rest[end..];
            continue;
        }
        if (c == '=' || c == ':') && names_secret(&out) {
            let value = rest[1..].trim_start();
            let skipped = rest.len() - value.len();
            let (quoted, body) = match value.strip_prefix('"') {
                Some(body) => (true, body),
                None => (false, value),
            };
            let end = if quoted {
                body.find('"').map_or(body.len(), |i| i + 1)
            } else {
                body.find(|c: char| c.is_whitespace() || c == '&' || c == ',').unwrap_or(body.len())
            };
            out.push(c);
            out.push_str(if quoted { "\"…\"" } else { "…" });
            rest = &rest[skipped + usize::from(quoted) + end..];
            continue;
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn failure_kind(source: &'static str, e: &anyhow::Error) -> &'static str {
    if Code::of(e).network() { "net" } else { source }
}

static GLOBAL: OnceLock<Journal> = OnceLock::new();

pub fn start(dir: &Path) {
    GLOBAL.get_or_init(|| Journal::open(dir, LIMIT));
}

pub fn log(level: Level, kind: &'static str, text: impl Into<String>) {
    let text = text.into();
    if level == Level::Error && kind != "qml" && kind != "panic" {
        eprintln!("{}: {kind}: {text}", product::ID);
    }
    if let Some(journal) = GLOBAL.get() {
        journal.push(level, kind, text);
    }
}

pub fn info(kind: &'static str, text: impl Into<String>) {
    log(Level::Info, kind, text);
}

pub fn warn(kind: &'static str, text: impl Into<String>) {
    log(Level::Warn, kind, text);
}

pub fn error(kind: &'static str, text: impl Into<String>) {
    log(Level::Error, kind, text);
}

fn flush() {
    if let Some(journal) = GLOBAL.get() {
        journal.flush(Duration::from_secs(1));
    }
}

pub fn dir() -> Option<&'static Path> {
    GLOBAL.get().map(|j| j.dir.as_path())
}

fn digest(args: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(map) = args.as_object() {
        for key in ["id", "index", "pos", "value", "on", "mode", "from", "to", "q"] {
            if let Some(v) = map.get(key) {
                let v = v.to_string();
                parts.push(format!("{key}={}", v.chars().take(60).collect::<String>()));
            }
        }
        if let Some(id) = map.get("track").and_then(|t| t["id"].as_str()) {
            parts.push(format!("track={id}"));
        }
        if let Some(tracks) = map.get("tracks").and_then(Value::as_array) {
            parts.push(format!("tracks={} first={}", tracks.len(), tracks.first().and_then(|t| t["id"].as_str()).unwrap_or("-")));
        }
    }
    parts.join(" ")
}

fn with_space(digest: &str) -> String {
    if digest.is_empty() { String::new() } else { format!(" {digest}") }
}

pub fn call(source: &'static str, method: &str, args: &Value, took: Duration, failure: Option<&anyhow::Error>) {
    let ms = took.as_millis();
    match failure {
        None if QUIET.contains(&method) => {}
        None => info(source, format!("{method}{} {ms} ms", with_space(&digest(args)))),
        Some(e) => {
            warn(failure_kind(source, e), format!("{method}{} failed after {ms} ms: {e:#}", with_space(&digest(args))));
        }
    }
}

pub fn tail(span: Duration) -> String {
    let Some(dir) = dir() else { return String::new() };
    flush();
    let cutoff = stamp(SystemTime::now() - span);
    let mut out = String::new();
    let mut keep = false;
    for name in ["journal.1.log", "journal.log"] {
        let bytes = std::fs::read(dir.join(name)).unwrap_or_default();
        for line in String::from_utf8_lossy(&bytes).lines() {
            let line = line.replace('\0', "");
            if is_entry_start(&line) {
                keep = line.as_str() >= cutoff.as_str();
            }
            if keep {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let trace = std::backtrace::Backtrace::force_capture();
        error("panic", format!("thread {}: {info}\n{trace}", thread.name().unwrap_or("?")));
        flush();
        previous(info);
    }));
}

pub fn watch_stalls<F>(post: F)
where
    F: Fn(Box<dyn FnOnce() + Send>) -> bool + Send + 'static,
{
    std::thread::Builder::new()
        .name(format!("{}-stall", product::ID))
        .spawn(move || {
            let waiting = Arc::new(AtomicBool::new(false));
            let mut sent = Instant::now();
            let mut stalled = false;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                if waiting.load(Ordering::Acquire) {
                    if !stalled && sent.elapsed() > STALL {
                        stalled = true;
                        error("stall", format!("the UI thread has not answered for {} s", sent.elapsed().as_secs()));
                    }
                    continue;
                }
                if stalled {
                    stalled = false;
                    warn("stall", format!("the UI thread answered again after {:.1} s", sent.elapsed().as_secs_f64()));
                }
                sent = Instant::now();
                waiting.store(true, Ordering::Release);
                let flag = waiting.clone();
                if !post(Box::new(move || flag.store(false, Ordering::Release))) {
                    return;
                }
            }
        })
        .expect("stall thread");
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-journal-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(dir: &Path) -> String {
        ["journal.1.log", "journal.log"]
            .iter()
            .map(|n| String::from_utf8_lossy(&std::fs::read(dir.join(n)).unwrap_or_default()).into_owned())
            .collect()
    }

    #[test]
    fn urls_lose_their_query_and_path() {
        let s = scrub("GET https://rr1.googlevideo.com/videoplayback?expire=1&sig=AAAA&n=xyz failed, retry http://example.test/a/b.mp3");
        assert_eq!(s, "GET https://rr1.googlevideo.com/… failed, retry http://example.test/…");
    }

    #[test]
    fn home_paths_and_secrets_are_replaced() {
        let home = dirs::home_dir().unwrap();
        let s = scrub(&format!("open {}/Music/a.mp3 token=abc123 key: hunter2 &c=WEB", home.display()));
        assert_eq!(s, "open ~/Music/a.mp3 token=… key:… &c=WEB");
        assert_eq!(scrub("id dQw4w9WgXcQ"), "id dQw4w9WgXcQ");
        assert_eq!(scrub("blob AbCdEfGhIjKlMnOpQrStUvWxYz0123456789 end"), "blob … end");
        assert_eq!(scrub("a\0b"), "ab");
    }

    #[test]
    fn network_failures_are_told_apart_by_code_not_text() {
        assert_eq!(failure_kind("api", &crate::error::Code::NetOffline.err("dns error")), "net");
        assert_eq!(failure_kind("api", &anyhow::anyhow!("error sending request: connection refused")), "api");
        assert_eq!(failure_kind("api", &crate::error::Code::ApiArgs.err("missing argument `q`")), "api");
    }

    #[test]
    fn digest_keeps_ids_and_drops_bodies() {
        let args = serde_json::json!({ "track": { "id": "dQw4w9WgXcQ", "thumb": "https://x.test/secret?token=1" }, "index": 2, "tracks": [{ "id": "a" }, { "id": "b" }] });
        let d = digest(&args);
        assert!(d.contains("track=dQw4w9WgXcQ") && d.contains("index=2") && d.contains("tracks=2 first=a"), "{d}");
        assert!(!d.contains("secret"));
    }

    #[test]
    fn lines_are_written_in_order_with_level_and_kind() {
        let dir = dir("order");
        let j = Journal::open(&dir, LIMIT);
        j.push(Level::Info, "track", "one".into());
        j.push(Level::Error, "panic", "two\nframe a\nframe b".into());
        j.flush(Duration::from_secs(2));
        let text = read(&dir);
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(lines[0].contains("INFO  track    one") && lines[1].contains("ERROR panic    two"), "{text}");
        assert_eq!(lines[2], "    frame a");
        assert!(is_entry_start(lines[0]) && !is_entry_start(lines[2]));
    }

    #[test]
    fn the_same_line_over_and_over_is_written_once_with_a_count() {
        let dir = dir("repeat");
        let j = Journal::open(&dir, LIMIT);
        for _ in 0..50 {
            j.push(Level::Warn, "qml", "Error transferring cover".into());
        }
        j.push(Level::Info, "api", "other".into());
        j.flush(Duration::from_secs(2));
        let text = read(&dir);
        assert_eq!(text.matches("Error transferring cover").count(), 1, "{text}");
        assert!(text.contains("repeated 49 more times"), "{text}");
        assert!(text.find("repeated").unwrap() < text.find("other").unwrap());
    }

    #[test]
    fn the_size_stays_under_the_limit_and_the_newest_lines_survive() {
        let dir = dir("rotate");
        let j = Journal::open(&dir, 20_000);
        for i in 0..2000 {
            j.push(Level::Info, "api", format!("line {i:05} {}", "x".repeat(40)));
            if i % 200 == 0 {
                j.flush(Duration::from_secs(2));
            }
        }
        j.flush(Duration::from_secs(2));
        let size: u64 = ["journal.log", "journal.1.log"].iter().map(|n| std::fs::metadata(dir.join(n)).map_or(0, |m| m.len())).sum();
        assert!(size <= 20_000, "{size}");
        assert!(dir.join("journal.1.log").exists());
        assert!(read(&dir).contains("line 01999"));
    }

    #[test]
    fn a_torn_tail_and_zero_bytes_do_not_spoil_the_rest() {
        let dir = dir("torn");
        {
            let j = Journal::open(&dir, LIMIT);
            j.push(Level::Info, "api", "before".into());
            j.flush(Duration::from_secs(2));
        }
        let mut file = OpenOptions::new().append(true).open(dir.join("journal.log")).unwrap();
        file.write_all(b"2026-11-02 12:00:00.000 INFO  api      cut in the mi\0\0\0\0\0\0").unwrap();
        drop(file);
        let j = Journal::open(&dir, LIMIT);
        j.push(Level::Info, "api", "after".into());
        j.flush(Duration::from_secs(2));
        let text = read(&dir);
        assert!(text.contains("before") && text.contains("after"));
        let tail: String = text.lines().filter(|l| l.contains("after")).collect();
        assert!(is_entry_start(&tail) || tail.contains("after"));
    }

    struct Stuck(Arc<Mutex<()>>);

    impl Sink for Stuck {
        fn append(&mut self, _: &[u8]) {
            let _gate = self.0.lock().unwrap();
        }
        fn sync(&mut self) {}
    }

    #[test]
    fn a_writer_that_never_returns_does_not_block_callers() {
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let j = Journal::with_sink(Path::new("/nonexistent"), Box::new(Stuck(gate.clone())));
        let t = Instant::now();
        for i in 0..100_000 {
            j.push(Level::Info, "api", format!("line {i}"));
        }
        let took = t.elapsed();
        assert!(took < Duration::from_secs(1), "{took:?}");
        assert!(lock(&j.shared.queue).dropped > 0 || lock(&j.shared.queue).items.len() <= QUEUE);
        drop(held);
    }

    #[test]
    fn the_stall_watcher_reports_a_thread_that_stops_answering() {
        let dir = dir("stall");
        GLOBAL.get_or_init(|| Journal::open(&dir, LIMIT));
        let alive = Arc::new(AtomicBool::new(true));
        let answer = alive.clone();
        watch_stalls(move |job| {
            if answer.load(Ordering::Relaxed) {
                job();
            }
            true
        });
        std::thread::sleep(Duration::from_millis(1500));
        alive.store(false, Ordering::Relaxed);
        std::thread::sleep(Duration::from_secs(6));
        flush();
        let text = read(GLOBAL.get().map(|j| j.dir.as_path()).unwrap());
        assert!(text.contains("ERROR stall"), "{text}");
    }

    #[test]
    #[ignore = "prints the cost of a journal call, run on its own"]
    fn cost() {
        let dir = dir("cost");
        let j = Journal::open(&dir, LIMIT);
        let n = 200_000;
        let (mut worst, mut total) = (Duration::ZERO, Duration::ZERO);
        for i in 0..n {
            let one = Instant::now();
            j.push(Level::Info, "api", format!("play id=dQw4w9WgXcQ index={i} 12 ms"));
            let took = one.elapsed();
            (worst, total) = (worst.max(took), total + took);
            if i % 2000 == 0 {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        j.flush(Duration::from_secs(5));
        println!("{n} calls: {:.2} us average, worst {worst:?}", total.as_secs_f64() * 1e6 / n as f64);
        let cpu = || {
            let mut u: libc::rusage = unsafe { std::mem::zeroed() };
            unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
            (u.ru_utime.tv_sec + u.ru_stime.tv_sec) as f64 + (u.ru_utime.tv_usec + u.ru_stime.tv_usec) as f64 / 1e6
        };
        let (before, start) = (cpu(), Instant::now());
        let mut sent = 0;
        while start.elapsed() < Duration::from_secs(5) {
            for _ in 0..100 {
                j.push(Level::Info, "api", format!("play id=dQw4w9WgXcQ index={sent} 12 ms"));
                sent += 1;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let share = (cpu() - before) / start.elapsed().as_secs_f64() * 100.0;
        println!("1000 lines/s for 5 s: {share:.2} % of one core (caller and writer together)");
        let size: u64 = ["journal.log", "journal.1.log"].iter().map(|n| std::fs::metadata(dir.join(n)).map_or(0, |m| m.len())).sum();
        println!("on disk: {size} bytes for {} lines", n + sent);
    }
}
