mod api;
mod app;
mod audio;
mod bridge;
mod cli;
mod client;
mod covers;
mod deps;
mod desktop;
mod diag;
mod discord;
mod downloads;
#[cfg(test)]
mod fakes;
mod fx;
mod glass;
mod handoff;
mod import;
mod mcp;
mod mpris;
mod mpv;
mod player;
mod portal;
mod qtlog;
mod render;
mod report;
mod server;
#[cfg(test)]
mod testkit;
mod tray;
mod ytdlp;

use cxx_qt_lib::{QFont, QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};
use snellar_core::product;

const DEV_RUNS: [&str; 3] = ["--mock", "--scenario", "--shot"];
const RAISE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    if !args.iter().any(|a| DEV_RUNS.iter().any(|flag| a.starts_with(flag)))
        && product::socket_path().is_some_and(|path| client::raise_running(&path, RAISE_PATIENCE))
    {
        return;
    }
    app::START.get_or_init(std::time::Instant::now);
    app::journal_start();
    qtlog::install();
    unsafe { std::env::set_var("QML_XHR_ALLOW_FILE_READ", "1") };
    if std::env::var_os("QT_WAYLAND_DECORATION").is_none() {
        unsafe { std::env::set_var("QT_WAYLAND_DECORATION", "bradient") };
    }
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        QQuickStyle::set_style(&QString::from("Basic"));
    }
    let mut app = QGuiApplication::new();
    if let Some(app) = app.as_mut() {
        let mut font = QFont::default();
        font.from_string(&QString::from("Commissioner"));
        app.set_application_font(&font);
    }
    QGuiApplication::set_desktop_file_name(&QString::from(product::ID));
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(product::QML_MAIN));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
    app::save_on_quit();
}
