//! Mailstore Core Service
//!
//! Central service for message storage, mailbox management, and indexing.

use anyhow::Result;
use clap::Parser;
use mail_common::Config;
use mailstore_core::{db, service, storage};
use tonic::transport::Server;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "mailstore-core")]
#[command(about = "Mailstore Core Service - Message storage and mailbox management")]
struct Cli {
    /// Configuration file path
    #[arg(short, long, default_value = "/etc/apex-mail/config.toml")]
    config: String,

    /// gRPC listen address
    #[arg(short, long, default_value = "127.0.0.1:50051")]
    listen: String,

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
        .with_target(true)
        .json()
        .init();

    info!("Starting Mailstore Core Service");

    // Load configuration
    let config = Config::load_from(&cli.config).unwrap_or_else(|e| {
        tracing::warn!(
            "Failed to load config from {}: {}, using defaults",
            cli.config,
            e
        );
        Config::default()
    });

    // Initialize database
    let db = db::Database::connect(&config.database).await?;
    db.migrate().await?;

    // Initialize blob storage
    let storage = storage::BlobStorage::new(&config.storage)?;

    // Create gRPC service
    let mailstore_service = service::MailstoreServiceImpl::new(db, storage, config.clone());

    let addr = cli.listen.parse()?;
    info!("Mailstore Core listening on {}", addr);

    Server::builder()
        .add_service(
            mail_proto::generated::mailstore_service_server::MailstoreServiceServer::new(
                mailstore_service,
            ),
        )
        .serve(addr)
        .await?;

    Ok(())
}
