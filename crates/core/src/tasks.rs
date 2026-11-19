use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use tokio::task::{Id, JoinSet};

use crate::journal;
use crate::sync::Lock;

#[derive(Default)]
pub struct TaskSet(Mutex<Running>);

#[derive(Default)]
struct Running {
    set: JoinSet<()>,
    names: HashMap<Id, &'static str>,
    /// Set by `abort_all`: the set is finished with, and what is spawned later is dropped instead of run.
    closed: bool,
}

impl TaskSet {
    pub fn spawn<F>(&self, name: &'static str, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let mut running = self.0.locked();
        if running.closed {
            return;
        }
        running.reap();
        let id = running.set.spawn(task).id();
        running.names.insert(id, name);
    }

    pub fn len(&self) -> usize {
        let mut running = self.0.locked();
        running.reap();
        running.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Aborts everything still running, as dropping the set would, while something else still holds it. The set is
    /// closed: a spawn after this drops its task, so a job racing the abort cannot start another one.
    pub fn abort_all(&self) {
        let mut running = self.0.locked();
        running.closed = true;
        running.set.abort_all();
    }
}

impl Running {
    fn reap(&mut self) {
        while let Some(done) = self.set.try_join_next_with_id() {
            let (id, panic) = match done {
                Ok((id, ())) => (id, None),
                Err(e) => (e.id(), e.is_panic().then(|| e.to_string())),
            };
            let name = self.names.remove(&id).unwrap_or("task");
            if let Some(panic) = panic {
                journal::error("task", format!("{name}: {panic}"));
            }
        }
    }
}
