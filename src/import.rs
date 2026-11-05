pub mod csv;
pub mod links;
pub mod score;

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
        let mut unmatched = Vec::new();
        let mut failure = None;
        let mut failed = 0;
        for (want, outcome) in listing.tracks.into_iter().zip(outcomes) {
            match outcome {
                Ok(Some(track)) => tracks.push(track),
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
        let replaced = self.running.lock().unwrap().replace((id, tasks));
        drop(replaced);
        let outcome = rx.await;
        let finished = {
            let mut running = self.running.lock().unwrap();
            if running.as_ref().is_some_and(|(current, _)| *current == id) { running.take() } else { None }
        };
        drop(finished);
        outcome.unwrap_or_else(|_| Err(Code::ImportCancelled.err("the import was cancelled")))
    }

    pub fn cancel(&self) -> bool {
        let running = self.running.lock().unwrap().take();
        running.is_some()
    }
}

#[cfg(test)]
mod tests {
    use snellar_core::model::Ref;
    use snellar_core::source::Capabilities;

    use super::*;
    use crate::fakes::{FakeSource, until};

    fn song(id: &str, title: &str, artist: &str, duration: f64) -> Track {
        Track {
            id: snellar_core::ids::TrackId::youtube(&format!("{id:_<11}")),
            title: title.into(),
            artists: vec![Ref { id: String::new(), name: artist.into() }],
            duration,
            ..Track::default()
        }
    }

    struct Rig {
        source: Arc<FakeSource>,
        library: Arc<Library>,
        events: Arc<Mutex<Vec<ImportProgress>>>,
    }

    impl Rig {
        fn new() -> Rig {
            let source = FakeSource::new(Capabilities::default());
            source.answer_search(vec![song("lucky", "Get Lucky", "Daft Punk", 248.0), song("creep", "Creep", "Radiohead", 238.0)]);
            Rig { source, library: Arc::new(Library::open(Path::new(":memory:")).unwrap()), events: Arc::default() }
        }

        fn job(&self) -> Job {
            let events = self.events.clone();
            let ui: Ui = Arc::new(move |e| {
                if let UiEvent::Import(p) = e {
                    events.lock().unwrap().push(p);
                }
            });
            let dead = reqwest::Client::builder().proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap()).build().unwrap();
            Job::new(self.source.clone(), self.library.clone(), dead, ui)
        }
    }

    fn csv(rows: &[&str]) -> Input {
        Input::Csv { text: format!("Track name,Artist name,Duration\n{}", rows.join("\n")), name: "mine".into() }
    }

    #[tokio::test]
    async fn found_songs_become_a_playlist_in_order_and_the_rest_is_reported() {
        let rig = Rig::new();
        let input = csv(&["Creep,Radiohead,3:58", "Nowhere Song,Nobody,", "Get Lucky (feat. Pharrell Williams),Daft Punk,4:08"]);
        let result = Imports::default().run(rig.job(), input, None).await.unwrap();
        assert_eq!((result.total, result.matched, result.name.as_str()), (3, 2, "mine"));
        assert_eq!(result.unmatched.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Nowhere Song"]);
        let list = rig.library.playlist(result.playlist.unwrap()).unwrap();
        assert_eq!(list.name, "mine");
        assert_eq!(list.tracks.iter().map(|t| t.item.title.as_str()).collect::<Vec<_>>(), ["Creep", "Get Lucky"]);
        let events = rig.events.lock().unwrap();
        assert_eq!(events.first().unwrap().stage, "reading");
        assert_eq!(events.last().unwrap(), &ImportProgress { stage: "matching".into(), done: 3, total: 3, matched: 2 });
        assert_eq!(rig.source.foreground_searches(), 0, "an import does not count as someone waiting");
        assert!(
            rig.source.calls().iter().any(|c| c == "search videos Nobody Nowhere Song"),
            "a song not found among songs is looked for among videos"
        );
    }

    #[tokio::test]
    async fn a_given_name_wins_and_nothing_found_makes_no_playlist() {
        let rig = Rig::new();
        let named = Imports::default().run(rig.job(), csv(&["Creep,Radiohead,"]), Some("  Road  ".into())).await.unwrap();
        assert_eq!(rig.library.playlist(named.playlist.unwrap()).unwrap().name, "Road");
        let none = Imports::default().run(rig.job(), csv(&["Nowhere,Nobody,"]), None).await.unwrap();
        assert_eq!((none.playlist, none.matched, none.unmatched.len()), (None, 0, 1));
        assert_eq!(rig.library.playlists().len(), 1);
    }

    #[tokio::test]
    async fn a_library_that_refuses_the_write_fails_the_import_instead_of_reporting_playlist_zero() {
        let dir = std::env::temp_dir().join(format!("snellar-import-refused-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.db");
        let rig = Rig { library: Arc::new(Library::open(&path).unwrap()), ..Rig::new() };
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TRIGGER refuse BEFORE INSERT ON playlist_items BEGIN SELECT RAISE(ABORT, 'disk full'); END;")
            .unwrap();
        let e = Imports::default().run(rig.job(), csv(&["Creep,Radiohead,"]), None).await.unwrap_err();
        assert_eq!(Code::of(&e), Code::LibraryFailed);
        assert!(rig.library.playlists().is_empty(), "no empty playlist is left behind");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_network_loss_midway_is_counted_apart_from_songs_not_found() {
        let rig = Rig::new();
        rig.source.cut_search("Blackout");
        let input = csv(&["Creep,Radiohead,3:58", "Blackout One,Lost,", "Blackout Two,Lost,", "Nowhere Song,Nobody,"]);
        let result = Imports::default().run(rig.job(), input, None).await.unwrap();
        assert_eq!((result.total, result.matched, result.failed), (4, 1, 2));
        assert_eq!(result.code, Some(Code::NetOffline));
        assert_eq!(
            result.unmatched.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
            ["Nowhere Song"],
            "only what was really looked for"
        );
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!((json["failed"].as_u64(), json["code"].as_str()), (Some(2), Some("net.offline")));
        assert_eq!(rig.library.playlist(result.playlist.unwrap()).unwrap().count, 1);
    }

    #[tokio::test]
    async fn at_most_four_searches_run_at_once_and_cancel_leaves_no_playlist() {
        let rig = Rig::new();
        let gate = rig.source.hold_search();
        let imports = Arc::new(Imports::default());
        let rows: Vec<String> = (0..10).map(|i| format!("Song {i},Artist,")).collect();
        let input = csv(&rows.iter().map(String::as_str).collect::<Vec<_>>());
        let running = tokio::spawn({
            let (imports, job) = (imports.clone(), rig.job());
            async move { imports.run(job, input, None).await }
        });
        until("four searches", || rig.source.calls().len() == SEARCHES_AT_ONCE).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(rig.source.calls().len(), SEARCHES_AT_ONCE, "a fifth search waits for a free slot");
        gate.add_permits(2);
        until("two more searches", || rig.source.calls().len() == SEARCHES_AT_ONCE + 2).await;
        assert!(imports.cancel());
        assert_eq!(Code::of(&running.await.unwrap().unwrap_err()), Code::ImportCancelled);
        gate.add_permits(100);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(rig.source.calls().len(), SEARCHES_AT_ONCE + 2, "nothing runs after the cancel");
        assert!(rig.library.playlists().is_empty());
        assert!(!imports.cancel(), "nothing left to cancel");
    }

    #[tokio::test]
    async fn a_newer_import_replaces_the_running_one() {
        let rig = Rig::new();
        let gate = rig.source.hold_search();
        let imports = Arc::new(Imports::default());
        let first = tokio::spawn({
            let (imports, job) = (imports.clone(), rig.job());
            async move { imports.run(job, csv(&["Creep,Radiohead,"]), None).await }
        });
        until("first search", || rig.source.calls().len() == 1).await;
        let second = tokio::spawn({
            let (imports, job) = (imports.clone(), rig.job());
            async move { imports.run(job, csv(&["Get Lucky,Daft Punk,"]), None).await }
        });
        assert_eq!(Code::of(&first.await.unwrap().unwrap_err()), Code::ImportCancelled);
        gate.add_permits(100);
        let result = second.await.unwrap().unwrap();
        assert_eq!(rig.library.playlist(result.playlist.unwrap()).unwrap().tracks[0].item.title, "Get Lucky");
        assert_eq!(rig.library.playlists().len(), 1);
    }

    async fn serving(answer: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/cover", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0; 2048];
                if tokio::io::AsyncReadExt::read(&mut socket, &mut request).await.unwrap_or(0) > 0 {
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", answer.len());
                    tokio::io::AsyncWriteExt::write_all(&mut socket, head.as_bytes()).await.ok();
                    tokio::io::AsyncWriteExt::write_all(&mut socket, &answer).await.ok();
                }
            }
        });
        url
    }

    fn sourced(cover: String) -> Listing {
        Listing {
            name: "From a link".into(),
            tracks: vec![ImportTrack { title: "Creep".into(), artists: vec!["Radiohead".into()], ..ImportTrack::default() }],
            cover: Some(cover),
            ..Listing::default()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_cover_of_the_source_is_saved_locally_and_a_bad_one_changes_nothing() {
        let dir = std::env::temp_dir().join(format!("snellar-import-cover-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let rig = Rig { library: Arc::new(Library::open(&dir.join("library.db")).unwrap()), ..Rig::new() };
        let covers = dir.join("covers");
        let job = || Job { covers: Some(covers.clone()), http: reqwest::Client::builder().no_proxy().build().unwrap(), ..rig.job() };

        let image = std::fs::read(format!("{}/tests/fixtures/cover.png", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let result = job().fill(sourced(serving(image).await), None).await.unwrap();
        let list = rig.library.playlist(result.playlist.unwrap()).unwrap();
        assert!(list.thumb.starts_with(&format!("file://{}/import-", covers.display())), "{}", list.thumb);
        assert!(Path::new(list.thumb.strip_prefix("file://").unwrap()).exists());

        let page = job().fill(sourced(serving(b"<html>".to_vec()).await), None).await.unwrap();
        let gone = job().fill(sourced("http://127.0.0.1:1/cover".into()), None).await.unwrap();
        for result in [page, gone] {
            let list = rig.library.playlist(result.playlist.unwrap()).unwrap();
            assert_eq!((list.cover.as_str(), list.count), ("", 1), "the import goes on with the first song's cover");
        }
        assert_eq!(std::fs::read_dir(&covers).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_silent_search_times_out_and_is_tried_once_more() {
        let rig = Rig::new();
        let _gate = rig.source.hold_search();
        let job = Job { timeout: Duration::from_millis(40), ..rig.job() };
        let e = Imports::default().run(job, csv(&["Creep,Radiohead,"]), None).await.unwrap_err();
        assert_eq!(Code::of(&e), Code::NetTimeout, "every search failed, so the import fails with the cause");
        assert_eq!(rig.source.calls().len(), 2);
    }

    #[tokio::test]
    async fn a_link_that_no_service_knows_is_refused_before_any_request() {
        let rig = Rig::new();
        let e = Imports::default().run(rig.job(), Input::Link("https://example.com/list".into()), None).await.unwrap_err();
        assert_eq!(Code::of(&e), Code::ImportUnsupported);
        let e = Imports::default().run(rig.job(), csv(&[]), None).await.unwrap_err();
        assert_eq!(Code::of(&e), Code::ImportEmpty);
    }

    #[tokio::test]
    async fn files_are_read_as_text_with_a_size_limit() {
        let dir = std::env::temp_dir().join(format!("snellar-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("Summer mix.csv");
        std::fs::write(&good, "Title,Artist\nCreep,Radiohead\n").unwrap();
        let Input::Csv { name, text } = Input::file(&format!("file://{}", good.display())).await.unwrap() else { panic!("a file is CSV") };
        assert_eq!((name.as_str(), text.lines().count()), ("Summer mix", 2));
        let binary = dir.join("cover.png");
        std::fs::write(&binary, [0x89, 0x50, 0xff, 0xfe, 0x00]).unwrap();
        for bad in [binary.display().to_string(), dir.join("missing.csv").display().to_string()] {
            assert_eq!(Code::of(&Input::file(&bad).await.err().unwrap()), Code::ImportFile, "{bad}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
