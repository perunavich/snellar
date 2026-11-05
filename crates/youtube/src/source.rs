use std::sync::Arc;

use anyhow::Result;

use snellar_core::error::Code;
use snellar_core::ids::TrackId;
use snellar_core::model::{Album, Artist, Discography, Kind, Lyrics, Mix, SearchResult, Suggestions, Track};
use snellar_core::source::{BoxFuture, Capabilities, Filter, Source, Stream};

use crate::lyrics::LyricsSource;
use crate::proxy::Proxy;
use crate::recs::Recs;
use crate::stream::Resolver;
use crate::ytm::{self, Ytm};

const RADIO_SIZE: usize = 25;

pub struct YouTube {
    ytm: Arc<Ytm>,
    recs: Arc<Recs>,
    resolver: Arc<Resolver>,
    proxy: Arc<Proxy>,
    lyrics: LyricsSource,
}

impl YouTube {
    pub fn new(ytm: Arc<Ytm>, recs: Arc<Recs>, resolver: Arc<Resolver>, proxy: Arc<Proxy>, lyrics: LyricsSource) -> Arc<YouTube> {
        Arc::new(YouTube { ytm, recs, resolver, proxy, lyrics })
    }
}

impl Source for YouTube {
    fn name(&self) -> &'static str {
        "YouTube"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { radio: true }
    }

    fn owns(&self, id: &TrackId) -> bool {
        id.video_id().is_some()
    }

    fn stream<'a>(&'a self, id: &'a TrackId, fresh: bool) -> BoxFuture<'a, Result<Stream>> {
        Box::pin(async move {
            let video = ytm::video(id)?;
            if fresh {
                self.resolver.invalidate(video);
            }
            self.resolver.resolve_with_ua(video).await?;
            Ok(Stream { url: self.proxy.url(video), format: self.resolver.format(video) })
        })
    }

    fn fallback(&self, id: &TrackId) -> Option<String> {
        id.video_id().map(|video| format!("ytdl://{video}"))
    }

    fn prefetch(&self, id: &TrackId) {
        if let Some(video) = id.video_id() {
            self.proxy.warm(video);
        }
    }

    fn radio<'a>(&'a self, seed: &'a Track, recent: &'a [Track]) -> BoxFuture<'a, Result<Vec<Track>>> {
        Box::pin(async move {
            match self.recs.next_tracks(seed, recent, RADIO_SIZE).await {
                Ok(tracks) if !tracks.is_empty() => Ok(tracks),
                _ => self.ytm.radio(&seed.id).await,
            }
        })
    }

    fn track<'a>(&'a self, id: &'a TrackId) -> BoxFuture<'a, Result<Track>> {
        Box::pin(async move {
            if let Some(track) = self.ytm.track(id).await? {
                return Ok(track);
            }
            self.ytm
                .radio(id)
                .await?
                .into_iter()
                .find(|t| t.id == *id)
                .ok_or_else(|| Code::YtUnavailable.err(format!("no track with id `{id}`")))
        })
    }

    fn link(&self, kind: Kind, id: &str) -> Option<String> {
        ytm::share_url(kind, id)
    }

    fn search<'a>(&'a self, q: &'a str, filter: Filter) -> BoxFuture<'a, Result<SearchResult>> {
        Box::pin(self.ytm.search(q, filter))
    }

    fn search_background<'a>(&'a self, q: &'a str, filter: Filter) -> BoxFuture<'a, Result<SearchResult>> {
        Box::pin(self.ytm.search_background(q, filter))
    }

    fn suggest<'a>(&'a self, q: &'a str) -> BoxFuture<'a, Result<Suggestions>> {
        Box::pin(self.ytm.suggest(q))
    }

    fn album<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Album>> {
        Box::pin(self.ytm.album(id))
    }

    fn artist<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Artist>> {
        Box::pin(self.ytm.artist(id))
    }

    fn discography<'a>(&'a self, artist: &'a Artist) -> BoxFuture<'a, Result<Discography>> {
        Box::pin(self.ytm.discography(artist))
    }

    fn artist_mix<'a>(&'a self, artist: &'a Artist, mix: Mix) -> BoxFuture<'a, Result<Vec<Track>>> {
        Box::pin(async move {
            let list = match mix {
                Mix::Radio => &artist.radio,
                Mix::Shuffle => &artist.shuffle,
            };
            match list {
                Some(list) => self.ytm.mix(list).await,
                None => Ok(Vec::new()),
            }
        })
    }

    fn playlist<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Album>> {
        Box::pin(self.ytm.playlist(id))
    }

    fn lyrics<'a>(&'a self, track: &'a Track) -> BoxFuture<'a, Result<Option<Lyrics>>> {
        Box::pin(self.lyrics.fetch(track))
    }
}
