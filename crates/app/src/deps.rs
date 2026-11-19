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
use crate::stamp;
use crate::ytdlp;

const ASK_LIMIT: Duration = Duration::from_secs(5);
const BUS_LIMIT: Duration = Duration::from_secs(2);
const TRAY_WATCHER: &str = "org.kde.StatusNotifierWatcher";
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const DISCORD_FOLDERS: [&str; 4] = ["", "app/com.discordapp.Discord", ".flatpak/dev.vencord.Vesktop/xdg-run", "snap.discord"];

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

fn qt_row(version: &str, bundled: bool) -> Component {
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

pub fn discord_state(on: bool, runtime: &Path) -> &'static str {
    match (on, on && discord_running(runtime)) {
        (false, _) => "off",
        (true, true) => "ok",
        (true, false) => "waiting",
    }
}

fn discord_row(on: bool, runtime: &Path) -> Component {
    row("discord", "", "", discord_state(on, runtime))
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
        stamp::commit(),
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
        let qt = qt_row(stamp::qt(), std::env::var_os("APPIMAGE").is_some());
        let discord = discord_row(settings::DISCORD_RPC.or_default(&self.library), &product::runtime_dir());
        let components = [mpv, ytdlp, ffmpeg, qt, audio, discord, portal, tray, blur_row(&status)];
        json!({ "details": details(&status, &components), "desktop": status, "components": components })
    }
}
