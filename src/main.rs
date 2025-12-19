//! VibeMQ - High-performance MQTT v3.1.1/v5.0 compliant broker
//!
//! Usage:
//!   vibemq [OPTIONS]
//!
//! Options:
//!   -c, --config <FILE>    Configuration file path
//!   -b, --bind <ADDR>      Bind address (default: 0.0.0.0:1883)
//!   -w, --workers <N>      Number of worker threads (default: CPU count)
//!   --max-connections <N>  Maximum connections (default: 100000)
//!   --max-packet-size <N>  Maximum packet size (default: 1MB)
//!   -l, --log-level        Log level (error, warn, info, debug, trace)
//!   -h, --help             Print help

// Use jemalloc for heap profiling when pprof feature is enabled
#[cfg(feature = "pprof")]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use vibemq::acl::AclProvider;
use vibemq::auth::AuthProvider;
use vibemq::broker::{Broker, BrokerConfig, TlsConfig};
use vibemq::config::Config;
use vibemq::hooks::CompositeHooks;
use vibemq::protocol::QoS;

/// Log level for CLI
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
enum LogLevel {
    /// Only errors
    Error,
    /// Warnings and errors
    #[default]
    Warn,
    /// Informational messages
    Info,
    /// Debug messages
    Debug,
    /// Trace messages (very verbose)
    Trace,
}

impl LogLevel {
    fn to_tracing_level(self) -> Level {
        match self {
            LogLevel::Error => Level::ERROR,
            LogLevel::Warn => Level::WARN,
            LogLevel::Info => Level::INFO,
            LogLevel::Debug => Level::DEBUG,
            LogLevel::Trace => Level::TRACE,
        }
    }
}

/// VibeMQ - High-performance MQTT broker
#[derive(Parser, Debug)]
#[command(name = "vibemq")]
#[command(author = "VibeMQ Contributors")]
#[command(version = "0.1.0")]
#[command(about = "High-performance MQTT v3.1.1/v5.0 compliant broker")]
struct Args {
    /// Configuration file path (TOML format)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// TCP bind address
    #[arg(short, long)]
    bind: Option<SocketAddr>,

    /// WebSocket bind address (optional, enables MQTT over WebSocket)
    #[arg(long)]
    ws_bind: Option<SocketAddr>,

    /// Number of worker threads (0 = auto)
    #[arg(short, long)]
    workers: Option<usize>,

    /// Maximum connections
    #[arg(long)]
    max_connections: Option<usize>,

    /// Maximum packet size in bytes
    #[arg(long)]
    max_packet_size: Option<usize>,

    /// Maximum QoS level (0, 1, or 2)
    #[arg(long)]
    max_qos: Option<u8>,

    /// Default keep alive in seconds
    #[arg(long)]
    keep_alive: Option<u16>,

    /// Enable retained messages
    #[arg(long)]
    retain: Option<bool>,

    /// Enable wildcard subscriptions
    #[arg(long)]
    wildcard_subs: Option<bool>,

    /// Maximum topic aliases
    #[arg(long)]
    max_topic_alias: Option<u16>,

    /// Receive maximum (flow control)
    #[arg(long)]
    receive_maximum: Option<u16>,

    /// Log level (error, warn, info, debug, trace)
    #[arg(short, long, value_enum)]
    log_level: Option<LogLevel>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Load configuration file if specified, otherwise use defaults
    let file_config = if let Some(config_path) = &args.config {
        match Config::load(config_path) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("Error loading config file: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        Config::default()
    };

    // Setup logging - CLI overrides config, config overrides default (warn)
    let log_level = args.log_level.unwrap_or_else(|| {
        // Parse from config string
        match file_config.log.level.to_lowercase().as_str() {
            "error" => LogLevel::Error,
            "warn" => LogLevel::Warn,
            "info" => LogLevel::Info,
            "debug" => LogLevel::Debug,
            "trace" => LogLevel::Trace,
            _ => LogLevel::Warn,
        }
    });

    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level.to_tracing_level())
        .with_target(false)
        .with_thread_ids(true)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    if args.config.is_some() {
        info!(
            "Loaded configuration from {:?}",
            args.config.as_ref().unwrap()
        );
    }

    // CLI args override file config
    let bind_addr = args.bind.unwrap_or(file_config.server.bind);
    let tls_bind_addr = file_config.server.tls_bind;
    let tls_config = file_config.server.tls.as_ref().map(|tls| TlsConfig {
        cert_path: tls.cert.clone(),
        key_path: tls.key.clone(),
        ca_cert_path: tls.ca_cert.clone(),
        require_client_cert: tls.require_client_cert,
    });
    let ws_bind_addr = args.ws_bind.or(file_config.server.ws_bind);
    let max_connections = args
        .max_connections
        .unwrap_or(file_config.limits.max_connections);
    let max_packet_size = args
        .max_packet_size
        .unwrap_or(file_config.limits.max_packet_size);
    let keep_alive = args
        .keep_alive
        .unwrap_or(file_config.session.default_keep_alive);
    let max_keep_alive = file_config.session.max_keep_alive;
    let max_topic_alias = args
        .max_topic_alias
        .unwrap_or(file_config.session.max_topic_aliases);
    let receive_maximum = args.receive_maximum.unwrap_or(65535);
    let retain_available = args.retain.unwrap_or(file_config.mqtt.retain_available);
    let wildcard_subs = args
        .wildcard_subs
        .unwrap_or(file_config.mqtt.wildcard_subscriptions);

    // Parse max QoS
    let max_qos_value = args.max_qos.unwrap_or(file_config.mqtt.max_qos);
    let max_qos = match max_qos_value {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => {
            eprintln!(
                "Invalid max-qos value: {}. Must be 0, 1, or 2.",
                max_qos_value
            );
            std::process::exit(1);
        }
    };

    // Determine worker count
    let workers = args.workers.unwrap_or(file_config.server.workers);
    let num_workers = if workers == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        workers
    };

    // Build broker configuration
    let broker_config = BrokerConfig {
        bind_addr,
        tls_bind_addr,
        tls_config,
        ws_bind_addr,
        ws_path: file_config.server.ws_path.clone(),
        max_connections,
        max_packet_size,
        default_keep_alive: keep_alive,
        max_keep_alive,
        session_expiry_check_interval: Duration::from_secs(
            file_config.session.expiry_check_interval,
        ),
        receive_maximum,
        max_qos,
        retain_available,
        wildcard_subscription_available: wildcard_subs,
        subscription_identifiers_available: file_config.mqtt.subscription_identifiers,
        shared_subscriptions_available: file_config.mqtt.shared_subscriptions,
        max_topic_alias,
        num_workers,
        sys_topics_enabled: file_config.mqtt.sys_topics,
        sys_topics_interval: Duration::from_secs(file_config.mqtt.sys_interval),
    };

    info!("Starting VibeMQ MQTT Broker");
    info!("  Bind address: {}", broker_config.bind_addr);
    if let Some(tls_addr) = &broker_config.tls_bind_addr {
        info!("  TLS address: {}", tls_addr);
    }
    if let Some(ws_addr) = &broker_config.ws_bind_addr {
        info!("  WebSocket address: {}", ws_addr);
    }
    info!("  Workers: {}", broker_config.num_workers);
    info!("  Max connections: {}", broker_config.max_connections);
    info!("  Max packet size: {} bytes", broker_config.max_packet_size);
    info!("  Max QoS: {:?}", broker_config.max_qos);

    // Log auth/ACL status
    if file_config.auth.enabled {
        info!(
            "  Authentication: enabled ({} users configured)",
            file_config.auth.users.len()
        );
    } else {
        info!("  Authentication: disabled");
    }
    if file_config.acl.enabled {
        info!(
            "  ACL: enabled ({} roles configured)",
            file_config.acl.roles.len()
        );
    } else {
        info!("  ACL: disabled");
    }

    // Create auth and ACL providers
    let auth_provider = Arc::new(AuthProvider::new(&file_config.auth));
    let acl_provider = Arc::new(AclProvider::new(&file_config.acl, auth_provider.clone()));

    // Compose hooks: auth first, then ACL
    let hooks = Arc::new(CompositeHooks::new().with(auth_provider).with(acl_provider));

    // Create broker with hooks
    let mut broker = Broker::with_hooks(broker_config, hooks);

    // Setup bridges if configured
    let enabled_bridges = file_config.bridge.iter().filter(|b| b.enabled).count();
    info!(
        "  Bridges: {} configured ({} enabled)",
        file_config.bridge.len(),
        enabled_bridges
    );
    if !file_config.bridge.is_empty() {
        for bridge_cfg in &file_config.bridge {
            let status = if bridge_cfg.enabled {
                "enabled"
            } else {
                "disabled"
            };
            info!(
                "    - {} -> {} ({}) [{}]",
                bridge_cfg.name, bridge_cfg.address, bridge_cfg.protocol, status
            );
            // Show forward rules
            for rule in &bridge_cfg.forwards {
                let direction = match rule.direction {
                    vibemq::bridge::ForwardDirection::Out => "->",
                    vibemq::bridge::ForwardDirection::In => "<-",
                    vibemq::bridge::ForwardDirection::Both => "<->",
                };
                info!(
                    "      {} {} {} (qos={}, retain={})",
                    rule.local_topic, direction, rule.remote_topic, rule.qos, rule.retain
                );
            }
        }
        let bridge_manager = broker.create_bridge_manager(file_config.bridge);
        broker.set_bridge_manager(bridge_manager);
    }

    // Setup clustering if configured
    let enabled_clusters = file_config.cluster.iter().filter(|c| c.enabled).count();
    if enabled_clusters > 0 {
        let cluster_cfg = &file_config.cluster[0]; // Only first cluster config is used
        info!(
            "  Cluster: enabled (gossip={}, peer={})",
            cluster_cfg.gossip_addr, cluster_cfg.peer_addr
        );
        if !cluster_cfg.seeds.is_empty() {
            info!("    Seeds: {}", cluster_cfg.seeds.join(", "));
        }

        match broker.create_cluster_manager(cluster_cfg.clone()).await {
            Ok(cluster_manager) => {
                broker.set_cluster_manager(cluster_manager);
            }
            Err(e) => {
                eprintln!("Error initializing cluster: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        info!("  Cluster: disabled");
    }

    // Setup metrics if configured
    if file_config.metrics.enabled {
        let metrics = Arc::new(vibemq::Metrics::new());
        broker.set_metrics(metrics.clone());
        info!("  Metrics: enabled (http://{})", file_config.metrics.bind);

        // Spawn metrics server
        let metrics_server = vibemq::MetricsServer::new(metrics, file_config.metrics.bind);
        tokio::spawn(async move {
            if let Err(e) = metrics_server.run().await {
                tracing::error!("Metrics server error: {}", e);
            }
        });
    } else {
        info!("  Metrics: disabled");
    }

    // Start profiling server if feature is enabled
    #[cfg(feature = "pprof")]
    {
        let pprof_addr: std::net::SocketAddr = vibemq::profiling::DEFAULT_BIND.parse().unwrap();
        info!("  Profiling: enabled (http://{})", pprof_addr);
        tokio::spawn(async move {
            if let Err(e) = vibemq::profiling::start_server(pprof_addr).await {
                tracing::error!("Profiling server error: {}", e);
            }
        });
    }

    // Run the broker (it handles Ctrl+C internally via the shutdown signal)
    broker.run().await?;

    Ok(())
}
