use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::jq::{Jq, JqError};
use crate::message::{Format, Freshness, Message};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusKind {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub profile_id: Option<String>,
    #[serde(default)]
    pub epoch: u64,
    pub status: StatusKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub broker: String,
}

impl SessionStatus {
    pub fn idle() -> Self {
        Self {
            profile_id: None,
            epoch: 0,
            status: StatusKind::Disconnected,
            error: None,
            broker: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    pub profile_id: String,
    #[serde(default)]
    pub epoch: u64,
    pub topics: u64,
    pub messages_total: u64,
    pub messages_per_sec: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNodeDto {
    pub segment: String,
    pub path: String,
    pub child_count: usize,
    pub has_payload: bool,
    pub retain: bool,
    pub freshness: Freshness,
    pub format: Format,
    pub last_ms: u64,
    pub history_len: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeBatch {
    #[serde(default)]
    pub profile_id: String,
    #[serde(default)]
    pub epoch: u64,
    pub upserts: Vec<TreeNodeDto>,
    pub deletes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDto {
    #[serde(default)]
    pub epoch: u64,
    pub topic: String,
    pub payload_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_json: Option<Value>,
    pub format: Format,
    pub retain: bool,
    pub qos: u8,
    pub timestamp: u64,
    pub size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryMeta {
    pub count: usize,
    pub latest_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItemDto {
    pub index: usize,
    pub timestamp: u64,
    pub format: Format,
    pub retain: bool,
    pub qos: u8,
    pub size: usize,
}

impl HistoryItemDto {
    pub fn from_message(index: usize, message: &Message) -> Self {
        Self {
            index,
            timestamp: system_time_ms(message.inbound.timestamp),
            format: message.format,
            retain: message.inbound.retain,
            qos: message.inbound.qos.as_u8(),
            size: message.inbound.payload.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitDto {
    pub path: String,
    pub highlights: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JqErrorDto {
    pub message: String,
    pub start: usize,
    pub end: usize,
}

impl From<JqError> for JqErrorDto {
    fn from(error: JqError) -> Self {
        Self {
            message: error.message,
            start: error.span.start,
            end: error.span.end,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JqApplyResult {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
    pub errors: Vec<JqErrorDto>,
}

impl JqApplyResult {
    pub fn from_message(message: &Message, filter: &str) -> Self {
        let Ok(value) = message.data.as_ref() else {
            return Self {
                text: String::new(),
                json: None,
                errors: vec![JqErrorDto {
                    message: "payload is not valid JSON".into(),
                    start: 0,
                    end: 0,
                }],
            };
        };
        match Jq::run(filter, value) {
            Ok(values) => Self::from_values(values),
            Err(errors) => Self {
                text: String::new(),
                json: None,
                errors: errors.into_iter().map(JqErrorDto::from).collect(),
            },
        }
    }

    fn from_values(mut values: Vec<Value>) -> Self {
        if values.len() == 1 {
            let json = values.pop();
            let text = json.as_ref().map(pretty_json).unwrap_or_default();
            return Self {
                text,
                json,
                errors: Vec::new(),
            };
        }
        Self {
            text: values
                .iter()
                .map(pretty_json)
                .collect::<Vec<_>>()
                .join("\n"),
            json: None,
            errors: Vec::new(),
        }
    }

    pub fn should_commit(&self) -> bool {
        self.errors.is_empty() && (self.json.is_some() || !self.text.is_empty())
    }
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

impl MessageDto {
    pub fn from_message(message: &Message) -> Self {
        let payload_json = message.data.as_ref().ok().cloned();
        // Text is a successful UTF-8 decode; only binary gets an inspector error.
        let error = match message.format {
            Format::Binary => message.data.as_ref().err().cloned(),
            Format::Json | Format::Text => None,
        };
        Self {
            epoch: 0,
            topic: message.inbound.topic.clone(),
            payload_text: message.text.clone(),
            payload_json,
            format: message.format,
            retain: message.inbound.retain,
            qos: message.inbound.qos.as_u8(),
            timestamp: system_time_ms(message.inbound.timestamp),
            size: message.inbound.payload.len(),
            error,
        }
    }
}

pub fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::message::{Inbound, QoS, decode_inbound};

    #[test]
    fn message_dto_maps_json() {
        let message = decode_inbound(Inbound {
            topic: "home/lamp".into(),
            payload: Bytes::from_static(br#"{"on":true}"#),
            retain: true,
            qos: QoS::AtLeastOnce,
            timestamp: UNIX_EPOCH,
        });
        let dto = MessageDto::from_message(&message);
        assert_eq!(dto.topic, "home/lamp");
        assert_eq!(dto.format, Format::Json);
        assert_eq!(dto.qos, 1);
        assert_eq!(dto.size, br#"{"on":true}"#.len());
        assert!(dto.payload_json.is_some());
        assert!(dto.error.is_none());
        assert!(dto.payload_text.contains('\n'));
    }

    #[test]
    fn message_dto_text_has_no_error() {
        let message = decode_inbound(Inbound {
            topic: "t".into(),
            payload: Bytes::from_static(b"hello"),
            retain: false,
            qos: QoS::AtMostOnce,
            timestamp: UNIX_EPOCH,
        });
        let dto = MessageDto::from_message(&message);
        assert_eq!(dto.format, Format::Text);
        assert_eq!(dto.payload_text, "hello");
        assert!(dto.error.is_none());
    }
}
