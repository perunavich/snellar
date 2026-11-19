use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ring::digest;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use snellar_core::error::Code;
use snellar_core::library::Library;
use snellar_core::{clock, journal, product, settings};
use snellar_youtube::net::{Http, Route, checked, net};

const RELEASES: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
const SUMS: &str = "SHA2-256SUMS";
const URL_ENV: &str = "SNELLAR_YTDLP_URL";
const SIZE_LIMIT: u64 = 256 << 20;
const RECHECK: Duration = Duration::from_secs(24 * 60 * 60);
const CONNECT_LIMIT: Duration = Duration::from_secs(10);
const VERSION_LIMIT: Duration = Duration::from_secs(30);
const PROGRESS_STEP: u8 = 10;
const BUSY_TRIES: u32 = 10;
const BUSY_WAIT: Duration = Duration::from_millis(50);

pub type Progress<'a> = dyn Fn(u8) + Send + Sync + 'a;

static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Outcome {
    Installed,
    Updated,
    Current,
}

fn asset() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("yt-dlp_linux"),
        "aarch64" => Some("yt-dlp_linux_aarch64"),
        _ => None,
    }
}

pub fn published<'a>(sums: &'a str, asset: &str) -> Option<&'a str> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == asset && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
    })
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash_of(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut context = digest::Context::new(&digest::SHA256);
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buffer).ok()?;
        if n == 0 {
            return Some(hex(context.finish().as_ref()));
        }
        context.update(&buffer[..n]);
    }
}

pub async fn text(client: &reqwest::Client, url: &str) -> Result<String> {
    checked(client.get(url).send().await.map_err(net)?)?.text().await.map_err(net)
}

pub async fn download(client: &reqwest::Client, url: &str, part: &Path, progress: &Progress<'_>) -> Result<String> {
    let mut response = checked(client.get(url).send().await.map_err(net)?)?;
    let total = response.content_length().unwrap_or(0);
    let mut told = 0;
    progress(0);
    let mut file = tokio::fs::File::create(part).await.with_context(|| format!("cannot create {}", part.display()))?;
    let mut context = digest::Context::new(&digest::SHA256);
    let mut size = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(net)? {
        size += chunk.len() as u64;
        if size > SIZE_LIMIT {
            bail!("the download is larger than {SIZE_LIMIT} bytes");
        }
        context.update(&chunk);
        file.write_all(&chunk).await?;
        let percent = (size * 100).checked_div(total).unwrap_or(0).min(100) as u8;
        if percent >= told + PROGRESS_STEP {
            told = percent - percent % PROGRESS_STEP;
            progress(told);
        }
    }
    file.flush().await?;
    Ok(hex(context.finish().as_ref()))
}

pub async fn output_of(bin: &Path, flag: &str) -> Option<String> {
    let mut command = Command::new(bin);
    command.arg(flag);
    output(command).await
}

pub async fn output(mut command: Command) -> Option<String> {
    command.env_remove("LD_LIBRARY_PATH").stdin(Stdio::null()).stderr(Stdio::null());
    for _ in 0..BUSY_TRIES {
        let run = command.output();
        match tokio::time::timeout(VERSION_LIMIT, run).await.ok()? {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => tokio::time::sleep(BUSY_WAIT).await,
            Err(_) => return None,
            Ok(out) => return out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned()),
        }
    }
    None
}

pub async fn version(bin: &Path) -> Option<String> {
    Some(output_of(bin, "--version").await?.lines().next().unwrap_or_default().trim().to_string())
}

async fn place(
    client: &reqwest::Client,
    base: &str,
    asset: &str,
    want: &str,
    part: &Path,
    target: &Path,
    progress: &Progress<'_>,
) -> Result<()> {
    let got = download(client, &format!("{base}/{asset}"), part, progress).await?;
    if got != want {
        return Err(Code::YtdlpChecksum.err(format!("{asset}: expected {want}, got {got}")));
    }
    std::fs::set_permissions(part, std::fs::Permissions::from_mode(0o755))?;
    if version(part).await.is_none() {
        return Err(Code::YtdlpFailed.err(format!("{asset} does not run here")));
    }
    std::fs::rename(part, target).with_context(|| format!("cannot place {}", target.display()))
}

pub async fn fetch(client: &reqwest::Client, base: &str, asset: &str, target: &Path, progress: &Progress<'_>) -> Result<Outcome> {
    let sums = text(client, &format!("{base}/{SUMS}")).await?;
    let want =
        published(&sums, asset).ok_or_else(|| Code::YtdlpChecksum.err(format!("{SUMS} does not list {asset}")))?.to_ascii_lowercase();
    let had = target.is_file();
    if hash_of(target).as_deref() == Some(want.as_str()) {
        return Ok(Outcome::Current);
    }
    let dir = target.parent().context("the tool has no folder")?;
    tokio::fs::create_dir_all(dir).await.with_context(|| format!("cannot create {}", dir.display()))?;
    let part: PathBuf = target.with_extension("part");
    let placed = place(client, base, asset, &want, &part, target, progress).await;
    if placed.is_err() {
        std::fs::remove_file(&part).ok();
    }
    placed.map(|()| if had { Outcome::Updated } else { Outcome::Installed })
}

pub fn needed(library: &Library, target: &Path, own: Option<&Path>, now: u64) -> bool {
    if !target.is_file() {
        return own.is_none();
    }
    let checked: u64 = settings::YTDLP_CHECKED.or_default(library);
    now.saturating_sub(checked) >= RECHECK.as_millis() as u64
}

pub fn due(library: &Library) -> bool {
    needed(library, &product::managed_ytdlp(), product::own_ytdlp().as_deref(), clock::unix_millis())
}

pub async fn keep(library: &Library, route: &Route, asked: bool, progress: &Progress<'_>) -> Result<Option<Outcome>> {
    let Some(asset) = asset() else { return Ok(None) };
    let _alone = ONE_AT_A_TIME.lock().await;
    let target = product::managed_ytdlp();
    let own = product::own_ytdlp();
    let wanted = if asked { target.is_file() || own.is_none() } else { needed(library, &target, own.as_deref(), clock::unix_millis()) };
    if !wanted {
        return Ok(None);
    }
    let base = std::env::var(URL_ENV).unwrap_or_else(|_| RELEASES.to_string());
    let http = Http::new(route, |builder| builder.connect_timeout(CONNECT_LIMIT));
    let outcome = fetch(&http.client(), &base, asset, &target, progress).await?;
    settings::YTDLP_CHECKED.set(library, &clock::unix_millis());
    journal::info("tools", format!("yt-dlp {outcome:?}"));
    Ok(Some(outcome))
}
