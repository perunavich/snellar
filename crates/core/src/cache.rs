use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::sync::{OwnedMutexGuard, Semaphore};

use crate::error::Code;
use crate::sync::Lock;

pub struct Bounded<V> {
    cap: usize,
    ttl: Duration,
    entries: HashMap<String, (Instant, V)>,
}

impl<V> Bounded<V> {
    pub fn new(cap: usize, ttl: Duration) -> Bounded<V> {
        Bounded { cap, ttl, entries: HashMap::new() }
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.get_aged(key).map(|(_, v)| v)
    }

    pub fn get_aged(&self, key: &str) -> Option<(Duration, &V)> {
        self.entries.get(key).map(|(at, v)| (at.elapsed(), v)).filter(|(age, _)| *age < self.ttl)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn insert(&mut self, key: String, value: V) {
        self.insert_aged(key, Duration::ZERO, value);
    }

    pub fn insert_aged(&mut self, key: String, age: Duration, value: V) {
        let Some(at) = Instant::now().checked_sub(age).filter(|_| age < self.ttl) else { return };
        let ttl = self.ttl;
        self.entries.retain(|_, (t, _)| t.elapsed() < ttl);
        while self.entries.len() >= self.cap && !self.entries.contains_key(&key) {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone()) else { break };
            self.entries.remove(&oldest);
        }
        self.entries.insert(key, (at, value));
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        let (at, value) = self.entries.remove(key)?;
        (at.elapsed() < self.ttl).then_some(value)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn aged(&self) -> impl Iterator<Item = (&String, Duration, &V)> {
        self.entries.iter().filter(|(_, (at, _))| at.elapsed() < self.ttl).map(|(k, (at, v))| (k, at.elapsed(), v))
    }
}

pub async fn write_atomic(path: &std::path::Path, bytes: Vec<u8>) -> std::io::Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}

pub struct Rule {
    pub name: &'static str,
    pub cap: usize,
    pub fresh: Duration,
    pub keep: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Warmed {
    Fresh,
    Busy,
    Fetched,
    Failed,
}

#[derive(Default)]
pub struct Flights {
    held: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

pub struct Flight<'a> {
    flights: &'a Flights,
    guard: Option<OwnedMutexGuard<()>>,
}

impl Flights {
    fn lock_for(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.held.locked().entry(key.into()).or_default().clone()
    }

    pub async fn hold(&self, key: &str) -> Flight<'_> {
        let guard = self.lock_for(key).lock_owned().await;
        Flight { flights: self, guard: Some(guard) }
    }

    pub fn try_hold(&self, key: &str) -> Option<Flight<'_>> {
        let guard = self.lock_for(key).try_lock_owned().ok()?;
        Some(Flight { flights: self, guard: Some(guard) })
    }

    pub fn len(&self) -> usize {
        self.held.locked().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn sweep(&self, held: &mut HashMap<String, Arc<tokio::sync::Mutex<()>>>) {
        held.retain(|_, lock| Arc::strong_count(lock) > 1);
    }
}

impl Drop for Flight<'_> {
    fn drop(&mut self) {
        let mut held = self.flights.held.locked();
        drop(self.guard.take());
        self.flights.sweep(&mut held);
    }
}

pub struct Memo {
    shelves: Mutex<HashMap<&'static str, Bounded<Value>>>,
    flights: Flights,
    background: Semaphore,
}

impl Memo {
    pub fn new(background: usize) -> Memo {
        Memo { shelves: Mutex::default(), flights: Flights::default(), background: Semaphore::new(background) }
    }

    fn aged(&self, rule: &Rule, key: &str) -> Option<(Duration, Value)> {
        let shelves = self.shelves.locked();
        shelves.get(rule.name)?.get_aged(key).map(|(age, v)| (age, v.clone()))
    }

    fn fresh(&self, rule: &Rule, key: &str) -> Option<Value> {
        self.aged(rule, key).filter(|(age, _)| *age < rule.fresh).map(|(_, v)| v)
    }

    pub fn peek(&self, rule: &Rule, key: &str) -> Option<Value> {
        self.aged(rule, key).map(|(_, v)| v)
    }

    fn put(&self, rule: &Rule, key: &str, value: Value) {
        let mut shelves = self.shelves.locked();
        shelves.entry(rule.name).or_insert_with(|| Bounded::new(rule.cap, rule.keep)).insert(key.into(), value);
    }

    pub async fn get(&self, rule: &Rule, key: &str, fetch: impl Future<Output = Result<Value>>) -> Result<Value> {
        if let Some(value) = self.fresh(rule, key) {
            return Ok(value);
        }
        let _flight = self.flights.hold(&format!("{}:{key}", rule.name)).await;
        if let Some(value) = self.fresh(rule, key) {
            return Ok(value);
        }
        let _quiet = match self.peek(rule, key) {
            Some(_) => Some(self.background.acquire().await.context(Code::Internal)?),
            None => None,
        };
        let value = fetch.await?;
        self.put(rule, key, value.clone());
        Ok(value)
    }

    pub async fn warm(&self, rule: &Rule, key: &str, fetch: impl Future<Output = Result<Value>>) -> Warmed {
        if self.fresh(rule, key).is_some() {
            return Warmed::Fresh;
        }
        let Some(_flight) = self.flights.try_hold(&format!("{}:{key}", rule.name)) else { return Warmed::Busy };
        let Ok(_quiet) = self.background.try_acquire() else { return Warmed::Busy };
        match fetch.await {
            Ok(value) => {
                self.put(rule, key, value);
                Warmed::Fetched
            }
            Err(_) => Warmed::Failed,
        }
    }
}
