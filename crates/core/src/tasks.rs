use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use tokio::task::{Id, JoinSet};

use crate::journal;

#[derive(Default)]
pub struct TaskSet(Mutex<Running>);

#[derive(Default)]
struct Running {
    set: JoinSet<()>,
    names: HashMap<Id, &'static str>,
}

impl TaskSet {
    pub fn spawn<F>(&self, name: &'static str, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let mut running = self.0.lock().unwrap();
        running.reap();
        let id = running.set.spawn(task).id();
        running.names.insert(id, name);
    }

    pub fn len(&self) -> usize {
        let mut running = self.0.lock().unwrap();
        running.reap();
        running.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::*;

    struct Flag(Arc<AtomicBool>);

    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn dropping_the_set_aborts_what_is_still_running() {
        let dropped = Arc::new(AtomicBool::new(false));
        let tasks = TaskSet::default();
        let flag = Flag(dropped.clone());
        tasks.spawn("sleeper", async move {
            let _flag = flag;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        });
        tokio::task::yield_now().await;
        assert_eq!(tasks.len(), 1);
        drop(tasks);
        for _ in 0..100 {
            if dropped.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the task outlived its owner");
    }

    #[tokio::test]
    async fn finished_and_panicked_tasks_are_reaped() {
        let tasks = TaskSet::default();
        tasks.spawn("quick", async {});
        tasks.spawn("broken", async { panic!("on purpose") });
        for _ in 0..100 {
            if tasks.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("{} tasks were never reaped", tasks.len());
    }
}
