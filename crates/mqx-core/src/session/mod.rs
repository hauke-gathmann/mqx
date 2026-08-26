mod dto;
mod live;
mod tls;

use std::time::{Duration, Instant, SystemTime};

use rumqttc::AsyncClient;

pub use dto::{
    HistoryItemDto, HistoryMeta, JqApplyResult, JqErrorDto, MessageDto, SearchHitDto, SessionStats,
    SessionStatus, StatusKind, TreeBatch, TreeNodeDto,
};
pub use live::{LiveHandle, SessionEvent};
pub use tls::broker_display;

use crate::{
    config::UiConfig,
    message::{Format, Freshness, Message},
    profiles::{ConnectionProfile, Subscription},
    tree::{SearchMode, TopicTree, UpsertOutcome},
};

pub type ProfileId = String;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected,
    Detached { since: Instant },
    Reconnecting { since: Instant },
    Disconnected,
    Error { msg: String },
}

impl Status {
    pub fn kind(&self) -> StatusKind {
        match self {
            Self::Connecting => StatusKind::Connecting,
            Self::Connected => StatusKind::Connected,
            Self::Detached { .. } => StatusKind::Detached,
            Self::Reconnecting { .. } => StatusKind::Reconnecting,
            Self::Disconnected => StatusKind::Disconnected,
            Self::Error { .. } => StatusKind::Error,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Sub {
    pub topic: String,
    pub qos: rumqttc::QoS,
}

/// Live MQTT today; playback is a v2 source that will share `TopicTree::upsert`.
pub enum Source {
    Live,
    #[allow(dead_code)]
    Playback,
}

pub struct Session {
    pub id: ProfileId,
    pub status: Status,
    /// Kept for v2 publish; subscribe/disconnect use it now.
    client: AsyncClient,
    tree: TopicTree,
    selected: Option<String>,
    subscriptions: Vec<Sub>,
    source: Source,
    broker: String,
    error: Option<String>,
    messages_total: u64,
    epoch: u64,
    last_rate: f64,
    fresh_until: Duration,
    stale_after: Duration,
    /// Connected ⇒ true, Detached ⇒ false. ConnAck does not flip this.
    pub ingest_enabled: bool,
}

pub struct ApplyResult {
    pub topic_message: Option<MessageDto>,
    pub upserts: Vec<TreeNodeDto>,
    pub deletes: Vec<String>,
}

impl Session {
    pub fn new(
        profile: &ConnectionProfile,
        client: AsyncClient,
        ui: &UiConfig,
        epoch: u64,
    ) -> Self {
        let subscriptions = if profile.subscriptions.is_empty() {
            vec![Sub {
                topic: Subscription::default().topic,
                qos: rumqttc::QoS::AtMostOnce,
            }]
        } else {
            profile
                .subscriptions
                .iter()
                .map(|sub| Sub {
                    topic: sub.topic.clone(),
                    qos: tls::qos_from_u8(sub.qos),
                })
                .collect()
        };

        Self {
            id: profile.id.clone(),
            status: Status::Connecting,
            client,
            tree: TopicTree::with_limits(ui.buffer_size, ui.ram_limit_bytes),
            selected: None,
            subscriptions,
            source: Source::Live,
            broker: broker_display(profile),
            error: None,
            messages_total: 0,
            epoch,
            last_rate: 0.0,
            fresh_until: ui.fresh_until,
            stale_after: ui.stale_after,
            ingest_enabled: true,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn client(&self) -> &AsyncClient {
        &self.client
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn subscriptions(&self) -> &[Sub] {
        &self.subscriptions
    }

    pub fn broker(&self) -> &str {
        &self.broker
    }

    pub fn select_topic(&mut self, topic: Option<String>) {
        self.selected = topic.filter(|t| !t.is_empty());
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn ram_exhausted(&self) -> bool {
        self.tree.ram_exhausted()
    }

    pub fn has_leaf(&self, topic: &str) -> bool {
        self.tree.get(topic).is_some()
    }

    pub fn set_ram_limit(&mut self, bytes: u64) -> Vec<TreeNodeDto> {
        self.tree.set_ram_limit(bytes);
        self.take_evicted_upserts()
    }

    pub fn set_status(&mut self, status: Status) {
        if let Status::Error { msg } = &status {
            self.error = Some(msg.clone());
        } else if matches!(
            status,
            Status::Connected | Status::Connecting | Status::Detached { .. }
        ) {
            self.error = None;
        }
        self.status = status;
    }

    /// Freeze or resume topic-tree ingest. Does not disconnect.
    /// Connecting/reconnecting keep their status; ConnAck applies this flag.
    pub fn set_ingest(&mut self, enabled: bool) {
        self.ingest_enabled = enabled;
        match self.status {
            Status::Connected if !enabled => {
                self.set_status(Status::Detached {
                    since: Instant::now(),
                });
            }
            Status::Detached { .. } if enabled => {
                self.set_status(Status::Connected);
            }
            _ => {}
        }
    }

    pub fn on_connack(&mut self) {
        if self.ingest_enabled {
            self.set_status(Status::Connected);
        } else {
            self.set_status(Status::Detached {
                since: Instant::now(),
            });
        }
    }

    pub fn set_io_error(&mut self, message: String) {
        self.error = Some(message);
        self.status = Status::Reconnecting {
            since: Instant::now(),
        };
    }

    pub fn set_subscribe_error(&mut self, message: String) {
        self.error = Some(message);
    }

    pub fn status_event(&self) -> SessionStatus {
        let error = match &self.status {
            Status::Error { msg } => Some(msg.clone()),
            _ => self.error.clone(),
        };
        SessionStatus {
            profile_id: Some(self.id.clone()),
            epoch: self.epoch,
            status: self.status.kind(),
            error,
            broker: self.broker.clone(),
            ram_exhausted: self.tree.ram_exhausted(),
        }
    }

    pub fn stats(&mut self, messages_per_sec: f64) -> SessionStats {
        self.last_rate = messages_per_sec;
        self.stats_snapshot()
    }

    pub fn stats_snapshot(&self) -> SessionStats {
        SessionStats {
            profile_id: self.id.clone(),
            epoch: self.epoch,
            topics: self.tree.topic_count() as u64,
            messages_total: self.messages_total,
            messages_per_sec: self.last_rate,
            stored_bytes: self.tree.stored_bytes(),
            ram_limit_bytes: self.tree.ram_limit(),
        }
    }

    pub fn tree_children(&self, path: &[String]) -> Vec<TreeNodeDto> {
        let joined = path.join("/");
        let Some(node) = self.tree.node(&joined) else {
            return Vec::new();
        };
        node.children
            .values()
            .map(|child| {
                let child_path = if joined.is_empty() {
                    child.segment.clone()
                } else {
                    format!("{joined}/{}", child.segment)
                };
                self.node_to_dto(&child_path, child)
            })
            .collect()
    }

    pub fn get_message(&self, topic: &str, index: Option<usize>) -> Option<MessageDto> {
        self.tree
            .get_message(topic, index)
            .map(MessageDto::from_message)
    }

    pub fn get_history_meta(&self, topic: &str) -> Option<HistoryMeta> {
        let leaf = self.tree.get(topic)?;
        Some(HistoryMeta {
            count: leaf.message_count(),
            latest_index: leaf.latest.seq as usize,
        })
    }

    pub fn list_history(&self, topic: &str) -> Option<Vec<HistoryItemDto>> {
        let leaf = self.tree.get(topic)?;
        let mut items: Vec<HistoryItemDto> = leaf
            .history
            .iter()
            .map(HistoryItemDto::from_message)
            .collect();
        items.push(HistoryItemDto::from_message(&leaf.latest));
        Some(items)
    }

    pub fn tree_search(&self, query: &str, mode: SearchMode) -> Vec<SearchHitDto> {
        self.tree
            .search(query, mode)
            .into_iter()
            .map(|hit| SearchHitDto {
                path: hit.path,
                highlights: hit.highlights,
            })
            .collect()
    }

    pub fn apply_jq(
        &self,
        topic: &str,
        index: Option<usize>,
        filter: &str,
    ) -> Option<JqApplyResult> {
        self.tree
            .get_message(topic, index)
            .map(|message| JqApplyResult::from_message(message, filter))
    }

    /// Only write path for live (and later playback) ingress.
    pub fn ingest(&mut self, message: Message) -> ApplyResult {
        if !self.ingest_enabled {
            return ApplyResult {
                topic_message: None,
                upserts: Vec::new(),
                deletes: Vec::new(),
            };
        }
        let topic = message.inbound.topic.clone();
        // WHY: empty retain is a delete; do not stream it as the selected payload.
        let retain_clear = message.inbound.retain && message.inbound.payload.is_empty();
        let topic_message = if retain_clear {
            None
        } else {
            self.selected
                .as_deref()
                .filter(|selected| *selected == topic)
                .map(|_| {
                    let mut dto = MessageDto::from_message(&message);
                    dto.epoch = self.epoch;
                    dto
                })
        };

        self.messages_total = self.messages_total.saturating_add(1);
        let outcome = self.tree.upsert(message);

        let mut upserts = Vec::new();
        let mut deletes = Vec::new();
        let mut emitted = topic_message;
        match outcome {
            UpsertOutcome::Inserted => {
                upserts.extend(self.collect_path_dtos(&topic));
            }
            UpsertOutcome::Updated => {
                if let Some(dto) = self.node_dto(&topic) {
                    upserts.push(dto);
                }
            }
            UpsertOutcome::Ignored => {
                emitted = None;
            }
            UpsertOutcome::Deleted { pruned } => {
                deletes = pruned;
                if self.tree.node(&topic).is_some() {
                    upserts.extend(self.collect_path_dtos(&topic));
                } else if let Some(parent) = parent_path(&topic)
                    && self.tree.node(parent).is_some()
                {
                    upserts.extend(self.collect_path_dtos(parent));
                }
            }
        }

        for dto in self.take_evicted_upserts() {
            if !upserts.iter().any(|existing| existing.path == dto.path) {
                upserts.push(dto);
            }
        }

        ApplyResult {
            topic_message: emitted,
            upserts,
            deletes,
        }
    }

    pub fn clear_tree(&mut self) {
        self.tree = TopicTree::with_limits(self.tree.buffer_size(), self.tree.ram_limit());
        self.selected = None;
        self.messages_total = 0;
        self.last_rate = 0.0;
    }

    fn take_evicted_upserts(&mut self) -> Vec<TreeNodeDto> {
        self.tree
            .take_evicted()
            .into_iter()
            .filter_map(|topic| self.node_dto(&topic))
            .collect()
    }

    fn collect_path_dtos(&self, topic: &str) -> Vec<TreeNodeDto> {
        if topic.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut acc = String::new();
        for segment in topic.split('/') {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(segment);
            if let Some(dto) = self.node_dto(&acc) {
                out.push(dto);
            }
        }
        out
    }

    fn node_dto(&self, path: &str) -> Option<TreeNodeDto> {
        self.tree
            .node(path)
            .map(|node| self.node_to_dto(path, node))
    }

    fn node_to_dto(&self, path: &str, node: &crate::tree::Node) -> TreeNodeDto {
        let now = SystemTime::now();
        let segment = if path.is_empty() {
            String::new()
        } else {
            path.rsplit('/').next().unwrap_or(path).to_string()
        };
        match &node.leaf {
            Some(leaf) => TreeNodeDto {
                segment,
                path: path.to_string(),
                child_count: node.children.len(),
                has_payload: true,
                retain: leaf.latest.inbound.retain,
                freshness: leaf
                    .latest
                    .freshness(now, self.fresh_until, self.stale_after),
                format: leaf.latest.format,
                last_ms: dto::system_time_ms(leaf.latest.inbound.timestamp),
                history_len: leaf.message_count(),
            },
            None => TreeNodeDto {
                segment,
                path: path.to_string(),
                child_count: node.children.len(),
                has_payload: false,
                retain: false,
                freshness: Freshness::Stale,
                format: Format::Text,
                last_ms: 0,
                history_len: 0,
            },
        }
    }
}

fn parent_path(topic: &str) -> Option<&str> {
    topic.rfind('/').map(|i| &topic[..i])
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use bytes::Bytes;
    use rumqttc::MqttOptions;

    use super::*;
    use crate::{
        message::{Inbound, QoS, decode_inbound},
        profiles::{Protocol, SessionConfig, TlsConfig},
        tree::SearchMode,
    };

    fn ui() -> UiConfig {
        UiConfig::default()
    }

    fn profile() -> ConnectionProfile {
        ConnectionProfile {
            id: "profile-1".into(),
            name: "Local".into(),
            protocol: Protocol::Mqtt,
            host: "localhost".into(),
            port: 1883,
            client_id: "mqx-test".into(),
            username: String::new(),
            tls: TlsConfig::default(),
            session: SessionConfig::default(),
            subscriptions: vec![Subscription {
                topic: "home/#".into(),
                qos: 0,
            }],
            last_will: None,
            websocket_path: None,
        }
    }

    fn session() -> Session {
        let (client, _eventloop) = AsyncClient::new(MqttOptions::new("t", "localhost", 1883), 10);
        Session::new(&profile(), client, &ui(), 1)
    }

    fn msg(topic: &str, payload: &[u8], retain: bool) -> Message {
        decode_inbound(Inbound {
            topic: topic.into(),
            payload: Bytes::copy_from_slice(payload),
            retain,
            qos: QoS::AtMostOnce,
            timestamp: SystemTime::now(),
        })
    }

    #[test]
    fn ingest_upserts_leaf_and_ancestors() {
        let mut session = session();
        let result = session.ingest(msg("home/living/lamp", b"on", false));
        let paths: Vec<_> = result.upserts.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(paths, ["home", "home/living", "home/living/lamp"]);
        assert!(result.deletes.is_empty());
        assert!(result.topic_message.is_none());

        let children = session.tree_children(&[]);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].path, "home");
        assert!(!children[0].has_payload);
        assert_eq!(children[0].child_count, 1);

        let living = session.tree_children(&["home".into()]);
        assert_eq!(living[0].path, "home/living");
    }

    #[test]
    fn ingest_empty_retain_deletes_leaf() {
        let mut session = session();
        session.ingest(msg("home/living/lamp", b"on", false));
        let result = session.ingest(msg("home/living/lamp", b"", true));
        assert_eq!(
            result.deletes,
            vec![
                "home/living/lamp".to_string(),
                "home/living".to_string(),
                "home".to_string()
            ]
        );
        assert!(result.topic_message.is_none());
        assert!(session.tree_children(&[]).is_empty());
    }

    #[test]
    fn ingest_empty_retain_on_parent_upserts_cleared_node() {
        let mut session = session();
        session.ingest(msg("home", b"house", false));
        session.ingest(msg("home/lamp", b"on", false));
        session.select_topic(Some("home".into()));
        let result = session.ingest(msg("home", b"", true));
        assert!(result.deletes.is_empty());
        assert!(result.topic_message.is_none());
        let home = result
            .upserts
            .iter()
            .find(|node| node.path == "home")
            .expect("cleared parent");
        assert!(!home.has_payload);
        assert_eq!(home.child_count, 1);
        assert!(session.get_message("home", None).is_none());
    }

    #[test]
    fn list_history_is_oldest_first_with_latest_last() {
        let mut session = session();
        session.ingest(msg("home/lamp", b"one", false));
        session.ingest(msg("home/lamp", b"two", false));
        session.ingest(msg("home/lamp", b"three", false));
        let items = session.list_history("home/lamp").unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].index, 0);
        assert_eq!(items[1].index, 1);
        assert_eq!(items[2].index, 2);
        assert_eq!(items[2].size, b"three".len());
        assert_eq!(
            session.get_history_meta("home/lamp").unwrap().latest_index,
            2
        );
    }

    #[test]
    fn ingest_selected_emits_message_immediately() {
        let mut session = session();
        session.select_topic(Some("home/lamp".into()));
        let result = session.ingest(msg("home/lamp", br#"{"on":true}"#, false));
        let dto = result.topic_message.expect("selected topic");
        assert_eq!(dto.topic, "home/lamp");
        assert_eq!(dto.format, Format::Json);
        assert_eq!(session.get_history_meta("home/lamp").unwrap().count, 1);
        assert_eq!(
            session.get_message("home/lamp", None).unwrap().payload_text,
            dto.payload_text
        );
    }

    #[test]
    fn default_status_and_source() {
        let session = session();
        assert!(matches!(session.status, Status::Connecting));
        assert!(session.ingest_enabled);
        assert!(matches!(session.source(), Source::Live));
        assert_eq!(session.subscriptions()[0].topic, "home/#");
        assert_eq!(session.broker(), "mqtt://localhost:1883");
        assert_eq!(session.epoch(), 1);
    }

    #[test]
    fn detached_publish_does_not_change_topic_count() {
        let mut session = session();
        session.set_status(Status::Connected);
        session.ingest(msg("home/lamp", b"on", false));
        assert_eq!(session.stats(0.0).topics, 1);

        session.set_ingest(false);
        assert!(matches!(session.status, Status::Detached { .. }));
        assert!(!session.ingest_enabled);
        session.ingest(msg("home/kitchen", b"1", false));
        session.ingest(msg("home/lamp", b"off", false));
        assert_eq!(session.stats(0.0).topics, 1);
        assert_eq!(session.stats(0.0).messages_total, 1);
    }

    #[test]
    fn go_live_subsequent_publish_upserts() {
        let mut session = session();
        session.set_status(Status::Connected);
        session.ingest(msg("home/lamp", b"on", false));
        session.set_ingest(false);
        session.ingest(msg("home/kitchen", b"1", false));
        assert_eq!(session.stats(0.0).topics, 1);

        session.set_ingest(true);
        assert!(matches!(session.status, Status::Connected));
        assert!(session.ingest_enabled);
        session.ingest(msg("home/kitchen", b"1", false));
        assert_eq!(session.stats(0.0).topics, 2);
    }

    #[test]
    fn connack_while_detached_stays_detached() {
        let mut session = session();
        session.set_status(Status::Connected);
        session.set_ingest(false);
        session.set_io_error("connection lost".into());
        assert!(matches!(session.status, Status::Reconnecting { .. }));
        assert!(!session.ingest_enabled);

        session.on_connack();
        assert!(matches!(session.status, Status::Detached { .. }));
        assert!(!session.ingest_enabled);
        assert_eq!(session.status.kind(), StatusKind::Detached);
    }

    #[test]
    fn connack_while_live_is_connected() {
        let mut session = session();
        session.set_io_error("connection lost".into());
        assert!(session.ingest_enabled);
        session.on_connack();
        assert!(matches!(session.status, Status::Connected));
        assert!(session.ingest_enabled);
    }

    #[test]
    fn set_ingest_does_not_disconnect() {
        let mut session = session();
        session.set_status(Status::Connected);
        session.set_ingest(false);
        assert!(!matches!(
            session.status,
            Status::Disconnected | Status::Error { .. }
        ));
        session.set_ingest(true);
        assert!(matches!(session.status, Status::Connected));
    }

    #[test]
    fn tree_search_keep_returns_substring_hit() {
        let mut session = session();
        session.ingest(msg("home/living/lamp", b"1", false));
        session.ingest(msg("home/kitchen/fridge", b"2", false));
        let hits = session.tree_search("lamp", SearchMode::Keep);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "home/living/lamp");
        assert!(!hits[0].highlights.is_empty());
    }

    #[test]
    fn apply_jq_extracts_field() {
        let mut session = session();
        session.ingest(msg("home/lamp", br#"{"bri":180}"#, false));
        let result = session.apply_jq("home/lamp", None, ".bri").unwrap();
        assert!(result.errors.is_empty());
        assert_eq!(result.json, Some(serde_json::json!(180)));
        assert_eq!(result.text, "180");
    }

    #[test]
    fn apply_jq_rejects_text_payload() {
        let mut session = session();
        session.ingest(msg("home/lamp", b"on", false));
        let result = session.apply_jq("home/lamp", None, ".").unwrap();
        assert!(!result.errors.is_empty());
        assert!(result.json.is_none());
    }

    #[test]
    fn apply_jq_compile_error() {
        let mut session = session();
        session.ingest(msg("home/lamp", br#"{"a":1}"#, false));
        let result = session.apply_jq("home/lamp", None, "[").unwrap();
        assert!(!result.errors.is_empty());
        assert!(!result.errors[0].message.is_empty());
    }

    #[test]
    fn apply_jq_runtime_error() {
        let mut session = session();
        session.ingest(msg("home/lamp", b"1", false));
        let result = session.apply_jq("home/lamp", None, ".[]").unwrap();
        assert!(!result.errors.is_empty());
        assert!(!result.should_commit());
    }

    #[test]
    fn ingest_retain_clear_counts_as_broker_message() {
        let mut session = session();
        session.ingest(msg("t", b"1", false));
        session.ingest(msg("t", b"", true));
        // Empty retain is a real MQTT packet; header totals include it.
        assert_eq!(session.stats(0.0).messages_total, 2);
        assert!(session.tree_children(&[]).is_empty());
    }

    #[test]
    fn ingest_skips_when_ram_exhausted() {
        let mut session = session();
        session.ingest(msg("a", b"hello", false));
        session.ingest(msg("b", b"world", false));
        let stored = session.stats(0.0).stored_bytes;
        session.set_ram_limit(stored.saturating_sub(1));
        assert!(session.ram_exhausted());

        session.select_topic(Some("c".into()));
        let result = session.ingest(msg("c", b"nope", false));
        assert!(result.topic_message.is_none());
        assert!(result.upserts.is_empty());
        assert!(session.get_message("c", None).is_none());
        assert_eq!(session.stats(0.0).messages_total, 3);
        assert!(session.status_event().ram_exhausted);
    }

    #[test]
    fn empty_retain_while_exhausted_allows_ingest_again() {
        let mut session = session();
        session.ingest(msg("a", b"hello", false));
        session.ingest(msg("b", b"world", false));
        let stored = session.stats(0.0).stored_bytes;
        session.set_ram_limit(stored.saturating_sub(1));
        assert!(session.ram_exhausted());

        let result = session.ingest(msg("a", b"", true));
        assert!(result.deletes.contains(&"a".to_string()));
        assert!(!session.ram_exhausted());
        assert!(session.get_message("a", None).is_none());

        let result = session.ingest(msg("c", b"ok", false));
        assert!(result.upserts.iter().any(|node| node.path == "c"));
        assert!(session.get_message("c", None).is_some());
    }

    #[test]
    fn set_ram_limit_returns_upserts_for_evicted_topics() {
        let mut session = session();
        session.ingest(msg("a", b"1", false));
        session.ingest(msg("a", b"2", false));
        session.ingest(msg("a", b"3", false));
        session.ingest(msg("b", b"1", false));
        session.ingest(msg("b", b"2", false));
        session.ingest(msg("b", b"3", false));
        let upserts = session.set_ram_limit(900);
        assert!(upserts.iter().any(|node| node.path == "a"));
        assert_eq!(session.get_history_meta("a").unwrap().count, 1);
    }
}
