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
        vec![UserConfig {
            username: "admin".to_string(),
            password: "secret".to_string(),
            role: None,
        }],
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
        vec![UserConfig {
            username: "admin".to_string(),
            password: "secret".to_string(),
            role: None,
        }],
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
        vec![UserConfig {
            username: "admin".to_string(),
            password: "secret".to_string(),
            role: None,
        }],
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
        vec![UserConfig {
            username: "admin".to_string(),
            password: "secret".to_string(),
            role: Some("admin_role".to_string()),
        }],
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
        vec![UserConfig {
            username: "admin".to_string(),
            password: "secret".to_string(),
            role: Some("admin_role".to_string()),
        }],
    );
    let provider = AuthProvider::new(&config);

    assert_eq!(provider.get_user_role("admin"), Some("admin_role"));
    assert_eq!(provider.get_user_role("unknown"), None);
}
