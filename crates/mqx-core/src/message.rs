use std::{
    fmt::{Display, Formatter},
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use serde::Serialize;
use serde_json::Value;

/// MQTT quality of service. Matches rumqttc's numbering so live ingress can map 1:1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum QoS {
    #[default]
    AtMostOnce = 0,
    AtLeastOnce = 1,
    ExactlyOnce = 2,
}

impl QoS {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::AtMostOnce),
            1 => Some(Self::AtLeastOnce),
            2 => Some(Self::ExactlyOnce),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Broker-arrival event: topic, raw payload, and MQTT metadata.
#[derive(Clone, Debug)]
pub struct Inbound {
    pub topic: String,
    pub payload: Bytes,
    pub retain: bool,
    pub qos: QoS,
    pub timestamp: SystemTime,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Json,
    #[default]
    Text,
    Binary,
}

impl Display for Format {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json => write!(f, "JSON"),
            Self::Text => write!(f, "Text"),
            Self::Binary => write!(f, "Binary"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    Retain,
    Fresh,
    Intime,
    Stale,
}

impl Freshness {
    pub fn classify(
        retain: bool,
        timestamp: SystemTime,
        now: SystemTime,
        fresh_until: Duration,
        stale_after: Duration,
    ) -> Self {
        if retain {
            return Self::Retain;
        }
        let age = now.duration_since(timestamp).unwrap_or_default();
        if age < fresh_until {
            Self::Fresh
        } else if age < stale_after {
            Self::Intime
        } else {
            Self::Stale
        }
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub inbound: Inbound,
    pub data: Result<Value, String>,
    pub format: Format,
    pub text: String,
}

impl Message {
    pub fn freshness(
        &self,
        now: SystemTime,
        fresh_until: Duration,
        stale_after: Duration,
    ) -> Freshness {
        Freshness::classify(
            self.inbound.retain,
            self.inbound.timestamp,
            now,
            fresh_until,
            stale_after,
        )
    }

    pub fn freshness_now(&self, fresh_until: Duration, stale_after: Duration) -> Freshness {
        self.freshness(SystemTime::now(), fresh_until, stale_after)
    }
}

/// Decode raw inbound bytes. Safe to call from a worker thread.
pub fn decode_inbound(inbound: Inbound) -> Message {
    if let Ok(text) = std::str::from_utf8(&inbound.payload) {
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.to_string());
            return Message {
                inbound,
                data: Ok(value),
                format: Format::Json,
                text: pretty,
            };
        }
        let text = text.to_string();
        return Message {
            inbound,
            data: Err("payload is not valid JSON".into()),
            format: Format::Text,
            text,
        };
    }

    Message {
        inbound,
        data: Err("payload is not valid UTF-8".into()),
        format: Format::Binary,
        text: "<binary>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound(payload: &[u8]) -> Inbound {
        Inbound {
            topic: "t".into(),
            payload: Bytes::copy_from_slice(payload),
            retain: false,
            qos: QoS::AtMostOnce,
            timestamp: SystemTime::now(),
        }
    }

    #[test]
    fn decode_json_pretty_prints() {
        let message = decode_inbound(inbound(br#"{"a":1}"#));
        assert_eq!(message.format, Format::Json);
        assert_eq!(message.data.unwrap()["a"], 1);
        assert!(message.text.contains('\n'), "expected pretty JSON");
        assert_eq!(message.inbound.payload.as_ref(), br#"{"a":1}"#);
    }

    #[test]
    fn decode_text_utf8() {
        let message = decode_inbound(inbound(b"hello"));
        assert_eq!(message.format, Format::Text);
        assert_eq!(message.text, "hello");
        assert!(message.data.is_err());
    }

    #[test]
    fn decode_binary() {
        let message = decode_inbound(inbound(&[0xff, 0xfe]));
        assert_eq!(message.format, Format::Binary);
        assert_eq!(message.text, "<binary>");
        assert!(message.data.is_err());
    }

    #[test]
    fn freshness_thresholds() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let fresh_until = Duration::from_millis(500);
        let stale_after = Duration::from_secs(5);

        assert_eq!(
            Freshness::classify(true, now, now, fresh_until, stale_after),
            Freshness::Retain
        );
        assert_eq!(
            Freshness::classify(
                false,
                now - Duration::from_millis(100),
                now,
                fresh_until,
                stale_after
            ),
            Freshness::Fresh
        );
        assert_eq!(
            Freshness::classify(
                false,
                now - Duration::from_secs(2),
                now,
                fresh_until,
                stale_after
            ),
            Freshness::Intime
        );
        assert_eq!(
            Freshness::classify(
                false,
                now - Duration::from_secs(6),
                now,
                fresh_until,
                stale_after
            ),
            Freshness::Stale
        );
    }
}
