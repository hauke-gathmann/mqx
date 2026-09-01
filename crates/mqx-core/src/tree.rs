use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::message::{Format, Message};

#[derive(Clone, Debug)]
pub struct TopicTree {
    root: Node,
    index: HashSet<String>,
    buffer_size: usize,
    ram_limit: u64,
    stored_bytes: u64,
    ram_exhausted: bool,
    extra_index: BTreeMap<(SystemTime, u64), String>,
    next_seq: u64,
    evicted: HashSet<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub segment: String,
    pub children: BTreeMap<String, Node>,
    pub leaf: Option<Leaf>,
}

#[derive(Clone, Debug)]
pub struct Leaf {
    pub latest: Message,
    pub history: VecDeque<Message>,
    pub received: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    Keep,
    Skip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub path: String,
    pub highlights: Vec<usize>,
    pub score: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpsertOutcome {
    Inserted,
    Updated,
    /// Leaf cleared on the inbound topic. `pruned` is only nodes then **removed**
    /// from the tree (the topic itself if it had no children, plus empty ancestors).
    /// A parent that still has children is not listed.
    Deleted {
        pruned: Vec<String>,
    },
    /// Ingest skipped because the store is over budget with no extras left.
    Ignored,
}

impl TopicTree {
    pub fn new(buffer_size: usize) -> Self {
        Self::with_limits(buffer_size, crate::config::DEFAULT_RAM_LIMIT_BYTES)
    }

    pub fn with_limits(buffer_size: usize, ram_limit: u64) -> Self {
        Self {
            root: Node::default(),
            index: HashSet::new(),
            buffer_size,
            ram_limit,
            stored_bytes: 0,
            ram_exhausted: false,
            extra_index: BTreeMap::new(),
            next_seq: 0,
            evicted: HashSet::new(),
        }
    }

    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    pub fn ram_limit(&self) -> u64 {
        self.ram_limit
    }

    pub fn stored_bytes(&self) -> u64 {
        self.stored_bytes
    }

    pub fn ram_exhausted(&self) -> bool {
        self.ram_exhausted
    }

    pub fn set_ram_limit(&mut self, ram_limit: u64) {
        self.ram_limit = ram_limit;
        self.evict();
    }

    pub fn take_evicted(&mut self) -> Vec<String> {
        self.evicted.drain().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn topic_count(&self) -> usize {
        self.index.len()
    }

    pub fn root(&self) -> &Node {
        &self.root
    }

    pub fn node(&self, path: &str) -> Option<&Node> {
        if path.is_empty() {
            return Some(&self.root);
        }
        let mut node = &self.root;
        for segment in path.split('/') {
            node = node.children.get(segment)?;
        }
        Some(node)
    }

    pub fn get(&self, topic: &str) -> Option<&Leaf> {
        self.node(topic)?.leaf.as_ref()
    }

    pub fn get_message(&self, topic: &str, index: Option<usize>) -> Option<&Message> {
        self.get(topic)?.get(index)
    }

    /// Only write path for the tree. Empty payload + retain deletes the leaf.
    pub fn upsert(&mut self, message: Message) -> UpsertOutcome {
        let delete = message.inbound.retain && message.inbound.payload.is_empty();
        if delete {
            self.delete_leaf(&message.inbound.topic)
        } else if self.ram_exhausted {
            UpsertOutcome::Ignored
        } else {
            self.insert_leaf(message)
        }
    }

    pub fn search(&self, query: &str, mode: SearchMode) -> Vec<SearchHit> {
        let mut hits = Vec::new();
        let mut topics: Vec<&str> = self.index.iter().map(String::as_str).collect();
        topics.sort_unstable();
        for (i, path) in topics.into_iter().enumerate() {
            let contains = !query.is_empty() && path.contains(query);
            let keep = match mode {
                SearchMode::Keep if query.is_empty() => true,
                SearchMode::Keep => contains,
                SearchMode::Skip => query.is_empty() || !contains,
            };
            if !keep {
                continue;
            }
            hits.push(SearchHit {
                path: path.to_string(),
                highlights: if contains {
                    substring_highlights(path, query)
                } else {
                    Vec::new()
                },
                score: -(i as i64),
            });
        }
        hits
    }

    pub fn iter_leaves(&self) -> Vec<(String, &Leaf)> {
        let mut topics: Vec<&str> = self.index.iter().map(String::as_str).collect();
        topics.sort_unstable();
        topics
            .into_iter()
            .filter_map(|topic| self.get(topic).map(|leaf| (topic.to_string(), leaf)))
            .collect()
    }

    fn insert_leaf(&mut self, mut message: Message) -> UpsertOutcome {
        message.seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let bytes = message_bytes(&message);
        let topic = message.inbound.topic.clone();

        let buffer_size = self.buffer_size;
        let (existed, extra_key, overflow) = {
            let node = self.ensure_node(&topic);
            let existed = node.leaf.is_some();
            let mut extra_key = None;
            let mut overflow = false;
            match &mut node.leaf {
                Some(leaf) => {
                    let previous = std::mem::replace(&mut leaf.latest, message);
                    extra_key = Some((previous.inbound.timestamp, previous.seq));
                    leaf.history.push_back(previous);
                    leaf.received = leaf.received.saturating_add(1);
                    overflow = buffer_size > 0 && leaf.message_count() > buffer_size;
                }
                None => {
                    node.leaf = Some(Leaf {
                        latest: message,
                        history: VecDeque::new(),
                        received: 1,
                    });
                }
            }
            (existed, extra_key, overflow)
        };

        if let Some(key) = extra_key {
            self.extra_index.insert(key, topic.clone());
        }
        self.stored_bytes = self.stored_bytes.saturating_add(bytes);
        if overflow {
            self.drop_topic_oldest_extra(&topic);
        }
        self.index.insert(topic);
        self.evict();
        if existed {
            UpsertOutcome::Updated
        } else {
            UpsertOutcome::Inserted
        }
    }

    fn delete_leaf(&mut self, topic: &str) -> UpsertOutcome {
        let leaf = if topic.is_empty() {
            self.root.leaf.take()
        } else {
            self.node_mut(topic).and_then(|node| node.leaf.take())
        };
        if let Some(leaf) = leaf {
            self.release_leaf(&leaf);
        }
        self.index.remove(topic);
        self.set_ram_exhausted(self.stored_bytes > self.ram_limit && self.extra_index.is_empty());

        let mut pruned = Vec::new();
        if topic.is_empty() {
            return UpsertOutcome::Deleted { pruned };
        }
        let segments: Vec<&str> = topic.split('/').collect();
        delete_walk(&mut self.root, &segments, "", &mut pruned);
        UpsertOutcome::Deleted { pruned }
    }

    fn ensure_node(&mut self, topic: &str) -> &mut Node {
        let mut node = &mut self.root;
        if !topic.is_empty() {
            for segment in topic.split('/') {
                node = node
                    .children
                    .entry(segment.to_string())
                    .or_insert_with(|| Node {
                        segment: segment.to_string(),
                        children: BTreeMap::new(),
                        leaf: None,
                    });
            }
        }
        node
    }

    fn node_mut(&mut self, path: &str) -> Option<&mut Node> {
        if path.is_empty() {
            return Some(&mut self.root);
        }
        let mut node = &mut self.root;
        for segment in path.split('/') {
            node = node.children.get_mut(segment)?;
        }
        Some(node)
    }

    fn pop_history_front(&mut self, topic: &str) -> Option<Message> {
        self.node_mut(topic)?.leaf.as_mut()?.history.pop_front()
    }

    fn take_extra(&mut self, topic: &str, seq: u64) -> Option<Message> {
        let history = &mut self.node_mut(topic)?.leaf.as_mut()?.history;
        let pos = history.iter().position(|message| message.seq == seq)?;
        history.remove(pos)
    }

    fn drop_topic_oldest_extra(&mut self, topic: &str) {
        if let Some(dropped) = self.pop_history_front(topic) {
            self.extra_index
                .remove(&(dropped.inbound.timestamp, dropped.seq));
            self.stored_bytes = self.stored_bytes.saturating_sub(message_bytes(&dropped));
            self.evicted.insert(topic.to_string());
        }
    }

    fn release_leaf(&mut self, leaf: &Leaf) {
        for message in &leaf.history {
            self.extra_index
                .remove(&(message.inbound.timestamp, message.seq));
            self.stored_bytes = self.stored_bytes.saturating_sub(message_bytes(message));
        }
        self.stored_bytes = self
            .stored_bytes
            .saturating_sub(message_bytes(&leaf.latest));
    }

    fn evict(&mut self) {
        if self.stored_bytes <= self.ram_limit {
            self.set_ram_exhausted(false);
            return;
        }
        let target = (u128::from(self.ram_limit) * 9 / 10) as u64;
        while self.stored_bytes > target {
            let Some(((_, seq), topic)) = self.extra_index.pop_first() else {
                self.set_ram_exhausted(self.stored_bytes > self.ram_limit);
                return;
            };
            if let Some(dropped) = self.take_extra(&topic, seq) {
                self.stored_bytes = self.stored_bytes.saturating_sub(message_bytes(&dropped));
                self.evicted.insert(topic);
            }
        }
        self.set_ram_exhausted(self.stored_bytes > self.ram_limit);
    }

    fn set_ram_exhausted(&mut self, exhausted: bool) {
        if self.ram_exhausted == exhausted {
            return;
        }
        self.ram_exhausted = exhausted;
        if exhausted {
            warn!(
                stored_bytes = self.stored_bytes,
                ram_limit = self.ram_limit,
                "topic store RAM budget exhausted"
            );
        } else {
            debug!(
                stored_bytes = self.stored_bytes,
                ram_limit = self.ram_limit,
                "topic store RAM budget recovered"
            );
        }
    }
}

fn message_bytes(message: &Message) -> u64 {
    const OVERHEAD: u64 = 192;
    let payload = message.inbound.payload.len() as u64;
    let text = message.text.len() as u64;
    match message.format {
        Format::Json => payload + text + text + OVERHEAD,
        Format::Text => payload + text + OVERHEAD,
        Format::Binary => payload + OVERHEAD,
    }
}

impl Leaf {
    pub fn message_count(&self) -> usize {
        self.history.len() + 1
    }

    pub fn get(&self, index: Option<usize>) -> Option<&Message> {
        match index {
            None => Some(&self.latest),
            Some(seq) if self.latest.seq == seq as u64 => Some(&self.latest),
            Some(seq) => self
                .history
                .iter()
                .find(|message| message.seq == seq as u64),
        }
    }
}

fn delete_walk(node: &mut Node, segments: &[&str], prefix: &str, pruned: &mut Vec<String>) -> bool {
    if let Some((segment, rest)) = segments.split_first() {
        let child_path = if prefix.is_empty() {
            (*segment).to_string()
        } else {
            format!("{prefix}/{segment}")
        };
        let prune_child = node
            .children
            .get_mut(*segment)
            .is_some_and(|child| delete_walk(child, rest, &child_path, pruned));
        if prune_child {
            node.children.remove(*segment);
            pruned.push(child_path);
        }
        return node.leaf.is_none() && node.children.is_empty();
    }

    node.leaf = None;
    node.children.is_empty()
}

fn substring_highlights(path: &str, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut highlights = Vec::new();
    let mut from = 0;
    while let Some(rel) = path[from..].find(query) {
        let at = from + rel;
        highlights.extend(at..at + query.len());
        from = at + query.len();
        if from >= path.len() {
            break;
        }
    }
    highlights
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use bytes::Bytes;

    use super::*;
    use crate::message::{Inbound, QoS, decode_inbound};

    fn msg(topic: &str, payload: &[u8], retain: bool) -> Message {
        decode_inbound(Inbound {
            topic: topic.into(),
            payload: Bytes::copy_from_slice(payload),
            retain,
            qos: QoS::AtMostOnce,
            dup: false,
            timestamp: SystemTime::now(),
        })
    }

    fn msg_at(topic: &str, payload: &[u8], secs: u64) -> Message {
        decode_inbound(Inbound {
            topic: topic.into(),
            payload: Bytes::copy_from_slice(payload),
            retain: false,
            qos: QoS::AtMostOnce,
            dup: false,
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
        })
    }

    #[test]
    fn insert_creates_segments_leaf_only_on_last() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("home/living/lamp", b"on", false));

        let home = tree.node("home").expect("home");
        assert!(home.leaf.is_none());
        assert_eq!(home.children.len(), 1);

        let living = tree.node("home/living").expect("living");
        assert!(living.leaf.is_none());
        assert_eq!(living.children.len(), 1);

        let lamp = tree.node("home/living/lamp").expect("lamp");
        assert!(lamp.leaf.is_some());
        assert!(lamp.children.is_empty());
        assert_eq!(tree.topic_count(), 1);
    }

    #[test]
    fn second_message_updates_latest_and_grows_history() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("home/living/lamp", b"1", false));
        tree.upsert(msg("home/living/lamp", b"2", false));

        let leaf = tree.get("home/living/lamp").expect("leaf");
        assert_eq!(leaf.latest.text, "2");
        assert_eq!(leaf.history.len(), 1);
        assert_eq!(leaf.history[0].text, "1");
        assert_eq!(leaf.received, 2);
        assert_eq!(leaf.message_count(), 2);
    }

    #[test]
    fn buffer_size_two_evicts_oldest() {
        let mut tree = TopicTree::new(2);
        tree.upsert(msg("t", b"1", false));
        tree.upsert(msg("t", b"2", false));
        tree.upsert(msg("t", b"3", false));

        let leaf = tree.get("t").expect("leaf");
        assert_eq!(leaf.latest.text, "3");
        assert_eq!(leaf.message_count(), 2);
        assert_eq!(leaf.history.len(), 1);
        assert_eq!(leaf.history[0].text, "2");
    }

    #[test]
    fn empty_retain_deletes_leaf_and_prunes_parents() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("home/living/lamp", b"on", false));
        let outcome = tree.upsert(msg("home/living/lamp", b"", true));

        assert_eq!(
            outcome,
            UpsertOutcome::Deleted {
                pruned: vec![
                    "home/living/lamp".into(),
                    "home/living".into(),
                    "home".into()
                ]
            }
        );
        assert!(tree.is_empty());
        assert!(tree.node("home").is_none());
        assert!(tree.root().children.is_empty());
    }

    #[test]
    fn empty_retain_keeps_siblings_and_parent_payload() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("home", b"house", false));
        tree.upsert(msg("home/living/lamp", b"on", false));
        tree.upsert(msg("home/living/temp", b"21", false));
        tree.upsert(msg("home/living/lamp", b"", true));

        assert!(tree.get("home/living/lamp").is_none());
        assert!(tree.get("home/living/temp").is_some());
        assert!(tree.get("home").is_some());
        assert!(tree.node("home/living").is_some());
        assert_eq!(tree.topic_count(), 2);
    }

    #[test]
    fn empty_retain_prunes_one_branch_not_the_tree() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("home/a/x", b"1", false));
        tree.upsert(msg("home/b", b"2", false));
        let outcome = tree.upsert(msg("home/a/x", b"", true));

        assert_eq!(
            outcome,
            UpsertOutcome::Deleted {
                pruned: vec!["home/a/x".into(), "home/a".into()]
            }
        );
        assert!(tree.node("home/a").is_none());
        assert!(tree.get("home/b").is_some());
        assert!(!tree.is_empty());
    }

    #[test]
    fn empty_retain_on_parent_does_not_prune_remaining_node() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("home", b"house", false));
        tree.upsert(msg("home/lamp", b"on", false));
        let outcome = tree.upsert(msg("home", b"", true));

        assert_eq!(outcome, UpsertOutcome::Deleted { pruned: vec![] });
        assert!(tree.get("home").is_none());
        assert!(tree.node("home").is_some());
        assert!(tree.get("home/lamp").is_some());
    }

    #[test]
    fn search_keep_and_skip_are_substring_on_full_path() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("home/living/lamp", b"1", false));
        tree.upsert(msg("home/kitchen/fridge", b"2", false));
        tree.upsert(msg("lamp", b"3", false));

        let keep = tree.search("living/lamp", SearchMode::Keep);
        assert_eq!(
            keep.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(),
            vec!["home/living/lamp"]
        );
        assert_eq!(keep[0].highlights, (5..16).collect::<Vec<_>>());

        let nested = tree.search("lamp", SearchMode::Keep);
        assert_eq!(
            nested.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(),
            vec!["home/living/lamp", "lamp"]
        );

        let skip = tree.search("kit", SearchMode::Skip);
        assert_eq!(
            skip.iter().map(|h| h.path.as_str()).collect::<Vec<_>>(),
            vec!["home/living/lamp", "lamp"]
        );
    }

    #[test]
    fn ram_budget_evicts_oldest_extras_keeps_latest() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg_at("a", b"1", 1));
        tree.upsert(msg_at("a", b"2", 2));
        tree.upsert(msg_at("a", b"3", 3));
        tree.upsert(msg_at("b", b"1", 4));
        tree.upsert(msg_at("b", b"2", 5));
        tree.upsert(msg_at("b", b"3", 6));

        // Each text "n" is 194 bytes. Drop the two oldest extras (a/1, a/2) first.
        tree.set_ram_limit(900);
        assert_eq!(tree.get("a").unwrap().history.len(), 0);
        assert_eq!(tree.get("a").unwrap().latest.text, "3");
        assert_eq!(tree.get("b").unwrap().history.len(), 2);
        assert_eq!(tree.get("b").unwrap().latest.text, "3");

        tree.set_ram_limit(400);
        assert_eq!(tree.get("a").unwrap().history.len(), 0);
        assert_eq!(tree.get("b").unwrap().history.len(), 0);
        assert_eq!(tree.get("a").unwrap().latest.text, "3");
        assert_eq!(tree.get("b").unwrap().latest.text, "3");
        assert!(!tree.ram_exhausted());
    }

    #[test]
    fn ram_budget_one_message_per_topic_does_not_drop_latest() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("a", b"hello", false));
        tree.upsert(msg("b", b"world", false));
        let stored = tree.stored_bytes();
        assert!(stored > 0);

        tree.set_ram_limit(stored.saturating_sub(1));
        assert_eq!(tree.topic_count(), 2);
        assert!(tree.get("a").is_some());
        assert!(tree.get("b").is_some());
        assert!(tree.get("a").unwrap().history.is_empty());
        assert!(tree.get("b").unwrap().history.is_empty());
        assert!(tree.ram_exhausted());
        assert_eq!(
            tree.upsert(msg("c", b"nope", false)),
            UpsertOutcome::Ignored
        );
        assert!(tree.get("c").is_none());
    }

    #[test]
    fn empty_retain_subtracts_bytes_and_clears_extras() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("t", b"1", false));
        tree.upsert(msg("t", b"2", false));
        assert!(tree.stored_bytes() > 0);
        assert_eq!(tree.get("t").unwrap().history.len(), 1);

        tree.upsert(msg("t", b"", true));
        assert_eq!(tree.stored_bytes(), 0);
        assert!(tree.get("t").is_none());
        assert!(!tree.ram_exhausted());
    }

    #[test]
    fn seq_is_stable_and_missing_seq_returns_none() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg_at("t", b"a", 1));
        tree.upsert(msg_at("t", b"b", 2));
        tree.upsert(msg_at("t", b"c", 3));

        let extra_seq = tree.get("t").unwrap().history[0].seq as usize;
        let latest_seq = tree.get("t").unwrap().latest.seq as usize;
        assert!(tree.get_message("t", Some(extra_seq)).is_some());
        assert_eq!(tree.get_message("t", Some(latest_seq)).unwrap().text, "c");

        tree.set_ram_limit(200);
        assert!(tree.get_message("t", Some(extra_seq)).is_none());
        assert_eq!(tree.get_message("t", Some(latest_seq)).unwrap().text, "c");
        assert_eq!(tree.get("t").unwrap().latest.seq as usize, latest_seq);
        assert!(tree.get_message("t", Some(9999)).is_none());
        assert_eq!(tree.get_message("t", None).unwrap().text, "c");
    }

    #[test]
    fn ram_evict_drops_indexed_seq_not_history_front() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg_at("t", b"old", 10));
        tree.upsert(msg_at("t", b"mid", 1));
        tree.upsert(msg_at("t", b"new", 2));
        assert_eq!(tree.get("t").unwrap().history[0].text, "old");

        tree.set_ram_limit(500);
        let leaf = tree.get("t").unwrap();
        assert_eq!(leaf.history.len(), 1);
        assert_eq!(leaf.history[0].text, "old");
        assert_eq!(leaf.latest.text, "new");
        assert!(tree.get_message("t", Some(1)).is_none());
    }

    #[test]
    fn empty_retain_while_exhausted_frees_budget() {
        let mut tree = TopicTree::new(0);
        tree.upsert(msg("a", b"hello", false));
        tree.upsert(msg("b", b"world", false));
        let stored = tree.stored_bytes();
        tree.set_ram_limit(stored.saturating_sub(1));
        assert!(tree.ram_exhausted());

        assert_eq!(
            tree.upsert(msg("a", b"", true)),
            UpsertOutcome::Deleted {
                pruned: vec!["a".into()]
            }
        );
        assert!(tree.get("a").is_none());
        assert!(tree.get("b").is_some());
        assert!(!tree.ram_exhausted());
        assert_eq!(tree.upsert(msg("c", b"ok", false)), UpsertOutcome::Inserted);
        assert!(tree.get("c").is_some());
    }
}
