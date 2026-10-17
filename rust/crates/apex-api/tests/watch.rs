//! Integration tests for the paid setup-watch sign-up over the real router:
//! a pending subscriber and a checkout link, no checks before payment, the
//! one-time email confirmation, webhook activation (idempotent under replay)
//! and the honest unpaid state. The tier comes from the reported app count;
//! there is no tier checkbox.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;
use std::sync::{Arc, Mutex};

use apex_api::app_ads_proof::HttpAppAdsProof;
use apex_api::{router, AppState, FixedClock, StaticAuditSource, StaticLedgerSource};
use apex_billing::adyen::{sign_notification_item, NotificationItem};
use apex_billing::{AdyenClient, FileCollectionStore};
use apex_connectors::transport::{HttpRequest, HttpResponse, Transport, TransportError};
use apex_notify::sink::{
    DeliveryMessage, DeliveryReceipt, DeliveryState, NotificationSink, SinkError,
};
use apex_watch::{Status, SubscriberStore, Tier};
use axum::http::{header, StatusCode};
use common::*;

const HTML: &str = "text/html";

#[derive(Debug, Default)]
struct CaptureMailer {
    sent: Mutex<Vec<DeliveryMessage>>,
}

impl CaptureMailer {
    fn texts(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|message| message.text.clone())
            .collect()
    }

    fn last_text(&self) -> String {
        self.texts().pop().unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl NotificationSink for CaptureMailer {
    async fn send(&self, message: &DeliveryMessage) -> Result<DeliveryReceipt, SinkError> {
        self.sent.lock().unwrap().push(message.clone());
        Ok(DeliveryReceipt {
            state: DeliveryState::Accepted,
            provider_reference: None,
        })
    }
}

/// An Adyen checkout transport that records every request and answers a
/// payment link. The URL is deterministic so the confirmation page can be
/// asserted exactly.
#[derive(Debug, Default)]
struct CheckoutTransport {
    calls: Mutex<Vec<HttpRequest>>,
}

#[async_trait::async_trait]
impl Transport for CheckoutTransport {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(request);
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: br#"{"id":"PL-watch-1","url":"https://checkout.example/watch/PL-watch-1"}"#
                .to_vec(),
        })
    }
}

fn build_app(
    root: &Path,
    with_adyen: bool,
) -> (
    axum::Router,
    Arc<SubscriberStore>,
    Arc<CaptureMailer>,
    Option<Arc<CheckoutTransport>>,
) {
    let store = Arc::new(SubscriberStore::open(root.join("watch")));
    let mailer = Arc::new(CaptureMailer::default());
    let mailer_sink: Arc<dyn NotificationSink> = mailer.clone();
    let signin = Arc::new(
        apex_api::SigninService::open(root)
            .with_public_base("https://console.example")
            .with_mailer(mailer_sink),
    );
    let collection = Arc::new(
        FileCollectionStore::open(&root.join("collection.jsonl")).expect("collection store"),
    );
    let mut state = AppState::new(
        Arc::new(StaticAuditSource::default()),
        Arc::new(StaticLedgerSource::default()),
    )
    .with_clock(Arc::new(FixedClock::new(1_700_000_000)))
    .with_watch(Arc::clone(&store))
    .with_signin(signin)
    .with_collection(collection)
    .with_app_ads_proof(Arc::new(
        apex_api::app_ads_proof::StaticAppAdsProof::accepting(Vec::<String>::new())
            .with_app_count("northstar.example", 3)
            .with_app_count("bigport.example", 12)
            .with_app_count("bare.example", 1),
    ));
    let transport = if with_adyen {
        let transport = Arc::new(CheckoutTransport::default());
        let adyen = AdyenClient::new(
            "checkout-key",
            "ApexMerchant",
            WEBHOOK_HMAC_KEY.to_vec(),
            "https://adyen.example",
        )
        .expect("adyen client");
        state = state
            .with_adyen(Arc::new(adyen))
            .with_billing_transport(transport.clone());
        Some(transport)
    } else {
        None
    };
    (router(state), store, mailer, transport)
}

async fn submit_watch(app: &axum::Router, body: &str) -> String {
    let response = send(app, post_form("/watch", body)).await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let location = response
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("location")
        .to_string();
    location
        .strip_prefix("/watch/requested?ref=")
        .expect("the confirmation reference")
        .to_string()
}

fn token_from_mail(text: &str) -> String {
    const MARK: &str = "/watch/confirm?token=";
    let start = text
        .find(MARK)
        .expect("the mail carries the confirmation link")
        + MARK.len();
    let token: String = text[start..].chars().take(32).collect();
    assert_eq!(token.len(), 32, "mail link: {text}");
    assert!(
        token.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "mail link: {text}"
    );
    token
}

/// A signed Adyen AUTHORISATION notification for `reference`, carrying the
/// stored card the payment link asked Adyen to keep.
fn watch_webhook(reference: &str, psp_reference: &str) -> Vec<u8> {
    watch_webhook_with_merchant(&format!("apex-watch:{reference}"), psp_reference)
}

/// The same notification with exactly `merchant_reference` supplied: the
/// canonical prefixed form or the bare subscriber id the live dogfood harness
/// posts.
fn watch_webhook_with_merchant(merchant_reference: &str, psp_reference: &str) -> Vec<u8> {
    let mut item = NotificationItem {
        psp_reference: psp_reference.to_string(),
        original_reference: String::new(),
        merchant_account_code: "ApexMerchant".to_string(),
        merchant_reference: merchant_reference.to_string(),
        amount_value: 9_900,
        amount_currency: "USD".to_string(),
        event_code: "AUTHORISATION".to_string(),
        success: "true".to_string(),
        hmac_signature: String::new(),
        payment_method: "visa".to_string(),
        stored_payment_method_id: "pm_watch_1".to_string(),
    };
    item.hmac_signature = sign_notification_item(WEBHOOK_HMAC_KEY, &item);
    serde_json::json!({
        "live": "false",
        "notificationItems": [{
            "NotificationRequestItem": {
                "pspReference": item.psp_reference,
                "originalReference": "",
                "merchantAccountCode": item.merchant_account_code,
                "merchantReference": item.merchant_reference,
                "amount": {"value": item.amount_value, "currency": item.amount_currency},
                "eventCode": item.event_code,
                "success": item.success,
                "paymentMethod": item.payment_method,
                "additionalData": {
                    "hmacSignature": item.hmac_signature,
                    "storedPaymentMethodId": item.stored_payment_method_id
                }
            }
        }]
    })
    .to_string()
    .into_bytes()
}

fn row_count(store: &SubscriberStore) -> usize {
    std::fs::read_to_string(store.subscribers_path())
        .unwrap_or_default()
        .lines()
        .count()
}

#[tokio::test]
async fn signup_creates_a_pending_subscriber_checkout_link_and_confirmation_mail() {
    let temp = TempDir::new("watch-pending");
    let (app, store, mailer, transport) = build_app(temp.path(), true);

    let reference = submit_watch(
        &app,
        "domain=northstar.example&email=founder%40northstar.example&accounts=",
    )
    .await;
    assert!(reference.starts_with("sub_"), "{reference}");

    // Pending: no check has run, the watch is not active and the monthly
    // line cannot bill yet.
    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 1);
    let subscriber = &rows[0];
    assert_eq!(subscriber.status, Status::Pending);
    assert!(!subscriber.is_active());
    assert!(!subscriber.is_checkable());
    assert_eq!(subscriber.tier, Tier::Normal);
    assert!(
        !subscriber.confirmed,
        "the email is confirmed through its link"
    );
    assert!(subscriber.last_check_at.is_none());
    assert_eq!(
        subscriber.payment_link.as_deref(),
        Some("https://checkout.example/watch/PL-watch-1")
    );
    assert_eq!(
        subscriber.shopper_reference.as_deref(),
        Some(reference.as_str()),
        "the subscriber id is the Adyen shopperReference"
    );
    assert!(store.list_active().expect("active").is_empty());

    // The checkout request used the subscriber as shopperReference, the
    // standard tier price, and the watch's deterministic reference.
    let transport = transport.expect("checkout transport");
    let body = {
        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].url, "https://adyen.example/v71/paymentLinks");
        match calls[0].body.as_ref().expect("json body") {
            apex_connectors::transport::HttpBody::Json(value) => value.clone(),
            other => panic!("unexpected body {other:?}"),
        }
    };
    assert_eq!(body["amount"]["value"], 9_900, "standard price in cents");
    assert_eq!(body["amount"]["currency"], "USD");
    assert_eq!(body["shopperReference"], reference.as_str());
    assert_eq!(body["storePaymentMethod"], true);
    assert_eq!(
        body["reference"],
        serde_json::json!(format!("apex-watch:{reference}"))
    );

    // The confirmation page shows the real checkout action and the tier
    // derived from the evidence.
    let page = send(&app, get(&format!("/watch/requested?ref={reference}"))).await;
    assert_eq!(page.status, 200);
    assert!(page.content_type().starts_with(HTML));
    assert!(page
        .body
        .contains("https://checkout.example/watch/PL-watch-1"));
    assert!(
        page.body.contains("standard ($99 per month)"),
        "{}",
        page.body
    );

    // The confirmation email carries the single-use link.
    let text = mailer.last_text();
    assert!(
        text.contains("https://console.example/watch/confirm?token="),
        "{text}"
    );
    let token = token_from_mail(&text);
    let confirmed = send(&app, get(&format!("/watch/confirm?token={token}"))).await;
    assert_eq!(confirmed.status, 200, "{}", confirmed.body);
    assert!(
        confirmed.body.contains("Your email is confirmed."),
        "{}",
        confirmed.body
    );
    assert!(
        !confirmed.body.contains(&token),
        "the token is never echoed"
    );
    assert!(store.list().expect("list")[0].confirmed);
    assert!(store.list().expect("list")[0].email_confirmed_at.is_some());

    // Single use.
    let again = send(&app, get(&format!("/watch/confirm?token={token}"))).await;
    assert_eq!(again.status, 200);
    assert!(again.body.contains("no longer valid"), "{}", again.body);
}

#[tokio::test]
async fn the_tier_comes_from_the_app_count_evidence_not_a_checkbox() {
    let temp = TempDir::new("watch-tier");
    let (app, store, _mailer, transport) = build_app(temp.path(), true);

    // The form has no tier checkbox and no app-count input: the evidence is
    // the domain's public app-ads.txt, fetched by the proof source.
    let form = send(&app, get("/watch/")).await;
    assert_eq!(form.status, 200);
    assert!(!form.body.contains("name=\"tier\""), "{}", form.body);
    assert!(!form.body.contains("type=\"checkbox\""), "{}", form.body);
    assert!(!form.body.contains("name=\"app_count\""), "{}", form.body);

    // The proof source records twelve publisher account ids for bigport, so
    // the pending subscriber is priced at the core tier.
    let reference = submit_watch(
        &app,
        "domain=bigport.example&email=ops%40bigport.example&accounts=",
    )
    .await;
    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, reference);
    assert_eq!(
        rows[0].tier,
        Tier::Core,
        "twelve account ids is the core tier"
    );

    let transport = transport.expect("checkout transport");
    let body = {
        let calls = transport.calls.lock().unwrap();
        match calls[0].body.as_ref().expect("json body") {
            apex_connectors::transport::HttpBody::Json(value) => value.clone(),
            other => panic!("unexpected body {other:?}"),
        }
    };
    assert_eq!(body["amount"]["value"], 19_900, "core price in cents");

    // Evidence unavailable is the normal tier, honestly recorded, and the
    // pending sign-up is never blocked by it.
    let unknown = submit_watch(
        &app,
        "domain=unknown.example&email=ops%40unknown.example&accounts=",
    )
    .await;
    let rows = store.list().expect("list");
    let row = rows.iter().find(|row| row.id == unknown).expect("row");
    assert_eq!(row.tier, Tier::Normal);
}

/// A watch sign-up performs exactly one guarded app-ads fetch when it has a
/// valid form and a configured store, and none when validation fails or the
/// store cannot accept the subscriber. The counter is deterministic: it
/// counts transport requests, not wall-clock time.
#[tokio::test]
async fn a_watch_request_fetches_app_ads_once_after_validation() {
    const APP_ADS: &str = "OWNERDOMAIN=northstar.example\n\
                           google.com, pub-1, DIRECT\n\
                           google.com, pub-2, RESELLER\n";
    let temp = TempDir::new("watch-proof-fetch");
    let transport = Arc::new(CountingAppAdsTransport::new(APP_ADS));
    let proof = HttpAppAdsProof::new(Arc::clone(&transport) as Arc<dyn Transport>)
        .with_resolver(Arc::new(PublicResolver));
    let state = AppState::new(
        Arc::new(StaticAuditSource::default()),
        Arc::new(StaticLedgerSource::default()),
    )
    .with_clock(Arc::new(FixedClock::new(1_700_000_000)))
    .with_watch(Arc::new(SubscriberStore::open(temp.path().join("watch"))))
    .with_app_ads_proof(Arc::new(proof));
    let app = router(state);

    // Invalid form: refused before any network work.
    let invalid = send(
        &app,
        post_form(
            "/watch",
            "domain=northstar.example&email=not-an-email&accounts=",
        ),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(transport.calls(), 0, "validation refuses before the fetch");

    // One valid submission is one fetch (the tier evidence is read once).
    submit_watch(
        &app,
        "domain=northstar.example&email=founder%40northstar.example&accounts=",
    )
    .await;
    assert_eq!(transport.calls(), 1, "one guarded fetch per request");

    // A repeat submission is a second request: one more fetch, never two
    // inside one request.
    submit_watch(
        &app,
        "domain=northstar.example&email=founder%40northstar.example&accounts=",
    )
    .await;
    assert_eq!(transport.calls(), 2);

    // No subscriber store: the form cannot be stored, so no fetch happens.
    let no_store = router(
        AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        )
        .with_clock(Arc::new(FixedClock::new(1_700_000_000)))
        .with_app_ads_proof(Arc::new(
            HttpAppAdsProof::new(Arc::clone(&transport) as Arc<dyn Transport>)
                .with_resolver(Arc::new(PublicResolver)),
        )),
    );
    let unavailable = send(
        &no_store,
        post_form(
            "/watch",
            "domain=northstar.example&email=founder%40northstar.example&accounts=",
        ),
    )
    .await;
    assert_eq!(unavailable.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        transport.calls(),
        2,
        "an unstorable sign-up does not fetch the evidence"
    );
}

#[tokio::test]
async fn an_unpaid_signup_is_honest_and_never_starts_checks() {
    let temp = TempDir::new("watch-unpaid");
    // No Adyen checkout configured: the sign-up is still recorded pending,
    // but the page says a payment link will be emailed instead of inventing
    // one.
    let (app, store, mailer, _) = build_app(temp.path(), false);
    let reference = submit_watch(
        &app,
        "domain=northstar.example&email=founder%40northstar.example&accounts=",
    )
    .await;
    let page = send(&app, get(&format!("/watch/requested?ref={reference}"))).await;
    assert_eq!(page.status, 200);
    assert!(
        page.body
            .contains("We will email a payment link before the watch starts."),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains("Complete the checkout"),
        "{}",
        page.body
    );

    let rows = store.list().expect("list");
    assert_eq!(rows[0].status, Status::Pending);
    assert!(rows[0].payment_link.is_none());
    assert!(!rows[0].is_checkable());
    assert!(store.list_active().expect("active").is_empty());
    // The confirmation email still goes out and still names the honest state.
    assert!(
        mailer.last_text().contains("We will email a payment link"),
        "{:?}",
        mailer.texts()
    );
}

#[tokio::test]
async fn an_accepted_payment_activates_the_watch_and_a_replay_changes_nothing() {
    let temp = TempDir::new("watch-webhook");
    let (app, store, mailer, _) = build_app(temp.path(), true);
    let reference = submit_watch(
        &app,
        "domain=northstar.example&email=founder%40northstar.example&accounts=",
    )
    .await;
    assert_eq!(store.list().expect("list")[0].status, Status::Pending);
    // The confirmation link is consumed before payment in this scenario; a
    // paid-but-unconfirmed row stays inactive (the unpaid state test covers
    // the no-payment case).
    let token = token_from_mail(&mailer.last_text());
    let confirmed = send(&app, get(&format!("/watch/confirm?token={token}"))).await;
    assert_eq!(confirmed.status, 200);

    let body = watch_webhook(&reference, "psp-watch-1");
    let accepted = send(&app, post_json("/v1/adyen/webhook", &body)).await;
    assert_eq!(accepted.status, 200, "{}", accepted.body);
    assert_eq!(accepted.json()["new"], 1);

    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 1, "activation appends a row, never a draft");
    assert_eq!(rows[0].status, Status::Active);
    assert!(rows[0].is_checkable());
    assert!(rows[0].paid_at.is_some());
    assert_eq!(
        rows[0].stored_payment_method_id.as_deref(),
        Some("pm_watch_1"),
        "the stored card is recorded for the monthly bill"
    );
    let paid_at = rows[0].paid_at;
    let after_first = row_count(&store);

    // A replayed delivery is accepted by Adyen's own retry contract and is a
    // no-op for the subscriber: no second row, no second paid outcome.
    let replay = send(&app, post_json("/v1/adyen/webhook", &body)).await;
    assert_eq!(replay.status, 200, "{}", replay.body);
    assert_eq!(replay.json()["new"], 0, "{}", replay.body);
    assert_eq!(row_count(&store), after_first, "replay appends nothing");
    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, Status::Active);
    assert_eq!(rows[0].paid_at, paid_at, "the paid instant is unchanged");
}

#[tokio::test]
async fn a_bare_subscriber_reference_also_activates_and_replays_idempotently() {
    let temp = TempDir::new("watch-bare-reference");
    let (app, store, mailer, _) = build_app(temp.path(), true);
    let reference = submit_watch(
        &app,
        "domain=bare.example&email=ops%40bare.example&accounts=",
    )
    .await;
    let token = token_from_mail(&mailer.last_text());
    assert_eq!(
        send(&app, get(&format!("/watch/confirm?token={token}")))
            .await
            .status,
        200
    );
    assert_eq!(store.list().expect("list")[0].status, Status::Pending);

    // The harness posts the bare subscriber id (no `apex-watch:` prefix).
    let body = watch_webhook_with_merchant(&reference, "psp-watch-bare");
    let accepted = send(&app, post_json("/v1/adyen/webhook", &body)).await;
    assert_eq!(accepted.status, 200, "{}", accepted.body);
    assert_eq!(accepted.json()["new"], 1);
    let rows = store.list().expect("list");
    assert_eq!(rows[0].id, reference);
    assert_eq!(rows[0].status, Status::Active);
    assert_eq!(
        rows[0].stored_payment_method_id.as_deref(),
        Some("pm_watch_1")
    );
    let after_first = row_count(&store);

    let replay = send(&app, post_json("/v1/adyen/webhook", &body)).await;
    assert_eq!(replay.status, 200, "{}", replay.body);
    assert_eq!(replay.json()["new"], 0);
    assert_eq!(row_count(&store), after_first, "replay appends nothing");
    assert_eq!(store.list().expect("list")[0].status, Status::Active);
}
