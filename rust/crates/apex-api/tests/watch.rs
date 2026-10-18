//! Integration tests for the paid setup-watch sign-up over the real router:
//! a pending subscriber and a checkout link, no checks before payment, the
//! one-time email confirmation, webhook activation (idempotent under replay)
//! and the honest unpaid state. The tier comes from the distinct apps the
//! subscriber names; there is no tier checkbox.

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

/// A VIES stub: every number is invalid, so the VAT treatment stays domestic
/// without a network call.
#[derive(Debug, Default)]
struct StubVies;

impl apex_billing::ViesClient for StubVies {
    fn check_vat_number(
        &self,
        _country: &str,
        _number: &str,
    ) -> Result<apex_billing::ViesCheck, apex_billing::ViesError> {
        Ok(apex_billing::ViesCheck::Invalid {
            evidence: "stub: not on the register".to_string(),
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
    let seller = apex_billing::BillingProfile {
        legal_name: "Apex Media OU".to_string(),
        address_lines: vec!["Sepapaja tn 6".to_string(), "15551 Tallinn".to_string()],
        country: "EE".to_string(),
        vat_number: "EE123456789".to_string(),
        vat_rate_bp: 2_000,
    };
    let watch_billing = Arc::new(apex_api::WatchBilling {
        seller: Arc::new(seller),
        register_path: root.join("billing").join("invoices.jsonl"),
        vies: Arc::new(StubVies),
    });
    let mut state = AppState::new(
        Arc::new(StaticAuditSource::default()),
        Arc::new(StaticLedgerSource::default()),
    )
    .with_clock(Arc::new(FixedClock::new(1_700_000_000)))
    .with_watch(Arc::clone(&store))
    .with_signin(signin)
    .with_collection(collection)
    .with_watch_billing(watch_billing)
    .with_app_ads_proof(Arc::new(
        apex_api::app_ads_proof::StaticAppAdsProof::accepting(Vec::<String>::new())
            // The account-id histogram is deliberately not tier evidence.
            .with_app_count("accounts.example", 40)
            .with_app_count("bigport.example", 12),
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
        confirmed.body.contains("Confirm your billing details."),
        "{}",
        confirmed.body
    );
    assert!(
        confirmed.body.contains("name=\"profile_token\""),
        "the confirmed page carries the billing-profile form: {}",
        confirmed.body
    );
    assert!(
        !confirmed.body.contains(&token),
        "the email token is never echoed"
    );
    assert!(store.list().expect("list")[0].confirmed);
    assert!(store.list().expect("list")[0].email_confirmed_at.is_some());

    // Single use.
    let again = send(&app, get(&format!("/watch/confirm?token={token}"))).await;
    assert_eq!(again.status, 200);
    assert!(again.body.contains("no longer valid"), "{}", again.body);
}

#[tokio::test]
async fn the_tier_comes_from_the_distinct_named_apps_not_account_ids() {
    let temp = TempDir::new("watch-tier");
    let (app, store, _mailer, transport) = build_app(temp.path(), true);

    // The form has no tier or app-count input: the subscriber names apps.
    let form = send(&app, get("/watch/")).await;
    assert_eq!(form.status, 200);
    assert!(form.body.contains("name=\"apps\""), "{}", form.body);
    assert!(
        form.body.contains(">Apps (optional)</label>"),
        "{}",
        form.body
    );
    assert!(!form.body.contains("name=\"tier\""), "{}", form.body);
    assert!(!form.body.contains("type=\"checkbox\""), "{}", form.body);
    assert!(!form.body.contains("name=\"app_count\""), "{}", form.body);

    // Nine named apps stay the standard tier.
    let nine = (1..=9)
        .map(|index| format!("com.example.app{index}"))
        .collect::<Vec<_>>()
        .join("%0A");
    let reference = submit_watch(
        &app,
        &format!("domain=nine.example&email=ops%40nine.example&apps={nine}"),
    )
    .await;
    let rows = store.list().expect("list");
    let row = rows.iter().find(|row| row.id == reference).expect("row");
    assert_eq!(row.tier, Tier::Normal, "nine apps is the standard tier");

    // Ten distinct apps are the core tier, and the checkout uses its price.
    let ten = (1..=10)
        .map(|index| format!("com.example.app{index}"))
        .collect::<Vec<_>>()
        .join("%0A");
    let reference = submit_watch(
        &app,
        &format!("domain=bigport.example&email=ops%40bigport.example&apps={ten}"),
    )
    .await;
    let rows = store.list().expect("list");
    let row = rows.iter().find(|row| row.id == reference).expect("row");
    assert_eq!(row.tier, Tier::Core, "ten distinct apps is the core tier");
    let transport = transport.expect("checkout transport");
    let body = {
        let calls = transport.calls.lock().unwrap();
        let call = calls
            .iter()
            .find(|call| {
                matches!(
                    &call.body,
                    Some(apex_connectors::transport::HttpBody::Json(value))
                        if value["shopperReference"] == reference
                )
            })
            .expect("the core signup checkout");
        match call.body.as_ref().expect("json body") {
            apex_connectors::transport::HttpBody::Json(value) => value.clone(),
            other => panic!("unexpected body {other:?}"),
        }
    };
    assert_eq!(body["amount"]["value"], 19_900, "core price in cents");

    // The Kelvin reproduction: 49 ASCII apps plus K (U+212A) and k are 50
    // distinct under the Unicode identity the tier count uses. The old
    // ASCII-only parser counted 51 and refused this valid core signup.
    let mut kelvin: Vec<String> = (1..=49)
        .map(|index| format!("com.example.app{index}"))
        .collect();
    kelvin.push("K".to_string());
    kelvin.push("k".to_string());
    let reference = submit_watch(
        &app,
        &format!(
            "domain=kelvin.example&email=ops%40kelvin.example&apps={}",
            kelvin.join("%0A")
        ),
    )
    .await;
    let rows = store.list().expect("list");
    let row = rows.iter().find(|row| row.id == reference).expect("row");
    assert_eq!(
        row.tier,
        Tier::Core,
        "50 distinct apps including the Kelvin fold are accepted and stored as core"
    );

    // Blank apps stay normal.
    let blank = submit_watch(
        &app,
        "domain=blank.example&email=ops%40blank.example&apps=%0A%20%0A",
    )
    .await;
    let rows = store.list().expect("list");
    assert_eq!(
        rows.iter().find(|row| row.id == blank).expect("row").tier,
        Tier::Normal
    );

    // Duplicates collapse exactly: ten copies of one id are one app.
    let duplicate = submit_watch(
        &app,
        "domain=duplicate.example&email=ops%40duplicate.example&apps=com.example.same%0Acom.example.same%0Acom.example.same%0Acom.example.same%0Acom.example.same%0Acom.example.same%0Acom.example.same%0Acom.example.same%0Acom.example.same%0Acom.example.same",
    )
    .await;
    let rows = store.list().expect("list");
    assert_eq!(
        rows.iter()
            .find(|row| row.id == duplicate)
            .expect("row")
            .tier,
        Tier::Normal,
        "duplicates collapse to one distinct app"
    );

    // Account ids never influence the tier: many account ids, zero apps.
    let accounts = submit_watch(
        &app,
        "domain=accounts.example&email=ops%40accounts.example&accounts=admob.com+%3D+pub-1%0Agoogle.com+%3D+pub-2",
    )
    .await;
    let rows = store.list().expect("list");
    assert_eq!(
        rows.iter()
            .find(|row| row.id == accounts)
            .expect("row")
            .tier,
        Tier::Normal,
        "account ids are not apps"
    );

    // Fifty-one distinct apps are a typed validation refusal.
    let fifty_one = (1..=51)
        .map(|index| format!("com.example.app{index}"))
        .collect::<Vec<_>>()
        .join("%0A");
    let refused = send(
        &app,
        post_form(
            "/watch",
            &format!("domain=too-many.example&email=ops%40too-many.example&apps={fifty_one}"),
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert!(refused.body.contains("up to 50 lines"), "{}", refused.body);
    assert_eq!(
        store.list().expect("list").len(),
        6,
        "the refused form stores nothing"
    );
}

/// The watch tier never fetches app-ads.txt: the account ids there are not
/// apps, so the form's named apps are the only evidence.
#[tokio::test]
async fn a_watch_request_never_fetches_app_ads_for_the_tier() {
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

    submit_watch(
        &app,
        "domain=northstar.example&email=founder%40northstar.example&apps=com.example.one",
    )
    .await;
    assert_eq!(
        transport.calls(),
        0,
        "the tier comes from the named apps, not app-ads.txt account ids"
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

/// The value of one hidden input in rendered HTML.
fn hidden_value(body: &str, name: &str) -> String {
    let marker = format!("name=\"{name}\" value=\"");
    let start = body
        .find(&marker)
        .unwrap_or_else(|| panic!("no hidden {name} in {body}"))
        + marker.len();
    let rest = &body[start..];
    rest[..rest.find('"').expect("closing quote")].to_string()
}

/// Percent-encode one form value (`+` is not used, so newlines stay exact).
fn form_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Submit a watch, read the confirmation mail, and return the confirmed page
/// (which carries the billing-profile form) plus the profile token.
async fn confirmed_billing_page(
    app: &axum::Router,
    store: &SubscriberStore,
    mailer: &CaptureMailer,
    body: &str,
) -> (String, String) {
    let reference = submit_watch(app, body).await;
    let token = token_from_mail(&mailer.last_text());
    let confirmed = send(app, get(&format!("/watch/confirm?token={token}"))).await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.body);
    assert!(
        confirmed.body.contains("Confirm your billing details."),
        "{}",
        confirmed.body
    );
    assert!(
        confirmed.body.contains("name=\"profile_token\""),
        "the form only appears after the email confirmation: {}",
        confirmed.body
    );
    let profile_token = hidden_value(&confirmed.body, "profile_token");
    assert!(
        store
            .get(&reference)
            .expect("row")
            .expect("subscriber")
            .confirmed
    );
    (reference, profile_token)
}

fn billing_profile_body(
    profile_token: &str,
    reference: &str,
    legal_name: &str,
    address_lines: &str,
    country: &str,
    vat_number: &str,
) -> String {
    format!(
        "profile_token={}&reference={}&legal_name={}&address_lines={}&country={}&vat_number={}",
        form_encode(profile_token),
        form_encode(reference),
        form_encode(legal_name),
        form_encode(address_lines),
        form_encode(country),
        form_encode(vat_number),
    )
}

async fn activate_via_webhook(app: &axum::Router, reference: &str) {
    let response = send(
        app,
        post_with_header(
            "/v1/adyen/webhook",
            "content-type",
            "application/json",
            &watch_webhook(reference, "psp-billing-1"),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
}

#[tokio::test]
async fn the_billing_profile_is_stored_and_the_first_invoice_is_gapless() {
    let temp = TempDir::new("watch-billing");
    let (app, store, mailer, _transport) = build_app(temp.path(), true);
    let (reference, profile_token) = confirmed_billing_page(
        &app,
        &store,
        &mailer,
        "domain=northstar.example&email=founder%40northstar.example&apps=",
    )
    .await;
    activate_via_webhook(&app, &reference).await;

    let response = send(
        &app,
        post_form(
            "/watch/billing-profile",
            &billing_profile_body(
                &profile_token,
                &reference,
                "Turbine OU",
                "Sepapaja tn 6\n15551 Tallinn",
                "ee",
                "EE123456789",
            ),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let location = response
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("location")
        .to_string();
    assert_eq!(
        location,
        format!("/watch/billing-profile/recorded?ref={reference}")
    );

    let subscriber = store.get(&reference).expect("row").expect("subscriber");
    let profile = subscriber.billing_profile.expect("stored profile");
    assert_eq!(profile.legal_name, "Turbine OU");
    assert_eq!(
        profile.address_lines,
        vec!["Sepapaja tn 6".to_string(), "15551 Tallinn".to_string()]
    );
    assert_eq!(profile.country, "EE", "the country is canonical uppercase");
    assert_eq!(profile.vat_number.as_deref(), Some("EE123456789"));
    assert_eq!(
        subscriber.first_invoice_number,
        Some(1),
        "the first payment gets the first gapless number"
    );
    assert_eq!(
        subscriber.last_billed_month.as_deref(),
        Some(apex_watch::current_month(1_700_000_000_000).as_str())
    );

    let register = apex_billing::InvoiceRegister::open(&temp.path().join("billing/invoices.jsonl"))
        .expect("register");
    assert_eq!(register.highest().expect("highest"), 1, "one gapless line");

    let recorded = send(&app, get(&location)).await;
    assert_eq!(recorded.status, StatusCode::OK);
    assert!(recorded.body.contains("invoice 1"), "{}", recorded.body);
    assert!(!recorded.body.contains(&profile_token), "no token echo");
}

#[tokio::test]
async fn a_replayed_billing_profile_post_is_refused_without_overwriting() {
    let temp = TempDir::new("watch-billing-replay");
    let (app, store, mailer, _transport) = build_app(temp.path(), true);
    let (reference, profile_token) = confirmed_billing_page(
        &app,
        &store,
        &mailer,
        "domain=northstar.example&email=founder%40northstar.example&apps=",
    )
    .await;
    activate_via_webhook(&app, &reference).await;

    let first = send(
        &app,
        post_form(
            "/watch/billing-profile",
            &billing_profile_body(
                &profile_token,
                &reference,
                "Turbine OU",
                "Sepapaja tn 6",
                "EE",
                "",
            ),
        ),
    )
    .await;
    assert_eq!(first.status, StatusCode::SEE_OTHER, "{}", first.body);

    let replay = send(
        &app,
        post_form(
            "/watch/billing-profile",
            &billing_profile_body(
                &profile_token,
                &reference,
                "Changed Ltd",
                "Elsewhere 1",
                "DE",
                "",
            ),
        ),
    )
    .await;
    assert_eq!(replay.status, StatusCode::OK);
    assert!(
        replay
            .body
            .contains("This billing link is no longer valid."),
        "{}",
        replay.body
    );

    let subscriber = store.get(&reference).expect("row").expect("subscriber");
    assert_eq!(
        subscriber
            .billing_profile
            .as_ref()
            .map(|profile| profile.legal_name.as_str()),
        Some("Turbine OU"),
        "a replay never overwrites the stored profile"
    );
    assert_eq!(subscriber.first_invoice_number, Some(1));
    let register = apex_billing::InvoiceRegister::open(&temp.path().join("billing/invoices.jsonl"))
        .expect("register");
    assert_eq!(register.highest().expect("highest"), 1, "no second number");
}

#[tokio::test]
async fn hostile_billing_fields_are_typed_refusals_with_nothing_stored() {
    let temp = TempDir::new("watch-billing-invalid");
    let (app, store, mailer, _transport) = build_app(temp.path(), true);
    let (reference, profile_token) = confirmed_billing_page(
        &app,
        &store,
        &mailer,
        "domain=northstar.example&email=founder%40northstar.example&apps=",
    )
    .await;

    let long_address = "a".repeat(501);
    let hostile_name = format!("\" onmouseover=\"alert(1){}", "x".repeat(190));
    let response = send(
        &app,
        post_form(
            "/watch/billing-profile",
            &billing_profile_body(
                &profile_token,
                &reference,
                &hostile_name,
                &long_address,
                "E1",
                "bad!",
            ),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    for message in [
        "Enter the legal name that should appear on the invoice.",
        "Enter one address line per line (at least one, at most ten, 500 characters each).",
        "Enter the two-letter country code.",
        "Enter a VAT number of 4 to 14 letters and digits, or leave it blank.",
        "Check the highlighted fields before sending.",
    ] {
        assert!(response.body.contains(message), "missing {message}");
    }
    assert!(
        response.body.contains(&long_address),
        "safe values are preserved"
    );
    assert!(response
        .body
        .contains("value=\"&quot; onmouseover=&quot;alert(1)"));
    assert!(!response.body.contains("onmouseover=\"alert(1)\""));

    let subscriber = store.get(&reference).expect("row").expect("subscriber");
    assert!(subscriber.billing_profile.is_none(), "nothing is stored");
    assert!(subscriber.first_invoice_number.is_none());
    assert!(
        subscriber.profile_token_used_at.is_none(),
        "a field refusal does not burn the token, so the form can be corrected"
    );
    assert!(!temp.path().join("billing/invoices.jsonl").exists());
}

#[tokio::test]
async fn a_subscriber_without_a_profile_is_still_refused_by_the_monthly_bill() {
    let temp = TempDir::new("watch-billing-missing");
    let (app, store, mailer, _transport) = build_app(temp.path(), true);
    let reference = submit_watch(
        &app,
        "domain=northstar.example&email=founder%40northstar.example&apps=",
    )
    .await;
    let token = token_from_mail(&mailer.last_text());
    let confirmed = send(&app, get(&format!("/watch/confirm?token={token}"))).await;
    assert_eq!(confirmed.status, StatusCode::OK);
    activate_via_webhook(&app, &reference).await;

    let seller = apex_billing::BillingProfile {
        legal_name: "Apex Media OU".to_string(),
        address_lines: vec!["Sepapaja tn 6".to_string()],
        country: "EE".to_string(),
        vat_number: "EE123456789".to_string(),
        vat_rate_bp: 2_000,
    };
    let register_path = temp.path().join("billing/invoices.jsonl");
    let vies = StubVies;
    let deps = apex_watch::BillDeps {
        seller: &seller,
        register_path: &register_path,
        vies: &vies,
        adyen: None,
        transport: None,
    };
    let month = apex_watch::current_month(1_700_000_000_000);
    let report = apex_watch::run_bill(&store, &month, &deps)
        .await
        .expect("bill");
    assert!(report.issued.is_empty(), "no number is consumed");
    assert_eq!(report.refused.len(), 1);
    assert!(
        report.refused[0].reason.contains("billing profile"),
        "{}",
        report.refused[0].reason
    );
}

#[tokio::test]
async fn a_pending_subscriber_stores_the_profile_and_defers_the_invoice() {
    let temp = TempDir::new("watch-billing-pending");
    let (app, store, mailer, _transport) = build_app(temp.path(), true);
    let (reference, profile_token) = confirmed_billing_page(
        &app,
        &store,
        &mailer,
        "domain=northstar.example&email=founder%40northstar.example&apps=",
    )
    .await;

    let response = send(
        &app,
        post_form(
            "/watch/billing-profile",
            &billing_profile_body(
                &profile_token,
                &reference,
                "Turbine OU",
                "Sepapaja tn 6",
                "EE",
                "",
            ),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let location = response
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .expect("location")
        .to_string();

    let subscriber = store.get(&reference).expect("row").expect("subscriber");
    assert!(!subscriber.is_active(), "the watch is still pending");
    assert!(
        subscriber.billing_profile.is_some(),
        "the profile is stored"
    );
    assert!(
        subscriber.first_invoice_number.is_none(),
        "no invoice before activation"
    );
    assert!(!temp.path().join("billing/invoices.jsonl").exists());

    let recorded = send(&app, get(&location)).await;
    assert_eq!(recorded.status, StatusCode::OK);
    assert!(
        recorded.body.contains(
            "The invoice for the first payment will be issued when your watch activates."
        ),
        "{}",
        recorded.body
    );
}

#[tokio::test]
async fn case_variants_do_not_consume_the_fifty_app_bound() {
    let temp = TempDir::new("watch-apps-case");
    let (app, store, _mailer, _transport) = build_app(temp.path(), true);
    // Fifty distinct apps plus one case variant of the first: still fifty
    // distinct, so the form is accepted and priced as core.
    let mut lines: Vec<String> = (1..=50)
        .map(|index| format!("com.example.app{index}"))
        .collect();
    lines.push("COM.EXAMPLE.APP1".to_string());
    let apps = lines.join("%0A");
    let reference = submit_watch(
        &app,
        &format!("domain=case.example&email=ops%40case.example&apps={apps}"),
    )
    .await;
    let row = store.get(&reference).expect("row").expect("subscriber");
    assert_eq!(
        row.tier,
        Tier::Core,
        "the case variant is the same app, so the fifty-line bound is not exceeded"
    );
}

#[tokio::test]
async fn subscriber_lookups_probe_one_indexed_row_at_every_size() {
    for subscribers in [500usize, 1_000, 2_000] {
        let temp = TempDir::new("watch-lookup-index");
        let (app, store, _mailer, _transport) = build_app(temp.path(), true);
        let mut last = String::new();
        for index in 0..subscribers {
            let outcome = store
                .subscribe(
                    1_700_000_000 + index as i64,
                    &format!("index{index}.example"),
                    &format!("ops{index}@index.example"),
                    "",
                    false,
                )
                .expect("subscribe");
            last = outcome.subscriber.id;
        }

        let before = store.indexed_row_probes();
        let requested = send(&app, get(&format!("/watch/requested?ref={last}"))).await;
        assert_eq!(requested.status, StatusCode::OK);
        let requested_probes = store.indexed_row_probes() - before;

        let before = store.indexed_row_probes();
        let recorded = send(
            &app,
            get(&format!("/watch/billing-profile/recorded?ref={last}")),
        )
        .await;
        assert_eq!(recorded.status, StatusCode::OK);
        let recorded_probes = store.indexed_row_probes() - before;

        println!(
            "subscribers={subscribers} requested_probes={requested_probes} recorded_probes={recorded_probes}"
        );
        assert_eq!(
            requested_probes, 1,
            "subscribers={subscribers}: the confirmation page uses the id index"
        );
        assert_eq!(
            recorded_probes, 1,
            "subscribers={subscribers}: the recorded page uses the id index"
        );
    }
}
