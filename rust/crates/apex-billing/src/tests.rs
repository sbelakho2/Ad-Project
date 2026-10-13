//! Adversarial tests: the pinned webhook wire vector, freshness boundaries,
//! rotation candidates, every malformed-header shape, invoice re-verification
//! refusals, and the Stripe invoice-item client over a recording mock
//! transport.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use apex_connectors::transport::{
    HttpMethod, HttpRequest, HttpResponse, Transport, TransportError,
};
use apex_proof::{ChangeKind, Keypair, Ledger, LedgerRecord};
use async_trait::async_trait;

use crate::encode::encode_form_component;
use crate::errors::BillingError;
use crate::invoice::{compute_invoice, BillingPeriod, BillingProofRecord, Invoice, InvoiceLine};
use crate::stripe::{
    bill_invoice, sign_stripe_payload, StripeClient, StripeItemResponse, StripeSignatureError,
    VerifiedEvent, MICROS_PER_MINOR_UNIT, STRIPE_API_BASE, STRIPE_CURRENCY,
};
use crate::webhook::{verify_stripe_signature, EventMeta, DEFAULT_TOLERANCE_SECS};

const SECRET: &[u8] = b"whsec_test_secret";

/// Body used for the pinned vector; the expected digest is quoted in the test
/// and was computed independently with `openssl dgst -sha256 -hmac`.
const PINNED_BODY: &[u8] = br#"{"id":"evt_1","type":"invoice.paid"}"#;
const PINNED_TIMESTAMP: i64 = 1_492_774_577;
const PINNED_DIGEST: &str = "f7a632a9ffdbb1ced2fc48e44c374ec2cb4292469db292862c06ae703e2befbd";

#[test]
fn pinned_vector_verifies_and_returns_timestamp() {
    let header = format!("t={PINNED_TIMESTAMP},v1={PINNED_DIGEST}");
    let verified = verify_stripe_signature(PINNED_BODY, &header, SECRET, 300, PINNED_TIMESTAMP)
        .expect("pinned vector must verify");
    assert_eq!(
        verified,
        EventMeta {
            timestamp: PINNED_TIMESTAMP
        }
    );
}

#[test]
fn sign_helper_reproduces_the_pinned_vector() {
    let header = sign_stripe_payload(SECRET, PINNED_BODY, PINNED_TIMESTAMP);
    assert_eq!(header, format!("t={PINNED_TIMESTAMP},v1={PINNED_DIGEST}"));
    verify_stripe_signature(PINNED_BODY, &header, SECRET, 0, PINNED_TIMESTAMP).expect("roundtrip");
}

#[test]
fn default_tolerance_is_the_customary_300_seconds() {
    assert_eq!(DEFAULT_TOLERANCE_SECS, 300);
}

#[test]
fn signature_is_sensitive_to_every_signed_input() {
    let header = sign_stripe_payload(SECRET, PINNED_BODY, PINNED_TIMESTAMP);
    // Another secret.
    assert_eq!(
        verify_stripe_signature(PINNED_BODY, &header, b"other", 300, PINNED_TIMESTAMP),
        Err(BillingError::InvalidSignature)
    );
    // Another body (even one byte appended).
    let mut tampered = PINNED_BODY.to_vec();
    tampered.push(b' ');
    assert_eq!(
        verify_stripe_signature(&tampered, &header, SECRET, 300, PINNED_TIMESTAMP),
        Err(BillingError::InvalidSignature)
    );
    // Another signed timestamp (the `t` inside the header is part of the MAC).
    let replayed = header.replace("t=1492774577", "t=1492774600");
    assert_eq!(
        verify_stripe_signature(PINNED_BODY, &replayed, SECRET, 300, 1_492_774_600),
        Err(BillingError::InvalidSignature)
    );
}

#[test]
fn freshness_boundary_is_inclusive_and_absolute() {
    let header = sign_stripe_payload(SECRET, PINNED_BODY, 1_000);
    // Exactly at tolerance: accepted, in both directions.
    verify_stripe_signature(PINNED_BODY, &header, SECRET, 60, 1_060).expect("old edge");
    verify_stripe_signature(PINNED_BODY, &header, SECRET, 60, 940).expect("future edge");
    // One second beyond tolerance: refused as a stale timestamp, with both
    // sides of the comparison quoted.
    assert_eq!(
        verify_stripe_signature(PINNED_BODY, &header, SECRET, 60, 1_061),
        Err(BillingError::StaleTimestamp {
            timestamp: 1_000,
            now_unix_s: 1_061
        })
    );
    assert_eq!(
        verify_stripe_signature(PINNED_BODY, &header, SECRET, 60, 939),
        Err(BillingError::StaleTimestamp {
            timestamp: 1_000,
            now_unix_s: 939
        })
    );
    // Zero tolerance pins the exact second.
    verify_stripe_signature(PINNED_BODY, &header, SECRET, 0, 1_000).expect("exact second");
    assert!(verify_stripe_signature(PINNED_BODY, &header, SECRET, 0, 1_001).is_err());
}

#[test]
fn extreme_clock_values_do_not_wrap() {
    // i128 skew arithmetic: the most distant clocks are stale, not wrapped
    // into an accepted value.
    let header = sign_stripe_payload(SECRET, PINNED_BODY, i64::MIN);
    assert_eq!(
        verify_stripe_signature(PINNED_BODY, &header, SECRET, 300, i64::MAX),
        Err(BillingError::StaleTimestamp {
            timestamp: i64::MIN,
            now_unix_s: i64::MAX
        })
    );
    let header = sign_stripe_payload(SECRET, PINNED_BODY, i64::MAX);
    assert!(matches!(
        verify_stripe_signature(PINNED_BODY, &header, SECRET, 300, i64::MIN),
        Err(BillingError::StaleTimestamp { .. })
    ));
    // With u64::MAX tolerance the skew is genuinely within tolerance and the
    // signature is authentic, so it verifies (no artificial refusal either).
    verify_stripe_signature(PINNED_BODY, &header, SECRET, u64::MAX, i64::MIN)
        .expect("max tolerance accepts the maximum representable skew");
}

#[test]
fn every_v1_candidate_is_tried_for_rotation() {
    let header = sign_stripe_payload(SECRET, PINNED_BODY, PINNED_TIMESTAMP);
    let parts: Vec<&str> = header.split(',').collect();
    let rotated = format!(
        "v1={},t={PINNED_TIMESTAMP},{},v0={}",
        "00".repeat(32),
        parts[1],
        "ff".repeat(32),
    );
    verify_stripe_signature(PINNED_BODY, &rotated, SECRET, 300, PINNED_TIMESTAMP)
        .expect("one matching candidate is enough");
    // Whitespace around items is tolerated.
    let spaced = format!("t={PINNED_TIMESTAMP}, v1={PINNED_DIGEST}");
    verify_stripe_signature(PINNED_BODY, &spaced, SECRET, 300, PINNED_TIMESTAMP)
        .expect("trimmed items");
}

#[test]
fn malformed_headers_are_refused_as_malformed_input() {
    for header in [
        "",
        "t=1492774577",
        "v1=ab",
        "t=notanumber,v1=ab",
        "t=,v1=ab",
        "t=1492774577,",
        "t=1492774577,,v1=ab",
        "t=1492774577,=ab",
        "t=1492774577,v1=",
        "no-equals-at-all",
        "t=1492774577,v0=ff",
    ] {
        assert!(
            matches!(
                verify_stripe_signature(PINNED_BODY, header, SECRET, 300, PINNED_TIMESTAMP),
                Err(BillingError::Malformed(_))
            ),
            "header {header:?} must be refused as malformed"
        );
    }
}

#[test]
fn undecodable_or_short_v1_values_never_match() {
    for value in [
        "zz".repeat(32),
        "ab".to_string(),
        PINNED_DIGEST[..62].to_string(),
    ] {
        let header = format!("t={PINNED_TIMESTAMP},v1={value}");
        assert_eq!(
            verify_stripe_signature(PINNED_BODY, &header, SECRET, 300, PINNED_TIMESTAMP),
            Err(BillingError::InvalidSignature),
            "value {value:?} must never verify"
        );
    }
}

#[test]
fn uppercase_hex_is_a_different_byte_string() {
    // Stripe emits lowercase hex; uppercase is decoded and compared as bytes,
    // which is why an uppercase digest of the same bytes still matches.
    let header = format!("t={PINNED_TIMESTAMP},v1={}", PINNED_DIGEST.to_uppercase());
    verify_stripe_signature(PINNED_BODY, &header, SECRET, 300, PINNED_TIMESTAMP)
        .expect("hex case is an encoding detail, not a different MAC");
}

#[test]
fn empty_payload_is_signed_like_any_other() {
    let header = sign_stripe_payload(SECRET, b"", 1_700_000_000);
    verify_stripe_signature(b"", &header, SECRET, 0, 1_700_000_000).expect("empty body");
    assert!(verify_stripe_signature(b" ", &header, SECRET, 0, 1_700_000_000).is_err());
}

#[test]
fn client_reuses_its_configured_secret_and_tolerance() {
    let client = StripeClient::new(SECRET.to_vec(), 120);
    assert_eq!(client.webhook_secret(), SECRET);
    assert_eq!(client.tolerance_secs(), 120);
    let header = sign_stripe_payload(SECRET, PINNED_BODY, 5_000);
    assert_eq!(
        client
            .verify(PINNED_BODY, &header, 5_120)
            .expect("within tolerance"),
        VerifiedEvent { timestamp: 5_000 }
    );
    assert_eq!(
        client.verify(PINNED_BODY, &header, 5_121),
        Err(StripeSignatureError::TimestampOutOfTolerance {
            skew_secs: 121,
            tolerance_secs: 120
        })
    );
    assert_eq!(
        StripeClient::new(b"wrong".to_vec(), 120).verify(PINNED_BODY, &header, 5_000),
        Err(StripeSignatureError::NoMatchingSignature)
    );
}

#[test]
fn error_messages_name_the_failure_without_secret_material() {
    let header = format!("t=1000,v1={}", "00".repeat(32));
    let error = verify_stripe_signature(PINNED_BODY, &header, SECRET, 10, 1_000).expect_err("no");
    let rendered = error.to_string();
    assert!(rendered.contains("verification failed"), "{rendered}");
    assert!(!rendered.contains("whsec"), "{rendered}");
    assert!(!rendered.contains(PINNED_DIGEST), "{rendered}");
}

// ---------------------------------------------------------------------------
// Invoice construction
// ---------------------------------------------------------------------------

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
    let temp = TempDir::new("billing-proof");
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
        ))
        .expect("append change");
    assert!(matches!(
        BillingProofRecord::from_ledger_record(&change),
        Err(BillingError::Malformed(_))
    ));
}

// ---------------------------------------------------------------------------
// Form encoding
// ---------------------------------------------------------------------------

#[test]
fn form_encoding_is_pinned() {
    assert_eq!(encode_form_component("abcXYZ019-._~"), "abcXYZ019-._~");
    assert_eq!(encode_form_component("a b"), "a%20b");
    assert_eq!(encode_form_component("a+b&c=d"), "a%2Bb%26c%3Dd");
    assert_eq!(
        encode_form_component("stripe-item:pub:exp:2023-11"),
        "stripe-item%3Apub%3Aexp%3A2023-11"
    );
    assert_eq!(encode_form_component("caf\u{00e9}"), "caf%C3%A9");
}

// ---------------------------------------------------------------------------
// Stripe invoice-item client
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct MockTransport {
    responses: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl MockTransport {
    fn with(responses: Vec<Result<HttpResponse, TransportError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().expect("lock").clone()
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        self.requests.lock().expect("lock").push(req);
        self.responses
            .lock()
            .expect("lock")
            .pop_front()
            .expect("mock transport exhausted")
    }
}

fn ok_item(id: &str) -> HttpResponse {
    HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: format!("{{\"id\":\"{id}\",\"object\":\"invoiceitem\"}}").into_bytes(),
    }
}

fn error_response(status: u16) -> HttpResponse {
    HttpResponse {
        status,
        headers: Vec::new(),
        body: b"{\"error\":{\"type\":\"api_error\"}}".to_vec(),
    }
}

fn invoice_with(lines: Vec<InvoiceLine>) -> Invoice {
    let total_micros = lines.iter().map(|line| line.fee_micros).sum();
    Invoice {
        publisher_id: "pub-1".to_string(),
        period: period(),
        line_items: lines,
        total_micros,
    }
}

fn invoice_line(seq: u64, experiment: &str, fee_micros: i64) -> InvoiceLine {
    InvoiceLine {
        seq,
        experiment_id: experiment.to_string(),
        proven_lift: fee_micros * 5,
        rate_bp: 2_000,
        fee_micros,
    }
}

#[tokio::test]
async fn bill_invoice_posts_one_idempotent_item_per_line() {
    let transport = MockTransport::with(vec![Ok(ok_item("ii_1")), Ok(ok_item("ii_2"))]);
    let invoice = invoice_with(vec![
        invoice_line(1, "exp-a", 500_000),
        invoice_line(2, "exp-b", 150_000),
    ]);
    let receipt = bill_invoice("sk_test_123", &transport, &invoice)
        .await
        .expect("billing succeeds");
    assert_eq!(
        receipt
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["ii_1", "ii_2"]
    );
    assert_eq!(receipt.total_minor_units, 65);
    assert_eq!(receipt.publisher_id, "pub-1");
    assert_eq!(receipt.year_month, "2023-11");
    assert_eq!(receipt.currency, STRIPE_CURRENCY);

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    let first = &requests[0];
    assert_eq!(first.method, HttpMethod::Post);
    assert_eq!(first.url, format!("{STRIPE_API_BASE}/v1/invoiceitems"));
    assert_eq!(first.header("Authorization"), Some("Bearer sk_test_123"));
    assert_eq!(
        first.header("Idempotency-Key"),
        Some("stripe-item:pub-1:exp-a:2023-11")
    );
    assert_eq!(first.header("Content-Type"), Some("application/json"));
    let body = first.body.as_ref().expect("body");
    assert_eq!(body["amount"], 50); // 500_000 micros = 50 cents
    assert_eq!(body["currency"], STRIPE_CURRENCY);
    assert!(first.retryable(), "the idempotency key makes retries safe");
}

#[tokio::test]
async fn zero_invoice_sends_no_request_at_all() {
    let transport = MockTransport::default();
    let receipt = bill_invoice("sk_test_123", &transport, &invoice_with(Vec::new()))
        .await
        .expect("empty invoice");
    assert!(receipt.items.is_empty());
    assert_eq!(receipt.total_minor_units, 0);
    assert!(
        transport.requests().is_empty(),
        "no request for a zero invoice"
    );
}

#[tokio::test]
async fn zero_fee_lines_are_skipped_not_billed() {
    let transport = MockTransport::with(vec![Ok(ok_item("ii_1"))]);
    let invoice = invoice_with(vec![
        invoice_line(1, "exp-zero", 0),
        invoice_line(2, "exp-paid", 500_000),
    ]);
    let receipt = bill_invoice("sk_test_123", &transport, &invoice)
        .await
        .expect("billing succeeds");
    assert_eq!(
        receipt.items,
        vec![StripeItemResponse { id: "ii_1".into() }]
    );
    assert_eq!(receipt.total_minor_units, 50);
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn sub_cent_fees_refuse_the_invoice_before_any_request() {
    let transport = MockTransport::default();
    let invoice = invoice_with(vec![invoice_line(9, "exp-a", 500_001)]);
    assert_eq!(
        bill_invoice("sk_test_123", &transport, &invoice).await,
        Err(BillingError::SubCentAmount { seq: 9 })
    );
    assert!(
        transport.requests().is_empty(),
        "refusal happens before I/O"
    );
}

#[tokio::test]
async fn first_item_failure_surfaces_the_underlying_error() {
    for (status, expected) in [
        (401u16, BillingError::Auth { status: 401 }),
        (403, BillingError::Auth { status: 403 }),
        (500, BillingError::Http { status: 500 }),
        (402, BillingError::Http { status: 402 }),
    ] {
        let transport = MockTransport::with(vec![Ok(error_response(status))]);
        let invoice = invoice_with(vec![invoice_line(1, "exp-a", 500_000)]);
        assert_eq!(
            bill_invoice("sk_test_123", &transport, &invoice).await,
            Err(expected),
            "status {status}"
        );
    }
}

#[tokio::test]
async fn partial_application_names_what_was_created() {
    let transport = MockTransport::with(vec![Ok(ok_item("ii_1")), Ok(error_response(500))]);
    let invoice = invoice_with(vec![
        invoice_line(1, "exp-a", 500_000),
        invoice_line(2, "exp-b", 150_000),
    ]);
    assert_eq!(
        bill_invoice("sk_test_123", &transport, &invoice).await,
        Err(BillingError::StripePartial {
            completed: vec!["ii_1".to_string()],
            failed_seq: 2
        })
    );
}

#[tokio::test]
async fn network_and_schema_failures_are_typed() {
    let transport = MockTransport::with(vec![Err(TransportError::Network("reset".into()))]);
    let invoice = invoice_with(vec![invoice_line(1, "exp-a", 500_000)]);
    assert!(matches!(
        bill_invoice("sk_test_123", &transport, &invoice).await,
        Err(BillingError::Network { .. })
    ));

    let transport = MockTransport::with(vec![Ok(HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: b"not json".to_vec(),
    })]);
    assert!(matches!(
        bill_invoice("sk_test_123", &transport, &invoice).await,
        Err(BillingError::Schema { .. })
    ));
}

#[test]
fn line_items_convert_only_exact_cents() {
    let line = invoice_line(3, "exp-a", 123_450_000);
    let item = crate::stripe::InvoiceLineItem::from_invoice_line(&line, "pub-1", "2023-11")
        .expect("exact cents");
    assert_eq!(item.amount_minor_units, 12_345);
    assert_eq!(item.idempotency_key, "stripe-item:pub-1:exp-a:2023-11");
    assert_eq!(item.currency, STRIPE_CURRENCY);

    let off_cent = invoice_line(4, "exp-b", MICROS_PER_MINOR_UNIT - 1);
    assert_eq!(
        crate::stripe::InvoiceLineItem::from_invoice_line(&off_cent, "pub-1", "2023-11"),
        Err(BillingError::SubCentAmount { seq: 4 })
    );
    let negative = invoice_line(5, "exp-c", -MICROS_PER_MINOR_UNIT);
    assert!(matches!(
        crate::stripe::InvoiceLineItem::from_invoice_line(&negative, "pub-1", "2023-11"),
        Err(BillingError::TamperedProof { seq: 5, .. })
    ));
}

/// Unique temporary directory removed on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "apex-billing-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
