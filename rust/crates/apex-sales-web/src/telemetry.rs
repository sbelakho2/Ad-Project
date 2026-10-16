//! The sales origin's bounded metrics: the shared `apex-metrics` registry
//! with this service's own bounded `service` vocabulary.
//!
//! The shared catalog's `service` vocabulary names the other origins
//! (`apex-api`, `apex-control`, `apex-runner`), so this crate declares the
//! same `apex_http_*` metric names with one additional bounded value,
//! `service="apex-sales-web"`, reusing everything shared: the registry's
//! validation itself, the renderer, the atomic textfile writer, the
//! route-template and method/status vocabularies and the catalog's latency
//! buckets. A future unification with the catalog must merge this
//! declaration rather than grow the shared vocabulary ad hoc.
//!
//! [`track_requests`] records every routed request using the matched route
//! template, never the raw path. With `APEX_METRICS_TEXTFILE` set,
//! [`spawn_textfile_exporter`] writes the registry render atomically every
//! 15 seconds and the binary writes the final file on graceful shutdown.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

pub use apex_metrics::catalog::HTTP_LATENCY_BUCKETS;
use apex_metrics::spec::{LabelSpec, MetricKind, MetricSpec, Vocab};
use apex_metrics::vocab::{Method, StatusClass};
use apex_metrics::Registry;

use crate::state::WebState;

/// The emitting service label.
pub const SERVICE: &str = "apex-sales-web";

/// The interval between atomic textfile exports.
pub const TEXTFILE_INTERVAL_SECONDS: u64 = 15;

/// The bounded `service` vocabulary of this origin.
const SERVICE_VALUES: &[&str] = &[SERVICE];

/// Total HTTP requests by status class (same shape as the catalog series).
pub static HTTP_REQUESTS_TOTAL: MetricSpec = MetricSpec {
    name: "apex_http_requests_total",
    help: "Total HTTP requests by status class.",
    kind: MetricKind::Counter,
    labels: &[
        LabelSpec {
            name: "service",
            vocab: Vocab::Values(SERVICE_VALUES),
        },
        LabelSpec {
            name: "route_template",
            vocab: Vocab::RouteTemplate,
        },
        LabelSpec {
            name: "method",
            vocab: Vocab::Values(Method::VALUES),
        },
        LabelSpec {
            name: "status_class",
            vocab: Vocab::Values(StatusClass::VALUES),
        },
    ],
};

/// Server response duration in seconds (same shape as the catalog series).
pub static HTTP_REQUEST_DURATION_SECONDS: MetricSpec = MetricSpec {
    name: "apex_http_request_duration_seconds",
    help: "HTTP response duration in seconds by route template.",
    kind: MetricKind::Histogram {
        buckets: HTTP_LATENCY_BUCKETS,
    },
    labels: &[
        LabelSpec {
            name: "service",
            vocab: Vocab::Values(SERVICE_VALUES),
        },
        LabelSpec {
            name: "route_template",
            vocab: Vocab::RouteTemplate,
        },
        LabelSpec {
            name: "method",
            vocab: Vocab::Values(Method::VALUES),
        },
    ],
};

/// Requests in progress (same shape as the catalog series).
pub static HTTP_REQUESTS_IN_FLIGHT: MetricSpec = MetricSpec {
    name: "apex_http_requests_in_flight",
    help: "HTTP requests currently being served.",
    kind: MetricKind::Gauge,
    labels: &[LabelSpec {
        name: "service",
        vocab: Vocab::Values(SERVICE_VALUES),
    }],
};

/// Record one finished request: the counter with its status class and the
/// duration histogram.
fn observe_request(
    registry: &Registry,
    route_template: &str,
    method: Method,
    status: StatusCode,
    elapsed_seconds: f64,
) {
    let status_class = StatusClass::of(status.as_u16());
    let counter_labels = [
        ("service", SERVICE),
        ("route_template", route_template),
        ("method", method.as_str()),
        ("status_class", status_class.as_str()),
    ];
    if let Err(error) = registry.increment(&HTTP_REQUESTS_TOTAL, &counter_labels) {
        tracing::debug!(%error, route_template, "request counter write refused");
    }
    let duration_labels = [
        ("service", SERVICE),
        ("route_template", route_template),
        ("method", method.as_str()),
    ];
    if let Err(error) = registry.observe(
        &HTTP_REQUEST_DURATION_SECONDS,
        &duration_labels,
        elapsed_seconds,
    ) {
        tracing::debug!(%error, route_template, "request duration write refused");
    }
}

/// Middleware: record every routed request in the registry. The route label
/// is the matched route template, never the raw path; the in-flight gauge
/// uses the registry's drop guard, so cancellation and error unwinding
/// cannot leak it. A request that cannot be attributed to a template is
/// simply not recorded per route.
pub async fn track_requests(
    State(state): State<WebState>,
    request: Request,
    next: Next,
) -> Response {
    let route_template = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    let method = Method::parse(request.method().as_str());
    let started = Instant::now();
    let in_flight_guard = match state
        .metrics
        .in_flight(&HTTP_REQUESTS_IN_FLIGHT, &[("service", SERVICE)])
    {
        Ok(guard) => Some(guard),
        Err(error) => {
            tracing::debug!(%error, "in-flight gauge write refused");
            None
        }
    };
    let response = next.run(request).await;
    let elapsed_seconds = started.elapsed().as_secs_f64();
    drop(in_flight_guard);
    if let (Some(route_template), Some(method)) = (route_template, method) {
        observe_request(
            &state.metrics,
            &route_template,
            method,
            response.status(),
            elapsed_seconds,
        );
    }
    response
}

/// Export the registry to `APEX_METRICS_TEXTFILE` once. A no-op when the
/// variable is unset or blank.
pub fn export_textfile(registry: &Registry) {
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

/// Spawn the periodic textfile exporter. Returns whether the task was
/// spawned; the binary calls [`export_textfile`] once more after graceful
/// shutdown.
pub fn spawn_textfile_exporter(registry: Arc<Registry>) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_records_map_every_status_family() {
        let registry = Registry::new();
        for (status, class) in [
            (StatusCode::OK, "2xx"),
            (StatusCode::SEE_OTHER, "3xx"),
            (StatusCode::NOT_FOUND, "4xx"),
            (StatusCode::INTERNAL_SERVER_ERROR, "5xx"),
        ] {
            observe_request(&registry, "/leads", Method::Get, status, 0.0);
            let labels = [
                ("service", SERVICE),
                ("route_template", "/leads"),
                ("method", "GET"),
                ("status_class", class),
            ];
            assert_eq!(
                registry
                    .counter_value(&HTTP_REQUESTS_TOTAL, &labels)
                    .unwrap(),
                1,
                "{status}"
            );
        }
        let family = registry
            .family_snapshot(HTTP_REQUEST_DURATION_SECONDS.name)
            .expect("duration family");
        assert_eq!(family.series.len(), 1, "one series for the one route");
    }

    #[test]
    fn the_service_value_is_bounded_to_this_origin() {
        let registry = Registry::new();
        let good = [
            ("service", SERVICE),
            ("route_template", "/publishers/{domain}"),
            ("method", "GET"),
            ("status_class", "2xx"),
        ];
        assert!(registry.increment(&HTTP_REQUESTS_TOTAL, &good).is_ok());
        let foreign = [
            ("service", "apex-api"),
            ("route_template", "/publishers/{domain}"),
            ("method", "GET"),
            ("status_class", "2xx"),
        ];
        assert!(registry.increment(&HTTP_REQUESTS_TOTAL, &foreign).is_err());
    }

    #[test]
    fn textfile_export_is_atomic_and_parseable() {
        let dir =
            std::env::temp_dir().join(format!("apex-sales-web-metrics-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("apex-sales-web.prom");
        let registry = Registry::new();
        observe_request(&registry, "/", Method::Get, StatusCode::OK, 0.1);
        apex_metrics::textfile::write_registry(&path, &registry).expect("export");
        let contents = std::fs::read_to_string(&path).expect("read");
        assert!(contents.ends_with('\n'), "{contents}");
        assert!(contents.contains(
            "apex_http_requests_total{service=\"apex-sales-web\",route_template=\"/\",method=\"GET\",status_class=\"2xx\"} 1"
        ));
        for line in contents.lines().filter(|line| !line.starts_with('#')) {
            let (series, value) = line.rsplit_once(' ').unwrap_or_else(|| panic!("{line}"));
            assert!(series.starts_with("apex_"), "{line}");
            assert!(value.parse::<f64>().is_ok(), "{line}");
        }
        assert!(!dir.join("apex-sales-web.prom.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
