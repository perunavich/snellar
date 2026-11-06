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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::unix::net::UnixListener;

    use super::*;

    #[test]
    fn only_gnome_loses_the_bar_and_a_chosen_decoration_keeps_it() {
        assert!(frameless(Desktop::Gnome, None));
        assert!(frameless(Desktop::Gnome, Some(OWN_DECORATION)));
        assert!(!frameless(Desktop::Gnome, Some("adwaita")));
        for desktop in [Desktop::Hyprland, Desktop::Kde, Desktop::Niri, Desktop::Sway, Desktop::SwayFx, Desktop::Cosmic, Desktop::Other] {
            assert!(!frameless(desktop, None), "{}", desktop.name());
        }
    }

    fn answering(name: &str, reply: &'static [u8], framed: bool) -> (std::path::PathBuf, std::thread::JoinHandle<Vec<u8>>) {
        let dir = std::env::temp_dir().join(format!("snellar-desktop-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ipc.1000.1.sock");
        std::fs::remove_file(&path).ok();
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut ask = vec![0u8; if framed { 14 } else { 64 }];
            let got = if framed { conn.read_exact(&mut ask).map(|_| ask.len()).unwrap() } else { conn.read(&mut ask).unwrap() };
            ask.truncate(got);
            if framed {
                let mut head = b"i3-ipc".to_vec();
                head.extend_from_slice(&(reply.len() as u32).to_ne_bytes());
                head.extend_from_slice(&ask[10..14]);
                conn.write_all(&head).unwrap();
            }
            conn.write_all(reply).unwrap();
            ask
        });
        (path, server)
    }

    #[test]
    fn swayfx_is_told_apart_by_its_real_version_reply() {
        let real = br#"{"human_readable":"0.6","sway_original_version":"1.12.0","variant":"sway","major":0,"minor":6,"patch":0,"loaded_config_file_name":"/c"}"#;
        let (path, server) = answering("fx", real, true);
        assert!(is_swayfx(&path.to_string_lossy()), "SwayFX 0.6 from any path");
        server.join().unwrap();
        let plain = br#"{"human_readable":"1.12","variant":"sway","major":1,"minor":12,"patch":0,"loaded_config_file_name":"/c"}"#;
        let (path, server) = answering("plain", plain, true);
        assert!(!is_swayfx(&path.to_string_lossy()), "plain Sway");
        server.join().unwrap();
    }

    #[test]
    fn hyprland_says_whether_blur_is_on() {
        let cases: [(&str, &[u8], Option<bool>); 5] = [
            ("on", br#"{"option": "decoration:blur:enabled", "bool": true, "set": true }"#, Some(true)),
            ("off", br#"{"option": "decoration:blur:enabled", "bool": false, "set": true }"#, Some(false)),
            ("oldon", br#"{"option": "decoration:blur:enabled", "int": 1, "set": false }"#, Some(true)),
            ("oldoff", br#"{"option": "decoration:blur:enabled", "int": 0, "set": false }"#, Some(false)),
            ("noise", b"unknown request", None),
        ];
        for (name, reply, want) in cases {
            let (path, server) = answering(name, reply, false);
            assert_eq!(hyprland_blurs(&path), want, "{name}");
            assert_eq!(server.join().unwrap(), b"j/getoption decoration:blur:enabled", "{name}");
        }
        assert_eq!(hyprland_blurs(Path::new("/no/such/socket")), None);
    }

    #[test]
    fn a_swayfx_session_reads_blur_from_its_config() {
        let (path, server) = answering("cfg", br#"{"config":"gaps outer 4\nblur enable\n"}"#, true);
        assert!(blurs(Desktop::SwayFx, |name| (name == "SWAYSOCK").then(|| path.to_string_lossy().into_owned())));
        assert_eq!(u32::from_ne_bytes(server.join().unwrap()[10..14].try_into().unwrap()), SWAY_GET_CONFIG);
        let (path, server) = answering("nocfg", br#"{"config":"gaps outer 4\n"}"#, true);
        assert!(!blurs(Desktop::SwayFx, |name| (name == "SWAYSOCK").then(|| path.to_string_lossy().into_owned())));
        server.join().unwrap();
    }

    #[test]
    fn a_swayfx_config_turns_blur_on_only_outside_comments_and_layer_rules() {
        for on in ["blur enable", "  blur on\n", "for_window [app_id=\"snellar\"] blur enable", "gaps outer 4\nblur true"] {
            assert!(config_blurs(on), "{on}");
        }
        for off in [
            "",
            "gaps outer 80",
            "blur disable",
            "# blur enable",
            "blur_radius 5",
            "layer_effects \"waybar\" blur enable",
            "layer_effects \"waybar\" {\n    blur enable;\n    corner_radius 6;\n}",
        ] {
            assert!(!config_blurs(off), "{off}");
        }
        assert!(config_blurs("layer_effects \"waybar\" {\n    blur enable;\n}\nblur enable"), "after the block");
    }

    #[test]
    fn each_desktop_is_told_apart_by_its_environment() {
        type Env = &'static [(&'static str, &'static str)];
        let cases: [(Env, bool, Desktop); 20] = [
            (&[("HYPRLAND_INSTANCE_SIGNATURE", "abc_123"), ("XDG_CURRENT_DESKTOP", "Hyprland")], false, Desktop::Hyprland),
            (&[("HYPRLAND_INSTANCE_SIGNATURE", "abc_123")], false, Desktop::Hyprland),
            (&[("XDG_CURRENT_DESKTOP", "Hyprland")], false, Desktop::Hyprland),
            (&[("XDG_CURRENT_DESKTOP", "KDE")], false, Desktop::Kde),
            (&[("KDE_FULL_SESSION", "true")], false, Desktop::Kde),
            (&[("XDG_CURRENT_DESKTOP", "GNOME")], false, Desktop::Gnome),
            (&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")], false, Desktop::Gnome),
            (&[("XDG_CURRENT_DESKTOP", "GNOME-Classic:GNOME")], false, Desktop::Gnome),
            (&[("SWAYSOCK", "/run/user/1000/sway-ipc.1000.42.sock"), ("XDG_CURRENT_DESKTOP", "sway")], false, Desktop::Sway),
            (&[("SWAYSOCK", "/run/user/1000/sway-ipc.1000.42.sock")], true, Desktop::SwayFx),
            (&[("SWAYSOCK", "/run/user/1000/sway-ipc.1000.42.sock"), ("XDG_CURRENT_DESKTOP", "swayfx")], false, Desktop::SwayFx),
            (&[("XDG_CURRENT_DESKTOP", "sway")], false, Desktop::Sway),
            (&[("XDG_CURRENT_DESKTOP", "niri")], false, Desktop::Niri),
            (&[("NIRI_SOCKET", "/run/user/1000/niri.sock")], false, Desktop::Niri),
            (&[("XDG_CURRENT_DESKTOP", "COSMIC")], false, Desktop::Cosmic),
            (&[("XDG_CURRENT_DESKTOP", "i3")], false, Desktop::Other),
            (&[("XDG_CURRENT_DESKTOP", "XFCE")], false, Desktop::Other),
            (&[("XDG_CURRENT_DESKTOP", "")], false, Desktop::Other),
            (&[], false, Desktop::Other),
            (&[("HYPRLAND_INSTANCE_SIGNATURE", ""), ("SWAYSOCK", "")], false, Desktop::Other),
        ];
        for (env, fx, want) in cases {
            let env: HashMap<&str, &str> = env.iter().copied().collect();
            let got = detect(|name| env.get(name).map(|v| v.to_string()), |_| fx);
            assert_eq!(got, want, "{env:?}");
        }
    }

    #[test]
    fn only_desktops_where_blur_is_on_get_glass_by_default() {
        let all = [
            Desktop::Hyprland,
            Desktop::Kde,
            Desktop::Gnome,
            Desktop::Sway,
            Desktop::SwayFx,
            Desktop::Niri,
            Desktop::Cosmic,
            Desktop::Other,
        ];
        let glass: Vec<Desktop> = all.into_iter().filter(|d| d.glass(true) == "glass").collect();
        assert_eq!(glass, [Desktop::Hyprland, Desktop::Gnome, Desktop::SwayFx]);
        assert!(all.iter().all(|d| d.glass(false) == "cover"), "blur off or unknown is always the cover");
    }

    #[test]
    fn the_hyprland_line_follows_its_version() {
        assert_eq!(hyprland_line(Some("0.57.0")), HYPRLAND_LUA_LINE);
        assert_eq!(hyprland_line(Some("0.58.1")), HYPRLAND_LUA_LINE);
        assert_eq!(hyprland_line(Some("1.0.0")), HYPRLAND_LUA_LINE);
        assert_eq!(hyprland_line(Some("0.55.2")), HYPRLAND_OLD_LINE);
        assert_eq!(hyprland_line(Some("0.56.9")), HYPRLAND_OLD_LINE);
        assert_eq!(hyprland_line(None), HYPRLAND_LUA_LINE, "nobody answered: the current syntax");
    }

    #[test]
    fn hyprland_says_its_version() {
        let (path, server) = answering("ver", br#"{"branch":"main","version":"0.55.2","tag":"v0.55.2"}"#, false);
        assert_eq!(hyprland_version(&path).as_deref(), Some("0.55.2"));
        assert_eq!(server.join().unwrap(), b"j/version");
        let (path, server) = answering("tag", br#"{"tag":"v0.57.1"}"#, false);
        assert_eq!(hyprland_version(&path).as_deref(), Some("0.57.1"));
        server.join().unwrap();
    }

    #[test]
    fn a_niri_config_turns_blur_on_only_in_code() {
        for on in ["window-rule {\n  background-effect { blur true; }\n}", "window-rule {\n  background-effect {\n    blur true\n  }\n}"] {
            assert!(niri_config_blurs(on), "{on}");
        }
        for off in [
            "",
            "// background-effect { blur true }",
            "background-effect { blur false }",
            "layout { gaps 8; }",
            "background-effect { xray true }",
        ] {
            assert!(!niri_config_blurs(off), "{off}");
        }
    }

    #[test]
    fn the_first_question_to_gnome_works_outside_any_runtime() {
        let found = std::thread::spawn(ask_gnome_now).join().expect("it must not panic");
        assert!(found.is_none_or(|state| !state.works), "no GNOME Shell on the test bus");
    }

    #[test]
    fn only_a_blur_that_is_off_gets_a_line_to_copy() {
        let on = described(Desktop::Hyprland, true, Some("x".into()), None);
        assert!(on["line"].is_null());
        assert_eq!(on["glass"], "glass");
        let off = described(Desktop::SwayFx, false, Some(SWAYFX_LINE.into()), None);
        assert_eq!((off["line"].as_str(), off["glass"].as_str()), (Some("blur enable"), Some("cover")));
        let empty = |_: &str| None;
        assert_eq!(config_line(Desktop::Kde, &empty), None);
        assert_eq!(config_line(Desktop::Gnome, &empty), None, "GNOME has a button, not a line");
        assert!(config_line(Desktop::Niri, &empty).unwrap().contains("background-effect { blur true }"));
        assert!(config_line(Desktop::Niri, &empty).unwrap().contains("draw-border-with-background false"));
    }

    #[test]
    fn a_working_extension_gives_gnome_glass_and_losing_it_goes_back_to_the_cover() {
        let works = glass::State { extension: "active", works: true, ..glass::State::default() };
        assert_eq!(described(Desktop::Gnome, works.works, None, Some(&works))["glass"], "glass");
        let lost = glass::State { extension: "off", ..glass::State::default() };
        assert_eq!(described(Desktop::Gnome, lost.works, None, Some(&lost))["glass"], "cover");
    }

    #[test]
    fn blur_is_off_when_nobody_can_say() {
        let none = |_: &str| None;
        assert!(!blurs(Desktop::Hyprland, none));
        assert!(!blurs(Desktop::SwayFx, none));
        let gone = |name: &str| Some(format!("/no/such/{name}"));
        assert!(!blurs(Desktop::Hyprland, gone));
        assert!(!blurs(Desktop::SwayFx, gone));
        assert!(!blurs(Desktop::Kde, gone));
    }

    #[test]
    fn the_sway_socket_names_its_version_and_pid() {
        let dir = std::env::temp_dir().join(format!("snellar-sway-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sway-ipc.1000.4242.sock");
        std::fs::remove_file(&path).ok();
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut ask = [0u8; 14];
            conn.read_exact(&mut ask).unwrap();
            assert_eq!(&ask[..6], b"i3-ipc");
            assert_eq!(u32::from_ne_bytes(ask[10..14].try_into().unwrap()), 7, "GET_VERSION");
            let body = br#"{"human_readable":"swayfx 0.4 (based on sway 1.9)","major":1,"minor":9}"#;
            let mut reply = b"i3-ipc".to_vec();
            reply.extend_from_slice(&(body.len() as u32).to_ne_bytes());
            reply.extend_from_slice(&7u32.to_ne_bytes());
            reply.extend_from_slice(body);
            conn.write_all(&reply).unwrap();
        });
        let version = sway_version(&path).unwrap();
        assert!(version.contains("swayfx 0.4"), "{version}");
        server.join().unwrap();
        assert_eq!(sway_pid(&path.to_string_lossy()), Some(4242));
        assert_eq!(sway_version(&dir.join("missing.sock")), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
