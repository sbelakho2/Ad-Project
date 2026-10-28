//! The live AppLovin MAX write adapter: the production [`PlatformOps`].
//!
//! The MAX path never mutates the live ad unit. A cycle reads the unit's
//! current floors, folds the proposed country-floor change into a copy, and
//! starts the experiment with that copy as the **test arm's** configuration;
//! the control arm and the live unit stay exactly as they were. A kept result
//! promotes the experiment to all users, a rollback or void deprecates it.
//!
//! Daily pulls come from the Reporting API grouped by `max_ad_unit_test`: the
//! control arm carries the empty label or the literal `Control` (the
//! reporting docs' own example value, matched case-insensitively and
//! trimmed); the test arm carries exactly one other label. The experiment
//! defines exactly two arms, so any second distinct test label fails the pull
//! typed, naming the labels seen, instead of folding rows into a phantom
//! third arm. MAX reports no user counts, so the arm value is revenue divided
//! by the arm's allocated share (the window does that division). A day is
//! final only once two independent reporting reads at least 24 hours apart
//! agree; the durable pull-evidence store owns that decision.
//!
//! Every write is gated by a caller-supplied allow-list (the verify-live
//! gate): by default the adapter refuses all writes rather than guessing that
//! an endpoint is safe.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use apex_connectors::max::{MaxCapFrequencyType, MaxClient, MaxFrequencyCap, MaxReportClient};
use apex_connectors::transport::Transport;
use apex_core::{CountryFloor, ExperimentResult, PlatformError, PlatformId, ReportRow};
use apex_proof::ChangeKind;
use async_trait::async_trait;

use crate::ops::{ExperimentDays, ExperimentRequest, PlatformOps};

/// A clock the adapter reads to bound the reporting window; injectable so the
/// pull window is deterministic in tests.
pub trait Wall: Send + Sync + std::fmt::Debug {
    /// Unix seconds.
    fn now_unix_s(&self) -> i64;
}

/// The process clock.
#[derive(Debug, Default)]
pub struct SystemWall;

impl Wall for SystemWall {
    fn now_unix_s(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

/// How many trailing days the adapter asks the reporting API for; generous
/// enough for any allowed window plus burn-in.
const LOOKBACK_DAYS: i64 = 63;
const DAY_SECS: i64 = 86_400;
/// The User-Level Ad Revenue API keeps 45 days; D2 retains that window per
/// player.
const PLAYER_LOOKBACK_DAYS: i64 = apex_lift::player_store::MAX_PLAYER_HISTORY_DAYS as i64;

pub struct MaxPlatformOps {
    management: MaxClient,
    reporting: MaxReportClient,
    wall: Arc<dyn Wall>,
    /// Verify-live allow-list: an operation here (e.g. `setCountryFloor`) is
    /// writable; anything else is refused before any side effect.
    writable: BTreeMap<String, bool>,
    /// The durable two-pull evidence for every arm-day this adapter reads.
    evidence: std::sync::Mutex<crate::pull_evidence::PullEvidenceStore>,
}

/// The platform namespace used for MAX pull-evidence files.
const EVIDENCE_NAMESPACE: &str = "applovin_max";

impl std::fmt::Debug for MaxPlatformOps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaxPlatformOps")
            .field("writable", &self.writable)
            .finish()
    }
}

impl MaxPlatformOps {
    /// Build the adapter. `writable` lists the operations the operator has
    /// verified against a live sandbox; without it every write is refused.
    pub fn new(
        api_key: &str,
        report_key: &str,
        management_base: &str,
        reporting_base: &str,
        transport: Arc<dyn Transport>,
        wall: Arc<dyn Wall>,
        writable: impl IntoIterator<Item = String>,
    ) -> Result<Self, PlatformError> {
        let management = MaxClient::new(api_key, management_base, Arc::clone(&transport))?;
        let reporting = MaxReportClient::new(report_key, reporting_base, transport)?;
        Ok(Self {
            management,
            reporting,
            wall,
            writable: writable.into_iter().map(|op| (op, true)).collect(),
            evidence: std::sync::Mutex::new(crate::pull_evidence::PullEvidenceStore::in_memory(
                EVIDENCE_NAMESPACE,
            )),
        })
    }

    /// Make the pull-evidence store durable under `root`. Production builds
    /// the adapter with this so a second read in a later process still sees
    /// the first one.
    pub fn with_evidence_root(
        mut self,
        root: impl Into<std::path::PathBuf>,
    ) -> Result<Self, PlatformError> {
        self.evidence = std::sync::Mutex::new(crate::pull_evidence::PullEvidenceStore::at(
            EVIDENCE_NAMESPACE,
            root,
        ));
        Ok(self)
    }

    fn schema(&self, operation: &str, detail: impl Into<String>) -> PlatformError {
        PlatformError::Schema {
            platform: PlatformId::AppLovinMax,
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    /// Read the unit and fold the proposed change into a copy of its floors.
    async fn test_arm_floors(
        &self,
        request: &ExperimentRequest,
    ) -> Result<Vec<CountryFloor>, PlatformError> {
        let unit = self
            .management
            .get_ad_unit(&request.spec.ad_unit_id)
            .await?;
        let mut floors = unit.country_floors.unwrap_or_default();
        match &request.change {
            ChangeKind::CountryFloorSet {
                country,
                floor_ecpm,
            } => {
                let found = floors
                    .iter()
                    .position(|floor| floor.country.eq_ignore_ascii_case(country));
                match found {
                    Some(index) => {
                        let group = floors[index]
                            .group
                            .clone()
                            .unwrap_or_else(|| "default".to_string());
                        let shared = floors
                            .iter()
                            .filter(|floor| floor.group.as_deref().unwrap_or("default") == group)
                            .count();
                        floors[index].floor_ecpm = *floor_ecpm;
                        if shared > 1 {
                            // A shared country group folds to ONE cpm (the
                            // first floor's): changing the country in place
                            // would either drop this floor or collateralize
                            // every other country in the group. Isolating the
                            // changed country keeps the pre-registered change
                            // to exactly one country.
                            floors[index].group = Some(isolated_floor_group(&group, country));
                        }
                    }
                    None => floors.push(CountryFloor {
                        country: country.clone(),
                        floor_ecpm: *floor_ecpm,
                        // A new country must not join an existing group
                        // whose first floor would overwrite its cpm.
                        group: Some(isolated_floor_group("default", country)),
                    }),
                }
            }
            _ => {
                return Err(self.schema(
                    "createExperiment",
                    "a MAX experiment carries a country-floor change only",
                ))
            }
        }
        Ok(floors)
    }
}

/// The deterministic singleton group a single changed country is folded into,
/// so `floor_groups`' first-floor-wins fold cannot drop or collateralize the
/// change: only this country carries this cpm.
fn isolated_floor_group(original_group: &str, country: &str) -> String {
    fn slug(text: &str) -> String {
        text.chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect()
    }
    format!("apex-{}-{}", slug(original_group), slug(country))
}

#[async_trait]
impl PlatformOps for MaxPlatformOps {
    fn platform(&self) -> PlatformId {
        PlatformId::AppLovinMax
    }

    async fn writable(&self, operation: &str) -> bool {
        self.writable.get(operation).copied().unwrap_or(false)
    }

    async fn start_experiment(&self, request: &ExperimentRequest) -> Result<String, PlatformError> {
        match &request.change {
            ChangeKind::CountryFloorSet { .. } => {
                let floors = self.test_arm_floors(request).await?;
                self.management
                    .create_experiment(&request.spec, Some(&floors))
                    .await
            }
            ChangeKind::FrequencyCapSet {
                ad_unit_id,
                cap_type,
                countries,
                session_limit,
                day_limit,
                minute_frequency,
            } => {
                if ad_unit_id != &request.spec.ad_unit_id {
                    return Err(self.schema(
                        "createExperiment",
                        format!(
                            "frequency cap targets ad unit '{ad_unit_id}' but the experiment is on '{}'",
                            request.spec.ad_unit_id
                        ),
                    ));
                }
                let cap_type = match cap_type.as_str() {
                    "session" => MaxCapFrequencyType::Session,
                    "time" => MaxCapFrequencyType::Time,
                    other => {
                        return Err(self.schema(
                            "createExperiment",
                            format!("unknown MAX frequency cap type '{other}'"),
                        ))
                    }
                };
                let cap = MaxFrequencyCap {
                    cap_type,
                    countries: countries.clone(),
                    session_limit: *session_limit,
                    day_limit: *day_limit,
                    minute_frequency: *minute_frequency,
                };
                // The connector rejects overlapping country lists across the
                // settings; one test-group cap per experiment is the A4 shape.
                self.management
                    .create_experiment_with_caps(&request.spec, &[cap])
                    .await
            }
            _ => Err(self.schema(
                "createExperiment",
                "a MAX experiment carries a country-floor or frequency-cap change only",
            )),
        }
    }

    async fn start_null_experiment(
        &self,
        spec: &apex_core::ExperimentSpec,
    ) -> Result<String, PlatformError> {
        // No floor change: both arms carry the live unit's configuration.
        self.management.create_experiment(spec, None).await
    }

    fn holdout_supported(&self) -> bool {
        true
    }

    fn supports_player_feed(&self) -> bool {
        true
    }

    async fn start_holdout(
        &self,
        request: &crate::ops::HoldoutRequest,
    ) -> Result<String, PlatformError> {
        // The live unit already serves the kept change. The holdout's test
        // arm is the old configuration at 10%, so the control side (which
        // keeps the new setting) is the exposed arm.
        match &request.change {
            ChangeKind::CountryFloorSet { .. } => {
                let old: Vec<CountryFloor> =
                    serde_json::from_str(&request.prev_value).map_err(|error| {
                        self.schema(
                            "createExperiment",
                            format!(
                                "the pre-registered old floor setting is not readable: {error}"
                            ),
                        )
                    })?;
                self.management
                    .create_experiment(&request.spec, Some(&old))
                    .await
            }
            ChangeKind::FrequencyCapSet { .. } => {
                // The old setting carried no cap; the holdout test arm gets
                // the uncapped configuration.
                self.management.create_experiment(&request.spec, None).await
            }
            _ => Err(self.schema(
                "createExperiment",
                "a MAX holdout carries a country-floor or frequency-cap change only",
            )),
        }
    }

    fn change_in_test_arm(&self) -> bool {
        true
    }

    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError> {
        self.management.get_experiment(experiment_id).await
    }

    async fn experiment_days(&self, experiment_id: &str) -> Result<ExperimentDays, PlatformError> {
        self.experiment_days_for(experiment_id, experiment_id).await
    }

    async fn experiment_days_for(
        &self,
        experiment_id: &str,
        instance_key: &str,
    ) -> Result<ExperimentDays, PlatformError> {
        let observed_at = self.wall.now_unix_s();
        let today = observed_at.div_euclid(DAY_SECS);
        let end = format_day(today - 1);
        let start = format_day(today - LOOKBACK_DAYS);
        let rows = self
            .reporting
            .experiment_rows(experiment_id, &start, &end)
            .await?;
        // The read is recorded as an observation; a day becomes final only
        // when a later, independent read is accepted by the store. The first
        // read is never relabelled as the second.
        let observations = observations_from_rows(&rows, observed_at)?;
        let mut evidence = self.evidence.lock().map_err(|_| {
            self.schema(
                "experiment_days",
                "the pull-evidence store lock is poisoned",
            )
        })?;
        evidence
            .record_read(instance_key, observations)
            .map_err(|error| self.schema("experiment_days", error))
    }

    async fn experiment_players(
        &self,
        experiment_id: &str,
        _instance_key: &str,
    ) -> Result<crate::ops::PlayerEvidence, PlatformError> {
        // D2 is the publisher-level, player-keyed feed: fetch one UTC day per
        // call and fold the rows into player-days. The arm label must come
        // with the feed (the documented user report has none); an unlabelled
        // feed refuses rather than inventing a split.
        let unit = self.management.get_ad_unit(experiment_id).await?;
        let platform = unit.platform.ok_or_else(|| {
            self.schema(
                "experiment_players",
                "the ad unit has no platform for the user revenue feed",
            )
        })?;
        let application = unit.package_name.ok_or_else(|| {
            self.schema(
                "experiment_players",
                "the ad unit has no package/bundle id for the user revenue feed",
            )
        })?;
        let observed_at = self.wall.now_unix_s();
        let today = observed_at.div_euclid(DAY_SECS);
        let mut feed = apex_lift::player_store::PlayerFeed::default();
        let mut labels = BTreeSet::new();
        for offset in (1..=PLAYER_LOOKBACK_DAYS).rev() {
            let day = format_day(today - offset);
            let day_rows = match self
                .reporting
                .user_revenue_day(&day, &platform, &application)
                .await
            {
                Ok(rows) => rows,
                Err(_) => continue,
            };
            for row in day_rows {
                let Some(label) = row.test_group.as_deref() else {
                    return Err(self.schema(
                        "experiment_players",
                        "the user revenue feed carries no experiment arm label; use the labelled postback feed",
                    ));
                };
                let label = label.trim();
                let is_control = label.is_empty() || label.eq_ignore_ascii_case("Control");
                if !is_control {
                    labels.insert(label.to_string());
                }
                feed.ingest(&[apex_lift::player_store::PlayerDay {
                    day: row.day,
                    user_id: row.user_id,
                    idfv: row.idfv,
                    idfa: row.idfa,
                    test: !is_control,
                    revenue_micros: row.revenue,
                }]);
            }
        }
        if labels.len() > 1 {
            return Err(self.schema(
                "experiment_players",
                format!(
                    "the user revenue feed reports {} test labels ({}); the experiment has exactly one test arm",
                    labels.len(),
                    labels.into_iter().collect::<Vec<_>>().join(", ")
                ),
            ));
        }
        feed.retain_last_days(apex_lift::player_store::MAX_PLAYER_HISTORY_DAYS);
        Ok(crate::ops::PlayerEvidence {
            rows: feed.rows(),
            pretest: BTreeMap::new(),
            // The user-level revenue API is application-scoped; the billing
            // scope is the ad unit. The runner refuses the fee until these
            // two scopes are the same.
            measurement_scope: Some(format!("application:{application}")),
            roster: None,
            eligible: None,
            certified_gain_lower: None,
            certified_delta_lower: None,
        })
    }

    async fn scope_revenue(
        &self,
        ad_unit_id: &str,
        start: &str,
        end: &str,
    ) -> Result<apex_core::Micros, PlatformError> {
        self.reporting.ad_unit_revenue(ad_unit_id, start, end).await
    }

    async fn current_settings(&self, ad_unit_id: &str) -> Result<String, PlatformError> {
        let unit = self.management.get_ad_unit(ad_unit_id).await?;
        let mut floors = unit.country_floors.unwrap_or_default();
        floors.sort_by(|a, b| a.country.cmp(&b.country));
        serde_json::to_string(&floors).map_err(|error| {
            self.schema("getAdUnit", format!("floors are not serializable: {error}"))
        })
    }

    async fn promote_experiment(&self, experiment_id: &str) -> Result<(), PlatformError> {
        self.management.promote_experiment(experiment_id).await
    }

    async fn deprecate_experiment(&self, experiment_id: &str) -> Result<(), PlatformError> {
        self.management.deprecate_experiment(experiment_id).await
    }

    async fn apply(&self, _action: &crate::model::Action, _key: &str) -> Result<(), PlatformError> {
        Err(self.schema(
            "apply",
            "a MAX change rides the experiment's test arm; the live unit is never written",
        ))
    }

    async fn rollback(&self, _action: &crate::model::Action) -> Result<(), PlatformError> {
        Err(self.schema(
            "rollback",
            "a MAX change rides the experiment's test arm; there is no live write to undo",
        ))
    }

    fn call_log(&self) -> Vec<String> {
        Vec::new()
    }
}

/// One read's per-day revenue, split into the two arms.
type ArmRevenue = BTreeMap<String, i64>;
/// A folded reporting read: `(control, test)`.
type FoldedRevenue = (ArmRevenue, ArmRevenue);
/// One arm-day observation from a reporting read.
type ArmObservation = (
    crate::pull_evidence::EvidenceArm,
    String,
    apex_lift::window::PullObservation,
);

/// Split reporting rows into the control and test arms by their
/// `max_ad_unit_test` label. The control arm is the empty label or the
/// literal `Control` (case-insensitive, trimmed) that the reporting docs use
/// in their own example; any other label is the test arm. The experiment
/// defines exactly two arms, so a second distinct non-empty test label fails
/// the read typed, naming the labels seen, rather than folding rows into the
/// wrong arm. A day's revenue is the sum of its rows; a provider row that
/// pushes the sum past i64 micros fails the pull with a typed error instead
/// of wrapping or panicking.
///
/// The experiment's own test-group name is not available at this call site:
/// the start call sends the experiment name and allocation, and MAX assigns
/// the reported label, so the label rule above is the contract. If the spec's
/// test-group name ever reaches this function, matching it explicitly would
/// take precedence over the `Control` literal for the test arm.
fn fold_rows(rows: &[ReportRow]) -> Result<FoldedRevenue, PlatformError> {
    let mut control: BTreeMap<String, i64> = BTreeMap::new();
    let mut test: BTreeMap<String, i64> = BTreeMap::new();
    let mut labels_seen: BTreeSet<String> = BTreeSet::new();
    let mut test_label: Option<String> = None;
    for row in rows {
        let label = row.test_group.as_deref().unwrap_or("").trim();
        if !label.is_empty() {
            labels_seen.insert(label.to_string());
        }
        let is_control = label.is_empty() || label.eq_ignore_ascii_case("Control");
        if !is_control {
            match test_label.as_deref() {
                None => test_label = Some(label.to_string()),
                Some(seen) if seen == label => {}
                Some(_) => {
                    let labels: Vec<&str> = labels_seen.iter().map(String::as_str).collect();
                    return Err(PlatformError::Schema {
                        platform: PlatformId::AppLovinMax,
                        operation: "experiment_days".into(),
                        detail: format!(
                            "max_ad_unit_test reports {} distinct non-empty labels ({}) but the experiment defines exactly two arms",
                            labels.len(),
                            labels.join(", ")
                        ),
                    });
                }
            }
        }
        let map = if is_control { &mut control } else { &mut test };
        let total = map.entry(row.day.clone()).or_default();
        *total = total
            .checked_add(row.revenue)
            .ok_or_else(|| PlatformError::Schema {
                platform: PlatformId::AppLovinMax,
                operation: "experiment_days".into(),
                detail: format!("revenue rows for {} overflow i64 micros", row.day),
            })?;
    }
    // A day one arm reported stays present on the other arm as zero, the
    // shared day shape the window expects.
    let days: BTreeSet<String> = control.keys().chain(test.keys()).cloned().collect();
    for day in days {
        control.entry(day.clone()).or_insert(0);
        test.entry(day).or_insert(0);
    }
    Ok((control, test))
}

/// Turn one reporting read into arm-day observations. MAX reports no user
/// count, so `users` and `fill` stay absent; the window's MAX value branch
/// divides revenue by the allocated share.
fn observations_from_rows(
    rows: &[ReportRow],
    observed_at_unix_s: i64,
) -> Result<Vec<ArmObservation>, PlatformError> {
    let (control, test) = fold_rows(rows)?;
    let days: BTreeSet<&String> = control.keys().chain(test.keys()).collect();
    let mut observations = Vec::with_capacity(days.len() * 2);
    for day in days {
        for (arm, map) in [
            (crate::pull_evidence::EvidenceArm::Control, &control),
            (crate::pull_evidence::EvidenceArm::Test, &test),
        ] {
            let revenue = map.get(day).copied().unwrap_or(0);
            observations.push((
                arm,
                day.clone(),
                apex_lift::window::PullObservation {
                    revenue,
                    users: None,
                    fill: None,
                    observed_at_unix_s,
                    source_sha256: apex_lift::window::arm_day_source_sha256(
                        day,
                        arm.as_str(),
                        revenue,
                        None,
                        None,
                    ),
                },
            ));
        }
    }
    Ok(observations)
}

use crate::calendar::format_day;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use apex_connectors::transport::{HttpRequest, HttpResponse, Transport, TransportError};
    use apex_core::{ExperimentSpec, ExperimentState};

    use super::*;
    use crate::calendar::day_number;

    /// A transport that answers from a fixed queue and records the requests.
    #[derive(Debug, Default)]
    struct FakeTransport {
        responses: Mutex<Vec<HttpResponse>>,
        seen: Mutex<Vec<HttpRequest>>,
    }

    impl FakeTransport {
        fn push(&self, status: u16, body: serde_json::Value) -> &Self {
            self.responses.lock().unwrap().push(HttpResponse {
                status,
                headers: vec![],
                body: serde_json::to_vec(&body).unwrap(),
            });
            self
        }
    }

    #[async_trait]
    impl Transport for FakeTransport {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.seen.lock().unwrap().push(req);
            Ok(self.responses.lock().unwrap().remove(0))
        }
    }

    #[derive(Debug)]
    struct StepWall(std::sync::atomic::AtomicI64);
    impl StepWall {
        fn new(unix_s: i64) -> Self {
            Self(std::sync::atomic::AtomicI64::new(unix_s))
        }
        fn set(&self, unix_s: i64) {
            self.0.store(unix_s, std::sync::atomic::Ordering::SeqCst);
        }
    }
    impl Wall for StepWall {
        fn now_unix_s(&self) -> i64 {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    fn ops_with_wall(
        transport: Arc<FakeTransport>,
        writable: bool,
        wall: Arc<StepWall>,
    ) -> MaxPlatformOps {
        let write = if writable {
            vec!["setCountryFloor".to_string()]
        } else {
            vec![]
        };
        MaxPlatformOps::new(
            "api-key",
            "report-key",
            "https://o.example.test/mediation/v1",
            "https://r.example.test",
            transport,
            wall,
            write,
        )
        .expect("adapter")
    }

    fn ops(transport: Arc<FakeTransport>, writable: bool) -> MaxPlatformOps {
        ops_with_wall(transport, writable, Arc::new(StepWall::new(1_800_000_000)))
    }

    fn spec() -> ExperimentSpec {
        ExperimentSpec {
            name: "autopilot-floor-u1-ee-2-5".into(),
            instance_id: String::new(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "raise the EE floor to 2.5".into(),
            test_group_allocation: 25,
        }
    }

    #[test]
    fn max_can_create_a_platform_holdout() {
        let adapter = ops(Arc::new(FakeTransport::default()), false);
        assert!(adapter.holdout_supported());
        assert!(
            adapter.supports_player_feed(),
            "MAX registers a per-player feed"
        );
    }

    #[test]
    fn day_formatting_round_trips_across_epoch_boundaries() {
        for day in [-1_000_i64, -1, 0, 1, 20_000, 19_723] {
            let text = format_day(day);
            assert_eq!(day_number(&text), Some(day), "{day} -> {text}");
        }
        assert_eq!(format_day(0), "1970-01-01");
        assert_eq!(day_number("not-a-day"), None);
        assert_eq!(day_number("2026-13-01"), None);
    }

    #[test]
    fn arms_split_on_the_control_label_and_one_read_has_no_second_pull() {
        let today = 20_000;
        let rows = vec![
            row_for(&format_day(today - 2), "Control", 900),
            row_for(&format_day(today - 2), "Control", 50),
            row_for(&format_day(today - 2), "Rewarded_B", 350),
            row_for(&format_day(today - 1), "Rewarded_B", 100),
            row_for(&format_day(today), "Rewarded_B", 5),
        ];
        let (control, test) = fold_rows(&rows).expect("fold");
        let older = format_day(today - 2);
        assert_eq!(control.get(&older), Some(&950), "same-day rows sum per arm");
        assert_eq!(test.get(&older), Some(&350));
        assert_eq!(test.get(&format_day(today - 1)), Some(&100));
        // A single read is the first observation only; the store never
        // relabels it as a second pull.
        let observations = observations_from_rows(&rows, today * DAY_SECS).expect("observations");
        assert_eq!(observations.len(), 6, "three days times two arms");
        let mut store = crate::pull_evidence::PullEvidenceStore::in_memory("test");
        let days = store.record_read("exp", observations).expect("record");
        assert_eq!(days.control[0].revenue, 950);
        assert_eq!(days.control[0].second_pull_revenue, None);
        assert_eq!(days.test[0].second_pull_revenue, None);
    }

    fn row_for(day: &str, label: &str, revenue: i64) -> ReportRow {
        ReportRow {
            day: day.to_string(),
            country: None,
            ad_unit_id: Some("u1".into()),
            test_group: Some(label.to_string()),
            revenue,
            impressions: 1,
        }
    }

    #[test]
    fn control_spellings_and_blanks_route_to_the_control_arm() {
        let today = 20_000;
        let day = format_day(today - 1);
        let rows: Vec<ReportRow> = ["control", "CONTROL", " Control ", " "]
            .iter()
            .map(|label| ReportRow {
                day: day.clone(),
                country: None,
                ad_unit_id: Some("u1".into()),
                test_group: Some((*label).to_string()),
                revenue: 100,
                impressions: 1,
            })
            .collect();
        let (control, test) = fold_rows(&rows).expect("fold");
        let day = format_day(today - 1);
        assert_eq!(
            control.get(&day),
            Some(&400),
            "every spelling sums as control"
        );
        assert_eq!(test.get(&day), Some(&0), "nothing lands in the test arm");
    }

    #[test]
    fn a_second_distinct_test_label_fails_the_read_typed() {
        fn row(label: &str, revenue: i64) -> ReportRow {
            ReportRow {
                day: "2026-10-01".into(),
                country: None,
                ad_unit_id: Some("u1".into()),
                test_group: Some(label.to_string()),
                revenue,
                impressions: 1,
            }
        }
        let rows = vec![
            row("Control", 100),
            row("Rewarded_B", 200),
            row("Rewarded_C", 300),
        ];
        let error = fold_rows(&rows).expect_err("a third label");
        match &error {
            PlatformError::Schema {
                operation, detail, ..
            } => {
                assert_eq!(operation, "experiment_days");
                for label in ["Control", "Rewarded_B", "Rewarded_C"] {
                    assert!(detail.contains(label), "names {label}: {detail}");
                }
            }
            other => panic!("expected a typed Schema error, got {other:?}"),
        }
        assert!(
            !error.is_retryable(),
            "a mislabeled report is not retryable"
        );

        // Two test labels without a control row are the same defect.
        let rows = vec![row("Rewarded_B", 200), row("Rewarded_C", 300)];
        let error = fold_rows(&rows).expect_err("two test labels");
        assert!(error.to_string().contains("Rewarded_B"), "{error}");
        assert!(error.to_string().contains("Rewarded_C"), "{error}");
    }

    #[test]
    fn a_single_label_leaves_the_other_arm_zero_filled() {
        let today = 20_000;
        let rows = vec![ReportRow {
            day: format_day(today - 1),
            country: None,
            ad_unit_id: Some("u1".into()),
            test_group: Some(String::new()),
            revenue: 700,
            impressions: 2,
        }];
        let (control, test) = fold_rows(&rows).expect("fold");
        let day = format_day(today - 1);
        assert_eq!(control.get(&day), Some(&700));
        assert_eq!(test.get(&day), Some(&0), "a shared day shape");
        let observations = observations_from_rows(&rows, today * DAY_SECS).expect("observations");
        let mut store = crate::pull_evidence::PullEvidenceStore::in_memory("test");
        let days = store.record_read("exp", observations).expect("record");
        assert_eq!(days.test[0].revenue, 0);
        assert_eq!(days.test[0].second_pull_revenue, None);
    }

    #[test]
    fn an_overflowing_day_revenue_fails_the_pull_typed() {
        // Two provider rows for one arm-day whose micros sum past i64::MAX:
        // the pull fails with a typed error naming the day, never a wrapped
        // revenue.
        let rows = vec![
            ReportRow {
                day: "2026-10-01".into(),
                country: None,
                ad_unit_id: Some("u1".into()),
                test_group: Some(String::new()),
                revenue: i64::MAX,
                impressions: 1,
            },
            ReportRow {
                day: "2026-10-01".into(),
                country: None,
                ad_unit_id: Some("u1".into()),
                test_group: Some(String::new()),
                revenue: 1,
                impressions: 1,
            },
        ];
        let error = fold_rows(&rows).expect_err("overflow");
        assert!(error.to_string().contains("2026-10-01"), "{error}");
        assert!(!error.is_retryable(), "a wrapped total is not retryable");
    }

    #[tokio::test]
    async fn the_whole_adapter_surface_is_exercised() {
        let transport = Arc::new(FakeTransport::default());
        // start_null_experiment: create an experiment with no floor change.
        transport.push(200, serde_json::json!({"id": "u1"}));
        // scope_revenue: a maxReport with two rows for the unit.
        transport.push(
            200,
            serde_json::json!({
                "code": 200,
                "results": [
                    {"day": "2026-10-01", "max_ad_unit_id": "u1", "estimated_revenue": "1.5", "impressions": "10"},
                    {"day": "2026-10-02", "max_ad_unit_id": "u1", "estimated_revenue": "2.5", "impressions": "10"},
                    {"day": "2026-10-02", "max_ad_unit_id": "u2", "estimated_revenue": "9.0", "impressions": "10"}
                ]
            }),
        );
        // current_settings: the unit's floors, unsorted on the wire.
        transport.push(
            200,
            serde_json::json!({
                "id": "u1",
                "name": "Rewarded",
                "ad_format": "REWARD",
                "bid_floors": [
                    {"country_group_name": "b", "cpm": "2.0", "countries": {"values": ["us"]}},
                    {"country_group_name": "a", "cpm": "1.0", "countries": {"values": ["ee"]}}
                ]
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);

        assert!(adapter.change_in_test_arm());
        assert!(adapter.call_log().is_empty());
        assert!(adapter
            .apply(
                &crate::model::Action::RemoveWaterfallEntry {
                    ad_unit_id: "u".into(),
                    entry_id: "e".into(),
                },
                "k"
            )
            .await
            .is_err());
        assert!(adapter
            .rollback(&crate::model::Action::RemoveWaterfallEntry {
                ad_unit_id: "u".into(),
                entry_id: "e".into(),
            })
            .await
            .is_err());

        let id = adapter
            .start_null_experiment(&spec())
            .await
            .expect("null experiment");
        assert_eq!(id, "u1");

        let revenue = adapter
            .scope_revenue("u1", "2026-10-01", "2026-10-02")
            .await
            .expect("revenue");
        assert_eq!(revenue, 4_000_000, "only u1 rows are summed");

        let settings = adapter.current_settings("u1").await.expect("settings");
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&settings).expect("json");
        assert_eq!(parsed.len(), 2);
        let countries: Vec<&str> = parsed
            .iter()
            .filter_map(|floor| floor.get("country").and_then(|v| v.as_str()))
            .collect();
        assert!(
            countries.windows(2).all(|w| w[0] <= w[1]),
            "sorted: {countries:?}"
        );

        assert!(SystemWall.now_unix_s() > 1_700_000_000);
        assert!(format!("{adapter:?}").contains("MaxPlatformOps"));
    }

    #[tokio::test]
    async fn writes_are_refused_without_the_allow_list() {
        let transport = Arc::new(FakeTransport::default());
        let adapter = ops(Arc::clone(&transport), false);
        assert!(!adapter.writable("setCountryFloor").await);
        assert!(transport.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn start_experiment_reads_floors_and_folds_the_change_into_the_test_arm() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "id": "u1",
                "name": "Rewarded",
                "ad_format": "REWARD",
                "platform": "android",
                "package_name": "com.example.app",
                "bid_floors": [
                    {"country_group_name": "default", "cpm": "1.0", "countries": {"values": ["us"]}}
                ]
            }),
        );
        transport.push(200, serde_json::json!({"id": "u1"}));
        let adapter = ops(Arc::clone(&transport), true);
        assert!(adapter.writable("setCountryFloor").await);

        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::CountryFloorSet {
                country: "ee".into(),
                floor_ecpm: 2_500_000,
            },
            prev_value: "{}".into(),
        };
        let id = adapter.start_experiment(&request).await.expect("start");
        assert_eq!(id, "u1");

        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].url.contains("/ad_unit/u1"));
        assert!(seen[1].url.contains("/ad_unit_experiment/u1"));
        let body = match seen[1].body.as_ref().expect("body") {
            apex_connectors::transport::HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        let floors = body["bid_floors"].as_array().expect("floors");
        // The existing group keeps its own cpm for its own country, and the
        // changed country is isolated so the group fold (first floor wins)
        // can neither drop the new 2.5 floor nor apply it to `us`.
        assert_eq!(floors.len(), 2, "the changed country gets its own group");
        let original = &floors[0];
        assert_eq!(original["country_group_name"], "default");
        assert_eq!(original["cpm"], "1", "us keeps its floor");
        let original_countries = original["countries"]["values"]
            .as_array()
            .expect("countries");
        assert_eq!(original_countries, &vec![serde_json::json!("us")]);
        let isolated = &floors[1];
        assert_eq!(
            isolated["country_group_name"], "apex-default-ee",
            "a deterministic singleton group"
        );
        assert_eq!(isolated["cpm"], "2.5", "the change actually reaches MAX");
        let isolated_countries = isolated["countries"]["values"]
            .as_array()
            .expect("countries");
        assert_eq!(isolated_countries, &vec![serde_json::json!("ee")]);
    }

    #[tokio::test]
    async fn a_country_in_a_shared_group_is_isolated_and_a_solo_country_keeps_its_group() {
        // Shared group: changing `ee` must not change `us`.
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "id": "u1",
                "name": "Rewarded",
                "ad_format": "REWARD",
                "platform": "android",
                "package_name": "com.example.app",
                "bid_floors": [
                    {
                        "country_group_name": "eea",
                        "cpm": "1.0",
                        "countries": {"values": ["us", "ee"]}
                    }
                ]
            }),
        );
        transport.push(200, serde_json::json!({"id": "u1"}));
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: "e".into(),
            spec: spec(),
            change: ChangeKind::CountryFloorSet {
                country: "ee".into(),
                floor_ecpm: 2_500_000,
            },
            prev_value: "{}".into(),
        };
        adapter.start_experiment(&request).await.expect("start");
        let body = {
            let seen = transport.seen.lock().unwrap();
            match seen[1].body.as_ref().expect("body") {
                apex_connectors::transport::HttpBody::Json(value) => value.clone(),
                other => panic!("expected JSON body, got {other:?}"),
            }
        };
        let floors = body["bid_floors"].as_array().expect("floors");
        assert_eq!(floors.len(), 2, "ee isolated from the shared group");
        assert_eq!(floors[0]["country_group_name"], "eea");
        assert_eq!(floors[0]["cpm"], "1", "us is collateralized by nothing");
        assert_eq!(floors[0]["countries"]["values"], serde_json::json!(["us"]));
        assert_eq!(floors[1]["country_group_name"], "apex-eea-ee");
        assert_eq!(floors[1]["cpm"], "2.5");
        assert_eq!(floors[1]["countries"]["values"], serde_json::json!(["ee"]));

        // A country alone in its group changes in place: nothing to isolate.
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "id": "u1",
                "name": "Rewarded",
                "ad_format": "REWARD",
                "platform": "android",
                "package_name": "com.example.app",
                "bid_floors": [
                    {
                        "country_group_name": "solo",
                        "cpm": "1.0",
                        "countries": {"values": ["ee"]}
                    }
                ]
            }),
        );
        transport.push(200, serde_json::json!({"id": "u1"}));
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: "e2".into(),
            spec: spec(),
            change: ChangeKind::CountryFloorSet {
                country: "ee".into(),
                floor_ecpm: 3_000_000,
            },
            prev_value: "{}".into(),
        };
        adapter.start_experiment(&request).await.expect("start");
        let body = {
            let seen = transport.seen.lock().unwrap();
            match seen[1].body.as_ref().expect("body") {
                apex_connectors::transport::HttpBody::Json(value) => value.clone(),
                other => panic!("expected JSON body, got {other:?}"),
            }
        };
        let floors = body["bid_floors"].as_array().expect("floors");
        assert_eq!(floors.len(), 1, "a solo country keeps its own group");
        assert_eq!(floors[0]["country_group_name"], "solo");
        assert_eq!(floors[0]["cpm"], "3", "and the change reaches MAX");
    }

    #[tokio::test]
    async fn experiment_reads_state_and_promote_deprecate_route_correctly() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({"id": "u1", "promote": false, "deprecate": false}),
        );
        transport.push(
            200,
            serde_json::json!({"id": "u1", "experiment_name": "promote"}),
        );
        transport.push(200, serde_json::json!({}));
        transport.push(
            200,
            serde_json::json!({"id": "u1", "experiment_name": "deprecate"}),
        );
        transport.push(200, serde_json::json!({}));
        let adapter = ops(Arc::clone(&transport), true);

        let state = adapter.experiment("u1").await.expect("experiment");
        assert_eq!(state.state, ExperimentState::Running);
        adapter.promote_experiment("u1").await.expect("promote");
        adapter.deprecate_experiment("u1").await.expect("deprecate");

        let seen = transport.seen.lock().unwrap();
        let writes: Vec<&HttpRequest> = seen
            .iter()
            .filter(|r| r.method == apex_connectors::transport::HttpMethod::Post)
            .collect();
        assert_eq!(writes.len(), 2);
        let promote_body = match writes[0].body.as_ref().unwrap() {
            apex_connectors::transport::HttpBody::Json(value) => value,
            other => panic!("expected JSON, got {other:?}"),
        };
        assert_eq!(promote_body["promote"], serde_json::json!(true));
        let deprecate_body = match writes[1].body.as_ref().unwrap() {
            apex_connectors::transport::HttpBody::Json(value) => value,
            other => panic!("expected JSON, got {other:?}"),
        };
        assert_eq!(deprecate_body["deprecate"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn a_non_floor_change_is_refused_before_any_write() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({"id": "u1", "name": "n", "ad_format": "REWARD"}),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: "e".into(),
            spec: spec(),
            change: ChangeKind::EcpmSet {
                mediation_group_id: "g".into(),
                ad_unit_id: "u1".into(),
                ecpm: 1,
            },
            prev_value: "{}".into(),
        };
        assert!(adapter.start_experiment(&request).await.is_err());
        assert_eq!(
            transport.seen.lock().unwrap().len(),
            0,
            "a non-floor/non-cap change is refused before any call"
        );
    }

    #[tokio::test]
    async fn experiment_days_pull_the_reporting_window() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "code": 200,
                "count": 1,
                "results": [{
                    "day": "2027-01-14",
                    "max_ad_unit_id": "u1",
                    "max_ad_unit_test": "",
                    "estimated_revenue": "1.25",
                    "impressions": "100"
                }]
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let days = adapter.experiment_days("u1").await.expect("days");
        assert_eq!(days.control.len(), 1);
        assert_eq!(days.control[0].revenue, 1_250_000);
        assert_eq!(
            days.control[0].second_pull_revenue, None,
            "one read is never finality evidence"
        );
        let seen = transport.seen.lock().unwrap();
        assert!(seen[0].url.starts_with("https://r.example.test/maxReport?"));
        assert!(seen[0].url.contains("start=2026-11-13"));
        assert!(seen[0].url.contains("end=2027-01-14"));
    }

    fn max_report(day: &str, control_revenue: &str, test_revenue: &str) -> serde_json::Value {
        serde_json::json!({
            "code": 200,
            "count": 2,
            "results": [
                {
                    "day": day,
                    "max_ad_unit_id": "u1",
                    "max_ad_unit_test": "",
                    "estimated_revenue": control_revenue,
                    "impressions": "10"
                },
                {
                    "day": day,
                    "max_ad_unit_id": "u1",
                    "max_ad_unit_test": "Rewarded_B",
                    "estimated_revenue": test_revenue,
                    "impressions": "10"
                }
            ]
        })
    }

    #[tokio::test]
    async fn two_pulls_need_a_distinct_read_at_least_twenty_four_hours_later() {
        let now = 1_800_000_000_i64;
        let today = now.div_euclid(DAY_SECS);
        let day = format_day(today - 2);
        let transport = Arc::new(FakeTransport::default());
        let wall = Arc::new(StepWall::new(now));
        let adapter = ops_with_wall(Arc::clone(&transport), true, Arc::clone(&wall));

        // First read.
        transport.push(200, max_report(&day, "1.00", "2.00"));
        let first = adapter.experiment_days("u1").await.expect("first read");
        assert_eq!(first.control[0].revenue, 1_000_000);
        assert_eq!(first.control[0].second_pull_revenue, None);

        // The same read replayed is rejected, even if it arrives later in the
        // process: its identity (observed-at and source hash) is unchanged.
        transport.push(200, max_report(&day, "1.00", "2.00"));
        let same = adapter.experiment_days("u1").await.expect("same read");
        assert_eq!(same.control[0].second_pull_revenue, None);

        // A changed read one hour later is still too early.
        wall.set(now + 3_600);
        transport.push(200, max_report(&day, "1.10", "2.20"));
        let soon = adapter.experiment_days("u1").await.expect("early read");
        assert_eq!(
            soon.control[0].revenue, 1_000_000,
            "the first observation stays the baseline"
        );
        assert_eq!(soon.control[0].second_pull_revenue, None);

        // A distinct read at least 24 hours after the first is accepted and
        // carries the second read's values and source hash.
        wall.set(now + crate::pull_evidence::SECOND_PULL_MIN_SECS);
        transport.push(200, max_report(&day, "1.10", "2.20"));
        let final_days = adapter.experiment_days("u1").await.expect("second read");
        assert_eq!(final_days.control[0].revenue, 1_000_000);
        assert_eq!(final_days.control[0].second_pull_revenue, Some(1_100_000));
        assert_eq!(final_days.control[0].second_pull_users, None);
        assert_eq!(final_days.test[0].second_pull_revenue, Some(2_200_000));
        let first_hash = final_days.control[0]
            .first_source_sha256
            .clone()
            .expect("first hash");
        let second_hash = final_days.control[0]
            .second_source_sha256
            .clone()
            .expect("second hash");
        assert_ne!(first_hash, second_hash, "two different reporting reads");
        assert!(final_days.control[0].second_observed_at_unix_s.is_some());
    }

    #[tokio::test]
    async fn the_second_pull_survives_a_process_restart() {
        let now = 1_800_000_000_i64;
        let today = now.div_euclid(DAY_SECS);
        let day = format_day(today - 2);
        let root = std::env::temp_dir().join(format!(
            "apex-max-evidence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        {
            let transport = Arc::new(FakeTransport::default());
            transport.push(200, max_report(&day, "1.00", "2.00"));
            let adapter = ops_with_wall(Arc::clone(&transport), true, Arc::new(StepWall::new(now)))
                .with_evidence_root(&root)
                .expect("durable store");
            let first = adapter.experiment_days("u1").await.expect("first read");
            assert_eq!(first.control[0].second_pull_revenue, None);
        }
        {
            let transport = Arc::new(FakeTransport::default());
            transport.push(200, max_report(&day, "1.00", "2.00"));
            let adapter = ops_with_wall(
                Arc::clone(&transport),
                true,
                Arc::new(StepWall::new(
                    now + crate::pull_evidence::SECOND_PULL_MIN_SECS,
                )),
            )
            .with_evidence_root(&root)
            .expect("durable store");
            let second = adapter.experiment_days("u1").await.expect("second read");
            assert_eq!(second.control[0].second_pull_revenue, Some(1_000_000));
            assert!(second.control[0].second_source_sha256.is_some());
        }
        std::fs::remove_dir_all(&root).expect("clean");
    }
    #[tokio::test]
    async fn a_frequency_cap_start_writes_the_cap_into_the_test_group_only() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, serde_json::json!({"id": "u1"}));
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::FrequencyCapSet {
                ad_unit_id: "u1".into(),
                cap_type: "session".into(),
                countries: vec!["us".into(), "ca".into()],
                session_limit: Some(4),
                day_limit: None,
                minute_frequency: None,
            },
            prev_value: "{}".into(),
        };
        let id = adapter.start_experiment(&request).await.expect("start");
        assert_eq!(id, "u1");
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "no ad-unit read is needed for a cap write");
        assert!(seen[0].url.contains("/ad_unit_experiment/u1"));
        let body = match seen[0].body.as_ref().expect("body") {
            apex_connectors::transport::HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        assert_eq!(body["test_group_allocation"], 25);
        let caps = body["frequency_capping_settings"].as_array().expect("caps");
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0]["type"], "session");
        assert_eq!(caps[0]["session_capping_settings"]["session_limit"], 4);
        assert_eq!(caps[0]["time_capping_settings"]["day_limit"], 0);
        assert_eq!(
            caps[0]["countries"]["values"],
            serde_json::json!(["us", "ca"])
        );
    }

    #[tokio::test]
    async fn a_frequency_cap_with_duplicate_countries_is_refused_before_the_write() {
        let transport = Arc::new(FakeTransport::default());
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::FrequencyCapSet {
                ad_unit_id: "u1".into(),
                cap_type: "time".into(),
                countries: vec!["us".into(), "US".into()],
                session_limit: None,
                day_limit: Some(10),
                minute_frequency: Some(30),
            },
            prev_value: "{}".into(),
        };
        assert!(adapter.start_experiment(&request).await.is_err());
        assert!(transport.seen.lock().unwrap().is_empty());
    }
}
