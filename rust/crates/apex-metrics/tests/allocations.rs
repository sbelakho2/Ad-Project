#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Allocation contract for the registry's steady state.
//!
//! A counting global allocator instruments the whole process. The test is a
//! single function on purpose: with more than one test the harness would run
//! them on parallel threads and their allocations would interleave in the
//! process-wide counters.

use std::alloc::System;
use std::hint::black_box;

use apex_metrics::catalog::{CONNECTOR_DURATION_SECONDS, HTTP_REQUESTS_IN_FLIGHT, JOB_RUNS_TOTAL};
use apex_metrics::Registry;
use stats_alloc::{Region, StatsAlloc};

#[global_allocator]
static GLOBAL: StatsAlloc<System> = StatsAlloc::system();

const COUNTER: [(&str, &str); 3] = [
    ("job_type", "daily"),
    ("platform", "applovin_max"),
    ("outcome", "completed"),
];
const COUNTER_OTHER: [(&str, &str); 3] = [
    ("job_type", "monthly"),
    ("platform", "applovin_max"),
    ("outcome", "completed"),
];
const HISTOGRAM: [(&str, &str); 2] = [("platform", "applovin_max"), ("operation", "maxReport")];
const GAUGE: [(&str, &str); 1] = [("service", "apex-api")];

#[test]
fn steady_state_observations_allocate_nothing_and_only_creation_allocates() {
    let registry = Registry::new();

    // Creating a family and its first series is the allocating path.
    let creation = Region::new(&GLOBAL);
    registry.increment(&JOB_RUNS_TOTAL, &COUNTER).unwrap();
    let first = creation.change();
    assert!(
        first.allocations > 0,
        "first creation must allocate, saw {first:?}"
    );

    // Repeating the identical observation on the existing series allocates
    // nothing: the key is hashed and compared through borrowed values.
    let repeat = Region::new(&GLOBAL);
    for _ in 0..1_000 {
        black_box(registry.increment(&JOB_RUNS_TOTAL, &COUNTER).unwrap());
    }
    let repeated = repeat.change();
    assert_eq!(
        repeated.allocations, 0,
        "an existing counter series allocates on observation: {repeated:?}"
    );
    assert_eq!(
        repeated.reallocations, 0,
        "an existing counter series reallocates: {repeated:?}"
    );

    // A different label set is a new series, so only that creation allocates.
    let another = Region::new(&GLOBAL);
    registry.increment(&JOB_RUNS_TOTAL, &COUNTER_OTHER).unwrap();
    let created = another.change();
    assert!(
        created.allocations > 0,
        "a new series must allocate, saw {created:?}"
    );
    let another_repeat = Region::new(&GLOBAL);
    registry.increment(&JOB_RUNS_TOTAL, &COUNTER_OTHER).unwrap();
    assert_eq!(another_repeat.change().allocations, 0);

    // Warm the histogram and gauge series through their own creation path.
    registry
        .observe(&CONNECTOR_DURATION_SECONDS, &HISTOGRAM, 1.0)
        .unwrap();
    let guard = registry
        .in_flight(&HTTP_REQUESTS_IN_FLIGHT, &GAUGE)
        .unwrap();
    drop(guard);

    // Every write path is allocation-free once its series exists.
    let steady = Region::new(&GLOBAL);
    for _ in 0..1_000 {
        black_box(registry.increment(&JOB_RUNS_TOTAL, &COUNTER).unwrap());
        registry
            .observe(&CONNECTOR_DURATION_SECONDS, &HISTOGRAM, 1.0)
            .unwrap();
        let guard = registry
            .in_flight(&HTTP_REQUESTS_IN_FLIGHT, &GAUGE)
            .unwrap();
        drop(guard);
    }
    let steady_stats = steady.change();
    assert_eq!(
        steady_stats.allocations, 0,
        "steady-state observations allocate: {steady_stats:?}"
    );
    assert_eq!(
        steady_stats.reallocations, 0,
        "steady-state observations reallocate: {steady_stats:?}"
    );

    // The rendered output is byte-exact for the measured series.
    let render_registry = Registry::new();
    render_registry
        .increment(&JOB_RUNS_TOTAL, &COUNTER)
        .unwrap();
    render_registry
        .increment(&JOB_RUNS_TOTAL, &COUNTER)
        .unwrap();
    assert_eq!(
        render_registry.render_prometheus(),
        "# HELP apex_job_runs_total Scheduled runner job runs by outcome.\n\
         # TYPE apex_job_runs_total counter\n\
         apex_job_runs_total{job_type=\"daily\",platform=\"applovin_max\",outcome=\"completed\"} 2\n"
    );
}
