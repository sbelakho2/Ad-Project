//! IMAP Edge Service
//!
//! IMAP4rev1/rev2 mailbox access for clients like Thunderbird (port 993).

use anyhow::Result;
use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "imap-edge")]
#[command(about = "IMAP Edge - Mailbox access for email clients")]
struct Cli {
    /// Listen address
    #[arg(short, long, default_value = "0.0.0.0:993")]
    listen: String,

    /// Hostname to announce
    #[arg(long, default_value = "mail.apexmediation.ee")]
    hostname: String,

    /// Mailstore gRPC address
    #[arg(long, default_value = "127.0.0.1:50051")]
    mailstore_addr: String,

    /// Log level
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();

    info!("Starting IMAP Edge Service on {}", cli.listen);

    // Keep running
    tokio::signal::ctrl_c().await?;

    Ok(())
}
