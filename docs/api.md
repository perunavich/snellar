# Snellar API

Generated from `api::METHODS` (`snellar ctl --docs`). Every method takes one JSON object and returns one JSON value. An argument marked `?` may be left out or null for its default; a value of another type is an error (`api.args`), never read as the default.

Call: `POST /call/<method>` on the Unix socket `$XDG_RUNTIME_DIR/snellar.sock` (override: `SNELLAR_SOCK`), or `snellar ctl <method> '<json>'`.

```sh
curl --unix-socket "$XDG_RUNTIME_DIR/snellar.sock" -d '{"q":"daft punk"}' http://snellar/call/search
snellar ctl now
```

Errors: status 400 with `{"error": "...", "code": "..."}` (bad arguments or a failed call), 404 for an unknown method. `error` is technical text for logs and scripts; `code` is stable and listed under Error codes below. The window shows its own translated text for the code, never `error`.

Events: `GET /events` is a Server-Sent Events stream: `snapshot` on connect, then `state`, `position` (about 5 per second while playing), `current`, `queue`, `liked`, `history` (a play was added to the history), `tools` (yt-dlp was fetched or checked), `update` (`updateStatus` has a new answer), `downloads` (the download queue changed: `{busy: string[]}`, the ids queued or downloading), `modes`, `stream` (what plays now: `{codec, bitrate, sampleRate, gain, leveling, local}`, bitrate in kbps and 0 when unknown), `toast` (`{code, cause?, arg?, detail?}`: `code` and `cause` from Error codes below, `arg` a name to show, such as the downloaded track, `detail` technical text), `reach` (whether YouTube can be reached over a direct connection: `null`, `net.offline` when neither YouTube nor any other host answers, `net.youtube_blocked` when YouTube does not answer and another host does; looked at after a failure, then every 5 s until YouTube answers, which sends `null`), `part` (the part of the day in local time, `morning`, `day`, `evening` or `night`, as `daypart` reads it: sent once the backend starts and when it changes, looked at every minute), `nudge` (a card to show: `{id, kind, track?, text?, cover?, timeout?, actions: [{id, primary?}]}`; `kind` `handoff` offers to continue `track`, with the actions `continue` and `ignore`; the window words the card itself; answer it with `nudgeAnswer`), `failure` (the track that plays or starts failed: `{id, code, tracks, stopped, retrying}`, `code` from Error codes below, `tracks` the ids of queued tracks that failed with their codes, `stopped` when playback stopped on it, `retrying` while an automatic retry waits; `null` once that is over), `library` (`null` once the library is open; otherwise `{code, path, detail, saving}`: the library file could not be opened, and `saving` is false while nothing is written to it), `import` (progress of `importPlaylist`: `{stage, done, total, matched}`), `desktop` (what the window needs for its background: `{desktop, frameless, blur, glass, line?, gnome?, reducedMotion, finished?}`; sent when the shell's blur settings change, and with `finished: true` after the window set them up or undid them).

Types: `Track {id, title, artists: [{id, name}], album?: {id, name}, duration, thumb, explicit}`, other shapes are in `crates/core/src/model.rs`.

Track ids name their source: `yt:` and the YouTube video id, as in `yt:dQw4w9WgXcQ`. Every argument that takes a track id or a `Track` also accepts the bare 11-character video id for now; answers always carry the prefixed form. Album, artist and playlist ids are unchanged.

## browse

| method | args | returns | |
|---|---|---|---|
| `search` | `q: string, filter?: all\|songs\|videos\|albums\|artists\|playlists` | `SearchResult` | Search YouTube Music. `top` is the top result as a `Hit` (see `suggest`): an artist, album, playlist, song or video (all filter). `playlists` holds YouTube playlists as cards (all and playlists filters); the playlists filter takes three community playlists, then three of YouTube Music's own, in turn, as the playlist shelf of the all filter does. `videos` holds music videos and uploads with `Artist - Title [tags]` names split into artist and title (all and videos filters). A pasted YouTube link (watch, youtu.be, music.youtube.com, shorts, embed) returns just that track in `linked`; a link to an album, artist or playlist returns `page: {kind, id}` and nothing else. Results are cached for two minutes. |
| `suggest` | `q: string` | `{queries: string[], hits: Hit[]}` | Search-box suggestions for a partial query: completed queries, and the artists, albums, songs and playlists YouTube Music offers for it. A `Hit` is a `Track`, `AlbumCard` or `ArtistCard` with `type`: `track`, `album`, `artist` or `playlist` (an `AlbumCard` whose id is a playlist id). |
| `home` | - | `{shelves: Shelf[], recent: {kind, id, item}[], problem?: string, retryAfter?: int}` | Personalised home shelves (served from cache, refreshed in the background) and what was played lately, without repeats: `kind` is `album`, `remote`, `playlist`, `artist` or, where nothing better exists, `track`; `item` is the card. When YouTube failed and something else can be shown, `problem` is the error code and what is shown is older or partial; with nothing to show the call fails with that code. |
| `homeRefresh` | - | `{shelves: Shelf[], recent: {kind, id, item}[], problem?: string, retryAfter?: int}` | Rebuild the home shelves now. A failure never replaces what was kept. |
| `album` | `id: string` | `Album` | Album or single with its tracks. `id` is an AlbumCard id from search. Pages (album, artist, discography, artistSongs, remotePlaylist) are answered from memory for two minutes; for ten minutes the window shows the kept page at once and asks again, a repeat after two minutes goes to the network and waits for one of two background slots. |
| `artist` | `id: string` | `Artist` | Artist page: top songs, albums, singles, related artists. |
| `discography` | `id: string` | `Discography` | Every album and every single or EP of an artist: `{artist: {id, name}, albums: AlbumCard[], singles: AlbumCard[]}` in the source's order, read through every page. An artist without a full list gives what its page shows. |
| `artistSongs` | `id: string` | `Track[]` | All popular songs of an artist, every page of the list behind Top songs; the top songs of the page when the artist has no such list. |
| `artistMix` | `id: string, kind: radio\|shuffle` | `Track[]` | The artist's own mix from the source: `radio` is the artist radio (the artist and similar ones), `shuffle` the artist's songs in a new order each time. Empty when the source has none; the window then falls back to the radio of the top song or a shuffle of `artistSongs`. |
| `remotePlaylist` | `id: string` | `Album` | A YouTube Music playlist with all its tracks, read through every page. Kept like `album`. |
| `warm` | `method: album\|artist\|remotePlaylist, id: string` | `null` | Fetch a page in the background because the person is likely to open it next (a song menu is open, the pointer rests on a link). Returns at once. Nothing happens when the page is already fresh in the cache, already being fetched, or two background page requests are running; a failure is dropped. |
| `lyrics` | `track: Track` | `Lyrics\|null` | Synced lines when available, else plain text; null when none is found. Asked of lrclib, NetEase, Kugou and YouTube Music at once; when YouTube Music has the text of this very video, only lyrics that agree with it are shown; synced lyrics in another script than that text are not judged by it and come before it; without it the lyrics most services agree on, and null when the most backed texts differ (decision 0094). Answers of the last 20 tracks are kept in memory: found lyrics for 30 minutes, none found for 3 minutes (not kept for a track without a length), failures not at all; asks for a track already being fetched wait for that fetch. |
| `readingLyrics` | `on: bool, away?: bool` | `null` | The window says whether a lyrics view is shown in a window on screen. While it is, and for 5 minutes after the last one closes (at once when it closes because the window was hidden or minimized: `away`), the lyrics of the next track in the queue are fetched ahead, one track, 3 s after the queue settles and never while a track is starting, so they show at once when it plays. Otherwise no track is sent to the lyrics services until its lyrics are asked for (decision 0094). |
| `link` | `kind: track\|album\|artist\|playlist, id: string` | `string\|null` | A link to share: the music.youtube.com page of a track id, album, artist or YouTube playlist id; null when there is none (a local playlist, an id the source does not know). |
| `track` | `id: string` | `Track` | Metadata for a track id: from the library when known, otherwise from YouTube Music. |
| `daypart` | `part?: morning\|day\|evening\|night, reshuffle?: bool` | `Shelf` | The pick for a part of the day (default: now, in local time; morning 5-11, day 11-17, evening 17-22, night 22-5, and while a listening session that began in the night goes on without a 4-hour break, the morning is still night), the same shelf as the first one of `home` (`kind` `daypart`). With enough listening at that time in the last 8 weeks (12 songs heard to the end, with someone at the computer, on 3 days, from 4 artists; weekdays and weekends apart when each has enough) it is built from those songs and `pick` names them: `{part, habits, habit: true, weekend, artists, plays, days}`, `artists` the three heard most, `habits` the parts of the day that have enough. Otherwise it is the Wave with `habit: false`. Kept 30 minutes; `reshuffle` builds a new one that leaves out the songs of the one kept. |
| `wave` | `mood?: string, language?: uk\|ru\|en\|ko\|ja\|zh, novelty?: number, limit?: int, reshuffle?: bool` | `Track[]` | Wave preview, the same list as the first track shelf of `home`: tracks built from the library's taste around the YouTube Music radio, kept for 30 minutes. Each argument given overrides that one knob of the saved `wave` setting for this call; the setting itself is not changed. `reshuffle` builds a new list that leaves out the one kept before (the New mix button). `novelty` is the share of unfamiliar artists, 0 to 1. |

## playback

| method | args | returns | |
|---|---|---|---|
| `nudgeAnswer` | `id: string, action: string` | `null` | Answer a nudge card. For the handoff offer `continue` pauses the other player and continues here from the same position, `ignore` drops it; a late or unknown id does nothing. |
| `play` | `tracks: Track[], index?: int, context?: {kind, item}` | `null` | Replace the queue with `tracks` and start playing at `index` (default 0). `context` names the collection they come from (`kind` as in `pin`, `item` its card), so that it shows in the recent cards on `home`. |
| `radio` | `track: Track` | `null` | Play `track` now and fill the queue with recommendations built from it. |
| `prefetch` | `id: string` | `null` | Warm the stream for a track id so playing it starts instantly. Call on hover or selection. |
| `enqueue` | `track?: Track, tracks?: Track[], next?: bool` | `null` | Add a track, or several in their order (an album, a playlist), after the current one (`next: true`) or at the end. Starts playing only when the queue has no current track (it was empty or cleared); after the queue played to its end the added tracks wait in the queue for `next` or `jump`, as in Spotify. |
| `cancelStart` | - | `null` | Stop waiting for the track that is starting, or for its next automatic retry. |
| `toggle` | - | `null` | Play or pause. |
| `next` | - | `null` | Next track; at the end of the queue autoplay appends recommendations. |
| `prev` | - | `null` | Restart the track if more than 3 s in, otherwise the previous one. |
| `seek` | `pos: number` | `null` | Seek to an absolute position in seconds. |
| `volume` | `value: number` | `null` | Volume 0 to 100. |
| `jump` | `index: int, id?: string` | `null` | Play the queue item at `index`. With `id` the track is found by id (nearest to `index`) even if the queue shifted since the list was read; it fails if the track left the queue. |
| `removeQueued` | `index: int, id?: string` | `null` | Remove the queue item at `index` (or the track `id` nearest to it); removing the current track plays the next. Removed tracks are not re-added by autoplay. |
| `moveQueued` | `from: int, to: int, id?: string` | `null` | Move a queue item (found by `id` when given); the current track stays current. |
| `clearQueue` | `keepCurrent?: bool` | `null` | Empty the queue and stop. With `keepCurrent` the playing track stays and keeps playing, everything else goes and is not brought back by autoplay. |
| `shuffle` | `on: bool` | `null` | Shuffle the upcoming tracks; turning it off restores the original order. |
| `repeat` | `mode: off\|all\|one` | `null` | Repeat mode. |
| `fx` | `fx: Fx` | `null` | Apply and save audio effects (EQ, bass, reverb, speed...). |
| `getFx` | - | `Fx` | Saved audio effects. |
| `audioDevices` | - | `{devices: {name, description}[], wanted, active}` | Output devices to offer: one per real output, from the sound server when there is one (decision 0070), without mpv's `auto`. `wanted` is the entry the saved choice (`audioDevice` setting) plays on, or the saved name while it is not connected; `active` is the device in use, `auto` while the saved one is not connected. |
| `now` | - | `{current: Track\|null, playing, loading, position, duration, volume, shuffle, repeat, stream, liked}` | Snapshot of what is playing. |
| `queue` | - | `{tracks: Track[], cursor: int}` | The queue; `cursor` is the current index or -1. |

## library

| method | args | returns | |
|---|---|---|---|
| `like` | `track: Track` | `bool` | Toggle a like; returns the new state. |
| `setLiked` | `on: bool, track?: Track, tracks?: Track[]` | `int` | Like (`on`) or unlike every given track, whatever its state was; returns how many changed. Liking takes a dislike back. |
| `isLiked` | `id: string` | `bool` | Whether a track id is liked. |
| `favorites` | - | `Track[]` | Liked tracks, newest first; `at` is when each was liked (ms since 1970). |
| `dislike` | `track: Track` | `null` | Mark a track disliked (removes its like); recommendations avoid the track and its artist. |
| `undislike` | `id: string` | `null` | Take back a dislike; a like it replaced is not restored. |
| `dislikes` | - | `Track[]` | Disliked tracks, newest first. |
| `history` | `limit?: int` | `Track[]` | Recently played distinct tracks (default 100); `at` is when each was last played (ms since 1970). |
| `clearHistory` | - | `null` | Delete play history (likes are kept). |
| `removeFromHistory` | `id: string` | `null` | Remove every play of a track id from the history and from what recommendations learn (taste, habits, `recsReport`), as if it was never played. When it is the song that plays, it plays on and this play is not written either. Likes and playlists are kept. |
| `recsReport` | `days?: int` | `{via, started, plays, earlySkips, completed, liked, repeated, failed, away}[]` | How recommendations fared on this computer over the last `days` (default 28), from the local history only: per source (`user` is your own picks, the baseline; `wave`, `radio`, `autoplay`, `because`, `daypart`, `shelf`; `before signals` are plays an older build recorded) and per how the play started (`chosen` by you, or `itself` after the previous one). Rates are shares of the plays that ended (failures, quits and stops are left out): `earlySkips` skipped within 30 s of time heard, `completed` heard to 80 %, `liked` liked within 7 days, `repeated` played again within 7 days; `failed` counts plays that failed; `away` is the share of time heard while the computer was idle or locked. |
| `pickPath` | `title: string, folder?: bool, filters?: string[], start?: string` | `{status: picked\|cancelled\|unavailable, path?}` | Ask the desktop's file chooser portal for a file or a folder. Only the window can call this: a dialog is never opened from the socket or MCP, which get `api.window_only`. `filters` read like `CSV files (*.csv)`. `unavailable` means there is no portal or it failed, and the window then shows its own dialog. |
| `download` | `track?: Track, tracks?: Track[]` | `null` | Download best audio of a track, or of several, to the downloads folder in the background; a toast reports each result. Downloads wait in one queue, two at a time, ahead of the songs of downloaded collections; none starts while the folder's disk has less than 1 GiB free (`download.no_space`). A finished file is named `Artist - Title [video id]` and linked to its track, which then plays from the file without the network; a file is deleted only when no downloaded track uses it. At start, songs that share one file with another song (downloads named before the video id was in the name) download again quietly, each into its own file; the shared file goes once no song uses it. |
| `downloadCollection` | `kind: playlist\|remote\|album, id: string\|int, on: bool` | `null` | Keep a playlist, a saved YouTube playlist or an album downloaded (`on`), or stop. On, its songs that have no file join the download queue behind single downloads, quietly, without a toast each; a remote playlist or album is read from the source and saved to the library if it was not. While on, songs added to a local playlist download too, and a removed song loses its file unless another downloaded collection or a single download keeps it; a saved YouTube playlist is read again at start. Off asks nothing: queued songs are dropped and the files go, except those another downloaded collection or a single download keeps. A network failure, a missing yt-dlp or a full disk stops the queue and it is tried again every minute; a song the source refuses is skipped until the next start. Unsaving a remote playlist or album turns it off. When the library cannot be read it fails with that `library.*` error and nothing changes; no file is ever removed on a failed read. `downloads` events say what is queued. |
| `downloadedCollections` | - | `{kind, id, total, done, refused, busy, stalled?}[]` | Collections kept downloaded, in the order they were turned on. `done` songs have a file, `refused` were refused by the source in this run, `busy` are queued or downloading; `stalled` is the code that stopped the queue (a `net.*` or `proxy.*` code, `ytdlp.missing` or `download.no_space`) while the collection is not complete. Complete is `done + refused == total`. |
| `downloads` | - | `{name, path, size, modified}[]` | Finished downloads, newest first. |
| `downloadedTracks` | - | `string[]` | Ids of tracks linked to a downloaded file that still exists. |
| `downloadedSongs` | `single?: bool` | `Track[]` | Songs with a downloaded file that is still there, the last downloaded first; `at` is when it was downloaded (ms since 1970). A song the library has no details for is named after its file. With `single`, only the songs downloaded on their own: a song a downloaded collection pulled in is left out unless it was also asked for by hand. |
| `playlists` | - | `Playlist[]` | Local playlists with counts and a cover (tracks omitted). |
| `recentPlaylists` | - | `Playlist[]` | Local playlists like `playlists`, the one last added to first. |
| `playlistsWith` | `trackId: string` | `int[]` | Ids of the local playlists that hold a track, lowest first; one query over every playlist, for the add-to-playlist menus to tick them. |
| `playlist` | `id: int` | `Playlist\|null` | One local playlist with its tracks in order; `at` on each track is when it was added (ms since 1970). |
| `createPlaylist` | `name: string` | `int` | Create a playlist; returns its id. |
| `renamePlaylist` | `id: int, name: string` | `null` | Rename a playlist. |
| `deletePlaylist` | `id: int` | `Playlist\|null` | Delete a playlist; returns it with its tracks (null if there was none), which `restorePlaylist` takes to undo. |
| `restorePlaylist` | `playlist: Playlist` | `int` | Bring back a deleted playlist from what `deletePlaylist` returned: its name and tracks in order with their `at`, under its old id when that is free. Returns the id. |
| `addToPlaylist` | `id: int, track?: Track, tracks?: Track[]` | `int` | Append a track, or several in their order; a track already in the playlist is skipped. Returns how many were added. |
| `movePlaylistItem` | `id: int, trackId: string, to: int` | `bool` | Move a track inside a playlist to index `to` (0 is first; past the end is last). False when the track is not in it. |
| `editPlaylist` | `id: int, name?: string, description?: string, cover?: string` | `null` | Change a playlist's name, description or cover; a field left out stays. `cover` is an image URL (https), one of the playlist's earlier covers from `playlistCovers`, or empty to use the first track's cover again. A local cover it replaces becomes an earlier cover. |
| `playlistCoverFile` | `id: int, path: string` | `string` | Copy a local PNG, JPEG or WebP image (at most 20 MB, its longer side reduced to 1024 pixels) into the data directory as the playlist's cover; returns its `file://` URL. The local cover it replaces becomes an earlier cover. |
| `playlistCovers` | `id: int` | `string[]` | The playlist's earlier local covers as `file://` URLs, the one shown last first, at most 7; older ones are deleted. |
| `importPlaylist` | `url?: string, csv?: string, path?: string, name?: string` | `ImportResult` | Import a playlist into a new local playlist: a public playlist or album link from Spotify, Deezer, Apple Music, Yandex Music or SoundCloud (`url`), CSV text (`csv`) or a CSV file (`path`) as Exportify and TuneMyMusic write them. Each song is looked up on YouTube Music by title, artist and length, four at a time; `import` events report `{stage: reading|matching, done, total, matched}`. Returns `{playlist, name, total, matched, unmatched: ImportTrack[], merged: ImportTrack[], hidden: int[], limited, sourceTotal, failed, code?}`: `playlist` is the new id, null when nothing was found; `matched` counts the songs added: rows that found a song already found for an earlier row are added once and listed in `merged`; `hidden` holds the places (from 1) of rows the service lists without a name, which Spotify does for songs not available in this country; `failed` counts the songs that could not be looked up because a search failed, and `code` says why; those are not in `unmatched`, which holds only songs that were looked for and not found; `ImportTrack` is `{title, artists, album, duration}`; `limited` means only part of the list could be read: Apple Music pages show at most 100 songs, and a long Spotify playlist is paged through Spotify's private web player API with the token its embed page issues, and stays at the embed's 100 songs or at the pages already read when that API refuses or changes; `sourceTotal` is the length of the whole list when the source reports it. `name` overrides the playlist name. The source's own cover, when it has one, is saved as the playlist's cover; a cover that cannot be fetched does not stop the import. A newer import cancels a running one. Fails with `import.blocked` when the service hides the playlist (private, sign-in, captcha, 403, 451): export it to CSV then. |
| `importCancel` | - | `bool` | Cancel the running import; it then fails with `import.cancelled` and makes no playlist. False when none runs. |
| `removeFromPlaylist` | `id: int, trackId?: string, trackIds?: string[]` | `null` | Remove a track, or several, from a playlist. |
| `savedAlbums` | - | `AlbumCard[]` | Saved albums, newest first. |
| `toggleAlbum` | `album: AlbumCard` | `bool` | Save or unsave an album; returns the new state. |
| `savedPlaylists` | - | `AlbumCard[]` | YouTube playlists saved to the library, newest first (`kind` is `Playlist`, `id` the playlist id for `remotePlaylist`). |
| `togglePlaylist` | `playlist: AlbumCard` | `bool` | Save or unsave a YouTube playlist; returns the new state. The tracks are not copied: the playlist is read again when opened. |
| `followed` | - | `ArtistCard[]` | Followed artists, newest first. Kept in the local library only. |
| `follow` | `artist: ArtistCard` | `bool` | Follow or unfollow an artist; returns the new state. |
| `library` | - | `{kind, id, item, at}[]` | Everything kept in the library, newest first: local playlists (`playlist`, `item` a `Playlist` without tracks), saved YouTube playlists (`remote`), albums (`album`) and followed artists (`artist`); `at` is when each was made or saved (ms since 1970). |
| `folders` | - | `{id, name, open}[]` | Playlist folders in the order they were made; `open` is whether the sidebar shows their playlists. A playlist names its folder in `folder`. |
| `createFolder` | `name: string` | `int` | Make a playlist folder; returns its id. |
| `renameFolder` | `id: int, name: string` | `null` | Rename a folder. |
| `openFolder` | `id: int, open: bool` | `null` | Remember whether a folder is open in the sidebar. |
| `deleteFolder` | `id: int` | `null` | Delete a folder; its playlists stay, outside any folder. |
| `moveToFolder` | `id: int, folder?: int\|null` | `bool` | Put a playlist in a folder, or out of any folder when `folder` is null or left out. False when the playlist or the folder does not exist. |
| `pins` | - | `{kind, id, item}[]` | What is pinned to the top of the sidebar, in the order it was pinned. `kind` is `playlist` (a local playlist, `id` its number as text), `remote` (a saved YouTube playlist), `album` or `artist`; `item` is the card as it was pinned. |
| `pin` | `kind: string, item: object` | `bool` | Pin or unpin an item to the sidebar; returns the new state. `item` is the card (`id` required); for a local playlist `{id}` is enough. |

## settings

| method | args | returns | |
|---|---|---|---|
| `getSetting` | `key: string` | `any` | Read a setting from the table below (null if unset). An unknown key is an error. |
| `setSetting` | `key: string, value: any` | `null` | Write a setting from the table below. An unknown key or a value of the wrong kind is an error; null clears it to the default. A saved value this build cannot read (`library.corrupt`) is copied to the first free `<key>.broken`, `<key>.broken.2`, … before it is replaced. The app does the same by itself at start for a download list it cannot read, resets that list and shows `download.list_set_aside`; a preference is never reset that way. |

## meta

| method | args | returns | |
|---|---|---|---|
| `libraryRetry` | - | `bool` | Try to open the library file again when it could not be opened; true when it opened. It answers before the rest of the app has started. |
| `libraryContinue` | - | `null` | Run without saving after the library could not be opened: everything in this session is lost at quit. Only the window asks for this. |
| `libraryFolder` | - | `bool` | Show the data folder with the library file in the file manager. |
| `raise` | - | `null` | Show and raise the window. A second start of the app calls this on the running one and exits. |
| `methods` | - | `Method[]` | This table. |
| `bugReport` | `note?: string` | `{name, dir}` | Freeze a bug report now: the window shot, the journal of the last five minutes, the player state, the open screen and the window size go into `reports/<date-time>/` in the data directory, and a card with a one-line field opens in the window (Enter stores the phrase, Esc keeps the report without one). Music is not interrupted. Without a window only the Rust half is saved. |
| `bugReportAttach` | `name: string, ui?: object, note?: string` | `null` | Add what the window saw (`ui`) or the phrase (`note`) to the report `name`. The window calls this itself after `bugReport`. |
| `bugReportPack` | `name: string, note?: string, reveal?: bool` | `{path, shown}` | Pack the report `name` into `reports/<name>.zip` and, unless `reveal` is false, show that file in the file manager. `note` is stored first, as in `bugReportAttach`. `shown` says whether a file manager was asked to show it. The archive holds the screenshot and the track list, so it shows what the person listens to; nothing is sent anywhere. |
| `dependencies` | - | `{desktop, components: {id, version, source, state}[], details}` | What Snellar runs on here and whether it really works: mpv, yt-dlp, ffmpeg, Qt, audio output, Discord, file portal, tray, window blur. `state` is ok, missing, broken, off, on, waiting or none. Reads only: nothing is installed and the desktop is not changed, here or anywhere else in this API. `details` is the same list as text, as bug reports hold it. |
| `discord` | - | `off\|ok\|waiting` | Whether the song playing can show in Discord: `off` when the `discordRpc` setting is off, `ok` when Discord is running, `waiting` when the setting is on and Discord is not running; Snellar connects by itself once it opens. |
| `updateStatus` | - | `{phase, version, checked, latest, percent, code?, place}` | How the self-update stands. `phase` is `development` (not an AppImage), `elsewhere` (an AppImage outside the installer's folder, named in `place`), `unsigned` (an installed AppImage built without an update key, it never checks), `off` (the `autoUpdate` setting), `idle`, `current` (a check found nothing newer), `checking`, `downloading` (`percent`), `ready` (`latest` starts at the next launch) or `failed` (`code` says why, the installed file was not touched, the next check tries again). `version` is this build for people; `checked` is the last time the release was read, in ms since 1970, 0 for never. Reads only: a check is never started from here. |
| `reports` | - | `Report[]` | Saved bug reports, newest first: `{name, path, note, track, screen, files}`. `snellar ctl reports` prints them as lines. |

## Settings

Generated from `settings::ALL` in `crates/core/src/settings.rs`. Values are JSON. `getSetting` returns null for an unset key; the default is what the app uses then. `*` matches any non-empty suffix. Scope `state` is saved by the app itself, not a preference. Section is where the Settings page shows it; none means the page does not.

| key | kind | default | scope | section | |
|---|---|---|---|---|---|
| `language` | text | `""` | ui | general | Interface language code; empty follows the system. |
| `accent` | text | `"glass"` | ui | appearance | Accent palette name, or `cover` to follow the cover. |
| `glass` | text | `"auto"` | ui | appearance | Window background: `glass` (see-through, the compositor blurs), `cover` (opaque, the blurred cover behind), `solid`, or `auto` to choose by desktop. |
| `backgroundDim` | number | `0.5` | ui | appearance | How dark the Cover and Solid window backgrounds are, from 0 (lightest) to 1 (darkest); 0.5 is the standard look. Glass ignores it. |
| `navRight` | bool | `false` | ui | appearance | Sidebar on the right, the layout mirrored. |
| `reduceMotion` | bool | `false` | ui | appearance | Keep decorative motion still: the sky behind the Home card does not breathe, drift or twinkle, and the sidebar folds without moving. Also follows the desktop's own reduced-motion setting, read through the desktop portal. |
| `sidebarCollapsed` | bool | `false` | ui | - | The sidebar is a narrow strip of icons, chosen with the button at its top. A window too narrow for the whole sidebar shows the strip without changing this. |
| `sort.*` | json | `null` | ui | - | Sort of an own list (`favorites`, `history`, `playlist`, `albums`, `artists`): `{key, reversed}`. |
| `timeLeft` | bool | `false` | ui | - | The number at the right end of the seek bar shows the time left, as `-2:15`, not the track's length; a click on it switches. |
| `volume` | number | `85` | player | - | Volume, 0 to 100. |
| `fx` | json | `{}` | player | - | Equalizer and effects (`Fx`); missing fields take their defaults. |
| `autoplay` | bool | `true` | player | playback | Keep playing related tracks after the queue ends. |
| `recsLive` | bool | `false` | app | playback | Recommendations may offer live recordings as they are; off, the studio original is looked up instead. |
| `recsCovers` | bool | `false` | app | playback | Recommendations may offer covers, tributes and karaoke as they are; off, the original is looked up instead. |
| `recsRemixes` | bool | `false` | app | playback | Recommendations may offer remixes as they are; off, the original is looked up instead. |
| `recsSpeed` | bool | `false` | app | playback | Recommendations may offer sped-up, slowed, nightcore, reverb, 8D and bass-boosted versions as they are; off, the original is looked up instead and the version is left out when there is none, in every list. |
| `recsSpam` | bool | `true` | app | playback | Recommendations leave out likely AI and spam artists: a title, channel or artist name that says it was made by AI, and artists with at least 200 releases and fewer than 1000 fans on Deezer. Artists you liked, played, listed or follow are never left out. |
| `audioDevice` | text | `"auto"` | player | audio | Output device: a `name` from `audioDevices`, or `auto`. A device that is not connected plays on `auto` until it is back. |
| `quality` | text | `"auto"` | player | audio | Stream quality: `auto` (the best Opus stream up to 160 kbps), `high` (the highest bitrate of any codec), `dataSaver` (the lowest bitrate). Applies from the next track. |
| `loudness` | bool | `true` | player | audio | Even out loudness between tracks: YouTube's loudness value lowers a loud track, a track without one is measured by mpv's `loudnorm` on its first play and plays with the gain from that measure after. Never raises a track above its own level. |
| `closeToBackground` | bool | `false` | ui | general | Closing the window keeps the music playing, with an icon in the tray to show the window again; only where a tray (StatusNotifierItem host) is running. |
| `queue` | json | `null` | state | - | The saved queue and cursor. |
| `position` | number | `0` | state | - | Seconds into the current track at the last save. |
| `handoff` | bool | `false` | app | playback | Offer to continue what another MPRIS player plays. |
| `discordRpc` | bool | `false` | app | services | Show the current track in Discord. |
| `discordAppId` | text | `""` | app | - | A custom Discord application id; empty uses Snellar's. |
| `downloadDir` | text | `""` | app | library | Download folder: a full path or one that starts with ~/; empty is ~/Music/Snellar. A relative path is refused with `download.folder_invalid`. |
| `downloadedCollections` | json | `{}` | state | - | Playlists and albums kept downloaded, and the songs downloaded only for them: `{collections: [{kind, id, tracks?}], pulled: string[]}`. Written by `downloadCollection`. |
| `downloadRepairs` | json | `[]` | state | - | Songs whose downloaded file was shared with another song (saved before the video id was in file names) and that still download again into their own file: `string[]` of track ids. Written by the downloads at start. |
| `downloadRepairsRefused` | json | `[]` | state | - | Songs from `downloadRepairs` YouTube refused for good (removed, private, age-restricted) or that are no YouTube track: `string[]` of track ids. Their repair is not tried again while they still share a file. Written by the downloads. |
| `lastfmKey` | text | `""` | app | services | Last.fm API key for similar artists. |
| `proxy` | text | `""` | app | services | HTTP proxy for every request Snellar makes, yt-dlp included (`http://host:port`); empty uses the system's `HTTPS_PROXY`. Applies at once. |
| `region` | text | `""` | app | - | YouTube Music region code; empty lets YouTube choose. |
| `ytdlpFallback` | bool | `true` | app | - | Use yt-dlp when a stream cannot be resolved directly. |
| `ytdlpChecked` | number | `0` | state | - | When Snellar last compared its yt-dlp with the latest release, in milliseconds. |
| `glassUndo` | json | `null` | state | - | What the Blur my Shell setup changed, kept for its undo. Written only by the window's setup button. |
| `autoUpdate` | bool | `true` | app | about | Keep an installed Snellar up to date: at most once a day the latest release is fetched in the background and starts at the next launch. |
| `updateChecked` | number | `0` | state | - | When Snellar last read the latest release of itself, in milliseconds. |
| `recsExplore` | number | `0.3` | app | - | Share of unfamiliar artists in recommendations, 0 to 1. |
| `wave` | json | `{}` | app | - | Saved wave knobs: `{mood?, language?, novelty?, limit?}`. |
| `recentSearches` | json | `[]` | state | - | What the empty search field offers again, newest first: `{query}` for a search that was run, `{hit}` (a `Hit`) for a suggestion that was opened. The window keeps at most 8 and writes the whole list. |

Renamed keys still read and write: a stored old value is read through the new key until the new key is written, and `setSetting` with the old key writes the new one.

| old key | new key | |
|---|---|---|
| `glassTinted` | `glass` | `true` (tinted) reads as `cover`, `false` (clear) as `glass` |

## Error codes

Generated from `error::ALL` in `crates/core/src/error.rs`. A code never changes meaning; a new failure gets a new code. Codes marked toast only appear as the `code` of a `toast` event.

| code | meaning |
|---|---|
| `net.offline` | the network is unreachable, or the request could not be sent |
| `net.dns` | the host name could not be resolved: no network, or DNS failed or is blocked |
| `net.connect` | the host refused or reset the connection |
| `net.tls` | the TLS handshake failed or the certificate was rejected |
| `net.timeout` | the host did not answer in time, or the stream stalled |
| `net.blocked` | the host answered 403 or 451: requests from this network are refused |
| `net.refused` | the host answered with a 4xx status other than 403, 429 and 451 |
| `net.limited` | the host answered 429, too many requests |
| `net.server` | the host answered with a 5xx status |
| `yt.parse` | a YouTube answer is not JSON or lacks what the parser needs |
| `yt.unavailable` | YouTube will not play this track (playability status) and no fallback could |
| `ytdlp.missing` | yt-dlp cannot be started |
| `ytdlp.failed` | yt-dlp ran and failed |
| `library.locked` | SQLite reports the library busy or locked by another process |
| `library.newer` | the library has a newer schema than this build; it is left untouched |
| `library.failed` | any other library or migration failure |
| `audio.failed` | mpv failed to start, exited or refused a command |
| `api.args` | a missing or invalid argument, an unknown setting or a value of the wrong kind |
| `api.method` | an unknown API method or route |
| `api.not_found` | the queue item, playlist or report asked for does not exist |
| `api.window_only` | only the window itself may ask for this; it opens a dialog in front of the person |
| `source.unsupported` | the source cannot do this, or the track belongs to another source |
| `internal` | anything not classified above |
| `play.failed` | toast: the current track cannot be played; `cause` says why |
| `autoplay.failed` | toast: radio top-up failed at the end of the queue; `cause` says why |
| `fx.reset` | toast: the saved audio effects were rejected by mpv and reset |
| `audio.restart` | toast: mpv stopped, it restarts on the next play |
| `library.unsaved` | toast: the library cannot be opened, the app runs on memory; `cause` says why |
| `download.done` | toast: `arg` was downloaded |
| `download.failed` | toast: the download of `arg` failed; `cause` says why |
| `cover.unreadable` | the image chosen as a playlist cover is not a PNG, JPEG or WebP file of at most 20 MB, or cannot be read |
| `import.blocked` | the service refused to show the playlist: 401, 403, 404 or 451, a private playlist, a login wall or a captcha |
| `import.unreadable` | the playlist page answered but holds no track list Snellar can read |
| `import.unsupported` | the link is not a Spotify, Deezer, Apple Music, Yandex Music or SoundCloud playlist or album |
| `import.columns` | the CSV file has no column with track names |
| `import.empty` | the playlist or file has no tracks |
| `shell.extension` | GNOME Shell could not install the extension, or does not offer to |
| `import.file` | the file cannot be read as UTF-8 text of at most 5 MB |
| `import.cancelled` | the import was cancelled, or replaced by a newer one |
| `proxy.invalid` | the proxy setting is not an http:// or https:// address with a host |
| `download.no_tool` | a download needs yt-dlp, it is not there and could not be fetched |
| `ytdlp.fetching` | yt-dlp is being fetched, the percentage is in `arg` |
| `ytdlp.ready` | yt-dlp was installed or updated |
| `ytdlp.current` | yt-dlp was asked to update and is the latest release |
| `ytdlp.fetch_failed` | the button could not fetch yt-dlp, the cause says why |
| `shell.blur` | GNOME Shell could not install or start Blur my Shell, or its settings did not take |
| `shell.blur_skipped` | Blur my Shell was not installed, usually because the person said no in GNOME's dialog; nothing changed |
| `shell.blur_undo` | the Blur my Shell settings could not be put back |
| `ytdlp.checksum` | the downloaded yt-dlp does not match the checksum its release publishes, nothing was installed |
| `proxy.auth` | the proxy answered 407: it wants a login |
| `proxy.failed` | the proxy answered with an error instead of opening the connection, such as 502 |
| `proxy.down` | a proxy is in use and it cannot be reached, or it cut the connection |
| `proxy.timeout` | a proxy is in use and no answer came through it in time |
| `proxy.blocked` | a proxy is in use and the host answered 403 or 451 through it |
| `library.corrupt` | the library file is not a readable database; it is left untouched |
| `update.no_release` | the latest release does not name a version Snellar can read; nothing was fetched |
| `update.checksum` | the downloaded Snellar does not match the checksum of its release, the installed one stays |
| `update.broken` | the downloaded Snellar does not unpack, the installed one stays |
| `update.place` | the new Snellar could not be written next to the installed one, the installed one stays |
| `update.signature` | the release's checksum file is not signed by the key built into this Snellar, or the signature is missing; nothing was fetched, the installed one stays |
| `download.no_space` | the disk of the download folder has less than 1 GiB free; no download starts until there is room |
| `download.folder_invalid` | the download folder given is relative or climbs out with ..; it must start with / or ~/ |
| `yt.age_restricted` | YouTube plays this track only to a signed-in adult (age restriction); retrying cannot help |
| `yt.removed` | YouTube says the track was removed, is private or does not exist; retrying cannot help |
| `yt.region` | YouTube does not offer the track in this country; retrying cannot help until the route changes |
| `net.youtube_blocked` | YouTube cannot be reached over a direct connection while other hosts answer: the network or the country blocks it; a proxy or a VPN may help |
| `download.list_set_aside` | toast: a saved download list this build cannot read was kept as `<key>.broken` (or `.broken.2`, …) and started again empty; downloaded files stay |

