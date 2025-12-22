//! $SYS Topics Publisher
//!
//! Publishes broker statistics as retained messages to standard $SYS/# topics.
//! Topics are updated periodically based on configuration.

use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;

use super::Broker;
use crate::metrics::Metrics;
use crate::protocol::QoS;

/// Version string for $SYS/broker/version
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Publish all $SYS topics as retained messages
pub fn publish_sys_topics(broker: &Broker, metrics: Option<&Metrics>, start_time: Instant) {
    let uptime = start_time.elapsed().as_secs();

    // Broker info (always available)
    publish(broker, "$SYS/broker/version", VERSION);
    publish(broker, "$SYS/broker/uptime", &uptime.to_string());

    // Session store stats (always available)
    let disconnected_count = broker.sessions.count_disconnected();
    let messages_stored = broker.sessions.total_queued_messages();
    let shared_subs_count = broker.subscriptions.shared_subscription_count();

    // Metrics-dependent stats
    if let Some(metrics) = metrics {
        // Client metrics - existing
        publish(
            broker,
            "$SYS/broker/clients/connected",
            &metrics.connections_current.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/clients/total",
            &metrics.connections_total.get().to_string(),
        );

        // Client metrics - new
        publish(
            broker,
            "$SYS/broker/clients/active",
            &metrics.connections_current.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/clients/inactive",
            &disconnected_count.to_string(),
        );
        publish(
            broker,
            "$SYS/broker/clients/disconnected",
            &disconnected_count.to_string(),
        );
        publish(
            broker,
            "$SYS/broker/clients/expired",
            &metrics.sessions_expired_total.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/clients/maximum",
            &metrics.connections_maximum.get().to_string(),
        );

        // Subscription metrics
        publish(
            broker,
            "$SYS/broker/subscriptions/count",
            &metrics.subscriptions_current.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/shared_subscriptions/count",
            &shared_subs_count.to_string(),
        );

        // Retained messages - existing
        publish(
            broker,
            "$SYS/broker/retained messages/count",
            &metrics.retained_messages_current.get().to_string(),
        );

        // Byte metrics - existing (general)
        publish(
            broker,
            "$SYS/broker/bytes/received",
            &metrics.messages_bytes_received.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/bytes/sent",
            &metrics.messages_bytes_sent.get().to_string(),
        );

        // Message metrics - new (total packets)
        publish(
            broker,
            "$SYS/broker/messages/received",
            &metrics.messages_total_received.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/messages/sent",
            &metrics.messages_total_sent.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/messages/stored",
            &messages_stored.to_string(),
        );

        // Publish metrics - new
        publish(
            broker,
            "$SYS/broker/publish/bytes/received",
            &metrics.messages_bytes_received.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/publish/bytes/sent",
            &metrics.messages_bytes_sent.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/publish/messages/received",
            &metrics.publish_messages_received.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/publish/messages/sent",
            &metrics.publish_messages_sent.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/publish/messages/dropped",
            &metrics.publish_messages_dropped.get().to_string(),
        );

        // Store metrics - new
        publish(
            broker,
            "$SYS/broker/store/messages/count",
            &metrics.retained_messages_current.get().to_string(),
        );
        publish(
            broker,
            "$SYS/broker/store/messages/bytes",
            &metrics.retained_bytes_current.get().to_string(),
        );
    }
}

/// Helper to publish a single $SYS topic as QoS 0 retained
fn publish(broker: &Broker, topic: &str, value: &str) {
    broker.publish(
        topic.to_string(),
        Bytes::from(value.to_string()),
        QoS::AtMostOnce,
        true, // retained
    );
}

/// Spawn the $SYS topics publishing task
pub fn spawn_sys_topics_task(
    broker: Arc<Broker>,
    metrics: Option<Arc<Metrics>>,
    interval_secs: u64,
    start_time: Instant,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs));

        // Publish immediately on startup
        publish_sys_topics(&broker, metrics.as_deref(), start_time);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    publish_sys_topics(&broker, metrics.as_deref(), start_time);
                }
                _ = shutdown_rx.recv() => {
                    tracing::debug!("$SYS topics task shutting down");
                    break;
                }
            }
        }
    });
}
