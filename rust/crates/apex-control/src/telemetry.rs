//! The operator origin's bounded metrics: the shared `apex-metrics` registry
//! bound to this service's vocabularies.
//!
//! The shared catalog (`apex-metrics`, the monitoring workstream) declares
//! `service` as `apex-api | apex-runner` and the operator-action `outcome` as
//! the generic job outcome set, so it cannot express this origin
//! (`service="apex-control"`) or the action outcomes the design names for the
//! control plane (page 64: Requested, Running, Confirmed, Failed, State
//! unknown; plus the gate outcome Denied). This crate's ownership boundary
//! forbids editing `apex-metrics`, so this module declares the same metric
//! names with this origin's bounded vocabularies while reusing everything
//! shared: the [`apex_metrics::Registry`] validation itself, the renderer,
//! the atomic textfile writer, the route-template and method/status
//! vocabularies and the catalog's latency buckets. Every value written here
//! is as bounded as a catalog write; a future unification with the catalog
//! must merge these declarations rather than grow them ad hoc.
//!
//! - [`HTTP_REQUESTS_TOTAL`], [`HTTP_REQUEST_DURATION_SECONDS`] and
//!   [`HTTP_REQUESTS_IN_FLIGHT`] mirror the `apex_http_*` series declared in
//!   the catalog, with `service="apex-control"`; [`track_requests`] records
//!   them for every routed control-plane request using the matched route
//!   template, never the raw path.
//! - [`OPERATOR_ACTIONS_TOTAL`] mirrors the catalog series; [`record_action`]
//!   is the action engine's single write path, with the bounded capability
//!   class, action class and outcome vocabularies below.
//!
//! With `APEX_METRICS_TEXTFILE` set, [`spawn_textfile_exporter`] writes the
//! registry render atomically every 15 seconds, and the binary writes the
//! final file once more on graceful shutdown. There is deliberately no
//! public `/metrics` route on the operator origin; the textfile is the
//! delivery path.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

pub use apex_metrics::catalog::{
    HTTP_REQUESTS_IN_FLIGHT, HTTP_REQUESTS_TOTAL, HTTP_REQUEST_DURATION_SECONDS,
    OPERATOR_ACTIONS_TOTAL,
};
use apex_metrics::vocab::{Method, StatusClass};
use apex_metrics::Registry;

use crate::capabilities::Capability;
use crate::state::ControlState;

/// The emitting service label.
pub const SERVICE: &str = "apex-control";

/// The interval between atomic textfile exports.
pub const TEXTFILE_INTERVAL_SECONDS: u64 = 15;

/// Record one submitted operator action. This is the only write path for
/// [`OPERATOR_ACTIONS_TOTAL`], so the bounded dimensions cannot be bypassed:
/// the registry validates every label value against the declared
/// vocabularies (`CapabilityClass`, `ActionClass`, `Outcome`).
pub fn record_action(registry: &Registry, capability: Capability, outcome: &'static str) {
    let labels = [
        ("capability_class", capability.capability_class()),
        ("action_class", capability.as_str()),
        ("outcome", outcome),
    ];
    if let Err(error) = registry.increment(&OPERATOR_ACTIONS_TOTAL, &labels) {
        // Declared values cannot be refused; log if a future change breaks
        // that instead of panicking in the action path.
        tracing::warn!(%error, "operator action metric refused");
    }
}

/// Record one finished request in the registry: the counter with its status
/// class and the duration histogram.
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

/// Middleware: record every control-plane response in the registry. The
/// route label is the matched route template from
/// [`axum::extract::MatchedPath`], never the raw path; the in-flight gauge
/// uses the registry's drop guard, so cancellation and error unwinding
/// cannot leak it. A request that cannot be attributed to a template (an
/// unmatched path) is simply not recorded per route.
pub async fn track_requests(
    State(state): State<ControlState>,
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

/// Spawn the periodic textfile exporter: with `APEX_METRICS_TEXTFILE` set,
/// the registry render is written atomically every
/// [`TEXTFILE_INTERVAL_SECONDS`] seconds (the first write happens
/// immediately). Returns whether the task was spawned; the task never exits,
/// and the binary calls [`export_textfile`] once more after its graceful
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn action_vocabularies_are_bounded_and_complete() {
        use apex_metrics::vocab::{ActionClass, CapabilityClass, Outcome};
        // The unified vocabulary is the union across operator surfaces, so it
        // must contain every capability this origin can exercise (superset,
        // not equality).
        assert!(ActionClass::VALUES.len() >= Capability::ALL.len());
        for capability in Capability::ALL {
            assert!(
                ActionClass::VALUES.contains(&capability.as_str()),
                "{} missing from the declared action classes",
                capability.as_str()
            );
            assert!(
                CapabilityClass::VALUES.contains(&capability.capability_class()),
                "{} class outside the declared set",
                capability.as_str()
            );
        }
        // The engine records exactly these outcomes (ActionState values plus
        // the gate outcomes).
        for outcome in [
            "requested",
            "confirmed",
            "failed",
            "denied",
            "step_up_required",
        ] {
            assert!(Outcome::VALUES.contains(&outcome), "{outcome}");
        }
        assert!(!Outcome::VALUES.contains(&"permitted"));
    }

    #[test]
    fn request_records_map_every_status_family() {
        let registry = Registry::new();
        for (status, class) in [
            (StatusCode::OK, "2xx"),
            (StatusCode::SEE_OTHER, "3xx"),
            (StatusCode::NOT_FOUND, "4xx"),
            (StatusCode::INTERNAL_SERVER_ERROR, "5xx"),
        ] {
            observe_request(&registry, "/control/", Method::Get, status, 0.0);
            let labels = [
                ("service", SERVICE),
                ("route_template", "/control/"),
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
        assert_eq!(
            family.series.len(),
            1,
            "one series for the one route/method"
        );
        let labels: Vec<(&str, &str)> = family.series[0]
            .labels
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        assert_eq!(
            labels,
            vec![
                ("service", "apex-control"),
                ("route_template", "/control/"),
                ("method", "GET"),
            ]
        );
        let histogram = family.series[0].histogram.as_ref().expect("histogram");
        assert_eq!(histogram.count, 4);
    }

    #[test]
    fn record_action_writes_exactly_the_bounded_dimensions() {
        let registry = Registry::new();
        record_action(&registry, Capability::IncidentAcknowledge, "confirmed");
        record_action(&registry, Capability::IncidentAcknowledge, "denied");
        let confirmed = [
            ("capability_class", "incident"),
            ("action_class", "incident_acknowledge"),
            ("outcome", "confirmed"),
        ];
        let denied = [
            ("capability_class", "incident"),
            ("action_class", "incident_acknowledge"),
            ("outcome", "denied"),
        ];
        assert_eq!(
            registry
                .counter_value(&OPERATOR_ACTIONS_TOTAL, &confirmed)
                .unwrap(),
            1
        );
        assert_eq!(
            registry
                .counter_value(&OPERATOR_ACTIONS_TOTAL, &denied)
                .unwrap(),
            1
        );
        // An outcome outside the declared set is refused by the registry,
        // not counted; the write path logs instead of panicking.
        record_action(&registry, Capability::IncidentAcknowledge, "permitted");
        assert!(registry
            .family_snapshot(OPERATOR_ACTIONS_TOTAL.name)
            .expect("family")
            .series
            .iter()
            .all(|series| series.labels.iter().all(|(_, value)| value != "permitted")));
    }

    #[test]
    fn textfile_export_is_atomic_and_parseable() {
        let dir =
            std::env::temp_dir().join(format!("apex-control-telemetry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("apex-control.prom");
        let registry = Registry::new();
        record_action(&registry, Capability::InvoiceHold, "confirmed");
        observe_request(&registry, "/control/", Method::Get, StatusCode::OK, 0.1);
        apex_metrics::textfile::write_registry(&path, &registry).expect("export");
        let contents = std::fs::read_to_string(&path).expect("read");
        assert!(contents.ends_with('\n'), "{contents}");
        assert!(contents.contains("# TYPE apex_operator_actions_total counter\n"));
        assert!(contents.contains(
            "apex_operator_actions_total{capability_class=\"finance\",action_class=\"invoice_hold\",outcome=\"confirmed\"} 1"
        ));
        assert!(contents.contains(
            "apex_http_requests_total{service=\"apex-control\",route_template=\"/control/\",method=\"GET\",status_class=\"2xx\"} 1"
        ));
        // Every non-comment line is a `series value` sample with a numeric
        // value, i.e. the file parses as Prometheus text.
        for line in contents.lines().filter(|line| !line.starts_with('#')) {
            let (series, value) = line.rsplit_once(' ').unwrap_or_else(|| panic!("{line}"));
            assert!(series.starts_with("apex_"), "{line}");
            assert!(value.parse::<f64>().is_ok(), "{line}");
        }
        // A second export replaces the file atomically and leaves no temp.
        apex_metrics::textfile::write_registry(&path, &registry).expect("re-export");
        assert!(!dir.join("apex-control.prom.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
