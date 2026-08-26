//! In-process **Dory** realtime dataflow used by the Windows and Linux desktop apps.
//!
//! The graph follows the same idea as DORA (dataflow-oriented realtime architecture):
//! typed stages, a directed pipeline, and a broadcast bus. Audio never leaves the
//! machine. Android talks to this graph through the LAN API, it does not run the model.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::state::{AppStateEnum, InjectionFeedback, StreamingTranscriptPayload};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Capture,
    Resample,
    Vad,
    Asr,
    Format,
    Inject,
    History,
    Api,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    None,
    Microphone,
    External,
}

#[derive(Debug, Clone)]
pub enum DoryEvent {
    State(AppStateEnum),
    Partial(StreamingTranscriptPayload),
    AudioLevel(f32),
    Final(StreamingTranscriptPayload),
    Injection(InjectionFeedback),
    Error(String),
    AutoStop,
    Stage(Stage),
}

#[derive(Clone)]
pub struct DoryBus {
    tx: broadcast::Sender<DoryEvent>,
}

impl DoryBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn emit(&self, event: DoryEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DoryEvent> {
        self.tx.subscribe()
    }
}

impl Default for DoryBus {
    fn default() -> Self {
        Self::new()
    }
}

/// YAML-equivalent graph used by the desktop dictation loop.
pub const PIPELINE: &[&str] = &[
    "capture",
    "resample",
    "vad",
    "asr",
    "format",
    "inject",
    "history",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_order_is_stable() {
        assert_eq!(PIPELINE[0], "capture");
        assert_eq!(PIPELINE[3], "asr");
        assert_eq!(PIPELINE[5], "inject");
    }
}
