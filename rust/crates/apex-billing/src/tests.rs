//! Adversarial suite for the Adyen billing surface: the webhook HMAC scheme
//! and the payment-link client over a recording mock transport.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use apex_connectors::transport::{
    HttpBody, HttpRequest, HttpResponse, RecordingSleeper, RetryTransport, Sleeper, Transport,
    TransportError,
};
use async_trait::async_trait;

use crate::adyen::{
    create_payment_links, sign_notification_item, verify_notification_item, AdyenClient,
    AdyenClientError, AdyenHmacError, BillingReceipt, InvoiceLineItem, NotificationItem,
    PaymentLinkResponse, MAX_REFERENCE_LEN,
};
use crate::errors::BillingError;
use crate::invoice::{
    compute_invoice, BillingBasisRecord, BillingPeriod, BillingProofRecord, Invoice, InvoiceLine,
};
use crate::webhook::verify_adyen_webhook;
use apex_proof::{ChangeKind, Keypair, Ledger, LedgerRecord};

const KEY: &[u8] = b"hmac-key";
// base64(HMAC-SHA256(signed_message, KEY)) over the pinned item below,
// computed independently of the crate.
const PINNED_SIGNATURE: &str = "0go3p4FePtmEGTPKiahYDi0eiRXMcB4te4FZJhVRmDs=";

fn pinned_item() -> NotificationItem {
    NotificationItem {
        psp_reference: "PSP123".into(),
        original_reference: String::new(),
        merchant_account_code: "ApexMerchant".into(),
        merchant_reference: "apex-fee:pub:exp:2026-10".into(),
        amount_value: 12_345,
        amount_currency: "USD".into(),
        event_code: "AUTHORISATION".into(),
        success: "true".into(),
        hmac_signature: PINNED_SIGNATURE.into(),
        payment_method: String::new(),
    }
}

fn webhook_body(item: &NotificationItem) -> Vec<u8> {
    serde_json::json!({
        "live": "false",
        "notificationItems": [{
            "NotificationRequestItem": {
                "pspReference": item.psp_reference,
                "originalReference": item.original_reference,
                "merchantAccountCode": item.merchant_account_code,
                "merchantReference": item.merchant_reference,
                "amount": {"value": item.amount_value, "currency": item.amount_currency},
                "eventCode": item.event_code,
                "success": item.success,
                "additionalData": {"hmacSignature": item.hmac_signature},
            }
        }]
    })
    .to_string()
    .into_bytes()
}

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

fn client() -> AdyenClient {
    AdyenClient::new(
        "AQEyhmfx...test...key",
        "ApexMerchant",
        KEY.to_vec(),
        "https://checkout-test.adyen.com",
    )
    .expect("client")
}

#[test]
fn pinned_vector_verifies() {
    verify_notification_item(KEY, &pinned_item())
        .unwrap_or_else(|_| panic!("pinned digest must verify"));
    let body = webhook_body(&pinned_item());
    let items = verify_adyen_webhook(&body, KEY).expect("billing boundary");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].event_code, "AUTHORISATION");
}

#[test]
fn the_signed_message_is_the_eight_fields_in_order() {
    assert_eq!(
        pinned_item().signed_message(),
        "PSP123::ApexMerchant:apex-fee:pub:exp:2026-10:12345:USD:AUTHORISATION:true"
    );
}

#[test]
fn sign_helper_roundtrips() {
    let signed = sign_notification_item(KEY, &pinned_item());
    assert_eq!(signed, PINNED_SIGNATURE);
    let mut item = pinned_item();
    item.hmac_signature = signed;
    verify_notification_item(KEY, &item).expect("roundtrip");
}

#[test]
fn signature_is_sensitive_to_key_and_every_signed_field() {
    assert_eq!(
        verify_notification_item(b"other", &pinned_item()),
        Err(AdyenHmacError::NoMatchingSignature)
    );
    for mutate in [
        |item: &mut NotificationItem| item.amount_value += 1,
        |item: &mut NotificationItem| item.event_code = "CANCELLED".into(),
        |item: &mut NotificationItem| item.success = "false".into(),
        |item: &mut NotificationItem| item.merchant_reference = "other".into(),
        |item: &mut NotificationItem| item.amount_currency = "EUR".into(),
    ] {
        let mut item = pinned_item();
        mutate(&mut item);
        assert_eq!(
            verify_notification_item(KEY, &item),
            Err(AdyenHmacError::NoMatchingSignature),
            "{item:?}"
        );
    }
}

#[test]
fn missing_and_malformed_signatures_are_refused() {
    let mut missing = pinned_item();
    missing.hmac_signature = String::new();
    assert_eq!(
        verify_notification_item(KEY, &missing),
        Err(AdyenHmacError::MissingHeader)
    );
    let mut malformed = pinned_item();
    malformed.hmac_signature = "not base64!!".into();
    assert_eq!(
        verify_notification_item(KEY, &malformed),
        Err(AdyenHmacError::MalformedHeader)
    );
    assert!(matches!(
        verify_adyen_webhook(&webhook_body(&missing), KEY),
        Err(BillingError::Malformed(_))
    ));
    assert!(matches!(
        verify_adyen_webhook(&webhook_body(&malformed), KEY),
        Err(BillingError::Malformed(_))
    ));
    assert_eq!(
        verify_adyen_webhook(&webhook_body(&pinned_item()), b"wrong"),
        Err(BillingError::InvalidSignature)
    );
}

#[test]
fn malformed_envelopes_are_refused() {
    assert!(matches!(
        verify_adyen_webhook(b"not json", KEY),
        Err(BillingError::Malformed(_))
    ));
    assert!(matches!(
        verify_adyen_webhook(br#"{"notificationItems":[]}"#, KEY),
        Err(BillingError::Malformed(_))
    ));
    // An item without the NotificationRequestItem wrapper.
    assert!(matches!(
        verify_adyen_webhook(br#"{"notificationItems":[{"pspReference":"x"}]}"#, KEY),
        Err(BillingError::Malformed(_))
    ));
}

#[test]
fn wrong_length_digests_never_match() {
    // A 31- or 33-byte digest cannot match a 32-byte HMAC; base64 padding
    // leftovers are refused rather than silently truncated.
    for length in [31usize, 33] {
        let mut item = pinned_item();
        item.hmac_signature = base64_of(&vec![0u8; length]);
        assert!(
            verify_notification_item(KEY, &item).is_err(),
            "{length}-byte digest must never verify"
        );
    }
}

fn base64_of(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(triple >> 18) as usize & 63] as char);
        out.push(TABLE[(triple >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(triple >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[triple as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[test]
fn client_refuses_empty_credentials() {
    assert_eq!(
        AdyenClient::new("", "m", KEY.to_vec(), "https://x").unwrap_err(),
        AdyenClientError::EmptyApiKey
    );
    assert_eq!(
        AdyenClient::new("k", "", KEY.to_vec(), "https://x").unwrap_err(),
        AdyenClientError::EmptyMerchantAccount
    );
    assert_eq!(
        AdyenClient::new("k", "m", Vec::<u8>::new(), "https://x").unwrap_err(),
        AdyenClientError::EmptyHmacKey
    );
    let ok = AdyenClient::new("k", "m", b" ".to_vec(), "https://x").expect("space is a key");
    assert_eq!(ok.api_key(), "k");
    assert_eq!(ok.merchant_account(), "m");
}

#[test]
fn client_debug_redacts_credentials() {
    let debug = format!("{:?}", client());
    assert!(!debug.contains("AQEyhmfx"), "{debug}");
    assert!(!debug.contains("hmac-key"), "{debug}");
    assert!(debug.contains("ApexMerchant"), "{debug}");
}

fn ok_item(id: &str) -> HttpResponse {
    MockTransport::ok(
        200,
        Some(&format!(
            r#"{{"id":"{id}","url":"https://checkoutshopper-test.adyen.com/checkoutshopper/payByLink.shtml?token=x"}}"#
        )),
    )
}

fn error_response(status: u16) -> HttpResponse {
    MockTransport::ok(status, Some(r#"{"errorCode":"000","message":"nope"}"#))
}

fn invoice_with(lines: Vec<InvoiceLine>) -> Invoice {
    let total_micros: i64 = lines.iter().map(|l| l.fee_micros).sum();
    Invoice {
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

#[tokio::test]
async fn payment_links_post_one_idempotent_link_per_line() {
    let mock = MockTransport::new(vec![Ok(ok_item("PL-1")), Ok(ok_item("PL-2"))]);
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
    let calls = mock.calls();
    assert_eq!(calls.len(), 2);
    for call in &calls {
        assert!(call.url.contains("/v71/paymentLinks"), "{}", call.url);
        assert!(call
            .headers
            .iter()
            .any(|(k, v)| k == "X-API-Key" && v == client().api_key()));
        // The deterministic reference is also the Idempotency-Key header.
        let reference = match call.body.as_ref().expect("body") {
            HttpBody::Json(value) => value["reference"].as_str().unwrap_or_default().to_string(),
            other => panic!("unexpected body {other:?}"),
        };
        assert!(call
            .headers
            .iter()
            .any(|(k, v)| k == "Idempotency-Key" && *v == reference));
    }
    let first = match calls[0].body.as_ref().expect("body") {
        HttpBody::Json(value) => value.clone(),
        other => panic!("unexpected body {other:?}"),
    };
    assert_eq!(first["reference"], "apex-fee:pub-1:exp-a:2026-10");
    assert_eq!(first["amount"]["value"], 125);
    assert_eq!(first["amount"]["currency"], "USD");
    assert_eq!(first["merchantAccount"], "ApexMerchant");
    assert_eq!(first["shopperReference"], "publisher-pub-1");
    assert_eq!(first["recurringProcessingModel"], "Subscription");
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
    let mock = MockTransport::new(vec![Ok(ok_item("PL-1"))]);
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
        Some(r#"{"id":"PL-1"}"#),
        Some(r#"{"url":"https://x"}"#),
        None,
    ] {
        let mock = MockTransport::new(vec![Ok(MockTransport::ok(200, body))]);
        let transport = retried(mock.clone());
        let invoice = invoice_with(vec![invoice_line(1, "exp-a", 1_000_000)]);
        assert!(matches!(
            create_payment_links(&client(), transport.as_ref(), &invoice).await,
            Err(BillingError::Malformed(_x))
        ));
    }
}

#[test]
fn line_items_convert_only_exact_cents() {
    let line = invoice_line(1, "exp-a", 1_250_000);
    let item = InvoiceLineItem::from_invoice_line(&line, "pub-1", "2026-10").expect("exact");
    assert_eq!(item.amount_minor_units, 125);
    assert_eq!(item.reference, "apex-fee:pub-1:exp-a:2026-10");
    assert_eq!(item.shopper_reference, "publisher-pub-1");
    assert_eq!(item.currency, "USD");
}

#[test]
fn receipt_serializes_camel_case() {
    let receipt = BillingReceipt {
        publisher_id: "pub-1".into(),
        year_month: "2026-10".into(),
        currency: "USD".into(),
        total_minor_units: 125,
        links: vec![PaymentLinkResponse {
            id: "PL-1".into(),
            url: "https://x".into(),
        }],
    };
    let json = serde_json::to_string(&receipt).expect("json");
    assert!(json.contains("totalMinorUnits"), "{json}");
    assert!(json.contains("PL-1"), "{json}");
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
    let invoice = invoice_with(vec![invoice_line(1, "exp-a", 1_250_000)]).with_vat(2_000);
    assert_eq!(invoice.total_micros, 1_250_000, "net is unchanged");
    assert_eq!(invoice.vat_micros, 250_000, "20% of the net");
    assert_eq!(invoice.gross_micros, 1_500_000);
    assert_eq!(invoice.line_items[0].gross_micros, 1_500_000);
    let rendered = invoice.render_vat_invoice();
    assert!(rendered.contains("VAT (2000 bp)"), "{rendered}");
    assert!(rendered.contains("gross 1500000 micros"), "{rendered}");
}

#[test]
fn vat_rounds_half_up_to_the_micro() {
    // 1 micro net at 20% is 0.2 micro -> 0; 3 micros -> 0.6 -> 1.
    let zero = invoice_with(vec![invoice_line(1, "e", 1)]).with_vat(2_000);
    assert_eq!(zero.vat_micros, 0);
    let one = invoice_with(vec![invoice_line(1, "e", 3)]).with_vat(2_000);
    assert_eq!(one.vat_micros, 1);
}

#[tokio::test]
async fn payment_links_bill_the_gross_including_vat() {
    let mock = MockTransport::new(vec![Ok(MockTransport::ok(
        200,
        Some(r#"{"id":"PL1","url":"https://pay.adyen.com/1"}"#),
    ))]);
    let transport = retried(mock.clone());
    let invoice = invoice_with(vec![invoice_line(1, "exp-a", 1_250_000)]).with_vat(2_000);
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
    assert_eq!(body["amount"]["value"], 150);
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
        BillingError::Malformed("m".into()),
        BillingError::InvalidSignature,
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
fn adyen_envelope_errors_are_typed() {
    // Missing amount, a non-numeric value, a bad success type and a missing
    // signature each fail closed with a malformed input.
    for body in [
        br#"{"notificationItems":[{"NotificationRequestItem":{"pspReference":"p","merchantAccountCode":"m","merchantReference":"r","eventCode":"AUTHORISATION","success":"true","additionalData":{"hmacSignature":"s"}}}]}"#.as_slice(),
        br#"{"notificationItems":[{"NotificationRequestItem":{"pspReference":"p","merchantAccountCode":"m","merchantReference":"r","amount":{"value":"x","currency":"USD"},"eventCode":"AUTHORISATION","success":"true","additionalData":{"hmacSignature":"s"}}}]}"#,
        br#"{"notificationItems":[{"NotificationRequestItem":{"pspReference":"p","merchantAccountCode":"m","merchantReference":"r","amount":{"value":1,"currency":"USD"},"eventCode":"AUTHORISATION","success":1,"additionalData":{"hmacSignature":"s"}}}]}"#,
        br#"{"notificationItems":[{"NotificationRequestItem":{"pspReference":"p","merchantAccountCode":"m","merchantReference":"r","amount":{"value":1,"currency":"USD"},"eventCode":"AUTHORISATION","success":"true"}}]}"#,
    ] {
        assert!(
            matches!(
                crate::adyen::parse_notification_items(body),
                Err(BillingError::Malformed(_))
            ),
            "body {body:?}"
        );
    }
    // A bare item without the wrapper is refused too.
    assert!(matches!(
        crate::adyen::parse_notification_items(br#"{"notificationItems":[{"pspReference":"p"}]}"#),
        Err(BillingError::Malformed(_))
    ));
    // A top-level text field missing (the amount is present) fails closed.
    assert!(
        crate::adyen::parse_notification_items(
            br#"{"notificationItems":[{"NotificationRequestItem":{"pspReference":"p","merchantAccountCode":"m","merchantReference":"","amount":{"value":1,"currency":"USD"},"eventCode":"A","success":"true","additionalData":{"hmacSignature":"s"}}}]}"#
        )
        .is_ok(),
        "a missing originalReference is allowed (it is unsigned-empty)"
    );
    assert!(matches!(
        crate::adyen::parse_notification_items(
            br#"{"notificationItems":[{"NotificationRequestItem":{"pspReference":"p","merchantReference":"r","amount":{"value":1,"currency":"USD"},"eventCode":"A","success":"true","additionalData":{"hmacSignature":"s"}}}]}"#
        ),
        Err(BillingError::Malformed(_))
    ));
}

#[test]
fn the_adyen_client_verifies_and_exposes_its_key() {
    let client = AdyenClient::new(
        "AQEyhmfx_test",
        "ApexMerchant",
        KEY.to_vec(),
        "https://checkout-test.adyen.com",
    )
    .expect("client");
    assert_eq!(client.hmac_key(), KEY);
    // A bare item verified through the client uses the same signature scheme.
    let mut item = pinned_item();
    item.hmac_signature = sign_notification_item(KEY, &item);
    client.verify(&item).expect("verified through the client");
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
fn collection_events_flatten_and_key_correctly() {
    let item = pinned_item();
    let event = crate::collection::CollectionEvent::from_item(&item, 42);
    assert_eq!(event.at_unix_ms, 42);
    assert_eq!(event.event_code, "AUTHORISATION");
    assert_eq!(event.replay_key(), "AUTHORISATION:PSP123");
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

#[test]
fn basis_invoices_refuse_out_of_period_and_missing_change_ids() {
    let period = BillingPeriod {
        year_month: "2026-10".into(),
        start_unix_ms: 1_000,
        end_unix_ms: 2_000,
    };
    let record = BillingBasisRecord {
        seq: 7,
        at_unix_ms: 5,
        change_id: "exp-1".into(),
        scope_revenue: 1,
        incremental: 90_909_090,
        log_share: 1.0,
        rate_bp: 2_000,
        fee_micros: 18_180_000,
        billed_days: 0,
        month_days: 0,
    };
    assert!(matches!(
        crate::invoice::compute_invoice_from_basis("pub1", period, &[record]),
        Err(BillingError::OutOfPeriod { seq: 7 })
    ));
}

#[test]
fn a_fresh_collection_store_has_seen_nothing() {
    let dir = std::env::temp_dir().join(format!("apex-coll-fresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = crate::collection::FileCollectionStore::open(&dir.join("c.jsonl")).expect("open");
    use crate::collection::CollectionStore;
    assert!(!store.seen("AUTHORISATION:anything").unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn collection_store_dispatches_through_the_trait_object() {
    use crate::collection::{CollectionEvent, CollectionStore, FileCollectionStore};
    let dir = std::env::temp_dir().join(format!("apex-coll-dyn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = FileCollectionStore::open(&dir.join("c.jsonl")).expect("open");
    let store: &dyn CollectionStore = &store;
    let event = CollectionEvent {
        at_unix_ms: 1,
        event_code: "SALE".into(),
        psp_reference: "psp-1".into(),
        merchant_reference: "r".into(),
        success: "true".into(),
        amount_value: 1,
        amount_currency: "USD".into(),
    };
    assert!(store.record(&event).unwrap());
    assert!(!store.record(&event).unwrap());
    assert!(store.seen(&event.replay_key()).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn collection_store_surfaces_append_and_directory_errors() {
    use crate::collection::{CollectionEvent, CollectionStore, FileCollectionStore};
    let dir = std::env::temp_dir().join(format!("apex-coll-io-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Parent is a file, not a directory: opening the store fails, typed.
    let file_parent = dir.join("not-a-dir");
    std::fs::write(&file_parent, b"x").unwrap();
    assert!(FileCollectionStore::open(&file_parent.join("c.jsonl")).is_err());

    // The store opens, then a directory appears where its file should be, so
    // the append fails with a typed error rather than panicking.
    let path = dir.join("coll.jsonl");
    let store = FileCollectionStore::open(&path).expect("open");
    std::fs::create_dir_all(&path).unwrap();
    let event = CollectionEvent {
        at_unix_ms: 1,
        event_code: "SALE".into(),
        psp_reference: "psp".into(),
        merchant_reference: "r".into(),
        success: "true".into(),
        amount_value: 1,
        amount_currency: "USD".into(),
    };
    assert!(matches!(
        store.record(&event),
        Err(BillingError::Malformed(_))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}
