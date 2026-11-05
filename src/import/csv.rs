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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/import/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn track(title: &str, artists: &[&str], album: &str, duration: f64) -> ImportTrack {
        ImportTrack { title: title.into(), artists: artists.iter().map(|a| a.to_string()).collect(), album: album.into(), duration }
    }

    #[test]
    fn exportify_with_uris_reads_quoted_fields_escaped_commas_and_milliseconds() {
        let text = fixture("exportify-2024.csv");
        assert!(text.starts_with('\u{feff}') && text.contains("\r\n"), "the fixture keeps the BOM and CRLF");
        let list = read(&text).unwrap();
        assert_eq!(list.tracks.len(), 6);
        assert_eq!(
            list.tracks[0],
            track(
                "Get Lucky (feat. Pharrell Williams & Nile Rodgers)",
                &["Daft Punk", "Pharrell Williams", "Nile Rodgers"],
                "Random Access Memories",
                369.626
            )
        );
        assert_eq!(list.tracks[1].artists, ["Tyler, The Creator", "Kali Uchis"]);
        assert_eq!(list.tracks[2].title, "Hello, \"World\"");
        assert_eq!(list.tracks[3], track("Группа крови", &["Кино"], "Группа крови", 285.0));
        assert_eq!(list.tracks[4].artists, ["Beyoncé"]);
        assert_eq!(list.tracks[5].title, "Bohemian Rhapsody - Remastered 2011");
        assert_eq!(list.name, "", "Exportify has no playlist column; the file name is used");
    }

    #[test]
    fn exportify_with_audio_features_finds_its_columns_by_name() {
        let list = read(&fixture("exportify-2025.csv")).unwrap();
        assert_eq!(list.tracks.len(), 3);
        assert_eq!(list.tracks[0], track("Starboy", &["The Weeknd", "Daft Punk"], "Starboy", 230.453));
        assert_eq!(list.tracks[2].album, "Currents");
    }

    #[test]
    fn tunemymusic_has_a_bom_a_playlist_name_and_no_duration() {
        let list = read(&fixture("tunemymusic.csv")).unwrap();
        assert_eq!(list.name, "Road trip");
        assert_eq!(list.tracks.len(), 4);
        assert_eq!(list.tracks[0], track("Blinding Lights", &["The Weeknd"], "After Hours", 0.0));
        assert_eq!(list.tracks[1].artists, ["Daft Punk", "Pharrell Williams"]);
        assert_eq!(list.tracks[3].title, "Спокойная ночь");
    }

    #[test]
    fn a_spreadsheet_with_semicolons_and_clock_durations() {
        let list = read(&fixture("semicolon.csv")).unwrap();
        assert_eq!(list.tracks.len(), 2);
        assert_eq!(list.tracks[0], track("Let Down", &["Radiohead"], "OK Computer", 299.0));
        assert_eq!(list.tracks[1].duration, 3723.0);
    }

    #[test]
    fn a_field_may_hold_a_line_break_and_blank_rows_are_skipped() {
        let list = read("Title,Artist\n\"Two\nlines\",A\n\n,\nNext,B").unwrap();
        assert_eq!(list.tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Two\nlines", "Next"]);
    }

    #[test]
    fn files_without_a_title_column_or_without_rows_are_refused_with_a_code() {
        assert_eq!(Code::of(&read("Artist,Album\nA,B").unwrap_err()), Code::ImportColumns);
        assert_eq!(Code::of(&read("").unwrap_err()), Code::ImportColumns);
        assert_eq!(Code::of(&read("\u{feff}Track name,Artist name\r\n").unwrap_err()), Code::ImportEmpty);
    }

    #[test]
    fn durations_in_every_form() {
        assert_eq!(seconds("225000", true), 225.0);
        assert_eq!(seconds("225", false), 225.0);
        assert_eq!(seconds("225000", false), 225.0);
        assert_eq!(seconds("3:45", false), 225.0);
        assert_eq!(seconds("x", false), 0.0);
        assert_eq!(seconds("-5", false), 0.0);
        assert_eq!(seconds("", true), 0.0);
    }
}
