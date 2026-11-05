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
    for missing in result["unmatched"].as_array().map_or(&[][..], Vec::as_slice) {
        let names: Vec<&str> = missing["artists"].as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(Value::as_str).collect();
        let length = missing["duration"].as_f64().filter(|d| *d > 0.0).map_or(String::new(), |d| format!(" ({})", clock(d)));
        text.push_str(&format!("\nnot found: {} - {}{length}", names.join(", "), missing["title"].as_str().unwrap_or("?")));
    }
    text
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample() -> Value {
        json!({"id": "abc", "title": "Song", "artists": [{"name": "A"}, {"name": "B"}], "album": {"name": "LP"}, "duration": 215.0})
    }

    #[test]
    fn track_line() {
        assert_eq!(track(&sample()), "A, B - Song [LP] (3:35)");
    }

    #[test]
    fn now_states() {
        assert_eq!(now(&json!({"current": null})), "Nothing is playing.");
        let playing = now(
            &json!({"current": sample(), "playing": true, "position": 65.0, "duration": 215.0, "volume": 85.0, "shuffle": false, "repeat": "off", "liked": true}),
        );
        assert!(playing.contains("playing 1:05 / 3:35, volume 85, shuffle off, repeat off, liked"), "{playing}");
    }

    #[test]
    fn report_lines() {
        assert_eq!(reports(&json!([])), "No bug reports.");
        let text = reports(
            &json!([{ "name": "2026-11-02_12-00-00", "path": "/d/reports/2026-11-02_12-00-00", "note": "lyrics", "track": "A - Song", "screen": "favorites" }, { "name": "2026-11-02_11-00-00", "path": "/d/x", "note": null, "track": null, "screen": null }]),
        );
        assert!(text.starts_with("2026-11-02_12-00-00  \"lyrics\" | A - Song | favorites\n    /d/reports"), "{text}");
        assert!(text.contains("2026-11-02_11-00-00  (no phrase)\n"), "{text}");
    }

    #[test]
    fn import_summary() {
        let done = json!({ "playlist": 7, "name": "Road", "total": 3, "matched": 2, "limited": true, "unmatched": [{ "title": "Gone", "artists": ["A", "B"], "duration": 61.0 }] });
        assert_eq!(
            import(&done),
            "Imported 2 of 3 songs into \"Road\" (playlist id 7). The page showed only the first 3 songs and may hold more; a CSV export has all of them.\nnot found: A, B - Gone (1:01)"
        );
        let whole = json!({ "playlist": 7, "name": "Road", "total": 150, "matched": 150, "limited": false, "sourceTotal": 150 });
        assert_eq!(import(&whole), "Imported 150 of 150 songs into \"Road\" (playlist id 7).");
        let cut = json!({ "playlist": 7, "name": "Road", "total": 100, "matched": 90, "limited": true, "sourceTotal": 139 });
        assert_eq!(
            import(&cut),
            "Imported 90 of 100 songs into \"Road\" (playlist id 7). The service showed 100 of the 139 songs; a CSV export has all of them."
        );
        assert_eq!(
            import(&json!({ "playlist": null, "name": "X", "total": 1, "unmatched": [] })),
            "None of the 1 songs of \"X\" was found; no playlist was made."
        );
    }

    #[test]
    fn queue_marks_cursor_and_truncates() {
        let tracks: Vec<Value> = (0..30).map(|_| sample()).collect();
        let text = queue(&json!({"tracks": tracks, "cursor": 2}), 5);
        assert!(text.starts_with("30 tracks, current index 2"));
        assert!(text.contains(">  2. A, B - Song"));
        assert!(text.ends_with("... and 24 more"));
    }
}
