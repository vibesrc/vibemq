//! ACL module tests

use super::*;
use crate::config::{AclConfig, AclPermissions, AclRole, AuthConfig, UserConfig};
use std::sync::Arc;

fn make_test_auth_provider() -> Arc<AuthProvider> {
    let auth_config = AuthConfig {
        enabled: true,
        allow_anonymous: false,
        users: vec![
            UserConfig {
                username: "admin".to_string(),
                password: Some("admin_pass".to_string()),
                password_hash: None,
                role: Some("admin".to_string()),
            },
            UserConfig {
                username: "sensor".to_string(),
                password: Some("sensor_pass".to_string()),
                password_hash: None,
                role: Some("device".to_string()),
            },
            UserConfig {
                username: "readonly".to_string(),
                password: Some("readonly_pass".to_string()),
                password_hash: None,
                role: Some("reader".to_string()),
            },
        ],
    };
    Arc::new(AuthProvider::new(&auth_config))
}

fn make_test_acl_config() -> AclConfig {
    AclConfig {
        enabled: true,
        roles: vec![
            AclRole {
                name: "admin".to_string(),
                publish: vec!["#".to_string()],
                subscribe: vec!["#".to_string()],
            },
            AclRole {
                name: "device".to_string(),
                publish: vec!["sensors/%c/#".to_string()],
                subscribe: vec!["commands/%c/#".to_string()],
            },
            AclRole {
                name: "reader".to_string(),
                publish: vec![],
                subscribe: vec!["sensors/#".to_string()],
            },
        ],
        default: AclPermissions {
            publish: vec![],
            subscribe: vec!["$SYS/broker/+".to_string()],
        },
    }
}

#[tokio::test]
async fn test_acl_disabled_allows_all() {
    let auth_provider = make_test_auth_provider();
    let acl_config = AclConfig {
        enabled: false,
        roles: vec![],
        default: AclPermissions::default(),
    };
    let provider = AclProvider::new(&acl_config, auth_provider);

    let result = provider
        .on_publish_check(
            "client1",
            Some("anyone"),
            "any/topic",
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();
    assert!(result, "Should allow when ACL is disabled");
}

#[tokio::test]
async fn test_admin_can_publish_anywhere() {
    let auth_provider = make_test_auth_provider();
    // Simulate authentication
    auth_provider
        .on_authenticate("admin_client", Some("admin"), Some(b"admin_pass"))
        .await
        .unwrap();

    let acl_config = make_test_acl_config();
    let provider = AclProvider::new(&acl_config, auth_provider);

    let result = provider
        .on_publish_check(
            "admin_client",
            Some("admin"),
            "any/topic/here",
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();
    assert!(result, "Admin should be able to publish anywhere");
}

#[tokio::test]
async fn test_device_can_publish_to_own_topic() {
    let auth_provider = make_test_auth_provider();
    // Simulate authentication
    auth_provider
        .on_authenticate("sensor_client", Some("sensor"), Some(b"sensor_pass"))
        .await
        .unwrap();

    let acl_config = make_test_acl_config();
    let provider = AclProvider::new(&acl_config, auth_provider);

    // Device can publish to sensors/{client_id}/#
    let result = provider
        .on_publish_check(
            "sensor_client",
            Some("sensor"),
            "sensors/sensor_client/temperature",
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();
    assert!(result, "Device should publish to its own topic");

    // Device cannot publish to other topic
    let result = provider
        .on_publish_check(
            "sensor_client",
            Some("sensor"),
            "sensors/other_client/temperature",
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();
    assert!(!result, "Device should NOT publish to other's topic");
}

#[tokio::test]
async fn test_readonly_cannot_publish() {
    let auth_provider = make_test_auth_provider();
    // Simulate authentication
    auth_provider
        .on_authenticate("reader_client", Some("readonly"), Some(b"readonly_pass"))
        .await
        .unwrap();

    let acl_config = make_test_acl_config();
    let provider = AclProvider::new(&acl_config, auth_provider);

    let result = provider
        .on_publish_check(
            "reader_client",
            Some("readonly"),
            "sensors/temp",
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();
    assert!(!result, "Readonly user should NOT be able to publish");
}

#[tokio::test]
async fn test_readonly_can_subscribe_to_sensors() {
    let auth_provider = make_test_auth_provider();
    // Simulate authentication
    auth_provider
        .on_authenticate("reader_client", Some("readonly"), Some(b"readonly_pass"))
        .await
        .unwrap();

    let acl_config = make_test_acl_config();
    let provider = AclProvider::new(&acl_config, auth_provider);

    let result = provider
        .on_subscribe_check(
            "reader_client",
            Some("readonly"),
            "sensors/temperature",
            QoS::AtMostOnce,
        )
        .await
        .unwrap();
    assert!(result, "Readonly user should subscribe to sensors");

    // Cannot subscribe to commands
    let result = provider
        .on_subscribe_check(
            "reader_client",
            Some("readonly"),
            "commands/device1",
            QoS::AtMostOnce,
        )
        .await
        .unwrap();
    assert!(!result, "Readonly user should NOT subscribe to commands");
}

#[test]
fn test_pattern_matching() {
    // Exact match
    assert!(AclProvider::mqtt_pattern_match("foo/bar", "foo/bar"));
    assert!(!AclProvider::mqtt_pattern_match("foo/bar", "foo/baz"));

    // Single level wildcard
    assert!(AclProvider::mqtt_pattern_match("foo/+/bar", "foo/xxx/bar"));
    assert!(!AclProvider::mqtt_pattern_match("foo/+/bar", "foo/xxx/baz"));

    // Multi level wildcard
    assert!(AclProvider::mqtt_pattern_match("foo/#", "foo/bar/baz"));
    assert!(AclProvider::mqtt_pattern_match("#", "any/topic/here"));
}

#[test]
fn test_variable_substitution() {
    // %c substitution
    assert!(AclProvider::matches_pattern(
        "sensors/%c/data",
        "sensors/client1/data",
        "client1",
        None
    ));
    assert!(!AclProvider::matches_pattern(
        "sensors/%c/data",
        "sensors/other/data",
        "client1",
        None
    ));

    // %u substitution
    assert!(AclProvider::matches_pattern(
        "users/%u/inbox",
        "users/admin/inbox",
        "client1",
        Some("admin")
    ));
}
