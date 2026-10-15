//! apex-metrics — bounded telemetry for the Apex product.
//!
//! The authority is section 06 of the design document (pages 111 and 112):
//! "New metrics use units and bounded dimensions. Financial and per-customer
//! facts remain in the analytics store." and "Do not generate a new metric
//! name or label value for every customer, experiment, URL, email or error
//! message."
//!
//! The crate is dependency-free on purpose: every binary in the workspace
//! can hold a [`Registry`] without a runtime.
//!
//! - [`catalog`] declares every metric the design names, each with declared
//!   histogram buckets and declared label vocabularies. An in-repo scheduler
//!   does not exist (systemd timers call `apex-runner`), so the overdue-jobs
//!   gauge `apex_jobs_overdue` is declared for the scheduler workstream and
//!   documented rather than written.
//! - [`spec::LabelValue`] syntactically refuses id-looking values (emails,
//!   URLs, query strings, `exp_0242`-style ids, `INV-2026-104`-style
//!   references), and [`Registry`] refuses any value outside the metric's
//!   declared vocabulary. Publisher ids, experiment ids, invoice ids and PSP
//!   references therefore cannot become labels.
//! - [`Registry::in_flight`] returns an [`InFlightGuard`] that decrements the
//!   gauge on drop, so cancellation and error unwinding cannot leak the
//!   in-flight gauge.
//! - [`render`] renders the Prometheus text format; [`textfile`] writes it
//!   atomically (temp file plus rename) for node_exporter's textfile
//!   collector under `APEX_METRICS_TEXTFILE`.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod catalog;
pub mod registry;
pub mod render;
pub mod spec;
pub mod textfile;
pub mod vocab;

pub use registry::{
    FamilySnapshot, HistogramSnapshot, InFlightGuard, Registry, SeriesSnapshot, Snapshot,
    WriteError,
};
pub use render::{
    escape_help, escape_label_value, format_float, render_metric, render_snapshot,
    PROMETHEUS_CONTENT_TYPE,
};
pub use spec::{LabelError, LabelSpec, LabelValue, MetricKind, MetricSpec, Vocab};
pub use textfile::{export_from_env, write_atomic, write_registry, TextfileError, TEXTFILE_ENV};
