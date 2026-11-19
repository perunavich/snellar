use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tokio::sync::{OnceCell, broadcast, mpsc};

use snellar_core::error::Code;
use snellar_core::journal;
use snellar_core::product;
use snellar_core::sync::Lock;
use snellar_core::ui::{Toast, Ui, UiEvent};
use snellar_youtube::proxy;

use crate::api::{Backend, LibraryGate};
use crate::handoff;
use crate::server;
use crate::tray::{self, Tray};

const QUIT_PATIENCE: Duration = Duration::from_secs(3);
const HAND_OFF: Duration = Duration::from_millis(250);

type Job = Pin<Box<dyn Future<Output = ()> + Send>>;

struct Lane(mpsc::UnboundedSender<Job>);

impl Lane {
    fn start(rt: &tokio::runtime::Handle) -> Lane {
        let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
        rt.spawn(async move {
            while let Some(job) = rx.recv().await {
                let mut task = tokio::spawn(job);
                tokio::select! {
                    _ = &mut task => {}
                    _ = tokio::time::sleep(HAND_OFF) => {}
                }
            }
        });
        Lane(tx)
    }

    fn submit(&self, job: Job) {
        self.0.send(job).ok();
    }
}

pub static START: OnceLock<Instant> = OnceLock::new();
static RUNNING: OnceLock<Arc<App>> = OnceLock::new();

pub fn save_on_quit() {
    let Some(app) = RUNNING.get() else { return };
    if let Some(player) = app.backend.get().and_then(|b| b.started_player()) {
        player.save_now();
    }
    journal::info("app", "saved on quit");
    journal::flush();
}

static TRACE: LazyLock<bool> = LazyLock::new(|| std::env::var_os(product::TRACE_ENV).is_some());

fn trace(what: &str) {
    if *TRACE {
        eprintln!("{} +{:>5}ms {what}", product::ID, START.get().map_or(0, |s| s.elapsed().as_millis()));
    }
}

pub fn journal_start() {
    journal::start(&product::data_dir());
    journal::install_panic_hook();
    journal::info("app", format!("{} {} ({}) pid {}", product::ID, product::VERSION, crate::stamp::commit(), std::process::id()));
}

fn note(event: &UiEvent, shown: &Mutex<(bool, bool)>) {
    match event {
        UiEvent::Current(Some(t)) => journal::info("track", format!("{} {} - {}", t.id, t.artist_line(), t.title)),
        UiEvent::Current(None) => journal::info("track", "none"),
        UiEvent::State(s) => {
            let mut seen = shown.locked();
            if *seen != (s.playing, s.loading) {
                *seen = (s.playing, s.loading);
                journal::info("player", format!("playing {} loading {}", s.playing, s.loading));
            }
        }
        UiEvent::Queue { tracks, cursor } => journal::info("queue", format!("{} tracks, cursor {cursor}", tracks.len())),
        UiEvent::Modes { shuffle, repeat } => journal::info("player", format!("shuffle {shuffle} repeat {repeat:?}")),
        UiEvent::Toast(t) => journal::warn(if t.cause.is_some_and(Code::network) { "net" } else { "toast" }, t.json()),
        UiEvent::Nudge(json) => journal::info(
            "nudge",
            serde_json::from_str::<Value>(json)
                .map(|n| format!("{} {}", n["kind"].as_str().unwrap_or("-"), n["track"].as_str().unwrap_or_default()))
                .unwrap_or_default(),
        ),
        _ => {}
    }
}

pub struct App {
    rt: Runtime,
    ui: Ui,
    events: broadcast::Sender<UiEvent>,
    listener: Mutex<Option<proxy::Bound>>,
    image_base: String,
    backend: OnceCell<Arc<Backend>>,
    data: PathBuf,
    cache: PathBuf,
    lane: Lane,
    tray: Arc<Tray>,
    gate: Arc<LibraryGate>,
}

impl App {
    pub fn start(ui: Ui) -> Arc<App> {
        let app = App::start_in(ui, product::data_dir(), product::cache_dir());
        RUNNING.set(app.clone()).ok();
        let updates = app.clone();
        app.rt.spawn(async move { updates.backend().await.start_updates() });
        let quit = app.ui.clone();
        app.rt.spawn(async move {
            let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) else { return };
            term.recv().await;
            save_on_quit();
            quit(UiEvent::TrayAction("quit".into()));
            tokio::time::sleep(QUIT_PATIENCE).await;
            // SAFETY: _exit ends the process at once and touches no shared state; Qt did not stop in time.
            unsafe { libc::_exit(143) };
        });
        app
    }

    pub fn start_in(ui: Ui, data: PathBuf, cache: PathBuf) -> Arc<App> {
        START.get_or_init(Instant::now);
        journal::start(&data);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .thread_name(product::ID)
            .build()
            .expect("tokio runtime");
        let listener = proxy::bind().expect("local proxy socket");
        let image_base = proxy::image_base(&listener);
        let (events, _) = broadcast::channel(256);
        let lane = Lane::start(rt.handle());
        let to_window = ui.clone();
        let tray = Tray::new(
            Arc::new(move |signal| {
                to_window(match signal {
                    tray::Signal::Ready(ready) => UiEvent::TrayReady(ready),
                    tray::Signal::Action(action) => UiEvent::TrayAction(action),
                })
            }),
            None,
        );
        let listener = Mutex::new(Some(listener));
        let app = Arc::new(App {
            rt,
            ui,
            events,
            listener,
            image_base,
            backend: OnceCell::new(),
            data,
            cache,
            lane,
            tray,
            gate: LibraryGate::new(),
        });
        let boot = app.clone();
        app.rt.spawn(async move {
            let backend = boot.backend().await;
            journal::info("app", format!("backend ready at +{} ms", START.get().map_or(0, |s| s.elapsed().as_millis())));
            tokio::spawn(server::serve(backend.clone(), boot.events.clone()));
            handoff::watch(backend.clone());
            let kept = backend.clone();
            tokio::spawn(async move { kept.keep_collections().await });
            let parts = backend.clone();
            tokio::spawn(async move { parts.watch_part().await });
            boot.watch_glass();
            match backend.player().await {
                Ok(player) => player.sense(crate::presence::watch(backend.tasks())),
                Err(e) => (boot.ui)(UiEvent::Toast(Toast::failed(Code::AudioFailed, &e))),
            }
            trace("player ready");
        });
        app
    }

    pub fn fetch_ytdlp(self: &Arc<Self>) {
        let app = self.clone();
        self.rt.spawn(async move {
            let toast = match app.backend().await.keep_ytdlp(true).await {
                Ok(Some(crate::ytdlp::Outcome::Current)) | Ok(None) => Toast::new(Code::YtdlpCurrent),
                Ok(Some(_)) => Toast::new(Code::YtdlpReady),
                Err(e) => Toast::failed(Code::YtdlpFetchFailed, &e),
            };
            (app.ui)(UiEvent::Toast(toast));
            (app.ui)(UiEvent::Tools);
        });
    }

    pub fn check_update(self: &Arc<Self>) {
        let app = self.clone();
        self.rt.spawn(async move { app.backend().await.check_update() });
    }

    pub fn restart_for_update(self: &Arc<Self>) {
        let app = self.clone();
        self.rt.spawn(async move {
            if app.backend().await.restart_for_update() {
                (app.ui)(UiEvent::TrayAction("quit".into()));
            }
        });
    }

    pub fn refresh_desktop(self: &Arc<Self>, finished: bool) {
        let app = self.clone();
        self.rt.spawn(async move {
            let mut status = app.backend().await.desktop_status().await;
            if finished {
                status["finished"] = json!(true);
            }
            (app.ui)(UiEvent::Desktop(status.to_string()));
        });
    }

    pub fn setup_glass(self: &Arc<Self>) {
        let app = self.clone();
        self.rt.spawn(async move {
            let backend = app.backend().await;
            match backend.setup_glass().await {
                Ok(crate::glass::Setup::Applied) => journal::info("glass", "Blur my Shell is set up"),
                Ok(crate::glass::Setup::Skipped) => (app.ui)(UiEvent::Toast(Toast::new(Code::ShellBlurSkipped))),
                Err(e) => (app.ui)(UiEvent::Toast(Toast::failed(Code::ShellBlur, &e))),
            }
            app.refresh_desktop(true);
        });
    }

    pub fn undo_glass(self: &Arc<Self>) {
        let app = self.clone();
        self.rt.spawn(async move {
            if let Err(e) = app.backend().await.undo_glass().await {
                (app.ui)(UiEvent::Toast(Toast::failed(Code::ShellBlurUndo, &e)));
            }
            app.refresh_desktop(true);
        });
    }

    fn watch_glass(self: &Arc<Self>) {
        if crate::desktop::kind() != crate::desktop::Desktop::Gnome {
            return;
        }
        let app = self.clone();
        self.rt.spawn(async move {
            let Ok(conn) = zbus::Connection::session().await else { return };
            let changed = app.clone();
            if let Err(e) = crate::glass::watch(conn, move || changed.refresh_desktop(false)).await {
                journal::warn("glass", format!("{e:#}"));
            }
        });
    }

    pub fn add_tray(&self) {
        let ui = self.ui.clone();
        self.rt.spawn(async move {
            let answer = match zbus::Connection::session().await {
                Ok(conn) => tray::add_tray_extension(&conn).await,
                Err(e) => Err(Code::ShellExtension.err(e.to_string())),
            };
            if let Err(e) = answer {
                ui(UiEvent::Toast(Toast::failed(Code::ShellExtension, &e)));
            }
        });
    }

    pub fn tray(&self, spec: &str) {
        let Ok(spec) = serde_json::from_str::<tray::Spec>(spec) else { return };
        let tray = self.tray.clone();
        self.rt.spawn(async move {
            if let Err(e) = tray.apply(spec).await {
                journal::warn("tray", format!("{e:#}"));
            }
        });
    }

    pub fn peek(&self, method: &str, args: &str) -> Option<String> {
        let args = serde_json::from_str(args).ok()?;
        Some(self.backend.get()?.peek(method, &args)?.to_string())
    }

    pub fn image_base(&self) -> &str {
        &self.image_base
    }

    async fn gate_answer(&self, method: &str) -> Option<anyhow::Result<Value>> {
        if self.backend.initialized() {
            return None;
        }
        self.gate.answer(method, &self.data).await
    }

    async fn backend(&self) -> &Arc<Backend> {
        self.backend
            .get_or_init(|| async {
                let listener = self.listener.locked().take().expect("proxy listener");
                let (ui, events) = (self.ui.clone(), self.events.clone());
                let shown = Mutex::new((false, false));
                let tee: Ui = Arc::new(move |event| {
                    note(&event, &shown);
                    if !matches!(event, UiEvent::Reply { .. }) {
                        events.send(event.clone()).ok();
                    }
                    ui(event);
                });
                let (data, cache, gate) = (self.data.clone(), self.cache.clone(), self.gate.clone());
                let backend =
                    tokio::task::spawn_blocking(move || Backend::build(tee, listener, data, cache, gate)).await.expect("backend init");
                trace("backend ready");
                backend
            })
            .await
    }

    pub fn request(self: &Arc<Self>, id: i32, method: String, args: String) {
        let app = self.clone();
        let ordered = crate::api::ordered(&method);
        let job = async move {
            let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
            let started = Instant::now();
            let outcome = if method == crate::api::PICK_PATH {
                crate::portal::answer(&args).await
            } else if let Some(answer) = app.gate_answer(&method).await {
                answer
            } else {
                app.backend().await.dispatch(&method, &args).await
            };
            journal::call("api", &method, crate::api::quiet(&method), &args, started.elapsed(), outcome.as_ref().err());
            let (ok, json) = match outcome {
                Ok(v) => (true, v.to_string()),
                Err(e) => (false, crate::api::failure_json(&e).to_string()),
            };
            trace(&format!("reply {method}"));
            (app.ui)(UiEvent::Reply { id, ok, json });
        };
        if ordered {
            self.lane.submit(Box::pin(job));
        } else {
            self.rt.spawn(job);
        }
    }
}
