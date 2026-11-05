use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

const CAPACITY: usize = 256;

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
        self.entries.get(key).filter(|(at, _)| at.elapsed() < self.ttl).map(|(_, v)| v)
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

#[derive(Default)]
pub struct Memo {
    entries: Mutex<HashMap<String, (Instant, Value)>>,
}

impl Memo {
    pub async fn get(&self, key: String, ttl: Duration, fetch: impl Future<Output = Result<Value>>) -> Result<Value> {
        if let Some((at, value)) = self.entries.lock().unwrap().get(&key)
            && at.elapsed() < ttl
        {
            return Ok(value.clone());
        }
        let value = fetch.await?;
        let mut entries = self.entries.lock().unwrap();
        if entries.len() >= CAPACITY {
            entries.retain(|_, (at, _)| at.elapsed() < ttl);
        }
        if entries.len() >= CAPACITY
            && let Some(oldest) = entries.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(key, (Instant::now(), value.clone()));
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    #[test]
    fn a_bounded_cache_never_grows_past_its_cap_and_keeps_the_newest() {
        let mut cache = Bounded::new(4, Duration::from_secs(60));
        for i in 0..50 {
            cache.insert(format!("k{i}"), i);
            assert!(cache.len() <= 4);
        }
        assert_eq!((cache.get("k49"), cache.get("k46"), cache.get("k45")), (Some(&49), Some(&46), None));
    }

    #[test]
    fn expired_entries_are_dropped_on_insert_and_never_returned() {
        let mut cache = Bounded::new(100, Duration::from_secs(60));
        cache.insert_aged("old".into(), Duration::from_secs(30), 1);
        cache.insert_aged("stale".into(), Duration::from_secs(61), 2);
        assert_eq!((cache.get("old"), cache.get("stale"), cache.len()), (Some(&1), None, 1));
        let mut short = Bounded::new(100, Duration::from_millis(20));
        short.insert("a".into(), 1);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(short.get("a"), None);
        assert_eq!(short.remove("a"), None);
        short.insert("b".into(), 2);
        short.insert("c".into(), 3);
        std::thread::sleep(Duration::from_millis(40));
        short.insert("d".into(), 4);
        assert_eq!(short.len(), 1, "the expired entries are still held in memory");
    }

    #[tokio::test]
    async fn a_failed_write_leaves_the_old_file_intact() {
        let dir = std::env::temp_dir().join(format!("snellar-atomic-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data.json.tmp")).unwrap();
        std::fs::write(dir.join("data.json"), b"old").unwrap();
        assert!(write_atomic(&dir.join("data.json"), b"new content".to_vec()).await.is_err());
        assert_eq!(std::fs::read(dir.join("data.json")).unwrap(), b"old");
        std::fs::remove_dir(dir.join("data.json.tmp")).unwrap();
        write_atomic(&dir.join("data.json"), b"new content".to_vec()).await.unwrap();
        assert_eq!(std::fs::read(dir.join("data.json")).unwrap(), b"new content");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn serves_repeats_until_ttl() {
        let memo = Memo::default();
        let calls = AtomicUsize::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(json!(calls.load(Ordering::Relaxed)))
        };
        assert_eq!(memo.get("a".into(), Duration::from_secs(60), fetch()).await.unwrap(), json!(1));
        assert_eq!(memo.get("a".into(), Duration::from_secs(60), fetch()).await.unwrap(), json!(1));
        assert_eq!(memo.get("b".into(), Duration::from_secs(60), fetch()).await.unwrap(), json!(2));
        assert_eq!(memo.get("a".into(), Duration::ZERO, fetch()).await.unwrap(), json!(3));
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        let memo = Memo::default();
        assert!(memo.get("a".into(), Duration::from_secs(60), async { anyhow::bail!("down") }).await.is_err());
        assert_eq!(memo.get("a".into(), Duration::from_secs(60), async { Ok(json!(1)) }).await.unwrap(), json!(1));
    }
}
