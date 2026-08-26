use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const RECORDING_KIND: &str = "mqx-recording";
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingHeader {
    pub kind: String,
    pub v: u32,
    pub started_at: String,
    pub ended_at: String,
    pub profile_id: String,
    pub profile_name: String,
    pub broker: String,
    pub messages: u64,
    pub topics: u64,
    pub app_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordEvent {
    pub t_ms: u64,
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
    #[serde(default)]
    pub dup: bool,
    pub payload: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordingLine {
    Header(RecordingHeader),
    Event(RecordEvent),
}

impl RecordingHeader {
    pub fn to_jsonl(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|source| Error::Recording(source.to_string()))
    }

    pub fn from_jsonl(line: &str) -> Result<Self> {
        let line = line.trim();
        if line.is_empty() {
            return Err(Error::Recording("empty recording header".into()));
        }
        serde_json::from_str(line).map_err(|source| Error::Recording(source.to_string()))
    }
}

impl RecordEvent {
    pub fn payload_bytes(&self) -> Result<Vec<u8>> {
        BASE64
            .decode(self.payload.as_bytes())
            .map_err(|_| Error::Recording("recording payload is not valid base64".into()))
    }

    pub fn to_jsonl(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|source| Error::Recording(source.to_string()))
    }

    pub fn from_jsonl(line: &str) -> Result<Self> {
        let line = line.trim();
        if line.is_empty() {
            return Err(Error::Recording("empty recording line".into()));
        }
        serde_json::from_str(line).map_err(|source| Error::Recording(source.to_string()))
    }
}

/// Header objects have top-level `kind`; mqtt-trace events do not.
pub fn sniff_line(line: &str) -> Result<RecordingLine> {
    let line = line.trim();
    if line.is_empty() {
        return Err(Error::Recording("empty recording line".into()));
    }
    if line_has_kind(line) {
        RecordingHeader::from_jsonl(line).map(RecordingLine::Header)
    } else {
        RecordEvent::from_jsonl(line).map(RecordingLine::Event)
    }
}

pub fn load_recording(
    path: impl AsRef<Path>,
) -> Result<(Option<RecordingHeader>, Vec<RecordEvent>)> {
    let path = path.as_ref();
    let file = std::fs::File::open(path)?;
    load_recording_from(BufReader::new(file)).map_err(|err| err.with_recording_path(path))
}

pub fn load_recording_from(
    mut reader: impl BufRead,
) -> Result<(Option<RecordingHeader>, Vec<RecordEvent>)> {
    let mut header = None;
    let mut events = Vec::new();
    let mut first_nonempty = true;
    let mut line_no = 0usize;
    while let Some(line) = read_line_capped(&mut reader, MAX_LINE_BYTES)
        .map_err(|err| err.with_recording_line(line_no + 1))?
    {
        line_no += 1;
        if line.trim().is_empty() {
            continue;
        }
        if first_nonempty {
            first_nonempty = false;
            match sniff_line(&line).map_err(|err| err.with_recording_line(line_no))? {
                RecordingLine::Header(parsed) => header = Some(parsed),
                RecordingLine::Event(event) => events.push(event),
            }
        } else {
            events.push(
                RecordEvent::from_jsonl(&line).map_err(|err| err.with_recording_line(line_no))?,
            );
        }
    }
    Ok((header, events))
}

pub fn wait_duration(t0_ms: u64, t_ms: u64, speed: f64, elapsed: Duration) -> Duration {
    if !speed.is_finite() || speed <= 0.0 {
        return Duration::ZERO;
    }
    let recorded = Duration::from_millis(t_ms.saturating_sub(t0_ms));
    let scaled = recorded.div_f64(speed);
    scaled.saturating_sub(elapsed)
}

fn line_has_kind(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line.trim())
        .ok()
        .is_some_and(|value| value.get("kind").is_some())
}

fn read_line_capped(reader: &mut impl BufRead, max: usize) -> Result<Option<String>> {
    let mut buf = Vec::new();
    // +2 so a max-length line ending in `\r\n` is not rejected.
    let n = reader
        .by_ref()
        .take(max as u64 + 2)
        .read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    if buf.len() > max {
        return Err(Error::Recording("line exceeds 16 MiB".into()));
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| Error::Recording("line is not valid UTF-8".into()))
}

impl Error {
    fn with_recording_path(self, path: &Path) -> Self {
        match self {
            Self::Recording(msg) => Self::Recording(format!("{}: {msg}", path.display())),
            other => other,
        }
    }

    fn with_recording_line(self, line: usize) -> Self {
        match self {
            Self::Recording(msg) => Self::Recording(format!("line {line}: {msg}")),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn binary_payload_roundtrip() {
        let event = RecordEvent {
            t_ms: 1_700_000_000_123,
            topic: "home/lamp".into(),
            qos: 1,
            retain: true,
            dup: false,
            payload: BASE64.encode([0_u8, 1, 2, 255]),
        };
        let line = event.to_jsonl().unwrap();
        let parsed = RecordEvent::from_jsonl(&line).unwrap();
        assert_eq!(parsed, event);
        assert_eq!(parsed.payload_bytes().unwrap(), vec![0, 1, 2, 255]);
    }

    #[test]
    fn header_camel_case_fixture() {
        let (header, events) = load_recording(fixture("header.jsonl")).unwrap();
        let header = header.expect("header");
        assert_eq!(header.kind, RECORDING_KIND);
        assert_eq!(header.v, 1);
        assert_eq!(header.started_at, "2026-08-26T12:00:00.000Z");
        assert_eq!(header.ended_at, "2026-08-26T12:00:32.100Z");
        assert_eq!(header.profile_id, "abc");
        assert_eq!(header.profile_name, "Home Assistant");
        assert_eq!(header.broker, "mqtts://ha.local:8883");
        assert_eq!(header.messages, 2);
        assert_eq!(header.topics, 1);
        assert_eq!(header.app_version, "0.2.0");
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload_bytes().unwrap(), vec![0, 1, 2, 255]);

        let json = header.to_jsonl().unwrap();
        for key in [
            "startedAt",
            "endedAt",
            "profileId",
            "profileName",
            "appVersion",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        for key in [
            "started_at",
            "ended_at",
            "profile_id",
            "profile_name",
            "app_version",
        ] {
            assert!(!json.contains(key), "snake_case leaked: {json}");
        }
    }

    #[test]
    fn headerless_parse() {
        let (header, events) = load_recording(fixture("headerless.jsonl")).unwrap();
        assert!(header.is_none());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, "home/lamp");
        assert!(!events[0].dup);
        assert_eq!(events[0].payload_bytes().unwrap(), b"{\"on\":true}");
    }

    #[test]
    fn sniff_kind_is_header() {
        let line = r#"{"kind":"mqx-recording","v":1,"startedAt":"2026-08-26T12:00:00.000Z","endedAt":"2026-08-26T12:00:32.100Z","profileId":"p","profileName":"n","broker":"mqtt://localhost:1883","messages":0,"topics":0,"appVersion":"0.2.0"}"#;
        assert!(matches!(
            sniff_line(line).unwrap(),
            RecordingLine::Header(_)
        ));
        let event = r#"{"t_ms":1,"topic":"t","qos":0,"retain":false,"payload":""}"#;
        assert!(matches!(
            sniff_line(event).unwrap(),
            RecordingLine::Event(_)
        ));
    }

    #[test]
    fn realtime_waits_recorded_gap() {
        let wait = wait_duration(1_000, 1_250, 1.0, Duration::from_millis(50));
        assert_eq!(wait, Duration::from_millis(200));
    }

    #[test]
    fn double_speed_halves_wait() {
        let wait = wait_duration(0, 1_000, 2.0, Duration::ZERO);
        assert_eq!(wait, Duration::from_millis(500));
    }

    #[test]
    fn max_speed_does_not_wait() {
        assert_eq!(wait_duration(0, 5_000, 0.0, Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn late_clock_does_not_sleep() {
        assert_eq!(
            wait_duration(0, 100, 1.0, Duration::from_millis(150)),
            Duration::ZERO
        );
    }

    #[test]
    fn capped_line_rejects_over_max() {
        let mut reader = BufReader::new(&b"abcdef\n"[..]);
        let err = read_line_capped(&mut reader, 4).unwrap_err();
        assert!(matches!(err, Error::Recording(_)));
    }

    #[test]
    fn capped_line_accepts_crlf_at_max() {
        let mut reader = BufReader::new(&b"abcd\r\n"[..]);
        let line = read_line_capped(&mut reader, 4).unwrap().unwrap();
        assert_eq!(line, "abcd");
    }

    #[test]
    fn capped_line_rejects_over_max_crlf() {
        let mut reader = BufReader::new(&b"abcde\r\n"[..]);
        let err = read_line_capped(&mut reader, 4).unwrap_err();
        assert!(matches!(err, Error::Recording(_)));
    }

    #[test]
    fn load_recording_skips_blank_lines() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(
            concat!(
                "\n",
                r#"{"kind":"mqx-recording","v":1,"startedAt":"2026-08-26T12:00:00.000Z","endedAt":"2026-08-26T12:00:01.000Z","profileId":"p","profileName":"n","broker":"mqtt://localhost:1883","messages":1,"topics":1,"appVersion":"0.2.0"}"#,
                "\n\n",
                r#"{"t_ms":1,"topic":"t","qos":0,"retain":false,"payload":"AA=="}"#,
                "\n",
            )
            .as_bytes(),
        )
        .unwrap();
        tmp.flush().unwrap();
        let (header, events) = load_recording(tmp.path()).unwrap();
        assert!(header.is_some());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, "t");
    }
}
