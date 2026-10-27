//! Adversarial suite for the Mollie billing surface: request/response
//! shaping, the unsigned webhook fetch boundary, and the invoice arithmetic.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use apex_connectors::transport::{
    HttpBody, HttpMethod, HttpRequest, HttpResponse, RecordingSleeper, RetryTransport, Sleeper,
    Transport, TransportError,
};
use async_trait::async_trait;

use crate::collection::{CollectionEvent, CollectionStore, FileCollectionStore};
use crate::errors::BillingError;
use crate::invoice::{
    compute_invoice, BillingBasisRecord, BillingPeriod, BillingProofRecord, Invoice, InvoiceLine,
};
use crate::mollie::{
    collect_invoice, collect_line, create_payment_links, format_amount, parse_amount,
    BillingReceipt, CollectionChannel, InvoiceLineItem, MollieClient, MollieClientError,
    MolliePayment, PaymentLinkResponse, StoredMandate, BILLING_CURRENCY, MAX_REFERENCE_LEN,
};
use crate::profile::BillingProfile;
use crate::vat::VatTreatment;
use crate::webhook::{parse_webhook_id, validate_payment};
use apex_proof::{ChangeKind, Keypair, Ledger, LedgerRecord};

const KEY: &str = "test_mollie_key";
const CUSTOMER_ID: &str = "cst_9Qx1AbCdEf";
const MANDATE_ID: &str = "mdt_7Yz2GhIjKl";
const PAYMENT_ID: &str = "tr_5Kp3MnOpQr";

struct MockTransport {
    script: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
    calls: Mutex<Vec<HttpRequest>>,
}

impl MockTransport {
    fn new(items: Vec<Result<HttpResponse, TransportError>>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(items.into()),
            calls: Mutex::new(vec![]),
        })
    }

    fn ok(status: u16, body: Option<&str>) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![],
            body: body.map(|b| b.as_bytes().to_vec()).unwrap_or_default(),
        }
    }

    fn calls(&self) -> Vec<HttpRequest> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        self.calls
            .lock()
            .map_err(|_| TransportError::Network("poisoned".into()))?
            .push(req);
        self.script
            .lock()
            .map_err(|_| TransportError::Network("poisoned".into()))?
            .pop_front()
            .unwrap_or(Err(TransportError::Network("script exhausted".into())))
    }
}

fn client() -> MollieClient {
    MollieClient::new(
        KEY,
        "https://api.mollie.test",
        "https://apex.test/v1/mollie/webhook",
        "https://apex.test/watch/return",
    )
    .expect("client")
}

fn link_response(id: &str) -> HttpResponse {
    MockTransport::ok(
        201,
        Some(&format!(
            r#"{{"resource":"payment","id":"{id}","status":"open","amount":{{"currency":"USD","value":"1.00"}},"metadata":{{"reference":"apex-fee:pub-1:exp-a:2026-10"}},"_links":{{"checkout":{{"href":"https://www.mollie.test/checkout/{id}","type":"text/html"}}}}}}"#
        )),
    )
}

fn customer_response() -> HttpResponse {
    MockTransport::ok(
        201,
        Some(r#"{"resource":"customer","id":"cst_9Qx1AbCdEf"}"#),
    )
}

fn first_payment_response() -> HttpResponse {
    MockTransport::ok(
        201,
        Some(&format!(
            r#"{{"resource":"payment","id":"{PAYMENT_ID}","status":"open","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"apex-fee:pub-1:exp-a:2026-10","publisher":"publisher-pub-1","amount":"1.25"}},"customerId":"{CUSTOMER_ID}","sequenceType":"first","_links":{{"checkout":{{"href":"https://www.mollie.test/checkout/{PAYMENT_ID}","type":"text/html"}}}}}}"#
        )),
    )
}

fn paid_response(reference: &str, amount: &str, customer: bool) -> HttpResponse {
    MockTransport::ok(
        200,
        Some(&format!(
            r#"{{"resource":"payment","id":"{PAYMENT_ID}","status":"paid","amount":{{"currency":"USD","value":"{amount}"}},"metadata":{{"reference":"{reference}","publisher":"publisher-pub-1","amount":"{amount}"}}{},"sequenceType":"recurring"}}"#,
            if customer {
                format!(r#","customerId":"{CUSTOMER_ID}","mandateId":"{MANDATE_ID}""#)
            } else {
                String::new()
            }
        )),
    )
}

fn refused_response(reference: &str, amount: &str) -> HttpResponse {
    MockTransport::ok(
        200,
        Some(&format!(
            r#"{{"resource":"payment","id":"{PAYMENT_ID}","status":"failed","amount":{{"currency":"USD","value":"{amount}"}},"metadata":{{"reference":"{reference}","publisher":"publisher-pub-1","amount":"{amount}"}},"customerId":"{CUSTOMER_ID}","mandateId":"{MANDATE_ID}","sequenceType":"recurring"}}"#
        )),
    )
}

fn error_response(status: u16) -> HttpResponse {
    MockTransport::ok(
        status,
        Some(r#"{"status":422,"title":"Unprocessable Entity"}"#),
    )
}

fn invoice_with(lines: Vec<InvoiceLine>) -> Invoice {
    let total_micros: i64 = lines.iter().map(|l| l.fee_micros).sum();
    Invoice {
        number: 0,
        publisher_id: "pub-1".to_string(),
        period: BillingPeriod {
            year_month: "2026-10".to_string(),
            start_unix_ms: 0,
            end_unix_ms: 1,
        },
        total_micros,
        line_items: lines,
        vat_rate_bp: 0,
        vat_micros: 0,
        gross_micros: total_micros,
        customer: None,
        vat_treatment: VatTreatment::NotApplied,
        vies_evidence: None,
    }
}

fn invoice_line(seq: u64, experiment: &str, fee_micros: i64) -> InvoiceLine {
    InvoiceLine {
        seq,
        experiment_id: experiment.to_string(),
        proven_lift: fee_micros * 2,
        rate_bp: 1_500,
        fee_micros,
        vat_micros: 0,
        gross_micros: fee_micros,
    }
}

fn retried(mock: Arc<MockTransport>) -> Arc<dyn Transport> {
    let sleeper: Arc<dyn Sleeper> = Arc::new(RecordingSleeper::default());
    Arc::new(RetryTransport::new(mock, 2, sleeper))
}

// ---------------------------------------------------------------------------
// Request shaping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn payment_links_post_one_idempotent_mollie_link_per_line() {
    let mock = MockTransport::new(vec![Ok(link_response("pl_1")), Ok(link_response("pl_2"))]);
    let transport = retried(mock.clone());
    let invoice = invoice_with(vec![
        invoice_line(1, "exp-a", 1_250_000),
        invoice_line(2, "exp-b", 2_000_000),
    ]);
    let receipt = create_payment_links(&client(), transport.as_ref(), &invoice)
        .await
        .expect("links");
    assert_eq!(receipt.links.len(), 2);
    assert_eq!(receipt.total_minor_units, 325);
    assert_eq!(receipt.year_month, "2026-10");
    assert_eq!(receipt.currency, "USD");
    let calls = mock.calls();
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert_eq!(call.method, HttpMethod::Post);
        assert_eq!(call.url, "https://api.mollie.test/v2/payments");
        assert!(call
            .headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer test_mollie_key"));
        let body = match call.body.as_ref().expect("body") {
            HttpBody::Json(value) => value.clone(),
            other => panic!("unexpected body {other:?}"),
        };
        let reference = body["metadata"]["reference"].as_str().unwrap_or_default();
        assert!(call
            .headers
            .iter()
            .any(|(k, v)| k == "Idempotency-Key" && v == reference));
        // Mollie's amount is a decimal string, never an integer.
        assert!(body["amount"]["value"].is_string());
        assert_eq!(body["amount"]["currency"], "USD");
        assert!(body["description"]
            .as_str()
            .unwrap_or_default()
            .contains("Apex proven-lift fee"));
        assert_eq!(body["redirectUrl"], "https://apex.test/watch/return");
        assert_eq!(body["webhookUrl"], "https://apex.test/v1/mollie/webhook");
        assert_eq!(body["metadata"]["publisher"], "publisher-pub-1");
    }
    let first = match calls[0].body.as_ref().expect("body") {
        HttpBody::Json(value) => value.clone(),
        other => panic!("unexpected body {other:?}"),
    };
    assert_eq!(
        first["metadata"]["reference"],
        "apex-fee:pub-1:exp-a:2026-10"
    );
    assert_eq!(first["metadata"]["amount"], "1.25");
    assert_eq!(
        receipt.links[0].url,
        "https://www.mollie.test/checkout/pl_1"
    );
}

#[tokio::test]
async fn the_first_payment_creates_a_customer_and_uses_sequence_first() {
    let mock = MockTransport::new(vec![Ok(customer_response()), Ok(first_payment_response())]);
    let transport = retried(mock.clone());
    let line = invoice_line(1, "exp-a", 1_250_000);
    let item = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("item");
    let link = client()
        .create_first_payment(transport.as_ref(), &item)
        .await
        .expect("first payment");
    assert_eq!(link.id, PAYMENT_ID);
    assert_eq!(link.customer_id.as_deref(), Some(CUSTOMER_ID));
    assert!(link.url.starts_with("https://www.mollie.test/checkout/"));

    let calls = mock.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].url, "https://api.mollie.test/v2/customers");
    assert_eq!(calls[0].operation, "createCustomer");
    assert_eq!(calls[1].url, "https://api.mollie.test/v2/payments");
    assert_eq!(calls[1].operation, "createFirstPayment");
    let body = match calls[1].body.as_ref().expect("body") {
        HttpBody::Json(value) => value.clone(),
        other => panic!("unexpected body {other:?}"),
    };
    assert_eq!(body["customerId"], CUSTOMER_ID);
    assert_eq!(body["sequenceType"], "first");
    assert_eq!(body["amount"]["value"], "1.25");
    assert_eq!(
        body["metadata"]["reference"],
        "apex-fee:pub-1:exp-a:2026-10"
    );
}

#[tokio::test]
async fn recurring_charge_uses_customer_and_mandate_and_sequence_recurring() {
    let mock = MockTransport::new(vec![Ok(paid_response(
        "apex-fee:pub-1:exp-a:2026-10",
        "1.25",
        true,
    ))]);
    let transport = retried(mock.clone());
    let line = invoice_line(1, "exp-a", 1_250_000);
    let item = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("item");
    let receipt = client()
        .charge_stored_payment_method(transport.as_ref(), &item, CUSTOMER_ID, MANDATE_ID)
        .await
        .expect("charge");
    assert_eq!(receipt.payment_id, PAYMENT_ID);
    assert_eq!(receipt.status, "paid");
    assert_eq!(receipt.amount_minor_units, 125);
    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, "https://api.mollie.test/v2/payments");
    let body = match calls[0].body.as_ref().expect("body") {
        HttpBody::Json(value) => value.clone(),
        other => panic!("unexpected body {other:?}"),
    };
    assert_eq!(body["customerId"], CUSTOMER_ID);
    assert_eq!(body["mandateId"], MANDATE_ID);
    assert_eq!(body["sequenceType"], "recurring");
    assert_eq!(body["amount"]["value"], "1.25");
}

#[tokio::test]
async fn a_non_paid_recurring_status_is_a_typed_refusal() {
    let mock = MockTransport::new(vec![Ok(refused_response(
        "apex-fee:pub-1:exp-a:2026-10",
        "1.25",
    ))]);
    let transport = retried(mock.clone());
    let line = invoice_line(1, "exp-a", 1_250_000);
    let item = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("item");
    let error = client()
        .charge_stored_payment_method(transport.as_ref(), &item, CUSTOMER_ID, MANDATE_ID)
        .await
        .expect_err("failed status");
    assert!(
        matches!(error, BillingError::StoredChargeRefused { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn zero_invoice_sends_no_request_at_all() {
    let mock = MockTransport::new(vec![]);
    let transport = retried(mock.clone());
    let receipt = create_payment_links(&client(), transport.as_ref(), &invoice_with(vec![]))
        .await
        .expect("empty");
    assert!(receipt.links.is_empty());
    assert_eq!(receipt.total_minor_units, 0);
    assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn zero_fee_lines_are_skipped_not_billed() {
    let mock = MockTransport::new(vec![Ok(link_response("pl_1"))]);
    let transport = retried(mock.clone());
    let invoice = invoice_with(vec![
        invoice_line(1, "exp-a", 1_000_000),
        invoice_line(2, "exp-zero", 0),
    ]);
    let receipt = create_payment_links(&client(), transport.as_ref(), &invoice)
        .await
        .expect("mixed");
    assert_eq!(receipt.links.len(), 1);
    assert_eq!(mock.calls().len(), 1);
}

#[tokio::test]
async fn sub_cent_fees_refuse_before_any_request() {
    let mock = MockTransport::new(vec![]);
    let transport = retried(mock.clone());
    let invoice = invoice_with(vec![invoice_line(1, "exp-a", 1_250_001)]);
    assert_eq!(
        create_payment_links(&client(), transport.as_ref(), &invoice).await,
        Err(BillingError::SubCentAmount { seq: 1 })
    );
    assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn unsafe_references_are_refused_before_any_request() {
    let mock = MockTransport::new(vec![]);
    let transport = retried(mock.clone());
    let mut line = invoice_line(1, "exp-a", 1_000_000);
    line.experiment_id = "x\r\n".to_string();
    assert!(matches!(
        create_payment_links(
            &client(),
            transport.as_ref(),
            &invoice_with(vec![line.clone()])
        )
        .await,
        Err(BillingError::UnsafeIdentifier { .. })
    ));
    let mut long = invoice_line(2, "exp-a", 1_000_000);
    long.experiment_id = "e".repeat(MAX_REFERENCE_LEN);
    assert!(matches!(
        create_payment_links(&client(), transport.as_ref(), &invoice_with(vec![long])).await,
        Err(BillingError::UnsafeIdentifier { .. })
    ));
}

#[tokio::test]
async fn auth_and_http_failures_are_typed() {
    let mock = MockTransport::new(vec![Ok(error_response(401))]);
    let transport = retried(mock.clone());
    let invoice = invoice_with(vec![invoice_line(1, "exp-a", 1_000_000)]);
    assert_eq!(
        create_payment_links(&client(), transport.as_ref(), &invoice).await,
        Err(BillingError::Auth { status: 401 })
    );
    let mock = MockTransport::new(vec![Ok(error_response(422))]);
    let transport = retried(mock.clone());
    assert_eq!(
        create_payment_links(&client(), transport.as_ref(), &invoice).await,
        Err(BillingError::Http { status: 422 })
    );
}

#[tokio::test]
async fn malformed_success_bodies_fail_closed() {
    for body in [
        Some(r#"{"id":"pl_1"}"#),
        Some(r#"{"_links":{"checkout":{"href":"https://x"}}}"#),
        Some(r#"{"id":"pl_1","_links":{}}"#),
        None,
    ] {
        let mock = MockTransport::new(vec![Ok(MockTransport::ok(201, body))]);
        let transport = retried(mock.clone());
        let invoice = invoice_with(vec![invoice_line(1, "exp-a", 1_000_000)]);
        assert!(matches!(
            create_payment_links(&client(), transport.as_ref(), &invoice).await,
            Err(BillingError::Malformed(_))
        ));
    }
}

#[tokio::test]
async fn malformed_line_items_are_refused_before_any_request() {
    let line = invoice_line(1, "exp-a", 1_250_000);
    let valid = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("item");
    let cases = [
        InvoiceLineItem {
            amount_minor_units: 0,
            ..valid.clone()
        },
        InvoiceLineItem {
            amount_minor_units: -1,
            ..valid.clone()
        },
        InvoiceLineItem {
            currency: "usd".to_string(),
            ..valid.clone()
        },
        InvoiceLineItem {
            currency: "US".to_string(),
            ..valid.clone()
        },
        InvoiceLineItem {
            reference: String::new(),
            ..valid.clone()
        },
        InvoiceLineItem {
            reference: "x".repeat(MAX_REFERENCE_LEN + 1),
            ..valid.clone()
        },
        InvoiceLineItem {
            reference: "apex\u{0}fee".to_string(),
            ..valid.clone()
        },
        InvoiceLineItem {
            customer_reference: "  ".to_string(),
            ..valid.clone()
        },
    ];
    for item in cases {
        let mock = MockTransport::new(vec![]);
        let transport = retried(mock.clone());
        assert!(
            matches!(
                client()
                    .create_payment_link(transport.as_ref(), &item)
                    .await,
                Err(BillingError::Malformed(_))
            ),
            "item must be refused locally: {item:?}"
        );
        assert!(mock.calls().is_empty(), "no request for {item:?}");
    }
}

#[test]
fn line_items_convert_only_exact_cents() {
    let line = invoice_line(1, "exp-a", 1_250_000);
    let item = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("exact");
    assert_eq!(item.amount_minor_units, 125);
    assert_eq!(item.reference, "apex-fee:pub-1:exp-a:2026-10");
    assert_eq!(item.customer_reference, "publisher-pub-1");
    assert_eq!(item.currency, BILLING_CURRENCY);
}

#[test]
fn amount_formatting_and_parsing_are_byte_exact() {
    assert_eq!(format_amount(0).unwrap(), "0.00");
    assert_eq!(format_amount(5).unwrap(), "0.05");
    assert_eq!(format_amount(125).unwrap(), "1.25");
    assert_eq!(format_amount(123_456).unwrap(), "1234.56");
    assert!(format_amount(-1).is_err());
    assert_eq!(parse_amount("0.05").unwrap(), 5);
    assert_eq!(parse_amount("1234.56").unwrap(), 123_456);
    assert_eq!(
        parse_amount("123456789012345.00").unwrap(),
        12_345_678_901_234_500
    );
    for bad in [
        "1",
        "1.2",
        "1.234",
        "-1.00",
        "+1.00",
        "1e2",
        "1,00",
        "",
        ".00",
        "1234567890123456.00",
        "1a.00",
        "1.2a",
    ] {
        assert!(parse_amount(bad).is_err(), "{bad:?} must be refused");
    }
}

#[test]
fn receipt_serializes_camel_case() {
    let receipt = BillingReceipt {
        publisher_id: "pub-1".into(),
        year_month: "2026-10".into(),
        currency: "USD".into(),
        total_minor_units: 125,
        links: vec![PaymentLinkResponse {
            id: "pl_1".into(),
            url: "https://x".into(),
            customer_id: None,
        }],
    };
    let json = serde_json::to_string(&receipt).expect("json");
    assert!(json.contains("totalMinorUnits"), "{json}");
    assert!(json.contains("pl_1"), "{json}");
}

#[test]
fn client_refuses_empty_credentials_and_redacts_the_key() {
    assert_eq!(
        MollieClient::new("", "https://api.mollie.com", "w", "r").unwrap_err(),
        MollieClientError::EmptyApiKey
    );
    assert_eq!(
        MollieClient::new("k", "", "w", "r").unwrap_err(),
        MollieClientError::EmptyApiBase
    );
    assert_eq!(
        MollieClient::new("k", "api.mollie.com", "w", "r").unwrap_err(),
        MollieClientError::EmptyApiBase
    );
    assert_eq!(
        MollieClient::new("k", "https://api.mollie.com", "", "r").unwrap_err(),
        MollieClientError::EmptyWebhookUrl
    );
    assert_eq!(
        MollieClient::new("k", "https://api.mollie.com", "w", " ").unwrap_err(),
        MollieClientError::EmptyRedirectUrl
    );
    let debug = format!("{:?}", client());
    assert!(!debug.contains(KEY), "{debug}");
    assert!(debug.contains("api.mollie.test"), "{debug}");
}

#[test]
fn stored_mandates_roundtrip_and_refuse_ambiguous_values() {
    let mandate = StoredMandate::new(CUSTOMER_ID, MANDATE_ID).expect("mandate");
    assert_eq!(mandate.encode(), format!("{CUSTOMER_ID}:{MANDATE_ID}"));
    assert_eq!(StoredMandate::parse(&mandate.encode()), Some(mandate));
    assert_eq!(StoredMandate::parse("only-customer"), None);
    assert_eq!(StoredMandate::parse(":mandate"), None);
    assert_eq!(StoredMandate::new("a:b", "m"), None);
}

#[tokio::test]
async fn an_accepted_recurring_charge_needs_no_link() {
    let mock = MockTransport::new(vec![Ok(paid_response(
        "apex-fee:pub-1:exp-a:2026-10",
        "1.25",
        false,
    ))]);
    let transport = retried(mock.clone());
    let line = invoice_line(1, "exp-a", 1_250_000);
    let item = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("item");
    let encoded = StoredMandate::new(CUSTOMER_ID, MANDATE_ID)
        .unwrap()
        .encode();
    let collected = collect_line(&client(), transport.as_ref(), &item, Some(&encoded))
        .await
        .expect("collected");
    assert_eq!(collected.channel, CollectionChannel::StoredMandate);
    assert_eq!(collected.payment_id.as_deref(), Some(PAYMENT_ID));
    assert_eq!(collected.status.as_deref(), Some("paid"));
    assert!(collected.payment_link.is_none());
    assert_eq!(mock.calls().len(), 1);
}

#[tokio::test]
async fn a_refused_recurring_charge_falls_back_to_a_link_with_the_reason() {
    let mock = MockTransport::new(vec![
        Ok(refused_response("apex-fee:pub-1:exp-a:2026-10", "1.25")),
        Ok(link_response("pl_9")),
    ]);
    let transport = retried(mock.clone());
    let line = invoice_line(1, "exp-a", 1_250_000);
    let item = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("item");
    let encoded = StoredMandate::new(CUSTOMER_ID, MANDATE_ID)
        .unwrap()
        .encode();
    let collected = collect_line(&client(), transport.as_ref(), &item, Some(&encoded))
        .await
        .expect("fallback");
    assert_eq!(collected.channel, CollectionChannel::PaymentLink);
    assert_eq!(collected.payment_link.as_ref().expect("link").id, "pl_9");
    assert!(collected
        .fallback_reason
        .as_deref()
        .unwrap_or_default()
        .contains("status failed"));
    assert_eq!(mock.calls().len(), 2);
    assert_eq!(mock.calls()[0].operation, "chargeStoredPaymentMethod");
    assert_eq!(mock.calls()[1].operation, "createCheckoutPayment");
}

#[tokio::test]
async fn a_missing_mandate_creates_a_link_with_the_typed_reason() {
    let mock = MockTransport::new(vec![
        Ok(link_response("pl_1")),
        Ok(link_response("pl_2")),
        Ok(link_response("pl_3")),
        Ok(link_response("pl_4")),
    ]);
    let transport = retried(mock.clone());
    let line = invoice_line(1, "exp-a", 1_250_000);
    let item = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("item");
    for absent in [None, Some(""), Some("   "), Some("garbage")] {
        let collected = collect_line(&client(), transport.as_ref(), &item, absent)
            .await
            .expect("link");
        assert_eq!(collected.channel, CollectionChannel::PaymentLink);
        assert_eq!(
            collected.fallback_reason.as_deref(),
            Some("no stored payment mandate is recorded")
        );
    }
    assert_eq!(mock.calls().len(), 4, "one link per attempt, no charge");
}

#[tokio::test]
async fn uncertain_charge_failures_never_fall_back() {
    let mock = MockTransport::new(vec![Err(TransportError::Network("dead".into()))]);
    let transport = retried(mock.clone());
    let line = invoice_line(1, "exp-a", 1_250_000);
    let item = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("item");
    let encoded = StoredMandate::new(CUSTOMER_ID, MANDATE_ID)
        .unwrap()
        .encode();
    let error = collect_line(&client(), transport.as_ref(), &item, Some(&encoded))
        .await
        .expect_err("typed");
    assert!(matches!(error, BillingError::Network { .. }), "{error:?}");
    assert_eq!(mock.calls().len(), 3, "retries only, never a link");
}

#[tokio::test]
async fn collect_invoice_charges_every_line_on_the_recorded_mandate() {
    let mock = MockTransport::new(vec![
        Ok(paid_response("apex-fee:pub-1:exp-a:2026-10", "1.25", true)),
        Ok(paid_response("apex-fee:pub-1:exp-b:2026-10", "2.00", true)),
    ]);
    let transport = retried(mock.clone());
    let invoice = invoice_with(vec![
        invoice_line(1, "exp-a", 1_250_000),
        invoice_line(2, "exp-b", 2_000_000),
    ]);
    let encoded = StoredMandate::new(CUSTOMER_ID, MANDATE_ID)
        .unwrap()
        .encode();
    let receipt = collect_invoice(&client(), transport.as_ref(), &invoice, Some(&encoded))
        .await
        .expect("receipt");
    assert_eq!(receipt.total_minor_units, 325);
    assert_eq!(receipt.lines.len(), 2);
    assert!(receipt
        .lines
        .iter()
        .all(|line| line.channel == CollectionChannel::StoredMandate));
    assert!(receipt.link_urls().is_empty());
    assert!(!receipt.has_links());
    assert_eq!(mock.calls().len(), 2);
}

#[tokio::test]
async fn billing_calls_never_carry_the_admob_label() {
    use apex_core::PlatformId;
    let mock = MockTransport::new(vec![Ok(link_response("pl_1"))]);
    let transport = retried(mock.clone());
    let invoice = invoice_with(vec![invoice_line(1, "exp-a", 1_250_000)]);
    create_payment_links(&client(), transport.as_ref(), &invoice)
        .await
        .expect("links");
    for call in mock.calls() {
        assert_eq!(call.platform, PlatformId::Billing);
        assert_ne!(call.platform, PlatformId::Admob);
    }
}

// ---------------------------------------------------------------------------
// Unsigned webhook boundary: parse id, fetch server-side, validate.
// ---------------------------------------------------------------------------

#[test]
fn webhook_form_parser_accepts_only_one_payment_id() {
    assert_eq!(
        parse_webhook_id(format!("id={PAYMENT_ID}").as_bytes()).unwrap(),
        PAYMENT_ID
    );
    assert_eq!(
        parse_webhook_id(format!("id={}&other=x", PAYMENT_ID).as_bytes()).unwrap(),
        PAYMENT_ID
    );
    for bad in [
        b"".as_slice(),
        b"id=",
        b"id=not-a-payment",
        b"id=pl_not_a_payment",
        format!("id={PAYMENT_ID}&id={PAYMENT_ID}").as_bytes(),
        b"amount=100.00&status=paid",
        b"id=%00",
    ] {
        assert!(parse_webhook_id(bad).is_err(), "{bad:?}");
    }
}

#[tokio::test]
async fn webhook_fetches_the_payment_instead_of_trusting_the_body() {
    let real = MolliePayment::from_json(
        &serde_json::from_str::<serde_json::Value>(&format!(
            r#"{{"id":"{PAYMENT_ID}","status":"paid","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"apex-fee:pub-1:exp-a:2026-10","publisher":"publisher-pub-1","amount":"1.25"}},"customerId":"{CUSTOMER_ID}","mandateId":"{MANDATE_ID}"}}"#
        ))
        .unwrap(),
    )
    .expect("payment");
    let body = format!("id={PAYMENT_ID}&status=paid&amount=0.01");
    let mock = MockTransport::new(vec![Ok(MockTransport::ok(
        200,
        Some(&serde_json::to_string(&serde_json::json!({
            "id": real.id,
            "status": real.status,
            "amount": {"currency": real.currency, "value": "1.25"},
            "metadata": {"reference": real.metadata_reference, "publisher": real.metadata_publisher, "amount": "1.25"},
            "customerId": real.customer_id,
            "mandateId": real.mandate_id,
        }))
        .unwrap()),
    ))]);
    let transport = retried(mock.clone());
    let fetched =
        crate::webhook::fetch_webhook_payment(&client(), transport.as_ref(), body.as_bytes())
            .await
            .expect("fetched");
    assert_eq!(fetched.status, "paid");
    assert_eq!(fetched.amount_minor_units, 125);
    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, HttpMethod::Get);
    assert_eq!(
        calls[0].url,
        format!("https://api.mollie.test/v2/payments/{PAYMENT_ID}")
    );
    assert!(calls[0].body.is_none(), "the POST body is never forwarded");
    // A forged body claiming 0.01 cannot change the fetched amount.
    assert_ne!(fetched.amount_minor_units, 1);
}

#[tokio::test]
async fn unknown_and_forged_webhook_ids_are_rejected_by_the_fetch() {
    let body = format!("id={PAYMENT_ID}");
    let mock = MockTransport::new(vec![Ok(error_response(404))]);
    let transport = retried(mock.clone());
    let error =
        crate::webhook::fetch_webhook_payment(&client(), transport.as_ref(), body.as_bytes())
            .await
            .expect_err("unknown payment");
    assert_eq!(error, BillingError::Http { status: 404 });

    // A fetched payment whose id does not match the posted id is refused.
    let mock = MockTransport::new(vec![Ok(MockTransport::ok(
        200,
        Some(
            r#"{"id":"tr_OtherPayment","status":"paid","amount":{"currency":"USD","value":"1.25"},"metadata":{"reference":"apex-fee:pub-1:exp-a:2026-10"}}"#,
        ),
    ))]);
    let transport = retried(mock.clone());
    let error =
        crate::webhook::fetch_webhook_payment(&client(), transport.as_ref(), body.as_bytes())
            .await
            .expect_err("id mismatch");
    assert!(
        matches!(
            error,
            BillingError::Malformed(_) | BillingError::WebhookRejected { .. }
        ),
        "{error:?}"
    );
}

#[test]
fn validation_rejects_forged_underpaid_and_mismatched_payments() {
    let good = MolliePayment::from_json(
        &serde_json::from_str::<serde_json::Value>(&format!(
            r#"{{"id":"{PAYMENT_ID}","status":"paid","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"apex-fee:pub-1:exp-a:2026-10","amount":"1.25"}}}}"#
        ))
        .unwrap(),
    )
    .expect("payment");
    assert!(validate_payment(&good, Some("apex-fee:pub-1:exp-a:2026-10"), Some(125)).is_ok());

    // Underpaid: the fetched amount (or its metadata) is short.
    let underpaid = PaymentForTest::parse(
        r#"{"id":"tr_5Kp3MnOpQr","status":"paid","amount":{"currency":"USD","value":"1.00"},"metadata":{"reference":"apex-fee:pub-1:exp-a:2026-10","amount":"1.00"}}"#,
    );
    assert!(matches!(
        validate_payment(&underpaid, Some("apex-fee:pub-1:exp-a:2026-10"), Some(125)),
        Err(BillingError::WebhookRejected { .. })
    ));
    // A payment whose metadata disagrees with its amount is refused even
    // without a caller expectation.
    let inconsistent = PaymentForTest::parse(
        r#"{"id":"tr_5Kp3MnOpQr","status":"paid","amount":{"currency":"USD","value":"1.25"},"metadata":{"reference":"apex-fee:pub-1:exp-a:2026-10","amount":"100.00"}}"#,
    );
    assert!(matches!(
        validate_payment(&inconsistent, None, None),
        Err(BillingError::WebhookRejected { .. })
    ));
    // Wrong reference / missing metadata.
    assert!(matches!(
        validate_payment(&good, Some("apex-fee:pub-9:exp-a:2026-10"), None),
        Err(BillingError::WebhookRejected { .. })
    ));
    let no_reference = PaymentForTest::parse(
        r#"{"id":"tr_5Kp3MnOpQr","status":"paid","amount":{"currency":"USD","value":"1.25"}}"#,
    );
    assert!(matches!(
        validate_payment(&no_reference, None, None),
        Err(BillingError::WebhookRejected { .. })
    ));
    // Wrong currency.
    let euro = PaymentForTest::parse(
        r#"{"id":"tr_5Kp3MnOpQr","status":"paid","amount":{"currency":"EUR","value":"1.25"},"metadata":{"reference":"apex-fee:pub-1:exp-a:2026-10","amount":"1.25"}}"#,
    );
    assert!(matches!(
        validate_payment(&euro, None, None),
        Err(BillingError::WebhookRejected { .. })
    ));
}

/// Small parse helper so hostile literals stay readable in the test body.
struct PaymentForTest;
impl PaymentForTest {
    fn parse(json: &str) -> MolliePayment {
        MolliePayment::from_json(&serde_json::from_str(json).expect("json")).expect("payment")
    }
}

#[test]
fn payment_shape_fails_closed_on_unknown_status_and_bad_ids() {
    for json in [
        r#"{"id":"pl_1","status":"paid","amount":{"currency":"USD","value":"1.00"}}"#,
        r#"{"id":"tr_5Kp3MnOpQr","status":"teleported","amount":{"currency":"USD","value":"1.00"}}"#,
        r#"{"id":"tr_5Kp3MnOpQr","status":"paid","amount":{"currency":"usd","value":"1.00"}}"#,
        r#"{"id":"tr_5Kp3MnOpQr","status":"paid","amount":{"currency":"USD","value":1}}"#,
        r#"{"id":"tr_5Kp3MnOpQr","status":"paid"}"#,
    ] {
        assert!(
            MolliePayment::from_json(&serde_json::from_str(json).expect("json")).is_err(),
            "{json}"
        );
    }
}

#[test]
fn collection_events_flatten_and_key_correctly() {
    let payment = MolliePayment::from_json(
        &serde_json::from_str::<serde_json::Value>(&format!(
            r#"{{"id":"{PAYMENT_ID}","status":"paid","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"apex-fee:pub:exp:2026-10"}},"customerId":"{CUSTOMER_ID}","mandateId":"{MANDATE_ID}"}}"#
        ))
        .unwrap(),
    )
    .expect("payment");
    let event = CollectionEvent::from_payment(&payment, 42);
    assert_eq!(event.at_unix_ms, 42);
    assert_eq!(event.payment_id, PAYMENT_ID);
    assert_eq!(event.status, "paid");
    assert_eq!(event.replay_key(), format!("{PAYMENT_ID}:paid"));
    assert_eq!(event.publisher_id(), Some("pub"));
    assert_eq!(
        event.stored_mandate(),
        Some(format!("{CUSTOMER_ID}:{MANDATE_ID}"))
    );
}

#[test]
fn collection_store_dedupes_on_payment_id_and_status() {
    let dir = std::env::temp_dir().join(format!("apex-coll-mollie-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FileCollectionStore::open(&dir.join("c.jsonl")).expect("open");
    let payment = MolliePayment::from_json(
        &serde_json::from_str::<serde_json::Value>(&format!(
            r#"{{"id":"{PAYMENT_ID}","status":"paid","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"apex-fee:pub:exp:2026-10"}}}}"#
        ))
        .unwrap(),
    )
    .expect("payment");
    let event = CollectionEvent::from_payment(&payment, 1);
    assert!(store.record(&event).unwrap());
    assert!(!store.record(&event).unwrap(), "replay is not new");
    let mut transitioned = event.clone();
    transitioned.status = "refunded".into();
    assert!(
        store.record(&transitioned).unwrap(),
        "status transition is new"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fresh_collection_store_has_seen_nothing() {
    let dir = std::env::temp_dir().join(format!("apex-coll-fresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FileCollectionStore::open(&dir.join("c.jsonl")).expect("open");
    assert!(!store.seen("tr_anything:paid").unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn collection_store_refuses_a_corrupt_log() {
    let dir = std::env::temp_dir().join(format!("apex-coll-corrupt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("coll.jsonl");
    std::fs::write(&path, b"not-json\n").unwrap();
    assert!(crate::collection::FileCollectionStore::open(&path).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn payment_links_bill_the_gross_including_vat() {
    let mock = MockTransport::new(vec![Ok(link_response("pl_1"))]);
    let transport = retried(mock.clone());
    let invoice = invoice_with(vec![invoice_line(1, "exp-a", 1_250_000)])
        .with_vat(2_000)
        .expect("vat");
    let receipt = create_payment_links(&client(), transport.as_ref(), &invoice)
        .await
        .expect("links");
    assert_eq!(
        receipt.total_minor_units, 150,
        "gross 1_500_000 micros = 150 cents"
    );
    let call = &mock.calls()[0];
    let body = match call.body.as_ref().expect("body") {
        HttpBody::Json(value) => value.clone(),
        other => panic!("unexpected body {other:?}"),
    };
    assert_eq!(body["amount"]["value"], "1.50");
}

fn period() -> BillingPeriod {
    BillingPeriod {
        year_month: "2023-11".to_string(),
        start_unix_ms: 1_698_796_800_000,
        end_unix_ms: 1_701_475_200_000,
    }
}

/// A consistent proof: `fee = lift * rate / 10_000`.
fn proof(seq: u64, experiment: &str, lift: i64, rate_bp: u16) -> BillingProofRecord {
    BillingProofRecord {
        seq,
        at_unix_ms: 1_700_000_000_000,
        publisher_id: "pub-1".to_string(),
        experiment_id: experiment.to_string(),
        proven_lift: lift,
        rate_bp,
        fee_micros: lift * i64::from(rate_bp) / 10_000,
    }
}

#[test]
fn invoice_reverifies_and_orders_lines_by_seq() {
    let invoice = compute_invoice(
        "pub-1",
        period(),
        &[
            proof(2, "exp-b", 1_000_000, 1_500),
            proof(1, "exp-a", 2_500_000, 2_000),
        ],
    )
    .expect("valid proofs");
    assert_eq!(invoice.line_items.len(), 2);
    assert_eq!(invoice.line_items[0].seq, 1);
    assert_eq!(invoice.line_items[0].fee_micros, 500_000);
    assert_eq!(invoice.line_items[1].seq, 2);
    assert_eq!(invoice.line_items[1].fee_micros, 150_000);
    assert_eq!(invoice.total_micros, 650_000);
}

#[test]
fn empty_proof_set_is_a_zero_invoice_not_an_error() {
    let invoice = compute_invoice("pub-1", period(), &[]).expect("no lift, no fee");
    assert!(invoice.line_items.is_empty());
    assert_eq!(invoice.total_micros, 0);
}

#[test]
fn invoice_refuses_proofs_outside_the_period() {
    let mut early = proof(1, "exp-a", 2_500_000, 2_000);
    early.at_unix_ms = 1_698_796_800_000 - 1;
    assert_eq!(
        compute_invoice("pub-1", period(), &[early]),
        Err(BillingError::OutOfPeriod { seq: 1 })
    );
    let mut late = proof(2, "exp-b", 2_500_000, 2_000);
    late.at_unix_ms = 1_701_475_200_000;
    assert_eq!(
        compute_invoice("pub-1", period(), &[late]),
        Err(BillingError::OutOfPeriod { seq: 2 })
    );
}

#[test]
fn invoice_refuses_another_publishers_proof() {
    let mut foreign = proof(5, "exp-a", 2_500_000, 2_000);
    foreign.publisher_id = "pub-2".to_string();
    assert_eq!(
        compute_invoice("pub-1", period(), &[foreign]),
        Err(BillingError::WrongPublisher { seq: 5 })
    );
}

#[test]
fn invoice_never_trusts_the_stored_fee() {
    let mut tampered = proof(7, "exp-a", 2_500_000, 2_000);
    tampered.fee_micros += 1;
    assert!(matches!(
        compute_invoice("pub-1", period(), &[tampered]),
        Err(BillingError::TamperedProof { seq: 7, .. })
    ));
}

#[test]
fn invoice_refuses_non_positive_lift_and_off_band_rates() {
    let mut zero_lift = proof(1, "exp-a", 2_500_000, 2_000);
    zero_lift.proven_lift = 0;
    assert!(matches!(
        compute_invoice("pub-1", period(), &[zero_lift]),
        Err(BillingError::TamperedProof { seq: 1, .. })
    ));
    let mut negative_lift = proof(2, "exp-b", 2_500_000, 2_000);
    negative_lift.proven_lift = -1;
    assert!(matches!(
        compute_invoice("pub-1", period(), &[negative_lift]),
        Err(BillingError::TamperedProof { seq: 2, .. })
    ));
    for rate in [1_499u16, 2_001] {
        let mut off_band = proof(3, "exp-c", 2_500_000, rate);
        off_band.fee_micros = 0;
        assert!(matches!(
            compute_invoice("pub-1", period(), &[off_band]),
            Err(BillingError::TamperedProof { seq: 3, .. })
        ));
    }
}

#[test]
fn invoice_refuses_duplicate_seqs_and_experiments() {
    let duplicate_seq = [
        proof(4, "exp-a", 1_000_000, 2_000),
        proof(4, "exp-b", 1_000_000, 2_000),
    ];
    assert!(matches!(
        compute_invoice("pub-1", period(), &duplicate_seq),
        Err(BillingError::Duplicate { seq: 4, .. })
    ));
    let duplicate_experiment = [
        proof(1, "exp-a", 1_000_000, 2_000),
        proof(2, "exp-a", 1_000_000, 2_000),
    ];
    assert!(matches!(
        compute_invoice("pub-1", period(), &duplicate_experiment),
        Err(BillingError::Duplicate {
            seq: 2,
            ref experiment_id
        }) if experiment_id == "exp-a"
    ));
}

#[test]
fn billing_proof_flattening_roundtrips_sealed_records() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("pub-1.jsonl");
    let mut ledger = Ledger::open(&path, Keypair::from_bytes(&[7u8; 32])).expect("open ledger");
    let sealed = ledger
        .append(LedgerRecord::billing_proof(
            1_700_000_000_000,
            "pub-1",
            "exp-1",
            2_500_000,
            2_000,
            500_000,
        ))
        .expect("append billing proof");
    let flattened = BillingProofRecord::from_ledger_record(&sealed).expect("billing proof");
    assert_eq!(flattened.seq, 0);
    assert_eq!(flattened.proven_lift, 2_500_000);
    assert_eq!(flattened.fee_micros, 500_000);

    let change = ledger
        .append(LedgerRecord::change_applied(
            1_700_000_000_001,
            apex_core::PlatformId::Admob,
            "u1",
            ChangeKind::CountryFloorSet {
                country: "EE".into(),
                floor_ecpm: 2_500_000,
            },
            "admob-floor:u1:EE",
            "0",
        ))
        .expect("append change");
    assert!(matches!(
        BillingProofRecord::from_ledger_record(&change),
        Err(BillingError::Malformed(_x))
    ));
}

#[test]
fn vat_is_added_on_top_of_the_net_fee() {
    let invoice = invoice_with(vec![invoice_line(1, "exp-a", 1_250_000)])
        .with_vat(2_000)
        .expect("vat");
    assert_eq!(invoice.total_micros, 1_250_000, "net is unchanged");
    assert_eq!(invoice.vat_micros, 250_000, "20% of the net");
    assert_eq!(invoice.gross_micros, 1_500_000);
    assert_eq!(invoice.line_items[0].gross_micros, 1_500_000);
    // A bare rate is not a decided treatment: the renderer refuses until
    // `apply_vat_treatment` records one (the golden tests cover the
    // rendered document itself).
    assert_eq!(
        invoice.render_vat_invoice(&seller_profile()),
        Err(BillingError::UnregisteredInvoice)
    );
}

/// A complete seller profile for the renderer refusals in this suite.
fn seller_profile() -> BillingProfile {
    BillingProfile {
        legal_name: "Apex Media OÜ".into(),
        address_lines: vec!["Sepapaja tn 6".into(), "15551 Tallinn".into()],
        country: "EE".into(),
        vat_number: "EE123456789".into(),
        vat_rate_bp: 2_000,
    }
}

#[test]
fn vat_overflow_is_refused_never_saturated() {
    let extreme = |fee_micros: i64| {
        invoice_with(vec![InvoiceLine {
            seq: 1,
            experiment_id: "e".into(),
            proven_lift: 1,
            rate_bp: 1_500,
            fee_micros,
            vat_micros: 0,
            gross_micros: fee_micros,
        }])
    };
    // VAT multiplication overflows: refused, not saturated.
    assert_eq!(
        extreme(i64::MAX).with_vat(2_000),
        Err(BillingError::Overflow)
    );

    // The line VAT fits, but net + VAT does not: still a typed refusal.
    assert_eq!(
        extreme(i64::MAX - 1).with_vat(1),
        Err(BillingError::Overflow)
    );
}

#[test]
fn vat_rounds_half_up_to_the_micro() {
    // 1 micro net at 20% is 0.2 micro -> 0; 3 micros -> 0.6 -> 1.
    let zero = invoice_with(vec![invoice_line(1, "e", 1)])
        .with_vat(2_000)
        .expect("vat");
    assert_eq!(zero.vat_micros, 0);
    let one = invoice_with(vec![invoice_line(1, "e", 3)])
        .with_vat(2_000)
        .expect("vat");
    assert_eq!(one.vat_micros, 1);
}

#[test]
fn every_billing_error_renders_and_converts() {
    // Formatting every variant covers each Display impl (thiserror generates
    // one per variant, which the function metric counts).
    let variants = [
        BillingError::OutOfPeriod { seq: 1 },
        BillingError::WrongPublisher { seq: 2 },
        BillingError::TamperedProof {
            seq: 3,
            reason: "why".into(),
        },
        BillingError::Duplicate {
            seq: 4,
            experiment_id: "e".into(),
        },
        BillingError::Overflow,
        BillingError::SubCentAmount { seq: 5 },
        BillingError::Auth { status: 401 },
        BillingError::Http { status: 500 },
        BillingError::Schema { detail: "d".into() },
        BillingError::Network { detail: "n".into() },
        BillingError::Transport { detail: "t".into() },
        BillingError::PartialCollection {
            completed: vec!["a".into()],
            failed_seq: 6,
            cause: "c".into(),
        },
        BillingError::UnsafeIdentifier {
            seq: 7,
            detail: "u".into(),
        },
        BillingError::MissingProfileField {
            party: "seller",
            field: "legalName",
        },
        BillingError::MissingCustomerProfile,
        BillingError::InvalidVatRate { detail: "d".into() },
        BillingError::InvalidVatNumber {
            party: "customer",
            detail: "d".into(),
        },
        BillingError::InvalidCountryCode {
            party: "seller",
            value: "X".into(),
        },
        BillingError::VatTreatmentNotApplied,
        BillingError::InconsistentVat { detail: "d".into() },
        BillingError::UnregisteredInvoice,
        BillingError::RegisterCorrupt { detail: "d".into() },
        BillingError::PlanStateCorrupt { detail: "d".into() },
        BillingError::StoredChargeRefused {
            detail: "d".into(),
            payment_id: None,
        },
        BillingError::Malformed("m".into()),
        BillingError::WebhookRejected { detail: "d".into() },
    ];
    for error in &variants {
        assert!(!error.to_string().is_empty());
        let _ = format!("{error:?}");
        // thiserror generates a `source()` per variant; call it so those
        // functions are covered.
        let _ = std::error::Error::source(error);
    }

    // Every conversion arm from the platform taxonomy.
    assert_eq!(
        BillingError::from(apex_core::PlatformError::Auth {
            platform: apex_core::PlatformId::Admob,
            operation: "op".into(),
            status: 403,
        }),
        BillingError::Auth { status: 403 }
    );
    let platform_cases = [
        apex_core::PlatformError::RateLimited {
            platform: apex_core::PlatformId::Admob,
            operation: "o".into(),
            retry_after: None,
        },
        apex_core::PlatformError::Http {
            platform: apex_core::PlatformId::Admob,
            operation: "o".into(),
            status: 401,
        },
        apex_core::PlatformError::Http {
            platform: apex_core::PlatformId::Admob,
            operation: "o".into(),
            status: 500,
        },
        apex_core::PlatformError::Schema {
            platform: apex_core::PlatformId::Admob,
            operation: "o".into(),
            detail: "d".into(),
        },
        apex_core::PlatformError::Network {
            platform: apex_core::PlatformId::Admob,
            operation: "o".into(),
            detail: "n".into(),
        },
        apex_core::PlatformError::Validation("v".into()),
    ];
    for error in platform_cases {
        assert!(!BillingError::from(error).to_string().is_empty());
    }
    // Every arm from the transport taxonomy, including a wrapped platform error.
    assert_eq!(
        BillingError::from(apex_connectors::transport::TransportError::Network(
            "x".into()
        )),
        BillingError::Network { detail: "x".into() }
    );
    assert!(matches!(
        BillingError::from(apex_connectors::transport::TransportError::Timeout(
            "t".into()
        )),
        BillingError::Network { ref detail } if detail.contains("timeout")
    ));
    assert_eq!(
        BillingError::from(apex_connectors::transport::TransportError::Platform(
            apex_core::PlatformError::Validation("inner".into())
        )),
        BillingError::Transport {
            detail: "inner".into()
        }
    );
}

#[test]
fn basis_invoices_rederive_the_fee_and_refuse_tampering() {
    let period = BillingPeriod {
        year_month: "2026-10".into(),
        start_unix_ms: 0,
        end_unix_ms: 1_000_000_000_000,
    };
    let record = |incremental: i64, log_share: f64, rate_bp: u16, fee: i64| BillingBasisRecord {
        seq: 7,
        at_unix_ms: 500,
        change_id: "exp-1".into(),
        scope_revenue: 1_000_000_000,
        incremental,
        log_share,
        rate_bp,
        fee_micros: fee,
        billed_days: 0,
        month_days: 0,
    };
    // fee = round(0.2 * 90_909_090 * 1.0) = 18_181_818 -> cent-exact 18_180_000.
    let invoice = crate::invoice::compute_invoice_from_basis(
        "pub1",
        period.clone(),
        &[record(90_909_090, 1.0, 2_000, 18_180_000)],
    )
    .expect("invoice");
    assert_eq!(invoice.line_items[0].fee_micros, 18_180_000);

    // A stored fee that does not re-derive is refused.
    assert!(matches!(
        crate::invoice::compute_invoice_from_basis(
            "pub1",
            period.clone(),
            &[record(90_909_090, 1.0, 2_000, 1)]
        ),
        Err(BillingError::TamperedProof { .. })
    ));
    // Non-positive lift, a rate outside the band and duplicates are refused.
    assert!(crate::invoice::compute_invoice_from_basis(
        "pub1",
        period.clone(),
        &[record(0, 1.0, 2_000, 0)]
    )
    .is_err());
    assert!(crate::invoice::compute_invoice_from_basis(
        "pub1",
        period.clone(),
        &[record(1, 1.0, 9_999, 0)]
    )
    .is_err());
    assert!(crate::invoice::compute_invoice_from_basis(
        "pub1",
        period.clone(),
        &[
            record(90_909_090, 1.0, 2_000, 18_180_000),
            record(90_909_090, 1.0, 2_000, 18_180_000)
        ]
    )
    .is_err());
}

#[test]
fn basis_invoices_pro_rate_and_round_to_the_cent() {
    let period = BillingPeriod {
        year_month: "2026-10".into(),
        start_unix_ms: 0,
        end_unix_ms: 1_000_000_000_000,
    };
    // 17 of 30 days: round(18_181_818 * 17/30) = 10_303_031 -> cent 10_300_000.
    let fee = crate::invoice::round_to_cent((18_181_818.0_f64 * 17.0 / 30.0).round() as i64);
    let record = BillingBasisRecord {
        seq: 7,
        at_unix_ms: 500,
        change_id: "exp-1".into(),
        scope_revenue: 1_000_000_000,
        incremental: 90_909_090,
        log_share: 1.0,
        rate_bp: 2_000,
        fee_micros: fee,
        billed_days: 17,
        month_days: 30,
    };
    let invoice =
        crate::invoice::compute_invoice_from_basis("pub1", period, &[record]).expect("invoice");
    assert_eq!(invoice.line_items[0].fee_micros, 10_300_000);
    assert_eq!(crate::invoice::round_to_cent(0), 0);
    assert_eq!(crate::invoice::round_to_cent(-5), -5);
}

#[test]
fn round_to_cent_near_i64_max_never_wraps() {
    // The cent rounding adds half a minor unit before dividing. A fee within
    // half a cent of i64::MAX must not overflow that add (debug panic) nor
    // wrap negative (release): it saturates to the cent-exact value.
    assert_eq!(
        crate::invoice::round_to_cent(i64::MAX),
        9_223_372_036_854_770_000
    );
    assert_eq!(
        crate::invoice::round_to_cent(i64::MAX - 4_999),
        9_223_372_036_854_770_000
    );
}

#[test]
fn basis_with_saturating_log_share_is_refused_never_wrapped() {
    // `log_share` is deserialized from the signed ledger payload and is not
    // range checked; a huge finite value saturates verify_basis's f64->i64
    // cast to i64::MAX. The cent rounding must refuse that record rather
    // than wrap the fee negative -- a wrapped fee equal to `wrapped` below
    // would otherwise pass the re-derivation equality check.
    let period = BillingPeriod {
        year_month: "2026-10".into(),
        start_unix_ms: 0,
        end_unix_ms: 1_000_000_000_000,
    };
    let wrapped = (i64::MAX.wrapping_add(5_000) / 10_000) * 10_000;
    assert!(wrapped < 0, "the wrap this test pins is negative");
    let record = BillingBasisRecord {
        seq: 7,
        at_unix_ms: 500,
        change_id: "exp-1".into(),
        scope_revenue: i64::MAX,
        incremental: i64::MAX,
        log_share: 1e300,
        rate_bp: 2_000,
        fee_micros: wrapped,
        billed_days: 0,
        month_days: 0,
    };
    assert!(matches!(
        crate::invoice::compute_invoice_from_basis("pub1", period, &[record]),
        Err(BillingError::TamperedProof { .. })
    ));
}
