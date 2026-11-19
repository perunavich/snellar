use std::time::Duration;

use anyhow::Result;
use reqwest::Url;
use serde_json::Value;
use snellar_core::error::Code;
use snellar_core::journal;
use snellar_core::model::ImportTrack;
use snellar_youtube::net::{checked, net};

use super::Listing;

const PAGE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_PAGE: usize = 32 * 1024 * 1024;
const MAX_REDIRECTS: usize = 10;
const SPOTIFY_PAGE: usize = 100;
const APPLE_PAGE: usize = 100;
const APPLE_COVER: &str = "1000";
const DEEZER_PAGES: usize = 40;
const SOUNDCLOUD_CHUNK: usize = 50;
const SPOTIFY_PATHFINDER: &str = "https://api-partner.spotify.com/pathfinder/v1/query";
const SPOTIFY_PLAYLIST_HASH: &str = "8964e8eafb21aa992a7d951d256d83285c04be2105d209262901de70cb97584a";
const SPOTIFY_PAGES: usize = 100;
const SPOTIFY_PAUSE: Duration = Duration::from_secs(1);
const COVER_HOSTS: [&str; 5] = ["scdn.co", "spotifycdn.com", "dzcdn.net", "mzstatic.com", "sndcdn.com"];
const YANDEX_COVERS: &str = "avatars.yandex.net";
const BLOCKED: [u16; 5] = [401, 403, 404, 410, 451];
const SHORT_HOSTS: [&str; 5] = ["spotify.link", "deezer.page.link", "link.deezer.com", "on.soundcloud.com", "dzr.page.link"];
const WALLS: [&str; 6] = ["showcaptcha", "checkcaptcha", "SmartCaptcha", "accounts.spotify.com/login", "passport.yandex", "/signin"];

#[derive(Debug, PartialEq)]
pub enum Link {
    Spotify { kind: &'static str, id: String },
    Deezer { kind: &'static str, id: String },
    Apple(String),
    Yandex(String),
    SoundCloud(String),
}

fn known(host: &str) -> bool {
    ["spotify.com", "deezer.com", "apple.com", "soundcloud.com", "yandex.net"]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
        || yandex_host(host)
        || SHORT_HOSTS.contains(&host)
}

fn yandex_host(host: &str) -> bool {
    ["music.yandex.ru", "music.yandex.com", "music.yandex.by", "music.yandex.kz", "music.yandex.uz"].contains(&host)
}

pub fn parse(text: &str) -> Option<Link> {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("spotify:") {
        let (kind, id) = rest.split_once(':')?;
        return spotify(kind, id);
    }
    let url = Url::parse(text).ok().filter(|u| u.scheme() == "https" || u.scheme() == "http")?;
    let host = url.host_str()?.trim_start_matches("www.").to_string();
    let parts: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    match host.as_str() {
        "open.spotify.com" | "play.spotify.com" => {
            let i = parts.iter().position(|p| ["playlist", "album", "track"].contains(p))?;
            spotify(parts[i], parts.get(i + 1)?)
        }
        "deezer.com" => {
            let i = parts.iter().position(|p| ["playlist", "album"].contains(p))?;
            let id = parts.get(i + 1).filter(|id| id.chars().all(|c| c.is_ascii_digit()))?;
            Some(Link::Deezer { kind: if parts[i] == "album" { "album" } else { "playlist" }, id: id.to_string() })
        }
        "music.apple.com" | "itunes.apple.com" => {
            parts.iter().any(|p| ["playlist", "album"].contains(p)).then(|| Link::Apple(format!("https://music.apple.com{}", url.path())))
        }
        h if yandex_host(h) => yandex(&parts).map(|path| Link::Yandex(format!("https://api.music.yandex.net/{path}"))),
        "soundcloud.com" | "m.soundcloud.com" => match parts.as_slice() {
            [user, "sets", name, ..] => Some(Link::SoundCloud(format!("https://soundcloud.com/{user}/sets/{name}"))),
            _ => None,
        },
        _ => None,
    }
}

fn spotify(kind: &str, id: &str) -> Option<Link> {
    let kind = ["playlist", "album", "track"].into_iter().find(|k| *k == kind)?;
    (id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| Link::Spotify { kind, id: id.into() })
}

fn yandex(parts: &[&str]) -> Option<String> {
    match parts {
        ["users", owner, "playlists", kind, ..] => Some(format!("users/{owner}/playlists/{kind}")),
        ["playlists", id, ..] => Some(format!("playlist/{id}")),
        ["album", id] | ["album", id, ..] if !parts.contains(&"track") => Some(format!("albums/{id}/with-tracks")),
        _ => None,
    }
}

pub async fn read(http: &reqwest::Client, text: &str) -> Result<Listing> {
    let link = match parse(text) {
        Some(link) => link,
        None => resolve_short(http, text).await?,
    };
    match link {
        Link::Spotify { kind, id } => {
            let (_, page) = get(http, &format!("https://open.spotify.com/embed/{kind}/{id}")).await?;
            spotify_whole(http, SPOTIFY_PATHFINDER, &page, kind, &id, SPOTIFY_PAUSE).await
        }
        Link::Deezer { kind, id } => read_deezer(http, kind, &id).await,
        Link::Apple(url) => apple_page(&get(http, &url).await?.1),
        Link::Yandex(url) => yandex_answer(&get(http, &url).await?.1),
        Link::SoundCloud(url) => read_soundcloud(http, &url).await,
    }
}

async fn resolve_short(http: &reqwest::Client, text: &str) -> Result<Link> {
    let unsupported = || Code::ImportUnsupported.err(format!("not a playlist or album link: {text}"));
    let url = Url::parse(text.trim()).map_err(|_| unsupported())?;
    if !url.host_str().is_some_and(|h| SHORT_HOSTS.contains(&h)) {
        return Err(unsupported());
    }
    let (end, body) = get(http, url.as_str()).await?;
    if let Some(link) = parse(end.as_str()) {
        return Ok(link);
    }
    body.split(['"', '\'', ' ', '<', '>'])
        .filter(|s| s.starts_with("https://"))
        .find_map(|s| parse(&s.replace("&amp;", "&")))
        .ok_or_else(unsupported)
}

pub fn only_known_redirects(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder.redirect(reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() < MAX_REDIRECTS && attempt.url().host_str().is_some_and(known) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    }))
}

async fn limited_text(response: reqwest::Response, limit: usize) -> Result<String> {
    Ok(String::from_utf8_lossy(&limited_bytes(response, limit).await?).into_owned())
}

async fn limited_bytes(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(net)? {
        body.extend_from_slice(&chunk);
        if body.len() > limit {
            return Err(Code::ImportUnreadable.err(format!("the page is larger than {limit} bytes")));
        }
    }
    Ok(body)
}

pub(crate) async fn cover_bytes(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    let response = checked(http.get(url).timeout(PAGE_TIMEOUT).send().await.map_err(net)?)?;
    limited_bytes(response, crate::covers::LIMIT as usize).await
}

fn refusal(status: reqwest::StatusCode, url: &str) -> Option<anyhow::Error> {
    if status.is_redirection() {
        return Some(Code::ImportUnsupported.err(format!("{url} redirects outside the known services")));
    }
    BLOCKED.contains(&status.as_u16()).then(|| Code::ImportBlocked.err(format!("{url} answered {status}")))
}

async fn answer_body(response: reqwest::Response, url: &str) -> Result<String> {
    let end = response.url().clone();
    let body = limited_text(checked(response)?, MAX_PAGE).await?;
    if walled(end.as_str()) || walled(&body) {
        return Err(Code::ImportBlocked.err(format!("{url} asks to sign in or solve a captcha")));
    }
    Ok(body)
}

fn walled(text: &str) -> bool {
    WALLS.iter().any(|w| text.contains(w))
}

async fn get(http: &reqwest::Client, url: &str) -> Result<(Url, String)> {
    let response = http.get(url).timeout(PAGE_TIMEOUT).send().await.map_err(net)?;
    let (status, end) = (response.status(), response.url().clone());
    if let Some(e) = refusal(status, url) {
        return Err(e);
    }
    if !end.host_str().is_some_and(known) {
        return Err(Code::ImportUnsupported.err(format!("{url} led to {end}")));
    }
    Ok((end, answer_body(response, url).await?))
}

fn script<'a>(html: &'a str, marker: &str) -> Option<&'a str> {
    let start = html.find(marker)?;
    let open = start + html[start..].find('>')? + 1;
    let len = html[open..].find("</script>")?;
    Some(&html[open..open + len])
}

fn unreadable(what: &str) -> anyhow::Error {
    Code::ImportUnreadable.err(format!("no track list in the {what}"))
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().trim().to_string()
}

fn cover_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok().filter(|u| u.scheme() == "https")?;
    let host = parsed.host_str()?;
    (host == YANDEX_COVERS || COVER_HOSTS.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))).then(|| url.to_string())
}

fn seconds(ms: &Value) -> f64 {
    ms.as_f64().map_or(0.0, |ms| ms / 1000.0)
}

fn spotify_data(html: &str) -> Result<Value> {
    script(html, "<script id=\"__NEXT_DATA__\"").and_then(|s| serde_json::from_str(s).ok()).ok_or_else(|| unreadable("Spotify page"))
}

fn spotify_listing(data: &Value) -> Result<Listing> {
    let page = &data["props"]["pageProps"];
    if let Some(status) = page["status"].as_u64().filter(|s| *s >= 400) {
        return Err(Code::ImportBlocked.err(format!("Spotify says {status}: private, removed or a wrong link")));
    }
    let entity = &page["state"]["data"]["entity"];
    let list = entity["trackList"].as_array().ok_or_else(|| unreadable("Spotify page"))?;
    let name = text(&entity["name"]);
    let album = if entity["type"] == "album" { name.clone() } else { String::new() };
    let tracks = list
        .iter()
        .map(|t| ImportTrack {
            title: text(&t["title"]),
            artists: t["subtitle"]
                .as_str()
                .unwrap_or_default()
                .split(',')
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect(),
            album: album.clone(),
            duration: seconds(&t["duration"]),
        })
        .filter(|t| !t.title.is_empty())
        .collect::<Vec<_>>();
    let limited = entity["type"] == "playlist" && list.len() >= SPOTIFY_PAGE;
    let hidden = list.iter().enumerate().filter(|(_, t)| text(&t["title"]).is_empty()).map(|(i, _)| i + 1).collect();
    Ok(Listing { name, tracks, limited, source_total: None, cover: spotify_cover(entity), hidden })
}

async fn spotify_whole(http: &reqwest::Client, endpoint: &str, html: &str, kind: &str, id: &str, pause: Duration) -> Result<Listing> {
    let data = spotify_data(html)?;
    let embed = spotify_listing(&data)?;
    let token = data["props"]["pageProps"]["state"]["settings"]["session"]["accessToken"].as_str().filter(|t| !t.is_empty());
    Ok(match token {
        Some(token) if kind == "playlist" && embed.limited => spotify_pages(http, endpoint, token, id, embed, pause).await,
        _ => embed,
    })
}

async fn spotify_pages(http: &reqwest::Client, endpoint: &str, token: &str, id: &str, embed: Listing, pause: Duration) -> Listing {
    let uri = format!("spotify:playlist:{id}");
    let (mut tracks, mut hidden, mut read, mut total) = (Vec::new(), Vec::new(), 0, None);
    for page in 0..SPOTIFY_PAGES {
        if page > 0 {
            tokio::time::sleep(pause).await;
        }
        let answer = pathfinder_page(http, endpoint, token, &uri, read).await.and_then(|a| pathfinder_items(&a));
        let (all, seen, songs, unnamed) = match answer {
            Ok(page) => page,
            Err(e) => {
                journal::warn("import", format!("Spotify stopped after {read} songs: {e:#}"));
                break;
            }
        };
        let all = *total.get_or_insert(all);
        hidden.extend(unnamed.into_iter().map(|at| read + at + 1));
        read += seen;
        tracks.extend(songs);
        if read >= all {
            break;
        }
    }
    match total {
        Some(all) => Listing { tracks, hidden, limited: read < all, source_total: Some(all), ..embed },
        None => embed,
    }
}

async fn pathfinder_page(http: &reqwest::Client, endpoint: &str, token: &str, uri: &str, offset: usize) -> Result<Value> {
    let request = serde_json::json!({
        "variables": { "uri": uri, "offset": offset, "limit": SPOTIFY_PAGE, "enableWatchFeedEntrypoint": true },
        "operationName": "fetchPlaylist",
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": SPOTIFY_PLAYLIST_HASH } },
    });
    let response = http
        .post(endpoint)
        .bearer_auth(token)
        .header(reqwest::header::REFERER, "https://open.spotify.com/")
        .json(&request)
        .timeout(PAGE_TIMEOUT)
        .send()
        .await
        .map_err(net)?;
    if let Some(e) = refusal(response.status(), endpoint) {
        return Err(e);
    }
    serde_json::from_str(&answer_body(response, endpoint).await?).map_err(|_| unreadable("Spotify answer"))
}

fn pathfinder_items(answer: &Value) -> Result<(usize, usize, Vec<ImportTrack>, Vec<usize>)> {
    if answer["errors"].as_array().is_some_and(|e| !e.is_empty()) {
        return Err(unreadable("Spotify answer"));
    }
    let content = &answer["data"]["playlistV2"]["content"];
    let total = content["totalCount"].as_u64().ok_or_else(|| unreadable("Spotify answer"))? as usize;
    let items = content["items"].as_array().ok_or_else(|| unreadable("Spotify answer"))?;
    let data: Vec<&Value> = items.iter().map(|i| &i["itemV2"]["data"]).collect();
    if !data.iter().any(|d| d["__typename"].is_string()) {
        return Err(unreadable("Spotify answer"));
    }
    let tracks: Vec<(usize, ImportTrack)> =
        data.into_iter().enumerate().filter(|(_, d)| d["__typename"] == "Track").map(|(at, d)| (at, pathfinder_track(d))).collect();
    let unnamed = tracks.iter().filter(|(_, t)| t.title.is_empty()).map(|(at, _)| *at).collect();
    let songs = tracks.into_iter().map(|(_, t)| t).filter(|t| !t.title.is_empty()).collect();
    Ok((total, items.len(), songs, unnamed))
}

fn pathfinder_track(t: &Value) -> ImportTrack {
    ImportTrack {
        title: text(&t["name"]),
        artists: t["artists"]["items"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|a| text(&a["profile"]["name"]))
            .filter(|a| !a.is_empty())
            .collect(),
        album: text(&t["albumOfTrack"]["name"]),
        duration: seconds(&t["trackDuration"]["totalMilliseconds"]),
    }
}

fn spotify_cover(entity: &Value) -> Option<String> {
    let sized = entity["visualIdentity"]["image"].as_array().and_then(|all| all.iter().max_by_key(|i| i["maxWidth"].as_u64()));
    sized.map_or(&entity["coverArt"]["sources"][0], |i| i)["url"].as_str().and_then(cover_url)
}

fn deezer_error(answer: &Value) -> Option<anyhow::Error> {
    let error = answer.get("error")?;
    let detail = format!("Deezer: {error}");
    Some(if error["code"] == 800 || error["type"] == "DataException" {
        Code::ImportBlocked.err(detail)
    } else {
        Code::ImportUnreadable.err(detail)
    })
}

fn deezer_track(t: &Value, album: &str) -> ImportTrack {
    let own = text(&t["album"]["title"]);
    ImportTrack {
        title: text(&t["title"]),
        artists: [text(&t["artist"]["name"])].into_iter().filter(|a| !a.is_empty()).collect(),
        album: if own.is_empty() { album.into() } else { own },
        duration: t["duration"].as_f64().unwrap_or(0.0),
    }
}

pub(crate) fn deezer_answer(answer: &Value) -> Result<(Listing, usize)> {
    if let Some(e) = deezer_error(answer) {
        return Err(e);
    }
    let name = text(&answer["title"]);
    let album = if answer["type"] == "album" { name.clone() } else { String::new() };
    let tracks = deezer_tracks(&answer["tracks"], &album);
    let total = answer["nb_tracks"].as_u64().map_or(tracks.len(), |n| n as usize);
    if answer["tracks"]["data"].as_array().is_none() {
        return Err(unreadable("Deezer answer"));
    }
    let cover = answer["picture_xl"].as_str().or(answer["cover_xl"].as_str()).and_then(cover_url);
    Ok((Listing { name, tracks, limited: false, source_total: Some(total), cover, ..Listing::default() }, total))
}

pub(crate) fn deezer_tracks(page: &Value, album: &str) -> Vec<ImportTrack> {
    page["data"].as_array().map_or(&[][..], Vec::as_slice).iter().map(|t| deezer_track(t, album)).filter(|t| !t.title.is_empty()).collect()
}

async fn read_deezer(http: &reqwest::Client, kind: &str, id: &str) -> Result<Listing> {
    let api = format!("https://api.deezer.com/{kind}/{id}");
    let answer: Value = serde_json::from_str(&get(http, &api).await?.1).map_err(|_| unreadable("Deezer answer"))?;
    let (mut listing, total) = deezer_answer(&answer)?;
    let album = if kind == "album" { listing.name.clone() } else { String::new() };
    for _ in 0..DEEZER_PAGES {
        if listing.tracks.len() >= total {
            break;
        }
        let page: Value = serde_json::from_str(&get(http, &format!("{api}/tracks?index={}&limit=100", listing.tracks.len())).await?.1)
            .map_err(|_| unreadable("Deezer answer"))?;
        let more = deezer_tracks(&page, &album);
        if more.is_empty() {
            break;
        }
        listing.tracks.extend(more);
    }
    listing.limited = listing.tracks.len() < total;
    Ok(listing)
}

pub(crate) fn apple_page(html: &str) -> Result<Listing> {
    let data: Value = script(html, "id=\"serialized-server-data\"")
        .and_then(|s| serde_json::from_str(s).ok())
        .ok_or_else(|| unreadable("Apple Music page"))?;
    let mut sections = Vec::new();
    collect(&data, &mut |v| {
        if v.get("itemKind").is_some() {
            sections.push(v.clone());
        }
    });
    let items = |kind: &str| -> Vec<Value> {
        sections.iter().filter(|s| s["itemKind"] == kind).flat_map(|s| s["items"].as_array().cloned().unwrap_or_default()).collect()
    };
    let header = items("containerDetailHeaderLockup").first().cloned().unwrap_or_default();
    let name = text(&header["title"]);
    let source_total = header["trackCount"].as_u64().map(|n| n as usize);
    let album = if html.contains("AlbumDetailPageIntent") || items("trackLockup").iter().all(|t| t["tertiaryLinks"].is_null()) {
        name.clone()
    } else {
        String::new()
    };
    let tracks: Vec<ImportTrack> = items("trackLockup")
        .iter()
        .map(|t| {
            let mut artists: Vec<String> =
                t["subtitleLinks"].as_array().map_or(&[][..], Vec::as_slice).iter().map(|l| text(&l["title"])).collect();
            if artists.is_empty() {
                artists.push(text(&t["artistName"]));
            }
            artists.retain(|a| !a.is_empty());
            let own = t["tertiaryLinks"][0]["title"].as_str().map(str::trim).unwrap_or_default();
            ImportTrack {
                title: text(&t["title"]),
                artists,
                album: if own.is_empty() { album.clone() } else { own.into() },
                duration: seconds(&t["duration"]),
            }
        })
        .filter(|t| !t.title.is_empty())
        .collect();
    if tracks.is_empty() {
        return Err(unreadable("Apple Music page"));
    }
    let limited = source_total.map_or(tracks.len() >= APPLE_PAGE, |total| tracks.len() < total);
    let cover = header["artwork"]["dictionary"]["url"]
        .as_str()
        .map(|u| u.replace("{w}", APPLE_COVER).replace("{h}", APPLE_COVER).replace("{c}", "bb").replace("{f}", "jpg"))
        .and_then(|u| cover_url(&u));
    Ok(Listing { name, tracks, limited, source_total, cover, ..Listing::default() })
}

fn collect(v: &Value, found: &mut impl FnMut(&Value)) {
    match v {
        Value::Object(map) => {
            found(v);
            map.values().for_each(|x| collect(x, found));
        }
        Value::Array(list) => list.iter().for_each(|x| collect(x, found)),
        _ => {}
    }
}

fn yandex_track(t: &Value) -> ImportTrack {
    let (title, version) = (text(&t["title"]), text(&t["version"]));
    ImportTrack {
        title: if version.is_empty() { title } else { format!("{title} ({version})") },
        artists: t["artists"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|a| text(&a["name"]))
            .filter(|a| !a.is_empty())
            .collect(),
        album: text(&t["albums"][0]["title"]),
        duration: seconds(&t["durationMs"]),
    }
}

pub(crate) fn yandex_answer(body: &str) -> Result<Listing> {
    let answer: Value = serde_json::from_str(body).map_err(|_| unreadable("Yandex Music answer"))?;
    if let Some(error) = answer.get("error") {
        return Err(Code::ImportBlocked.err(format!("Yandex Music: {error}")));
    }
    let result = &answer["result"];
    let raw: Vec<&Value> = match (result["tracks"].as_array(), result["volumes"].as_array()) {
        (Some(items), _) => items.iter().map(|i| if i["track"].is_object() { &i["track"] } else { i }).collect(),
        (None, Some(volumes)) => volumes.iter().filter_map(Value::as_array).flatten().collect(),
        _ => return Err(unreadable("Yandex Music answer")),
    };
    let tracks: Vec<ImportTrack> = raw.into_iter().map(yandex_track).filter(|t| !t.title.is_empty()).collect();
    let total = result["trackCount"].as_u64().map_or(tracks.len(), |n| n as usize);
    let cover = result["cover"]["uri"]
        .as_str()
        .or(result["ogImage"].as_str())
        .or(result["coverUri"].as_str())
        .and_then(|uri| cover_url(&format!("https://{}", uri.replace("%%", "1000x1000"))));
    Ok(Listing {
        name: text(&result["title"]),
        limited: tracks.len() < total,
        tracks,
        source_total: Some(total),
        cover,
        ..Listing::default()
    })
}

fn soundcloud_track(t: &Value) -> ImportTrack {
    let meta = &t["publisher_metadata"];
    let mut artist = text(&meta["artist"]);
    let mut title = text(&t["title"]);
    if let Some((left, right)) = title.split_once(" - ").map(|(l, r)| (l.trim().to_string(), r.trim().to_string())) {
        if artist.is_empty() {
            artist = left;
            title = right;
        } else if left.eq_ignore_ascii_case(&artist) {
            title = right;
        }
    }
    if artist.is_empty() {
        artist = text(&t["user"]["username"]);
    }
    ImportTrack {
        title,
        artists: [artist].into_iter().filter(|a| !a.is_empty()).collect(),
        album: text(&meta["album_title"]),
        duration: seconds(&t["duration"]),
    }
}

pub(crate) struct SoundCloudSet {
    pub listing: Listing,
    pub slots: Vec<(u64, Option<ImportTrack>)>,
    pub client: Option<String>,
}

pub(crate) fn soundcloud_page(html: &str) -> Result<SoundCloudSet> {
    let data: Value = html
        .split_once("window.__sc_hydration")
        .and_then(|(_, rest)| rest.split_once("</script>"))
        .map(|(s, _)| s.trim().trim_start_matches('=').trim().trim_end_matches(';'))
        .and_then(|s| serde_json::from_str(s).ok())
        .ok_or_else(|| unreadable("SoundCloud page"))?;
    let part = |name: &str| data.as_array().and_then(|all| all.iter().find(|h| h["hydratable"] == name)).map(|h| &h["data"]);
    let set = part("playlist").ok_or_else(|| unreadable("SoundCloud page"))?;
    let slots: Vec<(u64, Option<ImportTrack>)> = set["tracks"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|t| (t["id"].as_u64().unwrap_or(0), t["title"].is_string().then(|| soundcloud_track(t))))
        .collect();
    let client = part("apiClient").map(|c| text(&c["id"])).filter(|id| !id.is_empty());
    let source_total = Some(slots.len());
    let cover = set["artwork_url"].as_str().and_then(|u| cover_url(&u.replace("-large.", "-t500x500.")));
    Ok(SoundCloudSet { listing: Listing { name: text(&set["title"]), source_total, cover, ..Listing::default() }, slots, client })
}

pub(crate) fn soundcloud_tracks(answer: &Value) -> Vec<(u64, ImportTrack)> {
    answer.as_array().map_or(&[][..], Vec::as_slice).iter().filter_map(|t| Some((t["id"].as_u64()?, soundcloud_track(t)))).collect()
}

async fn read_soundcloud(http: &reqwest::Client, url: &str) -> Result<Listing> {
    let SoundCloudSet { mut listing, mut slots, client } = soundcloud_page(&get(http, url).await?.1)?;
    let missing: Vec<u64> = slots.iter().filter(|(_, t)| t.is_none()).map(|(id, _)| *id).filter(|id| *id > 0).collect();
    if let Some(client) = client {
        for chunk in missing.chunks(SOUNDCLOUD_CHUNK) {
            let ids = chunk.iter().map(u64::to_string).collect::<Vec<_>>().join("%2C");
            let Ok((_, body)) = get(http, &format!("https://api-v2.soundcloud.com/tracks?ids={ids}&client_id={client}")).await else {
                break;
            };
            let found = soundcloud_tracks(&serde_json::from_str(&body).unwrap_or(Value::Null));
            for (id, track) in found {
                if let Some(slot) = slots.iter_mut().find(|(i, t)| *i == id && t.is_none()) {
                    slot.1 = Some(track);
                }
            }
        }
    }
    listing.limited = slots.iter().any(|(_, t)| t.is_none());
    listing.tracks = slots.into_iter().filter_map(|(_, t)| t).filter(|t| !t.title.is_empty()).collect();
    Ok(listing)
}
