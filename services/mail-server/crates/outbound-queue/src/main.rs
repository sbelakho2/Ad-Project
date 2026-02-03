//! Outbound Queue Service
//!
//! Handles outbound email delivery with retry logic and Resend API integration.

mod resend;
mod queue;

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "outbound-queue")]
#[command(about = "Outbound Queue - Email delivery service")]
struct Cli {
    /// gRPC listen address
    #[arg(short, long, default_value = "127.0.0.1:50053")]
    listen: String,
    
    /// Resend API key
    #[arg(long, env = "RESEND_API_KEY")]
    resend_api_key: Option<String>,
    
    /// Default from address
    #[arg(long, default_value = "noreply@apexmediation.ee")]
    from_address: String,
    
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
    
    info!("Starting Outbound Queue Service");
    
    // Check for Resend API key
    if cli.resend_api_key.is_none() {
        tracing::warn!("RESEND_API_KEY not set - email sending will fail");
    }
    
    // TODO: Start gRPC server and queue processor
    info!("Outbound Queue listening on {}", cli.listen);
    
    // Keep running
    tokio::signal::ctrl_c().await?;
    
    Ok(())
}
