use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::timeout;
use zbus::fdo::DBusProxy;
use zbus::message::Type;
use zbus::{Connection, MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, Variant};

use snellar_core::error::Code;
use snellar_core::journal;

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const ANSWER_LIMIT: Duration = Duration::from_secs(10);
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const MOTION_LIMIT: Duration = Duration::from_millis(500);

static TOKENS: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Spec {
    pub folder: bool,
    pub title: String,
    pub filters: Vec<String>,
    pub start: String,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Picked(String),
    Cancelled,
    Unavailable,
}

impl Outcome {
    pub fn to_json(&self) -> Value {
        match self {
            Outcome::Picked(path) => json!({ "status": "picked", "path": path }),
            Outcome::Cancelled => json!({ "status": "cancelled" }),
            Outcome::Unavailable => json!({ "status": "unavailable" }),
        }
    }
}

pub async fn answer(args: &Value) -> Result<Value> {
    let spec: Spec = serde_json::from_value(args.clone()).map_err(|e| Code::ApiArgs.err(format!("bad argument: {e}")))?;
    Ok(pick_on(None, &spec, ANSWER_LIMIT).await.to_json())
}

async fn pick_on(address: Option<&str>, spec: &Spec, limit: Duration) -> Outcome {
    ask(address, spec, limit).await.unwrap_or_else(|e| {
        journal::info("portal", format!("no file dialog from the portal: {e:#}"));
        Outcome::Unavailable
    })
}

pub async fn reduced_motion() -> bool {
    reduced_motion_on(None, MOTION_LIMIT).await
}

async fn reduced_motion_on(address: Option<&str>, limit: Duration) -> bool {
    timeout(limit, ask_motion(address)).await.ok().flatten().unwrap_or(false)
}

async fn ask_motion(address: Option<&str>) -> Option<bool> {
    let conn = bus(address).await.ok()?;
    if let Some(value) = read_setting(&conn, "org.freedesktop.appearance", "reduced-motion").await {
        return u32::try_from(value).ok().map(|preference| preference == 1);
    }
    let animations = read_setting(&conn, "org.gnome.desktop.interface", "enable-animations").await?;
    bool::try_from(animations).ok().map(|on| !on)
}

async fn read_setting(conn: &Connection, namespace: &str, key: &str) -> Option<OwnedVariant> {
    let reply = conn.call_method(Some(SERVICE), PATH, Some(SETTINGS), "ReadOne", &(namespace, key)).await.ok()?;
    reply.body().deserialize::<OwnedVariant>().ok()
}

async fn bus(address: Option<&str>) -> zbus::Result<Connection> {
    match address {
        Some(address) => zbus::connection::Builder::address(address).build().await,
        None => zbus::connection::Builder::session().build().await,
    }
}

async fn ask(address: Option<&str>, spec: &Spec, limit: Duration) -> Result<Outcome> {
    let conn = timeout(limit, bus(address)).await.context("the session bus does not answer")??;
    let rule = MatchRule::builder().msg_type(Type::Signal).interface("org.freedesktop.portal.Request").member("Response").build()?;
    let mut responses = MessageStream::for_match_rule(rule, &conn, None).await?;
    let mut owners = DBusProxy::new(&conn).await?.receive_name_owner_changed_with_args(&[(0, SERVICE)]).await?;

    let token = format!("snellar{}_{}", std::process::id(), TOKENS.fetch_add(1, Ordering::Relaxed));
    let body = ("", spec.title.as_str(), options(spec, &token));
    let call = conn.call_method(Some(SERVICE), PATH, Some("org.freedesktop.portal.FileChooser"), "OpenFile", &body);
    let handle: OwnedObjectPath = timeout(limit, call).await.context("the portal does not answer")??.body().deserialize()?;

    loop {
        tokio::select! {
            message = responses.next() => {
                let message = message.context("the session bus went away")??;
                if message.header().path().is_none_or(|path| path.as_str() != handle.as_str()) {
                    continue;
                }
                let (code, results): (u32, HashMap<String, OwnedVariant>) = message.body().deserialize()?;
                return outcome(code, &results);
            }
            change = owners.next() => {
                let change = change.context("the session bus went away")?;
                if change.args()?.new_owner().is_none() {
                    bail!("the portal went away");
                }
            }
        }
    }
}

fn outcome(code: u32, results: &HashMap<String, OwnedVariant>) -> Result<Outcome> {
    match code {
        0 => {
            let uris = results.get("uris").map(|u| Vec::<String>::try_from(u.try_clone()?)).transpose()?.unwrap_or_default();
            uris.first().map_or(Ok(Outcome::Cancelled), |uri| Ok(Outcome::Picked(local_path(uri)?)))
        }
        1 => Ok(Outcome::Cancelled),
        other => bail!("the dialog ended with response {other}"),
    }
}

fn options<'a>(spec: &'a Spec, token: &'a str) -> HashMap<&'static str, Variant<'a>> {
    let mut options = HashMap::from([("handle_token", Variant::from(token)), ("modal", true.into()), ("directory", spec.folder.into())]);
    let filters: Vec<_> = spec.filters.iter().filter_map(|f| filter(f)).collect();
    if !filters.is_empty() {
        options.insert("filters", filters.into());
    }
    if !spec.start.is_empty() {
        let mut start = spec.start.clone().into_bytes();
        start.push(0);
        options.insert("current_folder", start.into());
    }
    options
}

fn filter(text: &str) -> Option<(String, Vec<(u32, String)>)> {
    let (_, patterns) = text.rsplit_once('(')?;
    let globs: Vec<_> = patterns.strip_suffix(')')?.split_whitespace().map(|glob| (0, glob.to_string())).collect();
    (!globs.is_empty()).then(|| (text.to_string(), globs))
}

fn local_path(uri: &str) -> Result<String> {
    let rest = uri.strip_prefix("file://").with_context(|| format!("not a local file: {uri}"))?;
    let path = &rest[rest.find('/').context("a file address without a path")?..];
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%').then(|| path.get(i + 1..i + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok())).flatten();
        out.push(escaped.unwrap_or(bytes[i]));
        i += if escaped.is_some() { 3 } else { 1 };
    }
    Ok(String::from_utf8(out)?)
}
