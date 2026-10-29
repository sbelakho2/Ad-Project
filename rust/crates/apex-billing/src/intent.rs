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
//! The intent journal is append-only JSONL: one full intent snapshot per
//! transition, plus the invoice-level payment-link record E7 appends once
//! per invoice (`apex-invoice:...`) and the invoice-level payment record the
//! webhook appends when the link collects (E7). Opening it is fail-closed:
//! an unreadable or corrupt journal is a typed error, never a silently empty
//! store. Every append is flushed with `sync_all` before the method returns,
//! so a caller never sees an acknowledgement that only lives in a buffer.
//!
//! ## Reload across processes
//!
//! The journal is shared: the monthly run appends intents in one process
//! while the API's webhook handler reads them in another. A store opened
//! before those appends is stale. [`PaymentIntentStore::reload`] compares the
//! file's length and mtime against the stamp cached at the last read and
//! rebuilds the in-memory index when another process appended. The webhook
//! path calls it explicitly (in [`PaymentIntentStore::reconcile_payment`]);
//! the read paths (`state`, `get`) deliberately do not, because a reload that
//! raced `persist` could momentarily hide the just-persisted snapshot.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::errors::BillingError;
use crate::invoice::Invoice;
use crate::mollie::{CollectionChannel, InvoiceLineItem, LineCollection, PaymentLinkResponse};
use crate::provider::{PaymentProvider, PaymentSequence, ProviderPayment, StoredCredential};
use crate::registry::{InvoiceRegistry, PROVIDER_MOLLIE};
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
    /// A first payment (`sequenceType: "first"`) that stores the mandate
    /// later months charge. Replays reuse the exact recorded channel so a
    /// recovery can never demote a first payment to a plain checkout.
    FirstPayment,
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
    /// The shopper-facing checkout URL of the current link submission, kept
    /// so a resumed pass can still hand the customer the link it created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_url: Option<String>,
    /// The Idempotency-Key of the current submission: plain line reference
    /// for a first attempt, `#retry<n>` after a definitive failure. A replay
    /// reuses the persisted key, so it can never open a second payment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
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
            checkout_url: None,
            idempotency_key: None,
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

/// One invoice-level Mollie payment link, appended to the intent journal
/// after collection creates it. Unlike a checkout payment the link does not
/// expire within hours, so it is the durable URL the invoice email, the
/// console and the dunning sweep hand the customer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLinkRecord {
    /// The invoice the link pays (`INV-<month>-<publisher>`).
    pub invoice_reference: String,
    pub publisher_id: String,
    pub year_month: String,
    /// The Mollie payment-link id (`pl_...`).
    pub payment_link_id: String,
    /// The shopper-facing payment URL.
    pub url: String,
    /// The invoice total the link is for, in minor units.
    pub amount_minor_units: i64,
    /// ISO currency code.
    pub currency: String,
    /// When the link was recorded, unix milliseconds.
    pub created_unix_ms: i64,
}

/// One invoice-level payment against the durable payment link, appended to
/// the intent journal when the webhook records it (E7). A payment-link
/// payment carries no line reference, so it can never attach to a line
/// intent; this invoice-level record is what durably marks the invoice paid
/// and stops the dunning sweep from chasing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoicePaymentRecord {
    /// The invoice the payment settles (`INV-<month>-<publisher>`).
    pub invoice_reference: String,
    pub publisher_id: String,
    pub year_month: String,
    /// The Mollie payment id (`tr_...`) that collected against the link.
    pub payment_id: String,
    /// The fetched provider status (`paid`, `refunded`, ...).
    pub status: String,
    /// The collected amount in minor units.
    pub amount_minor_units: i64,
    /// ISO currency code.
    pub currency: String,
    /// When the payment was recorded, unix milliseconds.
    pub created_unix_ms: i64,
}

impl InvoicePaymentRecord {
    /// Whether this record collected the invoice: the provider reported the
    /// collected status for the payment-link payment.
    #[must_use]
    pub fn is_paid(&self) -> bool {
        self.status == crate::mollie::COLLECTED_PAYMENT_STATUS
    }
}

/// A journal line: a line-charge intent snapshot, an invoice payment-link
/// record, or an invoice-level payment (E7). Untagged, oldest format first:
/// the three shapes carry disjoint required fields (an intent has
/// `intentId`/`seq`/`state`, a link record has `paymentLinkId`/`url`, an
/// invoice payment has `paymentId`/`status`), so none can parse as another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum JournalEntry {
    InvoiceLink(InvoiceLinkRecord),
    InvoicePayment(InvoicePaymentRecord),
    Intent(PaymentIntent),
}

/// An append-only, fail-closed intent journal with an in-memory latest-state
/// index.
///
/// When it is opened with an [`InvoiceRegistry`]
/// ([`PaymentIntentStore::open_with_registry`]) the provider-payment and
/// per-line lookups go through the registry's SQLite UNIQUE indexes instead
/// of the JSONL file scan; the journal itself stays the append-only durable
/// record and is unchanged.
#[derive(Debug)]
pub struct PaymentIntentStore {
    path: PathBuf,
    state: Mutex<IntentIndex>,
    append: Mutex<()>,
    /// The file stamp (length + mtime) the in-memory index reflects. A
    /// `reload` that observes a different stamp rebuilds the index, so a
    /// store opened before another process appended still sees the appends.
    stamp: Mutex<FileStamp>,
    /// The transactional lookup index shared across processes, when wired.
    /// `None` keeps the store purely JSONL-backed (identical behavior).
    registry: Option<Arc<InvoiceRegistry>>,
}

/// The observed file shape a cached index was built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

impl FileStamp {
    fn of(path: &Path) -> Result<Self, BillingError> {
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(Self {
                len: metadata.len(),
                modified: metadata.modified().ok(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                len: 0,
                modified: None,
            }),
            Err(error) => Err(BillingError::PaymentIntentStoreCorrupt {
                detail: format!("{} could not be read: {error}", path.display()),
            }),
        }
    }
}

#[derive(Debug, Default)]
struct IntentIndex {
    by_id: HashMap<String, PaymentIntent>,
    by_reference: HashMap<String, String>,
    by_provider: HashMap<String, String>,
    invoice_links: HashMap<String, InvoiceLinkRecord>,
    invoice_payments: HashMap<String, InvoicePaymentRecord>,
}

/// Parse the whole journal into a fresh index. Shared by `open` and `reload`
/// so both apply exactly the same fail-closed rules.
fn parse_journal(path: &Path) -> Result<IntentIndex, BillingError> {
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
                let entry: JournalEntry = serde_json::from_str(line).map_err(|error| {
                    BillingError::PaymentIntentStoreCorrupt {
                        detail: format!(
                            "{} line {} is corrupt: {error}",
                            path.display(),
                            line_number + 1
                        ),
                    }
                })?;
                match entry {
                    JournalEntry::Intent(intent) => index.observe(intent).map_err(|error| {
                        BillingError::PaymentIntentStoreCorrupt {
                            detail: format!("{} line {}: {error}", path.display(), line_number + 1),
                        }
                    })?,
                    JournalEntry::InvoiceLink(record) => {
                        index.observe_link(record).map_err(|error| {
                            BillingError::PaymentIntentStoreCorrupt {
                                detail: format!(
                                    "{} line {}: {error}",
                                    path.display(),
                                    line_number + 1
                                ),
                            }
                        })?;
                    }
                    JournalEntry::InvoicePayment(record) => {
                        index.observe_payment(record).map_err(|error| {
                            BillingError::PaymentIntentStoreCorrupt {
                                detail: format!(
                                    "{} line {}: {error}",
                                    path.display(),
                                    line_number + 1
                                ),
                            }
                        })?;
                    }
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(BillingError::PaymentIntentStoreCorrupt {
                detail: format!("{} could not be read: {error}", path.display()),
            })
        }
    }
    Ok(index)
}

impl PaymentIntentStore {
    /// Open (or create) the journal. Missing is empty; unreadable or corrupt
    /// is a typed refusal (never a silently empty store).
    pub fn open(path: &Path) -> Result<Self, BillingError> {
        Self::open_inner(path, None)
    }

    /// Open (or create) the journal with a transactional [`InvoiceRegistry`]
    /// beside it: provider-payment and per-line lookups resolve through the
    /// registry's SQLite UNIQUE indexes first and only fall back to the
    /// JSONL-derived index. The journal remains the durable record; the
    /// registry is the shared lookup index.
    pub fn open_with_registry(
        path: &Path,
        registry: Arc<InvoiceRegistry>,
    ) -> Result<Self, BillingError> {
        Self::open_inner(path, Some(registry))
    }

    fn open_inner(
        path: &Path,
        registry: Option<Arc<InvoiceRegistry>>,
    ) -> Result<Self, BillingError> {
        let index = parse_journal(path)?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            state: Mutex::new(index),
            append: Mutex::new(()),
            stamp: Mutex::new(FileStamp::of(path)?),
            registry,
        })
    }

    /// The journal path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Re-read the journal when another process appended to it: the file's
    /// length and mtime are compared against the stamp the in-memory index
    /// was built from, and a change rebuilds the index from the file (the
    /// journal is the source of truth, so a rebuild can never lose a local
    /// append that was not yet flushed — `persist` holds the same lock).
    /// Returns whether the index was rebuilt. This is the call the webhook
    /// path makes before looking intents up; it is deliberately *not* wired
    /// into the read paths, where a rebuild racing `persist` could hide the
    /// just-persisted snapshot.
    pub fn reload(&self) -> Result<bool, BillingError> {
        {
            let stamp = self.stamp()?;
            if *stamp == FileStamp::of(&self.path)? {
                return Ok(false);
            }
        }
        // Serialize rebuilds against appends: while the append lock is held
        // no local persist can interleave, and the rebuilt stamp is re-read
        // after the lock so a concurrent writer's bytes are all included.
        let _guard = self
            .append
            .lock()
            .map_err(|_| BillingError::PaymentIntentStoreCorrupt {
                detail: "intent append lock poisoned".into(),
            })?;
        let stamp = FileStamp::of(&self.path)?;
        {
            let cached = self.stamp()?;
            if *cached == stamp {
                return Ok(false);
            }
        }
        let index = parse_journal(&self.path)?;
        {
            let mut state = self.state()?;
            *state = index;
        }
        *self.stamp()? = stamp;
        Ok(true)
    }

    /// One intent by id.
    pub fn get(&self, intent_id: &str) -> Result<Option<PaymentIntent>, BillingError> {
        let state = self.state()?;
        Ok(state.by_id.get(intent_id).cloned())
    }

    /// Every intent's latest state (the journal's durable snapshot).
    pub fn intents(&self) -> Result<Vec<PaymentIntent>, BillingError> {
        let state = self.state()?;
        Ok(state.by_id.values().cloned().collect())
    }

    /// The recorded invoice-level payment link of one invoice, when any.
    pub fn invoice_link_for(
        &self,
        invoice_reference: &str,
    ) -> Result<Option<InvoiceLinkRecord>, BillingError> {
        let state = self.state()?;
        Ok(state.invoice_links.get(invoice_reference).cloned())
    }

    /// Every recorded invoice payment link (newest record per invoice).
    pub fn invoice_links(&self) -> Result<Vec<InvoiceLinkRecord>, BillingError> {
        let state = self.state()?;
        Ok(state.invoice_links.values().cloned().collect())
    }

    /// The newest recorded invoice-level payment of one invoice, when any.
    /// The journal is the source of truth: a paid record here is the durable
    /// "the invoice's payment link collected" fact (E7).
    pub fn invoice_payment_for(
        &self,
        invoice_reference: &str,
    ) -> Result<Option<InvoicePaymentRecord>, BillingError> {
        let state = self.state()?;
        Ok(state.invoice_payments.get(invoice_reference).cloned())
    }

    /// Whether the invoice's payment link has collected (E7): a paid
    /// invoice-level payment record exists for the reference.
    pub fn invoice_is_paid(&self, invoice_reference: &str) -> Result<bool, BillingError> {
        Ok(self
            .invoice_payment_for(invoice_reference)?
            .is_some_and(|payment| payment.is_paid()))
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

    /// The intent holding a provider payment id, if any. With a registry
    /// wired the lookup resolves through the registry's
    /// `UNIQUE(provider, external_payment_id)` index first — an indexed
    /// point query instead of deriving the answer from a JSONL scan — and
    /// falls back to the in-memory index otherwise.
    pub fn intent_for_provider_payment(
        &self,
        payment_id: &str,
    ) -> Result<Option<PaymentIntent>, BillingError> {
        if let Some(registry) = &self.registry {
            if let Some(intent_id) =
                registry.intent_for_provider_payment(PROVIDER_MOLLIE, payment_id)?
            {
                let indexed = {
                    let state = self.state()?;
                    state.by_id.get(&intent_id).cloned()
                };
                if indexed.is_some() {
                    return Ok(indexed);
                }
                // The registry knows the payment but this handle's journal
                // index is stale (another process appended the intent after
                // this store opened): one rebuild brings the snapshot in.
                self.reload()?;
                let state = self.state()?;
                return Ok(state.by_id.get(&intent_id).cloned());
            }
        }
        let state = self.state()?;
        Ok(state
            .by_provider
            .get(payment_id)
            .and_then(|id| state.by_id.get(id))
            .cloned())
    }

    /// A submitted intent for the same line that already carries a provider
    /// payment id (the recovery search: no second charge is created when
    /// another durable attempt knows the provider id). With a registry
    /// wired, the per-line candidates come from the registry's line-reference
    /// index instead of scanning the in-memory journal index.
    pub fn submitted_with_provider(
        &self,
        line_reference: &str,
        excluding: &str,
    ) -> Result<Option<PaymentIntent>, BillingError> {
        let is_submitted_with_provider = |intent: &PaymentIntent| {
            matches!(
                intent.state,
                PaymentIntentState::SubmittedPending
                    | PaymentIntentState::SubmittedUnknown
                    | PaymentIntentState::Paid
                    | PaymentIntentState::Failed
                    | PaymentIntentState::Refunded
            ) && intent.provider_payment_id.is_some()
        };
        if let Some(registry) = &self.registry {
            let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for intent_id in
                registry.intent_ids_for_line_reference(PROVIDER_MOLLIE, line_reference)?
            {
                // One intent can carry two recorded payment ids (a refused
                // charge and its typed fallback): visit it once.
                if intent_id == excluding || !seen.insert(intent_id.clone()) {
                    continue;
                }
                let candidate = {
                    let state = self.state()?;
                    state.by_id.get(&intent_id).cloned()
                };
                if candidate.as_ref().is_some_and(is_submitted_with_provider) {
                    return Ok(candidate);
                }
            }
            // Fall through to the journal-derived scan: a journal that
            // predates the registry carries no index rows yet.
        }
        Ok(self
            .intents_for_reference(line_reference)?
            .into_iter()
            .filter(|intent| intent.intent_id != excluding)
            .find(|intent| is_submitted_with_provider(intent)))
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
        let previous = intent.state;
        if previous == PaymentIntentState::SubmittedUnknown && intent.provider_payment_id.is_none()
        {
            // An uncertain submission without a durable id is a replay of the
            // exact same call: keep the Idempotency-Key and the window
            // anchor (re-anchoring would let a timeout loop restart the
            // window forever instead of failing closed after it), while still
            // counting the durable attempt.
            intent.attempts = intent.attempts.saturating_add(1);
            if intent.channel.is_none() {
                intent.channel = Some(channel);
            }
            if intent.customer_id.is_none() {
                intent.customer_id = customer_id.map(str::to_string);
            }
            if intent.mandate_id.is_none() {
                intent.mandate_id = mandate_id.map(str::to_string);
            }
            intent.updated_unix_ms = now_unix_ms;
            self.persist(intent.clone())?;
            return Ok(intent);
        }
        if previous == PaymentIntentState::Failed {
            // The earlier payment is definitively dead: its id must not
            // collide with the new submission's id, and the retry needs its
            // own Idempotency-Key so Mollie cannot replay the failed response.
            intent.provider_payment_id = None;
            intent.provider_status = None;
            intent.idempotency_key = Some(format!(
                "{}#retry{}",
                intent.line_reference,
                intent.attempts + 1
            ));
        } else {
            intent.idempotency_key = Some(intent.line_reference.clone());
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

    /// Persist the shopper-facing checkout URL once a link exists, so a
    /// resumed pass can still report the link it created.
    pub fn record_checkout_url(
        &self,
        intent_id: &str,
        url: &str,
        now_unix_ms: i64,
    ) -> Result<PaymentIntent, BillingError> {
        let mut intent = self.required(intent_id)?;
        if intent.checkout_url.as_deref() == Some(url) {
            return Ok(intent);
        }
        intent.checkout_url = Some(url.to_string());
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
        // status that is the durable truth: the status string only advances
        // when the state itself advanced to the reported state, so a `failed`
        // landing on a Refunded intent cannot relabel it.
        if merged == next {
            intent.provider_status = Some(provider_status.to_string());
        }
        intent.state = merged;
        intent.updated_unix_ms = now_unix_ms;
        self.persist(intent.clone())?;
        // Mirror the payment-id lookup row into the transactional registry
        // when one is wired: this is the UNIQUE(provider, external_payment_id)
        // fact the cross-process lookups resolve through. The journal append
        // above is already durable; a registry refusal surfaces (fail closed)
        // so a broken index is never silently inconsistent with the journal.
        if let (Some(registry), Some(payment_id)) =
            (&self.registry, intent.provider_payment_id.as_deref())
        {
            registry.record_intent_payment(
                PROVIDER_MOLLIE,
                payment_id,
                &intent.intent_id,
                &intent.line_reference,
                now_unix_ms,
            )?;
        }
        Ok(intent)
    }

    /// Apply a fetched provider payment to the intent that owns it (by
    /// provider id, or by its deterministic reference when the id was not
    /// persisted). Returns `None` when no intent matches. Replays and
    /// reordered deliveries are safe: terminal states never regress.
    ///
    /// This is the webhook handler's lookup path, so it reloads the journal
    /// first: the monthly run appends intents in another process, and a store
    /// opened at API start must see them before it answers a delivery (EE).
    /// With a registry wired, the by-payment-id lookup resolves through the
    /// registry's UNIQUE index after that reload.
    pub fn reconcile_payment(
        &self,
        payment: &ProviderPayment,
        now_unix_ms: i64,
    ) -> Result<Option<PaymentIntent>, BillingError> {
        self.reload()?;
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

    /// Apply a fetched provider payment to the invoice it settled through
    /// the durable payment link (E7). A payment-link payment carries the
    /// `apex-invoice:<publisher>:<period>` reference instead of a line
    /// reference, so it can never attach to a line intent; the fetched
    /// payment is recorded as the invoice's durable payment record instead,
    /// which is what marks the invoice paid. Returns `None` when the payment
    /// carries no usable invoice-link reference.
    ///
    /// Like [`PaymentIntentStore::reconcile_payment`], this is the webhook
    /// handler's path: it reloads the journal first (EE), and replays are
    /// safe — the same `(payment id, status)` is appended once.
    pub fn reconcile_invoice_payment(
        &self,
        payment: &ProviderPayment,
        now_unix_ms: i64,
    ) -> Result<Option<InvoicePaymentRecord>, BillingError> {
        self.reload()?;
        let Some(reference) = payment.metadata_reference.as_deref() else {
            return Ok(None);
        };
        let Some((publisher_id, year_month)) =
            crate::mollie::parse_invoice_link_reference(reference)
        else {
            return Ok(None);
        };
        let record = InvoicePaymentRecord {
            invoice_reference: format!("INV-{year_month}-{publisher_id}"),
            publisher_id,
            year_month,
            payment_id: payment.id.clone(),
            status: payment.status.clone(),
            amount_minor_units: payment.amount_minor_units,
            currency: payment.currency.clone(),
            created_unix_ms: now_unix_ms,
        };
        self.append_payment_record(&record)?;
        Ok(Some(record))
    }

    fn required(&self, intent_id: &str) -> Result<PaymentIntent, BillingError> {
        self.get(intent_id)?
            .ok_or_else(|| BillingError::PaymentIntentConflict {
                detail: format!("intent {intent_id} is not recorded"),
            })
    }

    /// Record the invoice-level payment link once it exists, so the invoice
    /// email, the console and the dunning sweep can hand the customer a URL
    /// that does not expire. Recording the same link again is a no-op; a
    /// *different* link for the same invoice is appended and the newest
    /// record wins on read (the journal is the source of truth).
    pub fn record_invoice_link(
        &self,
        invoice_reference: &str,
        invoice: &Invoice,
        link: &crate::mollie::PaymentLinkResponse,
        amount_minor_units: i64,
        now_unix_ms: i64,
    ) -> Result<InvoiceLinkRecord, BillingError> {
        let record = InvoiceLinkRecord {
            invoice_reference: invoice_reference.to_string(),
            publisher_id: invoice.publisher_id.clone(),
            year_month: invoice.period.year_month.clone(),
            payment_link_id: link.id.clone(),
            url: link.url.clone(),
            amount_minor_units,
            currency: crate::mollie::BILLING_CURRENCY.to_string(),
            created_unix_ms: now_unix_ms,
        };
        self.append_record(&record)?;
        Ok(record)
    }

    /// Record the invoice-level payment the durable link collected (E7).
    /// The same `(payment id, status)` is appended once — Mollie's retries
    /// and Apex's replays are idempotent; a *status transition* (open ->
    /// paid) appends the newest fact and wins on read, exactly like the
    /// collection store's replay key.
    pub fn record_invoice_payment(
        &self,
        record: &InvoicePaymentRecord,
    ) -> Result<(), BillingError> {
        self.append_payment_record(record)
    }

    fn append_payment_record(&self, record: &InvoicePaymentRecord) -> Result<(), BillingError> {
        let _guard = self
            .append
            .lock()
            .map_err(|_| BillingError::PaymentIntentStoreCorrupt {
                detail: "intent append lock poisoned".into(),
            })?;
        if let Some(existing) = self.state()?.invoice_payments.get(&record.invoice_reference) {
            if existing.payment_id == record.payment_id && existing.status == record.status {
                return Ok(());
            }
        }
        let mut line = serde_json::to_string(record).map_err(|error| {
            BillingError::PaymentIntentStoreCorrupt {
                detail: format!("invoice payment serialization: {error}"),
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
        self.state()?
            .observe_payment(record.clone())
            .map_err(|detail| BillingError::PaymentIntentStoreCorrupt {
                detail: format!("invoice payment index: {detail}"),
            })?;
        *self.stamp()? = FileStamp::of(&self.path)?;
        Ok(())
    }

    fn append_record(&self, record: &InvoiceLinkRecord) -> Result<(), BillingError> {
        let _guard = self
            .append
            .lock()
            .map_err(|_| BillingError::PaymentIntentStoreCorrupt {
                detail: "intent append lock poisoned".into(),
            })?;
        if let Some(existing) = self.state()?.invoice_links.get(&record.invoice_reference) {
            if existing.url == record.url && existing.payment_link_id == record.payment_link_id {
                return Ok(());
            }
        }
        let mut line = serde_json::to_string(record).map_err(|error| {
            BillingError::PaymentIntentStoreCorrupt {
                detail: format!("invoice link serialization: {error}"),
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
        self.state()?
            .observe_link(record.clone())
            .map_err(|detail| BillingError::PaymentIntentStoreCorrupt {
                detail: format!("invoice link index: {detail}"),
            })?;
        *self.stamp()? = FileStamp::of(&self.path)?;
        Ok(())
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
        // The index already reflects this append: refresh the stamp so a
        // later `reload` does not pointlessly rebuild from our own write.
        *self.stamp()? = FileStamp::of(&self.path)?;
        Ok(())
    }

    fn state(&self) -> Result<std::sync::MutexGuard<'_, IntentIndex>, BillingError> {
        self.state
            .lock()
            .map_err(|_| BillingError::PaymentIntentStoreCorrupt {
                detail: "intent index poisoned".into(),
            })
    }

    fn stamp(&self) -> Result<std::sync::MutexGuard<'_, FileStamp>, BillingError> {
        self.stamp
            .lock()
            .map_err(|_| BillingError::PaymentIntentStoreCorrupt {
                detail: "intent file stamp poisoned".into(),
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

    fn observe_link(&mut self, record: InvoiceLinkRecord) -> Result<(), String> {
        if record.invoice_reference.is_empty() || record.url.is_empty() {
            return Err("invoice link record carries an empty reference or URL".to_string());
        }
        // The journal is the source of truth: the newest link recorded for an
        // invoice is the one every reader reports.
        self.invoice_links
            .insert(record.invoice_reference.clone(), record);
        Ok(())
    }

    fn observe_payment(&mut self, record: InvoicePaymentRecord) -> Result<(), String> {
        if record.invoice_reference.is_empty() || record.payment_id.is_empty() {
            return Err("invoice payment record carries an empty reference or payment id".to_string());
        }
        // The journal is the source of truth: the newest payment recorded for
        // an invoice is the one every reader reports.
        self.invoice_payments
            .insert(record.invoice_reference.clone(), record);
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
/// intent is resumed or fetched instead of charged again. After the lines,
/// the invoice's durable provider **payment link** is created for the invoice
/// total and recorded in the journal (E7): it is the never-expiring URL the
/// invoice email and the console deliver.
///
/// The caller holds the provider behind the neutral [`PaymentProvider`] seam
/// (`&dyn PaymentProvider`); this function is agnostic of which PSP is wired.
pub async fn collect_invoice_with_intents(
    provider: &dyn PaymentProvider,
    transport: &dyn Transport,
    store: &PaymentIntentStore,
    invoice: &Invoice,
    stored_credential: Option<&str>,
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
            provider,
            transport,
            store,
            &intent,
            &item,
            stored_credential,
            now_unix_ms,
        )
        .await?;
        total_minor_units = total_minor_units
            .checked_add(item.amount_minor_units)
            .ok_or(BillingError::Overflow)?;
        lines.push(collected);
    }
    // E7: the invoice-level payment link is the customer's always-valid way
    // to pay the total. When the lines were charged by stored mandate — or a
    // mandate charge was attempted and fell back to its own per-line link —
    // a link for the full total would let the same invoice be paid twice,
    // so it is created only when no mandate charge was attempted.
    let mandate_attempted = lines.iter().any(|line| {
        line.channel == CollectionChannel::StoredMandate || line.fallback_reason.is_some()
    });
    let invoice_link = if lines.is_empty() || mandate_attempted {
        None
    } else {
        Some(
            ensure_invoice_payment_link(
                provider,
                transport,
                store,
                invoice,
                &invoice_reference,
                total_minor_units,
                now_unix_ms,
            )
            .await?,
        )
    };
    Ok(crate::mollie::CollectionReceipt {
        publisher_id: invoice.publisher_id.clone(),
        year_month: invoice.period.year_month.clone(),
        currency: crate::mollie::BILLING_CURRENCY.to_string(),
        total_minor_units,
        lines,
        invoice_link,
    })
}

/// Create — or return the already recorded — durable payment link for one
/// invoice. The journal is the idempotency store: a link recorded for the
/// invoice reference is reused, so a resumed pass never creates a second
/// link, and the deterministic reference is the provider's Idempotency-Key.
async fn ensure_invoice_payment_link(
    provider: &dyn PaymentProvider,
    transport: &dyn Transport,
    store: &PaymentIntentStore,
    invoice: &Invoice,
    invoice_reference: &str,
    total_minor_units: i64,
    now_unix_ms: i64,
) -> Result<crate::mollie::PaymentLinkResponse, BillingError> {
    if let Some(existing) = store.invoice_link_for(invoice_reference)? {
        return Ok(crate::mollie::PaymentLinkResponse {
            id: existing.payment_link_id,
            url: existing.url,
            customer_id: None,
        });
    }
    let link = provider
        .create_invoice_payment_link(
            transport,
            &invoice.publisher_id,
            &invoice.period.year_month,
            total_minor_units,
            crate::mollie::BILLING_CURRENCY,
            &format!("Apex invoice {invoice_reference}"),
        )
        .await?;
    store.record_invoice_link(
        invoice_reference,
        invoice,
        &link,
        total_minor_units,
        now_unix_ms,
    )?;
    Ok(link)
}

/// Resume one line's durable intent. A `Paid`/`Refunded` intent returns its
/// durable evidence without a provider call; a `SubmittedPending` intent fetches
/// its provider payment; an `Issued` intent submits; an unresolved
/// `SubmittedUnknown` intent is fetched or replayed within the documented
/// idempotency window and otherwise fails closed.
#[allow(clippy::too_many_arguments)]
async fn collect_line_with_intent(
    provider: &dyn PaymentProvider,
    transport: &dyn Transport,
    store: &PaymentIntentStore,
    intent: &PaymentIntent,
    item: &InvoiceLineItem,
    stored_credential: Option<&str>,
    now_unix_ms: i64,
) -> Result<LineCollection, BillingError> {
    match intent.state {
        // The reason field is the typed *fallback* reason only: a resumed
        // or already-collected line carries no fallback, so the invoice-link
        // gate below never mistakes a resume for a mandate refusal (E7).
        PaymentIntentState::Paid | PaymentIntentState::Refunded => Ok(line_from_intent(
            intent,
            item,
            String::new(),
        )),
        PaymentIntentState::SubmittedPending => {
            let Some(payment_id) = intent.provider_payment_id.as_deref() else {
                return Err(BillingError::PaymentIntentUncertain {
                    intent_id: intent.intent_id.clone(),
                });
            };
            let payment = provider.fetch_payment(transport, payment_id).await?;
            let updated = store.reconcile_payment(&payment, now_unix_ms)?;
            Ok(line_from_intent(
                updated.as_ref().unwrap_or(intent),
                item,
                String::new(),
            ))
        }
        PaymentIntentState::SubmittedUnknown => {
            resolve_unknown(
                provider,
                transport,
                store,
                intent,
                item,
                stored_credential,
                now_unix_ms,
            )
            .await
        }
        PaymentIntentState::Issued | PaymentIntentState::Failed => {
            submit(
                provider,
                transport,
                store,
                intent,
                item,
                stored_credential,
                None,
                now_unix_ms,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_unknown(
    provider: &dyn PaymentProvider,
    transport: &dyn Transport,
    store: &PaymentIntentStore,
    intent: &PaymentIntent,
    item: &InvoiceLineItem,
    stored_credential: Option<&str>,
    now_unix_ms: i64,
) -> Result<LineCollection, BillingError> {
    if let Some(payment_id) = intent.provider_payment_id.as_deref() {
        let payment = provider.fetch_payment(transport, payment_id).await?;
        let updated = store.reconcile_payment(&payment, now_unix_ms)?;
        return Ok(line_from_intent(
            updated.as_ref().unwrap_or(intent),
            item,
            String::new(),
        ));
    }
    if let Some(other) = store.submitted_with_provider(&item.reference, &intent.intent_id)? {
        if let Some(payment_id) = other.provider_payment_id.as_deref() {
            let payment = provider.fetch_payment(transport, payment_id).await?;
            let updated = store.reconcile_payment(&payment, now_unix_ms)?;
            return Ok(line_from_intent(
                updated.as_ref().unwrap_or(&other),
                item,
                String::new(),
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
        provider,
        transport,
        store,
        intent,
        item,
        stored_credential,
        Some(intent.channel.unwrap_or(PaymentIntentChannel::PaymentLink)),
        now_unix_ms,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn submit(
    provider: &dyn PaymentProvider,
    transport: &dyn Transport,
    store: &PaymentIntentStore,
    intent: &PaymentIntent,
    item: &InvoiceLineItem,
    stored_credential: Option<&str>,
    force_channel: Option<PaymentIntentChannel>,
    now_unix_ms: i64,
) -> Result<LineCollection, BillingError> {
    let credential = stored_credential.and_then(StoredCredential::parse);
    let (channel, credential_to_use) = match force_channel {
        Some(PaymentIntentChannel::PaymentLink) => (PaymentIntentChannel::PaymentLink, None),
        // A replayed first payment replays exactly: same endpoint, same key,
        // so the provider's idempotency window returns the original payment.
        Some(PaymentIntentChannel::FirstPayment) => (PaymentIntentChannel::FirstPayment, None),
        Some(PaymentIntentChannel::StoredMandate) => {
            let resolved = credential.clone().or_else(|| {
                Some(StoredCredential {
                    customer_id: intent.customer_id.clone()?,
                    credential_id: intent.mandate_id.clone()?,
                })
            });
            match resolved {
                Some(credential) => (PaymentIntentChannel::StoredMandate, Some(credential)),
                None => {
                    return Err(BillingError::PaymentIntentUncertain {
                        intent_id: intent.intent_id.clone(),
                    })
                }
            }
        }
        None => match credential {
            Some(credential) => (PaymentIntentChannel::StoredMandate, Some(credential)),
            // No stored credential: the *first* payment registers one, so
            // the next months charge the stored method instead of opening
            // checkout payments that expire within hours (E7).
            None => (PaymentIntentChannel::FirstPayment, None),
        },
    };
    let (customer_id, mandate_id) = match &credential_to_use {
        Some(credential) => (
            Some(credential.customer_id.as_str()),
            Some(credential.credential_id.as_str()),
        ),
        None => (None, None),
    };
    let started = store.begin_submission(
        &intent.intent_id,
        channel,
        customer_id,
        mandate_id,
        now_unix_ms,
    )?;
    let idempotency_key = started
        .idempotency_key
        .clone()
        .unwrap_or_else(|| item.reference.clone());
    match channel {
        PaymentIntentChannel::StoredMandate => {
            let Some(credential) = credential_to_use else {
                return Err(BillingError::PaymentIntentUncertain {
                    intent_id: intent.intent_id.clone(),
                });
            };
            match provider
                .charge_credential(transport, item, &credential, &idempotency_key)
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
                    let link_key = link_started
                        .idempotency_key
                        .clone()
                        .unwrap_or_else(|| item.reference.clone());
                    let link = provider
                        .create_payment_link(transport, item, &link_key, PaymentSequence::OneOff)
                        .await?;
                    store.record_provider(
                        &link_started.intent_id,
                        Some(&link.id),
                        "open",
                        now_unix_ms,
                    )?;
                    store.record_checkout_url(&link_started.intent_id, &link.url, now_unix_ms)?;
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
        PaymentIntentChannel::FirstPayment | PaymentIntentChannel::PaymentLink => {
            // A first payment stores the credential later months charge; the
            // forced plain-link channel (the typed refusal fallback, or a
            // replay that originally used a checkout) never re-runs it.
            let sequence = if channel == PaymentIntentChannel::FirstPayment {
                PaymentSequence::First
            } else {
                PaymentSequence::OneOff
            };
            let link = provider
                .create_payment_link(transport, item, &idempotency_key, sequence)
                .await?;
            store.record_provider(&started.intent_id, Some(&link.id), "open", now_unix_ms)?;
            store.record_checkout_url(&started.intent_id, &link.url, now_unix_ms)?;
            let mut line = line_from_intent(&started, item, String::new());
            line.channel = CollectionChannel::PaymentLink;
            line.first_payment = channel == PaymentIntentChannel::FirstPayment;
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
    let payment_link = if channel == CollectionChannel::PaymentLink {
        intent.checkout_url.clone().map(|url| PaymentLinkResponse {
            id: intent
                .provider_payment_id
                .clone()
                .unwrap_or_else(|| String::from("unknown")),
            url,
            customer_id: None,
        })
    } else {
        None
    };
    LineCollection {
        seq: item.seq,
        reference: item.reference.clone(),
        amount_minor_units: item.amount_minor_units,
        currency: item.currency.clone(),
        channel,
        payment_id: intent.provider_payment_id.clone(),
        status: intent.provider_status.clone(),
        payment_link,
        first_payment: intent.channel == Some(PaymentIntentChannel::FirstPayment),
        fallback_reason: (!fallback_reason.is_empty()).then_some(fallback_reason),
    }
}

/// How long after an invoice was issued a failed line stays untouched before
/// the dunning sweep chases it: the provider's own `failed`/`expired`
/// statuses are definitive, but the webhook and the customer deserve a grace
/// window before the reminders start.
pub const DUNNING_GRACE_MS: i64 = 3 * 24 * 3_600_000;

/// One invoice the dunning sweep chases (GG): failed line charges with no
/// later successful payment, and the durable payment link to re-send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DunningCandidate {
    /// The invoice reference (`INV-<month>-<publisher>`).
    pub invoice_reference: String,
    pub publisher_id: String,
    pub year_month: String,
    /// The unpaid failed line intents: one retry-schedule row each.
    pub unpaid: Vec<PaymentIntent>,
    /// The recorded invoice payment link, when one exists. Without one there
    /// is nothing to re-send and the sweep only writes the retry rows.
    pub payment_link: Option<InvoiceLinkRecord>,
}

impl DunningCandidate {
    /// The unpaid balance in micros: the amount the retry message states.
    #[must_use]
    pub fn unpaid_micros(&self) -> i64 {
        self.unpaid
            .iter()
            .map(|intent| intent.amount_minor_units.saturating_mul(10_000))
            .sum()
    }
}

/// The unpaid intents of one publisher past their due date. A line is chased
/// when its provider charge definitively failed (`failed`, `canceled` and
/// `expired` all record as `Failed`) and no later intent for the same line
/// collected. The lines stay grouped per invoice, so a sweep writes one
/// retry row per line and re-sends at most one link per invoice.
pub fn dunning_candidates(
    store: &PaymentIntentStore,
    publisher_id: &str,
    now_unix_ms: i64,
) -> Result<Vec<DunningCandidate>, BillingError> {
    let intents = store.intents()?;
    let mut by_invoice: std::collections::BTreeMap<String, Vec<&PaymentIntent>> =
        std::collections::BTreeMap::new();
    for intent in &intents {
        if intent.publisher_id == publisher_id {
            by_invoice
                .entry(intent.invoice_reference.clone())
                .or_default()
                .push(intent);
        }
    }
    let mut candidates = Vec::new();
    for (invoice_reference, lines) in by_invoice {
        // E7: the invoice's durable payment link may itself have collected.
        // A paid invoice-level payment record settles every line; chasing
        // it would ask a publisher to pay twice.
        if store.invoice_is_paid(&invoice_reference)? {
            continue;
        }
        let issued_at = lines
            .iter()
            .map(|intent| intent.created_unix_ms)
            .min()
            .unwrap_or_default();
        if issued_at.saturating_add(DUNNING_GRACE_MS) > now_unix_ms {
            continue;
        }
        let unpaid: Vec<PaymentIntent> = lines
            .iter()
            .filter(|intent| {
                intent.state == PaymentIntentState::Failed
                    && !lines.iter().any(|other| {
                        other.line_reference == intent.line_reference
                            && matches!(
                                other.state,
                                PaymentIntentState::Paid | PaymentIntentState::Refunded
                            )
                            && other.updated_unix_ms >= intent.updated_unix_ms
                    })
            })
            .map(|intent| (*intent).clone())
            .collect();
        if unpaid.is_empty() {
            continue;
        }
        let payment_link = store.invoice_link_for(&invoice_reference)?;
        candidates.push(DunningCandidate {
            publisher_id: unpaid[0].publisher_id.clone(),
            year_month: unpaid[0].year_month.clone(),
            invoice_reference,
            unpaid,
            payment_link,
        });
    }
    Ok(candidates)
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
    use crate::invoice::{BillingPeriod, Invoice, InvoiceLine};
    use crate::mollie::{MollieClient, MolliePayment};
    use crate::provider::ProviderPayment as ReconcilePayment;
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
        /// Every POST's (url, parsed JSON body), so tests can assert the
        /// endpoint shape (customers, payments, payment-links).
        posted: Mutex<Vec<(String, serde_json::Value)>>,
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

        /// The first-payment flow: a customer is created before the payment.
        fn with_first_payment() -> Self {
            let fake = Self::default();
            fake.script
                .lock()
                .unwrap()
                .push_back(Ok(unchecked_body("cst_1", "open", REFERENCE)));
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
            if let Some(HttpBody::Json(value)) = req.body.as_ref() {
                if req.method == apex_connectors::transport::HttpMethod::Post {
                    self.posted
                        .lock()
                        .unwrap()
                        .push((req.url.clone(), value.clone()));
                }
            }
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
        let fake = ProviderFake::with_first_payment();
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
        let fake = ProviderFake::with_first_payment();
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
        // The timeout hit the first-payment flow's customer call, before any
        // charge: no idempotent charge creation exists for the reference.
        assert_eq!(fake.create_count(REFERENCE), 0);

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
        assert_eq!(
            fake.create_count(REFERENCE),
            0,
            "no second charge: the unresolved submission fails closed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_mandate_refusal_falls_back_to_a_link_with_its_own_recording() {
        let (dir, store) = temp_store("fallback-link");
        let invoice = invoice();
        store
            .ensure_issued(&invoice, "INV-2026-10-pub-1", &item(), 1_000)
            .expect("issued");
        let fake = ProviderFake::default();
        // Mandate charge answers definitively failed (tr_A), then the link
        // creation answers with a different id (tr_B).
        fake.script
            .lock()
            .unwrap()
            .push_back(Ok(unchecked_body("tr_A", "failed", REFERENCE)));
        fake.script
            .lock()
            .unwrap()
            .push_back(Ok(unchecked_body("tr_B", "open", REFERENCE)));
        let receipt = collect_invoice_with_intents(
            &client(),
            &fake,
            &store,
            &invoice,
            Some("cus_1:mnd_1"),
            2_000,
        )
        .await
        .expect("the fallback link is recorded, never a conflict");
        assert_eq!(receipt.lines.len(), 1);
        let line = &receipt.lines[0];
        assert_eq!(line.channel, CollectionChannel::PaymentLink);
        let link = line.payment_link.as_ref().expect("the link is reported");
        assert_eq!(link.url, "https://pay.test/tr_B");
        // E7: a mandate charge was attempted, so the invoice total never
        // also gets a payment link — the fallback link is the paying URL.
        assert!(receipt.invoice_link.is_none());
        assert!(
            !fake
                .posted
                .lock()
                .unwrap()
                .iter()
                .any(|(url, _)| url.ends_with("/v2/payment-links")),
            "no payment-links call is made after a mandate attempt"
        );
        let intent = store
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .expect("present");
        assert_eq!(intent.state, PaymentIntentState::SubmittedPending);
        assert_eq!(intent.provider_payment_id.as_deref(), Some("tr_B"));
        assert_eq!(
            intent.checkout_url.as_deref(),
            Some("https://pay.test/tr_B")
        );
        // The refusal kept no stale id on the new submission.
        assert_ne!(intent.provider_payment_id.as_deref(), Some("tr_A"));
        // The link submission carries its own retry key, distinct from the
        // mandate attempt's reference key.
        assert!(intent
            .idempotency_key
            .as_deref()
            .is_some_and(|key| key.starts_with(REFERENCE) && key.contains("#retry")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn retrying_a_failed_link_uses_a_new_key_and_never_conflicts() {
        let (dir, store) = temp_store("failed-link-retry");
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
        store
            .record_provider(&intent.intent_id, Some("tr_old"), "failed", 1_500)
            .expect("failed");
        let fake = ProviderFake::with_first_payment();
        let receipt = collect_invoice_with_intents(&client(), &fake, &store, &invoice, None, 2_000)
            .await
            .expect("the failed payment retries under a fresh key");
        assert_eq!(receipt.lines[0].payment_id.as_deref(), None, "link arm");
        let intent = store
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .expect("present");
        // A new provider id replaced the dead one, under a #retry key.
        assert_eq!(intent.provider_payment_id.as_deref(), Some("tr_1"));
        assert_eq!(intent.state, PaymentIntentState::SubmittedPending);
        let retry_key = format!("{REFERENCE}#retry2");
        assert_eq!(intent.idempotency_key.as_deref(), Some(retry_key.as_str()));
        assert_eq!(
            fake.create_count(&retry_key),
            1,
            "the retry is its own idempotent call"
        );
        assert_eq!(
            fake.create_count(REFERENCE),
            0,
            "Mollie never replays the failed response for a retry"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_uncertain_replay_keeps_the_original_window_anchor() {
        let (dir, store) = temp_store("anchor");
        let invoice = invoice();
        let fake = ProviderFake::with_timeout();
        collect_invoice_with_intents(&client(), &fake, &store, &invoice, None, 1_000)
            .await
            .expect_err("uncertain");
        let intent = store
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .expect("present");
        assert_eq!(intent.submitted_unix_ms, Some(1_000));
        assert!(intent.provider_payment_id.is_none());

        // An inside-window replay does not re-anchor the window.
        let fake = ProviderFake::with_payment();
        collect_invoice_with_intents(
            &client(),
            &fake,
            &store,
            &invoice,
            None,
            1_000 + IDEMPOTENCY_WINDOW_MS - 1,
        )
        .await
        .expect("replay resolves");
        let intent = store
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .expect("present");
        assert_eq!(
            intent.submitted_unix_ms,
            Some(1_000),
            "the anchor is the FIRST submission, so the window still fails closed"
        );
        assert_eq!(intent.attempts, 2, "the replay is a durable attempt");
        assert_eq!(
            intent.idempotency_key.as_deref(),
            Some(REFERENCE),
            "the replay reused the original key"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recovered_link_line_carries_its_checkout_url_and_a_card_line_never_does() {
        let (dir, store) = temp_store("checkout-url");
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
        store
            .record_provider(&intent.intent_id, Some("tr_x"), "open", 1_100)
            .expect("open");
        store
            .record_checkout_url(&intent.intent_id, "https://pay.test/tr_x", 1_100)
            .expect("url");
        let intent = store.get(&intent.intent_id).expect("get").expect("present");
        let link_line = line_from_intent(&intent, &item(), String::new());
        assert_eq!(link_line.channel, CollectionChannel::PaymentLink);
        assert_eq!(
            link_line
                .payment_link
                .as_ref()
                .map(|link| link.url.as_str()),
            Some("https://pay.test/tr_x"),
            "a resumed pass still reports the link it created"
        );

        let mut card = intent.clone();
        card.channel = Some(PaymentIntentChannel::StoredMandate);
        let card_line = line_from_intent(&card, &item(), String::new());
        assert_eq!(card_line.channel, CollectionChannel::StoredMandate);
        assert!(card_line.payment_link.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_late_failed_delivery_never_relabels_a_refunded_intent() {
        let (dir, store) = temp_store("refund-relabel");
        let invoice = invoice();
        let intent = store
            .ensure_issued(&invoice, "INV-2026-10-pub-1", &item(), 1_000)
            .expect("issued");
        store
            .record_provider(&intent.intent_id, Some("tr_1"), "paid", 1_100)
            .expect("paid");
        store
            .record_provider(&intent.intent_id, Some("tr_1"), "refunded", 1_200)
            .expect("refunded");
        store
            .record_provider(&intent.intent_id, Some("tr_1"), "failed", 1_300)
            .expect("late failure is dropped");
        let intent = store.get(&intent.intent_id).expect("get").expect("present");
        assert_eq!(intent.state, PaymentIntentState::Refunded);
        assert_eq!(
            intent.provider_status.as_deref(),
            Some("refunded"),
            "the durable status stays the truth, not the reordered delivery"
        );
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
            .reconcile_payment(&ReconcilePayment::from(paid.clone()), 2_000)
            .expect("paid")
            .expect("known");
        // A replayed paid delivery.
        store
            .reconcile_payment(&ReconcilePayment::from(paid.clone()), 2_001)
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
                .reconcile_payment(&ReconcilePayment::from(reordered), 3_000)
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
            .reconcile_payment(&ReconcilePayment::from(refunded), 4_000)
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

    #[tokio::test]
    async fn a_pending_recurring_charge_is_submitted_and_never_falls_back() {
        // FF: SEPA debits settle over days. A `pending` recurring charge is a
        // live charge with a durable provider id — recording it as refused
        // would create a second checkout payment for the same line and the
        // later paid webhook would conflict.
        let (dir, store) = temp_store("pending-mandate");
        let invoice = invoice();
        let fake = ProviderFake::default();
        fake.script
            .lock()
            .unwrap()
            .push_back(Ok(unchecked_body("tr_sepa", "pending", REFERENCE)));
        let receipt = collect_invoice_with_intents(
            &client(),
            &fake,
            &store,
            &invoice,
            Some("cus_1:mnd_1"),
            1_000,
        )
        .await
        .expect("pending is submitted");
        assert_eq!(receipt.lines.len(), 1);
        assert_eq!(receipt.lines[0].channel, CollectionChannel::StoredMandate);
        assert_eq!(receipt.lines[0].status.as_deref(), Some("pending"));
        assert!(
            receipt.lines[0].payment_link.is_none(),
            "a pending charge never opens a second checkout"
        );
        let intent = store
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .expect("present");
        assert_eq!(intent.state, PaymentIntentState::SubmittedPending);
        assert_eq!(intent.provider_payment_id.as_deref(), Some("tr_sepa"));
        assert_eq!(intent.provider_status.as_deref(), Some("pending"));
        assert_eq!(fake.create_count(REFERENCE), 1);
        let posted = fake.posted.lock().unwrap();
        assert_eq!(
            posted
                .iter()
                .filter(|(url, _)| url.ends_with("/v2/payments"))
                .count(),
            1,
            "exactly one charge: the recurring charge, no fallback"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_webhook_from_another_process_sees_intents_appended_after_open() {
        // EE: the API opens the journal once; the monthly run appends later
        // in another process. The webhook lookup path (reconcile_payment)
        // reloads on the file-stamp change instead of answering "unknown".
        let (dir, store) = temp_store("cross-process");
        let invoice = invoice();
        assert!(
            !store.reload().expect("reload"),
            "an unchanged journal is not rebuilt"
        );

        // "Another process": a second store handle on the same file appends.
        let monthly = PaymentIntentStore::open(&dir.join("intents.jsonl")).expect("monthly");
        monthly
            .ensure_issued(&invoice, "INV-2026-10-pub-1", &item(), 1_000)
            .expect("issued");
        monthly
            .begin_submission(
                &PaymentIntent::id_for_reference(REFERENCE),
                PaymentIntentChannel::PaymentLink,
                None,
                None,
                1_100,
            )
            .expect("begin");

        // The stale API store still cannot see it...
        assert!(store
            .get(&PaymentIntent::id_for_reference(REFERENCE))
            .expect("get")
            .is_none());
        let paid = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(&format!(
                r#"{{"id":"tr_1","status":"paid","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"{REFERENCE}"}}}}"#
            ))
            .expect("json"),
        )
        .expect("payment");
        // ...but the webhook path reloads first and records the payment on
        // the intent the other process appended.
        let updated = store
            .reconcile_payment(&ReconcilePayment::from(paid.clone()), 2_000)
            .expect("reconcile")
            .expect("the appended intent is visible after the reload");
        assert_eq!(updated.state, PaymentIntentState::Paid);
        assert_eq!(updated.provider_payment_id.as_deref(), Some("tr_1"));

        // The second reload is a stamp no-op: nothing changed since.
        let monthly = PaymentIntentStore::open(&dir.join("intents.jsonl")).expect("reopen");
        assert!(
            !monthly.reload().expect("reload"),
            "a journal already in memory is not re-read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn without_a_mandate_the_first_payment_stores_one() {
        // E7: with no stored mandate the collection offers a first payment
        // (`sequenceType: "first"`); once paid, its webhook records the
        // customer and mandate, so later months charge the stored method.
        let (dir, store) = temp_store("first-payment");
        let invoice = invoice();
        let fake = ProviderFake::with_first_payment();
        let receipt = collect_invoice_with_intents(&client(), &fake, &store, &invoice, None, 1_000)
            .await
            .expect("collect");
        assert_eq!(receipt.lines[0].channel, CollectionChannel::PaymentLink);
        assert!(
            receipt.lines[0].first_payment,
            "the checkout is a first payment: completing it stores the mandate (E7)"
        );
        assert_eq!(
            receipt.lines[0]
                .payment_link
                .as_ref()
                .map(|link| link.url.as_str()),
            Some("https://pay.test/tr_1"),
            "the first payment's checkout URL is reported so it can be sent"
        );
        assert!(
            receipt.invoice_link.is_some(),
            "no mandate charge was attempted, so the durable invoice link exists"
        );
        let posted = fake.posted.lock().unwrap();
        let customers = posted
            .iter()
            .filter(|(url, _)| url.ends_with("/v2/customers"))
            .count();
        let payments = posted
            .iter()
            .filter(|(url, _)| url.ends_with("/v2/payments"))
            .count();
        assert_eq!(customers, 1, "one Mollie customer is created");
        assert_eq!(payments, 1);
        let (_, body) = posted
            .iter()
            .find(|(url, _)| url.ends_with("/v2/payments"))
            .expect("payment post");
        assert_eq!(body["sequenceType"], "first");
        assert_eq!(body["customerId"], "cst_1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_mandate_charged_invoice_never_also_creates_an_invoice_total_link() {
        // E7: the lines were charged by stored mandate; a payment link for
        // the same total would let the invoice be paid twice.
        let (dir, store) = temp_store("mandate-no-link");
        let invoice = invoice();
        let fake = ProviderFake::default();
        fake.script
            .lock()
            .unwrap()
            .push_back(Ok(unchecked_body("tr_card", "paid", REFERENCE)));
        let receipt = collect_invoice_with_intents(
            &client(),
            &fake,
            &store,
            &invoice,
            Some("cus_1:mnd_1"),
            1_000,
        )
        .await
        .expect("collect");
        assert_eq!(receipt.lines[0].channel, CollectionChannel::StoredMandate);
        assert!(
            receipt.invoice_link.is_none(),
            "no invoice-total link for mandate-charged lines"
        );
        assert!(
            store
                .invoice_link_for("INV-2026-10-pub-1")
                .expect("read")
                .is_none(),
            "nothing is recorded for the invoice"
        );
        assert!(!fake.posted.lock().unwrap().iter().any(|(url, _)| url
            .ends_with("/v2/payment-links")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_invoice_link_payment_marks_the_invoice_paid_and_stops_the_chase() {
        // E7: a payment-link payment carries the `apex-invoice:` reference
        // and no line reference; recording it marks the invoice paid so the
        // dunning sweep never chases it. Replays append nothing new.
        let (dir, store) = temp_store("invoice-paid");
        let invoice = invoice();
        let intent_id = PaymentIntent::id_for_reference(REFERENCE);
        store
            .ensure_issued(&invoice, "INV-2026-09-pub-1", &item(), 1_000)
            .expect("issued");
        store
            .record_provider(&intent_id, Some("tr_1"), "failed", 2_000)
            .expect("failed");

        let link_payment = MolliePayment::from_json(
            &serde_json::from_str::<serde_json::Value>(
                r#"{"id":"tr_link","status":"paid","amount":{"currency":"USD","value":"1.25"},"metadata":{"reference":"apex-invoice:pub-1:2026-09"}}"#,
            )
            .expect("json"),
        )
        .expect("payment");
        let now = 1_000 + DUNNING_GRACE_MS + 1;
        let recorded = store
            .reconcile_invoice_payment(&ReconcilePayment::from(link_payment.clone()), now)
            .expect("record")
            .expect("the invoice payment is recorded");
        assert_eq!(recorded.invoice_reference, "INV-2026-09-pub-1");
        assert_eq!(recorded.publisher_id, "pub-1");
        assert_eq!(recorded.year_month, "2026-09");
        assert!(recorded.is_paid());
        assert!(store.invoice_is_paid("INV-2026-09-pub-1").expect("read"));

        // The paid invoice is no longer chased.
        assert!(
            dunning_candidates(&store, "pub-1", now)
                .expect("candidates")
                .is_empty(),
            "a paid invoice is never chased"
        );

        // A replayed delivery appends nothing new.
        store
            .reconcile_invoice_payment(&ReconcilePayment::from(link_payment), now + 1)
            .expect("replay");
        let journal = std::fs::read_to_string(dir.join("intents.jsonl")).expect("journal");
        assert_eq!(
            journal
                .lines()
                .filter(|line| line.contains("\"paymentId\""))
                .count(),
            1,
            "the replayed link payment is idempotent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_invoice_payment_reference_must_parse_to_be_recorded() {
        // E7: only a well-formed `apex-invoice:<publisher>:<YYYY-MM>`
        // reference names an invoice; anything else (a line reference, a
        // path, a short period) records nothing.
        let (dir, store) = temp_store("invoice-ref-parse");
        let payment = |reference: &str| {
            MolliePayment::from_json(
                &serde_json::from_str::<serde_json::Value>(&format!(
                    r#"{{"id":"tr_link","status":"paid","amount":{{"currency":"USD","value":"1.25"}},"metadata":{{"reference":"{reference}"}}}}"#
                ))
                .expect("json"),
            )
            .expect("payment")
        };
        for bad in [
            "apex-fee:pub-1:exp-a:2026-09",
            "apex-invoice:../evil:2026-09",
            "apex-invoice:pub-1:2026-9",
            "apex-invoice:pub-1",
            "apex-invoice::2026-09",
        ] {
            let recorded = store
                .reconcile_invoice_payment(&ReconcilePayment::from(payment(bad)), 1_000)
                .expect("refused quietly");
            assert!(recorded.is_none(), "{bad:?}");
        }
        assert!(
            store
                .invoice_payment_for("INV-2026-09-pub-1")
                .expect("read")
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn collection_creates_one_durable_payment_link_for_the_invoice_total() {
        // E7: after the checkout payments, the invoice gets a Payment Links
        // API link for its total, recorded in the journal; a resumed pass
        // reuses the recorded link instead of creating another one.
        let (dir, store) = temp_store("invoice-link");
        let invoice = invoice();
        let fake = ProviderFake::with_first_payment();
        fake.script
            .lock()
            .unwrap()
            .push_back(Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: br#"{"resource":"payment-link","id":"pl_1","_links":{"paymentUrl":{"href":"https://pay.mollie.test/pl_1"}}}"#.to_vec(),
            }));
        let receipt = collect_invoice_with_intents(&client(), &fake, &store, &invoice, None, 1_000)
            .await
            .expect("collect");
        let link = receipt.invoice_link.as_ref().expect("invoice link");
        assert_eq!(link.id, "pl_1");
        assert_eq!(link.url, "https://pay.mollie.test/pl_1");

        let recorded = store
            .invoice_link_for("INV-2026-10-pub-1")
            .expect("read")
            .expect("recorded");
        assert_eq!(recorded.url, "https://pay.mollie.test/pl_1");
        assert_eq!(recorded.amount_minor_units, 125);
        assert_eq!(recorded.payment_link_id, "pl_1");

        // A resumed pass reuses the journal's link: no second link call.
        let receipt = collect_invoice_with_intents(&client(), &fake, &store, &invoice, None, 2_000)
            .await
            .expect("resume");
        let link = receipt.invoice_link.as_ref().expect("reused link");
        assert_eq!(link.id, "pl_1");
        let link_posts = fake
            .posted
            .lock()
            .unwrap()
            .iter()
            .filter(|(url, _)| url.ends_with("/v2/payment-links"))
            .count();
        assert_eq!(link_posts, 1, "the recorded link is reused, not recreated");

        // The link record survives a reopen beside the intents.
        let reopened = PaymentIntentStore::open(&dir.join("intents.jsonl")).expect("reopen");
        let record = reopened
            .invoice_link_for("INV-2026-10-pub-1")
            .expect("read")
            .expect("durable");
        assert_eq!(record.url, "https://pay.mollie.test/pl_1");
        assert_eq!(record.currency, "USD");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dunning_candidates_find_failed_lines_with_no_later_payment() {
        // GG: an unpaid invoice (all lines definitively failed, nothing paid
        // later) past its grace is a candidate; a paid line and a fresh
        // failure inside the grace window are not.
        let (dir, store) = temp_store("dunning");
        let invoice = invoice();
        let intent_id = PaymentIntent::id_for_reference(REFERENCE);
        store
            .ensure_issued(&invoice, "INV-2026-09-pub-1", &item(), 1_000)
            .expect("issued");
        store
            .record_provider(&intent_id, Some("tr_1"), "failed", 2_000)
            .expect("failed");

        // Inside the grace window: not yet chased.
        let now = 1_000 + DUNNING_GRACE_MS - 1;
        assert!(dunning_candidates(&store, "pub-1", now)
            .expect("candidates")
            .is_empty());

        // Past the grace: the failed line is chased, with the recorded link.
        let now = 1_000 + DUNNING_GRACE_MS + 1;
        let candidates = dunning_candidates(&store, "pub-1", now).expect("candidates");
        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        assert_eq!(candidate.invoice_reference, "INV-2026-09-pub-1");
        assert_eq!(candidate.unpaid.len(), 1);
        assert_eq!(candidate.unpaid[0].amount_minor_units, 125);
        assert!(candidate.payment_link.is_none(), "no link was recorded yet");

        // A later successful payment on the same line stops the chase.
        store
            .begin_submission(
                &intent_id,
                PaymentIntentChannel::PaymentLink,
                None,
                None,
                3_000,
            )
            .expect("retry begins a fresh submission");
        store
            .record_provider(&intent_id, Some("tr_2"), "paid", 4_000)
            .expect("paid");
        let candidates = dunning_candidates(&store, "pub-1", now).expect("candidates");
        assert!(candidates.is_empty(), "a paid line is not chased");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
