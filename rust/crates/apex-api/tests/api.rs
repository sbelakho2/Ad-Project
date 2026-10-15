//! Adversarial integration tests for the JSON API: routing, status codes,
//! content types, path-traversal refusal and the Adyen webhook matrix.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;

use apex_api::{router, MAX_BODY_BYTES};

use axum::body::Body;
use axum::http::header::CONTENT_LENGTH;
use axum::http::Request;
use common::*;
use serde_json::json;

const JSON: &str = "application/json";

#[tokio::test]
async fn healthz_returns_ok_json() {
    let temp = TempDir::new("healthz");
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, get("/healthz")).await;
    assert_eq!(response.status, 200);
    assert!(
        response.content_type().starts_with(JSON),
        "{}",
        response.content_type()
    );
    assert_eq!(response.json(), json!({"status": "ok"}));
}

#[tokio::test]
async fn healthz_rejects_post_with_405() {
    let temp = TempDir::new("healthz-405");
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, post("/healthz")).await;
    assert_eq!(response.status, 405);
}

#[tokio::test]
async fn audit_returns_report_json() {
    let temp = TempDir::new("audit");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, get("/v1/audit/admob")).await;
    assert_eq!(response.status, 200);
    assert!(response.content_type().starts_with(JSON));
    let report = response.json();
    let digest = report["snapshotDigest"].as_str().expect("digest");
    assert_eq!(digest.len(), 64);
    assert!(digest.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert_eq!(report["generatedBy"], "apex-audit v0.1.0");
    let findings = report["findings"].as_array().expect("findings");
    assert!(!findings.is_empty());
    assert!(findings
        .iter()
        .all(|finding| finding["estimatedMonthlyRecovery"].is_i64()));
    assert!(report["totalEstimatedMonthlyRecovery"].is_i64());
}

#[tokio::test]
async fn audit_unknown_platform_is_400() {
    let temp = TempDir::new("audit-400");
    let app = router(file_state(temp.path(), pub1_keys()));
    for platform in ["tiktok", "AdMob", "applovin", "max", "%3Cscript%3E"] {
        let response = send(&app, get(&format!("/v1/audit/{platform}"))).await;
        assert_eq!(response.status, 400, "platform {platform:?}");
        assert_eq!(response.json()["error"], "unknown_platform");
    }
    // An empty segment is not the `{platform}` route at all: the JSON 404
    // fallback answers, never a platform lookup.
    let response = send(&app, get("/v1/audit/")).await;
    assert_eq!(response.status, 404);
    assert_eq!(response.json(), json!({"error": "not_found"}));
}

#[tokio::test]
async fn audit_missing_snapshot_is_404() {
    let temp = TempDir::new("audit-404");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, get("/v1/audit/levelplay")).await;
    assert_eq!(response.status, 404);
    assert_eq!(response.json(), json!({"error": "not_found"}));
}

#[tokio::test]
async fn audit_snapshot_declaring_another_platform_is_internal_error() {
    let temp = TempDir::new("audit-mismatch");
    let mut snapshot = minimal_snapshot(&["u1"]);
    snapshot["platform"] = json!("levelplay");
    write_snapshot(temp.path(), "admob", &snapshot);
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, get("/v1/audit/admob")).await;
    assert_eq!(response.status, 500);
    assert_eq!(response.json()["error"], "internal");
}

#[tokio::test]
async fn ledger_returns_verified_records_in_read_only_mode() {
    let temp = TempDir::new("ledger");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(response.status, 200);
    assert!(response.content_type().starts_with(JSON));
    let body = response.json();
    assert_eq!(body["publisher"], PUBLISHER);
    assert_eq!(body["mode"], "read_only");
    let records = body["records"].as_array().expect("records");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["type"], "changeApplied");
    assert_eq!(records[0]["seq"], 0);
    assert_eq!(records[1]["seq"], 1);
    assert!(records[0]["hash"].as_str().expect("hash").len() == 64);
}

#[tokio::test]
async fn ledger_unverified_is_422_with_the_offending_line() {
    let temp = TempDir::new("ledger-422");
    write_snapshot(temp.path(), "admob", &minimal_snapshot(&["u1"]));
    let path = write_ledger(
        temp.path(),
        PUBLISHER,
        &seed_keypair(7),
        vec![change_record(1), change_record(2), change_record(3)],
    );
    tamper_line(&path, 3);
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(response.status, 422);
    assert_eq!(
        response.json(),
        json!({"error": "ledger_unverified", "line": 3})
    );
}

#[tokio::test]
async fn ledger_signed_with_a_foreign_key_is_422_on_line_one() {
    let temp = TempDir::new("ledger-wrong-key");
    write_snapshot(temp.path(), "admob", &minimal_snapshot(&["u1"]));
    // The file is signed with seed 9, but the allow-list pins seed 7.
    write_ledger(
        temp.path(),
        PUBLISHER,
        &seed_keypair(9),
        vec![change_record(1)],
    );
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(response.status, 422);
    assert_eq!(
        response.json(),
        json!({"error": "ledger_unverified", "line": 1})
    );
}

#[tokio::test]
async fn ledger_missing_file_is_404() {
    let temp = TempDir::new("ledger-404");
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(response.status, 404);
    assert_eq!(response.json()["error"], "not_found");
}

#[tokio::test]
async fn ledger_unknown_valid_publisher_is_closed_without_a_token() {
    // A publisher with no configured token is not readable at all: the route
    // fails closed with 503 rather than revealing whether a ledger exists.
    let temp = TempDir::new("ledger-unknown");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, get("/v1/ledger/not-onboarded")).await;
    assert_eq!(response.status, 503);
    assert_eq!(response.json()["error"], "ledger_auth_not_configured");
}

#[tokio::test]
async fn ledger_without_a_bearer_token_is_401() {
    let temp = TempDir::new("ledger-401");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, get(&format!("/v1/ledger/{PUBLISHER}"))).await;
    assert_eq!(response.status, 401);
    assert_eq!(response.json()["error"], "unauthorized");
}

#[tokio::test]
async fn ledger_with_a_wrong_token_is_401() {
    let temp = TempDir::new("ledger-wrong-token");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), "not-the-token"),
    )
    .await;
    assert_eq!(response.status, 401);
    assert_eq!(response.json()["error"], "unauthorized");
}

#[tokio::test]
async fn publisher_path_traversal_is_rejected() {
    let temp = TempDir::new("ledger-traversal");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    // `%2E%2E` decodes to `..`; `%2F` keeps the slash inside one segment.
    let hostile = [
        "%2E%2E",
        "%2E%2E%2F%2E%2E",
        "..%2Fetc%2Fpasswd",
        "a%2F..%2Fb",
        "%2E%2E%5C..",
        ".%2E",
        "pub1%00",
        "pub1%20",
        "p%C3%BCb",
        "pub%2B1",
        "%E4%B8%96%E7%95%8C",
    ];
    for publisher in hostile {
        let response = send(&app, get(&format!("/v1/ledger/{publisher}"))).await;
        assert_eq!(response.status, 400, "publisher {publisher:?}");
        assert_eq!(
            response.json()["error"],
            "invalid_publisher",
            "{publisher:?}"
        );
    }
}

#[tokio::test]
async fn publisher_length_boundary_is_64_bytes() {
    let temp = TempDir::new("ledger-length");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let exactly_64 = "a".repeat(64);
    let response = send(&app, get(&format!("/v1/ledger/{exactly_64}"))).await;
    // 64 bytes is a well-formed name: the name check passes and the publisher
    // is then refused for having no configured token (503).
    assert_eq!(
        response.status, 503,
        "64-char name must pass the name check"
    );
    let overlong = "a".repeat(65);
    let response = send(&app, get(&format!("/v1/ledger/{overlong}"))).await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"], "invalid_publisher");
}

#[tokio::test]
async fn cycle_requires_a_platform_parameter() {
    let temp = TempDir::new("cycle-platform");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, post(&format!("/v1/cycle/{PUBLISHER}"))).await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"], "platform_required");
}

#[tokio::test]
async fn cycle_summarises_audit_and_ledger_without_mutating() {
    let temp = TempDir::new("cycle-summary");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, post(&format!("/v1/cycle/{PUBLISHER}?platform=admob"))).await;
    assert_eq!(response.status, 200);
    let expected = report_for(&minimal_snapshot(&["u1"]));
    let critical = expected
        .findings
        .iter()
        .filter(|finding| finding.severity.as_str() == "critical")
        .count();
    let body = response.json();
    assert_eq!(body["mode"], "read_only");
    assert_eq!(body["publisher"], PUBLISHER);
    assert_eq!(body["records"], 2);
    assert_eq!(body["critical_findings"], critical);
    assert!(
        body["note"].as_str().expect("note").contains("read-only"),
        "{body}"
    );
    // Nothing mutated: the ledger file still verifies and has two records.
    let ledger = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(
        ledger.json()["records"].as_array().expect("records").len(),
        2
    );
}

#[tokio::test]
async fn cycle_unknown_platform_is_400() {
    let temp = TempDir::new("cycle-unknown");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        post(&format!("/v1/cycle/{PUBLISHER}?platform=tiktok")),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"], "unknown_platform");
}

#[tokio::test]
async fn cycle_rejects_invalid_publishers() {
    let temp = TempDir::new("cycle-badpub");
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, post("/v1/cycle/%2E%2E?platform=admob")).await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"], "invalid_publisher");
}

#[tokio::test]
async fn cycle_rejects_policy_outside_the_business_band() {
    let temp = TempDir::new("cycle-policy");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    for body in [
        r#"{"policy":{"minLiftRateBp":1000}}"#,
        r#"{"policy":{"minLiftRateBp":2500}}"#,
        r#"{"policy":{"maxFeeMicros":-1}}"#,
    ] {
        let response = send(
            &app,
            post_json(
                &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
                body.as_bytes(),
            ),
        )
        .await;
        assert_eq!(response.status, 400, "body {body}");
        assert_eq!(response.json()["error"], "invalid_policy", "body {body}");
    }
}

#[tokio::test]
async fn cycle_accepts_the_band_edges() {
    let temp = TempDir::new("cycle-band");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    for body in [
        r#"{"policy":{"minLiftRateBp":1500}}"#,
        r#"{"policy":{"minLiftRateBp":2000,"maxFeeMicros":0}}"#,
    ] {
        let response = send(
            &app,
            post_json(
                &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
                body.as_bytes(),
            ),
        )
        .await;
        assert_eq!(response.status, 200, "body {body}");
        assert_eq!(response.json()["mode"], "read_only");
    }
}

#[tokio::test]
async fn cycle_rejects_malformed_and_unknown_field_bodies() {
    let temp = TempDir::new("cycle-body");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    for body in [
        b"{".as_slice(),
        b"{\"policies\":{}}",
        b"[]",
        b"\"policy\"",
        b"null",
        b"12",
        b"{\"policy\":[]}",
        b"{\"policy\":\"admob\"}",
        b"{\"policy\":{\"min_lift\":1}}",
        b"{\"policy\":{\"minLiftRateBp\":\"high\"}}",
    ] {
        let response = send(
            &app,
            post_json(&format!("/v1/cycle/{PUBLISHER}?platform=admob"), body),
        )
        .await;
        assert_eq!(
            response.status,
            400,
            "body {:?}",
            String::from_utf8_lossy(body)
        );
        assert_eq!(response.json()["error"], "invalid_cycle_request");
    }
    // An empty body is the default request, not an error; `{}` and an explicit
    // null policy are the same request spelled out.
    let response = send(&app, post(&format!("/v1/cycle/{PUBLISHER}?platform=admob"))).await;
    assert_eq!(response.status, 200);
    for body in [b"{}".as_slice(), b"{\"policy\":null}"] {
        let response = send(
            &app,
            post_json(&format!("/v1/cycle/{PUBLISHER}?platform=admob"), body),
        )
        .await;
        assert_eq!(
            response.status,
            200,
            "body {:?}",
            String::from_utf8_lossy(body)
        );
    }
}

#[tokio::test]
async fn cycle_get_is_405() {
    let temp = TempDir::new("cycle-405");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, get(&format!("/v1/cycle/{PUBLISHER}?platform=admob"))).await;
    assert_eq!(response.status, 405);
}

#[tokio::test]
async fn webhook_without_configuration_is_503() {
    let temp = TempDir::new("webhook-503");
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, post_json("/v1/adyen/webhook", &webhook_body())).await;
    assert_eq!(response.status, 503);
    assert_eq!(
        response.json(),
        json!({"error": "adyen_webhook_not_configured"})
    );
}

#[tokio::test]
async fn webhook_valid_signature_is_200() {
    let temp = TempDir::new("webhook-200");
    let store = Arc::new(
        apex_billing::FileCollectionStore::open(&temp.path().join("collection.jsonl"))
            .expect("store"),
    );
    let state = file_state(temp.path(), pub1_keys())
        .with_adyen_hmac_key(WEBHOOK_HMAC_KEY.to_vec())
        .with_collection(store);
    let app = router(state);
    let response = send(&app, post_json("/v1/adyen/webhook", &webhook_body())).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(
        response.json(),
        json!({"received": true, "items": 1, "new": 1})
    );
    // A replay of the same (eventCode, pspReference) is accepted but recorded
    // once: no new event.
    let replay = send(&app, post_json("/v1/adyen/webhook", &webhook_body())).await;
    assert_eq!(replay.status, 200);
    assert_eq!(replay.json()["new"], 0, "replay is not re-recorded");
}

#[tokio::test]
async fn webhook_without_a_collection_store_is_503() {
    let temp = TempDir::new("webhook-no-coll");
    let state = file_state(temp.path(), pub1_keys()).with_adyen_hmac_key(WEBHOOK_HMAC_KEY.to_vec());
    let app = router(state);
    let response = send(&app, post_json("/v1/adyen/webhook", &webhook_body())).await;
    assert_eq!(response.status, 503);
    assert_eq!(response.json()["error"], "collection_store_not_configured");
}

#[tokio::test]
async fn webhook_key_can_come_from_the_adyen_client() {
    let temp = TempDir::new("webhook-client");
    let store = Arc::new(
        apex_billing::FileCollectionStore::open(&temp.path().join("collection.jsonl"))
            .expect("store"),
    );
    let state = file_state(temp.path(), pub1_keys())
        .with_adyen(Arc::new(
            apex_billing::AdyenClient::new(
                "AQEyhmfx_test_key",
                "ApexMerchant",
                WEBHOOK_HMAC_KEY.to_vec(),
                "https://checkout-test.adyen.com",
            )
            .expect("client"),
        ))
        .with_collection(store);
    let app = router(state);
    let response = send(&app, post_json("/v1/adyen/webhook", &webhook_body())).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.json()["received"], true);
}

#[tokio::test]
async fn webhook_missing_signature_is_400() {
    let temp = TempDir::new("webhook-no-sig");
    let state = file_state(temp.path(), pub1_keys()).with_adyen_hmac_key(WEBHOOK_HMAC_KEY.to_vec());
    let app = router(state);
    let body = webhook_body_with("", 1_000);
    let response = send(&app, post_json("/v1/adyen/webhook", &body)).await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json(), json!({"error": "malformed_event"}));
}

#[tokio::test]
async fn webhook_tampered_item_is_400() {
    let temp = TempDir::new("webhook-tampered");
    let state = file_state(temp.path(), pub1_keys()).with_adyen_hmac_key(WEBHOOK_HMAC_KEY.to_vec());
    let app = router(state);
    // A signed field changed (the amount) but the signature left as-is.
    let body = webhook_body_with(WEBHOOK_SIGNATURE, 2_000);
    let response = send(&app, post_json("/v1/adyen/webhook", &body)).await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json(), json!({"error": "invalid_signature"}));
}

#[tokio::test]
async fn webhook_wrong_key_is_400() {
    let temp = TempDir::new("webhook-wrong-key");
    let state =
        file_state(temp.path(), pub1_keys()).with_adyen_hmac_key(b"some_other_key".to_vec());
    let app = router(state);
    let response = send(&app, post_json("/v1/adyen/webhook", &webhook_body())).await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json(), json!({"error": "invalid_signature"}));
}

#[tokio::test]
async fn webhook_malformed_events_are_refused() {
    let temp = TempDir::new("webhook-event");
    let state = file_state(temp.path(), pub1_keys()).with_adyen_hmac_key(WEBHOOK_HMAC_KEY.to_vec());
    let app = router(state);
    for body in [
        b"not json".to_vec(),
        br#"{"eventCode":"AUTHORISATION"}"#.to_vec(),
        br#"{"notificationItems":[]}"#.to_vec(),
        br#"{"notificationItems":[{"pspReference":"9916934567890123"}]}"#.to_vec(),
    ] {
        let response = send(&app, post_json("/v1/adyen/webhook", &body)).await;
        assert_eq!(response.status, 400, "body {body:?}");
        assert_eq!(response.json(), json!({"error": "malformed_event"}));
    }
}

#[tokio::test]
async fn unknown_routes_fall_back_to_json_not_found() {
    let temp = TempDir::new("fallback");
    let app = router(file_state(temp.path(), pub1_keys()));
    for path in ["/nope", "/v1", "/console", "/v1/audit/admob/extra"] {
        let response = send(&app, get(path)).await;
        assert_eq!(response.status, 404, "{path}");
        assert!(response.content_type().starts_with(JSON), "{path}");
        assert_eq!(response.json(), json!({"error": "not_found"}), "{path}");
    }
}

#[tokio::test]
async fn every_api_success_carries_application_json() {
    let temp = TempDir::new("content-types");
    write_standard_fixture(temp.path());
    let store = Arc::new(
        apex_billing::FileCollectionStore::open(&temp.path().join("collection.jsonl"))
            .expect("store"),
    );
    let state = file_state(temp.path(), pub1_keys())
        .with_adyen_hmac_key(WEBHOOK_HMAC_KEY.to_vec())
        .with_collection(store);
    let app = router(state);
    let requests = [
        get("/healthz"),
        get("/v1/audit/admob"),
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
        post(&format!("/v1/cycle/{PUBLISHER}?platform=admob")),
        post_json("/v1/adyen/webhook", &webhook_body()),
    ];
    for request in requests {
        let uri = request.uri().to_string();
        let response = send(&app, request).await;
        assert_eq!(response.status, 200, "{uri}: {}", response.body);
        assert!(
            response.content_type().starts_with(JSON),
            "{uri}: {}",
            response.content_type()
        );
    }
}

// --- hardening regressions pinned from the adversarial verification report ---

#[tokio::test]
async fn cycle_duplicate_query_keys_are_json_bad_request() {
    // The rejection mapping yields the JSON shape; plain-text axum output
    // refusals must stay `application/json` with the pinned code.
    let temp = TempDir::new("cycle-dup-query");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        post(&format!(
            "/v1/cycle/{PUBLISHER}?platform=admob&platform=admob"
        )),
    )
    .await;
    assert_eq!(response.status, 400);
    assert!(
        response.content_type().starts_with(JSON),
        "{}",
        response.content_type()
    );
    assert_eq!(response.json(), json!({"error": "bad_request"}));
}

#[tokio::test]
async fn invalid_utf8_path_segments_are_json_bad_request() {
    // `%FF` is not valid UTF-8: both parameterised `/v1` routes must answer
    // with the pinned JSON refusal instead of axum's plain-text PathRejection.
    let temp = TempDir::new("invalid-utf8-path");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    for uri in ["/v1/audit/%FF", "/v1/ledger/%FF"] {
        let response = send(&app, get(uri)).await;
        assert_eq!(response.status, 400, "{uri}");
        assert!(
            response.content_type().starts_with(JSON),
            "{uri}: {}",
            response.content_type()
        );
        assert_eq!(response.json(), json!({"error": "bad_request"}), "{uri}");
    }
}

#[tokio::test]
async fn lead_capture_requires_configuration() {
    let temp = TempDir::new("leads-503");
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        post_json(
            "/v1/leads",
            br#"{"publisher":"pub1","email":"a@studio.example","platforms":["admob"]}"#,
        ),
    )
    .await;
    assert_eq!(response.status, 503);
    assert_eq!(response.json()["error"], "lead_capture_not_configured");
}

#[tokio::test]
async fn lead_capture_accepts_a_valid_lead_and_refuses_bad_shapes() {
    let temp = TempDir::new("leads-202");
    let store = Arc::new(apex_api::FileLeadStore::new(
        temp.path().join("leads.jsonl"),
    ));
    let app = router(file_state(temp.path(), pub1_keys()).with_leads(store));
    let response = send(
        &app,
        post_json(
            "/v1/leads",
            br#"{"publisher":"pub1","email":"a@studio.example","platforms":["admob"]}"#,
        ),
    )
    .await;
    assert_eq!(response.status, 202, "{}", response.body);
    assert_eq!(response.json(), json!({"received": true}));

    for body in [
        br#"{"publisher":"pub1","email":"no-at-sign","platforms":["admob"]}"#.to_vec(),
        br#"{"publisher":"../etc","email":"a@b.co","platforms":["admob"]}"#.to_vec(),
        br#"{"publisher":"pub1","email":"a@b.co","platforms":["tiktok"]}"#.to_vec(),
        br#"{"publisher":"pub1","email":"a@b.co","platforms":[]}"#.to_vec(),
    ] {
        let response = send(&app, post_json("/v1/leads", &body)).await;
        assert_eq!(response.status, 400, "body {body:?}");
        assert_eq!(response.json(), json!({"error": "invalid_lead"}));
    }
}

#[tokio::test]
async fn oversized_request_body_is_json_413() {
    let temp = TempDir::new("cycle-oversized");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    // With `Content-Length` the pre-routing guard refuses; without it the
    // `DefaultBodyLimit` inside the handler's `Bytes` extraction must still
    // produce the same shaped refusal through the response mapper.
    for declared in [true, false] {
        let body = vec![b'x'; MAX_BODY_BYTES + 1];
        let mut builder = Request::builder()
            .method("POST")
            .uri(format!("/v1/cycle/{PUBLISHER}?platform=admob"))
            .header("content-type", "application/json");
        if declared {
            builder = builder.header(CONTENT_LENGTH, body.len().to_string());
        }
        let request = builder.body(Body::from(body)).expect("request");
        let response = send(&app, request).await;
        assert_eq!(response.status, 413, "declared={declared}");
        assert!(
            response.content_type().starts_with(JSON),
            "declared={declared}: {}",
            response.content_type()
        );
        assert_eq!(
            response.json(),
            json!({"error": "bad_request"}),
            "declared={declared}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ledger_symlink_escaping_the_root_is_refused() {
    use std::os::unix::fs::symlink;

    // A perfectly valid signed ledger exists *outside* the configured root;
    // a symlink placed inside the root must not make the API serve it.
    let root = TempDir::new("symlink-root");
    let outside = TempDir::new("symlink-outside");
    let target = write_ledger(
        outside.path(),
        PUBLISHER,
        &seed_keypair(7),
        vec![change_record(1)],
    );
    symlink(&target, root.path().join(format!("{PUBLISHER}.jsonl"))).expect("create symlink");
    let app = router(file_state(root.path(), pub1_keys()));

    let response = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(response.status, 404, "{}", response.body);
    assert_eq!(response.json()["error"], "not_found");

    let response = send(
        &app,
        get(&format!(
            "/console/ledger?publisher={PUBLISHER}&token={LEDGER_TOKEN}"
        )),
    )
    .await;
    assert_eq!(response.status, 404);
    assert!(response.body.contains("no ledger exists"));
}

#[tokio::test]
async fn empty_ledger_verifies_to_zero_records() {
    // An empty file is a valid empty ledger in `Ledger::verify`; the
    // single verified read keeps that contract and serves zero records.
    let temp = TempDir::new("ledger-empty");
    write_ledger(temp.path(), PUBLISHER, &seed_keypair(7), vec![]);
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.json()["records"], json!([]));
}

#[test]
fn ledger_read_path_is_pinned_to_a_single_read() {
    // Source-level pin (compile-time `include_str!`) of the verification
    // The ledger is parsed from the one read that
    // `Ledger::read_verified` performed, never from a second whole-file read
    // that could observe a file swapped after verification. The only raw
    // `std::fs::read` left in sources.rs is the audit-snapshot reader.
    const SOURCES: &str = include_str!("../src/sources.rs");
    assert_eq!(
        SOURCES.matches("std::fs::read(").count(),
        1,
        "sources.rs must contain exactly one raw file read (the snapshot reader)"
    );
    assert!(
        SOURCES.contains("Ledger::read_verified"),
        "the ledger path must go through the single verified read"
    );
    assert!(
        !SOURCES.contains("PersistedLine"),
        "the post-verification re-parse must stay deleted"
    );
}
