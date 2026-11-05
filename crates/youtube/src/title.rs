const TAG_WORDS: [&str; 32] = [
    "official",
    "video",
    "audio",
    "lyric",
    "lyrics",
    "visualizer",
    "visualiser",
    "hd",
    "hq",
    "4k",
    "1080p",
    "mv",
    "wave",
    "phonk",
    "drift",
    "trap",
    "free",
    "download",
    "dl",
    "music",
    "clip",
    "out",
    "now",
    "new",
    "full",
    "song",
    "premiere",
    "exclusive",
    "release",
    "hip",
    "hop",
    "rap",
];
const SEPARATORS: [&str; 3] = [" - ", " – ", " — "];
const CREDIT_MARKS: [&str; 5] = [", ", " x ", " × ", " feat. ", " ft. "];

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub artists: Vec<String>,
    pub title: String,
}

fn only_tags(inner: &str) -> bool {
    let mut words = inner.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).peekable();
    words.peek().is_some() && words.all(|w| TAG_WORDS.contains(&w.to_lowercase().as_str()))
}

fn strip_tag_groups(title: &str) -> String {
    let mut out = String::new();
    let mut rest = title;
    while let Some(start) = rest.find(['(', '[', '【']) {
        let open = rest[start..].chars().next().unwrap_or('(');
        let close = match open {
            '(' => ')',
            '[' => ']',
            _ => '】',
        };
        let Some(len) = rest[start..].find(close) else { break };
        let inner = &rest[start + open.len_utf8()..start + len];
        out.push_str(&rest[..start]);
        if !only_tags(inner) {
            out.push_str(&rest[start..start + len + close.len_utf8()]);
        }
        rest = &rest[start + len + close.len_utf8()..];
    }
    out.push_str(rest);
    out
}

fn strip_pipe_tags(title: &str) -> &str {
    match title.rsplit_once(" | ") {
        Some((head, tail)) if only_tags(tail) => head,
        _ => title,
    }
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’' | '«' | '»' | '「' | '」'))
}

fn tidy(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn uploader_name(uploader: &str) -> String {
    let name = uploader.trim().trim_end_matches(" - Topic").trim();
    let lower = name.to_lowercase();
    let bare = if lower.ends_with("vevo") && name.len() > 4 { &name[..name.len() - 4] } else { name };
    bare.trim().to_string()
}

fn same(a: &str, b: &str) -> bool {
    !a.is_empty() && a.to_lowercase() == b.to_lowercase()
}

fn credit(part: &str, channel: &str) -> Vec<String> {
    if same(part, channel) { vec![channel.to_string()] } else { artists_of(part) }
}

fn artists_of(credit: &str) -> Vec<String> {
    let mut names = vec![credit.to_string()];
    for mark in CREDIT_MARKS {
        names = names.iter().flat_map(|n| n.split(mark).map(str::to_string)).collect();
    }
    names.iter().map(|n| tidy(unquote(n))).filter(|n| !n.is_empty()).collect()
}

pub fn parse(raw: &str, uploader: &str) -> Parsed {
    let cleaned = tidy(&strip_tag_groups(raw));
    let cleaned = strip_pipe_tags(&cleaned).trim().trim_end_matches(['-', '|']).trim().to_string();
    let channel = uploader_name(uploader);
    let unified = SEPARATORS.iter().fold(cleaned.clone(), |t, s| t.replace(s, " - "));
    let parts: Vec<&str> = unified.split(" - ").map(str::trim).filter(|p| !p.is_empty()).collect();
    let plain = |title: &str| Parsed {
        artists: if channel.is_empty() { Vec::new() } else { vec![channel.clone()] },
        title: unquote(title).to_string(),
    };
    if parts.len() < 2 {
        let title = if cleaned.is_empty() { raw.trim() } else { &cleaned };
        return plain(title);
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    let sentence = first.split_whitespace().count() > 6;
    if parts.len() == 2 && same(last, &channel) && !same(first, &channel) {
        return Parsed { artists: credit(last, &channel), title: unquote(first).to_string() };
    }
    if sentence && !same(first, &channel) {
        return plain(&unified);
    }
    Parsed { artists: credit(first, &channel), title: unquote(&parts[1..].join(" - ")).to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(raw: &str, uploader: &str) -> (Vec<String>, String) {
        let parsed = parse(raw, uploader);
        (parsed.artists, parsed.title)
    }

    fn expect(raw: &str, uploader: &str, artists: &[&str], title: &str) {
        assert_eq!(p(raw, uploader), (artists.iter().map(|a| a.to_string()).collect::<Vec<_>>(), title.to_string()), "{raw}");
    }

    #[test]
    fn plain_artist_title() {
        expect("IVOXYGEN - casino143", "IVOXYGEN", &["IVOXYGEN"], "casino143");
        expect("Burgos - Lost Cause", "Some Channel", &["Burgos"], "Lost Cause");
    }

    #[test]
    fn genre_tags_in_brackets_are_dropped() {
        expect("KEDELA - SATURN [Wave/Phonk]", "Phonk Hell", &["KEDELA"], "SATURN");
        expect("KEDELA - SATURN (Phonk)", "x", &["KEDELA"], "SATURN");
        expect("MXRGX - GET AWAY [FREE DOWNLOAD]", "x", &["MXRGX"], "GET AWAY");
        expect("Artist - Title (Official Music Video) [4K]", "x", &["Artist"], "Title");
        expect("Artist - Title (Official Audio)", "Artist Official", &["Artist"], "Title");
        expect("(Wave) Artist - Title", "x", &["Artist"], "Title");
    }

    #[test]
    fn japanese_brackets() {
        expect("【MV】YOASOBI - Idol", "x", &["YOASOBI"], "Idol");
    }

    #[test]
    fn meaningful_brackets_stay() {
        expect("Artist - Title (Slowed + Reverb)", "x", &["Artist"], "Title (Slowed + Reverb)");
        expect("Artist - Title (feat. Guest)", "x", &["Artist"], "Title (feat. Guest)");
        expect("Artist - Title (Live at Wembley)", "x", &["Artist"], "Title (Live at Wembley)");
    }

    #[test]
    fn dash_variants() {
        expect("Artist – Title", "x", &["Artist"], "Title");
        expect("Artist — Title", "x", &["Artist"], "Title");
        expect("Jay-Z - 99 Problems", "x", &["Jay-Z"], "99 Problems");
    }

    #[test]
    fn swapped_order_follows_the_uploader() {
        expect("Title - Artist", "Artist", &["Artist"], "Title");
        expect("title - ivoxygen", "IVOXYGEN", &["IVOXYGEN"], "title");
        expect("Artist - Title", "Artist", &["Artist"], "Title");
    }

    #[test]
    fn several_artists() {
        expect("Artist x Other - Title [FREE DOWNLOAD]", "x", &["Artist", "Other"], "Title");
        expect("Artist, Other - Title (Lyrics)", "x", &["Artist", "Other"], "Title");
        expect("Simon & Garfunkel - Sound of Silence", "x", &["Simon & Garfunkel"], "Sound of Silence");
    }

    #[test]
    fn no_separator_uses_the_channel() {
        expect("Just A Title [Phonk]", "Chan - Topic", &["Chan"], "Just A Title");
        expect("Song Name (Official Video)", "ArtistVEVO", &["Artist"], "Song Name");
    }

    #[test]
    fn extra_parts_stay_in_the_title() {
        expect("Artist - Title - Remix by X", "x", &["Artist"], "Title - Remix by X");
    }

    #[test]
    fn sentences_are_not_split() {
        expect(
            "How to make a beat in FL Studio for beginners - Part 2",
            "Producer Tips",
            &["Producer Tips"],
            "How to make a beat in FL Studio for beginners - Part 2",
        );
    }

    #[test]
    fn quotes_and_pipes() {
        expect("Artist - \"Title\"", "x", &["Artist"], "Title");
        expect("Artist - Title | Phonk", "x", &["Artist"], "Title");
        expect("Artist - Title | Official Video", "x", &["Artist"], "Title");
    }

    #[test]
    fn degenerate_titles_survive() {
        expect(" - ", "Chan", &["Chan"], "-");
        expect("[Phonk]", "Chan", &["Chan"], "[Phonk]");
        expect("", "Chan", &["Chan"], "");
    }

    #[test]
    fn unknown_uploader_gives_no_fallback_artist() {
        expect("Just A Title", "", &[], "Just A Title");
    }
}
