use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use tokio::process::Command;

use snellar_core::error::Code;
use snellar_core::ids::TrackId;
use snellar_core::library::Library;
use snellar_core::model::Track;
use snellar_core::settings;
use snellar_core::sync::Lock;
use snellar_core::ui::{Toast, Ui, UiEvent};
use snellar_youtube::stream::Resolver;
use snellar_youtube::ytm;

pub mod collections;

/// The state settings the downloads read with a type, each with the test of its saved value: the one place that
/// pairs such a setting with its type. An explicit write keeps aside, and `set_aside_unreadable` sets aside, only a
/// stored value its test refuses.
const SHAPED: [(&settings::Setting, settings::Reads); 3] = [
    (&settings::DOWNLOADED_COLLECTIONS, settings::reads_as::<collections::Kept>),
    (&settings::DOWNLOAD_REPAIRS, settings::reads_as::<Vec<TrackId>>),
    (&settings::DOWNLOAD_REPAIRS_REFUSED, settings::reads_as::<Vec<TrackId>>),
];

/// The test of a saved value for a setting in `SHAPED`; none for any other setting.
pub fn reads(key: &str) -> Option<settings::Reads> {
    SHAPED.iter().find(|(s, _)| s.key == key).map(|(_, reads)| *reads)
}

const DOWNLOAD_LIMIT: Duration = Duration::from_secs(15 * 60);
const AUDIO: [&str; 7] = ["opus", "m4a", "webm", "ogg", "mp3", "flac", "aac"];
const AT_ONCE: usize = 2;
const FREE_FLOOR: u64 = 1 << 30;
const NAME_BYTES: usize = 200;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    name: String,
    path: String,
    size: u64,
    modified: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Why {
    Single,
    Collected,
    Repair,
}

#[derive(Default)]
struct Queue {
    singles: VecDeque<Track>,
    collected: VecDeque<Track>,
    repairs: VecDeque<Track>,
    running: Vec<(TrackId, Why)>,
    /// Songs whose run ended and whose file is being kept: still busy, but asking for one again queues a new run.
    settling: Vec<TrackId>,
    /// Songs a downloaded collection waits for, whichever line they run in: a single download of the same song
    /// still tells the collection when it fails, or the collection would wait for it forever.
    told: HashSet<TrackId>,
    workers: usize,
}

impl Queue {
    fn waiting(&self) -> impl Iterator<Item = &Track> {
        self.singles.iter().chain(&self.collected).chain(&self.repairs)
    }

    fn busy(&self) -> Vec<TrackId> {
        let waiting = self.waiting().map(|t| t.id.clone());
        self.running.iter().map(|(id, _)| id.clone()).chain(self.settling.iter().cloned()).chain(waiting).collect()
    }

    fn want(&mut self, track: Track, why: Why) {
        if why == Why::Collected {
            self.told.insert(track.id.clone());
        }
        if let Some(run) = self.running.iter_mut().find(|(id, _)| *id == track.id) {
            if why == Why::Single || (why == Why::Collected && run.1 == Why::Repair) {
                run.1 = why;
            }
            return;
        }
        let ahead = |line: &VecDeque<Track>| line.iter().position(|t| t.id == track.id);
        if ahead(&self.singles).is_some() {
            return;
        }
        if let Some(at) = ahead(&self.collected) {
            if why != Why::Single {
                return;
            }
            self.collected.remove(at);
        }
        if let Some(at) = ahead(&self.repairs) {
            if why == Why::Repair {
                return;
            }
            self.repairs.remove(at);
        }
        match why {
            Why::Single => self.singles.push_back(track),
            Why::Collected => self.collected.push_back(track),
            Why::Repair => self.repairs.push_back(track),
        }
    }

    fn next(&mut self) -> Option<Track> {
        let (track, why) = match self.singles.pop_front() {
            Some(t) => (t, Why::Single),
            None => match self.collected.pop_front() {
                Some(t) => (t, Why::Collected),
                None => (self.repairs.pop_front()?, Why::Repair),
            },
        };
        self.running.push((track.id.clone(), why));
        Some(track)
    }

    /// Why the song ran in the end, and whether a collection waits for it. From here on the reason is fixed:
    /// the song counts as busy until `settled`, and asking for it again queues a new run.
    fn finish(&mut self, id: &TrackId) -> (Why, bool) {
        let why = self.running.iter().find(|(r, _)| r == id).map_or(Why::Collected, |(_, why)| *why);
        self.running.retain(|(r, _)| r != id);
        self.settling.push(id.clone());
        (why, self.told.remove(id))
    }

    fn settled(&mut self, id: &TrackId) {
        if let Some(at) = self.settling.iter().position(|s| s == id) {
            self.settling.remove(at);
        }
    }
}

pub type Finished = Box<dyn Fn(&Track, Result<(), Code>) + Send + Sync>;

/// The file yt-dlp saved, and whether this run made it: a song downloaded before keeps its old name.
#[derive(Debug)]
struct Fetched {
    path: PathBuf,
    fresh: bool,
}

pub struct Downloads {
    ui: Ui,
    library: Arc<Library>,
    bin: Option<PathBuf>,
    resolver: Arc<Resolver>,
    limit: Duration,
    floor: u64,
    queue: Mutex<Queue>,
    files: Mutex<()>,
    after: OnceLock<Finished>,
}

fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '/' || c.is_control() { ' ' } else { c }).collect::<String>().trim().to_string()
}

fn meta(value: &str) -> String {
    let text = value.replace('%', "%%").replace(':', "\\:");
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphabetic() || b == b'_') {
        format!("{text}%({}|)s", snellar_core::product::ID)
    } else {
        text
    }
}

fn options(ffmpeg: bool, thumbnail: bool) -> Vec<&'static str> {
    let mut args = vec!["-f", "bestaudio", "--no-playlist", "--no-warnings", "--print", "after_move:filepath"];
    if ffmpeg {
        args.extend(["-x", "--audio-format", "best", "--embed-metadata"]);
    }
    if ffmpeg && thumbnail {
        args.push("--embed-thumbnail");
    }
    args
}

fn free_bytes(dir: &Path) -> Option<u64> {
    let path = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is a NUL-terminated string that outlives the call, and `stat` is read only after statvfs filled it.
    let stat = unsafe {
        if libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return None;
        }
        stat.assume_init()
    };
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

async fn has_mutagen(bin: &Path) -> bool {
    let start = std::fs::read(bin).unwrap_or_default();
    if start.starts_with(b"\x7fELF") {
        return true;
    }
    let shebang = String::from_utf8_lossy(&start).lines().next().and_then(|l| l.strip_prefix("#!").map(str::to_string));
    Command::new(shebang.as_deref().filter(|line| line.contains("python")).unwrap_or("python3"))
        .args(["-c", "import mutagen"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

impl Downloads {
    pub fn new(ui: Ui, library: Arc<Library>, resolver: Arc<Resolver>) -> Arc<Downloads> {
        Arc::new(Downloads::with(ui, library, resolver, None, DOWNLOAD_LIMIT))
    }

    fn with(ui: Ui, library: Arc<Library>, resolver: Arc<Resolver>, bin: Option<PathBuf>, limit: Duration) -> Downloads {
        let (queue, files, after) = (Mutex::default(), Mutex::default(), OnceLock::new());
        Downloads { ui, library, bin, resolver, limit, floor: FREE_FLOOR, queue, files, after }
    }

    fn dir(&self) -> PathBuf {
        let custom = settings::DOWNLOAD_DIR.get::<String>(&self.library).and_then(|text| snellar_core::product::download_folder(&text));
        custom.unwrap_or_else(snellar_core::product::downloads_dir)
    }

    pub fn on_finished(&self, after: Finished) {
        if self.after.set(after).is_err() {
            snellar_core::journal::warn("downloads", "a second finish hook was ignored");
        }
    }

    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.locked()
    }

    fn files(&self) -> MutexGuard<'_, ()> {
        self.files.locked()
    }

    pub fn busy(&self) -> Vec<TrackId> {
        self.queue().busy()
    }

    pub fn tell(&self) {
        (self.ui)(UiEvent::Downloads(self.busy()));
    }

    pub fn start(self: &Arc<Self>, track: Track) {
        self.push(vec![track], Why::Single);
    }

    pub fn collect(self: &Arc<Self>, tracks: Vec<Track>) {
        if !tracks.is_empty() {
            self.push(tracks, Why::Collected);
        }
    }

    /// The one way back for a download list this build cannot read, run where the downloads start and when the library
    /// opens again, before anything reads them: each such list is kept aside and starts again from its default
    /// (`Setting::set_aside`), and one toast says so. No file is deleted; a list whose copy cannot be written stays.
    pub fn set_aside_unreadable(&self) {
        let _files = self.files();
        let mut set_aside = false;
        for (setting, reads) in SHAPED {
            match setting.set_aside(&self.library, reads) {
                Ok(done) => set_aside |= done,
                Err(e) => snellar_core::journal::warn("downloads", format!("{e:#}")),
            }
        }
        if set_aside {
            (self.ui)(UiEvent::Toast(Toast::new(Code::DownloadListSetAside)));
        }
    }

    /// Writes down the songs that share a file with another song, so they download again even when one of them
    /// is turned off before the repair runs, and answers those due now. Called at once at start; the repair
    /// itself waits. A song whose repair was refused for good is not written down again. A saved repair whose
    /// file is gone from a folder that is there was deleted and goes; one whose folder is missing (a drive not
    /// mounted yet) stays for a later start.
    pub fn note_repairs(&self) -> Vec<(TrackId, PathBuf)> {
        let _files = self.files();
        let read = self.library.download_paths().and_then(|rows| {
            let saved = settings::DOWNLOAD_REPAIRS.read::<Vec<TrackId>>(&self.library)?;
            let refused = settings::DOWNLOAD_REPAIRS_REFUSED.read::<Vec<TrackId>>(&self.library)?;
            Ok((rows, saved, refused))
        });
        let (rows, saved, refused) = match read {
            Ok(read) => read,
            Err(e) => {
                snellar_core::journal::warn("downloads", format!("no repair this time, the library cannot be read: {e:#}"));
                return Vec::new();
            }
        };
        let mut owners: HashMap<&Path, usize> = HashMap::new();
        for (_, path) in &rows {
            *owners.entry(path).or_default() += 1;
        }
        let listed = |id: &TrackId| rows.iter().any(|(row, _)| row == id);
        let kept_refused: Vec<TrackId> = refused.iter().flatten().filter(|id| listed(id)).cloned().collect();
        if refused.as_ref().is_some_and(|r| *r != kept_refused) {
            settings::DOWNLOAD_REPAIRS_REFUSED.set(&self.library, &kept_refused);
        }
        let deleted = |path: &Path| !path.is_file() && path.parent().is_some_and(Path::is_dir);
        let mut due = saved.clone().unwrap_or_default();
        due.extend(
            rows.iter()
                .filter(|(id, path)| owners[path.as_path()] > 1 && !deleted(path) && !due.contains(id))
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
        );
        due.retain(|id| !kept_refused.contains(id) && rows.iter().any(|(row, path)| row == id && !deleted(path)));
        if Some(&due) != saved.as_ref() {
            settings::DOWNLOAD_REPAIRS.set(&self.library, &due);
        }
        rows.into_iter().filter(|(id, path)| due.contains(id) && path.is_file()).collect()
    }

    /// Downloads again the songs `note_repairs` answered.
    pub fn run_repairs(self: &Arc<Self>, due: Vec<(TrackId, PathBuf)>) {
        let tracks: Vec<Track> =
            due.into_iter().map(|(id, path)| self.library.track(&id).unwrap_or_else(|| known_by_file(id, &path))).collect();
        if !tracks.is_empty() {
            snellar_core::journal::info("downloads", format!("{} songs shared a file with another song and download again", tracks.len()));
            self.push(tracks, Why::Repair);
        }
    }

    /// A repair YouTube refused for good: written down as refused first, so `note_repairs` does not add it back
    /// while it still shares the file, then taken off the repairs. When the refusal cannot be written, the
    /// repair stays and is tried at the next start.
    fn refused_repair(&self, id: &TrackId) {
        let Ok(refused) = settings::DOWNLOAD_REPAIRS_REFUSED.read::<Vec<TrackId>>(&self.library) else { return };
        let mut refused = refused.unwrap_or_default();
        if !refused.contains(id) {
            refused.push(id.clone());
            if !settings::DOWNLOAD_REPAIRS_REFUSED.set(&self.library, &refused) {
                return;
            }
        }
        self.repaired(id);
    }

    fn repaired(&self, id: &TrackId) {
        if let Ok(Some(mut due)) = settings::DOWNLOAD_REPAIRS.read::<Vec<TrackId>>(&self.library)
            && due.contains(id)
        {
            due.retain(|d| d != id);
            settings::DOWNLOAD_REPAIRS.set(&self.library, &due);
        }
    }

    /// Removes the song's download and its file once nothing else plays it. False when the library could not
    /// be read or written: then the row and the file stay as they were.
    pub fn forget(&self, id: &TrackId) -> bool {
        let _files = self.files();
        self.queue().repairs.retain(|t| t.id != *id);
        let path = match self.library.download_row(id) {
            Ok(row) => row.map(|(path, _)| path),
            Err(e) => {
                snellar_core::journal::warn("downloads", format!("{id} stays downloaded, the library cannot say where its file is: {e:#}"));
                return false;
            }
        };
        let Some(path) = path else { return true };
        if !self.library.forget_download(id) {
            return false;
        }
        remove_unused(&self.library, &path);
        true
    }

    /// Trims the collected songs still waiting; the caller tells the window once its own locks are let go.
    pub fn drop_collected(&self, keep: impl Fn(&TrackId) -> bool) {
        let mut q = self.queue();
        q.collected.retain(|t| keep(&t.id));
        q.told.retain(|id| keep(id));
    }

    fn push(self: &Arc<Self>, tracks: Vec<Track>, why: Why) {
        let spawn = {
            let mut q = self.queue();
            for track in tracks {
                q.want(track, why);
            }
            let spawn = q.waiting().count().min(AT_ONCE.saturating_sub(q.workers));
            q.workers += spawn;
            spawn
        };
        for _ in 0..spawn {
            let this = self.clone();
            tokio::spawn(async move { this.work().await });
        }
        self.tell();
    }

    async fn work(self: Arc<Self>) {
        loop {
            let next = {
                let mut q = self.queue();
                let next = q.next();
                if next.is_none() {
                    q.workers -= 1;
                }
                next
            };
            let Some(track) = next else { return };
            self.tell();
            let label = format!("{} - {}", clean(&track.artist_line()), clean(&track.title));
            let fetched = self.fetch(&track, &label).await;
            let (why, told, done) = self.keep(&track, fetched);
            let tell_collection = match why {
                Why::Single => {
                    let toast = match &done {
                        Ok(()) => Toast::new(Code::DownloadDone),
                        Err(e) if Code::of(e) == Code::YtdlpMissing => Toast::new(Code::DownloadNoTool),
                        Err(e) => Toast::failed(Code::DownloadFailed, e),
                    };
                    (self.ui)(UiEvent::Toast(toast.arg(label).track(&track.id)));
                    told && done.is_err()
                }
                Why::Collected => true,
                Why::Repair => false,
            };
            if let Some(after) = self.after.get().filter(|_| tell_collection) {
                after(&track, done.as_ref().map(drop).map_err(Code::of));
            }
            self.tell();
        }
    }

    /// Ends the song's run and keeps what it fetched. Why it ran is read under the files lock, the same lock
    /// `forget` takes, so a song turned off and on again while it ran is kept for the reason it has now. A repair
    /// YouTube refuses for good leaves the list of repairs, so yt-dlp does not run for it at every start; a
    /// region block is not for good, the route may change, so that repair stays.
    fn keep(&self, track: &Track, fetched: Result<Fetched>) -> (Why, bool, Result<()>) {
        let _files = self.files();
        let (why, told) = self.queue().finish(&track.id);
        let done = fetched.and_then(|file| self.own(track, &file, why));
        let for_good =
            done.as_ref().err().map(Code::of).is_some_and(|c| c.refusal_lasts() == Some(i64::MAX) || c == Code::SourceUnsupported);
        if why == Why::Repair && for_good {
            self.refused_repair(&track.id);
        }
        self.queue().settled(&track.id);
        (why, told, done)
    }

    /// Writes the fetched file down as the song's download. When the library cannot be read, a file this run
    /// made goes again (nothing can point at it yet) and an older one stays; the error stops the collections.
    fn own(&self, track: &Track, file: &Fetched, why: Why) -> Result<()> {
        let path = file.path.as_path();
        let before = match self.library.download_row(&track.id) {
            Ok(before) => before,
            Err(e) if file.fresh => {
                if let Err(gone) = std::fs::remove_file(path) {
                    snellar_core::journal::warn("downloads", format!("{}: {gone}", path.display()));
                }
                return Err(e.context("the new file is removed"));
            }
            Err(e) => return Err(e.context("the file stays where it is")),
        };
        let written = match (why, &before) {
            (Why::Repair, None) => {
                self.repaired(&track.id);
                remove_unused(&self.library, path);
                return Ok(());
            }
            (Why::Repair, Some(_)) => self.library.move_download(&track.id, path),
            (_, kept) => {
                let measured = track.id.video_id().and_then(|v| self.resolver.format(v)).and_then(|f| f.loudness_db);
                self.library.remember_track(track);
                self.library.set_download(&track.id, path, measured.or(kept.as_ref().and_then(|(_, loudness)| *loudness)))
            }
        };
        if !written {
            remove_unused(&self.library, path);
            return Err(Code::LibraryFailed.err(format!("the download of {} could not be written down", track.id)));
        }
        self.repaired(&track.id);
        if let Some((old, _)) = before.filter(|(old, _)| old != path) {
            remove_unused(&self.library, &old);
        }
        Ok(())
    }

    async fn fetch(&self, track: &Track, label: &str) -> Result<Fetched> {
        let (Some(url), Some(video)) = (ytm::watch_url(&track.id), track.id.video_id()) else {
            return Err(Code::SourceUnsupported.err(format!("`{}` is not a YouTube track", track.id)));
        };
        let dir = self.dir();
        tokio::fs::create_dir_all(&dir).await.with_context(|| format!("cannot create {}", dir.display()))?;
        if let Some(free) = free_bytes(&dir).filter(|free| *free < self.floor) {
            return Err(Code::DownloadNoSpace.err(format!("{} has {free} bytes free", dir.display())));
        }
        let tag = format!("[{video}].");
        let named = |name: &std::ffi::OsStr| name.to_string_lossy().contains(&tag);
        let earlier: HashSet<PathBuf> =
            std::fs::read_dir(&dir).map(|e| e.flatten().filter(|e| named(&e.file_name())).map(|e| e.path()).collect()).unwrap_or_default();
        let bin = self.bin.clone().unwrap_or_else(snellar_core::product::ytdlp_bin);
        let ffmpeg = snellar_core::product::ffmpeg_bin().is_some();
        let mut cmd = Command::new(&bin);
        cmd.args(options(ffmpeg, ffmpeg && has_mutagen(&bin).await));
        if let Some(proxy) = self.resolver.route().proxy() {
            cmd.arg("--proxy").arg(proxy);
        }
        if ffmpeg {
            cmd.arg("--parse-metadata").arg(format!("{}:%(meta_title)s", meta(&track.title)));
            cmd.arg("--parse-metadata").arg(format!("{}:%(meta_artist)s", meta(&track.artist_line())));
            if let Some(album) = &track.album {
                cmd.arg("--parse-metadata").arg(format!("{}:%(meta_album)s", meta(&album.name)));
            }
        }
        let running = cmd
            .arg("-o")
            .arg(dir.join(format!("{} [{video}].%(ext)s", fit(label, NAME_BYTES).replace('%', "%%"))))
            .arg(url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output();
        let out = tokio::time::timeout(self.limit, running)
            .await
            .map_err(|_| Code::YtdlpFailed.err(format!("yt-dlp did not finish in {} s", self.limit.as_secs())))?
            .with_context(|| format!("cannot start {}", bin.display()))
            .context(Code::YtdlpMissing)?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let cause = snellar_youtube::net::ytdlp_cause(&stderr).unwrap_or(Code::YtdlpFailed);
            return Err(cause.err(stderr.lines().rev().find(|l| l.contains("ERROR")).unwrap_or("yt-dlp failed").to_string()));
        }
        let path = saved_path(&String::from_utf8_lossy(&out.stdout))
            .ok_or_else(|| Code::YtdlpFailed.err("yt-dlp did not say where it saved the file"))?;
        Ok(Fetched { fresh: !earlier.contains(&path), path })
    }

    pub fn list(&self) -> Vec<Download> {
        let Ok(entries) = std::fs::read_dir(self.dir()) else { return Vec::new() };
        let mut files: Vec<Download> = entries
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| AUDIO.contains(&x)))
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
                Some(Download {
                    name: e.path().file_stem()?.to_string_lossy().into(),
                    path: e.path().to_string_lossy().into(),
                    size: meta.len(),
                    modified,
                })
            })
            .collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.modified));
        files
    }
}

fn fit(label: &str, bytes: usize) -> &str {
    let end = (0..=bytes.min(label.len())).rev().find(|&at| label.is_char_boundary(at)).unwrap_or(0);
    label[..end].trim_end()
}

/// A downloaded song the library has no details of, named after its file, the way Downloaded shows it.
fn known_by_file(id: TrackId, path: &Path) -> Track {
    let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Track { id, title, ..Track::default() }
}

fn remove_unused(library: &Library, path: &Path) {
    match library.download_paths() {
        Ok(rows) if rows.iter().any(|(_, kept)| kept == path) => return,
        Ok(_) => {}
        Err(e) => {
            let why = format!("{} stays, the library cannot say who uses it: {e:#}", path.display());
            return snellar_core::journal::warn("downloads", why);
        }
    }
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        snellar_core::journal::warn("downloads", format!("{}: {e}", path.display()));
    }
}

fn saved_path(stdout: &str) -> Option<PathBuf> {
    stdout.lines().rev().map(str::trim).find(|l| l.starts_with('/')).map(PathBuf::from)
}
