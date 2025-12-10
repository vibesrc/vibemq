//! MQTT Broker Core
//!
//! The main broker implementation that handles client connections,
//! message routing, and coordinates all components.

mod connection;
mod router;

pub use connection::Connection;
pub use router::MessageRouter;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ahash::AHashMap;
use bytes::Bytes;
use dashmap::DashMap;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};

use crate::bridge::BridgeManager;
use crate::cluster::ClusterManager;
use crate::hooks::{DefaultHooks, Hooks};
use crate::metrics::Metrics;
use crate::protocol::{Packet, Properties, ProtocolVersion, Publish, QoS};
use crate::session::SessionStore;
use crate::topic::SubscriptionStore;
use crate::transport::WsStream;

/// Broker configuration
#[derive(Debug, Clone)]
pub struct BrokerConfig {
    /// TCP bind address
    pub bind_addr: SocketAddr,
    /// WebSocket bind address (optional)
    pub ws_bind_addr: Option<SocketAddr>,
    /// WebSocket path (default: "/mqtt")
    pub ws_path: String,
    /// Maximum connections
    pub max_connections: usize,
    /// Maximum packet size
    pub max_packet_size: usize,
    /// Default keep alive (if client specifies 0)
    pub default_keep_alive: u16,
    /// Maximum keep alive
    pub max_keep_alive: u16,
    /// Session expiry check interval
    pub session_expiry_check_interval: Duration,
    /// Receive maximum (flow control)
    pub receive_maximum: u16,
    /// Maximum QoS
    pub max_qos: QoS,
    /// Retain available
    pub retain_available: bool,
    /// Wildcard subscription available
    pub wildcard_subscription_available: bool,
    /// Subscription identifiers available
    pub subscription_identifiers_available: bool,
    /// Shared subscriptions available
    pub shared_subscriptions_available: bool,
    /// Maximum topic alias
    pub max_topic_alias: u16,
    /// Number of worker tasks
    pub num_workers: usize,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:1883".parse().unwrap(),
            ws_bind_addr: None,
            ws_path: "/mqtt".to_string(),
            max_connections: 100_000,
            max_packet_size: 1024 * 1024, // 1 MB
            default_keep_alive: 60,
            max_keep_alive: 65535,
            session_expiry_check_interval: Duration::from_secs(60),
            receive_maximum: 65535,
            max_qos: QoS::ExactlyOnce,
            retain_available: true,
            wildcard_subscription_available: true,
            subscription_identifiers_available: true,
            shared_subscriptions_available: true,
            max_topic_alias: 65535,
            num_workers: num_cpus::get(),
        }
    }
}

// Helper to get number of CPUs
mod num_cpus {
    pub fn get() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    }
}

/// Retained message
#[derive(Debug, Clone)]
pub struct RetainedMessage {
    pub topic: String,
    pub payload: Bytes,
    pub qos: QoS,
    pub properties: Properties,
    pub timestamp: Instant,
}

/// Broker events
#[derive(Debug, Clone)]
pub enum BrokerEvent {
    /// Client connected
    ClientConnected {
        client_id: Arc<str>,
        protocol_version: ProtocolVersion,
    },
    /// Client disconnected
    ClientDisconnected { client_id: Arc<str> },
    /// Message published (includes payload for bridge forwarding)
    MessagePublished {
        topic: String,
        payload: Bytes,
        qos: QoS,
        retain: bool,
    },
    /// Subscription added (for cluster synchronization)
    SubscriptionAdded { filter: String, client_id: Arc<str> },
    /// Subscription removed (for cluster synchronization)
    SubscriptionRemoved { filter: String, client_id: Arc<str> },
}

/// The MQTT Broker
pub struct Broker {
    /// Configuration
    config: BrokerConfig,
    /// Session store
    sessions: Arc<SessionStore>,
    /// Subscription store
    subscriptions: Arc<SubscriptionStore>,
    /// Retained messages
    retained: Arc<DashMap<String, RetainedMessage>>,
    /// Active connections (client_id -> connection handle)
    connections: Arc<DashMap<Arc<str>, mpsc::Sender<Packet>>>,
    /// Shutdown signal
    shutdown: broadcast::Sender<()>,
    /// Event channel
    events: broadcast::Sender<BrokerEvent>,
    /// Hooks for auth/ACL and events
    hooks: Arc<dyn Hooks>,
    /// Bridge manager for remote broker connections
    bridge_manager: Option<Arc<BridgeManager>>,
    /// Cluster manager for horizontal scaling
    cluster_manager: Option<Arc<ClusterManager>>,
    /// Metrics for observability
    metrics: Option<Arc<Metrics>>,
}

impl Broker {
    /// Create a new broker with default hooks (allows everything)
    pub fn new(config: BrokerConfig) -> Self {
        Self::with_hooks(config, Arc::new(DefaultHooks))
    }

    /// Create a new broker with custom hooks
    pub fn with_hooks(config: BrokerConfig, hooks: Arc<dyn Hooks>) -> Self {
        let (shutdown, _) = broadcast::channel(1);
        let (events, _) = broadcast::channel(1024);

        Self {
            config,
            sessions: Arc::new(SessionStore::new()),
            subscriptions: Arc::new(SubscriptionStore::new()),
            retained: Arc::new(DashMap::new()),
            connections: Arc::new(DashMap::new()),
            shutdown,
            events,
            hooks,
            bridge_manager: None,
            cluster_manager: None,
            metrics: None,
        }
    }

    /// Set metrics for this broker
    pub fn set_metrics(&mut self, metrics: Arc<Metrics>) {
        self.metrics = Some(metrics);
    }

    /// Get metrics (if enabled)
    pub fn metrics(&self) -> Option<&Arc<Metrics>> {
        self.metrics.as_ref()
    }

    /// Set the bridge manager for this broker
    pub fn set_bridge_manager(&mut self, manager: BridgeManager) {
        self.bridge_manager = Some(Arc::new(manager));
    }

    /// Set the cluster manager for this broker
    pub fn set_cluster_manager(&mut self, manager: ClusterManager) {
        self.cluster_manager = Some(Arc::new(manager));
    }

    /// Create a cluster manager with inbound callback that publishes to this broker
    pub async fn create_cluster_manager(
        &self,
        config: crate::config::ClusterConfig,
    ) -> Result<ClusterManager, Box<dyn std::error::Error + Send + Sync>> {
        let retained = self.retained.clone();
        let sessions = self.sessions.clone();
        let subscriptions = self.subscriptions.clone();
        let connections = self.connections.clone();

        // Callback for messages received from cluster peers
        let inbound_callback = Arc::new(
            move |topic: String, payload: Bytes, qos: QoS, retain: bool, _origin_node: String| {
                debug!(
                    "Cluster inbound_callback: routing '{}' to local subscribers",
                    topic
                );

                // Create a publish packet
                let publish = Publish {
                    dup: false,
                    qos,
                    retain,
                    topic: topic.clone(),
                    packet_id: None,
                    payload: payload.clone(),
                    properties: Properties::default(),
                };

                // Handle retained message
                if retain {
                    if payload.is_empty() {
                        retained.remove(&topic);
                    } else {
                        retained.insert(
                            topic.clone(),
                            RetainedMessage {
                                topic: topic.clone(),
                                payload,
                                qos,
                                properties: Properties::default(),
                                timestamp: Instant::now(),
                            },
                        );
                    }
                }

                // Route to local subscribers only
                let matches = subscriptions.matches(&topic);

                // Deduplicate by client_id (keep highest QoS) - use AHashMap for faster lookup
                let mut client_qos: AHashMap<Arc<str>, QoS> =
                    AHashMap::with_capacity(matches.len());
                for sub in matches {
                    let entry = client_qos
                        .entry(sub.client_id.clone())
                        .or_insert(QoS::AtMostOnce);
                    if sub.qos > *entry {
                        *entry = sub.qos;
                    }
                }

                debug!(
                    "Cluster inbound_callback: found {} local subscribers for '{}'",
                    client_qos.len(),
                    topic
                );

                // Send to each local client
                for (client_id, sub_qos) in client_qos {
                    let effective_qos = qos.min(sub_qos);

                    if let Some(sender) = connections.get(&client_id) {
                        let mut publish = publish.clone();
                        publish.qos = effective_qos;
                        match sender.try_send(Packet::Publish(publish)) {
                            Ok(()) => {
                                debug!("Cluster inbound_callback: sent to client {}", client_id)
                            }
                            Err(e) => debug!(
                                "Cluster inbound_callback: failed to send to {}: {:?}",
                                client_id, e
                            ),
                        }
                    } else {
                        // Client disconnected, queue message if persistent session
                        if let Some(session) = sessions.get(client_id.as_ref()) {
                            let mut s = session.write();
                            if !s.clean_start {
                                let mut publish = publish.clone();
                                publish.qos = effective_qos;
                                s.queue_message(publish);
                            }
                        }
                    }
                }
            },
        );

        ClusterManager::new(config, inbound_callback).await
    }

    /// Create a bridge manager with inbound callback that publishes to this broker
    pub fn create_bridge_manager(
        &self,
        configs: Vec<crate::bridge::BridgeConfig>,
    ) -> BridgeManager {
        let retained = self.retained.clone();
        let sessions = self.sessions.clone();
        let subscriptions = self.subscriptions.clone();
        let connections = self.connections.clone();

        let inbound_callback = Arc::new(
            move |topic: String, payload: Bytes, qos: QoS, retain: bool| {
                // Create a publish packet
                let publish = Publish {
                    dup: false,
                    qos,
                    retain,
                    topic: topic.clone(),
                    packet_id: None,
                    payload: payload.clone(),
                    properties: Properties::default(),
                };

                // Handle retained message
                if retain {
                    if payload.is_empty() {
                        retained.remove(&topic);
                    } else {
                        retained.insert(
                            topic.clone(),
                            RetainedMessage {
                                topic: topic.clone(),
                                payload,
                                qos,
                                properties: Properties::default(),
                                timestamp: Instant::now(),
                            },
                        );
                    }
                }

                // Route to subscribers
                let matches = subscriptions.matches(&topic);

                // Deduplicate by client_id (keep highest QoS) - use AHashMap for faster lookup
                let mut client_qos: AHashMap<Arc<str>, QoS> =
                    AHashMap::with_capacity(matches.len());
                for sub in matches {
                    let entry = client_qos
                        .entry(sub.client_id.clone())
                        .or_insert(QoS::AtMostOnce);
                    if sub.qos > *entry {
                        *entry = sub.qos;
                    }
                }

                // Send to each client
                for (client_id, sub_qos) in client_qos {
                    let effective_qos = qos.min(sub_qos);

                    if let Some(sender) = connections.get(&client_id) {
                        let mut publish = publish.clone();
                        publish.qos = effective_qos;
                        let _ = sender.try_send(Packet::Publish(publish));
                    } else {
                        // Client disconnected, queue message if persistent session
                        if let Some(session) = sessions.get(client_id.as_ref()) {
                            let mut s = session.write();
                            if !s.clean_start {
                                let mut publish = publish.clone();
                                publish.qos = effective_qos;
                                s.queue_message(publish);
                            }
                        }
                    }
                }
            },
        );

        BridgeManager::from_configs(configs, inbound_callback)
    }

    /// Run the broker
    pub async fn run(&self) -> Result<(), std::io::Error> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        info!("MQTT/TCP listening on {}", self.config.bind_addr);

        // Spawn WebSocket listener if configured
        if let Some(ws_addr) = self.config.ws_bind_addr {
            let ws_listener = TcpListener::bind(ws_addr).await?;
            info!(
                "MQTT/WebSocket listening on {} (path: {})",
                ws_addr, self.config.ws_path
            );

            let sessions = self.sessions.clone();
            let subscriptions = self.subscriptions.clone();
            let retained = self.retained.clone();
            let connections = self.connections.clone();
            let config = self.config.clone();
            let events = self.events.clone();
            let shutdown = self.shutdown.clone();
            let hooks = self.hooks.clone();

            tokio::spawn(async move {
                loop {
                    match ws_listener.accept().await {
                        Ok((stream, addr)) => {
                            debug!("New WebSocket connection from {}", addr);
                            let sessions = sessions.clone();
                            let subscriptions = subscriptions.clone();
                            let retained = retained.clone();
                            let connections = connections.clone();
                            let config = config.clone();
                            let events = events.clone();
                            let hooks = hooks.clone();
                            let mut shutdown_rx = shutdown.subscribe();

                            tokio::spawn(async move {
                                // Perform WebSocket handshake with path validation
                                match WsStream::accept_with_path(stream, &config.ws_path).await {
                                    Ok(ws_stream) => {
                                        debug!("WebSocket handshake complete for {}", addr);
                                        let mut conn = Connection::new(
                                            ws_stream,
                                            addr,
                                            sessions,
                                            subscriptions,
                                            retained,
                                            connections,
                                            config,
                                            events,
                                            hooks,
                                        );

                                        let conn_fut = conn.run();
                                        tokio::pin!(conn_fut);

                                        loop {
                                            tokio::select! {
                                                biased;

                                                result = &mut conn_fut => {
                                                    if let Err(e) = result {
                                                        debug!("WebSocket connection error from {}: {}", addr, e);
                                                    }
                                                    break;
                                                }
                                                result = shutdown_rx.recv() => {
                                                    match result {
                                                        Ok(()) => break,
                                                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        debug!("WebSocket handshake failed for {}: {}", addr, e);
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            error!("Failed to accept WebSocket connection: {}", e);
                        }
                    }
                }
            });
        }

        // Spawn session expiry cleanup task
        let sessions = self.sessions.clone();
        let interval = self.config.session_expiry_check_interval;
        let mut shutdown_rx = self.shutdown.subscribe();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    biased;

                    _ = ticker.tick() => {
                        sessions.cleanup_expired();
                    }
                    result = shutdown_rx.recv() => {
                        match result {
                            Ok(()) => break,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });

        // Spawn bridge forwarding task if bridges are configured
        if let Some(ref bridge_manager) = self.bridge_manager {
            let bridge_manager = bridge_manager.clone();
            let mut events_rx = self.events.subscribe();
            let mut shutdown_rx = self.shutdown.subscribe();

            info!(
                "Starting bridge manager with {} bridge(s)",
                bridge_manager.bridge_count()
            );

            tokio::spawn(async move {
                // Start all bridges
                bridge_manager.start_all().await;

                loop {
                    tokio::select! {
                        biased;

                        result = events_rx.recv() => {
                            match result {
                                Ok(BrokerEvent::MessagePublished { topic, payload, qos, retain }) => {
                                    // Forward to bridges
                                    bridge_manager.forward_publish(&topic, payload, qos, retain).await;
                                }
                                Ok(_) => {} // Ignore other events
                                Err(broadcast::error::RecvError::Lagged(n)) => {
                                    warn!("Bridge event listener lagged, missed {} events", n);
                                }
                                Err(broadcast::error::RecvError::Closed) => break,
                            }
                        }
                        result = shutdown_rx.recv() => {
                            match result {
                                Ok(()) => {
                                    info!("Stopping bridges");
                                    bridge_manager.stop_all().await;
                                    break;
                                }
                                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    }
                }
            });
        }

        // Spawn cluster forwarding task if clustering is enabled
        if let Some(ref cluster_manager) = self.cluster_manager {
            let cluster_manager = cluster_manager.clone();
            let mut events_rx = self.events.subscribe();
            let mut shutdown_rx = self.shutdown.subscribe();

            info!(
                "Starting cluster manager (node_id={}, peers={})",
                cluster_manager.node_id(),
                cluster_manager.peer_count()
            );

            // Start cluster manager
            if let Err(e) = cluster_manager.start().await {
                error!("Failed to start cluster manager: {}", e);
            }

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;

                        result = events_rx.recv() => {
                            match result {
                                Ok(BrokerEvent::MessagePublished { topic, payload, qos, retain }) => {
                                    // Forward to cluster peers
                                    debug!("Cluster: forwarding publish to topic '{}' (peers={})", topic, cluster_manager.peer_count());
                                    cluster_manager.forward_publish(&topic, payload, qos, retain).await;
                                }
                                Ok(BrokerEvent::SubscriptionAdded { filter, client_id }) => {
                                    // Update cluster subscription state
                                    debug!("Cluster: subscription added '{}' by {}", filter, client_id);
                                    cluster_manager.add_subscription(filter).await;
                                }
                                Ok(BrokerEvent::SubscriptionRemoved { filter, client_id }) => {
                                    // Update cluster subscription state
                                    debug!("Cluster: subscription removed '{}' by {}", filter, client_id);
                                    cluster_manager.remove_subscription(&filter).await;
                                }
                                Ok(_) => {} // Ignore other events
                                Err(broadcast::error::RecvError::Lagged(n)) => {
                                    warn!("Cluster event listener lagged, missed {} events", n);
                                }
                                Err(broadcast::error::RecvError::Closed) => break,
                            }
                        }
                        result = shutdown_rx.recv() => {
                            match result {
                                Ok(()) => {
                                    info!("Stopping cluster manager");
                                    cluster_manager.stop().await;
                                    break;
                                }
                                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                Err(broadcast::error::RecvError::Closed) => break,
                            }
                        }
                    }
                }
            });
        }

        // Spawn metrics collection task if metrics are enabled
        if let Some(ref metrics) = self.metrics {
            let metrics = metrics.clone();
            let mut events_rx = self.events.subscribe();
            let mut shutdown_rx = self.shutdown.subscribe();

            info!("Starting metrics collection");

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;

                        result = events_rx.recv() => {
                            match result {
                                Ok(BrokerEvent::ClientConnected { protocol_version, .. }) => {
                                    let protocol = match protocol_version {
                                        ProtocolVersion::V311 => "v3.1.1",
                                        ProtocolVersion::V5 => "v5.0",
                                    };
                                    metrics.client_connected(protocol);
                                }
                                Ok(BrokerEvent::ClientDisconnected { .. }) => {
                                    // Note: We don't know the protocol here, so we just decrement total
                                    // In a more complete impl, we'd track protocol per client
                                    metrics.connections_current.dec();
                                }
                                Ok(BrokerEvent::MessagePublished { payload, .. }) => {
                                    metrics.messages_received_total.with_label_values(&["publish"]).inc();
                                    metrics.messages_bytes_received.inc_by(payload.len() as u64);
                                }
                                Ok(BrokerEvent::SubscriptionAdded { .. }) => {
                                    metrics.subscription_added();
                                }
                                Ok(BrokerEvent::SubscriptionRemoved { .. }) => {
                                    metrics.subscription_removed();
                                }
                                Err(broadcast::error::RecvError::Lagged(n)) => {
                                    warn!("Metrics event listener lagged, missed {} events", n);
                                }
                                Err(broadcast::error::RecvError::Closed) => break,
                            }
                        }
                        result = shutdown_rx.recv() => {
                            match result {
                                Ok(()) | Err(broadcast::error::RecvError::Closed) => break,
                                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            }
                        }
                    }
                }
            });
        }

        debug!("Starting TCP accept loop");
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    debug!("New TCP connection from {}", addr);
                    self.handle_connection(stream, addr);
                }
                Err(e) => {
                    error!("Failed to accept TCP connection: {}", e);
                }
            }
        }
    }

    /// Handle a new connection
    fn handle_connection(&self, stream: TcpStream, addr: SocketAddr) {
        let sessions = self.sessions.clone();
        let subscriptions = self.subscriptions.clone();
        let retained = self.retained.clone();
        let connections = self.connections.clone();
        let config = self.config.clone();
        let events = self.events.clone();
        let hooks = self.hooks.clone();
        let mut shutdown_rx = self.shutdown.subscribe();

        tokio::spawn(async move {
            let mut conn = Connection::new(
                stream,
                addr,
                sessions,
                subscriptions,
                retained,
                connections,
                config,
                events,
                hooks,
            );

            // Pin the connection future so we can poll it repeatedly
            let conn_fut = conn.run();
            tokio::pin!(conn_fut);

            loop {
                tokio::select! {
                    biased;

                    result = &mut conn_fut => {
                        if let Err(e) = result {
                            debug!("Connection error from {}: {}", addr, e);
                        }
                        break;
                    }
                    result = shutdown_rx.recv() => {
                        match result {
                            Ok(()) => {
                                debug!("Connection {} shutting down", addr);
                                break;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                debug!("Connection {} shutdown (channel closed)", addr);
                                break;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                // Missed some messages, continue running
                                continue;
                            }
                        }
                    }
                }
            }
        });
    }

    /// Shutdown the broker
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(());
    }

    /// Subscribe to broker events
    pub fn subscribe_events(&self) -> broadcast::Receiver<BrokerEvent> {
        self.events.subscribe()
    }

    /// Get session count
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Get connection count
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Get retained message count
    pub fn retained_count(&self) -> usize {
        self.retained.len()
    }

    /// Publish a message from the server
    pub fn publish(&self, topic: String, payload: Bytes, qos: QoS, retain: bool) {
        // Create a publish packet
        let publish = Publish {
            dup: false,
            qos,
            retain,
            topic: topic.clone(),
            packet_id: None,
            payload: payload.clone(),
            properties: Properties::default(),
        };

        // Handle retained message
        if retain {
            if payload.is_empty() {
                self.retained.remove(&topic);
            } else {
                self.retained.insert(
                    topic.clone(),
                    RetainedMessage {
                        topic: topic.clone(),
                        payload,
                        qos,
                        properties: Properties::default(),
                        timestamp: Instant::now(),
                    },
                );
            }
        }

        // Route to subscribers
        let matches = self.subscriptions.matches(&topic);

        // Deduplicate by client_id (keep highest QoS) - use AHashMap for faster lookup
        let mut client_qos: AHashMap<Arc<str>, QoS> = AHashMap::with_capacity(matches.len());
        for sub in matches {
            let entry = client_qos
                .entry(sub.client_id.clone())
                .or_insert(QoS::AtMostOnce);
            if sub.qos > *entry {
                *entry = sub.qos;
            }
        }

        // Send to each client
        for (client_id, sub_qos) in client_qos {
            let effective_qos = qos.min(sub_qos);

            if let Some(sender) = self.connections.get(&client_id) {
                let mut publish = publish.clone();
                publish.qos = effective_qos;

                // For QoS > 0, packet_id will be assigned by the connection handler
                let _ = sender.try_send(Packet::Publish(publish));
            } else {
                // Client disconnected, queue message if persistent session
                if let Some(session) = self.sessions.get(client_id.as_ref()) {
                    let mut s = session.write();
                    if !s.clean_start {
                        let mut publish = publish.clone();
                        publish.qos = effective_qos;
                        s.queue_message(publish);
                    }
                }
            }
        }
    }
}

impl Default for Broker {
    fn default() -> Self {
        Self::new(BrokerConfig::default())
    }
}
