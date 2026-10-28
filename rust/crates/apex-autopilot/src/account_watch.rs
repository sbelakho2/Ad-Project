//! A3 M1: the persistent account-metric and app-ads alert lifecycle.
//!
//! The first completed observation is the baseline. Later observations are
//! compared with it:
//!
//! - an account metric (eCPM, revenue, impressions) below
//!   `baseline * (1 - threshold)` raises [`Alert::AccountMetricDrop`];
//! - canonical app-ads.txt identities that disappeared raise
//!   [`Alert::AppAdsLinesLost`].
//!
//! State is written atomically and fsynced. Delivery is fail-closed: if the
//! sink refuses an alert the state is not advanced, so the next run retries
//! the same alert instead of losing it. A baseline older than 28 days is
//! refreshed from the current observation; before that it stays fixed, which
//! is the 28-day baseline early drift and rechecks are measured against.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::alerts::{app_ads_line_identities_lost, metric_dropped, Alert, AlertSink};

/// The default drop threshold: 20% below the baseline.
pub const DEFAULT_DROP_THRESHOLD: f64 = 0.20;

/// How long a baseline stays fixed before it is refreshed.
pub const BASELINE_WINDOW_MS: i64 = 28 * 86_400_000;

/// One unit's baseline and latest observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitObservation {
    pub revenue_30d: i64,
    pub impressions_30d: u64,
    /// Revenue per 1000 impressions, when impressions were reported.
    pub ecpm: Option<f64>,
}

impl UnitObservation {
    fn from_totals(revenue_30d: i64, impressions_30d: u64) -> Self {
        let ecpm =
            (impressions_30d > 0).then(|| revenue_30d as f64 * 1000.0 / impressions_30d as f64);
        Self {
            revenue_30d,
            impressions_30d,
            ecpm,
        }
    }
}

/// The persisted baseline for one publisher.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountWatchState {
    pub publisher: String,
    /// When the baseline was captured (Unix ms). Zero means "not captured".
    pub baseline_at_unix_ms: i64,
    /// When the latest observation was captured.
    pub checked_at_unix_ms: i64,
    /// Baseline per `platform|ad_unit_id`.
    pub units: BTreeMap<String, UnitObservation>,
    /// Latest observation per `platform|ad_unit_id`.
    pub latest: BTreeMap<String, UnitObservation>,
    /// Canonical app-ads.txt line identities at the baseline.
    pub app_ads_lines: Vec<String>,
}

/// A configured account watch: one state file and one drop threshold.
#[derive(Debug, Clone)]
pub struct AccountWatch {
    path: PathBuf,
    threshold: f64,
}

impl AccountWatch {
    /// Build the watch. A threshold is a fraction of the baseline and must be
    /// in `(0, 1]`; a value above 1.0 (or a non-finite one) is refused rather
    /// than silently disabling or over-triggering the check.
    pub fn new(path: impl Into<PathBuf>, threshold: f64) -> Result<Self, String> {
        if !threshold.is_finite() || threshold <= 0.0 || threshold > 1.0 {
            return Err(format!(
                "account-watch threshold must be in (0.0, 1.0]; got {threshold}"
            ));
        }
        Ok(Self {
            path: path.into(),
            threshold,
        })
    }

    /// The state file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The configured threshold.
    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    fn load(&self) -> Result<Option<AccountWatchState>, String> {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "{} could not be read: {error}",
                    self.path.display()
                ))
            }
        };
        serde_json::from_str(&raw).map(Some).map_err(|error| {
            format!(
                "{} is not valid account-watch state: {error}",
                self.path.display()
            )
        })
    }

    fn save(&self, state: &AccountWatchState) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    format!("{}: parent directory: {error}", self.path.display())
                })?;
            }
        }
        let json = serde_json::to_string_pretty(state)
            .map_err(|error| format!("account-watch state could not be serialized: {error}"))?;
        let tmp = self.path.with_extension("json.tmp");
        {
            use std::io::Write as _;
            let mut file = std::fs::File::create(&tmp)
                .map_err(|error| format!("{} could not be created: {error}", tmp.display()))?;
            file.write_all(json.as_bytes())
                .map_err(|error| format!("{} could not be written: {error}", tmp.display()))?;
            file.sync_all()
                .map_err(|error| format!("{} could not be fsynced: {error}", tmp.display()))?;
        }
        std::fs::rename(&tmp, &self.path).map_err(|error| {
            format!(
                "{} could not be renamed into place: {error}",
                self.path.display()
            )
        })
    }

    /// The authorization set to persist: only a present file carrying at
    /// least one authorization record replaces the last known-good set. An
    /// absent, empty or comment-only file must not erase the baseline that a
    /// later loss alert compares against.
    fn observed_app_ads_lines(snapshot: &apex_audit::Snapshot) -> Option<Vec<String>> {
        if !snapshot.app_ads_txt.present {
            return None;
        }
        let lines: Vec<String> = snapshot
            .app_ads_txt
            .lines
            .iter()
            .filter(|line| apex_audit::rules::app_ads_txt::is_authorization_line(line))
            .cloned()
            .collect();
        (!lines.is_empty()).then_some(lines)
    }

    /// Compare one snapshot with the persisted baseline, emit alerts, then
    /// advance the state. `Err` means an alert or the state write failed; the
    /// old state stays on disk so the next run retries.
    pub async fn observe(
        &self,
        publisher: &str,
        platform: &str,
        snapshot: &apex_audit::Snapshot,
        now_unix_ms: i64,
        sink: &dyn AlertSink,
    ) -> Result<Vec<Alert>, String> {
        let mut state = self.load()?.unwrap_or_else(|| AccountWatchState {
            publisher: publisher.to_string(),
            ..AccountWatchState::default()
        });
        if state.publisher.is_empty() {
            state.publisher = publisher.to_string();
        }
        let first = state.baseline_at_unix_ms == 0;
        let refresh =
            !first && now_unix_ms.saturating_sub(state.baseline_at_unix_ms) >= BASELINE_WINDOW_MS;

        let mut current: BTreeMap<String, UnitObservation> = BTreeMap::new();
        for row in &snapshot.country_revenue {
            let key = format!("{platform}|{}", row.ad_unit_id);
            let entry = current
                .entry(key)
                .or_insert_with(|| UnitObservation::from_totals(0, 0));
            entry.revenue_30d = entry.revenue_30d.saturating_add(row.revenue_30d);
            entry.impressions_30d = entry.impressions_30d.saturating_add(row.impressions_30d);
        }
        for observation in current.values_mut() {
            *observation =
                UnitObservation::from_totals(observation.revenue_30d, observation.impressions_30d);
        }

        let mut alerts = Vec::new();
        let mut delivery_failed: Option<String> = None;
        if !first && !refresh {
            for key in state.units.keys() {
                if !current.contains_key(key) {
                    let (_, ad_unit) = key.split_once('|').unwrap_or((platform, key.as_str()));
                    alerts.push(Alert::AccountUnitMissing {
                        publisher: publisher.to_string(),
                        ad_unit: ad_unit.to_string(),
                    });
                }
            }
            for (key, observed) in &current {
                let Some(baseline) = state.units.get(key) else {
                    continue;
                };
                for (metric, base, now) in [
                    ("ecpm", baseline.ecpm, observed.ecpm),
                    (
                        "revenue30d",
                        Some(baseline.revenue_30d as f64),
                        Some(observed.revenue_30d as f64),
                    ),
                    (
                        "impressions30d",
                        Some(baseline.impressions_30d as f64),
                        Some(observed.impressions_30d as f64),
                    ),
                ] {
                    let (Some(base), Some(now)) = (base, now) else {
                        continue;
                    };
                    if metric_dropped(base, now, self.threshold) {
                        let (platform_id, ad_unit) =
                            key.split_once('|').unwrap_or((platform, key.as_str()));
                        let alert = Alert::AccountMetricDrop {
                            publisher: publisher.to_string(),
                            ad_unit: ad_unit.to_string(),
                            metric: metric.to_string(),
                            observed: now,
                            baseline: base,
                        };
                        let _ = platform_id;
                        alerts.push(alert);
                    }
                }
            }
            // Preserve the last known-good authorization set when the file is
            // absent or empty. An empty present file loses every prior line
            // and must alert, not become the new baseline.
            if !state.app_ads_lines.is_empty() && snapshot.app_ads_txt.present {
                let lost =
                    app_ads_line_identities_lost(&state.app_ads_lines, &snapshot.app_ads_txt.lines);
                if lost > 0 {
                    alerts.push(Alert::AppAdsLinesLost {
                        publisher: publisher.to_string(),
                        domain: publisher.to_string(),
                        lost,
                    });
                }
            }
        }

        for alert in &alerts {
            if let Err(error) = sink.emit(alert.clone()).await {
                delivery_failed = Some(error.to_string());
                break;
            }
        }
        if let Some(error) = delivery_failed {
            return Err(format!(
                "account-watch alert delivery failed for {publisher}; state not advanced: {error}"
            ));
        }

        // The baseline is captured on the first observation and refreshed
        // every 28 days; the latest observation always advances after a
        // successful delivery.
        let observed_lines = Self::observed_app_ads_lines(snapshot);
        if first || refresh {
            state.units = current.clone();
            state.baseline_at_unix_ms = now_unix_ms;
            if let Some(lines) = observed_lines {
                state.app_ads_lines = lines;
            }
        } else {
            // A unit that first appears after the baseline gets its own
            // baseline now rather than being compared with nothing.
            for (key, observation) in &current {
                state
                    .units
                    .entry(key.clone())
                    .or_insert_with(|| observation.clone());
            }
            if let Some(lines) = observed_lines {
                state.app_ads_lines = lines;
            }
        }
        state.latest = current;
        state.checked_at_unix_ms = now_unix_ms;
        self.save(&state)?;
        Ok(alerts)
    }

    /// The eCPM drift of one unit from its fixed 28-day baseline:
    /// `(latest - baseline) / baseline`. `0.0` when the unit has no baseline
    /// or no measured eCPM yet.
    pub fn ecpm_drift(&self, platform: &str, ad_unit_id: &str) -> Result<f64, String> {
        let Some(state) = self.load()? else {
            return Ok(0.0);
        };
        let key = format!("{platform}|{ad_unit_id}");
        let (Some(baseline), Some(latest)) = (state.units.get(&key), state.latest.get(&key)) else {
            return Ok(0.0);
        };
        let (Some(base), Some(now)) = (baseline.ecpm, latest.ecpm) else {
            return Ok(0.0);
        };
        if base <= 0.0 || !base.is_finite() || !now.is_finite() {
            return Ok(0.0);
        }
        Ok((now - base) / base)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::alerts::RecordingAlertSink;
    use apex_audit::{AppAdsTxt, CountryRevenueRow, RevenueTotals, Snapshot};
    use apex_core::PlatformId;

    fn snapshot(unit: &str, revenue: i64, impressions: u64, lines: &[&str]) -> Snapshot {
        Snapshot {
            platform: PlatformId::AppLovinMax,
            fetched_at_unix_ms: 0,
            ad_units: Vec::new(),
            waterfall: Vec::new(),
            mediation_groups: Vec::new(),
            bidder_accounts: Vec::new(),
            country_revenue: vec![CountryRevenueRow {
                country: "US".into(),
                ad_unit_id: unit.into(),
                revenue_30d: revenue,
                impressions_30d: impressions,
            }],
            app_ads_txt: AppAdsTxt {
                present: true,
                lines: lines.iter().map(|line| (*line).to_string()).collect(),
                expected_networks: Vec::new(),
            },
            platform_totals_30d: RevenueTotals {
                revenue_30d: revenue,
                impressions_30d: impressions,
            },
            mediation_totals_30d: RevenueTotals {
                revenue_30d: revenue,
                impressions_30d: impressions,
            },
            mediation_totals_reported: true,
        }
    }

    fn watch(name: &str) -> (AccountWatch, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("apex-account-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (AccountWatch::new(dir.join("watch.json"), 0.2).unwrap(), dir)
    }

    #[tokio::test]
    async fn baseline_then_a_second_low_observation_alerts_and_persists() {
        let (watch, dir) = watch("drop");
        let sink = RecordingAlertSink::new();
        // $5 eCPM baseline.
        let first = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 5_000_000, 1_000, &["a,b,DIRECT"]),
                1_000,
                &sink,
            )
            .await
            .expect("baseline");
        assert!(first.is_empty(), "the first observation is the baseline");
        assert!(dir.join("watch.json").exists());
        // $3 eCPM is 40% down: alert.
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 3_000_000, 1_000, &["a,b,DIRECT"]),
                2_000,
                &sink,
            )
            .await
            .expect("compare");
        assert!(alerts
            .iter()
            .any(|a| matches!(a, Alert::AccountMetricDrop { metric, .. } if metric == "ecpm")));
        assert!(!sink.alerts().is_empty());
        assert!((watch.ecpm_drift("applovin_max", "u1").unwrap() + 0.4).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_unit_that_disappears_raises_missing_evidence_not_a_drop() {
        let (watch, dir) = watch("unit-missing");
        let sink = RecordingAlertSink::new();
        watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 5_000_000, 1_000, &["a,b,DIRECT"]),
                1_000,
                &sink,
            )
            .await
            .expect("baseline");
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u2", 5_000_000, 1_000, &["a,b,DIRECT"]),
                2_000,
                &sink,
            )
            .await
            .expect("compare");
        assert!(
            alerts.iter().any(
                |alert| matches!(alert, Alert::AccountUnitMissing { ad_unit, .. } if ad_unit == "u1")
            ),
            "{alerts:?}"
        );
        assert!(
            !alerts
                .iter()
                .any(|alert| matches!(alert, Alert::AccountMetricDrop { .. })),
            "absence is not a 100% drop"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn absent_and_empty_app_ads_preserve_the_last_known_good_lines() {
        let (watch, dir) = watch("ads-missing");
        let sink = RecordingAlertSink::new();
        let baseline = ["a,1,DIRECT", "b,2,DIRECT", "c,3,DIRECT"];
        watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 5_000_000, 1_000, &baseline),
                1_000,
                &sink,
            )
            .await
            .expect("baseline");

        let mut absent = snapshot("u1", 5_000_000, 1_000, &[]);
        absent.app_ads_txt.present = false;
        let alerts = watch
            .observe("northstar", "applovin_max", &absent, 2_000, &sink)
            .await
            .expect("absent");
        assert!(
            !alerts
                .iter()
                .any(|alert| matches!(alert, Alert::AppAdsLinesLost { .. })),
            "absent is the daily critical, not a line-loss alert"
        );
        let state = watch.load().expect("load").expect("state");
        assert_eq!(state.app_ads_lines, baseline);

        // A present but empty file loses every recorded authorization; the
        // alert names all three and the baseline is not replaced by emptiness.
        let empty = snapshot("u1", 5_000_000, 1_000, &[]);
        let alerts = watch
            .observe("northstar", "applovin_max", &empty, 3_000, &sink)
            .await
            .expect("empty");
        assert!(alerts
            .iter()
            .any(|alert| matches!(alert, Alert::AppAdsLinesLost { lost: 3, .. })));
        let state = watch.load().expect("load").expect("state");
        assert_eq!(state.app_ads_lines, baseline);

        // Restoring the same lines is clean and still keeps the baseline.
        let restored = snapshot("u1", 5_000_000, 1_000, &baseline);
        let alerts = watch
            .observe("northstar", "applovin_max", &restored, 4_000, &sink)
            .await
            .expect("restored");
        assert!(!alerts
            .iter()
            .any(|alert| matches!(alert, Alert::AppAdsLinesLost { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn app_ads_identity_loss_alerts_even_when_the_count_is_unchanged() {
        let (watch, dir) = watch("ads");
        let sink = RecordingAlertSink::new();
        let baseline = ["a,1,DIRECT", "b,2,DIRECT", "c,3,DIRECT"];
        watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 5_000_000, 1_000, &baseline),
                1_000,
                &sink,
            )
            .await
            .expect("baseline");
        let changed = ["a,1,DIRECT", "b,2,DIRECT", "d,4,DIRECT"];
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 5_000_000, 1_000, &changed),
                2_000,
                &sink,
            )
            .await
            .expect("compare");
        assert_eq!(
            alerts
                .iter()
                .filter(|a| matches!(a, Alert::AppAdsLinesLost { lost: 1, .. }))
                .count(),
            1,
            "{alerts:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delivery_failure_does_not_advance_the_state() {
        let (watch, dir) = watch("failclosed");
        let sink = RecordingAlertSink::new();
        watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 5_000_000, 1_000, &["a,1,DIRECT"]),
                1_000,
                &sink,
            )
            .await
            .unwrap();
        let before = std::fs::read_to_string(dir.join("watch.json")).unwrap();
        #[derive(Debug)]
        struct Failing;
        #[async_trait::async_trait]
        impl AlertSink for Failing {
            async fn emit(&self, _alert: Alert) -> Result<(), crate::alerts::AlertError> {
                Err(crate::alerts::AlertError("disk full".into()))
            }
        }
        let error = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 1_000_000, 1_000, &["a,1,DIRECT"]),
                2_000,
                &Failing,
            )
            .await
            .expect_err("delivery fails closed");
        assert!(error.contains("state not advanced"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dir.join("watch.json")).unwrap(),
            before
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thresholds_above_one_are_rejected() {
        assert!(AccountWatch::new("x", 1.0).is_ok());
        assert!(AccountWatch::new("x", 1.01).is_err());
        assert!(AccountWatch::new("x", f64::NAN).is_err());
        assert!(AccountWatch::new("x", 0.0).is_err());
    }
}
