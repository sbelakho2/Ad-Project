//! Prometheus text exposition.
//!
//! Renders a registry snapshot in the Prometheus text format (version
//! 0.0.4): `# HELP` and `# TYPE` lines, escaped label values, histogram
//! `_bucket` / `_sum` / `_count` series. Series order is canonical, so the
//! output is deterministic and diffable.

use crate::registry::{FamilySnapshot, Registry, SeriesSnapshot};
use crate::spec::{MetricKind, MetricSpec};

/// Escape a label value per the exposition format: backslash, double quote
/// and newline.
pub fn escape_label_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// Escape `# HELP` text: backslash and newline.
pub fn escape_help(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// Format a float the way Prometheus expects: integers without a decimal
/// point, `+Inf`/`-Inf`/`NaN` spelled out.
pub fn format_float(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "+Inf" } else { "-Inf" }.to_string();
    }
    if value == value.trunc() && value.abs() < 9.007_199_254_740_992e15 {
        return format!("{}", value as i64);
    }
    format!("{value}")
}

fn label_pairs(series: &SeriesSnapshot, extra: Option<(&str, String)>) -> String {
    if series.labels.is_empty() && extra.is_none() {
        return String::new();
    }
    let mut parts: Vec<String> = series
        .labels
        .iter()
        .map(|(name, value)| format!("{name}=\"{}\"", escape_label_value(value)))
        .collect();
    if let Some((name, value)) = extra {
        parts.push(format!("{name}=\"{}\"", escape_label_value(&value)));
    }
    format!("{{{}}}", parts.join(","))
}

fn render_series(out: &mut String, base: &str, series: &SeriesSnapshot, value: f64) {
    out.push_str(base);
    out.push_str(&label_pairs(series, None));
    out.push(' ');
    out.push_str(&format_float(value));
    out.push('\n');
}

impl Registry {
    /// Render every written family as Prometheus text.
    pub fn render_prometheus(&self) -> String {
        render_snapshot(&self.snapshot())
    }
}

/// Render a snapshot as Prometheus text (families already in canonical
/// order).
pub fn render_snapshot(snapshot: &[FamilySnapshot]) -> String {
    let mut out = String::new();
    for family in snapshot {
        out.push_str("# HELP ");
        out.push_str(family.name);
        out.push(' ');
        out.push_str(&escape_help(family.help));
        out.push('\n');
        out.push_str("# TYPE ");
        out.push_str(family.name);
        out.push(' ');
        out.push_str(type_name(family.kind));
        out.push('\n');
        for series in &family.series {
            render_one(&mut out, family, series);
        }
    }
    out
}

fn type_name(kind: MetricKind) -> &'static str {
    match kind {
        MetricKind::Counter => "counter",
        MetricKind::Gauge => "gauge",
        MetricKind::Histogram { .. } => "histogram",
    }
}

fn render_one(out: &mut String, family: &FamilySnapshot, series: &SeriesSnapshot) {
    match family.kind {
        MetricKind::Counter => {
            render_series(out, family.name, series, series.counter.unwrap_or(0) as f64);
        }
        MetricKind::Gauge => {
            render_series(out, family.name, series, series.gauge.unwrap_or(0.0));
        }
        MetricKind::Histogram { .. } => {
            let Some(histogram) = &series.histogram else {
                return;
            };
            let bucket_base = format!("{}_bucket", family.name);
            for (index, bound) in histogram.buckets.iter().enumerate() {
                let count = histogram.cumulative_counts.get(index).copied().unwrap_or(0);
                out.push_str(&bucket_base);
                out.push_str(&label_pairs(series, Some(("le", format_float(*bound)))));
                out.push(' ');
                out.push_str(&format_float(count as f64));
                out.push('\n');
            }
            out.push_str(&bucket_base);
            out.push_str(&label_pairs(series, Some(("le", "+Inf".to_string()))));
            out.push(' ');
            out.push_str(&format_float(histogram.count as f64));
            out.push('\n');
            render_series(out, &format!("{}_sum", family.name), series, histogram.sum);
            render_series(
                out,
                &format!("{}_count", family.name),
                series,
                histogram.count as f64,
            );
        }
    }
}

/// The text exposition content type (`GET /metrics` responses and textfile
/// scrapes share it).
pub const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// A metric declaration paired with a rendered check, used by tests to pin
/// output shape. Kept public so dependent crates can assert their own series
/// appear in an exported textfile.
pub fn render_metric(registry: &Registry, metric: &MetricSpec) -> String {
    let Some(family) = registry.family_snapshot(metric.name) else {
        return String::new();
    };
    let mut out = String::new();
    out.push_str("# HELP ");
    out.push_str(family.name);
    out.push(' ');
    out.push_str(&escape_help(family.help));
    out.push('\n');
    out.push_str("# TYPE ");
    out.push_str(family.name);
    out.push(' ');
    out.push_str(type_name(family.kind));
    out.push('\n');
    for series in &family.series {
        render_one(&mut out, &family, series);
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::catalog;
    use crate::registry::Registry;

    #[test]
    fn escaping_matches_the_exposition_format() {
        assert_eq!(escape_label_value(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(escape_label_value("line\nbreak"), "line\\nbreak");
        assert_eq!(escape_help("a\\b\nc"), "a\\\\b\\nc");
    }

    #[test]
    fn float_formatting_is_prometheus_shaped() {
        assert_eq!(format_float(1.0), "1");
        assert_eq!(format_float(0.5), "0.5");
        assert_eq!(format_float(0.01), "0.01");
        assert_eq!(format_float(f64::INFINITY), "+Inf");
        assert_eq!(format_float(f64::NEG_INFINITY), "-Inf");
        assert_eq!(format_float(f64::NAN), "NaN");
        assert_eq!(format_float(3_000.0), "3000");
    }

    #[test]
    fn counters_gauges_and_histograms_render_with_type_lines() {
        let registry = Registry::new();
        registry
            .increment(
                &catalog::JOB_RUNS_TOTAL,
                &[
                    ("job_type", "daily"),
                    ("platform", "applovin_max"),
                    ("outcome", "completed"),
                ],
            )
            .unwrap();
        registry
            .gauge_set(
                &catalog::HTTP_REQUESTS_IN_FLIGHT,
                &[("service", "apex-api")],
                2.0,
            )
            .unwrap();
        registry
            .observe(
                &catalog::JOB_DURATION_SECONDS,
                &[
                    ("job_type", "daily"),
                    ("platform", "applovin_max"),
                    ("outcome", "completed"),
                ],
                1.5,
            )
            .unwrap();
        let text = registry.render_prometheus();
        assert!(text.contains("# TYPE apex_job_runs_total counter\n"));
        assert!(text.contains(
            "apex_job_runs_total{job_type=\"daily\",platform=\"applovin_max\",outcome=\"completed\"} 1\n"
        ));
        assert!(text.contains("# TYPE apex_http_requests_in_flight gauge\n"));
        assert!(text.contains("apex_http_requests_in_flight{service=\"apex-api\"} 2\n"));
        assert!(text.contains("# TYPE apex_job_duration_seconds histogram\n"));
        assert!(text.contains(
            "apex_job_duration_seconds_bucket{job_type=\"daily\",platform=\"applovin_max\",outcome=\"completed\",le=\"2\"} 1\n"
        ));
        assert!(text.contains(
            "apex_job_duration_seconds_bucket{job_type=\"daily\",platform=\"applovin_max\",outcome=\"completed\",le=\"+Inf\"} 1\n"
        ));
        assert!(text.contains(
            "apex_job_duration_seconds_sum{job_type=\"daily\",platform=\"applovin_max\",outcome=\"completed\"} 1.5\n"
        ));
        assert!(text.contains(
            "apex_job_duration_seconds_count{job_type=\"daily\",platform=\"applovin_max\",outcome=\"completed\"} 1\n"
        ));
        // Every HELP line precedes its TYPE line, and the output ends with a
        // newline (textfile collectors require a clean final line).
        assert!(text.ends_with('\n'));
        let help_index = text.find("# HELP apex_job_runs_total").unwrap();
        let type_index = text.find("# TYPE apex_job_runs_total").unwrap();
        assert!(help_index < type_index);
    }

    #[test]
    fn an_empty_registry_renders_nothing() {
        assert_eq!(Registry::new().render_prometheus(), "");
    }

    #[test]
    fn unlabeled_series_and_missing_histogram_bodies_render_safely() {
        let snapshot = vec![
            FamilySnapshot {
                name: "apex_probe_no_labels",
                help: "No labels.",
                kind: MetricKind::Counter,
                series: vec![SeriesSnapshot {
                    labels: Vec::new(),
                    counter: Some(3),
                    gauge: None,
                    histogram: None,
                }],
            },
            FamilySnapshot {
                name: "apex_probe_missing_histogram",
                help: "Missing body.",
                kind: MetricKind::Histogram {
                    buckets: &[1.0, 5.0],
                },
                series: vec![SeriesSnapshot {
                    labels: vec![("service", "apex-api".to_string())],
                    counter: None,
                    gauge: None,
                    histogram: None,
                }],
            },
        ];
        let text = render_snapshot(&snapshot);
        assert!(text.contains("apex_probe_no_labels 3\n"));
        assert!(!text.contains("apex_probe_no_labels{"));
        assert!(text.contains("# TYPE apex_probe_missing_histogram histogram\n"));
        assert!(!text.contains("apex_probe_missing_histogram_bucket"));
        assert!(!text.contains("apex_probe_missing_histogram_count"));
    }

    #[test]
    fn render_metric_returns_only_that_family() {
        let registry = Registry::new();
        registry
            .increment(
                &catalog::JOB_RUNS_TOTAL,
                &[
                    ("job_type", "monthly"),
                    ("platform", "admob"),
                    ("outcome", "failed"),
                ],
            )
            .unwrap();
        let text = render_metric(&registry, &catalog::JOB_RUNS_TOTAL);
        assert!(text.contains("apex_job_runs_total"));
        assert!(!text.contains("apex_http"));
        assert!(render_metric(&registry, &catalog::HTTP_REQUESTS_TOTAL).is_empty());
        assert_eq!(
            PROMETHEUS_CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8"
        );
    }
}
