//! The live AdMob write adapter: monitoring and Managed, never a lift fee.
//!
//! AdMob's public API documents mediation A/B experiments as a lifecycle only
//! — `createMediationAbExperiment` and `stopMediationAbExperiment` — plus a
//! bare `variantLeader` verdict. It exposes **no per-variant revenue (or any
//! other per-variant metric)**, so the runner can start and stop an
//! experiment but can never measure lift from one:
//!
//! - [`start_experiment`](PlatformOps::start_experiment) creates the v1beta
//!   mediation A/B experiment carrying the proposed mediation-group eCPM
//!   change (the change rides the test arm; the group is untouched) and
//!   returns the experiment's resource name as the platform experiment id;
//! - [`deprecate_experiment`](PlatformOps::deprecate_experiment) stops the
//!   experiment through the v1beta stop call — the one documented end;
//! - the measured-lift surface refuses typed: there is no get/read method to
//!   trust ([`experiment`](PlatformOps::experiment)), and
//!   [`experiment_days`](PlatformOps::experiment_days) refuses with the
//!   no-measurement reason instead of inventing a signal. A promotion is
//!   never executed through the API either — the winner is applied in the
//!   AdMob UI.
//!
//! Because no measured lift can exist, the lift **fee** path excludes AdMob
//! at its eligibility gate (`apex_lift::billing::fee_eligibility`): a kept
//! AdMob change is recorded with a typed [`FeeIneligible`] decision and never
//! billed. Monitoring and the Managed experiment lifecycle are unaffected —
//! this adapter's job is exactly those.
//!
//! [`FeeIneligible`]: apex_lift::billing::FeeIneligible

use std::collections::BTreeMap;
use std::sync::Arc;

use apex_connectors::admob::AdmobClient;
use apex_connectors::transport::Transport;
use apex_core::{ExperimentResult, PlatformError, PlatformId};
use apex_proof::ChangeKind;
use async_trait::async_trait;

use crate::ops::{ExperimentDays, ExperimentRequest, PlatformOps};

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
            mediation_group_id, ..
        } = &request.change
        else {
            return Err(self.schema(
                "startExperiment",
                "an AdMob experiment carries a mediation-group eCPM change only",
            ));
        };
        let experiment = self
            .management
            .create_mediation_ab_experiment(mediation_group_id, &request.spec.name)
            .await?;
        Ok(experiment.name)
    }

    fn change_in_test_arm(&self) -> bool {
        true
    }

    /// AdMob documents no read method for a mediation A/B experiment (only
    /// create and stop), so the adapter refuses rather than guessing a state.
    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError> {
        Err(self.schema(
            "getExperiment",
            format!(
                "AdMob documents no read method for mediation A/B experiment '{experiment_id}'; only create and stop"
            ),
        ))
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
    /// created experiment's full resource name, so the trailing id segment is
    /// what the `:stop` path takes.
    async fn deprecate_experiment(&self, experiment_id: &str) -> Result<(), PlatformError> {
        let short_id = experiment_id
            .trim()
            .rsplit('/')
            .next()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| self.schema("stopMediationAbExperiment", "experiment id is empty"))?;
        self.management
            .stop_mediation_ab_experiment(short_id)
            .await?;
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
                "name": "accounts/pub-1234567890/mediationAbExperiments/exp-7",
                "mediationGroupId": "g1",
                "displayName": "autopilot-ecpm-g1",
                "state": "RUNNING",
                "variantLeader": null
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
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
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, HttpMethod::Post);
        assert_eq!(
            seen[0].url,
            "https://admob.test/v1beta/accounts/pub-1234567890/mediationAbExperiments"
        );
        let body = match seen[0].body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        assert_eq!(body["mediationGroupId"], serde_json::json!("g1"));
        assert_eq!(body["displayName"], serde_json::json!("autopilot-ecpm-g1"));
    }

    #[tokio::test]
    async fn the_mediation_floor_action_starts_the_group_ab_experiment() {
        // The frozen Action variant maps to the existing mediation-group eCPM
        // change; the adapter starts the v1beta experiment on that group.
        let transport = Arc::new(FakeTransport::default());
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
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, HttpMethod::Post);
        assert_eq!(
            seen[0].url,
            "https://admob.test/v1beta/accounts/pub-1234567890/mediationAbExperiments"
        );
        let body = match seen[0].body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        assert_eq!(body["mediationGroupId"], serde_json::json!("g1"));
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
        let transport = Arc::new(FakeTransport::default());
        transport.push(
            200,
            serde_json::json!({
                "name": "accounts/pub-1234567890/mediationAbExperiments/exp-7",
                "mediationGroupId": "g1",
                "displayName": "autopilot-ecpm-g1",
                "state": "STOPPED",
                "variantLeader": null
            }),
        );
        let adapter = ops(Arc::clone(&transport), true);
        adapter
            .deprecate_experiment("accounts/pub-1234567890/mediationAbExperiments/exp-7")
            .await
            .expect("stop");
        {
            let seen = transport.seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].method, HttpMethod::Post);
            assert!(seen[0]
                .url
                .ends_with("/v1beta/accounts/pub-1234567890/mediationAbExperiments/exp-7:stop"));
        }

        // No measurement ever comes out of AdMob: the pull refuses with the
        // exact typed no-measurement reason, and no promotion is faked.
        let error = adapter
            .experiment_days("accounts/pub-1234567890/mediationAbExperiments/exp-7")
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
        assert!(adapter
            .experiment("accounts/pub-1234567890/mediationAbExperiments/exp-7")
            .await
            .is_err());
        assert!(adapter
            .promote_experiment("accounts/pub-1234567890/mediationAbExperiments/exp-7")
            .await
            .is_err());
        assert!(
            transport.seen.lock().unwrap().len() == 1,
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
}
