#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, playing)]
        #[qproperty(bool, loading)]
        #[qproperty(f64, position)]
        #[qproperty(f64, duration)]
        #[qproperty(f64, volume)]
        #[qproperty(QString, current)]
        #[qproperty(QString, queue)]
        #[qproperty(i32, cursor)]
        #[qproperty(bool, shuffle)]
        #[qproperty(QString, repeat)]
        #[qproperty(bool, liked)]
        #[qproperty(QString, image_base)]
        #[qproperty(QString, stream)]
        #[qproperty(bool, tray_ready)]
        #[qproperty(QString, failure)]
        #[qproperty(QString, library)]
        type Core = super::CoreRust;

        #[qinvokable]
        fn request(self: Pin<&mut Core>, method: &QString, args: &QString) -> i32;

        #[qinvokable]
        fn settings(self: &Core) -> QString;

        #[qinvokable]
        fn desktop(self: &Core) -> QString;
        #[qinvokable]
        fn version(self: &Core) -> QString;

        #[qinvokable]
        fn tray(self: &Core, spec: &QString);

        #[qinvokable]
        #[cxx_name = "fetchYtdlp"]
        fn fetch_ytdlp(self: &Core);

        #[qinvokable]
        #[cxx_name = "checkUpdate"]
        fn check_update(self: &Core);

        #[qinvokable]
        #[cxx_name = "restartForUpdate"]
        fn restart_for_update(self: &Core);

        #[qinvokable]
        #[cxx_name = "refreshDesktop"]
        fn refresh_desktop(self: &Core);

        #[qinvokable]
        #[cxx_name = "setupGlass"]
        fn setup_glass(self: &Core);

        #[qinvokable]
        #[cxx_name = "undoGlass"]
        fn undo_glass(self: &Core);

        #[qinvokable]
        #[cxx_name = "addTray"]
        fn add_tray(self: &Core);

        #[qinvokable]
        #[cxx_name = "windowBar"]
        fn window_bar(self: &Core, background: &QString, text: &QString);

        #[qsignal]
        #[cxx_name = "trayAction"]
        fn tray_action(self: Pin<&mut Core>, action: QString);

        #[qsignal]
        fn reply(self: Pin<&mut Core>, id: i32, ok: bool, json: QString);

        #[qsignal]
        fn toast(self: Pin<&mut Core>, json: QString);

        #[qsignal]
        fn nudge(self: Pin<&mut Core>, json: QString);

        #[qsignal]
        #[cxx_name = "desktopChanged"]
        fn desktop_changed(self: Pin<&mut Core>, json: QString);

        #[qsignal]
        #[cxx_name = "toolsChanged"]
        fn tools_changed(self: Pin<&mut Core>);

        #[qsignal]
        #[cxx_name = "updateChanged"]
        fn update_changed(self: Pin<&mut Core>);

        #[qsignal]
        fn capture(self: Pin<&mut Core>, dir: QString);

        #[qsignal]
        #[cxx_name = "historyChanged"]
        fn history_changed(self: Pin<&mut Core>);

        #[qsignal]
        #[cxx_name = "importProgress"]
        fn import_progress(self: Pin<&mut Core>, json: QString);
    }

    impl cxx_qt::Threading for Core {}
    impl cxx_qt::Initialize for Core {}
}

use std::ffi::CString;
use std::os::raw::c_char;
use std::pin::Pin;
use std::sync::Arc;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

use snellar_core::ui::{Ui, UiEvent};
use snellar_core::{journal, settings};

use crate::app::App;

unsafe extern "C" {
    fn snellar_window_bar(background: *const c_char, text: *const c_char);
}

pub struct CoreRust {
    playing: bool,
    loading: bool,
    position: f64,
    duration: f64,
    volume: f64,
    current: QString,
    queue: QString,
    cursor: i32,
    shuffle: bool,
    repeat: QString,
    liked: bool,
    image_base: QString,
    stream: QString,
    tray_ready: bool,
    failure: QString,
    library: QString,
    app: Option<Arc<App>>,
    next_id: i32,
}

impl Default for CoreRust {
    fn default() -> Self {
        Self {
            playing: false,
            loading: false,
            position: 0.0,
            duration: 0.0,
            volume: 85.0,
            current: QString::from("null"),
            queue: QString::from("[]"),
            cursor: -1,
            shuffle: false,
            repeat: QString::from("off"),
            liked: false,
            image_base: QString::default(),
            stream: QString::from("null"),
            tray_ready: false,
            failure: QString::from("null"),
            library: QString::from("null"),
            app: None,
            next_id: 0,
        }
    }
}

fn json<T: serde::Serialize>(v: &T) -> QString {
    QString::from(serde_json::to_string(v).unwrap_or_else(|_| "null".into()).as_str())
}

fn apply(mut core: Pin<&mut qobject::Core>, ev: UiEvent) {
    match ev {
        UiEvent::Reply { id, ok, json } => core.reply(id, ok, QString::from(json.as_str())),
        UiEvent::State(s) => {
            core.as_mut().set_playing(s.playing);
            core.as_mut().set_loading(s.loading);
            core.as_mut().set_position(s.position);
            core.as_mut().set_duration(s.duration);
            core.as_mut().set_volume(s.volume);
        }
        UiEvent::Position(p) => core.set_position(p),
        UiEvent::Current(t) => core.set_current(json(&t)),
        UiEvent::Queue { tracks, cursor } => {
            core.as_mut().set_queue(json(&tracks));
            core.set_cursor(cursor);
        }
        UiEvent::Liked(v) => core.set_liked(v),
        UiEvent::History => core.history_changed(),
        UiEvent::Tools => core.tools_changed(),
        UiEvent::Update => core.update_changed(),
        UiEvent::Modes { shuffle, repeat } => {
            core.as_mut().set_shuffle(shuffle);
            let r = serde_json::to_value(repeat).ok().and_then(|v| v.as_str().map(String::from));
            core.set_repeat(QString::from(r.as_deref().unwrap_or("off")));
        }
        UiEvent::Toast(t) => core.toast(QString::from(t.json().as_str())),
        UiEvent::PlayFailure(f) => core.set_failure(json(&f)),
        UiEvent::Library(l) => core.set_library(json(&l)),
        UiEvent::Nudge(j) => core.nudge(QString::from(j.as_str())),
        UiEvent::Desktop(j) => core.desktop_changed(QString::from(j.as_str())),
        UiEvent::Capture(dir) => core.capture(QString::from(dir.as_str())),
        UiEvent::Import(p) => core.import_progress(json(&p)),
        UiEvent::Stream(s) => core.set_stream(json(&s)),
        UiEvent::TrayReady(ready) => core.set_tray_ready(ready),
        UiEvent::TrayAction(action) => core.tray_action(QString::from(action.as_str())),
    }
}

impl cxx_qt::Initialize for qobject::Core {
    fn initialize(mut self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        let beat = self.qt_thread();
        journal::watch_stalls(move |job| beat.queue(move |_| job()).is_ok());
        let ui: Ui = Arc::new(move |ev| {
            let _ = thread.queue(move |core| apply(core, ev));
        });
        let app = App::start(ui);
        self.as_mut().set_image_base(QString::from(app.image_base()));
        self.as_mut().rust_mut().app = Some(app);
    }
}

impl qobject::Core {
    pub fn request(mut self: Pin<&mut Self>, method: &QString, args: &QString) -> i32 {
        let mut rust = self.as_mut().rust_mut();
        rust.next_id += 1;
        let id = rust.next_id;
        if let Some(app) = rust.app.clone() {
            app.request(id, method.to_string(), args.to_string());
        }
        id
    }

    pub fn settings(&self) -> QString {
        json(&settings::registry())
    }

    pub fn version(&self) -> QString {
        QString::from(snellar_core::product::shown_version(snellar_core::product::VERSION).as_str())
    }

    pub fn desktop(&self) -> QString {
        json(crate::desktop::current())
    }

    pub fn window_bar(&self, background: &QString, text: &QString) {
        let (Ok(background), Ok(text)) = (CString::new(background.to_string()), CString::new(text.to_string())) else { return };
        // SAFETY: both strings outlive the call, which copies them, and it runs on the GUI thread after the application exists.
        unsafe { snellar_window_bar(background.as_ptr(), text.as_ptr()) };
    }

    pub fn fetch_ytdlp(&self) {
        if let Some(app) = &self.rust().app {
            app.fetch_ytdlp();
        }
    }

    pub fn check_update(&self) {
        if let Some(app) = &self.rust().app {
            app.check_update();
        }
    }

    pub fn restart_for_update(&self) {
        if let Some(app) = &self.rust().app {
            app.restart_for_update();
        }
    }

    pub fn refresh_desktop(&self) {
        if let Some(app) = &self.rust().app {
            app.refresh_desktop(false);
        }
    }

    pub fn setup_glass(&self) {
        if let Some(app) = &self.rust().app {
            app.setup_glass();
        }
    }

    pub fn undo_glass(&self) {
        if let Some(app) = &self.rust().app {
            app.undo_glass();
        }
    }

    pub fn add_tray(&self) {
        if let Some(app) = &self.rust().app {
            app.add_tray();
        }
    }

    pub fn tray(&self, spec: &QString) {
        if let Some(app) = &self.rust().app {
            app.tray(&spec.to_string());
        }
    }
}
