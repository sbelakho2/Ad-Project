//! Submission Service
//!
//! Authenticated client submission (port 587) for email clients like Thunderbird.

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "submission")]
#[command(about = "Submission - Authenticated email sending for clients")]
struct Cli {
    /// Listen address
    #[arg(short, long, default_value = "0.0.0.0:587")]
    listen: String,
    
    /// Hostname to announce
    #[arg(long, default_value = "mail.apexmediation.ee")]
    hostname: String,
    
    /// Mailstore gRPC address
    #[arg(long, default_value = "127.0.0.1:50051")]
    mailstore_addr: String,
    
    /// Outbound queue gRPC address
    #[arg(long, default_value = "127.0.0.1:50053")]
    outbound_addr: String,
    
    /// Require TLS
    #[arg(long, default_value = "true")]
    require_tls: bool,
    
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
    
    info!("Starting Submission Service on {}", cli.listen);
    info!("TLS required: {}", cli.require_tls);
    
    // Keep running
    tokio::signal::ctrl_c().await?;
    
    Ok(())
}
