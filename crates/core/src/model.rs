use serde::{Deserialize, Serialize};

use crate::error::Code;
use crate::ids::TrackId;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Ref {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub artists: Vec<Ref>,
    pub album: Option<Ref>,
    pub duration: f64,
    pub thumb: String,
    pub explicit: bool,
    pub video: bool,
}

impl Track {
    pub fn artist_line(&self) -> String {
        self.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AlbumCard {
    pub id: String,
    pub title: String,
    pub artists: Vec<Ref>,
    pub year: String,
    pub kind: String,
    pub thumb: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtistCard {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub thumb: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Hit {
    Track(Track),
    Album(AlbumCard),
    Artist(ArtistCard),
    Playlist(AlbumCard),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Suggestions {
    pub queries: Vec<String>,
    pub hits: Vec<Hit>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchResult {
    pub top: Option<Hit>,
    pub tracks: Vec<Track>,
    pub videos: Vec<Track>,
    pub linked: Option<Track>,
    pub page: Option<Page>,
    pub albums: Vec<AlbumCard>,
    pub artists: Vec<ArtistCard>,
    pub playlists: Vec<AlbumCard>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artists: Vec<Ref>,
    pub year: String,
    pub kind: String,
    pub thumb: String,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub thumb: String,
    pub banner: String,
    pub subscribers: String,
    pub description: String,
    pub top: Vec<Track>,
    pub albums: Vec<AlbumCard>,
    pub singles: Vec<AlbumCard>,
    pub related: Vec<ArtistCard>,
    pub songs: String,
    pub more_albums: Option<Browse>,
    pub more_singles: Option<Browse>,
    pub radio: Option<Browse>,
    pub shuffle: Option<Browse>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Mix {
    Radio,
    Shuffle,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Browse {
    pub id: String,
    pub params: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Discography {
    pub artist: Ref,
    pub albums: Vec<AlbumCard>,
    pub singles: Vec<AlbumCard>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Shelf {
    pub title: String,
    pub kind: String,
    pub arg: String,
    pub tracks: Vec<Track>,
    pub albums: Vec<AlbumCard>,
    pub artists: Vec<ArtistCard>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LyricLine {
    pub time: f64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Lyrics {
    pub synced: bool,
    pub lines: Vec<LyricLine>,
    pub plain: String,
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Dated<T> {
    #[serde(flatten)]
    pub item: T,
    #[serde(default)]
    pub at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub cover: String,
    pub folder: Option<i64>,
    pub count: usize,
    pub thumb: String,
    pub tracks: Vec<Dated<Track>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Folder {
    pub id: i64,
    pub name: String,
    pub open: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Pin {
    pub kind: String,
    pub id: String,
    pub item: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Track,
    Album,
    Artist,
    Playlist,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Page {
    pub kind: Kind,
    pub id: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Fx {
    pub enabled: bool,
    pub preamp: f64,
    pub eq: [f64; 10],
    pub bass: f64,
    pub speed: f64,
    pub pitch_lock: bool,
    pub reverb: f64,
    pub echo: f64,
    pub stereo: f64,
    pub normalize: bool,
}

impl Default for Fx {
    fn default() -> Self {
        Self {
            enabled: true,
            preamp: 0.0,
            eq: [0.0; 10],
            bass: 0.0,
            speed: 1.0,
            pitch_lock: false,
            reverb: 0.0,
            echo: 0.0,
            stereo: 1.0,
            normalize: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportTrack {
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub duration: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportResult {
    pub playlist: Option<i64>,
    pub name: String,
    pub total: usize,
    pub matched: usize,
    pub unmatched: Vec<ImportTrack>,
    pub limited: bool,
    pub source_total: Option<usize>,
    pub failed: usize,
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportProgress {
    pub stage: String,
    pub done: usize,
    pub total: usize,
    pub matched: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioFormat {
    pub codec: String,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub loudness_db: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Leveling {
    #[default]
    Off,
    Gain,
    Measured,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct StreamInfo {
    pub codec: String,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub gain: Option<f64>,
    pub leveling: Leveling,
    pub local: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
}
