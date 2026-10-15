//! The live AppLovin MAX write adapter: the production [`PlatformOps`].
//!
//! The MAX path never mutates the live ad unit. A cycle reads the unit's
//! current floors, folds the proposed country-floor change into a copy, and
//! starts the experiment with that copy as the **test arm's** configuration;
//! the control arm and the live unit stay exactly as they were. A kept result
//! promotes the experiment to all users, a rollback or void deprecates it.
//!
//! Daily pulls come from the Reporting API grouped by `max_ad_unit_test`: rows
//! with an empty arm label are the control arm, every other row is the test
//! arm. MAX reports no user counts, so the arm value is revenue divided by the
//! arm's allocated share (the window does that division), and a day is final
//! once it is strictly before the pull day.
//!
//! Every write is gated by a caller-supplied allow-list (the verify-live
//! gate): by default the adapter refuses all writes rather than guessing that
//! an endpoint is safe.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use apex_connectors::max::{MaxClient, MaxReportClient};
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

pub struct MaxPlatformOps {
    management: MaxClient,
    reporting: MaxReportClient,
    wall: Arc<dyn Wall>,
    /// Verify-live allow-list: an operation here (e.g. `setCountryFloor`) is
    /// writable; anything else is refused before any side effect.
    writable: BTreeMap<String, bool>,
}

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
        })
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
                match floors
                    .iter_mut()
                    .find(|floor| floor.country.eq_ignore_ascii_case(country))
                {
                    Some(existing) => existing.floor_ecpm = *floor_ecpm,
                    None => floors.push(CountryFloor {
                        country: country.clone(),
                        floor_ecpm: *floor_ecpm,
                        group: None,
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

#[async_trait]
impl PlatformOps for MaxPlatformOps {
    async fn writable(&self, operation: &str) -> bool {
        self.writable.get(operation).copied().unwrap_or(false)
    }

    async fn start_experiment(&self, request: &ExperimentRequest) -> Result<String, PlatformError> {
        let floors = self.test_arm_floors(request).await?;
        self.management
            .create_experiment(&request.spec, Some(&floors))
            .await
    }

    fn change_in_test_arm(&self) -> bool {
        true
    }

    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError> {
        self.management.get_experiment(experiment_id).await
    }

    async fn experiment_days(&self, experiment_id: &str) -> Result<ExperimentDays, PlatformError> {
        let today = self.wall.now_unix_s().div_euclid(DAY_SECS);
        let end = format_day(today - 1);
        let start = format_day(today - LOOKBACK_DAYS);
        let rows = self
            .reporting
            .experiment_rows(experiment_id, &start, &end)
            .await?;
        Ok(arms_from_rows(&rows, today))
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

/// Split reporting rows into the control and test arms by their
/// `max_ad_unit_test` label (empty means control).
fn arms_from_rows(rows: &[ReportRow], today_day: i64) -> ExperimentDays {
    let mut control: BTreeMap<String, i64> = BTreeMap::new();
    let mut test: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        let is_test = row
            .test_group
            .as_deref()
            .map(|label| !label.trim().is_empty())
            .unwrap_or(false);
        let map = if is_test { &mut test } else { &mut control };
        *map.entry(row.day.clone()).or_default() += row.revenue;
    }
    let days: BTreeSet<&String> = control.keys().chain(test.keys()).collect();
    let arm = |map: &BTreeMap<String, i64>, day: &str| -> apex_lift::window::ArmDay {
        let revenue = map.get(day).copied().unwrap_or(0);
        let final_day = day_number(day).is_some_and(|n| n < today_day);
        apex_lift::window::ArmDay {
            day: day.to_string(),
            revenue,
            users: None,
            second_pull_revenue: final_day.then_some(revenue),
            second_pull_users: None,
            fill: None,
        }
    };
    ExperimentDays {
        control: days.iter().map(|day| arm(&control, day)).collect(),
        test: days.iter().map(|day| arm(&test, day)).collect(),
    }
}

use crate::calendar::{day_number, format_day};

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use apex_connectors::transport::{HttpRequest, HttpResponse, Transport, TransportError};
    use apex_core::{ExperimentSpec, ExperimentState};

    use super::*;

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
    struct FixedWall(i64);
    impl Wall for FixedWall {
        fn now_unix_s(&self) -> i64 {
            self.0
        }
    }

    fn ops(transport: Arc<FakeTransport>, writable: bool) -> MaxPlatformOps {
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
            Arc::new(FixedWall(1_800_000_000)),
            write,
        )
        .expect("adapter")
    }

    fn spec() -> ExperimentSpec {
        ExperimentSpec {
            name: "autopilot-floor-u1-ee-2-5".into(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "raise the EE floor to 2.5".into(),
            test_group_allocation: 25,
        }
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
    fn arms_split_on_the_test_label_and_mark_final_days() {
        let today = 20_000;
        let rows = vec![
            ReportRow {
                day: format_day(today - 2),
                country: None,
                ad_unit_id: Some("u1".into()),
                test_group: Some(String::new()),
                revenue: 900,
                impressions: 3,
            },
            ReportRow {
                day: format_day(today - 2),
                country: None,
                ad_unit_id: Some("u1".into()),
                test_group: Some("test".into()),
                revenue: 350,
                impressions: 2,
            },
            ReportRow {
                day: format_day(today - 1),
                country: None,
                ad_unit_id: Some("u1".into()),
                test_group: Some("test".into()),
                revenue: 100,
                impressions: 1,
            },
            ReportRow {
                day: format_day(today),
                country: None,
                ad_unit_id: Some("u1".into()),
                test_group: Some("test".into()),
                revenue: 5,
                impressions: 1,
            },
        ];
        let arms = arms_from_rows(&rows, today);
        let older = format_day(today - 2);
        assert_eq!(arms.control[0].day, older);
        assert_eq!(arms.control[0].revenue, 900);
        assert_eq!(arms.control[0].second_pull_revenue, Some(900));
        assert_eq!(arms.test[0].revenue, 350);
        assert_eq!(arms.test[0].second_pull_revenue, Some(350));
        // A day strictly before the pull day is final; the pull day is not.
        assert_eq!(arms.test[1].second_pull_revenue, Some(100));
        assert_eq!(arms.test[2].second_pull_revenue, None);
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
        assert_eq!(floors.len(), 1, "existing group is reused");
        let countries = floors[0]["countries"]["values"]
            .as_array()
            .expect("countries");
        assert!(
            countries.contains(&serde_json::json!("us")),
            "existing floor kept"
        );
        assert!(
            countries.contains(&serde_json::json!("ee")),
            "the change is added"
        );
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
        assert_eq!(transport.seen.lock().unwrap().len(), 1, "no create call");
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
        let seen = transport.seen.lock().unwrap();
        assert!(seen[0].url.starts_with("https://r.example.test/maxReport?"));
        assert!(seen[0].url.contains("start=2026-11-13"));
        assert!(seen[0].url.contains("end=2027-01-14"));
    }
}
