//! `apex-sales-web`: serve the owner-only sales workspace.
//!
//! Configuration is the `APEX_SALES_*` namespace plus the web-only
//! `APEX_SALES_WEB_BIND` / `APEX_SALES_WEB_TOKEN` (see
//! [`apex_sales_web::config`]). With `APEX_SALES_WEB_TOKEN` unset the
//! service starts in the not-configured state: every page explains the
//! requirement and no session can be minted (fail closed). The startup
//! reporting and the listener wiring live in [`apex_sales_web::entry`] so
//! they are unit-tested; this binary only wires the process to them.
//!
//! Metrics: there is no public `/metrics` route. When
//! `APEX_METRICS_TEXTFILE` names a path, a background task exports the
//! bounded `apex-metrics` registry (route-template request series) to that
//! path atomically every 15 seconds, and the graceful shutdown handler
//! writes the final file before the process exits.

#![forbid(unsafe_code)]

use apex_sales_web::config::WebConfig;
use apex_sales_web::entry;
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
    eprintln!("{}", entry::startup_line(&state));
    let listener = tokio::net::TcpListener::bind(state.config.bind).await?;
    eprintln!(
        "apex-sales-web: listening on http://{} (session cookie apex_sales_session)",
        state.config.bind
    );
    entry::serve(listener, state, entry::shutdown_signal()).await?;
    Ok(())
}
