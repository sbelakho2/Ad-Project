//! The live AdMob write adapter: monitoring and Managed, never a lift fee.
//!
//! AdMob's public API documents mediation A/B experiments as a lifecycle —
//! `createMediationAbExperiment`, the experiment resource read, and
//! `stopMediationAbExperiment` — plus a bare `variantLeader` verdict. It
//! exposes **no per-variant revenue (or any other per-variant metric)**, so
//! the runner can start and end an experiment but can never measure lift from
//! one:
//!
//! - [`start_experiment`](PlatformOps::start_experiment) creates the v1beta
//!   mediation A/B experiment carrying the proposed mediation-group eCPM
//!   change (the change rides the test arm; the group is untouched) and
//!   returns the experiment's resource name as the platform experiment id;
//! - [`measures_variants`](PlatformOps::measures_variants) is `false`, so the
//!   daily pass never attempts a reporting pull;
//! - [`conclude_unmeasured_experiment`](PlatformOps::conclude_unmeasured_experiment)
//!   ends the test after its registered window with the documented `:stop`
//!   call and reads the state/`variantLeader` verdict: `TREATMENT` keeps the
//!   change, every other verdict leaves the original lines;
//! - [`experiment_days`](PlatformOps::experiment_days) still refuses with the
//!   no-measurement reason instead of inventing a signal, and
//!   [`promote_experiment`](PlatformOps::promote_experiment) refuses because
//!   the API documents no promotion.
//!
//! Because no measured lift can exist, the lift **fee** path excludes AdMob
//! at its eligibility gate (`apex_lift::billing::fee_eligibility`): a kept
//! AdMob conclusion is recorded with zero lift and a typed fee-ineligible
//! billing change, and never billed. Monitoring and the Managed experiment
//! lifecycle are unaffected — this adapter's job is exactly those.
//!
//! [`FeeIneligible`]: apex_lift::billing::FeeIneligible

use std::collections::BTreeMap;
use std::sync::Arc;

use apex_connectors::admob::{AdmobClient, MediationAbExperimentState};
use apex_connectors::transport::Transport;
use apex_core::{ExperimentResult, PlatformError, PlatformId};
use apex_proof::ChangeKind;
use async_trait::async_trait;

use crate::ops::{ExperimentDays, ExperimentRequest, PlatformOps, UnmeasuredConclusion};

pub struct AdmobPlatformOps {
    management: AdmobClient,
    /// Verify-live allow-list: an operation here (`updateMediationGroup`) is
    /// writable; anything else is refused before any side effect. Without it
    /// the runner starts no experiments at all.
    writable: BTreeMap<String, bool>,
}

impl std::fmt::Debug for AdmobPlatformOps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmobPlatformOps")
            .field("writable", &self.writable)
            .finish()
    }
}

impl AdmobPlatformOps {
    /// Build the adapter. `writable` lists the operations the operator has
    /// verified against a live sandbox; without it every write is refused.
    pub fn new(
        access_token: &str,
        account_id: &str,
        base_url: &str,
        transport: Arc<dyn Transport>,
        writable: impl IntoIterator<Item = String>,
    ) -> Result<Self, PlatformError> {
        let management = AdmobClient::new(access_token, account_id, base_url, transport)?;
        Ok(Self {
            management,
            writable: writable.into_iter().map(|op| (op, true)).collect(),
        })
    }

    fn schema(&self, operation: &str, detail: impl Into<String>) -> PlatformError {
        PlatformError::Schema {
            platform: PlatformId::Admob,
            operation: operation.into(),
            detail: detail.into(),
        }
    }
}

#[async_trait]
impl PlatformOps for AdmobPlatformOps {
    fn supports_player_feed(&self) -> bool {
        false
    }

    fn platform(&self) -> PlatformId {
        PlatformId::Admob
    }

    async fn writable(&self, operation: &str) -> bool {
        self.writable.get(operation).copied().unwrap_or(false)
    }

    /// Create the mediation A/B experiment whose test variant carries the
    /// proposed mediation-group eCPM change. No experiment starts unless the
    /// write was configured (the verify-live gate above); the returned v1beta
    /// resource name is the platform experiment id the ledger pins.
    async fn start_experiment(&self, request: &ExperimentRequest) -> Result<String, PlatformError> {
        let ChangeKind::EcpmSet {
            mediation_group_id,
            ad_unit_id,
            ecpm,
        } = &request.change
        else {
            return Err(self.schema(
                "startExperiment",
                "an AdMob experiment carries a mediation-group eCPM change only",
            ));
        };
        // The treatment arm must serve the changed line: take the raw line
        // that contains the target mapping and set its cpmMicros, then send
        // it as the documented treatmentMediationLines input.
        let lines = self
            .management
            .mediation_group_lines(mediation_group_id)
            .await?;
        let mut changed_lines: Vec<serde_json::Value> = Vec::new();
        // A group-scoped floor (the frozen Action variant names the group)
        // changes EVERY line of the group: taking only the first line would
        // leave the rest serving the old cpm inside the same treatment arm.
        if ad_unit_id == mediation_group_id {
            for line in lines {
                let mut line = line;
                line["cpmMicros"] = serde_json::json!(ecpm.to_string());
                changed_lines.push(line);
            }
        } else {
            // A mapping-scoped change reaches every line that carries the ad
            // unit (a unit can sit on several networks), in either documented
            // mapping shape: an array of {adUnitId, network} rows or an
            // object keyed by ad unit id.
            for line in lines {
                let contains = match line.get("adUnitMappings") {
                    Some(serde_json::Value::Array(mappings)) => mappings.iter().any(|mapping| {
                        mapping.get("adUnitId").and_then(|value| value.as_str())
                            == Some(ad_unit_id.as_str())
                    }),
                    Some(serde_json::Value::Object(mappings)) => {
                        mappings.contains_key(ad_unit_id.as_str())
                    }
                    _ => false,
                };
                if contains {
                    let mut line = line;
                    line["cpmMicros"] = serde_json::json!(ecpm.to_string());
                    changed_lines.push(line);
                }
            }
        }
        if changed_lines.is_empty() {
            return Err(self.schema(
                "startExperiment",
                format!(
                    "mediation group '{mediation_group_id}' has no line for ad unit '{ad_unit_id}'"
                ),
            ));
        }
        let treatment: Vec<serde_json::Value> = changed_lines
            .into_iter()
            .map(|line| serde_json::json!({ "mediationGroupLine": line }))
            .collect();
        let treatment = serde_json::Value::Array(treatment);
        let experiment = self
            .management
            .create_mediation_ab_experiment(
                mediation_group_id,
                &request.spec.name,
                Some(&treatment),
                Some(request.spec.test_group_allocation),
            )
            .await?;
        Ok(experiment.name)
    }

    fn change_in_test_arm(&self) -> bool {
        true
    }

    /// AdMob reports only a `variantLeader` verdict, never per-variant
    /// revenue, so the daily pass must not attempt a reporting pull.
    fn measures_variants(&self) -> bool {
        false
    }

    /// Read the experiment resource (v1beta `get`). The measurement surface
    /// still refuses: this returns state and the `variantLeader` verdict, not
    /// lift.
    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError> {
        let experiment = self
            .management
            .get_mediation_ab_experiment(experiment_id)
            .await?;
        Ok(ExperimentResult {
            experiment_id: experiment.name,
            state: match experiment.state {
                MediationAbExperimentState::Running => apex_core::ExperimentState::Running,
                MediationAbExperimentState::Stopped
                | MediationAbExperimentState::Ended
                | MediationAbExperimentState::Expired => apex_core::ExperimentState::Stopped,
            },
            results: Vec::new(),
            lift_percent_bp: None,
            confidence: None,
            winner_variant_id: experiment
                .variant_leader
                .map(|leader| leader.as_str().to_string()),
        })
    }

    /// End an AdMob test and return its verdict.
    ///
    /// The documented `:stop` call is the end. Its response is the experiment
    /// resource after the end: a `TREATMENT` (or legacy `VARIANT`) leader keeps
    /// the treatment; every other verdict leaves the original control lines.
    /// The pre-read is best-effort only for the documented get route: a
    /// platform (or simulator) without that route falls through to the stop
    /// response, which carries the same state and verdict.
    async fn conclude_unmeasured_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<UnmeasuredConclusion, PlatformError> {
        let prior = match self
            .management
            .get_mediation_ab_experiment(experiment_id)
            .await
        {
            Ok(experiment) => Some(experiment),
            // The resource read is optional here: the one documented end call
            // returns the same state and verdict. Only a missing route falls
            // through; auth, network and schema failures stay typed.
            Err(PlatformError::Http { status, .. }) if status == 404 || status == 405 => None,
            Err(error) => return Err(error),
        };
        let stopped = match self
            .management
            .stop_mediation_ab_experiment(experiment_id)
            .await
        {
            Ok(experiment) => experiment,
            Err(error) => {
                // A replay of an already-ended experiment is safe: the
                // resource read is authoritative when it says the end landed.
                if let Some(ended) = prior
                    .as_ref()
                    .filter(|experiment| experiment.state.is_ended())
                {
                    ended.clone()
                } else {
                    return Err(error);
                }
            }
        };
        if !stopped.state.is_ended() {
            return Err(self.schema(
                "stopMediationAbExperiment",
                format!(
                    "the stop call returned state '{}' instead of an ended experiment",
                    stopped.state.as_str()
                ),
            ));
        }
        let variant_leader = stopped
            .variant_leader
            .or_else(|| prior.and_then(|experiment| experiment.variant_leader));
        let keep_treatment = variant_leader.is_some_and(|leader| leader.keeps_treatment());
        Ok(UnmeasuredConclusion {
            experiment_state: stopped.state.as_str().to_string(),
            variant_leader: variant_leader.map(|leader| leader.as_str().to_string()),
            keep_treatment,
        })
    }

    /// The typed no-measurement refusal: AdMob reports a bare winner verdict,
    /// never a per-variant revenue or any other per-variant metric, so the
    /// lift engine can never be fed from this platform — and the fee gate
    /// excludes the platform for exactly that reason.
    async fn experiment_days(&self, _experiment_id: &str) -> Result<ExperimentDays, PlatformError> {
        Err(self.schema(
            "experimentDays",
            "no per-variant revenue measurement is documented for AdMob",
        ))
    }

    /// The mediation group's current mappings, canonically encoded, so the
    /// runner can detect a publisher edit during a test.
    fn supports_settings_read(&self) -> bool {
        true
    }

    async fn current_settings(&self, ad_unit_id: &str) -> Result<String, PlatformError> {
        let groups = self.management.list_mediation_groups().await?;
        let group = groups
            .iter()
            .find(|group| {
                group.mediation_group_id == ad_unit_id
                    || group
                        .mappings
                        .iter()
                        .any(|mapping| mapping.ad_unit_id == ad_unit_id)
            })
            .ok_or_else(|| {
                self.schema(
                    "current_settings",
                    format!("no mediation group matches scope '{ad_unit_id}'"),
                )
            })?;
        serde_json::to_string(group).map_err(|error| {
            self.schema(
                "current_settings",
                format!("mediation group not encodable: {error}"),
            )
        })
    }

    /// The winner of an AdMob mediation A/B experiment is applied in the
    /// AdMob UI; the API documents only `:stop`. Promotion refuses instead of
    /// pretending a winner was applied.
    async fn promote_experiment(&self, _experiment_id: &str) -> Result<(), PlatformError> {
        Err(self.schema(
            "promoteExperiment",
            "AdMob applies the mediation A/B winner in its UI; the API documents only stop",
        ))
    }

    /// Stop the experiment through the one documented end call — the
    /// rollback/void outcome for a Managed AdMob test. The ledger stores the
    /// created experiment's full resource name, so the connector can use the
    /// documented nested route (experiment under its mediation group) rather
    /// than trimming the group away.
    async fn deprecate_experiment(&self, experiment_id: &str) -> Result<(), PlatformError> {
        let id = experiment_id.trim();
        if id.is_empty() {
            return Err(self.schema("stopMediationAbExperiment", "experiment id is empty"));
        }
        self.management.stop_mediation_ab_experiment(id).await?;
        Ok(())
    }

    async fn apply(&self, _action: &crate::model::Action, _id: &str) -> Result<(), PlatformError> {
        // The change always rides the experiment's test arm; nothing is ever
        // applied live.
        Err(self.schema(
            "apply",
            "an AdMob change is carried by the mediation A/B experiment; there is no live apply path",
        ))
    }

    async fn rollback(&self, _action: &crate::model::Action) -> Result<(), PlatformError> {
        Err(self.schema(
            "rollback",
            "an AdMob change is carried by the mediation A/B experiment; there is no live rollback path",
        ))
    }

    fn call_log(&self) -> Vec<String> {
        Vec::new()
    }
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

    fn ops(transport: Arc<FakeTransport>, writable: bool) -> AdmobPlatformOps {
        let write = if writable {
            vec!["updateMediationGroup".to_string()]
        } else {
            vec![]
        };
        AdmobPlatformOps::new(
            "access-token",
            "accounts/pub-1234567890",
            "https://admob.test",
            transport,
            write,
        )
        .expect("adapter")
    }

    fn spec() -> ExperimentSpec {
        ExperimentSpec {
            name: "autopilot-ecpm-g1".into(),
            instance_id: String::new(),
            ad_unit_id: "g1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "raise the group eCPM to 1.75".into(),
            test_group_allocation: 50,
        }
    }

    #[tokio::test]
    async fn start_writes_the_v1beta_experiment_and_returns_its_resource_name() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "mediationGroupId": "g1",
                "name": "accounts/pub-1234567890/mediationGroups/g1",
                "mediationGroupLines": {
                    "l1": {
                        "adUnitMappings": [{"adUnitId": "u2", "network": "facebook"}],
                        "cpmMicros": "1500000"
                    }
                }
            }),
        );
        transport.push(
            200,
            serde_json::json!({
                "name": "accounts/pub-1234567890/mediationAbExperiments/exp-7",
                "mediationGroupId": "g1",
                "displayName": "autopilot-ecpm-g1",
                "state": "RUNNING",
                "variantLeader": null
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
        assert!(
            !adapter.supports_player_feed(),
            "AdMob documents no per-player feed"
        );
        assert!(adapter.writable("updateMediationGroup").await);
        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::EcpmSet {
                mediation_group_id: "g1".into(),
                ad_unit_id: "u2".into(),
                ecpm: 1_750_000,
            },
            prev_value: "{}".into(),
        };
        let id = adapter.start_experiment(&request).await.expect("start");
        assert_eq!(id, "accounts/pub-1234567890/mediationAbExperiments/exp-7");
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "group read then experiment create");
        assert_eq!(seen[0].method, HttpMethod::Get);
        assert_eq!(seen[1].method, HttpMethod::Post);
        assert_eq!(
            seen[1].url,
            "https://admob.test/v1beta/accounts/pub-1234567890/mediationGroups/g1/mediationAbExperiments"
        );
        let body = match seen[1].body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        assert_eq!(body["displayName"], serde_json::json!("autopilot-ecpm-g1"));
        assert_eq!(body["treatmentTrafficPercentage"], serde_json::json!("50"));
        assert_eq!(
            body["treatmentMediationLines"][0]["mediationGroupLine"]["cpmMicros"],
            serde_json::json!("1750000"),
            "the treatment arm carries the proposed eCPM"
        );
    }

    #[tokio::test]
    async fn a_group_floor_changes_every_line_of_the_group() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "mediationGroupId": "g1",
                "name": "accounts/pub-1234567890/mediationGroups/g1",
                "mediationGroupLines": {
                    "l1": {
                        "adUnitMappings": [{"adUnitId": "u1", "network": "admob"}],
                        "cpmMicros": "1500000"
                    },
                    "l2": {
                        "adUnitMappings": {"u2": "m2"},
                        "cpmMicros": "1600000"
                    }
                }
            }),
        );
        transport.push(
            200,
            serde_json::json!({
                "name": "accounts/pub-1234567890/mediationAbExperiments/exp-11",
                "mediationGroupId": "g1",
                "displayName": "autopilot-ecpm-g1",
                "state": "RUNNING",
                "variantLeader": null
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::EcpmSet {
                mediation_group_id: "g1".into(),
                ad_unit_id: "g1".into(),
                ecpm: 1_750_000,
            },
            prev_value: "{}".into(),
        };
        adapter.start_experiment(&request).await.expect("start");
        let seen = transport.seen.lock().unwrap();
        let body = match seen[1].body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        let treatment = body["treatmentMediationLines"]
            .as_array()
            .expect("treatment lines");
        assert_eq!(
            treatment.len(),
            2,
            "every line carries the new floor, not just the first"
        );
        for entry in treatment {
            assert_eq!(
                entry["mediationGroupLine"]["cpmMicros"],
                serde_json::json!("1750000")
            );
        }
    }

    #[tokio::test]
    async fn a_line_without_ad_unit_mappings_never_matches_a_mapping_scoped_change() {
        // `_ => false` is the whole safety of the shape match: a group whose
        // only line carries no mappings at all must refuse the change instead
        // of treating every line as a match.
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "mediationGroupId": "g1",
                "name": "accounts/pub-1234567890/mediationGroups/g1",
                "mediationGroupLines": {
                    "l1": {"cpmMicros": "1500000"}
                }
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::EcpmSet {
                mediation_group_id: "g1".into(),
                ad_unit_id: "u2".into(),
                ecpm: 1_750_000,
            },
            prev_value: "{}".into(),
        };
        let error = adapter
            .start_experiment(&request)
            .await
            .expect_err("a line without mappings is not a match");
        assert!(
            error.to_string().contains("no line for ad unit"),
            "{error:?}"
        );
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "no experiment was created");
    }

    #[tokio::test]
    async fn a_mapping_scoped_change_matches_the_object_mapping_shape() {
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "mediationGroupId": "g1",
                "name": "accounts/pub-1234567890/mediationGroups/g1",
                "mediationGroupLines": {
                    "l1": {
                        "adUnitMappings": {"u2": "m2"},
                        "cpmMicros": "1500000"
                    }
                }
            }),
        );
        transport.push(
            200,
            serde_json::json!({
                "name": "accounts/pub-1234567890/mediationAbExperiments/exp-12",
                "mediationGroupId": "g1",
                "displayName": "autopilot-ecpm-g1",
                "state": "RUNNING",
                "variantLeader": null
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: spec().name,
            spec: spec(),
            change: ChangeKind::EcpmSet {
                mediation_group_id: "g1".into(),
                ad_unit_id: "u2".into(),
                ecpm: 1_750_000,
            },
            prev_value: "{}".into(),
        };
        let id = adapter.start_experiment(&request).await.expect("start");
        assert!(id.ends_with("exp-12"), "{id}");
        let seen = transport.seen.lock().unwrap();
        let body = match seen[1].body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        assert_eq!(
            body["treatmentMediationLines"][0]["mediationGroupLine"]["cpmMicros"],
            serde_json::json!("1750000")
        );
    }

    #[tokio::test]
    async fn the_mediation_floor_action_starts_the_group_ab_experiment() {
        // The frozen Action variant maps to the existing mediation-group eCPM
        // change; the adapter starts the v1beta experiment on that group.
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "mediationGroupId": "g1",
                "name": "accounts/pub-1234567890/mediationGroups/g1",
                "mediationGroupLines": {
                    "l1": {
                        "adUnitMappings": [{"adUnitId": "u2", "network": "facebook"}],
                        "cpmMicros": "1500000"
                    }
                }
            }),
        );
        transport.push(
            200,
            serde_json::json!({
                "name": "accounts/pub-1234567890/mediationAbExperiments/exp-9",
                "mediationGroupId": "g1",
                "displayName": "autopilot-mf-g1-1-2",
                "state": "RUNNING",
                "variantLeader": null
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let change = crate::plan::change_kind_of(&crate::model::Action::SetAdmobMediationFloor {
            group_id: "g1".into(),
            floor_micros: 1_200_000,
        });
        assert_eq!(
            change,
            ChangeKind::EcpmSet {
                mediation_group_id: "g1".into(),
                ad_unit_id: "g1".into(),
                ecpm: 1_200_000,
            }
        );
        let request = ExperimentRequest {
            experiment_id: "autopilot-mf-g1-1-2".into(),
            spec: spec(),
            change,
            prev_value: "{}".into(),
        };
        let id = adapter.start_experiment(&request).await.expect("start");
        assert_eq!(id, "accounts/pub-1234567890/mediationAbExperiments/exp-9");
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "group read then experiment create");
        assert_eq!(seen[0].method, HttpMethod::Get);
        assert_eq!(seen[1].method, HttpMethod::Post);
        assert_eq!(
            seen[1].url,
            "https://admob.test/v1beta/accounts/pub-1234567890/mediationGroups/g1/mediationAbExperiments"
        );
        let body = match seen[1].body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        assert_eq!(
            body["treatmentMediationLines"][0]["mediationGroupLine"]["cpmMicros"],
            serde_json::json!("1200000")
        );
        assert_eq!(body["displayName"], serde_json::json!("autopilot-ecpm-g1"));
    }

    #[tokio::test]
    async fn a_non_ecpm_change_is_refused_before_any_call() {
        let transport = Arc::new(FakeTransport::default());
        let adapter = ops(Arc::clone(&transport), true);
        let request = ExperimentRequest {
            experiment_id: "e".into(),
            spec: spec(),
            change: ChangeKind::CountryFloorSet {
                country: "ee".into(),
                floor_ecpm: 1,
            },
            prev_value: "{}".into(),
        };
        let error = adapter
            .start_experiment(&request)
            .await
            .expect_err("refuse");
        match &error {
            PlatformError::Schema { operation, .. } => assert_eq!(operation, "startExperiment"),
            other => panic!("expected a typed Schema error, got {other:?}"),
        }
        assert!(transport.seen.lock().unwrap().is_empty(), "no call at all");
    }

    #[tokio::test]
    async fn stop_is_the_one_documented_end_and_the_lift_surface_refuses_typed() {
        let resource = "accounts/pub-1234567890/mediationGroups/g1/mediationAbExperiment/exp-7";
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "name": resource,
                "mediationGroupId": "g1",
                "displayName": "autopilot-ecpm-g1",
                "state": "RUNNING",
                "variantLeader": "TOO_EARLY_TO_CALL"
            }),
        );
        transport.push(
            200,
            serde_json::json!({
                "name": resource,
                "mediationGroupId": "g1",
                "displayName": "autopilot-ecpm-g1",
                "state": "ENDED",
                "variantLeader": "TREATMENT"
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
        // The documented get reads state and verdict; it is not lift.
        let read = adapter.experiment(resource).await.expect("experiment");
        assert_eq!(read.state, apex_core::ExperimentState::Running);
        assert_eq!(read.winner_variant_id.as_deref(), Some("TOO_EARLY_TO_CALL"));
        adapter.deprecate_experiment(resource).await.expect("stop");
        {
            let seen = transport.seen.lock().unwrap();
            assert_eq!(seen.len(), 2, "resource read then stop");
            assert_eq!(seen[0].method, HttpMethod::Get);
            assert_eq!(seen[1].method, HttpMethod::Post);
            assert_eq!(
                seen[1].url,
                "https://admob.test/v1beta/accounts/pub-1234567890/mediationGroups/g1/mediationAbExperiments/exp-7:stop",
                "the full resource name stops on the documented nested route"
            );
        }

        // No measurement ever comes out of AdMob: the pull refuses with the
        // exact typed no-measurement reason, and no promotion is faked.
        let error = adapter
            .experiment_days(resource)
            .await
            .expect_err("no measurement");
        match &error {
            PlatformError::Schema {
                platform,
                operation,
                detail,
            } => {
                assert_eq!(*platform, PlatformId::Admob);
                assert_eq!(operation, "experimentDays");
                assert_eq!(
                    detail,
                    apex_lift::billing::ADMOB_NO_MEASUREMENT_REASON,
                    "the connector's constant and the fee gate's reason are the same phrase"
                );
            }
            other => panic!("expected a typed Schema error, got {other:?}"),
        }
        assert!(adapter.promote_experiment(resource).await.is_err());
        assert!(
            transport.seen.lock().unwrap().len() == 2,
            "refusals make no call"
        );
    }

    #[tokio::test]
    async fn current_settings_reads_the_mediation_group() {
        let transport = Arc::new(FakeTransport::default());
        let groups_body = serde_json::json!({
            "mediationGroups": [{
                "mediationGroupId": "g1",
                "name": "Interstitial group",
                "format": "interstitial",
                "adUnitMappings": [
                    {"adUnitId": "u1", "network": "admob", "ecpm": 2},
                    {"adUnitId": "u2", "network": "admob", "ecpm": "1.75"}
                ]
            }]
        });
        transport.push(200, groups_body.clone());
        transport.push(200, groups_body.clone());
        transport.push(200, groups_body);
        let adapter = ops(Arc::clone(&transport), true);
        let settings = adapter.current_settings("g1").await.expect("settings");
        let parsed: serde_json::Value = serde_json::from_str(&settings).expect("json");
        assert_eq!(parsed["mediationGroupId"], serde_json::json!("g1"));
        assert_eq!(parsed["mappings"][0]["ecpm"], serde_json::json!(2_000_000));
        // A mapping's ad unit id resolves the same group.
        let by_unit = adapter.current_settings("u2").await.expect("by unit");
        assert_eq!(by_unit, settings);
        // An unknown scope is a typed refusal, not an empty encoding.
        assert!(adapter.current_settings("nope").await.is_err());
    }

    #[tokio::test]
    async fn writes_are_refused_without_the_allow_list_and_capabilities_report() {
        let transport = Arc::new(FakeTransport::default());
        let adapter = ops(Arc::clone(&transport), false);
        assert!(!adapter.writable("updateMediationGroup").await);
        assert!(transport.seen.lock().unwrap().is_empty());

        let transport = Arc::new(FakeTransport::default());
        let adapter = ops(Arc::clone(&transport), true);
        assert_eq!(adapter.platform(), PlatformId::Admob);
        assert!(adapter.change_in_test_arm());
        assert!(
            !adapter.measures_variants(),
            "AdMob has no per-variant measurement"
        );
        assert!(adapter.call_log().is_empty());
        assert!(format!("{adapter:?}").contains("AdmobPlatformOps"));
        assert!(adapter
            .apply(
                &crate::model::Action::SetMappingEcpm {
                    mediation_group_id: "g1".into(),
                    ad_unit_id: "u2".into(),
                    ecpm: 1
                },
                "k"
            )
            .await
            .is_err());
        assert!(adapter
            .rollback(&crate::model::Action::SetMappingEcpm {
                mediation_group_id: "g1".into(),
                ad_unit_id: "u2".into(),
                ecpm: 1
            })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn treatment_leader_keeps_the_change_and_control_rolls_back() {
        let resource = "accounts/pub-1234567890/mediationGroups/g1/mediationAbExperiment/exp-7";
        for (leader, expected_keep) in [
            ("TREATMENT", true),
            ("VARIANT", true),
            ("CONTROL", false),
            ("INSUFFICIENT_DATA", false),
        ] {
            let transport = Arc::new(FakeTransport::default());
            transport.push(
                200,
                serde_json::json!({
                    "name": resource,
                    "mediationGroupId": "g1",
                    "displayName": "autopilot-ecpm-g1",
                    "state": "RUNNING",
                    "variantLeader": "TOO_EARLY_TO_CALL"
                }),
            );
            transport.push(
                200,
                serde_json::json!({
                    "name": resource,
                    "mediationGroupId": "g1",
                    "displayName": "autopilot-ecpm-g1",
                    "state": "ENDED",
                    "variantLeader": leader
                }),
            );
            let adapter = ops(Arc::clone(&transport), true);
            let conclusion = adapter
                .conclude_unmeasured_experiment(resource)
                .await
                .expect("conclude");
            assert_eq!(conclusion.experiment_state, "ENDED", "{leader}");
            assert_eq!(
                conclusion.variant_leader.as_deref(),
                Some(leader),
                "{leader}"
            );
            assert_eq!(conclusion.keep_treatment, expected_keep, "{leader}");
            let seen = transport.seen.lock().unwrap();
            assert_eq!(seen.len(), 2, "{leader}: read then stop");
            assert_eq!(seen[0].method, HttpMethod::Get);
            assert_eq!(seen[1].method, HttpMethod::Post);
            assert_eq!(
                seen[1].url,
                "https://admob.test/v1beta/accounts/pub-1234567890/mediationGroups/g1/mediationAbExperiments/exp-7:stop",
                "{leader}"
            );
            drop(seen);
        }
    }

    #[tokio::test]
    async fn stop_replay_is_safe_and_uses_the_same_documented_route() {
        let resource = "accounts/pub-1234567890/mediationGroups/g1/mediationAbExperiment/exp-7";
        let transport = Arc::new(FakeTransport::default());
        for _ in 0..2 {
            transport.push(
                200,
                serde_json::json!({
                    "name": resource,
                    "mediationGroupId": "g1",
                    "displayName": "autopilot-ecpm-g1",
                    "state": "ENDED",
                    "variantLeader": "TREATMENT"
                }),
            );
        }
        let adapter = ops(Arc::clone(&transport), true);
        adapter.deprecate_experiment(resource).await.expect("first");
        adapter
            .deprecate_experiment(resource)
            .await
            .expect("replay");
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        for call in seen.iter() {
            assert_eq!(call.method, HttpMethod::Post);
            assert_eq!(
                call.url,
                "https://admob.test/v1beta/accounts/pub-1234567890/mediationGroups/g1/mediationAbExperiments/exp-7:stop"
            );
        }
    }

    #[tokio::test]
    async fn conclusion_falls_back_to_stop_when_the_get_route_is_absent() {
        // Some recordings (and the shipped simulator) expose only create and
        // stop. The stop response is the same resource, so the conclusion is
        // still available and the end is not blocked on the optional read.
        let resource = "accounts/pub-1234567890/mediationGroups/g1/mediationAbExperiment/exp-7";
        let transport = Arc::new(FakeTransport::default());
        transport.push(404, serde_json::json!({"error": "no route"}));
        transport.push(
            200,
            serde_json::json!({
                "name": resource,
                "mediationGroupId": "g1",
                "displayName": "autopilot-ecpm-g1",
                "state": "ENDED",
                "variantLeader": "CONTROL"
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
        let conclusion = adapter
            .conclude_unmeasured_experiment(resource)
            .await
            .expect("conclude");
        assert_eq!(conclusion.variant_leader.as_deref(), Some("CONTROL"));
        assert!(!conclusion.keep_treatment);
        assert_eq!(transport.seen.lock().unwrap().len(), 2);
    }
}
