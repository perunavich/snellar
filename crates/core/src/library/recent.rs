use std::collections::{HashMap, HashSet};

use crate::model::{AlbumCard, ArtistCard, Pin, Playlist, Ref, Track};

const ALBUM_TRACKS: usize = 2;
const ARTIST_TRACKS: usize = 3;

pub struct Play {
    pub track: Track,
    pub context: Option<Pin>,
}

fn album_of(t: &Track) -> Option<&Ref> {
    t.album.as_ref().filter(|a| !a.id.is_empty() && !a.name.is_empty())
}

fn artist_of(t: &Track) -> Option<&Ref> {
    t.artists.first().filter(|a| !a.id.is_empty() && !a.name.is_empty())
}

fn card(kind: &str, id: &str, item: impl serde::Serialize) -> Pin {
    Pin { kind: kind.into(), id: id.into(), item: serde_json::to_value(item).unwrap_or_default() }
}

fn recorded(context: &Pin, playlists: &[Playlist]) -> Option<Pin> {
    if context.kind != "playlist" {
        return Some(context.clone());
    }
    let list = playlists.iter().find(|p| p.id.to_string() == context.id)?;
    Some(card("playlist", &context.id, list))
}

fn album_card(t: &Track, album: &Ref) -> Pin {
    let item = AlbumCard {
        id: album.id.clone(),
        title: album.name.clone(),
        artists: t.artists.clone(),
        kind: "Album".into(),
        thumb: t.thumb.clone(),
        ..Default::default()
    };
    card("album", &album.id, item)
}

fn artist_card(t: &Track, artist: &Ref, followed: &[ArtistCard]) -> Pin {
    match followed.iter().find(|a| a.id == artist.id) {
        Some(known) => card("artist", &artist.id, known),
        None => {
            let item = ArtistCard { id: artist.id.clone(), name: artist.name.clone(), thumb: t.thumb.clone(), ..Default::default() };
            card("artist", &artist.id, item)
        }
    }
}

fn tally<'a>(tracks: &[(usize, &'a Track)], pick: impl Fn(&'a Track) -> Option<&'a Ref>) -> HashMap<&'a str, usize> {
    let mut count = HashMap::new();
    for r in tracks.iter().filter_map(|(_, t)| pick(t)) {
        *count.entry(r.id.as_str()).or_default() += 1;
    }
    count
}

pub fn shelf(plays: &[Play], playlists: &[Playlist], followed: &[ArtistCard], limit: usize) -> Vec<Pin> {
    let mut picks: Vec<(usize, Pin)> = Vec::new();
    let mut shown: HashSet<(String, String)> = HashSet::new();
    let mut seen = HashSet::new();
    let mut loose: Vec<(usize, &Track)> = Vec::new();
    for (at, play) in plays.iter().enumerate() {
        match play.context.as_ref().and_then(|c| recorded(c, playlists)) {
            Some(pin) => {
                if shown.insert((pin.kind.clone(), pin.id.clone())) {
                    picks.push((at, pin));
                }
            }
            None => {
                if seen.insert(&play.track.id) {
                    loose.push((at, &play.track));
                }
            }
        }
    }
    let mut add = |at: usize, pin: Pin| {
        if shown.insert((pin.kind.clone(), pin.id.clone())) {
            picks.push((at, pin));
        }
    };

    let in_album = tally(&loose, album_of);
    let mut rest = Vec::new();
    for (at, t) in loose {
        match album_of(t) {
            Some(a) if in_album[a.id.as_str()] >= ALBUM_TRACKS => add(at, album_card(t, a)),
            _ => rest.push((at, t)),
        }
    }
    let in_artist = tally(&rest, artist_of);
    let mut songs = Vec::new();
    for (at, t) in rest {
        match artist_of(t) {
            Some(a) if in_artist[a.id.as_str()] >= ARTIST_TRACKS => add(at, artist_card(t, a, followed)),
            _ => songs.push(t),
        }
    }
    let covered = |kind: &str, r: Option<&Ref>| r.is_some_and(|r| shown.contains(&(kind.to_string(), r.id.clone())));
    let songs: Vec<&Track> = songs.into_iter().filter(|t| !covered("album", album_of(t)) && !covered("artist", artist_of(t))).collect();

    picks.sort_by_key(|(at, _)| *at);
    let collections = picks.into_iter().map(|(_, pin)| pin);
    collections.chain(songs.into_iter().map(|t| card("track", t.id.as_str(), t))).take(limit).collect()
}
