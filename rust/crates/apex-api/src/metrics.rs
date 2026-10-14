//! Request counters and the Prometheus text endpoint.
//!
//! The counters are process lifetime totals per status class; they feed the
//! `apex_api_requests_total` series and the error-ratio alert rule in
//! `monitoring/alerts.yml`. No external dependency is needed for this shape.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::extract::Request;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// Process lifetime request counters.
#[derive(Debug)]
pub struct Metrics {
    started: Instant,
    ok: AtomicU64,
    client_error: AtomicU64,
    server_error: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            ok: AtomicU64::new(0),
            client_error: AtomicU64::new(0),
            server_error: AtomicU64::new(0),
        }
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one response by its status class.
    pub fn observe(&self, status: StatusCode) {
        let counter = if status.is_server_error() {
            &self.server_error
        } else if status.is_client_error() {
            &self.client_error
        } else {
            &self.ok
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Prometheus text exposition (version 0.0.4).
    pub fn render(&self) -> String {
        let ok = self.ok.load(Ordering::Relaxed);
        let client_error = self.client_error.load(Ordering::Relaxed);
        let server_error = self.server_error.load(Ordering::Relaxed);
        let uptime = self.started.elapsed().as_secs();
        format!(
            "# HELP apex_api_requests_total Total HTTP requests by status class.\n\
             # TYPE apex_api_requests_total counter\n\
             apex_api_requests_total{{status=\"2xx\"}} {ok}\n\
             apex_api_requests_total{{status=\"4xx\"}} {client_error}\n\
             apex_api_requests_total{{status=\"5xx\"}} {server_error}\n\
             # HELP apex_api_uptime_seconds Seconds since process start.\n\
             # TYPE apex_api_uptime_seconds gauge\n\
             apex_api_uptime_seconds {uptime}\n"
        )
    }
}

/// Middleware: count every response as it leaves the router.
pub async fn track_requests(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let response = next.run(request).await;
    state.metrics.observe(response.status());
    response
}

/// `GET /metrics` in Prometheus text format.
pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
}
