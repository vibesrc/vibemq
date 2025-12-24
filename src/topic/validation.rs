//! Topic name and filter validation
//!
//! Based on MQTT specification sections on topic names and topic filters.
//!
//! Key rules:
//! - Topic names MUST NOT contain wildcards (+ or #)
//! - Topic filters MAY contain wildcards
//! - Multi-level wildcard (#) must be the last character and preceded by /
//! - Single-level wildcard (+) must occupy entire level
//! - Topics starting with $ are system topics and have special matching rules

/// Represents a level in a topic
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopicLevel<'a> {
    /// Normal topic level
    Normal(&'a str),
    /// Single-level wildcard (+)
    SingleWildcard,
    /// Multi-level wildcard (#)
    MultiWildcard,
}

/// Parse topic into levels
pub fn parse_levels(topic: &str) -> impl Iterator<Item = TopicLevel<'_>> {
    topic.split('/').map(|level| match level {
        "+" => TopicLevel::SingleWildcard,
        "#" => TopicLevel::MultiWildcard,
        s => TopicLevel::Normal(s),
    })
}

/// Validate a topic name (used in PUBLISH)
///
/// Topic names:
/// - Must be at least 1 character
/// - Must not exceed 65535 bytes
/// - Must not contain null character
/// - Must not contain wildcards (+ or #)
/// - Must not exceed max_topic_levels if set (0 = unlimited)
pub fn validate_topic_name(topic: &str) -> Result<(), &'static str> {
    validate_topic_name_with_max_levels(topic, 0)
}

/// Validate a topic name with configurable max levels
///
/// Topic names:
/// - Must be at least 1 character
/// - Must not exceed 65535 bytes
/// - Must not contain null character
/// - Must not contain wildcards (+ or #)
/// - Must not exceed max_topic_levels if set (0 = unlimited)
pub fn validate_topic_name_with_max_levels(
    topic: &str,
    max_topic_levels: usize,
) -> Result<(), &'static str> {
    if topic.is_empty() {
        return Err("topic name cannot be empty");
    }

    if topic.len() > 65535 {
        return Err("topic name exceeds maximum length");
    }

    if topic.contains('\0') {
        return Err("topic name cannot contain null character");
    }

    if topic.contains('+') || topic.contains('#') {
        return Err("topic name cannot contain wildcards");
    }

    // Check max topic levels (0 = unlimited)
    if max_topic_levels > 0 {
        let level_count = topic.split('/').count();
        if level_count > max_topic_levels {
            return Err("topic exceeds maximum allowed levels");
        }
    }

    Ok(())
}

/// Validate a topic filter (used in SUBSCRIBE/UNSUBSCRIBE)
///
/// Topic filters:
/// - Must be at least 1 character
/// - Must not exceed 65535 bytes
/// - Must not contain null character
/// - Multi-level wildcard (#) must be:
///   - The only character in the filter, OR
///   - Preceded by a level separator (/)
///   - The last character
/// - Single-level wildcard (+) must occupy an entire level
/// - Shared subscriptions ($share/{group}/{filter}) are also supported
/// - Must not exceed max_topic_levels if set (0 = unlimited)
pub fn validate_topic_filter(filter: &str) -> Result<(), &'static str> {
    validate_topic_filter_with_max_levels(filter, 0)
}

/// Validate a topic filter with configurable max levels
///
/// Topic filters:
/// - Must be at least 1 character
/// - Must not exceed 65535 bytes
/// - Must not contain null character
/// - Multi-level wildcard (#) must be:
///   - The only character in the filter, OR
///   - Preceded by a level separator (/)
///   - The last character
/// - Single-level wildcard (+) must occupy an entire level
/// - Shared subscriptions ($share/{group}/{filter}) are also supported
/// - Must not exceed max_topic_levels if set (0 = unlimited)
pub fn validate_topic_filter_with_max_levels(
    filter: &str,
    max_topic_levels: usize,
) -> Result<(), &'static str> {
    if filter.is_empty() {
        return Err("topic filter cannot be empty");
    }

    if filter.len() > 65535 {
        return Err("topic filter exceeds maximum length");
    }

    if filter.contains('\0') {
        return Err("topic filter cannot contain null character");
    }

    // Handle shared subscription format: $share/{group}/{filter}
    let actual_filter = if let Some(rest) = filter.strip_prefix("$share/") {
        // Skip "$share/"
        if let Some(slash_pos) = rest.find('/') {
            let group = &rest[..slash_pos];
            let actual = &rest[slash_pos + 1..];
            if group.is_empty() {
                return Err("shared subscription group name cannot be empty");
            }
            if group.contains('+') || group.contains('#') {
                return Err("shared subscription group name cannot contain wildcards");
            }
            if actual.is_empty() {
                return Err("shared subscription filter cannot be empty");
            }
            actual
        } else {
            return Err("invalid shared subscription format");
        }
    } else {
        filter
    };

    let levels: Vec<&str> = actual_filter.split('/').collect();

    // Check max topic levels (0 = unlimited)
    if max_topic_levels > 0 && levels.len() > max_topic_levels {
        return Err("topic filter exceeds maximum allowed levels");
    }

    for (i, level) in levels.iter().enumerate() {
        if level.contains('#') {
            // # must be the entire level and the last level
            if *level != "#" {
                return Err("multi-level wildcard must occupy entire level");
            }
            if i != levels.len() - 1 {
                return Err("multi-level wildcard must be last level");
            }
        }

        if level.contains('+') {
            // + must be the entire level
            if *level != "+" {
                return Err("single-level wildcard must occupy entire level");
            }
        }
    }

    Ok(())
}

/// Check if a topic filter matches a topic name
///
/// Matching rules:
/// - / is the level separator
/// - + matches exactly one level
/// - # matches zero or more levels (must be last)
/// - $-topics don't match filters starting with + or #
pub fn topic_matches_filter(topic: &str, filter: &str) -> bool {
    // Topics starting with $ don't match filters starting with + or #
    if topic.starts_with('$') && (filter.starts_with('+') || filter.starts_with('#')) {
        return false;
    }

    let topic_levels: Vec<&str> = topic.split('/').collect();
    let filter_levels: Vec<&str> = filter.split('/').collect();

    let mut ti = 0;
    let mut fi = 0;

    while fi < filter_levels.len() {
        let filter_level = filter_levels[fi];

        if filter_level == "#" {
            // # matches everything remaining
            return true;
        }

        if ti >= topic_levels.len() {
            // No more topic levels but filter has more non-# levels
            return false;
        }

        if filter_level == "+" {
            // + matches any single level
            ti += 1;
            fi += 1;
        } else if filter_level == topic_levels[ti] {
            // Exact match
            ti += 1;
            fi += 1;
        } else {
            // No match
            return false;
        }
    }

    // Both must be exhausted for a match
    ti == topic_levels.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_topic_name() {
        assert!(validate_topic_name("test").is_ok());
        assert!(validate_topic_name("test/topic").is_ok());
        assert!(validate_topic_name("/test/topic").is_ok());
        assert!(validate_topic_name("test/topic/").is_ok());

        assert!(validate_topic_name("").is_err());
        assert!(validate_topic_name("test+topic").is_err());
        assert!(validate_topic_name("test#topic").is_err());
        assert!(validate_topic_name("test/+/topic").is_err());
        assert!(validate_topic_name("test/#").is_err());
    }

    #[test]
    fn test_validate_topic_filter() {
        assert!(validate_topic_filter("test").is_ok());
        assert!(validate_topic_filter("test/topic").is_ok());
        assert!(validate_topic_filter("+").is_ok());
        assert!(validate_topic_filter("#").is_ok());
        assert!(validate_topic_filter("test/+").is_ok());
        assert!(validate_topic_filter("test/#").is_ok());
        assert!(validate_topic_filter("+/test").is_ok());
        assert!(validate_topic_filter("+/+/+").is_ok());
        assert!(validate_topic_filter("test/+/topic").is_ok());

        assert!(validate_topic_filter("").is_err());
        assert!(validate_topic_filter("test+").is_err());
        assert!(validate_topic_filter("test#").is_err());
        assert!(validate_topic_filter("test/#/more").is_err());
        assert!(validate_topic_filter("+test").is_err());
    }

    #[test]
    fn test_topic_matches() {
        // Exact matches
        assert!(topic_matches_filter("test", "test"));
        assert!(topic_matches_filter("test/topic", "test/topic"));
        assert!(!topic_matches_filter("test", "test/topic"));
        assert!(!topic_matches_filter("test/topic", "test"));

        // Single-level wildcard
        assert!(topic_matches_filter("test/topic", "test/+"));
        assert!(topic_matches_filter("test/topic", "+/topic"));
        assert!(topic_matches_filter("test/topic", "+/+"));
        assert!(topic_matches_filter("a/b/c", "+/b/+"));
        assert!(!topic_matches_filter("test", "+/+"));
        assert!(!topic_matches_filter("test/topic/extra", "test/+"));

        // Multi-level wildcard
        assert!(topic_matches_filter("test", "#"));
        assert!(topic_matches_filter("test/topic", "#"));
        assert!(topic_matches_filter("test/topic/more", "#"));
        assert!(topic_matches_filter("test/topic", "test/#"));
        assert!(topic_matches_filter("test/topic/more", "test/#"));
        assert!(topic_matches_filter("test", "test/#"));
        assert!(!topic_matches_filter("other/topic", "test/#"));

        // $-topics
        assert!(!topic_matches_filter("$SYS/test", "+/test"));
        assert!(!topic_matches_filter("$SYS/test", "#"));
        assert!(topic_matches_filter("$SYS/test", "$SYS/+"));
        assert!(topic_matches_filter("$SYS/test", "$SYS/#"));
    }

    #[test]
    fn test_validate_topic_name_max_levels() {
        // 0 = unlimited (no limit enforced)
        assert!(validate_topic_name_with_max_levels("a/b/c/d/e", 0).is_ok());

        // Within limit
        assert!(validate_topic_name_with_max_levels("a", 3).is_ok());
        assert!(validate_topic_name_with_max_levels("a/b", 3).is_ok());
        assert!(validate_topic_name_with_max_levels("a/b/c", 3).is_ok());

        // Exceeds limit
        assert!(validate_topic_name_with_max_levels("a/b/c/d", 3).is_err());
        assert!(validate_topic_name_with_max_levels("a/b/c/d/e", 3).is_err());

        // Edge cases
        assert!(validate_topic_name_with_max_levels("a", 1).is_ok());
        assert!(validate_topic_name_with_max_levels("a/b", 1).is_err());

        // Empty levels still count
        assert!(validate_topic_name_with_max_levels("/a/b", 3).is_ok()); // 3 levels: "", "a", "b"
        assert!(validate_topic_name_with_max_levels("/a/b/c", 3).is_err()); // 4 levels: "", "a", "b", "c"
    }

    #[test]
    fn test_validate_topic_filter_max_levels() {
        // 0 = unlimited (no limit enforced)
        assert!(validate_topic_filter_with_max_levels("a/b/c/d/e", 0).is_ok());
        assert!(validate_topic_filter_with_max_levels("a/+/c/#", 0).is_ok());

        // Within limit
        assert!(validate_topic_filter_with_max_levels("a", 3).is_ok());
        assert!(validate_topic_filter_with_max_levels("a/b", 3).is_ok());
        assert!(validate_topic_filter_with_max_levels("a/b/c", 3).is_ok());
        assert!(validate_topic_filter_with_max_levels("a/+/c", 3).is_ok());
        assert!(validate_topic_filter_with_max_levels("a/b/#", 3).is_ok());

        // Exceeds limit
        assert!(validate_topic_filter_with_max_levels("a/b/c/d", 3).is_err());
        assert!(validate_topic_filter_with_max_levels("a/+/c/d", 3).is_err());
        assert!(validate_topic_filter_with_max_levels("a/b/c/#", 3).is_err()); // # counts as a level

        // Edge cases
        assert!(validate_topic_filter_with_max_levels("#", 1).is_ok());
        assert!(validate_topic_filter_with_max_levels("+", 1).is_ok());
        assert!(validate_topic_filter_with_max_levels("a/#", 1).is_err());

        // Shared subscriptions - only the actual filter part counts
        assert!(validate_topic_filter_with_max_levels("$share/group/a/b/c", 3).is_ok());
        assert!(validate_topic_filter_with_max_levels("$share/group/a/b/c/d", 3).is_err());
    }
}
