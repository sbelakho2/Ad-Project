//! The audit job: build one account snapshot per publisher and platform from
//! the same vaulted keys the runner already holds, and write each beside the
//! reads at `<root>/<publisher>/<platform>.json`. The snapshot is the audit's
//! input and the cycle's input, so one refresh serves both. A failed refresh
//! is recorded as the durable `audit_failed` event, not just an error line:
//! the audit lifecycle must be complete in analytics.
//!
//! AppLovin MAX, LevelPlay and AdMob all have a builder here, mirroring the
//! console's `LiveAuditor` (`apex-api::live`) read for read: MAX pulls ad units
//! and country revenue; LevelPlay pulls the app's instances (waterfall) and
//! its app-level revenue; AdMob pulls the mediation groups and its mediation
//! revenue report. Every builder fills both `platform_totals_30d` and the
//! independent mediation-side `mediation_totals_30d`, so the reporting-gap rule
//! can fire. All three run the same app-ads.txt check with the account's
//! developer site and expected networks.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use apex_audit::{CountryRevenueRow, RevenueTotals, Snapshot};
use apex_connectors::admob::AdmobClient;
use apex_connectors::app_ads::AppAdsTxtFetcher;
use apex_connectors::levelplay::LevelPlayClient;
use apex_connectors::max::{MaxClient, MaxReportClient, MAX_REPORTING_BASE};
use apex_connectors::snapshot::{
    admob_snapshot, levelplay_app_snapshot, max_snapshot, PublisherContext,
};
use apex_connectors::transport::Transport;
use apex_core::PlatformError;

use crate::registry::{VaultedCredentials, ADMOB_WIRE, LEVELPLAY_WIRE, MAX_WIRE};

/// The account-level context every snapshot builder reads: the developer site
/// and expected networks for the app-ads.txt check, the ad units LevelPlay
/// enumerates, and the bidder accounts copied into every snapshot. It is the
/// runner's half of `apex-api::live::LiveAccount`, so the snapshots the runner
/// writes and the console's live audit produces stay the same shape.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotAccount {
    /// The developer website hosting `app-ads.txt`.
    pub developer_site: String,
    /// Networks the publisher should authorize in app-ads.txt.
    pub expected_networks: Vec<String>,
    /// Ad unit ids the caller may enumerate for a per-unit waterfall read.
    /// The audit path uses LevelPlay's app-scoped instances read, so it does
    /// not need this list.
    pub ad_unit_ids: Vec<String>,
    /// Bidder accounts as apex knows them.
    pub bidder_accounts: Vec<apex_audit::BidderAccount>,
}

impl SnapshotAccount {
    /// The publisher context shared by every builder. Reporting data is added
    /// per platform by each builder before the snapshot is assembled.
    fn context(&self) -> PublisherContext {
        PublisherContext {
            developer_site: Some(self.developer_site.clone()),
            expected_networks: self.expected_networks.clone(),
            bidder_accounts: self.bidder_accounts.clone(),
            ..PublisherContext::default()
        }
    }
}

impl From<&crate::registry::RunnablePublisher> for SnapshotAccount {
    fn from(publisher: &crate::registry::RunnablePublisher) -> Self {
        Self {
            developer_site: publisher.developer_site.clone(),
            expected_networks: publisher.expected_networks.clone(),
            ad_unit_ids: publisher.ad_unit_ids.clone(),
            bidder_accounts: publisher.bidder_accounts.clone(),
        }
    }
}

/// Build a MAX snapshot: ad units and app-ads.txt from the management key,
/// country revenue from the report key when present.
#[allow(clippy::too_many_arguments)]
pub async fn max_account_snapshot(
    api_key: &str,
    report_key: Option<&str>,
    management_base: &str,
    reporting_base: &str,
    transport: Arc<dyn Transport>,
    account: &SnapshotAccount,
    now_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    let mut context = account.context();
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
        // The platform total is the exact sum of the same country rows the
        // floor rules read: one reporting pull serves both.
        context.platform_totals_30d = Some(sum_country_revenue(&context.country_revenue)?);
        // The mediation-side total is an independent aggregate read (the same
        // reporting source without the country/ad-unit breakdown), so the
        // reporting-gap rule can compare matched 30-day windows.
        let (start, end) = last_30_complete_days(now_unix_ms);
        let (rev, impressions) = report.totals(&start, &end).await?;
        context.mediation_totals_30d = Some(RevenueTotals {
            revenue_30d: rev,
            impressions_30d: impressions,
        });
    }
    let management = MaxClient::new(api_key, management_base, transport.clone())?;
    let fetcher = AppAdsTxtFetcher::new(transport);
    max_snapshot(&management, &fetcher, &context, now_unix_ms).await
}

/// Build a LevelPlay snapshot: the per-unit instances from the management key
/// (the waterfall the audit reads) and the app-ads.txt check, exactly like the
/// console's live audit.
#[allow(clippy::too_many_arguments)]
pub async fn levelplay_account_snapshot(
    secret_key: &str,
    refresh_token: &str,
    app_key: &str,
    base_url: &str,
    transport: Arc<dyn Transport>,
    account: &SnapshotAccount,
    now_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    let mut context = account.context();
    let client = LevelPlayClient::new(
        secret_key,
        refresh_token,
        app_key,
        base_url,
        Arc::clone(&transport),
    )?;
    // The app-level 30-day revenue read is the same reporting call the monthly
    // pass bills from; the audit stores it as the platform total so monitoring
    // activation has real evidence. A failed read fails the refresh typed.
    let (start, end) = last_30_complete_days(now_unix_ms);
    let revenue_30d = client.app_revenue(&start, &end).await?;
    context.platform_totals_30d = Some(RevenueTotals {
        revenue_30d,
        impressions_30d: 0,
    });
    // Independent mediation-side evidence: the AB-test breakdown of the same
    // reporting window. A reporting gap between the app total and this
    // breakdown is exactly what the audit's reporting-gap rule reports.
    let ab_rows = client.ab_test_report(&start, &end).await?;
    let mut mediation_revenue: i64 = 0;
    for row in &ab_rows {
        mediation_revenue = mediation_revenue
            .checked_add(row.revenue)
            .ok_or_else(|| PlatformError::Validation("revenue total overflow".into()))?;
    }
    context.mediation_totals_30d = Some(RevenueTotals {
        revenue_30d: mediation_revenue,
        impressions_30d: 0,
    });
    let fetcher = AppAdsTxtFetcher::new(transport);
    // The instances endpoint is app-scoped: one read, not one per configured
    // unit (which would fetch and duplicate the same app rows N times).
    levelplay_app_snapshot(&client, &fetcher, &context, now_unix_ms).await
}

/// Build an AdMob snapshot: the mediation groups from the management key and
/// the app-ads.txt check, exactly like the console's live audit.
#[allow(clippy::too_many_arguments)]
pub async fn admob_account_snapshot(
    access_token: &str,
    account_id: &str,
    base_url: &str,
    transport: Arc<dyn Transport>,
    account: &SnapshotAccount,
    now_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    let mut context = account.context();
    let client = AdmobClient::new(access_token, account_id, base_url, Arc::clone(&transport))?;
    // AdMob has two independent revenue reports for the same window: the
    // account's own network report (platform side) and the mediation report
    // (mediation side). Both are read so the reporting-gap rule can fire; a
    // failed read fails the refresh typed.
    let (start, end) = last_30_complete_days(now_unix_ms);
    let platform_revenue = client.network_revenue(&start, &end).await?;
    context.platform_totals_30d = Some(RevenueTotals {
        revenue_30d: platform_revenue,
        impressions_30d: 0,
    });
    let mediation_revenue = client.mediation_revenue(&start, &end).await?;
    context.mediation_totals_30d = Some(RevenueTotals {
        revenue_30d: mediation_revenue,
        impressions_30d: 0,
    });
    let fetcher = AppAdsTxtFetcher::new(transport);
    admob_snapshot(&client, &fetcher, &context, now_unix_ms).await
}

/// Build the snapshot for one platform from its vaulted key set. Unknown
/// platforms are a typed refusal, never a silently empty snapshot.
#[allow(clippy::too_many_arguments)]
pub async fn account_snapshot(
    platform_wire: &str,
    credentials: &VaultedCredentials,
    account: &SnapshotAccount,
    transport: Arc<dyn Transport>,
    now_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    match platform_wire {
        MAX_WIRE => {
            let reporting_base = credentials
                .reporting_base_url
                .clone()
                .unwrap_or_else(|| MAX_REPORTING_BASE.to_string());
            max_account_snapshot(
                &credentials.api_key,
                credentials.report_key.as_deref(),
                &credentials.base_url,
                &reporting_base,
                transport,
                account,
                now_unix_ms,
            )
            .await
        }
        LEVELPLAY_WIRE => {
            let refresh_token = credentials
                .refresh_token
                .as_deref()
                .filter(|token| !token.trim().is_empty())
                .ok_or_else(|| {
                    PlatformError::Validation(
                        "the LevelPlay key set has no refresh token".to_string(),
                    )
                })?;
            levelplay_account_snapshot(
                &credentials.api_key,
                refresh_token,
                &credentials.account_id,
                &credentials.base_url,
                transport,
                account,
                now_unix_ms,
            )
            .await
        }
        ADMOB_WIRE => {
            admob_account_snapshot(
                &credentials.api_key,
                &credentials.account_id,
                &credentials.base_url,
                transport,
                account,
                now_unix_ms,
            )
            .await
        }
        other => Err(PlatformError::Validation(format!(
            "no snapshot builder for platform '{other}'"
        ))),
    }
}

/// The exact platform total from the MAX country rows: revenue and impressions
/// are checked sums, so one malformed overflow fails the refresh typed instead
/// of wrapping.
fn sum_country_revenue(rows: &[CountryRevenueRow]) -> Result<RevenueTotals, PlatformError> {
    let mut revenue_30d: i64 = 0;
    let mut impressions_30d: u64 = 0;
    for row in rows {
        revenue_30d = revenue_30d
            .checked_add(row.revenue_30d)
            .ok_or_else(|| PlatformError::Validation("revenue total overflow".into()))?;
        impressions_30d = impressions_30d
            .checked_add(row.impressions_30d)
            .ok_or_else(|| PlatformError::Validation("impressions total overflow".into()))?;
    }
    Ok(RevenueTotals {
        revenue_30d,
        impressions_30d,
    })
}

/// The last 30 complete days, inclusive, in the MAX reporting convention: the
/// day before `now` is the last complete day, and the window starts 30 days
/// before today.
fn last_30_complete_days(now_unix_ms: i64) -> (String, String) {
    let today = now_unix_ms.div_euclid(86_400_000);
    (
        crate::calendar::format_day(today - 30),
        crate::calendar::format_day(today - 1),
    )
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
    let account = SnapshotAccount {
        developer_site: developer_site.to_string(),
        expected_networks,
        ..SnapshotAccount::default()
    };
    let snapshot = audited_platform_snapshot(
        MAX_WIRE,
        &VaultedCredentials {
            api_key: api_key.to_string(),
            account_id: String::new(),
            base_url: management_base.to_string(),
            report_key: report_key.map(str::to_string),
            reporting_base_url: Some(reporting_base.to_string()),
            refresh_token: None,
        },
        &account,
        transport,
        events,
        rule_version,
        now_unix_ms,
    )
    .await?;
    Ok(snapshot)
}

/// Build one platform's snapshot for the audit job and record the refresh
/// outcome: a failure emits the bounded `audit_failed` event (the snapshot
/// refresh error path) before the error is returned. The event is emitted only
/// when an emitter is attached.
#[allow(clippy::too_many_arguments)]
pub async fn audited_platform_snapshot(
    platform_wire: &str,
    credentials: &VaultedCredentials,
    account: &SnapshotAccount,
    transport: Arc<dyn Transport>,
    events: Option<&crate::events::RunnerEvents>,
    rule_version: &str,
    now_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    match account_snapshot(platform_wire, credentials, account, transport, now_unix_ms).await {
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

/// Per-process sequence for snapshot temp names, so two writers (or two
/// threads) never share a temp path.
static SNAPSHOT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Write a snapshot atomically (temp file then rename) so a reader never sees
/// a half-written snapshot. The temp name is unique per process and write, so
/// concurrent audits of the same publisher cannot rename each other's partial
/// file. Snapshots are derived data: the rename gives readers atomic
/// visibility, while a crash before the rename simply leaves the new snapshot
/// for the next audit to rebuild.
pub fn write_snapshot(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("snapshot directory: {error}"))?;
    }
    let bytes =
        serde_json::to_vec_pretty(snapshot).map_err(|error| format!("serialize: {error}"))?;
    let name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("snapshot.json");
    let sequence = SNAPSHOT_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!(".{name}.{}.{sequence}.tmp", std::process::id()));
    if let Err(error) = std::fs::write(&tmp, &bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("write: {error}"));
    }
    std::fs::rename(&tmp, path).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename: {error}")
    })?;
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

    #[test]
    fn the_max_platform_total_is_a_checked_sum() {
        let rows = vec![
            CountryRevenueRow {
                country: "EE".into(),
                ad_unit_id: "u1".into(),
                revenue_30d: i64::MAX,
                impressions_30d: u64::MAX,
            },
            CountryRevenueRow {
                country: "US".into(),
                ad_unit_id: "u1".into(),
                revenue_30d: 1,
                impressions_30d: 1,
            },
        ];
        assert!(matches!(
            sum_country_revenue(&rows),
            Err(PlatformError::Validation(_))
        ));
        assert_eq!(sum_country_revenue(&[]).expect("zero").revenue_30d, 0);
    }

    #[tokio::test]
    async fn the_audit_job_builds_a_snapshot_with_country_revenue() {
        let fake = Arc::new(Fake::default());
        // Reporting pull first, then the management list and app-ads.txt.
        fake.push(
            200,
            r#"{"code":200,"results":[
                {"day":"2026-10-01","country":"EE","max_ad_unit_id":"u1","estimated_revenue":"12.5","impressions":"100"},
                {"day":"2026-10-02","country":"US","max_ad_unit_id":"u1","estimated_revenue":"2.5","impressions":"50"}
            ]}"#,
        );
        // Independent mediation-side aggregate over the same window.
        fake.push(
            200,
            r#"{"code":200,"results":[
                {"day":"2026-10-01","estimated_revenue":"10.0","impressions":"100"},
                {"day":"2026-10-02","estimated_revenue":"2.0","impressions":"50"}
            ]}"#,
        );
        fake.push(
            200,
            r#"[{"id":"u1","name":"Rewarded","ad_format":"REWARD","platform":"android","package_name":"com.x"}]"#,
        );
        fake.push(200, "google.com, pub-1, DIRECT\n");
        let account = SnapshotAccount {
            developer_site: "studio.example.com".to_string(),
            expected_networks: vec!["google.com".to_string()],
            ..SnapshotAccount::default()
        };
        let snapshot = max_account_snapshot(
            "key",
            Some("report"),
            "https://o.example.test/mediation/v1",
            "https://r.example.test",
            fake.clone(),
            &account,
            1_800_000_000_000,
        )
        .await
        .expect("snapshot");
        assert_eq!(snapshot.ad_units.len(), 1);
        assert_eq!(snapshot.country_revenue.len(), 2);
        assert_eq!(snapshot.country_revenue[0].revenue_30d, 12_500_000);
        // The platform total is the exact sum of the country rows the floor
        // rules read, not a placeholder.
        assert_eq!(snapshot.platform_totals_30d.revenue_30d, 15_000_000);
        assert_eq!(snapshot.platform_totals_30d.impressions_30d, 150);
        // The mediation-side total is a separate read and disagrees by
        // $30,000,000 micros (20%), so the reporting-gap rule fires.
        assert_eq!(snapshot.mediation_totals_30d.revenue_30d, 12_000_000);
        assert!(snapshot.mediation_totals_reported);
        let findings = apex_audit::rules::reporting_gap::check(&snapshot);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].rule, apex_audit::RuleId::ReportingGap);
    }

    #[tokio::test]
    async fn the_levelplay_audit_job_reads_the_app_revenue_into_the_platform_total() {
        let fake = Arc::new(Fake::default());
        // The app revenue call authenticates, then reads reporting; the
        // management instances and app-ads check follow (the token is cached).
        fake.push(200, r#""bearer-token""#);
        fake.push(
            200,
            r#"{"code":200,"results":[
                {"date":"2026-09-18","revenue":"600000.00"},
                {"date":"2026-10-17","revenue":"5000.25"}
            ]}"#,
        );
        fake.push(
            200,
            r#"{"code":200,"results":[
                {"date":"2026-09-18","abTest":"A","revenue":"500000.00","activeUsers":"1000"},
                {"date":"2026-10-17","abTest":"B","revenue":"5000.25","activeUsers":"20"}
            ]}"#,
        );
        fake.push(
            200,
            r#"{"groups":[{"groupId":"2001","abTest":"A","instances":[{"instanceId":"e1"}]}]}"#,
        );
        fake.push(
            200,
            r#"{"instances":[{"instanceId":"e1","adapterNetwork":"admob","instanceName":"ADM_1","position":1,"price":3.2}]}"#,
        );
        fake.push(200, "admob.com, pub-9, DIRECT\n");
        let account = SnapshotAccount {
            developer_site: "studio.example.com".to_string(),
            expected_networks: vec![],
            ..SnapshotAccount::default()
        };
        let snapshot = levelplay_account_snapshot(
            "secret",
            "refresh",
            "app-key",
            "https://lp.example.test",
            fake.clone(),
            &account,
            1_800_000_000_000,
        )
        .await
        .expect("snapshot");
        assert_eq!(snapshot.platform, apex_core::PlatformId::LevelPlay);
        assert_eq!(
            snapshot.waterfall.len(),
            1,
            "no unit list still reads instances"
        );
        assert_eq!(
            snapshot.platform_totals_30d.revenue_30d, 605_000_250_000,
            "$605,000.25 in micros"
        );
        assert_eq!(snapshot.platform_totals_30d.impressions_30d, 0);
        assert_eq!(
            snapshot.mediation_totals_30d.revenue_30d, 505_000_250_000,
            "the AB-breakdown mediation total is read independently"
        );
        assert!(snapshot.mediation_totals_reported);
        // $100,000 below the platform total is a 16.5% gap: Critical.
        let findings = apex_audit::rules::reporting_gap::check(&snapshot);
        assert_eq!(findings.len(), 1, "{findings:?}");
    }

    #[tokio::test]
    async fn the_levelplay_audit_job_reads_the_app_instances_once_for_many_units() {
        // The instances endpoint is app-scoped; configured unit ids must not
        // fan out into one identical read per unit (and duplicate waterfall
        // rows / instance-price proposals).
        let fake = Arc::new(Fake::default());
        fake.push(200, r#""bearer-token""#);
        fake.push(
            200,
            r#"{"code":200,"results":[{"date":"2026-10-17","revenue":"1.0"}]}"#,
        );
        fake.push(
            200,
            r#"{"code":200,"results":[{"date":"2026-10-17","abTest":"A","revenue":"1.0","activeUsers":"10"}]}"#,
        );
        fake.push(
            200,
            r#"{"groups":[{"groupId":"2001","abTest":"A","instances":[{"instanceId":"e1"}]}]}"#,
        );
        fake.push(
            200,
            r#"{"instances":[{"instanceId":"e1","adapterNetwork":"admob","instanceName":"ADM_1","position":1,"price":3.2}]}"#,
        );
        fake.push(200, "admob.com, pub-9, DIRECT\n");
        let account = SnapshotAccount {
            developer_site: "studio.example.com".to_string(),
            ad_unit_ids: vec!["u1".to_string(), "u2".to_string(), "u3".to_string()],
            ..SnapshotAccount::default()
        };
        let snapshot = levelplay_account_snapshot(
            "secret",
            "refresh",
            "app-key",
            "https://lp.example.test",
            fake.clone(),
            &account,
            1_800_000_000_000,
        )
        .await
        .expect("snapshot");
        assert_eq!(snapshot.waterfall.len(), 1, "one app read, one row");
        let seen = fake.seen.lock().unwrap();
        assert_eq!(
            seen.iter()
                .filter(|request| request.operation == "listWaterfall")
                .count(),
            1,
            "one app-scoped instances read regardless of configured units"
        );
        assert_eq!(
            seen.len(),
            6,
            "auth, revenue, AB total, groups, instances and app-ads"
        );
    }

    #[tokio::test]
    async fn the_admob_audit_job_reads_the_mediation_report_into_the_platform_total() {
        let fake = Arc::new(Fake::default());
        // The network report is the platform side; the mediation report is
        // the independent mediation side. The groups and app-ads check follow.
        fake.push(
            200,
            r#"{"header":{"currencyCode":"USD"},"rows":[
                {"dimensionValues":{"DATE":{"value":"2026-09-18"}},"metricValues":{"ESTIMATED_EARNINGS":{"microsValue":"110000000000"}}}
            ]}"#,
        );
        fake.push(
            200,
            r#"{"header":{"currencyCode":"USD"},"rows":[
                {"dimensionValues":{"DATE":{"value":"2026-09-18"}},"metricValues":{"ESTIMATED_EARNINGS":{"microsValue":"100000000000"}}}
            ]}"#,
        );
        fake.push(
            200,
            r#"{"mediationGroups":[{"mediationGroupId":"g1","name":"Main","format":"interstitial","adUnitMappings":[{"adUnitId":"u1","network":"admob","ecpm":2}]}]}"#,
        );
        fake.push(200, "google.com, pub-1, DIRECT\n");
        let account = SnapshotAccount {
            developer_site: "studio.example.com".to_string(),
            expected_networks: vec!["google.com".to_string()],
            ..SnapshotAccount::default()
        };
        let snapshot = admob_account_snapshot(
            "token",
            "pub-1234567890",
            "https://admob.example.test",
            fake.clone(),
            &account,
            1_800_000_000_000,
        )
        .await
        .expect("snapshot");
        assert_eq!(snapshot.platform, apex_core::PlatformId::Admob);
        assert_eq!(snapshot.mediation_groups.len(), 1);
        assert_eq!(
            snapshot.platform_totals_30d.revenue_30d, 110_000_000_000,
            "$110,000.00 network report in micros"
        );
        assert_eq!(
            snapshot.mediation_totals_30d.revenue_30d, 100_000_000_000,
            "$100,000.00 mediation report in micros"
        );
        assert!(snapshot.mediation_totals_reported);
        // 9.09% is below the 10% critical boundary but above the 2% warning
        // boundary: the rule fires.
        let findings = apex_audit::rules::reporting_gap::check(&snapshot);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].severity, apex_audit::Severity::Warning);
    }

    #[tokio::test]
    async fn a_failed_revenue_read_is_a_typed_audit_failure() {
        // The mediation report is refused: the snapshot refresh (and therefore
        // the audit) fails typed instead of recording a zero total.
        let fake = Arc::new(Fake::default());
        fake.push(500, "report unavailable");
        let account = SnapshotAccount {
            developer_site: "studio.example.com".to_string(),
            expected_networks: vec![],
            ..SnapshotAccount::default()
        };
        let error = admob_account_snapshot(
            "token",
            "pub-1234567890",
            "https://admob.example.test",
            fake,
            &account,
            1_800_000_000_000,
        )
        .await
        .expect_err("the revenue read fails the refresh");
        match error {
            PlatformError::Http { status, .. } => assert_eq!(status, 500),
            other => panic!("expected a typed HTTP refusal, got {other:?}"),
        }
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
            mediation_totals_reported: false,
        };
        write_snapshot(&path, &snapshot).expect("write");
        let back: Snapshot = serde_json::from_slice(&std::fs::read(&path).unwrap()).expect("read");
        assert_eq!(back.fetched_at_unix_ms, 3);
        let entries: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec!["applovin_max.json".to_string()],
            "the unique temp file is renamed away, never left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
