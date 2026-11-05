use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedReceiver;

use snellar_core::source::BoxFuture;

#[derive(Debug)]
pub enum Event {
    Property(String, Value),
    EndFile { reason: String, error: Option<String> },
    Died,
}

pub trait AudioOut: Send + Sync {
    fn set<'a>(&'a self, name: &'a str, value: Value) -> BoxFuture<'a, Result<()>>;

    fn get<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Value>>;

    fn load<'a>(&'a self, url: &'a str, start: f64, gain: Option<f64>) -> BoxFuture<'a, Result<()>>;

    fn append<'a>(&'a self, url: &'a str, gain: Option<f64>) -> BoxFuture<'a, Result<()>>;

    fn clear_next(&self) -> BoxFuture<'_, Result<()>>;

    fn seek(&self, position: f64) -> BoxFuture<'_, Result<()>>;

    fn stop(&self) -> BoxFuture<'_, Result<()>>;
}

pub type Opened = (Arc<dyn AudioOut>, UnboundedReceiver<Event>);

pub type Launch = Arc<dyn Fn() -> BoxFuture<'static, Result<Opened>> + Send + Sync>;
