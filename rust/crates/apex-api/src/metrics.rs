//! Request metrics: the hand-rolled counters and the bounded registry series.
//!
//! The hand-rolled counters are process lifetime totals per status class; they
//! feed the `apex_api_requests_total` series and the error-ratio alert rule in
//! `monitoring/alerts.yml`, and the design keeps them during the metrics
//! migration ("Keep existing counters during migration", page 111). They are
//! unchanged.
//!
//! Alongside them, [`track_requests`] records the bounded `apex-metrics`
//! registry series declared in the shared catalog:
//!
//! - `apex_http_requests_total{service="apex-api",route_template,method,status_class}`,
//! - `apex_http_request_duration_seconds{service="apex-api",route_template,method}`,
//! - `apex_http_requests_in_flight{service="apex-api"}`.
//!
//! The route label is the matched **route template** from
//! [`axum::extract::MatchedPath`], never the raw request path, so object
//! identities in a URL can never become label values; the registry refuses
//! anything that is not a template shape in any case. A request whose route
//! cannot be identified (an unmatched path or an exotic method) is still
//! counted by the hand-rolled totals; the registry series only exist for
//! requests that can be attributed to a declared route template. The
//! in-flight gauge uses the registry's drop guard, so cancellation and error
//! unwinding cannot leak it.
//!
//! `GET /metrics` serves the hand-rolled text plus the registry render as one
//! Prometheus document on the API's own bind, for the operator scrape on the
//! loopback/control boundary. The public edge
//! (`infrastructure/caddy/Caddyfile`) refuses the `/metrics` path, so process
//! internals never reach the public API host. When `APEX_METRICS_TEXTFILE` is
//! set, [`spawn_textfile_exporter`] exports the registry render atomically
//! (temp file plus rename, [`apex_metrics::textfile`]) every 15 seconds
//! through a background task, and [`export_textfile`] writes the final file
//! once more on graceful shutdown. The periodic tick is deliberate: a
//! per-request write would couple request latency to disk I/O and expose a
//! partial file to a scrape, while the atomic writer plus a 15-second maximum
//! staleness keeps the node_exporter textfile collector honest at a bounded
//! cost.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

/// The emitting service label (`apex_metrics::vocab::Service::Api`).
const SERVICE: &str = "apex-api";

/// The interval between atomic textfile exports.
pub const TEXTFILE_INTERVAL_SECONDS: u64 = 15;

/// Process lifetime request counters plus the in-flight gauge.
#[derive(Debug)]
pub struct Metrics {
    started: Instant,
    ok: AtomicU64,
    client_error: AtomicU64,
    server_error: AtomicU64,
    in_flight: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            ok: AtomicU64::new(0),
            client_error: AtomicU64::new(0),
            server_error: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
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
        let in_flight = self.in_flight.load(Ordering::Relaxed);
        let uptime = self.started.elapsed().as_secs();
        let version = env!("CARGO_PKG_VERSION");
        format!(
            "# HELP apex_build_info Build info as labels; value is always 1.\n\
             # TYPE apex_build_info gauge\n\
             apex_build_info{{version=\"{version}\"}} 1\n\
             # HELP apex_api_requests_total Total HTTP requests by status class.\n\
             # TYPE apex_api_requests_total counter\n\
             apex_api_requests_total{{status=\"2xx\"}} {ok}\n\
             apex_api_requests_total{{status=\"4xx\"}} {client_error}\n\
             apex_api_requests_total{{status=\"5xx\"}} {server_error}\n\
             # HELP apex_api_requests_in_flight Requests currently being served.\n\
             # TYPE apex_api_requests_in_flight gauge\n\
             apex_api_requests_in_flight {in_flight}\n\
             # HELP apex_api_uptime_seconds Seconds since process start.\n\
             # TYPE apex_api_uptime_seconds gauge\n\
             apex_api_uptime_seconds {uptime}\n"
        )
    }
}

/// Middleware: count every response as it leaves the router, in the
/// hand-rolled totals and in the bounded registry series.
pub async fn track_requests(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    state.metrics.in_flight.fetch_add(1, Ordering::Relaxed);
    let started = Instant::now();
    // The router inserts `MatchedPath` before the route's layers run, so a
    // matched request always carries its template here and the raw path is
    // never eligible to become a label.
    let route_template = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    let method = apex_metrics::vocab::Method::parse(request.method().as_str());
    // The guard decrements the gauge on drop, which includes cancelled and
    // error-unwound requests.
    let in_flight_guard = match state.metrics_registry.in_flight(
        &apex_metrics::catalog::HTTP_REQUESTS_IN_FLIGHT,
        &[("service", SERVICE)],
    ) {
        Ok(guard) => Some(guard),
        Err(error) => {
            tracing::debug!(%error, "in-flight gauge write refused");
            None
        }
    };
    let response = next.run(request).await;
    let elapsed_seconds = started.elapsed().as_secs_f64();
    drop(in_flight_guard);
    state.metrics.in_flight.fetch_sub(1, Ordering::Relaxed);
    state.metrics.observe(response.status());
    if let (Some(route_template), Some(method)) = (route_template, method) {
        observe_request(
            &state.metrics_registry,
            &route_template,
            method,
            response.status(),
        );
        observe_duration(
            &state.metrics_registry,
            &route_template,
            method,
            elapsed_seconds,
        );
    }
    response
}

/// Record one finished request in the registry: the counter with its status
/// class.
fn observe_request(
    registry: &apex_metrics::Registry,
    route_template: &str,
    method: apex_metrics::vocab::Method,
    status: StatusCode,
) {
    let status_class = apex_metrics::vocab::StatusClass::of(status.as_u16());
    let labels = [
        ("service", SERVICE),
        ("route_template", route_template),
        ("method", method.as_str()),
        ("status_class", status_class.as_str()),
    ];
    if let Err(error) = registry.increment(&apex_metrics::catalog::HTTP_REQUESTS_TOTAL, &labels) {
        tracing::debug!(%error, route_template, "request counter write refused");
    }
}

/// Record one finished request's duration in the registry.
fn observe_duration(
    registry: &apex_metrics::Registry,
    route_template: &str,
    method: apex_metrics::vocab::Method,
    elapsed_seconds: f64,
) {
    let labels = [
        ("service", SERVICE),
        ("route_template", route_template),
        ("method", method.as_str()),
    ];
    if let Err(error) = registry.observe(
        &apex_metrics::catalog::HTTP_REQUEST_DURATION_SECONDS,
        &labels,
        elapsed_seconds,
    ) {
        tracing::debug!(%error, route_template, "request duration write refused");
    }
}

/// `GET /metrics` in Prometheus text format: the hand-rolled counters plus
/// the registry render, as one document.
pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let mut body = state.metrics.render();
    body.push_str(&state.metrics_registry.render_prometheus());
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

/// Export the registry to `APEX_METRICS_TEXTFILE` once. A no-op when the
/// variable is unset or blank.
pub fn export_textfile(registry: &apex_metrics::Registry) {
    match apex_metrics::export_from_env(|name| std::env::var(name).ok(), registry) {
        Ok(Some(path)) => {
            tracing::debug!(path = %path.display(), "metrics textfile exported");
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, "metrics textfile export failed");
        }
    }
}

/// Whether `APEX_METRICS_TEXTFILE` names a non-blank export path.
pub fn textfile_configured() -> bool {
    std::env::var(apex_metrics::TEXTFILE_ENV).is_ok_and(|value| !value.trim().is_empty())
}

/// Spawn the periodic textfile exporter: with `APEX_METRICS_TEXTFILE` set,
/// the registry render is written atomically every
/// [`TEXTFILE_INTERVAL_SECONDS`] seconds (the first write happens
/// immediately). Returns whether the task was spawned; the task never exits,
/// and the binary calls [`export_textfile`] once more after its graceful
/// shutdown.
pub fn spawn_textfile_exporter(registry: Arc<apex_metrics::Registry>) -> bool {
    if !textfile_configured() {
        return false;
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(TEXTFILE_INTERVAL_SECONDS));
        loop {
            interval.tick().await;
            export_textfile(&registry);
        }
    });
    true
}
