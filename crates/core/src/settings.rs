use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::library::Library;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Number,
    Text,
    Json,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Bool => "bool",
            Kind::Number => "number",
            Kind::Text => "text",
            Kind::Json => "json",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    App,
    Ui,
    Player,
    State,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Scope::App => "app",
            Scope::Ui => "ui",
            Scope::Player => "player",
            Scope::State => "state",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    General,
    Appearance,
    Playback,
    Audio,
    Library,
    Services,
    About,
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Appearance => "appearance",
            Section::Playback => "playback",
            Section::Audio => "audio",
            Section::Library => "library",
            Section::Services => "services",
            Section::About => "about",
        }
    }
}

#[derive(Debug)]
pub struct Setting {
    pub key: &'static str,
    pub kind: Kind,
    pub default: &'static str,
    pub scope: Scope,
    pub about: &'static str,
    pub section: Option<Section>,
}

const fn setting(key: &'static str, kind: Kind, default: &'static str, scope: Scope, about: &'static str) -> Setting {
    Setting { key, kind, default, scope, about, section: None }
}

pub const LANGUAGE: Setting =
    setting("language", Kind::Text, "\"\"", Scope::Ui, "Interface language code; empty follows the system.").on(Section::General);
pub const ACCENT: Setting =
    setting("accent", Kind::Text, "\"glass\"", Scope::Ui, "Accent palette name, or `cover` to follow the cover.").on(Section::Appearance);
pub const GLASS: Setting = setting(
    "glass",
    Kind::Text,
    "\"auto\"",
    Scope::Ui,
    "Window background: `glass` (see-through, the compositor blurs), `cover` (opaque, the blurred cover behind), `solid`, or `auto` to choose by desktop.",
)
.on(Section::Appearance);
pub const NAV_RIGHT: Setting =
    setting("navRight", Kind::Bool, "false", Scope::Ui, "Sidebar on the right, the layout mirrored.").on(Section::Appearance);
pub const SORT: Setting = setting(
    "sort.*",
    Kind::Json,
    "null",
    Scope::Ui,
    "Sort of an own list (`favorites`, `history`, `playlist`, `albums`, `artists`): `{key, reversed}`.",
);
pub const VOLUME: Setting = setting("volume", Kind::Number, "85", Scope::Player, "Volume, 0 to 100.");
pub const FX: Setting = setting("fx", Kind::Json, "{}", Scope::Player, "Equalizer and effects (`Fx`); missing fields take their defaults.");
pub const AUTOPLAY: Setting =
    setting("autoplay", Kind::Bool, "true", Scope::Player, "Keep playing related tracks after the queue ends.").on(Section::Playback);
pub const LOUDNESS: Setting = setting(
    "loudness",
    Kind::Bool,
    "true",
    Scope::Player,
    "Even out loudness between tracks: YouTube's loudness value lowers a loud track, a track without one is measured by mpv's `loudnorm`. Never raises a track above its own level.",
)
.on(Section::Audio);
pub const QUALITY: Setting = setting(
    "quality",
    Kind::Text,
    "\"auto\"",
    Scope::Player,
    "Stream quality: `auto` (the best Opus stream up to 160 kbps), `high` (the highest bitrate of any codec), `dataSaver` (the lowest bitrate). Applies from the next track.",
)
.on(Section::Audio);
pub const AUDIO_DEVICE: Setting = setting(
    "audioDevice",
    Kind::Text,
    "\"auto\"",
    Scope::Player,
    "Output device: a `name` from `audioDevices`, or `auto`. A device that is not connected plays on `auto` until it is back.",
)
.on(Section::Audio);
pub const CLOSE_TO_BACKGROUND: Setting = setting(
    "closeToBackground",
    Kind::Bool,
    "false",
    Scope::Ui,
    "Closing the window keeps the music playing, with an icon in the tray to show the window again; only where a tray (StatusNotifierItem host) is running.",
)
.on(Section::General);
pub const QUEUE: Setting = setting("queue", Kind::Json, "null", Scope::State, "The saved queue and cursor.");
pub const POSITION: Setting = setting("position", Kind::Number, "0", Scope::State, "Seconds into the current track at the last save.");
pub const HANDOFF: Setting =
    setting("handoff", Kind::Bool, "false", Scope::App, "Offer to continue what another MPRIS player plays.").on(Section::Playback);
pub const DISCORD_RPC: Setting =
    setting("discordRpc", Kind::Bool, "false", Scope::App, "Show the current track in Discord.").on(Section::Services);
pub const DISCORD_APP_ID: Setting =
    setting("discordAppId", Kind::Text, "\"\"", Scope::App, "A custom Discord application id; empty uses Snellar's.");
pub const DOWNLOAD_DIR: Setting =
    setting("downloadDir", Kind::Text, "\"\"", Scope::App, "Download folder; empty is ~/Music/Snellar.").on(Section::Library);
pub const LASTFM_KEY: Setting =
    setting("lastfmKey", Kind::Text, "\"\"", Scope::App, "Last.fm API key for similar artists.").on(Section::Services);
pub const PROXY: Setting = setting(
    "proxy",
    Kind::Text,
    "\"\"",
    Scope::App,
    "HTTP proxy for every request Snellar makes, yt-dlp included (`http://host:port`); empty uses the system's `HTTPS_PROXY`. Applies at once.",
)
.on(Section::Services);
pub const REGION: Setting = setting("region", Kind::Text, "\"\"", Scope::App, "YouTube Music region code; empty lets YouTube choose.");
pub const YTDLP_FALLBACK: Setting =
    setting("ytdlpFallback", Kind::Bool, "true", Scope::App, "Use yt-dlp when a stream cannot be resolved directly.");
pub const GLASS_UNDO: Setting = setting(
    "glassUndo",
    Kind::Json,
    "null",
    Scope::State,
    "What the Blur my Shell setup changed, kept for its undo. Written only by the window's setup button.",
);
pub const YTDLP_CHECKED: Setting = setting(
    "ytdlpChecked",
    Kind::Number,
    "0",
    Scope::State,
    "When Snellar last compared its yt-dlp with the latest release, in milliseconds.",
);
pub const AUTO_UPDATE: Setting = setting(
    "autoUpdate",
    Kind::Bool,
    "true",
    Scope::App,
    "Keep an installed Snellar up to date: at most once a day the latest release is fetched in the background and starts at the next launch.",
)
.on(Section::About);
pub const UPDATE_CHECKED: Setting =
    setting("updateChecked", Kind::Number, "0", Scope::State, "When Snellar last read the latest release of itself, in milliseconds.");
pub const RECS_EXPLORE: Setting =
    setting("recsExplore", Kind::Number, "0.3", Scope::App, "Share of unfamiliar artists in recommendations, 0 to 1.");
pub const WAVE: Setting = setting("wave", Kind::Json, "{}", Scope::App, "Saved wave knobs: `{mood?, language?, novelty?, limit?}`.");

pub static ALL: &[&Setting] = &[
    &LANGUAGE,
    &ACCENT,
    &GLASS,
    &NAV_RIGHT,
    &SORT,
    &VOLUME,
    &FX,
    &AUTOPLAY,
    &AUDIO_DEVICE,
    &QUALITY,
    &LOUDNESS,
    &CLOSE_TO_BACKGROUND,
    &QUEUE,
    &POSITION,
    &HANDOFF,
    &DISCORD_RPC,
    &DISCORD_APP_ID,
    &DOWNLOAD_DIR,
    &LASTFM_KEY,
    &PROXY,
    &REGION,
    &YTDLP_FALLBACK,
    &YTDLP_CHECKED,
    &GLASS_UNDO,
    &AUTO_UPDATE,
    &UPDATE_CHECKED,
    &RECS_EXPLORE,
    &WAVE,
];

pub struct Alias {
    pub old: &'static str,
    pub new: &'static Setting,
    pub value: fn(&Value) -> Value,
    pub about: &'static str,
}

fn glass_from_tinted(tinted: &Value) -> Value {
    json!(if tinted.as_bool() == Some(true) { "cover" } else { "glass" })
}

pub static ALIASES: &[Alias] = &[Alias {
    old: "glassTinted",
    new: &GLASS,
    value: glass_from_tinted,
    about: "`true` (tinted) reads as `cover`, `false` (clear) as `glass`",
}];

pub fn alias(key: &str) -> Option<&'static Alias> {
    ALIASES.iter().find(|a| a.old == key)
}

pub fn find(key: &str) -> Option<&'static Setting> {
    ALL.iter()
        .copied()
        .find(|s| match s.key.strip_suffix('*') {
            Some(prefix) => key.len() > prefix.len() && key.starts_with(prefix),
            None => s.key == key,
        })
        .or_else(|| alias(key).map(|a| a.new))
}

pub fn stored(library: &Library, key: &str) -> Option<Value> {
    if let Some(renamed) = alias(key) {
        return stored(library, renamed.new.key);
    }
    let parse = |raw: String| serde_json::from_str::<Value>(&raw).ok().filter(|v| !v.is_null());
    if let Some(raw) = library.setting(key) {
        return parse(raw);
    }
    ALIASES.iter().filter(|a| a.new.key == key).find_map(|a| library.setting(a.old).and_then(parse).map(|v| (a.value)(&v)))
}

pub fn registry() -> Value {
    let entry = |s: &&Setting| {
        json!({
            "key": s.key,
            "kind": s.kind.name(),
            "default": s.default_value(),
            "scope": s.scope.name(),
            "about": s.about,
            "section": s.section.map(Section::name),
        })
    };
    ALL.iter().map(entry).collect()
}

impl Setting {
    const fn on(self, section: Section) -> Setting {
        Setting { section: Some(section), ..self }
    }

    fn default_value(&self) -> Value {
        serde_json::from_str(self.default).expect("registry defaults are JSON")
    }

    pub fn accepts(&self, value: &Value) -> bool {
        match self.kind {
            Kind::Bool => value.is_boolean() || value.is_null(),
            Kind::Number => value.is_number() || value.is_null(),
            Kind::Text => value.is_string() || value.is_null(),
            Kind::Json => true,
        }
    }

    pub fn get<T: DeserializeOwned>(&self, library: &Library) -> Option<T> {
        stored(library, self.key).and_then(|v| serde_json::from_value(v).ok())
    }

    pub fn or_default<T: DeserializeOwned>(&self, library: &Library) -> T {
        self.get(library).unwrap_or_else(|| serde_json::from_str(self.default).expect("registry defaults fit their type"))
    }

    pub fn set<T: Serialize>(&self, library: &Library, value: &T) {
        library.set_setting(self.key, &serde_json::to_string(value).unwrap_or_default());
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::model::Fx;

    #[test]
    fn every_entry_is_well_formed_and_unique() {
        let mut keys: Vec<&str> = ALL.iter().map(|s| s.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), ALL.len());
        for s in ALL {
            assert!(s.accepts(&s.default_value()), "{} default fits {:?}", s.key, s.kind);
            assert!(!s.about.is_empty(), "{} has a description", s.key);
            assert_eq!(find(&s.key.replace('*', "x")).map(|f| f.key), Some(s.key));
        }
    }

    #[test]
    fn the_settings_page_shows_preferences_only() {
        let shown: Vec<&str> = ALL.iter().filter(|s| s.section.is_some()).map(|s| s.key).collect();
        assert_eq!(
            shown,
            [
                "language",
                "accent",
                "glass",
                "navRight",
                "autoplay",
                "audioDevice",
                "quality",
                "loudness",
                "closeToBackground",
                "handoff",
                "discordRpc",
                "downloadDir",
                "lastfmKey",
                "proxy",
                "autoUpdate"
            ]
        );
        for s in ALL.iter().filter(|s| s.section.is_some()) {
            assert!(s.scope != Scope::State && !s.key.ends_with('*'), "{} is a preference", s.key);
        }
        assert_eq!(registry()[0]["section"], "general");
        assert!(registry().as_array().unwrap().iter().any(|s| s["section"].is_null()));
    }

    #[test]
    fn every_key_ever_written_still_resolves() {
        let old = [
            "language",
            "accent",
            "glassTinted",
            "navRight",
            "volume",
            "fx",
            "autoplay",
            "queue",
            "position",
            "handoff",
            "discordRpc",
            "discordAppId",
            "downloadDir",
            "lastfmKey",
            "region",
            "ytdlpFallback",
            "ytdlpChecked",
            "glassUndo",
            "recsExplore",
            "wave",
        ];
        for key in old {
            assert!(find(key).is_some(), "{key}");
        }
        assert!(find("sort.").is_none());
        assert!(find("theme").is_none());
    }

    #[test]
    fn renamed_keys_still_read_their_old_values() {
        let l = Library::open(Path::new(":memory:")).unwrap();
        assert_eq!(GLASS.or_default::<String>(&l), "auto", "never set");
        for (old, new) in [("true", "cover"), ("false", "glass")] {
            l.set_setting("glassTinted", old);
            assert_eq!(GLASS.get::<String>(&l).as_deref(), Some(new), "glassTinted {old}");
        }
        assert_eq!(find("glassTinted").map(|s| s.key), Some("glass"));
        l.set_setting("glass", "\"solid\"");
        assert_eq!(GLASS.get::<String>(&l).as_deref(), Some("solid"), "the new key wins");
        l.set_setting("glass", "null");
        assert_eq!(GLASS.or_default::<String>(&l), "auto", "cleared means the default, not the old value");
        for a in ALIASES {
            assert!(ALL.iter().all(|s| s.key != a.old), "{} is not also a key", a.old);
            assert!(ALL.iter().any(|s| s.key == a.new.key), "{} points into the registry", a.old);
        }
    }

    #[test]
    fn defaults_match_what_the_code_assumed() {
        let l = Library::open(Path::new(":memory:")).unwrap();
        assert_eq!(VOLUME.or_default::<f64>(&l), 85.0);
        assert_eq!(FX.or_default::<Fx>(&l), Fx::default());
        assert!(AUTOPLAY.or_default::<bool>(&l));
        assert!(!HANDOFF.or_default::<bool>(&l));
        assert!(!DISCORD_RPC.or_default::<bool>(&l));
        assert!(!CLOSE_TO_BACKGROUND.or_default::<bool>(&l), "closing quits unless asked");
        assert!(YTDLP_FALLBACK.or_default::<bool>(&l));
        assert!(AUTO_UPDATE.or_default::<bool>(&l), "updates are on unless turned off");
        assert_eq!(POSITION.or_default::<f64>(&l), 0.0);
        assert_eq!(ACCENT.or_default::<String>(&l), "glass");
    }

    #[test]
    fn values_written_by_older_code_read_back() {
        let l = Library::open(Path::new(":memory:")).unwrap();
        for (raw, volume) in [("40", 40.0), ("42.5", 42.5)] {
            l.set_setting("volume", raw);
            assert_eq!(VOLUME.or_default::<f64>(&l), volume);
        }
        l.set_setting("handoff", "true");
        assert!(HANDOFF.or_default::<bool>(&l));
        l.set_setting("handoff", "garbage");
        assert!(!HANDOFF.or_default::<bool>(&l));
        VOLUME.set(&l, &50.0);
        assert_eq!(VOLUME.get::<f64>(&l), Some(50.0));
    }

    #[test]
    fn kinds_check_values() {
        assert!(HANDOFF.accepts(&json!(true)) && !HANDOFF.accepts(&json!("true")));
        assert!(VOLUME.accepts(&json!(3)) && !VOLUME.accepts(&json!("3")));
        assert!(LANGUAGE.accepts(&json!("uk")) && !LANGUAGE.accepts(&json!(1)));
        assert!(WAVE.accepts(&json!({ "mood": "calm" })));
        assert!(HANDOFF.accepts(&Value::Null));
    }
}
