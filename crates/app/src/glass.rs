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
