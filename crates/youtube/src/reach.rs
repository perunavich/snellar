use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::redirect::Policy;
use snellar_core::error::Code;
use snellar_core::journal;
use snellar_core::sync::Lock;
use tokio::task::JoinSet;

const LIMIT: Duration = Duration::from_secs(4);
const PATIENCE: Duration = Duration::from_secs(5);
const FRESH: Duration = Duration::from_secs(10);
pub const POLL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Verdict {
    #[default]
    Online,
    Offline,
    Blocked,
}

impl Verdict {
    pub fn code(self) -> Option<Code> {
        match self {
            Verdict::Online => None,
            Verdict::Offline => Some(Code::NetOffline),
            Verdict::Blocked => Some(Code::NetYoutubeBlocked),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Targets {
    pub youtube: String,
    pub neutral: Vec<String>,
    pub every: Duration,
}

impl Targets {
    // The neutral hosts are ones Snellar already talks to (lyrics, recommendations), not Google's.
    pub fn internet() -> Targets {
        Targets {
            youtube: "https://music.youtube.com/".into(),
            neutral: vec!["https://lrclib.net/".into(), "https://api.deezer.com/".into()],
            every: POLL,
        }
    }
}

type Tell = Box<dyn Fn(Option<Code>) + Send + Sync>;
// How a failed request is named; `net::code`, passed in so this module needs nothing from `net`.
pub type Name = fn(&reqwest::Error) -> Code;

pub struct Armed {
    targets: Targets,
    client: reqwest::Client,
    name: Name,
    tell: Tell,
}

impl Armed {
    pub(crate) fn every(&self) -> Duration {
        self.targets.every
    }
}

#[derive(Default)]
struct State {
    verdict: Verdict,
    checked: Option<Instant>,
    polling: bool,
}

#[derive(Default)]
pub struct Reach {
    armed: Mutex<Option<Arc<Armed>>>,
    state: Mutex<State>,
    pub(crate) flight: tokio::sync::Mutex<()>,
}

pub fn suspicious(code: Code) -> bool {
    matches!(code, Code::NetOffline | Code::NetDns | Code::NetConnect | Code::NetTls | Code::NetTimeout)
}

async fn answers(client: &reqwest::Client, name: Name, url: &str) -> Result<(), Code> {
    client.head(url).send().await.map(drop).map_err(|e| name(&e))
}

async fn any_answers(client: &reqwest::Client, name: Name, urls: &[String]) -> Option<String> {
    let mut asked = JoinSet::new();
    for url in urls {
        let (client, url) = (client.clone(), url.clone());
        asked.spawn(async move { answers(&client, name, &url).await.is_ok().then_some(url) });
    }
    while let Some(done) = asked.join_next().await {
        if let Ok(Some(url)) = done {
            return Some(url);
        }
    }
    None
}

impl Reach {
    pub fn arm(&self, targets: Targets, name: Name, tell: impl Fn(Option<Code>) + Send + Sync + 'static) {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(LIMIT)
            .timeout(PATIENCE)
            .build()
            .expect("reach client");
        *self.armed.locked() = Some(Arc::new(Armed { targets, client, name, tell: Box::new(tell) }));
    }

    pub fn verdict(&self) -> Verdict {
        self.state.locked().verdict
    }

    pub(crate) fn armed(&self) -> Option<Arc<Armed>> {
        self.armed.locked().clone()
    }

    pub(crate) fn fresh(&self) -> bool {
        self.state.locked().checked.is_some_and(|at| at.elapsed() < FRESH)
    }

    // YouTube answering at all, any status, is enough. Blocked needs YouTube failing and a
    // neutral host answering in the same look; when neither answers it is no connection.
    pub(crate) async fn look(armed: &Armed, full: bool) -> Verdict {
        if !full {
            return match answers(&armed.client, armed.name, &armed.targets.youtube).await {
                Ok(()) => Verdict::Online,
                Err(_) => Verdict::Blocked,
            };
        }
        let (youtube, neutral) = tokio::join!(
            answers(&armed.client, armed.name, &armed.targets.youtube),
            any_answers(&armed.client, armed.name, &armed.targets.neutral)
        );
        match (youtube, neutral) {
            (Ok(()), _) => Verdict::Online,
            (Err(code), Some(other)) => {
                journal::warn("net", format!("{} fails with {code} while {other} answers", armed.targets.youtube));
                Verdict::Blocked
            }
            (Err(code), None) => {
                journal::warn("net", format!("{} fails with {code} and no other host answers", armed.targets.youtube));
                Verdict::Offline
            }
        }
    }

    pub(crate) fn record(&self, seen: Verdict, armed: &Armed) -> bool {
        let mut st = self.state.locked();
        st.checked = Some(Instant::now());
        let changed = std::mem::replace(&mut st.verdict, seen) != seen;
        let start = seen != Verdict::Online && !st.polling;
        st.polling = seen != Verdict::Online;
        drop(st);
        if changed {
            journal::info("net", format!("reach: {seen:?}"));
            (armed.tell)(seen.code());
        }
        start
    }
}
