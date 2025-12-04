//! Topic Mapping for Bridge Forwarding
//!
//! Handles topic pattern matching and transformation between local and remote brokers.

use crate::config::ForwardRule;
use crate::protocol::QoS;
use crate::topic::validation::topic_matches_filter;

/// Maps topics between local and remote brokers based on forwarding rules
pub struct TopicMapper {
    /// Rules for outbound forwarding (local → remote)
    outbound_rules: Vec<CompiledRule>,
    /// Rules for inbound forwarding (remote → local)
    inbound_rules: Vec<CompiledRule>,
}

/// A compiled forwarding rule for efficient matching
#[derive(Debug, Clone)]
struct CompiledRule {
    /// Original local topic pattern
    local_pattern: String,
    /// Original remote topic pattern
    remote_pattern: String,
    /// Maximum QoS
    qos: QoS,
    /// Forward retained messages
    retain: bool,
    /// Prefix to strip from source topic
    strip_prefix: Option<String>,
    /// Prefix to add to destination topic
    add_prefix: Option<String>,
}

impl CompiledRule {
    fn from_forward_rule(rule: &ForwardRule, outbound: bool) -> Self {
        let qos = match rule.qos {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            _ => QoS::ExactlyOnce,
        };

        // Determine strip/add prefixes based on direction
        let (source_pattern, dest_pattern) = if outbound {
            (&rule.local_topic, &rule.remote_topic)
        } else {
            (&rule.remote_topic, &rule.local_topic)
        };

        // Extract prefix transformation
        // e.g., local="sensors/#" remote="edge/device01/sensors/#"
        // For outbound: strip nothing, add "edge/device01/"
        let (strip_prefix, add_prefix) =
            Self::compute_prefix_transform(source_pattern, dest_pattern);

        Self {
            local_pattern: rule.local_topic.clone(),
            remote_pattern: rule.remote_topic.clone(),
            qos,
            retain: rule.retain,
            strip_prefix,
            add_prefix,
        }
    }

    /// Compute prefix transformation between two patterns
    fn compute_prefix_transform(source: &str, dest: &str) -> (Option<String>, Option<String>) {
        // Find the common suffix (typically the wildcard part)
        let source_parts: Vec<&str> = source.split('/').collect();
        let dest_parts: Vec<&str> = dest.split('/').collect();

        // Check for simple prefix-based mapping
        // e.g., "sensors/#" → "edge/sensors/#"
        if source_parts.last() == dest_parts.last() {
            let source_prefix_len = source_parts.len() - 1;
            let dest_prefix_len = dest_parts.len() - 1;

            let strip = if source_prefix_len > 0 {
                Some(source_parts[..source_prefix_len].join("/") + "/")
            } else {
                None
            };

            let add = if dest_prefix_len > 0 {
                Some(dest_parts[..dest_prefix_len].join("/") + "/")
            } else {
                None
            };

            return (strip, add);
        }

        // For identical patterns, no transformation needed
        if source == dest {
            return (None, None);
        }

        // Complex mapping - just use the destination as-is for matching topics
        (None, None)
    }

    /// Check if a topic matches this rule's source pattern
    fn matches(&self, topic: &str, outbound: bool) -> bool {
        let filter = if outbound {
            &self.local_pattern
        } else {
            &self.remote_pattern
        };
        // topic_matches_filter takes (topic, filter)
        topic_matches_filter(topic, filter)
    }

    /// Transform a topic from source to destination
    fn transform(&self, topic: &str, outbound: bool) -> String {
        // Handle identical patterns
        if self.local_pattern == self.remote_pattern {
            return topic.to_string();
        }

        // Apply prefix transformation
        let mut result = topic.to_string();

        if let Some(ref strip) = self.strip_prefix {
            if let Some(stripped) = result.strip_prefix(strip.as_str()) {
                result = stripped.to_string();
            }
        }

        if let Some(ref add) = self.add_prefix {
            let dest_base = if outbound {
                // For outbound, add the remote prefix
                &self.remote_pattern
            } else {
                // For inbound, add the local prefix
                &self.local_pattern
            };

            // Get the prefix from destination pattern (before any wildcard)
            if let Some(wildcard_pos) = dest_base.find(['#', '+']) {
                let prefix = &dest_base[..wildcard_pos];
                result = format!("{}{}", prefix, result);
            } else {
                result = format!("{}{}", add, result);
            }
        }

        result
    }
}

impl TopicMapper {
    /// Create a new topic mapper from forwarding rules
    pub fn new(rules: &[ForwardRule]) -> Self {
        let outbound_rules: Vec<CompiledRule> = rules
            .iter()
            .filter(|r| r.is_outbound())
            .map(|r| CompiledRule::from_forward_rule(r, true))
            .collect();

        let inbound_rules: Vec<CompiledRule> = rules
            .iter()
            .filter(|r| r.is_inbound())
            .map(|r| CompiledRule::from_forward_rule(r, false))
            .collect();

        Self {
            outbound_rules,
            inbound_rules,
        }
    }

    /// Check if a local topic should be forwarded to the remote broker
    pub fn should_forward_outbound(&self, topic: &str) -> bool {
        self.outbound_rules.iter().any(|r| r.matches(topic, true))
    }

    /// Map a local topic to remote topic for outbound forwarding
    /// Returns (remote_topic, qos, retain) if the topic should be forwarded
    pub fn map_outbound(&self, topic: &str, qos: QoS, retain: bool) -> Option<(String, QoS, bool)> {
        for rule in &self.outbound_rules {
            if rule.matches(topic, true) {
                let remote_topic = rule.transform(topic, true);
                let effective_qos = qos.min(rule.qos);
                let effective_retain = retain && rule.retain;
                return Some((remote_topic, effective_qos, effective_retain));
            }
        }
        None
    }

    /// Check if a remote topic should be forwarded to the local broker
    pub fn should_forward_inbound(&self, topic: &str) -> bool {
        self.inbound_rules.iter().any(|r| r.matches(topic, false))
    }

    /// Map a remote topic to local topic for inbound forwarding
    /// Returns (local_topic, qos, retain) if the topic should be forwarded
    pub fn map_inbound(&self, topic: &str, qos: QoS, retain: bool) -> Option<(String, QoS, bool)> {
        for rule in &self.inbound_rules {
            if rule.matches(topic, false) {
                let local_topic = rule.transform(topic, false);
                let effective_qos = qos.min(rule.qos);
                let effective_retain = retain && rule.retain;
                return Some((local_topic, effective_qos, effective_retain));
            }
        }
        None
    }

    /// Get all remote topic filters for inbound subscriptions
    pub fn inbound_filters(&self) -> Vec<(&str, QoS)> {
        self.inbound_rules
            .iter()
            .map(|r| (r.remote_pattern.as_str(), r.qos))
            .collect()
    }

    /// Get all local topic filters for outbound subscriptions (for hooks)
    pub fn outbound_filters(&self) -> Vec<&str> {
        self.outbound_rules
            .iter()
            .map(|r| r.local_pattern.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ForwardDirection;

    fn make_rule(local: &str, remote: &str, direction: ForwardDirection) -> ForwardRule {
        ForwardRule {
            local_topic: local.to_string(),
            remote_topic: remote.to_string(),
            direction,
            qos: 1,
            retain: true,
        }
    }

    #[test]
    fn test_identical_topics() {
        let rules = vec![make_rule("test/#", "test/#", ForwardDirection::Both)];
        let mapper = TopicMapper::new(&rules);

        assert!(mapper.should_forward_outbound("test/foo"));
        assert!(mapper.should_forward_inbound("test/bar"));

        let (topic, _, _) = mapper
            .map_outbound("test/foo", QoS::AtLeastOnce, false)
            .unwrap();
        assert_eq!(topic, "test/foo");
    }

    #[test]
    fn test_prefix_mapping_outbound() {
        let rules = vec![make_rule(
            "sensors/#",
            "edge/device01/sensors/#",
            ForwardDirection::Out,
        )];
        let mapper = TopicMapper::new(&rules);

        assert!(mapper.should_forward_outbound("sensors/temp"));
        assert!(!mapper.should_forward_inbound("edge/device01/sensors/temp"));

        let (topic, _, _) = mapper
            .map_outbound("sensors/temp", QoS::AtLeastOnce, false)
            .unwrap();
        assert_eq!(topic, "edge/device01/sensors/temp");
    }

    #[test]
    fn test_qos_capping() {
        let mut rule = make_rule("test/#", "test/#", ForwardDirection::Out);
        rule.qos = 0;
        let rules = vec![rule];
        let mapper = TopicMapper::new(&rules);

        let (_, qos, _) = mapper
            .map_outbound("test/foo", QoS::ExactlyOnce, false)
            .unwrap();
        assert_eq!(qos, QoS::AtMostOnce);
    }

    #[test]
    fn test_retain_filtering() {
        let mut rule = make_rule("test/#", "test/#", ForwardDirection::Out);
        rule.retain = false;
        let rules = vec![rule];
        let mapper = TopicMapper::new(&rules);

        let (_, _, retain) = mapper
            .map_outbound("test/foo", QoS::AtLeastOnce, true)
            .unwrap();
        assert!(!retain);
    }

    #[test]
    fn test_inbound_filters() {
        let rules = vec![
            make_rule("local1/#", "remote1/#", ForwardDirection::In),
            make_rule("local2/#", "remote2/#", ForwardDirection::In),
        ];
        let mapper = TopicMapper::new(&rules);

        let filters = mapper.inbound_filters();
        assert_eq!(filters.len(), 2);
        assert!(filters.iter().any(|(f, _)| *f == "remote1/#"));
        assert!(filters.iter().any(|(f, _)| *f == "remote2/#"));
    }
}
