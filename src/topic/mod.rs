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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::protocol::QoS;

/// Maximum number of entries in the topic cache
const TOPIC_CACHE_MAX_SIZE: usize = 1024;

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

/// Cached topic match result
struct CachedMatch {
    subscriptions: SmallVec<[Subscription; 16]>,
    generation: u64,
}

/// Thread-safe subscription store using topic trie
pub struct SubscriptionStore {
    trie: RwLock<TopicTrie<Vec<Subscription>>>,
    /// Round-robin counters for shared subscriptions, keyed by share group
    share_counters: DashMap<Arc<str>, AtomicUsize>,
    /// Cache of topic -> matching subscriptions (invalidated on subscription changes)
    topic_cache: DashMap<String, CachedMatch>,
    /// Generation counter - incremented on any subscription change
    generation: AtomicU64,
}

impl SubscriptionStore {
    pub fn new() -> Self {
        Self {
            trie: RwLock::new(TopicTrie::new()),
            share_counters: DashMap::new(),
            topic_cache: DashMap::new(),
            generation: AtomicU64::new(0),
        }
    }

    /// Invalidate cache by incrementing generation
    #[inline]
    fn invalidate_cache(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        // Optionally clear cache if it's too large
        if self.topic_cache.len() > TOPIC_CACHE_MAX_SIZE * 2 {
            self.topic_cache.clear();
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
        drop(trie);
        self.invalidate_cache();
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
        let removed = if let Some(subs) = trie.get_mut(actual_filter) {
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
        };
        drop(trie);
        if removed {
            self.invalidate_cache();
        }
        removed
    }

    /// Remove all subscriptions for a client
    pub fn unsubscribe_all(&self, client_id: &str) {
        let mut trie = self.trie.write();
        trie.remove_by_predicate(|subs| {
            subs.retain(|s| s.client_id.as_ref() != client_id);
            subs.is_empty()
        });
        drop(trie);
        self.invalidate_cache();
    }

    /// Find all matching subscriptions for a topic
    /// For shared subscriptions, only one subscriber per share group is returned (round-robin)
    ///
    /// Performance: Uses topic cache for frequently-published topics (O(1) lookup)
    /// Cache is invalidated when subscriptions change.
    pub fn matches(&self, topic: &str) -> SmallVec<[Subscription; 16]> {
        let current_gen = self.generation.load(Ordering::Acquire);

        // Check cache first (only for non-shared subscriptions)
        if let Some(cached) = self.topic_cache.get(topic) {
            if cached.generation == current_gen {
                return cached.subscriptions.clone();
            }
        }

        // Cache miss or stale - compute matches
        let trie = self.trie.read();
        let mut result: SmallVec<[Subscription; 16]> = SmallVec::new();
        let mut share_groups: AHashMap<Arc<str>, SmallVec<[Subscription; 4]>> =
            AHashMap::with_capacity(4);
        let mut has_shared = false;

        trie.matches(topic, |subs| {
            for sub in subs {
                if let Some(ref group) = sub.share_group {
                    has_shared = true;
                    share_groups
                        .entry(group.clone())
                        .or_default()
                        .push(sub.clone());
                } else {
                    result.push(sub.clone());
                }
            }
        });
        drop(trie);

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
            result.push(subs[idx].clone());
        }

        // Cache result only if no shared subscriptions (round-robin makes them uncacheable)
        // and cache isn't too large
        if !has_shared && self.topic_cache.len() < TOPIC_CACHE_MAX_SIZE {
            self.topic_cache.insert(
                topic.to_string(),
                CachedMatch {
                    subscriptions: result.clone(),
                    generation: current_gen,
                },
            );
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
