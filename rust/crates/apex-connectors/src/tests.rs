//! Adversarial test suite for the connector layer: a scripted mock transport plus a
//! recording sleeper make every retry/error branch deterministic and socket-free.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use apex_core::{
    AdFormat, AdUnit, ExperimentMetric, ExperimentSpec, PlatformError, PlatformId,
    SourceVerification,
};
use async_trait::async_trait;

use crate::admob::{normalize_account_id, AdmobClient, ADMOB_ENDPOINTS};
use crate::levelplay::{LevelPlayClient, LEVELPLAY_ENDPOINTS};
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
    let retry = Arc::new(RetryTransport::new(
        mock,
        0,
        Arc::new(RecordingSleeper::default()),
    ));
    LevelPlayClient::new("tok", "refresh", "acct", "https://lp.test", retry).expect("client")
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
    // The first call mints the bearer; the second carries it.
    assert_eq!(mock.calls()[0].method, HttpMethod::Post);
    assert!(mock.calls()[0].url.ends_with("/partners/publisher/auth"));
    assert!(mock.calls()[1]
        .header("Authorization")
        .unwrap()
        .starts_with("Bearer "));
    assert!(mock.calls()[1].url.contains("/mediation/instances/v4"));

    let drift = Arc::new(MockTransport::scripted(vec![
        Ok(MockTransport::ok(200, Some(r#""bearer""#))),
        Ok(MockTransport::ok(
            200,
            Some(r#"{"instances":[{"instanceId":"e","network":"x"}]}"#),
        )),
    ]));
    let err = lp_client(drift).list_waterfall("u").await.unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }));
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
    assert!(call.url.contains("/mediation/instances/v4"));
    assert_eq!(
        call.body.as_ref().and_then(HttpBody::as_json).unwrap()["instanceId"],
        "e1"
    );
    assert_eq!(
        call.body.as_ref().and_then(HttpBody::as_json).unwrap()["price"],
        2_345_000
    );
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
    assert_eq!(
        mock.calls()[1].header("Idempotency-Key"),
        Some("levelplay-create:u1:mintegral:MTG_1:0.75")
    );
    client.remove_entry("u1", "e9").await.unwrap();
    assert_eq!(mock.calls()[2].method, HttpMethod::Delete);
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
        "https://admob.test/v1/accounts/pub-1234567890123456/mediationGroups/g1"
    );
    let mappings = patch.body.as_ref().and_then(HttpBody::as_json).unwrap()["adUnitMappings"]
        .as_array()
        .unwrap();
    assert_eq!(mappings.len(), 2, "FULL list written back");
    assert_eq!(
        mappings[0]["ecpm"], 2_000_000,
        "untouched mapping preserved"
    );
    assert_eq!(mappings[1]["ecpm"], 1_750_000);
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

    assert_eq!(LEVELPLAY_ENDPOINTS.len(), 5);
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

    assert_eq!(ADMOB_ENDPOINTS.len(), 3);
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
                "stripe-item:pub-1:secret-exp:2023-11".to_string(),
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
    assert!(mock.calls()[1].url.contains("/mediation/groups/v4"));
    assert_eq!(
        mock.calls()[1].header("Authorization"),
        Some("Bearer tok-2")
    );
}
