//! A3 M1: the persistent account-metric and app-ads alert lifecycle.
//!
//! The free monitoring compares every day with the same weekday's trailing
//! 28-day median per ad unit, so a quiet weekend does not read as a crash
//! and a bad Tuesday is caught the day it happens:
//!
//! - each daily pull records the unit's OWN rows for the last complete
//!   reporting day (HH); the history stores those daily values, so a unit
//!   that stops earning is compared on its first bad day — never as the
//!   difference of two rolling 30-day totals, whose window slide turns a
//!   stop into a negative (restatement) difference that used to be skipped;
//! - the day's revenue, impressions (the filled volume — the platform
//!   reports carry no request counts, so a fill drop with steady traffic
//!   lands here) and eCPM (the per-audience monetization — no active-user
//!   counts are reported either) are compared with the median of the same
//!   weekday's days inside the trailing 28-day window; below
//!   `median * (1 - threshold)` raises [`Alert::AccountMetricDrop`];
//! - canonical app-ads.txt identities that disappeared raise
//!   [`Alert::AppAdsLinesLost`]; a file that is completely absent loses every
//!   recorded identity and alerts with the full baseline count, so the
//!   disappearance of the authorization source itself is never silent.
//!
//! The first completed observation seeds the 28-day per-unit baseline that
//! [`AccountWatch::ecpm_drift`] and the missing-unit check read; drop alerts
//! start once a same-weekday prior exists (one week of daily pulls). A
//! baseline older than 28 days is refreshed from the current observation.
//! The baseline epoch is tracked **per platform**: one platform's refresh
//! never advances (and so never skips or delays) another platform's
//! 28-day schedule in the shared state file.
//!
//! History written before the daily-row model carried rolling 30-day totals;
//! those points cannot yield day values and are dropped on load.
//!
//! State is written atomically and fsynced. Delivery is fail-closed: if the
//! sink refuses an alert the state is not advanced, so the next run retries
//! the same alert instead of losing it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::alerts::{app_ads_line_identities_lost, metric_dropped, Alert, AlertSink};

/// The default drop threshold: 20% below the baseline.
pub const DEFAULT_DROP_THRESHOLD: f64 = 0.20;

/// How long a baseline stays fixed before it is refreshed.
pub const BASELINE_WINDOW_MS: i64 = 28 * 86_400_000;

/// How many days of daily report totals the watch keeps per unit for the
/// same-weekday medians.
pub const HISTORY_DAYS: i64 = 28;

/// One day's own values for one ad unit, as the daily pull reported them
/// (HH). The point carries the day's revenue and impressions directly, never
/// rolling totals to difference later.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayPoint {
    /// Unix day number (`unix_ms / 86_400_000`).
    pub day: i64,
    pub revenue: i64,
    pub impressions: u64,
    /// The point came from a pre-daily-row state file (rolling 30-day
    /// totals): it has no day value and is dropped on load. Never written.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legacy: bool,
}

impl DayPoint {
    /// A daily-row point.
    fn new(day: i64, revenue: i64, impressions: u64) -> Self {
        Self {
            day,
            revenue,
            impressions,
            legacy: false,
        }
    }
}

// A pre-HH state file stored `revenue30d`/`impressions30d` totals. Those
// deserialize into a legacy point that the observation prunes, so an old
// file loads cleanly instead of failing or fabricating zero-value days.
impl<'de> Deserialize<'de> for DayPoint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            day: i64,
            #[serde(default)]
            revenue: Option<i64>,
            #[serde(default)]
            impressions: Option<u64>,
            #[serde(default)]
            revenue_30d: Option<i64>,
            #[serde(default)]
            impressions_30d: Option<u64>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(match (raw.revenue, raw.impressions) {
            (Some(revenue), Some(impressions)) => Self {
                day: raw.day,
                revenue,
                impressions,
                legacy: false,
            },
            // The old totals keys (or no keys at all) mark a legacy point.
            _ => {
                let _ = (raw.revenue_30d, raw.impressions_30d);
                Self {
                    day: raw.day,
                    revenue: 0,
                    impressions: 0,
                    legacy: true,
                }
            }
        })
    }
}

/// One unit's daily metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DayMetric {
    Revenue,
    Impressions,
    Ecpm,
}

impl DayMetric {
    /// The alert metric name.
    fn name(self) -> &'static str {
        match self {
            DayMetric::Revenue => "revenue",
            DayMetric::Impressions => "impressions",
            DayMetric::Ecpm => "ecpm",
        }
    }

    /// The metric's value for one day's own numbers. `None` when the day
    /// cannot measure it (no impressions, no eCPM).
    fn value(self, revenue: i64, impressions: u64) -> Option<f64> {
        match self {
            DayMetric::Revenue => Some(revenue as f64),
            DayMetric::Impressions => Some(impressions as f64),
            DayMetric::Ecpm => {
                (impressions > 0).then(|| revenue as f64 * 1000.0 / impressions as f64)
            }
        }
    }
}

/// The weekday bucket of a Unix day number. Only equality matters (epoch day
/// 0 was a Thursday), so the fixed modulo is a stable grouping.
fn weekday_of(day: i64) -> i64 {
    day.rem_euclid(7)
}

/// The median of the same weekday's daily metric values inside the trailing
/// 28 days, excluding `today`: for every recorded point whose weekday
/// matches today's, the day value is the point's own daily row (HH — no
/// differencing, so a restatement can never mask a stop). `None` until at
/// least one prior same-weekday day exists — the warm-up a new watch needs.
fn same_weekday_median(points: &[DayPoint], today: i64, metric: DayMetric) -> Option<f64> {
    let weekday = weekday_of(today);
    let mut values: Vec<f64> = points
        .iter()
        .filter(|point| point.day < today && weekday_of(point.day) == weekday)
        .filter_map(|point| {
            metric
                .value(point.revenue, point.impressions)
                .filter(|value| value.is_finite())
        })
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    // The upper median: with an even count the higher of the two middle
    // values, so one low outlier week cannot lower the bar.
    Some(values[values.len() / 2])
}

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
    /// When each platform's baseline was captured (Unix ms), keyed by
    /// platform. Zero (or an absent entry) means "not captured": the next
    /// observation of that platform seeds its baseline, and the 28-day
    /// refresh window runs from that platform's own epoch — never from
    /// another platform's refresh.
    #[serde(default)]
    pub baseline_at: BTreeMap<String, i64>,
    /// The single global baseline timestamp state files carried before the
    /// per-platform epochs existed. Migrated into [`Self::baseline_at`] on
    /// the first observation after load; never written back.
    #[serde(
        default,
        rename = "baselineAtUnixMs",
        skip_serializing_if = "is_zero_timestamp"
    )]
    legacy_baseline_at_unix_ms: i64,
    /// When the latest observation was captured.
    pub checked_at_unix_ms: i64,
    /// Baseline per `platform|ad_unit_id`.
    pub units: BTreeMap<String, UnitObservation>,
    /// Latest observation per `platform|ad_unit_id`.
    pub latest: BTreeMap<String, UnitObservation>,
    /// Canonical app-ads.txt line identities at the baseline.
    pub app_ads_lines: Vec<String>,
    /// The trailing daily report totals per unit, for the same-weekday
    /// medians. Kept for [`HISTORY_DAYS`] days.
    #[serde(default)]
    pub history: BTreeMap<String, Vec<DayPoint>>,
}

/// The zero epoch means "not captured"; the migrated legacy field is skipped
/// when serialized so a saved state carries only the per-platform map.
fn is_zero_timestamp(value: &i64) -> bool {
    *value == 0
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
        // Two workers must not interleave load -> mutate -> rename: the
        // second rename would silently destroy the first writer's update.
        let _state_lock = crate::file_lock::StateLock::acquire(&self.path)?;
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
        })?;
        // The published state is only durable with its directory entry.
        #[cfg(unix)]
        if let Some(parent) = self.path.parent() {
            let parent = if parent.as_os_str().is_empty() {
                std::path::Path::new(".")
            } else {
                parent
            };
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| {
                    format!(
                        "{} directory could not be synced: {error}",
                        self.path.display()
                    )
                })?;
        }
        Ok(())
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

    /// Compare one snapshot with the trailing same-weekday medians, emit
    /// alerts, then advance the state. `Err` means an alert or the state
    /// write failed; the old state stays on disk so the next run retries.
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
        } else if state.publisher != publisher {
            // One state file holds one publisher's baseline: mixing two
            // publishers' baselines would fabricate drops, missing units and
            // identity losses for both.
            return Err(format!(
                "the account watch {} belongs to publisher {}, not {publisher}; refusing to mix baselines",
                self.path.display(),
                state.publisher
            ));
        }
        // Pre-daily-row history stored rolling 30-day totals: those points
        // carry no day value (HH) and are dropped here, so the medians they
        // would poison can never fire or mask a drop.
        for points in state.history.values_mut() {
            points.retain(|point| !point.legacy);
        }
        // Migrate the pre-per-platform single baseline timestamp: every
        // platform that already holds baselined units inherits the old
        // global epoch as its own, so a legacy state keeps its schedule
        // instead of re-baselining everything on the first new-format run.
        // The migrated timestamp is dropped from the state on the next save.
        let legacy_baseline_at = state.legacy_baseline_at_unix_ms;
        if legacy_baseline_at != 0 {
            state.legacy_baseline_at_unix_ms = 0;
            for key in state.units.keys() {
                if let Some((platform_of_key, _)) = key.split_once('|') {
                    state
                        .baseline_at
                        .entry(platform_of_key.to_string())
                        .or_insert(legacy_baseline_at);
                }
            }
        }
        let platform_prefix = format!("{platform}|");
        // Each platform carries its own baseline epoch: a refresh of one
        // platform must never skip or delay another platform's 28-day
        // schedule through a shared timestamp.
        let baseline_at = state.baseline_at.get(platform).copied().unwrap_or(0);
        let first = baseline_at == 0;
        let refresh = !first && now_unix_ms.saturating_sub(baseline_at) >= BASELINE_WINDOW_MS;
        let today = now_unix_ms.div_euclid(86_400_000);

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

        // HH: the day's own values, aggregated per unit from the snapshot's
        // daily rows. The rows describe the last complete reporting day;
        // their own `day` field is authoritative (a pull published late
        // still measures the day it names).
        let mut day_values: BTreeMap<String, (i64, u64)> = BTreeMap::new();
        let mut day_value_day: BTreeMap<String, i64> = BTreeMap::new();
        for row in &snapshot.daily_revenue {
            let key = format!("{platform}|{}", row.ad_unit_id);
            let entry = day_values.entry(key.clone()).or_insert((0, 0));
            entry.0 = entry.0.saturating_add(row.revenue);
            entry.1 = entry.1.saturating_add(row.impressions);
            let day = crate::calendar::day_number(&row.day).unwrap_or(today - 1);
            day_value_day
                .entry(key)
                .and_modify(|seen| *seen = (*seen).max(day))
                .or_insert(day);
        }

        let mut alerts = Vec::new();
        let mut delivery_failed: Option<String> = None;
        let mut identity_lost: u32 = 0;
        if !first && !refresh {
            // Only THIS platform's baselined units can go missing here: an
            // ad-unit-scoped key from another platform (or a platform whose
            // snapshot carries no unit report at all) is not this observation's
            // business.
            for key in state
                .units
                .keys()
                .filter(|key| key.starts_with(&platform_prefix))
            {
                if !current.contains_key(key) {
                    let (_, ad_unit) = key.split_once('|').unwrap_or((platform, key.as_str()));
                    alerts.push(Alert::AccountUnitMissing {
                        publisher: publisher.to_string(),
                        ad_unit: ad_unit.to_string(),
                    });
                }
            }
        }
        if !first {
            // Every day is compared with the same weekday's trailing 28-day
            // median per unit, on the day's own values (the daily rows the
            // pull carried). A unit that stops earning, loses fill or loses
            // price is caught on the first bad day, not after a window has
            // filled with bad days.
            for (key, (revenue, impressions)) in &day_values {
                let Some(points) = state.history.get(key) else {
                    continue;
                };
                let day = day_value_day.get(key).copied().unwrap_or(today - 1);
                for metric in [DayMetric::Revenue, DayMetric::Impressions, DayMetric::Ecpm] {
                    let (Some(baseline), Some(value)) = (
                        same_weekday_median(points, day, metric),
                        metric.value(*revenue, *impressions),
                    ) else {
                        continue;
                    };
                    if metric_dropped(baseline, value, self.threshold) {
                        let (_, ad_unit) = key.split_once('|').unwrap_or((platform, key.as_str()));
                        alerts.push(Alert::AccountMetricDrop {
                            publisher: publisher.to_string(),
                            ad_unit: ad_unit.to_string(),
                            metric: metric.name().to_string(),
                            observed: value,
                            baseline,
                        });
                    }
                }
            }
            // Preserve the last known-good authorization set when the file is
            // absent or empty. An empty present file loses every prior line
            // and must alert, not become the new baseline. A completely
            // absent file is the same loss — the authorization source
            // disappeared — so it alerts with the full baseline count too,
            // beside the daily "file not found" critical, and never erases
            // the baseline a later restore compares against.
            if !refresh && !state.app_ads_lines.is_empty() && !snapshot.app_ads_txt.present {
                alerts.push(Alert::AppAdsLinesLost {
                    publisher: publisher.to_string(),
                    domain: publisher.to_string(),
                    lost: app_ads_line_identities_lost(&state.app_ads_lines, &[]),
                });
            } else if !refresh && !state.app_ads_lines.is_empty() && snapshot.app_ads_txt.present {
                let lost =
                    app_ads_line_identities_lost(&state.app_ads_lines, &snapshot.app_ads_txt.lines);
                identity_lost = lost;
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
            // Refresh only this platform's slice: a second platform keeps its
            // own baseline in the shared file.
            state
                .units
                .retain(|key, _| !key.starts_with(&platform_prefix));
            state.units.extend(current.clone());
            state.baseline_at.insert(platform.to_string(), now_unix_ms);
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
            // While an identity is missing the baseline stays at the last
            // complete set, so a partial restore cannot mask the loss.
            if let Some(lines) = observed_lines {
                if identity_lost == 0 {
                    state.app_ads_lines = lines;
                }
            }
        }
        // Record the day's own values per unit for the same-weekday medians
        // (HH): a same-day re-pull replaces the point, and points older than
        // the 28-day window fall off. The slice is scoped like the latest
        // slice: one platform's observation must not erase another platform's
        // history.
        for (key, (revenue, impressions)) in &day_values {
            let day = day_value_day.get(key).copied().unwrap_or(today - 1);
            let point = DayPoint::new(day, *revenue, *impressions);
            let points = state.history.entry(key.clone()).or_default();
            match points.binary_search_by_key(&point.day, |recorded| recorded.day) {
                Ok(position) => points[position] = point,
                Err(position) => points.insert(position, point),
            }
            let cutoff = point.day - HISTORY_DAYS;
            points.retain(|recorded| recorded.day > cutoff);
        }
        state
            .history
            .retain(|key, _| !key.starts_with(&platform_prefix) || current.contains_key(key));
        // The latest slice is scoped the same way: one platform's observation
        // must not erase another platform's per-unit readings.
        state
            .latest
            .retain(|key, _| !key.starts_with(&platform_prefix));
        state.latest.extend(current);
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
    use apex_audit::{AppAdsTxt, CountryRevenueRow, DailyRevenueRow, RevenueTotals, Snapshot};
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
            daily_revenue: Vec::new(),
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

    /// The snapshot plus the day's own rows for one unit (HH): the 30-day
    /// totals drive the baseline/missing checks, the daily row drives the
    /// same-weekday medians.
    fn snapshot_with_daily(
        unit: &str,
        revenue_30d: i64,
        impressions_30d: u64,
        day: i64,
        day_revenue: i64,
        day_impressions: u64,
        lines: &[&str],
    ) -> Snapshot {
        let mut snapshot = snapshot(unit, revenue_30d, impressions_30d, lines);
        snapshot.daily_revenue = vec![DailyRevenueRow {
            day: crate::calendar::format_day(day),
            country: "US".into(),
            ad_unit_id: unit.into(),
            revenue: day_revenue,
            impressions: day_impressions,
        }];
        snapshot
    }

    fn watch(name: &str) -> (AccountWatch, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("apex-account-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (AccountWatch::new(dir.join("watch.json"), 0.2).unwrap(), dir)
    }

    /// A day-aligned timestamp for Unix day `d`, one millisecond in: unix ms
    /// zero is the state's "no baseline captured" sentinel.
    fn day_ms(d: i64) -> i64 {
        d * 86_400_000 + 1
    }

    /// Observe a steady day: the 30-day totals grow by the day's own earn,
    /// and the daily row carries exactly that earn.
    async fn observe_steady(
        watch: &AccountWatch,
        sink: &RecordingAlertSink,
        d: i64,
        day_revenue: i64,
        day_impressions: u64,
    ) -> Vec<Alert> {
        watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily(
                    "u1",
                    day_revenue * (d + 1),
                    day_impressions * (d + 1) as u64,
                    d,
                    day_revenue,
                    day_impressions,
                    &["a,b,DIRECT"],
                ),
                day_ms(d),
                sink,
            )
            .await
            .expect("steady day")
    }

    #[tokio::test]
    async fn baseline_then_a_second_low_observation_alerts_and_persists() {
        let (watch, dir) = watch("drop");
        let sink = RecordingAlertSink::new();
        // Seven steady days at a $5 eCPM day rate: 5_000_000 micros and 1_000
        // impressions land on the unit every day.
        for d in 0..8 {
            let alerts = observe_steady(&watch, &sink, d, 5_000_000, 1_000).await;
            assert!(alerts.is_empty(), "day {d}: {alerts:?}");
        }
        // Day 8 is the same weekday as day 1: the same-weekday median says a
        // $5 eCPM day, and this day earns $3 on the same delivery — the
        // price alert fires that day.
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily(
                    "u1",
                    5_000_000 * 8 + 3_000_000,
                    9_000,
                    8,
                    3_000_000,
                    1_000,
                    &["a,b,DIRECT"],
                ),
                day_ms(8),
                &sink,
            )
            .await
            .expect("compare");
        assert!(
            alerts
                .iter()
                .any(|a| matches!(a, Alert::AccountMetricDrop { metric, .. } if metric == "ecpm")),
            "{alerts:?}"
        );
        assert!(!sink.alerts().is_empty());
        let state = watch.load().expect("load").expect("state");
        assert_eq!(state.units["applovin_max|u1"].revenue_30d, 5_000_000);
        assert_eq!(state.latest["applovin_max|u1"].revenue_30d, 43_000_000);
        let drift = watch.ecpm_drift("applovin_max", "u1").expect("drift");
        // (43M/9000)/(5M/1000) - 1 = 43/45 - 1.
        assert!((drift + 2.0 / 45.0).abs() < 1e-9, "{drift}");
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
    async fn absent_and_empty_app_ads_alert_the_full_loss_and_keep_the_baseline() {
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

        // A completely absent file loses every recorded authorization: the
        // alert carries the full baseline count so monitoring sees the
        // authorization source disappear, not only the daily critical.
        let mut absent = snapshot("u1", 5_000_000, 1_000, &[]);
        absent.app_ads_txt.present = false;
        let alerts = watch
            .observe("northstar", "applovin_max", &absent, 2_000, &sink)
            .await
            .expect("absent");
        assert!(
            alerts
                .iter()
                .any(|alert| matches!(alert, Alert::AppAdsLinesLost { lost: 3, .. })),
            "the absent file alerts the full baseline loss: {alerts:?}"
        );
        let state = watch.load().expect("load").expect("state");
        assert_eq!(state.app_ads_lines, baseline, "absent preserves it");

        // A present but empty file loses the same authorizations; the alert
        // names all three and the baseline is not replaced by emptiness.
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
    async fn comment_only_and_partial_restores_keep_the_last_complete_baseline() {
        let (watch, dir) = watch("sticky-baseline");
        let sink = RecordingAlertSink::new();
        let baseline = ["a,1,DIRECT", "b,2,DIRECT"];
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

        // A comment-only file has no authorizations: it alerts as a full
        // loss and never becomes the baseline (H10 + H9).
        let comments = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 5_000_000, 1_000, &["# nothing left"]),
                2_000,
                &sink,
            )
            .await
            .expect("comments");
        assert!(comments
            .iter()
            .any(|alert| matches!(alert, Alert::AppAdsLinesLost { lost: 2, .. })));
        let state = watch.load().expect("load").expect("state");
        assert_eq!(state.app_ads_lines, baseline, "comment-only preserves it");

        // A partial restore keeps the full original baseline: the one still
        // missing identity must remain detectable.
        let partial = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u1", 5_000_000, 1_000, &["a,1,DIRECT"]),
                3_000,
                &sink,
            )
            .await
            .expect("partial");
        assert!(partial
            .iter()
            .any(|alert| matches!(alert, Alert::AppAdsLinesLost { lost: 1, .. })));
        let state = watch.load().expect("load").expect("state");
        assert_eq!(
            state.app_ads_lines, baseline,
            "a partial restore must not shrink the baseline"
        );

        // Reformatting the same two records is not a loss: no alert, and the
        // baseline advances to the equivalent spelling.
        let reformatted = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot(
                    "u1",
                    5_000_000,
                    1_000,
                    &[" A ,1, direct # moved ", "  b ,  2 ,DIRECT "],
                ),
                4_000,
                &sink,
            )
            .await
            .expect("reformatted");
        assert!(!reformatted
            .iter()
            .any(|alert| matches!(alert, Alert::AppAdsLinesLost { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn another_platforms_basline_never_looks_missing_or_wipes_units() {
        let (watch, dir) = watch("cross-platform");
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
            .expect("max baseline");

        // A platform whose snapshot carries no unit report (AdMob/LevelPlay
        // style) must not report MAX's units as missing, nor wipe them.
        let mut empty = snapshot("u1", 5_000_000, 1_000, &["a,1,DIRECT"]);
        empty.country_revenue.clear();
        let alerts = watch
            .observe("northstar", "admob", &empty, 2_000, &sink)
            .await
            .expect("admob observe");
        assert!(
            !alerts
                .iter()
                .any(|alert| matches!(alert, Alert::AccountUnitMissing { .. })),
            "another platform's units are not this observation's: {alerts:?}"
        );
        let state = watch.load().expect("load").expect("state");
        assert!(
            state
                .units
                .keys()
                .any(|key| key.starts_with("applovin_max|")),
            "the MAX baseline survives: {:?}",
            state.units.keys().collect::<Vec<_>>()
        );

        // MAX's own unit going missing still alerts.
        let mut gone = snapshot("u2", 5_000_000, 1_000, &["a,1,DIRECT"]);
        gone.country_revenue[0].ad_unit_id = "u2".into();
        let alerts = watch
            .observe("northstar", "applovin_max", &gone, 3_000, &sink)
            .await
            .expect("max missing");
        assert!(alerts.iter().any(
            |alert| matches!(alert, Alert::AccountUnitMissing { ad_unit, .. } if ad_unit == "u1")
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_state_belonging_to_another_publisher_is_refused() {
        let (watch, dir) = watch("publisher-mix");
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
            .expect("baseline");
        let error = watch
            .observe(
                "other-publisher",
                "applovin_max",
                &snapshot("u1", 5_000_000, 1_000, &["a,1,DIRECT"]),
                2_000,
                &sink,
            )
            .await
            .expect_err("one file, one publisher");
        assert!(error.contains("belongs to publisher"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_refresh_rebaselines_one_platform_without_erasing_the_other() {
        let (watch, dir) = watch("refresh-keeps-platforms");
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
            .expect("max baseline");
        let mut other = snapshot("u2", 4_000_000, 900, &["a,1,DIRECT"]);
        other.platform = apex_core::PlatformId::Admob;
        watch
            .observe("northstar", "admob", &other, 2_000, &sink)
            .await
            .expect("admob baseline");

        // MAX's baseline ages past the refresh window: only MAX's slice is
        // rebuilt, the other platform's keys stay in the shared file.
        let after_window = 1_000 + BASELINE_WINDOW_MS + 1_000;
        let refreshed = snapshot("u3", 5_500_000, 1_100, &["a,1,DIRECT"]);
        watch
            .observe("northstar", "applovin_max", &refreshed, after_window, &sink)
            .await
            .expect("max refresh");
        let state = watch.load().expect("load").expect("state");
        let keys: Vec<&String> = state.units.keys().collect();
        assert!(state.units.contains_key("applovin_max|u3"), "{keys:?}");
        assert!(!state.units.contains_key("applovin_max|u1"), "{keys:?}");
        assert!(
            state.units.contains_key("admob|u2"),
            "the other platform's baseline survives a refresh of this one: {keys:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delivery_failure_does_not_advance_the_state() {
        let (watch, dir) = watch("failclosed");
        let sink = RecordingAlertSink::new();
        for d in 0..8 {
            observe_steady(&watch, &sink, d, 100_000, 1_000).await;
        }
        let before = std::fs::read_to_string(dir.join("watch.json")).unwrap();
        #[derive(Debug)]
        struct Failing;
        #[async_trait::async_trait]
        impl AlertSink for Failing {
            async fn emit(&self, _alert: Alert) -> Result<(), crate::alerts::AlertError> {
                Err(crate::alerts::AlertError("disk full".into()))
            }
        }
        // Day 8 earns $1 on a $100 median: the alert exists, the sink refuses
        // it, and the whole state stays on the last good generation.
        let error = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily("u1", 800_000 + 1_000, 8_001, 8, 1_000, 1, &["a,1,DIRECT"]),
                day_ms(8),
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

    #[tokio::test]
    async fn a_unit_that_stops_earning_alerts_on_the_first_bad_day() {
        let (watch, dir) = watch("first-bad-day");
        let sink = RecordingAlertSink::new();
        for d in 0..8 {
            let alerts = observe_steady(&watch, &sink, d, 100_000, 100).await;
            assert!(alerts.is_empty(), "day {d}: {alerts:?}");
        }
        // The unit earns nothing today: the daily row is zero on both
        // metrics. The alert fires on this run — the first bad day — not
        // after a window and never masked as a restatement (HH).
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily("u1", 800_000, 800, 8, 0, 0, &["a,1,DIRECT"]),
                day_ms(8),
                &sink,
            )
            .await
            .expect("stopped day");
        assert!(
            alerts.iter().any(|alert| matches!(alert,
                Alert::AccountMetricDrop { metric, ad_unit, observed, baseline, .. }
                if metric == "revenue" && ad_unit == "u1" && *observed == 0.0 && *baseline == 100_000.0)),
            "{alerts:?}"
        );
        assert!(
            alerts
                .iter()
                .any(|alert| matches!(alert, Alert::AccountMetricDrop { metric, .. } if metric == "impressions")),
            "{alerts:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_weekday_median_ignores_the_quiet_weekday_and_flags_the_busy_one() {
        let (watch, dir) = watch("weekday-shape");
        let sink = RecordingAlertSink::new();
        // Weekday bucket 6 of the week earns a tenth of the other days. Two
        // full weeks seed both shapes; no day of a normal week alerts.
        let mut totals = (0_i64, 0_u64);
        for d in 0..14_i64 {
            let day = if d.rem_euclid(7) == 6 {
                (10_000, 100_u64)
            } else {
                (100_000, 1_000_u64)
            };
            totals.0 += day.0;
            totals.1 += day.1;
            let alerts = watch
                .observe(
                    "northstar",
                    "applovin_max",
                    &snapshot_with_daily(
                        "u1",
                        totals.0,
                        totals.1,
                        d,
                        day.0,
                        day.1,
                        &["a,1,DIRECT"],
                    ),
                    day_ms(d),
                    &sink,
                )
                .await
                .expect("seed day");
            assert!(alerts.is_empty(), "day {d}: {alerts:?}");
        }
        // A third quiet weekday at the usual quiet level is the normal
        // shape — a weekday-blind comparison would read it as a drop.
        totals.0 += 10_000;
        totals.1 += 100;
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily("u1", totals.0, totals.1, 20, 10_000, 100, &["a,1,DIRECT"]),
                day_ms(20),
                &sink,
            )
            .await
            .expect("quiet day");
        // Day 20 is bucket 6 (20 - 14 = 6): the quiet median, no alert.
        assert!(alerts.is_empty(), "a normal quiet day: {alerts:?}");
        // The same small day on a busy weekday is a real drop.
        totals.0 += 10_000;
        totals.1 += 100;
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily("u1", totals.0, totals.1, 21, 10_000, 100, &["a,1,DIRECT"]),
                day_ms(21),
                &sink,
            )
            .await
            .expect("busy day");
        assert!(
            alerts
                .iter()
                .any(|alert| matches!(alert, Alert::AccountMetricDrop { metric, .. } if metric == "revenue")),
            "the same total on a busy weekday alerts: {alerts:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_price_drop_alerts_on_ecpm_and_a_delivery_drop_on_impressions() {
        let (watch, dir) = watch("metric-mix");
        let sink = RecordingAlertSink::new();
        for d in 0..8 {
            observe_steady(&watch, &sink, d, 100_000, 1_000).await;
        }
        // Day 8 earns half the money on the same delivery: the price metric
        // (eCPM, the per-audience evidence for ARPDAU) drops with revenue,
        // delivery does not.
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily("u1", 850_000, 9_000, 8, 50_000, 1_000, &["a,1,DIRECT"]),
                day_ms(8),
                &sink,
            )
            .await
            .expect("price drop");
        assert!(
            alerts.iter().any(
                |alert| matches!(alert, Alert::AccountMetricDrop { metric, .. } if metric == "ecpm")
            ),
            "{alerts:?}"
        );
        assert!(
            !alerts
                .iter()
                .any(|alert| matches!(alert, Alert::AccountMetricDrop { metric, .. } if metric == "impressions")),
            "delivery held: {alerts:?}"
        );
        // Day 9 keeps the money but loses half the delivery: the fill-side
        // evidence drops, price rises, revenue is steady.
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily("u1", 950_000, 9_500, 9, 100_000, 500, &["a,1,DIRECT"]),
                day_ms(9),
                &sink,
            )
            .await
            .expect("delivery drop");
        assert!(
            alerts
                .iter()
                .any(|alert| matches!(alert, Alert::AccountMetricDrop { metric, .. } if metric == "impressions")),
            "{alerts:?}"
        );
        assert!(
            !alerts.iter().any(
                |alert| matches!(alert, Alert::AccountMetricDrop { metric, .. } if metric == "ecpm")
            ),
            "price rose: {alerts:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_pull_gap_never_distorts_a_day_value() {
        // HH: the day's value is the pull's own daily row, not a difference
        // of rolling totals — a missed pull cannot smear two days into one
        // comparison, and nothing is skipped as a restatement.
        let (watch, dir) = watch("gap");
        let sink = RecordingAlertSink::new();
        for d in 0..8 {
            observe_steady(&watch, &sink, d, 100_000, 1_000).await;
        }
        // The next pull lands after a 13-day gap and still reports the day's
        // own earn: it compares cleanly with the same weekday's median.
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily(
                    "u1",
                    800_000 + 1_300_000,
                    21_000,
                    21,
                    100_000,
                    1_000,
                    &["a,b,DIRECT"],
                ),
                day_ms(21),
                &sink,
            )
            .await
            .expect("gap day");
        assert!(alerts.is_empty(), "a normal day after a gap: {alerts:?}");
        // The row's own day field decides what was measured, so a pull
        // published late still compares the day it names.
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily(
                    "u1",
                    2_100_000 + 100_000,
                    22_000,
                    22,
                    100_000,
                    1_000,
                    &["a,b,DIRECT"],
                ),
                day_ms(30),
                &sink,
            )
            .await
            .expect("late pull");
        assert!(alerts.is_empty(), "{alerts:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_same_day_repull_replaces_the_point_instead_of_stacking() {
        let (watch, dir) = watch("same-day");
        let sink = RecordingAlertSink::new();
        for d in 0..8 {
            observe_steady(&watch, &sink, d, 100_000, 1_000).await;
        }
        // A second pull on the same day revises the point; it never becomes a
        // second sample.
        watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily("u1", 800_000, 8_000, 7, 100_000, 1_000, &["a,1,DIRECT"]),
                day_ms(7) + 3_600_000,
                &sink,
            )
            .await
            .expect("same-day repull");
        let state = watch.load().expect("load").expect("state");
        assert_eq!(
            state.history["applovin_max|u1"].len(),
            8,
            "eight days, one point each: {:?}",
            state.history["applovin_max|u1"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_median_ignores_one_bad_week_and_is_not_the_mean() {
        let (watch, dir) = watch("median");
        let sink = RecordingAlertSink::new();
        // Four weeks plus three steady days, except weekday bucket 3 gets
        // one bad day (10k). The trailing 28-day window leaves the three
        // same-weekday samples 10k, 100k, 100k for day 31.
        let mut totals = (0_i64, 0_u64);
        for d in 0..31_i64 {
            let day = if d == 10 {
                (10_000, 100_u64)
            } else {
                (100_000, 1_000_u64)
            };
            totals.0 += day.0;
            totals.1 += day.1;
            watch
                .observe(
                    "northstar",
                    "applovin_max",
                    &snapshot_with_daily(
                        "u1",
                        totals.0,
                        totals.1,
                        d,
                        day.0,
                        day.1,
                        &["a,1,DIRECT"],
                    ),
                    day_ms(d),
                    &sink,
                )
                .await
                .expect("seed day");
        }
        // Day 31 (bucket 3) earns 57% of a steady day. The same-weekday
        // median is 100k (the one bad week does not drag it down), so 57k
        // alerts. The mean (70k, threshold 56k) would not.
        totals.0 += 57_000;
        totals.1 += 570;
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily("u1", totals.0, totals.1, 31, 57_000, 570, &["a,1,DIRECT"]),
                day_ms(31),
                &sink,
            )
            .await
            .expect("half day");
        assert!(
            alerts.iter().any(|alert| matches!(alert,
                    Alert::AccountMetricDrop { metric, observed, baseline, .. }
                    if metric == "revenue" && *observed == 57_000.0 && *baseline == 100_000.0)),
            "the same-weekday median is the baseline: {alerts:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_state_from_the_totals_model_loads_without_fabricated_drops() {
        // History written before the daily-row model carries rolling 30-day
        // totals: it loads, is pruned as legacy, and can never read as a
        // zero-earn day or poison a median.
        let (watch, dir) = watch("legacy");
        let sink = RecordingAlertSink::new();
        for d in 0..8 {
            observe_steady(&watch, &sink, d, 100_000, 1_000).await;
        }
        // Rewrite one point into the legacy totals shape.
        let path = dir.join("watch.json");
        let mut state: AccountWatchState =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let points = state.history.get_mut("applovin_max|u1").expect("history");
        points[0] = serde_json::from_value(serde_json::json!({
            "day": points[0].day,
            "revenue30d": 100_000,
            "impressions30d": 1_000,
        }))
        .expect("legacy point");
        std::fs::write(&path, serde_json::to_string_pretty(&state).unwrap()).unwrap();

        // The next observation drops the legacy point and compares only the
        // real daily rows: the same weekday's median stays 100k and a steady
        // day stays quiet.
        let alerts = watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot_with_daily(
                    "u1",
                    800_000 + 100_000,
                    9_000,
                    8,
                    100_000,
                    1_000,
                    &["a,b,DIRECT"],
                ),
                day_ms(8),
                &sink,
            )
            .await
            .expect("post-legacy day");
        assert!(alerts.is_empty(), "{alerts:?}");
        let state = watch.load().expect("load").expect("state");
        let points = state.history["applovin_max|u1"].as_slice();
        assert!(
            points.iter().all(|point| !point.legacy),
            "legacy points are pruned: {points:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zero_epoch_is_the_only_skipped_legacy_timestamp() {
        // skip_serializing_if: only the zero epoch is dropped; a real
        // captured timestamp must always survive a save/load round trip.
        assert!(is_zero_timestamp(&0));
        assert!(!is_zero_timestamp(&1));
        assert!(!is_zero_timestamp(&i64::MAX));
    }

    #[test]
    fn thresholds_above_one_are_rejected() {
        assert!(AccountWatch::new("x", 1.0).is_ok());
        assert!(AccountWatch::new("x", 1.01).is_err());
        assert!(AccountWatch::new("x", f64::NAN).is_err());
        assert!(AccountWatch::new("x", 0.0).is_err());
    }

    #[tokio::test]
    async fn a_refresh_of_one_platform_never_skips_another_platforms_schedule() {
        // Both platforms capture their baselines on day 0. On day 28 MAX's
        // baseline ages out and refreshes. LevelPlay is due on its OWN
        // day-0 epoch as well: the timestamp the MAX refresh just wrote
        // must not skip it (the old global timestamp kept LevelPlay's stale
        // baseline and read its real unit replacement as a missing unit).
        let (watch, dir) = watch("per-platform-epoch");
        let sink = RecordingAlertSink::new();
        watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("mu1", 5_000_000, 1_000, &["a,b,DIRECT"]),
                day_ms(0),
                &sink,
            )
            .await
            .expect("max baseline");
        let mut levelplay = snapshot("lu1", 4_000_000, 800, &["a,b,DIRECT"]);
        levelplay.platform = apex_core::PlatformId::LevelPlay;
        watch
            .observe(
                "northstar",
                "levelplay",
                &levelplay,
                day_ms(0) + 1_000,
                &sink,
            )
            .await
            .expect("levelplay baseline");

        // Day 28: MAX refreshes onto a new unit. Only MAX's epoch advances.
        let max_refresh_at = day_ms(0) + BASELINE_WINDOW_MS + 1_000;
        watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("mu2", 5_500_000, 1_100, &["a,b,DIRECT"]),
                max_refresh_at,
                &sink,
            )
            .await
            .expect("max refresh");
        let state = watch.load().expect("load").expect("state");
        assert_eq!(state.baseline_at["applovin_max"], max_refresh_at);
        assert_eq!(
            state.baseline_at["levelplay"],
            day_ms(0) + 1_000,
            "MAX's refresh must not advance LevelPlay's epoch"
        );

        // LevelPlay observes moments later with its unit replaced. Its own
        // baseline is 28 days old, so it refreshes on its own schedule: the
        // replacement rebaselines silently instead of alerting the false
        // missing-unit the global-timestamp skip produced.
        let levelplay_at = max_refresh_at + 1_000;
        let mut replaced = snapshot("lu2", 4_100_000, 820, &["a,b,DIRECT"]);
        replaced.platform = apex_core::PlatformId::LevelPlay;
        let alerts = watch
            .observe("northstar", "levelplay", &replaced, levelplay_at, &sink)
            .await
            .expect("levelplay refresh");
        assert!(
            !alerts
                .iter()
                .any(|alert| matches!(alert, Alert::AccountUnitMissing { .. })),
            "LevelPlay refreshed on its own schedule: {alerts:?}"
        );
        let state = watch.load().expect("load").expect("state");
        assert_eq!(state.baseline_at["levelplay"], levelplay_at);
        assert!(state.units.contains_key("levelplay|lu2"));
        assert!(
            !state.units.contains_key("levelplay|lu1"),
            "the refresh replaced LevelPlay's own slice"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_legacy_single_timestamp_state_migrates_onto_per_platform_epochs() {
        // A state file from the single-timestamp model: one global epoch and
        // a MAX baseline. Loading it must give MAX its own epoch instead of
        // re-baselining (which would erase the fixed 28-day baseline).
        let (watch, dir) = watch("legacy-epoch");
        let legacy = serde_json::json!({
            "publisher": "northstar",
            "baselineAtUnixMs": 1_000_i64,
            "checkedAtUnixMs": 1_000_i64,
            "units": {
                "applovin_max|u1": {"revenue30d": 5_000_000_i64, "impressions30d": 1_000_u64, "ecpm": 5_000_000.0}
            },
            "latest": {
                "applovin_max|u1": {"revenue30d": 5_000_000_i64, "impressions30d": 1_000_u64, "ecpm": 5_000_000.0}
            },
            "appAdsLines": ["a,b,DIRECT"],
            "history": {},
        });
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join("watch.json"),
            serde_json::to_string_pretty(&legacy).expect("json"),
        )
        .expect("write");

        // The next MAX observation lands inside the 28-day window: it keeps
        // the inherited baseline (a re-baseline would replace the unit
        // slice) and records MAX's own epoch.
        let sink = RecordingAlertSink::new();
        watch
            .observe(
                "northstar",
                "applovin_max",
                &snapshot("u2", 5_100_000, 1_010, &["a,b,DIRECT"]),
                1_000 + BASELINE_WINDOW_MS - 1_000,
                &sink,
            )
            .await
            .expect("legacy continuation");
        let state = watch.load().expect("load").expect("state");
        assert_eq!(
            state.baseline_at["applovin_max"], 1_000,
            "the legacy epoch is inherited per platform"
        );
        assert!(
            state.units.contains_key("applovin_max|u1"),
            "the inherited baseline is not re-captured"
        );
        assert!(
            state.units.contains_key("applovin_max|u2"),
            "a first-seen unit is baselined in place"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
