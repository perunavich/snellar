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
use snellar_core::sync::Lock;
use snellar_core::ui::{Ui, UiEvent};
use snellar_core::{clock, journal, product, settings};
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
    compiled_key().map_or_else(|| "none".to_string(), |key| ytdlp::hex(&key))
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
            let wait = self.turn(clock::unix_millis()).await;
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
            let mut status = self.status.locked();
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
            let mut status = self.status.locked();
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
        let ready = self.status.locked().phase == Phase::Ready && target.is_file();
        ready && (RELAUNCH.set(target.clone()).is_ok() || RELAUNCH.get() == Some(target))
    }

    pub fn status(&self) -> Status {
        let mut status = self.status.locked().clone();
        status.checked = settings::UPDATE_CHECKED.or_default(&self.library);
        if matches!(self.place, Place::Installed(_)) && status.phase == Phase::Idle && !self.on() {
            status.phase = Phase::Off;
        }
        status
    }

    fn show(&self, change: impl FnOnce(&mut Status)) {
        change(&mut self.status.locked());
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
