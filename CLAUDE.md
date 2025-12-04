# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

```bash
# Build
cargo build                    # Debug build
cargo build --release          # Release build (optimized, LTO enabled)
cargo build --features tls     # Build with TLS support

# Run
cargo run                                # Run broker with defaults (0.0.0.0:1883)
cargo run -- -c config.toml              # Run with config file
cargo run -- -b 0.0.0.0:1884 -l debug    # Custom bind address and log level

# Test
cargo test                     # Run all tests
cargo test --test integration  # Run integration tests only
cargo test --test conformance  # Run MQTT conformance tests
cargo test <test_name>         # Run a specific test

# Lint and Format
cargo fmt                      # Format code
cargo clippy                   # Run linter
```

## Architecture Overview

VibeMQ is a high-performance MQTT v3.1.1/v5.0 broker built with Tokio for async I/O.

### Core Modules

- **`broker/`** - Main broker orchestration. `Broker` manages connections, sessions, subscriptions, and retained messages. `Connection` handles individual client TCP/WebSocket streams.
- **`protocol/`** - MQTT packet definitions (`Packet`, `Connect`, `Publish`, etc.), `QoS` levels, `ProtocolVersion`, `ReasonCode`, and `Properties` for v5.0.
- **`codec/`** - `Encoder` and `Decoder` for MQTT packet serialization. Handles both v3.1.1 and v5.0 wire formats.
- **`session/`** - `Session` tracks client state (subscriptions, inflight messages, packet IDs). `SessionStore` provides thread-safe session management with DashMap.
- **`topic/`** - `SubscriptionStore` uses a topic trie for efficient wildcard matching. Supports `+` (single-level) and `#` (multi-level) wildcards, plus shared subscriptions (`$share/{group}/{filter}`).
- **`hooks/`** - Extensibility via `Hooks` trait for auth, ACL, and event handling. `CompositeHooks` chains multiple implementations.
- **`auth/`** - `AuthProvider` implements `Hooks` for username/password authentication.
- **`acl/`** - `AclProvider` implements `Hooks` for topic-based publish/subscribe authorization.
- **`config/`** - TOML configuration with env var substitution (`${VAR:-default}`) and `VIBEMQ_*` prefix overrides.
- **`transport/`** - WebSocket support via `WsStream` wrapper around tokio-tungstenite.
- **`bridge/`** - MQTT bridging to external brokers. `BridgeClient` implements `RemotePeer` trait to connect as an MQTT client. `TopicMapper` handles topic pattern matching and remapping. `BridgeManager` coordinates multiple bridges.
- **`remote/`** - Shared abstractions for bridging and future clustering. `RemotePeer` trait defines interface for forwarding messages to remote brokers. Designed to be extended for clustering.

### Key Design Patterns

- **Concurrent data structures**: Uses `DashMap` for sessions, connections, and retained messages. `parking_lot::RwLock` for session internals.
- **Protocol version handling**: Encoder/Decoder track `ProtocolVersion` to serialize v3.1.1 vs v5.0 packets correctly.
- **Hooks composition**: Auth and ACL are composable hooks - all must return `Ok(true)` for operations to proceed.
- **Flow control**: v5.0 receive maximum and send quotas tracked per session.

### MQTT Spec Reference

The `spec/` directory contains markdown files documenting MQTT v3.1.1 and v5.0 packet formats and behaviors, organized by section number (e.g., `spec/v3.1.1/3.3_publish.md`).

## Configuration

The broker accepts TOML config files. Key sections:
- `[server]` - bind address, workers, WebSocket settings
- `[limits]` - max connections, packet size, inflight messages
- `[session]` - keep alive, topic aliases
- `[mqtt]` - QoS limits, feature flags (retain, wildcards, shared subs)
- `[auth]` - enable authentication, user list with roles
- `[acl]` - enable ACL, role-based topic patterns
- `[[bridge]]` - bridge connections to remote brokers with topic forwarding rules

## Bridging and Clustering

The `remote/` module provides shared abstractions (`RemotePeer` trait) used by both bridging and future clustering:
- **Bridging**: `bridge/` module implements `RemotePeer` as an MQTT client connecting to external brokers
- **Clustering** (future): Will implement `RemotePeer` with custom protocol for distributed broker nodes

Loop prevention strategies for bridges:
- `no_local`: MQTT v5.0 subscription flag (default, efficient)
- `user_property`: Tags messages with `x-vibemq-origin` user property
- `both`: Uses both strategies for maximum safety
