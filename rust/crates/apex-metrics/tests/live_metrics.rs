//! LIVE metrics verification: real registry arithmetic under real threads,
//! a real Prometheus exposition document parsed by the real `promtool`, and
//! the bounded handle cache / in-flight guard contracts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use apex_metrics::catalog::{self, HTTP_LATENCY_BUCKETS};
use apex_metrics::registry::HANDLE_CACHE_CAP;
use apex_metrics::{MetricSpec, Registry};

fn http_labels(route: &str) -> Vec<(&'static str, &str)> {
    vec![
        ("service", "apex-api"),
        ("route_template", route),
        ("method", "GET"),
        ("status_class", "2xx"),
    ]
}

fn histogram_labels(route: &str) -> Vec<(&'static str, &str)> {
    vec![
        ("service", "apex-api"),
        ("route_template", route),
        ("method", "GET"),
    ]
}

#[test]
fn real_prometheus_parser_accepts_the_rendered_scrape() {
    let registry = Registry::new();
    registry
        .increment(&catalog::HTTP_REQUESTS_TOTAL, &http_labels("/v1/live"))
        .expect("counter");
    registry
        .observe(
            &catalog::HTTP_REQUEST_DURATION_SECONDS,
            &histogram_labels("/v1/live"),
            0.0123,
        )
        .expect("histogram");
    registry
        .gauge_set(
            &catalog::HTTP_REQUESTS_IN_FLIGHT,
            &[("service", "apex-api")],
            3.0,
        )
        .expect("gauge");
    let exposition = registry.render_prometheus();
    assert!(exposition.contains("# TYPE apex_http_requests_total counter"));
    assert!(exposition.contains("# TYPE apex_http_request_duration_seconds histogram"));
    assert!(exposition.contains("apex_http_request_duration_seconds_bucket"));
    assert!(exposition.contains("le=\"+Inf\""));
    assert!(exposition.contains("apex_http_requests_in_flight{service=\"apex-api\"} 3"));

    let dir = std::env::temp_dir().join(format!("apex-metrics-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("scrape.prom");
    std::fs::write(&path, &exposition).expect("write scrape");
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let child = Command::new("promtool")
        .args(["check", "metrics"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    match child {
        Ok(mut child) => {
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(exposition.as_bytes())
                .expect("promtool stdin");
            let output = child.wait_with_output().expect("promtool output");
            assert!(
                output.status.success(),
                "promtool rejected the exposition:\nstdout: {}\nstderr: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                exposition
            );
            println!(
                "LIVE promtool check metrics: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            );
        }
        Err(error) => panic!("promtool is expected on this host: {error}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn histogram_snapshot_equals_a_manual_cumulative_count() {
    let registry = Registry::new();
    let labels = histogram_labels("/v1/histogram");
    let samples = [0.001, 0.004, 0.005, 0.006, 0.01, 0.02, 0.2, 1.0, 11.0, 2.5];
    for sample in samples {
        registry
            .observe(&catalog::HTTP_REQUEST_DURATION_SECONDS, &labels, sample)
            .expect("observe");
    }
    let family = registry
        .family_snapshot(catalog::HTTP_REQUEST_DURATION_SECONDS.name)
        .expect("family");
    assert_eq!(family.series.len(), 1);
    let histogram = family.series[0].histogram.as_ref().expect("histogram");
    assert_eq!(histogram.count, samples.len() as u64);
    let mut manual: Vec<u64> = HTTP_LATENCY_BUCKETS
        .iter()
        .map(|bound| samples.iter().filter(|sample| **sample <= *bound).count() as u64)
        .collect();
    // The snapshot documents the implicit +Inf bucket as the final entry.
    manual.push(samples.len() as u64);
    assert_eq!(histogram.cumulative_counts, manual);
    let expected_sum: f64 = samples.iter().sum();
    assert!(
        (histogram.sum - expected_sum).abs() < 1e-9,
        "sum {} != {expected_sum}",
        histogram.sum
    );
    // Cumulative counts are monotone and end at the observation count.
    for pair in histogram.cumulative_counts.windows(2) {
        assert!(pair[0] <= pair[1]);
    }
    assert_eq!(
        histogram.cumulative_counts.last().copied(),
        Some(samples.len() as u64)
    );
    assert_eq!(histogram.buckets, HTTP_LATENCY_BUCKETS);
}

#[test]
fn concurrent_observations_lose_no_counts() {
    let registry = Arc::new(Registry::new());
    let threads = 16usize;
    let per_thread = 1_000u64;
    let mut handles = Vec::new();
    for _ in 0..threads {
        let registry = Arc::clone(&registry);
        handles.push(std::thread::spawn(move || {
            for _ in 0..per_thread {
                registry
                    .increment(
                        &catalog::HTTP_REQUESTS_TOTAL,
                        &http_labels("/v1/concurrent"),
                    )
                    .expect("increment");
            }
        }));
    }
    for handle in handles {
        handle.join().expect("increment thread");
    }
    let total = registry
        .counter_value(
            &catalog::HTTP_REQUESTS_TOTAL,
            &http_labels("/v1/concurrent"),
        )
        .expect("counter");
    assert_eq!(total, threads as u64 * per_thread, "lost increments");

    // Histogram observations from several threads: the count and the sum
    // must both account for every observation.
    let observed = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let registry = Arc::clone(&registry);
        let observed = Arc::clone(&observed);
        handles.push(std::thread::spawn(move || {
            for index in 0..500 {
                registry
                    .observe(
                        &catalog::HTTP_REQUEST_DURATION_SECONDS,
                        &histogram_labels("/v1/concurrent"),
                        (index % 10) as f64 / 100.0,
                    )
                    .expect("observe");
                observed.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for handle in handles {
        handle.join().expect("observe thread");
    }
    let family = registry
        .family_snapshot(catalog::HTTP_REQUEST_DURATION_SECONDS.name)
        .expect("family");
    let histogram = family.series[0].histogram.as_ref().expect("histogram");
    assert_eq!(histogram.count, observed.load(Ordering::Relaxed));
    assert_eq!(histogram.count, 8 * 500);
    // 0.00..0.09 over 500 iterations, repeated 8 times: sum = 8 * 500 * 0.045.
    assert!(
        (histogram.sum - 180.0).abs() < 1e-6,
        "sum {}",
        histogram.sum
    );
}

#[test]
fn handle_cache_is_bounded_and_stable_under_many_series() {
    let registry = Registry::new();
    let metric: &'static MetricSpec = &catalog::HTTP_REQUESTS_TOTAL;
    // Warm one series repeatedly: the cache never grows for a stable series.
    for _ in 0..20 {
        registry
            .increment(metric, &http_labels("/v1/stable"))
            .unwrap();
    }
    let warmed = registry.handle_cache_len();
    assert_eq!(warmed, 1, "one stable series is one cache entry");
    assert_eq!(registry.handle_cache_len(), warmed);

    // More distinct templates than the cache capacity: the cache stays
    // bounded while every series keeps its real total in the family map.
    let distinct = HANDLE_CACHE_CAP + 500;
    for index in 0..distinct {
        let route = format!("/route-{index}");
        registry.increment(metric, &http_labels(&route)).unwrap();
    }
    assert!(
        registry.handle_cache_len() <= HANDLE_CACHE_CAP,
        "cache grew to {} (cap {HANDLE_CACHE_CAP})",
        registry.handle_cache_len()
    );
    let snapshot = registry.snapshot();
    let family = snapshot
        .iter()
        .find(|family| family.name == metric.name)
        .expect("family");
    assert_eq!(family.series.len(), distinct + 1);
    for series in &family.series {
        let route = series
            .labels
            .iter()
            .find(|(name, _)| *name == "route_template")
            .map(|(_, value)| value.as_str())
            .expect("route label");
        let expected = if route == "/v1/stable" { 20 } else { 1 };
        assert_eq!(series.counter, Some(expected), "{route}");
    }
    println!(
        "LIVE handle cache: {} distinct series, cache entries {} (cap {HANDLE_CACHE_CAP})",
        distinct + 1,
        registry.handle_cache_len()
    );
}

#[test]
fn in_flight_gauge_returns_to_zero_after_drop() {
    let registry = Registry::new();
    let labels = [("service", "apex-api")];
    let guards: Vec<_> = (0..5)
        .map(|_| {
            registry
                .in_flight(&catalog::HTTP_REQUESTS_IN_FLIGHT, &labels)
                .unwrap()
        })
        .collect();
    assert_eq!(
        registry
            .gauge_value(&catalog::HTTP_REQUESTS_IN_FLIGHT, &labels)
            .unwrap(),
        5.0
    );
    drop(guards);
    assert_eq!(
        registry
            .gauge_value(&catalog::HTTP_REQUESTS_IN_FLIGHT, &labels)
            .unwrap(),
        0.0
    );
}

#[tokio::test]
async fn in_flight_gauge_is_released_when_a_task_is_cancelled() {
    let registry = Arc::new(Registry::new());
    let labels = [("service", "apex-api")];
    let registry2 = Arc::clone(&registry);
    let handle = tokio::spawn(async move {
        let _guard = registry2
            .in_flight(&catalog::HTTP_REQUESTS_IN_FLIGHT, &labels)
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(
        registry
            .gauge_value(&catalog::HTTP_REQUESTS_IN_FLIGHT, &labels)
            .unwrap(),
        1.0
    );
    handle.abort();
    assert!(handle.await.is_err(), "the task was cancelled");
    assert_eq!(
        registry
            .gauge_value(&catalog::HTTP_REQUESTS_IN_FLIGHT, &labels)
            .unwrap(),
        0.0,
        "cancellation leaked the in-flight gauge"
    );
}
