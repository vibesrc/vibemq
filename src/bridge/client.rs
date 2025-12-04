//! MQTT Bridge Client
//!
//! Implements a client that connects to a remote MQTT broker and forwards
//! messages according to configured rules.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use parking_lot::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::codec::{Decoder, Encoder};
use crate::protocol::{
    Connect, Disconnect, Packet, Properties, ProtocolVersion, Publish, QoS, ReasonCode, Subscribe,
    Subscription, SubscriptionOptions,
};
use crate::remote::{RemoteError, RemotePeer, RemotePeerStatus};

use super::topic_mapper::TopicMapper;
use crate::config::BridgeConfig;

/// Message to send to the bridge client task
#[derive(Debug)]
enum BridgeCommand {
    /// Publish a message to the remote broker
    Publish {
        topic: String,
        payload: Bytes,
        qos: QoS,
        retain: bool,
    },
    /// Subscribe to a topic on the remote broker
    Subscribe { filter: String, qos: QoS },
    /// Unsubscribe from a topic on the remote broker
    Unsubscribe { filter: String },
    /// Shutdown the bridge
    Shutdown,
}

/// Callback for messages received from the remote broker
pub type InboundCallback = Arc<dyn Fn(String, Bytes, QoS, bool) + Send + Sync>;

/// MQTT Bridge Client
///
/// Connects to a remote MQTT broker and forwards messages bidirectionally
/// based on configured topic rules.
pub struct BridgeClient {
    /// Bridge configuration
    config: BridgeConfig,
    /// Topic mapper for transforming topics
    topic_mapper: TopicMapper,
    /// Current connection status
    status: Arc<RwLock<RemotePeerStatus>>,
    /// Command channel for sending operations to the connection task
    command_tx: Option<mpsc::Sender<BridgeCommand>>,
    /// Callback for inbound messages
    inbound_callback: Option<InboundCallback>,
    /// Next packet ID (for future QoS 1/2 tracking)
    #[allow(dead_code)]
    next_packet_id: AtomicU16,
}

impl BridgeClient {
    /// Create a new bridge client
    pub fn new(config: BridgeConfig) -> Self {
        let topic_mapper = TopicMapper::new(&config.forwards);

        Self {
            config,
            topic_mapper,
            status: Arc::new(RwLock::new(RemotePeerStatus::Disconnected)),
            command_tx: None,
            inbound_callback: None,
            next_packet_id: AtomicU16::new(1),
        }
    }

    /// Set the callback for inbound messages from the remote broker
    pub fn set_inbound_callback(&mut self, callback: InboundCallback) {
        self.inbound_callback = Some(callback);
    }

    /// Get the next packet ID (for future QoS 1/2 tracking)
    #[allow(dead_code)]
    fn next_packet_id(&self) -> u16 {
        let id = self.next_packet_id.fetch_add(1, Ordering::SeqCst);
        if id == 0 {
            self.next_packet_id.fetch_add(1, Ordering::SeqCst)
        } else {
            id
        }
    }

    /// Run the connection loop
    async fn connection_loop(
        config: BridgeConfig,
        topic_mapper: TopicMapper,
        status: Arc<RwLock<RemotePeerStatus>>,
        mut command_rx: mpsc::Receiver<BridgeCommand>,
        inbound_callback: Option<InboundCallback>,
    ) {
        let mut retry_interval = config.reconnect_interval_duration();
        let max_retry = config.max_reconnect_interval_duration();

        loop {
            *status.write() = RemotePeerStatus::Connecting;
            debug!("Bridge '{}': Connecting to {}", config.name, config.address);

            match Self::connect_and_run(
                &config,
                &topic_mapper,
                &status,
                &mut command_rx,
                &inbound_callback,
            )
            .await
            {
                Ok(()) => {
                    info!("Bridge '{}': Disconnected gracefully", config.name);
                    *status.write() = RemotePeerStatus::Disconnected;
                    return; // Clean shutdown
                }
                Err(e) => {
                    error!("Bridge '{}': Connection failed: {}", config.name, e);
                    *status.write() = RemotePeerStatus::Backoff;

                    debug!(
                        "Bridge '{}': Reconnecting in {:?}",
                        config.name, retry_interval
                    );

                    // Exponential backoff
                    tokio::time::sleep(retry_interval).await;
                    retry_interval = std::cmp::min(retry_interval * 2, max_retry);
                }
            }

            // Check for shutdown command
            match command_rx.try_recv() {
                Ok(BridgeCommand::Shutdown) | Err(mpsc::error::TryRecvError::Disconnected) => {
                    info!("Bridge '{}': Shutdown requested", config.name);
                    *status.write() = RemotePeerStatus::Disconnected;
                    return;
                }
                _ => {}
            }
        }
    }

    /// Connect to the remote broker and run the message loop
    async fn connect_and_run(
        config: &BridgeConfig,
        topic_mapper: &TopicMapper,
        status: &Arc<RwLock<RemotePeerStatus>>,
        command_rx: &mut mpsc::Receiver<BridgeCommand>,
        inbound_callback: &Option<InboundCallback>,
    ) -> Result<(), RemoteError> {
        let (host, port) = config.parse_address();

        // Connect with timeout
        let stream = timeout(
            config.connect_timeout_duration(),
            TcpStream::connect(format!("{}:{}", host, port)),
        )
        .await
        .map_err(|_| RemoteError::Timeout)?
        .map_err(|e| RemoteError::ConnectionLost(e.to_string()))?;

        debug!("Bridge '{}': TCP connected", config.name);

        // Set up encoder/decoder
        let encoder = Encoder::new(ProtocolVersion::V5);
        let mut decoder = Decoder::new();
        decoder.set_protocol_version(ProtocolVersion::V5);

        let (mut read_half, mut write_half) = stream.into_split();

        // Send CONNECT packet
        let connect = Packet::Connect(Box::new(Connect {
            protocol_version: ProtocolVersion::V5,
            client_id: config.client_id.clone(),
            clean_start: config.clean_start,
            keep_alive: config.keepalive,
            username: config.username.clone(),
            password: config.password.as_ref().map(|p| Bytes::from(p.clone())),
            will: None,
            properties: Properties::default(),
        }));

        let mut buf = BytesMut::new();
        encoder
            .encode(&connect, &mut buf)
            .map_err(|e| RemoteError::Other(format!("Encode error: {}", e)))?;
        write_half
            .write_all(&buf)
            .await
            .map_err(|e| RemoteError::ConnectionLost(e.to_string()))?;

        debug!("Bridge '{}': CONNECT sent", config.name);

        // Wait for CONNACK
        let mut read_buf = vec![0u8; 4096];
        let n = timeout(
            config.connect_timeout_duration(),
            read_half.read(&mut read_buf),
        )
        .await
        .map_err(|_| RemoteError::Timeout)?
        .map_err(|e| RemoteError::ConnectionLost(e.to_string()))?;

        if n == 0 {
            return Err(RemoteError::ConnectionLost("Connection closed".to_string()));
        }

        let (packet, _) = decoder
            .decode(&read_buf[..n])
            .map_err(|e| RemoteError::Other(format!("Decode error: {}", e)))?
            .ok_or_else(|| RemoteError::Other("Incomplete packet".to_string()))?;

        match packet {
            Packet::ConnAck(connack) => {
                if connack.reason_code != ReasonCode::Success {
                    return Err(RemoteError::Rejected(format!(
                        "CONNACK failed: {:?}",
                        connack.reason_code
                    )));
                }
                info!(
                    "Bridge '{}': Connected (session_present={})",
                    config.name, connack.session_present
                );
            }
            _ => {
                return Err(RemoteError::Other("Expected CONNACK".to_string()));
            }
        }

        *status.write() = RemotePeerStatus::Connected;

        // Subscribe to inbound topics with loop prevention
        let use_no_local = config.use_no_local();
        let inbound_filters = topic_mapper.inbound_filters();
        if !inbound_filters.is_empty() {
            let subscriptions: Vec<Subscription> = inbound_filters
                .iter()
                .map(|(filter, qos)| Subscription {
                    filter: filter.to_string(),
                    options: SubscriptionOptions {
                        qos: *qos,
                        no_local: use_no_local, // Prevents receiving our own messages
                        ..Default::default()
                    },
                })
                .collect();

            let subscribe = Packet::Subscribe(Subscribe {
                packet_id: 1,
                subscriptions,
                properties: Properties::default(),
            });

            buf.clear();
            encoder
                .encode(&subscribe, &mut buf)
                .map_err(|e| RemoteError::Other(format!("Encode error: {}", e)))?;
            write_half
                .write_all(&buf)
                .await
                .map_err(|e| RemoteError::ConnectionLost(e.to_string()))?;

            debug!(
                "Bridge '{}': Subscribed to {} inbound topics",
                config.name,
                inbound_filters.len()
            );
        }

        // Message loop
        let keepalive_interval = Duration::from_secs(config.keepalive as u64);
        let mut keepalive_timer = tokio::time::interval(keepalive_interval);
        keepalive_timer.reset();

        loop {
            tokio::select! {
                // Handle commands from the broker
                Some(cmd) = command_rx.recv() => {
                    match cmd {
                        BridgeCommand::Publish { topic, payload, qos, retain } => {
                            let packet_id = if qos != QoS::AtMostOnce {
                                Some(1) // Simplified - real impl would track packet IDs
                            } else {
                                None
                            };

                            let publish = Packet::Publish(Publish {
                                dup: false,
                                qos,
                                retain,
                                topic,
                                packet_id,
                                payload,
                                properties: Properties::default(),
                            });

                            buf.clear();
                            if encoder.encode(&publish, &mut buf).is_ok() {
                                if let Err(e) = write_half.write_all(&buf).await {
                                    return Err(RemoteError::ConnectionLost(e.to_string()));
                                }
                            }
                        }
                        BridgeCommand::Subscribe { filter, qos } => {
                            let subscribe = Packet::Subscribe(Subscribe {
                                packet_id: 2,
                                subscriptions: vec![Subscription {
                                    filter,
                                    options: SubscriptionOptions { qos, ..Default::default() },
                                }],
                                properties: Properties::default(),
                            });

                            buf.clear();
                            if encoder.encode(&subscribe, &mut buf).is_ok() {
                                let _ = write_half.write_all(&buf).await;
                            }
                        }
                        BridgeCommand::Unsubscribe { filter } => {
                            let unsubscribe = Packet::Unsubscribe(crate::protocol::Unsubscribe {
                                packet_id: 3,
                                filters: vec![filter],
                                properties: Properties::default(),
                            });

                            buf.clear();
                            if encoder.encode(&unsubscribe, &mut buf).is_ok() {
                                let _ = write_half.write_all(&buf).await;
                            }
                        }
                        BridgeCommand::Shutdown => {
                            // Send DISCONNECT
                            let disconnect = Packet::Disconnect(Disconnect {
                                reason_code: ReasonCode::Success,
                                properties: Properties::default(),
                            });

                            buf.clear();
                            if encoder.encode(&disconnect, &mut buf).is_ok() {
                                let _ = write_half.write_all(&buf).await;
                            }
                            return Ok(());
                        }
                    }
                }

                // Handle incoming packets from remote broker
                result = read_half.read(&mut read_buf) => {
                    let n = result.map_err(|e| RemoteError::ConnectionLost(e.to_string()))?;
                    if n == 0 {
                        return Err(RemoteError::ConnectionLost("Connection closed".to_string()));
                    }

                    if let Ok(Some((packet, _))) = decoder.decode(&read_buf[..n]) {
                        match packet {
                            Packet::Publish(publish) => {
                                // Forward to local broker via callback
                                if let Some(ref callback) = inbound_callback {
                                    if let Some((local_topic, qos, retain)) = topic_mapper.map_inbound(
                                        &publish.topic,
                                        publish.qos,
                                        publish.retain,
                                    ) {
                                        debug!(
                                            "Bridge '{}': Forwarding {} -> {}",
                                            config.name, publish.topic, local_topic
                                        );
                                        callback(local_topic, publish.payload, qos, retain);
                                    }
                                }

                                // Send PUBACK for QoS 1
                                if publish.qos == QoS::AtLeastOnce {
                                    if let Some(packet_id) = publish.packet_id {
                                        let puback = Packet::PubAck(crate::protocol::PubAck {
                                            packet_id,
                                            reason_code: ReasonCode::Success,
                                            properties: Properties::default(),
                                        });
                                        buf.clear();
                                        if encoder.encode(&puback, &mut buf).is_ok() {
                                            let _ = write_half.write_all(&buf).await;
                                        }
                                    }
                                }
                            }
                            Packet::PingResp => {
                                debug!("Bridge '{}': PINGRESP received", config.name);
                            }
                            Packet::SubAck(_) => {
                                debug!("Bridge '{}': SUBACK received", config.name);
                            }
                            Packet::PubAck(_) => {
                                debug!("Bridge '{}': PUBACK received", config.name);
                            }
                            Packet::Disconnect(disconnect) => {
                                warn!(
                                    "Bridge '{}': Received DISCONNECT: {:?}",
                                    config.name, disconnect.reason_code
                                );
                                return Err(RemoteError::ConnectionLost("Remote disconnected".to_string()));
                            }
                            _ => {}
                        }
                    }
                }

                // Send PINGREQ to keep connection alive
                _ = keepalive_timer.tick() => {
                    let pingreq = Packet::PingReq;
                    buf.clear();
                    if encoder.encode(&pingreq, &mut buf).is_ok() {
                        if let Err(e) = write_half.write_all(&buf).await {
                            return Err(RemoteError::ConnectionLost(e.to_string()));
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl RemotePeer for BridgeClient {
    fn name(&self) -> &str {
        &self.config.name
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
        // Map the topic and check if we should forward
        let (remote_topic, effective_qos, effective_retain) =
            match self.topic_mapper.map_outbound(topic, qos, retain) {
                Some(mapping) => mapping,
                None => return Ok(()), // Topic doesn't match any rules
            };

        // Send via command channel
        if let Some(ref tx) = self.command_tx {
            tx.send(BridgeCommand::Publish {
                topic: remote_topic,
                payload,
                qos: effective_qos,
                retain: effective_retain,
            })
            .await
            .map_err(|_| RemoteError::ConnectionLost("Command channel closed".to_string()))?;
        }

        Ok(())
    }

    async fn notify_subscribe(&self, filter: &str, qos: QoS) -> Result<(), RemoteError> {
        if let Some(ref tx) = self.command_tx {
            tx.send(BridgeCommand::Subscribe {
                filter: filter.to_string(),
                qos,
            })
            .await
            .map_err(|_| RemoteError::ConnectionLost("Command channel closed".to_string()))?;
        }
        Ok(())
    }

    async fn notify_unsubscribe(&self, filter: &str) -> Result<(), RemoteError> {
        if let Some(ref tx) = self.command_tx {
            tx.send(BridgeCommand::Unsubscribe {
                filter: filter.to_string(),
            })
            .await
            .map_err(|_| RemoteError::ConnectionLost("Command channel closed".to_string()))?;
        }
        Ok(())
    }

    fn should_forward(&self, topic: &str) -> bool {
        self.topic_mapper.should_forward_outbound(topic)
    }

    async fn start(&self) -> Result<(), RemoteError> {
        if !self.config.enabled {
            info!("Bridge '{}': Disabled, not starting", self.config.name);
            return Ok(());
        }

        // This is a simplified start - in production, we'd spawn the connection loop
        // and store the command_tx in self. For now, this shows the structure.
        info!("Bridge '{}': Starting", self.config.name);
        Ok(())
    }

    async fn stop(&self) -> Result<(), RemoteError> {
        if let Some(ref tx) = self.command_tx {
            let _ = tx.send(BridgeCommand::Shutdown).await;
        }
        info!("Bridge '{}': Stopped", self.config.name);
        Ok(())
    }
}

impl BridgeClient {
    /// Spawn the connection task and return the bridge client ready to use
    pub fn spawn(mut self, inbound_callback: InboundCallback) -> Arc<Self> {
        let (tx, rx) = mpsc::channel(1000);
        self.command_tx = Some(tx);
        self.inbound_callback = Some(inbound_callback);

        let config = self.config.clone();
        let topic_mapper = TopicMapper::new(&config.forwards);
        let status = self.status.clone();
        let callback = self.inbound_callback.clone();

        tokio::spawn(async move {
            Self::connection_loop(config, topic_mapper, status, rx, callback).await;
        });

        Arc::new(self)
    }
}
