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
const SPEED_TAIL: [&str; 19] = [
    "slowed",
    "bass",
    "boosted",
    "reverb",
    "sped",
    "speed",
    "up",
    "nightcore",
    "daycore",
    "8d",
    "audio",
    "super",
    "ultra",
    "and",
    "x",
    "to",
    "perfection",
    "version",
    "edit",
];
const SPEED_MARK: [&str; 6] = ["slowed", "sped", "nightcore", "daycore", "8d", "boosted"];
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

fn speed_tail(part: &str) -> bool {
    let words: Vec<String> = part.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect();
    words.iter().any(|w| SPEED_MARK.contains(&w.as_str())) && words.iter().all(|w| SPEED_TAIL.contains(&w.as_str()))
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
    if parts.len() == 2 && speed_tail(last) {
        return plain(&unified);
    }
    let sentence = first.split_whitespace().count() > 6;
    if parts.len() == 2 && same(last, &channel) && !same(first, &channel) {
        return Parsed { artists: credit(last, &channel), title: unquote(first).to_string() };
    }
    if sentence && !same(first, &channel) {
        return plain(&unified);
    }
    Parsed { artists: credit(first, &channel), title: unquote(&parts[1..].join(" - ")).to_string() }
}
