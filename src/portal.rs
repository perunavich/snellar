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
use zbus::{MatchRule, MessageStream, OwnedObjectPath, OwnedVariant, Variant};

use snellar_core::error::Code;
use snellar_core::journal;

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const ANSWER_LIMIT: Duration = Duration::from_secs(10);

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

async fn ask(address: Option<&str>, spec: &Spec, limit: Duration) -> Result<Outcome> {
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address),
        None => zbus::connection::Builder::session(),
    };
    let conn = timeout(limit, builder.build()).await.context("the session bus does not answer")??;
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use zbus::message::Header;
    use zbus::{Connection, fdo};

    use super::*;
    use crate::testkit::Bus;

    const LIMIT: Duration = Duration::from_millis(800);

    #[derive(Clone)]
    enum Act {
        Respond(u32, Vec<String>),
        RespondLate(u32, Vec<String>),
        Fail,
        Silent,
        Vanish,
    }

    struct Fake {
        act: Act,
        seen: Seen,
    }

    #[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
    impl Fake {
        async fn open_file(
            &self,
            #[zbus(header)] header: Header<'_>,
            #[zbus(connection)] conn: &Connection,
            parent: String,
            title: String,
            options: HashMap<String, OwnedVariant>,
        ) -> fdo::Result<OwnedObjectPath> {
            let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
            self.seen.lock().unwrap().push((parent, title, options));
            let sender = header.sender().unwrap().trim_start_matches(':').replace('.', "_");
            let handle = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
            let path = OwnedObjectPath::try_from(handle.clone()).unwrap();
            let respond = |code: u32, uris: Vec<String>, delay: Duration| {
                let conn = conn.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let mut results: HashMap<&str, Variant> = HashMap::new();
                    results.insert("uris", uris.into());
                    conn.emit_signal(None::<&str>, handle.as_str(), "org.freedesktop.portal.Request", "Response", &(code, results))
                        .await
                        .unwrap();
                });
            };
            match self.act.clone() {
                Act::Respond(code, uris) => respond(code, uris, Duration::from_millis(50)),
                Act::RespondLate(code, uris) => respond(code, uris, LIMIT * 3),
                Act::Fail => return Err(fdo::Error::Failed("the backend said no".into())),
                Act::Silent => std::future::pending::<()>().await,
                Act::Vanish => {
                    let conn = conn.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        conn.release_name("org.freedesktop.portal.Desktop").await.unwrap();
                    });
                }
            }
            Ok(path)
        }
    }

    type Seen = Arc<Mutex<Vec<(String, String, HashMap<String, OwnedVariant>)>>>;

    async fn portal(bus: &Bus, act: Act) -> (Connection, Seen) {
        let seen: Seen = Arc::default();
        let conn = zbus::connection::Builder::address(bus.address.as_str())
            .name("org.freedesktop.portal.Desktop")
            .serve_at("/org/freedesktop/portal/desktop", Fake { act, seen: seen.clone() })
            .build()
            .await
            .unwrap();
        (conn, seen)
    }

    fn cover() -> Spec {
        Spec {
            title: "Choose a cover".into(),
            filters: vec!["Images (*.png *.jpg)".into(), "No patterns".into()],
            start: "/tmp/My Music".into(),
            ..Spec::default()
        }
    }

    fn uri(s: &str) -> Vec<String> {
        vec![s.to_string()]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_picked_file_comes_back_as_a_plain_path_with_the_options_sent() {
        let bus = Bus::start();
        let (_portal, seen) = portal(&bus, Act::Respond(0, uri("file:///tmp/My%20Music/a%C3%A9.png"))).await;
        assert_eq!(pick_on(Some(&bus.address), &cover(), LIMIT).await, Outcome::Picked("/tmp/My Music/aé.png".into()));
        let seen = seen.lock().unwrap();
        let (parent, title, options) = &seen[0];
        assert_eq!((parent.as_str(), title.as_str()), ("", "Choose a cover"));
        assert!(!bool::try_from(&options["directory"]).unwrap());
        let start = Vec::<u8>::try_from(options["current_folder"].try_clone().unwrap()).unwrap();
        assert_eq!(start, b"/tmp/My Music\0");
        let filters = Vec::<(String, Vec<(u32, String)>)>::try_from(options["filters"].try_clone().unwrap()).unwrap();
        assert_eq!(filters, [("Images (*.png *.jpg)".to_string(), vec![(0, "*.png".to_string()), (0, "*.jpg".to_string())])]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_folder_asks_for_a_directory() {
        let bus = Bus::start();
        let (_portal, seen) = portal(&bus, Act::Respond(0, uri("file:///tmp/m"))).await;
        let spec = Spec { folder: true, title: "Folder".into(), ..Spec::default() };
        assert_eq!(pick_on(Some(&bus.address), &spec, LIMIT).await, Outcome::Picked("/tmp/m".into()));
        let seen = seen.lock().unwrap();
        assert!(bool::try_from(&seen[0].2["directory"]).unwrap());
        assert!(!seen[0].2.contains_key("current_folder") && !seen[0].2.contains_key("filters"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_is_not_a_failure() {
        let bus = Bus::start();
        let (_portal, _) = portal(&bus, Act::Respond(1, vec![])).await;
        assert_eq!(pick_on(Some(&bus.address), &cover(), LIMIT).await, Outcome::Cancelled);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_portal_on_the_bus_is_unavailable_at_once() {
        let bus = Bus::start();
        let started = std::time::Instant::now();
        assert_eq!(pick_on(Some(&bus.address), &cover(), Duration::from_secs(30)).await, Outcome::Unavailable);
        assert!(started.elapsed() < Duration::from_secs(5), "no waiting for what is not there");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_bus_is_unavailable() {
        assert_eq!(pick_on(Some("unix:path=/nonexistent/bus"), &cover(), LIMIT).await, Outcome::Unavailable);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_portal_error_is_unavailable() {
        let bus = Bus::start();
        let (_portal, _) = portal(&bus, Act::Fail).await;
        assert_eq!(pick_on(Some(&bus.address), &cover(), LIMIT).await, Outcome::Unavailable);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dialog_that_ended_some_other_way_is_unavailable() {
        let bus = Bus::start();
        let (_portal, _) = portal(&bus, Act::Respond(2, vec![])).await;
        assert_eq!(pick_on(Some(&bus.address), &cover(), LIMIT).await, Outcome::Unavailable);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_silent_service_times_out() {
        let bus = Bus::start();
        let (_portal, _) = portal(&bus, Act::Silent).await;
        assert_eq!(pick_on(Some(&bus.address), &cover(), LIMIT).await, Outcome::Unavailable);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_person_may_think_past_the_limit() {
        let bus = Bus::start();
        let (_portal, _) = portal(&bus, Act::RespondLate(0, uri("file:///tmp/late.csv"))).await;
        assert_eq!(pick_on(Some(&bus.address), &cover(), LIMIT).await, Outcome::Picked("/tmp/late.csv".into()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_portal_that_goes_away_ends_the_wait() {
        let bus = Bus::start();
        let (_portal, _) = portal(&bus, Act::Vanish).await;
        let answer = tokio::time::timeout(Duration::from_secs(10), pick_on(Some(&bus.address), &cover(), LIMIT)).await;
        assert_eq!(answer.expect("never blocks forever"), Outcome::Unavailable);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn only_local_files_count() {
        let bus = Bus::start();
        let (_portal, _) = portal(&bus, Act::Respond(0, uri("https://example.com/a.png"))).await;
        assert_eq!(pick_on(Some(&bus.address), &cover(), LIMIT).await, Outcome::Unavailable);
    }
}
