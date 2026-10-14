//! Outbound Queue Service
//!
//! Outbound email delivery through the Resend API plus the queue item model the
//! durable queue adapter stores. Operator test sends run via `--send-to`.

use anyhow::{bail, Result};
use clap::Parser;
use outbound_queue::queue::{QueueItem, QueueStatus};
use outbound_queue::resend::{ResendClient, SendEmailRequest};
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

    /// Send one message to this address and exit (operator delivery check)
    #[arg(long)]
    send_to: Option<String>,

    /// Subject for the `--send-to` message
    #[arg(long, default_value = "ApexMail delivery check")]
    send_subject: String,

    /// Body for the `--send-to` message
    #[arg(long, default_value = "ApexMail outbound delivery check.")]
    send_body: String,
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

    info!("Starting Outbound Queue Service");

    if let Some(to) = cli.send_to.clone() {
        let Some(api_key) = cli.resend_api_key.clone() else {
            bail!("--send-to requires RESEND_API_KEY (or the resend_api_key option)");
        };
        let client = ResendClient::new(api_key, cli.from_address.clone());
        let mut item = QueueItem::new(
            cli.from_address.clone(),
            vec![to.clone()],
            cli.send_subject.clone(),
        )
        .with_text(&cli.send_body);
        if item.status != QueueStatus::Pending {
            bail!("refusing to send a queue item in state {:?}", item.status);
        }
        item.status = QueueStatus::Sending;
        let id = client.send(SendEmailRequest::from(&item)).await?;
        item.status = QueueStatus::Sent;
        println!(
            "sent {} (queue item {}, created {})",
            id, item.id, item.created_at
        );
        return Ok(());
    }

    // Check for Resend API key
    if cli.resend_api_key.is_none() {
        tracing::warn!("RESEND_API_KEY not set - email sending will fail");
    }

    info!("Outbound Queue listening on {}", cli.listen);

    // Keep running
    tokio::signal::ctrl_c().await?;

    Ok(())
}
