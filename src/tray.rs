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
        self.0.lock().unwrap().spec.title.clone()
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
        (product::ID.to_owned(), Vec::new(), self.0.lock().unwrap().spec.title.clone(), String::new())
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
        self.0.lock().unwrap().act("window");
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        self.0.lock().unwrap().act("playPause");
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
        let shared = self.0.lock().unwrap();
        (shared.revision, layout(&shared.spec))
    }

    fn get_group_properties(&self, ids: Vec<i32>, _names: Vec<String>) -> Vec<(i32, HashMap<String, OwnedVariant>)> {
        let shared = self.0.lock().unwrap();
        let entries = shared.spec.entries.iter().zip(1..);
        entries.filter(|(_, id)| ids.is_empty() || ids.contains(id)).map(|(entry, id)| (id, properties(entry))).collect()
    }

    fn get_property(&self, id: i32, name: &str) -> fdo::Result<OwnedVariant> {
        let shared = self.0.lock().unwrap();
        let entry = usize::try_from(id - 1).ok().and_then(|i| shared.spec.entries.get(i));
        entry.and_then(|e| properties(e).remove(name)).ok_or_else(|| fdo::Error::InvalidArgs(format!("no property {name} on {id}")))
    }

    fn event(&self, id: i32, event: &str, _data: Variant<'_>, _timestamp: u32) {
        if event != "clicked" {
            return;
        }
        let shared = self.0.lock().unwrap();
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
            let mut shared = self.state.lock().unwrap();
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
        let notify = self.state.lock().unwrap().notify.clone();
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
        let notify = state.lock().unwrap().notify.clone();
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

const ICON: &[u8] = include_bytes!("../packaging/icons/hicolor/64x64/apps/snellar.png");

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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::fakes::until;
    use crate::testkit::Bus;

    struct FakeShell(&'static str, Arc<Mutex<Vec<String>>>);

    #[zbus::interface(name = "org.gnome.Shell.Extensions")]
    impl FakeShell {
        fn install_remote_extension(&self, uuid: &str) -> fdo::Result<String> {
            self.1.lock().unwrap().push(uuid.to_string());
            match self.0 {
                "no reply" => Err(fdo::Error::NoReply("the service left".into())),
                "failed" => Err(fdo::Error::Failed("no network".into())),
                answer => Ok(answer.to_string()),
            }
        }
    }

    async fn shell(bus: &Bus, answer: &'static str) -> (Connection, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let conn = zbus::connection::Builder::address(bus.address.as_str())
            .name("org.gnome.Shell.Extensions")
            .serve_at("/org/gnome/Shell/Extensions", FakeShell(answer, asked.clone()))
            .build()
            .await
            .unwrap();
        (conn, asked)
    }

    #[tokio::test]
    async fn gnome_answers_for_the_tray_extension() {
        let bus = Bus::start();
        let me = zbus::connection::Builder::address(bus.address.as_str()).build().await.unwrap();
        assert_eq!(Code::of(&add_tray_extension(&me).await.unwrap_err()), Code::ShellExtension, "no such service");
        let (_shell, asked) = shell(&bus, "successful").await;
        assert!(add_tray_extension(&me).await.unwrap(), "installed");
        assert_eq!(*asked.lock().unwrap(), ["appindicatorsupport@rgcjonas.gmail.com"]);
    }

    #[test]
    fn the_icon_travels_with_the_app() {
        let cache = std::env::temp_dir().join(format!("snellar-tray-icon-{}", std::process::id()));
        let root = icon_theme(&cache).unwrap();
        for file in [format!("{root}/{}.png", product::ID), format!("{root}/hicolor/64x64/apps/{}.png", product::ID)] {
            assert_eq!(std::fs::read(&file).unwrap(), ICON, "{file}");
        }
        assert_eq!(icon_theme(&cache).as_deref(), Some(root.as_str()), "a second time");
        std::fs::remove_dir_all(&cache).ok();
    }

    #[tokio::test]
    async fn gnome_going_quiet_after_the_dialog_is_not_a_failure() {
        let bus = Bus::start();
        let me = zbus::connection::Builder::address(bus.address.as_str()).build().await.unwrap();
        let (_shell, _) = shell(&bus, "no reply").await;
        assert!(!add_tray_extension(&me).await.unwrap());
    }

    #[tokio::test]
    async fn a_failed_install_is_reported() {
        let bus = Bus::start();
        let me = zbus::connection::Builder::address(bus.address.as_str()).build().await.unwrap();
        let (_shell, _) = shell(&bus, "failed").await;
        assert_eq!(Code::of(&add_tray_extension(&me).await.unwrap_err()), Code::ShellExtension);
    }

    #[tokio::test]
    async fn saying_no_in_the_dialog_is_not_an_error() {
        let bus = Bus::start();
        let me = zbus::connection::Builder::address(bus.address.as_str()).build().await.unwrap();
        let (_shell, _) = shell(&bus, "cancelled").await;
        assert!(!add_tray_extension(&me).await.unwrap());
    }

    #[derive(Default)]
    struct Hosted(Arc<Mutex<Vec<String>>>);

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Hosted {
        fn register_status_notifier_item(&self, service: &str) {
            self.0.lock().unwrap().push(service.to_owned());
        }
    }

    async fn host(bus: &Bus) -> (Connection, Arc<Mutex<Vec<String>>>) {
        let items = Arc::new(Mutex::new(Vec::new()));
        let conn = zbus::connection::Builder::address(bus.address.as_str())
            .name(WATCHER)
            .serve_at("/StatusNotifierWatcher", Hosted(items.clone()))
            .build()
            .await
            .unwrap();
        (conn, items)
    }

    fn spec(on: bool, play: &str) -> Spec {
        let entry = |id: &str, label: &str| Entry { id: id.into(), label: label.into(), enabled: true, separator: false };
        Spec {
            on,
            title: "Snellar".into(),
            entries: vec![
                entry("window", "Hide Snellar"),
                entry("playPause", play),
                entry("next", "Next"),
                Entry { separator: true, ..Entry::default() },
                entry("quit", "Quit Snellar"),
            ],
        }
    }

    fn rig(bus: &Bus) -> (Arc<Tray>, Arc<Mutex<Vec<Signal>>>) {
        let signals = Arc::new(Mutex::new(Vec::new()));
        let sink = signals.clone();
        (Tray::new(Arc::new(move |s| sink.lock().unwrap().push(s)), Some(bus.address.clone())), signals)
    }

    async fn call<R: serde::de::DeserializeOwned + zbus::zvariant::Type>(
        client: &Connection,
        path: &str,
        iface: &str,
        method: &str,
        body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    ) -> R {
        let name = item_name();
        client.call_method(Some(name.as_str()), path, Some(iface), method, body).await.unwrap().body().deserialize().unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_tray_registers_serves_its_menu_and_reports_clicks() {
        let bus = Bus::start();
        let (_host, items) = host(&bus).await;
        let (tray, signals) = rig(&bus);
        tray.apply(spec(true, "Pause")).await.unwrap();
        until("ready", || signals.lock().unwrap().contains(&Signal::Ready(true))).await;
        assert_eq!(*items.lock().unwrap(), [item_name()]);

        let client = zbus::connection::Builder::address(bus.address.as_str()).build().await.unwrap();
        let (revision, (root, _, children)): (u32, Layout) =
            call(&client, MENU_PATH, "com.canonical.dbusmenu", "GetLayout", &(0i32, -1i32, Vec::<String>::new())).await;
        assert_eq!((revision, root, children.len()), (2, 0, 5));
        let labels: Vec<Layout> = children.into_iter().map(|c| Layout::try_from(c).unwrap()).collect();
        let label = |i: usize| labels[i].1.get("label").map(|v| String::try_from(v.try_clone().unwrap()).unwrap());
        assert_eq!(
            (label(0).as_deref(), label(1).as_deref(), label(4).as_deref()),
            (Some("Hide Snellar"), Some("Pause"), Some("Quit Snellar"))
        );
        assert_eq!(String::try_from(labels[3].1["type"].try_clone().unwrap()).unwrap(), "separator");

        let _: () = call(&client, MENU_PATH, "com.canonical.dbusmenu", "Event", &(2i32, "clicked", Variant::from(0i32), 0u32)).await;
        let _: () = call(&client, MENU_PATH, "com.canonical.dbusmenu", "Event", &(3i32, "hovered", Variant::from(0i32), 0u32)).await;
        let _: () = call(&client, MENU_PATH, "com.canonical.dbusmenu", "Event", &(5i32, "clicked", Variant::from(0i32), 0u32)).await;
        let _: () = call(&client, ITEM_PATH, "org.kde.StatusNotifierItem", "Activate", &(0i32, 0i32)).await;
        let _: () = call(&client, ITEM_PATH, "org.kde.StatusNotifierItem", "SecondaryActivate", &(0i32, 0i32)).await;
        let actions: Vec<Signal> = signals.lock().unwrap().iter().filter(|s| matches!(s, Signal::Action(_))).cloned().collect();
        let expected = ["playPause", "quit", "window", "playPause"].map(|a| Signal::Action(a.into()));
        assert_eq!(actions, expected, "hover is not a click");

        tray.apply(spec(true, "Play")).await.unwrap();
        let (revision, (_, _, children)): (u32, Layout) =
            call(&client, MENU_PATH, "com.canonical.dbusmenu", "GetLayout", &(0i32, -1i32, Vec::<String>::new())).await;
        let second = Layout::try_from(children[1].try_clone().unwrap()).unwrap();
        assert_eq!((revision, String::try_from(second.1["label"].try_clone().unwrap()).unwrap()), (3, "Play".to_string()));

        tray.apply(spec(false, "Play")).await.unwrap();
        let bus_proxy = fdo::DBusProxy::new(&client).await.unwrap();
        let name = item_name();
        assert!(!bus_proxy.name_has_owner(name.as_str().try_into().unwrap()).await.unwrap(), "the item leaves the bus");
        assert_eq!(signals.lock().unwrap().last(), Some(&Signal::Ready(false)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn without_a_host_the_tray_is_not_ready_until_one_appears() {
        let bus = Bus::start();
        let (tray, signals) = rig(&bus);
        tray.apply(spec(true, "Play")).await.unwrap();
        until("not ready", || signals.lock().unwrap().contains(&Signal::Ready(false))).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (_host, items) = host(&bus).await;
        until("registered once the panel starts", || !items.lock().unwrap().is_empty()).await;
        until("ready", || signals.lock().unwrap().last() == Some(&Signal::Ready(true))).await;
    }
}
