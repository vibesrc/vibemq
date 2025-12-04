# VibeMQ Cluster Example

This example demonstrates horizontal clustering with 3 VibeMQ nodes.

## Kubernetes with HPA

For Kubernetes deployments with autoscaling, see `kubernetes.yaml`:

```bash
kubectl apply -f kubernetes.yaml
kubectl get pods -l app=vibemq -w
```

The cluster uses a headless service (`vibemq-headless`) for node discovery. Chitchat resolves the DNS name to all pod IPs automatically and refreshes every 60 seconds.

## Docker Compose

### Architecture

```
         ┌─────────────────────────────────────────┐
         │              Gossip Network             │
         │         (UDP, chitchat protocol)        │
         └─────────────────────────────────────────┘
                    ▲           ▲           ▲
                    │           │           │
              ┌─────┴─────┬─────┴─────┬─────┴─────┐
              │           │           │           │
         ┌────▼────┐ ┌────▼────┐ ┌────▼────┐      │
         │  node1  │ │  node2  │ │  node3  │      │
         │ :1883   │ │ :1884   │ │ :1885   │      │
         │ :7946   │ │         │ │         │      │
         │ :7947   │ │         │ │         │      │
         └─────────┘ └─────────┘ └─────────┘      │
              │           │           │           │
              └───────────┴───────────┴───────────┘
                    Peer TCP (bincode messages)
```

## Running

```bash
# Start the cluster
docker compose up --build

# In another terminal, subscribe on node1
mosquitto_sub -h localhost -p 1883 -t "test/#" -v

# In another terminal, publish on node2
mosquitto_pub -h localhost -p 1884 -t "test/hello" -m "Hello from node2!"

# The message should appear in the subscriber connected to node1
```

## Ports

| Node  | MQTT  | Gossip | Peer |
|-------|-------|--------|------|
| node1 | 1883  | 7946   | 7947 |
| node2 | 1884  | -      | -    |
| node3 | 1885  | -      | -    |

(Only node1 exposes gossip/peer ports externally; internal Docker networking handles inter-node communication)

## How It Works

1. **Node Discovery**: Nodes use the chitchat gossip protocol to discover each other
2. **Subscription Sync**: Each node advertises its subscriptions via gossip state
3. **Message Routing**: When a message is published, it's forwarded to nodes with matching subscriptions
4. **Loop Prevention**: Messages include origin node ID to prevent infinite loops

## Configuration

Each node has a `nodeN.toml` config file:

```toml
[[cluster]]
enabled = true
node_id = "node1"           # Unique node identifier
gossip_addr = "0.0.0.0:7946" # UDP port for gossip
peer_addr = "0.0.0.0:7947"   # TCP port for message relay
seeds = ["node1:7946"]       # Initial peers to contact
```

## Testing Scenarios

### Basic Pub/Sub Across Nodes
```bash
# Terminal 1: Subscribe on node1
mosquitto_sub -h localhost -p 1883 -t "#" -v

# Terminal 2: Publish on node3
mosquitto_pub -h localhost -p 1885 -t "sensors/temp" -m "25.5"
```

### Retained Messages
```bash
# Publish retained on node2
mosquitto_pub -h localhost -p 1884 -t "status/node2" -m "online" -r

# Subscribe on node1 (should receive retained message)
mosquitto_sub -h localhost -p 1883 -t "status/#" -v
```

### Node Failure
```bash
# Stop node2
docker compose stop node2

# Messages should still flow between node1 and node3
mosquitto_pub -h localhost -p 1885 -t "test/failover" -m "still working"
```

### Scaling
```bash
# Scale up
docker compose up -d --scale node2=3

# Scale down
docker compose up -d --scale node2=1
```
