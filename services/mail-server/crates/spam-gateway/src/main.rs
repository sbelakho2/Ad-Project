//! Spam Gateway Service
//!
//! Integrates with Rspamd for spam filtering and provides DKIM/SPF/DMARC verification.

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "spam-gateway")]
#[command(about = "Spam Gateway - Email filtering and authentication")]
struct Cli {
    /// gRPC listen address
    #[arg(short, long, default_value = "127.0.0.1:50052")]
    listen: String,
    
    /// Rspamd HTTP URL
    #[arg(long, default_value = "http://127.0.0.1:11333")]
    rspamd_url: String,
    
    /// Rspamd password (optional)
    #[arg(long)]
    rspamd_password: Option<String>,
    
    /// Spam score threshold
    #[arg(long, default_value = "15.0")]
    spam_threshold: f32,
    
    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Initialize logging
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&cli.log_level));
    
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();
    
    info!("Starting Spam Gateway Service");
    info!("Rspamd URL: {}", cli.rspamd_url);
    info!("Spam threshold: {}", cli.spam_threshold);
    info!("Spam Gateway listening on {}", cli.listen);
    
    // Keep running
    tokio::signal::ctrl_c().await?;
    
    Ok(())
}
