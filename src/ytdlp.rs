use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ring::digest;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use snellar_core::error::Code;
use snellar_core::library::Library;
use snellar_core::{journal, product, settings};
use snellar_youtube::net::{Http, Route, net};

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

fn hex(bytes: &[u8]) -> String {
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
    client.get(url).send().await.and_then(reqwest::Response::error_for_status).map_err(net)?.text().await.map_err(net)
}

pub async fn download(client: &reqwest::Client, url: &str, part: &Path, progress: &Progress<'_>) -> Result<String> {
    let mut response = client.get(url).send().await.and_then(reqwest::Response::error_for_status).map_err(net)?;
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

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub fn needed(library: &Library, target: &Path, own: Option<&Path>, now: u64) -> bool {
    if !target.is_file() {
        return own.is_none();
    }
    let checked: u64 = settings::YTDLP_CHECKED.or_default(library);
    now.saturating_sub(checked) >= RECHECK.as_millis() as u64
}

pub fn due(library: &Library) -> bool {
    needed(library, &product::managed_ytdlp(), product::own_ytdlp().as_deref(), now())
}

pub async fn keep(library: &Library, route: &Route, asked: bool, progress: &Progress<'_>) -> Result<Option<Outcome>> {
    let Some(asset) = asset() else { return Ok(None) };
    let _alone = ONE_AT_A_TIME.lock().await;
    let target = product::managed_ytdlp();
    let own = product::own_ytdlp();
    let wanted = if asked { target.is_file() || own.is_none() } else { needed(library, &target, own.as_deref(), now()) };
    if !wanted {
        return Ok(None);
    }
    let base = std::env::var(URL_ENV).unwrap_or_else(|_| RELEASES.to_string());
    let http = Http::new(route, |builder| builder.connect_timeout(CONNECT_LIMIT));
    let outcome = fetch(&http.client(), &base, asset, &target, progress).await?;
    settings::YTDLP_CHECKED.set(library, &now());
    journal::info("tools", format!("yt-dlp {outcome:?}"));
    Ok(Some(outcome))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::net::TcpListener;

    use super::*;

    const ASSET: &str = "yt-dlp_linux";
    const GOOD: &str = "#!/bin/sh\necho 2026.11.01\n";
    const NEWER: &str = "#!/bin/sh\necho 2026.12.01\n";

    fn sha(body: &str) -> String {
        hex(digest::digest(&digest::SHA256, body.as_bytes()).as_ref())
    }

    #[derive(Clone)]
    struct Release {
        files: Arc<Mutex<Vec<(String, String)>>>,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Release {
        async fn serve(sums: String, body: &str) -> (String, Release) {
            let release = Release {
                files: Arc::new(Mutex::new(vec![(SUMS.to_string(), sums), (ASSET.to_string(), body.to_string())])),
                asked: Arc::default(),
            };
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let served = release.clone();
            tokio::spawn(async move {
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    let served = served.clone();
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        reader.read_line(&mut line).await.unwrap();
                        let name = line.split_whitespace().nth(1).unwrap_or_default().trim_start_matches('/').to_string();
                        served.asked.lock().unwrap().push(name.clone());
                        let file = served.files.lock().unwrap().iter().find(|(n, _)| *n == name).map(|(_, b)| b.clone());
                        let reply = match file {
                            Some(body) => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()),
                            None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                        };
                        reader.get_mut().write_all(reply.as_bytes()).await.unwrap();
                    });
                }
            });
            (base, release)
        }

        fn publish(&self, body: &str) {
            let mut files = self.files.lock().unwrap();
            files[0].1 = format!("{}  {ASSET}\n", sha(body));
            files[1].1 = body.to_string();
        }
    }

    fn place_in(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-ytdlp-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir.join("tools").join("yt-dlp")
    }

    fn sums_for(body: &str) -> String {
        format!("0000000000000000000000000000000000000000000000000000000000000000  yt-dlp\n{}  {ASSET}\n", sha(body))
    }

    #[tokio::test]
    async fn it_installs_checks_and_updates_from_the_release() {
        let (base, release) = Release::serve(sums_for(GOOD), GOOD).await;
        let target = place_in("flow");
        let client = reqwest::Client::new();
        assert_eq!(fetch(&client, &base, ASSET, &target, &|_| {}).await.unwrap(), Outcome::Installed);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), GOOD);
        assert_eq!(version(&target).await.as_deref(), Some("2026.11.01"), "executable and it runs");
        release.asked.lock().unwrap().clear();
        assert_eq!(fetch(&client, &base, ASSET, &target, &|_| {}).await.unwrap(), Outcome::Current);
        assert_eq!(*release.asked.lock().unwrap(), [SUMS], "the same version is not downloaded again");
        release.publish(NEWER);
        assert_eq!(fetch(&client, &base, ASSET, &target, &|_| {}).await.unwrap(), Outcome::Updated);
        assert_eq!(version(&target).await.as_deref(), Some("2026.12.01"));
        assert!(!target.with_extension("part").exists());
    }

    #[tokio::test]
    async fn a_real_download_says_how_far_it_is_and_a_current_copy_says_nothing() {
        let body = format!("#!/bin/sh\necho 2026.11.01\n# {}\n", "x".repeat(1 << 20));
        let (base, _) = Release::serve(sums_for(&body), &body).await;
        let target = place_in("progress");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let tell = move |percent| sink.lock().unwrap().push(percent);
        let client = reqwest::Client::new();
        fetch(&client, &base, ASSET, &target, &tell).await.unwrap();
        let steps = seen.lock().unwrap().clone();
        assert_eq!((steps.first(), steps.last()), (Some(&0), Some(&100)), "{steps:?}");
        assert!(steps.windows(2).all(|w| w[0] < w[1]), "{steps:?}");
        seen.lock().unwrap().clear();
        assert_eq!(fetch(&client, &base, ASSET, &target, &tell).await.unwrap(), Outcome::Current);
        assert!(seen.lock().unwrap().is_empty(), "nothing to show when nothing comes down");
    }

    #[tokio::test]
    async fn a_wrong_checksum_installs_nothing_and_keeps_the_old_one() {
        let target = place_in("bad");
        let client = reqwest::Client::new();
        let (base, _) = Release::serve(sums_for("something else"), GOOD).await;
        assert_eq!(Code::of(&fetch(&client, &base, ASSET, &target, &|_| {}).await.unwrap_err()), Code::YtdlpChecksum);
        assert!(!target.exists() && !target.with_extension("part").exists(), "nothing is left");
        std::fs::write(&target, GOOD).unwrap();
        assert_eq!(Code::of(&fetch(&client, &base, ASSET, &target, &|_| {}).await.unwrap_err()), Code::YtdlpChecksum);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), GOOD, "the working one stays");
        assert!(!target.with_extension("part").exists());
    }

    #[tokio::test]
    async fn a_release_that_does_not_list_the_file_is_refused_before_downloading() {
        let target = place_in("unlisted");
        let (base, release) = Release::serve("abc  yt-dlp_linux\nnot a line\n".into(), GOOD).await;
        let err = fetch(&reqwest::Client::new(), &base, ASSET, &target, &|_| {}).await.unwrap_err();
        assert_eq!(Code::of(&err), Code::YtdlpChecksum);
        assert_eq!(*release.asked.lock().unwrap(), [SUMS]);
    }

    #[tokio::test]
    async fn a_file_that_does_not_run_is_not_installed() {
        let target = place_in("broken");
        let broken = "not a program";
        let (base, _) = Release::serve(sums_for(broken), broken).await;
        assert_eq!(Code::of(&fetch(&reqwest::Client::new(), &base, ASSET, &target, &|_| {}).await.unwrap_err()), Code::YtdlpFailed);
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn no_network_is_an_error_and_touches_nothing() {
        let target = place_in("offline");
        let err = fetch(&reqwest::Client::new(), "http://127.0.0.1:1", ASSET, &target, &|_| {}).await.unwrap_err();
        assert!(Code::of(&err).as_str().starts_with("net."), "{err:#}");
        assert!(!target.exists());
    }

    #[test]
    fn it_is_fetched_only_when_absent_or_a_day_old_and_never_over_a_persons_own() {
        let library = Library::open(Path::new(":memory:")).unwrap();
        let target = place_in("when");
        let own = Path::new("/usr/bin/yt-dlp");
        assert!(needed(&library, &target, None, 1), "nothing here");
        assert!(!needed(&library, &target, Some(own), 1), "their own is used and left alone");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, GOOD).unwrap();
        let day = RECHECK.as_millis() as u64;
        settings::YTDLP_CHECKED.set(&library, &(5 * day));
        assert!(!needed(&library, &target, None, 5 * day + 1));
        assert!(needed(&library, &target, Some(own), 6 * day), "ours stays up to date");
    }

    #[test]
    fn the_window_fetches_nothing_when_it_starts() {
        let main = include_str!("../qml/Main.qml");
        for call in ["keepTools", "fetchYtdlp"] {
            assert!(!main.contains(call), "{call}: the fetch belongs to Download and the Dependencies button");
        }
    }

    #[test]
    fn the_checksum_list_is_read_by_exact_name() {
        let hash = "a".repeat(64);
        let sums = format!("{}  yt-dlp\n{}  yt-dlp_linux_aarch64\n{hash} *yt-dlp_linux\n", "b".repeat(64), "c".repeat(64));
        assert_eq!(published(&sums, "yt-dlp_linux"), Some(hash.as_str()));
        assert_eq!(published(&sums, "yt-dlp_macos"), None);
    }
}
