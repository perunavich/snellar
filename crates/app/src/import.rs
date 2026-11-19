pub mod csv;
pub mod links;
pub mod score;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use futures_util::{StreamExt, stream};
use tokio::sync::oneshot;

use snellar_core::error::Code;
use snellar_core::journal;
use snellar_core::library::Library;
use snellar_core::model::{ImportProgress, ImportResult, ImportTrack, Track};
use snellar_core::source::{Filter, Source};
use snellar_core::sync::Lock;
use snellar_core::tasks::TaskSet;
use snellar_core::ui::{Ui, UiEvent};

const SEARCHES_AT_ONCE: usize = 4;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FILE: u64 = 5 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub name: String,
    pub tracks: Vec<ImportTrack>,
    pub limited: bool,
    pub source_total: Option<usize>,
    pub cover: Option<String>,
    pub hidden: Vec<usize>,
}

pub enum Input {
    Link(String),
    Csv { text: String, name: String },
}

impl Input {
    pub async fn file(path: &str) -> Result<Input> {
        let path = Path::new(path.strip_prefix("file://").unwrap_or(path));
        let unreadable = |why: String| Code::ImportFile.err(format!("{}: {why}", path.display()));
        let size = tokio::fs::metadata(path).await.map_err(|e| unreadable(e.to_string()))?.len();
        if size > MAX_FILE {
            return Err(unreadable(format!("{size} bytes, at most {MAX_FILE}")));
        }
        let bytes = tokio::fs::read(path).await.map_err(|e| unreadable(e.to_string()))?;
        let text = String::from_utf8(bytes).map_err(|_| unreadable("not UTF-8 text".into()))?;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Input::Csv { text, name })
    }
}

pub struct Job {
    pub source: Arc<dyn Source>,
    pub library: Arc<Library>,
    pub http: reqwest::Client,
    pub ui: Ui,
    pub timeout: Duration,
    pub covers: Option<PathBuf>,
}

impl Job {
    pub fn new(source: Arc<dyn Source>, library: Arc<Library>, http: reqwest::Client, ui: Ui) -> Job {
        Job { source, library, http, ui, timeout: SEARCH_TIMEOUT, covers: None }
    }

    async fn run(self, input: Input, name: Option<String>) -> Result<ImportResult> {
        progress(&self.ui, "reading", 0, 0, 0);
        let listing = match input {
            Input::Link(url) => {
                let mut listing = links::read(&self.http, &url).await?;
                if listing.name.is_empty() {
                    listing.name = reqwest::Url::parse(url.trim()).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or(url);
                }
                listing
            }
            Input::Csv { text, name } => {
                let mut listing = csv::read(&text)?;
                if listing.name.is_empty() {
                    listing.name = name;
                }
                listing
            }
        };
        self.fill(listing, name).await
    }

    async fn source_cover(&self, url: &str) -> Option<String> {
        let covers = self.covers.as_ref()?;
        let saved = match links::cover_bytes(&self.http, url).await {
            Ok(bytes) => crate::covers::store(covers, "import", bytes).await,
            Err(e) => Err(e),
        };
        saved.map_err(|e| journal::warn("import", format!("no cover from {url}: {e:#}"))).ok()
    }

    async fn fill(self, listing: Listing, name: Option<String>) -> Result<ImportResult> {
        if listing.tracks.is_empty() {
            return Err(Code::ImportEmpty.err("the playlist has no tracks"));
        }
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or(listing.name);
        let total = listing.tracks.len();
        journal::info("import", format!("{total} tracks for {name}"));
        let counts = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
        progress(&self.ui, "matching", 0, total, 0);
        let outcomes: Vec<Result<Option<Track>>> = stream::iter(listing.tracks.clone())
            .map(|want| {
                let (source, ui, counts, timeout) = (self.source.clone(), self.ui.clone(), counts.clone(), self.timeout);
                async move {
                    let found = find(&*source, &want, timeout).await;
                    let hit = usize::from(matches!(found, Ok(Some(_))));
                    let matched = counts.1.fetch_add(hit, Ordering::Relaxed) + hit;
                    progress(&ui, "matching", counts.0.fetch_add(1, Ordering::Relaxed) + 1, total, matched);
                    found
                }
            })
            .buffered(SEARCHES_AT_ONCE)
            .collect()
            .await;
        let mut tracks = Vec::new();
        let mut seen = HashSet::new();
        let mut unmatched = Vec::new();
        let mut merged = Vec::new();
        let mut failure = None;
        let mut failed = 0;
        for (want, outcome) in listing.tracks.into_iter().zip(outcomes) {
            match outcome {
                Ok(Some(track)) if seen.contains(&track.id) => merged.push(want),
                Ok(Some(track)) => {
                    seen.insert(track.id.clone());
                    tracks.push(track);
                }
                Ok(None) => unmatched.push(want),
                Err(e) => {
                    failed += 1;
                    failure = Some(e);
                }
            }
        }
        if tracks.is_empty()
            && let Some(e) = failure.take().filter(|_| failed + unmatched.len() == total)
        {
            return Err(e);
        }
        let code = failure.as_ref().map(Code::of);
        let playlist = if tracks.is_empty() {
            None
        } else {
            let cover = match &listing.cover {
                Some(url) => self.source_cover(url).await,
                None => None,
            };
            let id = self.library.create_filled_playlist(&name, &tracks);
            if let (Some(cover), Ok(id)) = (&cover, &id) {
                self.library.set_playlist_cover(*id, cover);
            } else if let (Some(cover), Some(dir)) = (&cover, &self.covers) {
                crate::covers::drop_local(dir, cover);
            }
            Some(id?)
        };
        journal::info("import", format!("{} of {total} found, playlist {playlist:?}", tracks.len()));
        Ok(ImportResult {
            playlist,
            name,
            total,
            matched: tracks.len(),
            unmatched,
            limited: listing.limited,
            source_total: listing.source_total,
            failed,
            merged,
            hidden: listing.hidden,
            code,
        })
    }
}

fn progress(ui: &Ui, stage: &str, done: usize, total: usize, matched: usize) {
    ui(UiEvent::Import(ImportProgress { stage: stage.into(), done, total, matched }));
}

async fn find(source: &dyn Source, want: &ImportTrack, timeout: Duration) -> Result<Option<Track>> {
    let artist = want.artists.first().map_or("", String::as_str);
    let query = format!("{artist} {}", score::base_title(&want.title)).trim().to_string();
    for filter in [Filter::Songs, Filter::Videos] {
        let found = search(source, &query, filter, timeout).await?;
        if let Some(track) = score::best(want, &found) {
            return Ok(Some(track.clone()));
        }
    }
    Ok(None)
}

async fn search(source: &dyn Source, query: &str, filter: Filter, timeout: Duration) -> Result<Vec<Track>> {
    let mut last = None;
    for _ in 0..2 {
        match tokio::time::timeout(timeout, source.search_background(query, filter)).await {
            Ok(Ok(found)) => return Ok(if filter == Filter::Videos { found.videos } else { found.tracks }),
            Ok(Err(e)) if Code::of(&e).network() => last = Some(e),
            Ok(Err(e)) => return Err(e),
            Err(_) => last = Some(Code::NetTimeout.err(format!("no answer to the search for {query}"))),
        }
    }
    Err(last.unwrap_or_else(|| Code::Internal.err("search gave nothing")))
}

#[derive(Default)]
pub struct Imports {
    running: Mutex<Option<(u64, TaskSet)>>,
    next: AtomicU64,
}

impl Imports {
    pub async fn run(&self, job: Job, input: Input, name: Option<String>) -> Result<ImportResult> {
        let (tx, rx) = oneshot::channel();
        let tasks = TaskSet::default();
        tasks.spawn("import", async move {
            tx.send(job.run(input, name).await).ok();
        });
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let replaced = self.running.locked().replace((id, tasks));
        drop(replaced);
        let outcome = rx.await;
        let finished = {
            let mut running = self.running.locked();
            if running.as_ref().is_some_and(|(current, _)| *current == id) { running.take() } else { None }
        };
        drop(finished);
        outcome.unwrap_or_else(|_| Err(Code::ImportCancelled.err("the import was cancelled")))
    }

    pub fn cancel(&self) -> bool {
        let running = self.running.locked().take();
        running.is_some()
    }
}
