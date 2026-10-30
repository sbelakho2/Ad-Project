//! Adversarial integration tests for the JSON API: routing, status codes,
//! content types, path-traversal refusal and the Mollie webhook matrix.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use apex_api::{router, AppState, MAX_BODY_BYTES};

use axum::body::Body;
use axum::http::header;
use axum::http::header::CONTENT_LENGTH;
use axum::http::{Request, StatusCode};
use common::*;
use serde_json::json;

const JSON: &str = "application/json";
const HTML: &str = "text/html";

/// The state the cycle tests drive: the file fixture plus the configured
/// bearer token (`APEX_LIVE_AUDIT_TOKEN` in the real binary).
fn token_state(dir: &TempDir) -> AppState {
    file_state(dir.path(), pub1_keys()).with_live_auditor(token_auditor(API_TOKEN))
}

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
    let app = router(token_state(&temp));
    let response = send(
        &app,
        get_with_bearer(&format!("/v1/audit/admob?publisher={PUBLISHER}"), API_TOKEN),
    )
    .await;
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
    let app = router(token_state(&temp));
    for platform in ["tiktok", "AdMob", "applovin", "max", "%3Cscript%3E"] {
        let response = send(
            &app,
            get_with_bearer(
                &format!("/v1/audit/{platform}?publisher={PUBLISHER}"),
                API_TOKEN,
            ),
        )
        .await;
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
    let app = router(token_state(&temp));
    let response = send(
        &app,
        get_with_bearer(
            &format!("/v1/audit/levelplay?publisher={PUBLISHER}"),
            API_TOKEN,
        ),
    )
    .await;
    assert_eq!(response.status, 404);
    assert_eq!(response.json(), json!({"error": "not_found"}));
}

#[tokio::test]
async fn audit_snapshot_declaring_another_platform_is_internal_error() {
    let temp = TempDir::new("audit-mismatch");
    let mut snapshot = minimal_snapshot(&["u1"]);
    snapshot["platform"] = json!("levelplay");
    write_snapshot(temp.path(), "admob", &snapshot);
    let app = router(token_state(&temp));
    let response = send(
        &app,
        get_with_bearer(&format!("/v1/audit/admob?publisher={PUBLISHER}"), API_TOKEN),
    )
    .await;
    assert_eq!(response.status, 500);
    assert_eq!(response.json()["error"], "internal");
}

#[tokio::test]
async fn audit_report_requires_the_bearer_and_names_one_publisher() {
    let temp = TempDir::new("audit-auth");
    write_standard_fixture(temp.path());

    // No configured auditor: the read path refuses instead of serving an
    // unauthenticated snapshot.
    let no_auditor = router(file_state(temp.path(), pub1_keys()));
    let response = send(
        &no_auditor,
        get(&format!("/v1/audit/admob?publisher={PUBLISHER}")),
    )
    .await;
    assert_eq!(response.status, 503);
    assert_eq!(response.json()["error"], "live_audit_not_configured");

    let app = router(token_state(&temp));
    // Missing and wrong bearers are both refused before any snapshot read.
    for request in [
        get(&format!("/v1/audit/admob?publisher={PUBLISHER}")),
        get_with_bearer(
            &format!("/v1/audit/admob?publisher={PUBLISHER}"),
            "not-the-token",
        ),
    ] {
        let response = send(&app, request).await;
        assert_eq!(response.status, 401);
        assert_eq!(response.json()["error"], "unauthorized");
    }
    // The publisher parameter is required, and a hostile one is the typed
    // path refusal, not a lookup.
    let missing = send(&app, get_with_bearer("/v1/audit/admob", API_TOKEN)).await;
    assert_eq!(missing.status, 400);
    assert_eq!(missing.json()["error"], "publisher_required");
    for publisher in ["%2E%2E", "a%2Fb", "a%5Cb", "p%C3%BCb", &"a".repeat(65)] {
        let response = send(
            &app,
            get_with_bearer(&format!("/v1/audit/admob?publisher={publisher}"), API_TOKEN),
        )
        .await;
        assert_eq!(response.status, 400, "publisher {publisher:?}");
        assert_eq!(response.json()["error"], "invalid_publisher");
    }
    let allowed = send(
        &app,
        get_with_bearer(&format!("/v1/audit/admob?publisher={PUBLISHER}"), API_TOKEN),
    )
    .await;
    assert_eq!(allowed.status, 200, "{}", allowed.body);
}

#[tokio::test]
async fn audit_report_never_reads_another_publishers_snapshot() {
    let temp = TempDir::new("audit-isolation");
    write_snapshot_for(
        temp.path(),
        PUBLISHER,
        "admob",
        &minimal_snapshot(&["pub1-unit"]),
    );
    write_snapshot_for(
        temp.path(),
        PUBLISHER2,
        "admob",
        &minimal_snapshot(&["pub2-secret-unit"]),
    );
    let state = file_state(
        temp.path(),
        HashMap::from([
            (PUBLISHER.to_string(), seed_keypair(7).verifying_key()),
            (PUBLISHER2.to_string(), seed_keypair(9).verifying_key()),
        ]),
    )
    .with_live_auditor(token_auditor(API_TOKEN));
    let app = router(state);

    let own = send(
        &app,
        get_with_bearer(&format!("/v1/audit/admob?publisher={PUBLISHER}"), API_TOKEN),
    )
    .await;
    assert_eq!(own.status, 200, "{}", own.body);
    assert!(own.body.contains("pub1-unit"), "{}", own.body);
    assert!(
        !own.body.contains("pub2-secret-unit"),
        "another publisher's unit leaked: {}",
        own.body
    );

    let other = send(
        &app,
        get_with_bearer(
            &format!("/v1/audit/admob?publisher={PUBLISHER2}"),
            API_TOKEN,
        ),
    )
    .await;
    assert_eq!(other.status, 200, "{}", other.body);
    assert!(other.body.contains("pub2-secret-unit"), "{}", other.body);
    assert!(!other.body.contains("pub1-unit"), "{}", other.body);
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
async fn ledger_and_cycle_bearer_tokens_are_bound_to_their_publisher() {
    // Property: a ledger bearer token is only valid for its own publisher,
    // on every publisher-scoped bearer route. Cross reads and cross writes
    // are401 before any data is touched, and the denial never echoes data.
    let temp = TempDir::new("tenant-bearer");
    write_standard_fixture(temp.path());
    // A ledger file for the second publisher: the correct-pair assertion must
    // prove the binding works, not that the route404s for missing files.
    write_ledger(temp.path(), PUBLISHER2, &seed_keypair(9), vec![]);
    let state = file_state(temp.path(), two_publisher_keys())
        .with_ledger_token(PUBLISHER2, LEDGER_TOKEN2.as_bytes().to_vec());
    let app = router(state);

    // The correct pairs succeed, which proves the refusals below are
    // the publisher binding and not a broken route.
    let own1 = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(own1.status, 200, "{}", own1.body);
    let own2 = send(
        &app,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER2}"), LEDGER_TOKEN2),
    )
    .await;
    assert_eq!(own2.status, 200, "{}", own2.body);

    for (uri, token) in [
        (format!("/v1/ledger/{PUBLISHER2}"), LEDGER_TOKEN),
        (format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN2),
    ] {
        let denied = send(&app, get_with_bearer(&uri, token)).await;
        assert_eq!(denied.status, 401, "cross read {uri}: {}", denied.body);
        assert_eq!(denied.json()["error"], "unauthorized");
        assert!(
            !denied.body.contains("sequence") && !denied.body.contains("sha256"),
            "the denial leaks ledger data: {}",
            denied.body
        );
    }

    // A cross-publisher POST is refused before it can start work. This app
    // also carries a live auditor, so the 401 below is the publisher binding
    // and not the missing-config short circuit.
    let state = file_state(temp.path(), two_publisher_keys())
        .with_ledger_token(PUBLISHER2, LEDGER_TOKEN2.as_bytes().to_vec())
        .with_live_auditor(token_auditor(API_TOKEN));
    let cycled = router(state);
    let cycle = send(
        &cycled,
        post_with_bearer(&format!("/v1/cycle/{PUBLISHER2}"), LEDGER_TOKEN, b"{}"),
    )
    .await;
    assert_eq!(cycle.status, 401, "{}", cycle.body);
    let cycle = send(
        &cycled,
        post_with_bearer(&format!("/v1/cycle/{PUBLISHER}"), LEDGER_TOKEN2, b"{}"),
    )
    .await;
    assert_eq!(cycle.status, 401, "{}", cycle.body);
    let _ = std::fs::remove_dir_all(temp.path());
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
async fn cycle_without_authorization_is_401() {
    // The cycle changes publisher state upstream, so it carries the same
    // bearer gate as `POST /v1/audit/{platform}`: the gate runs before any
    // path or query handling, so an anonymous caller learns nothing else.
    let temp = TempDir::new("cycle-401");
    write_standard_fixture(temp.path());
    let app = router(token_state(&temp));
    let response = send(&app, post(&format!("/v1/cycle/{PUBLISHER}?platform=admob"))).await;
    assert_eq!(response.status, 401, "{}", response.body);
    assert_eq!(response.json()["error"], "unauthorized");
}

#[tokio::test]
async fn cycle_with_a_wrong_authorization_is_401() {
    let temp = TempDir::new("cycle-401-wrong");
    write_standard_fixture(temp.path());
    let app = router(token_state(&temp));
    let response = send(
        &app,
        post_with_bearer(
            &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
            "not-the-token",
            b"",
        ),
    )
    .await;
    assert_eq!(response.status, 401, "{}", response.body);
    assert_eq!(response.json()["error"], "unauthorized");
}

#[tokio::test]
async fn cycle_without_a_configured_token_source_is_503() {
    // No `APEX_LIVE_AUDIT_TOKEN` (no live auditor at all): the route cannot
    // verify any bearer, so it fails closed instead of opening the cycle.
    let temp = TempDir::new("cycle-503");
    write_standard_fixture(temp.path());
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, post(&format!("/v1/cycle/{PUBLISHER}?platform=admob"))).await;
    assert_eq!(response.status, 503, "{}", response.body);
    assert_eq!(response.json()["error"], "live_audit_not_configured");
}

#[tokio::test]
async fn cycle_requires_a_platform_parameter() {
    let temp = TempDir::new("cycle-platform");
    write_standard_fixture(temp.path());
    let app = router(token_state(&temp));
    let response = send(
        &app,
        post_with_bearer(&format!("/v1/cycle/{PUBLISHER}"), API_TOKEN, b""),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"], "platform_required");
}

#[tokio::test]
async fn cycle_summarises_audit_and_ledger_without_mutating() {
    let temp = TempDir::new("cycle-summary");
    write_standard_fixture(temp.path());
    let app = router(token_state(&temp));
    let response = send(
        &app,
        post_with_bearer(
            &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
            API_TOKEN,
            b"",
        ),
    )
    .await;
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
    let app = router(token_state(&temp));
    let response = send(
        &app,
        post_with_bearer(
            &format!("/v1/cycle/{PUBLISHER}?platform=tiktok"),
            API_TOKEN,
            b"",
        ),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"], "unknown_platform");
}

#[tokio::test]
async fn cycle_rejects_invalid_publishers() {
    let temp = TempDir::new("cycle-badpub");
    let app = router(token_state(&temp));
    let response = send(
        &app,
        post_with_bearer("/v1/cycle/%2E%2E?platform=admob", API_TOKEN, b""),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"], "invalid_publisher");
}

#[tokio::test]
async fn cycle_rejects_policy_outside_the_business_band() {
    let temp = TempDir::new("cycle-policy");
    write_standard_fixture(temp.path());
    let app = router(token_state(&temp));
    for body in [
        r#"{"policy":{"minLiftRateBp":1000}}"#,
        r#"{"policy":{"minLiftRateBp":2500}}"#,
        r#"{"policy":{"maxFeeMicros":-1}}"#,
    ] {
        let response = send(
            &app,
            post_json_with_bearer(
                &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
                API_TOKEN,
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
    let app = router(token_state(&temp));
    for body in [
        r#"{"policy":{"minLiftRateBp":1500}}"#,
        r#"{"policy":{"minLiftRateBp":2000,"maxFeeMicros":0}}"#,
    ] {
        let response = send(
            &app,
            post_json_with_bearer(
                &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
                API_TOKEN,
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
    let app = router(token_state(&temp));
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
            post_json_with_bearer(
                &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
                API_TOKEN,
                body,
            ),
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
    let response = send(
        &app,
        post_with_bearer(
            &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
            API_TOKEN,
            b"",
        ),
    )
    .await;
    assert_eq!(response.status, 200);
    for body in [b"{}".as_slice(), b"{\"policy\":null}"] {
        let response = send(
            &app,
            post_json_with_bearer(
                &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
                API_TOKEN,
                body,
            ),
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
    let response = send(&app, post_form_bytes("/v1/mollie/webhook", &webhook_body())).await;
    assert_eq!(response.status, 503);
    assert_eq!(
        response.json(),
        json!({"error": "mollie_webhook_not_configured"})
    );
}

#[tokio::test]
async fn webhook_fetched_payment_is_200_and_replay_is_deduplicated() {
    let temp = TempDir::new("webhook-200");
    let collection_path = temp.path().join("collection.jsonl");
    let store = Arc::new(apex_billing::FileCollectionStore::open(&collection_path).expect("store"));
    let state = file_state(temp.path(), pub1_keys())
        .with_mollie_test()
        .with_collection(store)
        .with_collection_log_path(&collection_path);
    let app = router(state);
    let response = send(&app, post_form_bytes("/v1/mollie/webhook", &webhook_body())).await;
    assert_eq!(response.status, 200, "{}", response.body);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&collection_path)
            .expect("collection log")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "collection log must be 0600, got {mode:o}");
    }
    assert_eq!(
        response.json(),
        json!({"received": true, "id": MOLLIE_PAYMENT_ID, "status": "paid", "new": true})
    );
    // A replay of the same (payment id, status) is accepted but recorded once.
    let replay = send(&app, post_form_bytes("/v1/mollie/webhook", &webhook_body())).await;
    assert_eq!(replay.status, 200);
    assert_eq!(replay.json()["new"], false, "replay is not re-recorded");
}

#[tokio::test]
async fn webhook_ignores_forged_fields_in_the_post_body() {
    let temp = TempDir::new("webhook-forged-fields");
    let store = Arc::new(
        apex_billing::FileCollectionStore::open(&temp.path().join("collection.jsonl"))
            .expect("store"),
    );
    let state = file_state(temp.path(), pub1_keys())
        .with_mollie_test()
        .with_collection(store);
    let app = router(state);
    // Register a paid payment; the form body claims a different status and
    // amount. Only the id is read, and the fetched payment is authoritative.
    let body = webhook_body();
    let forged = format!(
        "{}&status=failed&amount=0.01&metadata={{\"reference\":\"forged\"}}",
        String::from_utf8(body).expect("form")
    );
    let response = send(
        &app,
        post_form_bytes("/v1/mollie/webhook", forged.as_bytes()),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.json()["status"], "paid");
    assert_eq!(response.json()["new"], true);
}

#[tokio::test]
async fn webhook_without_a_collection_store_is_503() {
    let temp = TempDir::new("webhook-no-coll");
    let state = file_state(temp.path(), pub1_keys()).with_mollie_test();
    let app = router(state);
    let response = send(&app, post_form_bytes("/v1/mollie/webhook", &webhook_body())).await;
    assert_eq!(response.status, 503);
    assert_eq!(response.json()["error"], "collection_store_not_configured");
}

#[tokio::test]
async fn webhook_unknown_payment_is_400() {
    let temp = TempDir::new("webhook-unknown");
    let store = Arc::new(
        apex_billing::FileCollectionStore::open(&temp.path().join("collection.jsonl"))
            .expect("store"),
    );
    let state = file_state(temp.path(), pub1_keys())
        .with_mollie_test()
        .with_collection(store);
    let app = router(state);
    let response = send(
        &app,
        post_form_bytes("/v1/mollie/webhook", b"id=tr_does_not_exist"),
    )
    .await;
    assert_eq!(response.status, 400);
    assert_eq!(response.json(), json!({"error": "unknown_payment"}));
}

#[tokio::test]
async fn webhook_rejects_underpaid_or_inconsistent_payments() {
    let temp = TempDir::new("webhook-underpaid");
    let store = Arc::new(
        apex_billing::FileCollectionStore::open(&temp.path().join("collection.jsonl"))
            .expect("store"),
    );
    let state = file_state(temp.path(), pub1_keys())
        .with_mollie_payment(json!({
            "id": "tr_underpaid_api",
            "status": "paid",
            "amount": {"currency": "USD", "value": "1.00"},
            "metadata": {"reference": "apex-fee:pub1:exp:2026-10", "amount": "100.00"}
        }))
        .with_collection(store);
    let app = router(state);
    let response = send(
        &app,
        post_form_bytes("/v1/mollie/webhook", &webhook_body_with("tr_underpaid_api")),
    )
    .await;
    assert_eq!(response.status, 400, "{}", response.body);
    assert_eq!(response.json(), json!({"error": "invalid_payment"}));
}

#[tokio::test]
async fn webhook_accepts_invoice_link_payments_and_marks_the_invoice_paid() {
    // E7: a payment-link payment carries the `apex-invoice:` reference (no
    // line reference), so the webhook accepts it and records it in the
    // intent journal — the durable "invoice paid" fact.
    let temp = TempDir::new("webhook-invoice");
    let collection_path = temp.path().join("collection.jsonl");
    let store = Arc::new(apex_billing::FileCollectionStore::open(&collection_path).expect("store"));
    let intents_path = temp.path().join("payment-intents.jsonl");
    let intents = Arc::new(apex_billing::PaymentIntentStore::open(&intents_path).expect("intents"));
    let state = file_state(temp.path(), pub1_keys())
        .with_mollie_payment(json!({
            "id": "tr_invoice_api",
            "status": "paid",
            "amount": {"currency": "USD", "value": "10.00"},
            "metadata": {
                "reference": "apex-invoice:pub1:2026-10",
                "publisher": "publisher-pub1",
                "amount": "10.00"
            }
        }))
        .with_collection(store)
        .with_collection_log_path(&collection_path)
        .with_payment_intents(intents.clone());
    let app = router(state);
    let response = send(
        &app,
        post_form_bytes("/v1/mollie/webhook", &webhook_body_with("tr_invoice_api")),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.json()["received"], true);
    let record = intents
        .invoice_payment_for("INV-2026-10-pub1")
        .expect("read")
        .expect("the invoice payment is recorded");
    assert_eq!(record.payment_id, "tr_invoice_api");
    assert_eq!(record.publisher_id, "pub1");
    assert_eq!(record.year_month, "2026-10");
    assert!(record.is_paid(), "the invoice is marked paid");
    assert!(intents.invoice_is_paid("INV-2026-10-pub1").expect("read"));

    // A replay is accepted and appends nothing new.
    let replay = send(
        &app,
        post_form_bytes("/v1/mollie/webhook", &webhook_body_with("tr_invoice_api")),
    )
    .await;
    assert_eq!(replay.status, 200);
    let journal = std::fs::read_to_string(&intents_path).expect("journal");
    assert_eq!(
        journal
            .lines()
            .filter(|line| line.contains("\"paymentId\""))
            .count(),
        1,
        "the replayed link payment is idempotent"
    );
}

#[tokio::test]
async fn webhook_records_refund_and_chargeback_reversals_as_their_own_events() {
    // DD: a Mollie payment keeps the collected status through a refund or a
    // chargeback; the reversal arrives as amountRefunded/amountChargedBack.
    // Each reversal is recorded as its own collection event, which is what
    // the runner's payment-event classification consumes.
    let temp = TempDir::new("webhook-reversal");
    let collection_path = temp.path().join("collection.jsonl");
    let store = Arc::new(apex_billing::FileCollectionStore::open(&collection_path).expect("store"));
    let state = file_state(temp.path(), pub1_keys())
        .with_mollie_payment(json!({
            "id": "tr_reversal_api",
            "status": "paid",
            "amount": {"currency": "USD", "value": "10.00"},
            "amountRefunded": {"currency": "USD", "value": "2.50"},
            "amountChargedBack": {"currency": "USD", "value": "1.00"},
            "metadata": {
                "reference": "apex-fee:pub1:exp:2026-10",
                "publisher": "publisher-pub1",
                "amount": "10.00"
            }
        }))
        .with_collection(store);
    let app = router(state);
    let response = send(
        &app,
        post_form_bytes("/v1/mollie/webhook", &webhook_body_with("tr_reversal_api")),
    )
    .await;
    assert_eq!(response.status, 200, "{}", response.body);
    let raw = std::fs::read_to_string(&collection_path).expect("collection log");
    let mut recorded: Vec<(String, i64)> = raw
        .lines()
        .map(|line| {
            let event: serde_json::Value = serde_json::from_str(line).expect("event");
            (
                event["status"].as_str().expect("status").to_string(),
                event["amountMinorUnits"].as_i64().expect("amount"),
            )
        })
        .collect();
    recorded.sort();
    assert_eq!(
        recorded,
        vec![
            ("charged_back".to_string(), 100),
            ("paid".to_string(), 1_000),
            ("refunded".to_string(), 250),
        ],
        "the paid event plus one event per reversal"
    );
}

#[tokio::test]
async fn webhook_malformed_forms_are_refused() {
    let temp = TempDir::new("webhook-form");
    let store = Arc::new(
        apex_billing::FileCollectionStore::open(&temp.path().join("collection.jsonl"))
            .expect("store"),
    );
    let state = file_state(temp.path(), pub1_keys())
        .with_mollie_test()
        .with_collection(store);
    let app = router(state);
    for body in [
        b"".to_vec(),
        b"not json".to_vec(),
        b"id=".to_vec(),
        b"id=pl_not_a_payment".to_vec(),
        b"id=tr_one&id=tr_two".to_vec(),
        b"amount=100.00&status=paid".to_vec(),
    ] {
        let response = send(&app, post_form_bytes("/v1/mollie/webhook", &body)).await;
        assert_eq!(response.status, 400, "body {body:?}");
        assert_eq!(response.json(), json!({"error": "malformed_event"}));
    }
}

#[tokio::test]
async fn public_post_rate_limit_counts_each_forwarded_client() {
    let temp = TempDir::new("public-rate-limit");
    let app = router(file_state(temp.path(), pub1_keys()));
    // A loopback proxy peer: its X-Forwarded-For speaks for the client.
    let request = |client: &str| {
        let uri = "/v1/leads";
        let body = br#"{}"#;
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, JSON)
            .header("x-forwarded-for", format!("198.51.100.1, {client}"))
            .body(Body::from(body.to_vec()))
            .expect("request");
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "127.0.0.1:41234"
                .parse::<std::net::SocketAddr>()
                .expect("peer"),
        ));
        request
    };

    // The first 30 requests for one key are not rate limited (their handler
    // refusal, e.g. 400/503, is unrelated to the limiter).
    for attempt in 0..30 {
        let response = send(&app, request("203.0.113.7")).await;
        assert_ne!(response.status, 429, "request {attempt}");
    }
    let limited = send(&app, request("203.0.113.7")).await;
    assert_eq!(limited.status, 429);
    assert_eq!(limited.json()["error"], "rate_limited");
    assert!(limited.json()["retry_after_seconds"].is_i64());

    // The spoofed first entry is ignored: the proxy-appended last hop is the
    // key, so a different last hop has its own budget.
    let other = send(&app, request("203.0.113.8")).await;
    assert_ne!(other.status, 429);
}

#[tokio::test]
async fn an_untrusted_peer_cannot_split_its_budget_with_forwarded_headers() {
    let temp = TempDir::new("xff-trust");
    let app = router(file_state(temp.path(), pub1_keys()));
    // The immediate peer is not loopback and not a configured trusted
    // proxy, so the forwarded header is default-denied: every request keys
    // on the peer address, and spraying XFF entries never mints a new
    // budget.
    let request = |client: &str| {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/leads")
            .header(header::CONTENT_TYPE, JSON)
            .header("x-forwarded-for", client)
            .body(Body::from(br#"{}"#.to_vec()))
            .expect("request");
        request.extensions_mut().insert(axum::extract::ConnectInfo(
            "198.51.100.20:9999"
                .parse::<std::net::SocketAddr>()
                .expect("peer"),
        ));
        request
    };
    for attempt in 0..30 {
        let response = send(&app, request("203.0.113.40")).await;
        assert_ne!(response.status, 429, "request {attempt}");
    }
    let limited = send(&app, request("203.0.113.41")).await;
    assert_eq!(
        limited.status, 429,
        "a different XFF hop is the same untrusted peer, so the budget holds"
    );
}

#[tokio::test]
async fn the_public_cancel_surface_is_rate_limited_too() {
    let temp = TempDir::new("cancel-rate-limit");
    let app = router(file_state(temp.path(), pub1_keys()));
    let request = |client: &str| {
        axum::http::Request::builder()
            .method("POST")
            .uri("/watch/cancel")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("x-forwarded-for", client)
            .body(Body::from("token=not-a-token"))
            .expect("request")
    };
    for attempt in 0..30 {
        let response = send(&app, request("203.0.113.30")).await;
        assert_ne!(response.status, 429, "request {attempt}");
    }
    let limited = send(&app, request("203.0.113.30")).await;
    assert_eq!(limited.status, 429);
}

#[tokio::test]
async fn machine_prefixes_keep_json_not_found() {
    let temp = TempDir::new("fallback-machine");
    let app = router(file_state(temp.path(), pub1_keys()));
    for path in [
        "/v1",
        "/v1/audit/admob/extra",
        "/healthz/extra",
        "/metrics/extra",
        "/assets/missing.css",
        "/favicon.ico.extra",
    ] {
        let response = send(&app, get(path)).await;
        assert_eq!(response.status, 404, "{path}");
        assert!(response.content_type().starts_with(JSON), "{path}");
        assert_eq!(response.json(), json!({"error": "not_found"}), "{path}");
    }
}

#[tokio::test]
async fn browser_paths_fall_back_to_the_matching_html_404_shell() {
    let temp = TempDir::new("fallback-html");
    let app = router(file_state(temp.path(), pub1_keys()));

    // Console paths use the console shell.
    let console = send(&app, get("/console/other")).await;
    assert_eq!(console.status, 404);
    assert!(
        console.content_type().starts_with(HTML),
        "{}",
        console.content_type()
    );
    assert!(console.body.contains("Not found."), "{}", console.body);
    assert!(
        console.body.contains("Workspace / "),
        "the console shell frames the page: {}",
        console.body
    );

    // Everything else uses the marketing shell.
    let marketing = send(&app, get("/nope")).await;
    assert_eq!(marketing.status, 404);
    assert!(
        marketing.content_type().starts_with(HTML),
        "{}",
        marketing.content_type()
    );
    assert!(marketing.body.contains("Not found."), "{}", marketing.body);
    assert!(
        marketing.body.contains("marketing-header"),
        "the marketing shell frames the page: {}",
        marketing.body
    );
    assert!(
        !marketing.body.contains("Workspace / "),
        "a marketing 404 does not claim the console shell"
    );
}

#[tokio::test]
async fn bare_console_redirects_to_the_advertised_change_log() {
    let temp = TempDir::new("console-bare");
    let app = router(file_state(temp.path(), pub1_keys()));
    for path in ["/console", "/console/"] {
        let response = send(&app, get(path)).await;
        assert_eq!(response.status, StatusCode::SEE_OTHER, "{path}");
        assert_eq!(
            response
                .headers
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("/console/ledger"),
            "{path}"
        );
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
    let state = token_state(&temp).with_mollie_test().with_collection(store);
    let app = router(state);
    let requests = [
        get("/healthz"),
        get_with_bearer(&format!("/v1/audit/admob?publisher={PUBLISHER}"), API_TOKEN),
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
        post_with_bearer(
            &format!("/v1/cycle/{PUBLISHER}?platform=admob"),
            API_TOKEN,
            b"",
        ),
        post_form_bytes("/v1/mollie/webhook", &webhook_body()),
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

#[tokio::test]
async fn metrics_stays_available_on_the_direct_bind() {
    // The edge refuses `/metrics` publicly (infrastructure/caddy); the API
    // itself keeps serving it on its own bind for the operator scrape.
    let temp = TempDir::new("metrics-direct");
    let app = router(file_state(temp.path(), pub1_keys()));
    let response = send(&app, get("/metrics")).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert!(
        response.content_type().starts_with("text/plain"),
        "{}",
        response.content_type()
    );
    assert!(
        response.body.contains("apex_api_uptime_seconds"),
        "{}",
        response.body
    );
}

// --- hardening regressions pinned from the adversarial verification report ---

#[tokio::test]
async fn cycle_duplicate_query_keys_are_json_bad_request() {
    // The rejection mapping yields the JSON shape; plain-text axum output
    // refusals must stay `application/json` with the pinned code.
    let temp = TempDir::new("cycle-dup-query");
    write_standard_fixture(temp.path());
    let app = router(token_state(&temp));
    let response = send(
        &app,
        post_with_bearer(
            &format!("/v1/cycle/{PUBLISHER}?platform=admob&platform=admob"),
            API_TOKEN,
            b"",
        ),
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
    let app = router(token_state(&temp));
    for (uri, request) in [
        ("/v1/audit/%FF", get_with_bearer("/v1/audit/%FF", API_TOKEN)),
        ("/v1/ledger/%FF", get("/v1/ledger/%FF")),
    ] {
        let response = send(&app, request).await;
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
async fn lead_capture_cannot_name_another_publisher_without_a_token() {
    let temp = TempDir::new("leads-attribution");
    let leads_path = temp.path().join("leads.jsonl");
    let store = Arc::new(apex_api::FileLeadStore::new(&leads_path));
    let app = router(file_state(temp.path(), pub1_keys()).with_leads(store));
    let body = br#"{"publisher":"victim","email":"a@studio.example","platforms":["admob"]}"#;
    let response = send(&app, post_json("/v1/leads", body)).await;
    assert_eq!(response.status, 202, "{}", response.body);
    let raw = std::fs::read_to_string(&leads_path).expect("lead row");
    let row: serde_json::Value = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
    assert_eq!(
        row["lead"]["publisher"], "studio-example",
        "an anonymous lead is attributed to its email domain, never the body: {row}"
    );

    // With a configured token, the caller may name the workspace it owns,
    // but only after authenticating.
    let store = Arc::new(apex_api::FileLeadStore::new(
        temp.path().join("leads2.jsonl"),
    ));
    let app = router(
        file_state(temp.path(), pub1_keys())
            .with_leads(store)
            .with_leads_token(b"lead-secret".to_vec()),
    );
    let unauthorized = send(&app, post_json("/v1/leads", body)).await;
    assert_eq!(unauthorized.status, 401);
    let wrong = send(
        &app,
        post_with_header("/v1/leads", "authorization", "Bearer nope", body),
    )
    .await;
    assert_eq!(wrong.status, 401);
    let authorized = send(
        &app,
        post_with_header("/v1/leads", "authorization", "Bearer lead-secret", body),
    )
    .await;
    assert_eq!(authorized.status, 202, "{}", authorized.body);
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

    // The console exchange authorizes against the allow-list, mints the
    // session, and the session's read answers 404 for the symlink.
    let exchange = send(
        &app,
        post_form(
            "/console/ledger",
            &format!("publisher={PUBLISHER}&token={LEDGER_TOKEN}"),
        ),
    )
    .await;
    assert_eq!(exchange.status, 303);
    let response = send(
        &app,
        get_with_cookie("/console/ledger", &open_ledger_session(&app).await),
    )
    .await;
    assert_eq!(response.status, 404);
    assert!(response.body.contains("No ledger is available."));
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

/// The frozen `paid` outcome rides an accepted Mollie payment event: exactly
/// one line per `(kind, ref)`, nothing for a replay, nothing for a refused
/// payment, and nothing when no appender is configured.
#[tokio::test]
async fn accepted_payment_events_append_the_paid_outcome_once() {
    let temp = TempDir::new("outcomes-paid");
    let collection = Arc::new(
        apex_billing::FileCollectionStore::open(&temp.path().join("collection.jsonl"))
            .expect("store"),
    );
    let outcomes_path = temp.path().join("sales").join("outcomes.jsonl");
    let state = file_state(temp.path(), pub1_keys())
        .with_mollie_test()
        .with_collection(collection)
        .with_outcomes(Arc::new(apex_api::OutcomesAppender::new(&outcomes_path)));
    let app = router(state);

    let first = send(&app, post_form_bytes("/v1/mollie/webhook", &webhook_body())).await;
    assert_eq!(first.status, 200, "{}", first.body);
    assert_eq!(first.json()["new"], true);
    let outcomes = std::fs::read_to_string(&outcomes_path).expect("outcomes");
    let line: serde_json::Value = serde_json::from_str(outcomes.trim()).expect("line");
    assert_eq!(line["publisher"], "pub1");
    assert_eq!(line["kind"], "paid");
    assert_eq!(line["ref"], MOLLIE_PAYMENT_ID);
    assert!(line["atUnixMs"].as_i64().expect("at") > 0);

    // The replay is accepted but neither the event nor the outcome repeats.
    let replay = send(&app, post_form_bytes("/v1/mollie/webhook", &webhook_body())).await;
    assert_eq!(replay.status, 200);
    assert_eq!(replay.json()["new"], false);
    assert_eq!(
        std::fs::read_to_string(&outcomes_path)
            .expect("outcomes")
            .lines()
            .count(),
        1
    );

    // A refused payment (success=false, validly signed) records the event
    // but appends no outcome.
    let refused = signed_webhook_body("false", "9916934567890999");
    let response = send(&app, post_form_bytes("/v1/mollie/webhook", &refused)).await;
    assert_eq!(response.status, 200, "{}", response.body);
    assert_eq!(response.json()["new"], true);
    assert_eq!(
        std::fs::read_to_string(&outcomes_path)
            .expect("outcomes")
            .lines()
            .count(),
        1
    );

    // Without an appender the same accepted event still succeeds (the
    // outcomes file is best effort).
    let temp2 = TempDir::new("outcomes-absent");
    let collection2 = Arc::new(
        apex_billing::FileCollectionStore::open(&temp2.path().join("collection.jsonl"))
            .expect("store"),
    );
    let state2 = file_state(temp2.path(), pub1_keys())
        .with_mollie_test()
        .with_collection(collection2);
    let response = send(
        &router(state2),
        post_form_bytes("/v1/mollie/webhook", &webhook_body()),
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["new"], true);
}

/// Register a fetched payment for `payment_id` and return the form body.
fn signed_webhook_body(success: &str, payment_id: &str) -> Vec<u8> {
    let status = if success == "true" { "paid" } else { "failed" };
    let payment_id = if payment_id.starts_with("tr_") {
        payment_id.to_string()
    } else {
        format!("tr_{payment_id}")
    };
    register_payment(json!({
        "id": payment_id,
        "status": status,
        "amount": {"currency": "USD", "value": "10.00"},
        "metadata": {
            "reference": "apex-fee:pub1:exp:2026-10",
            "publisher": "publisher-pub1",
            "amount": "10.00"
        }
    }));
    format!("id={payment_id}").into_bytes()
}

#[tokio::test]
async fn the_one_click_unsubscribe_appends_a_suppression_outcome() {
    let temp = TempDir::new("unsubscribe");
    let path = temp.path().join("outcomes.jsonl");
    let state = file_state(temp.path(), pub1_keys())
        .with_outcomes(Arc::new(apex_api::outcomes::OutcomesAppender::new(&path)));
    let app = router(state);
    let email = "maria@studio.example";
    std::env::set_var("APEX_SALES_TOKEN_SALT", "test-unsubscribe-salt");
    let token = apex_api::unsubscribe_token(email);
    let page = send(
        &app,
        get(&format!("/unsubscribe?email={email}&token={token}")),
    )
    .await;
    assert_eq!(page.status, 200);
    let form = format!("email={email}&token={token}");
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/unsubscribe")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(form))
        .expect("request");
    let response = send(&app, request).await;
    assert_eq!(response.status, 200);
    let line = std::fs::read_to_string(&path).expect("outcome");
    assert!(line.contains("\"kind\":\"unsubscribe\""), "{line}");
    assert!(line.contains(email), "{line}");
    let wrong = send(&app, get(&format!("/unsubscribe?email={email}&token=bad"))).await;
    assert_eq!(wrong.status, 400);
}
