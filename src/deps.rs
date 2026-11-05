use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use snellar_core::library::Library;
use snellar_core::{product, settings};

use crate::desktop;
use crate::player::Player;
use crate::report;
use crate::ytdlp;

const ASK_LIMIT: Duration = Duration::from_secs(5);
const BUS_LIMIT: Duration = Duration::from_secs(2);
const TRAY_WATCHER: &str = "org.kde.StatusNotifierWatcher";
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const DISCORD_FOLDERS: [&str; 4] = ["", "app/com.discordapp.Discord", ".flatpak/dev.vencord.Vesktop/xdg-run", "snap.discord"];

unsafe extern "C" {
    fn snellar_qt_version() -> *const std::os::raw::c_char;
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Component {
    pub id: &'static str,
    pub version: String,
    pub source: &'static str,
    pub state: &'static str,
}

fn row(id: &'static str, version: impl Into<String>, source: &'static str, state: &'static str) -> Component {
    Component { id, version: version.into(), source, state }
}

fn second_word(text: &str) -> String {
    text.lines().next().unwrap_or_default().split_whitespace().nth(1).unwrap_or_default().trim_start_matches('v').to_string()
}

fn third_word(text: &str) -> String {
    text.lines().next().unwrap_or_default().split_whitespace().nth(2).unwrap_or_default().to_string()
}

pub async fn mpv_row(bin: &Path) -> Component {
    let source = if bin.is_absolute() { "bundled" } else { "system" };
    match ytdlp::output_of(bin, "--version").await {
        Some(text) => row("mpv", second_word(&text), source, "ok"),
        None => row("mpv", "", source, "broken"),
    }
}

pub async fn ytdlp_row(managed: &Path, own: Option<&Path>) -> Component {
    let (bin, source) = if managed.is_file() {
        (managed, "home")
    } else if let Some(own) = own {
        (own, "system")
    } else {
        return row("yt-dlp", "", "", "missing");
    };
    match ytdlp::version(bin).await {
        Some(version) => row("yt-dlp", version, source, "ok"),
        None => row("yt-dlp", "", source, "broken"),
    }
}

pub async fn ffmpeg_row(bin: Option<&PathBuf>) -> Component {
    let Some(bin) = bin else { return row("ffmpeg", "", "", "missing") };
    match ytdlp::output_of(bin, "-version").await {
        Some(text) => row("ffmpeg", third_word(&text), "system", "ok"),
        None => row("ffmpeg", "", "system", "broken"),
    }
}

fn qt_row(bundled: bool) -> Component {
    // SAFETY: qVersion returns a pointer to a static NUL-terminated string.
    let version = unsafe { std::ffi::CStr::from_ptr(snellar_qt_version()) }.to_string_lossy().into_owned();
    row("qt", version, if bundled { "bundled" } else { "system" }, "ok")
}

async fn audio_row(player: Option<&Arc<Player>>) -> Component {
    let Some(player) = player else { return row("audio", "", "", "waiting") };
    match tokio::time::timeout(ASK_LIMIT, player.devices()).await {
        Ok(Ok(answer)) if answer["devices"].as_array().is_some_and(|d| !d.is_empty()) => {
            row("audio", answer["active"].as_str().unwrap_or_default(), "", "ok")
        }
        _ => row("audio", "", "", "broken"),
    }
}

fn discord_running(runtime: &Path) -> bool {
    DISCORD_FOLDERS.iter().any(|folder| (0..10).any(|n| UnixStream::connect(runtime.join(folder).join(format!("discord-ipc-{n}"))).is_ok()))
}

fn discord_row(on: bool, runtime: &Path) -> Component {
    let state = match (on, discord_running(runtime)) {
        (false, _) => "off",
        (true, true) => "ok",
        (true, false) => "waiting",
    };
    row("discord", "", "", state)
}

async fn portal_row(conn: Option<&zbus::Connection>) -> Component {
    let Some(conn) = conn else { return row("portal", "", "", "missing") };
    let ask = conn.call_method(Some(PORTAL), PORTAL_PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(FILE_CHOOSER, "version"));
    let Ok(Ok(reply)) = tokio::time::timeout(BUS_LIMIT, ask).await else { return row("portal", "", "", "missing") };
    let version = reply.body().deserialize::<zbus::OwnedVariant>().ok().and_then(|v| u32::try_from(&v).ok());
    row("portal", version.map_or(String::new(), |v| v.to_string()), "system", "ok")
}

async fn tray_row(conn: Option<&zbus::Connection>) -> Component {
    let Some(conn) = conn else { return row("tray", "", "", "none") };
    let hosted = async {
        let bus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
        bus.name_has_owner(TRAY_WATCHER.try_into().ok()?).await.ok()
    };
    let found = tokio::time::timeout(BUS_LIMIT, hosted).await.ok().flatten().unwrap_or(false);
    row("tray", "", if found { "system" } else { "" }, if found { "ok" } else { "none" })
}

fn blur_row(status: &Value) -> Component {
    row("blur", "", "", if status["blur"] == true { "on" } else { "off" })
}

pub fn details(status: &Value, components: &[Component]) -> String {
    let mut out = format!(
        "Snellar {} ({})\nDesktop: {}, blur {}, auto background {}\n",
        product::VERSION,
        report::COMMIT,
        status["desktop"].as_str().unwrap_or("?"),
        if status["blur"] == true { "on" } else { "off" },
        status["glass"].as_str().unwrap_or("?"),
    );
    for c in components {
        let version = if c.version.is_empty() { "-" } else { c.version.as_str() };
        let source = if c.source.is_empty() { "-" } else { c.source };
        out.push_str(&format!("{}: {version}, {source}, {}\n", c.id, c.state));
    }
    out
}

pub struct Probe {
    pub library: Arc<Library>,
    pub player: Option<Arc<Player>>,
}

impl Probe {
    pub async fn report(self) -> Value {
        let status = desktop::status(&self.library).await;
        let conn = tokio::time::timeout(BUS_LIMIT, zbus::Connection::session()).await.ok().and_then(Result::ok);
        let (managed, own, ffmpeg, mpv) = (product::managed_ytdlp(), product::own_ytdlp(), product::ffmpeg_bin(), product::mpv_bin());
        let (mpv, ytdlp, ffmpeg, audio, portal, tray) = tokio::join!(
            mpv_row(&mpv),
            ytdlp_row(&managed, own.as_deref()),
            ffmpeg_row(ffmpeg.as_ref()),
            audio_row(self.player.as_ref()),
            portal_row(conn.as_ref()),
            tray_row(conn.as_ref()),
        );
        let qt = qt_row(std::env::var_os("APPIMAGE").is_some());
        let discord = discord_row(settings::DISCORD_RPC.or_default(&self.library), &product::runtime_dir());
        let components = [mpv, ytdlp, ffmpeg, qt, audio, discord, portal, tray, blur_row(&status)];
        json!({ "details": details(&status, &components), "desktop": status, "components": components })
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    use super::*;

    fn tool(dir: &Path, name: &str, script: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-deps-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_tool_counts_only_if_it_runs() {
        let dir = folder("run");
        let good = tool(&dir, "mpv", "echo 'mpv v0.40.0-dirty Copyright (c) 2000-2025'");
        assert_eq!(mpv_row(&good).await, row("mpv", "0.40.0-dirty", "bundled", "ok"));
        let broken = tool(&dir, "broken", "exit 3");
        assert_eq!(mpv_row(&broken).await, row("mpv", "", "bundled", "broken"));
        assert_eq!(mpv_row(Path::new("no-such-mpv")).await, row("mpv", "", "system", "broken"));
        let ours = tool(&dir, "ours", "echo 2026.11.01");
        let own = tool(&dir, "own", "echo 2025.01.01");
        assert_eq!(ytdlp_row(&ours, Some(&own)).await, row("yt-dlp", "2026.11.01", "home", "ok"), "ours first");
        assert_eq!(ytdlp_row(&dir.join("none"), Some(&own)).await, row("yt-dlp", "2025.01.01", "system", "ok"));
        assert_eq!(ytdlp_row(&dir.join("none"), None).await, row("yt-dlp", "", "", "missing"), "not installed is not broken");
        assert_eq!(ytdlp_row(&broken, None).await, row("yt-dlp", "", "home", "broken"));
        let ffmpeg = tool(&dir, "ffmpeg", "echo 'ffmpeg version 7.1.2 Copyright'");
        assert_eq!(ffmpeg_row(Some(&ffmpeg)).await, row("ffmpeg", "7.1.2", "system", "ok"));
        assert_eq!(ffmpeg_row(None).await, row("ffmpeg", "", "", "missing"));
    }

    #[test]
    fn discord_is_read_from_its_socket_not_from_the_setting_alone() {
        let dir = folder("discord");
        assert_eq!(discord_row(false, &dir).state, "off");
        assert_eq!(discord_row(true, &dir).state, "waiting");
        let flatpak = dir.join("app/com.discordapp.Discord");
        std::fs::create_dir_all(&flatpak).unwrap();
        std::fs::write(flatpak.join("discord-ipc-0"), "").unwrap();
        assert_eq!(discord_row(true, &dir).state, "waiting", "a file left behind is not a running Discord");
        std::fs::remove_file(flatpak.join("discord-ipc-0")).unwrap();
        let _listener = UnixListener::bind(flatpak.join("discord-ipc-0")).unwrap();
        assert_eq!(discord_row(true, &dir).state, "ok");
        assert_eq!(discord_row(false, &dir).state, "off", "off stays off");
    }

    #[test]
    fn qt_reports_the_version_that_runs() {
        let qt = qt_row(true);
        assert!(qt.version.starts_with("6."), "{}", qt.version);
        assert_eq!((qt.source, qt.state), ("bundled", "ok"));
        assert_eq!(qt_row(false).source, "system");
    }

    #[test]
    fn the_details_name_every_component_and_mark_the_gaps() {
        let status = json!({ "desktop": "gnome", "blur": true, "glass": "glass" });
        let text = details(&status, &[row("mpv", "0.40", "bundled", "ok"), row("yt-dlp", "", "", "missing")]);
        assert!(text.contains("Desktop: gnome, blur on, auto background glass"), "{text}");
        assert!(text.contains("mpv: 0.40, bundled, ok"), "{text}");
        assert!(text.contains("yt-dlp: -, -, missing"), "{text}");
    }

    #[tokio::test]
    async fn no_portal_and_no_tray_on_a_bare_bus() {
        let bus = crate::testkit::Bus::start();
        let conn = zbus::connection::Builder::address(bus.address.as_str()).build().await.unwrap();
        assert_eq!(portal_row(Some(&conn)).await.state, "missing");
        assert_eq!(tray_row(Some(&conn)).await.state, "none");
        let host = zbus::connection::Builder::address(bus.address.as_str()).name(TRAY_WATCHER).build().await.unwrap();
        assert_eq!(tray_row(Some(&conn)).await.state, "ok");
        drop(host);
    }
}
