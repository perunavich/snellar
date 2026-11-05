use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tokio::process::Command;

use snellar_core::clock;
use snellar_core::error::Code;
use snellar_core::journal;

const SPAN: Duration = Duration::from_secs(5 * 60);
const REVEAL_WAIT: Duration = Duration::from_secs(3);
const FILES: [&str; 7] = ["report.md", "window.png", "journal.txt", "state.json", "ui.json", "note.txt", "components.txt"];

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("SNELLAR_COMMIT");

fn root(data: &Path) -> PathBuf {
    data.join("reports")
}

fn put(dir: &Path, file: &str, text: &str) -> Result<()> {
    std::fs::write(dir.join(file), journal::scrub(text)).with_context(|| format!("cannot write {file} in {}", dir.display()))
}

fn read(dir: &Path, file: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(file)).ok()
}

fn json_in(dir: &Path, file: &str) -> Value {
    read(dir, file).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
}

fn slim(track: &Value) -> Value {
    let artists: Vec<&str> = track["artists"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).collect();
    json!({ "id": track["id"], "title": track["title"], "artists": artists.join(", "), "duration": track["duration"] })
}

fn slim_player(player: &Value) -> Value {
    if player.is_null() {
        return Value::Null;
    }
    let mut now = player["now"].clone();
    if now["current"].is_object() {
        now["current"] = slim(&now["current"]);
    }
    let tracks: Vec<Value> = player["queue"]["tracks"].as_array().into_iter().flatten().map(slim).collect();
    json!({ "now": now, "queue": { "cursor": player["queue"]["cursor"], "tracks": tracks } })
}

fn valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '_')
}

fn folder(data: &Path, name: &str) -> Result<PathBuf> {
    if !valid(name) {
        return Err(Code::ApiArgs.err(format!("`{name}` is not a report name, see `reports`")));
    }
    let dir = root(data).join(name);
    if !dir.is_dir() {
        return Err(Code::ApiNotFound.err(format!("no report named `{name}`")));
    }
    Ok(dir)
}

pub fn create(data: &Path, note: Option<&str>, player: Value) -> Result<(String, PathBuf)> {
    let base = clock::stamp(SystemTime::now())[..19].replace(' ', "_").replace(':', "-");
    std::fs::create_dir_all(root(data))?;
    let mut name = base.clone();
    let mut n = 1;
    loop {
        match std::fs::create_dir(root(data).join(&name)) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                n += 1;
                name = format!("{base}-{n}");
            }
            Err(e) => return Err(e).context("cannot create the report folder"),
        }
    }
    let dir = root(data).join(&name);
    let time = clock::stamp(SystemTime::now());
    put(&dir, "journal.txt", &journal::tail(SPAN))?;
    put(
        &dir,
        "state.json",
        &serde_json::to_string_pretty(&json!({ "version": VERSION, "commit": COMMIT, "time": time, "player": slim_player(&player) }))?,
    )?;
    if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
        put(&dir, "note.txt", note)?;
    }
    summarize(&dir, &name)?;
    journal::info("report", format!("saved {name}"));
    Ok((name, dir))
}

pub fn attach(data: &Path, name: &str, ui: Option<&Value>, note: Option<&str>) -> Result<()> {
    let dir = folder(data, name)?;
    if let Some(ui) = ui.filter(|u| !u.is_null()) {
        put(&dir, "ui.json", &serde_json::to_string_pretty(ui)?)?;
    }
    if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
        put(&dir, "note.txt", note)?;
    }
    summarize(&dir, name)
}

pub fn components(data: &Path, name: &str, details: &str) -> Result<()> {
    let dir = folder(data, name)?;
    put(&dir, "components.txt", details)?;
    summarize(&dir, name)
}

fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

fn summarize(dir: &Path, name: &str) -> Result<()> {
    let (state, ui) = (json_in(dir, "state.json"), json_in(dir, "ui.json"));
    let backend = &state["player"];
    let shown = &ui["player"];
    let (current, position, duration, playing, queue, cursor) = if backend["now"]["current"].is_object() {
        (
            &backend["now"]["current"],
            &backend["now"]["position"],
            &backend["now"]["duration"],
            &backend["now"]["playing"],
            &backend["queue"]["tracks"],
            &backend["queue"]["cursor"],
        )
    } else {
        (&shown["current"], &shown["position"], &shown["duration"], &shown["playing"], &shown["queue"], &shown["cursor"])
    };
    let mut out = format!("# Bug report {name}\n\n");
    if let Some(note) = read(dir, "note.txt") {
        out.push_str(&format!("Note: {}\n\n", note.trim()));
    }
    out.push_str(&format!(
        "Version: {} ({})\nTime: {}\n",
        state["version"].as_str().unwrap_or(VERSION),
        state["commit"].as_str().unwrap_or(COMMIT),
        state["time"].as_str().unwrap_or("?")
    ));
    if ui.is_object() {
        let w = &ui["window"];
        out.push_str(&format!(
            "Screen: {} {}\nWindow: {}x{}, compact layout: {}\n",
            ui["screen"].as_str().unwrap_or("?"),
            ui["params"],
            w["width"],
            w["height"],
            w["compact"]
        ));
        let overlays = &ui["overlays"];
        out.push_str(&format!("Overlays: now playing {}, audio {}, tab {}\n", overlays["nowPlaying"], overlays["audio"], overlays["tab"]));
    } else {
        out.push_str("Screen: not captured (no window answered)\n");
    }
    if current.is_object() {
        out.push_str(&format!(
            "Track: {} - {} ({})\nPosition: {} of {}, {}\n",
            current["artists"].as_str().unwrap_or("?"),
            current["title"].as_str().unwrap_or("?"),
            current["id"].as_str().unwrap_or("?"),
            clock(position.as_f64().unwrap_or(0.0)),
            clock(duration.as_f64().unwrap_or(0.0)),
            if playing.as_bool().unwrap_or(false) { "playing" } else { "paused" }
        ));
    } else {
        out.push_str("Track: none\n");
    }
    out.push_str(&format!("Queue: {} tracks, cursor {}\n", queue.as_array().map_or(0, Vec::len), cursor));
    for (label, value) in [("Selected text", &ui["selection"]), ("Last toast", &ui["toast"])] {
        if let Some(text) = value.as_str().filter(|t| !t.is_empty()) {
            out.push_str(&format!("{label}: {text}\n"));
        }
    }
    if let Some(components) = read(dir, "components.txt") {
        out.push_str(&format!("\nComponents:\n{}", components.trim_end()));
        out.push('\n');
    }
    let present: Vec<&str> = FILES.iter().copied().filter(|f| dir.join(f).exists()).collect();
    out.push_str(&format!("\nFiles: {}\n", present.join(", ")));
    put(dir, "report.md", &out)
}

pub fn list(data: &Path) -> Value {
    let mut names: Vec<String> = std::fs::read_dir(root(data))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid(n))
        .collect();
    names.sort_by(|a, b| b.cmp(a));
    Value::Array(
        names
            .into_iter()
            .map(|name| {
                let dir = root(data).join(&name);
                let (state, ui) = (json_in(&dir, "state.json"), json_in(&dir, "ui.json"));
                let current = if state["player"]["now"]["current"].is_object() {
                    &state["player"]["now"]["current"]
                } else {
                    &ui["player"]["current"]
                };
                let track = current["title"].as_str().map(|t| format!("{} - {t}", current["artists"].as_str().unwrap_or("?")));
                let files: Vec<&str> = FILES.iter().copied().filter(|f| dir.join(f).exists()).collect();
                json!({
                    "name": name,
                    "path": dir,
                    "note": read(&dir, "note.txt").map(|n| n.trim().to_string()),
                    "track": track,
                    "screen": ui["screen"],
                    "files": files,
                })
            })
            .collect(),
    )
}

pub fn pack(data: &Path, name: &str, note: Option<&str>) -> Result<PathBuf> {
    attach(data, name, None, note)?;
    let dir = folder(data, name)?;
    let archive = root(data).join(format!("{name}.zip"));
    let part = root(data).join(format!("{name}.zip.tmp"));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&part).context("cannot create the archive")?);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for file in FILES {
        let Ok(bytes) = std::fs::read(dir.join(file)) else { continue };
        zip.start_file(format!("{name}/{file}"), options)?;
        zip.write_all(&bytes)?;
    }
    zip.finish()?;
    std::fs::rename(&part, &archive).context("cannot finish the archive")?;
    journal::info("report", format!("packed {name}"));
    Ok(archive)
}

fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            url.push(byte as char);
        } else {
            url.push_str(&format!("%{byte:02X}"));
        }
    }
    url
}

fn opener(dir: &Path) -> Command {
    let mut cmd = Command::new("xdg-open");
    cmd.arg(dir).env_remove("LD_LIBRARY_PATH").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd
}

async fn file_manager(path: &Path) -> zbus::Result<()> {
    const FILES: &str = "org.freedesktop.FileManager1";
    let bus = zbus::Connection::session().await?;
    bus.call_method(Some(FILES), "/org/freedesktop/FileManager1", Some(FILES), "ShowItems", &(vec![file_url(path)], "")).await?;
    Ok(())
}

pub async fn reveal(path: &Path) -> bool {
    if matches!(tokio::time::timeout(REVEAL_WAIT, file_manager(path)).await, Ok(Ok(()))) {
        return true;
    }
    path.parent().is_some_and(|dir| opener(dir).spawn().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-report-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn player() -> Value {
        json!({
            "now": { "current": { "id": "dQw4w9WgXcQ", "title": "Song", "artists": [{ "id": "a", "name": "Band" }], "duration": 200.0, "thumb": "https://x.test/c?token=1" }, "position": 83.0, "duration": 200.0, "playing": true },
            "queue": { "cursor": 0, "tracks": [{ "id": "dQw4w9WgXcQ", "title": "Song", "artists": [{ "name": "Band" }], "duration": 200.0 }] }
        })
    }

    #[test]
    fn a_report_has_the_state_the_summary_and_no_home_paths() {
        let data = data("full");
        let home = dirs::home_dir().unwrap();
        let (name, dir) = create(&data, Some("  lyrics broke "), player()).unwrap();
        let ui = json!({ "screen": "favorites", "params": {}, "window": { "width": 1440, "height": 900, "compact": false }, "overlays": { "nowPlaying": false, "audio": false, "tab": "next" }, "selection": format!("failed to read {}/x", home.display()), "toast": "boom" });
        attach(&data, &name, Some(&ui), None).unwrap();
        let summary = std::fs::read_to_string(dir.join("report.md")).unwrap();
        for want in [
            "Note: lyrics broke",
            "Screen: favorites",
            "Window: 1440x900",
            "Band - Song",
            "1:23 of 3:20, playing",
            "Queue: 1 tracks, cursor 0",
            "Selected text: failed to read ~/x",
            "Last toast: boom",
            "0.1.0",
        ] {
            assert!(summary.contains(want), "{want}\n{summary}");
        }
        for file in ["report.md", "journal.txt", "state.json", "ui.json", "note.txt"] {
            let text = std::fs::read_to_string(dir.join(file)).unwrap();
            assert!(!text.contains(home.to_str().unwrap()), "{file} holds the home path");
        }
        assert!(!std::fs::read_to_string(dir.join("state.json")).unwrap().contains("token"), "covers are not kept");
    }

    #[test]
    fn a_report_holds_the_list_from_the_dependencies_section() {
        let data = data("deps");
        let home = dirs::home_dir().unwrap();
        let (name, dir) = create(&data, None, player()).unwrap();
        components(&data, &name, &format!("mpv: 0.40, bundled, ok\nyt-dlp: -, -, missing\nroot: {}/x\n", home.display())).unwrap();
        let summary = std::fs::read_to_string(dir.join("report.md")).unwrap();
        assert!(summary.contains("Components:\nmpv: 0.40, bundled, ok\nyt-dlp: -, -, missing"), "{summary}");
        assert!(summary.contains("components.txt"), "{summary}");
        assert!(!std::fs::read_to_string(dir.join("components.txt")).unwrap().contains(home.to_str().unwrap()), "no home path");
        let zip = pack(&data, &name, None).unwrap();
        assert!(entries(&zip).contains(&format!("{name}/components.txt")));
    }

    #[test]
    fn the_phrase_can_come_later_and_the_list_shows_it() {
        let data = data("list");
        let (first, _) = create(&data, None, Value::Null).unwrap();
        assert!(list(&data)[0]["note"].is_null());
        attach(&data, &first, None, Some("late phrase")).unwrap();
        let (second, _) = create(&data, None, player()).unwrap();
        let all = list(&data);
        assert_eq!(all.as_array().unwrap().len(), 2);
        assert_eq!(all[0]["name"], json!(second), "newest first");
        assert_eq!(all[1]["note"], json!("late phrase"));
        assert_eq!(all[0]["track"], json!("Band - Song"));
    }

    #[test]
    fn two_reports_in_one_second_get_two_folders() {
        let data = data("same");
        let a = create(&data, None, Value::Null).unwrap().0;
        let b = create(&data, None, Value::Null).unwrap().0;
        assert_ne!(a, b);
    }

    #[test]
    fn names_that_leave_the_reports_folder_are_refused() {
        let data = data("names");
        for bad in ["..", "../x", "a/b", "", "2026-11-02_12-00-00/../.."] {
            assert!(attach(&data, bad, None, Some("x")).is_err(), "{bad}");
        }
        assert_eq!(Code::of(&attach(&data, "2026-01-01_00-00-00", None, Some("x")).unwrap_err()), Code::ApiNotFound);
    }

    fn entries(path: &Path) -> Vec<String> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        (0..zip.len()).map(|i| zip.by_index(i).unwrap().name().unwrap().to_string()).collect()
    }

    fn entry(path: &Path, name: &str) -> String {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        std::io::read_to_string(zip.by_name(name).unwrap()).unwrap()
    }

    #[test]
    fn a_packed_report_holds_its_files_under_its_name_and_only_those() {
        let data = data("pack");
        let (name, dir) = create(&data, None, player()).unwrap();
        std::fs::write(dir.join("stray.bin"), "not part of a report").unwrap();
        let zip = pack(&data, &name, Some("  it broke ")).unwrap();
        assert_eq!(zip, root(&data).join(format!("{name}.zip")));
        let names = entries(&zip);
        for want in ["report.md", "journal.txt", "state.json", "note.txt"] {
            assert!(names.contains(&format!("{name}/{want}")), "{want} in {names:?}");
        }
        assert!(!names.iter().any(|n| n.contains("stray")), "{names:?}");
        assert_eq!(entry(&zip, &format!("{name}/note.txt")), "it broke");
        assert!(
            entry(&zip, &format!("{name}/report.md")).contains("Note: it broke"),
            "the phrase typed before the click is in the summary"
        );
    }

    #[test]
    fn packing_again_replaces_the_archive_and_leaves_no_temporary_file() {
        let data = data("repack");
        let (name, _) = create(&data, None, Value::Null).unwrap();
        let first = pack(&data, &name, None).unwrap();
        assert!(!entries(&first).iter().any(|n| n.ends_with("note.txt")));
        let second = pack(&data, &name, Some("later")).unwrap();
        assert_eq!(first, second);
        assert!(entries(&second).iter().any(|n| n.ends_with("note.txt")));
        let stray: Vec<_> = std::fs::read_dir(root(&data))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .filter(|n| n.to_string_lossy().contains("tmp"))
            .collect();
        assert!(stray.is_empty(), "{stray:?}");
        assert_eq!(list(&data).as_array().unwrap().len(), 1, "the archive is not a report");
    }

    #[test]
    fn packing_refuses_names_that_leave_the_reports_folder() {
        let data = data("packnames");
        for bad in ["..", "../x", "a/b", ""] {
            assert!(pack(&data, bad, None).is_err(), "{bad}");
        }
        assert_eq!(Code::of(&pack(&data, "2026-01-01_00-00-00", None).unwrap_err()), Code::ApiNotFound);
    }

    #[test]
    fn a_path_becomes_a_file_url() {
        assert_eq!(file_url(Path::new("/home/a b/отчёт#1.zip")), "file:///home/a%20b/%D0%BE%D1%82%D1%87%D1%91%D1%82%231.zip");
    }

    #[test]
    fn the_folder_opener_does_not_hand_over_the_bundled_libraries() {
        let opener = opener(Path::new("/x"));
        assert!(opener.as_std().get_envs().any(|(k, v)| k == "LD_LIBRARY_PATH" && v.is_none()));
        assert_eq!(opener.as_std().get_program(), "xdg-open");
    }

    struct FakeFiles(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    #[zbus::interface(name = "org.freedesktop.FileManager1")]
    impl FakeFiles {
        #[zbus(name = "ShowItems")]
        fn show_items(&self, uris: Vec<String>, _startup: &str) {
            self.0.lock().unwrap().extend(uris);
        }
    }

    #[tokio::test]
    async fn the_file_manager_is_asked_to_select_the_archive() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let service = zbus::connection::Builder::session()
            .name("org.freedesktop.FileManager1")
            .serve_at("/org/freedesktop/FileManager1", FakeFiles(seen.clone()))
            .build()
            .await;
        let Ok(_service) = service else { return };
        reveal(Path::new("/tmp/a b.zip")).await;
        assert_eq!(*seen.lock().unwrap(), vec!["file:///tmp/a%20b.zip".to_string()]);
    }
}
