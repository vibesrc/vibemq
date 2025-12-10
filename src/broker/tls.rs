//! TLS configuration and acceptor setup
//!
//! Handles loading certificates and keys from PEM files and creating
//! TLS acceptors for secure MQTT connections.

use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;

use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::server::WebPkiClientVerifier;
use tokio_rustls::rustls::{RootCertStore, ServerConfig};
use tokio_rustls::TlsAcceptor;

use super::TlsConfig;

/// Error type for TLS configuration
#[derive(Debug)]
pub enum TlsError {
    /// IO error reading files
    Io(std::io::Error),
    /// Certificate parsing error
    CertificateError(String),
    /// Private key error
    PrivateKeyError(String),
    /// TLS configuration error
    ConfigError(String),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TlsError::Io(e) => write!(f, "IO error: {}", e),
            TlsError::CertificateError(msg) => write!(f, "Certificate error: {}", msg),
            TlsError::PrivateKeyError(msg) => write!(f, "Private key error: {}", msg),
            TlsError::ConfigError(msg) => write!(f, "TLS config error: {}", msg),
        }
    }
}

impl std::error::Error for TlsError {}

impl From<std::io::Error> for TlsError {
    fn from(e: std::io::Error) -> Self {
        TlsError::Io(e)
    }
}

/// Load certificates from a PEM file
fn load_certs(path: &str) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_reader_iter(reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::CertificateError(format!("Failed to parse certificates: {}", e)))?;

    if certs.is_empty() {
        return Err(TlsError::CertificateError(format!(
            "No certificates found in {}",
            path
        )));
    }

    Ok(certs)
}

/// Load private key from a PEM file
fn load_private_key(path: &str) -> Result<PrivateKeyDer<'static>, TlsError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    PrivateKeyDer::from_pem_reader(reader)
        .map_err(|e| TlsError::PrivateKeyError(format!("Failed to parse private key: {}", e)))
}

/// Load CA certificates into a root store
fn load_ca_certs(path: &str) -> Result<RootCertStore, TlsError> {
    let mut root_store = RootCertStore::empty();
    let certs = load_certs(path)?;

    for cert in certs {
        root_store.add(cert).map_err(|e| {
            TlsError::CertificateError(format!("Failed to add CA certificate: {}", e))
        })?;
    }

    Ok(root_store)
}

/// Load TLS configuration and create a TlsAcceptor
pub fn load_tls_config(config: &TlsConfig) -> Result<TlsAcceptor, TlsError> {
    // Load server certificate chain
    let certs = load_certs(&config.cert_path)?;

    // Load private key
    let key = load_private_key(&config.key_path)?;

    // Build server config
    let server_config = if config.require_client_cert {
        // Client certificate authentication required
        let ca_path = config.ca_cert_path.as_ref().ok_or_else(|| {
            TlsError::ConfigError(
                "ca_cert_path is required when require_client_cert is true".to_string(),
            )
        })?;

        let root_store = load_ca_certs(ca_path)?;
        let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
            .build()
            .map_err(|e| {
                TlsError::ConfigError(format!("Failed to build client verifier: {}", e))
            })?;

        ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(certs, key)
            .map_err(|e| TlsError::ConfigError(format!("Failed to build TLS config: {}", e)))?
    } else if let Some(ca_path) = &config.ca_cert_path {
        // Client certificate authentication optional (verify if provided)
        let root_store = load_ca_certs(ca_path)?;
        let client_verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
            .allow_unauthenticated()
            .build()
            .map_err(|e| {
                TlsError::ConfigError(format!("Failed to build client verifier: {}", e))
            })?;

        ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(certs, key)
            .map_err(|e| TlsError::ConfigError(format!("Failed to build TLS config: {}", e)))?
    } else {
        // No client certificate verification
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| TlsError::ConfigError(format!("Failed to build TLS config: {}", e)))?
    };

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_error_display() {
        let err = TlsError::CertificateError("test error".to_string());
        assert!(err.to_string().contains("Certificate error"));

        let err = TlsError::PrivateKeyError("key error".to_string());
        assert!(err.to_string().contains("Private key error"));

        let err = TlsError::ConfigError("config error".to_string());
        assert!(err.to_string().contains("TLS config error"));
    }
}
