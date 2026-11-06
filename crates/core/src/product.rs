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
    #[cfg(any(test, feature = "test-guard"))]
    assert!(
        std::env::var_os("XDG_DATA_HOME").is_some() && std::env::var_os("XDG_CACHE_HOME").is_some(),
        "tests must not touch the real library: use App::start_in with a testkit::Sandbox"
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_version_reads_for_people() {
        assert_eq!(shown_version("0.1.0-beta"), "0.1 beta");
        assert_eq!(shown_version("0.1.0"), "0.1");
        assert_eq!(shown_version("0.1.1-beta"), "0.1.1 beta");
        assert_eq!(shown_version("1.2.3"), "1.2.3");
        assert_eq!(shown_version("0.2.0-rc.1"), "0.2 rc.1");
    }

    #[test]
    fn shown_version_of_this_build_is_the_release() {
        assert_eq!(shown_version(VERSION), "0.1.1 beta");
    }

    #[test]
    fn a_tool_is_found_on_the_path_only_if_it_is_a_file() {
        let dir = std::env::temp_dir().join(format!("snellar-onpath-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("ffmpeg")).unwrap();
        assert_eq!(on_path(Some(dir.clone().into()), "ffmpeg"), None, "a folder of that name");
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/ffmpeg"), "").unwrap();
        let path = std::env::join_paths([dir.clone(), dir.join("bin")]).unwrap();
        assert_eq!(on_path(Some(path), "ffmpeg"), Some(dir.join("bin/ffmpeg")));
        assert_eq!(on_path(None, "ffmpeg"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_tool_beside_the_binary_wins_over_the_path() {
        let dir = std::env::temp_dir().join(format!("snellar-beside-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("snellar");
        assert_eq!(beside(Some(exe.clone()), "mpv"), PathBuf::from("mpv"), "nothing beside it, the name goes through PATH");
        std::fs::write(dir.join("mpv"), "").unwrap();
        assert_eq!(beside(Some(exe), "mpv"), dir.join("mpv"));
        assert_eq!(beside(None, "mpv"), PathBuf::from("mpv"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
