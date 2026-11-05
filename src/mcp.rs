use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use snellar_core::ids::TrackId;
use snellar_core::product::socket_path;

use crate::client::call_at;
use crate::render;

const INSTRUCTIONS: &str = "Controls a running Snellar music player. Typical flow: `search` to find tracks, `play` with a query or a track_id from the results, `now_playing` to check, `queue` and `control` to steer. Track ids look like `yt:dQw4w9WgXcQ` and come from `search`, `queue` and `now_playing` output (the `id` field); a bare YouTube video id is accepted too. If a tool says snellar is not running, ask the user to start the app.";

#[derive(Clone)]
pub struct Snellar {
    tool_router: ToolRouter<Self>,
    seen: Arc<Mutex<HashMap<String, Value>>>,
    socket: PathBuf,
}

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    #[schemars(description = "What to look for: a title, an artist, or both, e.g. \"daft punk one more time\".")]
    query: String,
    #[schemars(
        description = "songs (default), albums, artists, or all. Albums and artists return ids for `play` (album_id) and `library`."
    )]
    kind: Option<String>,
    #[schemars(description = "Maximum results per kind, 1 to 25, default 10.")]
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct PlayArgs {
    #[schemars(description = "Search text; plays the best matching song and queues the other matches after it.")]
    query: Option<String>,
    #[schemars(description = "A track id (`yt:...`) from a previous `search`, `queue` or `now_playing` result. Plays just that track.")]
    #[serde(alias = "video_id")]
    track_id: Option<String>,
    #[schemars(description = "An album id from `search` with kind albums. Plays the whole album from the first track.")]
    album_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct TrackRef {
    #[schemars(description = "Search text; uses the best matching song.")]
    query: Option<String>,
    #[schemars(description = "A track id (`yt:...`) from a previous result.")]
    #[serde(alias = "video_id")]
    track_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct EnqueueArgs {
    #[schemars(description = "Search text; uses the best matching song.")]
    query: Option<String>,
    #[schemars(description = "A track id (`yt:...`) from a previous result.")]
    #[serde(alias = "video_id")]
    track_id: Option<String>,
    #[schemars(description = "true: play right after the current track. false (default): add at the end.")]
    next: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct QueueArgs {
    #[schemars(description = "list (default), jump, remove, move or clear.")]
    action: Option<String>,
    #[schemars(
        description = "Queue position (0-based) for jump and remove, and the source position for move. Positions are shown by `list`."
    )]
    index: Option<usize>,
    #[schemars(description = "Destination position for move.")]
    to: Option<usize>,
    #[schemars(description = "How many entries `list` shows around the current track, default 20.")]
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct ControlArgs {
    #[schemars(description = "play_pause, next, previous, seek, volume, shuffle or repeat.")]
    action: String,
    #[schemars(
        description = "seek: position in seconds. volume: 0 to 100. shuffle: true or false. repeat: off, all or one. Ignored by the other actions."
    )]
    value: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
struct LikeArgs {
    #[schemars(description = "Track id (`yt:...`) to like; defaults to the track that is playing.")]
    #[serde(alias = "video_id")]
    track_id: Option<String>,
    #[schemars(description = "true: make sure it is liked. false: make sure it is not. Omit to toggle.")]
    liked: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct LibraryArgs {
    #[schemars(description = "favorites, history, playlists, playlist (needs id) or albums.")]
    what: String,
    #[schemars(description = "Playlist id, for what = playlist.")]
    id: Option<i64>,
    #[schemars(description = "Maximum entries, default 25.")]
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct LyricsArgs {
    #[schemars(description = "Track id (`yt:...`); defaults to the track that is playing.")]
    #[serde(alias = "video_id")]
    track_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ImportArgs {
    #[schemars(description = "A public playlist or album link from Spotify, Deezer, Apple Music, Yandex Music or SoundCloud.")]
    link: Option<String>,
    #[schemars(description = "Absolute path of a CSV file exported by Exportify or TuneMyMusic, instead of a link.")]
    csv_path: Option<String>,
    #[schemars(description = "Name for the new playlist; defaults to the name of the source playlist.")]
    name: Option<String>,
}

fn answer(outcome: Result<String>) -> CallToolResult {
    match outcome {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("{e:#}"))]),
    }
}

fn id_of(track: &Value) -> String {
    track["id"].as_str().unwrap_or_default().to_string()
}

#[tool_router]
impl Snellar {
    pub fn at(socket: PathBuf) -> Self {
        Self { tool_router: Self::tool_router(), seen: Arc::default(), socket }
    }

    async fn call(&self, method: &str, args: &Value) -> Result<Value> {
        call_at(&self.socket, method, args).await
    }

    fn remember(&self, tracks: &[Value]) {
        let mut seen = self.seen.lock().unwrap();
        for t in tracks.iter().filter(|t| t["id"].is_string()) {
            seen.insert(id_of(t), t.clone());
        }
    }

    async fn songs(&self, query: &str) -> Result<Vec<Value>> {
        let found = self.call("search", &json!({ "q": query, "filter": "songs" })).await?;
        let tracks = found["tracks"].as_array().cloned().unwrap_or_default();
        self.remember(&tracks);
        if tracks.is_empty() {
            bail!("no songs found for `{query}`; try fewer words or the artist alone");
        }
        Ok(tracks)
    }

    async fn track(&self, reference: &TrackRef) -> Result<Value> {
        match (&reference.query, &reference.track_id) {
            (Some(q), None) => Ok(self.songs(q).await?.remove(0)),
            (None, Some(id)) => {
                let id = TrackId::parse(id);
                if let Some(known) = self.seen.lock().unwrap().get(id.as_str()).cloned() {
                    return Ok(known);
                }
                let track = self.call("track", &json!({ "id": id })).await?;
                self.remember(std::slice::from_ref(&track));
                Ok(track)
            }
            _ => bail!("give exactly one of query or track_id"),
        }
    }

    async fn current(&self) -> Result<Value> {
        let now = self.call("now", &Value::Null).await?;
        if now["current"].is_null() {
            bail!("nothing is playing");
        }
        self.remember(std::slice::from_ref(&now["current"]));
        Ok(now["current"].clone())
    }

    #[tool(
        description = "Search the YouTube Music catalog. Returns numbered results with ids; pass an id to `play`, `enqueue`, `like` or `lyrics`. Does not change playback."
    )]
    async fn search(&self, Parameters(p): Parameters<SearchArgs>) -> CallToolResult {
        answer(self.search_text(p).await)
    }

    async fn search_text(&self, p: SearchArgs) -> Result<String> {
        let kind = p.kind.as_deref().unwrap_or("songs");
        if !["songs", "albums", "artists", "all"].contains(&kind) {
            bail!("kind must be songs, albums, artists or all");
        }
        let limit = p.limit.unwrap_or(10).clamp(1, 25);
        let found = self.call("search", &json!({ "q": p.query, "filter": kind })).await?;
        let list = |key: &str| found[key].as_array().map_or(&[][..], Vec::as_slice).iter().take(limit).cloned().collect::<Vec<_>>();
        let (tracks, albums, artists) = (list("tracks"), list("albums"), list("artists"));
        self.remember(&tracks);
        let mut sections = Vec::new();
        if kind == "songs" || kind == "all" {
            sections.push(format!("Songs:\n{}", render::results(&tracks)));
        }
        if kind == "albums" || kind == "all" {
            let lines: Vec<String> = albums
                .iter()
                .map(|a| {
                    format!(
                        "- {} - {} {} (album_id {})",
                        render::artists(a),
                        a["title"].as_str().unwrap_or("?"),
                        a["year"].as_str().unwrap_or(""),
                        a["id"].as_str().unwrap_or_default()
                    )
                })
                .collect();
            sections.push(format!("Albums:\n{}", lines.join("\n")));
        }
        if kind == "artists" || kind == "all" {
            let lines: Vec<String> = artists
                .iter()
                .map(|a| format!("- {} (artist_id {})", a["name"].as_str().unwrap_or("?"), a["id"].as_str().unwrap_or_default()))
                .collect();
            sections.push(format!("Artists:\n{}", lines.join("\n")));
        }
        if tracks.is_empty() && albums.is_empty() && artists.is_empty() {
            return Ok(format!("No results for `{}`.", p.query));
        }
        Ok(sections.join("\n\n"))
    }

    #[tool(
        description = "Start playing now, replacing the queue. Give exactly one of: query (best match plays, the other matches queue after it), track_id (a single track), or album_id (the whole album). Returns what is playing."
    )]
    async fn play(&self, Parameters(p): Parameters<PlayArgs>) -> CallToolResult {
        answer(self.play_text(p).await)
    }

    async fn play_text(&self, p: PlayArgs) -> Result<String> {
        let tracks = match (&p.query, &p.track_id, &p.album_id) {
            (Some(q), None, None) => self.songs(q).await?,
            (None, Some(id), None) => vec![self.track(&TrackRef { query: None, track_id: Some(id.clone()) }).await?],
            (None, None, Some(id)) => {
                let album = self.call("album", &json!({ "id": id })).await?;
                let tracks = album["tracks"].as_array().cloned().unwrap_or_default();
                self.remember(&tracks);
                if tracks.is_empty() {
                    bail!("album `{id}` has no tracks; use an album_id from `search` with kind albums");
                }
                tracks
            }
            _ => bail!("give exactly one of query, track_id or album_id"),
        };
        self.call("play", &json!({ "tracks": tracks, "index": 0 })).await?;
        Ok(format!(
            "Playing {}{}",
            render::track(&tracks[0]),
            if tracks.len() > 1 { format!("; {} more queued", tracks.len() - 1) } else { String::new() }
        ))
    }

    #[tool(
        description = "Play a track and then keep playing similar music: the queue is filled with recommendations built from it. Give a query or a track_id."
    )]
    async fn play_radio(&self, Parameters(p): Parameters<TrackRef>) -> CallToolResult {
        answer(
            async {
                let track = self.track(&p).await?;
                self.call("radio", &json!({ "track": track })).await?;
                Ok(format!("Radio started from {}", render::track(&track)))
            }
            .await,
        )
    }

    #[tool(
        description = "Add one track to the queue without interrupting playback: right after the current track (next = true) or at the end. If nothing is playing it starts playing."
    )]
    async fn enqueue(&self, Parameters(p): Parameters<EnqueueArgs>) -> CallToolResult {
        answer(
            async {
                let track = self.track(&TrackRef { query: p.query, track_id: p.track_id }).await?;
                let next = p.next.unwrap_or(false);
                self.call("enqueue", &json!({ "track": track, "next": next })).await?;
                Ok(format!("Queued {} {}", render::track(&track), if next { "to play next" } else { "at the end" }))
            }
            .await,
        )
    }

    #[tool(description = "What is playing right now: title, artists, position, volume, shuffle, repeat, liked, and the track id.")]
    async fn now_playing(&self) -> CallToolResult {
        answer(
            async {
                let now = self.call("now", &Value::Null).await?;
                self.remember(std::slice::from_ref(&now["current"]));
                Ok(render::now(&now))
            }
            .await,
        )
    }

    #[tool(
        description = "Show or edit the play queue. action list (default) shows entries around the current one with their positions; jump plays a position; remove drops one; move reorders (index to `to`); clear empties the queue and stops."
    )]
    async fn queue(&self, Parameters(p): Parameters<QueueArgs>) -> CallToolResult {
        answer(self.queue_text(p).await)
    }

    async fn queue_text(&self, p: QueueArgs) -> Result<String> {
        let index = || p.index.context("this action needs `index`");
        match p.action.as_deref().unwrap_or("list") {
            "list" => {
                let queue = self.call("queue", &Value::Null).await?;
                self.remember(queue["tracks"].as_array().map_or(&[][..], Vec::as_slice));
                Ok(render::queue(&queue, p.limit.unwrap_or(20).clamp(1, 100)))
            }
            "jump" => self.call("jump", &json!({ "index": index()? })).await.map(|_| "ok".into()),
            "remove" => self.call("removeQueued", &json!({ "index": index()? })).await.map(|_| "ok".into()),
            "move" => {
                self.call("moveQueued", &json!({ "from": index()?, "to": p.to.context("move needs `to`")? })).await.map(|_| "ok".into())
            }
            "clear" => self.call("clearQueue", &Value::Null).await.map(|_| "Queue cleared.".into()),
            other => bail!("unknown action `{other}`; use list, jump, remove, move or clear"),
        }
    }

    #[tool(
        description = "Transport and playback settings. action: play_pause, next, previous, seek (value = seconds from the start), volume (value 0-100), shuffle (value true/false), repeat (value off/all/one)."
    )]
    async fn control(&self, Parameters(p): Parameters<ControlArgs>) -> CallToolResult {
        answer(
            async {
                let value = p.value.unwrap_or(Value::Null);
                let need = |kind: &str| if value.is_null() { bail!("{} needs `value` ({kind})", p.action) } else { Ok(value.clone()) };
                match p.action.as_str() {
                    "play_pause" => self.call("toggle", &Value::Null).await?,
                    "next" => self.call("next", &Value::Null).await?,
                    "previous" => self.call("prev", &Value::Null).await?,
                    "seek" => self.call("seek", &json!({ "pos": need("seconds")? })).await?,
                    "volume" => self.call("volume", &json!({ "value": need("0-100")? })).await?,
                    "shuffle" => self.call("shuffle", &json!({ "on": need("true or false")? })).await?,
                    "repeat" => self.call("repeat", &json!({ "mode": need("off, all or one")? })).await?,
                    other => bail!("unknown action `{other}`; use play_pause, next, previous, seek, volume, shuffle or repeat"),
                };
                Ok(render::now(&self.call("now", &Value::Null).await?))
            }
            .await,
        )
    }

    #[tool(description = "Like or unlike a track (defaults to the one playing). Omit `liked` to toggle. Returns the resulting state.")]
    async fn like(&self, Parameters(p): Parameters<LikeArgs>) -> CallToolResult {
        answer(
            async {
                let track = match p.track_id {
                    Some(id) => self.track(&TrackRef { query: None, track_id: Some(id) }).await?,
                    None => self.current().await?,
                };
                let liked = self.call("isLiked", &json!({ "id": id_of(&track) })).await? == json!(true);
                let now = if p.liked.is_some_and(|want| want == liked) {
                    liked
                } else {
                    self.call("like", &json!({ "track": track })).await? == json!(true)
                };
                Ok(format!("{} is {}", render::track(&track), if now { "liked" } else { "not liked" }))
            }
            .await,
        )
    }

    #[tool(
        description = "Read the user's library. what: favorites (liked tracks, newest first), history (recently played), playlists (names and ids), playlist (tracks of one, needs id), albums (saved albums)."
    )]
    async fn library(&self, Parameters(p): Parameters<LibraryArgs>) -> CallToolResult {
        answer(self.library_text(p).await)
    }

    async fn library_text(&self, p: LibraryArgs) -> Result<String> {
        let limit = p.limit.unwrap_or(25).clamp(1, 200);
        let tracks = |list: &Value| {
            let found = list.as_array().map_or(&[][..], Vec::as_slice).iter().take(limit).cloned().collect::<Vec<_>>();
            self.remember(&found);
            found
        };
        let text = match p.what.as_str() {
            "favorites" => render::results(&tracks(&self.call("favorites", &Value::Null).await?)),
            "history" => render::results(&tracks(&self.call("history", &json!({ "limit": limit })).await?)),
            "playlists" => self
                .call("playlists", &Value::Null)
                .await?
                .as_array()
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .map(|l| format!("- {} ({} tracks, id {})", l["name"].as_str().unwrap_or("?"), l["count"], l["id"]))
                .collect::<Vec<_>>()
                .join("\n"),
            "playlist" => {
                let list = self.call("playlist", &json!({ "id": p.id.context("playlist needs `id`")? })).await?;
                if list.is_null() {
                    bail!("no such playlist; list them with what = playlists");
                }
                render::results(&tracks(&list["tracks"]))
            }
            "albums" => self
                .call("savedAlbums", &Value::Null)
                .await?
                .as_array()
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .take(limit)
                .map(|a| {
                    format!(
                        "- {} - {} (album_id {})",
                        render::artists(a),
                        a["title"].as_str().unwrap_or("?"),
                        a["id"].as_str().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            other => bail!("unknown value `{other}`; use favorites, history, playlists, playlist or albums"),
        };
        Ok(if text.is_empty() { "Nothing there yet.".into() } else { text })
    }

    #[tool(
        description = "Import a playlist into a new local playlist: songs are looked up on YouTube Music by title, artist and length. Takes a link or a CSV path. Can take a few minutes for long lists. Returns how many were found and lists the songs that were not. A private playlist or one the service hides needs a CSV export (Exportify for Spotify, TuneMyMusic for others)."
    )]
    async fn import_playlist(&self, Parameters(p): Parameters<ImportArgs>) -> CallToolResult {
        answer(
            async {
                let args = match (p.link, p.csv_path) {
                    (Some(link), _) => json!({ "url": link, "name": p.name }),
                    (None, Some(path)) => json!({ "path": path, "name": p.name }),
                    (None, None) => bail!("give `link` or `csv_path`"),
                };
                Ok(render::import(&self.call("importPlaylist", &args).await?))
            }
            .await,
        )
    }

    #[tool(
        description = "Lyrics for a track (defaults to the one playing). Synced lyrics come with [m:ss] timestamps, otherwise plain text."
    )]
    async fn lyrics(&self, Parameters(p): Parameters<LyricsArgs>) -> CallToolResult {
        answer(
            async {
                let track = match p.track_id {
                    Some(id) => self.track(&TrackRef { query: None, track_id: Some(id) }).await?,
                    None => self.current().await?,
                };
                let lyrics = self.call("lyrics", &json!({ "track": track })).await?;
                let synced: Vec<String> = lyrics["lines"]
                    .as_array()
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .map(|l| format!("[{}] {}", render::clock(l["time"].as_f64().unwrap_or(0.0)), l["text"].as_str().unwrap_or_default()))
                    .collect();
                let text = if lyrics["synced"] == json!(true) && !synced.is_empty() {
                    synced.join("\n")
                } else {
                    lyrics["plain"].as_str().unwrap_or_default().to_string()
                };
                if text.trim().is_empty() {
                    bail!("no lyrics found for {}", render::track(&track));
                }
                Ok(text)
            }
            .await,
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Snellar {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(INSTRUCTIONS)
    }
}

pub async fn serve() -> Result<()> {
    let socket = socket_path().context("XDG_RUNTIME_DIR is not set")?;
    let service = Snellar::at(socket).serve(rmcp::transport::stdio()).await.context("MCP handshake failed")?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};

    use super::*;

    struct Client {
        reader: BufReader<ReadHalf<DuplexStream>>,
        writer: WriteHalf<DuplexStream>,
        next: u64,
    }

    impl Client {
        async fn connect(socket: PathBuf) -> Client {
            let (near, far) = tokio::io::duplex(1 << 16);
            tokio::spawn(async move {
                if let Ok(service) = Snellar::at(socket).serve(far).await {
                    service.waiting().await.ok();
                }
            });
            let (read, write) = tokio::io::split(near);
            let mut client = Client { reader: BufReader::new(read), writer: write, next: 0 };
            let init = client
                .request(
                    "initialize",
                    json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } }),
                )
                .await;
            assert!(init["result"]["capabilities"]["tools"].is_object(), "{init}");
            client.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
            client
        }

        async fn send(&mut self, message: &Value) {
            self.writer.write_all(format!("{message}\n").as_bytes()).await.unwrap();
        }

        async fn request(&mut self, method: &str, params: Value) -> Value {
            self.next += 1;
            let id = self.next;
            self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await;
            loop {
                let mut line = String::new();
                assert!(self.reader.read_line(&mut line).await.unwrap() > 0, "server closed");
                let message: Value = serde_json::from_str(&line).unwrap();
                if message["id"] == json!(id) {
                    return message;
                }
            }
        }

        async fn tool(&mut self, name: &str, arguments: Value) -> (bool, String) {
            let result = self.request("tools/call", json!({ "name": name, "arguments": arguments })).await["result"].clone();
            (result["isError"] == json!(true), result["content"][0]["text"].as_str().unwrap_or_default().to_string())
        }
    }

    #[tokio::test]
    async fn lists_eleven_described_tools() {
        let mut client = Client::connect(std::env::temp_dir().join("snellar-no-such.sock")).await;
        let listed = client.request("tools/list", json!({})).await;
        let tools = listed["result"]["tools"].as_array().unwrap();
        let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["control", "enqueue", "import_playlist", "library", "like", "lyrics", "now_playing", "play", "play_radio", "queue", "search"]
        );
        assert!(tools.iter().all(|t| t["description"].as_str().is_some_and(|d| d.len() > 40)));
    }

    #[tokio::test]
    async fn a_bare_video_id_finds_the_track_a_result_showed_with_the_prefix() {
        let mcp = Snellar::at(std::env::temp_dir().join("snellar-no-such.sock"));
        let shown = json!({ "id": "yt:dQw4w9WgXcQ", "title": "Song" });
        mcp.remember(std::slice::from_ref(&shown));
        for given in [json!({ "track_id": "yt:dQw4w9WgXcQ" }), json!({ "track_id": "dQw4w9WgXcQ" }), json!({ "video_id": "dQw4w9WgXcQ" })] {
            let reference: TrackRef = serde_json::from_value(given.clone()).unwrap();
            assert_eq!(mcp.track(&reference).await.unwrap(), shown, "{given}");
        }
    }

    #[tokio::test]
    async fn tool_errors_are_readable_not_protocol_errors() {
        let mut client = Client::connect(std::env::temp_dir().join("snellar-no-such.sock")).await;
        let (failed, text) = client.tool("now_playing", json!({})).await;
        assert!(failed && text.contains("not running"), "{text}");
        let (failed, text) = client.tool("play", json!({})).await;
        assert!(failed && text.contains("exactly one of"), "{text}");
        let (failed, text) = client.tool("control", json!({ "action": "dance" })).await;
        assert!(failed && text.contains("unknown action"), "{text}");
        let (failed, text) = client.tool("search", json!({ "query": "x", "kind": "podcasts" })).await;
        assert!(failed && text.contains("kind must be"), "{text}");
    }

    #[tokio::test]
    #[ignore]
    async fn live_plays_a_track_through_mcp() {
        let sandbox = crate::testkit::Sandbox::new("e2e");
        let socket = sandbox.socket();
        let binary = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join(env!("CARGO_PKG_NAME"));
        let mut app = sandbox.app(&binary).spawn().unwrap_or_else(|e| panic!("build the app first ({}): {e}", binary.display()));
        let started = std::time::Instant::now();
        while !socket.exists() {
            assert!(started.elapsed() < Duration::from_secs(30), "the app never opened its socket");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let begun = std::time::Instant::now();
        let now = call_at(&socket, "now", &Value::Null).await.unwrap();
        println!("now over the socket: {:?}", begun.elapsed());
        assert!(now["volume"].is_number());

        let mut client = Client::connect(socket.clone()).await;
        assert!(!client.tool("control", json!({ "action": "volume", "value": 20 })).await.0);
        let (failed, text) = client.tool("search", json!({ "query": "daft punk one more time", "limit": 3 })).await;
        assert!(!failed && text.contains("One More Time"), "{text}");
        let (failed, text) = client.tool("play", json!({ "query": "daft punk one more time" })).await;
        assert!(!failed && text.starts_with("Playing"), "{text}");
        let begun = std::time::Instant::now();
        let outcome = loop {
            let (_, now) = client.tool("now_playing", json!({})).await;
            if now.contains("\nplaying ") && !now.contains("playing 0:00") {
                break Ok(now);
            }
            if begun.elapsed() > Duration::from_secs(30) {
                break Err(now);
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        let (_, queue) = client.tool("queue", json!({ "limit": 3 })).await;
        client.tool("control", json!({ "action": "play_pause" })).await;
        app.kill().ok();
        app.wait().ok();
        println!("{outcome:?}\n{queue}");
        assert!(outcome.is_ok(), "{outcome:?}");
        assert!(queue.contains('>'), "{queue}");
    }
}
