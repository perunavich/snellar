use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::{Notify, watch};

use snellar_core::cache::{Bounded, Flights};
use snellar_core::ids::TrackId;
use snellar_core::model::{Lyrics, Track};
use snellar_core::source::Source;
use snellar_core::sync::Lock;
use snellar_core::tasks::TaskSet;
use snellar_core::ui::UiEvent;

const KEPT: usize = 20;

#[derive(Clone, Copy)]
pub struct Timing {
    pub found: Duration,
    pub missing: Duration,
    pub reading: Duration,
    pub settle: Duration,
}

pub const TIMING: Timing = Timing {
    found: Duration::from_secs(30 * 60),
    missing: Duration::from_secs(3 * 60),
    reading: Duration::from_secs(5 * 60),
    settle: Duration::from_secs(3),
};

#[derive(Default)]
struct Reading {
    open: bool,
    closed: Option<Instant>,
}

pub struct LyricsMemo {
    source: Arc<dyn Source>,
    timing: Timing,
    kept: Mutex<Bounded<Option<Lyrics>>>,
    flights: Flights,
    reading: Mutex<Reading>,
    loading: AtomicBool,
    starting: Notify,
    next: watch::Sender<Option<Track>>,
    tasks: TaskSet,
}

impl LyricsMemo {
    pub fn new(source: Arc<dyn Source>, timing: Timing) -> Arc<LyricsMemo> {
        let (next, wanted) = watch::channel(None);
        let memo = Arc::new(LyricsMemo {
            source,
            timing,
            kept: Mutex::new(Bounded::new(KEPT, timing.found)),
            flights: Flights::default(),
            reading: Mutex::default(),
            loading: AtomicBool::new(false),
            starting: Notify::new(),
            next,
            tasks: TaskSet::default(),
        });
        memo.tasks.spawn("fetch the lyrics of the next track ahead", ahead(Arc::downgrade(&memo), wanted));
        memo
    }

    pub async fn get(&self, track: &Track) -> Result<Option<Lyrics>> {
        if let Some(kept) = self.peek(&track.id) {
            return Ok(kept);
        }
        let key = track.id.to_string();
        let _flight = self.flights.hold(&key).await;
        if let Some(kept) = self.peek(&track.id) {
            return Ok(kept);
        }
        self.fetch(track, key).await
    }

    pub fn peek(&self, id: &TrackId) -> Option<Option<Lyrics>> {
        self.kept.locked().get(&id.to_string()).cloned()
    }

    pub fn shown(&self, on: bool, away: bool) {
        let mut reading = self.reading.locked();
        let opened = on && !reading.open;
        if reading.open && !on {
            reading.closed = Some(Instant::now());
        }
        if away {
            reading.closed = None;
        }
        reading.open = on;
        drop(reading);
        if opened {
            self.next.send_modify(|_| ());
        }
    }

    pub fn hear(&self, event: &UiEvent) {
        match event {
            UiEvent::Queue { tracks, cursor } => {
                let next = usize::try_from(*cursor).ok().and_then(|c| tracks.get(c + 1)).cloned();
                self.next.send_if_modified(|was| {
                    let moved = was.as_ref().map(|t| &t.id) != next.as_ref().map(|t| &t.id);
                    *was = next;
                    moved
                });
            }
            UiEvent::State(state) => self.loading(state.loading),
            _ => {}
        }
    }

    fn loading(&self, now: bool) {
        match (self.loading.swap(now, Ordering::Relaxed), now) {
            (false, true) => self.starting.notify_waiters(),
            (true, false) => self.next.send_modify(|_| ()),
            _ => {}
        }
    }

    fn reading(&self) -> bool {
        let reading = self.reading.locked();
        reading.open || reading.closed.is_some_and(|at| at.elapsed() < self.timing.reading)
    }

    async fn fetch(&self, track: &Track, key: String) -> Result<Option<Lyrics>> {
        let found = self.source.lyrics(track).await?;
        if track.duration <= 0.0 {
            return Ok(found);
        }
        let age = if found.is_some() { Duration::ZERO } else { self.timing.found.saturating_sub(self.timing.missing) };
        self.kept.locked().insert_aged(key, age, found.clone());
        Ok(found)
    }

    async fn prefetch(&self, track: &Track) {
        if track.duration <= 0.0 {
            return;
        }
        let key = track.id.to_string();
        let Some(_flight) = self.flights.try_hold(&key) else { return };
        if self.peek(&track.id).is_none() {
            self.fetch(track, key).await.ok();
        }
    }
}

async fn ahead(memo: Weak<LyricsMemo>, mut wanted: watch::Receiver<Option<Track>>) {
    let Some(settle) = memo.upgrade().map(|m| m.timing.settle) else { return };
    while wanted.changed().await.is_ok() {
        while let Ok(again) = tokio::time::timeout(settle, wanted.changed()).await {
            if again.is_err() {
                return;
            }
        }
        let Some(memo) = memo.upgrade() else { return };
        let next = wanted.borrow_and_update().clone();
        let starting = memo.starting.notified();
        tokio::pin!(starting);
        starting.as_mut().enable();
        if let Some(track) = next.filter(|_| memo.reading() && !memo.loading.load(Ordering::Relaxed)) {
            tokio::select! {
                () = memo.prefetch(&track) => {}
                () = starting => {}
            }
        }
    }
}
