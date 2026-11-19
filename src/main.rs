mod bridge;
mod qtlog;

use cxx_qt_lib::{QFont, QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};
use snellar_core::product;

unsafe extern "C" {
    fn snellar_svg_provider(engine: *mut std::ffi::c_void);
    fn snellar_qt_version() -> *const std::os::raw::c_char;
}

const DEV_RUNS: [&str; 3] = ["--mock", "--scenario", "--shot"];
const RAISE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

fn main() {
    snellar_app::set_stamp(snellar_app::Stamp { commit: env!("SNELLAR_COMMIT"), qt: qt_version() });
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = snellar_app::run_cli(&args) {
        std::process::exit(code);
    }
    if !args.iter().any(|a| DEV_RUNS.iter().any(|flag| a.starts_with(flag)))
        && product::socket_path().is_some_and(|path| snellar_app::raise_running(&path, RAISE_PATIENCE))
    {
        return;
    }
    snellar_app::START.get_or_init(std::time::Instant::now);
    snellar_app::journal_start();
    qtlog::install();
    unsafe { std::env::set_var("QML_XHR_ALLOW_FILE_READ", "1") };
    if std::env::var_os("QT_WAYLAND_DECORATION").is_none() {
        unsafe { std::env::set_var("QT_WAYLAND_DECORATION", snellar_app::OWN_DECORATION) };
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
    if let Some(mut engine) = engine.as_mut() {
        // SAFETY: the engine is alive and not moved; the provider only adds itself to it before the first load.
        unsafe { snellar_svg_provider(std::ptr::from_mut(engine.as_mut().get_unchecked_mut()).cast()) };
        engine.load(&QUrl::from(product::QML_MAIN));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
    snellar_app::save_on_quit();
    snellar_app::relaunch_if_asked();
}

fn qt_version() -> String {
    // SAFETY: qVersion returns a pointer to a static NUL-terminated string.
    unsafe { std::ffi::CStr::from_ptr(snellar_qt_version()) }.to_string_lossy().into_owned()
}
