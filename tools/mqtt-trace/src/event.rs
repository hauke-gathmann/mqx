use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rumqttc::Publish;
use serde::{Deserialize, Serialize};

use crate::broker::qos_to_u8;

/// One captured MQTT publish. `t_ms` is Unix time in milliseconds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub t_ms: u64,
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
    #[serde(default)]
    pub dup: bool,
    pub payload: String,
}

impl TraceEvent {
    pub fn from_publish(publish: &Publish, t_ms: u64) -> Self {
        Self {
            t_ms,
            topic: publish.topic.clone(),
            qos: qos_to_u8(publish.qos),
            retain: publish.retain,
            dup: publish.dup,
            payload: BASE64.encode(publish.payload.as_ref()),
        }
    }

    pub fn payload_bytes(&self) -> Result<Vec<u8>> {
        BASE64
            .decode(self.payload.as_bytes())
            .context("trace payload is not valid base64")
    }

    pub fn to_jsonl(&self) -> Result<String> {
        serde_json::to_string(self).context("serialize trace event")
    }

    pub fn from_jsonl(line: &str) -> Result<Self> {
        let line = line.trim();
        if line.is_empty() {
            bail!("empty trace line");
        }
        serde_json::from_str(line).context("parse trace event")
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Header objects have top-level `kind`; mqtt-trace events do not.
pub fn is_recording_header(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line.trim())
        .ok()
        .is_some_and(|value| value.get("kind").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_roundtrip_binary_payload() {
        let event = TraceEvent {
            t_ms: 1_700_000_000_123,
            topic: "home/lamp".into(),
            qos: 1,
            retain: true,
            dup: false,
            payload: BASE64.encode([0_u8, 1, 2, 255]),
        };
        let line = event.to_jsonl().unwrap();
        let parsed = TraceEvent::from_jsonl(&line).unwrap();
        assert_eq!(parsed, event);
        assert_eq!(parsed.payload_bytes().unwrap(), vec![0, 1, 2, 255]);
    }

    #[test]
    fn detects_in_app_header_by_kind() {
        assert!(is_recording_header(
            r#"{"kind":"mqx-recording","v":1,"startedAt":"2026-08-26T12:00:00.000Z"}"#
        ));
        assert!(!is_recording_header(
            r#"{"t_ms":1,"topic":"home/lamp","qos":0,"retain":false,"payload":""}"#
        ));
    }
}
