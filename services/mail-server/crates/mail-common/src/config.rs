//! Configuration management for mail services.

use serde::Deserialize;
use std::path::PathBuf;

/// Main configuration structure.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    
    #[serde(default)]
    pub database: DatabaseConfig,
    
    #[serde(default)]
    pub storage: StorageConfig,
    
    #[serde(default)]
    pub tls: TlsConfig,
    
    #[serde(default)]
    pub rspamd: RspamdConfig,
    
    #[serde(default)]
    pub resend: ResendConfig,
    
    #[serde(default)]
    pub dkim: DkimConfig,
    
    #[serde(default)]
    pub grpc: GrpcConfig,
    
    #[serde(default)]
    pub limits: LimitsConfig,
}

/// Server identity configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Mail server hostname (e.g., mail.apexmediation.ee)
    #[serde(default = "default_hostname")]
    pub hostname: String,
    
    /// Primary domain (e.g., apexmediation.ee)
    #[serde(default = "default_domain")]
    pub domain: String,
    
    /// Additional domains to accept mail for
    #[serde(default)]
    pub additional_domains: Vec<String>,
    
    /// Bind address for services
    #[serde(default = "default_bind_addr")]
    pub bind_address: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            hostname: default_hostname(),
            domain: default_domain(),
            additional_domains: Vec::new(),
            bind_address: default_bind_addr(),
        }
    }
}

/// Database configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    /// PostgreSQL connection URL
    #[serde(default = "default_db_url")]
    pub url: String,
    
    /// Maximum connections in pool
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    
    /// Connection timeout in seconds
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: default_db_url(),
            max_connections: default_max_connections(),
            connect_timeout_secs: default_connect_timeout(),
        }
    }
}

/// Storage paths configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    /// Path for message blobs
    #[serde(default = "default_blob_path")]
    pub blob_path: PathBuf,
    
    /// Path for search index
    #[serde(default = "default_index_path")]
    pub index_path: PathBuf,
    
    /// Enable blob deduplication
    #[serde(default = "default_dedupe")]
    pub deduplicate: bool,
    
    /// Compress stored blobs
    #[serde(default = "default_compress")]
    pub compress: bool,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            blob_path: default_blob_path(),
            index_path: default_index_path(),
            deduplicate: default_dedupe(),
            compress: default_compress(),
        }
    }
}

/// TLS configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TlsConfig {
    /// Path to certificate chain
    pub cert_path: Option<PathBuf>,
    
    /// Path to private key
    pub key_path: Option<PathBuf>,
    
    /// Require TLS for all connections
    #[serde(default = "default_require_tls")]
    pub require_tls: bool,
    
    /// Minimum TLS version (1.2 or 1.3)
    #[serde(default = "default_min_tls_version")]
    pub min_version: String,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            cert_path: None,
            key_path: None,
            require_tls: default_require_tls(),
            min_version: default_min_tls_version(),
        }
    }
}

/// Rspamd integration configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RspamdConfig {
    /// Rspamd HTTP API URL
    #[serde(default = "default_rspamd_url")]
    pub url: String,
    
    /// Optional password
    #[serde(default)]
    pub password: Option<String>,
    
    /// Enable Rspamd integration
    #[serde(default = "default_rspamd_enabled")]
    pub enabled: bool,
    
    /// Timeout for Rspamd requests in seconds
    #[serde(default = "default_rspamd_timeout")]
    pub timeout_secs: u64,
    
    /// Score threshold for spam rejection
    #[serde(default = "default_spam_threshold")]
    pub spam_threshold: f32,
}

impl Default for RspamdConfig {
    fn default() -> Self {
        Self {
            url: default_rspamd_url(),
            password: None,
            enabled: default_rspamd_enabled(),
            timeout_secs: default_rspamd_timeout(),
            spam_threshold: default_spam_threshold(),
        }
    }
}

/// Resend API configuration (fallback sending).
#[derive(Debug, Clone, Deserialize)]
pub struct ResendConfig {
    /// Resend API key
    #[serde(default)]
    pub api_key: Option<String>,
    
    /// Enable Resend for outbound
    #[serde(default)]
    pub enabled: bool,
    
    /// Default from address
    #[serde(default = "default_from_address")]
    pub from_address: String,
}

impl Default for ResendConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            enabled: false,
            from_address: default_from_address(),
        }
    }
}

/// DKIM signing configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct DkimConfig {
    /// DKIM selector
    #[serde(default = "default_dkim_selector")]
    pub selector: String,
    
    /// Path to private key
    pub private_key_path: Option<PathBuf>,
    
    /// Enable DKIM signing
    #[serde(default)]
    pub enabled: bool,
}

impl Default for DkimConfig {
    fn default() -> Self {
        Self {
            selector: default_dkim_selector(),
            private_key_path: None,
            enabled: false,
        }
    }
}

/// gRPC internal API configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GrpcConfig {
    /// Mailstore core address
    #[serde(default = "default_mailstore_addr")]
    pub mailstore_addr: String,
    
    /// Spam gateway address
    #[serde(default = "default_spam_addr")]
    pub spam_gateway_addr: String,
    
    /// Outbound queue address
    #[serde(default = "default_outbound_addr")]
    pub outbound_queue_addr: String,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            mailstore_addr: default_mailstore_addr(),
            spam_gateway_addr: default_spam_addr(),
            outbound_queue_addr: default_outbound_addr(),
        }
    }
}

/// Rate limits and quotas.
#[derive(Debug, Clone, Deserialize)]
pub struct LimitsConfig {
    /// Max message size in bytes
    #[serde(default = "default_max_message_size")]
    pub max_message_size: usize,
    
    /// Max recipients per message
    #[serde(default = "default_max_recipients")]
    pub max_recipients: usize,
    
    /// Max messages per hour per user
    #[serde(default = "default_rate_limit")]
    pub messages_per_hour: u32,
    
    /// Default quota in bytes
    #[serde(default = "default_quota")]
    pub default_quota_bytes: u64,
    
    /// Max concurrent connections per IP
    #[serde(default = "default_max_connections_per_ip")]
    pub max_connections_per_ip: u32,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_message_size: default_max_message_size(),
            max_recipients: default_max_recipients(),
            messages_per_hour: default_rate_limit(),
            default_quota_bytes: default_quota(),
            max_connections_per_ip: default_max_connections_per_ip(),
        }
    }
}

// Default value functions
fn default_hostname() -> String { "mail.apexmediation.ee".to_string() }
fn default_domain() -> String { "apexmediation.ee".to_string() }
fn default_bind_addr() -> String { "0.0.0.0".to_string() }
fn default_db_url() -> String { "postgresql://mail:password@localhost/maildb".to_string() }
fn default_max_connections() -> u32 { 20 }
fn default_connect_timeout() -> u64 { 30 }
fn default_blob_path() -> PathBuf { PathBuf::from("/var/lib/apex-mail/blobs") }
fn default_index_path() -> PathBuf { PathBuf::from("/var/lib/apex-mail/index") }
fn default_dedupe() -> bool { true }
fn default_compress() -> bool { true }
fn default_require_tls() -> bool { true }
fn default_min_tls_version() -> String { "1.2".to_string() }
fn default_rspamd_url() -> String { "http://127.0.0.1:11333".to_string() }
fn default_rspamd_enabled() -> bool { false }
fn default_rspamd_timeout() -> u64 { 30 }
fn default_spam_threshold() -> f32 { 15.0 }
fn default_from_address() -> String { "noreply@apexmediation.ee".to_string() }
fn default_dkim_selector() -> String { "apex2026".to_string() }
fn default_mailstore_addr() -> String { "127.0.0.1:50051".to_string() }
fn default_spam_addr() -> String { "127.0.0.1:50052".to_string() }
fn default_outbound_addr() -> String { "127.0.0.1:50053".to_string() }
fn default_max_message_size() -> usize { 25 * 1024 * 1024 } // 25MB
fn default_max_recipients() -> usize { 100 }
fn default_rate_limit() -> u32 { 500 }
fn default_quota() -> u64 { 5 * 1024 * 1024 * 1024 } // 5GB
fn default_max_connections_per_ip() -> u32 { 50 }

impl Config {
    /// Load configuration from file and environment.
    pub fn load() -> anyhow::Result<Self> {
        // Load .env file if present
        dotenvy::dotenv().ok();
        
        let config_path = std::env::var("MAIL_CONFIG")
            .unwrap_or_else(|_| "/etc/apex-mail/config.toml".to_string());
        
        let settings = config::Config::builder()
            .add_source(config::File::with_name(&config_path).required(false))
            .add_source(config::Environment::with_prefix("MAIL").separator("__"))
            .build()?;
        
        Ok(settings.try_deserialize()?)
    }
    
    /// Load with custom config path.
    pub fn load_from(path: &str) -> anyhow::Result<Self> {
        let settings = config::Config::builder()
            .add_source(config::File::with_name(path))
            .add_source(config::Environment::with_prefix("MAIL").separator("__"))
            .build()?;
        
        Ok(settings.try_deserialize()?)
    }
}
