//! Adversarial test suite for the connector layer: a scripted mock transport plus a
//! recording sleeper make every retry/error branch deterministic and socket-free.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use apex_core::{
    AdFormat, AdUnit, ExperimentMetric, ExperimentSpec, PlatformError, PlatformId,
    SourceVerification,
};
use async_trait::async_trait;

use crate::admob::{
    normalize_account_id, AdmobClient, MediationAbExperimentState, MediationAbVariantLeader,
    ADMOB_ENDPOINTS, ADMOB_REVENUE_PER_VARIANT,
};
use crate::app_ads::AppAdsTxtFetcher;
use crate::levelplay::{AbTestReportRow, LevelPlayClient, LEVELPLAY_ENDPOINTS};
use crate::max::{MaxClient, MaxReportClient, MAX_ENDPOINTS};
use crate::transport::{
    HttpBody, HttpMethod, HttpRequest, HttpResponse, PlatformCaller, RecordingSleeper,
    RetryTransport, Transport, TransportError, FORM_CONTENT_TYPE,
};

struct MockTransport {
    script: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
    calls: Mutex<Vec<HttpRequest>>,
}

impl MockTransport {
    fn ok(status: u16, body: Option<&str>) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![],
            body: body.map(|b| b.as_bytes().to_vec()).unwrap_or_default(),
        }
    }

    fn status(status: u16) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![],
            body: vec![],
        }
    }

    fn with_header(mut resp: HttpResponse, k: &str, v: &str) -> HttpResponse {
        resp.headers.push((k.to_string(), v.to_string()));
        resp
    }

    fn scripted(items: Vec<Result<HttpResponse, TransportError>>) -> Self {
        Self {
            script: Mutex::new(items.into()),
            calls: Mutex::new(vec![]),
        }
    }

    fn calls(&self) -> Vec<HttpRequest> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(req);
        }
        let mut script = self.script.lock().expect("script lock");
        if script.len() > 1 {
            return script.pop_front().unwrap_or(Ok(MockTransport::status(200)));
        }
        match script.front() {
            Some(r) => clone_result(r),
            None => Ok(MockTransport::status(200)),
        }
    }
}

fn clone_result(r: &Result<HttpResponse, TransportError>) -> Result<HttpResponse, TransportError> {
    match r {
        Ok(resp) => Ok(resp.clone()),
        Err(TransportError::Network(m)) => Err(TransportError::Network(m.clone())),
        Err(TransportError::Timeout(m)) => Err(TransportError::Timeout(m.clone())),
        Err(TransportError::Platform(_)) => {
            Err(TransportError::Network("platform passthrough".into()))
        }
    }
}

fn caller(
    transport: Arc<MockTransport>,
    sleeper: Arc<RecordingSleeper>,
    retries: u32,
) -> PlatformCaller {
    let retry = Arc::new(RetryTransport::new(transport, retries, sleeper));
    PlatformCaller::new(
        PlatformId::AppLovinMax,
        "https://t".into(),
        retry,
        ("X-API-Key".into(), "k".into()),
    )
}

// ---------------- transport matrix ----------------

#[tokio::test]
async fn auth_401_fails_fast_single_attempt() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::status(
        401,
    ))]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let c = caller(mock.clone(), sleeper.clone(), 2);
    let err = c
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, PlatformError::Auth { status: 401, .. }),
        "got {err}"
    );
    assert_eq!(mock.calls().len(), 1);
    assert!(sleeper.slept_ms().is_empty());
}

#[tokio::test]
async fn auth_403_fails_fast() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::status(
        403,
    ))]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let err = caller(mock.clone(), sleeper, 2)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Auth { status: 403, .. }));
    assert_eq!(mock.calls().len(), 1);
}

#[tokio::test]
async fn rate_limit_honors_retry_after_seconds() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::with_header(
            MockTransport::status(429),
            "Retry-After",
            "3",
        )),
        Ok(MockTransport::ok(200, Some("{\"ok\":true}"))),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let value = caller(mock.clone(), sleeper.clone(), 2)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(sleeper.slept_ms(), vec![3_000]);
}

#[tokio::test]
async fn rate_limit_without_header_uses_base_backoff() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::status(429)),
        Ok(MockTransport::ok(200, None)),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    caller(mock.clone(), sleeper.clone(), 2)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap();
    assert_eq!(sleeper.slept_ms(), vec![250]);
}

#[tokio::test]
async fn rate_limit_terminal_error_carries_retry_after() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(
        MockTransport::with_header(MockTransport::status(429), "Retry-After", "7"),
    )]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let err = caller(mock.clone(), sleeper, 1)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap_err();
    match err {
        PlatformError::RateLimited { retry_after, .. } => {
            assert_eq!(retry_after, Some(std::time::Duration::from_secs(7)));
        }
        other => panic!("expected RateLimited, got {other}"),
    }
    assert_eq!(mock.calls().len(), 2); // 1 + 1 retry
}

#[tokio::test]
async fn retry_after_cap_is_enforced() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::with_header(
            MockTransport::status(429),
            "Retry-After",
            "9999",
        )),
        Ok(MockTransport::ok(200, None)),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    caller(mock.clone(), sleeper.clone(), 2)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap();
    assert_eq!(sleeper.slept_ms(), vec![60_000]);
}

#[tokio::test]
async fn retry_after_http_date_far_future_is_capped() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::with_header(
            MockTransport::status(429),
            "Retry-After",
            "Thu, 01 Jan 2099 00:00:00 GMT",
        )),
        Ok(MockTransport::ok(200, None)),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    caller(mock.clone(), sleeper.clone(), 2)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap();
    assert_eq!(sleeper.slept_ms(), vec![60_000]);
}

#[tokio::test]
async fn server_error_exhausts_bounded_retries() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::status(503)),
        Ok(MockTransport::status(503)),
        Ok(MockTransport::status(503)),
        Ok(MockTransport::status(503)),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let err = caller(mock.clone(), sleeper.clone(), 2)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, PlatformError::Http { status: 503, .. }),
        "got {err}"
    );
    assert_eq!(mock.calls().len(), 3); // 1 + 2 retries
    assert_eq!(sleeper.slept_ms(), vec![250, 500]); // bounded exponential
}

#[tokio::test]
async fn other_4xx_is_immediate() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::status(
        404,
    ))]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let err = caller(mock.clone(), sleeper, 3)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Http { status: 404, .. }));
    assert_eq!(mock.calls().len(), 1);
}

#[tokio::test]
async fn non_idempotent_write_is_never_retried() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::status(503)),
        Ok(MockTransport::ok(200, None)),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let err = caller(mock.clone(), sleeper.clone(), 3)
        .call(
            "op",
            HttpMethod::Post,
            "/x",
            Some(serde_json::json!({})),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Http { status: 503, .. }));
    assert_eq!(mock.calls().len(), 1);
    assert!(sleeper.slept_ms().is_empty());
}

#[tokio::test]
async fn idempotent_write_retries_and_sends_key() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::status(500)),
        Ok(MockTransport::ok(200, Some("{\"done\":true}"))),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let value = caller(mock.clone(), sleeper, 2)
        .call(
            "op",
            HttpMethod::Post,
            "/x",
            Some(serde_json::json!({})),
            Some("k-1"),
        )
        .await
        .unwrap();
    assert_eq!(value["done"], true);
    let calls = mock.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].header("Idempotency-Key"), Some("k-1"));
}

#[tokio::test]
async fn network_error_retryable_for_reads_only() {
    let net = Err(TransportError::Network("ECONNRESET".into()));
    let mock = Arc::new(MockTransport::scripted(vec![
        net,
        Ok(MockTransport::ok(200, Some("1"))),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let value = caller(mock.clone(), sleeper.clone(), 2)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap();
    assert_eq!(value, serde_json::json!(1));
    assert_eq!(mock.calls().len(), 2);

    let mock2 = Arc::new(MockTransport::scripted(vec![
        Err(TransportError::Network("ECONNRESET".into())),
        Ok(MockTransport::ok(200, None)),
    ]));
    let sleeper2 = Arc::new(RecordingSleeper::default());
    let err = caller(mock2.clone(), sleeper2, 2)
        .call("op", HttpMethod::Delete, "/x", None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Network { .. }));
    assert_eq!(mock2.calls().len(), 1);
}

#[tokio::test]
async fn redirects_pass_through_the_retry_wrapper_without_a_retry() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::with_header(
            MockTransport::status(302),
            "Location",
            "https://example.com/app-ads.txt",
        )),
        Ok(MockTransport::ok(200, Some("after"))),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let retry: Arc<dyn Transport> = Arc::new(RetryTransport::new(mock.clone(), 2, sleeper.clone()));
    let request = HttpRequest {
        platform: PlatformId::Admob,
        operation: "fetchAppAdsTxt".into(),
        method: HttpMethod::Get,
        url: "https://example.com/app-ads.txt".into(),
        headers: vec![],
        body: None,
        timeout_ms: 1_000,
    };
    let response = retry
        .request(request)
        .await
        .expect("redirect passes through");
    assert_eq!(response.status, 302);
    assert_eq!(
        response.header("location"),
        Some("https://example.com/app-ads.txt")
    );
    assert_eq!(mock.calls().len(), 1, "a redirect is not retried");
    assert!(sleeper.slept_ms().is_empty(), "no backoff for a redirect");
}

#[tokio::test]
async fn app_ads_404_through_the_retry_wrapper_is_one_call_and_absent() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::status(
        404,
    ))]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let retry: Arc<dyn Transport> = Arc::new(RetryTransport::new(mock.clone(), 2, sleeper.clone()));
    let txt = AppAdsTxtFetcher::new(retry)
        .fetch(PlatformId::Admob, "example.com", vec!["admob.com".into()])
        .await
        .expect("404 is absent, not a failed fetch");
    assert!(!txt.present);
    assert_eq!(
        mock.calls().len(),
        1,
        "404 is a terminal verdict, never a retry storm"
    );
    assert!(sleeper.slept_ms().is_empty(), "no backoff for a 404");
}

#[tokio::test]
async fn invalid_json_fails_closed_as_schema() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some("not json"),
    ))]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let err = caller(mock, sleeper, 0)
        .call("op", HttpMethod::Get, "/x", None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }), "got {err}");
}

#[tokio::test]
async fn empty_success_body_becomes_null() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        204, None,
    ))]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let value = caller(mock, sleeper, 0)
        .call("op", HttpMethod::Delete, "/x", None, None)
        .await
        .unwrap();
    assert!(value.is_null());
}

// ---------------- MAX client ----------------

fn ok(status: u16, body: Option<&str>) -> Result<HttpResponse, TransportError> {
    Ok(MockTransport::ok(status, body))
}

fn max_client(mock: Arc<MockTransport>, sleeper: Arc<RecordingSleeper>) -> MaxClient {
    let retry: Arc<dyn Transport> = Arc::new(RetryTransport::new(mock, 2, sleeper));
    MaxClient::new("key-1", "https://max.test", retry).expect("client")
}

fn report_client(mock: Arc<MockTransport>) -> MaxReportClient {
    MaxReportClient::new("report-key", "https://r.test", mock).expect("report client")
}

#[tokio::test]
async fn max_credentials_required() {
    let transport: Arc<dyn Transport> = Arc::new(MockTransport::scripted(vec![]));
    assert!(MaxClient::new("  ", "https://x", transport.clone()).is_err());
    assert!(MaxReportClient::new("  ", "https://r.test", transport).is_err());
}

#[tokio::test]
async fn max_list_pins_header_and_top_level_array() {
    let mock = Arc::new(MockTransport::scripted(vec![ok(
        200,
        Some(
            r#"[{"id":"u1","name":"Banner","platform":"ios","package_name":"com.x","ad_format":"BANNER"}]"#,
        ),
    )]));
    let units = max_client(mock.clone(), Arc::new(RecordingSleeper::default()))
        .list_ad_units(false)
        .await
        .expect("units");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].format, AdFormat::Banner);
    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0]
        .headers
        .iter()
        .any(|(k, v)| k == "Api-Key" && v == "key-1"));
    assert!(calls[0].url.contains("/ad_units"), "{}", calls[0].url);
    assert!(!calls[0].url.contains("bid_floors"), "{}", calls[0].url);
}

#[tokio::test]
async fn max_list_rejects_the_old_envelope() {
    let mock = Arc::new(MockTransport::scripted(vec![ok(
        200,
        Some(r#"{"ad_units":[]}"#),
    )]));
    let err = max_client(mock, Arc::new(RecordingSleeper::default()))
        .list_ad_units(true)
        .await
        .expect_err("must fail");
    assert!(matches!(err, PlatformError::Schema { .. }), "{err}");
}

#[tokio::test]
async fn max_get_ad_unit_requests_floors_via_fields() {
    let mock = Arc::new(MockTransport::scripted(vec![ok(
        200,
        Some(
            r#"{"id":"u1","name":"B","platform":"ios","package_name":"com.x","ad_format":"BANNER","bid_floors":[{"country_group_name":"t1","cpm":"2.5","countries":{"type":"INCLUDE","values":["ee"]}}]}"#,
        ),
    )]));
    let unit = max_client(mock.clone(), Arc::new(RecordingSleeper::default()))
        .get_ad_unit("u1")
        .await
        .expect("unit");
    assert_eq!(
        unit.country_floors.as_ref().unwrap()[0].floor_ecpm,
        2_500_000
    );
    assert_eq!(unit.country_floors.as_ref().unwrap()[0].country, "ee");
    let calls = mock.calls();
    assert!(
        calls[0].url.contains("fields=bid_floors"),
        "{}",
        calls[0].url
    );
}

#[tokio::test]
async fn max_update_floors_requires_read_back_fields() {
    let unit = AdUnit {
        platform_unit_id: "u1".into(),
        name: "B".into(),
        format: AdFormat::Banner,
        enabled: true,
        country_floors: None,
        platform: None,
        package_name: None,
    };
    let client = max_client(
        Arc::new(MockTransport::scripted(vec![])),
        Arc::new(RecordingSleeper::default()),
    );
    let err = client
        .update_floors(&unit, &[])
        .await
        .expect_err("must fail without platform/package");
    assert!(matches!(err, PlatformError::Schema { .. }), "{err}");
}

#[tokio::test]
async fn max_create_experiment_pins_path_allocation_and_description() {
    let mock = Arc::new(MockTransport::scripted(vec![ok(
        200,
        Some(r#"{"id":"u1","experiment_name":"t"}"#),
    )]));
    let client = max_client(mock.clone(), Arc::new(RecordingSleeper::default()));
    let spec = ExperimentSpec {
        name: "t".into(),
        instance_id: String::new(),
        ad_unit_id: "u1".into(),
        metric: ExperimentMetric::Ecpm,
        variants: 2,
        change_description: "raise t1 floor".into(),
        test_group_allocation: 25,
    };
    let id = client.create_experiment(&spec, None).await.expect("create");
    assert_eq!(id, "u1");
    let calls = mock.calls();
    assert!(
        calls[0].url.contains("/ad_unit_experiment/u1"),
        "{}",
        calls[0].url
    );
    let body = match calls[0].body.as_ref().expect("body") {
        HttpBody::Json(value) => value.clone(),
        other => panic!("unexpected body {other:?}"),
    };
    assert_eq!(body["test_group_allocation"], 25);
    assert_eq!(body["experiment_name"], "t");
}

#[tokio::test]
async fn max_create_experiment_rejects_bad_allocation_and_blank_description() {
    let client = max_client(
        Arc::new(MockTransport::scripted(vec![])),
        Arc::new(RecordingSleeper::default()),
    );
    let mut spec = ExperimentSpec {
        name: "t".into(),
        instance_id: String::new(),
        ad_unit_id: "u1".into(),
        metric: ExperimentMetric::Ecpm,
        variants: 2,
        change_description: "raise t1 floor".into(),
        test_group_allocation: 33,
    };
    assert!(matches!(
        client.create_experiment(&spec, None).await,
        Err(PlatformError::Validation(_))
    ));
    spec.test_group_allocation = 50;
    spec.change_description = "  ".into();
    assert!(matches!(
        client.create_experiment(&spec, None).await,
        Err(PlatformError::Validation(_))
    ));
}

#[tokio::test]
async fn max_report_totals_sum_rows() {
    let mock = Arc::new(MockTransport::scripted(vec![ok(
        200,
        Some(
            r#"{"code":200,"count":2,"results":[
            {"day":"2026-10-01","estimated_revenue":"1.50","impressions":"10"},
            {"day":"2026-10-02","estimated_revenue":"2.00","impressions":"5"}
        ]}"#,
        ),
    )]));
    let (revenue, impressions) = report_client(mock)
        .totals("2026-10-01", "2026-10-02")
        .await
        .expect("totals");
    assert_eq!(revenue, 3_500_000);
    assert_eq!(impressions, 15);
}

#[tokio::test]
async fn max_report_http_errors_surface() {
    let mock = Arc::new(MockTransport::scripted(vec![ok(
        200,
        Some(r#"{"code":400,"results":[],"count":0}"#),
    )]));
    let err = report_client(mock)
        .totals("2026-10-01", "2026-10-02")
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, PlatformError::Http { status: 400, .. }),
        "{err}"
    );
}

fn lp_client(mock: Arc<MockTransport>) -> LevelPlayClient {
    lp_client_with_key(mock, "acct")
}

fn lp_client_with_key(mock: Arc<MockTransport>, app_key: &str) -> LevelPlayClient {
    let retry = Arc::new(RetryTransport::new(
        mock,
        0,
        Arc::new(RecordingSleeper::default()),
    ));
    LevelPlayClient::new("tok", "refresh", app_key, "https://lp.test", retry).expect("client")
}

#[tokio::test]
async fn lp_impression_revenue_day_uses_the_csv_feed_not_the_reporting_api() {
    let csv = "Date,User ID,IDFV,IDFA,Revenue,Impressions,AB Testing,Platform\n\
2026-10-01 10:00:00,u1,idfv-1,idfa-1,0.500000,2,B,android\n\
2026-10-01 11:00:00,u1,idfv-1,idfa-1,0.250000,1,B,android\n\
2026-10-01 12:00:00,,idfv-2,,0.100000,1,A,android\n\
2026-10-01 13:00:00,,,,0.050000,1,A,android\n";
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer-token""#))),
        Ok(MockTransport::ok(200, Some(csv))),
    ]));
    let rows = lp_client(mock.clone())
        .impression_revenue_day("2026-10-01", "android")
        .await
        .expect("rows");
    // u1 folds across its two impressions; the IDFV player resolves; the
    // anonymous row is kept for the uncovered-share report.
    assert_eq!(rows.len(), 3, "{rows:?}");
    let user = rows
        .iter()
        .find(|row| row.player_key().as_deref() == Some("u1"))
        .expect("u1");
    assert_eq!(user.revenue, 750_000);
    assert_eq!(user.impressions, 3);
    assert_eq!(user.ab_test.as_deref(), Some("B"));
    assert_eq!(user.platform, "android");
    assert!(rows.iter().any(|row| row.player_key().is_none()));
    let calls = mock.calls();
    assert_eq!(calls.len(), 2, "auth + one impression feed read");
    assert!(calls[0].url.contains("/partners/publisher/auth"));
    assert_eq!(
        calls[1].url,
        "https://lp.test/levelPlay/impressionLevelRevenue/v1?appKey=acct&date=2026-10-01"
    );
    assert!(calls[1]
        .headers
        .iter()
        .any(|(name, value)| name == "Authorization" && value == "Bearer bearer-token"));
    assert!(!calls[1].url.contains("/levelPlay/reporting/v1"));
}

#[tokio::test]
async fn lp_impression_revenue_day_rejects_bad_days_and_missing_columns() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer-token""#))),
        Ok(MockTransport::ok(200, Some("Revenue,Impressions\n1.0,1\n"))),
    ]));
    let client = lp_client(mock);
    assert!(client
        .impression_revenue_day("2026-13-01", "android")
        .await
        .is_err());
    assert!(client
        .impression_revenue_day("2026-10-01", "android")
        .await
        .is_err());
}

#[tokio::test]
async fn lp_group_capping_writes_the_variant_group_contract() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer-token""#))),
        Ok(MockTransport::ok(200, Some("{}"))),
    ]));
    lp_client(mock.clone())
        .update_group_capping("g-b", Some(3), Some(1), Some(12))
        .await
        .expect("capping");
    let calls = mock.calls();
    assert_eq!(calls.len(), 2, "auth + one capping write");
    assert_eq!(calls[1].method, HttpMethod::Post);
    assert_eq!(calls[1].url, "https://lp.test/levelPlay/groups/v4/acct");
    match calls[1].body.as_ref().expect("body") {
        HttpBody::Json(value) => assert_eq!(
            *value,
            serde_json::json!({
                "groupId": "g-b",
                "sessionLimit": 3,
                "interstitialLimit": 1,
                "dayLimit": 12
            })
        ),
        other => panic!("expected JSON body, got {other:?}"),
    }
}

#[tokio::test]
async fn lp_list_waterfall_and_fallback() {
    let body = r#"{"instances":[
        {"instanceId":"e1","adapterNetwork":"admob","instanceName":"ADM_1","position":1,"price":3.2},
        {"id":"e2","network":"facebook","position":2,"price":"1.1"}
    ]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer-token""#))),
        Ok(MockTransport::ok(200, Some(body))),
    ]));
    let entries = lp_client(mock.clone()).list_waterfall("u/1").await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].instance_name, "ADM_1");
    assert_eq!(
        entries[1].instance_name, "e2",
        "fallback to the instance id"
    );
    assert_eq!(entries[0].price_ecpm, 3_200_000);
    assert_eq!(
        entries[1].price_ecpm, 1_100_000,
        "string decimals parse exactly"
    );
    // The first call mints the bearer; the second carries it. The documented
    // bearer exchange is a GET with the two credential headers.
    assert_eq!(mock.calls()[0].method, HttpMethod::Get);
    assert!(mock.calls()[0].url.ends_with("/partners/publisher/auth"));
    assert!(mock.calls()[1]
        .header("Authorization")
        .unwrap()
        .starts_with("Bearer "));
    // Byte-exact instances v4 URL: the app key is a path segment, not a query
    // parameter.
    assert_eq!(mock.calls()[1].method, HttpMethod::Get);
    assert_eq!(
        mock.calls()[1].url,
        "https://lp.test/levelPlay/network/instances/v4/acct/"
    );
    assert!(
        !mock.calls()[1].url.contains("?appKey="),
        "the app key must never be a query parameter: {}",
        mock.calls()[1].url
    );
    assert!(
        !mock.calls()[1].url.contains("partners/publisher/mediation"),
        "the old partner-gate mediation path must be gone: {}",
        mock.calls()[1].url
    );

    // A present but unparseable rate is still a typed refusal; an absent
    // rate is the documented "no instance-level rate" state and maps to 0.
    let drift = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(
            200,
            Some(r#"{"instances":[{"instanceId":"e","network":"x","rate":"not-a-rate"}]}"#),
        )),
    ]));
    let err = lp_client(drift).list_waterfall("u").await.unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }));
}

#[tokio::test]
async fn lp_v4_instances_read_numeric_ids_rate_and_network_name() {
    // The documented Instances v4 GET is a bare array with numeric
    // `instanceId`, `networkName` and an optional `rate` (not `price`).
    let body = r#"[
        {"instanceId":1072562,"instanceName":"Rewarded50","networkName":"unityAds",
         "adFormat":"rewarded","isBidder":true,"groups":[101663],"isLive":true},
        {"instanceId":1014428,"instanceName":"Default","networkName":"adManager",
         "adFormat":"interstitial","isBidder":false,"rate":25}
    ]"#;
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer-token""#))),
        Ok(MockTransport::ok(200, Some(body))),
    ]));
    let entries = lp_client(mock.clone()).list_waterfall("").await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].entry_id, "1072562");
    assert_eq!(entries[0].network, "unityAds");
    assert_eq!(
        entries[0].price_ecpm, 0,
        "an absent instance-level rate is the documented no-rate state"
    );
    assert_eq!(entries[1].entry_id, "1014428");
    assert_eq!(entries[1].network, "adManager");
    assert_eq!(entries[1].price_ecpm, 25_000_000);
    assert_eq!(
        entries[1].position, 2,
        "an absent position falls back to the row index"
    );
    assert_eq!(mock.calls()[0].method, HttpMethod::Get);
    assert_eq!(mock.calls()[1].method, HttpMethod::Get);
}

#[tokio::test]
async fn lp_reporting_v1_data_envelope_and_pagination() {
    // The documented envelope is data/page/pageSize/totalResults. The first
    // request stays byte-identical to the documented query; the continuation
    // names the page and page size.
    let page1 = r#"{"data":[
        {"date":"2026-10-01","abTest":"A","revenue":12.5,"activeUsers":120}
    ],"page":1,"pageSize":1,"totalResults":2}"#;
    let page2 = r#"{"data":[
        {"date":"2026-10-01","abTest":"B","revenue":13.25,"activeUsers":95}
    ],"page":2,"pageSize":1,"totalResults":2}"#;
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, Some(page1))),
        Ok(MockTransport::ok(200, Some(page2))),
    ]));
    let rows = lp_client(mock.clone())
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .expect("rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].revenue, 12_500_000);
    assert_eq!(rows[1].revenue, 13_250_000);
    assert_eq!(rows[1].active_users, 95);
    let calls = mock.calls();
    assert_eq!(calls.len(), 3, "auth plus two report pages");
    assert_eq!(
        calls[1].url,
        "https://lp.test/levelPlay/reporting/v1?appKey=acct&breakdowns=date,abTest&metrics=revenue,activeUsers&startDate=2026-10-01&endDate=2026-10-14"
    );
    assert_eq!(
        calls[2].url,
        "https://lp.test/levelPlay/reporting/v1?appKey=acct&breakdowns=date,abTest&metrics=revenue,activeUsers&startDate=2026-10-01&endDate=2026-10-14&page=2&resultsPerPage=5000"
    );
}

#[tokio::test]
async fn lp_reporting_truncated_pagination_is_a_typed_refusal() {
    // The server announces three rows but returns one and then an empty page:
    // the client must refuse the short measurement, never report it complete.
    let page1 = r#"{"data":[
        {"date":"2026-10-01","abTest":"A","revenue":1.0,"activeUsers":10}
    ],"page":1,"pageSize":1,"totalResults":3}"#;
    let page2 = r#"{"data":[],"page":2,"pageSize":0,"totalResults":3}"#;
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, Some(page1))),
        Ok(MockTransport::ok(200, Some(page2))),
    ]));
    let error = lp_client(mock)
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .expect_err("truncated");
    match &error {
        PlatformError::Schema { detail, .. } => {
            assert!(detail.contains("pagination truncated"), "{detail}");
            assert!(detail.contains("1 of 3"), "{detail}");
        }
        other => panic!("expected a typed Schema error, got {other:?}"),
    }
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn lp_a_401_refreshes_the_bearer_once_and_retries_the_call() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer-one""#))),
        Ok(MockTransport::status(401)),
        Ok(MockTransport::ok(200, Some(r#""bearer-two""#))),
        Ok(MockTransport::ok(
            200,
            Some(r#"{"instances":[{"instanceId":1,"networkName":"unityAds","rate":2}]}"#),
        )),
    ]));
    let entries = lp_client(mock.clone()).list_waterfall("u1").await.unwrap();
    assert_eq!(entries.len(), 1);
    let calls = mock.calls();
    assert_eq!(calls.len(), 4, "auth, 401, re-auth, retried read");
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.url.ends_with("/partners/publisher/auth"))
            .count(),
        2,
        "the dead token is re-minted exactly once"
    );
    assert_eq!(calls[1].header("Authorization"), Some("Bearer bearer-one"));
    assert_eq!(calls[3].header("Authorization"), Some("Bearer bearer-two"));
}

#[tokio::test]
async fn lp_auth_401_is_a_typed_auth_refusal() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::status(
        401,
    ))]));
    let error = lp_client(mock)
        .list_waterfall("u1")
        .await
        .expect_err("auth");
    assert!(
        matches!(error, PlatformError::Auth { status: 401, .. }),
        "{error}"
    );
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn lp_instances_and_groups_recordings_are_byte_exact() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, Some(r#"{"instances":[]}"#))),
        Ok(MockTransport::ok(
            200,
            Some(
                r#"{"groups":[{"groupId":"g1","name":"Main","floorPrice":1.5,"countriesRate":0.8}]}"#,
            ),
        )),
    ]));
    let client = lp_client(mock.clone());
    assert!(client.list_waterfall("u1").await.unwrap().is_empty());
    let groups = client.list_groups().await.expect("groups");
    assert_eq!(groups["groups"][0]["groupId"], "g1");
    assert_eq!(groups["groups"][0]["floorPrice"], 1.5);
    assert_eq!(groups["groups"][0]["countriesRate"], 0.8);
    let calls = mock.calls();
    // Recorded pair 1: instances v4, app key in the path, GET.
    assert_eq!(calls[1].method, HttpMethod::Get);
    assert_eq!(
        calls[1].url,
        "https://lp.test/levelPlay/network/instances/v4/acct/"
    );
    // Recorded pair 2: groups v4, app key in the path, GET.
    assert_eq!(calls[2].method, HttpMethod::Get);
    assert_eq!(calls[2].url, "https://lp.test/levelPlay/groups/v4/acct");
    assert_eq!(calls[2].header("Authorization"), Some("Bearer bearer"));
    for call in &calls {
        assert!(
            !call.url.contains("partners/publisher/mediation"),
            "no call may emit the old partner-gate form: {}",
            call.url
        );
    }
}

#[tokio::test]
async fn lp_hostile_app_key_is_percent_encoded_in_every_path() {
    let hostile = "app key/../x?y#z%";
    let encoded = "app%20key%2F..%2Fx%3Fy%23z%25";
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, Some(r#"{"instances":[]}"#))),
        Ok(MockTransport::ok(200, Some(r#"{"groups":[]}"#))),
        Ok(MockTransport::ok(
            200,
            Some(r#"{"code":200,"count":0,"results":[]}"#),
        )),
    ]));
    let client = lp_client_with_key(mock.clone(), hostile);
    client.list_waterfall("u1").await.unwrap();
    client.list_groups().await.unwrap();
    client
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .unwrap();
    let calls = mock.calls();
    assert_eq!(
        calls[1].url,
        format!("https://lp.test/levelPlay/network/instances/v4/{encoded}/")
    );
    assert_eq!(
        calls[2].url,
        format!("https://lp.test/levelPlay/groups/v4/{encoded}")
    );
    assert_eq!(
        calls[3].url,
        format!(
            "https://lp.test/levelPlay/reporting/v1?appKey={encoded}&breakdowns=date,abTest&metrics=revenue,activeUsers&startDate=2026-10-01&endDate=2026-10-14"
        )
    );
    for call in calls.iter().skip(1) {
        assert!(
            !call.url.contains(hostile),
            "the raw hostile key must never leak into the URL: {}",
            call.url
        );
    }
}

#[tokio::test]
async fn lp_reporting_ab_test_recording_parses_revenue_and_active_users() {
    let body = r#"{"code":200,"count":2,"results":[
        {"date":"2026-10-01","abTest":"Control","revenue":"12.50","activeUsers":"120"},
        {"date":"2026-10-01","abTest":"Variant A","revenue":13.25,"activeUsers":95}
    ]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, Some(body))),
    ]));
    let rows = lp_client(mock.clone())
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .expect("rows");
    assert_eq!(
        rows,
        vec![
            AbTestReportRow {
                day: "2026-10-01".into(),
                ab_test: "Control".into(),
                revenue: 12_500_000,
                active_users: 120,
            },
            AbTestReportRow {
                day: "2026-10-01".into(),
                ab_test: "Variant A".into(),
                revenue: 13_250_000,
                active_users: 95,
            },
        ],
        "the decimal currency amount parses exactly into micros"
    );
    let call = &mock.calls()[1];
    assert_eq!(call.method, HttpMethod::Get);
    assert_eq!(
        call.url,
        "https://lp.test/levelPlay/reporting/v1?appKey=acct&breakdowns=date,abTest&metrics=revenue,activeUsers&startDate=2026-10-01&endDate=2026-10-14"
    );
    assert!(call.url.contains("metrics=revenue,activeUsers"));
    assert!(call.url.contains("breakdowns=date,abTest"));
    assert!(call.header("Authorization").unwrap().starts_with("Bearer "));
}

#[tokio::test]
async fn lp_reporting_requires_every_row_field_naming_it() {
    // Each case omits exactly one required field; the typed schema error must
    // name that field instead of silently zeroing the measurement.
    let cases = [
        (
            r#"{"code":200,"count":1,"results":[{"date":"2026-10-01","abTest":"A","activeUsers":10}]}"#,
            "revenue",
        ),
        (
            r#"{"code":200,"count":1,"results":[{"date":"2026-10-01","abTest":"A","revenue":"1.00"}]}"#,
            "activeUsers",
        ),
        (
            r#"{"code":200,"count":1,"results":[{"abTest":"A","revenue":"1.00","activeUsers":10}]}"#,
            "date",
        ),
    ];
    for (body, field) in cases {
        let mock = Arc::new(MockTransport::scripted(vec![
            Ok(MockTransport::ok(200, Some(r#""bearer""#))),
            Ok(MockTransport::ok(200, Some(body))),
        ]));
        let error = lp_client(mock)
            .ab_test_report("2026-10-01", "2026-10-14")
            .await
            .expect_err("missing field");
        match &error {
            PlatformError::Schema { detail, .. } => {
                assert!(detail.contains(field), "names '{field}': {detail}")
            }
            other => panic!("expected a typed Schema error, got {other:?}"),
        }
        assert!(!error.is_retryable(), "schema drift is not retryable");
    }
}

#[tokio::test]
async fn lp_app_revenue_sums_the_reporting_month() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(
            200,
            Some(
                r#"{"code":200,"count":3,"results":[
                    {"date":"2026-10-01","revenue":"12.50"},
                    {"date":"2026-10-02","revenue":13.25},
                    {"date":"2026-10-02","revenue":"0.25"}
                ]}"#,
            ),
        )),
    ]));
    let revenue = lp_client(mock.clone())
        .app_revenue("2026-10-01", "2026-10-31")
        .await
        .expect("revenue");
    assert_eq!(revenue, 26_000_000, "12.50 + 13.25 + 0.25 = 26.00 units");
    let call = &mock.calls()[1];
    assert_eq!(call.method, HttpMethod::Get);
    assert_eq!(
        call.url,
        "https://lp.test/levelPlay/reporting/v1?appKey=acct&breakdowns=date&metrics=revenue&startDate=2026-10-01&endDate=2026-10-31"
    );
}

#[tokio::test]
async fn lp_app_revenue_is_read_once_per_window_for_every_scope() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(
            200,
            Some(r#"{"code":200,"count":1,"results":[{"date":"2026-10-01","revenue":"10.00"}]}"#),
        )),
    ]));
    let client = lp_client(mock.clone());
    for _ in 0..5 {
        assert_eq!(
            client
                .app_revenue("2026-10-01", "2026-10-31")
                .await
                .expect("revenue"),
            10_000_000
        );
    }
    assert_eq!(
        mock.calls().len(),
        2,
        "one auth plus one revenue read, not one read per ad-unit scope"
    );
}

#[tokio::test]
async fn lp_reporting_error_mapping_is_typed() {
    // In-body code != 200 maps to Http with that status.
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(
            200,
            Some(r#"{"code":403,"count":0,"results":[]}"#),
        )),
    ]));
    let err = lp_client(mock)
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, PlatformError::Http { status: 403, .. }),
        "{err}"
    );

    // A response without the results array is a schema refusal.
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, Some(r#"{"code":200}"#))),
    ]));
    let err = lp_client(mock)
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .expect_err("must fail");
    assert!(matches!(err, PlatformError::Schema { .. }), "{err}");

    // A malformed row (non-numeric activeUsers) is a schema refusal, not zero.
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(
            200,
            Some(
                r#"{"code":200,"count":1,"results":[{"date":"2026-10-01","abTest":"A","revenue":"1.00","activeUsers":"not a number"}]}"#,
            ),
        )),
    ]));
    let err = lp_client(mock)
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .expect_err("must fail");
    assert!(matches!(err, PlatformError::Schema { .. }), "{err}");

    // A row missing the abTest arm is a schema refusal.
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(
            200,
            Some(
                r#"{"code":200,"count":1,"results":[{"date":"2026-10-01","revenue":"1.00","activeUsers":1}]}"#,
            ),
        )),
    ]));
    let err = lp_client(mock)
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .expect_err("must fail");
    assert!(matches!(err, PlatformError::Schema { .. }), "{err}");

    // An HTTP failure status is classified by the retry layer: a terminal 404
    // is Http, a 429 is RateLimited.
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::status(404)),
    ]));
    let err = lp_client(mock)
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, PlatformError::Http { status: 404, .. }),
        "{err}"
    );

    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::status(429)),
    ]));
    let err = lp_client(mock)
        .ab_test_report("2026-10-01", "2026-10-14")
        .await
        .expect_err("must fail");
    assert!(matches!(err, PlatformError::RateLimited { .. }), "{err}");
}

#[tokio::test]
async fn lp_price_update_pins_body_and_key() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, None)),
    ]));
    let entry = lp_client(mock.clone())
        .update_entry_price("u1", "e1", 2_345_000)
        .await
        .unwrap();
    assert_eq!(entry.price_ecpm, 2_345_000);
    let call = &mock.calls()[1];
    assert_eq!(call.method, HttpMethod::Put);
    assert_eq!(
        call.url,
        "https://lp.test/levelPlay/network/instances/v4/acct/"
    );
    let body = call.body.as_ref().and_then(HttpBody::as_json).unwrap();
    assert_eq!(body[0]["instanceId"], "e1");
    assert_eq!(body[0]["rate"], "2.345");
    assert_eq!(
        call.header("Idempotency-Key"),
        Some("levelplay-price:u1:e1:2.345")
    );

    let client = lp_client(mock.clone());
    assert!(matches!(
        client.update_entry_price("u", "e", 0).await,
        Err(PlatformError::Validation(_))
    ));
    assert!(matches!(
        client.update_entry_price("u", "e", i64::MAX).await,
        Err(PlatformError::Validation(_))
    ));
}

#[tokio::test]
async fn lp_create_and_remove_contract() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, Some(r#"{"instanceId":"new-1"}"#))),
        Ok(MockTransport::ok(204, None)),
    ]));
    let client = lp_client(mock.clone());
    let id = client
        .create_entry("u1", "mintegral", "MTG_1", 750_000)
        .await
        .unwrap();
    assert_eq!(id, "new-1");
    assert_eq!(mock.calls()[1].method, HttpMethod::Post);
    assert_eq!(
        mock.calls()[1].url,
        "https://lp.test/levelPlay/network/instances/v4/acct/"
    );
    assert_eq!(
        mock.calls()[1].header("Idempotency-Key"),
        Some("levelplay-create:u1:mintegral:MTG_1:0.75")
    );
    client.remove_entry("u1", "e9").await.unwrap();
    assert_eq!(mock.calls()[2].method, HttpMethod::Delete);
    assert_eq!(
        mock.calls()[2].url,
        "https://lp.test/levelPlay/network/instances/v4/acct/"
    );
    assert_eq!(
        mock.calls()[2]
            .body
            .as_ref()
            .and_then(HttpBody::as_json)
            .cloned(),
        Some(serde_json::json!({"ids": ["e9"]})),
        "v4 deletes by an ids array in the body"
    );
    assert_eq!(
        mock.calls()[2].header("Idempotency-Key"),
        Some("levelplay-remove:u1:e9")
    );

    assert!(matches!(
        client.create_entry("u", " ", "i", 1_000_000).await,
        Err(PlatformError::Schema { .. })
    ));
    let missing = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, Some("{}"))),
    ]));
    assert!(matches!(
        lp_client(missing)
            .create_entry("u", "n", "i", 1_000_000)
            .await,
        Err(PlatformError::Schema { .. })
    ));
}

#[tokio::test]
async fn lp_bearer_token_is_minted_once_and_reused() {
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(200, Some(r#"{"instances":[]}"#))),
        Ok(MockTransport::ok(200, Some(r#"{"instances":[]}"#))),
    ]));
    let client = lp_client(mock.clone());
    client.list_waterfall("u1").await.unwrap();
    client.list_waterfall("u2").await.unwrap();
    let auth_calls = mock
        .calls()
        .iter()
        .filter(|call| call.url.ends_with("/partners/publisher/auth"))
        .count();
    assert_eq!(auth_calls, 1, "the token is minted once and cached");
}

// ---------------- AdMob client ----------------

fn admob_client(mock: Arc<MockTransport>) -> AdmobClient {
    let retry = Arc::new(RetryTransport::new(
        mock,
        0,
        Arc::new(RecordingSleeper::default()),
    ));
    AdmobClient::new("token", "pub-1234567890123456", "https://admob.test", retry).expect("client")
}

#[tokio::test]
async fn admob_account_normalization() {
    assert_eq!(
        normalize_account_id("pub-1234567890123456").unwrap(),
        "accounts/pub-1234567890123456"
    );
    assert_eq!(
        normalize_account_id("accounts/pub-1234567890123456").unwrap(),
        "accounts/pub-1234567890123456"
    );
    assert!(normalize_account_id("1234").is_err());
    assert!(normalize_account_id("pub-12").is_err());
    assert!(normalize_account_id("pub-12345x").is_err());
}

const ADMOB_GROUP: &str = r#"{"mediationGroupId":"g1","name":"Interstitial group","format":"interstitial","adUnitMappings":[
    {"adUnitId":"u1","network":"admob","ecpm":2},
    {"adUnitId":"u2","network":"facebook","eCPM":1.0}
]}"#;

#[tokio::test]
async fn admob_pagination_follows_tokens_and_bounds_runaway() {
    let page1 = format!(r#"{{"mediationGroups":[{ADMOB_GROUP}],"nextPageToken":"t1"}}"#);
    let page2 = r#"{"mediationGroups":[],"nextPageToken":""}"#;
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(&page1))),
        Ok(MockTransport::ok(200, Some(page2))),
    ]));
    let groups = admob_client(mock.clone())
        .list_mediation_groups()
        .await
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].mappings[1].ecpm, 1_000_000, "eCPM alias parses");
    // Recorded pair: mediation groups now live on v1beta; the v1 base must
    // never be emitted.
    let calls = mock.calls();
    assert_eq!(calls[0].method, HttpMethod::Get);
    assert_eq!(
        calls[0].url,
        "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationGroups"
    );
    assert_eq!(
        calls[1].url,
        "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationGroups?pageToken=t1"
    );
    for call in &calls {
        assert!(
            !call.url.contains("/v1/accounts"),
            "the v1 mediation-group base must be gone: {}",
            call.url
        );
    }

    let runaway_page = r#"{"mediationGroups":[],"nextPageToken":"again"}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(runaway_page),
    ))]));
    let err = admob_client(mock.clone())
        .list_mediation_groups()
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }));
    assert_eq!(mock.calls().len(), 50, "hard page bound enforced");
}

#[tokio::test]
async fn admob_update_mapping_is_read_modify_write_full_list() {
    let list = format!(r#"{{"mediationGroups":[{ADMOB_GROUP}]}}"#);
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(&list))),
        Ok(MockTransport::ok(200, None)),
    ]));
    let updated = admob_client(mock.clone())
        .update_mapping_ecpm("g1", "u2", 1_750_000)
        .await
        .unwrap();
    assert_eq!(updated.ecpm, 1_750_000);
    let patch = &mock.calls()[1];
    assert_eq!(patch.method, HttpMethod::Patch);
    assert_eq!(
        patch.url,
        "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationGroups/g1"
    );
    let mappings = patch.body.as_ref().and_then(HttpBody::as_json).unwrap()["adUnitMappings"]
        .as_array()
        .unwrap();
    assert_eq!(mappings.len(), 2, "FULL list written back");
    assert_eq!(
        mappings[0]["ecpm"], "2",
        "untouched mapping preserved as a currency amount"
    );
    assert_eq!(
        mappings[1]["ecpm"], "1.75",
        "the write is a currency amount, not integer micros"
    );
    assert_eq!(
        crate::parse_decimal_micros(mappings[1]["ecpm"].as_str().expect("string")).unwrap(),
        1_750_000,
        "the written value reads back as the same micros"
    );
    assert_eq!(
        patch.header("Idempotency-Key"),
        Some("admob-ecpm:g1:u2:1.75")
    );
}

#[tokio::test]
async fn admob_update_rejects_unknown_group_unit_and_bad_ecpm() {
    let list = format!(r#"{{"mediationGroups":[{ADMOB_GROUP}]}}"#);
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(&list),
    ))]));
    let client = admob_client(mock.clone());
    assert!(matches!(
        client.update_mapping_ecpm("gX", "u1", 1_000_000).await,
        Err(PlatformError::Schema { .. })
    ));
    assert!(matches!(
        client.update_mapping_ecpm("g1", "nope", 1_000_000).await,
        Err(PlatformError::Schema { .. })
    ));
    assert!(matches!(
        client.update_mapping_ecpm("g1", "u1", 0).await,
        Err(PlatformError::Validation(_))
    ));
    assert!(matches!(
        client.update_mapping_ecpm("g1", "u1", 500_000_001).await,
        Err(PlatformError::Validation(_))
    ));
    // Range validation runs before the group read; the two lookup errors each cost
    // exactly one list call and never reach the PATCH.
    assert_eq!(
        mock.calls().len(),
        2,
        "range validation performed zero calls; lookups cost one list each"
    );
}

#[tokio::test]
async fn admob_mediation_revenue_reads_the_documented_report_fixture() {
    // Recorded v1beta mediationReport:generate shape: a POST with the date
    // range, DATE dimension and ESTIMATED_EARNINGS metric, whose rows carry
    // money as microsValue strings. The two rows sum exactly into micros.
    let report = r#"{
        "header": {"currencyCode": "USD"},
        "rows": [
            {
                "dimensionValues": {"DATE": {"value": "2026-09-18"}},
                "metricValues": {"ESTIMATED_EARNINGS": {"microsValue": "60000000000"}}
            },
            {
                "dimensionValues": {"DATE": {"value": "2026-10-17"}},
                "metricValues": {"ESTIMATED_EARNINGS": {"microsValue": "40000000000"}}
            }
        ]
    }"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(report),
    ))]));
    let revenue = admob_client(mock.clone())
        .mediation_revenue("2026-09-18", "2026-10-17")
        .await
        .expect("revenue");
    assert_eq!(revenue, 100_000_000_000, "$100,000.00 in micros");
    let call = &mock.calls()[0];
    assert_eq!(call.method, HttpMethod::Post);
    assert_eq!(
        call.url,
        "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationReport:generate"
    );
    let body = call
        .body
        .as_ref()
        .and_then(HttpBody::as_json)
        .expect("body");
    assert_eq!(
        body["startDate"],
        serde_json::json!({"year": 2026, "month": 9, "day": 18})
    );
    assert_eq!(
        body["endDate"],
        serde_json::json!({"year": 2026, "month": 10, "day": 17})
    );
    assert_eq!(body["dimensions"], serde_json::json!(["DATE"]));
    assert_eq!(body["metrics"], serde_json::json!(["ESTIMATED_EARNINGS"]));
    assert_eq!(body["currencyCode"], serde_json::json!("USD"));

    // A malformed date refuses before any request.
    let mock = Arc::new(MockTransport::scripted(vec![]));
    let client = admob_client(mock.clone());
    assert!(matches!(
        client.mediation_revenue("2026-9-18", "2026-10-17").await,
        Err(PlatformError::Validation(_))
    ));
    assert!(mock.calls().is_empty());

    // A report without rows is a typed refusal, never a silent zero.
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(r#"{"header":{"currencyCode":"USD"}}"#),
    ))]));
    let error = admob_client(mock)
        .mediation_revenue("2026-09-18", "2026-10-17")
        .await
        .expect_err("schema");
    assert!(matches!(error, PlatformError::Schema { .. }), "{error}");

    // An empty rows array is a real zero.
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(r#"{"rows":[]}"#),
    ))]));
    assert_eq!(
        admob_client(mock)
            .mediation_revenue("2026-09-18", "2026-10-17")
            .await
            .expect("zero"),
        0
    );
}

#[tokio::test]
async fn admob_mediation_revenue_caches_one_read_per_window() {
    // The audit and the billing path can share one client and ask for the
    // same 30-day window; the second ask must not be a second platform read.
    let report = r#"{
        "rows": [
            {"metricValues": {"ESTIMATED_EARNINGS": {"microsValue": "2500000"}}}
        ]
    }"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(report),
    ))]));
    let client = admob_client(mock.clone());
    let first = client
        .mediation_revenue("2026-09-18", "2026-10-17")
        .await
        .expect("first");
    let second = client
        .mediation_revenue("2026-09-18", "2026-10-17")
        .await
        .expect("cached");
    assert_eq!(first, 2_500_000);
    assert_eq!(second, first);
    assert_eq!(mock.calls().len(), 1, "one report read serves both asks");
}

#[tokio::test]
async fn admob_mediation_revenue_sums_ten_thousand_rows_exactly() {
    // The parse is one pass over the rows (no per-row rescan) and the total
    // is exact at 10k rows: every row contributes one micro.
    const ROWS: usize = 10_000;
    let rows: Vec<serde_json::Value> = (0..ROWS)
        .map(|_| {
            serde_json::json!({
                "metricValues": {"ESTIMATED_EARNINGS": {"microsValue": "1"}}
            })
        })
        .collect();
    let body = serde_json::json!({"rows": rows}).to_string();
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(&body),
    ))]));
    let total = admob_client(mock.clone())
        .mediation_revenue("2026-09-18", "2026-10-17")
        .await
        .expect("revenue");
    assert_eq!(total, ROWS as i64);
    assert_eq!(mock.calls().len(), 1);
}

#[tokio::test]
async fn admob_mediation_revenue_refuses_a_truncated_page() {
    // A total from a single page is only trustworthy when the API says there
    // is no second page; a token is a typed refusal, never a silent partial
    // revenue figure.
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(
            r#"{"rows":[{"metricValues":{"ESTIMATED_EARNINGS":{"microsValue":"1"}}}],"nextPageToken":"next"}"#,
        ),
    ))]));
    let error = admob_client(mock)
        .mediation_revenue("2026-09-18", "2026-10-17")
        .await
        .expect_err("truncated");
    assert!(matches!(error, PlatformError::Schema { .. }), "{error}");
}

#[tokio::test]
async fn admob_401_fails_fast() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::status(
        401,
    ))]));
    let err = admob_client(mock.clone())
        .list_mediation_groups()
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Auth { status: 401, .. }));
    assert_eq!(mock.calls().len(), 1);
}

const ADMOB_EXPERIMENT_CREATED: &str = r#"{
    "name": "accounts/pub-1234567890123456/mediationAbExperiments/ab-1",
    "mediationGroupId": "g1",
    "displayName": "Floor test",
    "state": "RUNNING",
    "variantLeader": "VARIANT_LEADER_UNSPECIFIED"
}"#;

#[tokio::test]
async fn admob_create_mediation_ab_experiment_recording_is_typed_and_verdict_only() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(ADMOB_EXPERIMENT_CREATED),
    ))]));
    let experiment = admob_client(mock.clone())
        .create_mediation_ab_experiment("g1", "Floor test", None, None)
        .await
        .expect("create");
    assert_eq!(
        experiment.name,
        "accounts/pub-1234567890123456/mediationAbExperiments/ab-1"
    );
    assert_eq!(experiment.mediation_group_id, "g1");
    assert_eq!(experiment.display_name, "Floor test");
    assert_eq!(experiment.state, MediationAbExperimentState::Running);
    assert_eq!(
        experiment.variant_leader,
        Some(MediationAbVariantLeader::Unspecified)
    );
    assert_eq!(
        experiment.revenue_per_variant, ADMOB_REVENUE_PER_VARIANT,
        "the typed surface must not claim per-variant revenue"
    );
    assert!(
        !experiment.revenue_per_variant,
        "AdMob documents only a variantLeader verdict"
    );
    let call = &mock.calls()[0];
    assert_eq!(call.method, HttpMethod::Post);
    assert_eq!(
        call.url,
        "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationGroups/g1/mediationAbExperiments"
    );
    let body = call
        .body
        .as_ref()
        .and_then(HttpBody::as_json)
        .expect("body");
    assert!(
        body.get("mediationGroupId").is_none(),
        "the nested route carries the group in the path"
    );
    assert_eq!(body["displayName"], "Floor test");
}

#[tokio::test]
async fn admob_get_mediation_ab_experiment_reads_the_documented_nested_route() {
    let response = r#"{
        "name": "accounts/pub-1234567890123456/mediationGroups/g1/mediationAbExperiment/ab-1",
        "mediationGroupId": "g1",
        "displayName": "Floor test",
        "state": "RUNNING",
        "variantLeader": "TREATMENT"
    }"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(response),
    ))]));
    let experiment = admob_client(mock.clone())
        .get_mediation_ab_experiment(
            "accounts/pub-1234567890123456/mediationGroups/g1/mediationAbExperiment/ab-1",
        )
        .await
        .expect("get");
    assert_eq!(experiment.state, MediationAbExperimentState::Running);
    assert_eq!(
        experiment.variant_leader,
        Some(MediationAbVariantLeader::Treatment)
    );
    assert!(experiment.state.as_str() == "RUNNING");
    assert!(!experiment.state.is_ended());
    assert!(!experiment.revenue_per_variant);
    let call = &mock.calls()[0];
    assert_eq!(call.method, HttpMethod::Get);
    assert_eq!(
        call.url,
        "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationGroups/g1/mediationAbExperiments/ab-1"
    );
    assert!(call.body.is_none(), "the get carries no body");

    assert!(matches!(
        admob_client(Arc::new(MockTransport::scripted(vec![])))
            .get_mediation_ab_experiment(" ")
            .await,
        Err(PlatformError::Validation(_))
    ));
}

#[tokio::test]
async fn admob_stop_mediation_ab_experiment_recording_pins_path_and_parses_verdict() {
    let response = r#"{
        "name": "accounts/pub-1234567890123456/mediationAbExperiments/ab-1",
        "mediationGroupId": "g1",
        "displayName": "Floor test",
        "state": "STOPPED",
        "variantLeader": "VARIANT"
    }"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(response),
    ))]));
    let experiment = admob_client(mock.clone())
        .stop_mediation_ab_experiment("ab-1")
        .await
        .expect("stop");
    assert_eq!(experiment.state, MediationAbExperimentState::Stopped);
    assert_eq!(
        experiment.variant_leader,
        Some(MediationAbVariantLeader::Variant),
        "the stop response is the only place the verdict appears"
    );
    assert!(!experiment.revenue_per_variant);
    let call = &mock.calls()[0];
    assert_eq!(call.method, HttpMethod::Post);
    assert_eq!(
        call.url,
        "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationAbExperiments/ab-1:stop"
    );
    assert!(
        call.body.is_none(),
        "the :stop custom method carries no body"
    );
}

#[tokio::test]
async fn admob_experiment_error_mapping_is_typed() {
    // A transport-level failure status is classified before the body is read.
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::status(
        400,
    ))]));
    let err = admob_client(mock)
        .create_mediation_ab_experiment("g1", "Floor test", None, None)
        .await
        .expect_err("must fail");
    assert!(
        matches!(err, PlatformError::Http { status: 400, .. }),
        "{err}"
    );

    // Missing required fields and unknown enum values are schema refusals.
    for body in [
        r#"{"mediationGroupId":"g1","displayName":"d","state":"RUNNING"}"#,
        r#"{"name":"n","displayName":"d","state":"RUNNING"}"#,
        r#"{"name":"n","mediationGroupId":"g1","state":"RUNNING"}"#,
        r#"{"name":"n","mediationGroupId":"g1","displayName":"d"}"#,
        r#"{"name":"n","mediationGroupId":"g1","displayName":"d","state":"PAUSED"}"#,
        r#"{"name":"n","mediationGroupId":"g1","displayName":"d","state":"RUNNING","variantLeader":"WINNER"}"#,
        r#"{"name":"n","mediationGroupId":"g1","displayName":"d","state":"RUNNING","variantLeader":7}"#,
    ] {
        let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
            200,
            Some(body),
        ))]));
        let err = admob_client(mock)
            .stop_mediation_ab_experiment("ab-1")
            .await
            .expect_err("must fail");
        assert!(
            matches!(err, PlatformError::Schema { .. }),
            "body {body} must refuse: {err}"
        );
    }

    // Blank identifiers are refused before any request goes out.
    let mock = Arc::new(MockTransport::scripted(vec![]));
    let client = admob_client(mock.clone());
    assert!(matches!(
        client
            .create_mediation_ab_experiment(" ", "d", None, None)
            .await,
        Err(PlatformError::Validation(_))
    ));
    assert!(matches!(
        client
            .create_mediation_ab_experiment("g1", " ", None, None)
            .await,
        Err(PlatformError::Validation(_))
    ));
    assert!(matches!(
        client.stop_mediation_ab_experiment(" ").await,
        Err(PlatformError::Validation(_))
    ));
    assert!(mock.calls().is_empty(), "no request may go out");
}

#[tokio::test]
async fn admob_get_mediation_group_reads_the_single_resource() {
    let body = r#"{
        "name":"accounts/pub-1234567890123456/mediationGroups/g1",
        "mediationGroupId":"g1",
        "displayName":"Main",
        "format":"rewarded",
        "adUnitMappings":[{"adUnitId":"u1","network":"admob","ecpm":"1.25"}]
    }"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(body),
    ))]));
    let group = admob_client(mock.clone())
        .get_mediation_group("g1")
        .await
        .expect("group");
    assert_eq!(group.mediation_group_id, "g1");
    assert_eq!(group.format, apex_core::AdFormat::Rewarded);
    assert_eq!(group.mappings[0].ecpm, 1_250_000);
    assert_eq!(mock.calls()[0].method, HttpMethod::Get);
    assert_eq!(
        mock.calls()[0].url,
        "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationGroups/g1"
    );
    assert!(admob_client(Arc::new(MockTransport::scripted(vec![])))
        .get_mediation_group(" ")
        .await
        .is_err());
}

#[tokio::test]
async fn admob_reads_the_documented_v1beta_mediation_group_lines() {
    // The live v1beta resource carries `mediationGroupLines` (a map keyed by
    // line id) with `cpmMicros` in micros and the format under `targeting`.
    // Recorded legacy fixtures use `adUnitMappings`; both must parse to the
    // same typed mapping with exact micros.
    let body = r#"{"mediationGroups":[{
        "name":"accounts/pub-1234567890123456/mediationGroups/g1",
        "mediationGroupId":"g1",
        "displayName":"Main",
        "targeting":{"platform":"ANDROID","format":"INTERSTITIAL","adUnitIds":["u1","u2"]},
        "mediationGroupLines":{
            "line-1":{
                "id":"1","displayName":"AdMob","adSourceId":"admob","network":"admob",
                "cpmMode":"MANUAL","cpmMicros":"1750000",
                "adUnitMappings":{"u1":"m1","u2":"m2"},
                "state":"ENABLED","experimentVariant":"VARIANT_A"
            },
            "line-2":{
                "id":"2","displayName":"Removed","adSourceId":"meta",
                "cpmMicros":"9990000","adUnitMappings":{"u3":"m3"},"state":"REMOVED"
            }
        }
    }]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(body),
    ))]));
    let groups = admob_client(mock)
        .list_mediation_groups()
        .await
        .expect("canonical groups");
    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert_eq!(group.format, apex_core::AdFormat::Interstitial);
    assert_eq!(group.mappings.len(), 2, "removed line is not served");
    assert_eq!(group.mappings[0].ad_unit_id, "u1");
    assert_eq!(group.mappings[0].network, "admob");
    assert_eq!(group.mappings[0].ecpm, 1_750_000);
    assert_eq!(group.mappings[1].ecpm, 1_750_000);
}

#[tokio::test]
async fn admob_experiment_accepts_the_documented_v1beta_enums() {
    // v1beta documents RUNNING/ENDED/EXPIRED and CONTROL/TREATMENT/
    // INSUFFICIENT_DATA/TOO_EARLY_TO_CALL/NO_VARIANT_LEADER. The legacy
    // spellings stay readable for recordings.
    for (state, leader, expected_state, expected_leader) in [
        (
            "ENDED",
            "TREATMENT",
            MediationAbExperimentState::Ended,
            MediationAbVariantLeader::Treatment,
        ),
        (
            "EXPIRED",
            "NO_VARIANT_LEADER",
            MediationAbExperimentState::Expired,
            MediationAbVariantLeader::NoVariantLeader,
        ),
        (
            "RUNNING",
            "TOO_EARLY_TO_CALL",
            MediationAbExperimentState::Running,
            MediationAbVariantLeader::TooEarlyToCall,
        ),
        (
            "RUNNING",
            "INSUFFICIENT_DATA",
            MediationAbExperimentState::Running,
            MediationAbVariantLeader::InsufficientData,
        ),
    ] {
        let body = serde_json::json!({
            "name":"accounts/pub-1234567890123456/mediationAbExperiments/ab-1",
            "mediationGroupId":"g1",
            "displayName":"Floor test",
            "state":state,
            "variantLeader":leader,
        })
        .to_string();
        let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
            200,
            Some(&body),
        ))]));
        let experiment = admob_client(mock)
            .stop_mediation_ab_experiment("ab-1")
            .await
            .expect("documented enums");
        assert_eq!(experiment.state, expected_state, "{state}");
        assert_eq!(experiment.variant_leader, Some(expected_leader), "{leader}");
    }
}

#[tokio::test]
async fn admob_report_rejects_impossible_calendar_dates_before_any_call() {
    let mock = Arc::new(MockTransport::scripted(vec![]));
    let client = admob_client(mock.clone());
    for (start, end) in [
        ("2026-02-31", "2026-03-01"),
        ("2026-04-31", "2026-05-01"),
        ("2025-02-29", "2025-03-01"),
    ] {
        let error = client
            .mediation_revenue(start, end)
            .await
            .expect_err("date");
        assert!(
            matches!(error, PlatformError::Validation(_)),
            "{start}: {error}"
        );
    }
    assert!(
        mock.calls().is_empty(),
        "no request goes out for an impossible date"
    );

    // A real leap day is accepted.
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(r#"{"rows":[]}"#),
    ))]));
    assert_eq!(
        admob_client(mock)
            .mediation_revenue("2024-02-29", "2024-03-01")
            .await
            .expect("leap day"),
        0
    );
}

#[tokio::test]
async fn admob_stop_replay_uses_the_same_documented_route() {
    let resource = "accounts/pub-1234567890123456/mediationGroups/g1/mediationAbExperiment/ab-7";
    let stopped = format!(
        r#"{{"name":"{resource}","mediationGroupId":"g1","displayName":"Floor test","state":"ENDED","variantLeader":"TREATMENT"}}"#
    );
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(stopped.as_str()))),
        Ok(MockTransport::ok(200, Some(stopped.as_str()))),
    ]));
    let client = admob_client(mock.clone());
    for _ in 0..2 {
        let experiment = client
            .stop_mediation_ab_experiment(resource)
            .await
            .expect("stop replay");
        assert_eq!(experiment.state, MediationAbExperimentState::Ended);
        assert_eq!(
            experiment.variant_leader,
            Some(MediationAbVariantLeader::Treatment)
        );
    }
    let calls = mock.calls();
    assert_eq!(calls.len(), 2);
    for call in calls {
        assert_eq!(call.method, HttpMethod::Post);
        assert_eq!(
            call.url,
            "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationGroups/g1/mediationAbExperiments/ab-7:stop"
        );
        assert!(
            call.body.is_none(),
            "the :stop custom method carries no body"
        );
    }
}

#[tokio::test]
async fn admob_stop_by_bare_id_uses_the_group_remembered_at_create() {
    let created = r#"{
        "name":"accounts/pub-1234567890123456/mediationAbExperiments/ab-7",
        "mediationGroupId":"g1",
        "displayName":"Floor test",
        "state":"RUNNING",
        "variantLeader":null
    }"#;
    let stopped = r#"{
        "name":"accounts/pub-1234567890123456/mediationAbExperiments/ab-7",
        "mediationGroupId":"g1",
        "displayName":"Floor test",
        "state":"ENDED",
        "variantLeader":"TREATMENT"
    }"#;
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(created))),
        Ok(MockTransport::ok(200, Some(stopped))),
    ]));
    let client = admob_client(mock.clone());
    client
        .create_mediation_ab_experiment("g1", "Floor test", None, None)
        .await
        .expect("create");
    let experiment = client
        .stop_mediation_ab_experiment("ab-7")
        .await
        .expect("stop");
    assert_eq!(experiment.state, MediationAbExperimentState::Ended);
    let calls = mock.calls();
    assert_eq!(
        calls[1].url,
        "https://admob.test/v1beta/accounts/pub-1234567890123456/mediationGroups/g1/mediationAbExperiments/ab-7:stop",
        "a known experiment stops on the documented nested route"
    );
}

// ---------------- endpoint metadata ----------------

#[test]
fn endpoint_metas_are_pinned() {
    assert_eq!(MAX_ENDPOINTS.len(), 6);
    // Reads are doc-verified; writes stay verify-live until a sandbox call
    // confirms the shape.
    assert!(MAX_ENDPOINTS
        .iter()
        .filter(|m| m.method == apex_core::endpoint::HttpMethod::Get)
        .all(|m| m.verification == SourceVerification::Documented));
    assert!(MAX_ENDPOINTS
        .iter()
        .filter(|m| m.method != apex_core::endpoint::HttpMethod::Get)
        .all(|m| m.verification == SourceVerification::VerifyLive));
    assert!(!MAX_ENDPOINTS.iter().any(|m| m.writable()));

    assert_eq!(LEVELPLAY_ENDPOINTS.len(), 8);
    assert!(
        LEVELPLAY_ENDPOINTS
            .iter()
            .all(|m| m.verification == SourceVerification::VerifyLive),
        "LevelPlay paths are verify-live: no autopilot writes until sandbox confirmation"
    );
    assert!(!LEVELPLAY_ENDPOINTS
        .iter()
        .find(|m| m.name == "updateEntryPrice")
        .unwrap()
        .writable());
    // The corrected/added surface is present in the capability table: groups
    // and the date/abTest revenue+activeUsers report are GETs, so readiness
    // shows them as implemented rows.
    for (name, method) in [
        ("listGroups", apex_core::endpoint::HttpMethod::Get),
        (
            "reportAbTestActiveUsers",
            apex_core::endpoint::HttpMethod::Get,
        ),
    ] {
        let meta = LEVELPLAY_ENDPOINTS
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("missing endpoint meta '{name}'"));
        assert_eq!(meta.method, method, "{name}");
    }

    assert_eq!(ADMOB_ENDPOINTS.len(), 7);
    assert!(ADMOB_ENDPOINTS
        .iter()
        .filter(|m| m.method == apex_core::endpoint::HttpMethod::Get)
        .all(|m| m.verification == SourceVerification::Documented));
    assert!(ADMOB_ENDPOINTS
        .iter()
        .filter(|m| m.method != apex_core::endpoint::HttpMethod::Get)
        .all(|m| m.verification == SourceVerification::VerifyLive));
    assert!(
        !ADMOB_ENDPOINTS.iter().any(|m| m.writable()),
        "AdMob writes (including the A/B experiments) stay verify-live"
    );
    for name in [
        "createMediationAbExperiment",
        "getMediationAbExperiment",
        "stopMediationAbExperiment",
    ] {
        let meta = ADMOB_ENDPOINTS
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("missing endpoint meta '{name}'"));
        let expected = if name == "getMediationAbExperiment" {
            apex_core::endpoint::HttpMethod::Get
        } else {
            apex_core::endpoint::HttpMethod::Post
        };
        assert_eq!(meta.method, expected, "{name}");
    }
}

// ---------------- old-endpoint guards ----------------

#[test]
fn no_code_path_emits_the_old_endpoints() {
    // The LevelPlay instances/groups endpoints moved off the partner-gate
    // query form and the AdMob mediation endpoints moved off the v1 base; the
    // source itself is the guard (a fixture-only check could miss a code path
    // that is not exercised by the current tests).
    let levelplay = include_str!("levelplay.rs");
    assert!(
        !levelplay.contains("partners/publisher/mediation"),
        "the partner-gate mediation form must not come back"
    );
    // The reporting request carries revenue and active users over the date and
    // abTest breakdowns. The old activeUsers-only request is gone from every
    // code path: this source guard sees unexercised paths as well as fixtures.
    assert!(
        levelplay.contains("breakdowns=date,abTest"),
        "the reporting request must break down by date and abTest"
    );
    assert!(
        levelplay.contains("metrics=revenue,activeUsers"),
        "the reporting request must ask for revenue and active users together"
    );
    assert!(
        !levelplay.contains("breakdowns=abTest&metrics=activeUsers"),
        "the activeUsers-only reporting URL must not come back"
    );
    assert!(
        !levelplay.contains("metrics=activeUsers&"),
        "no code path may request active users without revenue"
    );
    let admob = include_str!("admob.rs");
    assert!(
        !admob.contains("/v1/"),
        "the AdMob v1 mediation-group base must not come back"
    );
}

// ---------------- body shape and diagnostics ----------------

#[test]
fn http_request_debug_redacts_credentials() {
    let request = HttpRequest {
        platform: PlatformId::Admob,
        operation: "op".to_string(),
        method: HttpMethod::Post,
        url: "https://api.example.test/v1/items".to_string(),
        headers: vec![
            (
                "Authorization".to_string(),
                "Bearer sk_live_SUPERSECRET".to_string(),
            ),
            (
                "Idempotency-Key".to_string(),
                "mollie-item:pub-1:secret-exp:2023-11".to_string(),
            ),
            ("Accept".to_string(), "application/json".to_string()),
            ("Content-Type".to_string(), FORM_CONTENT_TYPE.to_string()),
        ],
        body: Some(HttpBody::Form("amount=50&currency=usd".to_string())),
        timeout_ms: 10_000,
    };
    let rendered = format!("{request:?}");
    assert!(rendered.contains("[redacted]"), "{rendered}");
    assert!(!rendered.contains("SUPERSECRET"), "{rendered}");
    assert!(!rendered.contains("sk_live"), "{rendered}");
    assert!(!rendered.contains("secret-exp"), "{rendered}");
    // Non-sensitive material stays debuggable.
    assert!(rendered.contains("application/json"), "{rendered}");
    assert!(rendered.contains("amount=50"), "{rendered}");
    assert!(rendered.contains("api.example.test"), "{rendered}");
    assert_eq!(
        request.body.as_ref().and_then(HttpBody::as_form),
        Some("amount=50&currency=usd")
    );
    assert!(request.body.as_ref().and_then(HttpBody::as_json).is_none());
}

#[test]
fn http_body_from_json_value_is_json() {
    let body: HttpBody = serde_json::json!({"a": 1}).into();
    assert_eq!(body.as_json(), Some(&serde_json::json!({"a": 1})));
    assert!(body.as_form().is_none());
}

// ---------------- verifier-round 2 regressions ----------------

#[tokio::test]
async fn admob_duplicate_ad_unit_ids_inside_a_group_fail_closed() {
    let body = r#"{"mediationGroups":[{"mediationGroupId":"g1","name":"n","format":"interstitial",
        "adUnitMappings":[{"adUnitId":"u1","network":"admob","ecpm":2},
                          {"adUnitId":"u1","network":"facebook","ecpm":1}]}]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(body),
    ))]));
    let err = admob_client(mock)
        .list_mediation_groups()
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }), "got {err}");
}

#[tokio::test]
async fn max_report_revenue_overflow_is_a_schema_error_not_zero() {
    // Revenue 9007199254740992 micros-worth overflows i64; it must fail closed
    // instead of reading as a smaller number.
    let body = r#"{"code":200,"count":1,"results":[
        {"day":"2026-10-01","estimated_revenue":"9007199254740992.00","impressions":"1"}
    ]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(body),
    ))]));
    let err = report_client(mock)
        .totals("2026-10-01", "2026-10-01")
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }), "got {err}");
}

#[tokio::test]
async fn lp_negative_positions_are_refused() {
    let body =
        r#"{"waterfall":[{"entry_id":"e1","network":"admob","position":-5,"price_ecpm":1}]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(body),
    ))]));
    let err = lp_client(mock).list_waterfall("u1").await.unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }), "got {err}");
}

#[tokio::test]
async fn http_204_with_a_body_still_reads_as_null() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        204,
        Some("{\"a\":1}"),
    ))]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let value = caller(mock, sleeper, 0)
        .call("op", HttpMethod::Delete, "/x", None, Some("k"))
        .await
        .unwrap();
    assert!(
        value.is_null(),
        "204 must never surface a parsed body: {value}"
    );
}

#[tokio::test]
async fn admob_missing_format_defaults_to_banner_and_unknown_fails_closed() {
    let no_format = r#"{"mediationGroups":[{"mediationGroupId":"g1","name":"n",
        "adUnitMappings":[{"adUnitId":"u1","network":"admob","ecpm":2}]}]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(no_format),
    ))]));
    let groups = admob_client(mock).list_mediation_groups().await.unwrap();
    assert_eq!(
        groups[0].format,
        AdFormat::Banner,
        "missing format pins the TS-parity default"
    );

    let unknown = r#"{"mediationGroups":[{"mediationGroupId":"g1","name":"n","format":"video360",
        "adUnitMappings":[{"adUnitId":"u1","network":"admob","ecpm":2}]}]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(unknown),
    ))]));
    let err = admob_client(mock)
        .list_mediation_groups()
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }), "got {err}");
}

#[tokio::test]
async fn max_ad_unit_revenue_sums_only_the_requested_unit() {
    let body = r#"{"code":200,"count":3,"results":[
        {"day":"2026-10-01","max_ad_unit_id":"u1","estimated_revenue":"1.5","impressions":"10"},
        {"day":"2026-10-02","max_ad_unit_id":"u1","estimated_revenue":"2.5","impressions":"10"},
        {"day":"2026-10-02","max_ad_unit_id":"u2","estimated_revenue":"9.0","impressions":"10"}
    ]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![ok(200, Some(body))]));
    let total = report_client(mock.clone())
        .ad_unit_revenue("u1", "2026-10-01", "2026-10-02")
        .await
        .expect("revenue");
    assert_eq!(total, 4_000_000, "1.5 + 2.5 for u1 only");
    assert!(mock.calls()[0].url.contains("max_ad_unit_id"));
}

#[tokio::test]
async fn max_country_revenue_30d_requests_the_trailing_window() {
    let body = r#"{"code":200,"count":1,"results":[
        {"day":"2026-10-01","country":"EE","max_ad_unit_id":"u1","estimated_revenue":"1.25","impressions":"100"}
    ]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![ok(200, Some(body))]));
    // 1_700_000_000_000 ms is 2023-11-14; the window is the 30 days ending the
    // day before, exclusive of today.
    let rows = report_client(mock.clone())
        .country_revenue_30d(1_700_000_000_000)
        .await
        .expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].revenue, 1_250_000);
    let url = &mock.calls()[0].url;
    assert!(url.contains("start="), "{url}");
    assert!(url.contains("end="), "{url}");
    assert!(url.contains("country"), "{url}");
}

#[tokio::test]
async fn lp_groups_and_token_shapes_are_read() {
    // The auth endpoint may return a bare JSON string or an object.
    let mock = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#"{"token":"tok-2"}"#))),
        Ok(MockTransport::ok(
            200,
            Some(
                r#"{"groups":[{"groupId":"g1","name":"Main","floorPrice":1.5,"countriesRate":0.8}]}"#,
            ),
        )),
    ]));
    let groups = lp_client(mock.clone()).list_groups().await.expect("groups");
    assert_eq!(groups["groups"][0]["groupId"], "g1");
    assert_eq!(mock.calls()[1].method, HttpMethod::Get);
    assert_eq!(
        mock.calls()[1].url,
        "https://lp.test/levelPlay/groups/v4/acct"
    );
    assert_eq!(
        mock.calls()[1].header("Authorization"),
        Some("Bearer tok-2")
    );
}
