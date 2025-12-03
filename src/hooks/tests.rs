//! Hooks module tests

use super::*;

#[tokio::test]
async fn test_default_hooks_allow_all() {
    let hooks = DefaultHooks;

    // Test authentication
    let result = hooks
        .on_authenticate("client1", Some("user"), Some(b"pass"))
        .await
        .unwrap();
    assert!(result, "DefaultHooks should allow authentication");

    // Test publish
    let result = hooks
        .on_publish_check(
            "client1",
            Some("user"),
            "test/topic",
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();
    assert!(result, "DefaultHooks should allow publish");

    // Test subscribe
    let result = hooks
        .on_subscribe_check("client1", Some("user"), "test/#", QoS::AtLeastOnce)
        .await
        .unwrap();
    assert!(result, "DefaultHooks should allow subscribe");
}

struct AllowHooks;
struct DenyHooks;

#[async_trait]
impl Hooks for AllowHooks {
    async fn on_authenticate(
        &self,
        _client_id: &str,
        _username: Option<&str>,
        _password: Option<&[u8]>,
    ) -> HookResult<bool> {
        Ok(true)
    }

    async fn on_publish_check(
        &self,
        _client_id: &str,
        _username: Option<&str>,
        _topic: &str,
        _qos: QoS,
        _retain: bool,
    ) -> HookResult<bool> {
        Ok(true)
    }

    async fn on_subscribe_check(
        &self,
        _client_id: &str,
        _username: Option<&str>,
        _filter: &str,
        _qos: QoS,
    ) -> HookResult<bool> {
        Ok(true)
    }
}

#[async_trait]
impl Hooks for DenyHooks {
    async fn on_authenticate(
        &self,
        _client_id: &str,
        _username: Option<&str>,
        _password: Option<&[u8]>,
    ) -> HookResult<bool> {
        Ok(false)
    }

    async fn on_publish_check(
        &self,
        _client_id: &str,
        _username: Option<&str>,
        _topic: &str,
        _qos: QoS,
        _retain: bool,
    ) -> HookResult<bool> {
        Ok(false)
    }

    async fn on_subscribe_check(
        &self,
        _client_id: &str,
        _username: Option<&str>,
        _filter: &str,
        _qos: QoS,
    ) -> HookResult<bool> {
        Ok(false)
    }
}

#[tokio::test]
async fn test_composite_hooks_all_must_allow() {
    let hooks = CompositeHooks::new().with(AllowHooks).with(AllowHooks);

    let result = hooks
        .on_authenticate("client1", Some("user"), Some(b"pass"))
        .await
        .unwrap();
    assert!(result, "Both hooks allow, should be allowed");
}

#[tokio::test]
async fn test_composite_hooks_one_deny_fails() {
    let hooks = CompositeHooks::new().with(AllowHooks).with(DenyHooks);

    let result = hooks
        .on_authenticate("client1", Some("user"), Some(b"pass"))
        .await
        .unwrap();
    assert!(!result, "One hook denies, should be denied");
}

#[tokio::test]
async fn test_composite_hooks_publish_check() {
    let hooks = CompositeHooks::new().with(AllowHooks).with(DenyHooks);

    let result = hooks
        .on_publish_check(
            "client1",
            Some("user"),
            "test/topic",
            QoS::AtMostOnce,
            false,
        )
        .await
        .unwrap();
    assert!(!result, "One hook denies publish, should be denied");
}

#[tokio::test]
async fn test_composite_hooks_subscribe_check() {
    let hooks = CompositeHooks::new().with(AllowHooks).with(DenyHooks);

    let result = hooks
        .on_subscribe_check("client1", Some("user"), "test/#", QoS::AtLeastOnce)
        .await
        .unwrap();
    assert!(!result, "One hook denies subscribe, should be denied");
}

#[tokio::test]
async fn test_hook_error_display() {
    let internal = HookError::Internal("test error".to_string());
    assert_eq!(format!("{}", internal), "Internal error: test error");

    let auth_failed = HookError::AuthenticationFailed;
    assert_eq!(format!("{}", auth_failed), "Authentication failed");

    let auth_denied = HookError::AuthorizationDenied;
    assert_eq!(format!("{}", auth_denied), "Authorization denied");
}
