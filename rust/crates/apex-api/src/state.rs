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

use crate::clock::{Clock, SystemClock};
use crate::cycle::{CycleRunner, LedgerOnlyRunner};
use crate::error::ApiError;
use crate::leads::LeadStore;
use crate::live::{constant_time_eq, LiveAuditor};
use crate::sources::{AuditSource, LedgerSource};

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
    /// Lead capture behind `POST /v1/leads`. `None` makes the route answer
    /// `503 lead_capture_not_configured`.
    pub leads: Option<Arc<dyn LeadStore>>,
    /// Process request counters behind `GET /metrics`.
    pub metrics: Arc<crate::metrics::Metrics>,
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
            adyen_hmac_key: None,
            clock: Arc::new(SystemClock),
            cycle_runner,
            live_auditor: None,
            ledger_tokens: Arc::new(HashMap::new()),
            leads: None,
            metrics: Arc::new(crate::metrics::Metrics::new()),
        }
    }

    /// Attach the lead store.
    #[must_use]
    pub fn with_leads(mut self, store: Arc<dyn LeadStore>) -> Self {
        self.leads = Some(store);
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

    /// Authorize a read of `publisher`'s ledger. Fails closed: a publisher
    /// with no configured token is refused (`503`), and a missing or wrong
    /// token is `401`. The comparison is constant-time.
    pub fn authorize_ledger(
        &self,
        publisher: &str,
        authorization: Option<&str>,
    ) -> Result<(), ApiError> {
        let Some(expected) = self.ledger_tokens.get(publisher) else {
            return Err(ApiError::LedgerAuthNotConfigured);
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
