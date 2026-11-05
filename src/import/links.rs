use std::time::Duration;

use anyhow::Result;
use reqwest::Url;
use serde_json::Value;
use snellar_core::error::Code;
use snellar_core::journal;
use snellar_core::model::ImportTrack;
use snellar_youtube::net::net;

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
    let response = http.get(url).timeout(PAGE_TIMEOUT).send().await.map_err(net)?.error_for_status().map_err(net)?;
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
    let body = limited_text(response.error_for_status().map_err(net)?, MAX_PAGE).await?;
    if WALLS.iter().any(|w| end.as_str().contains(w) || body.contains(w)) {
        return Err(Code::ImportBlocked.err(format!("{url} asks to sign in or solve a captcha")));
    }
    Ok(body)
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

#[cfg(test)]
pub(crate) fn spotify_page(html: &str) -> Result<Listing> {
    spotify_listing(&spotify_data(html)?)
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
    let limited = entity["type"] == "playlist" && tracks.len() >= SPOTIFY_PAGE;
    Ok(Listing { name, tracks, limited, source_total: None, cover: spotify_cover(entity) })
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
    let (mut tracks, mut read, mut total) = (Vec::new(), 0, None);
    for page in 0..SPOTIFY_PAGES {
        if page > 0 {
            tokio::time::sleep(pause).await;
        }
        let answer = pathfinder_page(http, endpoint, token, &uri, read).await.and_then(|a| pathfinder_items(&a));
        let (all, seen, songs) = match answer {
            Ok(page) => page,
            Err(e) => {
                journal::warn("import", format!("Spotify stopped after {read} songs: {e:#}"));
                break;
            }
        };
        let all = *total.get_or_insert(all);
        read += seen;
        tracks.extend(songs);
        if read >= all {
            break;
        }
    }
    match total {
        Some(all) => Listing { tracks, limited: read < all, source_total: Some(all), ..embed },
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

fn pathfinder_items(answer: &Value) -> Result<(usize, usize, Vec<ImportTrack>)> {
    if answer["errors"].as_array().is_some_and(|e| !e.is_empty()) {
        return Err(unreadable("Spotify answer"));
    }
    let content = &answer["data"]["playlistV2"]["content"];
    let total = content["totalCount"].as_u64().ok_or_else(|| unreadable("Spotify answer"))? as usize;
    let items = content["items"].as_array().ok_or_else(|| unreadable("Spotify answer"))?;
    let known: Vec<&Value> = items.iter().map(|i| &i["itemV2"]["data"]).filter(|d| d["__typename"].is_string()).collect();
    if known.is_empty() {
        return Err(unreadable("Spotify answer"));
    }
    let songs = known.into_iter().filter(|d| d["__typename"] == "Track").map(pathfinder_track).filter(|t| !t.title.is_empty()).collect();
    Ok((total, items.len(), songs))
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
    Ok((Listing { name, tracks, limited: false, source_total: Some(total), cover }, total))
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
    Ok(Listing { name, tracks, limited, source_total, cover })
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
    Ok(Listing { name: text(&result["title"]), limited: tracks.len() < total, tracks, source_total: Some(total), cover })
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    async fn answering(answer: String) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                let mut request = [0; 2048];
                if socket.read(&mut request).await.unwrap_or(0) > 0 {
                    socket.write_all(answer.as_bytes()).await.ok();
                }
            }
        });
        (url, hits)
    }

    #[tokio::test]
    async fn a_redirect_to_a_host_no_service_owns_is_not_followed() {
        let (secret, hits) = answering("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".into()).await;
        let (start, _) =
            answering(format!("HTTP/1.1 302 Found\r\nLocation: {secret}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")).await;
        let http = only_known_redirects(reqwest::Client::builder().no_proxy()).build().unwrap();
        let e = get(&http, &start).await.unwrap_err();
        assert_eq!(Code::of(&e), Code::ImportUnsupported);
        assert_eq!(hits.load(Ordering::SeqCst), 0, "the other host was never asked");
    }

    #[tokio::test]
    async fn a_page_past_the_limit_is_refused_while_it_is_read() {
        let body = "x".repeat(4096);
        let (url, _) = answering(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let big = limited_text(http.get(&url).send().await.unwrap(), 1024).await.unwrap_err();
        assert_eq!(Code::of(&big), Code::ImportUnreadable);
        let fits = limited_text(http.get(&url).send().await.unwrap(), 8192).await.unwrap();
        assert_eq!(fits.len(), 4096);
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/import/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn json(name: &str) -> Value {
        serde_json::from_str(&fixture(name)).unwrap()
    }

    #[test]
    fn links_of_every_service_are_recognized() {
        let id = "37i9dQZF1DXcBWIGoYBM5M";
        let spotify = Some(Link::Spotify { kind: "playlist", id: id.into() });
        assert_eq!(parse(&format!("https://open.spotify.com/playlist/{id}?si=abc")), spotify);
        assert_eq!(parse(&format!("https://open.spotify.com/intl-de/playlist/{id}")), spotify);
        assert_eq!(parse(&format!("spotify:playlist:{id}")), spotify);
        assert_eq!(
            parse("https://open.spotify.com/album/4m2880jivSbbyEGAKfITCa"),
            Some(Link::Spotify { kind: "album", id: "4m2880jivSbbyEGAKfITCa".into() })
        );
        assert_eq!(parse("https://www.deezer.com/fr/playlist/908622995"), Some(Link::Deezer { kind: "playlist", id: "908622995".into() }));
        assert_eq!(parse("https://www.deezer.com/album/6575789"), Some(Link::Deezer { kind: "album", id: "6575789".into() }));
        assert_eq!(
            parse("https://music.apple.com/us/playlist/todays-hits/pl.f4d106fed2bd41149aaacabb233eb5eb?l=en"),
            Some(Link::Apple("https://music.apple.com/us/playlist/todays-hits/pl.f4d106fed2bd41149aaacabb233eb5eb".into()))
        );
        assert_eq!(
            parse("https://music.yandex.ru/users/yamusic-top/playlists/1076"),
            Some(Link::Yandex("https://api.music.yandex.net/users/yamusic-top/playlists/1076".into()))
        );
        assert_eq!(
            parse("https://music.yandex.com/playlists/lk.0d6f1a3c"),
            Some(Link::Yandex("https://api.music.yandex.net/playlist/lk.0d6f1a3c".into()))
        );
        assert_eq!(
            parse("https://music.yandex.ru/album/3389008"),
            Some(Link::Yandex("https://api.music.yandex.net/albums/3389008/with-tracks".into()))
        );
        assert_eq!(
            parse("https://soundcloud.com/user/sets/name?si=x"),
            Some(Link::SoundCloud("https://soundcloud.com/user/sets/name".into()))
        );
        for other in [
            "https://music.youtube.com/playlist?list=PLx",
            "https://open.spotify.com/artist/4tZwfgrHOc3mvqYlEYSvVi",
            "https://open.spotify.com/playlist/short",
            "https://soundcloud.com/user/track",
            "https://music.yandex.ru/album/1/track/2",
            "file:///etc/passwd",
            "not a link",
            "",
        ] {
            assert_eq!(parse(other), None, "{other}");
        }
    }

    #[test]
    fn a_spotify_embed_gives_titles_artists_and_lengths() {
        let list = spotify_page(&fixture("spotify-playlist-2026-11-03.html")).unwrap();
        assert_eq!(list.name, "Today’s Top Hits");
        assert_eq!(list.tracks.len(), 50);
        assert!(!list.limited);
        assert_eq!(
            list.tracks[0],
            ImportTrack { title: "Loser".into(), artists: vec!["Tame Impala".into()], album: String::new(), duration: 223.069 }
        );
        let several = list.tracks.iter().find(|t| t.title == "BbY WOW").unwrap();
        assert_eq!(several.artists, ["KAROL G", "Judeline", "rusowsky"], "artists are split on comma and no-break space");
        let album = spotify_page(&fixture("spotify-album-2026-11-03.html")).unwrap();
        assert_eq!((album.name.as_str(), album.tracks.len()), ("Random Access Memories", 13));
        assert_eq!(album.tracks[0].album, "Random Access Memories");
    }

    #[test]
    fn a_spotify_playlist_of_a_hundred_says_the_page_may_hold_more() {
        let list = spotify_page(&fixture("spotify-long-2026-11-03.html")).unwrap();
        assert_eq!(list.tracks.len(), 100);
        assert!(list.limited);
    }

    #[test]
    fn a_spotify_embed_does_not_say_how_long_the_playlist_is() {
        assert_eq!(spotify_page(&fixture("spotify-long-2026-11-03.html")).unwrap().source_total, None);
    }

    #[test]
    fn sources_that_report_the_length_of_the_list_give_the_real_numbers() {
        let (deezer, _) = deezer_answer(&json("deezer-playlist-2026-11-03.json")).unwrap();
        assert_eq!(deezer.source_total, Some(50));
        let apple = apple_page(&fixture("apple-playlist-2026-11-03.html")).unwrap();
        assert_eq!((apple.source_total, apple.limited), (Some(50), false));
        let cut = apple_page(&fixture("apple-playlist-2026-11-03.html").replace("\"trackCount\":50", "\"trackCount\":139")).unwrap();
        assert_eq!((cut.tracks.len(), cut.source_total, cut.limited), (50, Some(139), true));
        let yandex = yandex_answer(&fixture("yandex-playlist-handmade.json").replace("\"trackCount\":3", "\"trackCount\":451")).unwrap();
        assert_eq!((yandex.tracks.len(), yandex.source_total, yandex.limited), (3, Some(451), true));
        let set = soundcloud_page(&fixture("soundcloud-set-2026-11-03.html")).unwrap();
        assert_eq!(set.listing.source_total, Some(148));
    }

    #[test]
    fn the_cover_of_every_source_is_found_and_only_on_its_own_image_hosts() {
        let spotify = |entity: Value| {
            let page = serde_json::json!({ "props": { "pageProps": { "state": { "data": { "entity": entity } } } } });
            spotify_page(&format!("<script id=\"__NEXT_DATA__\" type=\"application/json\">{page}</script>")).unwrap().cover
        };
        assert_eq!(
            spotify(
                serde_json::json!({ "type": "playlist", "name": "x", "trackList": [], "coverArt": { "sources": [{ "url": "https://i.scdn.co/image/small" }] },
                "visualIdentity": { "image": [{ "url": "https://image-cdn-ak.spotifycdn.com/image/s", "maxWidth": 64 }, { "url": "https://image-cdn-ak.spotifycdn.com/image/l", "maxWidth": 640 }] } })
            ),
            Some("https://image-cdn-ak.spotifycdn.com/image/l".into()),
            "the largest size"
        );
        assert_eq!(
            spotify(
                serde_json::json!({ "type": "album", "name": "x", "trackList": [], "coverArt": { "sources": [{ "url": "https://i.scdn.co/image/a" }] } })
            ),
            Some("https://i.scdn.co/image/a".into())
        );
        for other in ["https://example.com/a.jpg", "http://i.scdn.co/a.jpg", "https://scdn.co.example.com/a.jpg", "file:///etc/passwd"] {
            assert_eq!(
                spotify(
                    serde_json::json!({ "type": "album", "name": "x", "trackList": [], "coverArt": { "sources": [{ "url": other }] } })
                ),
                None,
                "{other}"
            );
        }
        assert_eq!(spotify_page(&fixture("spotify-long-2026-11-03.html")).unwrap().cover, None, "a page without a cover has none");

        let (playlist, _) = deezer_answer(&json("deezer-playlist-2026-11-03.json")).unwrap();
        assert!(playlist.cover.unwrap().contains("/images/playlist/"));
        let (album, _) = deezer_answer(&json("deezer-album-2026-11-03.json")).unwrap();
        assert!(album.cover.unwrap().contains("/images/cover/"));

        let artwork =
            "\"artwork\":{\"dictionary\":{\"url\":\"https://is1-ssl.mzstatic.com/image/thumb/x/{w}x{h}{c}.{f}\"}},\"trackCount\":50";
        let apple = apple_page(&fixture("apple-playlist-2026-11-03.html").replace("\"trackCount\":50", artwork)).unwrap();
        assert_eq!(apple.cover.as_deref(), Some("https://is1-ssl.mzstatic.com/image/thumb/x/1000x1000bb.jpg"));

        let yandex = serde_json::json!({ "result": { "title": "x", "tracks": [], "cover": { "uri": "avatars.yandex.net/get-music-user-playlist/a/%%" } } });
        assert_eq!(
            yandex_answer(&yandex.to_string()).unwrap().cover.as_deref(),
            Some("https://avatars.yandex.net/get-music-user-playlist/a/1000x1000")
        );

        let set = r#"<script>window.__sc_hydration = [{"hydratable":"playlist","data":{"title":"x","artwork_url":"https://i1.sndcdn.com/artworks-a-large.jpg","tracks":[]}}];</script>"#;
        assert_eq!(soundcloud_page(set).ok().unwrap().listing.cover.as_deref(), Some("https://i1.sndcdn.com/artworks-a-t500x500.jpg"));
    }

    struct Pathfinder {
        url: String,
        requests: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Pathfinder {
        fn offsets(&self) -> Vec<u64> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|r| {
                    let body = r.split_once("\r\n\r\n").unwrap().1;
                    serde_json::from_str::<Value>(body).unwrap()["variables"]["offset"].as_u64().unwrap()
                })
                .collect()
        }
    }

    async fn pathfinder(answers: Vec<(u16, String)>) -> Pathfinder {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/query", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = requests.clone();
        tokio::spawn(async move {
            let mut next = 0;
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                loop {
                    let got = socket.read(&mut chunk).await.unwrap_or(0);
                    request.extend_from_slice(&chunk[..got]);
                    let text = String::from_utf8_lossy(&request);
                    let complete = text.split_once("\r\n\r\n").is_some_and(|(head, body)| {
                        let wanted = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length: ").map(str::to_string));
                        wanted.and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| body.len() >= n)
                    });
                    if got == 0 || complete {
                        break;
                    }
                }
                seen.lock().unwrap().push(String::from_utf8_lossy(&request).into_owned());
                let (status, body) = &answers[next.min(answers.len() - 1)];
                next += 1;
                let head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                socket.write_all(head.as_bytes()).await.ok();
                socket.write_all(body.as_bytes()).await.ok();
            }
        });
        Pathfinder { url, requests }
    }

    const SPOTIFY_ID: &str = "37i9dQZF1DX4o1oenSJRJd";

    fn embed_with_token() -> String {
        let page = fixture("spotify-long-2026-11-03.html");
        let start = page.find("application/json\">").unwrap() + "application/json\">".len();
        let end = page.find("</script>").unwrap();
        let mut data: Value = serde_json::from_str(&page[start..end]).unwrap();
        data["props"]["pageProps"]["state"]["settings"] =
            serde_json::json!({ "session": { "accessToken": "page-token", "isAnonymous": true } });
        format!("<script id=\"__NEXT_DATA__\" type=\"application/json\">{data}</script>")
    }

    async fn whole(server: &Pathfinder) -> Listing {
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        spotify_whole(&http, &server.url, &embed_with_token(), "playlist", SPOTIFY_ID, Duration::ZERO).await.unwrap()
    }

    fn first_page() -> String {
        fixture("spotify-pathfinder-0-2026-11-04.json")
    }

    fn last_page() -> String {
        fixture("spotify-pathfinder-100-2026-11-04.json")
    }

    #[tokio::test]
    async fn a_whole_spotify_playlist_is_read_over_two_pages() {
        let server = pathfinder(vec![(200, first_page()), (200, last_page())]).await;
        let list = whole(&server).await;
        assert_eq!((list.tracks.len(), list.limited, list.source_total), (150, false, Some(150)));
        assert_eq!(list.name, "All Out 2000s", "the name comes from the embed");
        assert_eq!(server.offsets(), [0, 100]);
        assert!(list.tracks.iter().all(|t| !t.title.is_empty() && !t.artists.is_empty() && t.duration > 0.0));
        assert_eq!(
            list.tracks[0],
            ImportTrack {
                title: "Oops!...I Did It Again".into(),
                artists: vec!["Britney Spears".into()],
                album: "Oops!... I Did It Again".into(),
                duration: 211.16
            }
        );
        let request = server.requests.lock().unwrap()[0].to_lowercase();
        assert!(request.starts_with("post "), "{request}");
        assert!(request.contains("authorization: bearer page-token"), "{request}");
        assert!(request.contains(&SPOTIFY_PLAYLIST_HASH.to_lowercase()));
        assert!(request.contains(&format!("spotify:playlist:{SPOTIFY_ID}").to_lowercase()));
    }

    #[tokio::test]
    async fn three_pages_are_read_in_order() {
        let total = |page: String, n: usize| page.replace("\"totalCount\":150", &format!("\"totalCount\":{n}"));
        let server =
            pathfinder(vec![(200, total(first_page(), 250)), (200, total(first_page(), 250)), (200, total(last_page(), 250))]).await;
        let list = whole(&server).await;
        assert_eq!((list.tracks.len(), list.limited, list.source_total), (250, false, Some(250)));
        assert_eq!(server.offsets(), [0, 100, 200]);
    }

    #[tokio::test]
    async fn a_failure_midway_keeps_the_pages_read_and_the_real_total_without_asking_again() {
        for status in [401, 403, 429, 500] {
            let server = pathfinder(vec![(200, first_page()), (status, "{}".into()), (200, last_page())]).await;
            let list = whole(&server).await;
            assert_eq!((list.tracks.len(), list.limited, list.source_total), (100, true, Some(150)), "{status}");
            assert_eq!(server.offsets(), [0, 100], "{status}");
        }
    }

    #[tokio::test]
    async fn a_refused_first_page_keeps_the_embed_without_asking_again() {
        let embed = spotify_page(&embed_with_token()).unwrap();
        for status in [401, 403, 429, 500] {
            let server = pathfinder(vec![(status, "{}".into()), (200, first_page())]).await;
            assert_eq!(whole(&server).await, embed, "{status}");
            assert_eq!(server.offsets(), [0], "{status}");
        }
    }

    #[tokio::test]
    async fn a_changed_answer_keeps_the_embed() {
        let embed = spotify_page(&embed_with_token()).unwrap();
        for answer in [
            "<html>",
            "{}",
            r#"{"data":{"playlist":{"items":[]}}}"#,
            r#"{"errors":[{"message":"PersistedQueryNotFound"}]}"#,
            r#"{"errors":[{"message":"x"}],"data":{"playlistV2":{"content":{"totalCount":1,"items":[{"itemV2":{"data":{"__typename":"Track","name":"a","artists":{"items":[]}}}}]}}}}"#,
            r#"{"data":{"playlistV2":{"content":{"items":[]}}}}"#,
            r#"{"data":{"playlistV2":{"content":{"totalCount":150,"items":[]}}}}"#,
            r#"{"data":{"playlistV2":{"content":{"totalCount":150,"items":[{"item":{"name":"moved"}}]}}}}"#,
        ] {
            let server = pathfinder(vec![(200, answer.into())]).await;
            assert_eq!(whole(&server).await, embed, "{answer}");
            assert_eq!(server.offsets(), [0], "{answer}");
        }
    }

    #[tokio::test]
    async fn a_changed_answer_midway_keeps_the_pages_read() {
        let server = pathfinder(vec![(200, first_page()), (200, "{}".into())]).await;
        let list = whole(&server).await;
        assert_eq!((list.tracks.len(), list.limited, list.source_total), (100, true, Some(150)));
    }

    #[tokio::test]
    async fn a_song_keeps_its_own_artists_and_what_is_not_a_song_is_skipped() {
        let answer = serde_json::json!({ "data": { "playlistV2": { "content": { "totalCount": 3, "items": [
            { "itemV2": { "data": { "__typename": "Track", "name": "Song", "trackDuration": { "totalMilliseconds": 1000 },
                "artists": { "items": [{ "profile": { "name": "Own" } }, { "profile": { "name": "Guest" } }] },
                "albumOfTrack": { "name": "Various", "artists": { "items": [{ "profile": { "name": "Various Artists" } }] } } } } },
            { "itemV2": { "data": { "__typename": "Episode", "name": "Talk" } } },
            { "itemV2": { "data": { "__typename": "NotFound" } } },
        ] } } } });
        let server = pathfinder(vec![(200, answer.to_string())]).await;
        let list = whole(&server).await;
        assert_eq!(list.tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Song"]);
        assert_eq!(list.tracks[0].artists, ["Own", "Guest"]);
        assert_eq!(list.tracks[0].album, "Various");
        assert_eq!((list.limited, list.source_total), (false, Some(3)), "all the list was read");
    }

    #[tokio::test]
    async fn the_page_cap_stops_a_list_that_never_ends() {
        let endless = first_page().replace("\"totalCount\":150", "\"totalCount\":999999");
        let server = pathfinder(vec![(200, endless)]).await;
        let list = whole(&server).await;
        assert_eq!(server.offsets().len(), SPOTIFY_PAGES);
        assert_eq!((list.tracks.len(), list.limited, list.source_total), (SPOTIFY_PAGES * 100, true, Some(999999)));
    }

    #[tokio::test]
    async fn nothing_is_sent_when_the_embed_already_has_it_all_or_has_no_token() {
        let server = pathfinder(vec![(200, first_page())]).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let short = fixture("spotify-playlist-2026-11-03.html");
        assert_eq!(spotify_whole(&http, &server.url, &short, "playlist", SPOTIFY_ID, Duration::ZERO).await.unwrap().tracks.len(), 50);
        let plain = fixture("spotify-long-2026-11-03.html");
        assert_eq!(spotify_whole(&http, &server.url, &plain, "playlist", SPOTIFY_ID, Duration::ZERO).await.unwrap().tracks.len(), 100);
        let album = fixture("spotify-album-2026-11-03.html");
        assert_eq!(spotify_whole(&http, &server.url, &album, "album", SPOTIFY_ID, Duration::ZERO).await.unwrap().tracks.len(), 13);
        assert!(server.offsets().is_empty());
    }

    #[tokio::test]
    async fn the_token_stays_out_of_errors_and_results() {
        let server = pathfinder(vec![(403, "{}".into())]).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let e = pathfinder_page(&http, &server.url, "page-token", "spotify:playlist:x", 0).await.unwrap_err();
        assert!(!format!("{e:#} {e:?}").contains("page-token"));
        assert_eq!(Code::of(&e), Code::ImportBlocked);
        assert!(!format!("{:?}", whole(&pathfinder(vec![(200, last_page())]).await).await).contains("page-token"));
    }

    #[tokio::test]
    #[ignore]
    async fn live_spotify_whole_playlist() {
        let link = std::env::var("SNELLAR_LIVE_SPOTIFY").unwrap_or_else(|_| format!("https://open.spotify.com/playlist/{SPOTIFY_ID}"));
        let http = only_known_redirects(
            reqwest::Client::builder().user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0"),
        )
        .build()
        .unwrap();
        let list = read(&http, &link).await.unwrap();
        println!(
            "{}: {} tracks of {:?}, limited {}, with artists {}, with length {}",
            list.name,
            list.tracks.len(),
            list.source_total,
            list.limited,
            list.tracks.iter().filter(|t| !t.artists.is_empty()).count(),
            list.tracks.iter().filter(|t| t.duration > 0.0).count()
        );
        assert!(list.tracks.iter().all(|t| !t.title.is_empty()));
    }

    #[test]
    fn a_missing_or_private_spotify_playlist_is_blocked_and_a_reshaped_page_unreadable() {
        assert_eq!(Code::of(&spotify_page(&fixture("spotify-missing-2026-11-03.html")).unwrap_err()), Code::ImportBlocked);
        assert_eq!(Code::of(&spotify_page("<html><body>new design</body></html>").unwrap_err()), Code::ImportUnreadable);
        assert_eq!(
            Code::of(&spotify_page(r#"<script id="__NEXT_DATA__" type="application/json">{"props":{}}</script>"#).unwrap_err()),
            Code::ImportUnreadable
        );
    }

    #[test]
    fn deezer_playlists_albums_pages_and_errors() {
        let (list, total) = deezer_answer(&json("deezer-playlist-2026-11-03.json")).unwrap();
        assert_eq!((list.name.as_str(), list.tracks.len(), total), ("En mode 60", 50, 50));
        assert_eq!(
            list.tracks[0],
            ImportTrack {
                title: "Hey Jude (Remastered 2015)".into(),
                artists: vec!["The Beatles".into()],
                album: "1 (Remastered 2015)".into(),
                duration: 429.0
            }
        );
        let (album, total) = deezer_answer(&json("deezer-album-2026-11-03.json")).unwrap();
        assert_eq!((album.name.as_str(), album.tracks.len(), total), ("Random Access Memories", 13, 13));
        assert_eq!(album.tracks[0].artists, ["Daft Punk"]);
        assert_eq!(album.tracks[0].album, "Random Access Memories");
        let page = deezer_tracks(&json("deezer-playlist-tracks-2026-11-03.json"), "");
        assert_eq!(page.len(), 25);
        assert_eq!(page[0], list.tracks[25], "the second page goes on where the first stopped");
        assert_eq!(Code::of(&deezer_answer(&json("deezer-missing-2026-11-03.json")).unwrap_err()), Code::ImportBlocked);
        assert_eq!(Code::of(&deezer_answer(&serde_json::json!({ "title": "x" })).unwrap_err()), Code::ImportUnreadable);
    }

    #[test]
    fn apple_music_playlists_and_albums() {
        let list = apple_page(&fixture("apple-playlist-2026-11-03.html")).unwrap();
        assert_eq!(list.name, "Today’s Hits");
        assert_eq!(list.tracks.len(), 50);
        assert_eq!(
            list.tracks[0],
            ImportTrack {
                title: "Solar Eclipse".into(),
                artists: vec!["Drake".into(), "Don Toliver".into()],
                album: "HABIBTI (FOMO)".into(),
                duration: 218.389
            }
        );
        let album = apple_page(&fixture("apple-album-2026-11-03.html")).unwrap();
        assert_eq!((album.name.as_str(), album.tracks.len()), ("Random Access Memories", 13));
        assert_eq!(
            album.tracks[0],
            ImportTrack {
                title: "Give Life Back to Music".into(),
                artists: vec!["Daft Punk".into()],
                album: "Random Access Memories".into(),
                duration: 274.403
            }
        );
        assert_eq!(Code::of(&apple_page("<html></html>").unwrap_err()), Code::ImportUnreadable);
    }

    #[test]
    fn yandex_music_playlists_albums_and_the_legal_block() {
        let list = yandex_answer(&fixture("yandex-playlist-handmade.json")).unwrap();
        assert_eq!(list.name, "Русский рок");
        assert_eq!(list.tracks.len(), 3, "an entry without a track is skipped");
        assert_eq!(list.tracks[1].title, "Владивосток 2000 (Remastered 2021)");
        assert_eq!(list.tracks[2].artists, ["Земфира", "Гость"]);
        assert_eq!(list.tracks[0].duration, 285.0);
        assert!(!list.limited);
        let album = yandex_answer(&fixture("yandex-album-handmade.json")).unwrap();
        assert_eq!(
            album.tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["Группа крови", "Закрой за мной дверь, я ухожу", "Спокойная ночь"]
        );
        assert_eq!(Code::of(&yandex_answer(&fixture("yandex-blocked-2026-11-03.json")).unwrap_err()), Code::ImportBlocked);
        assert_eq!(Code::of(&yandex_answer("<html>").unwrap_err()), Code::ImportUnreadable);
    }

    #[test]
    fn a_captcha_page_is_a_wall() {
        let page = fixture("yandex-captcha-handmade.html");
        assert!(WALLS.iter().any(|w| page.contains(w)));
    }

    #[test]
    fn a_soundcloud_set_gives_full_tracks_and_ids_for_the_rest() {
        let set = soundcloud_page(&fixture("soundcloud-set-2026-11-03.html")).unwrap();
        assert_eq!(set.listing.name, "lo-fi chill beats");
        assert_eq!(set.slots.len(), 148);
        assert_eq!(set.slots.iter().filter(|(_, t)| t.is_some()).count(), 5);
        assert_eq!(set.client.as_deref(), Some("0123456789abcdefghijABCDEFGHIJ01"));
        let first = set.slots[0].1.clone().unwrap();
        assert_eq!(first.artists, ["Nohone"]);
        assert_eq!(first.duration, 315.474);
        let more = soundcloud_tracks(&json("soundcloud-tracks-2026-11-03.json"));
        assert_eq!(more.len(), 3);
        let cafe = more.iter().find(|(_, t)| t.title == "Cafe Bahia").map(|(_, t)| t).unwrap();
        assert_eq!(cafe.artists, ["Leafar Zeral"], "the artist in front of the title goes");
        assert!(more.iter().all(|(id, _)| set.slots.iter().any(|(slot, t)| slot == id && t.is_none())));
        assert_eq!(Code::of(&soundcloud_page("<html></html>").err().unwrap()), Code::ImportUnreadable);
    }

    #[test]
    fn soundcloud_titles_with_an_artist_in_front() {
        let t = |v| soundcloud_track(&v);
        assert_eq!(t(serde_json::json!({ "title": "Artist - Song", "user": { "username": "uploader" } })).artists, ["Artist"]);
        assert_eq!(t(serde_json::json!({ "title": "Song", "user": { "username": "uploader" } })).artists, ["uploader"]);
        assert_eq!(t(serde_json::json!({ "title": "Other - Song", "publisher_metadata": { "artist": "Me" } })).title, "Other - Song");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_import_covers() {
        let http = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0")
            .build()
            .unwrap();
        let spotify = spotify_page(&get(&http, "https://open.spotify.com/embed/playlist/37i9dQZF1DX4o1oenSJRJd").await.unwrap().1).unwrap();
        let (deezer, _) =
            deezer_answer(&serde_json::from_str(&get(&http, "https://api.deezer.com/playlist/908622995").await.unwrap().1).unwrap())
                .unwrap();
        let apple = apple_page(&get(&http, "https://music.apple.com/us/album/random-access-memories/617154241").await.unwrap().1).unwrap();
        let cloud = soundcloud_page(&get(&http, "https://soundcloud.com/sc-playlists-vn/sets/lo-fi-chill-beats").await.unwrap().1)
            .ok()
            .unwrap()
            .listing;
        for (source, list) in [("spotify", spotify), ("deezer", deezer), ("apple", apple), ("soundcloud", cloud)] {
            let url = list.cover.unwrap_or_else(|| panic!("{source} names no cover"));
            let (image, kind) = crate::covers::shrink(&cover_bytes(&http, &url).await.unwrap()).unwrap();
            println!("{source}: {url} gives {} bytes of {kind}, {} tracks, of {:?}", image.len(), list.tracks.len(), list.source_total);
            assert!(!image.is_empty());
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_import_links() {
        let http = reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0")
            .build()
            .unwrap();
        for link in [
            "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "https://www.deezer.com/playlist/908622995",
            "https://music.apple.com/us/album/random-access-memories/617154241",
            "https://music.yandex.ru/users/yamusic-top/playlists/1076",
            "https://soundcloud.com/sc-playlists-vn/sets/lo-fi-chill-beats",
        ] {
            match read(&http, link).await {
                Ok(list) => {
                    println!("{link}: {} tracks, limited {}", list.tracks.len(), list.limited);
                    assert!(!list.tracks.is_empty() && list.tracks.iter().all(|t| !t.title.is_empty()), "{link}");
                }
                Err(e) => {
                    println!("{link}: {e:#}");
                    assert!([Code::ImportBlocked, Code::NetLimited].contains(&Code::of(&e)), "{link}: {e:#}");
                }
            }
        }
    }
}
