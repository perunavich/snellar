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
use snellar_core::product;

use crate::render;

const SPAN: Duration = Duration::from_secs(5 * 60);
const REVEAL_WAIT: Duration = Duration::from_secs(3);
const FILES: [&str; 7] = ["report.md", "window.png", "journal.txt", "state.json", "ui.json", "note.txt", "components.txt"];

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
        &serde_json::to_string_pretty(
            &json!({ "version": product::VERSION, "commit": crate::stamp::commit(), "time": time, "player": slim_player(&player) }),
        )?,
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
        state["version"].as_str().unwrap_or(product::VERSION),
        state["commit"].as_str().unwrap_or(crate::stamp::commit()),
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
            render::clock(position.as_f64().unwrap_or(0.0)),
            render::clock(duration.as_f64().unwrap_or(0.0)),
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
