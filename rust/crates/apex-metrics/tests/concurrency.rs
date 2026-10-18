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
    HTTP_REQUESTS_IN_FLIGHT, HTTP_REQUESTS_TOTAL, HTTP_REQUEST_DURATION_SECONDS,
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
