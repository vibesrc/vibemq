//! Topic Trie for efficient subscription matching
//!
//! A trie (prefix tree) data structure optimized for MQTT topic matching.
//! Supports wildcards (+ and #) for subscription filters.
//!
//! Performance optimizations:
//! - Uses iterator-based traversal to avoid Vec allocations on every operation
//! - Uses compact_str for memory-efficient topic level storage
//! - Pre-allocates children HashMap capacity for common workloads

use ahash::AHashMap;
use compact_str::CompactString;
use smallvec::SmallVec;

/// Node in the topic trie
#[derive(Debug)]
struct TrieNode<V> {
    /// Value stored at this node (subscription data)
    value: Option<V>,
    /// Children indexed by topic level (CompactString avoids heap allocation for short strings)
    children: AHashMap<CompactString, TrieNode<V>>,
    /// Single-level wildcard (+) child
    single_wildcard: Option<Box<TrieNode<V>>>,
    /// Multi-level wildcard (#) value
    multi_wildcard: Option<V>,
}

impl<V> TrieNode<V> {
    fn new() -> Self {
        Self {
            value: None,
            // Most nodes have few children, but some may have many
            children: AHashMap::with_capacity(4),
            single_wildcard: None,
            multi_wildcard: None,
        }
    }
}

impl<V> Default for TrieNode<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// Topic Trie for efficient subscription matching
#[derive(Debug)]
pub struct TopicTrie<V> {
    root: TrieNode<V>,
}

impl<V> TopicTrie<V> {
    pub fn new() -> Self {
        Self {
            root: TrieNode::new(),
        }
    }

    /// Insert a topic filter with associated value
    /// Uses iterator-based traversal to avoid Vec allocation
    pub fn insert(&mut self, filter: &str, value: V) {
        let mut node = &mut self.root;
        let mut levels = filter.split('/').peekable();

        while let Some(level) = levels.next() {
            let is_last = levels.peek().is_none();

            if level == "#" {
                // Multi-level wildcard - store value here
                node.multi_wildcard = Some(value);
                return;
            } else if level == "+" {
                // Single-level wildcard
                if node.single_wildcard.is_none() {
                    node.single_wildcard = Some(Box::new(TrieNode::new()));
                }
                node = node.single_wildcard.as_mut().unwrap();
            } else {
                // Normal level - use CompactString for efficient storage
                node = node.children.entry(CompactString::new(level)).or_default();
            }

            // If this is the last level, store the value
            if is_last {
                node.value = Some(value);
                return;
            }
        }
    }

    /// Get a mutable reference to the value at a filter
    /// Uses iterator-based traversal to avoid Vec allocation
    pub fn get_mut(&mut self, filter: &str) -> Option<&mut V> {
        let mut node = &mut self.root;
        let mut levels = filter.split('/').peekable();

        while let Some(level) = levels.next() {
            let is_last = levels.peek().is_none();

            if level == "#" {
                return node.multi_wildcard.as_mut();
            } else if level == "+" {
                node = node.single_wildcard.as_mut()?;
            } else {
                node = node.children.get_mut(level)?;
            }

            if is_last {
                return node.value.as_mut();
            }
        }

        None
    }

    /// Remove a filter from the trie
    /// Uses SmallVec to avoid heap allocation for typical topic depths (up to 8 levels)
    pub fn remove(&mut self, filter: &str) -> Option<V> {
        let levels: SmallVec<[&str; 8]> = filter.split('/').collect();
        Self::remove_recursive(&mut self.root, &levels, 0)
    }

    fn remove_recursive(node: &mut TrieNode<V>, levels: &[&str], index: usize) -> Option<V> {
        if index >= levels.len() {
            return node.value.take();
        }

        let level = levels[index];

        match level {
            "#" => node.multi_wildcard.take(),
            "+" => {
                if let Some(ref mut child) = node.single_wildcard {
                    if index + 1 >= levels.len() {
                        child.value.take()
                    } else {
                        Self::remove_recursive(child, levels, index + 1)
                    }
                } else {
                    None
                }
            }
            _ => {
                if let Some(child) = node.children.get_mut(level) {
                    if index + 1 >= levels.len() {
                        child.value.take()
                    } else {
                        Self::remove_recursive(child, levels, index + 1)
                    }
                } else {
                    None
                }
            }
        }
    }

    /// Remove entries by predicate (returns true if entry should be removed)
    pub fn remove_by_predicate<F>(&mut self, mut pred: F)
    where
        F: FnMut(&mut V) -> bool,
    {
        Self::remove_by_predicate_recursive(&mut self.root, &mut pred);
    }

    fn remove_by_predicate_recursive<F>(node: &mut TrieNode<V>, pred: &mut F)
    where
        F: FnMut(&mut V) -> bool,
    {
        if let Some(ref mut v) = node.value {
            if pred(v) {
                node.value = None;
            }
        }

        if let Some(ref mut v) = node.multi_wildcard {
            if pred(v) {
                node.multi_wildcard = None;
            }
        }

        if let Some(ref mut child) = node.single_wildcard {
            Self::remove_by_predicate_recursive(child, pred);
        }

        for child in node.children.values_mut() {
            Self::remove_by_predicate_recursive(child, pred);
        }
    }

    /// Find all matching subscriptions for a topic name
    /// Uses SmallVec to avoid heap allocation for typical topic depths (up to 8 levels)
    pub fn matches<F>(&self, topic: &str, mut callback: F)
    where
        F: FnMut(&V),
    {
        // $-topics don't match filters starting with + or #
        let is_system_topic = topic.starts_with('$');

        let levels: SmallVec<[&str; 8]> = topic.split('/').collect();
        Self::matches_recursive(&self.root, &levels, 0, is_system_topic, &mut callback);
    }

    fn matches_recursive<F>(
        node: &TrieNode<V>,
        levels: &[&str],
        index: usize,
        is_system_topic: bool,
        callback: &mut F,
    ) where
        F: FnMut(&V),
    {
        // Check multi-level wildcard at current level
        // (but not for $-topics at the root level)
        if !(is_system_topic && index == 0) {
            if let Some(ref v) = node.multi_wildcard {
                callback(v);
            }
        }

        if index >= levels.len() {
            // At end of topic - check for exact match
            if let Some(ref v) = node.value {
                callback(v);
            }
            return;
        }

        let level = levels[index];

        // Check single-level wildcard (but not for $-topics at root)
        if !(is_system_topic && index == 0) {
            if let Some(ref child) = node.single_wildcard {
                Self::matches_recursive(child, levels, index + 1, is_system_topic, callback);
            }
        }

        // Check exact match
        if let Some(child) = node.children.get(level) {
            Self::matches_recursive(child, levels, index + 1, is_system_topic, callback);
        }
    }
}

impl<V> Default for TopicTrie<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let mut trie = TopicTrie::new();
        trie.insert("test/topic", 1);

        let mut matches = Vec::new();
        trie.matches("test/topic", |v| matches.push(*v));
        assert_eq!(matches, vec![1]);

        matches.clear();
        trie.matches("test/other", |v| matches.push(*v));
        assert!(matches.is_empty());
    }

    #[test]
    fn test_single_wildcard() {
        let mut trie = TopicTrie::new();
        trie.insert("test/+", 1);
        trie.insert("+/topic", 2);
        trie.insert("+/+", 3);

        let mut matches = Vec::new();
        trie.matches("test/topic", |v| matches.push(*v));
        matches.sort();
        assert_eq!(matches, vec![1, 2, 3]);
    }

    #[test]
    fn test_multi_wildcard() {
        let mut trie = TopicTrie::new();
        trie.insert("#", 1);
        trie.insert("test/#", 2);

        let mut matches = Vec::new();
        trie.matches("test/topic/deep", |v| matches.push(*v));
        matches.sort();
        assert_eq!(matches, vec![1, 2]);
    }

    #[test]
    fn test_system_topics() {
        let mut trie = TopicTrie::new();
        trie.insert("#", 1);
        trie.insert("+/test", 2);
        trie.insert("$SYS/#", 3);

        // $SYS topics should not match # or +
        let mut matches = Vec::new();
        trie.matches("$SYS/test", |v| matches.push(*v));
        assert_eq!(matches, vec![3]);
    }

    #[test]
    fn test_remove() {
        let mut trie = TopicTrie::new();
        trie.insert("test/topic", 1);

        let removed = trie.remove("test/topic");
        assert_eq!(removed, Some(1));

        let mut matches = Vec::new();
        trie.matches("test/topic", |v| matches.push(*v));
        assert!(matches.is_empty());
    }
}
