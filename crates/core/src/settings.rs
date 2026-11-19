use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::error::Code;
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
pub const BACKGROUND_DIM: Setting = setting(
    "backgroundDim",
    Kind::Number,
    "0.5",
    Scope::Ui,
    "How dark the Cover and Solid window backgrounds are, from 0 (lightest) to 1 (darkest); 0.5 is the standard look. Glass ignores it.",
)
.on(Section::Appearance);
pub const REDUCE_MOTION: Setting = setting(
    "reduceMotion",
    Kind::Bool,
    "false",
    Scope::Ui,
    "Keep decorative motion still: the sky behind the Home card does not breathe, drift or twinkle, and the sidebar folds without moving. Also follows the desktop's own reduced-motion setting, read through the desktop portal.",
)
.on(Section::Appearance);
pub const NAV_RIGHT: Setting =
    setting("navRight", Kind::Bool, "false", Scope::Ui, "Sidebar on the right, the layout mirrored.").on(Section::Appearance);
pub const SIDEBAR_COLLAPSED: Setting = setting(
    "sidebarCollapsed",
    Kind::Bool,
    "false",
    Scope::Ui,
    "The sidebar is a narrow strip of icons, chosen with the button at its top. A window too narrow for the whole sidebar shows the strip without changing this.",
);
pub const SORT: Setting = setting(
    "sort.*",
    Kind::Json,
    "null",
    Scope::Ui,
    "Sort of an own list (`favorites`, `history`, `playlist`, `albums`, `artists`): `{key, reversed}`.",
);
pub const TIME_LEFT: Setting = setting(
    "timeLeft",
    Kind::Bool,
    "false",
    Scope::Ui,
    "The number at the right end of the seek bar shows the time left, as `-2:15`, not the track's length; a click on it switches.",
);
pub const VOLUME: Setting = setting("volume", Kind::Number, "85", Scope::Player, "Volume, 0 to 100.");
pub const FX: Setting = setting("fx", Kind::Json, "{}", Scope::Player, "Equalizer and effects (`Fx`); missing fields take their defaults.");
pub const AUTOPLAY: Setting =
    setting("autoplay", Kind::Bool, "true", Scope::Player, "Keep playing related tracks after the queue ends.").on(Section::Playback);
pub const RECS_LIVE: Setting = setting(
    "recsLive",
    Kind::Bool,
    "false",
    Scope::App,
    "Recommendations may offer live recordings as they are; off, the studio original is looked up instead.",
)
.on(Section::Playback);
pub const RECS_COVERS: Setting = setting(
    "recsCovers",
    Kind::Bool,
    "false",
    Scope::App,
    "Recommendations may offer covers, tributes and karaoke as they are; off, the original is looked up instead.",
)
.on(Section::Playback);
pub const RECS_REMIXES: Setting = setting(
    "recsRemixes",
    Kind::Bool,
    "false",
    Scope::App,
    "Recommendations may offer remixes as they are; off, the original is looked up instead.",
)
.on(Section::Playback);
pub const RECS_SPEED: Setting = setting(
    "recsSpeed",
    Kind::Bool,
    "false",
    Scope::App,
    "Recommendations may offer sped-up, slowed, nightcore, reverb, 8D and bass-boosted versions as they are; off, the original is looked up instead and the version is left out when there is none, in every list.",
)
.on(Section::Playback);
pub const RECS_SPAM: Setting = setting(
    "recsSpam",
    Kind::Bool,
    "true",
    Scope::App,
    "Recommendations leave out likely AI and spam artists: a title, channel or artist name that says it was made by AI, and artists with at least 200 releases and fewer than 1000 fans on Deezer. Artists you liked, played, listed or follow are never left out.",
)
.on(Section::Playback);
pub const LOUDNESS: Setting = setting(
    "loudness",
    Kind::Bool,
    "true",
    Scope::Player,
    "Even out loudness between tracks: YouTube's loudness value lowers a loud track, a track without one is measured by mpv's `loudnorm` on its first play and plays with the gain from that measure after. Never raises a track above its own level.",
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
    setting("downloadDir", Kind::Text, "\"\"", Scope::App, "Download folder: a full path or one that starts with ~/; empty is ~/Music/Snellar. A relative path is refused with `download.folder_invalid`.").on(Section::Library);
pub const DOWNLOAD_REPAIRS: Setting = setting(
    "downloadRepairs",
    Kind::Json,
    "[]",
    Scope::State,
    "Songs whose downloaded file was shared with another song (saved before the video id was in file names) and that still download again into their own file: `string[]` of track ids. Written by the downloads at start.",
);
pub const DOWNLOAD_REPAIRS_REFUSED: Setting = setting(
    "downloadRepairsRefused",
    Kind::Json,
    "[]",
    Scope::State,
    "Songs from `downloadRepairs` YouTube refused for good (removed, private, age-restricted) or that are no YouTube track: `string[]` of track ids. Their repair is not tried again while they still share a file. Written by the downloads.",
);
pub const DOWNLOADED_COLLECTIONS: Setting = setting(
    "downloadedCollections",
    Kind::Json,
    "{}",
    Scope::State,
    "Playlists and albums kept downloaded, and the songs downloaded only for them: `{collections: [{kind, id, tracks?}], pulled: string[]}`. Written by `downloadCollection`.",
);
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
pub const RECENT_SEARCHES: Setting = setting(
    "recentSearches",
    Kind::Json,
    "[]",
    Scope::State,
    "What the empty search field offers again, newest first: `{query}` for a search that was run, `{hit}` (a `Hit`) for a suggestion that was opened. The window keeps at most 8 and writes the whole list.",
);

pub static ALL: &[&Setting] = &[
    &LANGUAGE,
    &ACCENT,
    &GLASS,
    &BACKGROUND_DIM,
    &NAV_RIGHT,
    &REDUCE_MOTION,
    &SIDEBAR_COLLAPSED,
    &SORT,
    &TIME_LEFT,
    &VOLUME,
    &FX,
    &AUTOPLAY,
    &RECS_LIVE,
    &RECS_COVERS,
    &RECS_REMIXES,
    &RECS_SPEED,
    &RECS_SPAM,
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
    &DOWNLOADED_COLLECTIONS,
    &DOWNLOAD_REPAIRS,
    &DOWNLOAD_REPAIRS_REFUSED,
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
    &RECENT_SEARCHES,
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

/// The test of a saved value, such as `reads_as::<T>`.
pub type Reads = fn(&str) -> bool;

/// True when `raw` is a saved value `Setting::read::<T>` reads: the test `keep_aside` and `set_aside` take.
pub fn reads_as<T: DeserializeOwned>(raw: &str) -> bool {
    serde_json::from_str::<Option<T>>(raw).is_ok()
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

    /// The saved value; `null` is nothing saved, like `get`. A value that does not parse is an error, not
    /// "nothing saved", and a read never writes: the value stays as it is, so one a newer build saved
    /// survives this one. It is replaced only by an explicit `setSetting` after `keep_aside`, or for a state
    /// setting by `set_aside`.
    pub fn read<T: DeserializeOwned>(&self, library: &Library) -> anyhow::Result<Option<T>> {
        let Some(raw) = library.setting_read(self.key)? else { return Ok(None) };
        serde_json::from_str::<Option<T>>(&raw).map_err(|e| {
            Code::LibraryCorrupt
                .err(format!("setting `{}` is not what this build reads ({e}); it is left as it is until it is set aside", self.key))
        })
    }

    /// Before an explicit write: the value saved now, if `reads` refuses it, is copied to the first free name of
    /// `<key>.broken`, `<key>.broken.2`, …, so a copy kept earlier is never written over. False when it could
    /// not be kept; the write must not happen then.
    pub fn keep_aside(&self, library: &Library, reads: Reads) -> bool {
        match library.setting_read(self.key) {
            Ok(Some(raw)) if !reads(&raw) => self.put_aside(library, &raw).is_some(),
            Ok(_) => true,
            Err(_) => false,
        }
    }

    /// The way back for a state setting only the app writes, run once where its owner starts: a saved value
    /// `reads` refuses is kept aside as in `keep_aside`, and the setting starts again from its default. True when
    /// that happened. A preference is never reset here: its value stays until the person sets it.
    pub fn set_aside(&self, library: &Library, reads: Reads) -> anyhow::Result<bool> {
        if self.scope != Scope::State {
            return Ok(false);
        }
        let raw = match library.setting_read(self.key)? {
            Some(raw) if !reads(&raw) => raw,
            _ => return Ok(false),
        };
        let Some(aside) = self.put_aside(library, &raw) else {
            return Err(Code::LibraryFailed.err(format!("setting `{}` cannot be read and could not be kept aside", self.key)));
        };
        if !library.set_setting(self.key, self.default) {
            return Err(Code::LibraryFailed.err(format!("setting `{}` was kept aside as `{aside}` but could not be reset", self.key)));
        }
        crate::journal::warn(
            "settings",
            format!("setting `{}` could not be read; it was kept as `{aside}` and starts again from its default", self.key),
        );
        Ok(true)
    }

    /// The name the raw value is kept under: the copy that already holds it, or the first free name.
    fn put_aside(&self, library: &Library, raw: &str) -> Option<String> {
        let mut n = 1;
        loop {
            let aside = if n == 1 { format!("{}.broken", self.key) } else { format!("{}.broken.{n}", self.key) };
            match library.setting_read(&aside).ok()? {
                Some(kept) if kept == raw => return Some(aside),
                Some(_) => n += 1,
                None => return library.set_setting(&aside, raw).then_some(aside),
            }
        }
    }

    pub fn or_default<T: DeserializeOwned>(&self, library: &Library) -> T {
        self.get(library).unwrap_or_else(|| serde_json::from_str(self.default).expect("registry defaults fit their type"))
    }

    /// False when the setting could not be written.
    pub fn set<T: Serialize>(&self, library: &Library, value: &T) -> bool {
        library.set_setting(self.key, &serde_json::to_string(value).unwrap_or_default())
    }
}
