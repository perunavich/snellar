use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use zbus::{Connection, OwnedVariant};

use snellar_core::error::Code;
use snellar_core::library::Library;
use snellar_core::{journal, product, settings};

use crate::tray;

pub const EXTENSION: &str = "blur-my-shell@aunetx";
const ROOT_SCHEMA: &str = "org.gnome.shell.extensions.blur-my-shell";
const SCHEMA: &str = "org.gnome.shell.extensions.blur-my-shell.applications";
const BACKGROUND_SCHEMA: &str = "org.gnome.desktop.background";
const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";
const PIPELINE: &str = "pipeline_snellar";
const PIPELINE_BODY: &str = "{'name': <'Snellar'>, 'effects': <[<{'type': <'dual_kawase_blur'>, 'id': <'effect_snellar_blur'>, 'params': <{'unscaled_radius': <200>, 'brightness': <1.0>}>}>, <{'type': <'luminosity'>, 'id': <'effect_snellar_luminosity'>, 'params': <{'contrast': <0.95>, 'contrast_center': <0.5>, 'saturation_multiplicator': <0.988>, 'brightness_shift': <0.0>}>}>, <{'type': <'noise'>, 'id': <'effect_snellar_noise'>, 'params': <{'noise': <0.0117>, 'lightness': <0.5>}>}>]>}";
const CORNER_RADIUS: &str = "12";
const ALL_CORNERS: &str = "0";
const ACTIVE: f64 = 1.0;
const ASK_LIMIT: Duration = Duration::from_secs(2);
const KEY_LIMIT: Duration = Duration::from_secs(3);
const DIALOG_LIMIT: Duration = Duration::from_secs(300);
const SETTLE_TRIES: u32 = 40;
const SETTLE_STEP: Duration = Duration::from_millis(250);

#[zbus::proxy(
    interface = "org.gnome.Shell.Extensions",
    default_service = "org.gnome.Shell.Extensions",
    default_path = "/org/gnome/Shell/Extensions"
)]
trait Extensions {
    fn get_extension_info(&self, uuid: &str) -> zbus::Result<HashMap<String, OwnedVariant>>;
    fn enable_extension(&self, uuid: &str) -> zbus::Result<bool>;
    fn disable_extension(&self, uuid: &str) -> zbus::Result<bool>;
    fn uninstall_extension(&self, uuid: &str) -> zbus::Result<bool>;

    #[zbus(signal)]
    fn extension_state_changed(&self, uuid: &str, state: HashMap<String, OwnedVariant>) -> zbus::Result<()>;
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub extension: &'static str,
    pub version: String,
    pub works: bool,
    pub look: bool,
    pub undo: bool,
    pub wallpaper: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Saved {
    pub blur: String,
    pub whitelist: String,
    pub opacity: String,
    pub dynamic_opacity: String,
    pub pipeline: String,
    pub corner_radius: String,
    pub rounded_corners: String,
    pub installed: bool,
    pub was_enabled: bool,
}

#[derive(Debug, PartialEq)]
pub enum Setup {
    Applied,
    Skipped,
}

#[derive(Debug)]
struct Info {
    state: f64,
    enabled: bool,
    path: PathBuf,
    version: String,
}

fn number(value: &OwnedVariant) -> Option<f64> {
    f64::try_from(value).ok()
}

fn text(value: &OwnedVariant) -> Option<String> {
    String::try_from(value.try_clone().ok()?).ok()
}

fn read_info(map: &HashMap<String, OwnedVariant>) -> Option<Info> {
    let state = number(map.get("state")?)?;
    let version = map.get("version").and_then(|v| number(v).map(|n| n.to_string()).or_else(|| text(v))).unwrap_or_default();
    Some(Info {
        state,
        enabled: map.get("enabled").and_then(|v| bool::try_from(v).ok()).unwrap_or(state == ACTIVE),
        path: map.get("path").and_then(text).map(PathBuf::from).unwrap_or_default(),
        version,
    })
}

async fn info(conn: &Connection) -> Result<Option<Info>> {
    let shell = ExtensionsProxy::new(conn).await?;
    let map = tokio::time::timeout(ASK_LIMIT, shell.get_extension_info(EXTENSION)).await.context("GNOME Shell does not answer")??;
    Ok(read_info(&map))
}

struct Keys<'a> {
    bin: &'a Path,
    schemas: PathBuf,
}

impl Keys<'_> {
    fn command(&self) -> Command {
        let mut cmd = Command::new(self.bin);
        cmd.env("GSETTINGS_SCHEMA_DIR", &self.schemas).env_remove("LD_LIBRARY_PATH").stdin(Stdio::null()).stderr(Stdio::piped());
        cmd
    }

    async fn run(&self, mut cmd: Command) -> Result<String> {
        let out =
            tokio::time::timeout(KEY_LIMIT, cmd.output()).await.context("gsettings does not answer")?.context("cannot start gsettings")?;
        if !out.status.success() {
            anyhow::bail!("gsettings: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    async fn get(&self, key: &str) -> Result<String> {
        let mut cmd = self.command();
        cmd.args(["get", SCHEMA, key]);
        self.run(cmd).await
    }

    async fn set(&self, key: &str, value: &str) -> Result<()> {
        let mut cmd = self.command();
        cmd.args(["set", SCHEMA, key, value]);
        self.run(cmd).await.map(drop)
    }

    async fn pipelines(&self) -> Result<String> {
        let mut cmd = self.command();
        cmd.args(["get", ROOT_SCHEMA, "pipelines"]);
        self.run(cmd).await
    }

    async fn set_pipelines(&self, value: &str) -> Result<()> {
        let mut cmd = self.command();
        cmd.args(["set", ROOT_SCHEMA, "pipelines", value]);
        self.run(cmd).await.map(drop)
    }
}

fn entry_span(text: &str) -> Option<(usize, usize)> {
    let name = format!("'{PIPELINE}':");
    let start = text.find(&name)?;
    let (mut depth, mut quote, mut escaped) = (0usize, None, false);
    for (at, c) in text[start + name.len()..].char_indices() {
        match (c, quote) {
            _ if escaped => escaped = false,
            ('\\', Some(_)) => escaped = true,
            (_, Some(open)) => {
                if c == open {
                    quote = None;
                }
            }
            ('\'' | '"', None) => quote = Some(c),
            ('{', None) => depth += 1,
            ('}', None) => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some((start, start + name.len() + at + c.len_utf8()));
                }
            }
            _ => {}
        }
    }
    None
}

fn without_ours(pipelines: &str) -> String {
    let Some((start, end)) = entry_span(pipelines) else { return pipelines.to_string() };
    let after = pipelines[end..].trim_start();
    if let Some(rest) = after.strip_prefix(',') {
        return format!("{}{}", &pipelines[..start], rest.trim_start());
    }
    let before = pipelines[..start].trim_end().trim_end_matches(',').trim_end();
    format!("{before}{after}")
}

fn with_ours(pipelines: &str) -> String {
    let others = without_ours(pipelines);
    let plain = others.trim().trim_start_matches("@a{sa{sv}}").trim();
    let inside = plain.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')).unwrap_or(plain).trim();
    if inside.is_empty() {
        return format!("{{'{PIPELINE}': {PIPELINE_BODY}}}");
    }
    format!("{{'{PIPELINE}': {PIPELINE_BODY}, {inside}}}")
}

async fn looks_right(keys: &Keys<'_>) -> bool {
    let (Ok(pipeline), Ok(pipelines)) = (keys.get("pipeline").await, keys.pipelines().await) else { return false };
    unquoted(&pipeline) == PIPELINE && entry_span(&pipelines).is_some()
}

fn unquoted(value: &str) -> &str {
    value.trim().trim_matches('\'')
}

async fn read_key(gsettings: &Path, schema: &str, key: &str) -> Option<String> {
    let mut cmd = Command::new(gsettings);
    cmd.env_remove("LD_LIBRARY_PATH").stdin(Stdio::null()).stderr(Stdio::null()).args(["get", schema, key]);
    let out = tokio::time::timeout(KEY_LIMIT, cmd.output()).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

async fn wallpaper(gsettings: &Path) -> String {
    let dark = read_key(gsettings, INTERFACE_SCHEMA, "color-scheme").await.is_some_and(|scheme| unquoted(&scheme) == "prefer-dark");
    let key = if dark { "picture-uri-dark" } else { "picture-uri" };
    let uri = read_key(gsettings, BACKGROUND_SCHEMA, key).await.unwrap_or_default();
    let uri = unquoted(&uri);
    if uri.starts_with("file://") { uri.to_string() } else { String::new() }
}

fn listed(list: &str) -> Vec<&str> {
    let inside = list.trim().trim_start_matches("@as").trim().trim_start_matches('[').trim_end_matches(']');
    inside.split(',').map(|item| item.trim().trim_matches(['\'', '"'])).filter(|item| !item.is_empty()).collect()
}

fn with_us(list: &str) -> String {
    let mut items = listed(list);
    if !items.contains(&product::ID) {
        items.push(product::ID);
    }
    format!("[{}]", items.iter().map(|item| format!("'{item}'")).collect::<Vec<_>>().join(", "))
}

async fn applied(keys: &Keys<'_>) -> bool {
    let (Ok(blur), Ok(whitelist), Ok(opacity), Ok(dynamic)) =
        (keys.get("blur").await, keys.get("whitelist").await, keys.get("opacity").await, keys.get("dynamic-opacity").await)
    else {
        return false;
    };
    let everyone = keys.get("enable-all").await.is_ok_and(|v| v == "true");
    blur == "true" && opacity == "255" && dynamic == "false" && (everyone || listed(&whitelist).contains(&product::ID))
}

fn schemas(info: &Info) -> PathBuf {
    info.path.join("schemas")
}

pub async fn probe(conn: &Connection, gsettings: &Path) -> State {
    State { wallpaper: wallpaper(gsettings).await, ..extension_state(conn, gsettings).await }
}

async fn extension_state(conn: &Connection, gsettings: &Path) -> State {
    let Ok(found) = info(conn).await else { return State { extension: "unknown", ..State::default() } };
    let Some(found) = found else { return State { extension: "absent", ..State::default() } };
    let version = found.version.clone();
    if found.state != ACTIVE {
        return State { extension: "off", version, ..State::default() };
    }
    let keys = Keys { bin: gsettings, schemas: schemas(&found) };
    State { extension: "active", version, works: applied(&keys).await, look: looks_right(&keys).await, ..State::default() }
}

pub async fn state(conn: &Connection, gsettings: &Path, library: &Library) -> State {
    State { undo: settings::GLASS_UNDO.get::<Saved>(library).is_some(), ..probe(conn, gsettings).await }
}

async fn settled(conn: &Connection, want_active: bool) -> Result<Option<Info>> {
    let mut last = info(conn).await?;
    for _ in 0..SETTLE_TRIES {
        match &last {
            Some(found) if !want_active || found.state == ACTIVE => break,
            _ => {
                tokio::time::sleep(SETTLE_STEP).await;
                last = info(conn).await?;
            }
        }
    }
    Ok(last)
}

pub async fn setup(conn: &Connection, gsettings: &Path, library: &Library) -> Result<Setup> {
    let before = info(conn).await.map_err(|e| Code::ShellBlur.err(format!("{e:#}")))?;
    let first_time = before.is_none();
    let was_enabled = before.as_ref().is_some_and(|b| b.enabled);
    if first_time {
        let answer = tokio::time::timeout(DIALOG_LIMIT, tray::install_extension(conn, EXTENSION)).await;
        journal::info("glass", format!("GNOME answered the install: {answer:?}"));
        if let Ok(Err(e)) = answer {
            return Err(Code::ShellBlur.err(format!("{e:#}")));
        }
        if settled(conn, false).await?.is_none() {
            return Ok(Setup::Skipped);
        }
    }
    let shell = ExtensionsProxy::new(conn).await?;
    if !info(conn).await?.is_some_and(|found| found.state == ACTIVE) {
        shell.enable_extension(EXTENSION).await.map_err(|e| Code::ShellBlur.err(e.to_string()))?;
    }
    let found = settled(conn, true)
        .await?
        .filter(|found| found.state == ACTIVE)
        .ok_or_else(|| Code::ShellBlur.err("the extension did not start"))?;
    let keys = Keys { bin: gsettings, schemas: schemas(&found) };
    let earlier = settings::GLASS_UNDO.get::<Saved>(library);
    let mut saved = match earlier {
        Some(saved) => saved,
        None => Saved {
            blur: keys.get("blur").await?,
            whitelist: keys.get("whitelist").await?,
            opacity: keys.get("opacity").await?,
            dynamic_opacity: keys.get("dynamic-opacity").await?,
            installed: first_time,
            was_enabled,
            ..Saved::default()
        },
    };
    if saved.pipeline.is_empty() {
        saved.pipeline = keys.get("pipeline").await?;
        saved.corner_radius = keys.get("corner-radius").await?;
        saved.rounded_corners = keys.get("rounded-corners").await?;
    }
    settings::GLASS_UNDO.set(library, &saved);
    let whitelist = with_us(&keys.get("whitelist").await?);
    keys.set("blur", "true").await?;
    keys.set("whitelist", &whitelist).await?;
    keys.set("opacity", "255").await?;
    keys.set("dynamic-opacity", "false").await?;
    keys.set_pipelines(&with_ours(&keys.pipelines().await?)).await?;
    keys.set("pipeline", &format!("'{PIPELINE}'")).await?;
    keys.set("corner-radius", CORNER_RADIUS).await?;
    keys.set("rounded-corners", ALL_CORNERS).await?;
    if !applied(&keys).await || !looks_right(&keys).await {
        return Err(Code::ShellBlur.err("the settings did not take"));
    }
    Ok(Setup::Applied)
}

pub async fn undo(conn: &Connection, gsettings: &Path, library: &Library) -> Result<()> {
    let Some(saved) = settings::GLASS_UNDO.get::<Saved>(library) else { return Ok(()) };
    let shell = ExtensionsProxy::new(conn).await?;
    if let Some(found) = info(conn).await? {
        let keys = Keys { bin: gsettings, schemas: schemas(&found) };
        keys.set("blur", &saved.blur).await?;
        keys.set("whitelist", &saved.whitelist).await?;
        keys.set("opacity", &saved.opacity).await?;
        keys.set("dynamic-opacity", &saved.dynamic_opacity).await?;
        if !saved.pipeline.is_empty() {
            keys.set("pipeline", &saved.pipeline).await?;
            keys.set("corner-radius", &saved.corner_radius).await?;
            keys.set("rounded-corners", &saved.rounded_corners).await?;
        }
        let pipelines = keys.pipelines().await?;
        if entry_span(&pipelines).is_some() {
            keys.set_pipelines(&without_ours(&pipelines)).await?;
        }
        if saved.installed {
            shell.uninstall_extension(EXTENSION).await?;
        } else if !saved.was_enabled {
            shell.disable_extension(EXTENSION).await?;
        }
    }
    settings::GLASS_UNDO.set(library, &serde_json::Value::Null);
    Ok(())
}

pub async fn watch(conn: Connection, changed: impl Fn() + Send + 'static) -> Result<()> {
    let shell = ExtensionsProxy::new(&conn).await?;
    let mut signals = shell.receive_extension_state_changed().await?;
    while let Some(signal) = signals.next().await {
        if signal.args().is_ok_and(|args| args.uuid == EXTENSION) {
            changed();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};

    use zbus::{Variant, fdo};

    use super::*;
    use crate::testkit::Bus;

    const THEIR_PIPELINES: &str = "{'pipeline_default': {'name': <'Default'>, 'effects': <[<{'type': <'dual_kawase_blur'>}>]>}, 'pipeline_mine': {'name': <'It\\'s {mine}'>, 'effects': <@av []>}}";

    #[derive(Default)]
    struct Shell {
        present: bool,
        active: bool,
        on_install: &'static str,
        asked: Vec<String>,
    }

    struct FakeShell(Arc<Mutex<Shell>>);

    #[zbus::interface(name = "org.gnome.Shell.Extensions")]
    impl FakeShell {
        fn get_extension_info(&self, _uuid: &str) -> HashMap<String, OwnedVariant> {
            let shell = self.0.lock().unwrap();
            if !shell.present {
                return HashMap::new();
            }
            let owned = |v: Variant<'_>| OwnedVariant::try_from(v).unwrap();
            HashMap::from([
                ("state".to_string(), owned(Variant::F64(if shell.active { 1.0 } else { 2.0 }))),
                ("enabled".to_string(), owned(Variant::Bool(shell.active))),
                ("version".to_string(), owned(Variant::F64(77.0))),
                ("path".to_string(), owned(Variant::from("/no/such/extension"))),
            ])
        }

        fn install_remote_extension(&self, uuid: &str) -> fdo::Result<String> {
            let mut shell = self.0.lock().unwrap();
            shell.asked.push(format!("install {uuid}"));
            match shell.on_install {
                "no reply, installed" => {
                    shell.present = true;
                    shell.active = true;
                    Err(fdo::Error::NoReply("the dialog went away".into()))
                }
                "cancelled" => Ok("cancelled".into()),
                "failed" => Err(fdo::Error::Failed("no network".into())),
                _ => {
                    shell.present = true;
                    shell.active = true;
                    Ok("successful".into())
                }
            }
        }

        fn enable_extension(&self, uuid: &str) -> bool {
            let mut shell = self.0.lock().unwrap();
            shell.asked.push(format!("enable {uuid}"));
            shell.active = true;
            true
        }

        fn disable_extension(&self, uuid: &str) -> bool {
            let mut shell = self.0.lock().unwrap();
            shell.asked.push(format!("disable {uuid}"));
            shell.active = false;
            true
        }

        fn uninstall_extension(&self, uuid: &str) -> bool {
            let mut shell = self.0.lock().unwrap();
            shell.asked.push(format!("uninstall {uuid}"));
            shell.present = false;
            shell.active = false;
            true
        }
    }

    struct Rig {
        _bus: Bus,
        _service: Connection,
        me: Connection,
        shell: Arc<Mutex<Shell>>,
        dir: PathBuf,
        gsettings: PathBuf,
        library: Library,
    }

    impl Rig {
        async fn start(name: &str, shell: Shell) -> Rig {
            let bus = Bus::start();
            let shell = Arc::new(Mutex::new(shell));
            let service = zbus::connection::Builder::address(bus.address.as_str())
                .name("org.gnome.Shell.Extensions")
                .serve_at("/org/gnome/Shell/Extensions", FakeShell(shell.clone()))
                .build()
                .await
                .unwrap();
            let me = zbus::connection::Builder::address(bus.address.as_str()).build().await.unwrap();
            let dir = std::env::temp_dir().join(format!("snellar-glass-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            let gsettings = dir.join("gsettings");
            let script = format!(
                "#!/bin/sh\nd='{}'\nkey=$3\ncase $1 in\nget) [ -f \"$d/$key\" ] || exit 1; cat \"$d/$key\";;\nset) printf '%s\\n' \"$4\" > \"$d/$key\"; echo \"$key=$4\" >> \"$d/writes\";;\nesac\n",
                dir.display()
            );
            std::fs::write(&gsettings, script).unwrap();
            std::fs::set_permissions(&gsettings, std::fs::Permissions::from_mode(0o755)).unwrap();
            for (key, value) in [
                ("blur", "false"),
                ("whitelist", "@as []"),
                ("opacity", "215"),
                ("dynamic-opacity", "true"),
                ("enable-all", "false"),
                ("pipeline", "'pipeline_default'"),
                ("pipelines", THEIR_PIPELINES),
                ("corner-radius", "15"),
                ("rounded-corners", "1"),
            ] {
                std::fs::write(dir.join(key), format!("{value}\n")).unwrap();
            }
            Rig { _bus: bus, _service: service, me, shell, dir, gsettings, library: Library::open(Path::new(":memory:")).unwrap() }
        }

        fn key(&self, key: &str) -> String {
            std::fs::read_to_string(self.dir.join(key)).unwrap().trim().to_string()
        }

        fn writes(&self) -> Vec<String> {
            std::fs::read_to_string(self.dir.join("writes")).unwrap_or_default().lines().map(str::to_string).collect()
        }

        fn asked(&self) -> Vec<String> {
            self.shell.lock().unwrap().asked.clone()
        }

        async fn state(&self) -> State {
            state(&self.me, &self.gsettings, &self.library).await
        }

        async fn setup(&self) -> Result<Setup> {
            setup(&self.me, &self.gsettings, &self.library).await
        }

        async fn undo(&self) -> Result<()> {
            undo(&self.me, &self.gsettings, &self.library).await
        }
    }

    #[tokio::test]
    async fn the_whole_way_in_and_back_out_when_snellar_installed_it() {
        let rig = Rig::start("full", Shell::default()).await;
        let before = rig.state().await;
        assert_eq!((before.extension, before.works, before.undo), ("absent", false, false));
        assert!(rig.writes().is_empty(), "reading changes nothing");
        assert_eq!(rig.setup().await.unwrap(), Setup::Applied);
        assert_eq!(rig.asked(), [format!("install {EXTENSION}")]);
        assert_eq!(
            (rig.key("blur"), rig.key("whitelist"), rig.key("opacity"), rig.key("dynamic-opacity")),
            ("true".to_string(), "['snellar']".to_string(), "255".to_string(), "false".to_string())
        );
        assert_eq!(rig.key("pipeline"), "'pipeline_snellar'");
        assert_eq!((rig.key("corner-radius"), rig.key("rounded-corners")), ("12".to_string(), "0".to_string()));
        assert!(rig.key("pipelines").contains("'unscaled_radius': <200>"), "the look is written");
        let written: Vec<String> = rig.writes().iter().filter_map(|w| w.split('=').next().map(str::to_string)).collect();
        assert_eq!(
            written,
            ["blur", "whitelist", "opacity", "dynamic-opacity", "pipelines", "pipeline", "corner-radius", "rounded-corners"],
            "exactly the keys the page names"
        );
        let after = rig.state().await;
        assert_eq!((after.extension, after.works, after.look, after.undo, after.version.as_str()), ("active", true, true, true, "77"));
        rig.undo().await.unwrap();
        assert_eq!(
            (rig.key("blur"), rig.key("whitelist"), rig.key("opacity"), rig.key("dynamic-opacity")),
            ("false".to_string(), "@as []".to_string(), "215".to_string(), "true".to_string()),
            "the old values, exactly"
        );
        assert_eq!(rig.key("pipeline"), "'pipeline_default'", "the old pipeline, exactly");
        assert_eq!((rig.key("corner-radius"), rig.key("rounded-corners")), ("15".to_string(), "1".to_string()));
        assert_eq!(rig.key("pipelines"), THEIR_PIPELINES, "ours is gone, theirs are as they were");
        assert_eq!(rig.asked().last().unwrap(), &format!("uninstall {EXTENSION}"), "what Snellar installed leaves");
        let gone = rig.state().await;
        assert_eq!((gone.extension, gone.works, gone.undo), ("absent", false, false));
    }

    #[tokio::test]
    async fn no_reply_is_not_a_verdict_the_state_decides() {
        let rig = Rig::start("noreply", Shell { on_install: "no reply, installed", ..Shell::default() }).await;
        assert_eq!(rig.setup().await.unwrap(), Setup::Applied, "the extension is there and active");
        assert!(rig.state().await.works);
    }

    #[tokio::test]
    async fn saying_no_in_gnomes_dialog_changes_nothing() {
        let rig = Rig::start("cancel", Shell { on_install: "cancelled", ..Shell::default() }).await;
        assert_eq!(rig.setup().await.unwrap(), Setup::Skipped);
        assert!(rig.writes().is_empty());
        assert!(!rig.state().await.undo, "nothing to undo");
    }

    #[tokio::test]
    async fn a_failed_install_is_told_and_changes_nothing() {
        let rig = Rig::start("fail", Shell { on_install: "failed", ..Shell::default() }).await;
        assert_eq!(Code::of(&rig.setup().await.unwrap_err()), Code::ShellBlur);
        assert!(rig.writes().is_empty());
    }

    #[tokio::test]
    async fn an_extension_the_person_had_stays_after_undo_and_their_values_return() {
        let rig = Rig::start("had", Shell { present: true, active: true, ..Shell::default() }).await;
        std::fs::write(rig.dir.join("whitelist"), "['kitty', 'foot']\n").unwrap();
        assert_eq!(rig.setup().await.unwrap(), Setup::Applied);
        assert_eq!(rig.key("whitelist"), "['kitty', 'foot', 'snellar']", "their apps stay");
        assert!(rig.asked().is_empty(), "no dialog, it is already there");
        rig.undo().await.unwrap();
        assert_eq!(rig.key("whitelist"), "['kitty', 'foot']");
        assert!(rig.asked().is_empty(), "their extension stays on");
        assert!(rig.shell.lock().unwrap().present);
    }

    #[tokio::test]
    async fn a_disabled_extension_is_turned_on_and_back_off() {
        let rig = Rig::start("off", Shell { present: true, active: false, ..Shell::default() }).await;
        assert_eq!(rig.state().await.extension, "off");
        rig.setup().await.unwrap();
        assert_eq!(rig.asked(), [format!("enable {EXTENSION}")]);
        rig.undo().await.unwrap();
        assert_eq!(rig.asked().last().unwrap(), &format!("disable {EXTENSION}"));
    }

    #[tokio::test]
    async fn glass_stops_counting_when_any_setting_or_the_extension_stops() {
        let rig = Rig::start("stops", Shell { present: true, active: true, ..Shell::default() }).await;
        rig.setup().await.unwrap();
        assert!(rig.state().await.works);
        std::fs::write(rig.dir.join("opacity"), "200\n").unwrap();
        assert!(!rig.state().await.works, "opacity changed by hand");
        std::fs::write(rig.dir.join("opacity"), "255\n").unwrap();
        std::fs::write(rig.dir.join("whitelist"), "@as []\n").unwrap();
        assert!(!rig.state().await.works, "taken off the list");
        std::fs::write(rig.dir.join("enable-all"), "true\n").unwrap();
        assert!(rig.state().await.works, "unless every app is blurred");
        rig.shell.lock().unwrap().active = false;
        assert!(!rig.state().await.works, "extension off");
    }

    #[tokio::test]
    async fn a_second_setup_keeps_the_first_saved_values() {
        let rig = Rig::start("twice", Shell::default()).await;
        rig.setup().await.unwrap();
        std::fs::write(rig.dir.join("opacity"), "100\n").unwrap();
        rig.setup().await.unwrap();
        rig.undo().await.unwrap();
        assert_eq!(rig.key("opacity"), "215", "the value before Snellar touched anything");
    }

    #[tokio::test]
    async fn without_gnome_shell_nothing_is_said_to_work_and_setup_says_why() {
        let bus = Bus::start();
        let me = zbus::connection::Builder::address(bus.address.as_str()).build().await.unwrap();
        let library = Library::open(Path::new(":memory:")).unwrap();
        let found = state(&me, Path::new("/no/such/gsettings"), &library).await;
        assert_eq!((found.extension, found.works), ("unknown", false));
        assert_eq!(Code::of(&setup(&me, Path::new("/no/such/gsettings"), &library).await.unwrap_err()), Code::ShellBlur);
    }

    #[tokio::test]
    async fn a_pipeline_made_after_setup_survives_undo() {
        let rig = Rig::start("theirs", Shell { present: true, active: true, ..Shell::default() }).await;
        rig.setup().await.unwrap();
        let with_new = rig.key("pipelines").replacen(
            "{'pipeline_snellar'",
            "{'pipeline_new': {'name': <'New'>, 'effects': <@av []>}, 'pipeline_snellar'",
            1,
        );
        std::fs::write(rig.dir.join("pipelines"), format!("{with_new}\n")).unwrap();
        rig.undo().await.unwrap();
        let left = rig.key("pipelines");
        assert!(left.contains("'pipeline_new'") && left.contains("'pipeline_mine'"), "{left}");
        assert!(!left.contains(PIPELINE), "{left}");
    }

    #[tokio::test]
    async fn someone_who_set_up_the_four_keys_keeps_working_and_can_move_to_the_look() {
        let rig = Rig::start("old", Shell { present: true, active: true, ..Shell::default() }).await;
        for (key, value) in [("blur", "true"), ("whitelist", "['snellar']"), ("opacity", "255"), ("dynamic-opacity", "false")] {
            std::fs::write(rig.dir.join(key), format!("{value}\n")).unwrap();
        }
        let old = serde_json::json!({ "blur": "false", "whitelist": "@as []", "opacity": "215", "dynamicOpacity": "true", "installed": false, "wasEnabled": true });
        settings::GLASS_UNDO.set(&rig.library, &old);
        let before = rig.state().await;
        assert_eq!((before.works, before.look, before.undo), (true, false, true), "works, says the look is not there yet");
        assert!(rig.writes().is_empty(), "nothing moves without the click");
        assert_eq!(rig.setup().await.unwrap(), Setup::Applied);
        assert!(rig.state().await.look);
        rig.undo().await.unwrap();
        assert_eq!(
            (rig.key("blur"), rig.key("opacity"), rig.key("pipeline"), rig.key("corner-radius")),
            ("false".to_string(), "215".to_string(), "'pipeline_default'".to_string(), "15".to_string()),
            "back past both setups"
        );
        assert_eq!(rig.key("pipelines"), THEIR_PIPELINES);
    }

    #[tokio::test]
    async fn undo_of_an_old_save_leaves_the_pipeline_it_never_changed() {
        let rig = Rig::start("oldundo", Shell { present: true, active: true, ..Shell::default() }).await;
        std::fs::write(rig.dir.join("pipeline"), "'pipeline_mine'\n").unwrap();
        let old = serde_json::json!({ "blur": "false", "whitelist": "@as []", "opacity": "215", "dynamicOpacity": "true", "installed": false, "wasEnabled": true });
        settings::GLASS_UNDO.set(&rig.library, &old);
        rig.undo().await.unwrap();
        assert_eq!(rig.key("pipeline"), "'pipeline_mine'");
        assert_eq!(rig.key("pipelines"), THEIR_PIPELINES);
    }

    #[tokio::test]
    async fn the_wallpaper_is_read_for_the_colour_scheme_in_use() {
        let rig = Rig::start("wall", Shell { present: true, active: true, ..Shell::default() }).await;
        assert_eq!(rig.state().await.wallpaper, "", "no key, no wallpaper");
        std::fs::write(rig.dir.join("picture-uri"), "'file:///light%20one.jpg'\n").unwrap();
        std::fs::write(rig.dir.join("picture-uri-dark"), "'file:///dark.jpg'\n").unwrap();
        std::fs::write(rig.dir.join("color-scheme"), "'default'\n").unwrap();
        assert_eq!(rig.state().await.wallpaper, "file:///light%20one.jpg");
        std::fs::write(rig.dir.join("color-scheme"), "'prefer-dark'\n").unwrap();
        assert_eq!(rig.state().await.wallpaper, "file:///dark.jpg");
        std::fs::write(rig.dir.join("picture-uri-dark"), "'https://example.invalid/a.jpg'\n").unwrap();
        assert_eq!(rig.state().await.wallpaper, "", "only a local file");
    }

    #[test]
    fn our_pipeline_goes_in_and_out_without_touching_the_rest() {
        let added = with_ours(THEIR_PIPELINES);
        assert!(added.starts_with("{'pipeline_snellar': {'name': <'Snellar'>"));
        assert_eq!(without_ours(&added), THEIR_PIPELINES);
        assert_eq!(with_ours(&added), added, "a second time changes nothing");
        assert_eq!(without_ours(THEIR_PIPELINES), THEIR_PIPELINES);
        let alone = with_ours("@a{sa{sv}} {}");
        assert_eq!(alone, format!("{{'pipeline_snellar': {PIPELINE_BODY}}}"));
        assert_eq!(without_ours(&alone), "{}");
        let renamed = "{'pipeline_snellar': {'name': <\"It's {ours}\">, 'effects': <@av []>}, 'pipeline_a': {'name': <'A'>}}";
        assert_eq!(without_ours(renamed), "{'pipeline_a': {'name': <'A'>}}", "a quote inside a name is not the end of it");
        let last = format!("{{'pipeline_a': {{'name': <'A'>}}, 'pipeline_snellar': {PIPELINE_BODY}}}");
        assert_eq!(without_ours(&last), "{'pipeline_a': {'name': <'A'>}}");
    }

    #[test]
    fn gsettings_lists_are_read_and_extended() {
        assert_eq!(listed("@as []"), Vec::<&str>::new());
        assert_eq!(listed("['a', 'b c']"), ["a", "b c"]);
        assert_eq!(with_us("@as []"), "['snellar']");
        assert_eq!(with_us("['snellar', 'a']"), "['snellar', 'a']");
    }
}
