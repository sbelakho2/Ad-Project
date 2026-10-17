//! Shared server state [`AppState`].
//!
//! Everything the router needs is injected: sources, credentials, clock and
//! the cycle runner. `AppState` is `Clone` (it only holds `Arc`s and small
//! values) and every clone shares the same sources — no per-request state is
//! kept anywhere, which is what makes the read-only deployment trivially
//! reproducible.

use std::collections::HashMap;
use std::sync::Arc;

use apex_billing::AdyenClient;

use crate::analytics::AnalyticsAccess;
use crate::clock::{Clock, SystemClock};
use crate::cycle::{CycleRunner, LedgerOnlyRunner};
use crate::error::ApiError;
use crate::leads::LeadStore;
use crate::live::{constant_time_eq, LiveAuditor};
use crate::session::{LedgerSessions, DEFAULT_SESSION_CAPACITY, SESSION_IDLE_MS};
use crate::sources::{AuditSource, LedgerSource};
use apex_billing::CollectionStore;

/// A debug-printable holder for the Adyen checkout transport. The transport
/// trait itself is not `Debug`, and the binary's endpoint must never appear in
/// a state debug line anyway.
#[derive(Clone)]
pub struct BillingTransport(Arc<dyn apex_connectors::transport::Transport>);

impl std::fmt::Debug for BillingTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BillingTransport")
    }
}

impl BillingTransport {
    /// The underlying transport.
    pub fn get(&self) -> &dyn apex_connectors::transport::Transport {
        self.0.as_ref()
    }
}

/// The complete dependency set of the HTTP surface.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Where audit snapshots come from.
    pub audit_source: Arc<dyn AuditSource>,
    /// Where verified ledger records come from.
    pub ledger_source: Arc<dyn LedgerSource>,
    /// Adyen client. When configured, the webhook route takes the HMAC key
    /// from it; the state's explicit `adyen_hmac_key` is the fallback for
    /// deployments that hold only the key.
    pub adyen: Option<Arc<AdyenClient>>,
    /// The transport Adyen Checkout calls travel through (payment-link
    /// creation for the setup watch). `None` means no checkout call can be
    /// made: the watch page then shows the honest "we will email a payment
    /// link" state instead of inventing one.
    pub billing_transport: Option<Arc<BillingTransport>>,
    /// Explicit webhook HMAC key, used only when no Adyen client is
    /// configured.
    pub adyen_hmac_key: Option<Vec<u8>>,
    /// The clock, used by the live auditor for snapshot timestamps.
    pub clock: Arc<dyn Clock>,
    /// The cycle runner behind `POST /v1/cycle/{publisher}`.
    pub cycle_runner: Arc<dyn CycleRunner>,
    /// The live read-only auditor behind `POST /v1/audit/{platform}`. `None`
    /// makes the route answer `503 live_audit_not_configured`.
    pub live_auditor: Option<Arc<LiveAuditor>>,
    /// Per-publisher ledger tokens. A publisher with no token here cannot have
    /// its ledger read: the route answers `503 ledger_auth_not_configured`
    /// rather than serving it unauthenticated.
    pub ledger_tokens: Arc<HashMap<String, Vec<u8>>>,
    /// The live per-publisher token file behind the self-serve workspace
    /// (`APEX_LEDGER_TOKENS_FILE`). A free-audit submission extends it with
    /// the submission's publisher key, and both the ledger authorization and
    /// the console's "has any credentials" gate consult it, so a workspace
    /// created after boot is usable without a restart.
    pub workspaces: Option<Arc<crate::signin::WorkspaceTokens>>,
    /// The one-time sign-in link service (`signin.jsonl` under the leads
    /// root). `None` makes `GET /console/signin` answer the `503`
    /// not-configured panel: without a leads root the path fails closed.
    pub signin: Option<Arc<crate::signin::SigninService>>,
    /// The new-workspace ownership proof (the creating email domain's public
    /// `app-ads.txt`). `None` makes every new workspace unprovable, so a
    /// free-audit submission for an unknown workspace is refused typed
    /// rather than creating one.
    pub app_ads_proof: Option<Arc<dyn crate::signin::WorkspaceProofSource>>,
    /// Lead capture behind `POST /v1/leads`. `None` makes the route answer
    /// `503 lead_capture_not_configured`.
    pub leads: Option<Arc<dyn LeadStore>>,
    /// The setup-watch subscriber store behind `GET`/`POST /watch`. `None`
    /// makes `POST /watch` answer the not-configured page (`503`).
    pub watch: Option<Arc<apex_watch::SubscriberStore>>,
    /// The frozen sales-outcomes appender. `None` (tests, or a deployment
    /// that removed the variable) makes every call site a no-op; the primary
    /// action never depends on it.
    pub outcomes: Option<Arc<crate::outcomes::OutcomesAppender>>,
    /// Where accepted Adyen collection events are recorded (replay-protected).
    /// `None` makes the webhook answer `503 collection_store_not_configured`.
    pub collection: Option<Arc<dyn CollectionStore>>,
    /// In-memory browser sessions for the console (`POST /console/ledger`
    /// mints one; every console page resolves its publisher from it and the
    /// cookie carries only its opaque id).
    pub ledger_sessions: Arc<LedgerSessions>,
    /// The analytics reader and emitter (disabled when unset).
    pub analytics: AnalyticsAccess,
    /// The lead-id salt shared by the lead store and the analytics emitter.
    pub lead_id_salt: Arc<String>,
    /// The vault-backed connection store (`APEX_CREDENTIAL_ROOT` + vault key).
    pub connection_store: Option<Arc<crate::connections::VaultedConnectionStore>>,
    /// The live read path used to validate submitted connections (no bearer
    /// token: the console never touches the JSON API's authorization).
    pub connection_validator: Option<Arc<LiveAuditor>>,
    /// The proposal files the runner writes (`APEX_PROPOSALS_ROOT`).
    pub proposal_store: Option<Arc<dyn crate::proposals::ProposalStore>>,
    /// The runner's alert log for incident states (`APEX_ALERT_FILE`, with
    /// `APEX_ALERTS_FILE` as a fallback).
    pub alerts_path: Option<std::path::PathBuf>,
    /// The Adyen collection log for payment reads (`APEX_COLLECTION_FILE`).
    pub collection_log_path: Option<std::path::PathBuf>,
    /// Process request counters behind `GET /metrics`.
    pub metrics: Arc<crate::metrics::Metrics>,
    /// The bounded metrics registry (the shared `apex-metrics` registry) the
    /// request middleware writes and `GET /metrics` renders alongside the
    /// hand-rolled counters. `AppState::new` creates a fresh one, so every
    /// test process (and every embedded server) counts its own requests.
    pub metrics_registry: Arc<apex_metrics::Registry>,
}

impl AppState {
    /// Build state from the two sources: no Adyen credentials, a
    /// [`SystemClock`], the default tolerance and a [`LedgerOnlyRunner`] over
    /// the given sources.
    pub fn new(audit_source: Arc<dyn AuditSource>, ledger_source: Arc<dyn LedgerSource>) -> Self {
        let cycle_runner: Arc<dyn CycleRunner> = Arc::new(LedgerOnlyRunner::new(
            Arc::clone(&audit_source),
            Arc::clone(&ledger_source),
        ));
        Self {
            audit_source,
            ledger_source,
            adyen: None,
            billing_transport: None,
            adyen_hmac_key: None,
            clock: Arc::new(SystemClock),
            cycle_runner,
            live_auditor: None,
            ledger_tokens: Arc::new(HashMap::new()),
            workspaces: None,
            signin: None,
            app_ads_proof: None,
            leads: None,
            watch: None,
            outcomes: None,
            collection: None,
            ledger_sessions: Arc::new(LedgerSessions::new(
                DEFAULT_SESSION_CAPACITY,
                SESSION_IDLE_MS,
            )),
            analytics: AnalyticsAccess::disabled(crate::analytics::NOT_CONFIGURED_REASON),
            lead_id_salt: Arc::new(crate::leads::DEFAULT_LEAD_ID_SALT.to_string()),
            connection_store: None,
            connection_validator: None,
            proposal_store: None,
            alerts_path: None,
            collection_log_path: None,
            metrics: Arc::new(crate::metrics::Metrics::new()),
            metrics_registry: Arc::new(apex_metrics::Registry::new()),
        }
    }

    /// Attach the analytics access (reader + emitter).
    #[must_use]
    pub fn with_analytics(mut self, analytics: AnalyticsAccess) -> Self {
        self.analytics = analytics;
        self
    }

    /// Attach the lead-id salt shared with the lead store.
    #[must_use]
    pub fn with_lead_id_salt(mut self, salt: impl Into<String>) -> Self {
        let salt = salt.into();
        if !salt.trim().is_empty() {
            self.lead_id_salt = Arc::new(salt);
        }
        self
    }

    /// Attach the vault-backed connection store and its validator.
    #[must_use]
    pub fn with_connection_store(
        mut self,
        store: Arc<crate::connections::VaultedConnectionStore>,
    ) -> Self {
        self.connection_store = Some(store);
        self
    }

    /// Attach the live read path used to validate connection submissions.
    #[must_use]
    pub fn with_connection_validator(mut self, validator: Arc<LiveAuditor>) -> Self {
        self.connection_validator = Some(validator);
        self
    }

    /// Attach the proposal store.
    #[must_use]
    pub fn with_proposal_store(mut self, store: Arc<dyn crate::proposals::ProposalStore>) -> Self {
        self.proposal_store = Some(store);
        self
    }

    /// Attach the runner's alert log path.
    #[must_use]
    pub fn with_alerts_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.alerts_path = Some(path.into());
        self
    }

    /// Attach the collection log path for billing reads.
    #[must_use]
    pub fn with_collection_log_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.collection_log_path = Some(path.into());
        self
    }

    /// Attach the lead store.
    #[must_use]
    pub fn with_leads(mut self, store: Arc<dyn LeadStore>) -> Self {
        self.leads = Some(store);
        self
    }

    /// Attach the setup-watch subscriber store.
    #[must_use]
    pub fn with_watch(mut self, store: Arc<apex_watch::SubscriberStore>) -> Self {
        self.watch = Some(store);
        self
    }

    /// Attach the frozen sales-outcomes appender.
    #[must_use]
    pub fn with_outcomes(mut self, appender: Arc<crate::outcomes::OutcomesAppender>) -> Self {
        self.outcomes = Some(appender);
        self
    }

    /// Attach the collection store the webhook records accepted events into.
    #[must_use]
    pub fn with_collection(mut self, store: Arc<dyn CollectionStore>) -> Self {
        self.collection = Some(store);
        self
    }

    /// Replace the console session store (tests pin small capacities and TTLs).
    #[must_use]
    pub fn with_ledger_sessions(mut self, sessions: Arc<LedgerSessions>) -> Self {
        self.ledger_sessions = sessions;
        self
    }

    /// Attach the per-publisher ledger tokens.
    #[must_use]
    pub fn with_ledger_tokens(mut self, tokens: HashMap<String, Vec<u8>>) -> Self {
        self.ledger_tokens = Arc::new(tokens);
        self
    }

    /// Attach one publisher's ledger token.
    #[must_use]
    pub fn with_ledger_token(
        mut self,
        publisher: impl Into<String>,
        token: impl Into<Vec<u8>>,
    ) -> Self {
        let mut tokens = (*self.ledger_tokens).clone();
        tokens.insert(publisher.into(), token.into());
        self.ledger_tokens = Arc::new(tokens);
        self
    }

    /// Attach the live workspace token file (the self-serve seam).
    #[must_use]
    pub fn with_workspaces(mut self, workspaces: Arc<crate::signin::WorkspaceTokens>) -> Self {
        self.workspaces = Some(workspaces);
        self
    }

    /// Attach the one-time sign-in link service.
    #[must_use]
    pub fn with_signin(mut self, signin: Arc<crate::signin::SigninService>) -> Self {
        self.signin = Some(signin);
        self
    }

    /// Attach the new-workspace ownership proof source.
    #[must_use]
    pub fn with_app_ads_proof(
        mut self,
        proof: Arc<dyn crate::signin::WorkspaceProofSource>,
    ) -> Self {
        self.app_ads_proof = Some(proof);
        self
    }

    /// Authorize a read of `publisher`'s ledger. Fails closed: a publisher
    /// with no configured token is refused (`503`), and a missing or wrong
    /// token is `401`. The comparison is constant-time. Tokens come from the
    /// boot-time map first; the live workspace file is the fallback, so a
    /// workspace created by a free-audit submission authorizes without a
    /// restart.
    pub fn authorize_ledger(
        &self,
        publisher: &str,
        authorization: Option<&str>,
    ) -> Result<(), ApiError> {
        let expected = match self.ledger_tokens.get(publisher) {
            Some(token) => token.clone(),
            None => match self.workspaces.as_ref().and_then(|w| w.token(publisher)) {
                Some(token) => token.into_bytes(),
                None => return Err(ApiError::LedgerAuthNotConfigured),
            },
        };
        if expected.is_empty() {
            return Err(ApiError::Unauthorized);
        }
        let presented = authorization
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or_default();
        if constant_time_eq(presented.as_bytes(), expected.as_slice()) {
            Ok(())
        } else {
            Err(ApiError::Unauthorized)
        }
    }

    /// Attach the live read-only auditor.
    #[must_use]
    pub fn with_live_auditor(mut self, auditor: Arc<LiveAuditor>) -> Self {
        self.live_auditor = Some(auditor);
        self
    }

    /// Attach an Adyen client.
    #[must_use]
    pub fn with_adyen(mut self, adyen: Arc<AdyenClient>) -> Self {
        self.adyen = Some(adyen);
        self
    }

    /// Attach the transport Adyen Checkout calls travel through.
    #[must_use]
    pub fn with_billing_transport(
        mut self,
        transport: Arc<dyn apex_connectors::transport::Transport>,
    ) -> Self {
        self.billing_transport = Some(Arc::new(BillingTransport(transport)));
        self
    }

    /// Attach the webhook HMAC key alone (no Adyen client).
    #[must_use]
    pub fn with_adyen_hmac_key(mut self, hmac_key: impl Into<Vec<u8>>) -> Self {
        self.adyen_hmac_key = Some(hmac_key.into());
        self
    }

    /// Inject a clock (tests pin a [`crate::FixedClock`]).
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Inject a cycle runner (a deployment that wires `apex-autopilot`).
    #[must_use]
    pub fn with_cycle_runner(mut self, runner: Arc<dyn CycleRunner>) -> Self {
        self.cycle_runner = runner;
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::clock::FixedClock;
    use crate::sources::StaticAuditSource;
    use crate::sources::StaticLedgerSource;

    use super::*;

    #[test]
    fn defaults_are_read_only_without_adyen_and_with_the_system_clock() {
        let state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        );
        assert!(state.adyen.is_none());
        assert!(state.adyen_hmac_key.is_none());
        assert!(format!("{:?}", state.clock).contains("SystemClock"));
    }

    #[test]
    fn every_builder_sets_its_field() {
        let dir = std::env::temp_dir().join(format!("apex-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        )
        .with_adyen(Arc::new(
            apex_billing::AdyenClient::new("k", "m", b"h".to_vec(), "https://x").expect("client"),
        ))
        .with_leads(Arc::new(crate::leads::FileLeadStore::new(
            dir.join("leads.jsonl"),
        )))
        .with_collection(Arc::new(
            apex_billing::FileCollectionStore::open(&dir.join("coll.jsonl")).expect("coll"),
        ))
        .with_ledger_tokens(std::collections::HashMap::from([(
            "pub1".to_string(),
            b"t".to_vec(),
        )]))
        .with_ledger_token("pub2", b"t2".to_vec());
        assert!(state.adyen.is_some());
        assert!(state.leads.is_some());
        assert!(state.collection.is_some());
        assert_eq!(state.ledger_tokens.len(), 2);
        assert!(state.authorize_ledger("pub1", Some("Bearer t")).is_ok());
        assert!(state.authorize_ledger("pub2", Some("Bearer t2")).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builders_replace_each_field() {
        let state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        )
        .with_adyen_hmac_key(b"hmac-key".to_vec())
        .with_clock(Arc::new(FixedClock::new(5)));
        assert_eq!(state.adyen_hmac_key.as_deref(), Some(&b"hmac-key"[..]));
        assert_eq!(state.clock.now_unix_s(), 5);
    }
}
