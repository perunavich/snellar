use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use futures_util::StreamExt;
use serde::Deserialize;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, fdo};
use zbus::{ObjectPath, OwnedVariant, Variant};

use snellar_core::error::Code;
use snellar_core::sync::Lock;
use snellar_core::{journal, product};

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Spec {
    pub on: bool,
    pub title: String,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Signal {
    Ready(bool),
    Action(String),
}

pub type Notify = Arc<dyn Fn(Signal) + Send + Sync>;

struct Shared {
    spec: Spec,
    revision: u32,
    notify: Notify,
}

impl Shared {
    fn act(&self, action: &str) {
        (self.notify)(Signal::Action(action.to_owned()));
    }
}

type State = Arc<Mutex<Shared>>;

struct Item(State);

type Pixmap = (i32, i32, Vec<u8>);
type ToolTip = (String, Vec<Pixmap>, String, String);

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        product::ID
    }

    #[zbus(property)]
    fn title(&self) -> String {
        self.0.locked().spec.title.clone()
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        product::ID
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> String {
        icon_theme(&product::cache_dir()).unwrap_or_default()
    }

    #[zbus(property)]
    fn tool_tip(&self) -> ToolTip {
        (product::ID.to_owned(), Vec::new(), self.0.locked().spec.title.clone(), String::new())
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'_> {
        ObjectPath::from_static_str_unchecked(MENU_PATH)
    }

    fn activate(&self, _x: i32, _y: i32) {
        self.0.locked().act("window");
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        self.0.locked().act("playPause");
    }

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: &str) {}

    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

type Layout = (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>);

fn owned(value: Variant<'_>) -> OwnedVariant {
    OwnedVariant::try_from(value).expect("menu values hold no file descriptors")
}

fn properties(entry: &Entry) -> HashMap<String, OwnedVariant> {
    if entry.separator {
        return HashMap::from([("type".to_owned(), owned(Variant::from("separator")))]);
    }
    HashMap::from([
        ("label".to_owned(), owned(Variant::from(entry.label.as_str()))),
        ("enabled".to_owned(), owned(Variant::from(entry.enabled))),
        ("visible".to_owned(), owned(Variant::from(true))),
    ])
}

fn layout(spec: &Spec) -> Layout {
    let children = spec
        .entries
        .iter()
        .zip(1..)
        .map(|(entry, id): (&Entry, i32)| owned(Variant::from((id, properties(entry), Vec::<OwnedVariant>::new()))))
        .collect();
    (0, HashMap::from([("children-display".to_owned(), owned(Variant::from("submenu")))]), children)
}

struct Menu(State);

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(out_args("revision", "layout"))]
    fn get_layout(&self, _parent: i32, _depth: i32, _names: Vec<String>) -> (u32, Layout) {
        let shared = self.0.locked();
        (shared.revision, layout(&shared.spec))
    }

    fn get_group_properties(&self, ids: Vec<i32>, _names: Vec<String>) -> Vec<(i32, HashMap<String, OwnedVariant>)> {
        let shared = self.0.locked();
        let entries = shared.spec.entries.iter().zip(1..);
        entries.filter(|(_, id)| ids.is_empty() || ids.contains(id)).map(|(entry, id)| (id, properties(entry))).collect()
    }

    fn get_property(&self, id: i32, name: &str) -> fdo::Result<OwnedVariant> {
        let shared = self.0.locked();
        let entry = usize::try_from(id - 1).ok().and_then(|i| shared.spec.entries.get(i));
        entry.and_then(|e| properties(e).remove(name)).ok_or_else(|| fdo::Error::InvalidArgs(format!("no property {name} on {id}")))
    }

    fn event(&self, id: i32, event: &str, _data: Variant<'_>, _timestamp: u32) {
        if event != "clicked" {
            return;
        }
        let shared = self.0.locked();
        if let Some(entry) = usize::try_from(id - 1).ok().and_then(|i| shared.spec.entries.get(i)).filter(|e| e.enabled && !e.separator) {
            shared.act(&entry.id);
        }
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedVariant, u32)>) -> Vec<i32> {
        for (id, event, data, timestamp) in events {
            self.event(id, &event, Variant::from(data), timestamp);
        }
        Vec::new()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    #[zbus(signal)]
    async fn layout_updated(emitter: &SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.kde.StatusNotifierWatcher",
    default_service = "org.kde.StatusNotifierWatcher",
    default_path = "/StatusNotifierWatcher"
)]
trait Watcher {
    fn register_status_notifier_item(&self, service: &str) -> zbus::Result<()>;
}

pub struct Tray {
    state: State,
    address: Option<String>,
    serving: tokio::sync::Mutex<Option<(Connection, tokio::task::JoinHandle<()>)>>,
}

impl Tray {
    pub fn new(notify: Notify, address: Option<String>) -> Arc<Tray> {
        let state = Arc::new(Mutex::new(Shared { spec: Spec::default(), revision: 1, notify }));
        Arc::new(Tray { state, address, serving: tokio::sync::Mutex::default() })
    }

    pub async fn apply(&self, spec: Spec) -> Result<()> {
        let on = spec.on;
        let changed = {
            let mut shared = self.state.locked();
            let changed = shared.spec.title != spec.title || shared.spec.entries != spec.entries;
            if changed {
                shared.revision += 1;
            }
            shared.spec = spec;
            changed.then_some(shared.revision)
        };
        let mut serving = self.serving.lock().await;
        match (on, serving.is_some()) {
            (true, false) => *serving = Some(self.serve().await?),
            (false, true) => {
                if let Some((conn, watch)) = serving.take() {
                    watch.abort();
                    conn.release_name(item_name().as_str()).await?;
                }
                self.notify(Signal::Ready(false));
            }
            (true, true) => {
                if let (Some(revision), Some((conn, _))) = (changed, serving.as_ref()) {
                    let server = conn.object_server();
                    Menu::layout_updated(&SignalEmitter::new(conn, MENU_PATH)?, revision, 0).await?;
                    if let Ok(item) = server.interface::<_, Item>(ITEM_PATH).await {
                        Item::new_title(item.signal_emitter()).await?;
                        Item::new_tool_tip(item.signal_emitter()).await?;
                    }
                }
            }
            (false, false) => {}
        }
        Ok(())
    }

    fn notify(&self, signal: Signal) {
        let notify = self.state.locked().notify.clone();
        notify(signal);
    }

    async fn serve(&self) -> Result<(Connection, tokio::task::JoinHandle<()>)> {
        let name = item_name();
        let builder = match &self.address {
            Some(address) => zbus::connection::Builder::address(address.as_str()),
            None => zbus::connection::Builder::session(),
        };
        let conn = builder
            .name(name.as_str())
            .serve_at(ITEM_PATH, Item(self.state.clone()))
            .serve_at(MENU_PATH, Menu(self.state.clone()))
            .build()
            .await?;
        let watch = tokio::spawn(follow_watcher(conn.clone(), name, self.state.clone()));
        Ok((conn, watch))
    }
}

fn item_name() -> String {
    format!("org.kde.StatusNotifierItem-{}-1", std::process::id())
}

async fn register(conn: &Connection, name: &str) -> bool {
    let Ok(watcher) = WatcherProxy::new(conn).await else { return false };
    match watcher.register_status_notifier_item(name).await {
        Ok(()) => true,
        Err(e) => {
            journal::info("tray", format!("no tray host: {e}"));
            false
        }
    }
}

async fn follow_watcher(conn: Connection, name: String, state: State) {
    let tell = |ready| {
        let notify = state.locked().notify.clone();
        notify(Signal::Ready(ready));
    };
    let Ok(bus) = fdo::DBusProxy::new(&conn).await else { return };
    let Ok(mut changes) = bus.receive_name_owner_changed_with_args(&[(0, WATCHER)]).await else { return };
    tell(register(&conn, &name).await);
    while let Some(change) = changes.next().await {
        let Ok(args) = change.args() else { continue };
        let ready = args.new_owner().is_some() && register(&conn, &name).await;
        tell(ready);
    }
}

const ICON: &[u8] = include_bytes!("../../../packaging/icons/hicolor/64x64/apps/snellar.png");

fn icon_theme(cache: &Path) -> Option<String> {
    let root = cache.join("tray-icons");
    let sized = root.join("hicolor/64x64/apps");
    std::fs::create_dir_all(&sized).ok()?;
    for dir in [root.as_path(), sized.as_path()] {
        std::fs::write(dir.join(format!("{}.png", product::ID)), ICON).ok()?;
    }
    Some(root.to_string_lossy().into_owned())
}

const TRAY_EXTENSION: &str = "appindicatorsupport@rgcjonas.gmail.com";

#[zbus::proxy(
    interface = "org.gnome.Shell.Extensions",
    default_service = "org.gnome.Shell.Extensions",
    default_path = "/org/gnome/Shell/Extensions"
)]
trait ShellExtensions {
    fn install_remote_extension(&self, uuid: &str) -> zbus::Result<String>;
}

pub async fn install_extension(conn: &Connection, uuid: &str) -> Result<bool> {
    let shell = ShellExtensionsProxy::new(conn).await.map_err(|e| Code::ShellExtension.err(e.to_string()))?;
    match shell.install_remote_extension(uuid).await {
        Ok(answer) => {
            journal::info("tray", format!("GNOME answered {answer} for {uuid}"));
            Ok(answer == "successful")
        }
        Err(zbus::Error::FDO(e)) if matches!(*e, fdo::Error::NoReply(_)) => {
            journal::info("tray", format!("GNOME gave no answer for {uuid}"));
            Ok(false)
        }
        Err(e) => Err(Code::ShellExtension.err(e.to_string())),
    }
}

pub async fn add_tray_extension(conn: &Connection) -> Result<bool> {
    install_extension(conn, TRAY_EXTENSION).await
}
