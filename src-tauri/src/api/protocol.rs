use serde::{Deserialize, Serialize};

use crate::state::{LatencyMetrics, StreamingTranscriptPayload};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Start {
        language: Option<String>,
        format: Option<String>,
        sample_rate: Option<u32>,
        inject: Option<bool>,
    },
    Stop,
    Cancel,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Ready,
    Partial {
        committed_prefix: String,
        mutable_suffix: String,
        full_text: String,
        language: String,
        audio_level: f32,
    },
    Final {
        raw: String,
        text: String,
        language: String,
        metrics: LatencyMetrics,
    },
    Error {
        code: String,
        message: String,
    },
}

impl ServerMsg {
    pub fn from_partial(p: StreamingTranscriptPayload) -> Self {
        Self::Partial {
            committed_prefix: p.committed_prefix,
            mutable_suffix: p.mutable_suffix,
            full_text: p.full_text,
            language: p.language,
            audio_level: p.audio_level,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PairRequest {
    pub code: String,
    pub device_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PairResponse {
    pub token: String,
    pub server_name: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct InjectRequest {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub version: String,
    pub model_ready: bool,
    pub os: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ApiStatus {
    pub enabled: bool,
    pub running: bool,
    pub bind: String,
    pub port: u16,
    pub listen_addrs: Vec<String>,
    pub pairing_code: Option<String>,
    pub pairing_expires_in_sec: Option<u64>,
    pub qr_svg: Option<String>,
    pub pair_uri: Option<String>,
    pub devices: Vec<crate::pairing::PairedDevicePublic>,
    pub warning: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_start_deserializes() {
        let msg: ClientMsg = serde_json::from_str(
            r#"{"type":"start","language":"auto","format":"pcm_s16le","sample_rate":16000,"inject":false}"#,
        )
        .unwrap();
        match msg {
            ClientMsg::Start { sample_rate, inject, .. } => {
                assert_eq!(sample_rate, Some(16000));
                assert_eq!(inject, Some(false));
            }
            _ => panic!("expected start"),
        }
    }
}
