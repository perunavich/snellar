use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::process::Command;

use snellar_core::error::Code;
use snellar_core::library::Library;
use snellar_core::model::Track;
use snellar_core::settings;
use snellar_core::ui::{Toast, Ui, UiEvent};
use snellar_youtube::stream::Resolver;
use snellar_youtube::ytm;

const DOWNLOAD_LIMIT: Duration = Duration::from_secs(15 * 60);
const AUDIO: [&str; 7] = ["opus", "m4a", "webm", "ogg", "mp3", "flac", "aac"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    name: String,
    path: String,
    size: u64,
    modified: u64,
}

pub struct Downloads {
    ui: Ui,
    library: Arc<Library>,
    bin: Option<PathBuf>,
    resolver: Arc<Resolver>,
    limit: Duration,
}

fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '/' || c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string()
}

fn meta(value: &str) -> String {
    let text = value.replace('%', "%%").replace(':', "\\:");
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphabetic() || b == b'_') {
        format!("{text}%({}|)s", snellar_core::product::ID)
    } else {
        text
    }
}

fn options(ffmpeg: bool, thumbnail: bool) -> Vec<&'static str> {
    let mut args = vec!["-f", "bestaudio", "--no-playlist", "--no-warnings", "--print", "after_move:filepath"];
    if ffmpeg {
        args.extend(["-x", "--audio-format", "best", "--embed-metadata"]);
    }
    if ffmpeg && thumbnail {
        args.push("--embed-thumbnail");
    }
    args
}

async fn has_mutagen(bin: &Path) -> bool {
    let start = std::fs::read(bin).unwrap_or_default();
    if start.starts_with(b"\x7fELF") {
        return true;
    }
    let shebang = String::from_utf8_lossy(&start).lines().next().and_then(|l| l.strip_prefix("#!").map(str::to_string));
    Command::new(shebang.as_deref().filter(|line| line.contains("python")).unwrap_or("python3"))
        .args(["-c", "import mutagen"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

impl Downloads {
    pub fn new(ui: Ui, library: Arc<Library>, resolver: Arc<Resolver>) -> Arc<Downloads> {
        Arc::new(Downloads { ui, library, bin: None, resolver, limit: DOWNLOAD_LIMIT })
    }

    fn dir(&self) -> PathBuf {
        let custom = settings::DOWNLOAD_DIR.get::<String>(&self.library).filter(|s| !s.is_empty());
        custom.map(PathBuf::from).unwrap_or_else(snellar_core::product::downloads_dir)
    }

    pub fn start(self: &Arc<Self>, track: Track) {
        let this = self.clone();
        tokio::spawn(async move {
            let label = format!("{} - {}", clean(&track.artist_line()), clean(&track.title));
            let toast = match this.fetch(&track, &label).await {
                Ok(()) => Toast::new(Code::DownloadDone),
                Err(e) if Code::of(&e) == Code::YtdlpMissing => Toast::new(Code::DownloadNoTool),
                Err(e) => Toast::failed(Code::DownloadFailed, &e),
            };
            (this.ui)(UiEvent::Toast(toast.arg(label).track(&track.id)));
        });
    }

    async fn fetch(&self, track: &Track, label: &str) -> Result<()> {
        let url = ytm::watch_url(&track.id).ok_or_else(|| Code::SourceUnsupported.err(format!("`{}` is not a YouTube track", track.id)))?;
        let dir = self.dir();
        tokio::fs::create_dir_all(&dir).await.with_context(|| format!("cannot create {}", dir.display()))?;
        let bin = self.bin.clone().unwrap_or_else(snellar_core::product::ytdlp_bin);
        let ffmpeg = snellar_core::product::ffmpeg_bin().is_some();
        let mut cmd = Command::new(&bin);
        cmd.args(options(ffmpeg, ffmpeg && has_mutagen(&bin).await));
        if let Some(proxy) = self.resolver.route().proxy() {
            cmd.arg("--proxy").arg(proxy);
        }
        if ffmpeg {
            cmd.arg("--parse-metadata").arg(format!("{}:%(meta_title)s", meta(&track.title)));
            cmd.arg("--parse-metadata").arg(format!("{}:%(meta_artist)s", meta(&track.artist_line())));
            if let Some(album) = &track.album {
                cmd.arg("--parse-metadata").arg(format!("{}:%(meta_album)s", meta(&album.name)));
            }
        }
        let running = cmd
            .arg("-o")
            .arg(dir.join(format!("{}.%(ext)s", label.replace('%', "%%"))))
            .arg(url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output();
        let out = tokio::time::timeout(self.limit, running)
            .await
            .map_err(|_| Code::YtdlpFailed.err(format!("yt-dlp did not finish in {} s", self.limit.as_secs())))?
            .with_context(|| format!("cannot start {}", bin.display()))
            .context(Code::YtdlpMissing)?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let cause = snellar_youtube::net::ytdlp_cause(&stderr).unwrap_or(Code::YtdlpFailed);
            return Err(cause.err(stderr.lines().rev().find(|l| l.contains("ERROR")).unwrap_or("yt-dlp failed").to_string()));
        }
        let path = saved_path(&String::from_utf8_lossy(&out.stdout))
            .ok_or_else(|| Code::YtdlpFailed.err("yt-dlp did not say where it saved the file"))?;
        let loudness = track.id.video_id().and_then(|v| self.resolver.format(v)).and_then(|f| f.loudness_db);
        self.library.set_download(&track.id, &path, loudness);
        Ok(())
    }

    pub fn list(&self) -> Vec<Download> {
        let Ok(entries) = std::fs::read_dir(self.dir()) else { return Vec::new() };
        let mut files: Vec<Download> = entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| AUDIO.contains(&x)))
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
                Some(Download {
                    name: e.path().file_stem()?.to_string_lossy().into(),
                    path: e.path().to_string_lossy().into(),
                    size: meta.len(),
                    modified,
                })
            })
            .collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.modified));
        files
    }
}

fn saved_path(stdout: &str) -> Option<PathBuf> {
    stdout.lines().rev().map(str::trim).find(|l| l.starts_with('/')).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_that_hangs_is_stopped_and_told() {
        let dir = std::env::temp_dir().join(format!("snellar-downloads-hang-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("yt-dlp");
        std::fs::write(&bin, "#!/usr/bin/env -S sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let library = Arc::new(Library::open(Path::new(":memory:")).unwrap());
        library.set_setting("downloadDir", &serde_json::to_string(dir.to_str().unwrap()).unwrap());
        let resolver = Arc::new(Resolver::new(dir.clone()));
        let downloads = Downloads { ui: Arc::new(|_| {}), library, bin: Some(bin), resolver, limit: Duration::from_millis(200) };
        let track = Track { id: snellar_core::ids::TrackId::youtube("dQw4w9WgXcQ"), title: "t".into(), ..Track::default() };
        let done = tokio::time::timeout(Duration::from_secs(5), downloads.fetch(&track, "t")).await.expect("it returned");
        assert_eq!(Code::of(&done.unwrap_err()), Code::YtdlpFailed);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn without_ffmpeg_the_stream_is_kept_as_it_comes() {
        let bare = options(false, true);
        for needs_ffmpeg in ["-x", "--audio-format", "--embed-metadata", "--embed-thumbnail"] {
            assert!(!bare.contains(&needs_ffmpeg), "{needs_ffmpeg} fails the whole download when ffmpeg is absent");
        }
        assert!(bare.contains(&"bestaudio") && bare.contains(&"after_move:filepath"));
        let full = options(true, true);
        for wanted in ["-x", "--embed-metadata", "--embed-thumbnail"] {
            assert!(full.contains(&wanted), "{wanted}");
        }
        assert!(!options(true, false).contains(&"--embed-thumbnail"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_yt_dlp_says_so_and_nothing_else_breaks() {
        let dir = std::env::temp_dir().join(format!("snellar-downloads-none-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let library = Arc::new(Library::open(Path::new(":memory:")).unwrap());
        library.set_setting("downloadDir", &serde_json::to_string(dir.to_str().unwrap()).unwrap());
        let told = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = told.clone();
        let ui: Ui = Arc::new(move |e| {
            if let UiEvent::Toast(t) = e {
                sink.lock().unwrap().push((t.code, t.track))
            }
        });
        let resolver = Arc::new(Resolver::new(dir.clone()));
        let downloads = Arc::new(Downloads { ui, library, bin: Some(dir.join("no-such-tool")), resolver, limit: DOWNLOAD_LIMIT });
        downloads.start(Track { id: snellar_core::ids::TrackId::youtube("dQw4w9WgXcQ"), title: "t".into(), ..Track::default() });
        crate::fakes::until("the toast", || !told.lock().unwrap().is_empty()).await;
        assert_eq!(*told.lock().unwrap(), [(Code::DownloadNoTool, "yt:dQw4w9WgXcQ".to_string())]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_saved_file_is_the_last_path_yt_dlp_prints() {
        assert_eq!(saved_path("/m/Artist - Song.opus\n"), Some(PathBuf::from("/m/Artist - Song.opus")));
        assert_eq!(saved_path("[download] 100%\n/m/a.m4a\n"), Some(PathBuf::from("/m/a.m4a")));
        assert_eq!(saved_path(""), None);
    }

    #[test]
    fn names_are_filesystem_safe() {
        assert_eq!(clean("AC/DC \n"), "AC DC");
        assert_eq!(meta("50% a:b"), "50%% a\\:b");
    }

    #[test]
    fn a_one_word_title_is_not_taken_for_a_field_name() {
        for word in ["Intro", "Hello_World", "x"] {
            assert!(meta(word).contains("%("), "{word} would be read by yt-dlp as the field of that name and become NA");
        }
        assert_eq!(meta("Two words"), "Two words");
        assert_eq!(meta("Intro 2"), "Intro 2");
        assert_eq!(meta("Привет"), "Привет");
    }

    #[tokio::test]
    #[ignore]
    async fn live_download() {
        let dir = std::env::temp_dir().join("snellar-downloads-test");
        std::fs::remove_dir_all(&dir).ok();
        let library = Arc::new(Library::open(Path::new(":memory:")).unwrap());
        library.set_setting("downloadDir", &serde_json::to_string(dir.to_str().unwrap()).unwrap());
        let toasts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = toasts.clone();
        let downloads = Downloads::new(
            Arc::new(move |e| {
                if let UiEvent::Toast(t) = e {
                    sink.lock().unwrap().push(t)
                }
            }),
            library.clone(),
            Arc::new(Resolver::new(std::env::temp_dir())),
        );
        let track = Track {
            id: snellar_core::ids::TrackId::youtube("dQw4w9WgXcQ"),
            title: "Never Gonna Give You Up".into(),
            artists: vec![snellar_core::model::Ref { id: "a".into(), name: "Rick Astley".into() }],
            album: Some(snellar_core::model::Ref { id: "b".into(), name: "Whenever You Need Somebody".into() }),
            ..Track::default()
        };
        downloads.start(track);
        for _ in 0..120 {
            if !toasts.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        let files = downloads.list();
        assert_eq!(files.len(), 1, "{:?}", toasts.lock().unwrap());
        assert_eq!(files[0].name, "Rick Astley - Never Gonna Give You Up");
        assert_eq!(
            library.download(&snellar_core::ids::TrackId::youtube("dQw4w9WgXcQ")).map(|d| d.0.to_string_lossy().into_owned()),
            Some(files[0].path.clone())
        );
        println!("{} {} bytes", files[0].path, files[0].size);
    }
}
