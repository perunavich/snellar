use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

use snellar_core::source::BoxFuture;

#[derive(Debug)]
pub enum Event {
    Property(String, Value),
    /// mpv took the file loaded with this mark: its answer, in order with what it says of its files.
    Loaded(u64),
    /// mpv queued the file appended with this mark after the one it plays, in order the same way.
    Appended(u64),
    /// mpv took the seek sent with this mark: what it says after this is from after the seek.
    Seeked(u64),
    /// mpv took the clear of its list sent with this mark.
    Cleared(u64),
    EndFile {
        reason: String,
        error: Option<String>,
    },
    Died,
}

pub trait AudioOut: Send + Sync {
    fn set<'a>(&'a self, name: &'a str, value: Value) -> BoxFuture<'a, Result<()>>;

    fn get<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Value>>;

    fn af_command<'a>(&'a self, label: &'a str, target: &'a str, option: &'a str, value: &'a str) -> BoxFuture<'a, Result<()>>;

    fn load<'a>(&'a self, url: &'a str, start: f64, gain: Option<f64>, mark: u64) -> BoxFuture<'a, Result<()>>;

    fn append<'a>(&'a self, url: &'a str, gain: Option<f64>, mark: u64) -> BoxFuture<'a, Result<()>>;

    fn clear_next(&self, mark: u64) -> BoxFuture<'_, Result<()>>;

    fn seek(&self, position: f64, mark: u64) -> BoxFuture<'_, Result<()>>;

    fn stop(&self) -> BoxFuture<'_, Result<()>>;

    /// mpv is gone: what failed failed because of that, and its end is on its way as `Event::Died`.
    fn gone(&self) -> bool;
}

pub type Opened = (Arc<dyn AudioOut>, UnboundedReceiver<Event>);

pub type Launch = Arc<dyn Fn() -> BoxFuture<'static, Result<Opened>> + Send + Sync>;
