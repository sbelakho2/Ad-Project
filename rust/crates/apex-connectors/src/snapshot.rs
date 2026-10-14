//! Live audit snapshot assembly for the free-audit flow.
//!
//! The audit engine reads one [`Snapshot`] per platform and performs no I/O of
//! its own. This module is the I/O side: it calls the platform management reads
//! and the app-ads.txt fetcher, then assembles the snapshot in one place so the
//! API layer stays thin.
//!
//! Reporting data (country revenue and the two 30-day totals) and the bidder
//! accounts come from apex itself, not from the platform management APIs, so the
//! caller passes them in [`PublisherContext`]. Until the reporting reads land,
//! callers pass the defaults, and the revenue rules simply find nothing to
//! flag. An app-ads.txt check is part of every audit, so a developer site is
//! required and is validated before any request goes out.

use apex_audit::{AppAdsTxt, BidderAccount, CountryRevenueRow, RevenueTotals, Snapshot};
use apex_core::{AdUnit, MediationGroup, PlatformError, PlatformId, WaterfallEntry};

use crate::admob::AdmobClient;
use crate::app_ads::AppAdsTxtFetcher;
use crate::levelplay::LevelPlayClient;
use crate::max::MaxClient;

/// Totals for a platform that has no reporting data wired yet.
pub const ZERO_TOTALS: RevenueTotals = RevenueTotals {
    revenue_30d: 0,
    impressions_30d: 0,
};

/// Everything the audit needs that the platform management APIs do not supply.
#[derive(Debug, Clone, Default)]
pub struct PublisherContext {
    /// Networks the publisher should authorize in app-ads.txt.
    pub expected_networks: Vec<String>,
    /// The app developer website hosting `app-ads.txt`. Required.
    pub developer_site: Option<String>,
    /// Bidder accounts as apex knows them, including platform enablement.
    pub bidder_accounts: Vec<BidderAccount>,
    /// 30-day revenue rows for the floor rules.
    pub country_revenue: Vec<CountryRevenueRow>,
    /// Median 30-day platform totals for the reporting-gap rule.
    pub platform_totals_30d: Option<RevenueTotals>,
    /// Median 30-day mediation totals for the reporting-gap rule.
    pub mediation_totals_30d: Option<RevenueTotals>,
}

/// Assemble the AppLovin MAX snapshot: ad units from the management API, the
/// app-ads.txt check, and the caller-supplied reporting context.
pub async fn max_snapshot(
    client: &MaxClient,
    fetcher: &AppAdsTxtFetcher,
    context: &PublisherContext,
    fetched_at_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    let site = require_developer_site(PlatformId::AppLovinMax, context)?;
    let ad_units: Vec<AdUnit> = client.list_ad_units().await?;
    let app_ads_txt = fetcher
        .fetch(
            PlatformId::AppLovinMax,
            site,
            context.expected_networks.clone(),
        )
        .await?;
    Ok(finish(
        PlatformId::AppLovinMax,
        fetched_at_unix_ms,
        SnapshotReads {
            ad_units,
            ..SnapshotReads::default()
        },
        context,
        app_ads_txt,
    ))
}

/// Assemble the LevelPlay snapshot: the per-unit waterfalls from the management
/// API, the app-ads.txt check, and the caller-supplied reporting context.
///
/// LevelPlay exposes the waterfall per ad unit, so the caller enumerates the
/// units to audit. An empty list audits no units and yields an empty waterfall.
pub async fn levelplay_snapshot(
    client: &LevelPlayClient,
    fetcher: &AppAdsTxtFetcher,
    context: &PublisherContext,
    ad_unit_ids: &[String],
    fetched_at_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    let site = require_developer_site(PlatformId::LevelPlay, context)?;
    let mut waterfall: Vec<WaterfallEntry> = Vec::new();
    for ad_unit_id in ad_unit_ids {
        waterfall.extend(client.list_waterfall(ad_unit_id).await?);
    }
    let app_ads_txt = fetcher
        .fetch(
            PlatformId::LevelPlay,
            site,
            context.expected_networks.clone(),
        )
        .await?;
    Ok(finish(
        PlatformId::LevelPlay,
        fetched_at_unix_ms,
        SnapshotReads {
            waterfall,
            ..SnapshotReads::default()
        },
        context,
        app_ads_txt,
    ))
}

/// Assemble the AdMob snapshot: mediation groups from the management API, the
/// app-ads.txt check, and the caller-supplied reporting context.
pub async fn admob_snapshot(
    client: &AdmobClient,
    fetcher: &AppAdsTxtFetcher,
    context: &PublisherContext,
    fetched_at_unix_ms: i64,
) -> Result<Snapshot, PlatformError> {
    let site = require_developer_site(PlatformId::Admob, context)?;
    let mediation_groups: Vec<MediationGroup> = client.list_mediation_groups().await?;
    let app_ads_txt = fetcher
        .fetch(PlatformId::Admob, site, context.expected_networks.clone())
        .await?;
    Ok(finish(
        PlatformId::Admob,
        fetched_at_unix_ms,
        SnapshotReads {
            mediation_groups,
            ..SnapshotReads::default()
        },
        context,
        app_ads_txt,
    ))
}

#[derive(Debug, Default)]
struct SnapshotReads {
    ad_units: Vec<AdUnit>,
    waterfall: Vec<WaterfallEntry>,
    mediation_groups: Vec<MediationGroup>,
}

fn finish(
    platform: PlatformId,
    fetched_at_unix_ms: i64,
    reads: SnapshotReads,
    context: &PublisherContext,
    app_ads_txt: AppAdsTxt,
) -> Snapshot {
    Snapshot {
        platform,
        fetched_at_unix_ms,
        ad_units: reads.ad_units,
        waterfall: reads.waterfall,
        mediation_groups: reads.mediation_groups,
        bidder_accounts: context.bidder_accounts.clone(),
        country_revenue: context.country_revenue.clone(),
        app_ads_txt,
        platform_totals_30d: context.platform_totals_30d.unwrap_or(ZERO_TOTALS),
        mediation_totals_30d: context.mediation_totals_30d.unwrap_or(ZERO_TOTALS),
    }
}

fn require_developer_site(
    platform: PlatformId,
    context: &PublisherContext,
) -> Result<&str, PlatformError> {
    context
        .developer_site
        .as_deref()
        .map(str::trim)
        .filter(|site| !site.is_empty())
        .ok_or_else(|| {
            PlatformError::Validation(format!(
                "{platform} snapshot: a developer site is required for the app-ads.txt check"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_ads::AppAdsTxtFetcher;
    use crate::transport::{HttpRequest, HttpResponse, Transport, TransportError};
    use apex_audit::BidderAccount;
    use std::sync::{Arc, Mutex};

    struct ScriptedTransport {
        script: Mutex<std::collections::VecDeque<Result<HttpResponse, TransportError>>>,
        calls: Mutex<Vec<HttpRequest>>,
    }

    impl ScriptedTransport {
        fn new(items: Vec<Result<HttpResponse, TransportError>>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(items.into()),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> Vec<HttpRequest> {
            self.calls.lock().map(|c| c.clone()).unwrap_or_default()
        }
    }

    #[async_trait::async_trait]
    impl Transport for ScriptedTransport {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.calls
                .lock()
                .map_err(|_| TransportError::Network("poisoned".into()))?
                .push(req);
            self.script
                .lock()
                .map_err(|_| TransportError::Network("poisoned".into()))?
                .pop_front()
                .unwrap_or(Err(TransportError::Network("script exhausted".into())))
        }
    }

    fn ok(body: &str) -> Result<HttpResponse, TransportError> {
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: body.as_bytes().to_vec(),
        })
    }

    fn context() -> PublisherContext {
        PublisherContext {
            expected_networks: vec!["google.com".to_string()],
            developer_site: Some("studio.example.com".to_string()),
            bidder_accounts: vec![BidderAccount {
                network: "meta".to_string(),
                has_apex_managed_account: true,
                enabled_in_platform: false,
            }],
            country_revenue: vec![CountryRevenueRow {
                country: "US".to_string(),
                ad_unit_id: "u1".to_string(),
                revenue_30d: 1_000_000,
                impressions_30d: 500,
            }],
            platform_totals_30d: Some(RevenueTotals {
                revenue_30d: 10_000_000,
                impressions_30d: 5_000,
            }),
            mediation_totals_30d: None,
        }
    }

    #[tokio::test]
    async fn max_snapshot_assembles_platform_reads_and_app_ads() {
        let transport = ScriptedTransport::new(vec![
            ok(
                r#"{"ad_units":[{"ad_unit_id":"u1","name":"Banner","format":"banner","enabled":true}]}"#,
            ),
            ok("google.com, pub-1, DIRECT\n"),
        ]);
        let client = MaxClient::new("key", "acct", "https://api.example.test", transport.clone())
            .expect("client");
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let snapshot = max_snapshot(&client, &fetcher, &context(), 1_700_000_000_000)
            .await
            .expect("snapshot");

        assert_eq!(snapshot.platform, PlatformId::AppLovinMax);
        assert_eq!(snapshot.fetched_at_unix_ms, 1_700_000_000_000);
        assert_eq!(snapshot.ad_units.len(), 1);
        assert!(snapshot.waterfall.is_empty());
        assert!(snapshot.mediation_groups.is_empty());
        assert_eq!(snapshot.bidder_accounts.len(), 1);
        assert_eq!(snapshot.country_revenue.len(), 1);
        assert!(snapshot.app_ads_txt.present);
        assert_eq!(snapshot.app_ads_txt.lines.len(), 1);
        assert_eq!(snapshot.platform_totals_30d.revenue_30d, 10_000_000);
        assert_eq!(snapshot.mediation_totals_30d, ZERO_TOTALS);
        let calls = transport.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].url, "https://studio.example.com/app-ads.txt");
    }

    #[tokio::test]
    async fn levelplay_snapshot_fills_the_waterfall() {
        let transport = ScriptedTransport::new(vec![
            ok(
                r#"{"waterfall":[{"entry_id":"e1","network":"admob","instance_name":"ADM_1","position":1,"price_ecpm":3.2}]}"#,
            ),
            ok("admob.com, pub-9, DIRECT\n"),
        ]);
        let client = LevelPlayClient::new(
            "key",
            "app-key",
            "https://api.example.test",
            transport.clone(),
        )
        .expect("client");
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let snapshot = levelplay_snapshot(&client, &fetcher, &context(), &["u1".to_string()], 7)
            .await
            .expect("snapshot");
        assert_eq!(snapshot.platform, PlatformId::LevelPlay);
        assert_eq!(snapshot.waterfall.len(), 1);
        assert!(snapshot.ad_units.is_empty());
        assert_eq!(snapshot.waterfall[0].network, "admob");
    }

    #[tokio::test]
    async fn admob_snapshot_fills_mediation_groups() {
        let transport = ScriptedTransport::new(vec![
            ok(
                r#"{"mediationGroups":[{"mediationGroupId":"g1","name":"Main","format":"interstitial","adUnitMappings":[{"adUnitId":"u1","network":"admob","ecpm":2}]}]}"#,
            ),
            ok("google.com, pub-1, DIRECT\n"),
        ]);
        let client = AdmobClient::new(
            "key",
            "accounts/pub-1234567890",
            "https://api.example.test",
            transport.clone(),
        )
        .expect("client");
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let snapshot = admob_snapshot(&client, &fetcher, &context(), 9)
            .await
            .expect("snapshot");
        assert_eq!(snapshot.platform, PlatformId::Admob);
        assert_eq!(snapshot.mediation_groups.len(), 1);
        assert!(snapshot.waterfall.is_empty());
    }

    #[tokio::test]
    async fn every_snapshot_requires_a_developer_site_before_any_request() {
        let mut ctx = context();
        ctx.developer_site = None;
        let transport = ScriptedTransport::new(vec![]);
        let client = MaxClient::new("key", "acct", "https://api.example.test", transport.clone())
            .expect("client");
        let fetcher = AppAdsTxtFetcher::new(transport.clone());
        let err = max_snapshot(&client, &fetcher, &ctx, 1)
            .await
            .expect_err("must fail");
        assert!(matches!(err, PlatformError::Validation(_)), "got {err}");
        assert!(transport.calls().is_empty(), "no request may go out");
    }
}
