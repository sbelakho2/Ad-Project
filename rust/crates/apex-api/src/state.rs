//! Shared server state [`AppState`].
//!
//! Everything the router needs is injected: sources, credentials, clock and
//! the cycle runner. `AppState` is `Clone` (it only holds `Arc`s and small
//! values) and every clone shares the same sources — no per-request state is
//! kept anywhere, which is what makes the read-only deployment trivially
//! reproducible.

use std::sync::Arc;

use apex_billing::StripeClient;

use crate::clock::{Clock, SystemClock};
use crate::cycle::{CycleRunner, LedgerOnlyRunner};
use crate::sources::{AuditSource, LedgerSource};

/// The webhook freshness tolerance used when [`AppState::new`] builds the
/// state: 300 seconds, the customary Stripe default.
pub const DEFAULT_STRIPE_TOLERANCE_SECS: u64 = 300;

/// The complete dependency set of the HTTP surface.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Where audit snapshots come from.
    pub audit_source: Arc<dyn AuditSource>,
    /// Where verified ledger records come from.
    pub ledger_source: Arc<dyn LedgerSource>,
    /// Stripe API client (webhook secret carrier in this deployment).
    pub stripe: Option<Arc<StripeClient>>,
    /// Explicit webhook secret; takes precedence over the client's secret.
    pub webhook_secret: Option<Vec<u8>>,
    /// Maximum accepted Stripe signature timestamp skew, in seconds.
    pub stripe_tolerance_secs: u64,
    /// The clock used for Stripe signature freshness.
    pub clock: Arc<dyn Clock>,
    /// The cycle runner behind `POST /v1/cycle/{publisher}`.
    pub cycle_runner: Arc<dyn CycleRunner>,
}

impl AppState {
    /// Build state from the two sources: no Stripe credentials, a
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
            stripe: None,
            webhook_secret: None,
            stripe_tolerance_secs: DEFAULT_STRIPE_TOLERANCE_SECS,
            clock: Arc::new(SystemClock),
            cycle_runner,
        }
    }

    /// Attach a Stripe client.
    #[must_use]
    pub fn with_stripe(mut self, stripe: Arc<StripeClient>) -> Self {
        self.stripe = Some(stripe);
        self
    }

    /// Set an explicit webhook secret (preferred over the client's secret).
    #[must_use]
    pub fn with_webhook_secret(mut self, secret: impl Into<Vec<u8>>) -> Self {
        self.webhook_secret = Some(secret.into());
        self
    }

    /// Override the webhook freshness tolerance.
    #[must_use]
    pub fn with_stripe_tolerance_secs(mut self, secs: u64) -> Self {
        self.stripe_tolerance_secs = secs;
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
    fn defaults_are_read_only_with_the_stripe_tolerance_and_system_clock() {
        let state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        );
        assert!(state.stripe.is_none());
        assert!(state.webhook_secret.is_none());
        assert_eq!(state.stripe_tolerance_secs, DEFAULT_STRIPE_TOLERANCE_SECS);
        assert!(state.stripe_tolerance_secs > 0);
        assert!(format!("{:?}", state.clock).contains("SystemClock"));
    }

    #[test]
    fn builders_replace_each_field() {
        let state = AppState::new(
            Arc::new(StaticAuditSource::default()),
            Arc::new(StaticLedgerSource::default()),
        )
        .with_webhook_secret(b"whsec".to_vec())
        .with_stripe_tolerance_secs(60)
        .with_clock(Arc::new(FixedClock::new(5)));
        assert_eq!(state.webhook_secret.as_deref(), Some(&b"whsec"[..]));
        assert_eq!(state.stripe_tolerance_secs, 60);
        assert_eq!(state.clock.now_unix_s(), 5);
    }
}
