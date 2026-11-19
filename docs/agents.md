# Driving Snellar from an AI agent or a script

A running Snellar can be controlled from outside the window. There are three ways in, and all of them end up in the same dispatcher the window itself uses, so they can do what the window does:

| Way in | Start it with | Best for |
|---|---|---|
| MCP server | `snellar mcp` (stdio) | LLM agents that speak MCP: 11 tools with plain-text answers |
| Command line | `snellar ctl <method> '<json>'` | shell scripts, one-off calls, agents that can run commands |
| HTTP on a Unix socket | `curl --unix-socket "$XDG_RUNTIME_DIR/snellar.sock" ...` | anything else: every method, JSON in and out, a live event stream |

The method reference is [api.md](api.md): every method with its arguments and result, every setting and every error code. It is generated from the code, and `snellar ctl --docs` prints the same page for the build you have installed. Decision numbers mentioned there refer to the project's internal design notes, which are not part of this repository.

## Before you start

- **Snellar must be running.** The API lives inside the app. When it is not running, every call fails with `snellar is not running (nothing listens on <path>); start the app first`. Running `snellar` with no arguments starts the app; a second start only brings the open window to the front.
- **The command.** The installer puts a `snellar` command in `~/.local/bin`. If that folder is not on `PATH`, use the full path.
- **The socket** is `$XDG_RUNTIME_DIR/snellar.sock`. Set `SNELLAR_SOCK` to use another path; the app and every client read it.
- **Who can connect.** The socket is created with mode 0600 inside your runtime folder, so only your own user can reach it. There is no TCP port and no token. Browsers cannot reach a Unix socket. An agent in a sandbox or container needs the socket mounted inside it.
- **It acts on the real library.** Likes, playlists, history and settings changed through the API are the person's own. Ask before deleting a playlist, clearing the history or changing settings.

## MCP

`snellar mcp` is an MCP server on stdin and stdout. It needs `XDG_RUNTIME_DIR` (or `SNELLAR_SOCK`) in its environment to find the running app. In a client that takes a JSON config:

```json
{
  "mcpServers": {
    "snellar": { "command": "snellar", "args": ["mcp"] }
  }
}
```

The server starts even when the app is not running; each tool call then answers with the "not running" error above, so the agent can ask the person to open Snellar.

The server sends these instructions on connect: search first, then `play` with a query or a `track_id` from the results, `now_playing` to check, `queue` and `control` to steer.

Answers are short plain text written for a model to read, not JSON. Every track line ends with `(id yt:...)`, which is what `track_id` takes. A track id seen in an earlier answer of the same session is reused as is; any other id is looked up.

### Tools

| Tool | Arguments | What it does and returns |
|---|---|---|
| `search` | `query` (required), `kind`: `songs` (default), `albums`, `artists` or `all`, `limit`: 1 to 25, default 10 | Searches the catalog without touching playback. Returns sections `Songs:` (numbered track lines), `Albums:` (`- Artist - Title Year (album_id ...)`) and `Artists:` (`- Name (artist_id ...)`). |
| `play` | exactly one of `query`, `track_id`, `album_id` | Replaces the queue and starts playing. A query plays the best matching song and queues the other matches after it; `track_id` plays one track; `album_id` plays the album from its first track. Returns `Playing <track>; N more queued`. |
| `play_radio` | `query` or `track_id` | Plays the track and fills the queue with recommendations built from it. |
| `enqueue` | `query` or `track_id`, `next`: `true` to play right after the current track, `false` (default) for the end | Adds one track without interrupting playback. It starts playing only if the queue is empty. |
| `now_playing` | none | The current track, `playing`, `paused` or `loading`, position and length, volume, shuffle, repeat, whether it is liked, and its id. `Nothing is playing.` otherwise. |
| `queue` | `action`: `list` (default), `jump`, `remove`, `move` or `clear`; `index` for jump, remove and move; `to` for move; `limit` for list, 1 to 100, default 20 | `list` shows entries around the current one with their 0-based positions, the current one marked `>`. `jump` plays a position, `remove` drops one, `move` moves `index` to `to`, `clear` empties the queue and stops. |
| `control` | `action`: `play_pause`, `next`, `previous`, `seek`, `volume`, `shuffle` or `repeat`; `value`: seconds for seek, 0 to 100 for volume, `true`/`false` for shuffle, `off`/`all`/`one` for repeat | Transport and playback modes. Returns the same text as `now_playing`. |
| `like` | `track_id` (default: the playing track), `liked`: `true` or `false` (omit to toggle) | Returns `<track> is liked` or `is not liked`. |
| `library` | `what`: `favorites`, `history`, `playlists`, `playlist` (needs `id`) or `albums`; `limit`: 1 to 200, default 25 | Reads the library. Playlists come as `- Name (N tracks, id N)`, albums with their `album_id`. |
| `lyrics` | `track_id` (default: the playing track) | Synced lyrics as `[m:ss] line`, otherwise plain text. |
| `import_playlist` | `link`, or `csv_path` (an absolute path), and an optional `name` | Imports a public playlist or album from Spotify, Deezer, Apple Music, Yandex Music or SoundCloud, or a CSV file from Exportify or TuneMyMusic, into a new local playlist. Can take a few minutes. Returns how many songs were found and lists the ones that were not. |

`track_id` also accepts the bare 11-character YouTube video id, and `video_id` is accepted as another name for it. No tool takes an `artist_id`; artist pages are reached through the socket methods `artist`, `artistSongs`, `discography` and `artistMix`.

### Errors

A failed tool call is a normal tool result with `isError: true` and a sentence that says what to do, not a protocol error. Examples from the code:

- `give exactly one of query, track_id or album_id`
- ``no songs found for `...`; try fewer words or the artist alone``
- ``unknown action `dance`; use play_pause, next, previous, seek, volume, shuffle or repeat``
- ``seek needs `value` (seconds)``
- `nothing is playing` (for `like` and `lyrics` without a `track_id`)

A failure inside the app comes through as its technical text. The stable error code is only available over the socket.

## Command line

```sh
snellar ctl now
snellar ctl search '{"q": "boards of canada", "filter": "albums"}'
snellar ctl setSetting '{"key": "autoplay", "value": false}'
snellar ctl --methods    # every method and its arguments
snellar ctl --docs       # the full reference as Markdown, for this build
```

`snellar ctl <method> '<json>'` sends one call and prints the result as JSON on one line (`reports` and `importPlaylist` are printed as text). The argument is a JSON object and may be left out for methods without arguments. On failure it prints `snellar: <error>` to stderr and exits with 1.

`snellar ctl diag` checks DNS, TCP, TLS and HTTP to each host Snellar needs. It does not need the app to be running.

Shorter commands for people at a terminal:

| Command | Does |
|---|---|
| `snellar now` | what is playing, as text |
| `snellar queue` | the queue, 40 entries around the current one |
| `snellar search <words>` | songs, numbered, with ids |
| `snellar play <words>` | plays the best match and queues the other matches |
| `snellar play` | without words: play or pause |
| `snellar next`, `snellar prev`, `snellar toggle` | transport |
| `snellar vol <0-100>` | volume |
| `snellar like` | toggles the like on the playing track |

## HTTP on the Unix socket

HTTP/1.1 with JSON bodies. Each request gets one answer and the connection closes.

| Request | Answer |
|---|---|
| `POST /call/<method>` with the arguments as a JSON object (an empty body means no arguments) | the method's result |
| `GET /methods` | every method as `{name, group, args, returns, doc}` |
| `GET /now` | the same as `POST /call/now` |
| `GET /events` | a Server-Sent Events stream, below |

The host name in the URL is ignored; the examples use `http://snellar`.

```sh
sock="$XDG_RUNTIME_DIR/snellar.sock"

curl -s --unix-socket "$sock" http://snellar/now

curl -s --unix-socket "$sock" -d '{"q": "roygbiv", "filter": "songs"}' http://snellar/call/search \
  | jq -c '{tracks: .tracks[:5], index: 0}' \
  | curl -s --unix-socket "$sock" -d @- http://snellar/call/play
```

Methods that take a `Track` or `Track[]` (`play`, `enqueue`, `radio`, `like`, `addToPlaylist`, `download`, `lyrics`) expect track objects as `search`, `queue`, `album` and the other methods return them. Pass them on unchanged. If you only have an id, `track` with `{"id": "yt:..."}` returns the full object.

### Errors

| Status | When | Body |
|---|---|---|
| 400 | bad arguments, a body that is not JSON, or a call that failed | `{"error": "...", "code": "...", "retryAfter"?: seconds, "told"?: true}` |
| 404 | unknown method or route | ``{"error": "unknown method `x`, see GET /methods", "code": "api.method"}`` |
| 413 | body over 4 MiB | `{"error": "...", "code": "api.args"}` |

`code` is stable and listed in the error table of [api.md](api.md); decide on it, never on `error`, which is technical text for logs. A few that matter to agents:

- `api.args`: a missing argument, a value of the wrong type, an unknown setting. A wrong type is refused, never read as the default.
- `api.not_found`: the queue item, playlist or report does not exist (any more).
- `api.window_only`: only the window may call this method (`pickPath`, which opens a file dialog).
- `net.*` and `proxy.*`: the network or the proxy failed; `net.youtube_blocked` means YouTube is not reachable while other hosts are.
- `yt.age_restricted`, `yt.removed`, `yt.region`: YouTube will not play this track here; retrying does not help.

`retryAfter` is the number of seconds a host asked to wait (HTTP 429). `told: true` means the window has already shown the failure to the person, so there is no need to tell them again.

### Events

`GET /events` keeps the connection open and sends `event: <name>` and `data: <json>` blocks:

```sh
curl -sN --unix-socket "$XDG_RUNTIME_DIR/snellar.sock" http://snellar/events
```

| Event | Data |
|---|---|
| `snapshot` | `{now, queue}`: the results of `now` and `queue`. Sent on connect, and again if the client fell behind and events were dropped. |
| `state` | `{playing, loading, position, duration, volume}` |
| `position` | seconds, about 5 times a second while playing |
| `current` | the playing `Track`, or null |
| `queue` | `{tracks, cursor}` |
| `modes` | `{shuffle, repeat}` |
| `liked` | whether the playing track is liked |
| `stream` | `{codec, bitrate, sampleRate, gain, leveling, local}` of what plays now |
| `toast` | `{code, cause?, arg?, detail?, track?}`: a short message the window shows, by code |
| `failure` | the current track could not be played: `{id, code, tracks, stopped, retrying}`, or null when that is over |
| `history` | a play was added to the history (no data) |
| `downloads` | `{busy}`: ids queued or downloading |
| `import` | playlist import progress: `{stage, done, total, matched}`, `stage` is `reading` or `matching` |
| `reach` | null, `net.offline` or `net.youtube_blocked` |
| `part` | the part of the day: `morning`, `day`, `evening` or `night` |
| `nudge` | a card the window shows, answered with `nudgeAnswer` |
| `library` | the library file could not be opened: `{code, path, detail, saving}`, or null |
| `tools`, `update`, `desktop` | yt-dlp was fetched or checked; `updateStatus` changed; the desktop's window background support changed |

## Ids and shapes

- **Tracks**: `yt:` and the YouTube video id, as in `yt:dQw4w9WgXcQ`. Arguments also take the bare video id; answers always use the prefix.
- **Albums, artists, YouTube playlists**: the source's own string ids, as `search` returns them.
- **Local playlists and folders**: integers.
- **Track**: `{id, title, artists: [{id, name}], album?: {id, name}, duration, thumb, explicit}` plus a few optional fields. Lists of liked, played, downloaded or playlist tracks add `at`, milliseconds since 1970. The other shapes are in `crates/core/src/model.rs`.

## Recipes

Play an album:

```sh
id=$(snellar ctl search '{"q": "geogaddi", "filter": "albums"}' | jq -r '.albums[0].id')
snellar ctl album "{\"id\": \"$id\"}" | jq -c '{tracks: .tracks, index: 0}' | xargs -0 snellar ctl play
```

Add the playing track to a new playlist:

```sh
pl=$(snellar ctl createPlaylist '{"name": "Found today"}')
snellar ctl now | jq -c --argjson id "$pl" '{id: $id, track: .current}' | xargs -0 snellar ctl addToPlaylist
```

Turn autoplay off and check it:

```sh
snellar ctl setSetting '{"key": "autoplay", "value": false}'
snellar ctl getSetting '{"key": "autoplay"}'
```

Methods that change or delete the person's data: `deletePlaylist` (returns the playlist, which `restorePlaylist` takes back), `removeFromPlaylist`, `clearHistory` and `removeFromHistory` (no undo), `dislike`, `clearQueue`, `setSetting`, `importPlaylist` (makes a new playlist), `download` and `downloadCollection` (write files to the download folder).
