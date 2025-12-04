//! Cluster Peer
//!
//! Represents a connection to another node in the cluster.
//! Implements RemotePeer for unified message forwarding.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use parking_lot::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::protocol::QoS;
use crate::remote::{RemoteError, RemotePeer, RemotePeerStatus};
use crate::topic::topic_matches_filter;

use super::protocol::{frame_message, read_frame_length, ClusterMessage, CLUSTER_PROTOCOL_VERSION};

/// Commands sent to the peer connection task
#[derive(Debug)]
pub enum ClusterCommand {
    /// Forward a publish message
    Publish {
        topic: String,
        payload: Bytes,
        qos: QoS,
        retain: bool,
        origin_node: String,
    },
    /// Send subscription sync
    SyncSubscriptions { filters: Vec<String> },
    /// Send subscription update
    UpdateSubscriptions {
        added: Vec<String>,
        removed: Vec<String>,
    },
    /// Shutdown the connection
    Shutdown,
}

/// Callback for messages received from a cluster peer
pub type ClusterInboundCallback = Arc<dyn Fn(String, Bytes, QoS, bool, String) + Send + Sync>;

/// A connection to another cluster node
pub struct ClusterPeer {
    /// Remote node ID
    node_id: String,
    /// Remote peer address for TCP connection
    peer_addr: SocketAddr,
    /// Current connection status
    status: Arc<RwLock<RemotePeerStatus>>,
    /// Command channel for sending operations to the connection task
    command_tx: Option<mpsc::Sender<ClusterCommand>>,
    /// Remote node's subscriptions (updated via gossip)
    remote_subscriptions: Arc<RwLock<HashSet<String>>>,
    /// Our local node ID (for origin tracking)
    local_node_id: String,
}

impl ClusterPeer {
    /// Create a new cluster peer
    pub fn new(node_id: String, peer_addr: SocketAddr, local_node_id: String) -> Self {
        Self {
            node_id,
            peer_addr,
            status: Arc::new(RwLock::new(RemotePeerStatus::Disconnected)),
            command_tx: None,
            remote_subscriptions: Arc::new(RwLock::new(HashSet::new())),
            local_node_id,
        }
    }

    /// Get the remote node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Get the peer address
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    /// Update remote subscriptions (called when gossip state changes)
    pub fn update_remote_subscriptions(&self, filters: Vec<String>) {
        let mut subs = self.remote_subscriptions.write();
        subs.clear();
        subs.extend(filters);
    }

    /// Send a subscription sync to this peer
    pub async fn send_subscription_sync(&self, filters: Vec<String>) -> Result<(), RemoteError> {
        if let Some(ref tx) = self.command_tx {
            tx.send(ClusterCommand::SyncSubscriptions { filters })
                .await
                .map_err(|_| RemoteError::ConnectionLost("Command channel closed".to_string()))?;
        }
        Ok(())
    }

    /// Send a subscription update to this peer
    pub async fn send_subscription_update(
        &self,
        added: Vec<String>,
        removed: Vec<String>,
    ) -> Result<(), RemoteError> {
        if let Some(ref tx) = self.command_tx {
            tx.send(ClusterCommand::UpdateSubscriptions { added, removed })
                .await
                .map_err(|_| RemoteError::ConnectionLost("Command channel closed".to_string()))?;
        }
        Ok(())
    }

    /// Spawn the connection task and return the peer ready to use
    pub fn spawn(mut self, inbound_callback: ClusterInboundCallback) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(1000);
        self.command_tx = Some(tx);

        let node_id = self.node_id.clone();
        let local_node_id = self.local_node_id.clone();
        let peer_addr = self.peer_addr;
        let status = self.status.clone();
        let remote_subs = self.remote_subscriptions.clone();

        tokio::spawn(async move {
            Self::connection_loop(
                node_id,
                local_node_id,
                peer_addr,
                status,
                rx,
                inbound_callback,
                remote_subs,
            )
            .await;
        });

        Arc::new(self)
    }

    /// Run the connection loop with reconnection
    async fn connection_loop(
        node_id: String,
        local_node_id: String,
        peer_addr: SocketAddr,
        status: Arc<RwLock<RemotePeerStatus>>,
        mut command_rx: mpsc::Receiver<ClusterCommand>,
        inbound_callback: ClusterInboundCallback,
        remote_subs: Arc<RwLock<HashSet<String>>>,
    ) {
        let mut retry_interval = Duration::from_secs(1);
        let max_retry = Duration::from_secs(30);

        loop {
            *status.write() = RemotePeerStatus::Connecting;
            debug!("ClusterPeer '{}': Connecting to {}", node_id, peer_addr);

            match Self::connect_and_run(
                &node_id,
                &local_node_id,
                peer_addr,
                &status,
                &mut command_rx,
                &inbound_callback,
                &remote_subs,
            )
            .await
            {
                Ok(()) => {
                    info!("ClusterPeer '{}': Disconnected gracefully", node_id);
                    *status.write() = RemotePeerStatus::Disconnected;
                    return; // Clean shutdown
                }
                Err(e) => {
                    error!("ClusterPeer '{}': Connection failed: {}", node_id, e);
                    *status.write() = RemotePeerStatus::Backoff;

                    debug!(
                        "ClusterPeer '{}': Reconnecting in {:?}",
                        node_id, retry_interval
                    );

                    tokio::time::sleep(retry_interval).await;
                    retry_interval = std::cmp::min(retry_interval * 2, max_retry);
                }
            }

            // Check for shutdown command
            match command_rx.try_recv() {
                Ok(ClusterCommand::Shutdown) | Err(mpsc::error::TryRecvError::Disconnected) => {
                    info!("ClusterPeer '{}': Shutdown requested", node_id);
                    *status.write() = RemotePeerStatus::Disconnected;
                    return;
                }
                _ => {}
            }
        }
    }

    /// Connect to the peer and run the message loop
    async fn connect_and_run(
        node_id: &str,
        local_node_id: &str,
        peer_addr: SocketAddr,
        status: &Arc<RwLock<RemotePeerStatus>>,
        command_rx: &mut mpsc::Receiver<ClusterCommand>,
        inbound_callback: &ClusterInboundCallback,
        remote_subs: &Arc<RwLock<HashSet<String>>>,
    ) -> Result<(), RemoteError> {
        // Connect with timeout
        let stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(peer_addr))
            .await
            .map_err(|_| RemoteError::Timeout)?
            .map_err(|e| RemoteError::ConnectionLost(e.to_string()))?;

        debug!("ClusterPeer '{}': TCP connected", node_id);

        let (mut read_half, mut write_half) = stream.into_split();

        // Send Hello
        let hello = ClusterMessage::Hello {
            node_id: local_node_id.to_string(),
            version: CLUSTER_PROTOCOL_VERSION,
        };
        let frame = frame_message(&hello)
            .map_err(|e| RemoteError::Other(format!("Encode error: {}", e)))?;
        write_half
            .write_all(&frame)
            .await
            .map_err(|e| RemoteError::ConnectionLost(e.to_string()))?;

        debug!("ClusterPeer '{}': Hello sent", node_id);

        // Wait for HelloAck
        let mut read_buf = vec![0u8; 65536];
        let n = tokio::time::timeout(Duration::from_secs(10), read_half.read(&mut read_buf))
            .await
            .map_err(|_| RemoteError::Timeout)?
            .map_err(|e| RemoteError::ConnectionLost(e.to_string()))?;

        if n == 0 {
            return Err(RemoteError::ConnectionLost("Connection closed".to_string()));
        }

        // Parse length and message
        let len = read_frame_length(&read_buf[..n])
            .ok_or_else(|| RemoteError::Other("Invalid frame".to_string()))?;
        if n < 4 + len as usize {
            return Err(RemoteError::Other("Incomplete frame".to_string()));
        }

        let msg = ClusterMessage::decode(&read_buf[4..4 + len as usize])
            .map_err(|e| RemoteError::Other(format!("Decode error: {}", e)))?;

        match msg {
            ClusterMessage::HelloAck {
                node_id: peer_id,
                version,
            } => {
                if version != CLUSTER_PROTOCOL_VERSION {
                    return Err(RemoteError::Rejected(format!(
                        "Protocol version mismatch: {} vs {}",
                        version, CLUSTER_PROTOCOL_VERSION
                    )));
                }
                info!("ClusterPeer '{}': Connected (peer_id={})", node_id, peer_id);
            }
            _ => {
                return Err(RemoteError::Other("Expected HelloAck".to_string()));
            }
        }

        *status.write() = RemotePeerStatus::Connected;

        // Message loop
        let ping_interval = Duration::from_secs(15);
        let mut ping_timer = tokio::time::interval(ping_interval);
        ping_timer.reset();

        let mut buf_offset = 0usize;

        loop {
            tokio::select! {
                // Handle commands from the cluster manager
                Some(cmd) = command_rx.recv() => {
                    match cmd {
                        ClusterCommand::Publish { topic, payload, qos, retain, origin_node } => {
                            debug!("ClusterPeer '{}': sending publish '{}' over TCP", node_id, topic);
                            let msg = ClusterMessage::Publish {
                                topic: topic.clone(),
                                payload: payload.to_vec(),
                                qos: qos as u8,
                                retain,
                                origin_node,
                            };
                            if let Ok(frame) = frame_message(&msg) {
                                if let Err(e) = write_half.write_all(&frame).await {
                                    error!("ClusterPeer '{}': TCP write error: {}", node_id, e);
                                    return Err(RemoteError::ConnectionLost(e.to_string()));
                                }
                                debug!("ClusterPeer '{}': sent {} bytes for '{}'", node_id, frame.len(), topic);
                            }
                        }
                        ClusterCommand::SyncSubscriptions { filters } => {
                            let msg = ClusterMessage::SubscriptionSync { filters };
                            if let Ok(frame) = frame_message(&msg) {
                                let _ = write_half.write_all(&frame).await;
                            }
                        }
                        ClusterCommand::UpdateSubscriptions { added, removed } => {
                            let msg = ClusterMessage::SubscriptionUpdate { added, removed };
                            if let Ok(frame) = frame_message(&msg) {
                                let _ = write_half.write_all(&frame).await;
                            }
                        }
                        ClusterCommand::Shutdown => {
                            // Send Goodbye
                            let msg = ClusterMessage::Goodbye;
                            if let Ok(frame) = frame_message(&msg) {
                                let _ = write_half.write_all(&frame).await;
                            }
                            return Ok(());
                        }
                    }
                }

                // Handle incoming messages from peer
                result = read_half.read(&mut read_buf[buf_offset..]) => {
                    let n = result.map_err(|e| RemoteError::ConnectionLost(e.to_string()))?;
                    if n == 0 {
                        return Err(RemoteError::ConnectionLost("Connection closed".to_string()));
                    }

                    buf_offset += n;

                    // Process complete frames
                    while buf_offset >= 4 {
                        let len = read_frame_length(&read_buf).unwrap() as usize;
                        if buf_offset < 4 + len {
                            break; // Need more data
                        }

                        if let Ok(msg) = ClusterMessage::decode(&read_buf[4..4 + len]) {
                            match msg {
                                ClusterMessage::Publish { topic, payload, qos, retain, origin_node } => {
                                    // Always process messages from cluster peers
                                    let qos_level = match qos {
                                        0 => QoS::AtMostOnce,
                                        1 => QoS::AtLeastOnce,
                                        _ => QoS::ExactlyOnce,
                                    };
                                    debug!(
                                        "ClusterPeer '{}': Received publish on '{}' (origin={})",
                                        node_id, topic, origin_node
                                    );
                                    inbound_callback(
                                        topic,
                                        Bytes::from(payload),
                                        qos_level,
                                        retain,
                                        origin_node,
                                    );
                                }
                                ClusterMessage::SubscriptionSync { filters } => {
                                    debug!(
                                        "ClusterPeer '{}': Received subscription sync ({} filters)",
                                        node_id, filters.len()
                                    );
                                    let mut subs = remote_subs.write();
                                    subs.clear();
                                    subs.extend(filters);
                                }
                                ClusterMessage::SubscriptionUpdate { added, removed } => {
                                    debug!(
                                        "ClusterPeer '{}': Subscription update (+{}, -{})",
                                        node_id, added.len(), removed.len()
                                    );
                                    let mut subs = remote_subs.write();
                                    for f in removed {
                                        subs.remove(&f);
                                    }
                                    for f in added {
                                        subs.insert(f);
                                    }
                                }
                                ClusterMessage::Ping => {
                                    let pong = ClusterMessage::Pong;
                                    if let Ok(frame) = frame_message(&pong) {
                                        let _ = write_half.write_all(&frame).await;
                                    }
                                }
                                ClusterMessage::Pong => {
                                    debug!("ClusterPeer '{}': Pong received", node_id);
                                }
                                ClusterMessage::Goodbye => {
                                    info!("ClusterPeer '{}': Received Goodbye", node_id);
                                    return Err(RemoteError::ConnectionLost("Peer disconnected".to_string()));
                                }
                                _ => {}
                            }
                        }

                        // Shift buffer
                        read_buf.copy_within(4 + len..buf_offset, 0);
                        buf_offset -= 4 + len;
                    }
                }

                // Send periodic ping
                _ = ping_timer.tick() => {
                    let ping = ClusterMessage::Ping;
                    if let Ok(frame) = frame_message(&ping) {
                        if let Err(e) = write_half.write_all(&frame).await {
                            return Err(RemoteError::ConnectionLost(e.to_string()));
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl RemotePeer for ClusterPeer {
    fn name(&self) -> &str {
        &self.node_id
    }

    fn status(&self) -> RemotePeerStatus {
        *self.status.read()
    }

    async fn forward_publish(
        &self,
        topic: &str,
        payload: Bytes,
        qos: QoS,
        retain: bool,
    ) -> Result<(), RemoteError> {
        if let Some(ref tx) = self.command_tx {
            tx.send(ClusterCommand::Publish {
                topic: topic.to_string(),
                payload,
                qos,
                retain,
                origin_node: self.local_node_id.clone(),
            })
            .await
            .map_err(|_| RemoteError::ConnectionLost("Command channel closed".to_string()))?;
        }
        Ok(())
    }

    async fn notify_subscribe(&self, filter: &str, _qos: QoS) -> Result<(), RemoteError> {
        // Subscription notifications are handled via gossip + sync,
        // but we can send an incremental update here
        if let Some(ref tx) = self.command_tx {
            tx.send(ClusterCommand::UpdateSubscriptions {
                added: vec![filter.to_string()],
                removed: vec![],
            })
            .await
            .map_err(|_| RemoteError::ConnectionLost("Command channel closed".to_string()))?;
        }
        Ok(())
    }

    async fn notify_unsubscribe(&self, filter: &str) -> Result<(), RemoteError> {
        if let Some(ref tx) = self.command_tx {
            tx.send(ClusterCommand::UpdateSubscriptions {
                added: vec![],
                removed: vec![filter.to_string()],
            })
            .await
            .map_err(|_| RemoteError::ConnectionLost("Command channel closed".to_string()))?;
        }
        Ok(())
    }

    fn should_forward(&self, topic: &str) -> bool {
        // Check if the peer has any subscription that matches this topic
        let subs = self.remote_subscriptions.read();
        let subs_list: Vec<_> = subs.iter().cloned().collect();
        let matches = subs
            .iter()
            .any(|filter| topic_matches_filter(topic, filter));
        tracing::debug!(
            "ClusterPeer '{}': should_forward('{}')={} remote_subs={:?}",
            self.node_id,
            topic,
            matches,
            subs_list
        );
        matches
    }

    async fn start(&self) -> Result<(), RemoteError> {
        info!("ClusterPeer '{}': Starting", self.node_id);
        Ok(())
    }

    async fn stop(&self) -> Result<(), RemoteError> {
        if let Some(ref tx) = self.command_tx {
            let _ = tx.send(ClusterCommand::Shutdown).await;
        }
        info!("ClusterPeer '{}': Stopped", self.node_id);
        Ok(())
    }
}
