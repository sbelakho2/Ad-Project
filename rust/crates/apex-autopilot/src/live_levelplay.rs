//! The live LevelPlay write adapter: the production [`PlatformOps`] for
//! Unity LevelPlay publishers.
//!
//! LevelPlay documents A/B tests only in its publisher dashboard, so the
//! publisher starts the test there once (a control group `abTest: "A"` and a
//! variant group `abTest: "B"`). The runner's part is deliberately narrow:
//!
//! - it changes **only group B**: every write first reads the app's groups
//!   through the connectors' groups v4 call and requires exactly one variant
//!   group (the typed refusal otherwise), and an entry-targeting write is
//!   refused when the target is not among that group's instances — the
//!   control group A is never mutated;
//! - it reads A against B per day through the reporting v1 call with the
//!   `date` and `abTest` breakdowns and the `revenue` and `activeUsers`
//!   metrics, one call over the trailing window, oldest first;
//! - it never promotes or deprecates through the API: the winning variant is
//!   chosen in the dashboard, and the adapter refuses the promotion typed
//!   instead of fabricating it. The reporting revenue is also the scope
//!   revenue the monthly pass bills from.
//!
//! # The daily fold
//!
//! Each reporting row carries a day, an arm, revenue (micros) and active
//! users. The adapter carries the arm's daily revenue as the arm's value
//! (`ArmDay.revenue`) and the arm's active users as `ArmDay.users`, so the
//! lift window's existing users branch computes revenue per user — the same
//! measured quantity the pre-registered `revenue_per_user` metric names. A
//! row missing either field is already a typed connector refusal; this layer
//! only folds rows it has.
//!
//! Rows split strictly: `A` is the control arm, `B` the test arm, and any
//! other label fails the pull typed, naming it. A day is final only when a
//! later independent reporting read at least 24 hours after the first agrees,
//! and a day where only one arm reported gets the other arm zero-filled — both
//! exactly the MAX conventions.
//!
//! Every write is gated by a caller-supplied allow-list (the verify-live
//! gate): by default the adapter refuses all writes rather than guessing that
//! an endpoint is safe.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use apex_connectors::levelplay::{AbTestReportRow, LevelPlayClient};
use apex_connectors::transport::Transport;
use apex_core::{ExperimentResult, ExperimentState, PlatformError, PlatformId};
use apex_proof::ChangeKind;
use async_trait::async_trait;

use crate::live_max::Wall;
use crate::ops::{ExperimentDays, ExperimentRequest, PlatformOps, PromotionStatus};

/// How many trailing days the adapter asks the reporting API for; generous
/// enough for any allowed window plus burn-in (the same bound as MAX).
const LOOKBACK_DAYS: i64 = 63;
const DAY_SECS: i64 = 86_400;

pub struct LevelPlayPlatformOps {
    management: LevelPlayClient,
    wall: Arc<dyn Wall>,
    /// Verify-live allow-list: an operation here (`updateEntryPrice`,
    /// `createEntry`, `removeEntry`) is writable; anything else is refused
    /// before any side effect.
    writable: BTreeMap<String, bool>,
    /// The durable two-pull evidence for every arm-day this adapter reads.
    evidence: std::sync::Mutex<crate::pull_evidence::PullEvidenceStore>,
}

/// The platform namespace used for LevelPlay pull-evidence files.
const EVIDENCE_NAMESPACE: &str = "levelplay";

impl std::fmt::Debug for LevelPlayPlatformOps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LevelPlayPlatformOps")
            .field("writable", &self.writable)
            .finish()
    }
}

impl LevelPlayPlatformOps {
    /// Build the adapter. `writable` lists the operations the operator has
    /// verified against a live sandbox; without it every write is refused.
    pub fn new(
        secret_key: &str,
        refresh_token: &str,
        app_key: &str,
        base_url: &str,
        transport: Arc<dyn Transport>,
        wall: Arc<dyn Wall>,
        writable: impl IntoIterator<Item = String>,
    ) -> Result<Self, PlatformError> {
        let management =
            LevelPlayClient::new(secret_key, refresh_token, app_key, base_url, transport)?;
        Ok(Self {
            management,
            wall,
            writable: writable.into_iter().map(|op| (op, true)).collect(),
            evidence: std::sync::Mutex::new(crate::pull_evidence::PullEvidenceStore::in_memory(
                EVIDENCE_NAMESPACE,
            )),
        })
    }

    /// Make the pull-evidence store durable under `root`.
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
            platform: PlatformId::LevelPlay,
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    /// The app's groups as the API reports them, accepting the response's
    /// envelope or a bare array.
    fn groups_array(data: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
        data.as_array()
            .or_else(|| data.get("groups").and_then(serde_json::Value::as_array))
    }

    fn is_ab_group(group: &serde_json::Value, label: &str) -> bool {
        group
            .get("abTest")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case(label))
    }

    /// The one group created with `abTest: "B"` — the only group the runner
    /// may change. Zero or several variant groups is a typed refusal, never a
    /// silent pick.
    async fn group_b(&self) -> Result<serde_json::Value, PlatformError> {
        let groups = self.groups().await?;
        let variant_groups: Vec<&serde_json::Value> = groups
            .iter()
            .filter(|group| Self::is_ab_group(group, "B"))
            .collect();
        match variant_groups.as_slice() {
            [group] => Ok((*group).clone()),
            [] => Err(self.schema(
                "startExperiment",
                "no abTest \"B\" group exists; the publisher must start the dashboard A/B test first",
            )),
            groups => {
                let ids: Vec<String> = groups
                    .iter()
                    .filter_map(|group| group.get("groupId").and_then(Self::id_string))
                    .collect();
                Err(self.schema(
                    "startExperiment",
                    format!(
                        "{} groups carry abTest \"B\" ({}) but the runner changes exactly one variant group",
                        groups.len(),
                        ids.join(", ")
                    ),
                ))
            }
        }
    }

    /// The app's groups from the groups v4 read.
    async fn groups(&self) -> Result<Vec<serde_json::Value>, PlatformError> {
        let data = self.management.list_groups().await?;
        let groups = Self::groups_array(&data)
            .ok_or_else(|| self.schema("listGroups", "response has no groups array"))?;
        Ok(groups.clone())
    }

    /// A platform id that may be a JSON string or number, rendered as the
    /// string the deterministic experiment name and ledger use.
    fn id_string(value: &serde_json::Value) -> Option<String> {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_i64().map(|number| number.to_string()))
            .or_else(|| value.as_u64().map(|number| number.to_string()))
    }

    /// The variant-arm instance that mirrors `network`/`name` from the live
    /// waterfall. The B group must own a distinct instance row; writing the
    /// control's id would change both arms.
    fn variant_member_id(group_b: &serde_json::Value, network: &str, name: &str) -> Option<String> {
        group_b
            .get("instances")
            .and_then(|value| value.as_array())
            .and_then(|members| {
                members.iter().find(|member| {
                    member.get("networkName").and_then(|value| value.as_str()) == Some(network)
                        && (member.get("name").and_then(|value| value.as_str()) == Some(name)
                            || member.get("instanceName").and_then(|value| value.as_str())
                                == Some(name))
                })
            })
            .and_then(|member| member.get("id"))
            .and_then(Self::id_string)
    }

    /// The group's configuration without its identity fields: everything a
    /// second arm must mirror for an A/A read to be honest.
    fn arm_configuration(group: &serde_json::Value) -> serde_json::Value {
        let mut object = group.as_object().cloned().unwrap_or_default();
        for key in ["groupId", "abTest", "name"] {
            object.remove(key);
        }
        serde_json::Value::Object(object)
    }
}

#[async_trait]
impl PlatformOps for LevelPlayPlatformOps {
    fn platform(&self) -> PlatformId {
        PlatformId::LevelPlay
    }

    async fn writable(&self, operation: &str) -> bool {
        self.writable.get(operation).copied().unwrap_or(false)
    }

    async fn start_experiment(&self, request: &ExperimentRequest) -> Result<String, PlatformError> {
        // Only waterfall-entry changes exist on LevelPlay; anything else is
        // refused before the group read (or any other call).
        match &request.change {
            ChangeKind::WaterfallEntryPriceSet { .. }
            | ChangeKind::WaterfallEntryCreated { .. }
            | ChangeKind::WaterfallEntryRemoved { .. } => {}
            _ => {
                return Err(self.schema(
                    "startExperiment",
                    "a LevelPlay experiment carries a waterfall-entry change only",
                ))
            }
        }
        // The A/B gate: exactly one variant group must exist; it is the only
        // group any write may target.
        let group_b = self.group_b().await?;
        let group_id = group_b
            .get("groupId")
            .and_then(Self::id_string)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| self.schema("startExperiment", "group B missing its groupId"))?
            .to_string();
        match &request.change {
            ChangeKind::WaterfallEntryPriceSet {
                ad_unit_id,
                entry_id,
                price_ecpm,
            } => {
                let live = self
                    .management
                    .list_waterfall("")
                    .await?
                    .into_iter()
                    .find(|entry| entry.entry_id == *entry_id)
                    .ok_or_else(|| {
                        self.schema(
                            "startExperiment",
                            format!("live waterfall has no entry '{entry_id}'"),
                        )
                    })?;
                let variant_id =
                    Self::variant_member_id(&group_b, &live.network, &live.instance_name)
                        .ok_or_else(|| {
                            self.schema(
                                "startExperiment",
                                format!(
                            "the variant group has no distinct instance mirroring '{entry_id}'"
                        ),
                            )
                        })?;
                self.management
                    .update_entry_price(ad_unit_id, &variant_id, *price_ecpm)
                    .await?;
            }
            ChangeKind::WaterfallEntryCreated {
                ad_unit_id,
                network,
                instance_name,
                price_ecpm,
            } => {
                self.management
                    .create_entry(ad_unit_id, network, instance_name, *price_ecpm)
                    .await?;
            }
            ChangeKind::WaterfallEntryRemoved {
                ad_unit_id,
                entry_id,
            } => {
                let live = self
                    .management
                    .list_waterfall("")
                    .await?
                    .into_iter()
                    .find(|entry| entry.entry_id == *entry_id)
                    .ok_or_else(|| {
                        self.schema(
                            "startExperiment",
                            format!("live waterfall has no entry '{entry_id}'"),
                        )
                    })?;
                let variant_id =
                    Self::variant_member_id(&group_b, &live.network, &live.instance_name)
                        .ok_or_else(|| {
                            self.schema(
                                "startExperiment",
                                format!(
                            "the variant group has no distinct instance mirroring '{entry_id}'"
                        ),
                            )
                        })?;
                self.management
                    .remove_entry(ad_unit_id, &variant_id)
                    .await?;
            }
            _ => unreachable!("the change kind was checked above"),
        }
        // The handle must be unique per proposal: the B group is shared by
        // every experiment on the app, so the pre-registered name rides with
        // it and `experiment` splits it back off.
        Ok(format!("{group_id}::{}", request.experiment_id))
    }

    fn change_in_test_arm(&self) -> bool {
        true
    }

    /// LevelPlay promotes a winning A/B test only from its dashboard.
    fn promotion_requires_manual_action(&self) -> bool {
        true
    }

    /// Pending while the variant group still exists; once it is gone, the
    /// live waterfall decides whether the treatment was promoted (the target
    /// now serves everyone) or the control was kept.
    async fn promotion_status(
        &self,
        experiment_id: &str,
        change: &ChangeKind,
    ) -> Result<PromotionStatus, PlatformError> {
        let group_id = experiment_id
            .split_once("::")
            .map(|(group, _)| group)
            .unwrap_or(experiment_id);
        let data = self.management.list_groups().await?;
        let groups = Self::groups_array(&data)
            .ok_or_else(|| self.schema("promotionStatus", "response has no groups array"))?;
        if groups.iter().any(|group| {
            group.get("groupId").and_then(Self::id_string).as_deref() == Some(group_id)
        }) {
            return Ok(PromotionStatus::Pending);
        }
        let entries = self.management.list_waterfall("").await?;
        let live = |entry_id: &str| entries.iter().find(|entry| entry.entry_id == entry_id);
        let status = match change {
            ChangeKind::WaterfallEntryPriceSet {
                entry_id,
                price_ecpm,
                ..
            } => match live(entry_id) {
                Some(entry) if entry.price_ecpm == *price_ecpm => PromotionStatus::Confirmed,
                _ => PromotionStatus::Declined,
            },
            ChangeKind::WaterfallEntryRemoved { entry_id, .. } => {
                if live(entry_id).is_none() {
                    PromotionStatus::Confirmed
                } else {
                    PromotionStatus::Declined
                }
            }
            ChangeKind::WaterfallEntryCreated {
                instance_name,
                price_ecpm,
                ..
            } => match entries
                .iter()
                .find(|entry| entry.instance_name == *instance_name)
            {
                Some(entry) if entry.price_ecpm == *price_ecpm => PromotionStatus::Confirmed,
                _ => PromotionStatus::Declined,
            },
            _ => PromotionStatus::Declined,
        };
        Ok(status)
    }

    /// LevelPlay has no create-A/A API, so the only honest null experiment is
    /// the dashboard's own test **when its two arms are provably identical**:
    /// the read adopts the existing variant group as the null experiment
    /// (nothing is written). Groups whose configurations differ refuse typed —
    /// a B group carrying a change is not an A/A, and the adapter never
    /// fabricates one.
    async fn start_null_experiment(
        &self,
        _spec: &apex_core::ExperimentSpec,
    ) -> Result<String, PlatformError> {
        let groups = self.groups().await?;
        let controls: Vec<&serde_json::Value> = groups
            .iter()
            .filter(|group| Self::is_ab_group(group, "A"))
            .collect();
        let variants: Vec<&serde_json::Value> = groups
            .iter()
            .filter(|group| Self::is_ab_group(group, "B"))
            .collect();
        let [control] = controls.as_slice() else {
            return Err(self.schema(
                "startNullExperiment",
                "an A/A read needs exactly one abTest \"A\" control group",
            ));
        };
        let [variant] = variants.as_slice() else {
            return Err(self.schema(
                "startNullExperiment",
                "an A/A read needs exactly one abTest \"B\" variant group",
            ));
        };
        if Self::arm_configuration(variant) != Self::arm_configuration(control) {
            return Err(self.schema(
                "startNullExperiment",
                "the abTest \"B\" group differs from the control; an A/A read needs identical arm configurations",
            ));
        }
        variant
            .get("groupId")
            .and_then(Self::id_string)
            .filter(|id| !id.is_empty())
            .map(|id| format!("{id}::{}", _spec.name))
            .ok_or_else(|| self.schema("startNullExperiment", "group B missing its groupId"))
    }

    /// The group B's own presence is the experiment state proxy: the
    /// dashboard ends the test by removing the variant group (or promoting
    /// it), so a present `abTest: "B"` group with this id reads running and
    /// an absent one reads stopped. The lift verdict is never read from the
    /// dashboard; only the measured counts are.
    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError> {
        let data = self.management.list_groups().await?;
        let groups = Self::groups_array(&data)
            .ok_or_else(|| self.schema("experiment", "response has no groups array"))?;
        let group_id = experiment_id
            .split_once("::")
            .map(|(group, _)| group)
            .unwrap_or(experiment_id);
        let running = groups.iter().any(|group| {
            group.get("groupId").and_then(Self::id_string).as_deref() == Some(group_id)
                && Self::is_ab_group(group, "B")
        });
        Ok(ExperimentResult {
            experiment_id: experiment_id.to_string(),
            state: if running {
                ExperimentState::Running
            } else {
                ExperimentState::Stopped
            },
            results: vec![],
            lift_percent_bp: None,
            confidence: None,
            winner_variant_id: None,
        })
    }

    /// One reporting v1 call over the trailing window (the connector rows
    /// carry their own `date`), oldest first. The window ends yesterday. Each
    /// call is one independent observation; the durable pull-evidence store
    /// decides finality from the first and second reads.
    async fn experiment_days(&self, experiment_id: &str) -> Result<ExperimentDays, PlatformError> {
        self.experiment_days_for(experiment_id, experiment_id).await
    }

    async fn experiment_days_for(
        &self,
        _experiment_id: &str,
        instance_key: &str,
    ) -> Result<ExperimentDays, PlatformError> {
        let observed_at = self.wall.now_unix_s();
        let today = observed_at.div_euclid(DAY_SECS);
        let start = crate::calendar::format_day(today - LOOKBACK_DAYS);
        let end = crate::calendar::format_day(today - 1);
        let rows = self.management.ab_test_report(&start, &end).await?;
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

    /// The scope's reported revenue over `[start, end]`, in micros. The
    /// connector is bound to one app key, so this is the app-level revenue the
    /// monthly pass bills on like MAX's per-unit report; the scope id is the
    /// pre-registered ad unit the pass grouped the kept change under.
    async fn scope_revenue(
        &self,
        _ad_unit_id: &str,
        start: &str,
        end: &str,
    ) -> Result<apex_core::Micros, PlatformError> {
        self.management.app_revenue(start, end).await
    }

    /// The live instance's price in the exact shape the pre-registration
    /// pinned (`{"priceEcpm": <micros>, "<entry id>": <micros>}`). The runner
    /// mutates group B, so reading the live waterfall is the control-arm
    /// comparison: a publisher edit to the live instance voids the
    /// experiment, while the test arm's own write does not.
    async fn current_settings(&self, ad_unit_id: &str) -> Result<String, PlatformError> {
        let entries = self.management.list_waterfall("").await?;
        let entry = entries
            .iter()
            .find(|entry| entry.entry_id == ad_unit_id)
            .ok_or_else(|| {
                self.schema(
                    "current_settings",
                    format!("live waterfall has no entry '{ad_unit_id}'"),
                )
            })?;
        let mut previous = serde_json::Map::new();
        previous.insert("priceEcpm".to_string(), serde_json::json!(entry.price_ecpm));
        previous.insert(entry.entry_id.clone(), serde_json::json!(entry.price_ecpm));
        serde_json::to_string(&previous).map_err(|error| {
            self.schema(
                "current_settings",
                format!("instance price not encodable: {error}"),
            )
        })
    }

    /// LevelPlay has no documented promote API: the dashboard applies the
    /// winning variant. The runner refuses instead of pretending.
    async fn promote_experiment(&self, _experiment_id: &str) -> Result<(), PlatformError> {
        Err(self.schema(
            "promoteExperiment",
            "LevelPlay promotes the winning arm in the publisher dashboard; no promote API is documented",
        ))
    }

    /// Ending the dashboard A/B test is deleting its variant group, which the
    /// documented v4 groups API supports. The experiment handle carries the
    /// group id before `::`.
    async fn deprecate_experiment(&self, experiment_id: &str) -> Result<(), PlatformError> {
        let group_id = experiment_id
            .split_once("::")
            .map(|(group, _)| group)
            .filter(|group| !group.is_empty())
            .ok_or_else(|| {
                self.schema(
                    "deprecateExperiment",
                    format!("experiment '{experiment_id}' does not carry a variant group id"),
                )
            })?;
        self.management.delete_group(group_id).await
    }

    async fn apply(&self, _action: &crate::model::Action, _id: &str) -> Result<(), PlatformError> {
        // The change always rides group B; nothing is ever applied live.
        Err(self.schema(
            "apply",
            "a LevelPlay change is carried by the A/B test's variant group; there is no live apply path",
        ))
    }

    async fn rollback(&self, _action: &crate::model::Action) -> Result<(), PlatformError> {
        Err(self.schema(
            "rollback",
            "a LevelPlay change is carried by the A/B test's variant group; there is no live rollback path",
        ))
    }

    fn call_log(&self) -> Vec<String> {
        Vec::new()
    }
}

type ArmTotals = BTreeMap<String, (i64, u64)>;
/// A folded reporting read: `(control, test)`.
type FoldedTotals = (ArmTotals, ArmTotals);
/// One arm-day observation from a reporting read.
type ArmObservation = (
    crate::pull_evidence::EvidenceArm,
    String,
    apex_lift::window::PullObservation,
);

/// Fold one reporting read into the two arms: `A` is control, `B` is test,
/// any other label fails typed; a day where only one arm reported zero-fills
/// the other. Each arm-day sums its rows' revenue (checked micros) and active
/// users independently, so the window's users branch sees revenue per user.
fn fold_rows(rows: &[AbTestReportRow]) -> Result<FoldedTotals, PlatformError> {
    let mut control: BTreeMap<String, (i64, u64)> = BTreeMap::new();
    let mut test: BTreeMap<String, (i64, u64)> = BTreeMap::new();
    for row in rows {
        let label = row.ab_test.trim();
        let is_control = label.eq_ignore_ascii_case("A");
        let is_test = label.eq_ignore_ascii_case("B");
        if !is_control && !is_test {
            return Err(PlatformError::Schema {
                platform: PlatformId::LevelPlay,
                operation: "experiment_days".into(),
                detail: format!(
                    "abTest reports arm '{label}' but the experiment's groups are A and B"
                ),
            });
        }
        let map = if is_control { &mut control } else { &mut test };
        let total = map.entry(row.day.clone()).or_default();
        total.0 = total
            .0
            .checked_add(row.revenue)
            .ok_or_else(|| PlatformError::Schema {
                platform: PlatformId::LevelPlay,
                operation: "experiment_days".into(),
                detail: format!("revenue rows for {} overflow i64 micros", row.day),
            })?;
        total.1 = total
            .1
            .checked_add(row.active_users)
            .ok_or_else(|| PlatformError::Schema {
                platform: PlatformId::LevelPlay,
                operation: "experiment_days".into(),
                detail: format!("activeUsers rows for {} overflow u64", row.day),
            })?;
    }
    // A day one arm reported stays present on the other arm as zero, the
    // shared day shape the window expects.
    let days: BTreeSet<String> = control.keys().chain(test.keys()).cloned().collect();
    for day in days {
        control.entry(day.clone()).or_insert((0, 0));
        test.entry(day).or_insert((0, 0));
    }
    Ok((control, test))
}

/// Turn one reporting read into arm-day observations. Both revenue and the
/// active-user denominator are carried, so finality compares the same
/// quantities the measured revenue-per-user value uses.
fn observations_from_rows(
    rows: &[AbTestReportRow],
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
            let (revenue, users) = map.get(day).copied().unwrap_or((0, 0));
            observations.push((
                arm,
                day.clone(),
                apex_lift::window::PullObservation {
                    revenue,
                    users: Some(users),
                    fill: None,
                    observed_at_unix_s,
                    source_sha256: apex_lift::window::arm_day_source_sha256(
                        day,
                        arm.as_str(),
                        revenue,
                        Some(users),
                        None,
                    ),
                },
            ));
        }
    }
    Ok(observations)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use apex_connectors::transport::{
        HttpBody, HttpMethod, HttpRequest, HttpResponse, Transport, TransportError,
    };
    use apex_core::ExperimentSpec;

    use super::*;

    /// A transport that answers from a fixed queue — falling back to an empty
    /// (zero-row) A/B report once the queue drains, so the 63-day pull loop
    /// stays scriptable by day — and records every request.
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
            let scripted = self.responses.lock().unwrap();
            if scripted.is_empty() {
                return Ok(HttpResponse {
                    status: 200,
                    headers: vec![],
                    body: serde_json::to_vec(&serde_json::json!({
                        "code": 200,
                        "count": 0,
                        "results": []
                    }))
                    .unwrap(),
                });
            }
            drop(scripted);
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

    const NOW_S: i64 = 1_800_000_000;

    fn today_day() -> i64 {
        NOW_S / 86_400
    }

    fn auth() -> serde_json::Value {
        serde_json::json!("bearer")
    }

    /// One reporting row fixture: `(date, arm, revenue units, activeUsers)`.
    /// Revenue is written as a decimal currency string so the exact
    /// units-to-micros conversion is exercised end to end.
    fn report(rows: &[(&str, &str, &str, u64)]) -> serde_json::Value {
        serde_json::json!({
            "code": 200,
            "count": rows.len(),
            "results": rows
                .iter()
                .map(|(date, arm, revenue, users)| serde_json::json!({
                    "date": date,
                    "abTest": arm,
                    "revenue": revenue,
                    "activeUsers": users
                }))
                .collect::<Vec<_>>()
        })
    }

    /// `days` trailing days (yesterday back), both arms on every day, oldest
    /// first — the shape the one-window reporting pull returns.
    fn trailing_report(
        days: i64,
        control_revenue: &str,
        test_revenue: &str,
        users: u64,
    ) -> serde_json::Value {
        let today = today_day();
        let mut results = Vec::new();
        for offset in (1..=days).rev() {
            let day = crate::calendar::format_day(today - offset);
            for (arm, revenue) in [("A", control_revenue), ("B", test_revenue)] {
                results.push(serde_json::json!({
                    "date": day,
                    "abTest": arm,
                    "revenue": revenue,
                    "activeUsers": users
                }));
            }
        }
        serde_json::json!({"code": 200, "count": results.len(), "results": results})
    }

    fn typed_row(day: &str, arm: &str, revenue: i64, users: u64) -> AbTestReportRow {
        AbTestReportRow {
            day: day.to_string(),
            ab_test: arm.to_string(),
            revenue,
            active_users: users,
        }
    }

    fn groups_report(variant_instances: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "groups": [
                {
                    "groupId": "g-a",
                    "name": "Control",
                    "abTest": "A",
                    "floorPrice": 1.0,
                    "instances": [{"instanceId": "i-control"}]
                },
                {
                    "groupId": "g-b",
                    "name": "Variant B",
                    "abTest": "B",
                    "floorPrice": 2.0,
                    "instances": variant_instances
                }
            ]
        })
    }

    fn ops_with_wall(
        transport: Arc<FakeTransport>,
        writable: bool,
        wall: Arc<StepWall>,
    ) -> LevelPlayPlatformOps {
        let write = if writable {
            vec![
                "updateEntryPrice".to_string(),
                "createEntry".to_string(),
                "removeEntry".to_string(),
            ]
        } else {
            vec![]
        };
        LevelPlayPlatformOps::new(
            "secret-key",
            "refresh-token",
            "acct",
            "https://lp.test",
            transport,
            wall,
            write,
        )
        .expect("adapter")
    }

    fn ops(transport: Arc<FakeTransport>, writable: bool) -> LevelPlayPlatformOps {
        ops_with_wall(transport, writable, Arc::new(StepWall::new(NOW_S)))
    }

    fn spec() -> ExperimentSpec {
        ExperimentSpec {
            name: "autopilot-levelplay-u1-2-5".into(),
            instance_id: String::new(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "raise the waterfall entry to 2.5".into(),
            test_group_allocation: 50,
        }
    }

    fn assert_typed_schema(error: &PlatformError, operation: &str) {
        match error {
            PlatformError::Schema {
                platform,
                operation: op,
                ..
            } => {
                assert_eq!(*platform, PlatformId::LevelPlay);
                assert_eq!(op, operation);
            }
            other => panic!("expected a typed Schema error, got {other:?}"),
        }
        assert!(!error.is_retryable(), "schema drift is not retryable");
    }

    #[tokio::test]
    async fn an_aa_read_with_equal_revenue_and_users_folds_to_zero_revenue_lift() {
        let transport = Arc::new(FakeTransport::default());
        let wall = Arc::new(StepWall::new(NOW_S));
        transport.push(200, auth());
        // Ten equal-arm days; both the revenue and the active-user fields are
        // identical, so the users branch sees the same revenue per user. An
        // A/A can never manufacture revenue lift.
        transport.push(200, trailing_report(10, "100.00", "100.00", 5_000));
        let adapter = ops_with_wall(Arc::clone(&transport), true, Arc::clone(&wall));
        let first = adapter.experiment_days("g-b").await.expect("first");
        assert_eq!(first.control.len(), 10);
        assert!(first
            .control
            .iter()
            .all(|day| day.second_pull_revenue.is_none()));
        // A distinct read at least 24 hours later supplies the second pull.
        wall.set(NOW_S + crate::pull_evidence::SECOND_PULL_MIN_SECS);
        transport.push(200, trailing_report(10, "100.00", "100.00", 5_000));
        let days = adapter.experiment_days("g-b").await.expect("days");
        assert_eq!(days.control.len(), 10);
        assert_eq!(days.test.len(), 10);
        for (control, test) in days.control.iter().zip(&days.test) {
            assert_eq!(control.day, test.day);
            assert_eq!(control.revenue, test.revenue);
            assert_eq!(
                control.revenue, 100_000_000,
                "$100.00 parses to 100_000_000 micros"
            );
            assert_eq!(control.users, Some(5_000));
            assert_eq!(test.users, Some(5_000));
            assert_eq!(control.second_pull_revenue, Some(100_000_000));
            assert_eq!(control.second_pull_users, Some(5_000));
            assert!(control.second_observed_at_unix_s.is_some());
            assert!(control.second_source_sha256.is_some());
        }
        // The full fold: the window completes and the estimate is exactly zero.
        let window = apex_lift::window::build_window(&days.control, &days.test, 7, 0.5, 3);
        assert!(window.complete, "problems: {:?}", window.problems);
        assert_eq!(window.days.len(), 7);
        let estimate = apex_lift::estimate(
            &window
                .days
                .iter()
                .map(|day| day.log_ratio)
                .collect::<Vec<_>>(),
        )
        .expect("estimate");
        assert_eq!(estimate.lift, 0.0);
        assert_eq!(estimate.mean_log, 0.0);
        assert_eq!(estimate.lower_90, 0.0);
    }

    #[tokio::test]
    async fn a_single_arm_day_zero_fills_and_one_read_is_not_final() {
        let today = today_day();
        let yesterday = crate::calendar::format_day(today - 1);
        let pull_day = crate::calendar::format_day(today);
        let rows = vec![
            typed_row(&yesterday, "A", 700_000_000, 700),
            typed_row(&yesterday, "B", 900_000_000, 900),
            typed_row(&pull_day, "A", 100_000_000, 100),
        ];
        let (control, test) = fold_rows(&rows).expect("fold");
        assert_eq!(control.get(&yesterday), Some(&(700_000_000, 700)));
        assert_eq!(test.get(&yesterday), Some(&(900_000_000, 900)));
        assert_eq!(control.get(&pull_day), Some(&(100_000_000, 100)));
        assert_eq!(
            test.get(&pull_day),
            Some(&(0, 0)),
            "the missing B row zero-fills both fields"
        );
        // One read is only the first observation, never finality evidence.
        let mut store = crate::pull_evidence::PullEvidenceStore::in_memory("test");
        let days = store
            .record_read(
                "g-b",
                observations_from_rows(&rows, today * DAY_SECS).expect("observations"),
            )
            .expect("record");
        assert_eq!(days.control.len(), 2);
        assert_eq!(days.test.len(), 2);
        assert!(days
            .control
            .iter()
            .chain(days.test.iter())
            .all(|day| day.second_pull_revenue.is_none()));
    }

    #[test]
    fn a_third_arm_label_fails_the_fold_typed() {
        let rows = vec![
            typed_row("2027-01-13", "A", 1_000_000, 10),
            typed_row("2027-01-13", "NULL", 1_000_000, 10),
        ];
        let error = fold_rows(&rows).expect_err("third label");
        assert_typed_schema(&error, "experiment_days");
        assert!(error.to_string().contains("NULL"), "{error}");
    }

    #[tokio::test]
    async fn a_reporting_schema_drift_fails_typed() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        // A body without the `results` array is schema drift, not an empty day.
        transport.push(200, serde_json::json!({"code": 200}));
        let adapter = ops(Arc::clone(&transport), true);
        let error = adapter.experiment_days("g-b").await.expect_err("drift");
        match &error {
            PlatformError::Schema { operation, .. } => {
                assert_eq!(operation, "reportAbTestActiveUsers")
            }
            other => panic!("expected a typed Schema error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn group_b_writes_only_target_b_and_refuse_foreign_entries() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(
            200,
            groups_report(serde_json::json!([{
                "id": "i-b",
                "name": "High",
                "networkName": "admob"
            }])),
        );
        transport.push(
            200,
            serde_json::json!([{
                "instanceId": "i1",
                "instanceName": "High",
                "networkName": "admob",
                "rate": "2.0"
            }]),
        );
        transport.push(200, serde_json::json!({}));
        let adapter = ops(Arc::clone(&transport), true);
        assert!(adapter.writable("updateEntryPrice").await);

        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::WaterfallEntryPriceSet {
                ad_unit_id: "u1".into(),
                entry_id: "i1".into(),
                price_ecpm: 2_500_000,
            },
            prev_value: "{}".into(),
        };
        let id = adapter.start_experiment(&request).await.expect("start");
        assert!(
            id.starts_with("g-b::"),
            "a unique handle per proposal: {id}"
        );

        {
            let seen = transport.seen.lock().unwrap();
            assert_eq!(
                seen.len(),
                4,
                "auth, groups, live instances and exactly one write"
            );
            // The groups read matches the connector's recorded shape.
            assert_eq!(seen[1].method, HttpMethod::Get);
            assert_eq!(seen[1].url, "https://lp.test/levelPlay/groups/v4/acct");
            assert_eq!(
                seen[1]
                    .headers
                    .iter()
                    .find(|(name, _)| name.as_str() == "Authorization")
                    .map(|(_, value)| value.as_str()),
                Some("Bearer bearer")
            );
            // The single write is the connector's recorded instances call.
            assert_eq!(seen[3].method, HttpMethod::Put);
            assert_eq!(
                seen[3].url,
                "https://lp.test/levelPlay/network/instances/v4/acct/"
            );
            assert!(
                seen.iter().all(|request| !request.url.contains("g-a")),
                "the control group is never addressed: {:?}",
                seen.iter().map(|request| &request.url).collect::<Vec<_>>()
            );
            let body = match seen[3].body.as_ref().expect("body") {
                HttpBody::Json(value) => value.clone(),
                other => panic!("expected JSON body, got {other:?}"),
            };
            assert_eq!(
                body,
                serde_json::json!([{"instanceId": "i-b", "rate": "2.5"}])
            );
        }

        // An entry that is not a member of group B is refused with no write.
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(
            200,
            groups_report(serde_json::json!([{"instanceId": "i1"}])),
        );
        transport.push(
            200,
            serde_json::json!([{
                "instanceId": "i1",
                "instanceName": "High",
                "networkName": "admob",
                "rate": "2.0"
            }]),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::WaterfallEntryPriceSet {
                ad_unit_id: "u1".into(),
                entry_id: "i9".into(),
                price_ecpm: 2_500_000,
            },
            prev_value: "{}".into(),
        };
        let error = adapter
            .start_experiment(&request)
            .await
            .expect_err("foreign");
        assert_typed_schema(&error, "startExperiment");
        assert!(error.to_string().contains("i9"), "{error}");
        assert_eq!(
            transport.seen.lock().unwrap().len(),
            3,
            "groups and the live waterfall were read: no write"
        );

        // No variant group at all: refused, no write either.
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(
            200,
            serde_json::json!({"groups": [{"groupId": "g-a", "name": "Control", "abTest": "A"}]}),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let error = adapter.start_experiment(&request).await.expect_err("no B");
        assert_typed_schema(&error, "startExperiment");
        assert!(error.to_string().contains("dashboard"), "{error}");
        assert_eq!(transport.seen.lock().unwrap().len(), 2, "no write");
    }

    #[tokio::test]
    async fn a_non_waterfall_change_is_refused_before_any_call() {
        let transport = Arc::new(FakeTransport::default());
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
        let error = adapter
            .start_experiment(&request)
            .await
            .expect_err("refuse");
        assert_typed_schema(&error, "startExperiment");
        assert!(transport.seen.lock().unwrap().is_empty(), "no call at all");
    }

    #[tokio::test]
    async fn the_instance_price_action_writes_the_variant_group_instance() {
        // The frozen Action variant maps to the existing waterfall-entry price
        // change and the adapter writes it through the instances v4 call.
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(
            200,
            groups_report(serde_json::json!([{
                "id": "i-b",
                "name": "High",
                "networkName": "admob"
            }])),
        );
        transport.push(
            200,
            serde_json::json!([{
                "instanceId": "i1",
                "instanceName": "High",
                "networkName": "admob",
                "rate": "2.0"
            }]),
        );
        transport.push(200, serde_json::json!({}));
        let adapter = ops(Arc::clone(&transport), true);
        let change =
            crate::plan::change_kind_of(&crate::model::Action::SetLevelplayInstancePrice {
                instance_id: "i1".into(),
                price_micros: 2_400_000,
            });
        assert_eq!(
            change,
            ChangeKind::WaterfallEntryPriceSet {
                ad_unit_id: "i1".into(),
                entry_id: "i1".into(),
                price_ecpm: 2_400_000,
            }
        );
        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change,
            prev_value: "{}".into(),
        };
        let id = adapter.start_experiment(&request).await.expect("start");
        assert!(id.starts_with("g-b::"));
        let seen = transport.seen.lock().unwrap();
        assert_eq!(
            seen.len(),
            4,
            "auth, groups, live instances and exactly one write"
        );
        assert_eq!(seen[3].method, HttpMethod::Put);
        assert_eq!(
            seen[3].url,
            "https://lp.test/levelPlay/network/instances/v4/acct/"
        );
        let body = match seen[3].body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        assert_eq!(
            body,
            serde_json::json!([{"instanceId": "i-b", "rate": "2.4"}])
        );
    }

    #[tokio::test]
    async fn promotion_status_follows_the_variant_group_and_the_live_price() {
        use crate::ops::PromotionStatus;
        use apex_proof::ChangeKind;
        let change = ChangeKind::WaterfallEntryPriceSet {
            ad_unit_id: "u1".into(),
            entry_id: "i1".into(),
            price_ecpm: 2_400_000,
        };
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(
            200,
            groups_report(serde_json::json!([{"instanceId": "i1"}])),
        );
        let adapter = ops(Arc::clone(&transport), true);
        assert_eq!(
            adapter
                .promotion_status("g-b::exp", &change)
                .await
                .expect("pending"),
            PromotionStatus::Pending
        );
        transport.push(
            200,
            serde_json::json!({"groups": [{"groupId": "2001", "abTest": "A"}]}),
        );
        transport.push(
            200,
            serde_json::json!([{
                "instanceId": "i1",
                "instanceName": "High",
                "networkName": "admob",
                "rate": "2.4"
            }]),
        );
        assert_eq!(
            adapter
                .promotion_status("g-b::exp", &change)
                .await
                .expect("confirmed"),
            PromotionStatus::Confirmed
        );
        transport.push(
            200,
            serde_json::json!({"groups": [{"groupId": "2001", "abTest": "A"}]}),
        );
        transport.push(
            200,
            serde_json::json!([{
                "instanceId": "i1",
                "instanceName": "High",
                "networkName": "admob",
                "rate": "2.0"
            }]),
        );
        assert_eq!(
            adapter
                .promotion_status("g-b::exp", &change)
                .await
                .expect("declined"),
            PromotionStatus::Declined
        );
    }

    #[tokio::test]
    async fn created_entries_write_through_the_variant_group() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(200, groups_report(serde_json::json!([])));
        transport.push(200, serde_json::json!({"instanceId": "i-new"}));
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::WaterfallEntryCreated {
                ad_unit_id: "u1".into(),
                network: "admob".into(),
                instance_name: "admob_b".into(),
                price_ecpm: 1_250_000,
            },
            prev_value: "{}".into(),
        };
        let id = adapter.start_experiment(&request).await.expect("start");
        assert!(id.starts_with("g-b::"));
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[2].method, HttpMethod::Post);
        let body = match seen[2].body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        assert_eq!(
            body,
            serde_json::json!([{
                "instanceName": "admob_b",
                "adapterNetwork": "admob",
                "rate": "1.25"
            }])
        );
    }

    #[tokio::test]
    async fn experiment_reads_group_b_presence_and_promote_refuses_typed() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(
            200,
            groups_report(serde_json::json!([{"instanceId": "i1"}])),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let result = adapter.experiment("g-b").await.expect("read");
        assert_eq!(result.state, ExperimentState::Running);
        assert_eq!(result.experiment_id, "g-b");

        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(
            200,
            serde_json::json!({"groups": [{"groupId": "g-a", "abTest": "A"}]}),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let result = adapter.experiment("g-b").await.expect("read");
        assert_eq!(result.state, ExperimentState::Stopped);

        // Promotion is dashboard-only; a bare experiment id cannot name a
        // variant group to delete. Both refuse without a call.
        let transport = Arc::new(FakeTransport::default());
        let adapter = ops(Arc::clone(&transport), true);
        assert!(adapter.promote_experiment("g-b").await.is_err());
        assert!(adapter.deprecate_experiment("g-b").await.is_err());
        assert!(transport.seen.lock().unwrap().is_empty());

        // A real handle ends the test by deleting the documented variant
        // group, so settlement is not left unresolved.
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(
            200,
            serde_json::json!({"groups": [{"groupId": "g-a", "abTest": "A"}]}),
        );
        let adapter = ops(Arc::clone(&transport), true);
        adapter
            .deprecate_experiment("g-b::experiment-1")
            .await
            .expect("stop");
        {
            let seen = transport.seen.lock().unwrap();
            assert_eq!(seen.len(), 2, "auth + delete");
            assert_eq!(seen[1].method, HttpMethod::Delete);
            assert!(seen[1].url.contains("/levelPlay/groups/v4/acct"));
            let body = match seen[1].body.as_ref().expect("body") {
                HttpBody::Json(value) => value.clone(),
                other => panic!("expected JSON body, got {other:?}"),
            };
            assert_eq!(body, serde_json::json!({"ids": ["g-b"]}));
        }

        assert!(adapter.change_in_test_arm());
        assert!(adapter.call_log().is_empty());
        assert!(format!("{adapter:?}").contains("LevelPlayPlatformOps"));
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
    }

    #[tokio::test]
    async fn current_settings_encode_the_live_instance_price() {
        let transport = Arc::new(FakeTransport::default());
        let instances = serde_json::json!([{
            "instanceId": "i1",
            "adUnit": "u1",
            "instanceName": "i1",
            "networkName": "admob",
            "rate": "2.5"
        }]);
        transport.push(200, auth());
        transport.push(200, instances.clone());
        let adapter = ops(Arc::clone(&transport), true);
        let settings = adapter.current_settings("i1").await.expect("settings");
        let value: serde_json::Value = serde_json::from_str(&settings).expect("json");
        assert_eq!(value["priceEcpm"], serde_json::json!(2_500_000));
        assert_eq!(value["i1"], serde_json::json!(2_500_000));
        // Reading again returns the same canonical encoding.
        transport.push(200, instances);
        assert_eq!(settings, settings_from(&adapter).await);
        // The monthly pass's scope revenue is the app-level revenue report,
        // summed exactly into micros.
        transport.push(
            200,
            report(&[
                ("2027-01-01", "A", "10.50", 100),
                ("2027-01-02", "B", "4.25", 40),
            ]),
        );
        let revenue = adapter
            .scope_revenue("u1", "2027-01-01", "2027-01-31")
            .await
            .expect("revenue");
        assert_eq!(revenue, 14_750_000, "$14.75 in micros");
        let seen = transport.seen.lock().unwrap();
        assert_eq!(
            seen.last().expect("revenue request").url,
            "https://lp.test/levelPlay/reporting/v1?appKey=acct&breakdowns=date&metrics=revenue&startDate=2027-01-01&endDate=2027-01-31"
        );
    }

    async fn settings_from(adapter: &LevelPlayPlatformOps) -> String {
        adapter.current_settings("i1").await.expect("settings")
    }

    #[tokio::test]
    async fn a_null_experiment_is_only_adopted_when_the_arms_match() {
        // The dashboard test's two arms are identical apart from their
        // identity fields: the running test IS the A/A read, adopted with no
        // write at all.
        let identical = serde_json::json!({
            "groups": [
                {
                    "groupId": "g-a",
                    "name": "Control",
                    "abTest": "A",
                    "floorPrice": 1.5,
                    "countriesRate": 0.8,
                    "instances": [{"instanceId": "i1"}]
                },
                {
                    "groupId": "g-b",
                    "name": "Variant B",
                    "abTest": "B",
                    "floorPrice": 1.5,
                    "countriesRate": 0.8,
                    "instances": [{"instanceId": "i1"}]
                }
            ]
        });
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(200, identical);
        let adapter = ops(Arc::clone(&transport), true);
        let id = adapter.start_null_experiment(&spec()).await.expect("adopt");
        assert!(
            id.starts_with("g-b::"),
            "a unique null-experiment handle: {id}"
        );
        assert_eq!(
            transport.seen.lock().unwrap().len(),
            2,
            "the read is the only call: nothing is written"
        );

        // A variant arm carrying any change is not an A/A: refused typed.
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        transport.push(
            200,
            groups_report(serde_json::json!([{"instanceId": "i1"}])),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let error = adapter
            .start_null_experiment(&spec())
            .await
            .expect_err("a changed B arm is not an A/A");
        assert_typed_schema(&error, "startNullExperiment");
        assert!(error.to_string().contains("identical"), "{error}");
        assert_eq!(transport.seen.lock().unwrap().len(), 2, "no write");
    }

    #[tokio::test]
    async fn writes_are_refused_without_the_allow_list() {
        let transport = Arc::new(FakeTransport::default());
        let adapter = ops(Arc::clone(&transport), false);
        assert!(!adapter.writable("updateEntryPrice").await);
        assert!(!adapter.writable("createEntry").await);
        assert!(!adapter.writable("removeEntry").await);
        assert!(transport.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn experiment_days_pulls_one_reporting_window_oldest_first() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(200, auth());
        let day = crate::calendar::format_day(today_day() - 1);
        transport.push(
            200,
            report(&[(&day, "A", "10.00", 10), (&day, "B", "12.00", 12)]),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let days = adapter.experiment_days("g-b").await.expect("days");
        assert_eq!(days.control.len(), 1);
        assert_eq!(days.test.len(), 1);
        assert_eq!(days.control[0].revenue, 10_000_000);
        assert_eq!(days.control[0].users, Some(10));
        assert_eq!(days.test[0].revenue, 12_000_000);
        assert_eq!(days.test[0].users, Some(12));
        assert!(days.control[0].second_pull_revenue.is_none());
        let seen = transport.seen.lock().unwrap();
        let first_day = crate::calendar::format_day(today_day() - LOOKBACK_DAYS);
        assert_eq!(seen[0].url, "https://lp.test/partners/publisher/auth");
        assert_eq!(seen[1].method, HttpMethod::Get);
        assert_eq!(
            seen[1].url,
            format!(
                "https://lp.test/levelPlay/reporting/v1?appKey=acct&breakdowns=date,abTest&metrics=revenue,activeUsers&startDate={first_day}&endDate={day}"
            )
        );
        assert_eq!(seen.len(), 2, "one reporting window, not one call per day");
    }

    fn kept_change(lambda: f64) -> apex_lift::billing::KeptChange {
        apex_lift::billing::KeptChange {
            change_id: "autopilot-levelplay-u1-2-5".into(),
            lambda,
            rate_bp: 2_000,
            kept_day: "2026-12-15".into(),
            overridden_day: None,
            rolled_back_day: None,
            pre_registration_hash: "pre-levelplay".into(),
            result_hash: "result-levelplay".into(),
        }
    }

    #[tokio::test]
    async fn a_kept_levelplay_change_bills_the_revenue_lift_like_max() {
        // Ten days of equal users and a constant 3% revenue lift: the measured
        // value is revenue per user, so the conservative bound is a revenue
        // lift, not a user count. The active-user count is deliberately large
        // and constant; a fold that billed users would produce a wildly
        // different increment.
        let transport = Arc::new(FakeTransport::default());
        let wall = Arc::new(StepWall::new(NOW_S));
        transport.push(200, auth());
        transport.push(200, trailing_report(10, "10.00", "10.30", 1_000_000));
        transport.push(200, trailing_report(10, "10.00", "10.30", 1_000_000));
        // The monthly scope revenue the pass bills against: $1,000.00.
        transport.push(200, report(&[("2027-01-31", "A", "1000.00", 1_000_000)]));
        let adapter = ops_with_wall(Arc::clone(&transport), true, Arc::clone(&wall));

        let _first = adapter.experiment_days("g-b").await.expect("first");
        wall.set(NOW_S + crate::pull_evidence::SECOND_PULL_MIN_SECS);
        let days = adapter.experiment_days("g-b").await.expect("days");
        let window = apex_lift::window::build_window(&days.control, &days.test, 7, 0.5, 3);
        assert!(window.complete, "problems: {:?}", window.problems);
        let estimate = apex_lift::estimate(
            &window
                .days
                .iter()
                .map(|day| day.log_ratio)
                .collect::<Vec<_>>(),
        )
        .expect("estimate");
        assert!(
            (estimate.lift - 0.03).abs() < 1e-9,
            "the value is the 3% revenue lift, got {}",
            estimate.lift
        );
        let lambda = estimate.lower_90.max(0.0);
        assert!(lambda > 0.0, "a measured revenue lift is billable");
        assert!(lambda <= estimate.lift + 1e-12);

        let scope_revenue = adapter
            .scope_revenue("u1", "2027-01-01", "2027-01-31")
            .await
            .expect("scope revenue");
        assert_eq!(scope_revenue, 1_000_000_000, "$1,000.00 in micros");

        let change = kept_change(lambda);
        let run = apex_lift::run::run_month(
            "2027-01",
            "2027-01-01",
            31,
            scope_revenue,
            std::slice::from_ref(&change),
        )
        .expect("monthly run");
        assert_eq!(run.fees.len(), 1);
        let fee = &run.fees[0];
        let expected_incremental =
            (scope_revenue as f64 * (1.0 - 1.0 / (1.0 + lambda))).round() as i64;
        assert_eq!(
            fee.incremental, expected_incremental,
            "the conservative increment derives from scope revenue, not users"
        );
        assert_eq!(
            fee.fee_micros,
            (0.20 * fee.incremental as f64).round() as i64,
            "a single change bills its full log share at the 20% rate"
        );

        // The billable record has exactly the MAX monthly-billing shape and
        // carries the revenue basis the invoice re-derives from.
        let record = apex_lift::emit::monthly_billing(fee, "2027-01").expect("record");
        let json = record.record.canonical_json().expect("canonical");
        let value: serde_json::Value = serde_json::from_str(&json).expect("json");
        assert_eq!(value["type"], "monthlyBilling");
        assert_eq!(value["scopeRevenue"], 1_000_000_000);
        assert_eq!(value["incremental"], expected_incremental);
        assert_eq!(value["feeMicros"], fee.fee_micros);
        assert_eq!(value["billedDays"], 31);
        assert_eq!(value["monthDays"], 31);
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "billedDays",
                "changeId",
                "factor",
                "feeMicros",
                "incremental",
                "lambda",
                "logShare",
                "month",
                "monthDays",
                "preRegistrationHash",
                "rateBp",
                "resultHash",
                "scopeRevenue",
                "type",
            ],
            "the same signed billable record shape MAX produces"
        );
    }

    #[tokio::test]
    async fn an_aa_levelplay_read_bills_nothing() {
        let transport = Arc::new(FakeTransport::default());
        let wall = Arc::new(StepWall::new(NOW_S));
        transport.push(200, auth());
        // Equal revenue and equal users on every day, so the measured revenue
        // lift is zero.
        transport.push(200, trailing_report(10, "10.00", "10.00", 5_000));
        transport.push(200, trailing_report(10, "10.00", "10.00", 5_000));
        let adapter = ops_with_wall(Arc::clone(&transport), true, Arc::clone(&wall));
        let _first = adapter.experiment_days("g-b").await.expect("first");
        wall.set(NOW_S + crate::pull_evidence::SECOND_PULL_MIN_SECS);
        let days = adapter.experiment_days("g-b").await.expect("days");
        let window = apex_lift::window::build_window(&days.control, &days.test, 7, 0.5, 3);
        assert!(window.complete, "problems: {:?}", window.problems);
        let estimate = apex_lift::estimate(
            &window
                .days
                .iter()
                .map(|day| day.log_ratio)
                .collect::<Vec<_>>(),
        )
        .expect("estimate");
        assert_eq!(estimate.lift, 0.0, "an A/A cannot manufacture revenue lift");
        assert_eq!(estimate.lower_90, 0.0);
        let lambda = estimate.lower_90.max(0.0);
        assert_eq!(lambda, 0.0, "no positive billable lift");
        // The keep scan only forms a KeptChange for lambda > 0, so the monthly
        // pass sees no active change and bills nothing even with revenue.
        let run = apex_lift::run::run_month("2027-01", "2027-01-01", 31, 1_000_000_000, &[])
            .expect("monthly run");
        assert!(run.fees.is_empty(), "an A/A change bills nothing");
        assert_eq!(run.scope_revenue, 1_000_000_000);
    }

    #[tokio::test]
    async fn revenue_stable_but_users_changed_between_reads_is_not_final() {
        let transport = Arc::new(FakeTransport::default());
        let wall = Arc::new(StepWall::new(NOW_S));
        let day = crate::calendar::format_day(today_day() - 1);
        transport.push(200, auth());
        transport.push(
            200,
            report(&[(&day, "A", "10.00", 1_000), (&day, "B", "10.00", 1_000)]),
        );
        let adapter = ops_with_wall(Arc::clone(&transport), true, Arc::clone(&wall));
        let first = adapter.experiment_days("g-b").await.expect("first");
        assert!(first.control[0].second_pull_revenue.is_none());

        // A later read with the same revenue but a different denominator.
        wall.set(NOW_S + crate::pull_evidence::SECOND_PULL_MIN_SECS);
        transport.push(
            200,
            report(&[(&day, "A", "10.00", 900), (&day, "B", "10.00", 900)]),
        );
        let days = adapter.experiment_days("g-b").await.expect("second");
        assert_eq!(days.control[0].revenue, 10_000_000);
        assert_eq!(days.control[0].second_pull_revenue, Some(10_000_000));
        assert_eq!(days.control[0].users, Some(1_000));
        assert_eq!(days.control[0].second_pull_users, Some(900));
        assert!(
            !apex_lift::window::day_is_final(&days.control[0]),
            "revenue-stable but users-changing evidence is not final"
        );
        assert!(
            !apex_lift::window::day_is_final(&days.test[0]),
            "the same rule applies to both arms"
        );
    }
}
