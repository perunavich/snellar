use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Serialize;

use crate::error::Code;
use crate::ids::TrackId;
use crate::model::{ImportProgress, Repeat, StreamInfo, Track};

#[derive(Clone, Debug, Default)]
pub struct PlayState {
    pub playing: bool,
    pub loading: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Toast {
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<Code>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub arg: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub track: String,
}

impl Toast {
    pub fn new(code: Code) -> Toast {
        Toast { code, cause: None, arg: String::new(), detail: String::new(), track: String::new() }
    }

    pub fn failed(code: Code, e: &anyhow::Error) -> Toast {
        Toast { code, cause: Some(Code::of(e)), arg: String::new(), detail: format!("{e:#}"), track: String::new() }
    }

    pub fn arg(self, arg: impl Into<String>) -> Toast {
        Toast { arg: arg.into(), ..self }
    }

    pub fn track(self, track: &TrackId) -> Toast {
        Toast { track: track.to_string(), ..self }
    }

    pub fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlayFailure {
    pub id: TrackId,
    pub code: Code,
    pub tracks: BTreeMap<String, Code>,
    pub stopped: bool,
    pub retrying: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LibraryProblem {
    pub code: Code,
    pub path: String,
    pub detail: String,
    pub saving: bool,
}

#[derive(Clone, Debug)]
pub enum UiEvent {
    Reply { id: i32, ok: bool, json: String },
    State(PlayState),
    Position(f64),
    Current(Option<Track>),
    Queue { tracks: Vec<Track>, cursor: i32 },
    Liked(bool),
    History,
    Tools,
    Update,
    Modes { shuffle: bool, repeat: Repeat },
    Toast(Toast),
    PlayFailure(Option<PlayFailure>),
    Library(Option<LibraryProblem>),
    Nudge(String),
    Capture(String),
    Import(ImportProgress),
    Stream(StreamInfo),
    Desktop(String),
    TrayReady(bool),
    TrayAction(String),
}

pub type Ui = Arc<dyn Fn(UiEvent) + Send + Sync>;
