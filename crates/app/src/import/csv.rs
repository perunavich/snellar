use anyhow::Result;
use snellar_core::error::Code;
use snellar_core::model::ImportTrack;

use super::Listing;

const TITLE: [&str; 6] = ["track name", "title", "song", "song name", "track", "name"];
const ARTISTS: [&str; 6] = ["artist name(s)", "artist name", "artist names", "artist(s)", "artists", "artist"];
const ALBUM: [&str; 3] = ["album name", "album", "album title"];
const MILLIS: [&str; 4] = ["track duration (ms)", "duration (ms)", "duration_ms", "duration ms"];
const LENGTH: [&str; 3] = ["duration", "length", "time"];
const PLAYLIST: [&str; 2] = ["playlist name", "playlist"];

pub fn read(text: &str) -> Result<Listing> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = records(text, separator(text)).into_iter();
    let header: Vec<String> =
        rows.next().unwrap_or_default().iter().map(|h| h.trim().trim_start_matches('\u{feff}').to_lowercase()).collect();
    let column = |names: &[&str]| names.iter().find_map(|n| header.iter().position(|h| h == n));
    let title = column(&TITLE).ok_or_else(|| Code::ImportColumns.err(format!("no track name column in {header:?}")))?;
    let (artists, album, playlist) = (column(&ARTISTS), column(&ALBUM), column(&PLAYLIST));
    let duration = column(&MILLIS).map(|i| (i, true)).or_else(|| column(&LENGTH).map(|i| (i, false)));
    let mut listing = Listing::default();
    for row in rows {
        let cell = |i: Option<usize>| i.and_then(|i| row.get(i)).map_or("", |c| c.trim());
        let name = cell(Some(title));
        if name.is_empty() {
            continue;
        }
        if listing.name.is_empty() {
            listing.name = cell(playlist).into();
        }
        listing.tracks.push(ImportTrack {
            title: name.into(),
            artists: split_artists(cell(artists)),
            album: cell(album).into(),
            duration: duration.map_or(0.0, |(i, ms)| seconds(cell(Some(i)), ms)),
        });
    }
    if listing.tracks.is_empty() {
        return Err(Code::ImportEmpty.err("the file has a header and no tracks"));
    }
    Ok(listing)
}

fn separator(text: &str) -> char {
    let mut quoted = false;
    let mut counts = [(',', 0), (';', 0), ('\t', 0)];
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' if !quoted => break,
            _ if !quoted => {
                if let Some(n) = counts.iter_mut().find(|(s, _)| *s == c) {
                    n.1 += 1;
                }
            }
            _ => {}
        }
    }
    counts.iter().max_by_key(|(s, n)| (*n, *s == ',')).map_or(',', |(s, _)| *s)
}

fn records(text: &str, sep: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c != '"' {
                field.push(c);
            } else if chars.peek() == Some(&'"') {
                field.push('"');
                chars.next();
            } else {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ if c == sep => row.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

fn split_artists(cell: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut name = String::new();
    let mut chars = cell.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&',') => {
                name.push(',');
                chars.next();
            }
            ',' => names.push(std::mem::take(&mut name)),
            _ => name.push(c),
        }
    }
    names.push(name);
    names.into_iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect()
}

fn seconds(cell: &str, millis: bool) -> f64 {
    if cell.contains(':') {
        return cell.split(':').try_fold(0.0, |total, part| part.trim().parse::<f64>().map(|n| total * 60.0 + n)).unwrap_or(0.0);
    }
    match cell.parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 => {
            if millis || n >= 10_000.0 {
                n / 1000.0
            } else {
                n
            }
        }
        _ => 0.0,
    }
}
