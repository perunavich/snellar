use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as Atomic};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::Serialize;
use tokio::process::Command;
use tokio::sync::Notify;

use snellar_core::error::Code;
use snellar_core::library::Library;
use snellar_core::ui::{Ui, UiEvent};
use snellar_core::{journal, product, settings};
use snellar_youtube::net::{Http, Route, checked, net};

use crate::ytdlp::{self, Progress};

const RELEASES: &str = "https://github.com/perunavich/snellar/releases";
const URL_ENV: &str = "SNELLAR_UPDATE_URL";
const AFTER_START: Duration = Duration::from_secs(15);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
const HOUR: Duration = Duration::from_secs(60 * 60);
const CONNECT_LIMIT: Duration = Duration::from_secs(10);
const STALL_LIMIT: Duration = Duration::from_secs(60);
const LOWEST_NICE: libc::c_int = 19;
const IOPRIO_WHO_PROCESS: libc::c_long = 1;
const IOPRIO_THIS_THREAD: libc::c_long = 0;
const IOPRIO_IDLE: libc::c_long = 3 << 13;
const DESKTOP_ENTRY: &str = "snellar.desktop";
const EXTRACTED: &str = "appimage_extracted_";
const KEY_PEM: &str = include_str!(concat!(env!("OUT_DIR"), "/update-key.pub"));
const ED25519_SPKI: [u8; 12] = [0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];
const LEFT_BY_THE_RUNNING_APP: [&str; 6] = ["LD_LIBRARY_PATH", "APPDIR", "APPIMAGE", "ARGV0", "OWD", "QML_XHR_ALLOW_FILE_READ"];

static RELAUNCH: OnceLock<PathBuf> = OnceLock::new();

pub type Key = [u8; 32];

pub fn key_of(pem: &str) -> Option<Key> {
    let mut lines = pem.lines().map(str::trim).filter(|line| !line.is_empty());
    if lines.next() != Some("-----BEGIN PUBLIC KEY-----") {
        return None;
    }
    let mut body = String::new();
    for line in lines.by_ref() {
        if line == "-----END PUBLIC KEY-----" {
            let der = STANDARD.decode(&body).ok()?;
            return lines.next().is_none().then(|| der.strip_prefix(&ED25519_SPKI)?.try_into().ok()).flatten();
        }
        body.push_str(line);
    }
    None
}

pub fn compiled_key() -> Option<Key> {
    key_of(KEY_PEM)
}

pub fn compiled_key_hex() -> String {
    compiled_key().map_or_else(|| "none".to_string(), |key| key.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn signed(key: &Key, sums: &[u8], signature: &[u8]) -> bool {
    UnparsedPublicKey::new(&ED25519, key).verify(sums, signature).is_ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    numbers: [u64; 3],
    stage: Vec<String>,
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn stage_cmp(a: &str, b: &str) -> Ordering {
    match (all_digits(a), all_digits(b)) {
        (true, true) => a.parse::<u64>().ok().cmp(&b.parse::<u64>().ok()),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.cmp(b),
    }
}

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split_once('+').map_or(text, |(version, _)| version);
        let (numbers, stage) = text.split_once('-').map_or((text, None), |(numbers, stage)| (numbers, Some(stage)));
        let numbers: Vec<u64> = numbers.split('.').map(|n| if all_digits(n) { n.parse().ok() } else { None }).collect::<Option<_>>()?;
        let stage: Vec<String> = stage.map_or(Vec::new(), |stage| stage.split('.').map(String::from).collect());
        if stage.iter().any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
            return None;
        }
        Some(Version { numbers: numbers.try_into().ok()?, stage })
    }

    fn shown(tag: &str) -> String {
        product::shown_version(tag.strip_prefix('v').unwrap_or(tag))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Version) -> Ordering {
        self.numbers.cmp(&other.numbers).then_with(|| match (self.stage.is_empty(), other.stage.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self
                .stage
                .iter()
                .zip(&other.stage)
                .map(|(a, b)| stage_cmp(a, b))
                .find(|order| order.is_ne())
                .unwrap_or_else(|| self.stage.len().cmp(&other.stage.len())),
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Installed(PathBuf),
    Development,
    Elsewhere(String),
}

pub fn place_of(appimage: Option<OsString>, installed: Option<PathBuf>) -> Place {
    let Some(appimage) = appimage.map(PathBuf::from) else { return Place::Development };
    let Some(installed) = installed else { return Place::Elsewhere(appimage.display().to_string()) };
    match (appimage.canonicalize(), installed.canonicalize()) {
        (Ok(running), Ok(target)) if running == target && !installed.is_symlink() => Place::Installed(target),
        _ => Place::Elsewhere(appimage.display().to_string()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Development,
    Elsewhere,
    Unsigned,
    Off,
    Idle,
    Current,
    Checking,
    Downloading,
    Ready,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub version: String,
    pub checked: u64,
    pub latest: String,
    pub percent: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    pub place: String,
}

pub struct Updater {
    library: Arc<Library>,
    route: Route,
    ui: Ui,
    place: Place,
    key: Option<Key>,
    base: String,
    compiled: Version,
    status: Mutex<Status>,
    wake: Notify,
    asked: AtomicBool,
}

fn beside(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

fn lock_of(target: &Path) -> PathBuf {
    beside(target, ".lock")
}

fn part_of(target: &Path) -> PathBuf {
    beside(target, ".part")
}

fn check_dir_of(target: &Path) -> PathBuf {
    beside(target, ".check")
}

fn previous_of(target: &Path) -> PathBuf {
    beside(target, ".previous")
}

fn alone(target: &Path) -> Result<Option<File>> {
    let lock = File::options().create(true).truncate(false).write(true).open(lock_of(target))?;
    // SAFETY: the descriptor stays open for the call; flock changes nothing but the lock on it.
    let taken = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    Ok(taken.then_some(lock))
}

async fn latest_tag(client: &reqwest::Client, base: &str) -> Result<String> {
    let response = checked(client.get(format!("{base}/latest")).send().await.map_err(net)?)?;
    Ok(response.url().path_segments().and_then(|mut segments| segments.next_back()).unwrap_or_default().to_string())
}

async fn body(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = checked(client.get(url).send().await.map_err(net)?)?;
    Ok(response.bytes().await.map_err(net)?.to_vec())
}

async fn signature(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url).send().await.map_err(net)?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(Code::UpdateSignature.err(format!("{url} is not there")));
    }
    Ok(checked(response)?.bytes().await.map_err(net)?.to_vec())
}

async fn unpacks(part: &Path, check: &Path) -> bool {
    std::fs::remove_dir_all(check).ok();
    if std::fs::create_dir_all(check).is_err() {
        return false;
    }
    let mut command = Command::new(part);
    command
        .args(["--appimage-extract", DESKTOP_ENTRY])
        .current_dir(check)
        .env_remove("APPIMAGE_EXTRACT_AND_RUN")
        .env_remove("NO_CLEANUP")
        .env_remove("APPIMAGE")
        .env_remove("APPDIR");
    let ran = ytdlp::output(command).await.is_some();
    let found = check.join("squashfs-root").join(DESKTOP_ENTRY).is_file();
    std::fs::remove_dir_all(check).ok();
    ran && found
}

fn keep_previous(target: &Path) {
    let (previous, link) = (previous_of(target), beside(target, ".previous.new"));
    std::fs::remove_file(&link).ok();
    let kept = std::fs::hard_link(target, &link).and_then(|()| std::fs::rename(&link, &previous));
    if let Err(e) = kept {
        std::fs::remove_file(&link).ok();
        journal::warn("update", format!("no rollback copy this time: {e}"));
    }
}

async fn install(client: &reqwest::Client, url: &str, want: &str, target: &Path, part: &Path, progress: &Progress<'_>) -> Result<()> {
    let got = ytdlp::download(client, url, part, progress).await.map_err(cut_short)?;
    if got != want {
        return Err(Code::UpdateChecksum.err(format!("{url}: expected {want}, got {got}")));
    }
    std::fs::set_permissions(part, std::fs::Permissions::from_mode(0o755))?;
    if !unpacks(part, &check_dir_of(target)).await {
        return Err(Code::UpdateBroken.err(format!("{url} does not unpack here")));
    }
    File::open(part)?.sync_all()?;
    keep_previous(target);
    std::fs::rename(part, target).with_context(|| format!("cannot place {}", target.display()))?;
    File::open(target.parent().context("the app has no folder")?)?.sync_all()?;
    Ok(())
}

fn cut_short(e: anyhow::Error) -> anyhow::Error {
    if Code::of(&e) == Code::YtParse { e.context(Code::NetConnect) } else { e }
}

fn in_words(e: anyhow::Error) -> anyhow::Error {
    if Code::of(&e) == Code::Internal { e.context(Code::UpdatePlace) } else { e }
}

async fn place(client: &reqwest::Client, url: &str, want: &str, target: &Path, progress: &Progress<'_>) -> Result<()> {
    let lock = alone(target).with_context(|| format!("cannot write beside {}", target.display())).map_err(in_words)?;
    let Some(_lock) = lock else { return Err(Code::UpdatePlace.err("another Snellar is placing an update")) };
    let part = part_of(target);
    let placed = install(client, url, want, target, &part, progress).await.map_err(in_words);
    if placed.is_err() {
        std::fs::remove_file(&part).ok();
    }
    placed
}

fn leftovers(target: &Path) {
    std::fs::remove_file(part_of(target)).ok();
    std::fs::remove_dir_all(check_dir_of(target)).ok();
}

fn tidy(target: &Path) {
    if let Ok(Some(_lock)) = alone(target) {
        leftovers(target);
    }
}

fn sweep_extractions(target: &Path, own: &Path) {
    let Some(root) = target.parent().and_then(Path::parent) else { return };
    let Ok(entries) = std::fs::read_dir(root.join("run")) else { return };
    let own = own.canonicalize().ok();
    for entry in entries.flatten() {
        let path = entry.path();
        let old = entry.file_name().to_string_lossy().starts_with(EXTRACTED) && path.canonicalize().ok() != own;
        if old && let Err(e) = std::fs::remove_dir_all(&path) {
            journal::warn("update", format!("cannot remove {}: {e}", path.display()));
        }
    }
}

pub fn relaunch_if_asked() {
    let Some(target) = RELAUNCH.get() else { return };
    let mut command = std::process::Command::new(target);
    for name in LEFT_BY_THE_RUNNING_APP {
        command.env_remove(name);
    }
    // SAFETY: the closure runs between fork-free exec setup and execve and only makes one async-signal-safe system call.
    unsafe {
        command.pre_exec(|| {
            libc::syscall(libc::SYS_close_range, 3 as libc::c_uint, libc::c_uint::MAX, libc::CLOSE_RANGE_CLOEXEC);
            Ok(())
        });
    }
    let e = command.exec();
    journal::warn("update", format!("cannot start {}: {e}", target.display()));
}

fn step_aside() {
    // SAFETY: both calls take plain integers and change only this thread, whose children inherit them.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, LOWEST_NICE);
        libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, IOPRIO_THIS_THREAD, IOPRIO_IDLE);
    }
}

impl Updater {
    pub fn new(library: Arc<Library>, route: Route, ui: Ui, place: Place, key: Option<Key>, base: String, compiled: &str) -> Arc<Updater> {
        let (phase, shown) = match &place {
            Place::Installed(_) if key.is_none() => (Phase::Unsigned, String::new()),
            Place::Installed(_) => (Phase::Idle, String::new()),
            Place::Development => (Phase::Development, String::new()),
            Place::Elsewhere(path) => (Phase::Elsewhere, path.clone()),
        };
        let status = Status {
            phase,
            version: product::shown_version(compiled),
            checked: 0,
            latest: String::new(),
            percent: 0,
            code: None,
            place: shown,
        };
        Arc::new(Updater {
            library,
            route,
            ui,
            place,
            key,
            base,
            compiled: Version::parse(compiled).expect("the build's own version parses"),
            status: Mutex::new(status),
            wake: Notify::new(),
            asked: AtomicBool::new(false),
        })
    }

    pub fn for_this_run(library: Arc<Library>, route: &Route, ui: Ui) -> Arc<Updater> {
        let place = place_of(std::env::var_os("APPIMAGE"), product::installed_app_image());
        let base = std::env::var(URL_ENV).unwrap_or_else(|_| RELEASES.to_string());
        Updater::new(library, route.clone(), ui, place, compiled_key(), base, product::VERSION)
    }

    pub fn start(self: Arc<Self>) {
        if !matches!(self.place, Place::Installed(_)) || self.key.is_none() {
            return;
        }
        let spawned = std::thread::Builder::new().name("snellar-update".into()).spawn(move || {
            step_aside();
            match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime.block_on(self.run()),
                Err(e) => journal::warn("update", format!("{e}")),
            }
        });
        if let Err(e) = spawned {
            journal::warn("update", format!("{e}"));
        }
    }

    async fn run(&self) {
        tokio::select! {
            () = tokio::time::sleep(AFTER_START) => {}
            () = self.wake.notified() => self.wake.notify_one(),
        }
        if let Place::Installed(target) = &self.place {
            tidy(target);
            if let Some(own) = std::env::var_os("APPDIR") {
                sweep_extractions(target, Path::new(&own));
            }
        }
        loop {
            let wait = self.turn(ytdlp::now()).await;
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = self.wake.notified() => {}
            }
        }
    }

    fn on(&self) -> bool {
        settings::AUTO_UPDATE.or_default(&self.library)
    }

    pub fn toggled(&self) {
        {
            let mut status = self.status.lock().unwrap();
            if matches!(status.phase, Phase::Current | Phase::Failed) {
                status.phase = Phase::Idle;
                status.code = None;
            }
        }
        self.wake.notify_one();
    }

    pub fn check_now(&self) -> bool {
        if !matches!(self.place, Place::Installed(_)) || self.key.is_none() {
            return false;
        }
        {
            let mut status = self.status.lock().unwrap();
            if matches!(status.phase, Phase::Checking | Phase::Downloading | Phase::Ready) {
                return false;
            }
            status.phase = Phase::Checking;
            status.code = None;
            self.asked.store(true, Atomic::SeqCst);
        }
        (self.ui)(UiEvent::Update);
        self.wake.notify_one();
        true
    }

    pub fn restart(&self) -> bool {
        let Place::Installed(target) = &self.place else { return false };
        let ready = self.status.lock().unwrap().phase == Phase::Ready && target.is_file();
        ready && (RELAUNCH.set(target.clone()).is_ok() || RELAUNCH.get() == Some(target))
    }

    pub fn status(&self) -> Status {
        let mut status = self.status.lock().unwrap().clone();
        status.checked = settings::UPDATE_CHECKED.or_default(&self.library);
        if matches!(self.place, Place::Installed(_)) && status.phase == Phase::Idle && !self.on() {
            status.phase = Phase::Off;
        }
        status
    }

    fn show(&self, change: impl FnOnce(&mut Status)) {
        change(&mut self.status.lock().unwrap());
        (self.ui)(UiEvent::Update);
    }

    async fn turned_off(&self) {
        while self.on() {
            self.wake.notified().await;
        }
    }

    pub async fn turn(&self, now: u64) -> Duration {
        let (Place::Installed(target), Some(key)) = (&self.place, &self.key) else { return DAY };
        let asked = self.asked.swap(false, Atomic::SeqCst);
        if !asked {
            if !self.on() {
                return DAY;
            }
            let checked: u64 = settings::UPDATE_CHECKED.or_default(&self.library);
            if checked <= now && now - checked < DAY.as_millis() as u64 {
                return Duration::from_millis(checked + DAY.as_millis() as u64 - now).min(HOUR);
            }
            self.show(|status| {
                status.phase = Phase::Checking;
                status.code = None;
            });
        }
        let outcome = if asked {
            Some(self.check(target, key, now).await)
        } else {
            tokio::select! {
                outcome = self.check(target, key, now) => Some(outcome),
                () = self.turned_off() => None,
            }
        };
        match outcome {
            None => {
                leftovers(target);
                self.show(|status| status.phase = Phase::Idle);
            }
            Some(Ok(phase)) => self.show(|status| status.phase = phase),
            Some(Err(e)) => {
                journal::warn("update", format!("{e:#}"));
                let code = Code::of(&e);
                self.show(|status| {
                    status.phase = Phase::Failed;
                    status.code = Some(code);
                });
            }
        }
        HOUR
    }

    async fn check(&self, target: &Path, key: &Key, now: u64) -> Result<Phase> {
        let http = Http::new(&self.route, |builder| {
            builder.connect_timeout(CONNECT_LIMIT).read_timeout(STALL_LIMIT).user_agent(product::USER_AGENT)
        });
        let client = http.client();
        let tag = latest_tag(&client, &self.base).await?;
        settings::UPDATE_CHECKED.set(&self.library, &now);
        let Some(latest) = Version::parse(&tag) else {
            return Err(Code::UpdateNoRelease.err(format!("the latest release is named `{tag}`")));
        };
        if latest <= self.compiled {
            return Ok(Phase::Current);
        }
        let shown = Version::shown(&tag);
        self.show(|status| status.latest = shown);
        let asset = product::APP_IMAGE;
        let folder = format!("{}/download/{tag}", self.base);
        let sums_url = format!("{folder}/{asset}.sha256");
        let sums = body(&client, &sums_url).await?;
        let signature = signature(&client, &format!("{sums_url}.sig")).await?;
        if !signed(key, &sums, &signature) {
            return Err(Code::UpdateSignature.err(format!("{sums_url}.sig does not verify with the key of this build")));
        }
        let sums = String::from_utf8_lossy(&sums);
        let want = ytdlp::published(&sums, asset)
            .ok_or_else(|| Code::UpdateChecksum.err(format!("{asset}.sha256 does not list {asset}")))?
            .to_ascii_lowercase();
        if ytdlp::hash_of(target).as_deref() == Some(want.as_str()) {
            return Ok(Phase::Ready);
        }
        self.show(|status| {
            status.phase = Phase::Downloading;
            status.percent = 0;
        });
        let tell = |percent: u8| self.show(|status| status.percent = percent);
        place(&client, &format!("{folder}/{asset}"), &want, target, &tell).await?;
        journal::info("update", format!("{tag} is in place, it starts at the next launch"));
        Ok(Phase::Ready)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering as Counted};

    use ring::digest;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    use super::*;

    const NOW: u64 = 1_000 * DAY.as_millis() as u64;
    const OLD: &str = "#!/bin/sh\necho old\n";
    const ASSET: &str = product::APP_IMAGE;

    fn appimage(marker: &str) -> String {
        format!(
            "#!/bin/sh\n# {marker}\ncase \"$1\" in --appimage-extract) mkdir -p squashfs-root && echo entry > squashfs-root/{DESKTOP_ENTRY} ;; esac\n"
        )
    }

    fn sha(body: &str) -> String {
        digest::digest(&digest::SHA256, body.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }

    fn pair() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn signer() -> &'static Ed25519KeyPair {
        static SIGNER: OnceLock<Ed25519KeyPair> = OnceLock::new();
        SIGNER.get_or_init(pair)
    }

    fn public(pair: &Ed25519KeyPair) -> Key {
        pair.public_key().as_ref().try_into().unwrap()
    }

    fn sign(sum: &str) -> Vec<u8> {
        signer().sign(sum.as_bytes()).as_ref().to_vec()
    }

    #[derive(Clone, Default)]
    struct Site {
        tag: Arc<Mutex<String>>,
        sum: Arc<Mutex<String>>,
        sig: Arc<Mutex<Option<Vec<u8>>>>,
        body: Arc<Mutex<String>>,
        cut_after: Arc<Mutex<Option<usize>>>,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Site {
        async fn serve(tag: &str, body: &str) -> (String, Site) {
            let site = Site::default();
            site.publish(tag, body);
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/releases", listener.local_addr().unwrap());
            let served = site.clone();
            tokio::spawn(async move {
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let served = served.clone();
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        reader.read_line(&mut line).await.unwrap();
                        let path = line.split_whitespace().nth(1).unwrap_or_default().to_string();
                        served.asked.lock().unwrap().push(path.clone());
                        let reply = served.answer(&path);
                        reader.get_mut().write_all(&reply).await.ok();
                    });
                }
            });
            (base, site)
        }

        fn answer(&self, path: &str) -> Vec<u8> {
            let tag = self.tag.lock().unwrap().clone();
            let ok = |body: &[u8], length: usize| {
                let mut reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").into_bytes();
                reply.extend_from_slice(body);
                reply
            };
            if path == "/releases/latest" {
                return format!("HTTP/1.1 302 Found\r\nLocation: /releases/tag/{tag}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .into_bytes();
            }
            if path == format!("/releases/tag/{tag}") {
                return ok(b"a page", 6);
            }
            if path == format!("/releases/download/{tag}/{ASSET}.sha256") {
                let sum = self.sum.lock().unwrap().clone();
                return ok(sum.as_bytes(), sum.len());
            }
            if path == format!("/releases/download/{tag}/{ASSET}.sha256.sig")
                && let Some(sig) = self.sig.lock().unwrap().clone()
            {
                return ok(&sig, sig.len());
            }
            if path == format!("/releases/download/{tag}/{ASSET}") {
                let body = self.body.lock().unwrap().clone();
                return match *self.cut_after.lock().unwrap() {
                    Some(sent) => ok(&body.as_bytes()[..sent], body.len()),
                    None => ok(body.as_bytes(), body.len()),
                };
            }
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }

        fn publish(&self, tag: &str, body: &str) {
            *self.tag.lock().unwrap() = tag.to_string();
            self.set_sum(&format!("{}  {ASSET}\n", sha(body)));
            *self.body.lock().unwrap() = body.to_string();
        }

        fn set_sum(&self, sum: &str) {
            *self.sum.lock().unwrap() = sum.to_string();
            *self.sig.lock().unwrap() = Some(sign(sum));
        }

        fn asked_downloads(&self) -> usize {
            self.asked.lock().unwrap().iter().filter(|path| path.ends_with(ASSET)).count()
        }
    }

    struct Case {
        updater: Arc<Updater>,
        target: PathBuf,
        told: Arc<AtomicUsize>,
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-update-{name}-{}", std::process::id()));
        if dir.exists() {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).ok();
            std::fs::remove_dir_all(&dir).ok();
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn case(name: &str, base: &str, compiled: &str) -> Case {
        let root = folder(name);
        let versions = root.join("versions");
        std::fs::create_dir_all(&versions).unwrap();
        let target = versions.join(ASSET);
        std::fs::write(&target, OLD).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let library = Arc::new(Library::open(Path::new(":memory:")).unwrap());
        let told = Arc::new(AtomicUsize::new(0));
        let count = told.clone();
        let ui: Ui = Arc::new(move |_| {
            count.fetch_add(1, Counted::Relaxed);
        });
        let place = Place::Installed(target.clone());
        Case {
            updater: Updater::new(library, Route::default(), ui, place, Some(public(signer())), base.to_string(), compiled),
            target,
            told,
        }
    }

    fn installed(case: &Case) -> String {
        std::fs::read_to_string(&case.target).unwrap()
    }

    fn only_the_app_is_left(case: &Case) {
        let dir = case.target.parent().unwrap();
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.retain(|name| !name.ends_with(".lock"));
        names.sort();
        let mut want = vec![ASSET.to_string()];
        if previous_of(&case.target).exists() {
            want.push(format!("{ASSET}.previous"));
        }
        assert_eq!(names, want);
    }

    #[test]
    fn versions_order_like_semver() {
        let order = [
            "0.0.9",
            "0.1.0-alpha",
            "0.1.0-alpha.1",
            "0.1.0-alpha.beta",
            "0.1.0-beta",
            "0.1.0-beta.2",
            "0.1.0-beta.11",
            "0.1.0-rc.1",
            "0.1.0",
            "0.1.1-beta",
            "0.1.1",
            "0.2.0",
            "0.10.0",
            "1.0.0",
        ];
        for pair in order.windows(2) {
            let (low, high) = (Version::parse(pair[0]).unwrap(), Version::parse(pair[1]).unwrap());
            assert!(low < high, "{} < {}", pair[0], pair[1]);
            assert!(high > low, "{} > {}", pair[1], pair[0]);
        }
        assert_eq!(Version::parse("v0.1.0-beta"), Version::parse("0.1.0-beta"), "the tag's v");
        assert_eq!(Version::parse("0.1.0+build.3"), Version::parse("0.1.0"), "build metadata does not count");
    }

    #[test]
    fn what_is_not_a_version_is_refused() {
        for bad in ["releases", "", "v", "1", "1.2", "1.2.3.4", "1.2.x", "+1.2.3", "1.2.3-", "1.2.3-a..b", "1.2.3-a b", "../1.2.3"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert!(Version::parse(product::VERSION).is_some(), "this build's own version");
    }

    #[tokio::test]
    async fn a_newer_release_replaces_the_file_and_keeps_the_old_one_as_the_rollback() {
        let body = appimage("0.2.0");
        let (base, _) = Site::serve("v0.2.0", &body).await;
        let case = case("newer", &base, "0.1.0-beta");
        let inode = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&case.target).unwrap());
        let running = File::open(&case.target).unwrap();
        case.updater.turn(NOW).await;
        let status = case.updater.status();
        assert_eq!(status.phase, Phase::Ready, "{status:?}");
        assert_eq!((status.latest.as_str(), status.checked, status.code), ("0.2", NOW, None));
        assert_eq!(installed(&case), body);
        assert_ne!(std::fs::metadata(&case.target).unwrap().permissions().mode() & 0o111, 0, "executable");
        assert_eq!(std::fs::read_to_string(previous_of(&case.target)).unwrap(), OLD, "one rollback copy");
        assert_eq!(std::os::unix::fs::MetadataExt::ino(&running.metadata().unwrap()), inode, "a running copy keeps its file");
        assert_eq!(std::io::read_to_string(&running).unwrap(), OLD);
        only_the_app_is_left(&case);
        assert!(case.told.load(Counted::Relaxed) > 0, "the window hears of it");
    }

    #[tokio::test]
    async fn a_second_update_keeps_only_the_last_rollback() {
        let first = appimage("0.2.0");
        let (base, site) = Site::serve("v0.2.0", &first).await;
        let case = case("second", &base, "0.1.0");
        case.updater.turn(NOW).await;
        let second = appimage("0.3.0");
        site.publish("v0.3.0", &second);
        case.updater.turn(NOW + DAY.as_millis() as u64).await;
        assert_eq!(installed(&case), second);
        assert_eq!(std::fs::read_to_string(previous_of(&case.target)).unwrap(), first);
        only_the_app_is_left(&case);
    }

    #[tokio::test]
    async fn the_same_or_an_older_release_does_nothing() {
        for (tag, compiled) in
            [("v0.1.0-beta", "0.1.0-beta"), ("v0.1.0-alpha", "0.1.0-beta"), ("v0.0.9", "0.1.0-beta"), ("v0.1.0-beta", "0.1.0")]
        {
            let (base, site) = Site::serve(tag, &appimage("x")).await;
            let case = case(&format!("same{tag}{compiled}"), &base, compiled);
            case.updater.turn(NOW).await;
            assert_eq!(case.updater.status().phase, Phase::Current, "{tag} over {compiled}");
            assert_eq!(installed(&case), OLD);
            assert_eq!(site.asked_downloads(), 0);
            assert!(site.asked.lock().unwrap().iter().all(|path| !path.contains("sha256")), "not even the checksum");
            assert_eq!(case.updater.status().checked, NOW);
        }
    }

    #[tokio::test]
    async fn the_final_release_is_newer_than_its_own_beta() {
        let body = appimage("0.1.0");
        let (base, _) = Site::serve("v0.1.0", &body).await;
        let case = case("final", &base, "0.1.0-beta");
        case.updater.turn(NOW).await;
        assert_eq!(installed(&case), body);
    }

    #[tokio::test]
    async fn a_later_beta_is_read_by_number_not_by_letters() {
        let body = appimage("beta.11");
        let (base, _) = Site::serve("v0.1.0-beta.11", &body).await;
        let case = case("beta", &base, "0.1.0-beta.2");
        case.updater.turn(NOW).await;
        assert_eq!(installed(&case), body);
    }

    #[tokio::test]
    async fn a_wrong_checksum_leaves_the_file_and_the_next_check_tries_again() {
        let body = appimage("0.2.0");
        let (base, site) = Site::serve("v0.2.0", &body).await;
        site.set_sum(&format!("{}  {ASSET}\n", sha("something else")));
        let case = case("bad", &base, "0.1.0");
        case.updater.turn(NOW).await;
        let status = case.updater.status();
        assert_eq!((status.phase, status.code), (Phase::Failed, Some(Code::UpdateChecksum)));
        assert_eq!(installed(&case), OLD);
        only_the_app_is_left(&case);
        assert!(!previous_of(&case.target).exists(), "nothing was rotated");
        site.publish("v0.2.0", &body);
        case.updater.turn(NOW + DAY.as_millis() as u64).await;
        assert_eq!(case.updater.status().phase, Phase::Ready);
        assert_eq!(installed(&case), body);
    }

    #[tokio::test]
    async fn a_checksum_file_that_does_not_name_the_app_is_refused_before_downloading() {
        let (base, site) = Site::serve("v0.2.0", &appimage("0.2.0")).await;
        site.set_sum("abc  something\n");
        let case = case("unlisted", &base, "0.1.0");
        case.updater.turn(NOW).await;
        assert_eq!(case.updater.status().code, Some(Code::UpdateChecksum));
        assert_eq!(site.asked_downloads(), 0);
    }

    #[tokio::test]
    async fn a_download_cut_short_leaves_the_file() {
        let body = appimage("0.2.0");
        let (base, site) = Site::serve("v0.2.0", &body).await;
        *site.cut_after.lock().unwrap() = Some(body.len() / 2);
        let case = case("cut", &base, "0.1.0");
        case.updater.turn(NOW).await;
        let status = case.updater.status();
        assert_eq!(status.phase, Phase::Failed);
        assert!(status.code.is_some_and(Code::network), "{status:?}");
        assert_eq!(installed(&case), OLD);
        only_the_app_is_left(&case);
    }

    #[tokio::test]
    async fn a_file_that_does_not_unpack_leaves_the_file() {
        let broken = "#!/bin/sh\nexit 3\n";
        let (base, _) = Site::serve("v0.2.0", broken).await;
        let case = case("broken", &base, "0.1.0");
        case.updater.turn(NOW).await;
        assert_eq!(case.updater.status().code, Some(Code::UpdateBroken));
        assert_eq!(installed(&case), OLD);
        only_the_app_is_left(&case);
    }

    #[tokio::test]
    async fn a_server_that_is_down_changes_nothing_and_is_tried_again_soon() {
        let case = case("down", "http://127.0.0.1:1/releases", "0.1.0");
        let wait = case.updater.turn(NOW).await;
        let status = case.updater.status();
        assert_eq!(status.phase, Phase::Failed);
        assert!(status.code.is_some_and(Code::network), "{status:?}");
        assert_eq!(status.checked, 0, "no answer does not count as the day's check");
        assert_eq!(wait, HOUR);
        assert_eq!(installed(&case), OLD);
        only_the_app_is_left(&case);
    }

    #[tokio::test]
    async fn a_read_only_folder_is_a_status_and_nothing_is_downloaded() {
        let (base, site) = Site::serve("v0.2.0", &appimage("0.2.0")).await;
        let case = case("readonly", &base, "0.1.0");
        let dir = case.target.parent().unwrap().to_path_buf();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        case.updater.turn(NOW).await;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(case.updater.status().code, Some(Code::UpdatePlace));
        assert_eq!(installed(&case), OLD);
        assert_eq!(site.asked_downloads(), 0, "the 56 MB are not fetched for nothing");
    }

    #[tokio::test]
    async fn a_page_without_a_version_is_a_status() {
        let (base, _) = Site::serve("releases", "").await;
        let case = case("norelease", &base, "0.1.0");
        case.updater.turn(NOW).await;
        assert_eq!(case.updater.status().code, Some(Code::UpdateNoRelease));
        assert_eq!(installed(&case), OLD);
    }

    #[tokio::test]
    async fn turned_off_it_makes_no_request() {
        let (base, site) = Site::serve("v0.2.0", &appimage("0.2.0")).await;
        let case = case("off", &base, "0.1.0");
        settings::AUTO_UPDATE.set(&case.updater.library, &false);
        case.updater.turn(NOW).await;
        assert!(site.asked.lock().unwrap().is_empty());
        assert_eq!(case.updater.status().phase, Phase::Off);
        assert_eq!(installed(&case), OLD);
        settings::AUTO_UPDATE.set(&case.updater.library, &true);
        case.updater.turn(NOW).await;
        assert_eq!(case.updater.status().phase, Phase::Ready, "back on, it goes at once");
    }

    #[tokio::test]
    async fn it_asks_at_most_once_a_day() {
        let (base, site) = Site::serve("v0.1.0", &appimage("x")).await;
        let case = case("daily", &base, "0.1.0");
        let day = DAY.as_millis() as u64;
        case.updater.turn(NOW).await;
        let once = site.asked.lock().unwrap().len();
        assert!(once > 0);
        let wait = case.updater.turn(NOW + 1000).await;
        assert!(wait <= HOUR, "the wait stays short enough to survive a suspend: {wait:?}");
        case.updater.turn(NOW + day - 1).await;
        assert_eq!(site.asked.lock().unwrap().len(), once, "less than a day later");
        case.updater.turn(NOW + day).await;
        assert!(site.asked.lock().unwrap().len() > once, "a day later");
        let later = site.asked.lock().unwrap().len();
        settings::UPDATE_CHECKED.set(&case.updater.library, &(NOW * 2));
        case.updater.turn(NOW + day + 1).await;
        assert!(site.asked.lock().unwrap().len() > later, "a clock set back does not stop the checks");
    }

    fn asked_for_latest(site: &Site) -> usize {
        site.asked.lock().unwrap().iter().filter(|path| path.ends_with("/latest")).count()
    }

    #[tokio::test]
    async fn check_now_ignores_the_daily_limit() {
        let (base, site) = Site::serve("v0.1.0", &appimage("x")).await;
        let case = case("manual", &base, "0.1.0");
        case.updater.turn(NOW).await;
        case.updater.turn(NOW + 1).await;
        assert_eq!(asked_for_latest(&site), 1);
        assert!(case.updater.check_now());
        assert_eq!(case.updater.status().phase, Phase::Checking, "the row changes at once");
        case.updater.turn(NOW + 2).await;
        assert_eq!(asked_for_latest(&site), 2);
        assert_eq!(case.updater.status().phase, Phase::Current);
        assert_eq!(case.updater.status().checked, NOW + 2);
    }

    #[tokio::test]
    async fn repeat_clicks_do_not_start_a_second_run() {
        let (base, site) = Site::serve("v0.1.0", &appimage("x")).await;
        let case = case("repeat", &base, "0.1.0");
        assert!(case.updater.check_now());
        assert!(!case.updater.check_now());
        assert!(!case.updater.check_now());
        case.updater.turn(NOW).await;
        case.updater.turn(NOW + 1).await;
        assert_eq!(asked_for_latest(&site), 1);
        assert!(case.updater.check_now(), "free again once it finished");
    }

    #[tokio::test]
    async fn a_click_works_with_the_switch_off_and_nothing_happens_by_itself() {
        let body = appimage("0.2.0");
        let (base, site) = Site::serve("v0.2.0", &body).await;
        let case = case("offclick", &base, "0.1.0");
        settings::AUTO_UPDATE.set(&case.updater.library, &false);
        case.updater.turn(NOW).await;
        case.updater.turn(NOW + 2 * DAY.as_millis() as u64).await;
        assert!(site.asked.lock().unwrap().is_empty(), "off asks nothing by itself");
        assert_eq!(case.updater.status().phase, Phase::Off);
        assert!(case.updater.check_now());
        assert_eq!(case.updater.status().phase, Phase::Checking, "not hidden by the switch");
        case.updater.turn(NOW + 3 * DAY.as_millis() as u64).await;
        assert_eq!(case.updater.status().phase, Phase::Ready, "the click downloads and places it");
        assert_eq!(installed(&case), body);
        assert!(!case.updater.check_now(), "nothing to check once it waits for a restart");
    }

    #[tokio::test]
    async fn the_switch_going_off_does_not_stop_a_click() {
        let body = appimage("0.2.0");
        let (base, _) = Site::serve("v0.2.0", &body).await;
        let case = case("offduring", &base, "0.1.0");
        assert!(case.updater.check_now());
        settings::AUTO_UPDATE.set(&case.updater.library, &false);
        case.updater.toggled();
        case.updater.turn(NOW).await;
        assert_eq!(installed(&case), body);
    }

    #[tokio::test]
    async fn the_state_moves_from_checking_through_downloading_to_ready() {
        let body = format!("{}# {}\n", appimage("0.2.0"), "x".repeat(1 << 20));
        let (base, _) = Site::serve("v0.2.0", &body).await;
        let case = case("states", &base, "0.1.0");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (sink, updater) = (seen.clone(), case.updater.clone());
        let watching = tokio::spawn(async move {
            loop {
                let phase = updater.status().phase;
                {
                    let mut seen = sink.lock().unwrap();
                    if seen.last() != Some(&phase) {
                        seen.push(phase);
                    }
                }
                if phase == Phase::Ready {
                    return;
                }
                tokio::task::yield_now().await;
            }
        });
        case.updater.check_now();
        case.updater.turn(NOW).await;
        watching.await.unwrap();
        let seen = seen.lock().unwrap();
        assert!(seen.contains(&Phase::Checking) && seen.last() == Some(&Phase::Ready), "{seen:?}");
    }

    #[tokio::test]
    async fn a_failed_click_shows_why_and_the_next_click_runs() {
        let case = case("failclick", "http://127.0.0.1:1/releases", "0.1.0");
        assert!(case.updater.check_now());
        case.updater.turn(NOW).await;
        let status = case.updater.status();
        assert_eq!(status.phase, Phase::Failed);
        assert!(status.code.is_some_and(Code::network), "{status:?}");
        assert!(case.updater.check_now());
    }

    #[tokio::test]
    async fn restart_is_only_for_a_placed_update() {
        let body = appimage("0.2.0");
        let (base, _) = Site::serve("v0.2.0", &body).await;
        let case = case("restart", &base, "0.1.0");
        assert!(!case.updater.restart(), "nothing is placed yet");
        case.updater.turn(NOW).await;
        assert_eq!(case.updater.status().phase, Phase::Ready);
        assert!(case.updater.restart());
    }

    #[tokio::test]
    async fn outside_the_layout_a_click_does_nothing() {
        let (base, site) = Site::serve("v0.2.0", &appimage("0.2.0")).await;
        let library = Arc::new(Library::open(Path::new(":memory:")).unwrap());
        let ui: Ui = Arc::new(|_| {});
        let updater = Updater::new(library, Route::default(), ui, Place::Development, Some(public(signer())), base, "0.1.0");
        assert!(!updater.check_now());
        assert!(!updater.restart());
        updater.turn(NOW).await;
        assert!(site.asked.lock().unwrap().is_empty());
        assert_eq!(updater.status().phase, Phase::Development);
    }

    #[tokio::test]
    async fn a_new_file_already_in_place_is_not_fetched_twice() {
        let body = appimage("0.2.0");
        let (base, site) = Site::serve("v0.2.0", &body).await;
        let case = case("placed", &base, "0.1.0");
        std::fs::write(&case.target, &body).unwrap();
        case.updater.turn(NOW).await;
        assert_eq!(case.updater.status().phase, Phase::Ready);
        assert_eq!(site.asked_downloads(), 0);
    }

    #[tokio::test]
    async fn only_the_installers_layout_updates_itself() {
        let (base, site) = Site::serve("v0.2.0", &appimage("0.2.0")).await;
        let library = || Arc::new(Library::open(Path::new(":memory:")).unwrap());
        let ui = || -> Ui { Arc::new(|_| {}) };
        for (place, phase) in [(Place::Development, Phase::Development), (Place::Elsewhere("/tmp/x.AppImage".into()), Phase::Elsewhere)] {
            let updater = Updater::new(library(), Route::default(), ui(), place, Some(public(signer())), base.clone(), "0.1.0");
            updater.turn(NOW).await;
            assert_eq!(updater.status().phase, phase);
            settings::AUTO_UPDATE.set(&updater.library, &false);
            assert_eq!(updater.status().phase, phase, "the reason stays when the switch is off");
        }
        assert!(site.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn the_layout_is_read_from_appimage_and_the_home_of_this_user() {
        let dir = folder("layout");
        let target = dir.join("versions").join(ASSET);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, OLD).unwrap();
        let canonical = target.canonicalize().unwrap();
        assert_eq!(place_of(None, Some(target.clone())), Place::Development);
        assert_eq!(place_of(Some(target.clone().into()), Some(target.clone())), Place::Installed(canonical.clone()));
        let stamped = dir.join("versions/Snellar-0.1.0-beta-20261105-0257-12ca20d.AppImage");
        std::fs::write(&stamped, OLD).unwrap();
        assert_eq!(
            place_of(Some(stamped.clone().into()), Some(target.clone())),
            Place::Elsewhere(stamped.display().to_string()),
            "release.sh"
        );
        let downloads = dir.join(ASSET);
        std::fs::write(&downloads, OLD).unwrap();
        assert_eq!(place_of(Some(downloads.clone().into()), Some(target.clone())), Place::Elsewhere(downloads.display().to_string()));
        let home = dir.join("linked-home");
        std::os::unix::fs::symlink(&dir, &home).unwrap();
        assert_eq!(
            place_of(Some(target.clone().into()), Some(home.join("versions").join(ASSET))),
            Place::Installed(canonical),
            "a home behind a symlink"
        );
        let link = dir.join("versions/link.AppImage");
        std::os::unix::fs::symlink(&downloads, &link).unwrap();
        assert_eq!(
            place_of(Some(downloads.clone().into()), Some(link)),
            Place::Elsewhere(downloads.display().to_string()),
            "a link in the target's place"
        );
    }

    #[test]
    fn old_extractions_and_leftovers_are_swept_and_the_running_one_stays() {
        let dir = folder("sweep");
        let target = dir.join("versions").join(ASSET);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, OLD).unwrap();
        let run = dir.join("run");
        let (own, old) = (run.join(format!("{EXTRACTED}aaa")), run.join(format!("{EXTRACTED}bbb")));
        std::fs::create_dir_all(own.join("usr")).unwrap();
        std::fs::create_dir_all(old.join("usr")).unwrap();
        std::fs::write(run.join("a-temp-file"), "x").unwrap();
        std::fs::write(part_of(&target), "half").unwrap();
        std::fs::create_dir_all(check_dir_of(&target)).unwrap();
        tidy(&target);
        sweep_extractions(&target, &own);
        assert!(own.join("usr").is_dir(), "the one in use");
        assert!(!old.exists(), "the one before it");
        assert!(run.join("a-temp-file").exists(), "only extractions");
        assert!(!part_of(&target).exists() && !check_dir_of(&target).exists());
        assert!(target.exists());
    }

    #[tokio::test]
    async fn a_download_stops_when_the_switch_goes_off() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/releases", listener.local_addr().unwrap());
        let body = appimage("0.2.0");
        let sum = format!("{}  {ASSET}\n", sha(&body));
        let sig = sign(&sum);
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let (sum, sig) = (sum.clone(), sig.clone());
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let path = line.split_whitespace().nth(1).unwrap_or_default().to_string();
                    if path.ends_with("/latest") {
                        let reply =
                            "HTTP/1.1 302 Found\r\nLocation: /releases/tag/v0.2.0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        reader.get_mut().write_all(reply.as_bytes()).await.ok();
                    } else if path.ends_with(".sig") {
                        let mut reply =
                            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", sig.len()).into_bytes();
                        reply.extend_from_slice(&sig);
                        reader.get_mut().write_all(&reply).await.ok();
                    } else if path.ends_with("sha256") {
                        let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sum}", sum.len());
                        reader.get_mut().write_all(reply.as_bytes()).await.ok();
                    } else if path.ends_with(ASSET) {
                        reader.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nstart").await.ok();
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    } else {
                        reader.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.ok();
                    }
                });
            }
        });
        let case = case("stop", &base, "0.1.0");
        let updater = case.updater.clone();
        let turning = tokio::spawn(async move { updater.turn(NOW).await });
        while !part_of(&case.target).exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        settings::AUTO_UPDATE.set(&case.updater.library, &false);
        case.updater.toggled();
        tokio::time::timeout(Duration::from_secs(5), turning).await.expect("it stops").unwrap();
        assert_eq!(case.updater.status().phase, Phase::Off);
        assert_eq!(installed(&case), OLD);
        only_the_app_is_left(&case);
    }

    const OPENSSL_PEM: &str =
        "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAYHIMr10iqqGpAWLG+8sDG8zBjs2aqlDnZuCAhH6oqo0=\n-----END PUBLIC KEY-----\n";
    const OPENSSL_RAW: &str = "60720caf5d22aaa1a90162c6fbcb031bccc18ecd9aaa50e766e080847ea8aa8d";
    const OPENSSL_SUMS: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  Snellar-x86_64.AppImage\n";
    const OPENSSL_SIG: &str = "eXXH96yL8M6EBesAqLreDIC7RYuKyeAg2lGP5FLE8ToT/C1XqahbZzBDdEmKOESRZFuF6LSOWJ+gzgoO3zLRAA==";

    fn hex_of(key: &Key) -> String {
        key.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn a_key_from_openssl_reads_and_its_signature_verifies() {
        let key = key_of(OPENSSL_PEM).expect("the PEM openssl writes");
        assert_eq!(hex_of(&key), OPENSSL_RAW);
        let sig = STANDARD.decode(OPENSSL_SIG).unwrap();
        assert!(signed(&key, OPENSSL_SUMS.as_bytes(), &sig), "openssl pkeyutl -sign -rawin");
        assert_eq!(key_of(&OPENSSL_PEM.replace('\n', "\r\n")), Some(key), "CRLF lines");
    }

    #[test]
    fn what_is_not_an_ed25519_public_key_is_refused() {
        let x25519 = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VuAyEAYHIMr10iqqGpAWLG+8sDG8zBjs2aqlDnZuCAhH6oqo0=\n-----END PUBLIC KEY-----\n";
        let short = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAYHIMr10iqqGpAWLG+8sDG8zBjs2aqlDnZuCAhH6o\n-----END PUBLIC KEY-----\n";
        let private = OPENSSL_PEM.replace("PUBLIC", "PRIVATE");
        let unended = OPENSSL_PEM.replace("-----END PUBLIC KEY-----\n", "");
        let two = format!("{OPENSSL_PEM}{OPENSSL_PEM}");
        let garbled = OPENSSL_PEM.replace("MCow", "MC!w");
        for bad in ["", "none", x25519, short, private.as_str(), unended.as_str(), two.as_str(), garbled.as_str()] {
            assert_eq!(key_of(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_signature_holds_only_for_its_key_and_its_bytes() {
        let (key, other) = (public(signer()), public(&pair()));
        let sum = format!("{}  {ASSET}\n", sha("x"));
        let sig = sign(&sum);
        assert!(signed(&key, sum.as_bytes(), &sig));
        assert!(!signed(&other, sum.as_bytes(), &sig), "another key");
        assert!(!signed(&key, sum.replace('a', "b").as_bytes(), &sig), "a changed checksum");
        assert!(!signed(&key, format!("{sum}\n").as_bytes(), &sig), "one more byte");
        let mut flipped = sig.clone();
        flipped[10] ^= 1;
        assert!(!signed(&key, sum.as_bytes(), &flipped), "a changed signature");
        assert!(!signed(&key, sum.as_bytes(), &[]), "an empty one");
        assert!(!signed(&key, sum.as_bytes(), &sig[..63]), "a short one");
    }

    #[test]
    fn the_key_file_of_the_tree_is_the_key_of_the_build() {
        assert_eq!(compiled_key().is_some(), !KEY_PEM.is_empty(), "a key file that is there must read");
        assert_eq!(compiled_key_hex().len(), if KEY_PEM.is_empty() { 4 } else { 64 });
    }

    async fn refused(name: &str, spoil: impl FnOnce(&Site)) {
        let body = appimage("0.2.0");
        let (base, site) = Site::serve("v0.2.0", &body).await;
        spoil(&site);
        let case = case(name, &base, "0.1.0");
        case.updater.turn(NOW).await;
        let status = case.updater.status();
        assert_eq!((status.phase, status.code), (Phase::Failed, Some(Code::UpdateSignature)), "{name}");
        assert_eq!(installed(&case), OLD, "{name}");
        assert_eq!(site.asked_downloads(), 0, "{name}: nothing is fetched on a checksum it cannot trust");
        assert!(!previous_of(&case.target).exists(), "{name}");
        only_the_app_is_left(&case);
        site.publish("v0.2.0", &body);
        case.updater.turn(NOW + DAY.as_millis() as u64).await;
        assert_eq!(case.updater.status().phase, Phase::Ready, "{name}: a good release the next day goes in");
        assert_eq!(installed(&case), body);
    }

    #[tokio::test]
    async fn a_missing_signature_is_refused() {
        refused("nosig", |site| *site.sig.lock().unwrap() = None).await;
    }

    #[tokio::test]
    async fn a_signature_that_is_not_one_is_refused() {
        refused("badsig", |site| *site.sig.lock().unwrap() = Some(vec![7; 64])).await;
        refused("emptysig", |site| *site.sig.lock().unwrap() = Some(Vec::new())).await;
    }

    #[tokio::test]
    async fn a_signature_by_another_key_is_refused() {
        refused("otherkey", |site| {
            let sum = site.sum.lock().unwrap().clone();
            *site.sig.lock().unwrap() = Some(pair().sign(sum.as_bytes()).as_ref().to_vec());
        })
        .await;
    }

    #[tokio::test]
    async fn a_checksum_changed_after_signing_is_refused() {
        refused("tampered", |site| {
            let evil = appimage("evil");
            *site.sum.lock().unwrap() = format!("{}  {ASSET}\n", sha(&evil));
            *site.body.lock().unwrap() = evil;
        })
        .await;
    }

    #[tokio::test]
    async fn a_build_without_a_key_does_not_update_itself() {
        let (base, site) = Site::serve("v0.2.0", &appimage("0.2.0")).await;
        let library = Arc::new(Library::open(Path::new(":memory:")).unwrap());
        let root = folder("nokey");
        let target = root.join(ASSET);
        std::fs::write(&target, OLD).unwrap();
        let updater = Updater::new(library, Route::default(), Arc::new(|_| {}), Place::Installed(target.clone()), None, base, "0.1.0");
        assert_eq!(updater.status().phase, Phase::Unsigned);
        assert!(!updater.check_now(), "no button");
        assert_eq!(updater.turn(NOW).await, DAY);
        assert!(!updater.restart());
        settings::AUTO_UPDATE.set(&updater.library, &false);
        assert_eq!(updater.status().phase, Phase::Unsigned, "the reason stays when the switch is off");
        assert!(site.asked.lock().unwrap().is_empty(), "nothing is asked");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), OLD);
    }
}
