//! End-to-end latency benches for the HTTP surface: `/healthz` (the routing +
//! serialization floor) and `/console/audit?platform=admob` over a synthetic
//! 500-finding report (the console rendering path, which is the heaviest
//! response this deployment serves).
//!
//! Both benches drive the real router through `tower::ServiceExt::oneshot`, so
//! they measure handler + escaping + serialization, not the kernel network
//! stack. The CI lane owns the numeric budgets; this file exists to make them
//! measurable and to keep the console render path honest as it evolves.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::Arc;

use apex_api::{router, AppState, FixedClock, StaticAuditSource, StaticLedgerSource};
use apex_core::PlatformId;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use criterion::{criterion_group, criterion_main, Criterion};
use serde_json::json;
use tower::ServiceExt;

/// A snapshot that produces at least `findings` findings: every ad unit gets a
/// zero-revenue floor pair in `EE` (a stale-floor finding) plus a monetized
/// `US` row that supplies the eCPM basis.
fn synthetic_snapshot(findings: usize) -> apex_audit::Snapshot {
    let mut ad_units = Vec::with_capacity(findings);
    let mut country_revenue = Vec::with_capacity(findings * 2);
    for index in 0..findings {
        let unit_id = format!("u{index}");
        ad_units.push(json!({
            "platformUnitId": unit_id,
            "name": format!("unit {index}"),
            "format": "banner",
            "enabled": true,
            "countryFloors": [{"country": "EE", "floorEcpm": 2_500_000}]
        }));
        country_revenue.push(json!({
            "country": "EE",
            "adUnitId": unit_id,
            "revenue30d": 0,
            "impressions30d": 100_000
        }));
        country_revenue.push(json!({
            "country": "US",
            "adUnitId": unit_id,
            "revenue30d": 10_000_000,
            "impressions30d": 100_000
        }));
    }
    let snapshot = json!({
        "platform": "admob",
        "fetchedAtUnixMs": 1_700_000_000_000i64,
        "adUnits": ad_units,
        "waterfall": [],
        "mediationGroups": [],
        "bidderAccounts": [],
        "countryRevenue": country_revenue,
        "appAdsTxt": {"present": true, "lines": [], "expectedNetworks": []},
        "platformTotals30d": {"revenue30d": 10_000_000, "impressions30d": 100_000},
        "mediationTotals30d": {"revenue30d": 10_000_000, "impressions30d": 100_000}
    });
    serde_json::from_value(snapshot).expect("synthetic snapshot is well-formed")
}

fn bench_http(c: &mut Criterion) {
    let mut snapshots = HashMap::new();
    snapshots.insert(PlatformId::Admob, synthetic_snapshot(500));
    let state = AppState::new(
        Arc::new(StaticAuditSource::new(snapshots)),
        Arc::new(StaticLedgerSource::default()),
    )
    .with_clock(Arc::new(FixedClock::new(1_700_000_000)));
    let app = router(state);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let mut group = c.benchmark_group("apex-api");
    group.bench_function("healthz", |bencher| {
        bencher.to_async(&runtime).iter(|| {
            let app = app.clone();
            async move {
                let request = Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request");
                let response = app.oneshot(request).await.expect("infallible");
                assert_eq!(response.status(), StatusCode::OK);
                response
            }
        });
    });
    group.bench_function("console_audit_render_500_findings", |bencher| {
        bencher.to_async(&runtime).iter(|| {
            let app = app.clone();
            async move {
                let request = Request::builder()
                    .uri("/console/audit?platform=admob")
                    .body(Body::empty())
                    .expect("request");
                let response = app.oneshot(request).await.expect("infallible");
                assert_eq!(response.status(), StatusCode::OK);
                response
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_http);
criterion_main!(benches);
