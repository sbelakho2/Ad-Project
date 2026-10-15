//! The bounded telemetry registry.
//!
//! [`Registry`] stores counters, gauges and histograms keyed by their declared
//! [`MetricSpec`] and validated label set. It is the only writer path: a value
//! that is not part of a declared vocabulary is refused (with a typed error),
//! so no producer can mint a per-customer series. Rendering, snapshots and the
//! atomic textfile export are in [`crate::render`] and [`crate::textfile`].

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::spec::{LabelValue, MetricKind, MetricSpec};

/// Why a write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// A label or label value failed the syntactic gate.
    Label(crate::spec::LabelError),
    /// A label name the metric does not declare was supplied.
    UnknownLabel {
        /// The metric.
        metric: &'static str,
        /// The undeclared label name.
        label: String,
    },
    /// A declared label was not supplied.
    MissingLabel {
        /// The metric.
        metric: &'static str,
        /// The missing label name.
        label: &'static str,
    },
    /// A label name was supplied twice.
    DuplicateLabel {
        /// The metric.
        metric: &'static str,
        /// The duplicated label name.
        label: String,
    },
    /// A label value is not in the metric's declared vocabulary.
    UndeclaredValue {
        /// The metric.
        metric: &'static str,
        /// The label name.
        label: &'static str,
        /// The refused value.
        value: String,
        /// What the label accepts.
        accepts: &'static str,
    },
    /// The write names a metric of a different kind.
    KindMismatch {
        /// The metric.
        metric: &'static str,
        /// What the operation needs.
        expected: &'static str,
    },
    /// Two different declarations with the same name reached one registry.
    ConflictingDeclaration {
        /// The metric name.
        metric: &'static str,
    },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Label(error) => write!(f, "{error}"),
            WriteError::UnknownLabel { metric, label } => {
                write!(f, "{metric} has no declared label '{label}'")
            }
            WriteError::MissingLabel { metric, label } => {
                write!(f, "{metric} requires label '{label}'")
            }
            WriteError::DuplicateLabel { metric, label } => {
                write!(f, "{metric} label '{label}' was supplied twice")
            }
            WriteError::UndeclaredValue {
                metric,
                label,
                value,
                accepts,
            } => write!(
                f,
                "{metric} label '{label}' value '{value}' is outside the {accepts}"
            ),
            WriteError::KindMismatch { metric, expected } => {
                write!(f, "{metric} is not a {expected}")
            }
            WriteError::ConflictingDeclaration { metric } => {
                write!(f, "two declarations named '{metric}' disagree")
            }
        }
    }
}

impl std::error::Error for WriteError {}

impl From<crate::spec::LabelError> for WriteError {
    fn from(error: crate::spec::LabelError) -> Self {
        WriteError::Label(error)
    }
}

/// One series' stored value.
#[derive(Debug, Clone, Default)]
struct Series {
    counter: u64,
    gauge: f64,
    observations: u64,
    sum: f64,
    /// Cumulative counts per declared bucket; length = buckets + 1 (the
    /// implicit `+Inf` bucket).
    buckets: Vec<u64>,
}

#[derive(Debug)]
struct Family {
    spec: &'static MetricSpec,
    series: BTreeMap<Vec<String>, Series>,
}

/// A snapshot of one series.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesSnapshot {
    /// Labels in declaration order.
    pub labels: Vec<(&'static str, String)>,
    /// The counter total, for counters.
    pub counter: Option<u64>,
    /// The gauge value, for gauges.
    pub gauge: Option<f64>,
    /// The histogram state, for histograms.
    pub histogram: Option<HistogramSnapshot>,
}

/// A snapshot of one histogram series.
#[derive(Debug, Clone, PartialEq)]
pub struct HistogramSnapshot {
    /// Declared upper bounds.
    pub buckets: Vec<f64>,
    /// Cumulative counts per bucket (including the implicit `+Inf` bucket).
    pub cumulative_counts: Vec<u64>,
    /// Total observations (the `+Inf` bucket).
    pub count: u64,
    /// Sum of all observations.
    pub sum: f64,
}

/// A snapshot of one metric family.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilySnapshot {
    /// The metric name.
    pub name: &'static str,
    /// The `# HELP` text.
    pub help: &'static str,
    /// Counter, gauge or histogram.
    pub kind: MetricKind,
    /// Series in canonical (label-sorted) order.
    pub series: Vec<SeriesSnapshot>,
}

/// Everything the registry holds, in canonical order.
pub type Snapshot = Vec<FamilySnapshot>;

/// The process-wide telemetry registry.
#[derive(Debug, Default)]
pub struct Registry {
    families: Mutex<BTreeMap<&'static str, Family>>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment a counter by one and return the new total.
    pub fn increment(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> Result<u64, WriteError> {
        self.increment_by(metric, labels, 1)
    }

    /// Increment a counter and return the new total.
    pub fn increment_by(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        delta: u64,
    ) -> Result<u64, WriteError> {
        let key = validate(metric, labels, "counter")?;
        let mut families = self.families();
        let family = declare(&mut families, metric)?;
        let series = family.series.entry(key).or_default();
        series.counter = series.counter.saturating_add(delta);
        Ok(series.counter)
    }

    /// Set a gauge to `value`.
    pub fn gauge_set(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        value: f64,
    ) -> Result<(), WriteError> {
        let key = validate(metric, labels, "gauge")?;
        let mut families = self.families();
        let family = declare(&mut families, metric)?;
        let series = family.series.entry(key).or_default();
        series.gauge = value;
        Ok(())
    }

    /// Add `delta` to a gauge.
    pub fn gauge_add(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        delta: f64,
    ) -> Result<(), WriteError> {
        let key = validate(metric, labels, "gauge")?;
        let mut families = self.families();
        let family = declare(&mut families, metric)?;
        let series = family.series.entry(key).or_default();
        series.gauge += delta;
        Ok(())
    }

    /// The current value of a gauge series, when it exists.
    pub fn gauge_value(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> Result<f64, WriteError> {
        let key = validate(metric, labels, "gauge")?;
        let families = self.families();
        Ok(families
            .get(metric.name)
            .and_then(|family| family.series.get(&key))
            .map(|series| series.gauge)
            .unwrap_or(0.0))
    }

    /// Observe one histogram value.
    pub fn observe(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
        value: f64,
    ) -> Result<(), WriteError> {
        let buckets = metric.buckets().ok_or(WriteError::KindMismatch {
            metric: metric.name,
            expected: "histogram",
        })?;
        let key = validate(metric, labels, "histogram")?;
        let mut families = self.families();
        let family = declare(&mut families, metric)?;
        let series = family.series.entry(key).or_default();
        if series.buckets.is_empty() {
            series.buckets = vec![0; buckets.len() + 1];
        }
        for (index, bound) in buckets.iter().enumerate() {
            if value <= *bound {
                series.buckets[index] += 1;
            }
        }
        series.buckets[buckets.len()] += 1;
        series.observations += 1;
        series.sum += value;
        Ok(())
    }

    /// An in-flight guard: increments the gauge now and decrements it on drop,
    /// so cancellation and error unwinding cannot leak the gauge.
    pub fn in_flight(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> Result<InFlightGuard<'_>, WriteError> {
        let key = validate(metric, labels, "gauge")?;
        {
            let mut families = self.families();
            let family = declare(&mut families, metric)?;
            let series = family.series.entry(key.clone()).or_default();
            series.gauge += 1.0;
        }
        Ok(InFlightGuard {
            registry: self,
            metric,
            labels: key,
        })
    }

    /// A snapshot of every family and series, in canonical order.
    pub fn snapshot(&self) -> Snapshot {
        self.families()
            .values()
            .map(|family| FamilySnapshot {
                name: family.spec.name,
                help: family.spec.help,
                kind: family.spec.kind,
                series: family
                    .series
                    .iter()
                    .map(|(key, series)| series_snapshot(family.spec, key, series))
                    .collect(),
            })
            .collect()
    }

    /// The snapshot of one family by name.
    pub fn family_snapshot(&self, name: &str) -> Option<FamilySnapshot> {
        self.snapshot()
            .into_iter()
            .find(|family| family.name == name)
    }

    /// The current counter total of one series, or zero when never written.
    pub fn counter_value(
        &self,
        metric: &'static MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> Result<u64, WriteError> {
        let key = validate(metric, labels, "counter")?;
        let families = self.families();
        Ok(families
            .get(metric.name)
            .and_then(|family| family.series.get(&key))
            .map(|series| series.counter)
            .unwrap_or(0))
    }

    /// Lock the family map, recovering a poisoned lock instead of panicking:
    /// a panicked peer must not take the whole process's telemetry down.
    fn families(&self) -> std::sync::MutexGuard<'_, BTreeMap<&'static str, Family>> {
        self.families
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Validate a label set against a metric declaration and return the canonical
/// series key (values in declaration order).
fn validate(
    metric: &'static MetricSpec,
    labels: &[(&'static str, &str)],
    expected: &'static str,
) -> Result<Vec<String>, WriteError> {
    match (metric.kind, expected) {
        (MetricKind::Counter, "counter") => {}
        (MetricKind::Gauge, "gauge") => {}
        (MetricKind::Histogram { .. }, "histogram") => {}
        _ => {
            return Err(WriteError::KindMismatch {
                metric: metric.name,
                expected,
            })
        }
    }
    let mut values: BTreeMap<&str, &str> = BTreeMap::new();
    for (name, value) in labels {
        let declared = metric.labels.iter().find(|label| label.name == *name);
        let Some(declared) = declared else {
            return Err(WriteError::UnknownLabel {
                metric: metric.name,
                label: (*name).to_string(),
            });
        };
        let value = LabelValue::new(name, value)?.value();
        if !declared.vocab.accepts(value) {
            return Err(WriteError::UndeclaredValue {
                metric: metric.name,
                label: name,
                value: value.to_string(),
                accepts: declared.vocab.kind_name(),
            });
        }
        if values.insert(name, value).is_some() {
            return Err(WriteError::DuplicateLabel {
                metric: metric.name,
                label: (*name).to_string(),
            });
        }
    }
    let mut key = Vec::with_capacity(metric.labels.len());
    for declared in metric.labels {
        match values.get(declared.name) {
            Some(value) => key.push((*value).to_string()),
            None => {
                return Err(WriteError::MissingLabel {
                    metric: metric.name,
                    label: declared.name,
                })
            }
        }
    }
    Ok(key)
}

/// Get or declare the family for `metric`; a conflicting re-declaration under
/// the same name is refused so two crates cannot silently merge series.
fn declare<'a>(
    families: &'a mut BTreeMap<&'static str, Family>,
    metric: &'static MetricSpec,
) -> Result<&'a mut Family, WriteError> {
    use std::collections::btree_map::Entry;
    match families.entry(metric.name) {
        Entry::Occupied(existing) => {
            if existing.get().spec != metric {
                return Err(WriteError::ConflictingDeclaration {
                    metric: metric.name,
                });
            }
            Ok(existing.into_mut())
        }
        Entry::Vacant(slot) => Ok(slot.insert(Family {
            spec: metric,
            series: BTreeMap::new(),
        })),
    }
}

fn series_snapshot(spec: &MetricSpec, key: &[String], series: &Series) -> SeriesSnapshot {
    let labels = spec
        .labels
        .iter()
        .zip(key.iter())
        .map(|(label, value)| (label.name, value.clone()))
        .collect();
    match spec.kind {
        MetricKind::Counter => SeriesSnapshot {
            labels,
            counter: Some(series.counter),
            gauge: None,
            histogram: None,
        },
        MetricKind::Gauge => SeriesSnapshot {
            labels,
            counter: None,
            gauge: Some(series.gauge),
            histogram: None,
        },
        MetricKind::Histogram { buckets } => SeriesSnapshot {
            labels,
            counter: None,
            gauge: None,
            histogram: Some(HistogramSnapshot {
                buckets: buckets.to_vec(),
                cumulative_counts: series.buckets.clone(),
                count: series.observations,
                sum: series.sum,
            }),
        },
    }
}

/// One in-flight operation; decrements its gauge on drop.
#[derive(Debug)]
pub struct InFlightGuard<'a> {
    registry: &'a Registry,
    metric: &'static MetricSpec,
    labels: Vec<String>,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        let mut families = self.registry.families();
        if let Ok(family) = declare(&mut families, self.metric) {
            if let Some(series) = family.series.get_mut(&self.labels) {
                series.gauge -= 1.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::catalog;
    use crate::vocab::{JobType, Outcome, Platform};

    fn job_labels<'a>(
        job: &'a str,
        platform: &'a str,
        outcome: &'a str,
    ) -> Vec<(&'static str, &'a str)> {
        vec![
            ("job_type", job),
            ("platform", platform),
            ("outcome", outcome),
        ]
    }

    #[test]
    fn counters_accumulate_per_series() {
        let registry = Registry::new();
        let metric = &catalog::JOB_RUNS_TOTAL;
        let labels = job_labels("daily", "applovin_max", "completed");
        assert_eq!(registry.increment(metric, &labels).unwrap(), 1);
        assert_eq!(registry.increment(metric, &labels).unwrap(), 2);
        assert_eq!(registry.increment_by(metric, &labels, 5).unwrap(), 7);
        let other = job_labels("monthly", "applovin_max", "completed");
        assert_eq!(registry.increment(metric, &other).unwrap(), 1);
        assert_eq!(registry.counter_value(metric, &labels).unwrap(), 7);
    }

    #[test]
    fn id_like_label_values_are_refused_everywhere() {
        let registry = Registry::new();
        let metric = &catalog::PAYMENT_EVENTS_TOTAL;
        // The metric has no publisher/experiment/invoice/PSP label at all.
        for name in [
            "publisher_id",
            "experiment_id",
            "invoice_id",
            "psp_reference",
        ] {
            let labels = vec![
                (name, "value"),
                ("event_class", "authorisation"),
                ("result", "accepted"),
            ];
            let error = registry.increment(metric, &labels).expect_err(name);
            assert!(matches!(error, WriteError::UnknownLabel { .. }), "{error}");
        }
        // An id-like value on a declared label is refused by the syntax gate.
        let labels = vec![
            ("provider", "adyen"),
            ("event_class", "authorisation"),
            ("result", "psp_123456"),
        ];
        let error = registry.increment(metric, &labels).expect_err("id value");
        assert!(
            matches!(
                error,
                WriteError::Label(crate::spec::LabelError::IdentifierLike { .. })
            ),
            "{error}"
        );
        // A value outside the declared vocabulary is refused.
        let labels = vec![
            ("provider", "northstar"),
            ("event_class", "authorisation"),
            ("result", "accepted"),
        ];
        let error = registry
            .increment(metric, &labels)
            .expect_err("publisher id");
        assert!(
            matches!(error, WriteError::UndeclaredValue { .. }),
            "{error}"
        );
        // Emails and URLs never pass the syntax gate.
        for bad in ["ops@example.com", "https://example.com/x"] {
            let error = LabelValue::new("x", bad).expect_err(bad);
            assert!(matches!(WriteError::from(error), WriteError::Label(_)));
        }
    }

    #[test]
    fn missing_duplicate_and_unknown_labels_are_refused() {
        let registry = Registry::new();
        let metric = &catalog::JOB_RUNS_TOTAL;
        let error = registry
            .increment(metric, &[("platform", "admob"), ("outcome", "completed")])
            .expect_err("missing job_type");
        assert!(matches!(
            error,
            WriteError::MissingLabel {
                label: "job_type",
                ..
            }
        ));
        let duplicate = vec![
            ("job_type", "daily"),
            ("job_type", "monthly"),
            ("platform", "admob"),
            ("outcome", "completed"),
        ];
        let error = registry
            .increment(metric, &duplicate)
            .expect_err("duplicate");
        assert!(
            matches!(error, WriteError::DuplicateLabel { .. }),
            "{error}"
        );
        let undeclared = job_labels("welcome", "admob", "completed");
        let error = registry
            .increment(metric, &undeclared)
            .expect_err("job type");
        assert!(
            matches!(error, WriteError::UndeclaredValue { .. }),
            "{error}"
        );
    }

    #[test]
    fn kind_mismatches_are_refused() {
        let registry = Registry::new();
        let labels = job_labels("daily", "applovin_max", "completed");
        assert!(matches!(
            registry.gauge_set(&catalog::JOB_RUNS_TOTAL, &labels, 1.0),
            Err(WriteError::KindMismatch { .. })
        ));
        assert!(matches!(
            registry.observe(&catalog::JOB_RUNS_TOTAL, &labels, 1.0),
            Err(WriteError::KindMismatch { .. })
        ));
        let gauge_labels = [("service", "apex-api")];
        assert!(matches!(
            registry.increment(&catalog::HTTP_REQUESTS_IN_FLIGHT, &gauge_labels),
            Err(WriteError::KindMismatch { .. })
        ));
    }

    #[test]
    fn gauges_set_add_and_read() {
        let registry = Registry::new();
        let metric = &catalog::REPORT_ACCOUNTS;
        let labels = [("platform", "applovin_max"), ("readiness_class", "final")];
        registry.gauge_set(metric, &labels, 3.0).unwrap();
        registry.gauge_add(metric, &labels, 2.5).unwrap();
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 5.5);
        assert_eq!(
            registry
                .gauge_value(
                    metric,
                    &[("platform", "admob"), ("readiness_class", "stale")]
                )
                .unwrap(),
            0.0
        );
        registry.gauge_set(metric, &labels, -1.0).unwrap();
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), -1.0);
    }

    #[test]
    fn histograms_keep_cumulative_buckets_count_and_sum() {
        let registry = Registry::new();
        let metric = &catalog::CONNECTOR_DURATION_SECONDS;
        let labels = [("platform", "applovin_max"), ("operation", "maxReport")];
        for value in [0.01, 0.3, 2.0, 90.0] {
            registry.observe(metric, &labels, value).unwrap();
        }
        let family = registry.family_snapshot(metric.name).expect("family");
        let series = &family.series[0];
        let histogram = series.histogram.as_ref().expect("histogram");
        assert_eq!(histogram.count, 4);
        assert!((histogram.sum - 92.31).abs() < 1e-9);
        // 0.05 bucket: only the 0.01 observation.
        assert_eq!(histogram.cumulative_counts[0], 1);
        // 0.5 bucket: 0.01 and 0.3.
        assert_eq!(histogram.cumulative_counts[3], 2);
        // +Inf bucket: everything.
        assert_eq!(*histogram.cumulative_counts.last().unwrap(), 4);
        assert_eq!(
            histogram.cumulative_counts.len(),
            histogram.buckets.len() + 1
        );
    }

    #[test]
    fn conflicts_and_debug_surfaces_work() {
        let registry = Registry::new();
        let labels = job_labels("daily", "applovin_max", "completed");
        registry
            .increment(&catalog::JOB_RUNS_TOTAL, &labels)
            .unwrap();
        assert!(!format!("{registry:?}").is_empty());
        let error = WriteError::ConflictingDeclaration { metric: "x" };
        assert!(error.to_string().contains("disagree"));
        let error = WriteError::from(crate::spec::LabelError::Empty { label: "y".into() });
        assert!(error.to_string().contains("empty"));
        for job in JobType::VALUES {
            registry
                .increment(
                    &catalog::JOB_RUNS_TOTAL,
                    &job_labels(job, "admob", "completed"),
                )
                .unwrap();
        }
        for platform in Platform::VALUES {
            registry
                .increment(
                    &catalog::JOB_RUNS_TOTAL,
                    &job_labels("daily", platform, Outcome::Completed.as_str()),
                )
                .unwrap();
        }
    }

    #[test]
    fn in_flight_guard_decrements_on_drop() {
        let registry = Registry::new();
        let metric = &catalog::HTTP_REQUESTS_IN_FLIGHT;
        let labels = [("service", "apex-api")];
        let outer = registry.in_flight(metric, &labels).unwrap();
        {
            let _inner = registry.in_flight(metric, &labels).unwrap();
            assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 2.0);
        }
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 1.0);
        drop(outer);
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 0.0);
        // Dropping again (by scope exit) never underflows past zero here
        // because each guard decrements exactly once.
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 0.0);
    }

    #[tokio::test]
    async fn in_flight_guard_cleans_up_when_the_task_is_cancelled() {
        // Leak the registry so the spawned (possibly aborted) task can hold
        // the guard; this is a test process, so the memory is reclaimed at
        // exit.
        let registry: &'static Registry = Box::leak(Box::new(Registry::new()));
        let metric = &catalog::HTTP_REQUESTS_IN_FLIGHT;
        let labels = [("service", "apex-runner")];
        let guard = registry.in_flight(metric, &labels).unwrap();
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 1.0);
        // Simulate a cancelled request: the future holding the guard is
        // dropped, which must run the guard's Drop.
        let pending = async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        };
        let handle = tokio::spawn(pending);
        tokio::task::yield_now().await;
        handle.abort();
        let _ = handle.await;
        assert_eq!(registry.gauge_value(metric, &labels).unwrap(), 0.0);
    }

    #[test]
    fn snapshot_is_canonical_and_complete() {
        let registry = Registry::new();
        registry
            .increment(
                &catalog::JOB_RUNS_TOTAL,
                &job_labels("monthly", "applovin_max", "completed"),
            )
            .unwrap();
        registry
            .increment(
                &catalog::JOB_RUNS_TOTAL,
                &job_labels("daily", "applovin_max", "completed"),
            )
            .unwrap();
        registry
            .gauge_set(
                &catalog::HTTP_REQUESTS_IN_FLIGHT,
                &[("service", "apex-api")],
                1.0,
            )
            .unwrap();
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 2);
        // Families sort by name: apex_http_* before apex_job_*.
        assert_eq!(snapshot[0].name, catalog::HTTP_REQUESTS_IN_FLIGHT.name);
        assert_eq!(snapshot[1].name, catalog::JOB_RUNS_TOTAL.name);
        // Series sort by their label tuple: daily before monthly.
        assert_eq!(snapshot[1].series[0].labels[0].1, "daily");
        assert_eq!(snapshot[1].series[0].counter, Some(1));
        assert!(registry.family_snapshot("apex_missing").is_none());
    }
}
