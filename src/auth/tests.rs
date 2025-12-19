//! Auth module tests

use super::*;
use crate::config::{AuthConfig, UserConfig};

fn make_auth_config(enabled: bool, allow_anonymous: bool, users: Vec<UserConfig>) -> AuthConfig {
    AuthConfig {
        enabled,
        allow_anonymous,
        users,
    }
}

fn make_user_plaintext(username: &str, password: &str, role: Option<&str>) -> UserConfig {
    UserConfig {
        username: username.to_string(),
        password: Some(password.to_string()),
        password_hash: None,
        role: role.map(|s| s.to_string()),
    }
}

fn make_user_hashed(username: &str, password_hash: &str, role: Option<&str>) -> UserConfig {
    UserConfig {
        username: username.to_string(),
        password: None,
        password_hash: Some(password_hash.to_string()),
        role: role.map(|s| s.to_string()),
    }
}

#[tokio::test]
async fn test_auth_disabled_allows_all() {
    let config = make_auth_config(false, false, vec![]);
    let provider = AuthProvider::new(&config);

    let result = provider
        .on_authenticate("client1", Some("user"), Some(b"pass"))
        .await
        .unwrap();
    assert!(result, "Should allow when auth is disabled");
}

#[tokio::test]
async fn test_auth_enabled_rejects_unknown_user() {
    let config = make_auth_config(
        true,
        false,
        vec![make_user_plaintext("admin", "secret", None)],
    );
    let provider = AuthProvider::new(&config);

    let result = provider
        .on_authenticate("client1", Some("unknown"), Some(b"pass"))
        .await
        .unwrap();
    assert!(!result, "Should reject unknown user");
}

#[tokio::test]
async fn test_auth_enabled_rejects_wrong_password() {
    let config = make_auth_config(
        true,
        false,
        vec![make_user_plaintext("admin", "secret", None)],
    );
    let provider = AuthProvider::new(&config);

    let result = provider
        .on_authenticate("client1", Some("admin"), Some(b"wrong"))
        .await
        .unwrap();
    assert!(!result, "Should reject wrong password");
}

#[tokio::test]
async fn test_auth_enabled_accepts_valid_credentials() {
    let config = make_auth_config(
        true,
        false,
        vec![make_user_plaintext("admin", "secret", None)],
    );
    let provider = AuthProvider::new(&config);

    let result = provider
        .on_authenticate("client1", Some("admin"), Some(b"secret"))
        .await
        .unwrap();
    assert!(result, "Should accept valid credentials");
}

#[tokio::test]
async fn test_anonymous_allowed() {
    let config = make_auth_config(true, true, vec![]);
    let provider = AuthProvider::new(&config);

    let result = provider
        .on_authenticate("client1", None, None)
        .await
        .unwrap();
    assert!(result, "Should allow anonymous when configured");
}

#[tokio::test]
async fn test_anonymous_rejected() {
    let config = make_auth_config(true, false, vec![]);
    let provider = AuthProvider::new(&config);

    let result = provider
        .on_authenticate("client1", None, None)
        .await
        .unwrap();
    assert!(!result, "Should reject anonymous when not allowed");
}

#[tokio::test]
async fn test_client_username_tracking() {
    let config = make_auth_config(
        true,
        false,
        vec![make_user_plaintext("admin", "secret", Some("admin_role"))],
    );
    let provider = AuthProvider::new(&config);

    // Authenticate
    let _ = provider
        .on_authenticate("client1", Some("admin"), Some(b"secret"))
        .await
        .unwrap();

    // Check username is tracked
    assert_eq!(
        provider.get_client_username("client1"),
        Some("admin".to_string())
    );

    // Disconnect
    provider.on_client_disconnected("client1", true).await;

    // Check username is removed
    assert_eq!(provider.get_client_username("client1"), None);
}

#[test]
fn test_get_user_role() {
    let config = make_auth_config(
        true,
        false,
        vec![make_user_plaintext("admin", "secret", Some("admin_role"))],
    );
    let provider = AuthProvider::new(&config);

    assert_eq!(provider.get_user_role("admin"), Some("admin_role"));
    assert_eq!(provider.get_user_role("unknown"), None);
}

// Argon2 hash for "secret" generated with default params
const TEST_ARGON2_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$3QUugnyLZGsTrETNoga03Q$Tnmpw8w1t/PzI36MTps259IB7ntGAb4NA0KlYD9Yzlw";

#[tokio::test]
async fn test_password_hash_accepts_valid() {
    let config = make_auth_config(
        true,
        false,
        vec![make_user_hashed("admin", TEST_ARGON2_HASH, None)],
    );
    let provider = AuthProvider::new(&config);

    let result = provider
        .on_authenticate("client1", Some("admin"), Some(b"secret"))
        .await
        .unwrap();
    assert!(result, "Should accept valid password against hash");
}

#[tokio::test]
async fn test_password_hash_rejects_wrong_password() {
    let config = make_auth_config(
        true,
        false,
        vec![make_user_hashed("admin", TEST_ARGON2_HASH, None)],
    );
    let provider = AuthProvider::new(&config);

    let result = provider
        .on_authenticate("client1", Some("admin"), Some(b"wrong"))
        .await
        .unwrap();
    assert!(!result, "Should reject wrong password against hash");
}

#[tokio::test]
async fn test_mixed_auth_methods() {
    let config = make_auth_config(
        true,
        false,
        vec![
            make_user_plaintext("plain_user", "plainpass", None),
            make_user_hashed("hash_user", TEST_ARGON2_HASH, None),
        ],
    );
    let provider = AuthProvider::new(&config);

    // Plaintext user should work
    let result = provider
        .on_authenticate("client1", Some("plain_user"), Some(b"plainpass"))
        .await
        .unwrap();
    assert!(result, "Plaintext user should authenticate");

    // Hashed user should work
    let result = provider
        .on_authenticate("client2", Some("hash_user"), Some(b"secret"))
        .await
        .unwrap();
    assert!(result, "Hashed user should authenticate");
}
