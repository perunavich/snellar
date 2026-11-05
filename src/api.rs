use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use snellar_core::cache::Memo;
use snellar_core::error::{self, Code, Told};
use snellar_core::ids::TrackId;
use snellar_core::journal;
use snellar_core::library::Library;
use snellar_core::model::{Artist, Fx, Kind, Mix, Pin, Playlist, Repeat, Shelf, Track};
use snellar_core::settings;
use snellar_core::source::{Filter, Source};
use snellar_core::ui::{LibraryProblem, PlayFailure, Toast, Ui, UiEvent};
use snellar_youtube::lyrics::LyricsSource;
use snellar_youtube::net::{Http, Route};
use snellar_youtube::proxy::{Bound, Proxy};
use snellar_youtube::recs::Recs;
use snellar_youtube::source::YouTube;
use snellar_youtube::stream::{Quality, Resolver};
use snellar_youtube::ytm::Ytm;

use crate::covers;
use crate::downloads::Downloads;
use crate::fx;
use crate::handoff::{self, Handoff};
use crate::import::{self, Imports, Input};
use crate::mpv;
use crate::player::Player;
use crate::report;

pub const PICK_PATH: &str = "pickPath";
const HANDOFF_OFFER: Duration = Duration::from_secs(15);
const SEARCH_TTL: Duration = Duration::from_secs(120);
const SUGGEST_TTL: Duration = Duration::from_secs(600);
const PAGE_TTL: Duration = Duration::from_secs(600);
const RECENT_CARDS: usize = 20;
const CONNECT_LIMIT: Duration = Duration::from_secs(4);
const LOCK_RECHECK: Duration = Duration::from_secs(3);
const NO_RECHECK: Duration = Duration::from_secs(365 * 24 * 3600);
const LIBRARY_READS: [&str; 10] =
    ["favorites", "savedAlbums", "savedPlaylists", "followed", "history", "playlists", "playlist", "library", "folders", "dislikes"];

#[derive(Serialize)]
pub struct Method {
    pub name: &'static str,
    pub group: &'static str,
    pub args: &'static str,
    pub returns: &'static str,
    pub doc: &'static str,
}

const fn m(name: &'static str, group: &'static str, args: &'static str, returns: &'static str, doc: &'static str) -> Method {
    Method { name, group, args, returns, doc }
}

#[rustfmt::skip]
pub const METHODS: &[Method] = &[
    m("search", "browse", "q: string, filter?: all|songs|videos|albums|artists", "SearchResult", "Search YouTube Music. `top` is the top result as a `Hit` (see `suggest`): an artist, album, playlist, song or video (all filter). `playlists` holds YouTube playlists as cards (all filter). `videos` holds music videos and uploads with `Artist - Title [tags]` names split into artist and title (all and videos filters). A pasted YouTube link (watch, youtu.be, music.youtube.com, shorts, embed) returns just that track in `linked`; a link to an album, artist or playlist returns `page: {kind, id}` and nothing else. Results are cached for two minutes."),
    m("suggest", "browse", "q: string", "{queries: string[], hits: Hit[]}", "Search-box suggestions for a partial query: completed queries, and the artists, albums, songs and playlists YouTube Music offers for it. A `Hit` is a `Track`, `AlbumCard` or `ArtistCard` with `type`: `track`, `album`, `artist` or `playlist` (an `AlbumCard` whose id is a playlist id)."),
    m("home", "browse", "", "{shelves: Shelf[], recent: {kind, id, item}[], problem?: string, retryAfter?: int}", "Personalised home shelves (served from cache, refreshed in the background) and what was played lately, without repeats: `kind` is `album`, `remote`, `playlist`, `artist` or, where nothing better exists, `track`; `item` is the card. When YouTube failed and something else can be shown, `problem` is the error code and what is shown is older or partial; with nothing to show the call fails with that code."),
    m("homeRefresh", "browse", "", "{shelves: Shelf[], recent: {kind, id, item}[], problem?: string, retryAfter?: int}", "Rebuild the home shelves now. A failure never replaces what was kept."),
    m("album", "browse", "id: string", "Album", "Album or single with its tracks. `id` is an AlbumCard id from search."),
    m("artist", "browse", "id: string", "Artist", "Artist page: top songs, albums, singles, related artists."),
    m("discography", "browse", "id: string", "Discography", "Every album and every single or EP of an artist: `{artist: {id, name}, albums: AlbumCard[], singles: AlbumCard[]}` in the source's order, read through every page. An artist without a full list gives what its page shows."),
    m("artistSongs", "browse", "id: string", "Track[]", "All popular songs of an artist, every page of the list behind Top songs; the top songs of the page when the artist has no such list."),
    m("artistMix", "browse", "id: string, kind: radio|shuffle", "Track[]", "The artist's own mix from the source: `radio` is the artist radio (the artist and similar ones), `shuffle` the artist's songs in a new order each time. Empty when the source has none; the window then falls back to the radio of the top song or a shuffle of `artistSongs`."),
    m("remotePlaylist", "browse", "id: string", "Album", "A YouTube Music playlist with all its tracks, read through every page."),
    m("lyrics", "browse", "track: Track", "Lyrics", "Synced lines when available, else plain text."),
    m("link", "browse", "kind: track|album|artist|playlist, id: string", "string|null", "A link to share: the music.youtube.com page of a track id, album, artist or YouTube playlist id; null when there is none (a local playlist, an id the source does not know)."),
    m("track", "browse", "id: string", "Track", "Metadata for a track id: from the library when known, otherwise from YouTube Music."),
    m("nudgeAnswer", "playback", "id: string, action: string", "null", "Answer a nudge card. For the handoff offer `continue` pauses the other player and continues here from the same position, `ignore` drops it; a late or unknown id does nothing."),
    m("wave", "browse", "mood?: string, language?: uk|ru|en|ko|ja|zh, novelty?: number, limit?: int, reshuffle?: bool", "Track[]", "Wave preview, the same list as the first track shelf of `home`: tracks built from the library's taste around the YouTube Music radio, kept for 30 minutes unless `reshuffle`. Without arguments the saved `wave` setting is used. `novelty` is the share of unfamiliar artists, 0 to 1."),
    m("play", "playback", "tracks: Track[], index?: int, context?: {kind, item}", "null", "Replace the queue with `tracks` and start playing at `index` (default 0). `context` names the collection they come from (`kind` as in `pin`, `item` its card), so that it shows in the recent cards on `home`."),
    m("radio", "playback", "track: Track", "null", "Play `track` now and fill the queue with recommendations built from it."),
    m("prefetch", "playback", "id: string", "null", "Warm the stream for a track id so playing it starts instantly. Call on hover or selection."),
    m("enqueue", "playback", "track?: Track, tracks?: Track[], next?: bool", "null", "Add a track, or several in their order (an album, a playlist), after the current one (`next: true`) or at the end. Starts playing if nothing is."),
    m("cancelStart", "playback", "", "null", "Stop waiting for the track that is starting, or for its next automatic retry."),
    m("toggle", "playback", "", "null", "Play or pause."),
    m("next", "playback", "", "null", "Next track; at the end of the queue autoplay appends recommendations."),
    m("prev", "playback", "", "null", "Restart the track if more than 3 s in, otherwise the previous one."),
    m("seek", "playback", "pos: number", "null", "Seek to an absolute position in seconds."),
    m("volume", "playback", "value: number", "null", "Volume 0 to 100."),
    m("jump", "playback", "index: int, id?: string", "null", "Play the queue item at `index`. With `id` the track is found by id (nearest to `index`) even if the queue shifted since the list was read; it fails if the track left the queue."),
    m("removeQueued", "playback", "index: int, id?: string", "null", "Remove the queue item at `index` (or the track `id` nearest to it); removing the current track plays the next. Removed tracks are not re-added by autoplay."),
    m("moveQueued", "playback", "from: int, to: int, id?: string", "null", "Move a queue item (found by `id` when given); the current track stays current."),
    m("clearQueue", "playback", "keepCurrent?: bool", "null", "Empty the queue and stop. With `keepCurrent` the playing track stays and keeps playing, everything else goes and is not brought back by autoplay."),
    m("shuffle", "playback", "on: bool", "null", "Shuffle the upcoming tracks; turning it off restores the original order."),
    m("repeat", "playback", "mode: off|all|one", "null", "Repeat mode."),
    m("fx", "playback", "fx: Fx", "null", "Apply and save audio effects (EQ, bass, reverb, speed...)."),
    m("getFx", "playback", "", "Fx", "Saved audio effects."),
    m("audioDevices", "playback", "", "{devices: {name, description}[], wanted, active}", "Output devices mpv sees now (without its `auto` entry), the saved choice (`audioDevice` setting) and the device in use; `active` is `auto` while the saved one is not connected."),
    m("now", "playback", "", "{current: Track|null, playing, loading, position, duration, volume, shuffle, repeat, stream, liked}", "Snapshot of what is playing."),
    m("queue", "playback", "", "{tracks: Track[], cursor: int}", "The queue; `cursor` is the current index or -1."),
    m("like", "library", "track: Track", "bool", "Toggle a like; returns the new state."),
    m("setLiked", "library", "on: bool, track?: Track, tracks?: Track[]", "int", "Like (`on`) or unlike every given track, whatever its state was; returns how many changed. Liking takes a dislike back."),
    m("isLiked", "library", "id: string", "bool", "Whether a track id is liked."),
    m("favorites", "library", "", "Track[]", "Liked tracks, newest first; `at` is when each was liked (ms since 1970)."),
    m("dislike", "library", "track: Track", "null", "Mark a track disliked (removes its like); recommendations avoid the track and its artist."),
    m("undislike", "library", "id: string", "null", "Take back a dislike; a like it replaced is not restored."),
    m("dislikes", "library", "", "Track[]", "Disliked tracks, newest first."),
    m("history", "library", "limit?: int", "Track[]", "Recently played distinct tracks (default 100); `at` is when each was last played (ms since 1970)."),
    m("clearHistory", "library", "", "null", "Delete play history (likes are kept)."),
    m("pickPath", "library", "title: string, folder?: bool, filters?: string[], start?: string", "{status: picked|cancelled|unavailable, path?}", "Ask the desktop's file chooser portal for a file or a folder. Only the window can call this: a dialog is never opened from the socket or MCP, which get `api.window_only`. `filters` read like `CSV files (*.csv)`. `unavailable` means there is no portal or it failed, and the window then shows its own dialog."),
    m("download", "library", "track?: Track, tracks?: Track[]", "null", "Download best audio of a track, or of several, to the downloads folder in the background; a toast reports each result. A finished file is linked to its track, which then plays from the file without the network."),
    m("downloads", "library", "", "{name, path, size, modified}[]", "Finished downloads, newest first."),
    m("playlists", "library", "", "Playlist[]", "Local playlists with counts and a cover (tracks omitted)."),
    m("playlist", "library", "id: int", "Playlist|null", "One local playlist with its tracks in order; `at` on each track is when it was added (ms since 1970)."),
    m("createPlaylist", "library", "name: string", "int", "Create a playlist; returns its id."),
    m("renamePlaylist", "library", "id: int, name: string", "null", "Rename a playlist."),
    m("deletePlaylist", "library", "id: int", "Playlist|null", "Delete a playlist; returns it with its tracks (null if there was none), which `restorePlaylist` takes to undo."),
    m("restorePlaylist", "library", "playlist: Playlist", "int", "Bring back a deleted playlist from what `deletePlaylist` returned: its name and tracks in order with their `at`, under its old id when that is free. Returns the id."),
    m("addToPlaylist", "library", "id: int, track?: Track, tracks?: Track[]", "int", "Append a track, or several in their order; a track already in the playlist is skipped. Returns how many were added."),
    m("movePlaylistItem", "library", "id: int, trackId: string, to: int", "bool", "Move a track inside a playlist to index `to` (0 is first; past the end is last). False when the track is not in it."),
    m("editPlaylist", "library", "id: int, name?: string, description?: string, cover?: string", "null", "Change a playlist's name, description or cover; a field left out stays. `cover` is an image URL (https), or empty to use the first track's cover again."),
    m("playlistCoverFile", "library", "id: int, path: string", "string", "Copy a local PNG, JPEG or WebP image (at most 20 MB, its longer side reduced to 1024 pixels) into the data directory as the playlist's cover; returns its `file://` URL. The previous local cover is deleted."),
    m("importPlaylist", "library", "url?: string, csv?: string, path?: string, name?: string", "ImportResult", "Import a playlist into a new local playlist: a public playlist or album link from Spotify, Deezer, Apple Music, Yandex Music or SoundCloud (`url`), CSV text (`csv`) or a CSV file (`path`) as Exportify and TuneMyMusic write them. Each song is looked up on YouTube Music by title, artist and length, four at a time; `import` events report `{stage: reading|matching, done, total, matched}`. Returns `{playlist, name, total, matched, unmatched: ImportTrack[], limited, sourceTotal, failed, code?}`: `playlist` is the new id, null when nothing was found; `failed` counts the songs that could not be looked up because a search failed, and `code` says why; those are not in `unmatched`, which holds only songs that were looked for and not found; `ImportTrack` is `{title, artists, album, duration}`; `limited` means only part of the list could be read: Apple Music pages show at most 100 songs, and a long Spotify playlist is paged through Spotify's private web player API with the token its embed page issues, and stays at the embed's 100 songs or at the pages already read when that API refuses or changes; `sourceTotal` is the length of the whole list when the source reports it. `name` overrides the playlist name. The source's own cover, when it has one, is saved as the playlist's cover; a cover that cannot be fetched does not stop the import. A newer import cancels a running one. Fails with `import.blocked` when the service hides the playlist (private, sign-in, captcha, 403, 451): export it to CSV then."),
    m("importCancel", "library", "", "bool", "Cancel the running import; it then fails with `import.cancelled` and makes no playlist. False when none runs."),
    m("removeFromPlaylist", "library", "id: int, trackId?: string, trackIds?: string[]", "null", "Remove a track, or several, from a playlist."),
    m("savedAlbums", "library", "", "AlbumCard[]", "Saved albums, newest first."),
    m("toggleAlbum", "library", "album: AlbumCard", "bool", "Save or unsave an album; returns the new state."),
    m("savedPlaylists", "library", "", "AlbumCard[]", "YouTube playlists saved to the library, newest first (`kind` is `Playlist`, `id` the playlist id for `remotePlaylist`)."),
    m("togglePlaylist", "library", "playlist: AlbumCard", "bool", "Save or unsave a YouTube playlist; returns the new state. The tracks are not copied: the playlist is read again when opened."),
    m("followed", "library", "", "ArtistCard[]", "Followed artists, newest first. Kept in the local library only."),
    m("follow", "library", "artist: ArtistCard", "bool", "Follow or unfollow an artist; returns the new state."),
    m("library", "library", "", "{kind, id, item, at}[]", "Everything kept in the library, newest first: local playlists (`playlist`, `item` a `Playlist` without tracks), saved YouTube playlists (`remote`), albums (`album`) and followed artists (`artist`); `at` is when each was made or saved (ms since 1970)."),
    m("folders", "library", "", "{id, name, open}[]", "Playlist folders in the order they were made; `open` is whether the sidebar shows their playlists. A playlist names its folder in `folder`."),
    m("createFolder", "library", "name: string", "int", "Make a playlist folder; returns its id."),
    m("renameFolder", "library", "id: int, name: string", "null", "Rename a folder."),
    m("openFolder", "library", "id: int, open: bool", "null", "Remember whether a folder is open in the sidebar."),
    m("deleteFolder", "library", "id: int", "null", "Delete a folder; its playlists stay, outside any folder."),
    m("moveToFolder", "library", "id: int, folder?: int|null", "bool", "Put a playlist in a folder, or out of any folder when `folder` is null or left out. False when the playlist or the folder does not exist."),
    m("pins", "library", "", "{kind, id, item}[]", "What is pinned to the top of the sidebar, in the order it was pinned. `kind` is `playlist` (a local playlist, `id` its number as text), `remote` (a saved YouTube playlist), `album` or `artist`; `item` is the card as it was pinned."),
    m("pin", "library", "kind: string, item: object", "bool", "Pin or unpin an item to the sidebar; returns the new state. `item` is the card (`id` required); for a local playlist `{id}` is enough."),
    m("getSetting", "settings", "key: string", "any", "Read a setting from the table below (null if unset). An unknown key is an error."),
    m("setSetting", "settings", "key: string, value: any", "null", "Write a setting from the table below. An unknown key or a value of the wrong kind is an error; null clears it to the default."),
    m("libraryRetry", "meta", "", "bool", "Try to open the library file again when it could not be opened; true when it opened. It answers before the rest of the app has started."),
    m("libraryContinue", "meta", "", "null", "Run without saving after the library could not be opened: everything in this session is lost at quit. Only the window asks for this."),
    m("libraryFolder", "meta", "", "bool", "Show the data folder with the library file in the file manager."),
    m("raise", "meta", "", "null", "Show and raise the window. A second start of the app calls this on the running one and exits."),
    m("methods", "meta", "", "Method[]", "This table."),
    m("bugReport", "meta", "note?: string", "{name, dir}", "Freeze a bug report now: the window shot, the journal of the last five minutes, the player state, the open screen and the window size go into `reports/<date-time>/` in the data directory, and a card with a one-line field opens in the window (Enter stores the phrase, Esc keeps the report without one). Music is not interrupted. Without a window only the Rust half is saved."),
    m("bugReportAttach", "meta", "name: string, ui?: object, note?: string", "null", "Add what the window saw (`ui`) or the phrase (`note`) to the report `name`. The window calls this itself after `bugReport`."),
    m("bugReportPack", "meta", "name: string, note?: string, reveal?: bool", "{path, shown}", "Pack the report `name` into `reports/<name>.zip` and, unless `reveal` is false, show that file in the file manager. `note` is stored first, as in `bugReportAttach`. `shown` says whether a file manager was asked to show it. The archive holds the screenshot and the track list, so it shows what the person listens to; nothing is sent anywhere."),
    m("dependencies", "meta", "", "{desktop, components: {id, version, source, state}[], details}", "What Snellar runs on here and whether it really works: mpv, yt-dlp, ffmpeg, Qt, audio output, Discord, file portal, tray, window blur. `state` is ok, missing, broken, off, on, waiting or none. Reads only: nothing is installed and the desktop is not changed, here or anywhere else in this API. `details` is the same list as text, as bug reports hold it."),
    m("reports", "meta", "", "Report[]", "Saved bug reports, newest first: `{name, path, note, track, screen, files}`. `snellar ctl reports` prints them as lines."),
];

pub fn markdown() -> String {
    let mut out = String::from(
        "# Snellar API\n\nGenerated from `api::METHODS` (`snellar ctl --docs`). Every method takes one JSON object and returns one JSON value.\n\n",
    );
    out.push_str("Call: `POST /call/<method>` on the Unix socket `$XDG_RUNTIME_DIR/snellar.sock` (override: `SNELLAR_SOCK`), or `snellar ctl <method> '<json>'`.\n\n");
    out.push_str("```sh\ncurl --unix-socket \"$XDG_RUNTIME_DIR/snellar.sock\" -d '{\"q\":\"daft punk\"}' http://snellar/call/search\nsnellar ctl now\n```\n\n");
    out.push_str("Errors: status 400 with `{\"error\": \"...\", \"code\": \"...\"}` (bad arguments or a failed call), 404 for an unknown method. `error` is technical text for logs and scripts; `code` is stable and listed under Error codes below. The window shows its own translated text for the code, never `error`.\n\n");
    out.push_str("Events: `GET /events` is a Server-Sent Events stream: `snapshot` on connect, then `state`, `position` (about 5 per second while playing), `current`, `queue`, `liked`, `history` (a play was added to the history), `tools` (yt-dlp was fetched or checked), `modes`, `stream` (what plays now: `{codec, bitrate, sampleRate, gain, leveling, local}`, bitrate in kbps and 0 when unknown), `toast` (`{code, cause?, arg?, detail?}`: `code` and `cause` from Error codes below, `arg` a name to show, such as the downloaded track, `detail` technical text), `nudge` (a card to show: `{id, kind, track?, text?, cover?, timeout?, actions: [{id, primary?}]}`; `kind` `handoff` offers to continue `track`, with the actions `continue` and `ignore`; the window words the card itself; answer it with `nudgeAnswer`).\n\n");
    out.push_str("Types: `Track {id, title, artists: [{id, name}], album?: {id, name}, duration, thumb, explicit}`, other shapes are in `crates/core/src/model.rs`.\n\n");
    out.push_str("Track ids name their source: `yt:` and the YouTube video id, as in `yt:dQw4w9WgXcQ`. Every argument that takes a track id or a `Track` also accepts the bare 11-character video id for now; answers always carry the prefixed form. Album, artist and playlist ids are unchanged.\n");
    let mut group = "";
    for method in METHODS {
        if method.group != group {
            group = method.group;
            out.push_str(&format!("\n## {group}\n\n| method | args | returns | |\n|---|---|---|---|\n"));
        }
        let cell = |s: &str| if s.is_empty() { "-".to_string() } else { format!("`{}`", s.replace('|', "\\|")) };
        out.push_str(&format!("| `{}` | {} | {} | {} |\n", method.name, cell(method.args), cell(method.returns), method.doc));
    }
    out.push_str("\n## Settings\n\nGenerated from `settings::ALL` in `crates/core/src/settings.rs`. Values are JSON. `getSetting` returns null for an unset key; the default is what the app uses then. `*` matches any non-empty suffix. Scope `state` is saved by the app itself, not a preference. Section is where the Settings page shows it; none means the page does not.\n\n| key | kind | default | scope | section | |\n|---|---|---|---|---|---|\n");
    for s in settings::ALL {
        let section = s.section.map_or("-", settings::Section::name);
        out.push_str(&format!("| `{}` | {} | `{}` | {} | {section} | {} |\n", s.key, s.kind.name(), s.default, s.scope.name(), s.about));
    }
    out.push_str("\nRenamed keys still read and write: a stored old value is read through the new key until the new key is written, and `setSetting` with the old key writes the new one.\n\n| old key | new key | |\n|---|---|---|\n");
    for a in settings::ALIASES {
        out.push_str(&format!("| `{}` | `{}` | {} |\n", a.old, a.new.key, a.about));
    }
    out.push_str("\n## Error codes\n\nGenerated from `error::ALL` in `crates/core/src/error.rs`. A code never changes meaning; a new failure gets a new code. Codes marked toast only appear as the `code` of a `toast` event.\n\n| code | meaning |\n|---|---|\n");
    for (code, about) in error::ALL {
        out.push_str(&format!("| `{code}` | {about} |\n"));
    }
    out
}

pub struct Backend {
    ui: Ui,
    source: Arc<dyn Source>,
    ytm: Arc<Ytm>,
    recs: Arc<Recs>,
    downloads: Arc<Downloads>,
    library: Arc<Library>,
    resolver: Arc<Resolver>,
    data: PathBuf,
    memo: Memo,
    player: OnceCell<Arc<Player>>,
    handoff: Handoff,
    http: Http,
    imports: Imports,
    saving: AtomicBool,
}

fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T> {
    let v = args.get(key).cloned().ok_or_else(|| Code::ApiArgs.err(format!("missing argument `{key}`")))?;
    serde_json::from_value(v).with_context(|| format!("bad argument `{key}`")).context(Code::ApiArgs)
}

fn card(kind: String, item: Value) -> Result<Pin> {
    if !["playlist", "remote", "album", "artist"].contains(&kind.as_str()) {
        return Err(Code::ApiArgs.err(format!("not a collection: `{kind}`: playlist, remote, album or artist")));
    }
    let id = match &item["id"] {
        Value::String(s) if !s.is_empty() => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(Code::ApiArgs.err("`item.id` is missing")),
    };
    Ok(Pin { kind, id, item })
}

fn check_id(id: &TrackId) -> Result<()> {
    if id.video_id().is_some() {
        return Ok(());
    }
    Err(Code::ApiArgs.err(format!(
        "`{id}` is not a track id: `yt:` and a YouTube video id (11 characters: letters, digits, - and _), or the bare video id"
    )))
}

fn track_id(args: &Value, key: &str) -> Result<TrackId> {
    let id: TrackId = arg(args, key)?;
    check_id(&id)?;
    Ok(id)
}

fn registered(key: &str) -> Result<&'static settings::Setting> {
    settings::find(key)
        .ok_or_else(|| Code::ApiArgs.err(format!("unknown setting `{key}` (see the settings table in `snellar ctl --docs`)")))
}

fn track_arg(args: &Value, key: &str) -> Result<Track> {
    let track: Track = arg(args, key)?;
    check_id(&track.id)?;
    Ok(track)
}

fn one_or_many(args: &Value) -> Result<Vec<Track>> {
    if args.get("tracks").is_none() {
        return Ok(vec![track_arg(args, "track")?]);
    }
    let tracks: Vec<Track> = arg(args, "tracks")?;
    tracks.iter().try_for_each(|t| check_id(&t.id))?;
    Ok(tracks)
}

fn opt<T: DeserializeOwned + Default>(args: &Value, key: &str) -> T {
    args.get(key).cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

fn named(event: UiEvent, route: &Route) -> UiEvent {
    match event {
        UiEvent::Toast(toast) => UiEvent::Toast(Toast { cause: toast.cause.map(|c| route.blame(c)), ..toast }),
        UiEvent::PlayFailure(Some(failure)) => UiEvent::PlayFailure(Some(PlayFailure {
            code: route.blame(failure.code),
            tracks: failure.tracks.into_iter().map(|(id, code)| (id, route.blame(code))).collect(),
            ..failure
        })),
        other => other,
    }
}

fn warm_up(http: &Http) {
    let http = http.client();
    tokio::spawn(async move {
        let _ = http.head("https://music.youtube.com/").timeout(Duration::from_secs(5)).send().await;
    });
}

fn browser(b: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    b.user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0")
        .gzip(true)
        .brotli(true)
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(120))
        .connect_timeout(CONNECT_LIMIT)
}

fn http_client(route: &Route) -> Http {
    Http::new(route, browser)
}

fn import_client(route: &Route) -> Http {
    Http::new(route, |b| import::links::only_known_redirects(browser(b)))
}

pub fn failure_json(e: &anyhow::Error) -> Value {
    let mut out = json!({ "error": format!("{e:#}"), "code": Code::of(e) });
    if let Some(seconds) = Code::retry_after(e) {
        out["retryAfter"] = json!(seconds);
    }
    if e.downcast_ref::<Told>().is_some() {
        out["told"] = json!(true);
    }
    out
}

enum Asked {
    Retry,
    Memory,
}

pub struct LibraryGate {
    asked: Mutex<Option<Asked>>,
    wake: Condvar,
}

impl LibraryGate {
    pub fn new() -> Arc<LibraryGate> {
        Arc::new(LibraryGate { asked: Mutex::new(None), wake: Condvar::new() })
    }

    fn tell(&self, asked: Asked) {
        *self.asked.lock().unwrap() = Some(asked);
        self.wake.notify_all();
    }

    fn wait(&self, patience: Duration) -> Asked {
        let held = self.asked.lock().unwrap();
        let (mut held, _) = self.wake.wait_timeout_while(held, patience, |asked| asked.is_none()).unwrap();
        held.take().unwrap_or(Asked::Retry)
    }

    pub async fn answer(&self, method: &str, data: &Path) -> Option<Result<Value>> {
        match method {
            "libraryContinue" => self.tell(Asked::Memory),
            "libraryRetry" => self.tell(Asked::Retry),
            "libraryFolder" => return Some(Ok(json!(report::reveal(&data.join("library.db")).await))),
            _ => return None,
        }
        Some(Ok(Value::Null))
    }
}

fn open_library(ui: &Ui, path: &Path, gate: &LibraryGate) -> (Arc<Library>, bool) {
    loop {
        let e = match Library::open(path) {
            Ok(library) => {
                ui(UiEvent::Library(None));
                return (Arc::new(library), true);
            }
            Err(e) => e,
        };
        let code = e.code();
        journal::error("library", format!("{}: {e}", path.display()));
        let problem = LibraryProblem { code, path: path.display().to_string(), detail: e.to_string(), saving: true };
        ui(UiEvent::Library(Some(problem.clone())));
        let patience = if code == Code::LibraryLocked { LOCK_RECHECK } else { NO_RECHECK };
        if let Asked::Memory = gate.wait(patience) {
            ui(UiEvent::Library(Some(LibraryProblem { saving: false, ..problem })));
            ui(UiEvent::Toast(Toast::failed(Code::LibraryUnsaved, &anyhow::Error::new(e).context(code))));
            return (Arc::new(Library::open(Path::new(":memory:")).expect("in-memory library")), false);
        }
    }
}

impl Backend {
    pub fn build(ui: Ui, listener: Bound, data: PathBuf, cache: PathBuf, gate: Arc<LibraryGate>) -> Arc<Backend> {
        let route = Route::default();
        let (http, resolver) = std::thread::scope(|s| {
            let resolver = s.spawn(|| Arc::new(Resolver::with_route(cache.clone(), &route)));
            (http_client(&route), resolver.join().expect("resolver"))
        });
        let import_http = import_client(&route);
        Backend::assemble(ui, listener, data, cache, (http, import_http), resolver, &gate)
    }

    fn assemble(
        ui: Ui,
        listener: Bound,
        data: PathBuf,
        cache: PathBuf,
        (http, import_http): (Http, Http),
        resolver: Arc<Resolver>,
        gate: &LibraryGate,
    ) -> Arc<Backend> {
        std::fs::create_dir_all(&data).ok();
        std::fs::create_dir_all(cache.join("img")).ok();
        let report_dir = data.clone();
        let (library, saving) = open_library(&ui, &data.join("library.db"), gate);
        let told = ui.clone();
        library.on_write_failure(move |toast| told(UiEvent::Toast(toast)));
        let ytm = Arc::new(Ytm::new(http.clone()));
        if let Some(enabled) = settings::YTDLP_FALLBACK.get(&library) {
            resolver.set_ytdlp(enabled);
        }
        resolver.set_quality(Quality::parse(&settings::QUALITY.or_default::<String>(&library)));
        if let Some(region) = settings::REGION.get::<String>(&library) {
            ytm.set_region(&region);
        }
        let stored = settings::PROXY.get::<String>(&library).unwrap_or_default();
        match Route::parse(&stored) {
            Ok(proxy) => resolver.route().set(proxy),
            Err(e) => journal::warn("net", format!("the saved proxy is ignored: {e:#}")),
        }
        warm_up(&http);
        let proxy = Proxy::serve(listener, resolver.clone(), cache.join("img")).expect("local proxy");
        let playing = Arc::new(AtomicBool::new(false));
        let seen_playing = playing.clone();
        let blamed = resolver.route().clone();
        let ui: Ui = Arc::new(move |event| {
            if let UiEvent::State(state) = &event {
                seen_playing.store(state.playing, Ordering::Relaxed);
            }
            ui(named(event, &blamed));
        });
        let recs = Recs::new(http.clone(), ytm.clone(), library.clone(), cache);
        let source = YouTube::new(ytm.clone(), recs.clone(), resolver.clone(), proxy, LyricsSource::new(http));
        Arc::new(Backend {
            handoff: Handoff::new(playing, HANDOFF_OFFER),
            ui: ui.clone(),
            downloads: Downloads::new(ui, library.clone(), resolver.clone()),
            source,
            recs,
            ytm,
            library,
            resolver,
            data: report_dir,
            memo: Memo::default(),
            player: OnceCell::new(),
            http: import_http,
            imports: Imports::default(),
            saving: AtomicBool::new(saving),
        })
    }

    fn library_again(&self) -> bool {
        if self.saving.load(Ordering::Relaxed) {
            return true;
        }
        let path = self.data.join("library.db");
        match self.library.reopen(&path) {
            Ok(()) => {
                self.saving.store(true, Ordering::Relaxed);
                self.emit(UiEvent::Library(None));
                true
            }
            Err(e) => {
                let problem = LibraryProblem { code: e.code(), path: path.display().to_string(), detail: e.to_string(), saving: false };
                self.emit(UiEvent::Library(Some(problem)));
                false
            }
        }
    }

    pub fn probe(&self) -> crate::deps::Probe {
        crate::deps::Probe { library: self.library.clone(), player: self.player.get().cloned() }
    }

    pub async fn desktop_status(&self) -> Value {
        crate::desktop::status(&self.library).await
    }

    pub async fn setup_glass(&self) -> Result<crate::glass::Setup> {
        crate::desktop::setup_glass(&self.library).await
    }

    pub async fn undo_glass(&self) -> Result<()> {
        crate::desktop::undo_glass(&self.library).await
    }

    pub fn ytdlp_due(&self) -> bool {
        crate::ytdlp::due(&self.library)
    }

    pub async fn keep_ytdlp(&self, asked: bool) -> Result<Option<crate::ytdlp::Outcome>> {
        let ui = self.ui.clone();
        let tell = move |percent: u8| ui(UiEvent::Toast(Toast::new(Code::YtdlpFetching).arg(percent.to_string())));
        crate::ytdlp::keep(&self.library, self.resolver.route(), asked, &tell).await
    }

    pub fn started_player(&self) -> Option<&Arc<Player>> {
        self.player.get()
    }

    pub async fn player(&self) -> Result<&Arc<Player>> {
        self.player
            .get_or_try_init(|| {
                Player::start(self.ui.clone(), self.source.clone(), mpv::launch(self.resolver.route().clone()), self.library.clone())
            })
            .await
    }

    #[cfg(test)]
    pub(crate) fn offline_at(dir: &std::path::Path, ui: Ui, gate: &LibraryGate) -> Arc<Backend> {
        let dead = reqwest::Proxy::all("http://127.0.0.1:1").unwrap();
        let http = Http::from(reqwest::Client::builder().proxy(dead).build().unwrap());
        let resolver = Arc::new(Resolver::new(dir.join("cache")));
        let listener = snellar_youtube::proxy::bind().unwrap();
        Backend::assemble(ui, listener, dir.join("data"), dir.join("cache"), (http.clone(), http), resolver, gate)
    }

    #[cfg(test)]
    pub(crate) fn offline(dir: &std::path::Path, ui: Ui) -> Arc<Backend> {
        let dead = reqwest::Proxy::all("http://127.0.0.1:1").unwrap();
        let http = Http::from(reqwest::Client::builder().proxy(dead).build().unwrap());
        let resolver = Arc::new(Resolver::new(dir.join("cache")));
        let listener = snellar_youtube::proxy::bind().unwrap();
        let backend =
            Backend::assemble(ui, listener, dir.join("data"), dir.join("cache"), (http.clone(), http), resolver, &LibraryGate::new());
        backend.library.set_setting("downloadDir", &serde_json::to_string(&dir.join("downloads")).unwrap());
        backend
    }

    pub(crate) fn emit(&self, event: UiEvent) {
        (self.ui)(event);
    }

    pub(crate) async fn track(&self, id: &TrackId) -> Result<Track> {
        check_id(id)?;
        if let Some(track) = self.library.track(id) {
            return Ok(track);
        }
        self.source.track(id).await
    }

    async fn artist(&self, id: &str) -> Result<Artist> {
        let page = self.memo.get(format!("artist:{id}"), PAGE_TTL, async { Ok(json!(self.source.artist(id).await?)) }).await?;
        serde_json::from_value(page).context(Code::Internal)
    }

    async fn artist_songs(&self, id: &str) -> Result<Vec<Track>> {
        let artist = self.artist(id).await?;
        if artist.songs.is_empty() {
            return Ok(artist.top);
        }
        let songs = self.source.playlist(&artist.songs).await?.tracks;
        Ok(if songs.is_empty() { artist.top } else { songs })
    }

    async fn now(&self) -> Result<Value> {
        let mut now = self.player().await?.snapshot();
        let liked = now["current"]["id"].as_str().is_some_and(|id| self.library.is_liked(&TrackId::parse(id)));
        now["liked"] = json!(liked);
        Ok(now)
    }

    pub async fn dispatch(&self, method: &str, a: &Value) -> Result<Value> {
        let failed_reads = self.library.failed_reads();
        let answer = self.answer(method, a).await;
        let answer = match answer {
            Ok(_) if LIBRARY_READS.contains(&method) && self.library.failed_reads() != failed_reads => {
                let code = self.library.read_failure();
                Err(code.err(format!("reading {method} from the library failed")))
            }
            other => other,
        };
        answer.map_err(|e| {
            let code = Code::of(&e);
            let named = self.resolver.route().blame(code);
            if named == code { e } else { e.context(named) }
        })
    }

    fn home_page(&self, shelves: Result<Vec<Shelf>>) -> Result<Value> {
        let recent = self.library.recent(RECENT_CARDS);
        let (shelves, problem) = match shelves {
            Ok(shelves) => (shelves, self.recs.home_problem()),
            Err(e) if !recent.is_empty() => (Vec::new(), Some((Code::of(&e), Code::retry_after(&e)))),
            Err(e) => return Err(e),
        };
        let mut page = json!({ "shelves": shelves, "recent": recent });
        if let Some((code, after)) = problem {
            page["problem"] = json!(self.resolver.route().blame(code));
            if let Some(after) = after {
                page["retryAfter"] = json!(after);
            }
        }
        Ok(page)
    }

    async fn answer(&self, method: &str, a: &Value) -> Result<Value> {
        let lib = &self.library;
        Ok(match method {
            "search" => {
                let (q, filter) = (arg::<String>(a, "q")?, opt::<String>(a, "filter"));
                let key = format!("search:{filter}:{q}");
                self.memo.get(key, SEARCH_TTL, async { Ok(json!(self.source.search(&q, Filter::parse(&filter)).await?)) }).await?
            }
            "suggest" => {
                let q = arg::<String>(a, "q")?;
                self.memo.get(format!("suggest:{q}"), SUGGEST_TTL, async { Ok(json!(self.source.suggest(&q).await?)) }).await?
            }
            "home" => self.home_page(self.recs.home().await)?,
            "homeRefresh" => self.home_page(self.recs.refresh_home().await)?,
            "libraryRetry" => json!(self.library_again()),
            "libraryContinue" => Value::Null,
            "libraryFolder" => json!(report::reveal(&self.data.join("library.db")).await),
            "cancelStart" => {
                self.player().await?.cancel_start().await;
                Value::Null
            }
            "album" => {
                let id = arg::<String>(a, "id")?;
                self.memo.get(format!("album:{id}"), PAGE_TTL, async { Ok(json!(self.source.album(&id).await?)) }).await?
            }
            "artist" => json!(self.artist(&arg::<String>(a, "id")?).await?),
            "discography" => {
                let id = arg::<String>(a, "id")?;
                let fetch = async { Ok(json!(self.source.discography(&self.artist(&id).await?).await?)) };
                self.memo.get(format!("discography:{id}"), PAGE_TTL, fetch).await?
            }
            "artistSongs" => {
                let id = arg::<String>(a, "id")?;
                let fetch = async { Ok(json!(self.artist_songs(&id).await?)) };
                self.memo.get(format!("artistSongs:{id}"), PAGE_TTL, fetch).await?
            }
            "artistMix" => {
                let artist = self.artist(&arg::<String>(a, "id")?).await?;
                json!(self.source.artist_mix(&artist, arg::<Mix>(a, "kind")?).await?)
            }
            "remotePlaylist" => json!(self.source.playlist(&arg::<String>(a, "id")?).await?),
            "lyrics" => json!(self.source.lyrics(&track_arg(a, "track")?).await?),
            "track" => json!(self.track(&track_id(a, "id")?).await?),
            "link" => json!(self.source.link(arg::<Kind>(a, "kind")?, &arg::<String>(a, "id")?)),
            "nudgeAnswer" => {
                let (id, action) = (arg::<String>(a, "id")?, arg::<String>(a, "action")?);
                if let Some((track, position)) = self.handoff.respond(&id, &action).await? {
                    self.player().await?.radio_from(track, position).await?;
                }
                Value::Null
            }

            "play" => {
                let tracks: Vec<Track> = arg(a, "tracks")?;
                tracks.iter().try_for_each(|t| check_id(&t.id))?;
                let context = match a.get("context").filter(|c| !c.is_null()) {
                    Some(c) => Some(card(arg(c, "kind")?, arg(c, "item")?)?),
                    None => None,
                };
                self.player().await?.play(tracks, opt(a, "index"), context).await?;
                Value::Null
            }
            "wave" => {
                let knobs =
                    if a.as_object().is_some_and(|o| !o.is_empty()) { serde_json::from_value(a.clone())? } else { self.recs.knobs() };
                json!(self.recs.wave(&knobs, &[]).await?)
            }
            "dislike" => {
                lib.dislike(&track_arg(a, "track")?);
                Value::Null
            }
            "undislike" => {
                lib.undislike(&track_id(a, "id")?);
                Value::Null
            }
            "dislikes" => json!(lib.disliked()),
            "radio" => {
                self.player().await?.radio(track_arg(a, "track")?).await?;
                Value::Null
            }
            "prefetch" => {
                let id = track_id(a, "id")?;
                let player = self.player().await?;
                player.prefetch(&id);
                if let Some(track) = lib.track(&id).filter(|_| player.is_near(&id)) {
                    self.recs.warm(&track);
                }
                Value::Null
            }
            "enqueue" => {
                let tracks = one_or_many(a)?;
                self.player().await?.enqueue(tracks, opt(a, "next")).await?;
                Value::Null
            }
            "toggle" => self.player().await?.toggle().await.map(|_| Value::Null)?,
            "next" => self.player().await?.next().await.map(|_| Value::Null)?,
            "prev" => self.player().await?.prev().await.map(|_| Value::Null)?,
            "seek" => self.player().await?.seek(arg(a, "pos")?).await.map(|_| Value::Null)?,
            "volume" => self.player().await?.set_volume(arg(a, "value")?).await.map(|_| Value::Null)?,
            "jump" => self.player().await?.jump(arg(a, "index")?, opt::<Option<TrackId>>(a, "id").as_ref()).await.map(|_| Value::Null)?,
            "removeQueued" => {
                self.player().await?.remove(arg(a, "index")?, opt::<Option<TrackId>>(a, "id").as_ref()).await?;
                Value::Null
            }
            "moveQueued" => {
                self.player().await?.move_item(arg(a, "from")?, arg(a, "to")?, opt::<Option<TrackId>>(a, "id").as_ref()).await?;
                Value::Null
            }
            "clearQueue" => {
                let player = self.player().await?;
                if !(opt::<bool>(a, "keepCurrent") && player.clear_others()) {
                    player.clear().await;
                }
                Value::Null
            }
            "shuffle" => {
                self.player().await?.set_shuffle(arg(a, "on")?).await;
                Value::Null
            }
            "repeat" => {
                self.player().await?.set_repeat(arg::<Repeat>(a, "mode")?).await;
                Value::Null
            }
            "fx" => {
                let fx = fx::clamp(&arg::<Fx>(a, "fx")?);
                self.player().await?.set_fx(fx.clone()).await?;
                settings::FX.set(lib, &fx);
                Value::Null
            }
            "audioDevices" => self.player().await?.devices().await?,
            "getFx" => settings::FX.get::<Value>(lib).unwrap_or_else(|| json!(Fx::default())),

            "like" => {
                let t = track_arg(a, "track")?;
                let liked = lib.toggle_like(&t);
                if self.player.get().and_then(|p| p.current()).is_some_and(|c| c.id == t.id) {
                    (self.ui)(UiEvent::Liked(liked));
                }
                json!(liked)
            }
            "setLiked" => {
                let on: bool = arg(a, "on")?;
                let tracks = one_or_many(a)?;
                let changed = lib.set_liked(&tracks, on);
                if let Some(c) = self.player.get().and_then(|p| p.current()).filter(|c| tracks.iter().any(|t| t.id == c.id)) {
                    (self.ui)(UiEvent::Liked(lib.is_liked(&c.id)));
                }
                json!(changed)
            }
            "isLiked" => json!(lib.is_liked(&arg(a, "id")?)),
            "favorites" => json!(lib.liked()),
            "clearHistory" => {
                lib.clear_history();
                self.recs.forget();
                Value::Null
            }
            "download" => {
                for t in one_or_many(a)? {
                    self.downloads.start(t);
                }
                Value::Null
            }
            "downloads" => json!(self.downloads.list()),
            PICK_PATH => {
                arg::<String>(a, "title")?;
                return Err(Code::ApiWindowOnly.err("pickPath opens a dialog, so only the window can call it"));
            }
            "history" => json!(lib.played(match opt::<usize>(a, "limit") {
                0 => 100,
                n => n,
            })),
            "playlists" => json!(lib.playlists()),
            "playlist" => json!(lib.playlist(arg(a, "id")?)),
            "createPlaylist" => json!(lib.create_playlist(&arg::<String>(a, "name")?)?),
            "renamePlaylist" => {
                lib.rename_playlist(arg(a, "id")?, &arg::<String>(a, "name")?);
                Value::Null
            }
            "deletePlaylist" => json!(lib.delete_playlist(arg(a, "id")?)?),
            "restorePlaylist" => {
                let list: Playlist = arg(a, "playlist")?;
                list.tracks.iter().try_for_each(|t| check_id(&t.item.id))?;
                json!(lib.restore_playlist(&list)?)
            }
            "addToPlaylist" => {
                let tracks = one_or_many(a)?;
                json!(lib.add_to_playlist(arg(a, "id")?, &tracks)?)
            }
            "movePlaylistItem" => json!(lib.move_in_playlist(arg(a, "id")?, &arg(a, "trackId")?, arg(a, "to")?)),
            "editPlaylist" => {
                let id = arg(a, "id")?;
                let old = lib.playlist(id).ok_or_else(|| Code::ApiNotFound.err(format!("no playlist {id}")))?;
                if let Some(name) = opt::<Option<String>>(a, "name").filter(|n| !n.trim().is_empty()) {
                    lib.rename_playlist(id, name.trim());
                }
                if let Some(description) = opt::<Option<String>>(a, "description") {
                    lib.describe_playlist(id, description.trim());
                }
                if let Some(cover) = opt::<Option<String>>(a, "cover") {
                    if !cover.is_empty() && !cover.starts_with("https://") {
                        return Err(Code::ApiArgs.err("`cover` is an https URL or empty; a local image goes through `playlistCoverFile`"));
                    }
                    lib.set_playlist_cover(id, &cover);
                    covers::drop_local(&self.data.join("covers"), &old.cover);
                }
                Value::Null
            }
            "playlistCoverFile" => {
                let id = arg(a, "id")?;
                let old = lib.playlist(id).ok_or_else(|| Code::ApiNotFound.err(format!("no playlist {id}")))?;
                let dir = self.data.join("covers");
                let cover = covers::from_file(&dir, id, Path::new(&arg::<String>(a, "path")?)).await?;
                lib.set_playlist_cover(id, &cover);
                covers::drop_local(&dir, &old.cover);
                json!(cover)
            }
            "importPlaylist" => {
                let input = match (opt::<String>(a, "url"), opt::<String>(a, "csv"), opt::<String>(a, "path")) {
                    (url, _, _) if !url.trim().is_empty() => Input::Link(url),
                    (_, csv, _) if !csv.is_empty() => Input::Csv { text: csv, name: String::new() },
                    (_, _, path) if !path.is_empty() => Input::file(&path).await?,
                    _ => return Err(Code::ApiArgs.err("give `url`, `csv` or `path`")),
                };
                let job = import::Job {
                    covers: Some(self.data.join("covers")),
                    ..import::Job::new(self.source.clone(), self.library.clone(), self.http.client(), self.ui.clone())
                };
                json!(self.imports.run(job, input, opt::<Option<String>>(a, "name")).await?)
            }
            "importCancel" => json!(self.imports.cancel()),
            "removeFromPlaylist" => {
                let id = arg(a, "id")?;
                let ids: Vec<TrackId> = if a.get("trackIds").is_some() { arg(a, "trackIds")? } else { vec![arg(a, "trackId")?] };
                for track in &ids {
                    lib.remove_from_playlist(id, track);
                }
                Value::Null
            }
            "savedAlbums" => json!(lib.saved_albums()),
            "toggleAlbum" => json!(lib.toggle_album(&arg(a, "album")?)),
            "savedPlaylists" => json!(lib.saved_playlists()),
            "togglePlaylist" => json!(lib.toggle_playlist(&arg(a, "playlist")?)),
            "followed" => json!(lib.followed()),
            "follow" => json!(lib.toggle_follow(&arg(a, "artist")?)),
            "folders" => json!(lib.folders()),
            "createFolder" => json!(lib.create_folder(arg::<String>(a, "name")?.trim())?),
            "renameFolder" => {
                lib.rename_folder(arg(a, "id")?, arg::<String>(a, "name")?.trim());
                Value::Null
            }
            "openFolder" => {
                lib.open_folder(arg(a, "id")?, arg(a, "open")?);
                Value::Null
            }
            "deleteFolder" => {
                lib.delete_folder(arg(a, "id")?);
                Value::Null
            }
            "moveToFolder" => json!(lib.move_to_folder(arg(a, "id")?, opt(a, "folder"))),
            "library" => json!(lib.saved()),
            "pins" => json!(lib.pins()),
            "pin" => {
                json!(lib.toggle_pin(&card(arg(a, "kind")?, arg(a, "item")?)?))
            }
            "getSetting" => {
                let key = arg::<String>(a, "key")?;
                registered(&key)?;
                settings::stored(lib, &key).unwrap_or(Value::Null)
            }
            "setSetting" => {
                let asked = arg::<String>(a, "key")?;
                let setting = registered(&asked)?;
                let given = a.get("value").cloned().unwrap_or(Value::Null);
                let (key, value) = match settings::alias(&asked) {
                    Some(alias) if !given.is_null() => (setting.key, (alias.value)(&given)),
                    Some(_) => (setting.key, given),
                    None => (asked.as_str(), given),
                };
                if !setting.accepts(&value) {
                    return Err(Code::ApiArgs.err(format!("setting `{key}` takes a {} value", setting.kind.name())));
                }
                if let Some(quality) = value.as_str().filter(|q| key == "quality" && !["auto", "high", "dataSaver"].contains(q)) {
                    return Err(Code::ApiArgs.err(format!("quality `{quality}`: auto, high or dataSaver")));
                }
                let proxy = if key == "proxy" { Some(Route::parse(value.as_str().unwrap_or_default())?) } else { None };
                lib.set_setting(key, &value.to_string());
                if let Some(proxy) = proxy {
                    self.resolver.route().set(proxy);
                }
                if key == "ytdlpFallback" {
                    self.resolver.set_ytdlp(value.as_bool().unwrap_or(true));
                }
                if key == "audioDevice" {
                    self.player().await?.set_device(value.as_str().unwrap_or("auto")).await?;
                }
                if key == "loudness" {
                    self.player().await?.set_loudness(value.as_bool().unwrap_or(true)).await?;
                }
                if key == "quality" {
                    self.resolver.set_quality(Quality::parse(value.as_str().unwrap_or_default()));
                }
                if key == "region" {
                    self.ytm.set_region(value.as_str().unwrap_or_default());
                }
                Value::Null
            }
            "now" => self.now().await?,
            "queue" => self.player().await?.queue(),
            "raise" => {
                self.emit(UiEvent::TrayAction("raise".into()));
                Value::Null
            }
            "methods" => json!(METHODS),
            "bugReport" => {
                let player = self.player.get().map_or(Value::Null, |p| json!({ "now": p.snapshot(), "queue": p.queue() }));
                let (name, dir) = report::create(&self.data, opt::<Option<String>>(a, "note").as_deref(), player)?;
                self.emit(UiEvent::Capture(dir.display().to_string()));
                let (probe, data, named) = (self.probe(), self.data.clone(), name.clone());
                tokio::spawn(async move {
                    let found = probe.report().await;
                    if let Err(e) = report::components(&data, &named, found["details"].as_str().unwrap_or_default()) {
                        journal::warn("report", format!("{e:#}"));
                    }
                });
                json!({ "name": name, "dir": dir })
            }
            "bugReportAttach" => {
                report::attach(&self.data, &arg::<String>(a, "name")?, a.get("ui"), opt::<Option<String>>(a, "note").as_deref())?;
                Value::Null
            }
            "bugReportPack" => {
                let (data, name, note) = (self.data.clone(), arg::<String>(a, "name")?, opt::<Option<String>>(a, "note"));
                let path = tokio::task::spawn_blocking(move || report::pack(&data, &name, note.as_deref()))
                    .await
                    .context("the packing task died")??;
                let shown = opt::<Option<bool>>(a, "reveal").unwrap_or(true) && report::reveal(&path).await;
                json!({ "path": path, "shown": shown })
            }
            "reports" => report::list(&self.data),
            "dependencies" => self.probe().report().await,
            _ => return Err(Code::ApiMethod.err(format!("unknown method `{method}`"))),
        })
    }
}

impl handoff::Host for Backend {
    fn handoff(&self) -> &Handoff {
        &self.handoff
    }

    fn handoff_enabled(&self) -> bool {
        settings::HANDOFF.or_default(&self.library)
    }

    fn emit(&self, event: UiEvent) {
        Backend::emit(self, event)
    }

    async fn track(&self, id: &TrackId) -> Result<Track> {
        Backend::track(self, id).await
    }

    async fn songs(&self, query: &str) -> Result<Vec<Track>> {
        Ok(self.source.search(query, Filter::Songs).await?.tracks)
    }
}

#[cfg(test)]
mod doc_current;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    enum Expect {
        Ok,
        Reachable,
    }

    fn id(name: &str) -> String {
        format!("{name:_<11}")
    }

    fn track(name: &str) -> Value {
        json!({ "id": id(name), "title": format!("title {name}"), "artists": [{ "id": "a", "name": "Artist" }], "duration": 200.0, "thumb": "t" })
    }

    async fn backend(dir: &std::path::Path) -> Arc<Backend> {
        Backend::offline(dir, Arc::new(|_| {}))
    }

    fn cases() -> Vec<(&'static str, Value, Expect)> {
        let t = || track("t1");
        vec![
            ("search", json!({ "q": "x" }), Expect::Reachable),
            ("suggest", json!({ "q": "x" }), Expect::Reachable),
            ("home", json!({}), Expect::Reachable),
            ("homeRefresh", json!({}), Expect::Reachable),
            ("album", json!({ "id": "a" }), Expect::Reachable),
            ("artist", json!({ "id": "a" }), Expect::Reachable),
            ("discography", json!({ "id": "a" }), Expect::Reachable),
            ("artistSongs", json!({ "id": "a" }), Expect::Reachable),
            ("artistMix", json!({ "id": "a", "kind": "radio" }), Expect::Reachable),
            ("remotePlaylist", json!({ "id": "a" }), Expect::Reachable),
            ("lyrics", json!({ "track": t() }), Expect::Reachable),
            ("nudgeAnswer", json!({ "id": "0", "action": "ignore" }), Expect::Ok),
            ("track", json!({ "id": id("t1") }), Expect::Reachable),
            ("link", json!({ "kind": "album", "id": "MPREb_x" }), Expect::Ok),
            ("play", json!({ "tracks": [t()], "index": 0 }), Expect::Ok),
            ("radio", json!({ "track": t() }), Expect::Reachable),
            ("wave", json!({ "limit": 5 }), Expect::Reachable),
            ("dislike", json!({ "track": t() }), Expect::Ok),
            ("undislike", json!({ "id": id("t1") }), Expect::Ok),
            ("dislikes", json!({}), Expect::Ok),
            ("prefetch", json!({ "id": id("t1") }), Expect::Ok),
            ("enqueue", json!({ "track": track("t2"), "next": true }), Expect::Ok),
            ("cancelStart", json!({}), Expect::Reachable),
            ("libraryRetry", json!({}), Expect::Ok),
            ("libraryContinue", json!({}), Expect::Ok),
            ("libraryFolder", json!({}), Expect::Reachable),
            ("toggle", json!({}), Expect::Reachable),
            ("next", json!({}), Expect::Reachable),
            ("prev", json!({}), Expect::Reachable),
            ("seek", json!({ "pos": 1.0 }), Expect::Reachable),
            ("volume", json!({ "value": 40 }), Expect::Ok),
            ("jump", json!({ "index": 0 }), Expect::Reachable),
            ("removeQueued", json!({ "index": 0 }), Expect::Ok),
            ("moveQueued", json!({ "from": 0, "to": 1 }), Expect::Reachable),
            ("clearQueue", json!({}), Expect::Ok),
            ("shuffle", json!({ "on": true }), Expect::Ok),
            ("repeat", json!({ "mode": "all" }), Expect::Ok),
            ("fx", json!({ "fx": Fx::default() }), Expect::Ok),
            ("getFx", json!({}), Expect::Ok),
            ("audioDevices", json!({}), Expect::Ok),
            ("now", json!({}), Expect::Ok),
            ("raise", json!({}), Expect::Ok),
            ("queue", json!({}), Expect::Ok),
            ("like", json!({ "track": t() }), Expect::Ok),
            ("setLiked", json!({ "tracks": [t(), track("t2")], "on": true }), Expect::Ok),
            ("isLiked", json!({ "id": "t1" }), Expect::Ok),
            ("favorites", json!({}), Expect::Ok),
            ("history", json!({ "limit": 5 }), Expect::Ok),
            ("clearHistory", json!({}), Expect::Ok),
            ("download", json!({ "track": t() }), Expect::Ok),
            ("downloads", json!({}), Expect::Ok),
            ("pickPath", json!({ "title": "x" }), Expect::Reachable),
            ("playlists", json!({}), Expect::Ok),
            ("playlist", json!({ "id": 1 }), Expect::Ok),
            ("createPlaylist", json!({ "name": "mix" }), Expect::Ok),
            ("renamePlaylist", json!({ "id": 1, "name": "mix2" }), Expect::Ok),
            ("deletePlaylist", json!({ "id": 99 }), Expect::Ok),
            ("restorePlaylist", json!({ "playlist": { "id": 98, "name": "back", "tracks": [t()] } }), Expect::Ok),
            ("addToPlaylist", json!({ "id": 1, "track": t() }), Expect::Ok),
            ("movePlaylistItem", json!({ "id": 1, "trackId": "t1", "to": 0 }), Expect::Ok),
            ("editPlaylist", json!({ "id": 1, "description": "d" }), Expect::Ok),
            ("playlistCoverFile", json!({ "id": 1, "path": "/nonexistent.png" }), Expect::Reachable),
            ("removeFromPlaylist", json!({ "id": 1, "trackId": "t1" }), Expect::Ok),
            ("importPlaylist", json!({ "csv": "Title,Artist\nCreep,Radiohead" }), Expect::Reachable),
            ("importCancel", json!({}), Expect::Ok),
            ("savedAlbums", json!({}), Expect::Ok),
            ("toggleAlbum", json!({ "album": { "id": "al", "title": "Album" } }), Expect::Ok),
            ("savedPlaylists", json!({}), Expect::Ok),
            ("togglePlaylist", json!({ "playlist": { "id": "PLx", "title": "Mix" } }), Expect::Ok),
            ("followed", json!({}), Expect::Ok),
            ("follow", json!({ "artist": { "id": "UCx", "name": "X" } }), Expect::Ok),
            ("library", json!({}), Expect::Ok),
            ("folders", json!({}), Expect::Ok),
            ("createFolder", json!({ "name": "f" }), Expect::Ok),
            ("renameFolder", json!({ "id": 1, "name": "g" }), Expect::Ok),
            ("openFolder", json!({ "id": 1, "open": false }), Expect::Ok),
            ("moveToFolder", json!({ "id": 1, "folder": 1 }), Expect::Ok),
            ("deleteFolder", json!({ "id": 1 }), Expect::Ok),
            ("pins", json!({}), Expect::Ok),
            ("pin", json!({ "kind": "album", "item": { "id": "MPREb_x", "title": "X" } }), Expect::Ok),
            ("getSetting", json!({ "key": "volume" }), Expect::Ok),
            ("setSetting", json!({ "key": "recsExplore", "value": 0.5 }), Expect::Ok),
            ("methods", json!({}), Expect::Ok),
            ("bugReport", json!({ "note": "x" }), Expect::Ok),
            ("bugReportAttach", json!({ "name": "2026-01-01_00-00-00" }), Expect::Reachable),
            ("bugReportPack", json!({ "name": "2026-01-01_00-00-00" }), Expect::Reachable),
            ("reports", json!({}), Expect::Ok),
            ("dependencies", json!({}), Expect::Ok),
        ]
    }

    #[test]
    fn every_method_has_a_case() {
        let covered: HashSet<_> = cases().iter().map(|c| c.0).collect();
        let documented: HashSet<_> = METHODS.iter().map(|m| m.name).collect();
        assert_eq!(covered, documented, "METHODS and the test table must list the same methods");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "starts mpv and resolves streams, needs network"]
    async fn every_method_is_dispatched() {
        let dir = std::env::temp_dir().join(format!("snellar-api-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        b.dispatch("setSetting", &json!({ "key": "downloadDir", "value": dir.join("dl") })).await.unwrap();

        for (name, args, expect) in &cases() {
            let result = b.dispatch(name, args).await;
            match (expect, &result) {
                (Expect::Ok, Err(e)) => panic!("{name} failed: {e:#}"),
                (Expect::Reachable, Err(e)) => {
                    let message = format!("{e:#}");
                    assert!(
                        !message.contains("missing argument") && !message.contains("bad argument") && !message.contains("unknown method"),
                        "{name}: {message}"
                    );
                }
                _ => {}
            }
        }
    }

    fn library_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("snellar-api-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        dir
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_library_that_cannot_open_waits_for_the_person_and_keeps_the_file() {
        let dir = library_dir("damaged");
        let file = dir.join("data/library.db");
        let bytes = vec![7u8; 8192];
        std::fs::write(&file, &bytes).unwrap();
        let shown = Arc::new(Mutex::new(Vec::new()));
        let sink = shown.clone();
        let ui: Ui = Arc::new(move |e| {
            if let UiEvent::Library(problem) = e {
                sink.lock().unwrap().push(problem);
            }
        });
        let gate = LibraryGate::new();
        let (at, opening) = (dir.clone(), gate.clone());
        let started = tokio::task::spawn_blocking(move || Backend::offline_at(&at, ui, &opening));
        while shown.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!started.is_finished(), "nothing runs until the person chose");
        let problem = shown.lock().unwrap()[0].clone().unwrap();
        assert_eq!(problem.code, Code::LibraryCorrupt);
        assert!(problem.saving && problem.path.ends_with("library.db"));
        gate.answer("libraryRetry", &dir).await.unwrap().unwrap();
        while shown.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!started.is_finished(), "Try again on a damaged file stays there");
        gate.answer("libraryContinue", &dir).await.unwrap().unwrap();
        let b = started.await.unwrap();
        assert!(!shown.lock().unwrap().last().cloned().flatten().unwrap().saving, "the session says it is not saving");
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "the file is untouched");
        assert!(!b.library_again(), "still damaged");
        std::fs::remove_file(&file).unwrap();
        assert!(b.library_again(), "opens once it is fixed");
        assert_eq!(shown.lock().unwrap().last().cloned().flatten(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn home_with_only_what_was_played_still_opens_and_says_why() {
        let dir = library_dir("recent");
        let b = backend(&dir).await;
        assert!(Code::of(&b.dispatch("homeRefresh", &json!({})).await.unwrap_err()).network(), "nothing to show: the failure");
        b.library.push_played(&serde_json::from_value(track("t1")).unwrap(), None);
        let page = b.dispatch("homeRefresh", &json!({})).await.unwrap();
        assert!(!page["recent"].as_array().unwrap().is_empty());
        assert!(page["problem"].as_str().unwrap().contains('.'), "{page}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dialog_is_never_opened_from_a_remote_call() {
        let dir = std::env::temp_dir().join(format!("snellar-api-pick-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        let err = b.dispatch(PICK_PATH, &json!({ "title": "x" })).await.unwrap_err();
        assert_eq!(Code::of(&err), Code::ApiWindowOnly);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_arguments_are_named() {
        let dir = std::env::temp_dir().join(format!("snellar-api-args-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        for method in METHODS.iter().filter(|m| m.args.split(", ").any(|a| !a.is_empty() && !a.contains('?'))) {
            let err = b.dispatch(method.name, &json!({})).await.unwrap_err();
            assert!(format!("{err:#}").contains("missing argument"), "{}: {err:#}", method.name);
        }
        let err = b.dispatch("nope", &json!({})).await.unwrap_err();
        assert!(format!("{err:#}").contains("unknown method"));
        let err = b.dispatch("seek", &json!({ "pos": "soon" })).await.unwrap_err();
        assert!(format!("{err:#}").contains("bad argument"));
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "starts mpv and resolves streams, needs network"]
    async fn queue_flow_with_playback() {
        let dir = std::env::temp_dir().join(format!("snellar-api-flow-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        let ids = |q: &Value| q["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();

        b.dispatch("play", &json!({ "tracks": [track("a"), track("b"), track("c")], "index": 1 })).await.unwrap();
        let queue = b.dispatch("queue", &json!({})).await.unwrap();
        assert_eq!((ids(&queue), queue["cursor"].as_i64()), (vec![id("a"), id("b"), id("c")], Some(1)));
        assert_eq!(b.dispatch("now", &json!({})).await.unwrap()["current"]["id"], json!(id("b")));

        b.dispatch("enqueue", &json!({ "track": track("x"), "next": true })).await.unwrap();
        b.dispatch("moveQueued", &json!({ "from": 3, "to": 0 })).await.unwrap();
        b.dispatch("removeQueued", &json!({ "index": 1 })).await.unwrap();
        let queue = b.dispatch("queue", &json!({})).await.unwrap();
        assert_eq!((ids(&queue), queue["cursor"].as_i64()), (vec![id("c"), id("b"), id("x")], Some(1)));

        b.dispatch("shuffle", &json!({ "on": true })).await.unwrap();
        b.dispatch("repeat", &json!({ "mode": "one" })).await.unwrap();
        let now = b.dispatch("now", &json!({})).await.unwrap();
        assert_eq!((now["shuffle"].clone(), now["repeat"].clone()), (json!(true), json!("one")));

        b.dispatch("clearQueue", &json!({})).await.unwrap();
        assert!(ids(&b.dispatch("queue", &json!({})).await.unwrap()).is_empty());
        assert!(b.dispatch("now", &json!({})).await.unwrap()["current"].is_null());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn library_and_settings_flow() {
        let dir = std::env::temp_dir().join(format!("snellar-api-lib-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        let real = track("a");
        assert_eq!(b.dispatch("like", &json!({ "track": real })).await.unwrap(), json!(true));
        assert_eq!(b.dispatch("isLiked", &json!({ "id": id("a") })).await.unwrap(), json!(true));
        assert_eq!(b.dispatch("favorites", &json!({})).await.unwrap().as_array().unwrap().len(), 1);
        assert_eq!(b.dispatch("track", &json!({ "id": id("a") })).await.unwrap()["title"], "title a");
        let list = b.dispatch("createPlaylist", &json!({ "name": "road" })).await.unwrap();
        b.dispatch("addToPlaylist", &json!({ "id": list, "track": real })).await.unwrap();
        assert_eq!(b.dispatch("playlist", &json!({ "id": list })).await.unwrap()["count"], 1);
        b.dispatch("setSetting", &json!({ "key": "wave", "value": { "mood": "calm" } })).await.unwrap();
        assert_eq!(b.dispatch("getSetting", &json!({ "key": "wave" })).await.unwrap()["mood"], "calm");
        assert_eq!(b.dispatch("methods", &json!({})).await.unwrap().as_array().unwrap().len(), METHODS.len());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_refuses_a_context_that_is_not_a_collection() {
        let dir = std::env::temp_dir().join(format!("snellar-api-context-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        for context in
            [json!({ "kind": "wave", "item": { "id": "x" } }), json!({ "kind": "album", "item": {} }), json!({ "kind": "album" })]
        {
            let refused = b.dispatch("play", &json!({ "tracks": [track("a")], "context": context })).await.unwrap_err();
            assert_eq!(Code::of(&refused), Code::ApiArgs, "{context}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn clearing_history_removes_the_listens_and_the_shelves_built_from_them() {
        let dir = std::env::temp_dir().join(format!("snellar-api-clear-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        let t: Track = serde_json::from_value(track("a")).unwrap();
        b.library.push_history(&t);
        b.library.record_listen(&t.id, 120.0, false);
        let home = dir.join("cache/home.json");
        std::fs::write(&home, "{}").unwrap();
        b.dispatch("clearHistory", &json!({})).await.unwrap();
        assert!(b.library.history(10).is_empty() && b.library.listens(10).is_empty());
        assert!(!home.exists(), "the cached home shelves were built from the history");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_write_is_told_in_a_toast_and_an_id_write_answers_with_its_code() {
        let dir = std::env::temp_dir().join(format!("snellar-api-refused-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let toasts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = toasts.clone();
        let b = Backend::offline(
            &dir,
            Arc::new(move |e| {
                if let UiEvent::Toast(t) = e {
                    sink.lock().unwrap().push(t)
                }
            }),
        );
        rusqlite::Connection::open(dir.join("data/library.db"))
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER no_likes BEFORE INSERT ON likes BEGIN SELECT RAISE(ABORT, 'disk full'); END;
                 CREATE TRIGGER no_lists BEFORE INSERT ON playlists BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
            )
            .unwrap();
        for _ in 0..5 {
            b.dispatch("like", &json!({ "track": { "id": "dQw4w9WgXcQ", "title": "t", "thumb": "x" } })).await.unwrap();
        }
        {
            let seen = toasts.lock().unwrap();
            assert_eq!(seen.iter().map(|t| t.code).collect::<Vec<_>>(), [Code::LibraryFailed], "one toast, not five");
        }
        let e = b.dispatch("createPlaylist", &json!({ "name": "road" })).await.unwrap_err();
        assert_eq!(Code::of(&e), Code::LibraryFailed);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nothing_in_the_api_can_install_or_change_the_shell() {
        for method in METHODS {
            let name = method.name.to_lowercase();
            for word in ["install", "setupglass", "undoglass", "addtray", "fetchytdlp", "blur", "extension"] {
                assert!(!name.contains(word), "{} would reach the shell from the CLI or MCP", method.name);
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_go_through_the_registry() {
        let dir = std::env::temp_dir().join(format!("snellar-api-settings-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        for key in ["theme", "sort.", ""] {
            let err = b.dispatch("getSetting", &json!({ "key": key })).await.unwrap_err();
            assert!(format!("{err:#}").contains("unknown setting"), "{key}: {err}");
            assert!(b.dispatch("setSetting", &json!({ "key": key, "value": 1 })).await.is_err(), "{key}");
        }
        for (key, value) in [("handoff", json!("true")), ("volume", json!("50")), ("language", json!(1))] {
            let err = b.dispatch("setSetting", &json!({ "key": key, "value": value })).await.unwrap_err();
            assert!(format!("{err:#}").contains("takes a"), "{key}: {err}");
            assert_eq!(b.library.setting(key), None, "{key} not written");
        }
        assert!(b.dispatch("setSetting", &json!({ "key": "quality", "value": "lossless" })).await.is_err());
        assert_eq!(b.library.setting("quality"), None, "a wrong quality is not written");
        assert_eq!(b.resolver.quality(), Quality::Auto);
        b.dispatch("setSetting", &json!({ "key": "quality", "value": "dataSaver" })).await.unwrap();
        assert_eq!(b.resolver.quality(), Quality::DataSaver, "applies to the next resolve");
        b.dispatch("setSetting", &json!({ "key": "sort.history", "value": true })).await.unwrap();
        assert_eq!(b.dispatch("getSetting", &json!({ "key": "sort.history" })).await.unwrap(), json!(true));
        b.dispatch("setSetting", &json!({ "key": "language", "value": "uk" })).await.unwrap();
        assert_eq!(b.dispatch("getSetting", &json!({ "key": "language" })).await.unwrap(), json!("uk"));

        b.library.set_setting("glassTinted", "true");
        assert_eq!(b.dispatch("getSetting", &json!({ "key": "glass" })).await.unwrap(), json!("cover"), "the old key reads as the new one");
        assert_eq!(b.dispatch("getSetting", &json!({ "key": "glassTinted" })).await.unwrap(), json!("cover"));
        b.dispatch("setSetting", &json!({ "key": "glassTinted", "value": false })).await.unwrap();
        assert_eq!(b.library.setting("glass").as_deref(), Some("\"glass\""), "an old caller writes the new key");
        assert_eq!(b.library.setting("glassTinted").as_deref(), Some("true"), "the old value stays for an older build");
        b.dispatch("setSetting", &json!({ "key": "glass", "value": null })).await.unwrap();
        assert_eq!(b.dispatch("getSetting", &json!({ "key": "glass" })).await.unwrap(), Value::Null, "cleared, not the old value again");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failures_carry_a_code_for_the_window_to_translate() {
        let dir = std::env::temp_dir().join(format!("snellar-api-codes-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        let code = |r: Result<Value>| Code::of(&r.unwrap_err());
        assert_eq!(code(b.dispatch("noSuchMethod", &json!({})).await), Code::ApiMethod);
        assert_eq!(code(b.dispatch("search", &json!({})).await), Code::ApiArgs);
        assert_eq!(code(b.dispatch("like", &json!({ "track": { "id": "sc:123", "title": "x" } })).await), Code::ApiArgs);
        assert_eq!(code(b.dispatch("getSetting", &json!({ "key": "theme" })).await), Code::ApiArgs);
        assert_eq!(code(b.dispatch("bugReportAttach", &json!({ "name": "2026-01-01_00-00-00" })).await), Code::ApiNotFound);
        assert_eq!(code(b.dispatch("removeQueued", &json!({ "index": 99 })).await), Code::ApiNotFound);
        assert_eq!(code(b.dispatch("importPlaylist", &json!({ "url": " " })).await), Code::ApiArgs);
        assert_eq!(code(b.dispatch("importPlaylist", &json!({ "url": "https://example.com/x" })).await), Code::ImportUnsupported);
        assert_eq!(code(b.dispatch("importPlaylist", &json!({ "csv": "Artist\nA" })).await), Code::ImportColumns);
        assert_eq!(code(b.dispatch("importPlaylist", &json!({ "path": dir.join("none.csv") })).await), Code::ImportFile);
        assert_eq!(code(b.dispatch("importPlaylist", &json!({ "csv": "Title,Artist\nCreep,Radiohead" })).await), Code::NetConnect);
        assert_eq!(b.dispatch("importCancel", &json!({})).await.unwrap(), json!(false));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_local_cover_is_copied_checked_and_replaced() {
        let dir = std::env::temp_dir().join(format!("snellar-api-cover-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let b = backend(&dir).await;
        let list = b.dispatch("createPlaylist", &json!({ "name": "p" })).await.unwrap();
        let png = dir.join("cover.PNG");
        std::fs::copy(format!("{}/tests/fixtures/cover.png", env!("CARGO_MANIFEST_DIR")), &png).unwrap();
        let first = b.dispatch("playlistCoverFile", &json!({ "id": list, "path": png })).await.unwrap();
        let first = first.as_str().unwrap().to_owned();
        let copied = Path::new(first.strip_prefix("file://").unwrap()).to_owned();
        assert_eq!(copied.parent().unwrap(), dir.join("data").join("covers"));
        assert_eq!(&std::fs::read(&copied).unwrap()[..4], b"\x89PNG", "the copy is an image");
        assert_eq!(b.dispatch("playlist", &json!({ "id": list })).await.unwrap()["thumb"], json!(first));

        let text = dir.join("notes.png");
        std::fs::write(&text, "not an image").unwrap();
        let code = |r: Result<Value>| r.err().map(|e| Code::of(&e));
        assert_eq!(code(b.dispatch("playlistCoverFile", &json!({ "id": list, "path": text })).await), Some(Code::CoverUnreadable));
        let forged = dir.join("forged.png");
        std::fs::write(&forged, b"\x89PNG\r\n\x1a\nrest").unwrap();
        assert_eq!(code(b.dispatch("playlistCoverFile", &json!({ "id": list, "path": forged })).await), Some(Code::CoverUnreadable));
        assert_eq!(
            code(b.dispatch("playlistCoverFile", &json!({ "id": list, "path": dir.join("gone.png") })).await),
            Some(Code::CoverUnreadable)
        );
        assert_eq!(code(b.dispatch("editPlaylist", &json!({ "id": list, "cover": "file:///etc/passwd" })).await), Some(Code::ApiArgs));
        assert!(copied.exists(), "a refused image keeps the cover");

        b.dispatch("editPlaylist", &json!({ "id": list, "cover": "https://i.ytimg.com/x", "description": " road songs ", "name": "  " }))
            .await
            .unwrap();
        let p = b.dispatch("playlist", &json!({ "id": list })).await.unwrap();
        assert_eq!((&p["thumb"], &p["description"], &p["name"]), (&json!("https://i.ytimg.com/x"), &json!("road songs"), &json!("p")));
        assert!(!copied.exists(), "the replaced local cover is deleted");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unset_handoff_stays_off() {
        let dir = std::env::temp_dir().join(format!("snellar-api-handoff-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        assert_eq!(b.dispatch("getSetting", &json!({ "key": "handoff" })).await.unwrap(), Value::Null);
        assert!(!handoff::Host::handoff_enabled(&*b));
        b.dispatch("setSetting", &json!({ "key": "handoff", "value": true })).await.unwrap();
        assert!(handoff::Host::handoff_enabled(&*b));
        b.dispatch("setSetting", &json!({ "key": "handoff", "value": null })).await.unwrap();
        assert!(!handoff::Host::handoff_enabled(&*b));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ids_that_are_not_video_ids_never_reach_the_player() {
        let dir = std::env::temp_dir().join(format!("snellar-api-ids-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        for bad in ["https://example.invalid/a.mp3", "short", "has space!!", "ünïcödé-id__", "yt:short", "sc:dQw4w9WgXcQ"] {
            let hostile = json!({ "id": bad, "title": "x", "artists": [], "duration": 1.0, "thumb": "" });
            let calls = [
                ("play", json!({ "tracks": [hostile], "index": 0 })),
                ("enqueue", json!({ "track": hostile })),
                ("radio", json!({ "track": hostile })),
                ("like", json!({ "track": hostile })),
                ("download", json!({ "track": hostile })),
                ("lyrics", json!({ "track": hostile })),
                ("addToPlaylist", json!({ "id": 1, "track": hostile })),
                ("prefetch", json!({ "id": bad })),
                ("track", json!({ "id": bad })),
            ];
            for (method, args) in calls {
                let err = b.dispatch(method, &args).await.unwrap_err();
                assert!(format!("{err:#}").contains("not a track id"), "{method} {bad}: {err:#}");
            }
        }
        assert!(b.dispatch("favorites", &json!({})).await.unwrap().as_array().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bare_video_ids_are_still_accepted_and_answered_with_the_prefix() {
        let dir = std::env::temp_dir().join(format!("snellar-api-bare-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        let (bare, prefixed) = (id("a"), format!("yt:{}", id("a")));
        let saved = json!({ "tracks": [track("q"), track("a"), track("r")], "cursor": 0 });
        b.library.set_setting("queue", &saved.to_string());

        assert_eq!(b.dispatch("like", &json!({ "track": track("a") })).await.unwrap(), json!(true));
        for form in [&bare, &prefixed] {
            assert_eq!(b.dispatch("isLiked", &json!({ "id": form })).await.unwrap(), json!(true), "{form}");
            assert_eq!(b.dispatch("track", &json!({ "id": form })).await.unwrap()["id"], json!(prefixed), "{form}");
        }
        assert_eq!(b.dispatch("favorites", &json!({})).await.unwrap()[0]["id"], json!(prefixed));
        let list = b.dispatch("createPlaylist", &json!({ "name": "road" })).await.unwrap();
        b.dispatch("addToPlaylist", &json!({ "id": list, "track": track("a") })).await.unwrap();
        assert_eq!(b.dispatch("playlist", &json!({ "id": list })).await.unwrap()["tracks"][0]["id"], json!(prefixed));
        b.dispatch("removeFromPlaylist", &json!({ "id": list, "trackId": bare })).await.unwrap();
        assert_eq!(b.dispatch("playlist", &json!({ "id": list })).await.unwrap()["count"], 0);

        let queued = |q: Value| q["tracks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let [q, r] = ["q", "r"].map(|n| format!("yt:{}", id(n)));
        assert_eq!(queued(b.dispatch("queue", &json!({})).await.unwrap()), [q.as_str(), &prefixed, &r]);
        b.dispatch("moveQueued", &json!({ "from": 9, "to": 2, "id": bare })).await.unwrap();
        assert_eq!(queued(b.dispatch("queue", &json!({})).await.unwrap()), [q.as_str(), &r, &prefixed]);
        b.dispatch("removeQueued", &json!({ "index": 0, "id": bare })).await.unwrap();
        assert_eq!(queued(b.dispatch("queue", &json!({})).await.unwrap()), [q.as_str(), &r]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enqueue_takes_a_list_in_its_order() {
        let dir = std::env::temp_dir().join(format!("snellar-api-enqueue-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        b.library.set_setting("queue", &json!({ "tracks": [track("q"), track("r")], "cursor": 0 }).to_string());
        b.dispatch("enqueue", &json!({ "tracks": [track("x"), track("y")], "next": true })).await.unwrap();
        b.dispatch("enqueue", &json!({ "tracks": [track("u"), track("v")] })).await.unwrap();
        let queue = b.dispatch("queue", &json!({})).await.unwrap();
        let got: Vec<TrackId> =
            serde_json::from_value(queue["tracks"].as_array().unwrap().iter().map(|t| t["id"].clone()).collect()).unwrap();
        let want: Vec<TrackId> = ["q", "x", "y", "r", "u", "v"].iter().map(|n| TrackId::parse(&id(n))).collect();
        assert_eq!(got, want);
        let err = b.dispatch("enqueue", &json!({ "tracks": [{ "id": "sc:1", "title": "x" }] })).await.unwrap_err();
        assert_eq!(Code::of(&err), Code::ApiArgs);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn links_come_from_the_source() {
        let dir = std::env::temp_dir().join(format!("snellar-api-link-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        let link = |kind: &str, id: &str| {
            let b = b.clone();
            let args = json!({ "kind": kind, "id": id });
            async move { b.dispatch("link", &args).await.unwrap() }
        };
        assert_eq!(link("track", &id("a")).await, json!(format!("https://music.youtube.com/watch?v={}", id("a"))));
        assert_eq!(link("artist", "UCx").await, json!("https://music.youtube.com/channel/UCx"));
        assert_eq!(link("playlist", "7").await, json!("https://music.youtube.com/playlist?list=7"));
        assert_eq!(link("artist", "").await, Value::Null);
        let err = b.dispatch("link", &json!({ "kind": "folder", "id": "x" })).await.unwrap_err();
        assert_eq!(Code::of(&err), Code::ApiArgs);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn queue_edits_out_of_range_fail_loudly() {
        let dir = std::env::temp_dir().join(format!("snellar-api-range-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        for (method, args) in
            [("jump", json!({ "index": 999 })), ("removeQueued", json!({ "index": 999 })), ("moveQueued", json!({ "from": 0, "to": 5 }))]
        {
            let err = b.dispatch(method, &args).await.unwrap_err();
            assert!(format!("{err:#}").contains("queue"), "{method}: {err:#}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fx_is_clamped_and_only_saved_once_applied() {
        let dir = std::env::temp_dir().join(format!("snellar-api-fx-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let b = backend(&dir).await;
        b.dispatch("fx", &json!({ "fx": { "speed": 0, "reverb": 9, "bass": 1000 } })).await.unwrap();
        let saved = b.dispatch("getFx", &json!({})).await.unwrap();
        assert_eq!((saved["speed"].clone(), saved["reverb"].clone(), saved["bass"].clone()), (json!(0.25), json!(1.0), json!(20.0)));
    }
}
