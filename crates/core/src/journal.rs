use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde_json::Value;

use crate::clock::stamp;
use crate::error::Code;
use crate::product;
use crate::sync::Lock;

const LIMIT: u64 = 5 * 1024 * 1024;
const QUEUE: usize = 4096;
const BATCH: Duration = Duration::from_millis(50);
const SYNC_EVERY: Duration = Duration::from_secs(5);
const LINE_MAX: usize = 16 * 1024;
const STALL: Duration = Duration::from_secs(3);
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
        let mut queue = self.shared.queue.locked();
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
        let mut queue = self.shared.queue.locked();
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
        let mut queue = shared.queue.locked();
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
        let mut queue = shared.queue.locked();
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
        shared.queue.locked().written = upto;
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

pub fn flush() {
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

pub fn call(source: &'static str, method: &str, quiet: bool, args: &Value, took: Duration, failure: Option<&anyhow::Error>) {
    let ms = took.as_millis();
    match failure {
        None if quiet => {}
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
