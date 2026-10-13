//! Adversarial test suite for the connector layer: a scripted mock transport plus a
//! recording sleeper make every retry/error branch deterministic and socket-free.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use apex_core::{
    AdFormat, ExperimentMetric, ExperimentSpec, PlatformError, PlatformId, SourceVerification,
};
use async_trait::async_trait;

use crate::admob::{normalize_account_id, AdmobClient, ADMOB_ENDPOINTS};
use crate::levelplay::{LevelPlayClient, LEVELPLAY_ENDPOINTS};
use crate::max::{MaxClient, MAX_ENDPOINTS};
use crate::transport::{
    HttpMethod, HttpRequest, HttpResponse, PlatformCaller, RecordingSleeper, RetryTransport,
    Transport, TransportError,
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

fn max_client(mock: Arc<MockTransport>, sleeper: Arc<RecordingSleeper>) -> MaxClient {
    let retry = Arc::new(RetryTransport::new(mock, 0, sleeper));
    MaxClient::new("key-1", "acct-1", "https://max.test", retry).expect("client")
}

#[tokio::test]
async fn max_credentials_required() {
    let retry = Arc::new(RetryTransport::new(
        Arc::new(MockTransport::scripted(vec![])),
        0,
        Arc::new(RecordingSleeper::default()),
    ));
    assert!(MaxClient::new("  ", "a", "https://x", retry.clone()).is_err());
    assert!(MaxClient::new("k", "", "https://x", retry).is_err());
}

#[tokio::test]
async fn max_list_ad_units_parses_and_validates() {
    let body = r#"{"ad_units":[{"ad_unit_id":"u1","name":"Banner","format":"banner","enabled":true},{"id":"u2","name":"Rewarded","format":"rewarded"}]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(body),
    ))]));
    let units = max_client(mock.clone(), Arc::new(RecordingSleeper::default()))
        .list_ad_units()
        .await
        .unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].format, AdFormat::Banner);
    assert!(units[1].enabled, "enabled defaults true");
    assert!(mock.calls()[0].header("X-API-Key") == Some("key-1"));

    let drift = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(r#"{"ad_units":[{"name":"x"}]}"#),
    ))]));
    let err = max_client(drift, Arc::new(RecordingSleeper::default()))
        .list_ad_units()
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }));

    let unknown_format = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(r#"{"ad_units":[{"ad_unit_id":"u","name":"n","format":"video360"}]}"#),
    ))]));
    let err = max_client(unknown_format, Arc::new(RecordingSleeper::default()))
        .list_ad_units()
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }));
}

#[tokio::test]
async fn max_set_country_floor_pins_path_and_body() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200, None,
    ))]));
    let floor = max_client(mock.clone(), Arc::new(RecordingSleeper::default()))
        .set_country_floor("unit/a b", " ee ", 2_500_000)
        .await
        .unwrap();
    assert_eq!(floor.country, "EE");
    let call = &mock.calls()[0];
    assert_eq!(call.method, HttpMethod::Put);
    assert_eq!(
        call.url,
        "https://max.test/v1/max/ad_units/unit%2Fa%20b/floors/EE"
    );
    assert_eq!(call.body.as_ref().unwrap()["floor_ecpm"], 2_500_000);
}

#[tokio::test]
async fn max_set_country_floor_validation() {
    let mock = Arc::new(MockTransport::scripted(vec![]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let client = max_client(mock.clone(), sleeper);
    assert!(matches!(
        client.set_country_floor("u", "EST", 1).await,
        Err(PlatformError::Validation(_))
    ));
    assert!(matches!(
        client.set_country_floor("u", "EE", -1).await,
        Err(PlatformError::Validation(_))
    ));
    assert!(matches!(
        client.set_country_floor("u", "EE", 500_000_001).await,
        Err(PlatformError::Validation(_))
    ));
    assert!(
        mock.calls().is_empty(),
        "validation must not perform network calls"
    );
}

#[tokio::test]
async fn max_create_experiment_full_contract() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(r#"{"experiment_id":"exp-9"}"#),
    ))]));
    let client = max_client(mock.clone(), Arc::new(RecordingSleeper::default()));
    let spec = ExperimentSpec {
        name: "floors-ee".into(),
        ad_unit_id: "u1".into(),
        metric: ExperimentMetric::Ecpm,
        variants: 2,
        change_description: "raise EE floor to 2.0".into(),
    };
    let id = client.create_experiment(&spec).await.unwrap();
    assert_eq!(id, "exp-9");
    let call = &mock.calls()[0];
    assert_eq!(
        call.header("Idempotency-Key"),
        Some("max-experiment:floors-ee:u1")
    );
    assert_eq!(call.body.as_ref().unwrap()["variant_count"], 2);
    assert_eq!(call.body.as_ref().unwrap()["metric"], "ecpm");

    let bad = ExperimentSpec {
        variants: 5,
        ..spec.clone()
    };
    assert!(matches!(
        client.create_experiment(&bad).await,
        Err(PlatformError::Schema { .. })
    ));
    let bad2 = ExperimentSpec {
        change_description: "   ".into(),
        ..spec
    };
    assert!(matches!(
        client.create_experiment(&bad2).await,
        Err(PlatformError::Schema { .. })
    ));

    let missing = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some("{}"),
    ))]));
    let err = max_client(missing, Arc::new(RecordingSleeper::default()))
        .create_experiment(&bad2_spec())
        .await
        .unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }));
}

fn bad2_spec() -> ExperimentSpec {
    ExperimentSpec {
        name: "n".into(),
        ad_unit_id: "u".into(),
        metric: ExperimentMetric::Revenue,
        variants: 2,
        change_description: "change".into(),
    }
}

#[tokio::test]
async fn max_get_experiment_math_and_state() {
    let body = r#"{
        "state": "completed",
        "lift_percent": 7.5,
        "confidence": 0.98,
        "winner_variant_id": "v2",
        "variants": [
            {"variant_id":"v1","impressions":1000,"revenue":5},
            {"variant_id":"v2","impressions":0,"revenue":0}
        ]
    }"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(body),
    ))]));
    let result = max_client(mock, Arc::new(RecordingSleeper::default()))
        .get_experiment("exp-1")
        .await
        .unwrap();
    assert_eq!(result.state, apex_core::ExperimentState::Completed);
    assert_eq!(result.lift_percent_bp, Some(750));
    assert_eq!(result.confidence, Some(9_800));
    assert_eq!(result.winner_variant_id.as_deref(), Some("v2"));
    // revenue "5" is 5 currency units => 5_000_000 micros; 5_000_000 * 1000 / 1000 impressions
    assert_eq!(result.results[0].ecpm, 5_000_000);
    assert_eq!(
        result.results[1].ecpm, 0,
        "zero impressions -> zero ecpm, never divide"
    );

    for bad_body in [
        r#"{"variants":[]}"#,
        r#"{"state":"running"}"#,
        r#"{"state":"running","variants":[{"variant_id":1}]}"#,
    ] {
        let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
            200,
            Some(bad_body),
        ))]));
        let err = max_client(mock, Arc::new(RecordingSleeper::default()))
            .get_experiment("e")
            .await
            .unwrap_err();
        assert!(
            matches!(err, PlatformError::Schema { .. }),
            "body {bad_body} must fail closed"
        );
    }

    let weird = r#"{"state":"paused_by_platform","variants":[]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(weird),
    ))]));
    let result = max_client(mock, Arc::new(RecordingSleeper::default()))
        .get_experiment("e")
        .await
        .unwrap();
    assert_eq!(
        result.state,
        apex_core::ExperimentState::Running,
        "unknown states never count as finished"
    );
}

// ---------------- LevelPlay client ----------------

fn lp_client(mock: Arc<MockTransport>) -> LevelPlayClient {
    let retry = Arc::new(RetryTransport::new(
        mock,
        0,
        Arc::new(RecordingSleeper::default()),
    ));
    LevelPlayClient::new("tok", "acct", "https://lp.test", retry).expect("client")
}

#[tokio::test]
async fn lp_list_waterfall_and_fallback() {
    let body = r#"{"waterfall":[
        {"entry_id":"e1","network":"admob","instance_name":"ADM_1","position":1,"price_ecpm":3.2},
        {"id":"e2","network":"facebook","position":2,"price_ecpm":"1.1"}
    ]}"#;
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(body),
    ))]));
    let entries = lp_client(mock.clone()).list_waterfall("u/1").await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].instance_name, "ADM_1");
    assert_eq!(entries[1].instance_name, "facebook", "fallback to network");
    assert_eq!(entries[0].price_ecpm, 3_200_000);
    assert_eq!(
        entries[1].price_ecpm, 1_100_000,
        "string decimals parse exactly"
    );
    assert!(mock.calls()[0]
        .header("Authorization")
        .unwrap()
        .starts_with("Bearer "));

    let drift = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some(r#"{"waterfall":[{"entry_id":"e","network":"x"}]}"#),
    ))]));
    let err = lp_client(drift).list_waterfall("u").await.unwrap_err();
    assert!(matches!(err, PlatformError::Schema { .. }));
}

#[tokio::test]
async fn lp_price_update_pins_body_and_key() {
    let mock = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200, None,
    ))]));
    let entry = lp_client(mock.clone())
        .update_entry_price("u1", "e1", 2_345_000)
        .await
        .unwrap();
    assert_eq!(entry.price_ecpm, 2_345_000);
    let call = &mock.calls()[0];
    assert_eq!(call.method, HttpMethod::Patch);
    assert_eq!(call.url, "https://lp.test/v1/adunits/u1/waterfall/e1");
    assert_eq!(call.body.as_ref().unwrap()["price_ecpm"], 2_345_000);
    assert_eq!(
        call.header("Idempotency-Key"),
        Some("levelplay-price:u1:e1")
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
        Ok(MockTransport::ok(200, Some(r#"{"entry_id":"new-1"}"#))),
        Ok(MockTransport::ok(204, None)),
    ]));
    let client = lp_client(mock.clone());
    let id = client
        .create_entry("u1", "mintegral", "MTG_1", 750_000)
        .await
        .unwrap();
    assert_eq!(id, "new-1");
    assert_eq!(
        mock.calls()[0].header("Idempotency-Key"),
        Some("levelplay-create:u1:mintegral:MTG_1")
    );
    client.remove_entry("u1", "e9").await.unwrap();
    assert_eq!(mock.calls()[1].method, HttpMethod::Delete);
    assert_eq!(
        mock.calls()[1].header("Idempotency-Key"),
        Some("levelplay-remove:u1:e9")
    );

    assert!(matches!(
        client.create_entry("u", " ", "i", 1_000_000).await,
        Err(PlatformError::Schema { .. })
    ));
    let missing = Arc::new(MockTransport::scripted(vec![Ok(MockTransport::ok(
        200,
        Some("{}"),
    ))]));
    assert!(matches!(
        lp_client(missing)
            .create_entry("u", "n", "i", 1_000_000)
            .await,
        Err(PlatformError::Schema { .. })
    ));
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
    let mappings = patch.body.as_ref().unwrap()["adUnitMappings"]
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
    // Range validation runs BEFORE the group read; the two lookup errors each cost
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
    assert_eq!(MAX_ENDPOINTS.len(), 5);
    assert!(MAX_ENDPOINTS
        .iter()
        .all(|m| m.verification == SourceVerification::Documented));
    assert!(MAX_ENDPOINTS
        .iter()
        .find(|m| m.name == "setCountryFloor")
        .unwrap()
        .writable());

    assert_eq!(LEVELPLAY_ENDPOINTS.len(), 4);
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
