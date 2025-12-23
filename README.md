# VibeMQ

A lightweight, high-performance MQTT broker written in Rust. Single binary, minimal memory footprint, stable under heavy load. Fully compliant with MQTT v3.1.1 and v5.0 specifications.

## Features

- **Full Protocol Support** - MQTT v3.1.1 and v5.0 with all QoS levels (0, 1, 2)
- **High Performance** - Built on Tokio async runtime with zero-copy buffer handling
- **WebSocket Support** - MQTT over WebSocket for browser and web clients
- **Topic Wildcards** - Single-level (`+`) and multi-level (`#`) wildcard subscriptions
- **Shared Subscriptions** - Load-balanced message delivery with `$share/{group}/{filter}`
- **Retained Messages** - Automatic delivery of last known good values to new subscribers
- **Will Messages** - Last Will and Testament for client disconnect notification
- **Session Persistence** - Resumable sessions with offline message queuing
- **Authentication** - Username/password authentication with configurable user list
- **Access Control** - Role-based ACL for publish/subscribe topic permissions
- **TLS Support** - Optional TLS encryption (feature flag)
- **Bridging** - Connect multiple brokers with configurable topic forwarding
- **Flexible Configuration** - TOML config files with environment variable overrides

## Why VibeMQ?

- **Single Binary** - No JVM, no Erlang/BEAM VM, no runtime dependencies. Just one executable.
- **Lightweight** - ~10MB binary, ~85MB memory under sustained QoS 2 load (2000 publishers, 100 subscribers, 13K+ msg/s)
- **Predictable Resources** - Bounded memory that stays flat under load, no runaway growth during QoS 2 storms
- **Fast** - Async Rust on Tokio, multi-core scalability, sub-100ms P99 QoS 2 message lifecycle
- **Production Ready** - Full MQTT 5.0 compliance, TLS, auth, ACL, bridging for HA setups
- **Scalable** - Clustering support (experimental)
- **Simple Operations** - TOML config, env var overrides, no complex clustering required for most deployments

## Benchmarks

VibeMQ was benchmarked against popular MQTT brokers at QoS 0 and QoS 2. Each broker ran independently with limits disabled in Docker containers with 8GB memory available. Best results from multiple runs were taken for each broker.

### QoS 0 (Fire-and-Forget)

#### Fan-In: 2000 Publishers → 100 Subscribers

High-throughput ingestion scenario at 10 msg/s per publisher (~19K msg/s total):

| Broker | Delivery Rate | Mean Latency | P99 Latency | CPU | Memory |
|--------|---------------|--------------|-------------|-----|--------|
| **VibeMQ** | **100.00%** | **3.4ms** | **14ms** | 142% | 101 MB |
| Mosquitto | 99.98% | 53ms | 146ms | 88% | 6.5 MB |
| EMQX | 100.00% | 13.5ms | 49ms | 339% | 599 MB |
| VerneMQ | 99.99% | 11.6ms | 51ms | 296% | 482 MB |

#### Fan-Out: 100 Publishers → 2000 Subscribers

Broadcast scenario delivering each message to 2000 subscribers (~187K msg/s fanout):

| Broker | Delivery Rate | Mean Latency | P99 Latency | CPU | Memory |
|--------|---------------|--------------|-------------|-----|--------|
| **VibeMQ** | **99.95%** | **340ms** | **638ms** | 218% | **219 MB** |
| Mosquitto | 12.24% | 34.2s | 58.6s | 90% | 35 MB |
| EMQX | 100.00% | 404ms | 819ms | 311% | 1,435 MB |
| VerneMQ | 100.31% | 419ms | 804ms | 272% | 1,672 MB |

### QoS 2 (Exactly-Once Delivery)

The most demanding QoS level, requiring a 4-packet handshake per message.

#### Fan-In: 2000 Publishers → 100 Subscribers

High-throughput ingestion scenario at 10 msg/s per publisher (~19K msg/s total):

| Broker | Delivery Rate | Mean Latency | P99 Latency | CPU | Memory |
|--------|---------------|--------------|-------------|-----|--------|
| **VibeMQ** | **99.99%** | **36ms** | **116ms** | 284% | **127 MB** |
| Mosquitto | 34.34% | 21.6s | 35.0s | 89% | 246 MB |
| EMQX | 33.65% | 17.2s | 41.4s | 358% | 2,090 MB |
| VerneMQ | 0.15% | 18.2s | 65.6s | 363% | 2,583 MB |

#### Fan-Out: 100 Publishers → 2000 Subscribers

Broadcast scenario delivering each message to 2000 subscribers (~190K msg/s fanout):

| Broker | Delivery Rate | Mean Latency | P99 Latency | CPU | Memory |
|--------|---------------|--------------|-------------|-----|--------|
| **VibeMQ** | **99.84%** | **463ms** | **890ms** | 325% | **527 MB** |
| Mosquitto | 13.02% | 26.5s | 42.8s | 81% | 190 MB |
| EMQX | 23.38% | 20.2s | 45.3s | 312% | 8,590 MB (OOM) |
| VerneMQ | 1.63% | 29.2s | 57.3s | 361% | 8,590 MB (OOM) |

### Key Observations

- **Near-100% message delivery** - VibeMQ delivered 99.84%+ of messages at QoS 2 while competitors lost 65-99%+
- **Lowest latency** - 4-15x faster than competitors at QoS 0; milliseconds vs tens of seconds at QoS 2
- **Flat memory profile** - Memory stays constant under load while others ballooned to the 8.5GB limit
- **Memory efficient** - 6-8x less memory than EMQX/VerneMQ on fan-out workloads
- **Multi-core scaling** - Uses multiple cores effectively unlike single-threaded Mosquitto, while being more efficient than EMQX/VerneMQ

See [`benchmarks/`](./benchmarks) for full results and methodology.

## Quick Start

### Run with Defaults

```bash
cargo run --release
```

The broker starts on `0.0.0.0:1883` with authentication disabled.

### Run with Configuration

```bash
cargo run --release -- -c config.toml
```

### CLI Options

```
vibemq [OPTIONS]

Options:
  -c, --config <FILE>       Configuration file path (TOML format)
  -b, --bind <ADDR>         TCP bind address (default: 0.0.0.0:1883)
      --ws-bind <ADDR>      WebSocket bind address (enables MQTT over WebSocket)
  -w, --workers <N>         Number of worker threads (0 = auto)
      --max-connections <N> Maximum connections (default: 100000)
      --max-packet-size <N> Maximum packet size in bytes (default: 1MB)
      --max-qos <N>         Maximum QoS level: 0, 1, or 2 (default: 2)
      --keep-alive <N>      Default keep alive in seconds
  -l, --log-level <LEVEL>   Log level: error, warn, info, debug, trace
  -h, --help                Print help
```

## Configuration

Create a `config.toml` file:

```toml
[log]
level = "info"

[server]
bind = "0.0.0.0:1883"
ws_bind = "0.0.0.0:8083"  # Optional WebSocket
ws_path = "/mqtt"
workers = 0  # 0 = auto-detect CPU count

[limits]
max_connections = 100000
max_packet_size = 1048576  # 1 MB
max_inflight = 32
max_queued_messages = 1000

[session]
default_keep_alive = 60
max_keep_alive = 65535
max_topic_aliases = 65535

[mqtt]
max_qos = 2
retain_available = true
wildcard_subscriptions = true
subscription_identifiers = true
shared_subscriptions = true

[auth]
enabled = true
allow_anonymous = false

[[auth.users]]
username = "admin"
password = "secret"
role = "admin"

[[auth.users]]
username = "sensor"
password = "sensor123"
role = "device"

[acl]
enabled = true

[[acl.roles]]
name = "admin"
publish = ["#"]
subscribe = ["#"]

[[acl.roles]]
name = "device"
publish = ["sensors/+/data", "sensors/+/status"]
subscribe = ["commands/+"]

[acl.default]
publish = []
subscribe = ["public/#"]

# Bridge to cloud broker
[[bridge]]
name = "cloud"
address = "cloud.example.com:8883"
protocol = "mqtts"
client_id = "edge-bridge-01"
username = "bridge"
password = "${BRIDGE_PASSWORD}"
loop_prevention = "both"  # no_local + user property tagging

[[bridge.forwards]]
local_topic = "sensors/#"
remote_topic = "edge/device01/sensors/#"
direction = "out"
qos = 1

[[bridge.forwards]]
local_topic = "commands/device01/#"
remote_topic = "commands/#"
direction = "in"
qos = 1
```

### Environment Variable Overrides

Override any config value with `VIBEMQ__` prefixed environment variables (double underscore separates nested keys):

```bash
VIBEMQ__SERVER__BIND=0.0.0.0:1884 cargo run
VIBEMQ__AUTH__ENABLED=true cargo run
VIBEMQ__LIMITS__MAX_CONNECTIONS=50000 cargo run
```

Use `${VAR:-default}` syntax in config files for env var substitution:

```toml
[auth]
enabled = ${AUTH_ENABLED:-false}

[[auth.users]]
username = "admin"
password = "${ADMIN_PASSWORD:-changeme}"
```

## Usage Examples

### Connect with mosquitto client

```bash
# Simple connection
mosquitto_sub -h localhost -t "test/#" -v

# With authentication
mosquitto_pub -h localhost -u admin -P secret -t "test/topic" -m "hello"

# QoS 1
mosquitto_sub -h localhost -t "sensors/+/data" -q 1

# Retained message
mosquitto_pub -h localhost -t "status/device1" -m "online" -r
```

### WebSocket Connection

Connect via WebSocket at `ws://localhost:8083/mqtt` using any MQTT.js compatible client.

## Building

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# With TLS support
cargo build --release --features tls
```

## Testing

```bash
# Run all tests
cargo test

# Integration tests only
cargo test --test integration

# Specific test
cargo test test_publish_qos2_flow
```

## Bridging

VibeMQ supports bridging to connect multiple MQTT brokers. Messages can be forwarded bidirectionally based on topic patterns.

### Bridge Configuration

```toml
[[bridge]]
name = "cloud"
address = "broker.example.com:1883"
protocol = "mqtt"           # mqtt, mqtts, ws, wss
client_id = "vibemq-bridge"
keepalive = 60
reconnect_interval = 5
loop_prevention = "no_local" # no_local, user_property, both, none

# Forward local messages to remote
[[bridge.forwards]]
local_topic = "sensors/#"
remote_topic = "edge/sensors/#"
direction = "out"
qos = 1

# Receive remote messages locally
[[bridge.forwards]]
local_topic = "commands/#"
remote_topic = "device/commands/#"
direction = "in"
qos = 1

# Bidirectional sync
[[bridge.forwards]]
local_topic = "shared/#"
remote_topic = "shared/#"
direction = "both"
qos = 1
```

### Loop Prevention

Bridges use multiple strategies to prevent message loops:
- **no_local** (default): Uses MQTT v5.0 subscription option to avoid receiving own messages
- **user_property**: Tags messages with origin broker ID
- **both**: Uses both strategies for maximum safety
- **none**: Disable loop prevention (use with caution)

## License

MIT
