use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use snellar_core::error::Code;
use snellar_core::ids::TrackId;
use snellar_core::journal;
use snellar_core::model::{
    Album, AlbumCard, Artist, ArtistCard, Browse, Discography, Hit, Kind, Page, Ref, SearchResult, Shelf, Suggestions, Track,
};
pub use snellar_core::source::Filter;

use crate::net::{Http, checked, net};
use crate::title;

const ORIGIN: &str = "https://music.youtube.com";
const HEDGE_AFTER: Duration = Duration::from_millis(1200);
const CLIENT_VERSION: &str = "1.20250929.03.00";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const SONGS_PARAMS: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";
const ALBUMS_PARAMS: &str = "EgWKAQIYAWoMEA4QChADEAQQCRAF";
const ARTISTS_PARAMS: &str = "EgWKAQIgAWoMEA4QChADEAQQCRAF";
const VIDEOS_PARAMS: &str = "EgWKAQIQAWoMEA4QChADEAQQCRAF";
const MAX_PAGES: usize = 20;
const KINDS: [&str; 10] = ["Song", "Video", "Album", "Single", "EP", "Artist", "Playlist", "Episode", "Podcast", "Profile"];

pub struct Ytm {
    http: Http,
    origin: String,
    visitor: OnceLock<String>,
    region: Mutex<String>,
    foreground: AtomicUsize,
}

struct Busy<'a>(&'a AtomicUsize);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

enum Item {
    Track(Track),
    Album(AlbumCard),
    Artist(ArtistCard),
    Playlist(AlbumCard),
}

#[derive(Default)]
struct Sub {
    kind: String,
    artists: Vec<Ref>,
    album: Option<Ref>,
    year: String,
    duration: f64,
    rest: Vec<String>,
}

fn at<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |v, k| match k.parse::<usize>() {
        Ok(i) => v.get(i),
        Err(_) => v.get(k),
    })
}

fn text<'a>(v: &'a Value, path: &str) -> Option<&'a str> {
    at(v, path)?.as_str()
}

fn items<'a>(v: &'a Value, path: &str) -> &'a [Value] {
    at(v, path).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn deep<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m.get(key).or_else(|| m.values().find_map(|c| deep(c, key))),
        Value::Array(a) => a.iter().find_map(|c| deep(c, key)),
        _ => None,
    }
}

fn collect<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, c) in m {
                if k == key {
                    out.push(c);
                } else {
                    collect(c, key, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|c| collect(c, key, out)),
        _ => {}
    }
}

fn run_text(runs: &[Value]) -> String {
    runs.iter().filter_map(|r| text(r, "text")).collect()
}

fn thumb(v: &Value) -> String {
    let Some(url) = v
        .get("thumbnail")
        .or_else(|| v.get("thumbnailRenderer"))
        .and_then(|t| deep(t, "thumbnails"))
        .and_then(|t| text(t.as_array()?.last()?, "url"))
    else {
        return String::new();
    };
    match url.rsplit_once("=w") {
        Some((base, _)) if url.contains("googleusercontent.com") || url.contains("ggpht.com") => {
            format!("{base}=w544-h544-l90-rj")
        }
        _ => url.to_string(),
    }
}

fn parse_duration(s: &str) -> Option<f64> {
    let parts = s.split(':').map(str::parse::<u32>).collect::<Result<Vec<_>, _>>().ok()?;
    (2..=3).contains(&parts.len()).then(|| parts.iter().fold(0, |a, n| a * 60 + n) as f64)
}

fn split_sub<'a>(runs: impl IntoIterator<Item = &'a Value>) -> Sub {
    let mut sub = Sub::default();
    for run in runs {
        let Some(t) = text(run, "text") else { continue };
        let browse = text(run, "navigationEndpoint.browseEndpoint.browseId").unwrap_or_default();
        if browse.starts_with("UC") || browse.starts_with("MPLA") {
            sub.artists.push(Ref { id: browse.trim_start_matches("MPLA").into(), name: t.into() });
        } else if browse.starts_with("MPRE") {
            sub.album = Some(Ref { id: browse.into(), name: t.into() });
        } else if let Some(d) = parse_duration(t) {
            sub.duration = d;
        } else if t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit()) {
            sub.year = t.into();
        } else if KINDS.contains(&t) {
            if sub.kind.is_empty() {
                sub.kind = t.into();
            }
        } else if !matches!(t.trim(), "" | "•" | "&" | ",") {
            sub.rest.push(t.into());
        }
    }
    sub
}

fn explicit(r: &Value) -> bool {
    ["badges", "subtitleBadges"]
        .iter()
        .any(|k| items(r, k).iter().any(|b| text(b, "musicInlineBadgeRenderer.icon.iconType") == Some("MUSIC_EXPLICIT_BADGE")))
}

fn build(title: &str, video: Option<&str>, browse: Option<&str>, sub: Sub, r: &Value) -> Option<Item> {
    let thumb = thumb(r);
    if let Some(id) = video {
        let artists = if sub.artists.is_empty() {
            sub.rest
                .iter()
                .filter(|s| !s.contains(|c: char| c.is_ascii_digit()))
                .map(|name| Ref { id: String::new(), name: name.clone() })
                .take(1)
                .collect()
        } else {
            sub.artists
        };
        return Some(Item::Track(Track {
            id: TrackId::youtube(id),
            title: title.into(),
            artists,
            album: sub.album,
            duration: sub.duration,
            thumb,
            explicit: explicit(r),
            video: false,
        }));
    }
    let id = browse?;
    if id.starts_with("MPRE") {
        let kind = if sub.kind.is_empty() { "Album".into() } else { sub.kind };
        Some(Item::Album(AlbumCard { id: id.into(), title: title.into(), artists: sub.artists, year: sub.year, kind, thumb }))
    } else if id.starts_with("UC") {
        let subtitle = sub.rest.into_iter().next().unwrap_or_default();
        Some(Item::Artist(ArtistCard { id: id.into(), name: title.into(), subtitle, thumb }))
    } else if let Some(list) = id.strip_prefix("VL").filter(|list| browse_safe(list)) {
        let kind = "Playlist".into();
        Some(Item::Playlist(AlbumCard { id: list.into(), title: title.into(), artists: sub.artists, kind, thumb, ..AlbumCard::default() }))
    } else {
        None
    }
}

fn list_item(r: &Value, watch_link: bool) -> Option<Item> {
    let title = text(r, "flexColumns.0.musicResponsiveListItemFlexColumnRenderer.text.runs.0.text")?;
    let flex = items(r, "flexColumns").iter().skip(1);
    let runs = flex
        .filter_map(|c| at(c, "musicResponsiveListItemFlexColumnRenderer.text.runs"))
        .chain(items(r, "fixedColumns").iter().filter_map(|c| at(c, "musicResponsiveListItemFixedColumnRenderer.text.runs")))
        .filter_map(Value::as_array)
        .flatten();
    let video = text(r, "playlistItemData.videoId").or_else(|| text(r, "navigationEndpoint.watchEndpoint.videoId").filter(|_| watch_link));
    let browse = text(r, "navigationEndpoint.browseEndpoint.browseId");
    build(title, video, browse, split_sub(runs), r)
}

fn two_row(r: &Value) -> Option<Item> {
    let title = text(r, "title.runs.0.text")?;
    let video = text(r, "navigationEndpoint.watchEndpoint.videoId");
    let browse = text(r, "navigationEndpoint.browseEndpoint.browseId");
    build(title, video, browse, split_sub(items(r, "subtitle.runs")), r)
}

fn entry(e: &Value) -> Option<Item> {
    if let Some(r) = e.get("musicTwoRowItemRenderer") { two_row(r) } else { list_item(e.get("musicResponsiveListItemRenderer")?, false) }
}

fn panel_track(r: &Value) -> Option<Track> {
    let sub = split_sub(items(r, "longBylineText.runs"));
    let video = text(r, "navigationEndpoint.watchEndpoint.watchEndpointMusicSupportedConfigs.watchEndpointMusicConfig.musicVideoType")
        .is_some_and(|kind| kind != "MUSIC_VIDEO_TYPE_ATV");
    let track = Track {
        id: TrackId::youtube(text(r, "videoId")?),
        title: run_text(items(r, "title.runs")),
        artists: sub.artists,
        album: sub.album,
        duration: parse_duration(text(r, "lengthText.runs.0.text")?).unwrap_or_default(),
        thumb: thumb(r),
        explicit: explicit(r),
        video,
    };
    Some(if video { clean_video(track) } else { track })
}

fn clean_video(mut track: Track) -> Track {
    let uploader = track.artists.first().cloned().unwrap_or_default();
    let parsed = title::parse(&track.title, &uploader.name);
    track.title = parsed.title;
    track.artists = parsed
        .artists
        .into_iter()
        .map(|name| Ref { id: if name.to_lowercase() == uploader.name.to_lowercase() { uploader.id.clone() } else { String::new() }, name })
        .collect();
    track
}

pub(crate) fn video(id: &TrackId) -> Result<&str> {
    id.video_id().ok_or_else(|| Code::SourceUnsupported.err(format!("`{id}` is not a YouTube track")))
}

pub fn watch_url(id: &TrackId) -> Option<String> {
    id.video_id().map(|v| format!("https://music.youtube.com/watch?v={v}"))
}

fn browse_safe(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn share_url(kind: Kind, id: &str) -> Option<String> {
    match kind {
        Kind::Track => watch_url(&TrackId::parse(id)),
        _ if !browse_safe(id) => None,
        Kind::Album if id.starts_with("MPRE") => Some(format!("https://music.youtube.com/browse/{id}")),
        Kind::Album | Kind::Playlist => Some(format!("https://music.youtube.com/playlist?list={}", id.strip_prefix("VL").unwrap_or(id))),
        Kind::Artist if id.starts_with("UC") => Some(format!("https://music.youtube.com/channel/{id}")),
        Kind::Artist => None,
    }
}

fn youtube_url(query: &str) -> Option<(String, reqwest::Url)> {
    let q = query.trim();
    let lower = q.to_lowercase();
    let bare = ["youtube.com/", "youtu.be/", "www.", "m.", "music.youtube.com/"].iter().any(|p| lower.starts_with(p));
    let full = if lower.starts_with("http://") || lower.starts_with("https://") {
        q.to_string()
    } else if bare {
        format!("https://{q}")
    } else {
        return None;
    };
    let url = reqwest::Url::parse(&full).ok()?;
    let host = url.host_str()?.to_lowercase();
    let host = host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("music.").to_string();
    Some((host, url))
}

fn page_link(query: &str) -> Option<Page> {
    let (host, url) = youtube_url(query)?;
    if host != "youtube.com" || video_id(query).is_some() {
        return None;
    }
    let mut segments = url.path_segments()?.filter(|s| !s.is_empty());
    let page = |kind, id: &str| browse_safe(id).then(|| Page { kind, id: id.to_string() });
    match (segments.next()?, segments.next()) {
        ("playlist", None) => {
            let list = url.query_pairs().find(|(k, _)| k == "list")?.1.into_owned();
            let kind = if list.starts_with("OLAK5uy_") { Kind::Album } else { Kind::Playlist };
            page(kind, &list)
        }
        ("browse", Some(id)) if id.starts_with("MPRE") => page(Kind::Album, id),
        ("browse" | "channel", Some(id)) if id.starts_with("UC") => page(Kind::Artist, id),
        ("browse", Some(id)) if id.starts_with("VL") => page(Kind::Playlist, id.strip_prefix("VL").unwrap_or(id)),
        _ => None,
    }
}

pub fn video_id(query: &str) -> Option<String> {
    let (host, url) = youtube_url(query)?;
    let valid = |id: &str| id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    let mut segments = url.path_segments()?.filter(|s| !s.is_empty());
    let candidate = match host.as_str() {
        "youtu.be" => segments.next().map(String::from),
        "youtube.com" | "youtube-nocookie.com" => {
            url.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.into_owned()).or_else(|| match (segments.next(), segments.next()) {
                (Some("shorts" | "embed" | "live" | "v"), Some(id)) => Some(id.into()),
                _ => None,
            })
        }
        _ => None,
    };
    candidate.filter(|id| valid(id))
}

fn shelf_title(s: &Value) -> &str {
    text(s, "header.musicCarouselShelfBasicHeaderRenderer.title.runs.0.text").or_else(|| text(s, "title.runs.0.text")).unwrap_or_default()
}

fn fill(tracks: &mut [Track], album: &AlbumCard) {
    for t in tracks {
        if t.thumb.is_empty() {
            t.thumb = album.thumb.clone();
        }
        if t.album.is_none() {
            t.album = Some(Ref { id: album.id.clone(), name: album.title.clone() });
        }
        if t.artists.is_empty() {
            t.artists = album.artists.clone();
        }
    }
}

fn tracks_of(shelf: Option<&Value>) -> Vec<Track> {
    shelf.map_or_else(Vec::new, |s| tracks_in(items(s, "contents")))
}

fn header_card(resp: &Value, id: &str) -> Result<AlbumCard> {
    let header = deep(resp, "musicResponsiveHeaderRenderer")
        .or_else(|| deep(resp, "musicDetailHeaderRenderer"))
        .context("page has no header")
        .context(Code::YtParse)?;
    let mut sub = split_sub(items(header, "subtitle.runs"));
    sub.artists.extend(split_sub(items(header, "straplineTextOne.runs")).artists);
    Ok(AlbumCard {
        id: id.into(),
        title: run_text(items(header, "title.runs")),
        artists: sub.artists,
        year: sub.year,
        kind: sub.kind,
        thumb: thumb(header),
    })
}

fn sections<'a>(resp: &'a Value, root: &str) -> &'a [Value] {
    items(resp, &format!("contents.{root}.tabs.0.tabRenderer.content.sectionListRenderer.contents"))
}

impl Ytm {
    pub fn new(http: impl Into<Http>) -> Self {
        Self::at(http, ORIGIN.into())
    }

    pub(crate) fn at(http: impl Into<Http>, origin: String) -> Self {
        Self { http: http.into(), origin, visitor: OnceLock::new(), region: Mutex::default(), foreground: AtomicUsize::new(0) }
    }

    pub fn set_region(&self, code: &str) {
        *self.region.lock().unwrap() = code.trim().to_uppercase();
    }

    fn busy(&self) -> Busy<'_> {
        self.foreground.fetch_add(1, Ordering::Relaxed);
        Busy(&self.foreground)
    }

    async fn idle(&self) {
        for _ in 0..100 {
            if self.foreground.load(Ordering::Relaxed) == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn call(&self, endpoint: &str, body: Value) -> Result<Value> {
        let mut first = Box::pin(self.call_once(endpoint, body.clone()));
        let result = match tokio::time::timeout(HEDGE_AFTER, &mut first).await {
            Ok(done) => done,
            Err(_) => {
                let mut second = Box::pin(self.call_once(endpoint, body.clone()));
                tokio::select! {
                    r = &mut first => if r.is_ok() { r } else { second.await },
                    r = &mut second => if r.is_ok() { r } else { first.await },
                }
            }
        };
        match result {
            Err(e) if e.downcast_ref::<reqwest::Error>().is_some_and(|r| r.is_timeout() || r.is_connect()) => {
                self.call_once(endpoint, body).await
            }
            other => other,
        }
    }

    async fn call_once(&self, endpoint: &str, mut body: Value) -> Result<Value> {
        let mut client = json!({"clientName": "WEB_REMIX", "clientVersion": CLIENT_VERSION, "hl": "en"});
        let region = self.region.lock().unwrap().clone();
        if !region.is_empty() {
            client["gl"] = json!(region);
        }
        if let Some(v) = self.visitor.get() {
            client["visitorData"] = json!(v);
        }
        body["context"] = json!({"client": client});
        let sent = self
            .http
            .client()
            .post(format!("{}/youtubei/v1/{endpoint}?prettyPrint=false", self.origin))
            .header("Origin", "https://music.youtube.com")
            .header("Referer", "https://music.youtube.com/")
            .header("User-Agent", USER_AGENT)
            .json(&body)
            .timeout(Duration::from_secs(6))
            .send()
            .await
            .map_err(net)
            .with_context(|| format!("YouTube Music {endpoint} request failed"))?;
        let resp: Value = checked(sent)
            .with_context(|| format!("YouTube Music {endpoint} was rejected"))?
            .json()
            .await
            .map_err(net)
            .with_context(|| format!("YouTube Music {endpoint} returned invalid JSON"))?;
        if let Some(v) = text(&resp, "responseContext.visitorData") {
            let _ = self.visitor.set(v.into());
        }
        Ok(resp)
    }

    async fn search_page(&self, q: &str, params: Option<&str>) -> Result<Value> {
        let mut body = json!({"query": q});
        if let Some(p) = params {
            body["params"] = json!(p);
        }
        self.call("search", body).await
    }

    pub async fn search(&self, q: &str, filter: Filter) -> Result<SearchResult> {
        let _busy = self.busy();
        self.search_inner(q, filter).await
    }

    pub async fn search_background(&self, q: &str, filter: Filter) -> Result<SearchResult> {
        self.search_inner(q, filter).await
    }

    pub async fn search_quiet(&self, q: &str, filter: Filter) -> Result<SearchResult> {
        self.idle().await;
        self.search_inner(q, filter).await
    }

    async fn search_inner(&self, q: &str, filter: Filter) -> Result<SearchResult> {
        let params = match filter {
            Filter::Songs => Some(SONGS_PARAMS),
            Filter::Albums => Some(ALBUMS_PARAMS),
            Filter::Artists => Some(ARTISTS_PARAMS),
            Filter::Videos => Some(VIDEOS_PARAMS),
            Filter::All => None,
        };
        let mut out = SearchResult::default();
        if let Some(track) = match video_id(q) {
            Some(id) => self.track(&TrackId::youtube(&id)).await.ok().flatten(),
            None => None,
        } {
            if track.video {
                out.videos.push(track.clone());
            } else {
                out.tracks.push(track.clone());
            }
            out.linked = Some(track);
            return Ok(out);
        }
        if let Some(page) = page_link(q) {
            return Ok(SearchResult { page: Some(page), ..SearchResult::default() });
        }
        if filter == Filter::All {
            let (songs, mixed, videos) = tokio::try_join!(
                self.search_page(q, Some(SONGS_PARAMS)),
                self.search_page(q, None),
                self.search_page(q, Some(VIDEOS_PARAMS)),
            )?;
            return search_all(&songs, &mixed, &videos);
        }
        let page = self.search_page(q, params).await?;
        search_filtered(&page, filter)
    }

    pub async fn track(&self, id: &TrackId) -> Result<Option<Track>> {
        let video_id = video(id)?;
        let watch = self.call("next", json!({"videoId": video_id})).await?;
        Ok(watch_track(&watch, video_id))
    }

    pub async fn suggest(&self, q: &str) -> Result<Suggestions> {
        let resp = self.call("music/get_search_suggestions", json!({"input": q})).await?;
        Ok(suggestions(&resp))
    }

    async fn album_browse_id(&self, playlist_id: &str) -> Result<String> {
        let resp = self.call("browse", json!({"browseId": format!("VL{playlist_id}")})).await?;
        let mut ids = Vec::new();
        collect(&resp, "browseId", &mut ids);
        ids.into_iter()
            .filter_map(Value::as_str)
            .find(|id| id.starts_with("MPRE"))
            .map(String::from)
            .with_context(|| format!("no album found for playlist {playlist_id}"))
            .context(Code::YtParse)
    }

    pub async fn album(&self, id: &str) -> Result<Album> {
        let _busy = self.busy();
        let id = match id.starts_with("MPRE") {
            true => id.to_string(),
            false => self.album_browse_id(id).await?,
        };
        let resp = self.call("browse", json!({"browseId": id})).await?;
        album_page(&resp, id)
    }

    pub async fn playlist(&self, id: &str) -> Result<Album> {
        let id = id.strip_prefix("VL").unwrap_or(id);
        let resp = {
            let _busy = self.busy();
            self.call("browse", json!({"browseId": format!("VL{id}")})).await?
        };
        let (mut album, next) = playlist_page(&resp, id)?;
        album.tracks = self.continue_from(next, album.tracks, tracks_in).await;
        Ok(album)
    }

    pub async fn playlist_first_page(&self, id: &str) -> Result<Album> {
        let _busy = self.busy();
        let id = id.strip_prefix("VL").unwrap_or(id);
        let resp = self.call("browse", json!({"browseId": format!("VL{id}")})).await?;
        Ok(playlist_page(&resp, id)?.0)
    }

    pub async fn artist(&self, id: &str) -> Result<Artist> {
        let _busy = self.busy();
        self.artist_quiet(id).await
    }

    pub async fn artist_quiet(&self, id: &str) -> Result<Artist> {
        let resp = self.call("browse", json!({"browseId": id})).await?;
        artist_page(&resp, id)
    }

    pub async fn discography(&self, artist: &Artist) -> Result<Discography> {
        let by = Ref { id: artist.id.clone(), name: artist.name.clone() };
        let (albums, singles) = tokio::try_join!(
            self.releases(artist.more_albums.as_ref(), &artist.albums, &by),
            self.releases(artist.more_singles.as_ref(), &artist.singles, &by),
        )?;
        Ok(Discography { artist: by, albums, singles })
    }

    async fn releases(&self, more: Option<&Browse>, shown: &[AlbumCard], by: &Ref) -> Result<Vec<AlbumCard>> {
        let Some(more) = more else { return Ok(shown.to_vec()) };
        let resp = {
            let _busy = self.busy();
            self.call("browse", json!({"browseId": more.id, "params": more.params})).await?
        };
        let (first, next) = discography_page(&resp, by);
        let all = self.continue_from(next, first, |list| cards(list, by)).await;
        Ok(full_or_shown(all, shown))
    }

    async fn continue_from<T>(&self, mut next: Option<String>, mut out: Vec<T>, parse: impl Fn(&[Value]) -> Vec<T>) -> Vec<T> {
        let mut seen = HashSet::new();
        for _ in 0..MAX_PAGES {
            let Some(token) = next.take().filter(|t| seen.insert(t.clone())) else { break };
            match self.call("browse", json!({"continuation": token})).await {
                Ok(resp) => {
                    let (list, more) = continued(&resp);
                    out.extend(parse(list));
                    next = more;
                }
                Err(e) => {
                    journal::warn("ytm", format!("continuation stopped after {} items: {e:#}", out.len()));
                    break;
                }
            }
        }
        out
    }

    pub async fn mix(&self, list: &Browse) -> Result<Vec<Track>> {
        let _busy = self.busy();
        let body = json!({"playlistId": list.id, "params": list.params, "isAudioOnly": true, "enablePersistentPlaylistPanel": true});
        Ok(mix_tracks(&self.call("next", body).await?))
    }

    pub async fn radio(&self, id: &TrackId) -> Result<Vec<Track>> {
        let video_id = video(id)?;
        let body = json!({
            "videoId": video_id,
            "playlistId": format!("RDAMVM{video_id}"),
            "params": "wAEB",
            "isAudioOnly": true,
            "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
            "enablePersistentPlaylistPanel": true,
        });
        let resp = self.call("next", body).await?;
        radio_tracks(&resp, id)
    }

    pub async fn moods(&self) -> Result<Vec<(String, String)>> {
        let page = self.call("browse", json!({"browseId": "FEmusic_moods_and_genres"})).await?;
        Ok(mood_buttons(&page))
    }

    pub async fn mood_playlists(&self, params: &str) -> Result<Vec<String>> {
        let page = self.call("browse", json!({"browseId": "FEmusic_moods_and_genres_category", "params": params})).await?;
        let mut cards = Vec::new();
        collect(&page, "musicTwoRowItemRenderer", &mut cards);
        Ok(cards
            .into_iter()
            .filter_map(|c| text(c, "navigationEndpoint.browseEndpoint.browseId")?.strip_prefix("VL").map(String::from))
            .collect())
    }

    pub async fn lyrics(&self, id: &TrackId) -> Result<Option<String>> {
        let video_id = video(id)?;
        let watch = self.call("next", json!({"videoId": video_id})).await?;
        let Some(browse) = lyrics_browse_id(&watch) else {
            return Ok(None);
        };
        let page = self.call("browse", json!({"browseId": browse})).await?;
        Ok(lyrics_text(&page))
    }

    pub async fn home(&self) -> Result<Vec<Shelf>> {
        let shelves = self.home_once().await?;
        if shelves.is_empty() { self.home_once().await } else { Ok(shelves) }
    }

    async fn home_once(&self) -> Result<Vec<Shelf>> {
        let resp = self.call("browse", json!({"browseId": "FEmusic_home"})).await?;
        home_shelves(&resp)
    }
}

fn search_all(songs: &Value, mixed: &Value, videos: &Value) -> Result<SearchResult> {
    for page in [songs, mixed, videos] {
        check_search(page)?;
    }
    let mut out = SearchResult {
        videos: video_tracks(videos),
        tracks: search_items(songs)
            .into_iter()
            .filter_map(|i| match i {
                Item::Track(t) => Some(t),
                _ => None,
            })
            .collect(),
        top: top_result(mixed),
        ..Default::default()
    };
    for item in search_items(mixed) {
        match item {
            Item::Album(a) => out.albums.push(a),
            Item::Artist(a) => out.artists.push(a),
            Item::Playlist(p) => out.playlists.push(p),
            Item::Track(_) => {}
        }
    }
    Ok(out)
}

fn top_result(mixed: &Value) -> Option<Hit> {
    let card = sections(mixed, "tabbedSearchResultsRenderer").iter().find_map(|s| s.get("musicCardShelfRenderer"))?;
    let title = run_text(items(card, "title.runs"));
    let to = at(card, "title.runs.0.navigationEndpoint")?;
    let sub = split_sub(items(card, "subtitle.runs"));
    let watch = text(to, "watchEndpoint.videoId");
    let video = text(to, "watchEndpoint.watchEndpointMusicSupportedConfigs.watchEndpointMusicConfig.musicVideoType")
        .is_some_and(|kind| kind != "MUSIC_VIDEO_TYPE_ATV");
    match build(&title, watch, text(to, "browseEndpoint.browseId"), sub, card)? {
        Item::Track(t) if video => Some(Hit::Track(clean_video(Track { video: true, ..t }))),
        item => Some(hit(item)),
    }
}

fn search_filtered(page: &Value, filter: Filter) -> Result<SearchResult> {
    check_search(page)?;
    let mut out = SearchResult::default();
    if filter == Filter::Videos {
        out.videos = video_tracks(page);
        return Ok(out);
    }
    for item in search_items(page) {
        match item {
            Item::Track(t) => out.tracks.push(t),
            Item::Album(a) => out.albums.push(a),
            Item::Artist(a) => out.artists.push(a),
            Item::Playlist(p) => out.playlists.push(p),
        }
    }
    Ok(out)
}

fn check_search(page: &Value) -> Result<()> {
    let list = at(page, "contents.tabbedSearchResultsRenderer.tabs.0.tabRenderer.content.sectionListRenderer.contents")
        .and_then(Value::as_array)
        .context("the search answer has no result list")
        .context(Code::YtParse)?;
    let known = |s: &Value| ["musicShelfRenderer", "itemSectionRenderer", "musicCardShelfRenderer"].iter().any(|k| s.get(*k).is_some());
    if !list.is_empty() && !list.iter().any(known) {
        return Err(Code::YtParse.err("the search answer has only sections this build cannot read"));
    }
    Ok(())
}

fn watch_track(watch: &Value, video_id: &str) -> Option<Track> {
    let mut panels = Vec::new();
    collect(watch, "playlistPanelVideoRenderer", &mut panels);
    panels.into_iter().find(|r| text(r, "videoId") == Some(video_id)).and_then(panel_track)
}

fn suggestions(resp: &Value) -> Suggestions {
    let mut out = Suggestions::default();
    for c in items(resp, "contents").iter().flat_map(|s| items(s, "searchSuggestionsSectionRenderer.contents")) {
        if let Some(q) = text(c, "searchSuggestionRenderer.navigationEndpoint.searchEndpoint.query") {
            out.queries.push(q.into());
        } else if let Some(item) = c.get("musicResponsiveListItemRenderer").map_or_else(|| entry(c), |r| list_item(r, true)) {
            out.hits.push(hit(item));
        }
    }
    out
}

fn hit(item: Item) -> Hit {
    match item {
        Item::Track(t) => Hit::Track(t),
        Item::Album(a) => Hit::Album(a),
        Item::Artist(a) => Hit::Artist(a),
        Item::Playlist(p) => Hit::Playlist(p),
    }
}

fn album_page(resp: &Value, id: String) -> Result<Album> {
    let card = header_card(resp, &id)?;
    let mut tracks = tracks_of(deep(resp, "musicShelfRenderer"));
    fill(&mut tracks, &card);
    let kind = if card.kind.is_empty() { "Album".into() } else { card.kind };
    Ok(Album { id, title: card.title, artists: card.artists, year: card.year, kind, thumb: card.thumb, tracks })
}

fn playlist_page(resp: &Value, id: &str) -> Result<(Album, Option<String>)> {
    let card = header_card(resp, id)?;
    let shelf = deep(resp, "musicPlaylistShelfRenderer").or_else(|| deep(resp, "musicShelfRenderer"));
    let next = shelf.and_then(|s| chunk(s).1);
    let album = Album {
        id: id.into(),
        title: card.title,
        artists: card.artists,
        year: card.year,
        kind: "playlist".into(),
        thumb: card.thumb,
        tracks: tracks_of(shelf),
    };
    Ok((album, next))
}

fn tracks_in(list: &[Value]) -> Vec<Track> {
    list.iter()
        .filter_map(entry)
        .filter_map(|i| match i {
            Item::Track(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn artist_page(resp: &Value, id: &str) -> Result<Artist> {
    let header = deep(resp, "musicImmersiveHeaderRenderer")
        .or_else(|| deep(resp, "musicVisualHeaderRenderer"))
        .context("page has no artist header")
        .context(Code::YtParse)?;
    let image = thumb(header);
    let name = run_text(items(header, "title.runs"));
    let mut artist = Artist {
        id: id.into(),
        thumb: thumb_or(header.get("foregroundThumbnail"), &image),
        banner: image,
        subscribers: run_text(items(header, "subscriptionButton.subscribeButtonRenderer.subscriberCountText.runs")),
        description: run_text(items(header, "description.runs")),
        radio: watch_link(header, "startRadioButton"),
        shuffle: watch_link(header, "playButton"),
        name: name.clone(),
        ..Default::default()
    };
    let by = Ref { id: id.into(), name };
    for section in sections(resp, "singleColumnBrowseResultsRenderer") {
        if let Some(shelf) = section.get("musicShelfRenderer") {
            if artist.top.is_empty() {
                artist.top = tracks_of(Some(shelf));
                artist.songs = songs_link(shelf);
            }
        } else if let Some(shelf) = section.get("musicCarouselShelfRenderer") {
            let found = items(shelf, "contents").iter().filter_map(entry);
            match shelf_title(shelf) {
                "Albums" => {
                    artist.albums = albums_of(found, &by);
                    artist.more_albums = more_link(shelf);
                }
                "Singles" | "Singles & EPs" => {
                    artist.singles = albums_of(found, &by);
                    artist.more_singles = more_link(shelf);
                }
                "Fans might also like" => {
                    artist.related = found
                        .filter_map(|i| match i {
                            Item::Artist(a) => Some(a),
                            _ => None,
                        })
                        .collect()
                }
                _ => {}
            }
        } else if artist.description.is_empty()
            && let Some(d) = section.get("musicDescriptionShelfRenderer")
        {
            artist.description = run_text(items(d, "description.runs"));
        }
    }
    Ok(artist)
}

fn songs_link(shelf: &Value) -> String {
    let id = text(shelf, "bottomEndpoint.browseEndpoint.browseId")
        .or_else(|| text(shelf, "title.runs.0.navigationEndpoint.browseEndpoint.browseId"))
        .unwrap_or_default();
    id.strip_prefix("VL").filter(|id| browse_safe(id)).unwrap_or_default().into()
}

fn more_link(shelf: &Value) -> Option<Browse> {
    let head = at(shelf, "header.musicCarouselShelfBasicHeaderRenderer")?;
    let to = at(head, "moreContentButton.buttonRenderer.navigationEndpoint.browseEndpoint")
        .or_else(|| at(head, "title.runs.0.navigationEndpoint.browseEndpoint"))?;
    let id = text(to, "browseId").filter(|id| browse_safe(id))?;
    Some(Browse { id: id.into(), params: text(to, "params").unwrap_or_default().into() })
}

fn token(holder: &Value, list: &[Value]) -> Option<String> {
    text(holder, "continuations.0.nextContinuationData.continuation")
        .or_else(|| list.iter().rev().find_map(|e| text(e, "continuationItemRenderer.continuationEndpoint.continuationCommand.token")))
        .filter(|t| !t.is_empty())
        .map(String::from)
}

fn chunk(holder: &Value) -> (&[Value], Option<String>) {
    let list = holder.get("items").or_else(|| holder.get("contents")).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    (list, token(holder, list))
}

fn continued(resp: &Value) -> (&[Value], Option<String>) {
    if let Some(holder) = at(resp, "continuationContents").and_then(Value::as_object).and_then(|m| m.values().next()) {
        return chunk(holder);
    }
    items(resp, "onResponseReceivedActions")
        .iter()
        .find_map(|a| at(a, "appendContinuationItemsAction.continuationItems").and_then(Value::as_array))
        .map_or((&[][..], None), |list| (list.as_slice(), token(&Value::Null, list)))
}

fn cards(list: &[Value], by: &Ref) -> Vec<AlbumCard> {
    albums_of(list.iter().filter_map(entry), by)
}

fn full_or_shown(mut all: Vec<AlbumCard>, shown: &[AlbumCard]) -> Vec<AlbumCard> {
    if all.is_empty() {
        return shown.to_vec();
    }
    let mut seen = HashSet::new();
    all.retain(|a| seen.insert(a.id.clone()));
    all
}

fn discography_page(resp: &Value, by: &Ref) -> (Vec<AlbumCard>, Option<String>) {
    let Some(holder) = deep(resp, "gridRenderer").or_else(|| deep(resp, "musicCarouselShelfRenderer")) else {
        return (Vec::new(), None);
    };
    let (list, next) = chunk(holder);
    (cards(list, by), next)
}

fn watch_link(header: &Value, button: &str) -> Option<Browse> {
    let to = at(header, &format!("{button}.buttonRenderer.navigationEndpoint.watchPlaylistEndpoint"))
        .or_else(|| at(header, &format!("{button}.buttonRenderer.navigationEndpoint.watchEndpoint")))?;
    let id = text(to, "playlistId").filter(|id| browse_safe(id))?;
    Some(Browse { id: id.into(), params: text(to, "params").unwrap_or_default().into() })
}

fn mix_tracks(resp: &Value) -> Vec<Track> {
    let mut panels = Vec::new();
    collect(resp, "playlistPanelVideoRenderer", &mut panels);
    panels.into_iter().filter_map(panel_track).collect()
}

fn radio_tracks(resp: &Value, seed: &TrackId) -> Result<Vec<Track>> {
    deep(resp, "playlistPanelRenderer").context("the radio answer has no track list").context(Code::YtParse)?;
    Ok(mix_tracks(resp).into_iter().filter(|t| t.id != *seed).collect())
}

fn mood_buttons(page: &Value) -> Vec<(String, String)> {
    let mut buttons = Vec::new();
    collect(page, "musicNavigationButtonRenderer", &mut buttons);
    buttons
        .into_iter()
        .filter_map(|b| Some((text(b, "buttonText.runs.0.text")?.to_string(), text(b, "clickCommand.browseEndpoint.params")?.to_string())))
        .collect()
}

fn lyrics_browse_id(watch: &Value) -> Option<&str> {
    let tabs = items(watch, "contents.singleColumnMusicWatchNextResultsRenderer.tabbedRenderer.watchNextTabbedResultsRenderer.tabs");
    tabs.iter().find_map(|t| text(t, "tabRenderer.endpoint.browseEndpoint.browseId").filter(|id| id.starts_with("MPLY")))
}

fn lyrics_text(page: &Value) -> Option<String> {
    let lyrics = deep(page, "musicDescriptionShelfRenderer").map(|d| run_text(items(d, "description.runs")));
    lyrics.filter(|l| !l.trim().is_empty())
}

fn home_shelves(resp: &Value) -> Result<Vec<Shelf>> {
    at(resp, "contents.singleColumnBrowseResultsRenderer.tabs.0.tabRenderer.content.sectionListRenderer.contents")
        .and_then(Value::as_array)
        .context("the home answer has no section list")
        .context(Code::YtParse)?;
    let mut shelves = Vec::new();
    for section in sections(resp, "singleColumnBrowseResultsRenderer") {
        let Some(raw) = section.get("musicCarouselShelfRenderer") else { continue };
        let mut shelf = Shelf { title: shelf_title(raw).into(), ..Default::default() };
        for item in items(raw, "contents").iter().filter_map(entry) {
            match item {
                Item::Track(t) => shelf.tracks.push(t),
                Item::Album(a) => shelf.albums.push(a),
                Item::Artist(a) => shelf.artists.push(a),
                Item::Playlist(_) => {}
            }
        }
        if !(shelf.tracks.is_empty() && shelf.albums.is_empty() && shelf.artists.is_empty()) {
            shelves.push(shelf);
        }
    }
    Ok(shelves)
}

fn thumb_or(v: Option<&Value>, fallback: &str) -> String {
    v.map(|v| thumb(&json!({"thumbnail": v}))).filter(|t| !t.is_empty()).unwrap_or_else(|| fallback.into())
}

fn albums_of(found: impl Iterator<Item = Item>, by: &Ref) -> Vec<AlbumCard> {
    found
        .filter_map(|i| match i {
            Item::Album(mut a) => {
                if a.artists.is_empty() {
                    a.artists = vec![by.clone()];
                }
                Some(a)
            }
            _ => None,
        })
        .collect()
}

fn video_tracks(resp: &Value) -> Vec<Track> {
    search_items(resp)
        .into_iter()
        .filter_map(|i| match i {
            Item::Track(t) => Some(clean_video(Track { video: true, ..t })),
            _ => None,
        })
        .collect()
}

fn search_items(resp: &Value) -> Vec<Item> {
    sections(resp, "tabbedSearchResultsRenderer")
        .iter()
        .filter_map(|s| s.get("musicShelfRenderer").or_else(|| s.get("itemSectionRenderer")))
        .flat_map(|s| items(s, "contents"))
        .filter_map(entry)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn ytm() -> Ytm {
        Ytm::new(reqwest::Client::new())
    }

    fn show(t: &Track) -> String {
        format!("{} | {} | {}s | {} | {:?} | {}", t.id, t.title, t.duration, t.artist_line(), t.album.as_ref().map(|a| &a.name), t.thumb)
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("3:45"), Some(225.0));
        assert_eq!(parse_duration("1:02:03"), Some(3723.0));
        assert_eq!(parse_duration("2020"), None);
        assert_eq!(parse_duration("1.2M plays"), None);
    }

    const ID: &str = "dQw4w9WgXcQ";

    #[test]
    fn parsed_tracks_carry_the_prefix_and_requests_get_the_bare_id() {
        let panel = json!({ "videoId": ID, "title": { "runs": [{ "text": "Song" }] }, "lengthText": { "runs": [{ "text": "3:32" }] } });
        let track = panel_track(&panel).unwrap();
        assert_eq!(track.id, "yt:dQw4w9WgXcQ");
        assert_eq!(video(&track.id).unwrap(), ID);
        assert_eq!(watch_url(&track.id).as_deref(), Some("https://music.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(video(&TrackId::parse("sc:123")).is_err() && watch_url(&TrackId::parse("short")).is_none());
    }

    #[test]
    fn links_give_a_video_id() {
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?v=dQw4w9WgXcQ&list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG&index=2",
            "http://www.youtube.com/watch?feature=share&v=dQw4w9WgXcQ&t=43s",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?si=abcdef&t=10",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&si=xyz",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "youtube.com/watch?v=dQw4w9WgXcQ",
            "www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ?start=3",
            "https://www.youtube.com/live/dQw4w9WgXcQ",
            "  https://youtu.be/dQw4w9WgXcQ  ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(video_id(link).as_deref(), Some(ID), "{link}");
        }
    }

    #[test]
    fn other_input_is_not_a_link() {
        for q in [
            "dQw4w9WgXcQ",
            "firstlights",
            "radiohead creep",
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
            "https://www.youtube.com/@channel",
            "https://youtu.be/short",
            "https://www.youtube.com/watch?v=tooLongToBeAnId",
            "",
        ] {
            assert_eq!(video_id(q), None, "{q}");
        }
    }

    #[test]
    fn links_to_albums_artists_and_playlists_open_their_page() {
        let page = |kind, id: &str| Some(Page { kind, id: id.into() });
        for (link, want) in [
            ("https://music.youtube.com/browse/MPREb_4pL8gzRtw1p", page(Kind::Album, "MPREb_4pL8gzRtw1p")),
            (
                "https://music.youtube.com/playlist?list=OLAK5uy_nMr9h2VlS-2PULNz3M3XVXQj_P3C2bqaY&si=x",
                page(Kind::Album, "OLAK5uy_nMr9h2VlS-2PULNz3M3XVXQj_P3C2bqaY"),
            ),
            (
                "https://www.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
                page(Kind::Playlist, "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG"),
            ),
            ("music.youtube.com/browse/VLPLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG", page(Kind::Playlist, "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG")),
            ("https://music.youtube.com/channel/UCiMhD4jzUqG-IgPzUmmytRQ", page(Kind::Artist, "UCiMhD4jzUqG-IgPzUmmytRQ")),
            ("https://www.youtube.com/channel/UCiMhD4jzUqG-IgPzUmmytRQ/videos", page(Kind::Artist, "UCiMhD4jzUqG-IgPzUmmytRQ")),
            ("https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG", None),
            ("https://www.youtube.com/@channel", None),
            ("https://example.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG", None),
            ("https://music.youtube.com/playlist?list=a%22b", None),
            ("queen", None),
        ] {
            assert_eq!(page_link(link), want, "{link}");
        }
    }

    #[test]
    fn share_urls_open_the_same_page() {
        for (kind, id) in [
            (Kind::Album, "MPREb_4pL8gzRtw1p"),
            (Kind::Album, "OLAK5uy_nMr9h2VlS"),
            (Kind::Artist, "UCiMhD4jzUqG-IgPzUmmytRQ"),
            (Kind::Playlist, "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG"),
        ] {
            let url = share_url(kind, id).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(page_link(&url), Some(Page { kind, id: id.into() }), "{url}");
        }
        assert_eq!(share_url(Kind::Track, "yt:dQw4w9WgXcQ").as_deref(), Some("https://music.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert_eq!(share_url(Kind::Track, "dQw4w9WgXcQ").as_deref(), Some("https://music.youtube.com/watch?v=dQw4w9WgXcQ"));
        for (kind, id) in [(Kind::Track, "sc:1"), (Kind::Artist, "x"), (Kind::Album, ""), (Kind::Playlist, "a/b"), (Kind::Album, "a?b")] {
            assert_eq!(share_url(kind, id), None, "{id}");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_videos_filter_and_all_section() {
        let y = ytm();
        for q in ["IVOXYGEN", "KEDELA SATURN", "burgos lost cause"] {
            let r = y.search(q, Filter::Videos).await.unwrap();
            println!("videos {q:?}: {}", r.videos.len());
            r.videos.iter().take(6).for_each(|t| println!("  {} | {} | {}", t.id, t.title, t.artist_line()));
            assert!(!r.videos.is_empty() && r.videos.iter().all(|t| t.video && t.id.video_id().is_some() && !t.title.is_empty()), "{q}");
            assert!(r.tracks.is_empty());
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let all = y.search("phonk drift", Filter::All).await.unwrap();
        println!("all: tracks={} videos={}", all.tracks.len(), all.videos.len());
        assert!(!all.tracks.is_empty() && !all.videos.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn live_video_titles_are_split() {
        let y = ytm();
        let mut checked = 0;
        for q in ["phonk drift mix", "KEDELA", "slowed reverb phonk"] {
            for t in y.search(q, Filter::Videos).await.unwrap().videos {
                println!("{} | {}", t.title, t.artist_line());
                assert!(
                    !t.title.contains("[Wave")
                        && !t.title.to_lowercase().contains("(official")
                        && !t.title.to_lowercase().contains("[free"),
                    "{}",
                    t.title
                );
                assert!(!t.artists.is_empty() && t.artists.iter().all(|a| !a.name.is_empty()));
                checked += 1;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert!(checked >= 10);
    }

    #[tokio::test]
    #[ignore]
    async fn live_pasted_links_open_the_track() {
        let y = ytm();
        let found = y.search("IVOXYGEN", Filter::Videos).await.unwrap().videos;
        let mut ids = vec![ID.to_string()];
        ids.extend(found.iter().take(2).filter_map(|t| t.id.video_id().map(String::from)));
        for id in ids {
            for link in [
                format!("https://www.youtube.com/watch?v={id}"),
                format!("https://youtu.be/{id}"),
                format!("https://music.youtube.com/watch?v={id}&list=RDAMVM{id}"),
            ] {
                for filter in [Filter::All, Filter::Songs] {
                    let r = y.search(&link, filter).await.unwrap();
                    let linked = r.linked.unwrap_or_else(|| panic!("no track for {link}"));
                    println!("{link} -> {} | {} | {}", linked.id, linked.title, linked.artist_line());
                    assert_eq!(linked.id, TrackId::youtube(&id));
                    assert!(linked.duration > 0.0 && !linked.title.is_empty());
                }
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
        }
        let gone = y.search("https://youtu.be/AAAAAAAAAAA", Filter::All).await.unwrap();
        assert!(gone.linked.is_none());
    }

    #[tokio::test]
    #[ignore]
    async fn live_search() {
        let y = ytm();
        let t = Instant::now();
        let r = y.search("daft punk", Filter::All).await.unwrap();
        println!(
            "search all {:?}: top={:?} tracks={} albums={} artists={}",
            t.elapsed(),
            r.top.is_some(),
            r.tracks.len(),
            r.albums.len(),
            r.artists.len()
        );
        r.tracks.iter().take(3).for_each(|t| println!("  {}", show(t)));
        println!("  {:?}\n  {:?}\n  {:?}", r.top, r.albums.first(), r.artists.first());
        assert!(r.top.is_some() && !r.tracks.is_empty() && !r.albums.is_empty() && !r.artists.is_empty());
        for f in [Filter::Songs, Filter::Albums, Filter::Artists] {
            let t = Instant::now();
            let r = y.search("radiohead", f).await.unwrap();
            println!("search {f:?} {:?}: tracks={} albums={} artists={}", t.elapsed(), r.tracks.len(), r.albums.len(), r.artists.len());
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_suggest() {
        let s = ytm().suggest("radio").await.unwrap();
        println!("{s:?}");
        assert!(!s.queries.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn live_album() {
        let y = ytm();
        let r = y.search("random access memories", Filter::Albums).await.unwrap();
        let card = r.albums.first().unwrap();
        println!("{card:?}");
        let a = y.album(&card.id).await.unwrap();
        println!("{} | {:?} | {} | {} | tracks={} thumb={}", a.title, a.artists, a.year, a.kind, a.tracks.len(), a.thumb);
        a.tracks.iter().take(3).for_each(|t| println!("  {}", show(t)));
        assert!(a.tracks.len() > 5 && a.tracks.iter().all(|t| !t.thumb.is_empty() && t.album.is_some() && t.duration > 0.0));
        let resp = y.call("browse", json!({"browseId": a.id})).await.unwrap();
        let olak = deep(&resp, "playlistId").and_then(Value::as_str).unwrap().to_string();
        let again = y.album(&olak).await.unwrap();
        println!("{olak} -> {} {}", again.id, again.tracks.len());
        assert_eq!(again.id, a.id);
    }

    #[tokio::test]
    #[ignore]
    async fn live_playlist() {
        let p = ytm().playlist("PLMC9KNkIncKvYin_USF1qoJQnIyMAfRxl").await.unwrap();
        println!("{} | {} | {:?} | tracks={}", p.title, p.kind, p.artists, p.tracks.len());
        p.tracks.iter().take(3).for_each(|t| println!("  {}", show(t)));
        assert!(!p.tracks.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn live_artist() {
        let y = ytm();
        let r = y.search("daft punk", Filter::Artists).await.unwrap();
        let t = Instant::now();
        let a = y.artist(&r.artists[0].id).await.unwrap();
        println!("artist {:?}: {} subs={:?} thumb={} banner={}", t.elapsed(), a.name, a.subscribers, a.thumb, a.banner);
        println!(
            "  desc={:.80} top={} albums={} singles={} related={}",
            a.description,
            a.top.len(),
            a.albums.len(),
            a.singles.len(),
            a.related.len()
        );
        a.top.iter().take(2).for_each(|t| println!("  {}", show(t)));
        println!("  {:?}", a.albums.first());
        assert!(!a.top.is_empty() && !a.albums.is_empty() && !a.banner.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn live_discography() {
        let y = ytm();
        for id in ["UCEPMVbUzImPl4p8k4LkGevA", "UCuAXFkgsw1L7xaCfnd5JJOw"] {
            let a = y.artist(id).await.unwrap();
            let d = y.discography(&a).await.unwrap();
            println!("{}: page {}+{}, all {}+{}", a.name, a.albums.len(), a.singles.len(), d.albums.len(), d.singles.len());
            assert!(d.albums.len() >= a.albums.len() && d.singles.len() >= a.singles.len());
            assert!(d.albums.iter().chain(&d.singles).all(|c| c.id.starts_with("MPRE") && !c.title.is_empty()));
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_artist_songs_and_long_playlists() {
        let y = ytm();
        let a = y.artist("UCEPMVbUzImPl4p8k4LkGevA").await.unwrap();
        println!("songs list {:?}, top {}", a.songs, a.top.len());
        if !a.songs.is_empty() {
            assert!(y.playlist(&a.songs).await.unwrap().tracks.len() >= a.top.len());
        }
        let long = y.playlist("PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI").await.unwrap();
        println!("long playlist: {}", long.tracks.len());
        assert!(long.tracks.iter().all(|t| t.id.video_id().is_some()));
    }

    #[tokio::test]
    #[ignore]
    async fn live_artist_mixes() {
        let y = ytm();
        let a = y.artist("UCEPMVbUzImPl4p8k4LkGevA").await.unwrap();
        for list in [a.radio.as_ref(), a.shuffle.as_ref()].into_iter().flatten() {
            let tracks = y.mix(list).await.unwrap();
            println!("{}: {}", list.id, tracks.len());
            assert!(!tracks.is_empty() && tracks.iter().all(|t| t.id.video_id().is_some()));
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_radio() {
        let t = Instant::now();
        let r = ytm().radio(&TrackId::youtube(ID)).await.unwrap();
        println!("radio {:?}: {}", t.elapsed(), r.len());
        r.iter().take(3).for_each(|t| println!("  {}", show(t)));
        assert!(r.len() > 10 && r.iter().all(|t| t.id != TrackId::youtube(ID)));
    }

    #[tokio::test]
    #[ignore]
    async fn live_lyrics() {
        let l = ytm().lyrics(&TrackId::youtube(ID)).await.unwrap().unwrap();
        println!("{} chars: {:.60}", l.len(), l);
        assert!(l.contains("Never gonna give you up"));
    }

    #[tokio::test]
    #[ignore]
    async fn live_home() {
        let t = Instant::now();
        let h = ytm().home().await.unwrap();
        println!("home {:?}: {} shelves", t.elapsed(), h.len());
        for s in &h {
            println!("  {} | tracks={} albums={} artists={}", s.title, s.tracks.len(), s.albums.len(), s.artists.len());
        }
        assert!(!h.is_empty());
    }
}

#[cfg(test)]
mod fixtures {
    use snellar_core::error::Code;

    use super::*;

    fn load(raw: &str) -> Value {
        serde_json::from_str(raw).unwrap()
    }

    const SONGS: &str = include_str!("../../../tests/fixtures/youtube/search-songs-queen-2026-11-03.json");
    const MIXED: &str = include_str!("../../../tests/fixtures/youtube/search-all-queen-2026-11-03.json");
    const VIDEOS: &str = include_str!("../../../tests/fixtures/youtube/search-videos-queen-2026-11-03.json");
    const SUGGEST: &str = include_str!("../../../tests/fixtures/youtube/suggest-quee-2026-11-03.json");
    const NEXT: &str = include_str!("../../../tests/fixtures/youtube/next-dqw4w9wgxcq-2026-11-03.json");
    const LYRICS: &str = include_str!("../../../tests/fixtures/youtube/lyrics-dqw4w9wgxcq-2026-11-03.json");
    const RADIO: &str = include_str!("../../../tests/fixtures/youtube/radio-dqw4w9wgxcq-2026-11-03.json");
    const ARTIST: &str = include_str!("../../../tests/fixtures/youtube/artist-queen-2026-11-03.json");
    const PLAYLIST: &str = include_str!("../../../tests/fixtures/youtube/playlist-queen-pl2t-2026-11-03.json");
    const HOME: &str = include_str!("../../../tests/fixtures/youtube/home-2026-11-03.json");
    const QUEEN: &str = "UCEPMVbUzImPl4p8k4LkGevA";
    const RICK: &str = "dQw4w9WgXcQ";

    fn playable(t: &Track) -> bool {
        t.id.video_id().is_some() && !t.title.is_empty() && t.duration > 0.0 && !t.thumb.is_empty()
    }

    #[test]
    fn a_reduced_catalog_search_reads_the_top_artist_the_artists_and_the_videos() {
        let r = search_all(&load(SONGS), &load(MIXED), &load(VIDEOS)).unwrap();
        let Some(Hit::Artist(top)) = r.top else { panic!("top artist card") };
        assert_eq!((top.id.as_str(), top.name.as_str()), (QUEEN, "Queen"));
        assert!(!top.subtitle.is_empty() && top.thumb.ends_with("=w544-h544-l90-rj"), "{top:?}");
        assert_eq!(r.artists.len(), 6);
        assert!(r.artists.iter().all(|a| a.id.starts_with("UC") && !a.name.is_empty()), "{:?}", r.artists);
        assert_eq!(r.videos.len(), 20);
        assert!(r.videos.iter().all(|v| v.video && playable(v)), "{:?}", r.videos.iter().find(|v| !playable(v)));
        assert_eq!(r.videos[0].artists.first().map(|a| a.id.as_str()), Some(QUEEN));
        assert!(r.tracks.is_empty() && r.albums.is_empty(), "the reduced catalog has no songs or albums");
        assert_eq!(r.playlists.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["PLKg5chIaZMD0", "PLP-gn2q7QgG4", "PLZr6N9My42EY"]);
        let list = &r.playlists[0];
        assert_eq!((list.title.as_str(), list.kind.as_str()), ("Queen + Paul Rodgers - Return of the Champions", "Playlist"));
        assert_eq!(list.artists.first().map(|a| a.name.as_str()), Some("Google User"));
        assert!(r.playlists.iter().all(|p| !p.thumb.is_empty()));
    }

    const TOP_SONG: &str = include_str!("../../../tests/fixtures/youtube/search-top-song-handmade-2026-11-03.json");
    const TOP_VIDEO: &str = include_str!("../../../tests/fixtures/youtube/search-top-video-home-2026-11-03.json");
    const TOP_ALBUM: &str = include_str!("../../../tests/fixtures/youtube/search-top-album-home-2026-11-03.json");
    const TOP_PLAYLIST: &str = include_str!("../../../tests/fixtures/youtube/search-top-playlist-handmade-2026-11-03.json");

    #[test]
    fn the_top_result_can_be_a_song_a_video_an_album_or_a_playlist() {
        let Some(Hit::Track(song)) = top_result(&load(TOP_SONG)) else { panic!("a song") };
        assert_eq!((song.id.as_str(), song.title.as_str()), ("yt:fJ9rUzIMcZQ", "Bohemian Rhapsody"));
        assert_eq!((song.duration, song.video), (355.0, false));
        assert_eq!((song.artist_line(), song.album.map(|a| a.name)), ("Queen".into(), Some("A Night At The Opera".into())));
        let Some(Hit::Track(video)) = top_result(&load(TOP_VIDEO)) else { panic!("a video") };
        assert!(video.video);
        assert_eq!((video.id.as_str(), video.title.as_str(), video.duration), ("yt:dQw4w9WgXcQ", "Never Gonna Give You Up", 214.0));
        let Some(Hit::Album(album)) = top_result(&load(TOP_ALBUM)) else { panic!("an album") };
        assert_eq!((album.title.as_str(), album.year.as_str(), album.kind.as_str()), ("A Night At The Opera", "1975", "Album"));
        let Some(Hit::Playlist(list)) = top_result(&load(TOP_PLAYLIST)) else { panic!("a playlist") };
        assert_eq!(list.id, "PLqueenGreatestHitsHandmade");
        let mut broken = load(TOP_SONG);
        *broken.pointer_mut("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicCardShelfRenderer/title").unwrap() = json!("x");
        assert!(top_result(&broken).is_none() && top_result(&json!({})).is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn background_matching_does_not_wait_for_paging_or_for_import() {
        let long = load(LONG);
        let next = load(LONG_NEXT);
        let empty = load(SONGS);
        let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let gate = crate::fake_music::Gate { when: "continuation", permits: permits.clone() };
        let music = crate::fake_music::serve(
            move |body| {
                if body.get("query").is_some() {
                    empty.clone()
                } else if body.get("continuation").is_some() {
                    next.clone()
                } else {
                    long.clone()
                }
            },
            Some(gate),
        )
        .await;
        let ytm = music.ytm.clone();
        let paging = tokio::spawn({
            let ytm = ytm.clone();
            async move { ytm.playlist("PLlong").await }
        });
        for _ in 0..400 {
            if music.seen.lock().unwrap().iter().any(|b| b.get("continuation").is_some()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(music.seen.lock().unwrap().iter().any(|b| b.get("continuation").is_some()), "paging began");
        let started = std::time::Instant::now();
        let quiet = tokio::time::timeout(Duration::from_secs(3), ytm.search_quiet("x", Filter::Songs)).await;
        assert!(quiet.is_ok_and(|r| r.is_ok()), "a quiet search waited {:?} behind paging", started.elapsed());
        permits.add_permits(100);
        assert!(paging.await.unwrap().unwrap().tracks.len() > 100, "paging still reads every page");

        let held = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let gate = crate::fake_music::Gate { when: "import me", permits: held.clone() };
        let quiet_answer = load(SONGS);
        let music = crate::fake_music::serve(move |_| quiet_answer.clone(), Some(gate)).await;
        let ytm = music.ytm.clone();
        let importing = tokio::spawn({
            let ytm = ytm.clone();
            async move { ytm.search_background("import me", Filter::Songs).await }
        });
        for _ in 0..400 {
            if !music.seen.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let quiet = tokio::time::timeout(Duration::from_secs(3), ytm.search_quiet("other", Filter::Songs)).await;
        assert!(quiet.is_ok_and(|r| r.is_ok()), "a quiet search waited behind an import search");
        held.add_permits(1);
        importing.await.unwrap().unwrap();
    }

    #[test]
    fn a_row_with_only_a_watch_link_is_a_hit_in_suggestions_and_nothing_in_lists() {
        let row = json!({"musicResponsiveListItemRenderer": {
            "flexColumns": [{"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "A clip"}]}}}],
            "navigationEndpoint": {"watchEndpoint": {"videoId": "dQw4w9WgXcQ"}}
        }});
        assert!(entry(&row).is_none(), "track parsing that feeds recommendations reads playlistItemData only");
        let answer = json!({"contents": [{"searchSuggestionsSectionRenderer": {"contents": [row]}}]});
        assert!(matches!(&suggestions(&answer).hits[..], [Hit::Track(t)] if t.title == "A clip"));
    }

    #[test]
    fn a_search_with_no_results_is_empty_not_an_error() {
        let r = search_filtered(&load(SONGS), Filter::Songs).unwrap();
        assert!(r.tracks.is_empty() && r.albums.is_empty() && r.artists.is_empty() && r.videos.is_empty() && r.top.is_none());
    }

    #[test]
    fn an_answer_of_an_unknown_shape_is_a_parse_error_not_an_empty_list() {
        let parse = |e: anyhow::Error| Code::of(&e) == Code::YtParse;
        let unknown = json!({"contents": {"somethingNewRenderer": {"tabs": []}}});
        assert!(search_filtered(&unknown, Filter::Songs).is_err_and(parse));
        assert!(search_filtered(&unknown, Filter::Videos).is_err_and(parse));
        assert!(search_all(&unknown, &load(MIXED), &load(VIDEOS)).is_err_and(parse));
        let renamed = json!({"contents": {"tabbedSearchResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [{"musicResultsV2Renderer": {}}]}}}}]}}});
        assert!(search_filtered(&renamed, Filter::Songs).is_err_and(parse), "sections nobody can read are not no results");
        assert!(radio_tracks(&unknown, &TrackId::youtube("dQw4w9WgXcQ")).is_err_and(parse));
        assert!(home_shelves(&unknown).is_err_and(parse));
    }

    #[test]
    fn discography_keeps_the_albums_shown_when_the_full_list_comes_back_empty() {
        let card = |id: &str| AlbumCard { id: id.into(), ..AlbumCard::default() };
        let shown = [card("MPREa"), card("MPREb")];
        let by = Ref { id: "UCx".into(), name: "x".into() };
        assert_eq!(discography_page(&json!({"contents": {"somethingNew": {}}}), &by).0.len(), 0);
        assert_eq!(full_or_shown(Vec::new(), &shown).len(), 2);
        assert_eq!(
            full_or_shown(vec![card("MPREc"), card("MPREc"), card("MPREa")], &shown).len(),
            2,
            "a real full list wins, without repeats"
        );
    }

    const HOME_SONGS: &str = include_str!("../../../tests/fixtures/youtube/search-songs-home-queen-2026-11-03.json");
    const HOME_MIXED: &str = include_str!("../../../tests/fixtures/youtube/search-all-home-queen-2026-11-03.json");
    const HOME_VIDEOS: &str = include_str!("../../../tests/fixtures/youtube/search-videos-home-queen-2026-11-03.json");

    #[test]
    fn a_home_connection_search_has_songs_albums_artists_videos_and_lists() {
        let r = search_all(&load(HOME_SONGS), &load(HOME_MIXED), &load(HOME_VIDEOS)).unwrap();
        assert!(matches!(&r.top, Some(Hit::Artist(a)) if a.id == QUEEN), "{:?}", r.top);
        assert_eq!((r.tracks.len(), r.albums.len(), r.artists.len(), r.videos.len(), r.playlists.len()), (20, 3, 6, 20, 6));
        assert!(r.tracks.iter().all(playable), "{:?}", r.tracks.iter().find(|t| !playable(t)));
        assert!(r.tracks.iter().all(|t| !t.artists.is_empty() && !t.video));
        assert!(r.albums.iter().all(|a| a.id.starts_with("MPRE") && a.year.len() == 4 && !a.artists.is_empty()), "{:?}", r.albums);
        assert!(r.artists.iter().all(|a| a.id.starts_with("UC") && !a.name.is_empty()));
        assert!(r.videos.iter().all(|v| v.video && !v.title.is_empty()));
        let songs = search_filtered(&load(HOME_SONGS), Filter::Songs).unwrap();
        assert_eq!(songs.tracks.len(), 20);
    }

    #[test]
    fn suggestions_keep_their_order() {
        let s = suggestions(&load(SUGGEST));
        assert_eq!(s.queries.len(), 13);
        assert_eq!(&s.queries[..3], ["queen", "queencard", "trap queen"]);
        assert!(s.hits.is_empty(), "the reduced catalog has no second section");
    }

    const HOME_SUGGEST: &str = include_str!("../../../tests/fixtures/youtube/suggest-home-quee-2026-11-03.json");

    #[test]
    fn suggestions_carry_artists_albums_and_songs() {
        let s = suggestions(&load(HOME_SUGGEST));
        assert_eq!(&s.queries[..2], ["queen", "queens of the stone age"]);
        let kinds: Vec<&str> = s
            .hits
            .iter()
            .map(|h| match h {
                Hit::Artist(a) => a.id.as_str(),
                Hit::Album(a) | Hit::Playlist(a) => a.id.as_str(),
                Hit::Track(t) => t.id.as_str(),
            })
            .collect();
        assert_eq!(kinds, [QUEEN, "UCcYgNw9ne8v1cxcAD_sIOWQ", "MPREb_m2xZZHGzRl1", "UC6ghsPzRFFa6WMxaavyYb6w", "yt:v_gO_P8gug8"]);
        let Hit::Album(album) = &s.hits[2] else { panic!("an album") };
        assert_eq!((album.year.as_str(), album.kind.as_str()), ("2018", "Album"));
        let Hit::Track(song) = &s.hits[4] else { panic!("a song") };
        assert_eq!((song.album.as_ref().map(|a| a.name.as_str()), song.duration), (Some("I feel"), 0.0), "a suggestion row has no length");
        let json = serde_json::to_value(&s.hits).unwrap();
        let types: Vec<&str> = json.as_array().unwrap().iter().map(|h| h["type"].as_str().unwrap()).collect();
        assert_eq!(types, ["artist", "artist", "album", "artist", "track"]);
        assert_eq!(json[2]["kind"], "Album", "the album keeps its own kind next to the type");
        let broken = json!([{}, 4, {"musicResponsiveListItemRenderer": {}}]);
        assert!(suggestions(&json!({"contents": [{"searchSuggestionsSectionRenderer": {"contents": broken}}]})).hits.is_empty());
    }

    #[test]
    fn the_watch_page_gives_the_asked_track_and_its_lyrics_tab() {
        let watch = load(NEXT);
        let t = watch_track(&watch, RICK).expect("the track");
        assert_eq!((t.id.as_str(), t.title.as_str(), t.duration), ("yt:dQw4w9WgXcQ", "Never Gonna Give You Up", 214.0));
        assert_eq!(t.artists.first().map(|a| (a.id.as_str(), a.name.as_str())), Some(("UCwZEU0wAwIyZb4x5G_KJp2w", "Rick Astley")));
        assert!(watch_track(&watch, "aaaaaaaaaaa").is_none());
        assert_eq!(lyrics_browse_id(&watch), Some("MPLYt_6AEOVn4k62q"));
        let lyrics = lyrics_text(&load(LYRICS)).expect("lyrics");
        assert!(lyrics.starts_with("We're no strangers to love\n") && lyrics.lines().count() > 20, "{lyrics:.80}");
    }

    #[test]
    fn radio_leaves_out_its_seed() {
        let seed = TrackId::youtube(RICK);
        let radio = radio_tracks(&load(RADIO), &seed).unwrap();
        assert_eq!(radio.len(), 49);
        assert!(radio.iter().all(|t| t.id != seed && playable(t)), "{:?}", radio.iter().find(|t| !playable(t)));
    }

    #[test]
    fn an_artist_page_reads_its_header_even_without_shelves() {
        let a = artist_page(&load(ARTIST), QUEEN).unwrap();
        assert_eq!((a.id.as_str(), a.name.as_str(), a.subscribers.as_str()), (QUEEN, "Queen", "19.1M"));
        assert!(!a.thumb.is_empty() && !a.banner.is_empty());
        assert!(a.description.starts_with("Queen are a British rock band"), "{:.60}", a.description);
        assert!(a.albums.is_empty() && a.singles.is_empty() && a.top.is_empty(), "the reduced catalog has no music shelves");
        assert_eq!(Code::of(&artist_page(&json!({}), QUEEN).unwrap_err()), Code::YtParse);
    }

    #[test]
    fn a_playlist_page_reads_its_header_and_tracks() {
        let (p, next) = playlist_page(&load(PLAYLIST), "PL2tMgWgIvcWkMsdZGTqrmPTpTPYjPTfuw").unwrap();
        assert_eq!(next, None, "the related playlists below the list are not more of it");
        assert_eq!((p.title.as_str(), p.kind.as_str()), ("Queen Live Performances", "playlist"));
        assert_eq!(p.tracks.len(), 37);
        assert!(p.tracks.iter().all(playable), "{:?}", p.tracks.iter().find(|t| !playable(t)));
        assert_eq!(Code::of(&album_page(&json!({}), "MPREb_x".into()).unwrap_err()), Code::YtParse);
    }

    const ARTIST_FULL: &str = include_str!("../../../tests/fixtures/youtube/artist-home-queen-2026-11-03.json");
    const ALBUMS_FULL: &str = include_str!("../../../tests/fixtures/youtube/discography-home-queen-albums-2026-11-03.json");
    const SINGLES: &str = include_str!("../../../tests/fixtures/youtube/discography-home-queen-singles-2026-11-03.json");
    const OPERA: &str = include_str!("../../../tests/fixtures/youtube/album-home-opera-2026-11-03.json");
    const UPCOMING: &str = include_str!("../../../tests/fixtures/youtube/album-home-upcoming-2026-11-03.json");
    const HOME_FULL: &str = include_str!("../../../tests/fixtures/youtube/home-home-2026-11-03.json");
    const CAIRO: &str = include_str!("../../../tests/fixtures/youtube/next-qcuhgxfs-ym-2026-11-03.json");

    const SECTIONS: &str = "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents";

    fn queen() -> Ref {
        Ref { id: QUEEN.into(), name: "Queen".into() }
    }

    #[test]
    fn a_full_artist_page_links_to_its_whole_discography() {
        let a = artist_page(&load(ARTIST_FULL), QUEEN).unwrap();
        assert_eq!((a.albums.len(), a.singles.len(), a.top.len(), a.related.len()), (10, 10, 5, 10));
        let albums = a.more_albums.expect("albums link");
        let singles = a.more_singles.expect("singles link");
        assert_eq!(albums.id, format!("MPAD{QUEEN}"));
        assert_ne!(albums.params, singles.params);
        assert!(a.albums.iter().all(|c| c.artists == [queen()] && c.id.starts_with("MPRE")), "{:?}", a.albums[0]);
    }

    #[test]
    fn the_discography_of_a_home_connection_is_one_whole_page() {
        let (albums, next) = discography_page(&load(ALBUMS_FULL), &queen());
        assert_eq!((albums.len(), next), (57, None), "no continuation, unlike the page the cloud session guessed");
        assert_eq!(albums.iter().map(|a| &a.id).collect::<HashSet<_>>().len(), 57);
        assert!(albums.iter().all(|a| a.kind == "Album" && a.year.len() == 4 && a.artists == [queen()] && !a.thumb.is_empty()));

        let (singles, next) = discography_page(&load(SINGLES), &queen());
        assert_eq!((singles.len(), next), (24, None));
        assert!(singles.iter().all(|s| (s.kind == "Single" || s.kind == "EP") && s.id.starts_with("MPRE")), "{:?}", singles[0]);
    }

    #[test]
    fn a_grid_with_a_token_continues_and_its_next_page_is_read() {
        let row = |id: &str| {
            json!({"musicTwoRowItemRenderer": {
                "title": {"runs": [{"text": "x"}]},
                "navigationEndpoint": {"browseEndpoint": {"browseId": id}}
            }})
        };
        let page = json!({"grid": {"gridRenderer": {"items": [row("MPREa")], "continuations": [{"nextContinuationData": {"continuation": "t2"}}]}}});
        let (first, next) = discography_page(&page, &queen());
        assert_eq!((first.len(), next.as_deref()), (1, Some("t2")));
        let rest = json!({"continuationContents": {"gridContinuation": {"items": [row("MPREb")]}}});
        let (list, more) = continued(&rest);
        assert_eq!((cards(list, &queen()).len(), more), (1, None));
    }

    #[test]
    fn a_real_album_page_has_timed_tracks_and_an_upcoming_one_is_still_an_album() {
        let opera = album_page(&load(OPERA), "MPREb_eEpQf8QskKl".into()).unwrap();
        assert_eq!(
            (opera.title.as_str(), opera.year.as_str(), opera.kind.as_str(), opera.tracks.len()),
            ("A Night At The Opera", "1975", "Album", 12)
        );
        assert!(opera.tracks.iter().all(playable) && opera.tracks.iter().all(|t| t.album.is_some() && !t.artists.is_empty()));
        let upcoming = album_page(&load(UPCOMING), "MPREb_gEXfEPkLxFU".into()).unwrap();
        assert_eq!((upcoming.kind.as_str(), upcoming.year.as_str()), ("Album", "2026"), "the header says Upcoming Album");
        assert_eq!(upcoming.tracks.len(), 27);
    }

    #[test]
    fn home_rows_and_top_songs_carry_no_length_while_the_watch_page_does() {
        let home = home_shelves(&load(HOME_FULL)).unwrap();
        assert_eq!((home.len(), home[0].title.as_str(), home[0].tracks.len()), (1, "Quick picks", 20));
        assert!(
            home[0].tracks.iter().all(|t| t.duration == 0.0 && t.id.video_id().is_some() && !t.title.is_empty()),
            "these rows have no length column"
        );
        let top = artist_page(&load(ARTIST_FULL), QUEEN).unwrap().top;
        assert!(top.iter().all(|t| t.duration == 0.0 && t.album.is_some()), "plays and album, no length");
        let cairo = watch_track(&load(CAIRO), "QCUHgXfs_YM").expect("the track");
        assert_eq!((cairo.title.as_str(), cairo.duration), ("Cairo FM", 182.0));
    }

    #[test]
    fn every_length_a_saved_response_holds_is_understood() {
        fn walk(v: &Value, found: &mut Vec<String>) {
            match v {
                Value::Array(list) => list.iter().for_each(|x| walk(x, found)),
                Value::Object(map) => {
                    found.extend(text(v, "lengthText.runs.0.text").map(str::to_string));
                    found.extend(
                        text(v, "musicResponsiveListItemFixedColumnRenderer.text.runs.0.text")
                            .filter(|t| t.contains(':'))
                            .map(str::to_string),
                    );
                    map.values().for_each(|x| walk(x, found));
                }
                _ => {}
            }
        }
        let mut found = Vec::new();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/youtube");
        for file in std::fs::read_dir(dir).unwrap().flatten() {
            walk(&serde_json::from_slice(&std::fs::read(file.path()).unwrap()).unwrap(), &mut found);
        }
        assert!(found.len() > 300, "{}", found.len());
        let lost: Vec<&String> = found.iter().filter(|t| parse_duration(t).is_none_or(|d| d <= 0.0)).collect();
        assert!(lost.is_empty(), "{lost:?}");
    }

    #[test]
    fn a_broken_discography_page_is_never_a_panic() {
        assert_eq!(discography_page(&json!({}), &queen()).0.len(), 0);
        assert_eq!(continued(&json!({"continuationContents": {"gridContinuation": 3}})).0.len(), 0);
        assert_eq!(continued(&json!({"onResponseReceivedActions": [{}]})).1, None);
        let mut page = load(ALBUMS_FULL);
        let grid = page.pointer_mut(&format!("{SECTIONS}/0/gridRenderer")).unwrap();
        let list = grid["items"].as_array_mut().unwrap();
        list[0]["musicTwoRowItemRenderer"].as_object_mut().unwrap().remove("title");
        list[1]["musicTwoRowItemRenderer"].as_object_mut().unwrap().remove("subtitle");
        list[2]["musicTwoRowItemRenderer"].as_object_mut().unwrap().remove("navigationEndpoint");
        list[3]["musicTwoRowItemRenderer"].as_object_mut().unwrap().remove("thumbnailRenderer");
        list[4] = json!({"musicTwoRowItemRenderer": null});
        list[5] = json!("nonsense");
        grid["continuations"] = json!([{"nextContinuationData": {"continuation": 7}}]);
        let (cards, next) = discography_page(&page, &queen());
        assert_eq!((cards.len(), next), (57 - 4, None));
        assert!(cards.iter().any(|c| c.year.is_empty() && c.kind == "Album"), "a card without a subtitle is still an album");
    }

    const LONG: &str = include_str!("../../../tests/fixtures/youtube/playlist-long-2026-11-03.json");
    const LONG_NEXT: &str = include_str!("../../../tests/fixtures/youtube/playlist-long-next-2026-11-03.json");

    #[test]
    fn a_long_playlist_continues_past_its_first_hundred() {
        let (p, next) = playlist_page(&load(LONG), "PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI").unwrap();
        assert_eq!(p.tracks.len(), 100);
        let rest = load(LONG_NEXT);
        let (list, more) = continued(&rest);
        let tail = tracks_in(list);
        assert!(next.is_some_and(|t| t.len() > 20) && more.is_none());
        assert_eq!(tail.len(), 82);
        assert!(p.tracks.iter().chain(&tail).all(|t| t.id.video_id().is_some() && !t.title.is_empty()));
    }

    #[test]
    fn the_top_songs_link_to_the_whole_list() {
        let a = artist_page(&load(ARTIST_FULL), QUEEN).unwrap();
        assert_eq!(a.songs, "OLAK5uy_k1shthGfZd9raDVyW94KdQI-Mv1zbvGsQ");
        assert_eq!(a.top[0].title, "Bohemian Rhapsody");
        assert!(artist_page(&load(ARTIST), QUEEN).unwrap().songs.is_empty());
        let mut page = load(ARTIST_FULL);
        let shelf = page.pointer_mut(&format!("{SECTIONS}/0/musicShelfRenderer")).unwrap().as_object_mut().unwrap();
        shelf.remove("bottomEndpoint");
        assert_eq!(artist_page(&page, QUEEN).unwrap().songs, "OLAK5uy_k1shthGfZd9raDVyW94KdQI-Mv1zbvGsQ", "the title links too");
        *page.pointer_mut(&format!("{SECTIONS}/0/musicShelfRenderer/title")).unwrap() = Value::Null;
        assert!(artist_page(&page, QUEEN).unwrap().songs.is_empty());
    }

    const ARTIST_RADIO: &str = include_str!("../../../tests/fixtures/youtube/artist-radio-queen-2026-11-03.json");
    const ARTIST_SHUFFLE: &str = include_str!("../../../tests/fixtures/youtube/artist-shuffle-queen-2026-11-03.json");

    #[test]
    fn the_artist_header_gives_its_own_radio_and_shuffle() {
        let a = artist_page(&load(ARTIST), QUEEN).unwrap();
        assert_eq!(a.radio, Some(Browse { id: "RDEMbHaAxpOZhcVmmF6I3y0siA".into(), params: "wAEB".into() }));
        assert_eq!(a.shuffle, Some(Browse { id: "RDAObHaAxpOZhcVmmF6I3y0siA".into(), params: "wAEB8gECGAE%3D".into() }));
        let home = artist_page(&load(ARTIST_FULL), QUEEN).unwrap();
        assert_eq!(home.radio.map(|r| r.id), Some("RDEMbHaAxpOZhcVmmF6I3y0siA".into()), "a home connection links with watchEndpoint");
        assert!(home.shuffle.is_some());
        let radio = mix_tracks(&load(ARTIST_RADIO));
        let shuffle = mix_tracks(&load(ARTIST_SHUFFLE));
        assert_eq!((radio.len(), shuffle.len()), (3, 2), "the reduced catalog is short; a home connection gives about 50");
        assert!(radio.iter().chain(&shuffle).all(|t| t.id.video_id().is_some() && !t.title.is_empty() && t.duration > 0.0));
        let header = json!({"title": {"runs": [{"text": "Queen"}]}, "startRadioButton": {"buttonRenderer": {}}});
        let bare = artist_page(&json!({"header": {"musicImmersiveHeaderRenderer": header}}), QUEEN).unwrap();
        assert!(bare.radio.is_none() && bare.shuffle.is_none());
        assert!(mix_tracks(&json!({})).is_empty());
    }

    #[test]
    fn a_small_artist_without_a_more_button_has_no_link() {
        let mut page = load(ARTIST_FULL);
        let sections = page.pointer_mut(SECTIONS).unwrap();
        let header = sections[1].pointer_mut("/musicCarouselShelfRenderer/header/musicCarouselShelfBasicHeaderRenderer").unwrap();
        header.as_object_mut().unwrap().remove("moreContentButton");
        header["title"]["runs"][0].as_object_mut().unwrap().remove("navigationEndpoint");
        let a = artist_page(&page, QUEEN).unwrap();
        assert!(a.more_albums.is_none() && a.more_singles.is_some());
        assert_eq!(a.albums.len(), 10);
    }

    fn empty_home() -> Value {
        json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": []}}}}]}}})
    }

    async fn home_asked(answers: Vec<Value>) -> (Vec<Shelf>, usize) {
        let asked = AtomicUsize::new(0);
        let music = crate::fake_music::serve(
            move |body| {
                assert_eq!(body["browseId"], "FEmusic_home");
                let n = asked.fetch_add(1, Ordering::Relaxed);
                answers[n.min(answers.len() - 1)].clone()
            },
            None,
        )
        .await;
        let shelves = music.ytm.home().await.unwrap();
        let requests = music.seen.lock().unwrap().len();
        (shelves, requests)
    }

    #[tokio::test]
    async fn an_empty_home_is_asked_for_once_more() {
        let (shelves, requests) = home_asked(vec![empty_home(), load(HOME_FULL)]).await;
        assert!(!shelves.is_empty(), "the second answer is used");
        assert_eq!(requests, 2);
    }

    #[tokio::test]
    async fn a_home_that_stays_empty_is_asked_for_only_twice() {
        let (shelves, requests) = home_asked(vec![empty_home()]).await;
        assert!(shelves.is_empty());
        assert_eq!(requests, 2);
    }

    #[tokio::test]
    async fn a_full_home_is_asked_for_once() {
        let (shelves, requests) = home_asked(vec![load(HOME_FULL)]).await;
        assert!(!shelves.is_empty());
        assert_eq!(requests, 1);
    }

    #[test]
    fn home_drops_shelves_with_nothing_playable() {
        let page = load(HOME);
        let mut cards = Vec::new();
        collect(&page, "musicTwoRowItemRenderer", &mut cards);
        assert_eq!(cards.len(), 15, "the capture has one podcast shelf");
        assert!(home_shelves(&page).unwrap().is_empty());
    }
}
