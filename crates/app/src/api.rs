use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::FutureExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::{Notify, OnceCell};

use snellar_core::cache::{Memo, Rule};
use snellar_core::error::{self, Code, Told};
use snellar_core::ids::TrackId;
use snellar_core::journal;
use snellar_core::library::Library;
use snellar_core::model::{Album, AlbumCard, Artist, Fx, Kind, Mix, Part, Pin, Playlist, Repeat, Shelf, Track};
use snellar_core::settings;
use snellar_core::source::{Filter, Source};
use snellar_core::sync::Lock;
use snellar_core::tasks::TaskSet;
use snellar_core::ui::{LibraryProblem, PlayFailure, Toast, Ui, UiEvent};
use snellar_youtube::lyrics::LyricsSource;
use snellar_youtube::net::{Http, Route};
use snellar_youtube::proxy::{Bound, Proxy};
use snellar_youtube::reach::Targets;
use snellar_youtube::recs::{Recs, WaveKnobs};
use snellar_youtube::source::YouTube;
use snellar_youtube::stream::{Quality, Resolver};
use snellar_youtube::ytm::Ytm;

use crate::covers;
use crate::downloads::Downloads;
use crate::downloads::collections::{self, Collections};
use crate::fx;
use crate::handoff::{self, Handoff};
use crate::import::{self, Imports, Input};
use crate::lyrics::{self, LyricsMemo};
use crate::mpv;
use crate::player::Player;
use crate::report;
use crate::update::Updater;

pub const PICK_PATH: &str = "pickPath";
const HANDOFF_OFFER: Duration = Duration::from_secs(15);
const FRESH: Duration = Duration::from_secs(120);
const KEEP: Duration = Duration::from_secs(600);
const PAGES: Rule = Rule { name: "page", cap: 50, fresh: FRESH, keep: KEEP };
const SEARCHES: Rule = Rule { name: "search", cap: 50, fresh: FRESH, keep: KEEP };
const SUGGESTIONS: Rule = Rule { name: "suggest", cap: 256, fresh: KEEP, keep: KEEP };
const BACKGROUND_PAGES: usize = 2;
const RECENT_CARDS: usize = 20;
const REPORT_PLAYS: usize = 20_000;
const CONNECT_LIMIT: Duration = Duration::from_secs(4);
const LOCK_RECHECK: Duration = Duration::from_secs(3);
const NO_RECHECK: Duration = Duration::from_secs(365 * 24 * 3600);
const COLLECTIONS_AFTER_START: Duration = Duration::from_secs(20);
const COLLECTIONS_RETRY: Duration = Duration::from_secs(60);
const PART_CHECK: Duration = Duration::from_secs(60);

#[derive(Serialize)]
pub struct Method {
    pub name: &'static str,
    pub group: &'static str,
    pub args: &'static str,
    pub returns: &'static str,
    pub doc: &'static str,
    #[serde(skip)]
    pub library_read: bool,
    #[serde(skip)]
    pub ordered: bool,
    #[serde(skip)]
    pub quiet: bool,
    #[serde(skip)]
    pub warmed: bool,
}

const fn m(name: &'static str, group: &'static str, args: &'static str, returns: &'static str, doc: &'static str) -> Method {
    Method { name, group, args, returns, doc, library_read: false, ordered: false, quiet: false, warmed: false }
}

impl Method {
    const fn library_read(self) -> Method {
        Method { library_read: true, ..self }
    }

    const fn ordered(self) -> Method {
        Method { ordered: true, ..self }
    }

    const fn quiet(self) -> Method {
        Method { quiet: true, ..self }
    }

    const fn warmed(self) -> Method {
        Method { warmed: true, ..self }
    }
}

pub fn method(name: &str) -> Option<&'static Method> {
    METHODS.iter().find(|m| m.name == name)
}

fn is(name: &str, flag: fn(&Method) -> bool) -> bool {
    method(name).is_some_and(flag)
}

pub fn ordered(name: &str) -> bool {
    is(name, |m| m.ordered)
}

pub fn quiet(name: &str) -> bool {
    is(name, |m| m.quiet)
}

#[rustfmt::skip]
pub const METHODS: &[Method] = &[
    m("search", "browse", "q: string, filter?: all|songs|videos|albums|artists|playlists", "SearchResult", "Search YouTube Music. `top` is the top result as a `Hit` (see `suggest`): an artist, album, playlist, song or video (all filter). `playlists` holds YouTube playlists as cards (all and playlists filters); the playlists filter takes three community playlists, then three of YouTube Music's own, in turn, as the playlist shelf of the all filter does. `videos` holds music videos and uploads with `Artist - Title [tags]` names split into artist and title (all and videos filters). A pasted YouTube link (watch, youtu.be, music.youtube.com, shorts, embed) returns just that track in `linked`; a link to an album, artist or playlist returns `page: {kind, id}` and nothing else. Results are cached for two minutes."),
    m("suggest", "browse", "q: string", "{queries: string[], hits: Hit[]}", "Search-box suggestions for a partial query: completed queries, and the artists, albums, songs and playlists YouTube Music offers for it. A `Hit` is a `Track`, `AlbumCard` or `ArtistCard` with `type`: `track`, `album`, `artist` or `playlist` (an `AlbumCard` whose id is a playlist id)."),
    m("home", "browse", "", "{shelves: Shelf[], recent: {kind, id, item}[], problem?: string, retryAfter?: int}", "Personalised home shelves (served from cache, refreshed in the background) and what was played lately, without repeats: `kind` is `album`, `remote`, `playlist`, `artist` or, where nothing better exists, `track`; `item` is the card. When YouTube failed and something else can be shown, `problem` is the error code and what is shown is older or partial; with nothing to show the call fails with that code."),
    m("homeRefresh", "browse", "", "{shelves: Shelf[], recent: {kind, id, item}[], problem?: string, retryAfter?: int}", "Rebuild the home shelves now. A failure never replaces what was kept."),
    m("album", "browse", "id: string", "Album", "Album or single with its tracks. `id` is an AlbumCard id from search. Pages (album, artist, discography, artistSongs, remotePlaylist) are answered from memory for two minutes; for ten minutes the window shows the kept page at once and asks again, a repeat after two minutes goes to the network and waits for one of two background slots.").warmed(),
    m("artist", "browse", "id: string", "Artist", "Artist page: top songs, albums, singles, related artists.").warmed(),
    m("discography", "browse", "id: string", "Discography", "Every album and every single or EP of an artist: `{artist: {id, name}, albums: AlbumCard[], singles: AlbumCard[]}` in the source's order, read through every page. An artist without a full list gives what its page shows."),
    m("artistSongs", "browse", "id: string", "Track[]", "All popular songs of an artist, every page of the list behind Top songs; the top songs of the page when the artist has no such list."),
    m("artistMix", "browse", "id: string, kind: radio|shuffle", "Track[]", "The artist's own mix from the source: `radio` is the artist radio (the artist and similar ones), `shuffle` the artist's songs in a new order each time. Empty when the source has none; the window then falls back to the radio of the top song or a shuffle of `artistSongs`."),
    m("remotePlaylist", "browse", "id: string", "Album", "A YouTube Music playlist with all its tracks, read through every page. Kept like `album`.").warmed(),
    m("warm", "browse", "method: album|artist|remotePlaylist, id: string", "null", "Fetch a page in the background because the person is likely to open it next (a song menu is open, the pointer rests on a link). Returns at once. Nothing happens when the page is already fresh in the cache, already being fetched, or two background page requests are running; a failure is dropped."),
    m("lyrics", "browse", "track: Track", "Lyrics|null", "Synced lines when available, else plain text; null when none is found. Asked of lrclib, NetEase, Kugou and YouTube Music at once; when YouTube Music has the text of this very video, only lyrics that agree with it are shown; synced lyrics in another script than that text are not judged by it and come before it; without it the lyrics most services agree on, and null when the most backed texts differ (decision 0094). Answers of the last 20 tracks are kept in memory: found lyrics for 30 minutes, none found for 3 minutes (not kept for a track without a length), failures not at all; asks for a track already being fetched wait for that fetch."),
    m("readingLyrics", "browse", "on: bool, away?: bool", "null", "The window says whether a lyrics view is shown in a window on screen. While it is, and for 5 minutes after the last one closes (at once when it closes because the window was hidden or minimized: `away`), the lyrics of the next track in the queue are fetched ahead, one track, 3 s after the queue settles and never while a track is starting, so they show at once when it plays. Otherwise no track is sent to the lyrics services until its lyrics are asked for (decision 0094).").ordered(),
    m("link", "browse", "kind: track|album|artist|playlist, id: string", "string|null", "A link to share: the music.youtube.com page of a track id, album, artist or YouTube playlist id; null when there is none (a local playlist, an id the source does not know)."),
    m("track", "browse", "id: string", "Track", "Metadata for a track id: from the library when known, otherwise from YouTube Music."),
    m("nudgeAnswer", "playback", "id: string, action: string", "null", "Answer a nudge card. For the handoff offer `continue` pauses the other player and continues here from the same position, `ignore` drops it; a late or unknown id does nothing."),
    m("daypart", "browse", "part?: morning|day|evening|night, reshuffle?: bool", "Shelf", "The pick for a part of the day (default: now, in local time; morning 5-11, day 11-17, evening 17-22, night 22-5, and while a listening session that began in the night goes on without a 4-hour break, the morning is still night), the same shelf as the first one of `home` (`kind` `daypart`). With enough listening at that time in the last 8 weeks (12 songs heard to the end, with someone at the computer, on 3 days, from 4 artists; weekdays and weekends apart when each has enough) it is built from those songs and `pick` names them: `{part, habits, habit: true, weekend, artists, plays, days}`, `artists` the three heard most, `habits` the parts of the day that have enough. Otherwise it is the Wave with `habit: false`. Kept 30 minutes; `reshuffle` builds a new one that leaves out the songs of the one kept."),
    m("wave", "browse", "mood?: string, language?: uk|ru|en|ko|ja|zh, novelty?: number, limit?: int, reshuffle?: bool", "Track[]", "Wave preview, the same list as the first track shelf of `home`: tracks built from the library's taste around the YouTube Music radio, kept for 30 minutes. Each argument given overrides that one knob of the saved `wave` setting for this call; the setting itself is not changed. `reshuffle` builds a new list that leaves out the one kept before (the New mix button). `novelty` is the share of unfamiliar artists, 0 to 1."),
    m("play", "playback", "tracks: Track[], index?: int, context?: {kind, item}", "null", "Replace the queue with `tracks` and start playing at `index` (default 0). `context` names the collection they come from (`kind` as in `pin`, `item` its card), so that it shows in the recent cards on `home`.").ordered(),
    m("radio", "playback", "track: Track", "null", "Play `track` now and fill the queue with recommendations built from it.").ordered(),
    m("prefetch", "playback", "id: string", "null", "Warm the stream for a track id so playing it starts instantly. Call on hover or selection.").quiet(),
    m("enqueue", "playback", "track?: Track, tracks?: Track[], next?: bool", "null", "Add a track, or several in their order (an album, a playlist), after the current one (`next: true`) or at the end. Starts playing only when the queue has no current track (it was empty or cleared); after the queue played to its end the added tracks wait in the queue for `next` or `jump`, as in Spotify.").ordered(),
    m("cancelStart", "playback", "", "null", "Stop waiting for the track that is starting, or for its next automatic retry."),
    m("toggle", "playback", "", "null", "Play or pause."),
    m("next", "playback", "", "null", "Next track; at the end of the queue autoplay appends recommendations.").ordered(),
    m("prev", "playback", "", "null", "Restart the track if more than 3 s in, otherwise the previous one.").ordered(),
    m("seek", "playback", "pos: number", "null", "Seek to an absolute position in seconds."),
    m("volume", "playback", "value: number", "null", "Volume 0 to 100."),
    m("jump", "playback", "index: int, id?: string", "null", "Play the queue item at `index`. With `id` the track is found by id (nearest to `index`) even if the queue shifted since the list was read; it fails if the track left the queue.").ordered(),
    m("removeQueued", "playback", "index: int, id?: string", "null", "Remove the queue item at `index` (or the track `id` nearest to it); removing the current track plays the next. Removed tracks are not re-added by autoplay.").ordered(),
    m("moveQueued", "playback", "from: int, to: int, id?: string", "null", "Move a queue item (found by `id` when given); the current track stays current.").ordered(),
    m("clearQueue", "playback", "keepCurrent?: bool", "null", "Empty the queue and stop. With `keepCurrent` the playing track stays and keeps playing, everything else goes and is not brought back by autoplay.").ordered(),
    m("shuffle", "playback", "on: bool", "null", "Shuffle the upcoming tracks; turning it off restores the original order.").ordered(),
    m("repeat", "playback", "mode: off|all|one", "null", "Repeat mode."),
    m("fx", "playback", "fx: Fx", "null", "Apply and save audio effects (EQ, bass, reverb, speed...)."),
    m("getFx", "playback", "", "Fx", "Saved audio effects.").quiet(),
    m("audioDevices", "playback", "", "{devices: {name, description}[], wanted, active}", "Output devices to offer: one per real output, from the sound server when there is one (decision 0070), without mpv's `auto`. `wanted` is the entry the saved choice (`audioDevice` setting) plays on, or the saved name while it is not connected; `active` is the device in use, `auto` while the saved one is not connected."),
    m("now", "playback", "", "{current: Track|null, playing, loading, position, duration, volume, shuffle, repeat, stream, liked}", "Snapshot of what is playing.").quiet(),
    m("queue", "playback", "", "{tracks: Track[], cursor: int}", "The queue; `cursor` is the current index or -1.").quiet(),
    m("like", "library", "track: Track", "bool", "Toggle a like; returns the new state."),
    m("setLiked", "library", "on: bool, track?: Track, tracks?: Track[]", "int", "Like (`on`) or unlike every given track, whatever its state was; returns how many changed. Liking takes a dislike back."),
    m("isLiked", "library", "id: string", "bool", "Whether a track id is liked.").quiet(),
    m("favorites", "library", "", "Track[]", "Liked tracks, newest first; `at` is when each was liked (ms since 1970).").library_read(),
    m("dislike", "library", "track: Track", "null", "Mark a track disliked (removes its like); recommendations avoid the track and its artist."),
    m("undislike", "library", "id: string", "null", "Take back a dislike; a like it replaced is not restored."),
    m("dislikes", "library", "", "Track[]", "Disliked tracks, newest first.").library_read(),
    m("history", "library", "limit?: int", "Track[]", "Recently played distinct tracks (default 100); `at` is when each was last played (ms since 1970).").library_read(),
    m("clearHistory", "library", "", "null", "Delete play history (likes are kept)."),
    m("removeFromHistory", "library", "id: string", "null", "Remove every play of a track id from the history and from what recommendations learn (taste, habits, `recsReport`), as if it was never played. When it is the song that plays, it plays on and this play is not written either. Likes and playlists are kept."),
    m("recsReport", "library", "days?: int", "{via, started, plays, earlySkips, completed, liked, repeated, failed, away}[]", "How recommendations fared on this computer over the last `days` (default 28), from the local history only: per source (`user` is your own picks, the baseline; `wave`, `radio`, `autoplay`, `because`, `daypart`, `shelf`; `before signals` are plays an older build recorded) and per how the play started (`chosen` by you, or `itself` after the previous one). Rates are shares of the plays that ended (failures, quits and stops are left out): `earlySkips` skipped within 30 s of time heard, `completed` heard to 80 %, `liked` liked within 7 days, `repeated` played again within 7 days; `failed` counts plays that failed; `away` is the share of time heard while the computer was idle or locked."),
    m("pickPath", "library", "title: string, folder?: bool, filters?: string[], start?: string", "{status: picked|cancelled|unavailable, path?}", "Ask the desktop's file chooser portal for a file or a folder. Only the window can call this: a dialog is never opened from the socket or MCP, which get `api.window_only`. `filters` read like `CSV files (*.csv)`. `unavailable` means there is no portal or it failed, and the window then shows its own dialog."),
    m("download", "library", "track?: Track, tracks?: Track[]", "null", "Download best audio of a track, or of several, to the downloads folder in the background; a toast reports each result. Downloads wait in one queue, two at a time, ahead of the songs of downloaded collections; none starts while the folder's disk has less than 1 GiB free (`download.no_space`). A finished file is named `Artist - Title [video id]` and linked to its track, which then plays from the file without the network; a file is deleted only when no downloaded track uses it. At start, songs that share one file with another song (downloads named before the video id was in the name) download again quietly, each into its own file; the shared file goes once no song uses it."),
    m("downloadCollection", "library", "kind: playlist|remote|album, id: string|int, on: bool", "null", "Keep a playlist, a saved YouTube playlist or an album downloaded (`on`), or stop. On, its songs that have no file join the download queue behind single downloads, quietly, without a toast each; a remote playlist or album is read from the source and saved to the library if it was not. While on, songs added to a local playlist download too, and a removed song loses its file unless another downloaded collection or a single download keeps it; a saved YouTube playlist is read again at start. Off asks nothing: queued songs are dropped and the files go, except those another downloaded collection or a single download keeps. A network failure, a missing yt-dlp or a full disk stops the queue and it is tried again every minute; a song the source refuses is skipped until the next start. Unsaving a remote playlist or album turns it off. When the library cannot be read it fails with that `library.*` error and nothing changes; no file is ever removed on a failed read. `downloads` events say what is queued."),
    m("downloadedCollections", "library", "", "{kind, id, total, done, refused, busy, stalled?}[]", "Collections kept downloaded, in the order they were turned on. `done` songs have a file, `refused` were refused by the source in this run, `busy` are queued or downloading; `stalled` is the code that stopped the queue (a `net.*` or `proxy.*` code, `ytdlp.missing` or `download.no_space`) while the collection is not complete. Complete is `done + refused == total`."),
    m("downloads", "library", "", "{name, path, size, modified}[]", "Finished downloads, newest first."),
    m("downloadedTracks", "library", "", "string[]", "Ids of tracks linked to a downloaded file that still exists.").library_read(),
    m("downloadedSongs", "library", "single?: bool", "Track[]", "Songs with a downloaded file that is still there, the last downloaded first; `at` is when it was downloaded (ms since 1970). A song the library has no details for is named after its file. With `single`, only the songs downloaded on their own: a song a downloaded collection pulled in is left out unless it was also asked for by hand.").library_read(),
    m("playlists", "library", "", "Playlist[]", "Local playlists with counts and a cover (tracks omitted).").library_read(),
    m("recentPlaylists", "library", "", "Playlist[]", "Local playlists like `playlists`, the one last added to first.").library_read(),
    m("playlistsWith", "library", "trackId: string", "int[]", "Ids of the local playlists that hold a track, lowest first; one query over every playlist, for the add-to-playlist menus to tick them.").library_read(),
    m("playlist", "library", "id: int", "Playlist|null", "One local playlist with its tracks in order; `at` on each track is when it was added (ms since 1970).").library_read(),
    m("createPlaylist", "library", "name: string", "int", "Create a playlist; returns its id."),
    m("renamePlaylist", "library", "id: int, name: string", "null", "Rename a playlist."),
    m("deletePlaylist", "library", "id: int", "Playlist|null", "Delete a playlist; returns it with its tracks (null if there was none), which `restorePlaylist` takes to undo."),
    m("restorePlaylist", "library", "playlist: Playlist", "int", "Bring back a deleted playlist from what `deletePlaylist` returned: its name and tracks in order with their `at`, under its old id when that is free. Returns the id."),
    m("addToPlaylist", "library", "id: int, track?: Track, tracks?: Track[]", "int", "Append a track, or several in their order; a track already in the playlist is skipped. Returns how many were added."),
    m("movePlaylistItem", "library", "id: int, trackId: string, to: int", "bool", "Move a track inside a playlist to index `to` (0 is first; past the end is last). False when the track is not in it."),
    m("editPlaylist", "library", "id: int, name?: string, description?: string, cover?: string", "null", "Change a playlist's name, description or cover; a field left out stays. `cover` is an image URL (https), one of the playlist's earlier covers from `playlistCovers`, or empty to use the first track's cover again. A local cover it replaces becomes an earlier cover."),
    m("playlistCoverFile", "library", "id: int, path: string", "string", "Copy a local PNG, JPEG or WebP image (at most 20 MB, its longer side reduced to 1024 pixels) into the data directory as the playlist's cover; returns its `file://` URL. The local cover it replaces becomes an earlier cover."),
    m("playlistCovers", "library", "id: int", "string[]", "The playlist's earlier local covers as `file://` URLs, the one shown last first, at most 7; older ones are deleted."),
    m("importPlaylist", "library", "url?: string, csv?: string, path?: string, name?: string", "ImportResult", "Import a playlist into a new local playlist: a public playlist or album link from Spotify, Deezer, Apple Music, Yandex Music or SoundCloud (`url`), CSV text (`csv`) or a CSV file (`path`) as Exportify and TuneMyMusic write them. Each song is looked up on YouTube Music by title, artist and length, four at a time; `import` events report `{stage: reading|matching, done, total, matched}`. Returns `{playlist, name, total, matched, unmatched: ImportTrack[], merged: ImportTrack[], hidden: int[], limited, sourceTotal, failed, code?}`: `playlist` is the new id, null when nothing was found; `matched` counts the songs added: rows that found a song already found for an earlier row are added once and listed in `merged`; `hidden` holds the places (from 1) of rows the service lists without a name, which Spotify does for songs not available in this country; `failed` counts the songs that could not be looked up because a search failed, and `code` says why; those are not in `unmatched`, which holds only songs that were looked for and not found; `ImportTrack` is `{title, artists, album, duration}`; `limited` means only part of the list could be read: Apple Music pages show at most 100 songs, and a long Spotify playlist is paged through Spotify's private web player API with the token its embed page issues, and stays at the embed's 100 songs or at the pages already read when that API refuses or changes; `sourceTotal` is the length of the whole list when the source reports it. `name` overrides the playlist name. The source's own cover, when it has one, is saved as the playlist's cover; a cover that cannot be fetched does not stop the import. A newer import cancels a running one. Fails with `import.blocked` when the service hides the playlist (private, sign-in, captcha, 403, 451): export it to CSV then."),
    m("importCancel", "library", "", "bool", "Cancel the running import; it then fails with `import.cancelled` and makes no playlist. False when none runs."),
    m("removeFromPlaylist", "library", "id: int, trackId?: string, trackIds?: string[]", "null", "Remove a track, or several, from a playlist."),
    m("savedAlbums", "library", "", "AlbumCard[]", "Saved albums, newest first.").library_read(),
    m("toggleAlbum", "library", "album: AlbumCard", "bool", "Save or unsave an album; returns the new state."),
    m("savedPlaylists", "library", "", "AlbumCard[]", "YouTube playlists saved to the library, newest first (`kind` is `Playlist`, `id` the playlist id for `remotePlaylist`).").library_read(),
    m("togglePlaylist", "library", "playlist: AlbumCard", "bool", "Save or unsave a YouTube playlist; returns the new state. The tracks are not copied: the playlist is read again when opened."),
    m("followed", "library", "", "ArtistCard[]", "Followed artists, newest first. Kept in the local library only.").library_read(),
    m("follow", "library", "artist: ArtistCard", "bool", "Follow or unfollow an artist; returns the new state."),
    m("library", "library", "", "{kind, id, item, at}[]", "Everything kept in the library, newest first: local playlists (`playlist`, `item` a `Playlist` without tracks), saved YouTube playlists (`remote`), albums (`album`) and followed artists (`artist`); `at` is when each was made or saved (ms since 1970).").library_read(),
    m("folders", "library", "", "{id, name, open}[]", "Playlist folders in the order they were made; `open` is whether the sidebar shows their playlists. A playlist names its folder in `folder`.").library_read(),
    m("createFolder", "library", "name: string", "int", "Make a playlist folder; returns its id."),
    m("renameFolder", "library", "id: int, name: string", "null", "Rename a folder."),
    m("openFolder", "library", "id: int, open: bool", "null", "Remember whether a folder is open in the sidebar."),
    m("deleteFolder", "library", "id: int", "null", "Delete a folder; its playlists stay, outside any folder."),
    m("moveToFolder", "library", "id: int, folder?: int|null", "bool", "Put a playlist in a folder, or out of any folder when `folder` is null or left out. False when the playlist or the folder does not exist."),
    m("pins", "library", "", "{kind, id, item}[]", "What is pinned to the top of the sidebar, in the order it was pinned. `kind` is `playlist` (a local playlist, `id` its number as text), `remote` (a saved YouTube playlist), `album` or `artist`; `item` is the card as it was pinned.").library_read(),
    m("pin", "library", "kind: string, item: object", "bool", "Pin or unpin an item to the sidebar; returns the new state. `item` is the card (`id` required); for a local playlist `{id}` is enough."),
    m("getSetting", "settings", "key: string", "any", "Read a setting from the table below (null if unset). An unknown key is an error.").quiet(),
    m("setSetting", "settings", "key: string, value: any", "null", "Write a setting from the table below. An unknown key or a value of the wrong kind is an error; null clears it to the default. A saved value this build cannot read (`library.corrupt`) is copied to the first free `<key>.broken`, `<key>.broken.2`, … before it is replaced. The app does the same by itself at start for a download list it cannot read, resets that list and shows `download.list_set_aside`; a preference is never reset that way."),
    m("libraryRetry", "meta", "", "bool", "Try to open the library file again when it could not be opened; true when it opened. It answers before the rest of the app has started."),
    m("libraryContinue", "meta", "", "null", "Run without saving after the library could not be opened: everything in this session is lost at quit. Only the window asks for this."),
    m("libraryFolder", "meta", "", "bool", "Show the data folder with the library file in the file manager."),
    m("raise", "meta", "", "null", "Show and raise the window. A second start of the app calls this on the running one and exits."),
    m("methods", "meta", "", "Method[]", "This table.").quiet(),
    m("bugReport", "meta", "note?: string", "{name, dir}", "Freeze a bug report now: the window shot, the journal of the last five minutes, the player state, the open screen and the window size go into `reports/<date-time>/` in the data directory, and a card with a one-line field opens in the window (Enter stores the phrase, Esc keeps the report without one). Music is not interrupted. Without a window only the Rust half is saved."),
    m("bugReportAttach", "meta", "name: string, ui?: object, note?: string", "null", "Add what the window saw (`ui`) or the phrase (`note`) to the report `name`. The window calls this itself after `bugReport`.").ordered(),
    m("bugReportPack", "meta", "name: string, note?: string, reveal?: bool", "{path, shown}", "Pack the report `name` into `reports/<name>.zip` and, unless `reveal` is false, show that file in the file manager. `note` is stored first, as in `bugReportAttach`. `shown` says whether a file manager was asked to show it. The archive holds the screenshot and the track list, so it shows what the person listens to; nothing is sent anywhere."),
    m("dependencies", "meta", "", "{desktop, components: {id, version, source, state}[], details}", "What Snellar runs on here and whether it really works: mpv, yt-dlp, ffmpeg, Qt, audio output, Discord, file portal, tray, window blur. `state` is ok, missing, broken, off, on, waiting or none. Reads only: nothing is installed and the desktop is not changed, here or anywhere else in this API. `details` is the same list as text, as bug reports hold it."),
    m("discord", "meta", "", "off|ok|waiting", "Whether the song playing can show in Discord: `off` when the `discordRpc` setting is off, `ok` when Discord is running, `waiting` when the setting is on and Discord is not running; Snellar connects by itself once it opens."),
    m("updateStatus", "meta", "", "{phase, version, checked, latest, percent, code?, place}", "How the self-update stands. `phase` is `development` (not an AppImage), `elsewhere` (an AppImage outside the installer's folder, named in `place`), `unsigned` (an installed AppImage built without an update key, it never checks), `off` (the `autoUpdate` setting), `idle`, `current` (a check found nothing newer), `checking`, `downloading` (`percent`), `ready` (`latest` starts at the next launch) or `failed` (`code` says why, the installed file was not touched, the next check tries again). `version` is this build for people; `checked` is the last time the release was read, in ms since 1970, 0 for never. Reads only: a check is never started from here."),
    m("reports", "meta", "", "Report[]", "Saved bug reports, newest first: `{name, path, note, track, screen, files}`. `snellar ctl reports` prints them as lines."),
];

pub fn markdown() -> String {
    let mut out = String::from(
        "# Snellar API\n\nGenerated from `api::METHODS` (`snellar ctl --docs`). Every method takes one JSON object and returns one JSON value. An argument marked `?` may be left out or null for its default; a value of another type is an error (`api.args`), never read as the default.\n\n",
    );
    out.push_str("Call: `POST /call/<method>` on the Unix socket `$XDG_RUNTIME_DIR/snellar.sock` (override: `SNELLAR_SOCK`), or `snellar ctl <method> '<json>'`.\n\n");
    out.push_str("```sh\ncurl --unix-socket \"$XDG_RUNTIME_DIR/snellar.sock\" -d '{\"q\":\"daft punk\"}' http://snellar/call/search\nsnellar ctl now\n```\n\n");
    out.push_str("Errors: status 400 with `{\"error\": \"...\", \"code\": \"...\"}` (bad arguments or a failed call), 404 for an unknown method. `error` is technical text for logs and scripts; `code` is stable and listed under Error codes below. The window shows its own translated text for the code, never `error`.\n\n");
    out.push_str("Events: `GET /events` is a Server-Sent Events stream: `snapshot` on connect, then `state`, `position` (about 5 per second while playing), `current`, `queue`, `liked`, `history` (a play was added to the history), `tools` (yt-dlp was fetched or checked), `update` (`updateStatus` has a new answer), `downloads` (the download queue changed: `{busy: string[]}`, the ids queued or downloading), `modes`, `stream` (what plays now: `{codec, bitrate, sampleRate, gain, leveling, local}`, bitrate in kbps and 0 when unknown), `toast` (`{code, cause?, arg?, detail?}`: `code` and `cause` from Error codes below, `arg` a name to show, such as the downloaded track, `detail` technical text), `reach` (whether YouTube can be reached over a direct connection: `null`, `net.offline` when neither YouTube nor any other host answers, `net.youtube_blocked` when YouTube does not answer and another host does; looked at after a failure, then every 5 s until YouTube answers, which sends `null`), `part` (the part of the day in local time, `morning`, `day`, `evening` or `night`, as `daypart` reads it: sent once the backend starts and when it changes, looked at every minute), `nudge` (a card to show: `{id, kind, track?, text?, cover?, timeout?, actions: [{id, primary?}]}`; `kind` `handoff` offers to continue `track`, with the actions `continue` and `ignore`; the window words the card itself; answer it with `nudgeAnswer`), `failure` (the track that plays or starts failed: `{id, code, tracks, stopped, retrying}`, `code` from Error codes below, `tracks` the ids of queued tracks that failed with their codes, `stopped` when playback stopped on it, `retrying` while an automatic retry waits; `null` once that is over), `library` (`null` once the library is open; otherwise `{code, path, detail, saving}`: the library file could not be opened, and `saving` is false while nothing is written to it), `import` (progress of `importPlaylist`: `{stage, done, total, matched}`), `desktop` (what the window needs for its background: `{desktop, frameless, blur, glass, line?, gnome?, reducedMotion, finished?}`; sent when the shell's blur settings change, and with `finished: true` after the window set them up or undid them).\n\n");
    out.push_str("Types: `Track {id, title, artists: [{id, name}], album?: {id, name}, duration, thumb, explicit}`, other shapes are in `crates/core/src/model.rs`.\n\n");
    out.push_str("Track ids name their source: `yt:` and the YouTube video id, as in `yt:dQw4w9WgXcQ`. Every argument that takes a track id or a `Track` also accepts the bare 11-character video id for now; answers always carry the prefixed form. Album, artist and playlist ids are unchanged.\n");
    let mut groups: Vec<&str> = Vec::new();
    for method in METHODS {
        if !groups.contains(&method.group) {
            groups.push(method.group);
        }
    }
    let cell = |s: &str| if s.is_empty() { "-".to_string() } else { format!("`{}`", s.replace('|', "\\|")) };
    for group in groups {
        out.push_str(&format!("\n## {group}\n\n| method | args | returns | |\n|---|---|---|---|\n"));
        for method in METHODS.iter().filter(|m| m.group == group) {
            out.push_str(&format!("| `{}` | {} | {} | {} |\n", method.name, cell(method.args), cell(method.returns), method.doc));
        }
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
    collections: Arc<Collections>,
    updates: Arc<Updater>,
    library: Arc<Library>,
    resolver: Arc<Resolver>,
    data: PathBuf,
    memo: Arc<Memo>,
    lyrics: Arc<LyricsMemo>,
    warming: TaskSet,
    player: OnceCell<Arc<Player>>,
    handoff: Handoff,
    http: Http,
    imports: Imports,
    saving: AtomicBool,
    fetches_tools: bool,
    collections_after: Duration,
    library_back: Notify,
}

fn wave_knobs(saved: WaveKnobs, asked: &Value) -> Result<WaveKnobs> {
    let mut knobs = json!(saved);
    let defaults = json!(WaveKnobs::default());
    if let (Some(saved), Some(asked)) = (knobs.as_object_mut(), asked.as_object()) {
        saved.extend(asked.iter().map(|(k, v)| (k.clone(), if v.is_null() { defaults[k].clone() } else { v.clone() })));
    }
    serde_json::from_value(knobs).context("bad wave arguments").context(Code::ApiArgs)
}

fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T> {
    let v = args.get(key).cloned().ok_or_else(|| Code::ApiArgs.err(format!("missing argument `{key}`")))?;
    serde_json::from_value(v).with_context(|| format!("bad argument `{key}`")).context(Code::ApiArgs)
}

fn card(kind: String, item: Value) -> Result<Pin> {
    if !["playlist", "remote", "album", "artist"].contains(&kind.as_str()) {
        return Err(Code::ApiArgs.err(format!("not a collection: `{kind}`: playlist, remote, album or artist")));
    }
    let id = id_text(&item["id"]).ok_or_else(|| Code::ApiArgs.err("`item.id` is missing"))?;
    Ok(Pin { kind, id, item })
}

fn id_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
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
    if given(args, "tracks").is_none() {
        return Ok(vec![track_arg(args, "track")?]);
    }
    let tracks: Vec<Track> = arg(args, "tracks")?;
    tracks.iter().try_for_each(|t| check_id(&t.id))?;
    Ok(tracks)
}

fn given<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    args.get(key).filter(|v| !v.is_null())
}

fn opt<T: DeserializeOwned + Default>(args: &Value, key: &str) -> Result<T> {
    match given(args, key) {
        None => Ok(T::default()),
        Some(v) => serde_json::from_value(v.clone()).with_context(|| format!("bad argument `{key}`")).context(Code::ApiArgs),
    }
}

const NET_SETTINGS: [settings::Setting; 4] = [settings::YTDLP_FALLBACK, settings::QUALITY, settings::REGION, settings::PROXY];

fn checked(key: &str, value: Value) -> Result<Value> {
    if value.is_null() {
        return Ok(value);
    }
    if key == settings::QUALITY.key {
        let name = value.as_str().unwrap_or_default();
        if Quality::named(name).is_none() {
            let names: Vec<&str> = Quality::ALL.iter().map(|q| q.name()).collect();
            return Err(Code::ApiArgs.err(format!("quality `{name}`: {}", names.join(", "))));
        }
    }
    if key == settings::PROXY.key {
        Route::parse(value.as_str().unwrap_or_default())?;
    }
    if key == settings::FX.key {
        let fx: Fx = serde_json::from_value(value).context("setting `fx` takes an `Fx` object").context(Code::ApiArgs)?;
        return Ok(json!(fx::clamp(&fx)));
    }
    if key == settings::DOWNLOAD_DIR.key
        && let Some(folder) = value.as_str().filter(|f| !f.is_empty() && snellar_core::product::download_folder(f).is_none())
    {
        return Err(Code::DownloadFolderInvalid.err(format!("download folder `{folder}` is not a full path")));
    }
    Ok(value)
}

fn apply_to_net(key: &str, lib: &Library, resolver: &Resolver, ytm: &Ytm) {
    match key {
        k if k == settings::YTDLP_FALLBACK.key => resolver.set_ytdlp(settings::YTDLP_FALLBACK.or_default(lib)),
        k if k == settings::QUALITY.key => resolver.set_quality(Quality::parse(&settings::QUALITY.or_default::<String>(lib))),
        k if k == settings::REGION.key => ytm.set_region(&settings::REGION.or_default::<String>(lib)),
        k if k == settings::PROXY.key => match Route::parse(&settings::PROXY.or_default::<String>(lib)) {
            Ok(proxy) => resolver.route().set(proxy),
            Err(e) => journal::warn("net", format!("the saved proxy is ignored: {e:#}")),
        },
        _ => {}
    }
}

fn memo_key(method: &str, a: &Value) -> Result<Option<(&'static Rule, String)>> {
    Ok(Some(match method {
        "search" => (&SEARCHES, format!("{}:{}", opt::<String>(a, "filter")?, arg::<String>(a, "q")?)),
        "suggest" => (&SUGGESTIONS, arg::<String>(a, "q")?),
        "album" | "artist" | "remotePlaylist" | "discography" | "artistSongs" => (&PAGES, format!("{method}:{}", arg::<String>(a, "id")?)),
        _ => return Ok(None),
    }))
}

async fn page(source: &dyn Source, method: &str, id: &str) -> Result<Value> {
    Ok(match method {
        "album" => json!(source.album(id).await?),
        "artist" => json!(source.artist(id).await?),
        "remotePlaylist" => json!(source.playlist(id).await?),
        other => return Err(Code::Internal.err(format!("`{other}` is not a page"))),
    })
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
    let (client, route) = (http.client(), http.route().clone());
    tokio::spawn(async move {
        if let Err(e) = client.head("https://music.youtube.com/").timeout(Duration::from_secs(5)).send().await {
            route.settle(snellar_youtube::net::code(&e)).await;
        }
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

/// A request that panics is answered `internal`; the panic itself is in the journal.
async fn guarded(method: &str, answer: impl Future<Output = Result<Value>>) -> Result<Value> {
    AssertUnwindSafe(answer).catch_unwind().await.unwrap_or_else(|_| Err(Code::Internal.err(format!("{method} panicked"))))
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
        *self.asked.locked() = Some(asked);
        self.wake.notify_all();
    }

    fn wait(&self, patience: Duration) -> Asked {
        let held = self.asked.locked();
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
        let told = ui.clone();
        route.watch(Targets::internet(), move |code| told(UiEvent::Reach(code)));
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
        if let Some(live) = library.playlist_covers().ok().filter(|_| saving) {
            covers::sweep(&data.join("covers"), &live);
        }
        let told = ui.clone();
        library.on_write_failure(move |toast| told(UiEvent::Toast(toast)));
        let ytm = Arc::new(Ytm::new(http.clone()));
        for setting in NET_SETTINGS {
            apply_to_net(setting.key, &library, &resolver, &ytm);
        }
        warm_up(&http);
        let proxy = Proxy::serve(listener, resolver.clone(), cache.join("img")).expect("local proxy");
        let playing = Arc::new(AtomicBool::new(false));
        let seen_playing = playing.clone();
        let blamed = resolver.route().clone();
        let recs = Recs::new(http.clone(), ytm.clone(), library.clone(), cache);
        let source: Arc<dyn Source> = YouTube::new(ytm.clone(), recs.clone(), resolver.clone(), proxy, LyricsSource::new(http));
        let lyrics = LyricsMemo::new(source.clone(), lyrics::TIMING);
        let heard = lyrics.clone();
        let ui: Ui = Arc::new(move |event| {
            if let UiEvent::State(state) = &event {
                seen_playing.store(state.playing, Ordering::Relaxed);
            }
            heard.hear(&event);
            ui(named(event, &blamed));
        });
        let downloads = Downloads::new(ui.clone(), library.clone(), resolver.clone());
        Arc::new(Backend {
            handoff: Handoff::new(playing, HANDOFF_OFFER),
            ui: ui.clone(),
            updates: Updater::for_this_run(library.clone(), resolver.route(), ui.clone()),
            collections: Collections::new(library.clone(), downloads.clone()),
            downloads,
            source,
            recs,
            ytm,
            library,
            resolver,
            data: report_dir,
            memo: Arc::new(Memo::new(BACKGROUND_PAGES)),
            lyrics,
            warming: TaskSet::default(),
            player: OnceCell::new(),
            http: import_http,
            imports: Imports::default(),
            saving: AtomicBool::new(saving),
            fetches_tools: true,
            collections_after: COLLECTIONS_AFTER_START,
            library_back: Notify::new(),
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
                self.downloads.set_aside_unreadable();
                self.downloads.note_repairs();
                self.collections.prune();
                self.collections.sync();
                self.library_back.notify_one();
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

    pub fn start_updates(&self) {
        self.updates.clone().start();
    }

    pub fn check_update(&self) {
        self.updates.check_now();
    }

    pub fn restart_for_update(&self) -> bool {
        self.updates.restart()
    }

    pub fn tasks(&self) -> &TaskSet {
        &self.warming
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

    pub(crate) fn emit(&self, event: UiEvent) {
        (self.ui)(event);
    }

    fn heart(&self, touched: &[Track], liked: impl FnOnce(&TrackId) -> bool) {
        let playing = self.player.get().and_then(|p| p.current()).filter(|c| touched.iter().any(|t| t.id == c.id));
        if let Some(current) = playing {
            (self.ui)(UiEvent::Liked(liked(&current.id)));
        }
    }

    pub(crate) async fn track(&self, id: &TrackId) -> Result<Track> {
        check_id(id)?;
        if let Some(track) = self.library.track(id) {
            return Ok(track);
        }
        self.source.track(id).await
    }

    async fn artist(&self, id: &str) -> Result<Artist> {
        let page = self.memo.get(&PAGES, &format!("artist:{id}"), page(&*self.source, "artist", id)).await?;
        serde_json::from_value(page).context(Code::Internal)
    }

    async fn memoized(&self, method: &str, a: &Value) -> Result<Value> {
        let (rule, key) = memo_key(method, a)?.ok_or_else(|| Code::Internal.err(format!("`{method}` is not memoized")))?;
        let fetch = async {
            Ok(match method {
                "search" => json!(self.source.search(&arg::<String>(a, "q")?, Filter::parse(&opt::<String>(a, "filter")?)).await?),
                "suggest" => json!(self.source.suggest(&arg::<String>(a, "q")?).await?),
                "discography" => json!(self.source.discography(&self.artist(&arg::<String>(a, "id")?).await?).await?),
                "artistSongs" => json!(self.artist_songs(&arg::<String>(a, "id")?).await?),
                _ => page(&*self.source, method, &arg::<String>(a, "id")?).await?,
            })
        };
        self.memo.get(rule, &key, fetch).await
    }

    pub fn peek(&self, method: &str, a: &Value) -> Option<Value> {
        if method == "lyrics" {
            return self.lyrics.peek(&track_arg(a, "track").ok()?.id).map(|kept| json!(kept));
        }
        let (rule, key) = memo_key(method, a).ok()??;
        self.memo.peek(rule, &key)
    }

    fn warm(&self, a: &Value) -> Result<()> {
        let (method, id) = (arg::<String>(a, "method")?, arg::<String>(a, "id")?);
        if !is(&method, |m| m.warmed) {
            let warmed: Vec<&str> = METHODS.iter().filter(|m| m.warmed).map(|m| m.name).collect();
            return Err(Code::ApiArgs.err(format!("cannot warm `{method}`: {}", warmed.join(", "))));
        }
        let (rule, key) = memo_key(&method, a)?.ok_or_else(|| Code::Internal.err(format!("`{method}` is not memoized")))?;
        let (memo, source) = (self.memo.clone(), self.source.clone());
        self.warming.spawn("warm", async move {
            memo.warm(rule, &key, page(&*source, &method, &id)).await;
        });
        Ok(())
    }

    async fn collection_page(&self, kind: &str, id: &str) -> Result<Album> {
        let method = if kind == "album" { "album" } else { "remotePlaylist" };
        serde_json::from_value(self.memoized(method, &json!({ "id": id })).await?).context(Code::Internal)
    }

    async fn download_collection(&self, a: &Value) -> Result<()> {
        let kind = arg::<String>(a, "kind")?;
        if !collections::KINDS.contains(&kind.as_str()) {
            return Err(Code::ApiArgs.err(format!("cannot download a `{kind}`: playlist, remote or album")));
        }
        let id = a.get("id").and_then(id_text).ok_or_else(|| Code::ApiArgs.err("`id` is missing"))?;
        if !arg::<bool>(a, "on")? {
            return self.collections.set(&kind, &id, false, &[]);
        }
        if kind == "playlist" {
            let number = id.parse().map_err(|_| Code::ApiArgs.err(format!("`{id}` is not a playlist number")))?;
            self.library.playlist_ids(number)?.ok_or_else(|| Code::ApiNotFound.err(format!("no playlist {id}")))?;
            return self.collections.set(&kind, &id, true, &[]);
        }
        let page = self.collection_page(&kind, &id).await?;
        let card = AlbumCard {
            id: id.clone(),
            title: page.title,
            artists: page.artists,
            year: page.year,
            kind: if kind == "remote" { "Playlist".into() } else { page.kind },
            thumb: page.thumb,
        };
        let lib = &self.library;
        if kind == "album" && !lib.saved_albums().iter().any(|s| s.id == id) {
            lib.toggle_album(&card);
        }
        if kind == "remote" && !lib.saved_playlists().iter().any(|s| s.id == id) {
            lib.toggle_playlist(&card);
        }
        self.collections.set(&kind, &id, true, &page.tracks)
    }

    /// yt-dlp is readied first when songs wait for a repair, so they are not fetched with a missing or old copy.
    /// Only the songs this one look found run, so none runs without yt-dlp readied.
    async fn repair(&self) {
        let due = self.downloads.note_repairs();
        if !due.is_empty() {
            self.ready_ytdlp().await;
        }
        self.downloads.run_repairs(due);
    }

    pub async fn ready_ytdlp(&self) {
        if !self.fetches_tools || !self.ytdlp_due() {
            return;
        }
        if let Err(e) = self.keep_ytdlp(false).await {
            journal::warn("tools", format!("yt-dlp stays as it is: {e:#}"));
        }
        self.emit(UiEvent::Tools);
    }

    pub async fn watch_part(&self) {
        let mut told = None;
        loop {
            let part = self.recs.part_now();
            if told != Some(part) {
                self.emit(UiEvent::Part(part));
                told = Some(part);
            }
            tokio::time::sleep(PART_CHECK).await;
        }
    }

    async fn refresh_remotes(&self) {
        for id in self.collections.remotes() {
            let kept = match self.collection_page("remote", &id).await {
                Ok(page) => self.collections.set("remote", &id, true, &page.tracks),
                Err(e) => Err(e),
            };
            if let Err(e) = kept {
                journal::info("downloads", format!("playlist {id} keeps the songs it had: {e:#}"));
            }
        }
    }

    /// Download lists this build cannot read are set aside and songs sharing a file are written down at once; the
    /// downloads start after a pause, and the remote playlists are asked again then and whenever the library is
    /// opened again with Try again.
    pub async fn keep_collections(&self) {
        self.downloads.set_aside_unreadable();
        self.downloads.note_repairs();
        tokio::time::sleep(self.collections_after).await;
        self.collections.prune();
        self.repair().await;
        let mut remotes = true;
        loop {
            if remotes {
                self.refresh_remotes().await;
            }
            if self.collections.missing().unwrap_or(true) {
                self.ready_ytdlp().await;
                self.collections.sync();
            }
            let retry = async {
                tokio::time::sleep(COLLECTIONS_RETRY).await;
                while self.collections.stalled().is_none() {
                    tokio::time::sleep(COLLECTIONS_RETRY).await;
                }
            };
            remotes = tokio::select! {
                () = self.library_back.notified() => true,
                () = retry => false,
            };
            if remotes {
                self.repair().await;
            }
        }
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

    async fn apply(&self, key: &str) -> Result<()> {
        let lib = &self.library;
        apply_to_net(key, lib, &self.resolver, &self.ytm);
        match key {
            k if k == settings::AUTO_UPDATE.key => self.updates.toggled(),
            k if k == settings::AUDIO_DEVICE.key => {
                self.player().await?.set_device(&settings::AUDIO_DEVICE.or_default::<String>(lib)).await?;
            }
            k if k == settings::LOUDNESS.key => self.player().await?.set_loudness(settings::LOUDNESS.or_default(lib)).await?,
            k if k == settings::VOLUME.key => self.player().await?.set_volume(settings::VOLUME.or_default(lib)).await?,
            k if k == settings::FX.key => self.player().await?.set_fx(settings::FX.or_default(lib)).await?,
            k if k == settings::DISCORD_RPC.key || k == settings::DISCORD_APP_ID.key => {
                if let Some(player) = self.player.get() {
                    player.discord_changed();
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn dispatch(&self, method: &str, a: &Value) -> Result<Value> {
        let failed_reads = self.library.failed_reads();
        let answer = guarded(method, self.answer(method, a)).await;
        let answer = match answer {
            Ok(_) if is(method, |m| m.library_read) && self.library.failed_reads() != failed_reads => {
                let code = self.library.read_failure();
                Err(code.err(format!("reading {method} from the library failed")))
            }
            other => other,
        };
        let Err(e) = answer else { return answer };
        let code = Code::of(&e);
        self.resolver.route().settle(code).await;
        let named = self.resolver.route().blame(code);
        Err(if named == code { e } else { e.context(named) })
    }

    async fn home_page(&self, shelves: Result<Vec<Shelf>>) -> Result<Value> {
        let recent = self.library.recent(RECENT_CARDS);
        let (shelves, problem) = match shelves {
            Ok(shelves) => (shelves, self.recs.home_problem()),
            Err(e) if !recent.is_empty() => (Vec::new(), Some((Code::of(&e), Code::retry_after(&e)))),
            Err(e) => return Err(e),
        };
        let mut page = json!({ "shelves": shelves, "recent": recent });
        if let Some((code, after)) = problem {
            self.resolver.route().settle(code).await;
            page["problem"] = json!(self.resolver.route().blame(code));
            if let Some(after) = after {
                page["retryAfter"] = json!(after);
            }
        }
        Ok(page)
    }

    async fn answer(&self, method: &str, a: &Value) -> Result<Value> {
        if memo_key(method, a)?.is_some() {
            return self.memoized(method, a).await;
        }
        let lib = &self.library;
        Ok(match method {
            "warm" => {
                self.warm(a)?;
                Value::Null
            }
            "home" => self.home_page(self.recs.home().await).await?,
            "homeRefresh" => self.home_page(self.recs.refresh_home().await).await?,
            "libraryRetry" => json!(self.library_again()),
            "libraryContinue" => Value::Null,
            "libraryFolder" => json!(report::reveal(&self.data.join("library.db")).await),
            "cancelStart" => {
                self.player().await?.cancel_start().await;
                Value::Null
            }
            "artistMix" => {
                let artist = self.artist(&arg::<String>(a, "id")?).await?;
                json!(self.source.artist_mix(&artist, arg::<Mix>(a, "kind")?).await?)
            }
            "lyrics" => json!(self.lyrics.get(&track_arg(a, "track")?).await?),
            "readingLyrics" => {
                self.lyrics.shown(arg::<bool>(a, "on")?, opt::<bool>(a, "away")?);
                Value::Null
            }
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
                let context = match given(a, "context") {
                    Some(c) => Some(card(arg(c, "kind")?, arg(c, "item")?)?),
                    None => None,
                };
                self.player().await?.play(tracks, opt(a, "index")?, context).await?;
                Value::Null
            }
            "wave" => {
                let knobs = wave_knobs(self.recs.knobs(), a)?;
                json!(self.recs.wave(&knobs, &[]).await?)
            }
            "daypart" => {
                let part = given(a, "part")
                    .map(|p| serde_json::from_value::<Part>(p.clone()))
                    .transpose()
                    .context("bad part of the day")
                    .context(Code::ApiArgs)?;
                json!(self.recs.daypart(part, opt(a, "reshuffle")?, &[]).await)
            }
            "dislike" => {
                let t = track_arg(a, "track")?;
                lib.dislike(&t);
                self.heart(std::slice::from_ref(&t), |_| false);
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
                self.player().await?.enqueue(tracks, opt(a, "next")?).await?;
                Value::Null
            }
            "toggle" => self.player().await?.toggle().await.map(|_| Value::Null)?,
            "next" => self.player().await?.next().await.map(|_| Value::Null)?,
            "prev" => self.player().await?.prev().await.map(|_| Value::Null)?,
            "seek" => self.player().await?.seek(arg(a, "pos")?).await.map(|_| Value::Null)?,
            "volume" => self.player().await?.set_volume(arg(a, "value")?).await.map(|_| Value::Null)?,
            "jump" => self.player().await?.jump(arg(a, "index")?, opt::<Option<TrackId>>(a, "id")?.as_ref()).await.map(|_| Value::Null)?,
            "removeQueued" => {
                self.player().await?.remove(arg(a, "index")?, opt::<Option<TrackId>>(a, "id")?.as_ref()).await?;
                Value::Null
            }
            "moveQueued" => {
                self.player().await?.move_item(arg(a, "from")?, arg(a, "to")?, opt::<Option<TrackId>>(a, "id")?.as_ref()).await?;
                Value::Null
            }
            "clearQueue" => {
                let player = self.player().await?;
                if !(opt::<bool>(a, "keepCurrent")? && player.clear_others().await) {
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
                self.player().await?.set_fx(fx).await?;
                Value::Null
            }
            "audioDevices" => self.player().await?.devices().await?,
            "getFx" => settings::FX.get::<Value>(lib).unwrap_or_else(|| json!(Fx::default())),

            "like" => {
                let t = track_arg(a, "track")?;
                let liked = lib.toggle_like(&t);
                self.heart(std::slice::from_ref(&t), |_| liked);
                json!(liked)
            }
            "setLiked" => {
                let on: bool = arg(a, "on")?;
                let tracks = one_or_many(a)?;
                let changed = lib.set_liked(&tracks, on);
                self.heart(&tracks, |id| lib.is_liked(id));
                json!(changed)
            }
            "isLiked" => json!(lib.is_liked(&arg(a, "id")?)),
            "favorites" => json!(lib.liked()),
            "recsReport" => {
                let days = match opt::<u64>(a, "days")? {
                    0 => 28,
                    n => n,
                };
                let since = snellar_core::clock::unix_millis() as i64 - (days as i64) * 86_400_000;
                let likes = lib.liked().into_iter().map(|d| (d.item.id, d.at)).collect();
                json!(snellar_core::signals::report(&lib.plays(REPORT_PLAYS), &likes, since))
            }
            "removeFromHistory" => {
                let id = track_id(a, "id")?;
                match self.player.get() {
                    Some(player) => player.remove_from_history(&id),
                    None => {
                        lib.forget_track(&id);
                        self.emit(UiEvent::History);
                    }
                }
                Value::Null
            }
            "clearHistory" => {
                lib.clear_history();
                self.recs.forget();
                Value::Null
            }
            "download" => {
                let tracks = one_or_many(a)?;
                self.ready_ytdlp().await;
                self.collections.claimed(&tracks.iter().map(|t| t.id.clone()).collect::<Vec<_>>())?;
                for t in tracks {
                    self.downloads.start(t);
                }
                Value::Null
            }
            "downloadCollection" => {
                self.ready_ytdlp().await;
                self.download_collection(a).await?;
                Value::Null
            }
            "downloadedCollections" => json!(self.collections.progress()),
            "downloads" => json!(self.downloads.list()),
            "downloadedTracks" => json!(self.library.downloaded()),
            "downloadedSongs" => {
                let songs = lib.downloaded_songs();
                json!(if opt::<bool>(a, "single")? { self.collections.singles(songs) } else { songs })
            }
            PICK_PATH => {
                arg::<String>(a, "title")?;
                return Err(Code::ApiWindowOnly.err("pickPath opens a dialog, so only the window can call it"));
            }
            "history" => json!(lib.played(match opt::<usize>(a, "limit")? {
                0 => 100,
                n => n,
            })),
            "playlists" => json!(lib.playlists()),
            "recentPlaylists" => json!(lib.recent_playlists()),
            "playlistsWith" => json!(lib.playlists_with(&arg(a, "trackId")?)),
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
                let id = lib.restore_playlist(&list)?;
                self.collections.playlist_changed(id);
                json!(id)
            }
            "addToPlaylist" => {
                let (id, tracks) = (arg::<i64>(a, "id")?, one_or_many(a)?);
                let added = lib.add_to_playlist(id, &tracks)?;
                if added > 0 {
                    self.collections.playlist_changed(id);
                }
                json!(added)
            }
            "movePlaylistItem" => json!(lib.move_in_playlist(arg(a, "id")?, &arg(a, "trackId")?, arg(a, "to")?)),
            "editPlaylist" => {
                let id = arg(a, "id")?;
                let old = lib.playlist(id).ok_or_else(|| Code::ApiNotFound.err(format!("no playlist {id}")))?;
                if let Some(name) = opt::<Option<String>>(a, "name")?.filter(|n| !n.trim().is_empty()) {
                    lib.rename_playlist(id, name.trim());
                }
                if let Some(description) = opt::<Option<String>>(a, "description")? {
                    lib.describe_playlist(id, description.trim());
                }
                if let Some(cover) = opt::<Option<String>>(a, "cover")?.filter(|c| *c != old.cover) {
                    let dir = self.data.join("covers");
                    let cover = match cover {
                        c if c.is_empty() || c.starts_with("https://") => c,
                        c if c.starts_with("file://") => covers::reuse(&dir, id, &c)?,
                        _ => return Err(Code::ApiArgs.err("`cover` is an https URL, an earlier cover from `playlistCovers` or empty")),
                    };
                    lib.set_playlist_cover(id, &cover);
                    covers::keep(&dir, id, &old.cover, &cover);
                }
                Value::Null
            }
            "playlistCoverFile" => {
                let id = arg(a, "id")?;
                let old = lib.playlist(id).ok_or_else(|| Code::ApiNotFound.err(format!("no playlist {id}")))?;
                let dir = self.data.join("covers");
                let cover = covers::from_file(&dir, id, Path::new(&arg::<String>(a, "path")?)).await?;
                lib.set_playlist_cover(id, &cover);
                covers::keep(&dir, id, &old.cover, &cover);
                json!(cover)
            }
            "playlistCovers" => {
                let id = arg(a, "id")?;
                let p = lib.playlist(id).ok_or_else(|| Code::ApiNotFound.err(format!("no playlist {id}")))?;
                json!(covers::earlier(&self.data.join("covers"), id, &p.cover))
            }
            "importPlaylist" => {
                let input = match (opt::<String>(a, "url")?, opt::<String>(a, "csv")?, opt::<String>(a, "path")?) {
                    (url, _, _) if !url.trim().is_empty() => Input::Link(url),
                    (_, csv, _) if !csv.is_empty() => Input::Csv { text: csv, name: String::new() },
                    (_, _, path) if !path.is_empty() => Input::file(&path).await?,
                    _ => return Err(Code::ApiArgs.err("give `url`, `csv` or `path`")),
                };
                let job = import::Job {
                    covers: Some(self.data.join("covers")),
                    ..import::Job::new(self.source.clone(), self.library.clone(), self.http.client(), self.ui.clone())
                };
                json!(self.imports.run(job, input, opt::<Option<String>>(a, "name")?).await?)
            }
            "importCancel" => json!(self.imports.cancel()),
            "removeFromPlaylist" => {
                let id = arg(a, "id")?;
                let ids: Vec<TrackId> = if given(a, "trackIds").is_some() { arg(a, "trackIds")? } else { vec![arg(a, "trackId")?] };
                for track in &ids {
                    lib.remove_from_playlist(id, track);
                }
                self.collections.release();
                Value::Null
            }
            "savedAlbums" => json!(lib.saved_albums()),
            "toggleAlbum" => {
                let album: AlbumCard = arg(a, "album")?;
                if lib.album_saved(&album.id)? {
                    self.collections.set("album", &album.id, false, &[])?;
                }
                json!(lib.toggle_album(&album))
            }
            "savedPlaylists" => json!(lib.saved_playlists()),
            "togglePlaylist" => {
                let list: AlbumCard = arg(a, "playlist")?;
                if lib.playlist_saved(&list.id)? {
                    self.collections.set("remote", &list.id, false, &[])?;
                }
                json!(lib.toggle_playlist(&list))
            }
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
            "moveToFolder" => json!(lib.move_to_folder(arg(a, "id")?, opt(a, "folder")?)),
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
                if let Some(reads) = crate::downloads::reads(key)
                    && !setting.keep_aside(lib, reads)
                {
                    return Err(Code::LibraryFailed
                        .err(format!("setting `{key}` holds a value this build cannot read and it could not be kept aside")));
                }
                lib.set_setting(key, &checked(key, value)?.to_string());
                self.apply(key).await?;
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
                let (name, dir) = report::create(&self.data, opt::<Option<String>>(a, "note")?.as_deref(), player)?;
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
                report::attach(&self.data, &arg::<String>(a, "name")?, given(a, "ui"), opt::<Option<String>>(a, "note")?.as_deref())?;
                Value::Null
            }
            "bugReportPack" => {
                let (data, name, note) = (self.data.clone(), arg::<String>(a, "name")?, opt::<Option<String>>(a, "note")?);
                let path = tokio::task::spawn_blocking(move || report::pack(&data, &name, note.as_deref()))
                    .await
                    .context("the packing task died")??;
                let shown = opt::<Option<bool>>(a, "reveal")?.unwrap_or(true) && report::reveal(&path).await;
                json!({ "path": path, "shown": shown })
            }
            "reports" => report::list(&self.data),
            "dependencies" => self.probe().report().await,
            "discord" => {
                json!(crate::deps::discord_state(settings::DISCORD_RPC.or_default(&self.library), &snellar_core::product::runtime_dir()))
            }
            "updateStatus" => json!(self.updates.status()),
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
