//! The provider-neutral billing domain.
//!
//! [`PaymentProvider`] is the seam between Apex's billing logic and the
//! payment service provider behind it. Everything downstream of this trait —
//! the durable intent saga in [`crate::intent`], the runner's monthly pass —
//! speaks only the neutral vocabulary defined here: invoice lines, payment
//! links, stored credentials, fetched payments, refunds. Mollie is the
//! implementation this deployment wires in ([`PaymentProvider for
//! MollieClient`]); a different PSP is a different `impl`, not a rewrite.
//!
//! Mollie-specific concepts are mapped, not leaked:
//!
//! - a *mandate* becomes a [`StoredCredential`] (`customerId:mandateId`
//!   encoded exactly as before, so stored rows keep working);
//! - a *first payment* (`sequenceType: "first"`) becomes
//!   [`PaymentSequence::First`], a plain checkout payment
//!   [`PaymentSequence::OneOff`];
//! - a fetched [`MolliePayment`] becomes a neutral [`ProviderPayment`]
//!   (with `mandateId` carried as `credential_id`), so the intent journal
//!   never names a Mollie concept.
//!
//! The trait is object-safe: callers hold `&dyn PaymentProvider` or
//! `Arc<dyn PaymentProvider>`.

use async_trait::async_trait;

use apex_connectors::transport::Transport;

use crate::errors::BillingError;
use crate::mollie::{
    InvoiceLineItem, MollieClient, MolliePayment, PaymentLinkResponse, PaymentReceipt,
    StoredMandate,
};

/// Which submission shape a [`PaymentProvider::create_payment_link`] call
/// asks the provider for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentSequence {
    /// A one-off checkout payment for the line.
    OneOff,
    /// A first payment that also stores the payer's credential, so later
    /// months charge [`PaymentProvider::charge_credential`] instead of
    /// opening checkout payments that expire within hours.
    First,
}

/// A stored payer credential a recurring charge runs against. The neutral
/// form of a Mollie mandate; the encoded `customerId:credentialId` form is
/// exactly the stored-mandate string the collection log already persists.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCredential {
    /// The provider customer the credential belongs to.
    pub customer_id: String,
    /// The provider credential id (Mollie mandate id).
    pub credential_id: String,
}

impl StoredCredential {
    /// Build a credential, refusing empty parts or a part that cannot be
    /// encoded (`:` is the part separator).
    pub fn new(customer_id: impl Into<String>, credential_id: impl Into<String>) -> Option<Self> {
        let customer_id = customer_id.into().trim().to_string();
        let credential_id = credential_id.into().trim().to_string();
        if customer_id.is_empty()
            || credential_id.is_empty()
            || customer_id.contains(':')
            || credential_id.contains(':')
        {
            return None;
        }
        Some(Self {
            customer_id,
            credential_id,
        })
    }

    /// Parse the encoded `customerId:credentialId` form.
    pub fn parse(value: &str) -> Option<Self> {
        let (customer_id, credential_id) = value.trim().split_once(':')?;
        Self::new(customer_id, credential_id)
    }

    /// Encode for the collection log's single stored-credential field.
    pub fn encode(&self) -> String {
        format!("{}:{}", self.customer_id, self.credential_id)
    }
}

impl From<StoredMandate> for StoredCredential {
    fn from(mandate: StoredMandate) -> Self {
        Self {
            customer_id: mandate.customer_id,
            credential_id: mandate.mandate_id,
        }
    }
}

impl From<StoredCredential> for StoredMandate {
    fn from(credential: StoredCredential) -> Self {
        StoredMandate {
            customer_id: credential.customer_id,
            mandate_id: credential.credential_id,
        }
    }
}

/// A fetched payment, reduced to the fields billing validates and records.
/// The provider-neutral record the intent journal and the webhook path work
/// with: no Mollie type, no Mollie status vocabulary, no Mollie id shape.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPayment {
    /// The provider payment id.
    pub id: String,
    /// The provider status string.
    pub status: String,
    /// Amount in minor units.
    pub amount_minor_units: i64,
    /// ISO currency code.
    pub currency: String,
    /// The deterministic billing reference carried in metadata, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_reference: Option<String>,
    /// The publisher reference carried in metadata, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_publisher: Option<String>,
    /// The metadata amount, when present, parsed to minor units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_amount_minor_units: Option<i64>,
    /// The payer customer the payment ran for, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,
    /// The stored credential the payment ran against, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    /// The submission kind the provider reports (`first`, `recurring`,
    /// `oneoff`), when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence_type: Option<String>,
    /// The refunded amount the provider reports against this payment, in
    /// minor units (DD). A payment keeps the collected status through a
    /// refund; this field is the reversal the books record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_refunded_minor_units: Option<i64>,
    /// The chargeback amount the provider reports against this payment, in
    /// minor units (DD). A payment keeps the collected status through a
    /// chargeback; this field is the reversal the books record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_charged_back_minor_units: Option<i64>,
}

impl From<MolliePayment> for ProviderPayment {
    fn from(payment: MolliePayment) -> Self {
        Self {
            id: payment.id,
            status: payment.status,
            amount_minor_units: payment.amount_minor_units,
            currency: payment.currency,
            metadata_reference: payment.metadata_reference,
            metadata_publisher: payment.metadata_publisher,
            metadata_amount_minor_units: payment.metadata_amount_minor_units,
            customer_id: payment.customer_id,
            credential_id: payment.mandate_id,
            sequence_type: payment.sequence_type,
            amount_refunded_minor_units: payment.amount_refunded_minor_units,
            amount_charged_back_minor_units: payment.amount_charged_back_minor_units,
        }
    }
}

/// The outcome of [`PaymentProvider::issue_refund`]: the provider's refund
/// record and the payment it drew money back from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefundReceipt {
    /// The provider refund id.
    pub refund_id: String,
    /// The payment the refund was issued against.
    pub payment_id: String,
    /// The refund status the provider reported (`pending`, `refunded`, ...).
    pub status: String,
    /// The refunded amount in minor units.
    pub amount_minor_units: i64,
    /// ISO currency code.
    pub currency: String,
}

/// The payment service provider behind Apex billing. Every method is a
/// network call over the shared [`Transport`] and every failure is a typed
/// [`BillingError`]; the implementations never retry at this layer.
#[async_trait]
pub trait PaymentProvider: Send + Sync {
    /// Create one checkout payment (or first payment) for one invoice line
    /// under an explicit idempotency key. A resumed submission reuses the
    /// persisted key, so the provider's deduplication window returns the
    /// original payment instead of opening a second one.
    async fn create_payment_link(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        idempotency_key: &str,
        sequence: PaymentSequence,
    ) -> Result<PaymentLinkResponse, BillingError>;

    /// Create one payment link for a whole invoice's total: the durable,
    /// never-expiring URL the invoice email and the console hand the
    /// customer. The deterministic reference is the idempotency key.
    async fn create_invoice_payment_link(
        &self,
        transport: &dyn Transport,
        publisher_id: &str,
        year_month: &str,
        amount_minor_units: i64,
        currency: &str,
        description: &str,
    ) -> Result<PaymentLinkResponse, BillingError>;

    /// Charge a stored credential (`sequenceType: "recurring"` at Mollie).
    /// A definitively refused charge is the typed
    /// [`BillingError::StoredChargeRefused`] the caller falls back from;
    /// uncertain failures propagate (a possibly-landed charge must never be
    /// retried through a link).
    async fn charge_credential(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        credential: &StoredCredential,
        idempotency_key: &str,
    ) -> Result<PaymentReceipt, BillingError>;

    /// Fetch one payment by its provider id. The webhook's only source of
    /// truth: a delivery body is never trusted over the fetched object.
    async fn fetch_payment(
        &self,
        transport: &dyn Transport,
        payment_id: &str,
    ) -> Result<ProviderPayment, BillingError>;

    /// Every payment made against one payment link, oldest first as the
    /// provider reports them. The dunning sweep and the reconciliation pass
    /// use it to see whether a durable link has since been paid.
    async fn fetch_payment_link_payments(
        &self,
        transport: &dyn Transport,
        link_id: &str,
    ) -> Result<Vec<ProviderPayment>, BillingError>;

    /// Issue a refund for a collected payment. The receipt is durable
    /// evidence a dispute audit can cite; a refused or malformed refund
    /// response is a typed refusal, never a silently assumed refund.
    async fn issue_refund(
        &self,
        transport: &dyn Transport,
        payment_id: &str,
        amount_minor_units: i64,
        description: &str,
    ) -> Result<RefundReceipt, BillingError>;
}

#[async_trait]
impl PaymentProvider for MollieClient {
    async fn create_payment_link(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        idempotency_key: &str,
        sequence: PaymentSequence,
    ) -> Result<PaymentLinkResponse, BillingError> {
        match sequence {
            PaymentSequence::OneOff => {
                self.create_payment_link_key(transport, item, idempotency_key)
                    .await
            }
            PaymentSequence::First => {
                self.create_first_payment_key(transport, item, idempotency_key)
                    .await
            }
        }
    }

    async fn create_invoice_payment_link(
        &self,
        transport: &dyn Transport,
        publisher_id: &str,
        year_month: &str,
        amount_minor_units: i64,
        currency: &str,
        description: &str,
    ) -> Result<PaymentLinkResponse, BillingError> {
        self.create_invoice_payment_link(
            transport,
            publisher_id,
            year_month,
            amount_minor_units,
            currency,
            description,
        )
        .await
    }

    async fn charge_credential(
        &self,
        transport: &dyn Transport,
        item: &InvoiceLineItem,
        credential: &StoredCredential,
        idempotency_key: &str,
    ) -> Result<PaymentReceipt, BillingError> {
        self.charge_stored_payment_method_key(
            transport,
            item,
            &credential.customer_id,
            &credential.credential_id,
            idempotency_key,
        )
        .await
    }

    async fn fetch_payment(
        &self,
        transport: &dyn Transport,
        payment_id: &str,
    ) -> Result<ProviderPayment, BillingError> {
        let payment = MollieClient::fetch_payment(self, transport, payment_id).await?;
        Ok(ProviderPayment::from(payment))
    }

    async fn fetch_payment_link_payments(
        &self,
        transport: &dyn Transport,
        link_id: &str,
    ) -> Result<Vec<ProviderPayment>, BillingError> {
        let payments = MollieClient::fetch_payment_link_payments(self, transport, link_id).await?;
        Ok(payments.into_iter().map(ProviderPayment::from).collect())
    }

    async fn issue_refund(
        &self,
        transport: &dyn Transport,
        payment_id: &str,
        amount_minor_units: i64,
        description: &str,
    ) -> Result<RefundReceipt, BillingError> {
        MollieClient::issue_refund(self, transport, payment_id, amount_minor_units, description)
            .await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use apex_connectors::transport::{
        HttpBody, HttpRequest, HttpResponse, Transport, TransportError,
    };
    use async_trait::async_trait;

    use super::*;
    use crate::intent::{
        collect_invoice_with_intents, PaymentIntent, PaymentIntentState, PaymentIntentStore,
    };
    use crate::invoice::{BillingPeriod, Invoice, InvoiceLine};
    use crate::vat::VatTreatment;

    const REFERENCE: &str = "apex-fee:pub-1:exp-a:2026-10";

    fn client() -> MollieClient {
        MollieClient::new(
            "key",
            "https://api.mollie.test",
            "https://apex.test/v1/mollie/webhook",
            "https://apex.test/watch/return",
        )
        .expect("client")
    }

    fn payment_body(id: &str, status: &str) -> String {
        format!(
            r#"{{"id":"{id}","status":"{status}","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"{REFERENCE}"}}}}"#
        )
    }

    /// A provider that speaks only the neutral trait: no Mollie types, no
    /// transport. The neutrality test below drives the whole intent saga
    /// through it.
    #[derive(Default)]
    struct NeutralFake {
        links: Mutex<VecDeque<PaymentLinkResponse>>,
        invoices: Mutex<VecDeque<PaymentLinkResponse>>,
        charges: Mutex<VecDeque<PaymentReceipt>>,
        fetched: Mutex<VecDeque<ProviderPayment>>,
        refunds: Mutex<VecDeque<RefundReceipt>>,
        submitted: Mutex<Vec<(String, PaymentSequence)>>,
    }

    impl NeutralFake {
        fn with_checkout_link() -> Self {
            let fake = Self::default();
            fake.links.lock().unwrap().push_back(PaymentLinkResponse {
                id: "pay_1".into(),
                url: "https://provider.test/pay_1".into(),
                customer_id: None,
            });
            fake.invoices
                .lock()
                .unwrap()
                .push_back(PaymentLinkResponse {
                    id: "link_1".into(),
                    url: "https://provider.test/link_1".into(),
                    customer_id: None,
                });
            fake
        }
    }

    #[async_trait]
    impl PaymentProvider for NeutralFake {
        async fn create_payment_link(
            &self,
            _transport: &dyn Transport,
            item: &InvoiceLineItem,
            idempotency_key: &str,
            sequence: PaymentSequence,
        ) -> Result<PaymentLinkResponse, BillingError> {
            self.submitted
                .lock()
                .unwrap()
                .push((idempotency_key.to_string(), sequence));
            self.links
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| BillingError::Malformed("no scripted link".into()))
                .map(|mut link| {
                    link.id = format!("pay_{}", item.seq);
                    link
                })
        }

        async fn create_invoice_payment_link(
            &self,
            _transport: &dyn Transport,
            _publisher_id: &str,
            _year_month: &str,
            _amount_minor_units: i64,
            _currency: &str,
            _description: &str,
        ) -> Result<PaymentLinkResponse, BillingError> {
            self.invoices
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| BillingError::Malformed("no scripted invoice link".into()))
        }

        async fn charge_credential(
            &self,
            _transport: &dyn Transport,
            _item: &InvoiceLineItem,
            _credential: &StoredCredential,
            _idempotency_key: &str,
        ) -> Result<PaymentReceipt, BillingError> {
            self.charges
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| BillingError::Malformed("no scripted charge".into()))
        }

        async fn fetch_payment(
            &self,
            _transport: &dyn Transport,
            payment_id: &str,
        ) -> Result<ProviderPayment, BillingError> {
            self.fetched
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| BillingError::Malformed("no scripted payment".into()))
                .map(|mut payment| {
                    payment.id = payment_id.to_string();
                    payment
                })
        }

        async fn fetch_payment_link_payments(
            &self,
            _transport: &dyn Transport,
            link_id: &str,
        ) -> Result<Vec<ProviderPayment>, BillingError> {
            Ok(vec![ProviderPayment {
                id: format!("{link_id}-p1"),
                status: "paid".into(),
                amount_minor_units: 125,
                currency: "USD".into(),
                metadata_reference: Some(REFERENCE.into()),
                metadata_publisher: None,
                metadata_amount_minor_units: None,
                customer_id: None,
                credential_id: None,
                sequence_type: None,
                amount_refunded_minor_units: None,
                amount_charged_back_minor_units: None,
            }])
        }

        async fn issue_refund(
            &self,
            _transport: &dyn Transport,
            payment_id: &str,
            amount_minor_units: i64,
            _description: &str,
        ) -> Result<RefundReceipt, BillingError> {
            self.refunds
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| BillingError::Malformed("no scripted refund".into()))
                .map(|mut refund| {
                    refund.payment_id = payment_id.to_string();
                    refund.amount_minor_units = amount_minor_units;
                    refund
                })
        }
    }

    fn invoice() -> Invoice {
        Invoice {
            number: 1,
            publisher_id: "pub-1".into(),
            period: BillingPeriod {
                year_month: "2026-10".into(),
                start_unix_ms: 0,
                end_unix_ms: 1_000,
            },
            line_items: vec![InvoiceLine {
                seq: 1,
                experiment_id: "exp-a".into(),
                proven_lift: 2_500_000,
                rate_bp: 2_000,
                fee_micros: 1_250_000,
                vat_micros: 0,
                gross_micros: 1_250_000,
                holdout: None,
            }],
            total_micros: 1_250_000,
            vat_rate_bp: 0,
            vat_micros: 0,
            gross_micros: 1_250_000,
            customer: None,
            vat_treatment: VatTreatment::NotApplied,
            vies_evidence: None,
        }
    }

    fn temp_store(label: &str) -> (std::path::PathBuf, PaymentIntentStore) {
        let dir = std::env::temp_dir().join(format!(
            "apex-provider-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("intents.jsonl");
        let store = PaymentIntentStore::open(&path).expect("open");
        (dir, store)
    }

    #[tokio::test]
    async fn the_intent_saga_runs_against_any_payment_provider() {
        // The whole collection drives a provider that is not Mollie and has
        // no transport: intent.rs speaks the trait, not the client type.
        let (dir, store) = temp_store("neutral");
        let invoice = invoice();
        let provider = NeutralFake::with_checkout_link();
        let receipt =
            collect_invoice_with_intents(&provider, &NullTransport, &store, &invoice, None, 1_000)
                .await
                .expect("collect through the neutral provider");
        assert_eq!(receipt.lines.len(), 1);
        assert_eq!(
            receipt.lines[0]
                .payment_link
                .as_ref()
                .map(|l| l.url.as_str()),
            Some("https://provider.test/pay_1")
        );
        assert_eq!(
            receipt.invoice_link.as_ref().map(|l| l.id.as_str()),
            Some("link_1"),
            "the invoice-level link came from the neutral provider too"
        );
        let submitted = provider.submitted.lock().unwrap();
        assert_eq!(submitted.len(), 1);
        assert_eq!(
            submitted[0].0, REFERENCE,
            "the deterministic reference is the idempotency key"
        );
        assert_eq!(
            submitted[0].1,
            PaymentSequence::First,
            "no stored credential means a first payment"
        );
        let intent = store
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .expect("present");
        assert_eq!(intent.state, PaymentIntentState::SubmittedPending);
        assert_eq!(intent.provider_payment_id.as_deref(), Some("pay_1"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_neutral_provider_fetches_and_refunds() {
        let provider = NeutralFake::with_checkout_link();
        provider.fetched.lock().unwrap().push_back(ProviderPayment {
            id: "pay_1".into(),
            status: "paid".into(),
            amount_minor_units: 125,
            currency: "USD".into(),
            metadata_reference: Some(REFERENCE.into()),
            metadata_publisher: None,
            metadata_amount_minor_units: None,
            customer_id: None,
            credential_id: None,
            sequence_type: None,
            amount_refunded_minor_units: None,
            amount_charged_back_minor_units: None,
        });
        provider.refunds.lock().unwrap().push_back(RefundReceipt {
            refund_id: "ref_1".into(),
            payment_id: String::new(),
            status: "pending".into(),
            amount_minor_units: 0,
            currency: "USD".into(),
        });

        let payment = PaymentProvider::fetch_payment(&provider, &NullTransport, "pay_1")
            .await
            .expect("fetch");
        assert_eq!(payment.id, "pay_1");
        assert_eq!(payment.status, "paid");
        let link_payments =
            PaymentProvider::fetch_payment_link_payments(&provider, &NullTransport, "link_1")
                .await
                .expect("link payments");
        assert_eq!(link_payments.len(), 1);
        assert_eq!(
            link_payments[0].metadata_reference.as_deref(),
            Some(REFERENCE)
        );
        let refund =
            PaymentProvider::issue_refund(&provider, &NullTransport, "pay_1", 125, "goodwill")
                .await
                .expect("refund");
        assert_eq!(refund.payment_id, "pay_1");
        assert_eq!(refund.amount_minor_units, 125);
    }

    /// The Mollie implementation maps its own vocabulary into the neutral
    /// records: mandate -> credential, fetched JSON -> ProviderPayment.
    #[test]
    fn mollie_maps_into_the_neutral_records() {
        let payment = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(&payment_body("tr_1", "paid"))
                .expect("json"),
        )
        .expect("payment");
        let neutral = ProviderPayment::from(payment);
        assert_eq!(neutral.id, "tr_1");
        assert_eq!(neutral.status, "paid");
        assert_eq!(neutral.metadata_reference.as_deref(), Some(REFERENCE));

        // DD: the reversal amounts travel into the neutral record.
        let reversed = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(
                r#"{"id":"tr_1","status":"paid","amount":{"currency":"USD","value":"1.25"},"metadata":{"reference":"apex-fee:pub-1:exp-a:2026-10"},"amountRefunded":{"currency":"USD","value":"0.25"},"amountChargedBack":{"currency":"USD","value":"0.50"}}"#,
            )
            .expect("json"),
        )
        .expect("payment");
        let neutral = ProviderPayment::from(reversed);
        assert_eq!(neutral.amount_refunded_minor_units, Some(25));
        assert_eq!(neutral.amount_charged_back_minor_units, Some(50));

        let credential =
            StoredCredential::from(StoredMandate::parse("cus_1:mnd_1").expect("mandate"));
        assert_eq!(credential.customer_id, "cus_1");
        assert_eq!(credential.credential_id, "mnd_1");
        assert_eq!(credential.encode(), "cus_1:mnd_1");
        assert_eq!(
            StoredMandate::from(credential),
            StoredMandate::parse("cus_1:mnd_1").expect("mandate")
        );
        assert!(StoredCredential::parse("no-separator").is_none());
        assert!(
            StoredCredential::parse("a:b:c").is_none(),
            "a part with a separator is refused"
        );
    }

    /// A transport fake for the two new Mollie endpoints.
    #[derive(Default)]
    struct EndpointFake {
        script: Mutex<VecDeque<HttpResponse>>,
        calls: Mutex<Vec<HttpRequest>>,
    }

    /// A transport the neutral fake never touches (it fakes at the trait
    /// boundary, not the HTTP boundary).
    struct NullTransport;

    #[async_trait]
    impl Transport for NullTransport {
        async fn request(&self, _req: HttpRequest) -> Result<HttpResponse, TransportError> {
            Err(TransportError::Network(
                "no transport behind the neutral fake".into(),
            ))
        }
    }

    #[async_trait]
    impl Transport for EndpointFake {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.calls.lock().unwrap().push(req);
            self.script
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| TransportError::Network("no scripted response".into()))
        }
    }

    fn scripted(body: &str) -> EndpointFake {
        let fake = EndpointFake::default();
        fake.script.lock().unwrap().push_back(HttpResponse {
            status: 200,
            headers: vec![],
            body: body.as_bytes().to_vec(),
        });
        fake
    }

    #[tokio::test]
    async fn mollie_fetches_the_payments_of_one_payment_link() {
        let fake = scripted(
            r#"{"count":2,"_embedded":{"payments":[
                {"id":"tr_1","status":"paid","amount":{"currency":"USD","value":"1.25"},"metadata":{"reference":"apex-fee:pub-1:exp-a:2026-10"}},
                {"id":"tr_2","status":"open","amount":{"currency":"USD","value":"0.75"}}
            ]}}"#,
        );
        let payments = client()
            .fetch_payment_link_payments(&fake, "pl_1")
            .await
            .expect("payments");
        assert_eq!(payments.len(), 2);
        assert_eq!(payments[0].id, "tr_1");
        assert_eq!(payments[0].status, "paid");
        assert_eq!(payments[1].amount_minor_units, 75);
        {
            let calls = fake.calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(
                calls[0].url,
                "https://api.mollie.test/v2/payment-links/pl_1/payments"
            );
            assert_eq!(calls[0].method.as_str(), "GET");
        }

        // Fail closed: a link-shaped id is required, and a response without
        // the embedded list is a typed refusal, never an empty list.
        assert!(client()
            .fetch_payment_link_payments(&fake, "tr_1")
            .await
            .is_err());
        let empty = scripted(r#"{"count":0}"#);
        assert!(
            client()
                .fetch_payment_link_payments(&empty, "pl_1")
                .await
                .is_err(),
            "a missing payments list is malformed"
        );
    }

    #[tokio::test]
    async fn mollie_issues_a_refund_and_validates_the_receipt() {
        let fake = scripted(
            r#"{"resource":"refund","id":"re_1","status":"pending","amount":{"currency":"USD","value":"1.25"}}"#,
        );
        let receipt = client()
            .issue_refund(&fake, "tr_1", 125, "invoice corrected")
            .await
            .expect("refund");
        assert_eq!(receipt.refund_id, "re_1");
        assert_eq!(receipt.payment_id, "tr_1");
        assert_eq!(receipt.status, "pending");
        assert_eq!(receipt.amount_minor_units, 125);
        assert_eq!(receipt.currency, "USD");
        {
            let calls = fake.calls.lock().unwrap();
            assert_eq!(
                calls[0].url,
                "https://api.mollie.test/v2/payments/tr_1/refunds"
            );
            assert_eq!(calls[0].method.as_str(), "POST");
            let idempotency = calls[0]
                .headers
                .iter()
                .find(|(name, _)| name == "Idempotency-Key")
                .map(|(_, value)| value.clone());
            assert_eq!(
                idempotency.as_deref(),
                Some("refund:tr_1:125"),
                "the refund is idempotent on (payment, amount)"
            );
            match calls[0].body.as_ref() {
                Some(HttpBody::Json(value)) => {
                    assert_eq!(value["amount"]["value"], "1.25");
                    assert_eq!(value["amount"]["currency"], "USD");
                    assert_eq!(value["description"], "invoice corrected");
                }
                other => panic!("expected a JSON refund body, got {other:?}"),
            }
        }

        // Local refusals before any request: bad payment id, non-positive
        // amount, unusable description.
        assert!(client()
            .issue_refund(&fake, "nope", 125, "d")
            .await
            .is_err());
        assert!(client().issue_refund(&fake, "tr_1", 0, "d").await.is_err());
        assert!(client()
            .issue_refund(&fake, "tr_1", 125, " ")
            .await
            .is_err());

        // A receipt that disagrees with the request is a typed refusal.
        let mismatch = scripted(
            r#"{"id":"re_1","status":"pending","amount":{"currency":"USD","value":"9.99"}}"#,
        );
        assert!(client()
            .issue_refund(&mismatch, "tr_1", 125, "d")
            .await
            .is_err());
    }

    #[test]
    fn a_link_id_is_shape_checked_like_a_payment_id() {
        assert!(crate::mollie::is_payment_link_id("pl_abc123"));
        assert!(!crate::mollie::is_payment_link_id("tr_abc123"));
        assert!(!crate::mollie::is_payment_link_id("pl_"));
        assert!(!crate::mollie::is_payment_link_id(""));
    }
}
