use snellar_core::model::{ImportTrack, Track};

const ENOUGH: f64 = 0.75;
const TITLE_FLOOR: f64 = 0.7;
const ARTIST_FLOOR: f64 = 0.5;
const SAME_LENGTH: f64 = 3.0;
const FAR_LENGTH: f64 = 20.0;
const OTHER_RECORDING: f64 = 45.0;
const OTHER_VERSION: f64 = 0.85;
const VERSIONS: [&str; 14] = [
    "live",
    "remix",
    "acoustic",
    "instrumental",
    "karaoke",
    "cover",
    "sped",
    "slowed",
    "nightcore",
    "8d",
    "demo",
    "unplugged",
    "reverb",
    "acapella",
];
const TAIL_WORDS: [&str; 16] = [
    "remaster",
    "version",
    "edit",
    "mix",
    "live",
    "mono",
    "stereo",
    "bonus",
    "deluxe",
    "from ",
    "feat",
    "acoustic",
    "demo",
    "anniversary",
    "instrumental",
    "recorded",
];
const COMMON_WORDS: [&str; 16] = ["the", "a", "an", "and", "of", "los", "las", "la", "le", "les", "el", "die", "der", "das", "de", "y"];
const FEAT: [&str; 5] = [" feat. ", " feat ", " ft. ", " ft ", " featuring "];

pub fn best<'a>(want: &ImportTrack, found: &'a [Track]) -> Option<&'a Track> {
    let mut best: Option<(f64, &Track)> = None;
    for track in found {
        let s = score(want, track);
        if s >= ENOUGH && best.is_none_or(|(top, _)| s > top) {
            best = Some((s, track));
        }
    }
    best.map(|(_, t)| t)
}

pub fn score(want: &ImportTrack, have: &Track) -> f64 {
    let wanted = key(&base_title(&want.title));
    let title = after_dash(&have.title).into_iter().map(|t| similarity(&wanted, &key(&base_title(t)), false)).fold(0.0, f64::max);
    if title < TITLE_FLOOR {
        return 0.0;
    }
    let artist = artist_score(want, have);
    if artist.is_some_and(|a| a < ARTIST_FLOOR) {
        return 0.0;
    }
    let length = (want.duration > 0.0 && have.duration > 0.0).then(|| (want.duration - have.duration).abs());
    if length.is_some_and(|d| d > OTHER_RECORDING) {
        return 0.0;
    }
    let length = length.map(|d| 1.0 - ((d - SAME_LENGTH).max(0.0) / (FAR_LENGTH - SAME_LENGTH)).min(1.0));
    let parts = [(title, 0.5), (artist.unwrap_or(-1.0), 0.3), (length.unwrap_or(-1.0), 0.2)];
    let (sum, weight) = parts.iter().filter(|(v, _)| *v >= 0.0).fold((0.0, 0.0), |(s, w), (v, k)| (s + v * k, w + k));
    let total = sum / weight;
    if versions(&want.title) == versions(&have.title) { total } else { total * OTHER_VERSION }
}

fn artist_score(want: &ImportTrack, have: &Track) -> Option<f64> {
    let wanted: Vec<String> = want.artists.iter().map(|a| key(a)).filter(|a| !a.is_empty()).collect();
    let found: Vec<String> = have.artists.iter().map(|a| key(a.name.trim_end_matches(" - Topic"))).filter(|a| !a.is_empty()).collect();
    if wanted.is_empty() || found.is_empty() {
        return None;
    }
    let pairs = wanted.iter().flat_map(|w| found.iter().map(move |h| same_artist(w, h)));
    Some(pairs.chain([same_artist(&wanted.join(" "), &found.join(" "))]).fold(0.0, f64::max))
}

fn same_artist(a: &str, b: &str) -> f64 {
    let telling = |name: &str| name.split(' ').filter(|w| !COMMON_WORDS.contains(w)).collect::<Vec<_>>().join(" ");
    let (left, right) = (telling(a), telling(b));
    if left.is_empty() || right.is_empty() { similarity(a, b, false) } else { similarity(&left, &right, true) }
}

fn after_dash(title: &str) -> Vec<&str> {
    let mut forms = vec![title];
    if let Some((_, rest)) = title.split_once(" - ") {
        forms.push(rest);
    }
    forms
}

pub fn base_title(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in title.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    if let Some((head, tail)) = out.split_once(" - ")
        && TAIL_WORDS.iter().any(|w| tail.to_lowercase().contains(w))
    {
        out = head.into();
    }
    let lower = out.to_lowercase();
    if let Some(i) = FEAT.iter().filter_map(|m| lower.find(m)).min() {
        out.truncate(i);
    }
    let out = out.trim().to_string();
    if out.is_empty() { title.trim().to_string() } else { out }
}

pub fn key(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            '\'' | '’' | '`' | 'ʼ' => {}
            '&' => out.push_str(" and "),
            _ => match fold(c) {
                Some(latin) => out.push_str(latin),
                None if c.is_alphanumeric() => out.push(c),
                None => out.push(' '),
            },
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fold(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' | 'ă' => "a",
        'ç' | 'ć' | 'č' => "c",
        'ď' | 'đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => "e",
        'ğ' => "g",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ı' => "i",
        'ł' => "l",
        'ñ' | 'ń' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
        'ř' => "r",
        'ś' | 'š' | 'ş' | 'ș' => "s",
        'ť' | 'ț' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => "u",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' | 'ґ' => "g",
        'д' => "d",
        'е' | 'ё' | 'э' => "e",
        'є' => "ye",
        'ж' => "zh",
        'з' => "z",
        'и' | 'і' => "i",
        'ї' => "yi",
        'й' | 'ы' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' | 'ь' => "",
        'ю' => "yu",
        'я' => "ya",
        _ => return None,
    })
}

fn similarity(a: &str, b: &str, overlap: bool) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b || a.replace(' ', "") == b.replace(' ', "") {
        return 1.0;
    }
    let (left, right): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
    let mut free = vec![true; right.len()];
    let mut same = 0;
    for word in &left {
        if let Some(i) = (0..right.len()).find(|&i| free[i] && alike(word, right[i])) {
            free[i] = false;
            same += 1;
        }
    }
    let same = f64::from(same);
    if overlap { same / left.len().min(right.len()) as f64 } else { 2.0 * same / (left.len() + right.len()) as f64 }
}

fn alike(a: &str, b: &str) -> bool {
    a == b
        || (a.chars().count() >= 4
            && b.chars().count() >= 4
            && 1.0 - distance(a, b) as f64 / a.chars().count().max(b.chars().count()) as f64 >= 0.8)
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(ca != *cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

fn versions(title: &str) -> Vec<&'static str> {
    let words = key(title);
    let words: Vec<&str> = words.split(' ').collect();
    VERSIONS.iter().copied().filter(|v| words.contains(v)).collect()
}

#[cfg(test)]
mod tests {
    use snellar_core::ids::TrackId;
    use snellar_core::model::Ref;

    use super::*;

    #[test]
    fn one_shared_common_word_is_not_the_same_artist() {
        let want = |artist: &str| ImportTrack {
            title: "Mr. Brightside".into(),
            artists: vec![artist.into()],
            album: String::new(),
            duration: 0.0,
        };
        let have = |artist: &str| Track {
            id: TrackId::youtube("aaaaaaaaaaa"),
            title: "Mr. Brightside".into(),
            artists: vec![Ref { id: String::new(), name: artist.into() }],
            ..Track::default()
        };
        assert_eq!(score(&want("The Killers"), &have("The Beatles")), 0.0);
        assert_eq!(score(&want("Los Lobos"), &have("Los Bunkers")), 0.0);
        assert!(score(&want("The Killers"), &have("Killers")) >= ENOUGH, "the article may be missing");
        assert!(score(&want("The The"), &have("The The")) >= ENOUGH, "a name of only common words still matches itself");
        assert!(score(&want("Simon & Garfunkel"), &have("Paul Simon")) >= ENOUGH, "a real shared name still counts");
    }

    fn want(title: &str, artists: &[&str], duration: f64) -> ImportTrack {
        ImportTrack { title: title.into(), artists: artists.iter().map(|a| a.to_string()).collect(), album: String::new(), duration }
    }

    fn have(id: &str, title: &str, artists: &[&str], duration: f64) -> Track {
        Track {
            id: TrackId::youtube(id),
            title: title.into(),
            artists: artists.iter().map(|a| Ref { id: String::new(), name: a.to_string() }).collect(),
            duration,
            ..Track::default()
        }
    }

    #[test]
    fn the_same_song_scores_full() {
        assert_eq!(score(&want("Get Lucky", &["Daft Punk"], 248.0), &have("a", "Get Lucky", &["Daft Punk"], 248.0)), 1.0);
    }

    #[test]
    fn remastered_feat_and_brackets_do_not_count() {
        let found = have("a", "Hey Jude", &["The Beatles"], 431.0);
        for title in [
            "Hey Jude (Remastered 2015)",
            "Hey Jude - Remastered 2015",
            "Hey Jude [2015 Mix]",
            "Hey Jude feat. Nobody",
            "Hey Jude (feat. Nobody)",
        ] {
            assert!(score(&want(title, &["The Beatles"], 429.0), &found) > 0.95, "{title}");
        }
        assert_eq!(base_title("Bohemian Rhapsody - Remastered 2011"), "Bohemian Rhapsody");
        assert_eq!(base_title("Song - Single Version"), "Song");
        assert_eq!(base_title("Hold On - We're Going Home"), "Hold On - We're Going Home", "a dash that is part of the title stays");
        assert_eq!(base_title("(Untitled)"), "(Untitled)", "a title made only of brackets is kept");
    }

    #[test]
    fn a_length_within_three_seconds_is_the_same_and_far_off_is_another_recording() {
        let w = want("Creep", &["Radiohead"], 238.0);
        assert_eq!(score(&w, &have("a", "Creep", &["Radiohead"], 241.0)), 1.0);
        let ten = score(&w, &have("a", "Creep", &["Radiohead"], 248.0));
        assert!((ENOUGH..1.0).contains(&ten), "ten seconds off still matches: {ten}");
        assert!(score(&w, &have("a", "Creep", &["Radiohead"], 258.0)) < score(&w, &have("a", "Creep", &["Radiohead"], 248.0)));
        assert_eq!(score(&w, &have("a", "Creep", &["Radiohead"], 300.0)), 0.0, "a minute longer is another recording");
        assert_eq!(
            score(&want("Creep", &["Radiohead"], 0.0), &have("a", "Creep", &["Radiohead"], 300.0)),
            1.0,
            "unknown length is not held against it"
        );
    }

    #[test]
    fn another_artist_or_another_title_is_rejected() {
        let w = want("Creep", &["Radiohead"], 238.0);
        assert_eq!(score(&w, &have("a", "Creep", &["TLC"], 238.0)), 0.0);
        assert_eq!(score(&w, &have("a", "Karma Police", &["Radiohead"], 238.0)), 0.0);
        assert_eq!(score(&want("Hey Jude", &["The Beatles"], 0.0), &have("a", "Hey", &["The Beatles"], 0.0)), 0.0);
    }

    #[test]
    fn transliteration_diacritics_and_case_do_not_matter() {
        assert!(score(&want("Группа крови", &["Кино"], 285.0), &have("a", "Gruppa Krovi", &["Kino"], 286.0)) > 0.95);
        assert!(score(&want("Хлеб", &["Мумий Тролль"], 0.0), &have("a", "Hleb", &["Mumiy Troll"], 0.0)) >= ENOUGH);
        assert!(score(&want("Crazy In Love", &["Beyoncé"], 236.0), &have("a", "CRAZY IN LOVE", &["Beyonce", "JAY-Z"], 236.0)) > 0.95);
        assert!(score(&want("Don't Stop Me Now", &["Queen"], 0.0), &have("a", "Don’t Stop Me Now", &["Queen"], 0.0)) > 0.95);
        assert_eq!(key("Simon & Garfunkel"), "simon and garfunkel");
        assert_eq!(key("Їжак, Ґанок"), "yizhak ganok");
    }

    #[test]
    fn one_of_several_artists_is_enough() {
        let w = want("Get Lucky", &["Daft Punk", "Pharrell Williams", "Nile Rodgers"], 369.0);
        assert!(score(&w, &have("a", "Get Lucky (feat. Pharrell Williams and Nile Rodgers)", &["Daft Punk"], 369.0)) > 0.95);
        assert!(score(&want("Song", &["Topic Band"], 0.0), &have("a", "Song", &["Topic Band - Topic"], 0.0)) > 0.95);
    }

    #[test]
    fn a_video_title_with_the_artist_in_front_still_matches() {
        let found = have("a", "Radiohead - Creep (Official Video)", &["Radiohead"], 239.0);
        assert!(score(&want("Creep", &["Radiohead"], 238.0), &found) > 0.95);
    }

    #[test]
    fn a_live_or_remixed_take_loses_to_the_studio_one() {
        let w = want("Creep", &["Radiohead"], 238.0);
        let live = have("live", "Creep (Live)", &["Radiohead"], 238.0);
        let studio = have("studio", "Creep", &["Radiohead"], 240.0);
        assert!(score(&w, &live) < score(&w, &studio));
        assert_eq!(best(&w, &[live.clone(), studio.clone()]).unwrap().id, studio.id);
        assert_eq!(best(&want("Creep (Live)", &["Radiohead"], 238.0), &[studio, live]).unwrap().id.as_str(), "yt:live");
    }

    #[test]
    fn best_keeps_the_first_of_equals_and_gives_nothing_below_the_bar() {
        let w = want("Creep", &["Radiohead"], 238.0);
        let one = have("one", "Creep", &["Radiohead"], 238.0);
        let two = have("two", "Creep", &["Radiohead"], 238.0);
        assert_eq!(best(&w, &[one.clone(), two]).unwrap().id, one.id);
        assert!(best(&w, &[have("x", "Creep", &["TLC"], 238.0)]).is_none());
        assert!(best(&w, &[]).is_none());
    }

    #[test]
    fn empty_and_odd_input_never_panics() {
        let odd = [want("", &[], 0.0), want("((", &[""], f64::NAN), want("ß", &["&"], -1.0)];
        for w in &odd {
            for h in [have("a", "", &[], 0.0), have("a", "))", &[""], f64::INFINITY)] {
                let s = score(w, &h);
                assert!((0.0..=1.0).contains(&s), "{s}");
            }
        }
    }
}
