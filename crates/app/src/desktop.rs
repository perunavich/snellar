use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Value, json};
use snellar_core::journal;
use snellar_core::library::Library;

use crate::glass;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    Hyprland,
    Kde,
    Gnome,
    Sway,
    SwayFx,
    Niri,
    Cosmic,
    Other,
}

impl Desktop {
    pub fn name(self) -> &'static str {
        match self {
            Desktop::Hyprland => "hyprland",
            Desktop::Kde => "kde",
            Desktop::Gnome => "gnome",
            Desktop::Sway => "sway",
            Desktop::SwayFx => "swayfx",
            Desktop::Niri => "niri",
            Desktop::Cosmic => "cosmic",
            Desktop::Other => "other",
        }
    }

    pub fn glass(self, blurs: bool) -> &'static str {
        match self {
            Desktop::Hyprland | Desktop::SwayFx | Desktop::Gnome if blurs => "glass",
            _ => "cover",
        }
    }
}

const SWAY_GET_VERSION: u32 = 7;
const SWAY_GET_CONFIG: u32 = 9;
const SWAY_TRUE: [&str; 7] = ["1", "yes", "on", "true", "enable", "enabled", "active"];
const GSETTINGS: &str = "gsettings";
pub const OWN_DECORATION: &str = "bradient";
const FIRST_ANSWER_LIMIT: Duration = Duration::from_millis(500);
const HYPRLAND_LUA_SINCE: (u32, u32) = (0, 57);
const HYPRLAND_LUA_LINE: &str = "hl.config({ decoration = { blur = { enabled = true } } })";
const HYPRLAND_OLD_LINE: &str = "decoration { blur { enabled = true } }";
const SWAYFX_LINE: &str = "blur enable";
const NIRI_LINE: &str =
    "window-rule {\n    match app-id=\"snellar\"\n    draw-border-with-background false\n    background-effect { blur true }\n}";

fn detect(var: impl Fn(&str) -> Option<String>, swayfx: impl FnOnce(&str) -> bool) -> Desktop {
    let set = |name: &str| var(name).filter(|v| !v.is_empty());
    let current = set("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let names: Vec<&str> = current.split(':').map(str::trim).collect();
    let named = |wanted: &[&str]| names.iter().any(|n| wanted.contains(n));
    if set("HYPRLAND_INSTANCE_SIGNATURE").is_some() || named(&["hyprland"]) {
        return Desktop::Hyprland;
    }
    if let Some(socket) = set("SWAYSOCK") {
        return if named(&["swayfx"]) || swayfx(&socket) { Desktop::SwayFx } else { Desktop::Sway };
    }
    if named(&["swayfx"]) {
        return Desktop::SwayFx;
    }
    if named(&["sway"]) {
        return Desktop::Sway;
    }
    if named(&["niri"]) || set("NIRI_SOCKET").is_some() {
        return Desktop::Niri;
    }
    if named(&["cosmic"]) {
        return Desktop::Cosmic;
    }
    if named(&["kde", "plasma"]) || set("KDE_FULL_SESSION").is_some() {
        return Desktop::Kde;
    }
    if named(&["gnome", "gnome-classic", "gnome-flashback", "ubuntu", "pop", "zorin"]) {
        return Desktop::Gnome;
    }
    Desktop::Other
}

fn sway_ask(socket: &Path, kind: u32) -> Option<String> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    stream.set_write_timeout(Some(Duration::from_millis(300))).ok()?;
    let mut ask = b"i3-ipc".to_vec();
    ask.extend_from_slice(&0u32.to_ne_bytes());
    ask.extend_from_slice(&kind.to_ne_bytes());
    stream.write_all(&ask).ok()?;
    let mut head = [0u8; 14];
    stream.read_exact(&mut head).ok()?;
    if &head[..6] != b"i3-ipc" {
        return None;
    }
    let len = u32::from_ne_bytes(head[6..10].try_into().ok()?) as usize;
    let mut body = vec![0u8; len.min(64 << 10)];
    stream.read_exact(&mut body).ok()?;
    String::from_utf8(body).ok()
}

fn sway_version(socket: &Path) -> Option<String> {
    sway_ask(socket, SWAY_GET_VERSION)
}

fn sway_config(socket: &Path) -> Option<String> {
    let reply: Value = serde_json::from_str(&sway_ask(socket, SWAY_GET_CONFIG)?).ok()?;
    reply["config"].as_str().map(str::to_string)
}

fn sway_pid(socket: &str) -> Option<u32> {
    Path::new(socket).file_name()?.to_str()?.strip_suffix(".sock")?.rsplit('.').next()?.parse().ok()
}

fn is_swayfx(socket: &str) -> bool {
    let said = sway_version(Path::new(socket)).is_some_and(|v| {
        let v = v.to_lowercase();
        v.contains("swayfx") || v.contains("sway_original_version")
    });
    let binary = sway_pid(socket)
        .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok())
        .is_some_and(|exe| exe.to_string_lossy().to_lowercase().contains("swayfx"));
    said || binary
}

fn hyprland_blurs(socket: &Path) -> Option<bool> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    stream.set_write_timeout(Some(Duration::from_millis(300))).ok()?;
    stream.write_all(b"j/getoption decoration:blur:enabled").ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    let reply: Value = serde_json::from_str(&reply).ok()?;
    reply["bool"].as_bool().or_else(|| reply["int"].as_i64().map(|n| n != 0))
}

fn config_blurs(config: &str) -> bool {
    let mut in_layer_block = false;
    config.lines().map(str::trim).any(|line| {
        if in_layer_block {
            in_layer_block = !line.starts_with('}');
            return false;
        }
        if line.starts_with("layer_effects") {
            in_layer_block = line.ends_with('{');
            return false;
        }
        let words: Vec<&str> = line.split([' ', '\t', ';', ',']).filter(|w| !w.is_empty()).collect();
        matches!(words.first(), Some(&"blur" | &"for_window")) && words.windows(2).any(|w| w[0] == "blur" && SWAY_TRUE.contains(&w[1]))
    })
}

fn hyprland_socket(var: &impl Fn(&str) -> Option<String>) -> Option<std::path::PathBuf> {
    Some(Path::new(&var("XDG_RUNTIME_DIR")?).join("hypr").join(var("HYPRLAND_INSTANCE_SIGNATURE")?).join(".socket.sock"))
}

fn hyprland_version(socket: &Path) -> Option<String> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(300))).ok()?;
    stream.set_write_timeout(Some(Duration::from_millis(300))).ok()?;
    stream.write_all(b"j/version").ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    let reply: Value = serde_json::from_str(&reply).ok()?;
    reply["version"].as_str().or(reply["tag"].as_str()).map(|v| v.trim_start_matches('v').to_string())
}

fn hyprland_line(version: Option<&str>) -> &'static str {
    let number = |part: Option<&str>| part.and_then(|p| p.parse::<u32>().ok());
    let mut parts = version.unwrap_or_default().split('.');
    match (number(parts.next()), number(parts.next())) {
        (Some(major), Some(minor)) if (major, minor) < HYPRLAND_LUA_SINCE => HYPRLAND_OLD_LINE,
        _ => HYPRLAND_LUA_LINE,
    }
}

fn niri_config_blurs(config: &str) -> bool {
    let code: String = config.lines().map(|line| line.split("//").next().unwrap_or_default()).collect::<Vec<_>>().join("\n");
    code.split("background-effect").skip(1).any(|rest| {
        let block = rest.trim_start().strip_prefix('{').map(|body| body.split('}').next().unwrap_or_default());
        block.is_some_and(|body| body.split([';', '\n']).any(|item| item.split_whitespace().eq(["blur", "true"])))
    })
}

fn niri_config(var: &impl Fn(&str) -> Option<String>) -> Option<String> {
    let path =
        var("NIRI_CONFIG").map(std::path::PathBuf::from).or_else(|| var("XDG_CONFIG_HOME").map(|c| Path::new(&c).join("niri/config.kdl")));
    let path = path.or_else(|| var("HOME").map(|h| Path::new(&h).join(".config/niri/config.kdl")))?;
    std::fs::read_to_string(path).ok()
}

fn config_line(desktop: Desktop, var: &impl Fn(&str) -> Option<String>) -> Option<String> {
    match desktop {
        Desktop::Hyprland => {
            let version = hyprland_socket(var).and_then(|socket| hyprland_version(&socket));
            Some(hyprland_line(version.as_deref()).to_string())
        }
        Desktop::SwayFx => Some(SWAYFX_LINE.to_string()),
        Desktop::Niri => Some(NIRI_LINE.to_string()),
        _ => None,
    }
}

fn blurs(desktop: Desktop, var: impl Fn(&str) -> Option<String>) -> bool {
    match desktop {
        Desktop::Hyprland => hyprland_socket(&var).is_some_and(|socket| hyprland_blurs(&socket) == Some(true)),
        Desktop::Niri => niri_config(&var).is_some_and(|config| niri_config_blurs(&config)),
        Desktop::SwayFx => var("SWAYSOCK").and_then(|socket| sway_config(Path::new(&socket))).is_some_and(|config| config_blurs(&config)),
        _ => false,
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn kind() -> Desktop {
    static FOUND: OnceLock<Desktop> = OnceLock::new();
    *FOUND.get_or_init(|| detect(env, is_swayfx))
}

fn frameless(desktop: Desktop, decoration: Option<&str>) -> bool {
    desktop == Desktop::Gnome && matches!(decoration, None | Some(OWN_DECORATION))
}

fn described(desktop: Desktop, blur: bool, line: Option<String>, gnome: Option<&glass::State>) -> Value {
    json!({
        "desktop": desktop.name(),
        "frameless": frameless(desktop, env("QT_WAYLAND_DECORATION").as_deref()),
        "blur": blur,
        "glass": desktop.glass(blur),
        "line": if blur { None } else { line },
        "gnome": gnome,
    })
}

fn ask_gnome_now() -> Option<glass::State> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    runtime.block_on(async {
        let asked = async {
            let conn = zbus::Connection::session().await.ok()?;
            Some(glass::probe(&conn, Path::new(GSETTINGS)).await)
        };
        tokio::time::timeout(FIRST_ANSWER_LIMIT, asked).await.ok().flatten()
    })
}

fn gnome_first_answer() -> glass::State {
    let asked = std::thread::spawn(ask_gnome_now);
    asked.join().ok().flatten().unwrap_or(glass::State { extension: "unknown", ..glass::State::default() })
}

pub fn current() -> &'static Value {
    static FOUND: OnceLock<Value> = OnceLock::new();
    FOUND.get_or_init(|| {
        let desktop = kind();
        let found = if desktop == Desktop::Gnome {
            let gnome = gnome_first_answer();
            described(desktop, gnome.works, None, Some(&gnome))
        } else {
            let blur = blurs(desktop, env);
            described(desktop, blur, if blur { None } else { config_line(desktop, &env) }, None)
        };
        journal::info("app", format!("desktop {} blur {} auto background {}", desktop.name(), found["blur"], found["glass"]));
        found
    })
}

pub async fn status(library: &Library) -> Value {
    let mut found = looks(library).await;
    found["reducedMotion"] = json!(crate::portal::reduced_motion().await);
    found
}

async fn looks(library: &Library) -> Value {
    let desktop = kind();
    if desktop == Desktop::Gnome {
        let gnome = match zbus::Connection::session().await {
            Ok(conn) => glass::state(&conn, Path::new(GSETTINGS), library).await,
            Err(_) => glass::State { extension: "unknown", ..glass::State::default() },
        };
        return described(desktop, gnome.works, None, Some(&gnome));
    }
    tokio::task::spawn_blocking(move || {
        let blur = blurs(desktop, env);
        described(desktop, blur, if blur { None } else { config_line(desktop, &env) }, None)
    })
    .await
    .unwrap_or_else(|_| described(desktop, false, None, None))
}

pub async fn setup_glass(library: &Library) -> anyhow::Result<glass::Setup> {
    let conn = zbus::Connection::session().await?;
    glass::setup(&conn, Path::new(GSETTINGS), library).await
}

pub async fn undo_glass(library: &Library) -> anyhow::Result<()> {
    let conn = zbus::Connection::session().await?;
    glass::undo(&conn, Path::new(GSETTINGS), library).await
}
