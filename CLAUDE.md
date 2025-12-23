# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

```bash
# Build
cargo build                    # Debug build
cargo build --release          # Release build (optimized, LTO enabled)

# Run
cargo run                                # Run broker with defaults (0.0.0.0:1883)
cargo run -- -c config.toml              # Run with config file
cargo run -- -b 0.0.0.0:1884 -l debug    # Custom bind address and log level

# Test
cargo test                     # Run all tests
cargo test --test integration  # Run integration tests only
cargo test --test conformance  # Run MQTT conformance tests
cargo test --test bridge       # Run bridge tests
cargo test <test_name>         # Run a specific test

# Lint and Format
cargo fmt                      # Format code
cargo clippy                   # Run linter

# CI Checks
cargo deny check               # License/advisory/source checks
cargo audit                    # Security audit

# Docker
docker build -t vibemq .       # Build container image
docker compose up              # Run with compose.yml
```

## Profiling

When built with `--features pprof`, a profiling server runs on `http://127.0.0.1:6060`:

- `/debug/pprof/profile?seconds=30` - CPU profile
- `/debug/pprof/heap` - Heap profile dump
- `/debug/pprof/flamegraph` - Flamegraph visualization

Requires `libunwind-dev` on Linux. Run with `MALLOC_CONF=prof:true` for heap profiling.

## Metrics

Prometheus metrics exposed at `/metrics` when `[metrics]` is enabled in config. Key metrics include connection counts, message rates by type, subscription counts, QoS inflight messages, and latency histograms.

## $SYS Topics

Standard broker statistics published as retained messages (enabled by default, 10s interval):
- `$SYS/broker/version` - VibeMQ version
- `$SYS/broker/uptime` - Seconds since start
- `$SYS/broker/clients/connected` - Current connected clients
- `$SYS/broker/clients/total` - Total connections since start
- `$SYS/broker/subscriptions/count` - Current subscriptions
- `$SYS/broker/retained messages/count` - Current retained messages
- `$SYS/broker/bytes/received` - Total bytes received
- `$SYS/broker/bytes/sent` - Total bytes sent

## Architecture Overview

VibeMQ is a high-performance MQTT v3.1.1/v5.0 broker built with Tokio for async I/O.

### Core Modules

- **`broker/`** - Main broker orchestration. `Broker` manages connections, sessions, subscriptions, and retained messages. `Connection` handles individual client TCP/WebSocket streams.
- **`protocol/`** - MQTT packet definitions (`Packet`, `Connect`, `Publish`, etc.), `QoS` levels, `ProtocolVersion`, `ReasonCode`, and `Properties` for v5.0.
- **`codec/`** - `Encoder` and `Decoder` for MQTT packet serialization. Handles both v3.1.1 and v5.0 wire formats.
- **`session/`** - `Session` tracks client state (subscriptions, inflight messages, packet IDs). `SessionStore` provides thread-safe session management with DashMap.
- **`topic/`** - `SubscriptionStore` uses a topic trie for efficient wildcard matching. Supports `+` (single-level) and `#` (multi-level) wildcards, plus shared subscriptions (`$share/{group}/{filter}`).
- **`hooks/`** - Extensibility via `Hooks` trait for auth, ACL, and event handling. `CompositeHooks` chains multiple implementations.
- **`auth/`** - `AuthProvider` implements `Hooks` for username/password authentication. Supports plaintext passwords (`password`) or argon2 hashes (`password_hash`).
- **`acl/`** - `AclProvider` implements `Hooks` for topic-based publish/subscribe authorization. Supports `%c` (client_id) and `%u` (username) substitution in topic patterns.
- **`config/`** - TOML configuration with env var substitution (`${VAR:-default}`) and `VIBEMQ__*` prefix overrides (double underscore for nesting).
- **`transport/`** - WebSocket support via `WsStream` wrapper around tokio-tungstenite.
- **`bridge/`** - MQTT bridging to external brokers. `BridgeClient` implements `RemotePeer` trait to connect as an MQTT client. `TopicMapper` handles topic pattern matching and remapping. `BridgeManager` coordinates multiple bridges.
- **`cluster/`** - Gossip-based clustering via Chitchat protocol. Nodes discover each other via UDP gossip and forward messages via TCP with bincode serialization.
- **`remote/`** - Shared abstractions for bridging and clustering. `RemotePeer` trait defines interface for forwarding messages to remote brokers.
- **`metrics/`** - Prometheus metrics collection and HTTP endpoint.
- **`profiling/`** - Optional CPU and heap profiling with pprof integration.

### Key Design Patterns

- **Concurrent data structures**: Uses `DashMap` for sessions, connections, and retained messages. `parking_lot::RwLock` for session internals.
- **Protocol version handling**: Encoder/Decoder track `ProtocolVersion` to serialize v3.1.1 vs v5.0 packets correctly.
- **Hooks composition**: Auth and ACL are composable hooks - all must return `Ok(true)` for operations to proceed.
- **Flow control**: v5.0 receive maximum and send quotas tracked per session.

### MQTT Spec Reference

The `docs/mqtt-rfc/` directory contains comprehensive MQTT v3.1.1 and v5.0 specification documentation in RFC style, organized by section number (e.g., `docs/mqtt-rfc/v3.1.1/05.03-publish.md`). Each version includes an `appendix-a-normative.md` with all normative statements for conformance testing.

## Configuration

The broker accepts TOML config files. Key sections:
- `[server]` - bind address, workers, WebSocket settings, TLS configuration
- `[server.tls]` - TLS certificate/key paths, client cert auth settings
- `[limits]` - max connections, packet size, inflight messages, queued messages, retry interval
- `[session]` - keep alive, topic aliases, expiry check interval
- `[mqtt]` - QoS limits, feature flags (retain, wildcards, shared subs), $SYS topics
- `[auth]` - enable authentication, user list with `password` (plaintext) or `password_hash` (argon2)
- `[acl]` - enable ACL, role-based topic patterns with `%c`/`%u` substitution
- `[metrics]` - enable Prometheus metrics endpoint
- `[[bridge]]` - bridge connections to remote brokers with topic forwarding rules
- `[[cluster]]` - clustering with gossip discovery and peer addresses

### Environment Variables

- Config values support `${VAR}` and `${VAR:-default}` substitution
- `VIBEMQ__*` prefixed env vars override config (e.g., `VIBEMQ__SERVER__BIND`, `VIBEMQ__AUTH__ENABLED`)

### TLS Configuration Example

```toml
[server]
bind = "0.0.0.0:1883"
tls_bind = "0.0.0.0:8883"  # Optional TLS listener

[server.tls]
cert = "/path/to/cert.pem"
key = "/path/to/key.pem"
ca_cert = "/path/to/ca.pem"  # Optional, for client cert auth
require_client_cert = false   # Set true for mTLS
```

## Bridging and Clustering

The `remote/` module provides shared abstractions (`RemotePeer` trait) used by both bridging and clustering:

- **Bridging**: `bridge/` module implements `RemotePeer` as an MQTT client connecting to external brokers
- **Clustering**: `cluster/` module implements gossip-based node discovery (Chitchat) with TCP message forwarding

Loop prevention strategies:
- `no_local`: MQTT v5.0 subscription flag (default, efficient)
- `user_property`: Tags messages with `x-vibemq-origin` user property
- `both`: Uses both strategies for maximum safety

## Examples

- `examples/bridge/` - Bridge configuration with Docker Compose
- `examples/cluster/` - Clustering with Docker Compose and Kubernetes

## Testing

Tests use atomic port counters to avoid conflicts. Helper types:
- `TestClient` - Full client with encoder/decoder and protocol version tracking
- `RawClient` - Low-level client for protocol violation testing

Both MQTT v3.1.1 and v5.0 are tested with version parameters.
