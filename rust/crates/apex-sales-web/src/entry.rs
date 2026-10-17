//! Process entry points shared by the binaries and their tests: the startup
//! reporting line and the listener wiring. The `apex-sales-web` binary's
//! `main` stays a thin call into this module, so the boot logic is unit
//! tested here instead of being untestable inside a binary.

use std::future::Future;
use std::sync::Arc;

use crate::routes::router;
use crate::state::WebState;

/// The exact startup line for the state the process booted in. The
/// not-configured state explains the requirement and keeps the fail-closed
/// contract visible in the logs; the configured state names the state
/// directory it serves.
pub fn startup_line(state: &WebState) -> String {
    match &state.unconfigured_reason {
        Some(reason) => format!(
            "apex-sales-web: UNCONFIGURED: {reason}; every page explains the requirement and no session can be minted"
        ),
        None => format!(
            "apex-sales-web: owner gate configured; state directory {}",
            state.config.sales_dir.display()
        ),
    }
}

/// Serve `state` on an already-bound listener until `shutdown` resolves.
///
/// The periodic textfile exporter is spawned when `APEX_METRICS_TEXTFILE` is
/// set, and the final export runs after the graceful shutdown, exactly once
/// per process.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: WebState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), std::io::Error> {
    let registry = Arc::clone(&state.metrics);
    crate::telemetry::spawn_textfile_exporter(Arc::clone(&registry));
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await?;
    crate::telemetry::export_textfile(&registry);
    Ok(())
}

/// Resolve when the process is asked to stop (Ctrl-C, or SIGTERM on Unix),
/// so the final metrics textfile export runs before the process exits.
pub async fn shutdown_signal() {
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

#[cfg(test)]
mod tests {
    use apex_sales::cli::Env;

    use super::*;
    use crate::config::WebConfig;

    fn state(configured: bool, sales_dir: &str) -> WebState {
        let mut env = Env::new();
        env.insert("APEX_SALES_DIR".into(), sales_dir.into());
        if configured {
            env.insert("APEX_SALES_WEB_TOKEN".into(), "owner-token".into());
        }
        WebState::new(WebConfig::from_env(&env).expect("config"))
    }

    #[test]
    fn startup_lines_name_the_state_and_keep_the_fail_closed_contract() {
        let unconfigured = state(false, "/tmp/apex-sales-web-entry");
        let line = startup_line(&unconfigured);
        assert!(line.contains("UNCONFIGURED"), "{line}");
        assert!(line.contains("APEX_SALES_WEB_TOKEN is not set"), "{line}");
        assert!(line.contains("no session can be minted"), "{line}");

        let configured = state(true, "/tmp/apex-sales-web-entry");
        let line = startup_line(&configured);
        assert!(line.contains("owner gate configured"), "{line}");
        assert!(line.contains("/tmp/apex-sales-web-entry"), "{line}");
        assert!(!line.contains("UNCONFIGURED"), "{line}");
    }

    #[tokio::test]
    async fn serve_answers_requests_until_the_shutdown_future_resolves() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ephemeral listener");
        let addr = listener.local_addr().expect("bound address");
        let state = state(false, "/tmp/apex-sales-web-entry");
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve(listener, state, async move {
            let _ = shutdown_rx.await;
        }));

        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("write request");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("read response");
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("ok"), "{response}");

        shutdown_tx.send(()).expect("request shutdown");
        server.await.expect("server task").expect("graceful serve");
    }
}
