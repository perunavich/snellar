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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::Ref;

    fn song(id: &str, album: Option<&str>, artist: &str) -> Track {
        Track {
            id: id.into(),
            title: format!("song {id}"),
            artists: vec![Ref { id: format!("ar_{artist}"), name: artist.into() }],
            album: album.map(|a| Ref { id: format!("al_{a}"), name: format!("album {a}") }),
            thumb: format!("thumb {id}"),
            ..Default::default()
        }
    }

    fn loose(track: Track) -> Play {
        Play { track, context: None }
    }

    fn from(track: Track, kind: &str, id: &str) -> Play {
        Play { track, context: Some(Pin { kind: kind.into(), id: id.into(), item: json!({ "id": id, "title": id }) }) }
    }

    fn cards(pins: &[Pin]) -> Vec<String> {
        pins.iter().map(|p| format!("{}:{}", p.kind, p.id)).collect()
    }

    fn list(id: i64, name: &str) -> Playlist {
        Playlist { id, name: name.into(), count: 2, thumb: "cover".into(), ..Default::default() }
    }

    #[test]
    fn plays_from_one_collection_make_one_card_at_its_newest_play() {
        let plays = [
            from(song("a", Some("x"), "p"), "album", "al_x"),
            from(song("b", None, "q"), "remote", "PLq"),
            from(song("c", Some("x"), "p"), "album", "al_x"),
            from(song("d", None, "r"), "artist", "ar_r"),
            from(song("e", Some("x"), "p"), "album", "al_x"),
        ];
        assert_eq!(cards(&shelf(&plays, &[], &[], 10)), ["album:al_x", "remote:PLq", "artist:ar_r"]);
    }

    #[test]
    fn a_local_playlist_shows_its_current_name_and_a_deleted_one_falls_back_to_its_tracks() {
        let plays = [from(song("a", None, "p"), "playlist", "1"), from(song("b", None, "q"), "playlist", "2")];
        let found = shelf(&plays, &[list(1, "renamed")], &[], 10);
        assert_eq!(cards(&found), ["playlist:1".to_string(), format!("track:{}", plays[1].track.id)]);
        assert_eq!(found[0].item["name"], "renamed");
        assert_eq!(found[0].item["thumb"], "cover");
    }

    #[test]
    fn history_without_a_context_gives_albums_and_artists_it_can_show_honestly() {
        let plays = [
            loose(song("a", None, "lone")),
            loose(song("b", Some("x"), "p")),
            loose(song("c", Some("one"), "q")),
            loose(song("d", Some("x"), "p")),
            loose(song("e", Some("two"), "q")),
            loose(song("f", Some("three"), "q")),
        ];
        let found = shelf(&plays, &[], &[], 10);
        assert_eq!(cards(&found), ["album:al_x".to_string(), "artist:ar_q".into(), format!("track:{}", plays[0].track.id)]);
        assert_eq!((found[0].item["title"].as_str(), found[0].item["thumb"].as_str()), (Some("album x"), Some("thumb b")));
        assert_eq!(found[0].item["artists"][0]["name"], "p");
        assert_eq!(found[0].item["kind"], "Album");
        assert_eq!(found[1].item["name"], "q");
    }

    #[test]
    fn the_same_song_played_twice_is_not_two_plays_of_an_album() {
        let plays = [loose(song("a", Some("x"), "p")), loose(song("a", Some("x"), "p")), loose(song("a", Some("x"), "p"))];
        assert_eq!(cards(&shelf(&plays, &[], &[], 10)), [format!("track:{}", plays[0].track.id)]);
    }

    #[test]
    fn a_song_adds_no_card_when_its_album_or_artist_is_already_on_the_shelf() {
        let plays = [
            loose(song("a", Some("x"), "p")),
            loose(song("b", None, "q")),
            from(song("c", Some("x"), "p"), "album", "al_x"),
            from(song("d", None, "q"), "artist", "ar_q"),
        ];
        assert_eq!(cards(&shelf(&plays, &[], &[], 10)), ["album:al_x", "artist:ar_q"]);
    }

    #[test]
    fn collections_come_before_single_songs_even_when_the_songs_are_newer() {
        let plays = [loose(song("a", None, "p")), loose(song("b", None, "q")), from(song("c", None, "r"), "album", "al_c")];
        let found = cards(&shelf(&plays, &[], &[], 2));
        assert_eq!(found, ["album:al_c".to_string(), format!("track:{}", plays[0].track.id)]);
    }

    #[test]
    fn songs_without_a_usable_album_or_artist_id_stay_songs() {
        let mut bare = song("a", Some("x"), "p");
        bare.album = Some(Ref { id: String::new(), name: "no id".into() });
        bare.artists[0].id = String::new();
        let mut twin = song("b", Some("x"), "p");
        twin.album = bare.album.clone();
        twin.artists[0].id = String::new();
        let plays = [loose(bare), loose(twin)];
        let found = shelf(&plays, &[], &[], 10);
        assert!(found.iter().all(|p| p.kind == "track"), "{:?}", cards(&found));
    }

    #[test]
    fn an_artist_card_prefers_the_picture_of_a_followed_artist() {
        let plays = [loose(song("a", None, "q")), loose(song("b", None, "q")), loose(song("c", None, "q"))];
        let followed = [ArtistCard { id: "ar_q".into(), name: "q".into(), subtitle: "5M".into(), thumb: "portrait".into() }];
        let found = shelf(&plays, &[], &followed, 10);
        assert_eq!(found[0].item["thumb"], "portrait");
        assert_eq!(found[0].item["subtitle"], "5M");
        assert_eq!(shelf(&plays, &[], &[], 10)[0].item["thumb"], "thumb a");
    }
}
