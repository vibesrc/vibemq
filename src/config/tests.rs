//! Config module tests

use super::*;

#[test]
fn test_substitute_env_vars_simple() {
    std::env::set_var("TEST_VAR_SIMPLE", "hello");
    let result = substitute_env_vars("value = \"${TEST_VAR_SIMPLE}\"");
    assert_eq!(result, "value = \"hello\"");
    std::env::remove_var("TEST_VAR_SIMPLE");
}

#[test]
fn test_substitute_env_vars_with_default() {
    // Unset var should use default
    std::env::remove_var("TEST_VAR_UNSET");
    let result = substitute_env_vars("value = \"${TEST_VAR_UNSET:-default_value}\"");
    assert_eq!(result, "value = \"default_value\"");

    // Set var should use env value
    std::env::set_var("TEST_VAR_SET", "env_value");
    let result = substitute_env_vars("value = \"${TEST_VAR_SET:-default_value}\"");
    assert_eq!(result, "value = \"env_value\"");
    std::env::remove_var("TEST_VAR_SET");
}

#[test]
fn test_substitute_env_vars_multiple() {
    std::env::set_var("TEST_HOST", "localhost");
    std::env::set_var("TEST_PORT", "1883");
    let result = substitute_env_vars("bind = \"${TEST_HOST}:${TEST_PORT}\"");
    assert_eq!(result, "bind = \"localhost:1883\"");
    std::env::remove_var("TEST_HOST");
    std::env::remove_var("TEST_PORT");
}

#[test]
fn test_substitute_env_vars_missing_no_default() {
    std::env::remove_var("TEST_VAR_MISSING");
    let result = substitute_env_vars("value = \"${TEST_VAR_MISSING}\"");
    assert_eq!(result, "value = \"\"");
}

#[test]
fn test_load_config_with_env_substitution() {
    // Create a temp config file with env var references
    let temp_dir = std::env::temp_dir();
    let config_path = temp_dir.join("vibemq_test_config.toml");

    std::env::set_var("TEST_BIND_HOST", "127.0.0.1");
    std::env::set_var("TEST_BIND_PORT", "1885");

    let config_content = r#"
[server]
bind = "${TEST_BIND_HOST}:${TEST_BIND_PORT}"
workers = ${TEST_WORKERS:-4}
"#;

    std::fs::write(&config_path, config_content).unwrap();

    let config = Config::load(&config_path).unwrap();
    assert_eq!(config.server.bind.to_string(), "127.0.0.1:1885");
    assert_eq!(config.server.workers, 4); // Uses default

    // Cleanup
    std::fs::remove_file(&config_path).ok();
    std::env::remove_var("TEST_BIND_HOST");
    std::env::remove_var("TEST_BIND_PORT");
}

#[test]
fn test_default_config() {
    let config = Config::default();
    assert_eq!(config.server.bind.port(), 1883);
    assert_eq!(config.limits.max_connections, 100_000);
    assert_eq!(config.limits.max_inflight, 32);
    assert_eq!(config.mqtt.max_qos, 2);
    assert!(!config.auth.enabled);
    assert!(!config.acl.enabled);
}

#[test]
fn test_parse_minimal_config() {
    let toml = r#"
[server]
bind = "127.0.0.1:1883"
"#;

    let config = Config::parse(toml).unwrap();
    assert_eq!(config.server.bind.to_string(), "127.0.0.1:1883");
}

#[test]
fn test_parse_full_config() {
    let toml = r##"
[server]
bind = "0.0.0.0:1883"
ws_bind = "0.0.0.0:9001"
workers = 4

[limits]
max_connections = 50000
max_packet_size = 1048576
max_inflight = 16
max_queued_messages = 500
max_awaiting_rel = 50
retry_interval = 20

[session]
default_keep_alive = 30
max_keep_alive = 300
expiry_check_interval = 30
max_topic_aliases = 100

[mqtt]
max_qos = 2
retain_available = true
wildcard_subscriptions = true
subscription_identifiers = true
shared_subscriptions = false

[auth]
enabled = true
allow_anonymous = false

[[auth.users]]
username = "admin"
password = "secret123"
role = "admin"

[[auth.users]]
username = "sensor1"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$3QUugnyLZGsTrETNoga03Q$Tnmpw8w1t/PzI36MTps259IB7ntGAb4NA0KlYD9Yzlw"
role = "device"

[acl]
enabled = true

[[acl.roles]]
name = "admin"
publish = ["#"]
subscribe = ["#"]

[[acl.roles]]
name = "device"
publish = ["sensors/%c/#"]
subscribe = ["commands/%c/#"]

[acl.default]
publish = []
subscribe = ["$SYS/broker/+"]
"##;

    let config = Config::parse(toml).unwrap();
    assert_eq!(config.server.workers, 4);
    assert_eq!(config.limits.max_connections, 50000);
    assert_eq!(config.limits.max_inflight, 16);
    assert!(config.auth.enabled);
    assert!(!config.auth.allow_anonymous);
    assert_eq!(config.auth.users.len(), 2);
    assert_eq!(config.auth.users[0].username, "admin");
    assert_eq!(config.auth.users[0].password, Some("secret123".to_string()));
    assert_eq!(config.auth.users[1].username, "sensor1");
    assert!(config.auth.users[1].password_hash.is_some());
    assert!(config.acl.enabled);
    assert_eq!(config.acl.roles.len(), 2);
}

#[test]
fn test_invalid_max_qos() {
    let toml = r#"
[mqtt]
max_qos = 3
"#;

    let result = Config::parse(toml);
    assert!(result.is_err());
}

#[test]
fn test_invalid_max_inflight() {
    let toml = r#"
[limits]
max_inflight = 0
"#;

    let result = Config::parse(toml);
    assert!(result.is_err());
}

#[test]
fn test_invalid_acl_role_reference() {
    let toml = r#"
[auth]
enabled = true

[[auth.users]]
username = "admin"
password = "secret"
role = "nonexistent_role"

[acl]
enabled = true
"#;

    let result = Config::parse(toml);
    assert!(result.is_err());
}

#[test]
fn test_build_role_map() {
    let toml = r##"
[acl]
enabled = true

[[acl.roles]]
name = "admin"
publish = ["#"]
subscribe = ["#"]

[[acl.roles]]
name = "device"
publish = ["sensors/#"]
subscribe = ["commands/#"]
"##;

    let config = Config::parse(toml).unwrap();
    let role_map = config.build_role_map();
    assert!(role_map.contains_key("admin"));
    assert!(role_map.contains_key("device"));
    assert_eq!(role_map.get("admin").unwrap().publish, vec!["#"]);
}

#[test]
fn test_build_user_map() {
    let toml = r#"
[auth]
enabled = true

[[auth.users]]
username = "alice"
password = "pass1"

[[auth.users]]
username = "bob"
password = "pass2"
"#;

    let config = Config::parse(toml).unwrap();
    let user_map = config.build_user_map();
    assert!(user_map.contains_key("alice"));
    assert!(user_map.contains_key("bob"));
    assert_eq!(
        user_map.get("alice").unwrap().password,
        Some("pass1".to_string())
    );
}

#[test]
fn test_user_missing_password_and_hash() {
    let toml = r#"
[auth]
enabled = true

[[auth.users]]
username = "admin"
"#;

    let result = Config::parse(toml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("must have either 'password' or 'password_hash'"));
}

#[test]
fn test_user_both_password_and_hash() {
    let toml = r#"
[auth]
enabled = true

[[auth.users]]
username = "admin"
password = "plaintext"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$3QUugnyLZGsTrETNoga03Q$Tnmpw8w1t/PzI36MTps259IB7ntGAb4NA0KlYD9Yzlw"
"#;

    let result = Config::parse(toml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("cannot have both"));
}

#[test]
fn test_user_invalid_hash_format() {
    let toml = r#"
[auth]
enabled = true

[[auth.users]]
username = "admin"
password_hash = "not-a-valid-hash"
"#;

    let result = Config::parse(toml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("invalid password_hash format"));
}

#[test]
fn test_user_empty_password() {
    let toml = r#"
[auth]
enabled = true

[[auth.users]]
username = "admin"
password = ""
"#;

    let result = Config::parse(toml);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("empty password"));
}

#[test]
fn test_user_valid_password_hash() {
    let toml = r#"
[auth]
enabled = true

[[auth.users]]
username = "admin"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$3QUugnyLZGsTrETNoga03Q$Tnmpw8w1t/PzI36MTps259IB7ntGAb4NA0KlYD9Yzlw"
"#;

    let result = Config::parse(toml);
    assert!(result.is_ok());
}

#[test]
fn test_parse_bridge_config() {
    let toml = r##"
[auth]
allow_anonymous = true

[[bridge]]
name = "node1"
address = "node1:1883"
protocol = "mqtt"
client_id = "node2-bridge"
enabled = true
loop_prevention = "no_local"

[[bridge.forwards]]
local_topic = "#"
remote_topic = "#"
direction = "both"
retain = true
qos = 1
"##;

    let config = Config::parse(toml).unwrap();
    assert_eq!(config.bridge.len(), 1);
    assert_eq!(config.bridge[0].name, "node1");
    assert_eq!(config.bridge[0].address, "node1:1883");
    assert!(config.bridge[0].enabled);
    assert_eq!(config.bridge[0].forwards.len(), 1);
    assert_eq!(config.bridge[0].forwards[0].local_topic, "#");
}

#[test]
fn test_load_bridge_config_from_file() {
    let temp_dir = std::env::temp_dir();
    let config_path = temp_dir.join("vibemq_bridge_test.toml");

    let config_content = r##"
[auth]
allow_anonymous = true

[[bridge]]
name = "test-bridge"
address = "remote:1883"
protocol = "mqtt"
client_id = "bridge-client"
enabled = true
loop_prevention = "no_local"

[[bridge.forwards]]
local_topic = "#"
remote_topic = "#"
direction = "both"
retain = true
qos = 1
"##;

    std::fs::write(&config_path, config_content).unwrap();

    let config = Config::load(&config_path).unwrap();

    // Debug: print what we got
    eprintln!("Bridges loaded: {}", config.bridge.len());
    for b in &config.bridge {
        eprintln!(
            "  Bridge: {} -> {} ({} forwards)",
            b.name,
            b.address,
            b.forwards.len()
        );
    }

    assert_eq!(
        config.bridge.len(),
        1,
        "Expected 1 bridge but got {}",
        config.bridge.len()
    );
    assert_eq!(config.bridge[0].name, "test-bridge");
    assert_eq!(config.bridge[0].forwards.len(), 1);

    std::fs::remove_file(&config_path).ok();
}
