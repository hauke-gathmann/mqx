//! UI-agnostic MQTT explorer core: topic tree, JSON decode, jq, and connection profiles.

mod config;
mod error;
mod jq;
mod message;
mod profiles;
mod session;
mod tree;

pub use config::{AppConfig, AppDirs, KeyConfig, UiConfig};
pub use error::{Error, Result};
pub use jq::{Jq, JqError, JqHistory};
pub use message::{Format, Freshness, Inbound, Message, QoS, decode_inbound};
pub use profiles::{
    ConnectionProfile, LastWill, ProfileStore, ProfileSummary, ProfileView, Protocol,
    SessionConfig, Subscription, TlsConfig,
};
pub use session::{
    ApplyResult, HistoryItemDto, HistoryMeta, JqApplyResult, JqErrorDto, LiveHandle, MessageDto,
    ProfileId, SearchHitDto, Session, SessionEvent, SessionStats, SessionStatus, Source, Status,
    StatusKind, Sub, TreeBatch, TreeNodeDto, broker_display,
};
pub use tree::{Leaf, Node, SearchHit, SearchMode, TopicTree, UpsertOutcome};
