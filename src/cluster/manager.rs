//! Cluster Manager
//!
//! Coordinates gossip-based cluster membership and message forwarding
//! between VibeMQ nodes.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use chitchat::transport::UdpTransport;
use chitchat::{spawn_chitchat, ChitchatConfig, ChitchatHandle, ChitchatId, FailureDetectorConfig};
use dashmap::DashMap;
use parking_lot::RwLock;
use tokio::net::TcpListener;
use tracing::{debug, error, info, warn};

use crate::config::ClusterConfig;
use crate::protocol::QoS;
use crate::remote::RemotePeer;
use crate::remote::RemotePeerStatus;

use super::peer::{ClusterInboundCallback, ClusterPeer};
use super::protocol::{frame_message, read_frame_length, ClusterMessage, CLUSTER_PROTOCOL_VERSION};

/// Chitchat state keys
const KEY_PEER_ADDR: &str = "peer_addr";
const KEY_SUBSCRIPTIONS: &str = "subscriptions";

/// Cluster manager for gossip-based horizontal scaling
pub struct ClusterManager {
    /// Our node ID
    node_id: String,
    /// Cluster configuration
    config: ClusterConfig,
    /// Chitchat handle for gossip communication
    chitchat: ChitchatHandle,
    /// Connected peer nodes
    peers: Arc<DashMap<String, Arc<ClusterPeer>>>,
    /// Local subscriptions (topic filters we have subscribers for)
    local_subscriptions: Arc<RwLock<HashSet<String>>>,
    /// Callback for inbound messages from cluster peers
    inbound_callback: ClusterInboundCallback,
}

impl ClusterManager {
    /// Create a new cluster manager
    pub async fn new(
        config: ClusterConfig,
        inbound_callback: ClusterInboundCallback,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let node_id = config.get_node_id();
        let gossip_advertise_addr = config.get_gossip_advertise_addr();
        let peer_advertise_addr = config.get_peer_advertise_addr();

        info!(
            "Starting cluster node: {} (gossip_advertise={}, peer_advertise={})",
            node_id, gossip_advertise_addr, peer_advertise_addr
        );

        // Create chitchat ID with the advertise address (what peers use to reach us)
        let chitchat_id = ChitchatId::new(node_id.clone(), 0, gossip_advertise_addr);

        // Parse seed nodes as strings (chitchat expects Vec<String>)
        let seed_nodes: Vec<String> = config.seeds.clone();

        // Configure failure detector
        let failure_detector_config = FailureDetectorConfig {
            phi_threshold: 8.0,
            initial_interval: config.gossip_interval_duration(),
            ..Default::default()
        };

        // Create chitchat config
        let chitchat_config = ChitchatConfig {
            chitchat_id: chitchat_id.clone(),
            cluster_id: "vibemq".to_string(),
            gossip_interval: config.gossip_interval_duration(),
            listen_addr: config.gossip_addr, // Bind address (0.0.0.0 is fine)
            seed_nodes,
            failure_detector_config,
            marked_for_deletion_grace_period: config.dead_node_grace_period_duration(),
            catchup_callback: None,
            extra_liveness_predicate: None,
        };

        // Create UDP transport
        let transport = UdpTransport;

        // Initial key-value pairs for our node - use advertise address for peer_addr
        let initial_kvs = vec![
            (KEY_PEER_ADDR.to_string(), peer_advertise_addr.to_string()),
            (KEY_SUBSCRIPTIONS.to_string(), "[]".to_string()),
        ];

        // Spawn chitchat
        let chitchat = spawn_chitchat(chitchat_config, initial_kvs, &transport).await?;

        Ok(Self {
            node_id,
            config,
            chitchat,
            peers: Arc::new(DashMap::new()),
            local_subscriptions: Arc::new(RwLock::new(HashSet::new())),
            inbound_callback,
        })
    }

    /// Get our node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Get the number of peers
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Get the number of connected peers
    pub fn connected_peer_count(&self) -> usize {
        self.peers
            .iter()
            .filter(|p| p.value().status() == RemotePeerStatus::Connected)
            .count()
    }

    /// Update local subscriptions and sync to gossip state
    pub async fn update_subscriptions(&self, filters: HashSet<String>) {
        {
            let mut subs = self.local_subscriptions.write();
            *subs = filters.clone();
        }

        // Serialize to JSON for gossip state
        let json = serde_json::to_string(&filters.iter().collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".to_string());

        debug!(
            "Cluster: updating gossip state with subscriptions: {}",
            json
        );

        // Update chitchat state using with_chitchat
        self.chitchat
            .with_chitchat(|cc| {
                cc.self_node_state()
                    .set(KEY_SUBSCRIPTIONS.to_string(), json.clone());
            })
            .await;

        debug!("Cluster: gossip state updated");
    }

    /// Add a subscription filter
    pub async fn add_subscription(&self, filter: String) {
        debug!("Cluster: adding subscription filter '{}'", filter);
        let filters = {
            let mut subs = self.local_subscriptions.write();
            subs.insert(filter);
            subs.clone()
        };
        self.update_subscriptions(filters).await;
    }

    /// Remove a subscription filter
    pub async fn remove_subscription(&self, filter: &str) {
        let filters = {
            let mut subs = self.local_subscriptions.write();
            subs.remove(filter);
            subs.clone()
        };
        self.update_subscriptions(filters).await;
    }

    /// Forward a published message to peers that have matching subscriptions
    pub async fn forward_publish(&self, topic: &str, payload: Bytes, qos: QoS, retain: bool) {
        for peer in self.peers.iter() {
            let peer_ref = peer.value();
            let status = peer_ref.status();
            let should_fwd = peer_ref.should_forward(topic);
            debug!(
                "Cluster forward check: peer='{}' status={:?} should_forward={} topic='{}'",
                peer_ref.node_id(),
                status,
                should_fwd,
                topic
            );
            if status == RemotePeerStatus::Connected && should_fwd {
                debug!("Cluster: forwarding to peer '{}'", peer_ref.node_id());
                if let Err(e) = peer_ref
                    .forward_publish(topic, payload.clone(), qos, retain)
                    .await
                {
                    warn!(
                        "Failed to forward message to peer '{}': {}",
                        peer_ref.node_id(),
                        e
                    );
                }
            }
        }
    }

    /// Start the cluster manager background tasks
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(
            "Cluster manager starting on gossip={}, peer={}",
            self.config.gossip_addr, self.config.peer_addr
        );

        // Spawn peer listener (accepts incoming TCP connections from other nodes)
        let listener = TcpListener::bind(self.config.peer_addr).await?;
        let inbound_callback = self.inbound_callback.clone();
        let local_node_id = self.node_id.clone();
        let local_subs = self.local_subscriptions.clone();

        tokio::spawn(async move {
            Self::peer_listener_loop(listener, inbound_callback, local_node_id, local_subs).await;
        });

        // Spawn gossip watcher (discovers new peers, connects to them)
        let chitchat = self.chitchat.chitchat();
        let peers = self.peers.clone();
        let config = self.config.clone();
        let inbound_callback = self.inbound_callback.clone();
        let local_node_id = self.node_id.clone();

        tokio::spawn(async move {
            Self::gossip_watcher_loop(chitchat, peers, config, inbound_callback, local_node_id)
                .await;
        });

        Ok(())
    }

    /// Stop the cluster manager
    pub async fn stop(&self) {
        info!("Stopping cluster manager");

        // Stop all peer connections
        for peer in self.peers.iter() {
            let _ = peer.value().stop().await;
        }

        // Shutdown chitchat - it will stop when handle is dropped
    }

    /// Listen for incoming peer connections
    async fn peer_listener_loop(
        listener: TcpListener,
        inbound_callback: ClusterInboundCallback,
        local_node_id: String,
        local_subs: Arc<RwLock<HashSet<String>>>,
    ) {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    debug!("Incoming cluster peer connection from {}", addr);

                    let callback = inbound_callback.clone();
                    let node_id = local_node_id.clone();
                    let subs = local_subs.clone();

                    tokio::spawn(async move {
                        if let Err(e) =
                            Self::handle_incoming_peer(stream, callback, node_id, subs).await
                        {
                            debug!("Incoming peer connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept peer connection: {}", e);
                }
            }
        }
    }

    /// Handle an incoming peer connection
    async fn handle_incoming_peer(
        stream: tokio::net::TcpStream,
        inbound_callback: ClusterInboundCallback,
        local_node_id: String,
        local_subs: Arc<RwLock<HashSet<String>>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut read_half, mut write_half) = stream.into_split();
        let mut read_buf = vec![0u8; 65536];

        // Wait for Hello
        let n =
            tokio::time::timeout(Duration::from_secs(10), read_half.read(&mut read_buf)).await??;

        if n == 0 {
            return Err("Connection closed".into());
        }

        let len = read_frame_length(&read_buf[..n]).ok_or("Invalid frame")?;
        if n < 4 + len as usize {
            return Err("Incomplete frame".into());
        }

        let msg = ClusterMessage::decode(&read_buf[4..4 + len as usize])?;

        let peer_node_id = match msg {
            ClusterMessage::Hello { node_id, version } => {
                if version != CLUSTER_PROTOCOL_VERSION {
                    return Err(format!(
                        "Protocol version mismatch: {} vs {}",
                        version, CLUSTER_PROTOCOL_VERSION
                    )
                    .into());
                }
                node_id
            }
            _ => return Err("Expected Hello".into()),
        };

        info!("Incoming cluster peer: {}", peer_node_id);

        // Send HelloAck
        let ack = ClusterMessage::HelloAck {
            node_id: local_node_id.clone(),
            version: CLUSTER_PROTOCOL_VERSION,
        };
        let frame = frame_message(&ack)?;
        write_half.write_all(&frame).await?;

        // Send our subscriptions
        let subs: Vec<String> = local_subs.read().iter().cloned().collect();
        let sync = ClusterMessage::SubscriptionSync { filters: subs };
        let frame = frame_message(&sync)?;
        write_half.write_all(&frame).await?;

        // Message loop
        let mut buf_offset = 0usize;

        loop {
            let n = read_half.read(&mut read_buf[buf_offset..]).await?;
            if n == 0 {
                info!("Cluster peer '{}' disconnected", peer_node_id);
                return Ok(());
            }

            buf_offset += n;

            while buf_offset >= 4 {
                let len = read_frame_length(&read_buf).unwrap() as usize;
                if buf_offset < 4 + len {
                    break;
                }

                if let Ok(msg) = ClusterMessage::decode(&read_buf[4..4 + len]) {
                    match msg {
                        ClusterMessage::Publish {
                            topic,
                            payload,
                            qos,
                            retain,
                            origin_node,
                        } => {
                            debug!(
                                "Cluster inbound: received publish '{}' from peer {} (origin={})",
                                topic, peer_node_id, origin_node
                            );
                            // Always process messages from cluster peers - the origin_node
                            // is used to prevent the original publisher from getting the
                            // message echoed back, but that's handled by not re-forwarding
                            // cluster messages (the inbound_callback doesn't emit events)
                            let qos_level = match qos {
                                0 => QoS::AtMostOnce,
                                1 => QoS::AtLeastOnce,
                                _ => QoS::ExactlyOnce,
                            };
                            debug!("Cluster inbound: calling inbound_callback for '{}'", topic);
                            inbound_callback(
                                topic,
                                Bytes::from(payload),
                                qos_level,
                                retain,
                                origin_node,
                            );
                        }
                        ClusterMessage::Ping => {
                            let pong = ClusterMessage::Pong;
                            if let Ok(frame) = frame_message(&pong) {
                                let _ = write_half.write_all(&frame).await;
                            }
                        }
                        ClusterMessage::Goodbye => {
                            info!("Cluster peer '{}' said goodbye", peer_node_id);
                            return Ok(());
                        }
                        _ => {}
                    }
                }

                read_buf.copy_within(4 + len..buf_offset, 0);
                buf_offset -= 4 + len;
            }
        }
    }

    /// Watch gossip state for new peers and connect to them
    async fn gossip_watcher_loop(
        chitchat: Arc<tokio::sync::Mutex<chitchat::Chitchat>>,
        peers: Arc<DashMap<String, Arc<ClusterPeer>>>,
        config: ClusterConfig,
        inbound_callback: ClusterInboundCallback,
        local_node_id: String,
    ) {
        let mut known_nodes: HashSet<String> = HashSet::new();

        loop {
            tokio::time::sleep(config.gossip_interval_duration()).await;

            // Get current cluster state
            let cluster_state = {
                let cc = chitchat.lock().await;
                cc.state_snapshot()
            };

            // Find new nodes
            for node_state in &cluster_state.node_states {
                let node_id_str = node_state.chitchat_id().node_id.clone();

                // Skip ourselves
                if node_id_str == local_node_id {
                    continue;
                }

                // Check if this is a new node
                if !known_nodes.contains(&node_id_str) {
                    known_nodes.insert(node_id_str.clone());

                    // Get peer address from gossip state - this should be the advertise address
                    let gossip_addr = node_state.chitchat_id().gossip_advertise_addr;

                    if let Some(peer_addr_str) = node_state.get(KEY_PEER_ADDR) {
                        if let Ok(peer_addr) = peer_addr_str.parse::<SocketAddr>() {
                            info!(
                                "Discovered new cluster peer: {} at peer={} gossip={}",
                                node_id_str, peer_addr, gossip_addr
                            );

                            // Create and spawn peer connection
                            let peer = ClusterPeer::new(
                                node_id_str.clone(),
                                peer_addr,
                                local_node_id.clone(),
                            );
                            let peer = peer.spawn(inbound_callback.clone());
                            peers.insert(node_id_str.clone(), peer);
                        }
                    }
                }

                // Update peer subscriptions from gossip state
                if let Some(peer) = peers.get(&node_id_str) {
                    if let Some(subs_json) = node_state.get(KEY_SUBSCRIPTIONS) {
                        if let Ok(filters) = serde_json::from_str::<Vec<String>>(subs_json) {
                            debug!(
                                "Cluster: updating peer '{}' subscriptions from gossip: {:?}",
                                node_id_str, filters
                            );
                            peer.update_remote_subscriptions(filters);
                        }
                    }
                }
            }

            // Remove dead nodes
            let current_nodes: HashSet<String> = cluster_state
                .node_states
                .iter()
                .map(|ns| ns.chitchat_id().node_id.clone())
                .collect();

            let dead_nodes: Vec<String> = known_nodes
                .iter()
                .filter(|n| *n != &local_node_id && !current_nodes.contains(*n))
                .cloned()
                .collect();

            for node_id in dead_nodes {
                info!("Cluster peer '{}' left the cluster", node_id);
                known_nodes.remove(&node_id);
                if let Some((_, peer)) = peers.remove(&node_id) {
                    let _ = peer.stop().await;
                }
            }
        }
    }
}

// ClusterManager is Send + Sync because all its fields are thread-safe
unsafe impl Send for ClusterManager {}
unsafe impl Sync for ClusterManager {}
