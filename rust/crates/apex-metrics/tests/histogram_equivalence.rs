#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Equivalence proof for the per-bucket histogram representation.
//!
//! The registry now stores one count per bucket and prefix-sums when it
//! snapshots; it used to store cumulative counts and increment every bucket
//! whose bound admitted the observation. These tests keep the old cumulative
//! algorithm verbatim and assert the rendered exposition is byte-identical
//! for every declared boundary and for values below and above the range.

use apex_metrics::catalog::{
    CONNECTOR_DURATION_SECONDS, HTTP_REQUEST_DURATION_SECONDS, JOB_DURATION_SECONDS,
};
use apex_metrics::{
    render_snapshot, FamilySnapshot, HistogramSnapshot, MetricKind, MetricSpec, Registry,
    SeriesSnapshot,
};

/// The pre-optimization observation rule: every bucket whose bound admits
/// the value is incremented, and the implicit `+Inf` bucket always is.
struct CumulativeReference {
    bounds: &'static [f64],
    cumulative: Vec<u64>,
    count: u64,
    sum: f64,
}

impl CumulativeReference {
    fn new(bounds: &'static [f64]) -> Self {
        // Declared bounds plus the implicit +Inf bucket.
        Self {
            bounds,
            cumulative: vec![0; bounds.len() + 1],
            count: 0,
            sum: 0.0,
        }
    }

    fn observe(&mut self, value: f64) {
        for (index, bound) in self.bounds.iter().enumerate() {
            if value <= *bound {
                self.cumulative[index] += 1;
            }
        }
        *self.cumulative.last_mut().expect("the +Inf bucket exists") += 1;
        self.count += 1;
        self.sum += value;
    }
}

fn assert_observations_render_identically(metric: &'static MetricSpec, values: &[f64]) {
    let bounds = metric.buckets().expect("histogram");
    let labels: &[(&'static str, &'static str)] = match metric.name {
        name if name == CONNECTOR_DURATION_SECONDS.name => {
            &[("platform", "applovin_max"), ("operation", "maxReport")]
        }
        name if name == JOB_DURATION_SECONDS.name => &[
            ("job_type", "daily"),
            ("platform", "applovin_max"),
            ("outcome", "completed"),
        ],
        _ => &[
            ("service", "apex-api"),
            ("route_template", "/v1/reports"),
            ("method", "GET"),
        ],
    };

    let registry = Registry::new();
    for value in values {
        registry
            .observe(metric, labels, *value)
            .expect("histogram write");
    }

    // Reference snapshot computed with the old cumulative algorithm. Label
    // order and values are taken from the registry snapshot, which is part of
    // the public contract and therefore not what this test is pinning.
    let observed = registry.snapshot();
    let observed_family = observed.first().expect("one family");
    let observed_labels = observed_family.series[0].labels.clone();
    let mut reference = CumulativeReference::new(bounds);
    for value in values {
        reference.observe(*value);
    }
    let expected = FamilySnapshot {
        name: metric.name,
        help: metric.help,
        kind: metric.kind,
        series: vec![SeriesSnapshot {
            labels: observed_labels,
            counter: None,
            gauge: None,
            histogram: Some(HistogramSnapshot {
                buckets: bounds.to_vec(),
                cumulative_counts: reference.cumulative,
                count: reference.count,
                sum: reference.sum,
            }),
        }],
    };

    assert_eq!(
        render_snapshot(&[expected]),
        registry.render_prometheus(),
        "{} must render byte-identically to the old cumulative algorithm for {values:?}",
        metric.name
    );
    // The public snapshot shape stays cumulative and ends at the count.
    let histogram = observed_family.series[0]
        .histogram
        .as_ref()
        .expect("histogram");
    assert_eq!(histogram.count, values.len() as u64);
    assert_eq!(
        histogram.cumulative_counts.last().copied(),
        Some(values.len() as u64)
    );
    assert_eq!(histogram.cumulative_counts.len(), bounds.len() + 1);
}

fn boundaries_cases(metric: &'static MetricSpec) -> Vec<Vec<f64>> {
    let bounds = metric.buckets().expect("histogram");
    let mut cases = Vec::new();
    for (index, bound) in bounds.iter().enumerate() {
        // The boundary itself, one representable step below it, and one
        // above it. The first case straddles the lower edge of the range.
        let below = if index == 0 {
            bound - 1.0
        } else {
            bound - f64::EPSILON * bound.abs().max(1.0)
        };
        let above = bound + f64::EPSILON * bound.abs().max(1.0);
        cases.push(vec![below, *bound, above]);
    }
    cases.push(vec![bounds[0] - 1_000.0]);
    cases.push(vec![bounds[bounds.len() - 1] + 1_000.0]);
    cases.push(vec![f64::NEG_INFINITY, f64::INFINITY]);
    cases.push(vec![f64::NAN]);
    cases
}

#[test]
fn every_boundary_and_out_of_range_value_renders_identically() {
    for metric in [
        &CONNECTOR_DURATION_SECONDS,
        &JOB_DURATION_SECONDS,
        &HTTP_REQUEST_DURATION_SECONDS,
    ] {
        for values in boundaries_cases(metric) {
            assert_observations_render_identically(metric, &values);
        }
    }
}

#[test]
fn cumulative_snapshot_matches_a_hand_checked_example() {
    let metric = &CONNECTOR_DURATION_SECONDS;
    let labels = [("platform", "applovin_max"), ("operation", "maxReport")];
    let registry = Registry::new();
    // 0.05 bucket: -1 and 0.01. 0.1 bucket: exactly 0.1. 0.25 bucket: 0.2.
    // +Inf: 100.
    for value in [-1.0, 0.01, 0.1, 0.2, 100.0] {
        registry.observe(metric, &labels, value).unwrap();
    }
    let family = registry.family_snapshot(metric.name).expect("family");
    let histogram = family.series[0].histogram.as_ref().expect("histogram");
    assert_eq!(
        histogram.cumulative_counts,
        vec![2, 3, 4, 4, 4, 4, 4, 4, 4, 4, 5]
    );
    assert_eq!(histogram.count, 5);
    assert!((histogram.sum - 99.31).abs() < 1e-9);
    assert_eq!(
        metric.kind,
        MetricKind::Histogram {
            buckets: metric.buckets().unwrap()
        }
    );
}
