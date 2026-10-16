//! `apex-sales-web`: serve the owner-only sales workspace.
//!
//! Configuration is the `APEX_SALES_*` namespace plus the web-only
//! `APEX_SALES_WEB_BIND` / `APEX_SALES_WEB_TOKEN` (see
//! [`apex_sales_web::config`]). With `APEX_SALES_WEB_TOKEN` unset the
//! service starts in the not-configured state: every page explains the
//! requirement and no session can be minted (fail closed).
//!
//! Metrics: there is no public `/metrics` route. When
//! `APEX_METRICS_TEXTFILE` names a path, a background task exports the
//! bounded `apex-metrics` registry (route-template request series) to that
//! path atomically every 15 seconds, and the graceful shutdown handler
//! writes the final file before the process exits.

#![forbid(unsafe_code)]

use std::sync::Arc;

use apex_sales_web::config::WebConfig;
use apex_sales_web::routes::router;
use apex_sales_web::state::WebState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Operational log lines (tracing warn/error) must be visible in
    // production: initialize the stderr subscriber once, tolerating an
    // already-installed global subscriber.
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();

    let config = WebConfig::capture().map_err(std::io::Error::other)?;
    let state = WebState::new(config);
    match &state.unconfigured_reason {
        Some(reason) => {
            eprintln!(
                "apex-sales-web: UNCONFIGURED: {reason}; every page explains the requirement and no session can be minted"
            );
        }
        None => {
            eprintln!(
                "apex-sales-web: owner gate configured; state directory {}",
                state.config.sales_dir.display()
            );
        }
    }
    let metrics_registry = Arc::clone(&state.metrics);
    apex_sales_web::telemetry::spawn_textfile_exporter(Arc::clone(&metrics_registry));
    let listener = tokio::net::TcpListener::bind(state.config.bind).await?;
    eprintln!(
        "apex-sales-web: listening on http://{} (session cookie apex_sales_session)",
        state.config.bind
    );
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    apex_sales_web::telemetry::export_textfile(&metrics_registry);
    Ok(())
}

/// Resolve when the process is asked to stop (Ctrl-C, or SIGTERM on Unix),
/// so the final metrics textfile export runs before the process exits.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
