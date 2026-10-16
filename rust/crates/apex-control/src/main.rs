//! `apex-control` binary: serve the operator control plane.
//!
//! Besides serving, the binary dispatches one clap-free operational
//! subcommand before any configuration or listener is touched:
//! `apex-control hash-password` reads one password line from stdin and
//! prints exactly one argon2id PHC string for the `APEX_CONTROL_OPERATORS`
//! file (see [`apex_control::cli`]); `apex-control --help` prints the
//! usage. No argument serves the control plane.
//!
//! Configuration is the `APEX_CONTROL_*` namespace documented on
//! [`apex_control::config`] plus `APEX_ENVIRONMENT` and the analytics store
//! pair `APEX_CLICKHOUSE_HTTP` / `APEX_CLICKHOUSE_DB`. The binary never
//! holds a customer credential: platform operations become durable operator
//! jobs for the autopilot workstream, and every read comes from the shared
//! analytics read models or the durable runner files.
//!
//! Startup follows the module rules of [`apex_control::state`]: an absent
//! operator file starts the service in the logged unconfigured state (the
//! sign-in page explains the requirement; no session can be minted and there
//! is no default password), while a present but invalid file refuses
//! startup.
//!
//! Metrics: the operator origin has no public `/metrics` route. When
//! `APEX_METRICS_TEXTFILE` names a path, a background task exports the
//! bounded `apex-metrics` registry (route-template request series and the
//! `apex_operator_actions_total` action outcomes) to that path atomically
//! every 15 seconds, and the graceful shutdown signal handler below writes
//! the final file before the process exits. The periodic write bounds both
//! the export cost and the file's staleness; per-request writes would couple
//! request handling to disk I/O.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::sync::Arc;

use apex_control::config::ControlConfig;
use apex_control::data::{
    AnalyticsSource, ControlData, DurableReader, LiveData, OperationsEvidence,
};
use apex_control::routes::router;
use apex_control::state::ControlState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(command) = std::env::args().nth(1) {
        match command.as_str() {
            "hash-password" => return hash_password(),
            "--help" | "-h" => {
                print_usage(&mut std::io::stdout())?;
                return Ok(());
            }
            other => {
                eprintln!("apex-control: unknown command '{other}'");
                print_usage(&mut std::io::stderr())?;
                std::process::exit(2);
            }
        }
    }
    // Operational log lines (tracing warn/error) must be visible in
    // production: initialize the stderr subscriber once, tolerating an
    // already-installed global subscriber.
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();

    let config = ControlConfig::from_env()?;
    let analytics =
        AnalyticsSource::build(config.clickhouse.as_ref()).map_err(|error| error.reason)?;
    let data = ControlData::Live(Box::new(LiveData {
        analytics,
        durable: DurableReader::new(config.ledger_root.clone(), config.invoice_register.clone()),
        operations: OperationsEvidence::load(&config.operations_path),
    }));
    let state = ControlState::new(config, data)?;
    match &state.unconfigured_reason {
        Some(reason) => {
            eprintln!(
                "apex-control: UNCONFIGURED: {reason}; /control/login explains the required configuration and no session can be minted"
            );
        }
        None => {
            eprintln!(
                "apex-control: {} operator(s) configured; environment {}",
                state.operators.len(),
                state.environment()
            );
        }
    }
    if state.emitter.is_none() {
        eprintln!(
            "apex-control: no analytics store configured (APEX_CLICKHOUSE_HTTP/APEX_CLICKHOUSE_DB); the action journal remains the durable record"
        );
    } else if let Some(emitter) = &state.emitter {
        // Re-deliver batches spooled during an earlier outage before new
        // actions append more.
        let _ = emitter.replay_spool().await;
    }
    // With `APEX_METRICS_TEXTFILE` set, a background task exports the
    // registry to that file every 15 seconds; the graceful shutdown below
    // writes the final file before the process exits.
    let metrics_registry = Arc::clone(&state.metrics);
    apex_control::telemetry::spawn_textfile_exporter(Arc::clone(&metrics_registry));
    let listener = tokio::net::TcpListener::bind(state.config.bind).await?;
    eprintln!(
        "apex-control: listening on http://{} (session cookie apex_operator_session, path /control)",
        state.config.bind
    );
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    apex_control::telemetry::export_textfile(&metrics_registry);
    Ok(())
}

/// One password line on stdin, one argon2id PHC string on stdout (and nothing
/// else); a blank password is a typed error with exit code 1. The password is
/// never echoed.
fn hash_password() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match apex_control::cli::hash_password_line(&mut stdin.lock(), &mut stdout.lock()) {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("apex-control: {error}");
            std::process::exit(1);
        }
    }
}

/// The clap-free usage text shared by `--help` and the unknown-command path.
fn print_usage(out: &mut dyn std::io::Write) -> std::io::Result<()> {
    writeln!(
        out,
        "usage: apex-control [hash-password]\n\n  (no arguments)   serve the operator control plane (configuration via APEX_CONTROL_*)\n  hash-password    read one password line from stdin, print an argon2id PHC string\n  --help, -h       print this usage"
    )
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
