use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use snellar_core::error::Code;
use snellar_core::ids::{TrackId, valid_id};
use snellar_core::journal;
use snellar_core::model::{
    Album, AlbumCard, Artist, ArtistCard, Browse, Discography, Hit, Kind, Page, Ref, SearchResult, Shelf, Suggestions, Track, Upload,
};
pub use snellar_core::source::Filter;
use snellar_core::sync::Lock;

use crate::net::{Http, checked, net};
use crate::title;

const ORIGIN: &str = "https://music.youtube.com";
const HEDGE_AFTER: Duration = Duration::from_millis(1200);
const QUEUE_LENGTHS_AT_ONCE: usize = 100;
const CLIENT_VERSION: &str = "1.20250929.03.00";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const SONGS_PARAMS: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";
const ALBUMS_PARAMS: &str = "EgWKAQIYAWoMEA4QChADEAQQCRAF";
const ARTISTS_PARAMS: &str = "EgWKAQIgAWoMEA4QChADEAQQCRAF";
const VIDEOS_PARAMS: &str = "EgWKAQIQAWoMEA4QChADEAQQCRAF";
const COMMUNITY_PLAYLISTS_PARAMS: &str = "EgeKAQQoAEABagwQDhAKEAMQBBAJEAU=";
const FEATURED_PLAYLISTS_PARAMS: &str = "EgeKAQQoADgBagwQDhAKEAMQBBAJEAU=";
const PLAYLIST_RUN: usize = 3;
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
    named: Vec<(usize, Ref)>,
}

impl Sub {
    fn credited(&self) -> Vec<Ref> {
        let part = self.named.iter().find(|(_, r)| !r.id.is_empty()).or(self.named.first()).map(|(part, _)| *part);
        self.named.iter().filter(|(p, _)| Some(*p) == part).map(|(_, r)| r.clone()).collect()
    }
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
    let mut part = 0;
    for run in runs {
        let Some(t) = text(run, "text") else { continue };
        let browse = text(run, "navigationEndpoint.browseEndpoint.browseId").unwrap_or_default();
        if t.trim() == "•" {
            part += 1;
        } else if browse.starts_with("UC") || browse.starts_with("MPLA") {
            let artist = Ref { id: browse.trim_start_matches("MPLA").into(), name: t.into() };
            sub.named.push((part, artist.clone()));
            sub.artists.push(artist);
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
        } else if !matches!(t.trim(), "" | "&" | ",") {
            sub.rest.push(t.into());
            if !t.contains(|c: char| c.is_ascii_digit()) {
                sub.named.push((part, Ref { id: String::new(), name: t.into() }));
            }
        }
    }
    sub
}

const VIDEO_TYPE: &str = "watchEndpointMusicSupportedConfigs.watchEndpointMusicConfig.musicVideoType";

fn upload(kind: Option<&str>) -> Upload {
    match kind {
        Some("MUSIC_VIDEO_TYPE_ATV") => Upload::Song,
        Some("MUSIC_VIDEO_TYPE_OMV") => Upload::Video,
        Some("MUSIC_VIDEO_TYPE_OFFICIAL_SOURCE_MUSIC") => Upload::Official,
        Some("MUSIC_VIDEO_TYPE_UGC") => Upload::User,
        Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE") => Upload::Podcast,
        _ => Upload::Unknown,
    }
}

fn row_upload(r: &Value) -> Upload {
    let paths = [
        "overlay.musicItemThumbnailOverlayRenderer.content.musicPlayButtonRenderer.playNavigationEndpoint.watchEndpoint",
        "navigationEndpoint.watchEndpoint",
        "thumbnailOverlay.musicItemThumbnailOverlayRenderer.content.musicPlayButtonRenderer.playNavigationEndpoint.watchEndpoint",
    ];
    paths.iter().map(|p| upload(text(r, &format!("{p}.{VIDEO_TYPE}")))).find(|u| !u.is_unknown()).unwrap_or_default()
}

fn explicit(r: &Value) -> bool {
    ["badges", "subtitleBadges"]
        .iter()
        .any(|k| items(r, k).iter().any(|b| text(b, "musicInlineBadgeRenderer.icon.iconType") == Some("MUSIC_EXPLICIT_BADGE")))
}

fn build(title: &str, video: Option<&str>, browse: Option<&str>, sub: Sub, r: &Value) -> Option<Item> {
    let thumb = thumb(r);
    if let Some(id) = video {
        let artists = sub.credited();
        return Some(Item::Track(Track {
            id: TrackId::youtube(id),
            title: title.into(),
            artists,
            album: sub.album,
            duration: sub.duration,
            thumb,
            explicit: explicit(r),
            video: false,
            upload: row_upload(r),
            ..Track::default()
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
    let artists = sub.credited();
    let raw = text(r, &format!("navigationEndpoint.watchEndpoint.{VIDEO_TYPE}"));
    let video = raw.is_some_and(|kind| kind != "MUSIC_VIDEO_TYPE_ATV");
    let track = Track {
        id: TrackId::youtube(text(r, "videoId")?),
        title: run_text(items(r, "title.runs")),
        artists,
        album: sub.album,
        duration: parse_duration(text(r, "lengthText.runs.0.text")?).unwrap_or_default(),
        thumb: thumb(r),
        explicit: explicit(r),
        video,
        upload: upload(raw),
        ..Track::default()
    };
    Some(if video { clean_video(track) } else { track })
}

fn clean_video(mut track: Track) -> Track {
    let uploader = track.artists.first().cloned().unwrap_or_default();
    track.uploader = Some(Box::new(uploader.clone())).filter(|u| !u.name.is_empty());
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
    candidate.filter(|id| valid_id(id))
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
        *self.region.locked() = code.trim().to_uppercase();
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
        let region = self.region.locked().clone();
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
            Filter::All | Filter::Playlists => None,
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
        if filter == Filter::Playlists {
            let (community, featured) = tokio::try_join!(
                self.search_page(q, Some(COMMUNITY_PLAYLISTS_PARAMS)),
                self.search_page(q, Some(FEATURED_PLAYLISTS_PARAMS)),
            )?;
            return search_playlists(&community, &featured);
        }
        let page = self.search_page(q, params).await?;
        search_filtered(&page, filter)
    }

    pub async fn track(&self, id: &TrackId) -> Result<Option<Track>> {
        let video_id = video(id)?;
        let watch = self.call("next", json!({"videoId": video_id})).await?;
        Ok(watch_track(&watch, video_id))
    }

    pub async fn lengths(&self, ids: &[TrackId]) -> Result<HashMap<TrackId, f64>> {
        let _busy = self.busy();
        let videos: Vec<&str> = ids.iter().filter_map(|id| video(id).ok()).collect();
        let mut found = HashMap::new();
        for chunk in videos.chunks(QUEUE_LENGTHS_AT_ONCE) {
            let resp = self.call("music/get_queue", json!({"videoIds": chunk})).await?;
            found.extend(queue_lengths(&resp));
        }
        Ok(found)
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

fn search_playlists(community: &Value, featured: &Value) -> Result<SearchResult> {
    let (community, featured) =
        (search_filtered(community, Filter::Playlists)?.playlists, search_filtered(featured, Filter::Playlists)?.playlists);
    let mut seen = HashSet::new();
    let mut playlists = Vec::new();
    for at in (0..community.len().max(featured.len())).step_by(PLAYLIST_RUN) {
        for list in [&community, &featured] {
            playlists.extend(list.iter().skip(at).take(PLAYLIST_RUN).filter(|p| seen.insert(p.id.clone())).cloned());
        }
    }
    Ok(SearchResult { playlists, ..SearchResult::default() })
}

fn top_result(mixed: &Value) -> Option<Hit> {
    let card = sections(mixed, "tabbedSearchResultsRenderer").iter().find_map(|s| s.get("musicCardShelfRenderer"))?;
    let title = run_text(items(card, "title.runs"));
    let to = at(card, "title.runs.0.navigationEndpoint")?;
    let sub = split_sub(items(card, "subtitle.runs"));
    let watch = text(to, "watchEndpoint.videoId");
    let raw = text(to, &format!("watchEndpoint.{VIDEO_TYPE}"));
    let video = raw.is_some_and(|kind| kind != "MUSIC_VIDEO_TYPE_ATV");
    match build(&title, watch, text(to, "browseEndpoint.browseId"), sub, card)? {
        Item::Track(t) if video => Some(Hit::Track(clean_video(Track { video: true, upload: upload(raw), ..t }))),
        Item::Track(t) => Some(Hit::Track(Track { upload: upload(raw), ..t })),
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
        listeners: run_text(items(header, "monthlyListenerCount.runs")),
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

fn queue_lengths(resp: &Value) -> HashMap<TrackId, f64> {
    let mut panels = Vec::new();
    collect(resp, "playlistPanelVideoRenderer", &mut panels);
    panels
        .into_iter()
        .filter_map(|r| Some((TrackId::youtube(text(r, "videoId")?), parse_duration(text(r, "lengthText.runs.0.text")?)?)))
        .filter(|(_, seconds)| *seconds > 0.0)
        .collect()
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

pub(crate) fn lyrics_text(page: &Value) -> Option<String> {
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
        if !shelf.is_empty() {
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
