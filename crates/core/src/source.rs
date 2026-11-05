use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use crate::error::Code;
use crate::ids::TrackId;
use crate::model::{Album, Artist, AudioFormat, Discography, Kind, Lyrics, Mix, Ref, SearchResult, Suggestions, Track};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Songs,
    Albums,
    Artists,
    Videos,
}

impl Filter {
    pub fn parse(s: &str) -> Filter {
        match s {
            "songs" => Filter::Songs,
            "albums" => Filter::Albums,
            "artists" => Filter::Artists,
            "videos" => Filter::Videos,
            _ => Filter::All,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stream {
    pub url: String,
    pub format: Option<AudioFormat>,
}

impl Stream {
    pub fn url(url: impl Into<String>) -> Stream {
        Stream { url: url.into(), format: None }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub radio: bool,
}

fn unsupported<'a, T: 'a>(what: &'static str) -> BoxFuture<'a, Result<T>> {
    Box::pin(async move { Err(Code::SourceUnsupported.err(format!("this source has no {what}"))) })
}

pub trait Source: Send + Sync {
    fn name(&self) -> &'static str;

    fn capabilities(&self) -> Capabilities;

    fn owns(&self, id: &TrackId) -> bool;

    fn stream<'a>(&'a self, id: &'a TrackId, fresh: bool) -> BoxFuture<'a, Result<Stream>>;

    fn fallback(&self, _id: &TrackId) -> Option<String> {
        None
    }

    fn prefetch(&self, _id: &TrackId) {}

    fn link(&self, _kind: Kind, _id: &str) -> Option<String> {
        None
    }

    fn radio<'a>(&'a self, _seed: &'a Track, _recent: &'a [Track]) -> BoxFuture<'a, Result<Vec<Track>>> {
        unsupported("radio")
    }

    fn track<'a>(&'a self, _id: &'a TrackId) -> BoxFuture<'a, Result<Track>> {
        unsupported("track lookup")
    }

    fn search<'a>(&'a self, _q: &'a str, _filter: Filter) -> BoxFuture<'a, Result<SearchResult>> {
        unsupported("search")
    }

    fn search_background<'a>(&'a self, q: &'a str, filter: Filter) -> BoxFuture<'a, Result<SearchResult>> {
        self.search(q, filter)
    }

    fn suggest<'a>(&'a self, _q: &'a str) -> BoxFuture<'a, Result<Suggestions>> {
        unsupported("suggestions")
    }

    fn album<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, Result<Album>> {
        unsupported("albums")
    }

    fn artist<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, Result<Artist>> {
        unsupported("artists")
    }

    fn discography<'a>(&'a self, artist: &'a Artist) -> BoxFuture<'a, Result<Discography>> {
        let artist = Discography {
            artist: Ref { id: artist.id.clone(), name: artist.name.clone() },
            albums: artist.albums.clone(),
            singles: artist.singles.clone(),
        };
        Box::pin(async move { Ok(artist) })
    }

    fn artist_mix<'a>(&'a self, _artist: &'a Artist, _mix: Mix) -> BoxFuture<'a, Result<Vec<Track>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn playlist<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, Result<Album>> {
        unsupported("playlists")
    }

    fn lyrics<'a>(&'a self, _track: &'a Track) -> BoxFuture<'a, Result<Option<Lyrics>>> {
        unsupported("lyrics")
    }
}
