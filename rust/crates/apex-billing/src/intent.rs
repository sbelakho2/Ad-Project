//! Durable Apex-owned payment intents.
//!
//! The provider is never the first place a charge exists. Before any Mollie
//! call the invoice line's intent is persisted (and fsynced) as [`Issued`];
//! immediately before the provider request it is persisted again as
//! [`SubmittedUnknown`] with the attempt count advanced. The provider payment
//! id is persisted as soon as the response is parsed. A crash at any point
//! therefore leaves a durable fact on which recovery can reason:
//!
//! - crash before the provider call: the intent is `Issued` and the next pass
//!   submits it;
//! - crash after the provider id was received but before it was persisted: the
//!   intent is `SubmittedUnknown`; a same-request replay with the documented
//!   one-hour `Idempotency-Key` window (or a known provider id) resolves it —
//!   recovery never creates a second charge silently;
//! - crash after the provider id was persisted: the intent carries the id and
//!   the next pass fetches that payment instead of creating another.
//!
//! [`SubmittedPending`]: PaymentIntentState::SubmittedPending
//! [`SubmittedUnknown`]: PaymentIntentState::SubmittedUnknown
//! [`Issued`]: PaymentIntentState::Issued
//!
//! ## Idempotency-Key is not exactly-once
//!
//! Mollie's `Idempotency-Key` is a short-window safety feature: the provider
//! documents it as a one-hour deduplication window. It is used here *only* as
//! a bounded replay mechanism for an intent that was already marked submitted.
//! It is never treated as proof that a charge did or did not happen; outside
//! the window an unresolved intent fails closed as
//! [`BillingError::PaymentIntentUncertain`].
//!
//! ## Journal
//!
//! The intent journal is append-only JSONL (one full intent snapshot per
//! transition). Opening it is fail-closed: an unreadable or corrupt journal is
//! a typed error, never a silently empty store. Every append is flushed with
//! `sync_all` before the method returns, so a caller never sees an
//! acknowledgement that only lives in a buffer.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::errors::BillingError;
use crate::invoice::Invoice;
use crate::mollie::{
    CollectionChannel, InvoiceLineItem, LineCollection, MollieClient, MolliePayment,
    PaymentLinkResponse, StoredMandate,
};
use apex_connectors::transport::Transport;

/// The provider's documented `Idempotency-Key` deduplication window.
pub const IDEMPOTENCY_WINDOW_MS: i64 = 3_600_000;

/// The default intent journal beside the invoice register.
pub const DEFAULT_INTENT_PATH: &str = ".apex/billing/payment-intents.jsonl";

/// The durable lifecycle of one invoice line's charge.
///
/// `SubmittedUnknown` means the request may or may not have reached Mollie;
/// `SubmittedPending` means the provider acknowledged it with a payment id but
/// has not collected yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaymentIntentState {
    /// Persisted before any provider call: the charge is ours to submit.
    Issued,
    /// The provider acknowledged a payment id; it is not collected yet.
    SubmittedPending,
    /// The provider request was started but its outcome is not durable.
    SubmittedUnknown,
    /// The provider reports the payment as collected.
    Paid,
    /// The provider refused the payment.
    Failed,
    /// The provider reports the payment as refunded.
    Refunded,
}

impl PaymentIntentState {
    /// Whether no further provider submission may change this state.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Paid | Self::Failed | Self::Refunded)
    }
}

/// Which Mollie operation an intent submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaymentIntentChannel {
    /// A stored mandate charge (`sequenceType: "recurring"`).
    StoredMandate,
    /// A one-off checkout payment link.
    PaymentLink,
}

/// One durable invoice-line charge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentIntent {
    /// Apex-owned unique intent id (derived from the deterministic line
    /// reference, so it is stable across passes).
    pub intent_id: String,
    /// The invoice the line belongs to (`INV-<month>-<publisher>`).
    pub invoice_reference: String,
    /// The deterministic provider reference (`apex-fee:<publisher>:...`).
    pub line_reference: String,
    pub publisher_id: String,
    pub year_month: String,
    pub seq: u64,
    /// Amount in minor units (cents) sent to Mollie.
    pub amount_minor_units: i64,
    /// ISO currency code sent to Mollie.
    pub currency: String,
    pub state: PaymentIntentState,
    /// The durable Mollie payment id, once the response was parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_payment_id: Option<String>,
    /// The last provider status recorded for that payment id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_status: Option<String>,
    /// When Apex first recorded the intent.
    pub created_unix_ms: i64,
    /// When the intent last changed.
    pub updated_unix_ms: i64,
    /// Provider submissions attempted for this line.
    #[serde(default)]
    pub attempts: u32,
    /// The channel of the current submission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<PaymentIntentChannel>,
    /// The stored-mandate customer id for a recurring submission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,
    /// The stored mandate id for a recurring submission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mandate_id: Option<String>,
    /// When the current submission started (the idempotency-window anchor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_unix_ms: Option<i64>,
}

impl PaymentIntent {
    /// The stable intent id for a deterministic provider reference.
    #[must_use]
    pub fn id_for_reference(reference: &str) -> String {
        format!("pi:{reference}")
    }

    /// Build a new `Issued` intent for one invoice line.
    pub fn issued(
        invoice: &Invoice,
        invoice_reference: &str,
        item: &InvoiceLineItem,
        now_unix_ms: i64,
    ) -> Self {
        Self {
            intent_id: Self::id_for_reference(&item.reference),
            invoice_reference: invoice_reference.to_string(),
            line_reference: item.reference.clone(),
            publisher_id: invoice.publisher_id.clone(),
            year_month: invoice.period.year_month.clone(),
            seq: item.seq,
            amount_minor_units: item.amount_minor_units,
            currency: item.currency.clone(),
            state: PaymentIntentState::Issued,
            provider_payment_id: None,
            provider_status: None,
            created_unix_ms: now_unix_ms,
            updated_unix_ms: now_unix_ms,
            attempts: 0,
            channel: None,
            customer_id: None,
            mandate_id: None,
            submitted_unix_ms: None,
        }
    }

    fn immutable_fields_match(&self, other: &Self) -> bool {
        self.intent_id == other.intent_id
            && self.line_reference == other.line_reference
            && self.invoice_reference == other.invoice_reference
            && self.publisher_id == other.publisher_id
            && self.year_month == other.year_month
            && self.seq == other.seq
            && self.amount_minor_units == other.amount_minor_units
            && self.currency == other.currency
    }
}

/// An append-only, fail-closed intent journal with an in-memory latest-state
/// index.
#[derive(Debug)]
pub struct PaymentIntentStore {
    path: PathBuf,
    state: Mutex<IntentIndex>,
    append: Mutex<()>,
}

#[derive(Debug, Default)]
struct IntentIndex {
    by_id: HashMap<String, PaymentIntent>,
    by_reference: HashMap<String, String>,
    by_provider: HashMap<String, String>,
}

impl PaymentIntentStore {
    /// Open (or create) the journal. Missing is empty; unreadable or corrupt
    /// is a typed refusal (never a silently empty store).
    pub fn open(path: &Path) -> Result<Self, BillingError> {
        let mut index = IntentIndex::default();
        match std::fs::read(path) {
            Ok(bytes) => {
                let text = String::from_utf8(bytes).map_err(|error| {
                    BillingError::PaymentIntentStoreCorrupt {
                        detail: format!("{} is not UTF-8: {error}", path.display()),
                    }
                })?;
                for (line_number, line) in text.lines().enumerate() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let intent: PaymentIntent = serde_json::from_str(line).map_err(|error| {
                        BillingError::PaymentIntentStoreCorrupt {
                            detail: format!(
                                "{} line {} is corrupt: {error}",
                                path.display(),
                                line_number + 1
                            ),
                        }
                    })?;
                    index.observe(intent).map_err(|error| {
                        BillingError::PaymentIntentStoreCorrupt {
                            detail: format!("{} line {}: {error}", path.display(), line_number + 1),
                        }
                    })?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(BillingError::PaymentIntentStoreCorrupt {
                    detail: format!("{} could not be read: {error}", path.display()),
                })
            }
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            state: Mutex::new(index),
            append: Mutex::new(()),
        })
    }

    /// The journal path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One intent by id.
    pub fn get(&self, intent_id: &str) -> Result<Option<PaymentIntent>, BillingError> {
        let state = self.state()?;
        Ok(state.by_id.get(intent_id).cloned())
    }

    /// Every intent for one deterministic line reference.
    pub fn intents_for_reference(
        &self,
        line_reference: &str,
    ) -> Result<Vec<PaymentIntent>, BillingError> {
        let state = self.state()?;
        Ok(state
            .by_reference
            .iter()
            .filter(|(reference, _)| reference.as_str() == line_reference)
            .filter_map(|(_, id)| state.by_id.get(id).cloned())
            .collect())
    }

    /// The intent holding a Mollie payment id, if any.
    pub fn intent_for_provider_payment(
        &self,
        payment_id: &str,
    ) -> Result<Option<PaymentIntent>, BillingError> {
        let state = self.state()?;
        Ok(state
            .by_provider
            .get(payment_id)
            .and_then(|id| state.by_id.get(id))
            .cloned())
    }

    /// A submitted intent for the same line that already carries a provider
    /// payment id (the recovery search: no second charge is created when
    /// another durable attempt knows the provider id).
    pub fn submitted_with_provider(
        &self,
        line_reference: &str,
        excluding: &str,
    ) -> Result<Option<PaymentIntent>, BillingError> {
        Ok(self
            .intents_for_reference(line_reference)?
            .into_iter()
            .filter(|intent| intent.intent_id != excluding)
            .find(|intent| {
                matches!(
                    intent.state,
                    PaymentIntentState::SubmittedPending
                        | PaymentIntentState::SubmittedUnknown
                        | PaymentIntentState::Paid
                        | PaymentIntentState::Failed
                        | PaymentIntentState::Refunded
                ) && intent.provider_payment_id.is_some()
            }))
    }

    /// Ensure the durable `Issued` intent exists before any provider call.
    /// Replaying the same line returns the existing intent unchanged; a
    /// different amount/reference for the same id is a typed conflict.
    pub fn ensure_issued(
        &self,
        invoice: &Invoice,
        invoice_reference: &str,
        item: &InvoiceLineItem,
        now_unix_ms: i64,
    ) -> Result<PaymentIntent, BillingError> {
        let candidate = PaymentIntent::issued(invoice, invoice_reference, item, now_unix_ms);
        let existing = {
            let state = self.state()?;
            state.by_id.get(&candidate.intent_id).cloned()
        };
        match existing {
            Some(existing) => {
                if !existing.immutable_fields_match(&candidate) {
                    return Err(BillingError::PaymentIntentConflict {
                        detail: format!(
                            "intent {} was issued for a different line, amount or currency",
                            existing.intent_id
                        ),
                    });
                }
                Ok(existing)
            }
            None => {
                self.persist(candidate.clone())?;
                Ok(candidate)
            }
        }
    }

    /// Persist the start of a provider submission: state becomes
    /// `SubmittedUnknown`, the attempt count advances, and the channel/mandate
    /// used is recorded so an uncertain outcome can be replayed exactly.
    /// This append is fsynced *before* the caller performs the provider call.
    pub fn begin_submission(
        &self,
        intent_id: &str,
        channel: PaymentIntentChannel,
        customer_id: Option<&str>,
        mandate_id: Option<&str>,
        now_unix_ms: i64,
    ) -> Result<PaymentIntent, BillingError> {
        let mut intent = self.required(intent_id)?;
        if matches!(
            intent.state,
            PaymentIntentState::Paid
                | PaymentIntentState::Refunded
                | PaymentIntentState::SubmittedPending
        ) {
            return Err(BillingError::PaymentIntentConflict {
                detail: format!(
                    "intent {intent_id} is {} and cannot submit again",
                    state_label(intent.state)
                ),
            });
        }
        intent.state = PaymentIntentState::SubmittedUnknown;
        intent.channel = Some(channel);
        intent.customer_id = customer_id.map(str::to_string);
        intent.mandate_id = mandate_id.map(str::to_string);
        intent.attempts = intent.attempts.saturating_add(1);
        intent.submitted_unix_ms = Some(now_unix_ms);
        intent.updated_unix_ms = now_unix_ms;
        self.persist(intent.clone())?;
        Ok(intent)
    }

    /// Persist the provider payment id and its status after a parsed response.
    /// A completed state never regresses to a pending or failed state when a
    /// reordered delivery is applied later.
    pub fn record_provider(
        &self,
        intent_id: &str,
        provider_payment_id: Option<&str>,
        provider_status: &str,
        now_unix_ms: i64,
    ) -> Result<PaymentIntent, BillingError> {
        let mut intent = self.required(intent_id)?;
        let next = state_from_provider_status(provider_status);
        if let Some(existing) = intent.provider_payment_id.as_deref() {
            if let Some(incoming) = provider_payment_id {
                // A failed recurring payment may be followed by the typed
                // checkout fallback, which has its own provider id. Any other
                // id change is a conflict.
                if existing != incoming && intent.state != PaymentIntentState::Failed {
                    return Err(BillingError::PaymentIntentConflict {
                        detail: format!(
                            "intent {intent_id} already carries provider payment {existing}, refusing {incoming}"
                        ),
                    });
                }
            }
        }
        if let Some(payment_id) = provider_payment_id {
            intent.provider_payment_id = Some(payment_id.to_string());
        }
        let merged = merge_state(intent.state, next);
        // A reordered pending/failed delivery never overwrites the completed
        // status that is the durable truth.
        if merged == next || merged == PaymentIntentState::Refunded {
            intent.provider_status = Some(provider_status.to_string());
        }
        intent.state = merged;
        intent.updated_unix_ms = now_unix_ms;
        self.persist(intent.clone())?;
        Ok(intent)
    }

    /// Apply a fetched Mollie payment to the intent that owns it (by provider
    /// id, or by its deterministic reference when the id was not persisted).
    /// Returns `None` when no intent matches. Replays and reordered deliveries
    /// are safe: terminal states never regress.
    pub fn reconcile_payment(
        &self,
        payment: &MolliePayment,
        now_unix_ms: i64,
    ) -> Result<Option<PaymentIntent>, BillingError> {
        let intent = match self.intent_for_provider_payment(&payment.id)? {
            Some(intent) => Some(intent),
            None => {
                let reference = payment.metadata_reference.as_deref().unwrap_or_default();
                if reference.is_empty() {
                    None
                } else {
                    let state = self.state()?;
                    state
                        .by_reference
                        .get(reference)
                        .and_then(|id| state.by_id.get(id))
                        .cloned()
                }
            }
        };
        let Some(intent) = intent else {
            return Ok(None);
        };
        self.record_provider(
            &intent.intent_id,
            Some(&payment.id),
            &payment.status,
            now_unix_ms,
        )
        .map(Some)
    }

    fn required(&self, intent_id: &str) -> Result<PaymentIntent, BillingError> {
        self.get(intent_id)?
            .ok_or_else(|| BillingError::PaymentIntentConflict {
                detail: format!("intent {intent_id} is not recorded"),
            })
    }

    fn persist(&self, intent: PaymentIntent) -> Result<(), BillingError> {
        let _guard = self
            .append
            .lock()
            .map_err(|_| BillingError::PaymentIntentStoreCorrupt {
                detail: "intent append lock poisoned".into(),
            })?;
        let mut line = serde_json::to_string(&intent).map_err(|error| {
            BillingError::PaymentIntentStoreCorrupt {
                detail: format!("intent serialization: {error}"),
            }
        })?;
        line.push('\n');
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        let mut state = self.state()?;
        state
            .observe(intent)
            .map_err(|detail| BillingError::PaymentIntentStoreCorrupt {
                detail: format!("intent index: {detail}"),
            })?;
        Ok(())
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, IntentIndex>, BillingError> {
        self.state
            .lock()
            .map_err(|_| BillingError::PaymentIntentStoreCorrupt {
                detail: "intent index poisoned".into(),
            })
    }
}

impl IntentIndex {
    fn observe(&mut self, intent: PaymentIntent) -> Result<(), String> {
        if let Some(existing) = self.by_id.get(&intent.intent_id) {
            if !existing.immutable_fields_match(&intent) {
                return Err(format!(
                    "intent {} changed its immutable line identity",
                    intent.intent_id
                ));
            }
        }
        if let Some(payment_id) = intent.provider_payment_id.as_deref() {
            if let Some(owner) = self.by_provider.get(payment_id) {
                if owner != &intent.intent_id {
                    return Err(format!(
                        "provider payment {payment_id} is recorded for two intents ({owner}, {})",
                        intent.intent_id
                    ));
                }
            }
            self.by_provider
                .insert(payment_id.to_string(), intent.intent_id.clone());
        }
        self.by_reference
            .insert(intent.line_reference.clone(), intent.intent_id.clone());
        self.by_id.insert(intent.intent_id.clone(), intent);
        Ok(())
    }
}

fn state_label(state: PaymentIntentState) -> &'static str {
    match state {
        PaymentIntentState::Issued => "issued",
        PaymentIntentState::SubmittedPending => "submittedPending",
        PaymentIntentState::SubmittedUnknown => "submittedUnknown",
        PaymentIntentState::Paid => "paid",
        PaymentIntentState::Failed => "failed",
        PaymentIntentState::Refunded => "refunded",
    }
}

fn state_from_provider_status(status: &str) -> PaymentIntentState {
    match status {
        "paid" => PaymentIntentState::Paid,
        "refunded" | "charged_back" => PaymentIntentState::Refunded,
        "failed" | "canceled" | "expired" => PaymentIntentState::Failed,
        "open" | "pending" | "authorized" => PaymentIntentState::SubmittedPending,
        other => {
            if other.trim().is_empty() {
                PaymentIntentState::SubmittedUnknown
            } else {
                PaymentIntentState::SubmittedPending
            }
        }
    }
}

/// Merge a fetched provider status into the durable state without regressing
/// a completed payment. `Paid` may become `Refunded`; a reordered pending or
/// failed delivery never turns a paid intent back.
fn merge_state(current: PaymentIntentState, next: PaymentIntentState) -> PaymentIntentState {
    match (current, next) {
        (PaymentIntentState::Refunded, _) => PaymentIntentState::Refunded,
        (PaymentIntentState::Paid, PaymentIntentState::SubmittedPending)
        | (PaymentIntentState::Paid, PaymentIntentState::SubmittedUnknown)
        | (PaymentIntentState::Paid, PaymentIntentState::Failed) => PaymentIntentState::Paid,
        (_, next) => next,
    }
}

/// Collect one invoice through durable intents: every non-zero line is
/// recorded as `Issued` before its provider call, and an already submitted
/// intent is resumed or fetched instead of charged again.
pub async fn collect_invoice_with_intents(
    client: &MollieClient,
    transport: &dyn Transport,
    store: &PaymentIntentStore,
    invoice: &Invoice,
    stored_mandate: Option<&str>,
    now_unix_ms: i64,
) -> Result<crate::mollie::CollectionReceipt, BillingError> {
    let invoice_reference = format!("INV-{}-{}", invoice.period.year_month, invoice.publisher_id);
    let mut lines = Vec::new();
    let mut total_minor_units = 0i64;
    for line in &invoice.line_items {
        let billable = if line.gross_micros > 0 {
            line.gross_micros
        } else {
            line.fee_micros
        };
        if billable == 0 {
            continue;
        }
        let item = InvoiceLineItem::from_invoice_line(
            line,
            &invoice.publisher_id,
            &invoice.period.year_month,
        )?;
        let intent = store.ensure_issued(invoice, &invoice_reference, &item, now_unix_ms)?;
        let collected = collect_line_with_intent(
            client,
            transport,
            store,
            &intent,
            &item,
            stored_mandate,
            now_unix_ms,
        )
        .await?;
        total_minor_units = total_minor_units
            .checked_add(item.amount_minor_units)
            .ok_or(BillingError::Overflow)?;
        lines.push(collected);
    }
    Ok(crate::mollie::CollectionReceipt {
        publisher_id: invoice.publisher_id.clone(),
        year_month: invoice.period.year_month.clone(),
        currency: crate::mollie::BILLING_CURRENCY.to_string(),
        total_minor_units,
        lines,
    })
}

/// Resume one line's durable intent. A `Paid`/`Refunded` intent returns its
/// durable evidence without a provider call; a `SubmittedPending` intent fetches
/// its provider payment; an `Issued` intent submits; an unresolved
/// `SubmittedUnknown` intent is fetched or replayed within the documented
/// idempotency window and otherwise fails closed.
#[allow(clippy::too_many_arguments)]
async fn collect_line_with_intent(
    client: &MollieClient,
    transport: &dyn Transport,
    store: &PaymentIntentStore,
    intent: &PaymentIntent,
    item: &InvoiceLineItem,
    stored_mandate: Option<&str>,
    now_unix_ms: i64,
) -> Result<LineCollection, BillingError> {
    match intent.state {
        PaymentIntentState::Paid | PaymentIntentState::Refunded => Ok(line_from_intent(
            intent,
            item,
            format!("intent already {}", state_label(intent.state)),
        )),
        PaymentIntentState::SubmittedPending => {
            let Some(payment_id) = intent.provider_payment_id.as_deref() else {
                return Err(BillingError::PaymentIntentUncertain {
                    intent_id: intent.intent_id.clone(),
                });
            };
            let payment = client.fetch_payment(transport, payment_id).await?;
            let updated = store.reconcile_payment(&payment, now_unix_ms)?;
            Ok(line_from_intent(
                updated.as_ref().unwrap_or(intent),
                item,
                "fetched the durable submitted payment".to_string(),
            ))
        }
        PaymentIntentState::SubmittedUnknown => {
            resolve_unknown(
                client,
                transport,
                store,
                intent,
                item,
                stored_mandate,
                now_unix_ms,
            )
            .await
        }
        PaymentIntentState::Issued | PaymentIntentState::Failed => {
            submit(
                client,
                transport,
                store,
                intent,
                item,
                stored_mandate,
                None,
                now_unix_ms,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_unknown(
    client: &MollieClient,
    transport: &dyn Transport,
    store: &PaymentIntentStore,
    intent: &PaymentIntent,
    item: &InvoiceLineItem,
    stored_mandate: Option<&str>,
    now_unix_ms: i64,
) -> Result<LineCollection, BillingError> {
    if let Some(payment_id) = intent.provider_payment_id.as_deref() {
        let payment = client.fetch_payment(transport, payment_id).await?;
        let updated = store.reconcile_payment(&payment, now_unix_ms)?;
        return Ok(line_from_intent(
            updated.as_ref().unwrap_or(intent),
            item,
            "fetched the durable payment id".to_string(),
        ));
    }
    if let Some(other) = store.submitted_with_provider(&item.reference, &intent.intent_id)? {
        if let Some(payment_id) = other.provider_payment_id.as_deref() {
            let payment = client.fetch_payment(transport, payment_id).await?;
            let updated = store.reconcile_payment(&payment, now_unix_ms)?;
            return Ok(line_from_intent(
                updated.as_ref().unwrap_or(&other),
                item,
                "fetched a known submitted intent for this line".to_string(),
            ));
        }
    }
    let within_window = intent
        .submitted_unix_ms
        .and_then(|submitted| now_unix_ms.checked_sub(submitted))
        .is_some_and(|elapsed| (0..=IDEMPOTENCY_WINDOW_MS).contains(&elapsed));
    if !within_window {
        return Err(BillingError::PaymentIntentUncertain {
            intent_id: intent.intent_id.clone(),
        });
    }
    // Same provider request, same Idempotency-Key: a bounded replay. This is
    // not a second charge attempt and is the only recovery available when the
    // id was received but the process died before persisting it.
    submit(
        client,
        transport,
        store,
        intent,
        item,
        stored_mandate,
        Some(intent.channel.unwrap_or(PaymentIntentChannel::PaymentLink)),
        now_unix_ms,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn submit(
    client: &MollieClient,
    transport: &dyn Transport,
    store: &PaymentIntentStore,
    intent: &PaymentIntent,
    item: &InvoiceLineItem,
    stored_mandate: Option<&str>,
    force_channel: Option<PaymentIntentChannel>,
    now_unix_ms: i64,
) -> Result<LineCollection, BillingError> {
    let mandate = stored_mandate.and_then(StoredMandate::parse);
    let (channel, customer_id, mandate_id) = match force_channel {
        Some(PaymentIntentChannel::PaymentLink) => (PaymentIntentChannel::PaymentLink, None, None),
        Some(PaymentIntentChannel::StoredMandate) => {
            let customer = intent
                .customer_id
                .clone()
                .or_else(|| mandate.as_ref().map(|m| m.customer_id.clone()));
            let mandate_id = intent
                .mandate_id
                .clone()
                .or_else(|| mandate.as_ref().map(|m| m.mandate_id.clone()));
            match (customer, mandate_id) {
                (Some(customer), Some(mandate_id)) => (
                    PaymentIntentChannel::StoredMandate,
                    Some(customer),
                    Some(mandate_id),
                ),
                _ => {
                    return Err(BillingError::PaymentIntentUncertain {
                        intent_id: intent.intent_id.clone(),
                    })
                }
            }
        }
        None => match mandate {
            Some(mandate) => (
                PaymentIntentChannel::StoredMandate,
                Some(mandate.customer_id),
                Some(mandate.mandate_id),
            ),
            None => (PaymentIntentChannel::PaymentLink, None, None),
        },
    };
    let started = store.begin_submission(
        &intent.intent_id,
        channel,
        customer_id.as_deref(),
        mandate_id.as_deref(),
        now_unix_ms,
    )?;
    match channel {
        PaymentIntentChannel::StoredMandate => {
            let (Some(customer_id), Some(mandate_id)) = (customer_id, mandate_id) else {
                return Err(BillingError::PaymentIntentUncertain {
                    intent_id: intent.intent_id.clone(),
                });
            };
            match client
                .charge_stored_payment_method(transport, item, &customer_id, &mandate_id)
                .await
            {
                Ok(receipt) => {
                    store.record_provider(
                        &started.intent_id,
                        Some(&receipt.payment_id),
                        &receipt.status,
                        now_unix_ms,
                    )?;
                    let mut line = line_from_intent(&started, item, String::new());
                    line.channel = CollectionChannel::StoredMandate;
                    line.payment_id = Some(receipt.payment_id);
                    line.status = Some(receipt.status);
                    line.payment_link = None;
                    line.fallback_reason = None;
                    Ok(line)
                }
                Err(BillingError::StoredChargeRefused { detail, payment_id }) => {
                    // Definitive refusal: record it, then the typed fallback
                    // creates a link. The refusal is never silent.
                    store.record_provider(
                        &started.intent_id,
                        payment_id.as_deref(),
                        "failed",
                        now_unix_ms,
                    )?;
                    // The fallback link is its own provider submission: mark
                    // it unknown before the call so an uncertain link failure
                    // is recovered, never re-created blind.
                    let link_started = store.begin_submission(
                        &started.intent_id,
                        PaymentIntentChannel::PaymentLink,
                        None,
                        None,
                        now_unix_ms,
                    )?;
                    let link = client.create_payment_link(transport, item).await?;
                    store.record_provider(
                        &link_started.intent_id,
                        Some(&link.id),
                        "open",
                        now_unix_ms,
                    )?;
                    let mut line = line_from_intent(&link_started, item, detail);
                    line.channel = CollectionChannel::PaymentLink;
                    line.payment_id = None;
                    line.status = None;
                    line.payment_link = Some(link);
                    Ok(line)
                }
                Err(other) => Err(other),
            }
        }
        PaymentIntentChannel::PaymentLink => {
            let link = client.create_payment_link(transport, item).await?;
            store.record_provider(&started.intent_id, Some(&link.id), "open", now_unix_ms)?;
            let mut line = line_from_intent(&started, item, String::new());
            line.channel = CollectionChannel::PaymentLink;
            line.payment_id = None;
            line.status = None;
            line.payment_link = Some(link);
            Ok(line)
        }
    }
}

fn line_from_intent(
    intent: &PaymentIntent,
    item: &InvoiceLineItem,
    fallback_reason: String,
) -> LineCollection {
    let channel = match intent.channel {
        Some(PaymentIntentChannel::StoredMandate) => CollectionChannel::StoredMandate,
        _ => CollectionChannel::PaymentLink,
    };
    LineCollection {
        seq: item.seq,
        reference: item.reference.clone(),
        amount_minor_units: item.amount_minor_units,
        currency: item.currency.clone(),
        channel,
        payment_id: intent.provider_payment_id.clone(),
        status: intent.provider_status.clone(),
        payment_link: None::<PaymentLinkResponse>,
        fallback_reason: (!fallback_reason.is_empty()).then_some(fallback_reason),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use apex_connectors::transport::{HttpRequest, HttpResponse, Transport, TransportError};
    use async_trait::async_trait;

    use super::*;
    use crate::invoice::{BillingPeriod, Invoice, InvoiceLine};
    use crate::vat::VatTreatment;

    const REFERENCE: &str = "apex-fee:pub-1:exp-a:2026-10";

    #[derive(Default)]
    struct ProviderFake {
        script: Mutex<VecDeque<Result<HttpResponse, TransportError>>>,
        /// Idempotency-Key -> cached provider response body.
        idempotent: Mutex<HashMap<String, String>>,
        /// Provider payment id -> response body.
        payments: Mutex<HashMap<String, String>>,
        calls: Mutex<Vec<(String, String)>>,
        creates: Mutex<HashMap<String, u64>>,
    }

    impl ProviderFake {
        fn with_payment() -> Self {
            let fake = Self::default();
            fake.script
                .lock()
                .unwrap()
                .push_back(Ok(unchecked_body("tr_1", "open", REFERENCE)));
            fake
        }

        fn with_timeout() -> Self {
            let fake = Self::default();
            fake.script
                .lock()
                .unwrap()
                .push_back(Err(TransportError::Timeout("provider stalled".into())));
            fake
        }

        fn create_count(&self, key: &str) -> u64 {
            self.creates.lock().unwrap().get(key).copied().unwrap_or(0)
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    fn unchecked_body(id: &str, status: &str, reference: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![],
            body: format!(
                r#"{{"id":"{id}","status":"{status}","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"{reference}","publisher":"publisher-pub-1","amount":"1.25"}},"_links":{{"checkout":{{"href":"https://pay.test/{id}"}}}}}}"#
            )
            .into_bytes(),
        }
    }

    #[async_trait]
    impl Transport for ProviderFake {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.calls
                .lock()
                .unwrap()
                .push((req.method.as_str().to_string(), req.url.clone()));
            let key = req
                .headers
                .iter()
                .find(|(name, _)| name == "Idempotency-Key")
                .map(|(_, value)| value.clone());
            if req.method == apex_connectors::transport::HttpMethod::Get {
                let id = req.url.rsplit('/').next().unwrap_or_default();
                if let Some(body) = self.payments.lock().unwrap().get(id) {
                    let mut value: serde_json::Value = serde_json::from_str(body)
                        .map_err(|_| TransportError::Network("bad cached body".into()))?;
                    value["status"] = serde_json::Value::String("paid".into());
                    return Ok(HttpResponse {
                        status: 200,
                        headers: vec![],
                        body: value.to_string().into_bytes(),
                    });
                }
                return Ok(HttpResponse {
                    status: 404,
                    headers: vec![],
                    body: br#"{"status":404}"#.to_vec(),
                });
            }
            if let Some(key) = key.as_ref() {
                if let Some(body) = self.idempotent.lock().unwrap().get(key) {
                    return Ok(HttpResponse {
                        status: 200,
                        headers: vec![],
                        body: body.clone().into_bytes(),
                    });
                }
                *self.creates.lock().unwrap().entry(key.clone()).or_default() += 1;
            }
            let response = self
                .script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(unchecked_body("tr_x", "open", REFERENCE)))?;
            if let Some(key) = key.as_deref() {
                if let Ok(text) = std::str::from_utf8(&response.body) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
                        if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
                            self.payments
                                .lock()
                                .unwrap()
                                .insert(id.to_string(), text.to_string());
                        }
                    }
                    self.idempotent.lock().unwrap().insert(
                        key.to_string(),
                        String::from_utf8_lossy(&response.body).into_owned(),
                    );
                }
            }
            Ok(response)
        }
    }

    fn item() -> InvoiceLineItem {
        InvoiceLineItem {
            seq: 1,
            experiment_id: "exp-a".into(),
            amount_minor_units: 125,
            currency: "USD".into(),
            customer_reference: "publisher-pub-1".into(),
            reference: REFERENCE.into(),
            description: "Apex proven-lift fee".into(),
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

    fn client() -> MollieClient {
        MollieClient::new(
            "key",
            "https://api.mollie.test",
            "https://apex.test/v1/mollie/webhook",
            "https://apex.test/watch/return",
        )
        .expect("client")
    }

    fn temp_store(label: &str) -> (std::path::PathBuf, PaymentIntentStore) {
        let dir = std::env::temp_dir().join(format!(
            "apex-intent-{}-{label}-{}",
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
    async fn crash_before_provider_call_resumes_the_durable_issued_intent() {
        let (dir, store) = temp_store("before-call");
        let invoice = invoice();
        let issued = store
            .ensure_issued(&invoice, "INV-2026-10-pub-1", &item(), 1_000)
            .expect("issued");
        assert_eq!(issued.state, PaymentIntentState::Issued);
        drop(store);

        // A new process reopens the journal and finds the issued intent.
        let reopened = PaymentIntentStore::open(&dir.join("intents.jsonl")).expect("reopen");
        let fake = ProviderFake::with_payment();
        let receipt =
            collect_invoice_with_intents(&client(), &fake, &reopened, &invoice, None, 2_000)
                .await
                .expect("collect");
        assert_eq!(receipt.lines.len(), 1);
        assert_eq!(fake.create_count(REFERENCE), 1, "exactly one charge");
        let intent = reopened
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .expect("present");
        assert_eq!(intent.state, PaymentIntentState::SubmittedPending);
        assert_eq!(intent.provider_payment_id.as_deref(), Some("tr_1"));
        assert_eq!(intent.attempts, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_provider_id_received_before_a_crash_is_recovered_by_same_key_replay() {
        let (dir, store) = temp_store("after-id");
        let invoice = invoice();
        let intent = store
            .ensure_issued(&invoice, "INV-2026-10-pub-1", &item(), 1_000)
            .expect("issued");
        store
            .begin_submission(
                &intent.intent_id,
                PaymentIntentChannel::PaymentLink,
                None,
                None,
                1_000,
            )
            .expect("begin");
        // The provider accepted the charge, then the process died before the
        // id could be persisted.
        let fake = ProviderFake::with_payment();
        let first = client()
            .create_payment_link(&fake, &item())
            .await
            .expect("provider accepted");
        assert_eq!(first.id, "tr_1");
        assert_eq!(fake.create_count(REFERENCE), 1);

        let receipt = collect_invoice_with_intents(&client(), &fake, &store, &invoice, None, 1_500)
            .await
            .expect("recover");
        assert!(receipt.lines[0].payment_link.is_some());
        assert_eq!(
            fake.create_count(REFERENCE),
            1,
            "the replay reused the idempotency key; no second charge"
        );
        let recovered = store.get(&intent.intent_id).expect("get").expect("present");
        assert_eq!(recovered.provider_payment_id.as_deref(), Some("tr_1"));
        assert_eq!(recovered.state, PaymentIntentState::SubmittedPending);
        assert_eq!(recovered.attempts, 2, "the replay is a durable attempt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_persisted_provider_id_is_fetched_not_charged_again() {
        let (dir, store) = temp_store("after-persist");
        let invoice = invoice();
        let fake = ProviderFake::with_payment();
        collect_invoice_with_intents(&client(), &fake, &store, &invoice, None, 1_000)
            .await
            .expect("collect");
        assert_eq!(fake.create_count(REFERENCE), 1);

        // A rerun before the webhook arrives must fetch the durable payment.
        let reopened = PaymentIntentStore::open(&dir.join("intents.jsonl")).expect("reopen");
        let receipt =
            collect_invoice_with_intents(&client(), &fake, &reopened, &invoice, None, 2_000)
                .await
                .expect("resume");
        assert_eq!(fake.create_count(REFERENCE), 1, "no second creation");
        assert!(fake.call_count() >= 2, "the durable payment was fetched");
        let intent = reopened
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .expect("present");
        assert_eq!(intent.state, PaymentIntentState::Paid);
        assert_eq!(intent.provider_payment_id.as_deref(), Some("tr_1"));
        assert_eq!(receipt.total_minor_units, 125);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_uncertain_timeout_outside_the_window_refuses_a_second_charge() {
        let (dir, store) = temp_store("timeout");
        let invoice = invoice();
        let fake = ProviderFake::with_timeout();
        let error = collect_invoice_with_intents(&client(), &fake, &store, &invoice, None, 1_000)
            .await
            .expect_err("uncertain");
        assert!(matches!(error, BillingError::Network { .. }), "{error:?}");
        let intent = store
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .expect("present");
        assert_eq!(intent.state, PaymentIntentState::SubmittedUnknown);
        assert!(intent.provider_payment_id.is_none());
        assert_eq!(fake.create_count(REFERENCE), 1);

        // After the documented one-hour window with no durable id, recovery
        // fails closed: no request is sent and no second charge exists.
        let error = collect_invoice_with_intents(
            &client(),
            &fake,
            &store,
            &invoice,
            None,
            1_000 + IDEMPOTENCY_WINDOW_MS + 1,
        )
        .await
        .expect_err("must refuse");
        assert!(
            matches!(error, BillingError::PaymentIntentUncertain { .. }),
            "{error:?}"
        );
        assert_eq!(fake.create_count(REFERENCE), 1, "no second charge");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn replayed_and_reordered_webhooks_never_regress_a_paid_intent() {
        let (dir, store) = temp_store("reordered");
        let invoice = invoice();
        let intent = store
            .ensure_issued(&invoice, "INV-2026-10-pub-1", &item(), 1_000)
            .expect("issued");
        store
            .begin_submission(
                &intent.intent_id,
                PaymentIntentChannel::PaymentLink,
                None,
                None,
                1_000,
            )
            .expect("begin");
        let paid = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(&format!(
                r#"{{"id":"tr_1","status":"paid","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"{REFERENCE}"}}}}"#
            ))
            .expect("json"),
        )
        .expect("payment");
        store
            .reconcile_payment(&paid, 2_000)
            .expect("paid")
            .expect("known");
        // A replayed paid delivery.
        store
            .reconcile_payment(&paid, 2_001)
            .expect("replay")
            .expect("known");
        for status in ["open", "pending", "failed"] {
            let reordered = MolliePayment::from_json(
                &serde_json::from_str::<serde_json::Value>(&format!(
                    r#"{{"id":"tr_1","status":"{status}","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"{REFERENCE}"}}}}"#
                ))
                .expect("json"),
            )
            .expect("payment");
            store
                .reconcile_payment(&reordered, 3_000)
                .expect("reordered")
                .expect("known");
            let intent = store.get(&intent.intent_id).expect("get").expect("present");
            assert_eq!(intent.state, PaymentIntentState::Paid, "status {status}");
            assert_eq!(intent.provider_status.as_deref(), Some("paid"));
        }
        let refunded = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(&format!(
                r#"{{"id":"tr_1","status":"refunded","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"{REFERENCE}"}}}}"#
            ))
            .expect("json"),
        )
        .expect("payment");
        store
            .reconcile_payment(&refunded, 4_000)
            .expect("refunded")
            .expect("known");
        let intent = store.get(&intent.intent_id).expect("get").expect("present");
        assert_eq!(intent.state, PaymentIntentState::Refunded);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_intents_are_idempotent_and_conflicts_are_refused() {
        let (dir, store) = temp_store("duplicate");
        let invoice = invoice();
        let first = store
            .ensure_issued(&invoice, "INV-2026-10-pub-1", &item(), 1_000)
            .expect("first");
        let second = store
            .ensure_issued(&invoice, "INV-2026-10-pub-1", &item(), 2_000)
            .expect("idempotent");
        assert_eq!(first, second);
        assert_eq!(
            std::fs::read_to_string(dir.join("intents.jsonl"))
                .expect("journal")
                .lines()
                .count(),
            1,
            "an issued replay appends nothing"
        );

        let mut changed = item();
        changed.amount_minor_units = 999;
        assert!(matches!(
            store.ensure_issued(&invoice, "INV-2026-10-pub-1", &changed, 3_000),
            Err(BillingError::PaymentIntentConflict { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_or_corrupt_intent_journals_fail_closed() {
        let dir = std::env::temp_dir().join(format!("apex-intent-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let corrupt = dir.join("corrupt.jsonl");
        std::fs::write(&corrupt, "not json\n").expect("write");
        assert!(matches!(
            PaymentIntentStore::open(&corrupt),
            Err(BillingError::PaymentIntentStoreCorrupt { .. })
        ));
        // A directory in place of the journal is unreadable, not empty.
        assert!(matches!(
            PaymentIntentStore::open(&dir),
            Err(BillingError::PaymentIntentStoreCorrupt { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
