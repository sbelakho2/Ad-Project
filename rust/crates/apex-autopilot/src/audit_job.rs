//! The audit job: build the AppLovin MAX account snapshot the write cycle
//! consumes, from the same keys the runner already holds. The snapshot is the
//! audit's input and the cycle's input, so one refresh serves both. A failed
//! refresh is recorded as the durable `audit_failed` event, not just an error
//! line: the audit lifecycle must be complete in analytics.

use std::path::Path;
use std::sync::Arc;

use apex_audit::{CountryRevenueRow, Snapshot};
use apex_connectors::app_ads::AppAdsTxtFetcher;
use apex_connectors::max::{MaxClient, MaxReportClient};
use apex_connectors::snapshot::{max_snapshot, PublisherContext};
use apex_connectors::transport::Transport;
use apex_core::PlatformError;

/// Build a MAX snapshot: ad units and app-ads.txt from the management key,
/// country revenue from the report key when present.
#[allow(clippy::too_many_arguments)]
pub async fn max_account_snapshot(
    api_key: &str,
    report_key: Option<&str>,
    management_base: &str,
    reporting_base: &str,
    transport: Arc<dyn Transport>,
    developer_site: &str,
    expected_networks: Vec<String>,
    now_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    let mut context = PublisherContext {
        developer_site: Some(developer_site.to_string()),
        expected_networks,
        ..PublisherContext::default()
    };
    if let Some(report_key) = report_key {
        let report = MaxReportClient::new(report_key, reporting_base, Arc::clone(&transport))?;
        let rows = report.country_revenue_30d(now_unix_ms).await?;
        context.country_revenue = rows
            .into_iter()
            .filter_map(|row| {
                Some(CountryRevenueRow {
                    country: row.country?,
                    ad_unit_id: row.ad_unit_id?,
                    revenue_30d: row.revenue,
                    impressions_30d: row.impressions,
                })
            })
            .collect();
    }
    let management = MaxClient::new(api_key, management_base, transport.clone())?;
    let fetcher = AppAdsTxtFetcher::new(transport);
    max_snapshot(&management, &fetcher, &context, now_unix_ms).await
}

/// Build the snapshot for the audit job and record the refresh outcome: a
/// failure emits the bounded `audit_failed` event (the snapshot refresh error
/// path) before the error is returned, so the audit's durable history is
/// complete. The event is emitted only when an emitter is attached.
#[allow(clippy::too_many_arguments)]
pub async fn audited_snapshot(
    api_key: &str,
    report_key: Option<&str>,
    management_base: &str,
    reporting_base: &str,
    transport: Arc<dyn Transport>,
    developer_site: &str,
    expected_networks: Vec<String>,
    events: Option<&crate::events::RunnerEvents>,
    rule_version: &str,
    now_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    match max_account_snapshot(
        api_key,
        report_key,
        management_base,
        reporting_base,
        transport,
        developer_site,
        expected_networks,
        now_unix_ms,
    )
    .await
    {
        Ok(snapshot) => Ok(snapshot),
        Err(error) => {
            if let Some(events) = events {
                events
                    .audit_failed(rule_version, snapshot_error_class(&error), now_unix_ms)
                    .await;
            }
            Err(error)
        }
    }
}

/// The bounded error class of a snapshot refresh failure, with the same
/// classes the runner's pull events use.
fn snapshot_error_class(source: &PlatformError) -> &'static str {
    match source {
        PlatformError::Auth { .. } => "auth_refusal",
        PlatformError::RateLimited { .. } => "rate_limit",
        PlatformError::Schema { .. } => "schema_failure",
        PlatformError::Network { .. } | PlatformError::Http { .. } => "transport_failure",
        PlatformError::Validation(_) => "validation_failure",
    }
}

/// Write a snapshot atomically (temp file then rename) so a reader never sees
/// a half-written snapshot.
pub fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("snapshot directory: {error}"))?;
    }
    let bytes =
        serde_json::to_vec_pretty(snapshot).map_err(|error| format!("serialize: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).map_err(|error| format!("write: {error}"))?;
    std::fs::rename(&tmp, path).map_err(|error| format!("rename: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use apex_connectors::transport::{HttpRequest, HttpResponse, TransportError};

    use super::*;

    #[derive(Default)]
    struct Fake {
        seen: Mutex<Vec<HttpRequest>>,
        queue: Mutex<Vec<HttpResponse>>,
    }

    impl Fake {
        fn push(&self, status: u16, body: &str) {
            self.queue.lock().unwrap().push(HttpResponse {
                status,
                headers: vec![],
                body: body.as_bytes().to_vec(),
            });
        }
    }

    #[async_trait::async_trait]
    impl Transport for Fake {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.seen.lock().unwrap().push(req);
            Ok(self.queue.lock().unwrap().remove(0))
        }
    }

    #[tokio::test]
    async fn the_audit_job_builds_a_snapshot_with_country_revenue() {
        let fake = Arc::new(Fake::default());
        // Reporting pull first, then the management list and app-ads.txt.
        fake.push(
            200,
            r#"{"code":200,"results":[{"day":"2026-10-01","country":"EE","max_ad_unit_id":"u1","estimated_revenue":"12.5","impressions":"100"}]}"#,
        );
        fake.push(
            200,
            r#"[{"id":"u1","name":"Rewarded","ad_format":"REWARD","platform":"android","package_name":"com.x"}]"#,
        );
        fake.push(200, "google.com, pub-1, DIRECT\n");
        let snapshot = max_account_snapshot(
            "key",
            Some("report"),
            "https://o.example.test/mediation/v1",
            "https://r.example.test",
            fake.clone(),
            "studio.example.com",
            vec!["google.com".to_string()],
            1_800_000_000_000,
        )
        .await
        .expect("snapshot");
        assert_eq!(snapshot.ad_units.len(), 1);
        assert_eq!(snapshot.country_revenue.len(), 1);
        assert_eq!(snapshot.country_revenue[0].revenue_30d, 12_500_000);
    }

    #[tokio::test]
    async fn a_failed_snapshot_refresh_emits_audit_failed() {
        use apex_analytics::store::{EventStore as _, ProjectionStore as _};
        use apex_analytics::{AnalyticsClient, MemoryStore};

        let store = Arc::new(MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let events = crate::events::RunnerEvents::new(
            Arc::new(AnalyticsClient::new(
                store.clone(),
                "dev",
                "apex-runner-test",
            )),
            "northstar",
        )
        .with_environment("dev");
        let fake = Arc::new(Fake::default());
        // The management call is refused: the refresh fails and the failure
        // is a durable `audit_failed` fact, not only an stderr line.
        fake.push(500, "boom");
        let error = audited_snapshot(
            "key",
            None,
            "https://o.example.test/mediation/v1",
            "https://r.example.test",
            fake.clone(),
            "studio.example.com",
            vec![],
            Some(&events),
            "v0.1.0",
            1_800_000_000_000,
        )
        .await
        .expect_err("refused");
        assert!(!error.to_string().is_empty());
        let stored = store.load_events().await.expect("events");
        assert_eq!(stored.len(), 1, "one audit_failed event landed");
        assert_eq!(stored[0].event_type.as_str(), "audit_failed");
        assert_eq!(stored[0].publisher_id.as_deref(), Some("northstar"));
        assert_eq!(stored[0].payload.rule_version.as_deref(), Some("v0.1.0"));
        assert!(stored[0].payload.error_class.is_some());

        // Without an emitter the failure is still returned typed.
        let fake = Arc::new(Fake::default());
        fake.push(500, "boom");
        assert!(audited_snapshot(
            "key",
            None,
            "https://o.example.test/mediation/v1",
            "https://r.example.test",
            fake,
            "studio.example.com",
            vec![],
            None,
            "v0.1.0",
            1_800_000_000_000,
        )
        .await
        .is_err());
    }

    #[test]
    fn writing_a_snapshot_round_trips() {
        let dir = std::env::temp_dir().join(format!("apex-audit-job-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("applovin_max.json");
        let snapshot = Snapshot {
            platform: apex_core::PlatformId::AppLovinMax,
            fetched_at_unix_ms: 3,
            ad_units: vec![],
            waterfall: vec![],
            mediation_groups: vec![],
            bidder_accounts: vec![],
            country_revenue: vec![],
            app_ads_txt: apex_audit::AppAdsTxt {
                present: false,
                lines: vec![],
                expected_networks: vec![],
            },
            platform_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
            mediation_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: 0,
                impressions_30d: 0,
            },
        };
        write_snapshot(&path, &snapshot).expect("write");
        let back: Snapshot = serde_json::from_slice(&std::fs::read(&path).unwrap()).expect("read");
        assert_eq!(back.fetched_at_unix_ms, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
