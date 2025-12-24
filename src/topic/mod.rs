//! Topic matching and subscription management
//!
//! Implements topic name/filter validation and a topic trie for efficient
//! subscription matching based on spec/v3.1.1/4.7_topic-names-and-filters.md
//! and spec/v5.0/4.7_topic-names-and-filters.md
//!
//! Performance optimizations:
//! - Uses callback-based matching to avoid intermediate allocations
//! - Uses SmallVec for typical workloads (few matching subscriptions per topic)
//! - Pre-allocates result vectors with reasonable capacity

mod trie;
pub mod validation;

pub use trie::TopicTrie;
pub use validation::{
    topic_matches_filter, validate_topic_filter, validate_topic_filter_with_max_levels,
    validate_topic_name, validate_topic_name_with_max_levels, TopicLevel,
};

use ahash::AHashMap;
use dashmap::DashMap;
use parking_lot::RwLock;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::protocol::QoS;

/// A subscription entry
#[derive(Debug, Clone)]
pub struct Subscription {
    /// Client ID
    pub client_id: Arc<str>,
    /// Subscription QoS
    pub qos: QoS,
    /// No local flag (v5.0) - don't send messages to the client that published them
    pub no_local: bool,
    /// Retain as published flag (v5.0)
    pub retain_as_published: bool,
    /// Subscription identifier (v5.0)
    pub subscription_id: Option<u32>,
    /// Share group name (v5.0) - for shared subscriptions ($share/{group}/{filter})
    pub share_group: Option<Arc<str>>,
}

/// Parse a shared subscription filter
/// Returns (share_group, actual_filter) if it's a shared subscription, or None
pub fn parse_shared_subscription(filter: &str) -> Option<(&str, &str)> {
    if let Some(rest) = filter.strip_prefix("$share/") {
        // Format: $share/{group}/{filter}
        // Skip "$share/"
        if let Some(slash_pos) = rest.find('/') {
            let group = &rest[..slash_pos];
            let actual_filter = &rest[slash_pos + 1..];
            if !group.is_empty() && !actual_filter.is_empty() {
                return Some((group, actual_filter));
            }
        }
    }
    None
}

/// Thread-safe subscription store using topic trie
pub struct SubscriptionStore {
    trie: RwLock<TopicTrie<Vec<Subscription>>>,
    /// Round-robin counters for shared subscriptions, keyed by share group
    share_counters: DashMap<Arc<str>, AtomicUsize>,
}

impl SubscriptionStore {
    pub fn new() -> Self {
        Self {
            trie: RwLock::new(TopicTrie::new()),
            share_counters: DashMap::new(),
        }
    }

    /// Add a subscription
    pub fn subscribe(&self, filter: &str, mut subscription: Subscription) {
        // Check if this is a shared subscription
        let actual_filter = if let Some((group, actual)) = parse_shared_subscription(filter) {
            subscription.share_group = Some(group.into());
            // Ensure we have a counter for this share group
            self.share_counters
                .entry(group.into())
                .or_insert_with(|| AtomicUsize::new(0));
            actual
        } else {
            filter
        };

        let mut trie = self.trie.write();
        if let Some(subs) = trie.get_mut(actual_filter) {
            // For shared subscriptions, also match on share_group
            subs.retain(|s| {
                !(s.client_id == subscription.client_id
                    && s.share_group == subscription.share_group)
            });
            subs.push(subscription);
        } else {
            trie.insert(actual_filter, vec![subscription]);
        }
    }

    /// Remove a subscription
    pub fn unsubscribe(&self, filter: &str, client_id: &str) -> bool {
        // Check if this is a shared subscription
        let (actual_filter, share_group) =
            if let Some((group, actual)) = parse_shared_subscription(filter) {
                (actual, Some(group))
            } else {
                (filter, None)
            };

        let mut trie = self.trie.write();
        if let Some(subs) = trie.get_mut(actual_filter) {
            let len_before = subs.len();
            subs.retain(|s| {
                if s.client_id.as_ref() != client_id {
                    return true;
                }
                // Match share group if this is a shared subscription
                match (&s.share_group, share_group) {
                    (Some(sg), Some(fg)) => sg.as_ref() != fg,
                    (None, None) => false, // Remove non-shared sub
                    _ => true,             // Mismatch, keep
                }
            });
            let removed = subs.len() != len_before;
            if subs.is_empty() {
                trie.remove(actual_filter);
            }
            removed
        } else {
            false
        }
    }

    /// Remove all subscriptions for a client
    pub fn unsubscribe_all(&self, client_id: &str) {
        let mut trie = self.trie.write();
        trie.remove_by_predicate(|subs| {
            subs.retain(|s| s.client_id.as_ref() != client_id);
            subs.is_empty()
        });
    }

    /// Find all matching subscriptions for a topic
    /// For shared subscriptions, only one subscriber per share group is returned (round-robin)
    ///
    /// Performance: Uses SmallVec to avoid heap allocation for typical workloads
    /// (most topics have fewer than 16 subscribers)
    pub fn matches(&self, topic: &str) -> SmallVec<[Subscription; 16]> {
        let trie = self.trie.read();
        // Pre-allocate with reasonable capacity for typical workloads
        let mut result: SmallVec<[Subscription; 16]> = SmallVec::new();
        // Use AHashMap for faster hashing, SmallVec for values (most share groups have few subscribers)
        let mut share_groups: AHashMap<Arc<str>, SmallVec<[Subscription; 4]>> =
            AHashMap::with_capacity(4);

        trie.matches(topic, |subs| {
            for sub in subs {
                if let Some(ref group) = sub.share_group {
                    // Collect shared subscriptions by group
                    share_groups
                        .entry(group.clone())
                        .or_default()
                        .push(sub.clone());
                } else {
                    // Non-shared subscriptions go directly to result
                    result.push(sub.clone());
                }
            }
        });

        // For each share group, pick one subscriber using round-robin
        for (group, subs) in share_groups {
            if subs.is_empty() {
                continue;
            }
            // Get or create counter for this group
            let counter = self
                .share_counters
                .entry(group)
                .or_insert_with(|| AtomicUsize::new(0));
            let idx = counter.fetch_add(1, Ordering::Relaxed) % subs.len();
            result.push(subs[idx].clone());
        }

        result
    }

    /// Find all matching subscriptions using a callback to avoid allocation
    /// For shared subscriptions, only one subscriber per share group is called (round-robin)
    ///
    /// Note: For shared subscriptions, this still needs to clone subscriptions temporarily
    /// to handle the round-robin selection. For non-shared subscriptions, the callback
    /// is invoked immediately without cloning.
    pub fn matches_with_callback<F>(&self, topic: &str, mut callback: F)
    where
        F: FnMut(&Subscription),
    {
        let trie = self.trie.read();
        // Temporary storage for share group selection (must clone due to callback lifetime)
        let mut share_groups: AHashMap<Arc<str>, SmallVec<[Subscription; 4]>> =
            AHashMap::with_capacity(4);

        trie.matches(topic, |subs| {
            for sub in subs {
                if let Some(ref group) = sub.share_group {
                    // Collect shared subscriptions by group (clone needed for round-robin selection)
                    share_groups
                        .entry(group.clone())
                        .or_default()
                        .push(sub.clone());
                } else {
                    // Non-shared subscriptions get called immediately (no clone!)
                    callback(sub);
                }
            }
        });

        // For each share group, pick one subscriber using round-robin
        for (group, subs) in share_groups {
            if subs.is_empty() {
                continue;
            }
            let counter = self
                .share_counters
                .entry(group)
                .or_insert_with(|| AtomicUsize::new(0));
            let idx = counter.fetch_add(1, Ordering::Relaxed) % subs.len();
            callback(&subs[idx]);
        }
    }

    /// Count the number of shared subscriptions
    /// For $SYS/broker/shared_subscriptions/count
    pub fn shared_subscription_count(&self) -> usize {
        let trie = self.trie.read();
        let mut count = 0;
        trie.for_each(|subs| {
            count += subs.iter().filter(|s| s.share_group.is_some()).count();
        });
        count
    }
}

impl Default for SubscriptionStore {
    fn default() -> Self {
        Self::new()
    }
}
