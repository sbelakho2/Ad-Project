//! Integration tests for the paid setup-watch sign-up over the real router:
//! a pending subscriber with no checkout link at sign-up, the one-time email
//! confirmation, the billing profile recorded before the Mollie link is
//! created, webhook activation (idempotent under replay) and the honest
//! unpaid state. The tier comes from the distinct apps the public App Store
//! confirms; there is no tier checkbox.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use apex_api::app_ads_proof::HttpAppAdsProof;
use apex_api::{router, AppState, FixedClock, StaticAuditSource, StaticLedgerSource};
use apex_billing::{FileCollectionStore, MollieClient};
use apex_connectors::transport::{HttpRequest, HttpResponse, Transport, TransportError};
use apex_notify::sink::{
    DeliveryMessage, DeliveryReceipt, DeliveryState, NotificationSink, SinkError,
};
use apex_watch::{Status, SubscriberStore, Tier};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
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

/// An Mollie checkout transport that records every request and answers both
/// surfaces the watch uses: the public App Store lookup (a real result for
/// `com.example.app*`, no result for anything else) and the Mollie payment
/// link. The link URL is deterministic so the confirmation page can be
/// asserted exactly.
#[derive(Debug, Default)]
struct CheckoutTransport {
    calls: Mutex<Vec<HttpRequest>>,
    verified: Mutex<std::collections::BTreeSet<String>>,
    fail_links: AtomicBool,
    play: AtomicBool,
}

impl CheckoutTransport {
    /// Treat exactly these app ids as real public listings instead of the
    /// default `com.example.app*` rule.
    fn set_verified(&self, apps: &[&str]) {
        let mut verified = self.verified.lock().unwrap();
        verified.clear();
        verified.extend(apps.iter().map(|app| (*app).to_string()));
    }

    /// Make every subsequent payment-link request answer an honest failure.
    fn fail_links(&self) {
        self.fail_links.store(true, Ordering::SeqCst);
    }

    fn enable_play_listing(&self) {
        self.play.store(true, Ordering::SeqCst);
    }

    fn is_verified(&self, app: &str) -> bool {
        let explicit = self.verified.lock().unwrap();
        !explicit.is_empty() && explicit.contains(app)
            || explicit.is_empty() && app.starts_with("com.example.app")
    }

    fn app_calls(&self) -> Vec<HttpRequest> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.url.starts_with("https://itunes.apple.com/lookup"))
            .cloned()
            .collect()
    }

    fn link_calls(&self) -> Vec<HttpRequest> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| {
                call.url.contains("api.mollie.test")
                    && call.method == apex_connectors::transport::HttpMethod::Post
            })
            .cloned()
            .collect()
    }
}

#[async_trait::async_trait]
impl Transport for CheckoutTransport {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls.lock().unwrap().push(request.clone());
        if request.url.starts_with("https://itunes.apple.com/lookup") {
            let app = request
                .url
                .split_once("bundleId=")
                .map(|(_, app)| app)
                .or_else(|| request.url.split_once("id=").map(|(_, app)| app))
                .unwrap_or_default();
            let result_count = usize::from(self.is_verified(app));
            let body = serde_json::json!({
                "resultCount": result_count,
                "results": if result_count == 0 {
                    serde_json::json!([])
                } else {
                    serde_json::json!([{"trackName": "Test App"}])
                }
            });
            return Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: serde_json::to_vec(&body).unwrap(),
            });
        }
        if !self.play.load(Ordering::SeqCst) && request.url.starts_with("https://play.google.com/")
        {
            return Ok(HttpResponse {
                status: 404,
                headers: vec![],
                body: Vec::new(),
            });
        }
        if self.play.load(Ordering::SeqCst)
            && request
                .url
                .starts_with("https://play.google.com/store/apps/details")
        {
            return Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: br#"<html><h1 itemprop="name">Android App</h1><a href="/store/apps/dev?id=Dev_1">Developer</a></html>"#.to_vec(),
            });
        }
        if self.play.load(Ordering::SeqCst)
            && request
                .url
                .starts_with("https://play.google.com/store/apps/dev")
        {
            let mut body = String::from("<html>");
            for index in 0..11 {
                body.push_str(&format!(
                    "<a href=\"/store/apps/details?id=com.example.play{index}\">App</a>"
                ));
            }
            body.push_str("</html>");
            return Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: body.into_bytes(),
            });
        }
        if request.url.contains("api.mollie.test") {
            if self.fail_links.load(Ordering::SeqCst) {
                return Ok(HttpResponse {
                    status: 500,
                    headers: vec![],
                    body: br#"{"status":500,"title":"simulated refusal"}"#.to_vec(),
                });
            }
            if let Some(response) = common::mollie_api(&request) {
                return Ok(response);
            }
        }
        Ok(HttpResponse {
            status: 404,
            headers: vec![],
            body: Vec::new(),
        })
    }
}

fn build_app(
    root: &Path,
    with_mollie: bool,
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
    let transport = if with_mollie {
        let transport = Arc::new(CheckoutTransport::default());
        let mollie = MollieClient::new(
            "checkout-key",
            "https://api.mollie.test",
            "https://apex.test/v1/mollie/webhook",
            "https://apex.test/watch/return",
        )
        .expect("mollie client");
        state = state
            .with_mollie(Arc::new(mollie))
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

/// Follow the emailed link exactly as a person does: the GET must render the
/// inert confirmation form, and only the POST consumes the token.
async fn confirm_watch(app: &axum::Router, token: &str) -> common::TestResponse {
    let landing = send(app, get(&format!("/watch/confirm?token={token}"))).await;
    assert_eq!(landing.status, StatusCode::OK, "{}", landing.body);
    assert!(
        landing
            .body
            .contains("<form method=\"post\" action=\"/watch/confirm\">"),
        "the GET renders the confirmation button: {}",
        landing.body
    );
    assert_eq!(hidden_value(&landing.body, "token"), token);
    send(app, post_form("/watch/confirm", &format!("token={token}"))).await
}

/// Register the payment a webhook will name and return the form body.
fn watch_webhook(reference: &str, payment_id: &str) -> Vec<u8> {
    watch_webhook_with_reference(&format!("apex-watch:{reference}"), payment_id)
}

fn payment_with_id(
    payment_id: &str,
    reference: &str,
    amount_minor_units: i64,
    status: &str,
    currency: &str,
) -> serde_json::Value {
    let amount = format!(
        "{}.{:02}",
        amount_minor_units / 100,
        amount_minor_units % 100
    );
    serde_json::json!({
        "resource": "payment",
        "id": payment_id,
        "status": status,
        "amount": {"currency": currency, "value": amount},
        "metadata": {
            "reference": reference,
            "publisher": "publisher-pub1",
            "amount": amount
        },
        "customerId": "cst_watch_1",
        "mandateId": "mdt_watch_1",
        "sequenceType": "first"
    })
}

/// The same delivery with exactly `reference` supplied: the canonical
/// prefixed form or the bare subscriber id the live dogfood harness posts.
fn watch_webhook_with_reference(reference: &str, payment_id: &str) -> Vec<u8> {
    let payment_id = if payment_id.starts_with("tr_") {
        payment_id.to_string()
    } else {
        format!("tr_{payment_id}")
    }
    .replace('-', "_");
    register_payment(payment_with_id(
        &payment_id,
        reference,
        9_900,
        "paid",
        "USD",
    ));
    format!("id={payment_id}").into_bytes()
}

/// A fetched payment with every field chosen by the caller. The arguments
/// keep the old notification-shaped call sites; the event code and success
/// flag map onto Mollie's status.
#[allow(clippy::too_many_arguments)]
fn signed_notification(
    reference: &str,
    payment_id: &str,
    event_kind: &str,
    success: &str,
    amount_minor_units: i64,
    currency: &str,
    _merchant_account_code: &str,
) -> Vec<u8> {
    let status = match event_kind {
        "AUTHORISATION" if success == "true" => "paid",
        "AUTHORISATION" => "failed",
        "REFUND" => "refunded",
        "DISPUTE" | "CHARGEBACK" | "NOTIFICATION_OF_CHARGEBACK" => "charged_back",
        "RECONCILIATION" => "authorized",
        other => other,
    };
    let payment_id = if payment_id.starts_with("tr_") {
        payment_id.to_string()
    } else {
        format!("tr_{payment_id}")
    }
    .replace('-', "_");
    register_payment(payment_with_id(
        &payment_id,
        reference,
        amount_minor_units,
        status,
        currency,
    ));
    format!("id={payment_id}").into_bytes()
}

fn row_count(store: &SubscriberStore) -> usize {
    std::fs::read_to_string(store.subscribers_path())
        .unwrap_or_default()
        .lines()
        .count()
}

#[tokio::test]
async fn signup_records_pending_and_sends_only_the_confirmation_mail() {
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
    assert_eq!(subscriber.verified_app_count, Some(0));
    assert!(
        !subscriber.confirmed,
        "the email is confirmed through its link"
    );
    assert!(subscriber.last_check_at.is_none());
    assert!(
        subscriber.payment_link.is_none(),
        "signup must not create a checkout link"
    );
    assert_eq!(
        subscriber.customer_reference.as_deref(),
        Some(reference.as_str()),
        "the subscriber id is the Mollie shopperReference"
    );
    assert!(store.list_active().expect("active").is_empty());

    // No checkout request and no payment link on the requested page.
    let transport = transport.expect("checkout transport");
    assert!(
        transport.link_calls().is_empty(),
        "no payment link is created at signup"
    );
    assert!(
        transport.app_calls().is_empty(),
        "no apps were submitted, so no lookup was needed"
    );
    let page = send(&app, get(&format!("/watch/requested?ref={reference}"))).await;
    assert_eq!(page.status, 200);
    assert!(page.content_type().starts_with(HTML));
    assert!(
        page.body.contains(
            "We will show your payment link here once your billing details are recorded."
        ),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains("Complete the checkout"),
        "{}",
        page.body
    );
    assert!(
        page.body.contains("standard ($99 per month)"),
        "{}",
        page.body
    );

    // The confirmation email carries only the single-use email link.
    let text = mailer.last_text();
    assert!(
        text.contains("https://console.example/watch/confirm?token="),
        "{text}"
    );
    assert!(
        text.contains(
            "We will show your payment link here once your billing details are recorded."
        ),
        "the signup mail does not invent a checkout link: {text}"
    );
    let token = token_from_mail(&text);
    let confirmed = confirm_watch(&app, &token).await;
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
    assert!(
        transport.link_calls().is_empty(),
        "confirming the email still creates no link"
    );

    // The billing-profile POST records the profile first, then creates the
    // checkout link and redirects to the requested page that shows it.
    let profile_token = hidden_value(&confirmed.body, "profile_token");
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
    assert_eq!(location, format!("/watch/requested?ref={reference}"));

    let subscriber = store.get(&reference).expect("row").expect("subscriber");
    assert!(
        subscriber.billing_profile.is_some(),
        "the profile is recorded first"
    );
    assert_eq!(
        subscriber.payment_link.as_deref(),
        Some("https://checkout.example/tr_first_api_test")
    );
    assert!(subscriber.payment_link_error.is_none());
    let link_calls = transport.link_calls();
    assert_eq!(
        link_calls.len(),
        2,
        "the profile POST creates the customer and first payment"
    );
    assert_eq!(link_calls[0].url, "https://api.mollie.test/v2/customers");
    let body = match link_calls[1].body.as_ref().expect("json body") {
        apex_connectors::transport::HttpBody::Json(value) => value.clone(),
        other => panic!("unexpected body {other:?}"),
    };
    assert_eq!(
        body["amount"]["value"], "99.00",
        "standard price in dollars"
    );
    assert_eq!(body["amount"]["currency"], "USD");
    assert_eq!(body["metadata"]["publisher"], reference.as_str());

    assert_eq!(
        body["metadata"]["reference"],
        serde_json::json!(format!("apex-watch:{reference}"))
    );

    let page = send(&app, get(&location)).await;
    assert_eq!(page.status, 200);
    assert!(
        page.body
            .contains("https://checkout.example/tr_first_api_test"),
        "{}",
        page.body
    );
    assert!(page.body.contains("Complete the checkout"), "{}", page.body);

    // A replay is refused and creates no second link.
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
    assert_eq!(
        transport.link_calls().len(),
        2,
        "the replay creates no second customer/payment pair"
    );

    // Single use.
    let again = send(&app, post_form("/watch/confirm", &format!("token={token}"))).await;
    assert_eq!(again.status, 200);
    assert!(again.body.contains("no longer valid"), "{}", again.body);
}

#[tokio::test]
async fn watch_confirm_get_is_inert_and_post_consumes_once() {
    let temp = TempDir::new("watch-confirm-inert");
    let (app, store, mailer, _transport) = build_app(temp.path(), true);
    let reference = submit_watch(
        &app,
        "domain=northstar.example&email=founder%40northstar.example&accounts=",
    )
    .await;
    let token = token_from_mail(&mailer.last_text());

    // A mail scanner (or a person rereading the mail) may follow the link
    // repeatedly: every GET renders the button and none consumes the token.
    for attempt in 0..2 {
        let landing = send(&app, get(&format!("/watch/confirm?token={token}"))).await;
        assert_eq!(landing.status, StatusCode::OK, "GET {attempt}");
        assert!(
            landing
                .body
                .contains("<form method=\"post\" action=\"/watch/confirm\">"),
            "GET {attempt}: {}",
            landing.body
        );
        assert_eq!(hidden_value(&landing.body, "token"), token);
        let subscriber = store.get(&reference).expect("row").expect("subscriber");
        assert!(!subscriber.confirmed, "GET {attempt} must not confirm");
        assert!(
            subscriber.confirmation_used_at.is_none(),
            "GET {attempt} must not consume the token"
        );
    }

    let confirmed = send(&app, post_form("/watch/confirm", &format!("token={token}"))).await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.body);
    assert!(
        confirmed.body.contains("Confirm your billing details."),
        "{}",
        confirmed.body
    );
    assert!(
        store
            .get(&reference)
            .expect("row")
            .expect("subscriber")
            .confirmed
    );

    let replay = send(&app, post_form("/watch/confirm", &format!("token={token}"))).await;
    assert_eq!(replay.status, StatusCode::OK);
    assert!(
        replay.body.contains("no longer valid"),
        "the second POST refuses: {}",
        replay.body
    );
}

#[tokio::test]
async fn the_tier_comes_from_the_distinct_verified_apps_not_account_ids() {
    let temp = TempDir::new("watch-tier");
    let (app, store, mailer, transport) = build_app(temp.path(), true);

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
    assert_eq!(row.verified_app_count, Some(9));

    // Ten distinct verified apps are the core tier; the checkout link is
    // created only after the email confirmation and billing profile, and it
    // uses the verified tier's price.
    let ten = (1..=10)
        .map(|index| format!("com.example.app{index}"))
        .collect::<Vec<_>>()
        .join("%0A");
    let (reference, profile_token) = confirmed_billing_page(
        &app,
        &store,
        &mailer,
        &format!("domain=bigport.example&email=ops%40bigport.example&apps={ten}"),
    )
    .await;
    let row = store.get(&reference).expect("row").expect("subscriber");
    assert_eq!(row.tier, Tier::Core, "ten verified apps is the core tier");
    assert_eq!(row.verified_app_count, Some(10));
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
    let transport = transport.expect("checkout transport");
    let body = {
        let calls = transport.link_calls();
        let call = calls
            .iter()
            .find(|call| {
                matches!(
                    &call.body,
                    Some(apex_connectors::transport::HttpBody::Json(value))
                        if value["metadata"]["publisher"] == reference
                )
            })
            .expect("the core profile checkout");
        match call.body.as_ref().expect("json body") {
            apex_connectors::transport::HttpBody::Json(value) => value.clone(),
            other => panic!("unexpected body {other:?}"),
        }
    };
    assert_eq!(body["amount"]["value"], "199.00", "core price in dollars");

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

#[tokio::test]
async fn a_failed_checkout_link_still_records_the_profile_and_is_honest() {
    let temp = TempDir::new("watch-link-failure");
    let (app, store, mailer, transport) = build_app(temp.path(), true);
    let (reference, profile_token) = confirmed_billing_page(
        &app,
        &store,
        &mailer,
        "domain=linkfail.example&email=ops%40linkfail.example&apps=",
    )
    .await;
    let transport = transport.expect("checkout transport");
    transport.fail_links();

    let response = send(
        &app,
        post_form(
            "/watch/billing-profile",
            &billing_profile_body(
                &profile_token,
                &reference,
                "Linkfail OU",
                "Sepapaja tn 6",
                "EE",
                "EE123456789",
            ),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response
            .headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("/watch/requested?ref={reference}").as_str())
    );

    let subscriber = store.get(&reference).expect("row").expect("subscriber");
    assert_eq!(
        subscriber
            .billing_profile
            .as_ref()
            .map(|profile| profile.legal_name.as_str()),
        Some("Linkfail OU"),
        "the profile is recorded before the link attempt"
    );
    assert!(
        subscriber.payment_link.is_none(),
        "no link is stored when Mollie refuses"
    );
    let error = subscriber
        .payment_link_error
        .as_deref()
        .expect("the honest failure is recorded");
    assert!(error.contains("payment link"), "{error}");
    assert!(
        !subscriber.is_checkable(),
        "still pending without a payment"
    );

    let requested = send(&app, get(&format!("/watch/requested?ref={reference}"))).await;
    assert_eq!(requested.status, StatusCode::OK);
    assert!(
        requested
            .body
            .contains(apex_api::html::watch_html::WATCH_PAYMENT_LINK_FAILED),
        "{}",
        requested.body
    );
    assert!(!requested.body.contains("Complete the checkout"));
}

#[tokio::test]
async fn an_android_package_and_its_play_catalogue_reach_core() {
    let temp = TempDir::new("watch-play-tier");
    let (app, store, _mailer, transport) = build_app(temp.path(), true);
    let transport = transport.expect("checkout transport");
    transport.enable_play_listing();
    let reference = submit_watch(
        &app,
        "domain=play-tier.example&email=ops%40play-tier.example&apps=com.example.android",
    )
    .await;
    let row = store.get(&reference).expect("row").expect("subscriber");
    assert_eq!(
        row.verified_app_count,
        Some(11),
        "the Play catalogue is the tier count"
    );
    assert_eq!(row.tier, Tier::Core);
    assert!(
        transport.calls.lock().unwrap().iter().any(|call| call
            .url
            .starts_with("https://play.google.com/store/apps/dev")),
        "the catalogue request is wired through the API transport"
    );
}

#[tokio::test]
async fn only_the_verified_app_count_decides_the_tier() {
    let temp = TempDir::new("watch-verified-count");
    let (app, store, _mailer, transport) = build_app(temp.path(), true);
    let transport = transport.expect("checkout transport");

    // Ten submitted ids but only nine are real listings: the submitted
    // count is not trusted.
    transport.set_verified(&[
        "com.real.app1",
        "com.real.app2",
        "com.real.app3",
        "com.real.app4",
        "com.real.app5",
        "com.real.app6",
        "com.real.app7",
        "com.real.app8",
        "com.real.app9",
    ]);
    let submitted = (1..=10)
        .map(|index| format!("com.real.app{index}"))
        .collect::<Vec<_>>()
        .join("%0A");
    let reference = submit_watch(
        &app,
        &format!("domain=nine-real.example&email=ops%40nine-real.example&apps={submitted}"),
    )
    .await;
    let row = store.get(&reference).expect("row").expect("subscriber");
    assert_eq!(
        row.verified_app_count,
        Some(9),
        "the unconfirmed tenth id is not counted"
    );
    assert_eq!(row.tier, Tier::Normal);
    assert_eq!(
        transport.app_calls().len(),
        14,
        "every distinct submitted app plus four developer-catalogue lookups"
    );

    // Twelve submitted ids with ten verified is core.
    let verified: Vec<String> = (1..=10)
        .map(|index| format!("com.real.core{index}"))
        .collect();
    transport.set_verified(&verified.iter().map(String::as_str).collect::<Vec<&str>>());
    let mut submitted = verified.clone();
    submitted.push("com.real.core11".to_string());
    submitted.push("com.real.core12".to_string());
    let reference = submit_watch(
        &app,
        &format!(
            "domain=ten-real.example&email=ops%40ten-real.example&apps={}",
            submitted.join("%0A")
        ),
    )
    .await;
    let row = store.get(&reference).expect("row").expect("subscriber");
    assert_eq!(row.verified_app_count, Some(10));
    assert_eq!(row.tier, Tier::Core);

    // An Android-only package id the lookup cannot confirm stays standard.
    let reference = submit_watch(
        &app,
        "domain=android.example&email=ops%40android.example&apps=com.example.android.package",
    )
    .await;
    let row = store.get(&reference).expect("row").expect("subscriber");
    assert_eq!(row.verified_app_count, Some(0));
    assert_eq!(row.tier, Tier::Normal);
    assert!(
        row.apps_verification_error.is_none(),
        "a definite no-result answer is not a network error: {:?}",
        row.apps_verification_error
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
    let proof = HttpAppAdsProof::new(
        Arc::clone(&transport) as Arc<dyn apex_api::app_ads_proof::PinnedTransport>
    )
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
    // No Mollie checkout configured: the sign-up is still recorded pending,
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
        page.body.contains(
            "We will show your payment link here once your billing details are recorded."
        ),
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
        mailer.last_text().contains(
            "We will show your payment link here once your billing details are recorded."
        ),
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
    let confirmed = confirm_watch(&app, &token).await;
    assert_eq!(confirmed.status, 200);

    let body = watch_webhook(&reference, "psp-watch-1");
    let accepted = send(&app, post_form_bytes("/v1/mollie/webhook", &body)).await;
    assert_eq!(accepted.status, 200, "{}", accepted.body);
    assert_eq!(accepted.json()["new"], true);

    let rows = store.list().expect("list");
    assert_eq!(rows.len(), 1, "activation appends a row, never a draft");
    assert_eq!(rows[0].status, Status::Active);
    assert!(
        !rows[0].is_checkable(),
        "activation without a recorded billing profile must not make the row check-eligible"
    );
    assert!(rows[0].paid_at.is_some());
    assert_eq!(
        rows[0].stored_mandate.as_deref(),
        Some("cst_watch_1:mdt_watch_1"),
        "the stored card is recorded for the monthly bill"
    );
    let paid_at = rows[0].paid_at;
    let after_first = row_count(&store);

    // A replayed delivery is accepted by Mollie's own retry contract and is a
    // no-op for the subscriber: no second row, no second paid outcome.
    let replay = send(&app, post_form_bytes("/v1/mollie/webhook", &body)).await;
    assert_eq!(replay.status, 200, "{}", replay.body);
    assert_eq!(replay.json()["new"], false, "{}", replay.body);
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
    assert_eq!(confirm_watch(&app, &token).await.status, 200);
    assert_eq!(store.list().expect("list")[0].status, Status::Pending);

    // The harness posts the bare subscriber id (no `apex-watch:` prefix).
    let body = watch_webhook_with_reference(&reference, "tr_watch_bare");
    let accepted = send(&app, post_form_bytes("/v1/mollie/webhook", &body)).await;
    assert_eq!(accepted.status, 200, "{}", accepted.body);
    assert_eq!(accepted.json()["new"], true);
    let rows = store.list().expect("list");
    assert_eq!(rows[0].id, reference);
    assert_eq!(rows[0].status, Status::Active);
    assert_eq!(
        rows[0].stored_mandate.as_deref(),
        Some("cst_watch_1:mdt_watch_1")
    );
    let after_first = row_count(&store);

    let replay = send(&app, post_form_bytes("/v1/mollie/webhook", &body)).await;
    assert_eq!(replay.status, 200, "{}", replay.body);
    assert_eq!(replay.json()["new"], false);
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
    let confirmed = confirm_watch(app, &token).await;
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
            "/v1/mollie/webhook",
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
async fn a_self_reported_legacy_core_claim_is_not_charged_at_the_first_invoice() {
    let temp = TempDir::new("watch-legacy-core");
    let (app, store, _mailer, _transport) = build_app(temp.path(), true);
    // The legacy direct signup could self-report core and pay before any
    // profile or App Store verification existed.
    let row = store
        .subscribe(
            1_700_000_000_000,
            "legacy-core.example",
            "ops@legacy-core.example",
            "",
            true,
        )
        .expect("legacy subscribe")
        .subscriber;
    assert_eq!(row.tier, Tier::Core);
    let token = store
        .create_profile_token_at(1_700_000_000_000, &row.id)
        .expect("profile token");
    let response = send(
        &app,
        post_form(
            "/watch/billing-profile",
            &billing_profile_body(
                &token,
                &row.id,
                "Legacy OU",
                "Sepapaja tn 6",
                "EE",
                "EE123456789",
            ),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);

    let subscriber = store.get(&row.id).expect("row").expect("subscriber");
    assert_eq!(
        subscriber.tier,
        Tier::Normal,
        "a legacy core claim with no verified apps never keeps the core price"
    );
    assert_eq!(subscriber.tier_previous, Some(Tier::Core));
    assert_eq!(subscriber.first_invoice_number, Some(1));
    let reason = subscriber
        .apps_verification_error
        .expect("the honest not-re-verified state");
    assert!(reason.starts_with("not re-verified:"), "{reason}");

    let register =
        std::fs::read_to_string(temp.path().join("billing/invoices.jsonl")).expect("register");
    let invoice: serde_json::Value =
        serde_json::from_str(register.lines().next().expect("one line")).expect("invoice json");
    assert_eq!(
        invoice["lineItems"][0]["feeMicros"], 99_000_000,
        "the invoice uses the standard price"
    );
    assert_eq!(invoice["grossMicros"], 118_800_000);
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
    let confirmed = confirm_watch(&app, &token).await;
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
        mollie: None,
        transport: None,
        lookup: None,
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
    assert_eq!(location, format!("/watch/requested?ref={reference}"));

    let subscriber = store.get(&reference).expect("row").expect("subscriber");
    assert!(!subscriber.is_active(), "the watch is still pending");
    assert!(
        subscriber.billing_profile.is_some(),
        "the profile is stored"
    );
    assert!(
        subscriber.payment_link.is_some(),
        "the checkout link is created after the profile"
    );
    assert!(
        subscriber.first_invoice_number.is_none(),
        "no invoice before activation"
    );
    assert!(!temp.path().join("billing/invoices.jsonl").exists());
    assert!(!subscriber.is_checkable(), "still no accepted payment");

    let recorded = send(&app, get(&location)).await;
    assert_eq!(recorded.status, StatusCode::OK);
    assert!(
        recorded.body.contains("Complete the checkout"),
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

/// Cancellation is a real one-click surface: GET is inert, POST consumes the
/// token once, a one-click POST with only the query token works, and unknown
/// or spent links are 404 rather than a success page.
#[tokio::test]
async fn cancel_is_inert_on_get_and_single_use_on_post() {
    let dir = TempDir::new("watch-cancel");
    let (app, store, _mailer, _transport) = build_app(dir.path(), false);
    let reference = submit_watch(
        &app,
        "domain=accounts.example&email=owner%40accounts.example&apps=com.example.one&accounts=google.com%20%3D%20pub-1",
    )
    .await;
    let subscriber = store.get(&reference).unwrap().expect("subscriber");
    let (raw, hash) = SubscriberStore::mint_cancel_token();
    let mut updated = subscriber.clone();
    updated.cancel_token_hash = hash;
    store.update(&updated).unwrap();

    // Two GETs are inert; the subscriber is still pending.
    for _ in 0..2 {
        let page = send(&app, get(&format!("/watch/cancel?token={raw}"))).await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.body);
        assert!(
            page.body.contains("Cancel your setup watch?"),
            "{}",
            page.body
        );
        assert_eq!(
            store.get(&reference).unwrap().unwrap().status,
            Status::Pending
        );
    }

    // The POST consumes it once.
    let canceled = send(&app, post_form("/watch/cancel", &format!("token={raw}"))).await;
    assert_eq!(canceled.status, StatusCode::OK, "{}", canceled.body);
    assert!(canceled.body.contains("Your setup watch is cancelled."));
    assert_eq!(
        store.get(&reference).unwrap().unwrap().status,
        Status::Canceled
    );

    // Replay and unknown tokens are 404, never a success page.
    for token in [raw.as_str(), "ffffffffffffffffffffffffffffffff"] {
        let page = send(&app, get(&format!("/watch/cancel?token={token}"))).await;
        assert_eq!(page.status, StatusCode::NOT_FOUND);
        let posted = send(&app, post_form("/watch/cancel", &format!("token={token}"))).await;
        assert_eq!(posted.status, StatusCode::NOT_FOUND);
    }

    // RFC 8058 one-click: POST with the token in the query and an empty body.
    let second = submit_watch(
        &app,
        "domain=bigport.example&email=owner%40bigport.example&apps=com.example.two&accounts=google.com%20%3D%20pub-2",
    )
    .await;
    let subscriber = store.get(&second).unwrap().expect("subscriber");
    let (raw_two, hash_two) = SubscriberStore::mint_cancel_token();
    let mut updated = subscriber;
    updated.cancel_token_hash = hash_two;
    store.update(&updated).unwrap();
    let one_click = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("/watch/cancel?token={raw_two}"))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(""))
            .unwrap(),
    )
    .await;
    assert_eq!(one_click.status, StatusCode::OK, "{}", one_click.body);
    assert_eq!(
        store.get(&second).unwrap().unwrap().status,
        Status::Canceled
    );
}

/// A filtered-out reference never renders a success-style page.
#[tokio::test]
async fn unknown_watch_references_are_honest_404s() {
    let dir = TempDir::new("watch-invalid-ref");
    let (app, _store, _mailer, _transport) = build_app(dir.path(), false);
    for path in [
        "/watch/requested?ref=sub_ffffffffffffffffffffffffffffffff",
        "/watch/billing-profile/recorded?ref=sub_ffffffffffffffffffffffffffffffff",
        "/watch/requested",
    ] {
        let page = send(&app, get(path)).await;
        assert_eq!(page.status, StatusCode::NOT_FOUND, "{path}: {}", page.body);
        assert!(
            page.body.contains("This link is not valid or has expired."),
            "{}",
            page.body
        );
    }
}

#[tokio::test]
async fn refund_and_chargeback_events_are_recorded_but_never_activate() {
    let temp = TempDir::new("watch-refund-order");
    let (app, store, _mailer, _transport) = build_app(temp.path(), true);
    let reference = submit_watch(
        &app,
        "domain=refund.example&email=ops%40refund.example&apps=",
    )
    .await;
    let merchant = format!("apex-watch:{reference}");
    let rows_after_signup = row_count(&store);

    // A validly signed REFUND arriving before the authorisation is accepted
    // (Mollie retries out of order) but marks nothing paid.
    let refund = signed_notification(
        &merchant,
        "psp-refund-before-auth",
        "REFUND",
        "true",
        9_900,
        "USD",
        "ApexMerchant",
    );
    let response = send(&app, post_form_bytes("/v1/mollie/webhook", &refund)).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json()["new"], true);
    assert_eq!(
        store.get(&reference).unwrap().unwrap().status,
        Status::Pending,
        "a refund is not a payment authorisation"
    );

    let chargeback = signed_notification(
        &merchant,
        "psp-chargeback-before-auth",
        "CHARGEBACK",
        "true",
        9_900,
        "USD",
        "ApexMerchant",
    );
    let response = send(&app, post_form_bytes("/v1/mollie/webhook", &chargeback)).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json()["new"], true);
    assert_eq!(
        store.get(&reference).unwrap().unwrap().status,
        Status::Pending,
        "a chargeback is not a payment authorisation"
    );
    assert_eq!(row_count(&store), rows_after_signup, "no activation row");

    // The authorisation that arrives later is the only event that activates.
    let auth = signed_notification(
        &merchant,
        "psp-auth-after-funds-events",
        "AUTHORISATION",
        "true",
        9_900,
        "USD",
        "ApexMerchant",
    );
    let response = send(&app, post_form_bytes("/v1/mollie/webhook", &auth)).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(
        store.get(&reference).unwrap().unwrap().status,
        Status::Active
    );
    let rows_after_activation = row_count(&store);
    assert!(
        rows_after_activation > rows_after_signup,
        "activation appends the active row"
    );

    // Replayed funds events and a replayed authorisation change nothing.
    for body in [&refund, &chargeback, &auth] {
        let response = send(&app, post_form_bytes("/v1/mollie/webhook", body)).await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(
            response.json()["new"],
            false,
            "duplicate delivery is a no-op"
        );
    }
    assert_eq!(
        store.get(&reference).unwrap().unwrap().status,
        Status::Active
    );
    assert_eq!(row_count(&store), rows_after_activation, "no second row");
}

#[tokio::test]
async fn authorisation_amount_and_currency_must_match_the_tier() {
    let temp = TempDir::new("watch-amount");
    let (app, store, _mailer, _transport) = build_app(temp.path(), true);
    let reference = submit_watch(
        &app,
        "domain=amount.example&email=ops%40amount.example&apps=",
    )
    .await;
    let merchant = format!("apex-watch:{reference}");

    for (psp, amount, currency) in [
        ("psp-underpaid", 100, "USD"),
        ("psp-zero", 0, "USD"),
        ("psp-wrong-currency", 9_900, "EUR"),
    ] {
        let body = signed_notification(
            &merchant,
            psp,
            "AUTHORISATION",
            "true",
            amount,
            currency,
            "ApexMerchant",
        );
        let response = send(&app, post_form_bytes("/v1/mollie/webhook", &body)).await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{}",
            response.body
        );
        assert_eq!(response.json()["error"], "invalid_payment");
        assert_eq!(
            store.get(&reference).unwrap().unwrap().status,
            Status::Pending,
            "amount {amount} {currency} is not the $99 tier price"
        );
    }

    let correct = signed_notification(
        &merchant,
        "psp-correct",
        "AUTHORISATION",
        "true",
        9_900,
        "USD",
        "ApexMerchant",
    );
    let response = send(&app, post_form_bytes("/v1/mollie/webhook", &correct)).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(
        store.get(&reference).unwrap().unwrap().status,
        Status::Active
    );
}

#[tokio::test]
async fn oversized_webhook_bodies_are_json_413() {
    let temp = TempDir::new("watch-oversized");
    let (app, _store, _mailer, _transport) = build_app(temp.path(), true);
    let body = vec![b'x'; apex_api::MAX_BODY_BYTES + 1];
    let response = send(&app, post_form_bytes("/v1/mollie/webhook", &body)).await;
    assert_eq!(response.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(response.json(), serde_json::json!({"error": "bad_request"}));
}
