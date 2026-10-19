#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Contention harness for the bounded registry.
//!
//! The old registry guarded every observation with one process-wide `Mutex`.
//! This harness drives the HTTP trio (`requests_total`,
//! `request_duration_seconds`, `requests_in_flight`) from many threads and
//! reports the lock waits the registry actually observed. It asserts the
//! deterministic facts (exact totals, the guard returning to zero and byte
//! equality with a sequential render); wall time and throughput are printed
//! for the report and are never asserted.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::Instant;

use apex_metrics::catalog::{
    CONNECTOR_DURATION_SECONDS, HTTP_REQUESTS_IN_FLIGHT, HTTP_REQUESTS_TOTAL,
    HTTP_REQUEST_DURATION_SECONDS,
};
use apex_metrics::Registry;

const SERVICE: [(&str, &str); 1] = [("service", "apex-api")];
const ROUTES: [(&str, &str); 4] = [
    ("service", "apex-api"),
    ("route_template", "/v1/reports"),
    ("method", "GET"),
    ("status_class", "2xx"),
];
const DURATION: [(&str, &str); 3] = [
    ("service", "apex-api"),
    ("route_template", "/v1/reports"),
    ("method", "GET"),
];

fn run_threads(registry: &Registry, threads: usize, per_thread: usize) {
    let barrier = Arc::new(Barrier::new(threads));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..per_thread {
                    registry
                        .increment(&HTTP_REQUESTS_TOTAL, &ROUTES)
                        .expect("counter write");
                    registry
                        .observe(&HTTP_REQUEST_DURATION_SECONDS, &DURATION, 1.0)
                        .expect("histogram write");
                    let guard = registry
                        .in_flight(&HTTP_REQUESTS_IN_FLIGHT, &SERVICE)
                        .expect("in-flight write");
                    drop(guard);
                }
            });
        }
    });
}

/// The pre-fix shape: every observation locks one process-wide mutex. It
/// exists only to show the contention the redesign removed; it asserts
/// nothing.
fn global_mutex_baseline(threads: usize, per_thread: usize) -> (u64, f64, u64) {
    let state = Arc::new(Mutex::new((0u64, 0u64)));
    let waits = Arc::new(AtomicU64::new(0));
    let barrier = Arc::new(Barrier::new(threads));
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let state = Arc::clone(&state);
            let waits = Arc::clone(&waits);
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..per_thread {
                    if state.try_lock().is_err() {
                        waits.fetch_add(1, Ordering::Relaxed);
                    }
                    let mut guard = state.lock().expect("baseline mutex");
                    guard.0 += 1;
                    guard.1 += 1;
                }
            });
        }
    });
    let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
    (
        waits.load(Ordering::Relaxed),
        elapsed,
        threads as u64 * per_thread as u64,
    )
}

fn sequential_reference(operations: u64) -> Registry {
    let registry = Registry::new();
    for _ in 0..operations {
        registry
            .increment(&HTTP_REQUESTS_TOTAL, &ROUTES)
            .expect("counter write");
        registry
            .observe(&HTTP_REQUEST_DURATION_SECONDS, &DURATION, 1.0)
            .expect("histogram write");
        let guard = registry
            .in_flight(&HTTP_REQUESTS_IN_FLIGHT, &SERVICE)
            .expect("in-flight write");
        drop(guard);
    }
    registry
}

#[test]
fn concurrent_writes_are_exact_and_wait_free_in_steady_state() {
    for threads in [1usize, 64, 128] {
        let per_thread = 2_000usize;
        let registry = Registry::new();
        // Warm the family and series maps: creation is the only path that
        // takes a write lock. Steady-state observations must not wait.
        run_threads(&registry, 1, 1);
        let before = registry.lock_waits();
        let started = Instant::now();
        run_threads(&registry, threads, per_thread);
        let elapsed = started.elapsed();

        let expected = threads.saturating_mul(per_thread) as u64 + 1;
        assert_eq!(
            registry
                .counter_value(&HTTP_REQUESTS_TOTAL, &ROUTES)
                .expect("read"),
            expected,
            "{threads} threads"
        );
        let family = registry
            .family_snapshot(HTTP_REQUEST_DURATION_SECONDS.name)
            .expect("histogram family");
        let histogram = family.series[0].histogram.as_ref().expect("histogram");
        assert_eq!(histogram.count, expected, "{threads} threads");
        assert!((histogram.sum - expected as f64).abs() < 1e-9);
        assert_eq!(
            registry
                .gauge_value(&HTTP_REQUESTS_IN_FLIGHT, &SERVICE)
                .expect("gauge"),
            0.0,
            "every in-flight guard decremented on drop"
        );

        let after = registry.lock_waits();
        let waits = (
            after.read_waits.saturating_sub(before.read_waits),
            after.write_waits.saturating_sub(before.write_waits),
        );
        let rendered = registry.render_prometheus();
        assert_eq!(
            rendered,
            sequential_reference(expected).render_prometheus(),
            "concurrent render equals the sequential render at {threads} threads"
        );
        let per_second = expected as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
        let (base_waits, base_ms, base_ops) = global_mutex_baseline(threads, per_thread);
        eprintln!(
            "threads={threads} ops={expected} wall_ms={:.2} read_waits={} write_waits={} ops_per_s={per_second:.0} baseline_wall_ms={base_ms:.2} baseline_lock_waits={base_waits} baseline_ops={base_ops}",
            elapsed.as_secs_f64() * 1_000.0,
            waits.0,
            waits.1
        );
        // The steady-state path takes read locks only; no writer exists, so
        // nothing can make a read wait. (A regression to a write lock or to a
        // global mutex would show up here.)
        assert_eq!(waits, (0, 0), "{threads} threads: {waits:?}");
    }
}

#[test]
fn sixteen_threads_and_a_scrape_lose_no_bucket_counts() {
    const THREADS: usize = 16;
    const ROUNDS: usize = 4_000;
    let bounds = CONNECTOR_DURATION_SECONDS.buckets().expect("histogram");
    // One value per bucket (the first is below the range, the last above it),
    // so every observation lands in a known bucket and the counts are easy to
    // check exactly.
    let mut values: Vec<f64> = Vec::with_capacity(bounds.len() + 1);
    for (index, bound) in bounds.iter().enumerate() {
        values.push(if index == 0 {
            bound - 1.0
        } else {
            bounds[index - 1] + (bound - bounds[index - 1]) / 2.0
        });
    }
    values.push(bounds[bounds.len() - 1] + 1.0);
    let per_bucket = (THREADS * ROUNDS) as u64;
    let labels = [("platform", "applovin_max"), ("operation", "maxReport")];

    let registry = Registry::new();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let registry = &registry;
    let values = &values;
    std::thread::scope(|scope| {
        let mut writers = Vec::with_capacity(THREADS);
        for _ in 0..THREADS {
            writers.push(scope.spawn(move || {
                for _ in 0..ROUNDS {
                    for value in values {
                        registry
                            .observe(&CONNECTOR_DURATION_SECONDS, &labels, *value)
                            .expect("histogram write");
                    }
                }
            }));
        }
        let stopped = Arc::clone(&done);
        // Scrape while the writers run: a scrape may be a moment behind, but
        // no bucket may ever move backwards and the final scrape must be
        // exact.
        let scraper = scope.spawn(move || {
            let mut previous: Option<Vec<u64>> = None;
            let mut scrapes = 0usize;
            while !stopped.load(Ordering::Relaxed) {
                let family = registry
                    .family_snapshot(CONNECTOR_DURATION_SECONDS.name)
                    .expect("family");
                let histogram = family.series[0].histogram.as_ref().expect("histogram");
                if let Some(previous) = &previous {
                    for (now, before) in histogram.cumulative_counts.iter().zip(previous) {
                        assert!(now >= before, "a scrape lost a bucket count");
                    }
                }
                previous = Some(histogram.cumulative_counts.clone());
                scrapes += 1;
                std::thread::yield_now();
            }
            (scrapes, previous)
        });
        for writer in writers {
            writer.join().expect("writer");
        }
        // Writers are joined, so this snapshot is stable; tell the scraper to
        // stop and let it finish its last pass.
        done.store(true, Ordering::Relaxed);
        let (scrapes, observed) = scraper.join().expect("scrape");
        assert!(observed.is_some(), "the scrape ran at least once");
        assert!(scrapes > 0);
    });

    let family = registry
        .family_snapshot(CONNECTOR_DURATION_SECONDS.name)
        .expect("family");
    let histogram = family.series[0].histogram.as_ref().expect("histogram");
    let expected: Vec<u64> = (1..=values.len() as u64)
        .map(|buckets| buckets * per_bucket)
        .collect();
    assert_eq!(histogram.cumulative_counts, expected);
    assert_eq!(histogram.count, values.len() as u64 * per_bucket);
    let expected_sum: f64 = values.iter().sum::<f64>() * (THREADS * ROUNDS) as f64;
    assert!(
        (histogram.sum - expected_sum).abs() < 1e-6 * expected_sum.abs().max(1.0),
        "sum {} != {expected_sum}",
        histogram.sum
    );
}
