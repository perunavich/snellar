use std::path::PathBuf;
use std::sync::LazyLock;

pub const NAME: &str = "Snellar";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ID: &str = "snellar";
pub const QML_MAIN: &str = "qrc:/qt/qml/Snellar/qml/Main.qml";
pub const MPRIS_BUS: &str = "org.mpris.MediaPlayer2.snellar";
pub const MPRIS_TRACK: &str = "/org/snellar/track";
pub const USER_AGENT: &str = "Snellar/0.1 (https://github.com/perunavich/snellar)";
pub const SOCKET_ENV: &str = "SNELLAR_SOCK";
pub const TRACE_ENV: &str = "SNELLAR_TRACE";
pub const SOCKET_FILE: &str = "snellar.sock";
pub const MPV_SOCKET_PREFIX: &str = "snellar-mpv-";
pub const YTDLP: &str = "yt-dlp";
pub const APP_IMAGE: &str = "Snellar-x86_64.AppImage";

#[expect(clippy::disallowed_methods, reason = "product is the one place that resolves the data root")]
static DATA: LazyLock<PathBuf> = LazyLock::new(|| root(dirs::data_dir()));
#[expect(clippy::disallowed_methods, reason = "product is the one place that resolves the cache root")]
static CACHE: LazyLock<PathBuf> = LazyLock::new(|| root(dirs::cache_dir()));

fn root(base: Option<PathBuf>) -> PathBuf {
    base.unwrap_or_else(std::env::temp_dir).join(ID)
}

pub fn shown_version(version: &str) -> String {
    let (numbers, stage) = version.split_once('-').map_or((version, None), |(n, s)| (n, Some(s)));
    let short = numbers.strip_suffix(".0").unwrap_or(numbers);
    stage.map_or_else(|| short.to_string(), |s| format!("{short} {s}"))
}

fn beside(exe: Option<PathBuf>, name: &str) -> PathBuf {
    exe.and_then(|exe| Some(exe.parent()?.join(name))).filter(|tool| tool.is_file()).unwrap_or_else(|| name.into())
}

pub fn mpv_bin() -> PathBuf {
    beside(std::env::current_exe().ok(), "mpv")
}

pub fn managed_ytdlp() -> PathBuf {
    data_dir().join("tools").join(YTDLP)
}

fn on_path(path: Option<std::ffi::OsString>, name: &str) -> Option<PathBuf> {
    std::env::split_paths(&path?).map(|dir| dir.join(name)).find(|tool| tool.is_file())
}

pub fn own_ytdlp() -> Option<PathBuf> {
    let local = dirs::home_dir().map(|home| home.join(".local/bin").join(YTDLP)).filter(|tool| tool.exists());
    local.or_else(|| on_path(std::env::var_os("PATH"), YTDLP))
}

pub fn ytdlp_bin() -> PathBuf {
    Some(managed_ytdlp()).filter(|tool| tool.is_file()).or_else(own_ytdlp).unwrap_or_else(|| YTDLP.into())
}

pub fn ffmpeg_bin() -> Option<PathBuf> {
    on_path(std::env::var_os("PATH"), "ffmpeg")
}

pub fn installed_app_image() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".local/opt").join(ID).join("versions").join(APP_IMAGE))
}

pub fn data_dir() -> PathBuf {
    DATA.clone()
}

pub fn cache_dir() -> PathBuf {
    CACHE.clone()
}

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

pub fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(SOCKET_ENV) {
        return Some(path.into());
    }
    Some(PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join(SOCKET_FILE))
}

pub fn downloads_dir() -> PathBuf {
    dirs::audio_dir().or_else(|| dirs::home_dir().map(|h| h.join("Music"))).unwrap_or_default().join(NAME)
}

pub fn download_folder(text: &str) -> Option<PathBuf> {
    let text = text.trim();
    let path = match text.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => dirs::home_dir()?.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(text),
    };
    let escapes = path.components().any(|part| part == std::path::Component::ParentDir);
    (path.is_absolute() && !escapes).then_some(path)
}
