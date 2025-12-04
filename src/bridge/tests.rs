//! Bridge Module Tests

use crate::config::{BridgeConfig, BridgeProtocol, ForwardDirection, ForwardRule, LoopPrevention};
use crate::protocol::QoS;

use super::topic_mapper::TopicMapper;

// =============================================================================
// Configuration Tests
// =============================================================================

#[test]
fn test_bridge_config_defaults() {
    let config = BridgeConfig::default();

    assert_eq!(config.name, "default");
    assert_eq!(config.address, "localhost:1883");
    assert_eq!(config.protocol, BridgeProtocol::Mqtt);
    assert_eq!(config.keepalive, 60);
    assert!(config.clean_start);
    assert!(config.enabled);
    assert_eq!(config.loop_prevention, LoopPrevention::NoLocal);
}

#[test]
fn test_bridge_protocol_defaults() {
    assert_eq!(BridgeProtocol::Mqtt.default_port(), 1883);
    assert_eq!(BridgeProtocol::Mqtts.default_port(), 8883);
    assert_eq!(BridgeProtocol::Ws.default_port(), 80);
    assert_eq!(BridgeProtocol::Wss.default_port(), 443);

    assert!(!BridgeProtocol::Mqtt.uses_tls());
    assert!(BridgeProtocol::Mqtts.uses_tls());
    assert!(!BridgeProtocol::Ws.uses_tls());
    assert!(BridgeProtocol::Wss.uses_tls());

    assert!(!BridgeProtocol::Mqtt.uses_websocket());
    assert!(!BridgeProtocol::Mqtts.uses_websocket());
    assert!(BridgeProtocol::Ws.uses_websocket());
    assert!(BridgeProtocol::Wss.uses_websocket());
}

#[test]
fn test_parse_address_with_port() {
    let config = BridgeConfig {
        address: "broker.example.com:9883".to_string(),
        ..Default::default()
    };
    let (host, port) = config.parse_address();
    assert_eq!(host, "broker.example.com");
    assert_eq!(port, 9883);
}

#[test]
fn test_parse_address_without_port() {
    let config = BridgeConfig {
        address: "broker.example.com".to_string(),
        protocol: BridgeProtocol::Mqtts,
        ..Default::default()
    };
    let (host, port) = config.parse_address();
    assert_eq!(host, "broker.example.com");
    assert_eq!(port, 8883); // Default for mqtts
}

#[test]
fn test_forward_rule_directions() {
    let out_rule = ForwardRule {
        local_topic: "local/#".to_string(),
        remote_topic: "remote/#".to_string(),
        direction: ForwardDirection::Out,
        qos: 1,
        retain: true,
    };
    assert!(out_rule.is_outbound());
    assert!(!out_rule.is_inbound());

    let in_rule = ForwardRule {
        direction: ForwardDirection::In,
        ..out_rule.clone()
    };
    assert!(!in_rule.is_outbound());
    assert!(in_rule.is_inbound());

    let both_rule = ForwardRule {
        direction: ForwardDirection::Both,
        ..out_rule.clone()
    };
    assert!(both_rule.is_outbound());
    assert!(both_rule.is_inbound());
}

#[test]
fn test_loop_prevention_config() {
    let config = BridgeConfig {
        loop_prevention: LoopPrevention::NoLocal,
        ..Default::default()
    };
    assert!(config.use_no_local());
    assert!(!config.use_origin_property());

    let config = BridgeConfig {
        loop_prevention: LoopPrevention::UserProperty,
        ..Default::default()
    };
    assert!(!config.use_no_local());
    assert!(config.use_origin_property());

    let config = BridgeConfig {
        loop_prevention: LoopPrevention::Both,
        ..Default::default()
    };
    assert!(config.use_no_local());
    assert!(config.use_origin_property());

    let config = BridgeConfig {
        loop_prevention: LoopPrevention::None,
        ..Default::default()
    };
    assert!(!config.use_no_local());
    assert!(!config.use_origin_property());
}

#[test]
fn test_origin_id() {
    let config = BridgeConfig {
        name: "my-bridge".to_string(),
        origin_id: None,
        ..Default::default()
    };
    assert_eq!(config.get_origin_id(), "my-bridge");

    let config = BridgeConfig {
        name: "my-bridge".to_string(),
        origin_id: Some("custom-origin".to_string()),
        ..Default::default()
    };
    assert_eq!(config.get_origin_id(), "custom-origin");
}

// =============================================================================
// Topic Mapper Tests
// =============================================================================

fn make_rule(local: &str, remote: &str, direction: ForwardDirection, qos: u8) -> ForwardRule {
    ForwardRule {
        local_topic: local.to_string(),
        remote_topic: remote.to_string(),
        direction,
        qos,
        retain: true,
    }
}

#[test]
fn test_topic_mapper_identical_patterns() {
    let rules = vec![make_rule("test/#", "test/#", ForwardDirection::Both, 1)];
    let mapper = TopicMapper::new(&rules);

    assert!(mapper.should_forward_outbound("test/foo"));
    assert!(mapper.should_forward_outbound("test/foo/bar"));
    assert!(mapper.should_forward_inbound("test/foo"));

    let (topic, qos, _) = mapper
        .map_outbound("test/foo/bar", QoS::AtLeastOnce, false)
        .unwrap();
    assert_eq!(topic, "test/foo/bar");
    assert_eq!(qos, QoS::AtLeastOnce);
}

#[test]
fn test_topic_mapper_prefix_mapping() {
    let rules = vec![make_rule(
        "sensors/#",
        "edge/device01/sensors/#",
        ForwardDirection::Out,
        1,
    )];
    let mapper = TopicMapper::new(&rules);

    assert!(mapper.should_forward_outbound("sensors/temp"));
    assert!(mapper.should_forward_outbound("sensors/humidity/room1"));
    assert!(!mapper.should_forward_outbound("actuators/fan"));
    assert!(!mapper.should_forward_inbound("edge/device01/sensors/temp"));

    let (topic, _, _) = mapper
        .map_outbound("sensors/temp", QoS::AtLeastOnce, false)
        .unwrap();
    assert_eq!(topic, "edge/device01/sensors/temp");
}

#[test]
fn test_topic_mapper_qos_capping() {
    let rules = vec![make_rule("test/#", "test/#", ForwardDirection::Out, 0)];
    let mapper = TopicMapper::new(&rules);

    // QoS should be capped to rule's QoS
    let (_, qos, _) = mapper
        .map_outbound("test/foo", QoS::ExactlyOnce, false)
        .unwrap();
    assert_eq!(qos, QoS::AtMostOnce);
}

#[test]
fn test_topic_mapper_retain_filtering() {
    let rules = vec![ForwardRule {
        local_topic: "test/#".to_string(),
        remote_topic: "test/#".to_string(),
        direction: ForwardDirection::Out,
        qos: 1,
        retain: false,
    }];
    let mapper = TopicMapper::new(&rules);

    // Retain should be filtered out
    let (_, _, retain) = mapper
        .map_outbound("test/foo", QoS::AtLeastOnce, true)
        .unwrap();
    assert!(!retain);
}

#[test]
fn test_topic_mapper_no_match() {
    let rules = vec![make_rule(
        "sensors/#",
        "sensors/#",
        ForwardDirection::Out,
        1,
    )];
    let mapper = TopicMapper::new(&rules);

    // Should not match unrelated topics
    assert!(!mapper.should_forward_outbound("actuators/fan"));
    assert!(mapper
        .map_outbound("actuators/fan", QoS::AtLeastOnce, false)
        .is_none());
}

#[test]
fn test_topic_mapper_inbound_rules() {
    let rules = vec![
        make_rule("local1/#", "remote1/#", ForwardDirection::In, 1),
        make_rule("local2/#", "remote2/#", ForwardDirection::In, 2),
    ];
    let mapper = TopicMapper::new(&rules);

    let filters = mapper.inbound_filters();
    assert_eq!(filters.len(), 2);
    assert!(filters.iter().any(|(f, _)| *f == "remote1/#"));
    assert!(filters.iter().any(|(f, _)| *f == "remote2/#"));
}

#[test]
fn test_topic_mapper_outbound_filters() {
    let rules = vec![
        make_rule("sensors/#", "remote/sensors/#", ForwardDirection::Out, 1),
        make_rule("status/#", "remote/status/#", ForwardDirection::Out, 1),
    ];
    let mapper = TopicMapper::new(&rules);

    let filters = mapper.outbound_filters();
    assert_eq!(filters.len(), 2);
    assert!(filters.contains(&"sensors/#"));
    assert!(filters.contains(&"status/#"));
}

#[test]
fn test_topic_mapper_single_level_wildcard() {
    let rules = vec![make_rule(
        "sensors/+/temperature",
        "remote/+/temp",
        ForwardDirection::Out,
        1,
    )];
    let mapper = TopicMapper::new(&rules);

    assert!(mapper.should_forward_outbound("sensors/kitchen/temperature"));
    assert!(mapper.should_forward_outbound("sensors/bedroom/temperature"));
    assert!(!mapper.should_forward_outbound("sensors/kitchen/humidity"));
    assert!(!mapper.should_forward_outbound("sensors/floor1/room1/temperature"));
}

#[test]
fn test_topic_mapper_bidirectional() {
    let rules = vec![make_rule("shared/#", "shared/#", ForwardDirection::Both, 1)];
    let mapper = TopicMapper::new(&rules);

    // Should forward in both directions
    assert!(mapper.should_forward_outbound("shared/data"));
    assert!(mapper.should_forward_inbound("shared/data"));

    // Inbound filters should include the remote pattern
    let inbound = mapper.inbound_filters();
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0].0, "shared/#");
}

#[test]
fn test_topic_mapper_multiple_rules() {
    let rules = vec![
        make_rule("sensors/#", "edge/sensors/#", ForwardDirection::Out, 1),
        make_rule("commands/#", "edge/commands/#", ForwardDirection::In, 1),
        make_rule("sync/#", "sync/#", ForwardDirection::Both, 2),
    ];
    let mapper = TopicMapper::new(&rules);

    // Outbound
    assert!(mapper.should_forward_outbound("sensors/temp"));
    assert!(mapper.should_forward_outbound("sync/data"));
    assert!(!mapper.should_forward_outbound("commands/start"));

    // Inbound
    assert!(mapper.should_forward_inbound("edge/commands/start"));
    assert!(mapper.should_forward_inbound("sync/data"));
    assert!(!mapper.should_forward_inbound("edge/sensors/temp"));
}

// =============================================================================
// Config Deserialization Tests
// =============================================================================

#[test]
fn test_bridge_config_toml_parsing() {
    let toml_str = r#"
        name = "cloud"
        address = "cloud.example.com:8883"
        protocol = "mqtts"
        client_id = "edge-01"
        username = "bridge"
        password = "secret"
        keepalive = 30
        clean_start = false
        loop_prevention = "both"

        [[forwards]]
        local_topic = "sensors/#"
        remote_topic = "edge/sensors/#"
        direction = "out"
        qos = 1

        [[forwards]]
        local_topic = "commands/#"
        remote_topic = "cloud/commands/#"
        direction = "in"
        qos = 2
    "#;

    let config: BridgeConfig = toml::from_str(toml_str).unwrap();

    assert_eq!(config.name, "cloud");
    assert_eq!(config.address, "cloud.example.com:8883");
    assert_eq!(config.protocol, BridgeProtocol::Mqtts);
    assert_eq!(config.client_id, "edge-01");
    assert_eq!(config.username, Some("bridge".to_string()));
    assert_eq!(config.password, Some("secret".to_string()));
    assert_eq!(config.keepalive, 30);
    assert!(!config.clean_start);
    assert_eq!(config.loop_prevention, LoopPrevention::Both);
    assert_eq!(config.forwards.len(), 2);

    assert_eq!(config.forwards[0].local_topic, "sensors/#");
    assert_eq!(config.forwards[0].remote_topic, "edge/sensors/#");
    assert_eq!(config.forwards[0].direction, ForwardDirection::Out);
    assert_eq!(config.forwards[0].qos, 1);

    assert_eq!(config.forwards[1].local_topic, "commands/#");
    assert_eq!(config.forwards[1].direction, ForwardDirection::In);
    assert_eq!(config.forwards[1].qos, 2);
}

#[test]
fn test_bridge_config_toml_minimal() {
    let toml_str = r#"
        name = "simple"
        address = "localhost"

        [[forwards]]
        local_topic = "test/#"
        remote_topic = "test/#"
    "#;

    let config: BridgeConfig = toml::from_str(toml_str).unwrap();

    assert_eq!(config.name, "simple");
    assert_eq!(config.address, "localhost");
    assert_eq!(config.protocol, BridgeProtocol::Mqtt); // Default
    assert_eq!(config.keepalive, 60); // Default
    assert!(config.clean_start); // Default
    assert_eq!(config.forwards.len(), 1);
    assert_eq!(config.forwards[0].direction, ForwardDirection::Out); // Default
    assert_eq!(config.forwards[0].qos, 1); // Default
}
