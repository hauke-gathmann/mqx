//! UI-agnostic MQTT explorer core: topic tree, JSON decode, jq, and connection profiles.

mod config;
mod error;
mod jq;
mod message;
mod profiles;
mod record;
mod session;
mod tree;

pub use config::{
    AppConfig, AppDirs, DEFAULT_RAM_LIMIT_BYTES, KeyConfig, RAM_LIMIT_MAX_BYTES,
    RAM_LIMIT_MIN_BYTES, RecordConfig, UiConfig, clamp_ram_limit_bytes,
};
pub use error::{Error, Result};
pub use jq::{Jq, JqError, JqHistory};
pub use message::{Format, Freshness, Inbound, Message, QoS, decode_inbound};
pub use profiles::{
    ConnectionProfile, LastWill, ProfileStore, ProfileSummary, ProfileView, Protocol,
    SessionConfig, Subscription, TlsConfig,
};
pub use record::{
    MAX_LINE_BYTES, MAX_RECORDING_BYTES, RECORDER_QUEUE, RECORDING_KIND, RecordEvent, RecordStatus,
    Recorder, RecordingHeader, RecordingInfo, RecordingLine, RecordingScanCache, StoppedRecording,
    check_recording_size, discard_recording, inspect_recording, list_recording_files,
    list_recordings, load_recording, load_recording_from, load_replay_events, save_recording,
    sniff_line, unix_ms_to_rfc3339, validate_recording_name, wait_duration,
};
pub use session::{
    ApplyResult, HistoryItemDto, HistoryMeta, JqApplyResult, JqErrorDto, LiveHandle, MessageDto,
    PlaybackProgress, PlaybackState, ProfileId, RecordingContext, ReplayJob, SearchHitDto, Session,
    SessionEvent, SessionStats, SessionStatus, Source, Status, StatusKind, Sub, TreeBatch,
    TreeNodeDto, assert_replay_target, broker_display, run_replay,
};
pub use tree::{Leaf, Node, SearchHit, SearchMode, TopicTree, UpsertOutcome};
