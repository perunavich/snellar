use serde_json::Value;

pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

pub fn artists(track: &Value) -> String {
    let names: Vec<&str> = track["artists"].as_array().into_iter().flatten().filter_map(|a| a["name"].as_str()).collect();
    names.join(", ")
}

pub fn track(track: &Value) -> String {
    let mut line = format!("{} - {}", artists(track), track["title"].as_str().unwrap_or("?"));
    if let Some(album) = track["album"]["name"].as_str() {
        line.push_str(&format!(" [{album}]"));
    }
    if let Some(duration) = track["duration"].as_f64().filter(|d| *d > 0.0) {
        line.push_str(&format!(" ({})", clock(duration)));
    }
    line
}

pub fn now(now: &Value) -> String {
    if now["current"].is_null() {
        return "Nothing is playing.".into();
    }
    let status = match (now["playing"].as_bool(), now["loading"].as_bool()) {
        (_, Some(true)) => "loading",
        (Some(true), _) => "playing",
        _ => "paused",
    };
    let id = now["current"]["id"].as_str().unwrap_or_default();
    format!(
        "{}\n{status} {} / {}, volume {}, shuffle {}, repeat {}, {}liked\nid {id}",
        track(&now["current"]),
        clock(now["position"].as_f64().unwrap_or(0.0)),
        clock(now["duration"].as_f64().unwrap_or(0.0)),
        now["volume"].as_f64().unwrap_or(0.0).round(),
        if now["shuffle"].as_bool() == Some(true) { "on" } else { "off" },
        now["repeat"].as_str().unwrap_or("off"),
        if now["liked"].as_bool() == Some(true) { "" } else { "not " },
    )
}

pub fn queue(queue: &Value, limit: usize) -> String {
    let tracks = queue["tracks"].as_array().map_or(&[][..], Vec::as_slice);
    if tracks.is_empty() {
        return "The queue is empty.".into();
    }
    let cursor = queue["cursor"].as_i64().unwrap_or(-1);
    let first = (cursor.max(0) as usize).saturating_sub(limit / 4);
    let mut lines: Vec<String> = tracks
        .iter()
        .enumerate()
        .skip(first)
        .take(limit)
        .map(|(i, t)| {
            format!("{}{:>3}. {} (id {})", if i as i64 == cursor { ">" } else { " " }, i, track(t), t["id"].as_str().unwrap_or_default())
        })
        .collect();
    if first + limit < tracks.len() {
        lines.push(format!("... and {} more", tracks.len() - first - limit));
    }
    format!("{} tracks, current index {cursor}\n{}", tracks.len(), lines.join("\n"))
}

pub fn results(tracks: &[Value]) -> String {
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{:>2}. {} (id {})", i + 1, track(t), t["id"].as_str().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn reports(list: &Value) -> String {
    let rows = list.as_array().map_or(&[][..], Vec::as_slice);
    if rows.is_empty() {
        return "No bug reports.".into();
    }
    rows.iter()
        .map(|r| {
            let said = r["note"].as_str().map_or("(no phrase)".to_string(), |n| format!("\"{n}\""));
            let on = r["track"].as_str().map_or(String::new(), |t| format!(" | {t}"));
            let screen = r["screen"].as_str().map_or(String::new(), |s| format!(" | {s}"));
            format!("{}  {said}{on}{screen}\n    {}", r["name"].as_str().unwrap_or("?"), r["path"].as_str().unwrap_or("?"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn import(result: &Value) -> String {
    let name = result["name"].as_str().unwrap_or_default();
    let mut text = match result["playlist"].as_i64() {
        Some(id) => format!("Imported {} of {} songs into \"{name}\" (playlist id {id}).", result["matched"], result["total"]),
        None => format!("None of the {} songs of \"{name}\" was found; no playlist was made.", result["total"]),
    };
    if result["limited"] == Value::Bool(true) {
        match result["sourceTotal"].as_u64() {
            Some(all) => {
                text.push_str(&format!(" The service showed {} of the {all} songs; a CSV export has all of them.", result["total"]))
            }
            None => text.push_str(&format!(
                " The page showed only the first {} songs and may hold more; a CSV export has all of them.",
                result["total"]
            )),
        }
    }
    let hidden: Vec<String> = result["hidden"].as_array().map_or(&[][..], Vec::as_slice).iter().map(Value::to_string).collect();
    if !hidden.is_empty() {
        text.push_str(&format!(
            " {} songs are not available in your country, and the service does not say what they are; their places in the playlist: {}.",
            hidden.len(),
            hidden.join(", ")
        ));
    }
    for (label, key) in [("not found", "unmatched"), ("already in the playlist", "merged")] {
        for row in result[key].as_array().map_or(&[][..], Vec::as_slice) {
            let names: Vec<&str> = row["artists"].as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(Value::as_str).collect();
            let length = row["duration"].as_f64().filter(|d| *d > 0.0).map_or(String::new(), |d| format!(" ({})", clock(d)));
            text.push_str(&format!("\n{label}: {} - {}{length}", names.join(", "), row["title"].as_str().unwrap_or("?")));
        }
    }
    text
}
