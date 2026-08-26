use std::collections::{BTreeMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::message::Message;

#[derive(Clone, Debug)]
pub struct TopicTree {
    root: Node,
    index: HashSet<String>,
    buffer_size: usize,
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
}

impl TopicTree {
    pub fn new(buffer_size: usize) -> Self {
        Self {
            root: Node::default(),
            index: HashSet::new(),
            buffer_size,
        }
    }

    pub fn buffer_size(&self) -> usize {
        self.buffer_size
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

    fn insert_leaf(&mut self, message: Message) -> UpsertOutcome {
        let topic = message.inbound.topic.clone();
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

        let existed = node.leaf.is_some();
        match &mut node.leaf {
            Some(leaf) => {
                leaf.history
                    .push_back(std::mem::replace(&mut leaf.latest, message));
                if self.buffer_size > 0 && leaf.message_count() > self.buffer_size {
                    leaf.history.pop_front();
                }
                leaf.received = leaf.received.saturating_add(1);
            }
            None => {
                node.leaf = Some(Leaf {
                    latest: message,
                    history: VecDeque::new(),
                    received: 1,
                });
            }
        }

        self.index.insert(topic);
        if existed {
            UpsertOutcome::Updated
        } else {
            UpsertOutcome::Inserted
        }
    }

    fn delete_leaf(&mut self, topic: &str) -> UpsertOutcome {
        self.index.remove(topic);
        let mut pruned = Vec::new();
        if topic.is_empty() {
            self.root.leaf.take();
            return UpsertOutcome::Deleted { pruned };
        }
        let segments: Vec<&str> = topic.split('/').collect();
        delete_walk(&mut self.root, &segments, "", &mut pruned);
        UpsertOutcome::Deleted { pruned }
    }
}

impl Leaf {
    pub fn message_count(&self) -> usize {
        self.history.len() + 1
    }

    pub fn get(&self, index: Option<usize>) -> Option<&Message> {
        match index {
            None => Some(&self.latest),
            Some(i) if i == self.history.len() => Some(&self.latest),
            Some(i) => self.history.get(i),
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

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use bytes::Bytes;

    use super::*;
    use crate::message::{Inbound, QoS, decode_inbound};

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
