//! The scheduled job runner: the daily measurement pass and the monthly
//! billing pass over a durable ledger.
//!
//! Nothing here is a daemon; each method is one idempotent job an operator (or
//! a systemd timer) invokes on a schedule. The daily pass records the raw
//! platform pull, settles every experiment whose window is complete, and
//! leaves running experiments untouched. The monthly pass bills each kept
//! change for the month from the same signed ledger.
//!
//! # VAT issuance (monthly)
//!
//! For every publisher due a fee the monthly pass loads the seller's billing
//! profile (`apex_billing::profile::BillingProfile::from_env()`, or the
//! profile injected with [`Runner::with_billing_profile`]) and the
//! publisher's customer profile from
//! `<connections root>/<publisher>.billing.json` — the connections root is
//! the vault root the console writes (`APEX_CONNECTIONS_ROOT`, falling back
//! to `APEX_CREDENTIAL_ROOT`); the file is exactly the
//! `apex_billing::profile::CustomerProfile` JSON shape (`legalName`,
//! `addressLines`, `country`, `vatNumber`). The invoice gets the customer
//! ([`apex_billing::Invoice::with_customer`]), its treatment is decided
//! against the seller profile
//! ([`apex_billing::Invoice::apply_vat_treatment`]), the next gapless number
//! is assigned through the invoice register
//! ([`apex_billing::InvoiceRegister::open`] at
//! [`apex_billing::DEFAULT_REGISTER_PATH`] unless
//! [`Runner::with_register_path`] names another), and the lawful document is
//! rendered ([`apex_billing::Invoice::render_vat_invoice`]). Every missing
//! piece is a typed refusal recorded in the report's `invoice_refusal` — a
//! half-empty invoice is never printed, and a refused publisher consumes no
//! register number and is never collected.
//!
//! # Fleet mode
//!
//! One [`Runner`] serves one publisher. The multi-publisher `dispatch_fleet`
//! path in `cli` builds one runner per registered publisher (see
//! `crate::registry`), so every per-publisher collaborator below (alerts,
//! notifications, events, priors, verifier) stays per publisher; fleet-level
//! work (projection refresh, verified payment events) runs once per run in
//! the fleet driver. With [`Runner::with_owner_scoped_operator_jobs`] the
//! daily pass claims only operator jobs whose target its own ledger owns.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use apex_core::PlatformId;
use apex_lift::guardrails::FillFormat;
use apex_metrics::vocab::{Decision, RestoreOperation, RestoreOutcome};
use apex_proof::{ChangeKind, LedgerLog, LedgerRecord};

use crate::alerts::{Alert, AlertSink, NoopAlertSink};
use crate::ledger_index::{parse_lift_record, LedgerIndex};
use crate::model::{AutopilotPolicy, PlanError, SettleOutcome};
use crate::ops::PlatformOps;
use crate::settle::settle_experiment_indexed;

/// How many consecutive daily passes may fail to read a running test's scope
/// settings before the publisher-edit check escalates to Unknown and voids
/// (LL). A single provider timeout or 5xx holds the test pending instead.
const SETTINGS_READ_RETRY_PASSES: u32 = 3;

/// What one daily pass did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DailyReport {
    /// Experiments whose reporting was pulled (one data-pull record each).
    pub pulled: u32,
    /// Experiments settled this pass.
    pub settled: u32,
    /// Samples observed as still pending this pass.
    pub pending: u32,
    /// Reporting pulls that failed this pass.
    pub pull_failures: u32,
    /// Settle outcomes in ledger order (experiment id, status label).
    pub outcomes: Vec<(String, String)>,
    /// Experiments that could not be processed (id, reason). The pass
    /// continues past a failure so one bad test never starves the rest.
    pub failures: Vec<(String, String)>,
    /// Operator jobs consumed this pass, when the jobs directory is
    /// configured.
    pub operator_jobs: Option<crate::operator_jobs::OperatorJobsReport>,
    /// Payment events processed this pass, when the collection file is
    /// configured.
    pub payments: Option<crate::payment_events::PaymentsReport>,
    /// The dunning sweep outcome: unpaid intents past their due date, the
    /// retry rows written and the payment-link reminders sent. `None` when
    /// this publisher keeps no payment-intent journal.
    pub dunning: Option<DunningReport>,
    /// Projection refresh outcome, when the analytics store is configured.
    pub projections: Option<crate::projections_job::ProjectionRefreshReport>,
    /// Why the projection refresh failed, when it did; the failure stays
    /// visible and the pass reports it instead of hiding it.
    pub projection_failure: Option<String>,
}

/// What one daily dunning sweep did (GG): the retry schedule has a writer,
/// the unpaid invoice's link is re-sent, and both are visible in the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DunningReport {
    /// Unpaid invoices found past their due date this pass.
    pub unpaid_invoices: u32,
    /// Retry rows appended to the retry schedule file.
    pub retry_rows: u32,
    /// Payment-link reminders (re-)sent this pass.
    pub reminders: u32,
}

/// The account-readiness counts one daily pull pass observed, mapped to the
/// bounded readiness vocabulary. Every account the pass attempted lands in
/// exactly one class:
///
/// - final: the pull returned data covering the last closed day,
/// - partial: the pull returned data, but only up to an earlier day (the
///   window is not fully reported yet),
/// - disconnected: the pull was refused on auth (the credential cannot read),
/// - stale: the pull failed for any other reason, so the account was not
///   refreshed this pass.
///
/// Accounts the pass never attempted are unknown to it and are not counted;
/// the gauges are only written when at least one account was attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct PullReadiness {
    final_accounts: u32,
    partial_accounts: u32,
    stale_accounts: u32,
    disconnected_accounts: u32,
}

impl PullReadiness {
    /// How many accounts the pass attempted.
    fn attempted(&self) -> u32 {
        self.final_accounts
            .saturating_add(self.partial_accounts)
            .saturating_add(self.stale_accounts)
            .saturating_add(self.disconnected_accounts)
    }
}

/// The runner holds the platform adapter and the policy; the ledger is passed
/// per job so the same runner serves any publisher's ledger.
#[derive(Clone)]
pub struct Runner {
    /// The primary adapter (the publisher's validated platform). Kept as the
    /// default for single-platform deployment and every caller that asks for
    /// "the platform".
    ops: Arc<dyn PlatformOps>,
    /// The per-platform adapters of a multi-platform publisher, keyed by the
    /// snapshot/proposal platform. A proposal is always executed through the
    /// adapter of the platform it belongs to, never through the primary one.
    platform_ops: HashMap<PlatformId, Arc<dyn PlatformOps>>,
    policy: AutopilotPolicy,
    fill_format: FillFormat,
    alerts: Arc<dyn AlertSink>,
    /// The publisher this runner bills; required for the monthly pass.
    publisher_id: String,
    /// Override of the seller profile's domestic VAT rate in basis points
    /// (0 = use the profile's own rate).
    vat_rate_bp: u16,
    /// The seller's billing profile; when absent the monthly pass loads it
    /// from the environment (`APEX_BILLING_PROFILE`).
    billing_profile: Option<apex_billing::BillingProfile>,
    /// The VIES seam reverse charge is confirmed through; `None` uses the
    /// production client. Tests inject a stub so no test reaches the network.
    vies: Option<Arc<dyn apex_billing::ViesClient>>,
    /// The vault root holding the per-publisher customer billing profiles
    /// (`<root>/<publisher>.billing.json`).
    connections_root: Option<std::path::PathBuf>,
    /// The invoice register path; `None` means
    /// [`apex_billing::DEFAULT_REGISTER_PATH`].
    register_path: Option<std::path::PathBuf>,
    /// A3 M1: the persistent account-metric/app-ads watch. `None` keeps the
    /// lifecycle off (no baseline file is read or written).
    account_watch: Option<crate::account_watch::AccountWatch>,
    /// The durable payment-intent journal; `None` means the default beside the
    /// register (`<register parent>/payment-intents.jsonl`).
    payment_intents_path: Option<std::path::PathBuf>,
    /// Whether the operator-jobs consumer claims only jobs this ledger owns
    /// (the fleet daily pass), or every queued job (legacy single mode).
    owner_scoped_operator_jobs: bool,
    /// The payment-provider collection path: provider + transport. Absent in
    /// dry-run deployments, which record billing but never collect. The
    /// runner holds the neutral [`apex_billing::PaymentProvider`] seam, not
    /// a concrete PSP client: the deployment wires Mollie behind it and a
    /// different provider is a different `impl`.
    provider: Option<(
        Arc<dyn apex_billing::PaymentProvider>,
        Arc<dyn apex_connectors::transport::Transport>,
    )>,
    /// The bucket model and where it persists; learning follows results.
    priors: Option<apex_lift::priors::LiftPriors>,
    priors_path: Option<std::path::PathBuf>,
    /// The bounded telemetry registry handle, when configured.
    telemetry: Option<crate::telemetry::RunnerTelemetry>,
    /// The analytics emitter, when configured.
    events: Option<Arc<crate::events::RunnerEvents>>,
    /// The notification plan, when contacts are configured.
    notify: Option<Arc<crate::notify::NotifyPlan>>,
    /// The operator-jobs directory consumed by the daily pass.
    operator_jobs_dir: Option<std::path::PathBuf>,
    /// Where the console reads proposals and their approval decisions. When
    /// set, a held proposal is written here and an approval recorded by the
    /// console unlocks the same change on the next cycle.
    proposals_root: Option<std::path::PathBuf>,
    /// The verified provider-event collection file.
    payment_events_path: Option<std::path::PathBuf>,
    /// The consumed-offset cursor beside the collection file; `None` means the
    /// default `<collection>.cursor` next to it.
    payment_cursor_path: Option<std::path::PathBuf>,
    /// The collected-reference ledger that payment messages read balances
    /// from; rows are appended when collection runs.
    payment_ledger_path: Option<std::path::PathBuf>,
    /// The scheduled-retry file written by the finance boundary.
    payment_retries_path: Option<std::path::PathBuf>,
    /// The ledger verifier used for the run's verification line and events.
    verifier: Option<Arc<dyn crate::verify::LedgerVerifier>>,
    /// The daily projection refresh, when the analytics store is configured.
    projections: Option<crate::projections_job::RunnerProjections>,
    /// The control plane's intervention register; when configured, the
    /// cycle skips any platform whose latest applicable record is a pause.
    interventions_path: Option<std::path::PathBuf>,
    /// How many reporting pulls one daily pass may have in flight. The reads
    /// are independent; ledger appends and settlements stay serial.
    reporting_concurrency: usize,
    /// The fleet-wide operator-jobs lock: concurrent publisher passes share
    /// one jobs directory, so claiming/transitioning a job is serialized.
    operator_jobs_lock: Option<Arc<tokio::sync::Mutex<()>>>,
}

impl std::fmt::Debug for Runner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runner")
            .field("policy", &self.policy)
            .finish()
    }
}

// Prior-model batch instrumentation. A daily pass clones the model once and
// writes it once, however many experiments settled; the test asserts both.
#[cfg(test)]
thread_local! {
    static PRIOR_WRITES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static PRIOR_CLONES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_prior_io() {
    PRIOR_WRITES.with(|count| count.set(0));
    PRIOR_CLONES.with(|count| count.set(0));
}

#[cfg(test)]
fn prior_writes() -> u64 {
    PRIOR_WRITES.with(std::cell::Cell::get)
}

#[cfg(test)]
fn prior_clones() -> u64 {
    PRIOR_CLONES.with(std::cell::Cell::get)
}

fn persist_priors(path: &std::path::Path, priors: &apex_lift::priors::LiftPriors) {
    #[cfg(test)]
    PRIOR_WRITES.with(|count| count.set(count.get() + 1));
    let json = match serde_json::to_string_pretty(priors) {
        Ok(json) => json,
        Err(error) => {
            eprintln!("apex-runner: lift priors could not be serialized: {error}");
            return;
        }
    };
    // The priors feed every future decision: a truncated or torn file must
    // surface, never reload as an empty model that forgets everything.
    let written = (|| -> std::io::Result<()> {
        let tmp = path.with_extension("priors.tmp");
        let mut file = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut file, json.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            let parent = if parent.as_os_str().is_empty() {
                std::path::Path::new(".")
            } else {
                parent
            };
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = written {
        eprintln!(
            "apex-runner: lift priors could not be persisted to {}: {error}",
            path.display()
        );
    }
}

impl Runner {
    pub fn new(
        ops: Arc<dyn PlatformOps>,
        policy: AutopilotPolicy,
        fill_format: FillFormat,
    ) -> Self {
        Self {
            ops,
            platform_ops: HashMap::new(),
            policy,
            fill_format,
            alerts: Arc::new(NoopAlertSink),
            publisher_id: String::new(),
            vat_rate_bp: 0,
            billing_profile: None,
            vies: None,
            connections_root: None,
            account_watch: None,
            register_path: None,
            payment_intents_path: None,
            owner_scoped_operator_jobs: false,
            provider: None,
            priors: None,
            priors_path: None,
            telemetry: None,
            events: None,
            projections: None,
            notify: None,
            operator_jobs_dir: None,
            proposals_root: None,
            payment_events_path: None,
            payment_cursor_path: None,
            payment_ledger_path: None,
            payment_retries_path: None,
            verifier: None,
            interventions_path: None,
            reporting_concurrency: 4,
            operator_jobs_lock: None,
        }
    }

    /// Attach the adapter for one platform. A multi-platform publisher has one
    /// entry per vaulted platform, so a LevelPlay or AdMob proposal is never
    /// sent through the MAX adapter.
    #[must_use]
    pub fn with_platform_ops(mut self, platform: PlatformId, ops: Arc<dyn PlatformOps>) -> Self {
        self.platform_ops.insert(platform, ops);
        self
    }

    /// The adapter that executes `platform`'s work. The primary adapter is the
    /// fallback only when it *is* that platform; a missing secondary adapter is
    /// a typed refusal, never a silent cross-platform route.
    pub fn ops_for(&self, platform: PlatformId) -> Result<&dyn PlatformOps, PlanError> {
        if let Some(ops) = self.platform_ops.get(&platform) {
            return Ok(ops.as_ref());
        }
        if self.ops.platform() == platform {
            return Ok(self.ops.as_ref());
        }
        Err(PlanError::Validation(format!(
            "no runner adapter is configured for platform {}",
            platform.as_str()
        )))
    }
}

/// The runner is the create-recovery's adapter source: each intent replays on
/// the adapter of the platform it was issued on.
impl crate::settle::PlatformOpsSource for Runner {
    fn adapter(&self, platform: PlatformId) -> Result<&dyn PlatformOps, PlanError> {
        self.ops_for(platform)
    }
}

impl Runner {
    /// Read fleet pause/resume interventions from `path` before each cycle.
    #[must_use]
    pub fn with_interventions(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.interventions_path = Some(path.into());
        self
    }

    /// The intervention register path, when configured.
    pub fn interventions_path(&self) -> Option<&std::path::Path> {
        self.interventions_path.as_deref()
    }

    /// The directory the console reads proposal files and decisions from.
    #[must_use]
    pub fn with_proposals_root(mut self, root: impl Into<std::path::PathBuf>) -> Self {
        self.proposals_root = Some(root.into());
        self
    }

    /// The proposal root, when configured.
    pub fn proposals_root(&self) -> Option<&std::path::Path> {
        self.proposals_root.as_deref()
    }

    /// The publisher this runner bills (required for the monthly pass).
    #[must_use]
    pub fn with_publisher(mut self, publisher_id: impl Into<String>) -> Self {
        self.publisher_id = publisher_id.into();
        self
    }

    /// Override the seller profile's domestic VAT rate, in basis points. The
    /// monthly pass applies the rate through the profile's VAT treatment
    /// decision; 0 (the default) keeps the profile's own `vatRateBp`. The
    /// historical `APEX_BILLING_VAT_RATE_BP`/`APEX_INVOICE_VAT_BP` reads land
    /// here.
    #[must_use]
    pub fn with_vat_rate(mut self, vat_rate_bp: u16) -> Self {
        self.vat_rate_bp = vat_rate_bp;
        self
    }

    /// Inject the seller's billing profile instead of loading the deployment
    /// file named by `APEX_BILLING_PROFILE` at monthly-pass time.
    #[must_use]
    pub fn with_billing_profile(mut self, profile: apex_billing::BillingProfile) -> Self {
        self.billing_profile = Some(profile);
        self
    }

    /// Inject the VIES register seam instead of the production client. Tests
    /// use this with a stub so no test reaches the network.
    #[must_use]
    pub fn with_vies(mut self, vies: Arc<dyn apex_billing::ViesClient>) -> Self {
        self.vies = Some(vies);
        self
    }

    /// The vault root whose `<publisher>.billing.json` customer profiles the
    /// monthly pass attaches (the same root the runner registry reads).
    #[must_use]
    pub fn with_connections_root(mut self, root: impl Into<std::path::PathBuf>) -> Self {
        self.connections_root = Some(root.into());
        self
    }

    /// A3 M1: attach the persistent account-metric/app-ads watch. The
    /// threshold is validated here, so a value above 1.0 is a typed error.
    pub fn with_account_watch(
        mut self,
        path: impl Into<std::path::PathBuf>,
        threshold: f64,
    ) -> Result<Self, String> {
        self.account_watch = Some(crate::account_watch::AccountWatch::new(path, threshold)?);
        Ok(self)
    }

    /// A3 M1: compare every snapshot with the persisted baseline and emit
    /// alerts. The first observation records the baseline; delivery is
    /// fail-closed inside [`crate::account_watch::AccountWatch::observe`].
    pub async fn observe_account_snapshots(
        &self,
        snapshots: &[apex_audit::Snapshot],
        now_unix_ms: i64,
    ) -> Result<Vec<crate::alerts::Alert>, String> {
        let Some(watch) = self.account_watch.as_ref() else {
            return Ok(Vec::new());
        };
        let publisher = self.publisher_id();
        let mut alerts = Vec::new();
        for snapshot in snapshots {
            alerts.extend(
                watch
                    .observe(
                        publisher,
                        snapshot.platform.as_str(),
                        snapshot,
                        now_unix_ms,
                        self.alerts.as_ref(),
                    )
                    .await?,
            );
        }
        Ok(alerts)
    }

    /// The invoice register path; the default is
    /// [`apex_billing::DEFAULT_REGISTER_PATH`].
    #[must_use]
    pub fn with_register_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.register_path = Some(path.into());
        self
    }

    /// The durable payment-intent journal. The default is
    /// `payment-intents.jsonl` beside the invoice register, so the intent is
    /// recorded before any provider call on the same durable volume.
    #[must_use]
    pub fn with_payment_intents(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.payment_intents_path = Some(path.into());
        self
    }

    /// Claim only operator jobs whose target this ledger owns (fleet mode);
    /// the legacy single-publisher pass claims every queued job.
    #[must_use]
    pub fn with_owner_scoped_operator_jobs(mut self, scoped: bool) -> Self {
        self.owner_scoped_operator_jobs = scoped;
        self
    }

    /// Serialize operator-job consumption across the publisher passes of one
    /// fleet: every pass reads the same jobs directory, so the claim and the
    /// two state transitions must not interleave.
    #[must_use]
    pub fn with_operator_jobs_lock(mut self, lock: Arc<tokio::sync::Mutex<()>>) -> Self {
        self.operator_jobs_lock = Some(lock);
        self
    }

    /// The maximum number of reporting pulls in flight in one daily pass.
    #[must_use]
    pub fn with_reporting_concurrency(mut self, concurrency: usize) -> Self {
        self.reporting_concurrency = concurrency.max(1);
        self
    }

    /// Load the recorded managed ad-ops plan. `Ok(None)` when nothing is
    /// recorded — a plan that was never activated bills nothing. Without a
    /// connections root there is no plan state to read. Monitoring has no
    /// plan state: it is free and runs for every connected account.
    fn load_managed_plan_state(
        &self,
        publisher_id: &str,
    ) -> Result<Option<apex_billing::PlanState>, PlanError> {
        let Some(root) = &self.connections_root else {
            return Ok(None);
        };
        let plan = apex_billing::PlanId::Managed;
        let path = apex_billing::PlanState::path_for(root, publisher_id, plan);
        apex_billing::PlanState::load(&path)
            .map_err(|error| PlanError::Validation(format!("{} state: {error}", plan.slug())))
    }

    /// Attach the payment-provider collection path (provider + transport).
    /// The provider is wired through the neutral
    /// [`apex_billing::PaymentProvider`] trait; the deployment passes the
    /// Mollie client (`Arc::new(MollieClient::new(..))`), which coerces to
    /// the trait object, and the monthly pass collects through the trait.
    #[must_use]
    pub fn with_payment_provider(
        mut self,
        provider: Arc<dyn apex_billing::PaymentProvider>,
        transport: Arc<dyn apex_connectors::transport::Transport>,
    ) -> Self {
        self.provider = Some((provider, transport));
        self
    }

    /// Load the bucket model and persist updates to `path` after every
    /// settled test, so the next cycle's proposals learn from results.
    #[must_use]
    pub fn with_priors(
        mut self,
        priors: apex_lift::priors::LiftPriors,
        path: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.priors = Some(priors);
        self.priors_path = Some(path.into());
        self
    }

    /// Attach the alert sink that receives customer-impact alerts.
    #[must_use]
    pub fn with_alerts(mut self, alerts: Arc<dyn AlertSink>) -> Self {
        self.alerts = alerts;
        self
    }

    /// Attach the bounded telemetry registry handle.
    #[must_use]
    pub fn with_telemetry(mut self, telemetry: crate::telemetry::RunnerTelemetry) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    /// Attach the analytics emitter.
    #[must_use]
    pub fn with_events(mut self, events: Arc<crate::events::RunnerEvents>) -> Self {
        self.events = Some(events);
        self
    }

    /// Attach the notification plan.
    #[must_use]
    pub fn with_notifications(mut self, notify: Arc<crate::notify::NotifyPlan>) -> Self {
        self.notify = Some(notify);
        self
    }

    /// Refresh the analytics projections at the end of the daily pass.
    #[must_use]
    pub fn with_projections(
        mut self,
        projections: crate::projections_job::RunnerProjections,
    ) -> Self {
        self.projections = Some(projections);
        self
    }

    /// Consume operator jobs from `dir` in the daily pass.
    #[must_use]
    pub fn with_operator_jobs(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.operator_jobs_dir = Some(dir.into());
        self
    }

    /// Process verified payment events from `events` (the collection file),
    /// resolving received-message balances from `ledger` and reading
    /// scheduled retries from `retries`.
    #[must_use]
    pub fn with_payment_events(
        mut self,
        events: impl Into<std::path::PathBuf>,
        ledger: Option<std::path::PathBuf>,
        retries: Option<std::path::PathBuf>,
    ) -> Self {
        self.payment_events_path = Some(events.into());
        self.payment_ledger_path = ledger;
        self.payment_retries_path = retries;
        self
    }

    /// Override the consumed-offset cursor file for the payment collection
    /// (`APEX_PAYMENT_CURSOR`); the default is `<collection>.cursor` beside
    /// the collection file.
    #[must_use]
    pub fn with_payment_cursor(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.payment_cursor_path = Some(path.into());
        self
    }

    /// Attach the ledger verifier used for the run's verification line and
    /// the `ledger_verified` events.
    #[must_use]
    pub fn with_verifier(mut self, verifier: Arc<dyn crate::verify::LedgerVerifier>) -> Self {
        self.verifier = Some(verifier);
        self
    }

    /// The telemetry handle, when configured.
    pub fn telemetry(&self) -> Option<&crate::telemetry::RunnerTelemetry> {
        self.telemetry.as_ref()
    }

    /// The analytics emitter, when configured.
    pub fn events(&self) -> Option<&Arc<crate::events::RunnerEvents>> {
        self.events.as_ref()
    }

    /// The notification plan, when configured.
    pub fn notify(&self) -> Option<&Arc<crate::notify::NotifyPlan>> {
        self.notify.as_ref()
    }

    /// The ledger verifier, when configured.
    pub fn verifier(&self) -> Option<&Arc<dyn crate::verify::LedgerVerifier>> {
        self.verifier.as_ref()
    }

    /// One daily pass: pull reporting for every running experiment, record the
    /// pull, and settle any whose window has completed. A failure on one
    /// experiment is recorded (and alerted) without starving the rest.
    ///
    /// The pass also consumes operator jobs and verified payment events when
    /// they are configured, and every pull, decision, guardrail and restore
    /// leaves a durable analytics event beside its ledger record.
    pub async fn run_daily(
        &self,
        ledger: &mut dyn LedgerLog,
        now_unix_ms: i64,
    ) -> Result<DailyReport, PlanError> {
        let mut report = DailyReport::default();
        let job_id = crate::cli::run_reference(now_unix_ms);
        let today = now_unix_ms.div_euclid(86_400_000);
        let range_end = crate::calendar::format_day(today - 1);

        // Operator jobs first: a requested re-read or reconcile is a
        // read-only recovery step and never blocks the measurement pass.
        // Fleet publisher passes run concurrently but share one jobs
        // directory; the fleet-wide lock makes claim/transition atomic.
        let _operator_jobs_guard = if self.operator_jobs_dir.is_some() {
            match &self.operator_jobs_lock {
                Some(lock) => Some(lock.lock().await),
                None => None,
            }
        } else {
            None
        };
        if let Some(dir) = &self.operator_jobs_dir {
            // The due set and the owned-target set are computed once per pass
            // and reused for every job: neither the ledger scan nor the
            // change-id scan repeats per job.
            let due_set: std::collections::HashSet<String> = self
                .due_rechecks(ledger, now_unix_ms)?
                .into_iter()
                .map(|due| due.change_id)
                .collect();
            let due = |change_id: &str| due_set.contains(change_id);
            // In fleet mode a pass claims only the jobs its own ledger owns;
            // another publisher's job stays queued for its owner. The legacy
            // single-publisher pass claims everything, exactly as before.
            let owned = if self.owner_scoped_operator_jobs {
                owned_targets(ledger)
            } else {
                std::collections::HashSet::new()
            };
            let belongs = |job: &crate::operator_jobs::OperatorJob| {
                if !self.owner_scoped_operator_jobs {
                    return true;
                }
                match job.kind.as_str() {
                    // A reconnection request names the publisher, not an
                    // experiment; only that publisher's pass notifies it.
                    "reconnection_request" => job.target == self.publisher_id,
                    // A retry only re-enqueues the referenced job; whichever
                    // pass claims it, the retried job keeps its own owner.
                    "job_retry" => true,
                    _ => owned.contains(&job.target),
                }
            };
            let jobs = crate::operator_jobs::consume_operator_jobs(
                dir,
                self.ops.as_ref(),
                &due,
                &belongs,
                self.notify.as_deref(),
                self.telemetry.as_ref(),
                &self.publisher_id,
                now_unix_ms,
            )
            .await
            .map_err(|error| PlanError::Validation(error.to_string()))?;
            report.operator_jobs = Some(jobs);
        }

        // Verified provider events react into messages and analytics; the
        // ledger is untouched by payment processing. The pass consumes only
        // the collection file's new lines, through the consumed-offset
        // cursor, and advances the cursor atomically after processing: a
        // crash re-processes the last batch, which the dedup-keyed emits and
        // the delivery log absorb.
        if let Some(path) = &self.payment_events_path {
            let cursor_path = self
                .payment_cursor_path
                .clone()
                .unwrap_or_else(|| crate::payment_events::default_cursor_path(path));
            let cursor = crate::payment_events::read_payment_cursor(&cursor_path)
                .map_err(|error| PlanError::Validation(error.to_string()))?;
            let (events, consumed) =
                crate::payment_events::read_new_collection_events_from(path, cursor)
                    .map_err(|error| PlanError::Validation(error.to_string()))?;
            // Partial receipts accumulate from the durable historical file,
            // not only from this pass's new tail.
            let historical_received = crate::payment_events::read_received_totals(path)
                .map_err(|error| PlanError::Validation(error.to_string()))?;
            let payment_ledger = match &self.payment_ledger_path {
                Some(path) => crate::payment_events::read_payment_ledger(path)
                    .map_err(|error| PlanError::Validation(error.to_string()))?,
                None => Vec::new(),
            };
            let retries = match &self.payment_retries_path {
                Some(path) => crate::payment_events::read_scheduled_retries(path)
                    .map_err(|error| PlanError::Validation(error.to_string()))?,
                None => Vec::new(),
            };
            report.payments = Some(
                crate::payment_events::process_payment_events_with_history(
                    &historical_received,
                    &events,
                    &payment_ledger,
                    &retries,
                    self.notify.as_deref(),
                    self.events.as_deref(),
                    self.telemetry.as_ref(),
                    &self.publisher_id,
                    now_unix_ms,
                )
                .await
                .map_err(|error| PlanError::Validation(error.to_string()))?,
            );
            // Only a pass that read new lines advances the cursor; a read
            // failure above left it untouched so the batch is retried.
            if consumed != cursor {
                crate::payment_events::write_payment_cursor(&cursor_path, consumed)
                    .map_err(|error| PlanError::Validation(error.to_string()))?;
            }
        }

        // The dunning sweep (GG): unpaid intents past their due date get
        // their retry rows written and the payment link re-sent. A publisher
        // that keeps no intent journal sweeps nothing.
        report.dunning = self
            .dunning_sweep(now_unix_ms)
            .await
            .map_err(|error| PlanError::Validation(format!("dunning: {error}")))?;

        let mut readiness = PullReadiness::default();
        // One mutable model for the whole pass. Every settled experiment feeds
        // this copy; the file is written once at the end instead of cloning the
        // model and rewriting the pretty JSON per experiment.
        let mut priors = self.priors.clone();
        #[cfg(test)]
        if priors.is_some() {
            PRIOR_CLONES.with(|count| count.set(count.get() + 1));
        }
        let mut priors_dirty = false;
        // One index serves the running scan and every settle below; a
        // per-experiment ledger scan would repeat the whole history.
        let mut ledger_index = LedgerIndex::build(ledger);
        // Experiment-create recovery before the scan: an intent whose platform
        // create was accepted but whose bookkeeping never landed is replayed
        // here, so the recovered experiment is settled by this same pass.
        let create_recovery = crate::settle::reconcile_experiment_creates(
            ledger,
            &mut ledger_index,
            self,
            now_unix_ms,
        )
        .await;
        for (experiment_id, reason) in &create_recovery.failures {
            report
                .failures
                .push((experiment_id.clone(), format!("create replay: {reason}")));
        }
        let scan = running_scan_indexed(&ledger_index)?;
        // A signed result without its observation is a settled experiment
        // the settle path refuses. Report that typed refusal without pulling:
        // a repeated pass appends nothing for an experiment that can never
        // settle again.
        for experiment_id in &scan.settled_by_result {
            let refusal =
                PlanError::Validation(format!("experiment {experiment_id} is already settled"));
            report
                .failures
                .push((experiment_id.clone(), refusal.to_string()));
        }
        let experiments = scan.experiments;
        // A platform that documents no per-variant reporting (AdMob) is never
        // asked for daily arms: its settle path concludes from the platform
        // verdict instead. The capability is read from the experiment's own
        // platform adapter, never the publisher's primary one.
        let experiment_measures = |experiment: &RunningExperiment| {
            self.ops_for(experiment.platform)
                .is_ok_and(|ops| ops.measures_variants())
        };
        // Reporting reads are independent of one another and of the ledger;
        // announce them in the original order, then fetch them with bounded
        // concurrency. The ledger append and the settlement still run one
        // experiment at a time below, in this same original order.
        for experiment in &experiments {
            let range_start = experiment.started_day.clone();
            let attempt = 1u32;
            // An experiment that started today has no completed day to pull:
            // the event range would be inverted, and the validator rightly
            // refuses it. The pull still runs and returns an empty window.
            let announce = !experiment.pending_terminal
                && experiment_measures(experiment)
                && range_start <= range_end;
            if let Some(events) = self.events.as_ref().filter(|_| announce) {
                events
                    .report_pull_started(
                        &job_id,
                        "experiment_days",
                        &experiment.experiment_id,
                        &range_start,
                        &range_end,
                        attempt,
                        now_unix_ms,
                    )
                    .await;
            }
        }
        let mut pulls = self.pull_reporting_bounded(&experiments).await;
        for (index, experiment) in experiments.iter().enumerate() {
            let exp_measures = experiment_measures(experiment);
            let days = if exp_measures && !experiment.pending_terminal {
                let range_start = experiment.started_day.clone();
                let attempt = 1u32;
                let (days, pull_elapsed_ms) = match pulls.get_mut(index).and_then(Option::take) {
                    Some(pull) => pull,
                    None => {
                        return Err(PlanError::Validation(format!(
                            "missing reporting pull for {}",
                            experiment.experiment_id
                        )))
                    }
                };
                let days = match days {
                    Ok(days) => days,
                    Err(source) => {
                        let announce = range_start <= range_end;
                        if let Some(events) = self.events.as_ref().filter(|_| announce) {
                            events
                                .report_pull_failed(
                                    &job_id,
                                    "experiment_days",
                                    &experiment.experiment_id,
                                    &range_start,
                                    &range_end,
                                    attempt,
                                    error_class_of(&source),
                                    now_unix_ms,
                                )
                                .await;
                        }
                        if let Some(notify) = self.notify.as_ref() {
                            if matches!(source, apex_core::PlatformError::Auth { .. }) {
                                // The design's reporting-access message exists
                                // for exactly this state: the credential was
                                // refused.
                                notify
                                    .reporting_access(
                                        &self.publisher_id,
                                        &crate::calendar::format_day(today),
                                        now_unix_ms,
                                    )
                                    .await;
                            }
                        }
                        self.alert_pull_failure(&experiment.experiment_id, &source)
                            .await;
                        // The pull outcome is the readiness observation: an
                        // auth refusal means the credential is disconnected,
                        // any other failure leaves the account stale for now.
                        match source {
                            apex_core::PlatformError::Auth { .. } => {
                                readiness.disconnected_accounts += 1;
                            }
                            _ => readiness.stale_accounts += 1,
                        }
                        report.pull_failures += 1;
                        report
                            .failures
                            .push((experiment.experiment_id.clone(), source.to_string()));
                        continue;
                    }
                };
                let rows = apex_lift::emit::day_rows(&days.control, &days.test);
                let day = days
                    .control
                    .last()
                    .map(|d| d.day.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                let actual_start = days
                    .control
                    .first()
                    .or_else(|| days.test.first())
                    .map(|d| d.day.clone())
                    .unwrap_or_else(|| range_start.clone());
                let actual_end = days
                    .control
                    .last()
                    .or_else(|| days.test.last())
                    .map(|d| d.day.clone())
                    .unwrap_or_else(|| range_end.clone());
                // Coverage readiness: the pull is final when it returned data
                // covering the last closed day; anything less (earlier
                // coverage or no reported rows) is partial.
                let reported_days = days.control.len() + days.test.len();
                if reported_days > 0 && actual_end.as_str() >= range_end.as_str() {
                    readiness.final_accounts += 1;
                } else {
                    readiness.partial_accounts += 1;
                }
                if let Some(events) = self.events.as_ref() {
                    events
                        .report_pull_completed(
                            &job_id,
                            "experiment_days",
                            &experiment.experiment_id,
                            &actual_start,
                            &actual_end,
                            rows.as_array().map(Vec::len).unwrap_or(0) as u64,
                            "final",
                            pull_elapsed_ms,
                            now_unix_ms,
                        )
                        .await;
                }
                let pull = apex_lift::emit::data_pull(&day, &rows)
                    .map_err(|error| PlanError::Validation(format!("data pull: {error}")))?;
                let payload = pull
                    .record
                    .canonical_json()
                    .map_err(|error| PlanError::Validation(format!("data pull: {error}")))?;
                let pulled = ledger
                    .append(LedgerRecord::lift_record(
                        now_unix_ms,
                        &pull.sha256,
                        &payload,
                    ))
                    .map_err(PlanError::Ledger)?;
                ledger_index.observe(&pulled);
                report.pulled += 1;
                if let Some(events) = self.events.as_ref() {
                    events
                        .experiment_pull(
                            &experiment.experiment_id,
                            &pull.sha256,
                            &actual_start,
                            &actual_end,
                            now_unix_ms,
                        )
                        .await;
                }
                Some(days)
            } else {
                None
            };

            let status_ops = self.ops_for(experiment.platform).ok();
            let edit_check = if exp_measures && !experiment.pending_terminal {
                // A manual promotion is expected to change the live settings
                // to exactly the tested value; that is the promotion, not a
                // publisher edit. The adapter's promotion_status decides
                // whether it was promoted, declined or is still pending.
                if ledger_index
                    .promotion_deadline(&experiment.experiment_id, experiment.change_seq)
                    .is_some()
                {
                    crate::settle::EditCheck::VerifiedSame
                } else {
                    let check = self
                        .edit_check_with(status_ops.unwrap_or(self.ops.as_ref()), experiment)
                        .await;
                    // LL: a single failed settings read (provider timeout or
                    // 5xx) must not void a running test. The failure is the
                    // settlement state: hold the test pending and retry on
                    // the next daily pass, and void only after three
                    // consecutive daily passes could not read.
                    if matches!(check, crate::settle::EditCheck::Unknown) {
                        ledger
                            .append(LedgerRecord::settings_read_failed(
                                now_unix_ms,
                                experiment.platform,
                                &experiment.experiment_id,
                                &experiment.ad_unit_id,
                            ))
                            .map_err(PlanError::Ledger)?;
                        let streak = ledger_index
                            .settings_failure_streak(&experiment.experiment_id, now_unix_ms)
                            + 1;
                        if streak < SETTINGS_READ_RETRY_PASSES {
                            report.pending += 1;
                            continue;
                        }
                    }
                    check
                }
            } else {
                crate::settle::EditCheck::VerifiedSame
            };
            let settle_ops = match self.ops_for(experiment.platform) {
                Ok(ops) => ops,
                Err(reason) => {
                    report
                        .failures
                        .push((experiment.experiment_id.clone(), reason.to_string()));
                    continue;
                }
            };
            let outcome = match settle_experiment_indexed(
                settle_ops,
                ledger,
                &mut ledger_index,
                &experiment.experiment_id,
                experiment.change_seq,
                self.fill_format,
                &edit_check,
                days.as_ref(),
                now_unix_ms,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(PlanError::RollbackFailed { reason }) => {
                    if let Err(error) = self
                        .alerts
                        .emit(Alert::FailedRollback {
                            experiment_id: experiment.experiment_id.clone(),
                            detail: reason.clone(),
                        })
                        .await
                    {
                        eprintln!("apex-runner: alert delivery failed: {error}");
                    }
                    report
                        .failures
                        .push((experiment.experiment_id.clone(), reason));
                    continue;
                }
                Err(other) => {
                    report
                        .failures
                        .push((experiment.experiment_id.clone(), other.to_string()));
                    continue;
                }
            };
            if let SettleOutcome::PromotionPending {
                experiment_id,
                instructions,
                deadline_unix_ms,
            } = &outcome
            {
                report.pending += 1;
                if let Some(notify) = self.notify.as_ref() {
                    notify
                        .promotion_required(
                            &self.publisher_id,
                            experiment_id,
                            instructions,
                            *deadline_unix_ms,
                            now_unix_ms,
                        )
                        .await;
                }
                continue;
            }
            // A pending window is not a settlement; everything else is.
            if let SettleOutcome::Pending { .. } = &outcome {
                report.pending += 1;
                continue;
            }
            match &outcome {
                SettleOutcome::Kept { lift, lambda, .. } => {
                    self.emit_decision(
                        &experiment.experiment_id,
                        Decision::Keep,
                        &[],
                        Some((lift * 10_000.0).round() as i64),
                        Some((lambda * 10_000.0).round() as i64),
                        now_unix_ms,
                    )
                    .await;
                }
                SettleOutcome::RolledBack {
                    reason,
                    guardrails,
                    unmeasured,
                    ..
                } => {
                    self.emit_decision(
                        &experiment.experiment_id,
                        Decision::Rollback,
                        guardrails,
                        None,
                        None,
                        now_unix_ms,
                    )
                    .await;
                    // An unmeasured conclusion is the platform's own end,
                    // not a guardrail stop: the original lines already
                    // stand, so there is no restore to confirm and no
                    // guardrail alert to raise.
                    if !unmeasured {
                        self.confirm_restore(&experiment.experiment_id, guardrails, now_unix_ms)
                            .await;
                        if let Err(error) = self
                            .alerts
                            .emit(Alert::GuardrailStop {
                                experiment_id: experiment.experiment_id.clone(),
                                detail: reason.clone(),
                            })
                            .await
                        {
                            eprintln!("apex-runner: alert delivery failed: {error}");
                        }
                        if let Some(notify) = self.notify.as_ref() {
                            notify
                                .guardrail_stop(
                                    &self.publisher_id,
                                    &experiment.experiment_id,
                                    now_unix_ms,
                                )
                                .await;
                        }
                    }
                }
                SettleOutcome::Voided { reason, guardrails } => {
                    self.emit_decision(
                        &experiment.experiment_id,
                        Decision::Void,
                        guardrails,
                        None,
                        None,
                        now_unix_ms,
                    )
                    .await;
                    self.confirm_restore(&experiment.experiment_id, guardrails, now_unix_ms)
                        .await;
                    if let Err(error) = self
                        .alerts
                        .emit(Alert::GuardrailStop {
                            experiment_id: experiment.experiment_id.clone(),
                            detail: reason.clone(),
                        })
                        .await
                    {
                        eprintln!("apex-runner: alert delivery failed: {error}");
                    }
                    if let Some(notify) = self.notify.as_ref() {
                        notify
                            .guardrail_stop(
                                &self.publisher_id,
                                &experiment.experiment_id,
                                now_unix_ms,
                            )
                            .await;
                    }
                }
                SettleOutcome::Pending { .. } | SettleOutcome::PromotionPending { .. } => {}
            }
            if let Some(model) = priors.as_mut() {
                priors_dirty |= self.learn(model, experiment, &ledger_index, &outcome);
            }
            report.settled += 1;
            report
                .outcomes
                .push((experiment.experiment_id.clone(), status_label(&outcome)));
        }

        // One durability write per pass: all settled outcomes are applied to
        // the mutable model above, then persisted together. A pass with no new
        // learning leaves the file untouched.
        if priors_dirty {
            if let (Some(model), Some(path)) = (&priors, &self.priors_path) {
                persist_priors(path, model);
            }
        }

        // The readiness gauges carry exactly what the pass observed. When no
        // account was attempted, no class is known (and none is written):
        // a gauge is never fabricated from an absent observation.
        if readiness.attempted() > 0 {
            if let Some(telemetry) = self.telemetry.as_ref() {
                use apex_metrics::vocab::ReadinessClass;
                telemetry
                    .report_accounts(ReadinessClass::Final, f64::from(readiness.final_accounts));
                telemetry.report_accounts(
                    ReadinessClass::Partial,
                    f64::from(readiness.partial_accounts),
                );
                telemetry
                    .report_accounts(ReadinessClass::Stale, f64::from(readiness.stale_accounts));
                telemetry.report_accounts(
                    ReadinessClass::Disconnected,
                    f64::from(readiness.disconnected_accounts),
                );
            }
        }

        // The projection refresh runs last so the day's settle work is part
        // of the facts the projections are rebuilt from. A failure is
        // recorded and reported, never silently skipped.
        if let Some(projections) = &self.projections {
            let batch_id = crate::projections_job::daily_batch_id(now_unix_ms);
            match projections
                .refresh(&batch_id, self.telemetry.as_ref(), now_unix_ms)
                .await
            {
                Ok(refresh) => report.projections = Some(refresh),
                Err(error) => {
                    eprintln!("apex-runner: projection refresh failed: {error}");
                    report.projection_failure = Some(error);
                }
            }
        }

        Ok(report)
    }

    /// Pull reporting for every running experiment with at most
    /// [`Self::reporting_concurrency`] reads in flight, returning the results
    /// in the original experiment order. A failed pull is a value in the
    /// result vector, never a cancelled sibling.
    async fn pull_reporting_bounded(
        &self,
        experiments: &[RunningExperiment],
    ) -> Vec<
        Option<(
            Result<crate::ops::ExperimentDays, apex_core::PlatformError>,
            u64,
        )>,
    > {
        use futures_util::stream::{self, StreamExt};

        let concurrency = self.reporting_concurrency.max(1);
        let fetched: Vec<(
            usize,
            Result<crate::ops::ExperimentDays, apex_core::PlatformError>,
            u64,
        )> = stream::iter(
            experiments
                .iter()
                .enumerate()
                .filter_map(|(index, experiment)| {
                    // A pending terminal proof is reconciled from the ledger,
                    // not re-pulled; an unmeasured platform has no arms.
                    if experiment.pending_terminal {
                        return None;
                    }
                    let ops: Arc<dyn PlatformOps> =
                        if let Some(ops) = self.platform_ops.get(&experiment.platform) {
                            Arc::clone(ops)
                        } else if self.ops.platform() == experiment.platform {
                            Arc::clone(&self.ops)
                        } else {
                            return None;
                        };
                    if !ops.measures_variants() {
                        return None;
                    }
                    let experiment_id = experiment.experiment_id.clone();
                    let instance_key = experiment.instance_key.clone();
                    Some(async move {
                        let started = std::time::Instant::now();
                        let days = ops.experiment_days_for(&experiment_id, &instance_key).await;
                        (index, days, started.elapsed().as_millis() as u64)
                    })
                }),
        )
        .buffer_unordered(concurrency)
        .collect()
        .await;
        let mut indexed = Vec::with_capacity(experiments.len());
        indexed.resize_with(experiments.len(), || None);
        for (index, days, elapsed_ms) in fetched {
            if let Some(slot) = indexed.get_mut(index) {
                *slot = Some((days, elapsed_ms));
            }
        }
        indexed
    }

    /// Record one terminal decision: the bounded metric, the
    /// `experiment_decided` event, and one `guardrail_fired` event per fired
    /// guardrail (with its bounded action).
    async fn emit_decision(
        &self,
        experiment_id: &str,
        decision: Decision,
        guardrails: &[apex_lift::guardrails::GuardrailEvent],
        point_estimate_bp: Option<i64>,
        decision_bound_bp: Option<i64>,
        now_unix_ms: i64,
    ) {
        let reason = crate::telemetry::decision_reason_class(decision, guardrails);
        if let Some(telemetry) = self.telemetry.as_ref() {
            telemetry.decision(decision, reason);
        }
        if let Some(events) = self.events.as_ref() {
            events
                .experiment_decided(
                    experiment_id,
                    decision.as_str(),
                    point_estimate_bp,
                    decision_bound_bp,
                    reason.as_str(),
                    now_unix_ms,
                )
                .await;
            for guardrail in guardrails {
                let bounded = crate::telemetry::guardrail_of(guardrail);
                events
                    .guardrail_fired(
                        bounded.as_str(),
                        guardrail_input(guardrail),
                        "registered threshold (see pre-registration)",
                        now_unix_ms,
                    )
                    .await;
            }
        }
        for guardrail in guardrails {
            if let Some(telemetry) = self.telemetry.as_ref() {
                telemetry.guardrail(
                    crate::telemetry::guardrail_of(guardrail),
                    crate::telemetry::guardrail_action(guardrail),
                );
            }
        }
    }

    /// The restore contract after a stop request: the request is recorded,
    /// then the platform state is read back. An unconfirmed restore is a
    /// failed restore with a stable incident reference, never an assumed
    /// success.
    async fn confirm_restore(
        &self,
        experiment_id: &str,
        guardrails: &[apex_lift::guardrails::GuardrailEvent],
        now_unix_ms: i64,
    ) {
        let operation_id = format!("op_{experiment_id}");
        let guard_type = guardrails
            .first()
            .map(crate::telemetry::guardrail_of)
            .map(|guardrail| guardrail.as_str())
            .unwrap_or("stop_requested");
        if let Some(telemetry) = self.telemetry.as_ref() {
            telemetry.restore(RestoreOperation::Restore, RestoreOutcome::Requested);
        }
        if let Some(events) = self.events.as_ref() {
            events
                .restore_requested(guard_type, &operation_id, "stopped", now_unix_ms)
                .await;
        }
        let read = self.ops.experiment(experiment_id).await;
        let (confirmed, error_class) = match &read {
            Ok(result) if result.state == apex_core::ExperimentState::Stopped => (true, None),
            Ok(_) => (false, Some("not_stopped")),
            Err(_) => (false, Some("read_failed")),
        };
        if confirmed {
            if let Some(telemetry) = self.telemetry.as_ref() {
                telemetry.restore(RestoreOperation::Restore, RestoreOutcome::Confirmed);
            }
            if let Some(events) = self.events.as_ref() {
                events
                    .restore_confirmed(
                        guard_type,
                        &operation_id,
                        "stopped",
                        "platform read reports stopped",
                        now_unix_ms,
                    )
                    .await;
            }
            return;
        }
        let error_class = error_class.unwrap_or("unconfirmed");
        if let Some(telemetry) = self.telemetry.as_ref() {
            telemetry.restore(RestoreOperation::Restore, RestoreOutcome::Failed);
        }
        if let Some(events) = self.events.as_ref() {
            events
                .restore_failed(
                    guard_type,
                    &operation_id,
                    "stopped",
                    error_class,
                    now_unix_ms,
                )
                .await;
        }
        if let Some(notify) = self.notify.as_ref() {
            notify
                .restore_failed(
                    &self.publisher_id,
                    experiment_id,
                    &crate::notify::incident_id_for(experiment_id),
                    now_unix_ms,
                )
                .await;
        }
    }

    /// Feed a settled test back into the bucket model, so the next cycle's
    /// proposals learn from results (keeps and rollbacks alike, or the model
    /// conditions on the decision). Returns whether the model changed; the
    /// caller persists the batched model once at the end of the pass.
    fn learn(
        &self,
        priors: &mut apex_lift::priors::LiftPriors,
        experiment: &RunningExperiment,
        index: &LedgerIndex,
        outcome: &SettleOutcome,
    ) -> bool {
        let label = status_label(outcome);
        if !matches!(label.as_str(), "kept" | "rolled_back") {
            return false;
        }
        // A verdict-only conclusion carries no daily ratios to learn from.
        if matches!(
            outcome,
            SettleOutcome::Kept {
                unmeasured: true,
                ..
            } | SettleOutcome::RolledBack {
                unmeasured: true,
                ..
            }
        ) {
            return false;
        }
        let Some(bucket) = change_bucket(index, experiment.change_seq) else {
            return false;
        };
        let ratios = match outcome {
            SettleOutcome::Kept {
                daily_log_ratios, ..
            }
            | SettleOutcome::RolledBack {
                daily_log_ratios, ..
            } => daily_log_ratios.as_slice(),
            _ => return false,
        };
        apex_lift::emit::apply_decision_to_priors(priors, &bucket, &label, ratios);
        true
    }

    /// The policy this runner applies (exposed so callers can pin it in logs).
    pub fn policy(&self) -> &AutopilotPolicy {
        &self.policy
    }

    /// The publisher this runner bills.
    pub fn publisher_id(&self) -> &str {
        &self.publisher_id
    }

    /// The bucket model this runner learns into, when configured.
    pub fn runner_priors(&self) -> Option<&apex_lift::priors::LiftPriors> {
        self.priors.as_ref()
    }

    /// The platform adapter behind the write loop.
    pub fn platform_ops(&self) -> &dyn PlatformOps {
        self.ops.as_ref()
    }

    /// Emit the right alert for a failed daily pull: an expired token when the
    /// platform refused the call on auth, otherwise a reporting failure.
    async fn alert_pull_failure(&self, experiment_id: &str, source: &apex_core::PlatformError) {
        let alert = match source {
            apex_core::PlatformError::Auth { platform, .. } => Alert::ExpiredToken {
                platform: platform.as_str().to_string(),
                detail: format!("daily pull for {experiment_id} was refused on auth"),
            },
            other => Alert::PullFailure {
                experiment_id: experiment_id.to_string(),
                detail: other.to_string(),
            },
        };
        if let Err(error) = self.alerts.emit(alert).await {
            eprintln!("apex-runner: alert delivery failed: {error}");
        }
    }

    /// The tri-state publisher-edit check for one running experiment: the
    /// live settings are compared against the pre-registered before-value and
    /// the outcome says whether the comparison actually happened. A provider
    /// read failure is [`EditCheck::Unknown`] — a failure to look is never
    /// "no edit" — and the settlement voids on it (after the LL retry, see
    /// `SETTINGS_READ_RETRY_PASSES`).
    async fn edit_check_with(
        &self,
        ops: &dyn PlatformOps,
        experiment: &RunningExperiment,
    ) -> crate::settle::EditCheck {
        use crate::settle::EditCheck;
        let Some(change) = experiment.change.clone() else {
            // The tested change is not in the ledger: the settle path refuses
            // on it anyway, and there is nothing to scope a comparison to.
            return EditCheck::VerifiedSame;
        };
        // N2: a MAX frequency-cap (ad-load) change registers the conservative
        // baseline as its before-value, and the unit read carries floors, not
        // caps. Comparing the whole documents always mismatched, so every
        // ad-load test voided. The specific setting this change modifies has
        // no read on the unit, so the check is not armed: nothing checkable,
        // never "edited".
        if matches!(change, ChangeKind::FrequencyCapSet { .. }) {
            return EditCheck::VerifiedSame;
        }
        if experiment.expected_settings.trim().is_empty() {
            // No before-value was pinned, so there is no claim to verify; the
            // check is not armed and no read failure can be attributed to it.
            return EditCheck::VerifiedSame;
        }
        if !ops.supports_settings_read() {
            // The adapter documents no settings read: a capability question,
            // not a read failure. The check stays unarmed.
            return EditCheck::VerifiedSame;
        }
        match ops.current_settings(&experiment.ad_unit_id).await {
            Ok(current) => {
                match changed_setting_agrees(&change, &current, &experiment.expected_settings) {
                    // Compare the setting the change modifies, not the whole
                    // scope document (N2): a floor test's before-value is the
                    // floor JSON, while the read may carry the full unit config.
                    Some(true) => EditCheck::VerifiedSame,
                    Some(false) => EditCheck::VerifiedChanged(vec![experiment.ad_unit_id.clone()]),
                    // The read cannot verify this change's setting: a failure to
                    // look, never a verified edit.
                    None => EditCheck::Unknown,
                }
            }
            Err(_) => EditCheck::Unknown,
        }
    }
}

/// How the edit check's scoped comparison came out: the changed setting
/// agrees, it disagrees, or the read cannot verify it.
enum SettingVerdict {
    Same,
    Changed,
    Unverifiable,
}

/// Whether the setting `change` modifies is the same in the live `current`
/// document and the pre-registered `expected` value. Only the changed
/// setting is compared (N2); a full-document comparison always mismatched
/// because the registered before-value is the unit's floor JSON while the
/// read carries the full ad-unit config.
fn changed_setting_agrees(change: &ChangeKind, current: &str, expected: &str) -> Option<bool> {
    let verdict = match change {
        // A MAX ad-load (frequency-cap) change pins a conservative baseline
        // of the caps, and the unit read carries floors, never caps: the
        // specific setting this change modifies has no read to compare
        // against, so the verdict is unverifiable (the check is unarmed).
        ChangeKind::FrequencyCapSet { .. } => SettingVerdict::Unverifiable,
        // A MAX floor move pins the floors: compare the floor ECPM values
        // per country, insensitive to key order, extra config fields and
        // the floor list's own field names.
        ChangeKind::CountryFloorSet { .. } => {
            match (floors_of(current), floors_of(expected)) {
                (Some(live), Some(pinned)) if live == pinned => SettingVerdict::Same,
                (Some(_), Some(_)) => SettingVerdict::Changed,
                // A read without a comparable floor list cannot verify.
                _ => SettingVerdict::Unverifiable,
            }
        }
        // A LevelPlay group-capping change rides the variant group, which is
        // not a waterfall entry. The checkable claim is that the pinned
        // capping group is still the one serving the scope; a lookup that
        // cannot place it is a failure to look, never a verified edit.
        ChangeKind::LevelplayGroupCapping { group_id, .. } => {
            match json_string_field(current, &["group", "groupId"]) {
                Some(live) if live == *group_id => SettingVerdict::Same,
                Some(_) => SettingVerdict::Changed,
                None => SettingVerdict::Unverifiable,
            }
        }
        // The mediation-group and waterfall reads pin the compared document
        // itself; the whole-document comparison is the scoped one.
        _ => {
            if same_settings(current, expected) {
                SettingVerdict::Same
            } else {
                SettingVerdict::Changed
            }
        }
    };
    match verdict {
        SettingVerdict::Same => Some(true),
        SettingVerdict::Changed => Some(false),
        SettingVerdict::Unverifiable => None,
    }
}

/// The `country -> floorEcpm` map of a settings document: the floor list is
/// either the document itself (the registered before-value shape) or a
/// `countryFloors`/`floors` field of a full unit config.
fn floors_of(settings: &str) -> Option<BTreeMap<String, i64>> {
    let parsed: serde_json::Value = serde_json::from_str(settings).ok()?;
    let floors = match &parsed {
        serde_json::Value::Array(_) => Some(parsed),
        serde_json::Value::Object(map) => map
            .get("countryFloors")
            .or_else(|| map.get("country_floors"))
            .or_else(|| map.get("floors"))
            .or_else(|| map.get("bidFloors"))
            .or_else(|| map.get("bid_floors"))
            .cloned(),
        _ => None,
    }?;
    let list = floors.as_array()?;
    let mut map = BTreeMap::new();
    for floor in list {
        let country = floor.get("country").and_then(serde_json::Value::as_str)?;
        let ecpm = floor.get("floorEcpm").and_then(serde_json::Value::as_i64)?;
        map.insert(country.to_string(), ecpm);
    }
    Some(map)
}

/// The first present string field of a JSON document, tried in order.
fn json_string_field(settings: &str, fields: &[&str]) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(settings).ok()?;
    fields
        .iter()
        .find_map(|field| parsed.get(*field))
        .and_then(|value| {
            value.as_str().map(str::to_string).or_else(|| {
                value
                    .as_i64()
                    .map(|number| number.to_string())
                    .or_else(|| value.as_u64().map(|number| number.to_string()))
            })
        })
}

/// Whether this ledger owns an operator job target: an experiment id, an
/// experiment name, or a change id recorded on the signed chain. The fleet
/// daily pass claims only its own jobs; a target no publisher owns stays
/// queued (visible as skipped) instead of being consumed by the wrong
/// adapter. The set is built in one ledger pass.
fn owned_targets(ledger: &dyn LedgerLog) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for record in ledger.records() {
        match record {
            LedgerRecord::ExperimentStarted {
                experiment_id,
                spec,
                ..
            } => {
                out.insert(experiment_id.clone());
                out.insert(spec.name.clone());
                if !spec.instance_id.trim().is_empty() {
                    out.insert(spec.instance_id.clone());
                }
            }
            LedgerRecord::LiftRecord { payload, .. } => match parse_lift_record(payload).ok() {
                Some(apex_lift::records::LiftRecord::PreRegistration {
                    experiment_id,
                    instance_id,
                    ..
                })
                | Some(apex_lift::records::LiftRecord::Result {
                    experiment_id,
                    instance_id,
                    ..
                }) => {
                    out.insert(experiment_id);
                    if !instance_id.trim().is_empty() {
                        out.insert(instance_id);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

/// One scope's kept changes for the monthly pass: legacy lift-fee changes
/// (no holdout state) and A3 holdout changes (state-backed).
#[derive(Default)]
struct ScopeBilling {
    platform: Option<apex_core::PlatformId>,
    legacy: Vec<apex_lift::billing::KeptChange>,
    holdout: Vec<ScopeHoldout>,
}

/// One holdout-backed change waiting for its trailing 28-day reading.
struct ScopeHoldout {
    change: apex_lift::billing::KeptChange,
    holdout: LedgerHoldout,
}

/// What one monthly billing pass did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonthlyReport {
    pub month: String,
    /// Scopes (ad units) that had at least one kept change in force.
    pub billed_scopes: u32,
    /// One entry per billed change: its id and the fee in micros.
    pub fees: Vec<(String, i64)>,
    /// The invoice's gross total (net + vat), in micros.
    pub invoice_total_micros: i64,
    /// The invoice's display reference (`INV-<month>-<publisher>`), derived
    /// deterministically because the ledger carries no invoice registry.
    pub invoice_reference: String,
    /// The register number assigned by the invoice register, when the VAT
    /// invoice was issued. A refused invoice is never numbered.
    pub invoice_number: Option<u64>,
    /// The rendered VAT invoice document, when issuance succeeded.
    pub rendered_invoice: Option<String>,
    /// The typed reason the publisher's invoice was refused (a missing
    /// customer profile, a refused treatment, an unusable register). No
    /// half-empty document is ever printed for a refused invoice.
    pub invoice_refusal: Option<String>,
    /// The Mollie payment links created for this invoice (the first payment
    /// and every typed fallback), when collection is wired.
    pub payment_links: Vec<String>,
    /// The PSP references of the stored-card charges that collected without
    /// a link, when collection is wired. The pairing of the two channel
    /// lists is the collection evidence: every billed line landed on exactly
    /// one of them.
    pub card_charges: Vec<String>,
    /// Whether collection ran (an Mollie client was attached).
    pub collected: bool,
    /// Kept changes whose platform documents no per-variant measurement and
    /// which are therefore never billed, with the typed reason. Recorded on
    /// the ledger as a `feeIneligible` billing change and surfaced here so the
    /// operator sees why a kept experiment produced no fee line.
    pub fee_ineligible: Vec<apex_lift::billing::FeeIneligible>,
    /// Holdout instances rolled back this pass for a confident harm, with the
    /// measured lift that decided it.
    pub holdout_rollbacks: Vec<(String, f64)>,
    /// Holdout months that billed nothing because the measured lift was zero
    /// or below, with the reason the invoice line states.
    pub holdout_no_fee: Vec<(String, String)>,
}

impl Runner {
    /// How many days after a sweep the scheduled retry (and its reminder)
    /// lands.
    pub const DUNNING_RETRY_IN_DAYS: i64 = 7;

    /// The daily dunning sweep (GG): this publisher's unpaid intents past
    /// their due date get (1) a retry-schedule row per failed line — written
    /// with the writer the schedule never had — and (2) the recorded invoice
    /// payment link re-sent. Without a payment-intent journal there is
    /// nothing to sweep; a missing journal sweeps nothing (missing is empty,
    /// exactly like every other payment file reader). A read or write
    /// failure fails the pass closed instead of silently dropping the
    /// reminder.
    pub async fn dunning_sweep(
        &self,
        now_unix_ms: i64,
    ) -> Result<Option<DunningReport>, PlanError> {
        let journal = self.payment_intents_journal()?;
        let Some(journal) = journal else {
            return Ok(None);
        };
        // The monthly run appends intents in another process: the stamp
        // check in `reload` picks those appends up before the sweep reads.
        let intents =
            apex_billing::PaymentIntentStore::open(&journal).map_err(PlanError::Billing)?;
        intents.reload().map_err(PlanError::Billing)?;
        let candidates =
            apex_billing::intent::dunning_candidates(&intents, &self.publisher_id, now_unix_ms)
                .map_err(PlanError::Billing)?;
        let mut report = DunningReport::default();
        if candidates.is_empty() {
            return Ok(Some(report));
        }
        // Already-scheduled rows: a (reference, retry day) already scheduled
        // is never appended twice, so the daily sweep stays idempotent.
        let scheduled = match &self.payment_retries_path {
            Some(path) => crate::payment_events::read_scheduled_retries(path)
                .map_err(|error| PlanError::Validation(error.to_string()))?,
            None => Vec::new(),
        };
        let retry_day = crate::calendar::format_day(
            now_unix_ms.div_euclid(86_400_000) + Self::DUNNING_RETRY_IN_DAYS,
        );
        let mut rows: Vec<crate::payment_events::ScheduledRetry> = Vec::new();
        for candidate in &candidates {
            report.unpaid_invoices += 1;
            for line in &candidate.unpaid {
                let attempt = scheduled
                    .iter()
                    .filter(|row| row.reference == line.line_reference)
                    .map(|row| row.attempt)
                    .max()
                    .unwrap_or(0)
                    + 1;
                let already = scheduled.iter().any(|row| {
                    row.reference == line.line_reference
                        && row.state == "scheduled"
                        && row.retry_on.as_deref() == Some(retry_day.as_str())
                }) || rows.iter().any(|row| {
                    row.reference == line.line_reference
                        && row.retry_on.as_deref() == Some(retry_day.as_str())
                });
                if already {
                    continue;
                }
                rows.push(crate::payment_events::ScheduledRetry {
                    invoice_reference: candidate.invoice_reference.clone(),
                    amount_micros: line
                        .amount_minor_units
                        .saturating_mul(apex_billing::MICROS_PER_MINOR_UNIT),
                    currency: line.currency.clone(),
                    retry_on: Some(retry_day.clone()),
                    reference: line.line_reference.clone(),
                    state: "scheduled".to_string(),
                    at_unix_ms: now_unix_ms,
                    attempt,
                });
            }
        }
        if !rows.is_empty() {
            let Some(retries_path) = &self.payment_retries_path else {
                return Err(PlanError::Validation(
                    "unpaid invoices are due for dunning but APEX_PAYMENT_RETRIES is not configured; the retry rows cannot be written".to_string(),
                ));
            };
            report.retry_rows = crate::payment_events::write_scheduled_retries(retries_path, &rows)
                .map_err(|error| PlanError::Validation(error.to_string()))?
                as u32;
        }
        // Re-send the payment link: the reminder's dedup identity carries the
        // retry attempt, so every scheduled retry reminds exactly once.
        for candidate in &candidates {
            let Some(link) = &candidate.payment_link else {
                continue;
            };
            let Some(notify) = self.notify.as_ref() else {
                continue;
            };
            let attempt = rows
                .iter()
                .filter(|row| row.invoice_reference == candidate.invoice_reference)
                .map(|row| row.attempt)
                .chain(
                    scheduled
                        .iter()
                        .filter(|row| {
                            candidate
                                .unpaid
                                .iter()
                                .any(|line| line.line_reference == row.reference)
                        })
                        .map(|row| row.attempt),
                )
                .max()
                .unwrap_or(1);
            notify
                .invoice_ready(
                    &candidate.publisher_id,
                    &candidate.invoice_reference,
                    candidate.unpaid_micros(),
                    candidate.unpaid.first().map(|line| line.currency.as_str()),
                    &link.url,
                    attempt,
                    now_unix_ms,
                )
                .await;
            report.reminders += 1;
        }
        Ok(Some(report))
    }

    /// The payment-intent journal path, when one exists on disk. The default
    /// sits beside the invoice register, exactly like the monthly pass.
    fn payment_intents_journal(&self) -> Result<Option<std::path::PathBuf>, PlanError> {
        let path = match &self.payment_intents_path {
            Some(path) => path.clone(),
            None => self
                .register_path
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from(apex_billing::DEFAULT_REGISTER_PATH))
                .parent()
                .map(|parent| parent.join("payment-intents.jsonl"))
                .unwrap_or_else(|| std::path::PathBuf::from("payment-intents.jsonl")),
        };
        match std::fs::metadata(&path) {
            Ok(_) => Ok(Some(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(PlanError::Validation(format!(
                "{} could not be inspected: {error}",
                path.display()
            ))),
        }
    }

    /// One monthly billing pass: bill every kept change still in force for
    /// `month`, from the platform-reported revenue of its pre-registered
    /// scope. A change past its term or covered by a publisher override is
    /// skipped by [`apex_lift::run::run_month`].
    pub async fn run_monthly(
        &self,
        ledger: &mut dyn LedgerLog,
        publisher_id: &str,
        month: &str,
        month_start: &str,
        month_days: u32,
        now_unix_ms: i64,
    ) -> Result<MonthlyReport, PlanError> {
        if publisher_id.trim().is_empty() {
            return Err(PlanError::Validation(
                "the monthly pass needs a publisher id to bill".into(),
            ));
        }
        // The seller profile is the compliance gate of the whole pass: the
        // deployment file named by `APEX_BILLING_PROFILE` (or the injected
        // profile), loaded before any ledger record so an incomplete
        // deployment refuses the pass instead of printing a half-empty
        // invoice. The runner's explicit rate override lands on the profile.
        let mut profile = match &self.billing_profile {
            Some(profile) => profile.clone(),
            None => {
                apex_billing::profile::BillingProfile::from_env().map_err(PlanError::Billing)?
            }
        };
        if self.vat_rate_bp != 0 {
            profile.vat_rate_bp = self.vat_rate_bp;
        }
        let month_end = month_end(month_start, month_days)?;
        // The fee accrues over the closed month: date every billing record at
        // the month's last day so the invoice's period check holds.
        let accrual_at = crate::calendar::day_number(&month_end)
            .ok_or_else(|| PlanError::Validation("bad month end".into()))?
            * 86_400_000;
        let period = apex_billing::BillingPeriod {
            year_month: month.to_string(),
            start_unix_ms: crate::calendar::day_number(month_start)
                .ok_or_else(|| PlanError::Validation("bad month start".into()))?
                * 86_400_000,
            end_unix_ms: (crate::calendar::day_number(&month_end)
                .ok_or_else(|| PlanError::Validation("bad month end".into()))?
                + 1)
                * 86_400_000,
        };
        let mut by_scope: std::collections::BTreeMap<String, ScopeBilling> =
            std::collections::BTreeMap::new();
        let holdouts = ledger_holdouts(ledger)?;
        let existing_billing = ledger_monthly_billing(ledger, month)?;
        let mut report = MonthlyReport {
            month: month.to_string(),
            ..MonthlyReport::default()
        };
        // Monthly idempotency: the register is the authority for numbering.
        // A retried pass for the same publisher and month does not issue a
        // second invoice, but it *does* continue the existing invoice through
        // collection: a crash after issuance and before submission must be
        // recoverable on the next pass.
        let register_path = self
            .register_path
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from(apex_billing::DEFAULT_REGISTER_PATH));
        // Only read a register that exists (or a configured path): an
        // unconfigured deployment with no register must not create one just
        // to discover it has nothing to replay.
        let register_exists = self.register_path.is_some() || register_path.exists();
        let existing_invoice = if register_exists {
            apex_billing::InvoiceRegister::open(&register_path)
                .map_err(PlanError::Billing)?
                .stored_invoice(publisher_id, month)
                .map_err(PlanError::Billing)?
        } else {
            None
        };
        let invoice_reference = invoice_reference(publisher_id, month);
        report.invoice_reference = invoice_reference.clone();
        let recovered_existing = existing_invoice.is_some();
        let (invoice, issued) = if let Some(existing) = existing_invoice {
            report.invoice_number = Some(existing.number);
            report.invoice_total_micros = existing.gross_micros;
            report.rendered_invoice = Some(
                existing
                    .render_vat_invoice(&profile)
                    .map_err(PlanError::Billing)?,
            );
            (existing, true)
        } else {
            for (scope, platform, change) in kept_changes(ledger)? {
                // The fee-eligibility gate of the lift engine: a kept change on a
                // platform whose API documents no per-variant measurement can
                // never produce a measured lift, so it is excluded before any fee
                // computation sees it. The typed decision is recorded on the
                // ledger (a billing change, the same record the term-end and
                // override stops use) and in the report; nothing is billed.
                if let Err(ineligible) = apex_lift::billing::fee_eligibility(platform) {
                    // Eligibility is platform-invariant: the typed decision
                    // is recorded on the append-only ledger once per change,
                    // while every pass still reports it to the operator.
                    let change_key = format!("\"changeId\":\"{}\"", change.change_id);
                    let reason_key = format!("\"reason\":\"{}\"", ineligible.reason);
                    let already_recorded = ledger.records().iter().any(|record| {
                        matches!(
                            record,
                            LedgerRecord::LiftRecord { payload, .. }
                                if payload.contains(&change_key) && payload.contains(&reason_key)
                        )
                    });
                    if !already_recorded {
                        let record = apex_lift::emit::billing_change(
                            &change.change_id,
                            &month_end,
                            &ineligible.reason,
                            0.0,
                        )
                        .map_err(|error| {
                            PlanError::Validation(format!("billing change: {error}"))
                        })?;
                        let payload = record.record.canonical_json().map_err(|error| {
                            PlanError::Validation(format!("billing change: {error}"))
                        })?;
                        ledger
                            .append(LedgerRecord::lift_record(
                                accrual_at,
                                &record.sha256,
                                &payload,
                            ))
                            .map_err(PlanError::Ledger)?;
                    }
                    report.fee_ineligible.push(ineligible);
                    continue;
                }
                let entry = by_scope.entry(scope).or_default();
                entry.platform.get_or_insert(platform);
                match holdouts.get(&change.change_id) {
                    Some(holdout) => entry.holdout.push(ScopeHoldout {
                        change,
                        holdout: LedgerHoldout {
                            state: holdout.state.clone(),
                            hash: holdout.hash.clone(),
                        },
                    }),
                    None => entry.legacy.push(change),
                }
            }
            let mut basis: Vec<apex_billing::BillingBasisRecord> = Vec::new();
            let mut fees_by_change: std::collections::BTreeMap<
                String,
                apex_lift::billing::FeeLine,
            > = std::collections::BTreeMap::new();
            // Simultaneous certificates: each statement gets a share of the
            // family error before any cohort is certified.
            let holdout_count: usize = by_scope.values().map(|billing| billing.holdout.len()).sum();
            let certificate_alpha = certificate_family_alpha(holdout_count);
            for (scope, scope_billing) in by_scope {
                let mut scope_change_ids: std::collections::BTreeSet<String> = scope_billing
                    .legacy
                    .iter()
                    .map(|change| change.change_id.clone())
                    .collect();
                scope_change_ids.extend(
                    scope_billing
                        .holdout
                        .iter()
                        .map(|entry| entry.change.change_id.clone()),
                );
                let scope_ops: &dyn PlatformOps = match scope_billing.platform {
                    Some(platform) => self.ops_for(platform)?,
                    None => self.ops.as_ref(),
                };
                let revenue = scope_ops
                    .scope_revenue(&scope, month_start, &month_end)
                    .await
                    .map_err(|source| PlanError::ApplyFailed { source })?;
                let mut run_fees: Vec<apex_lift::billing::FeeLine> = Vec::new();
                let mut run_skipped: Vec<(String, apex_lift::billing::BillingStop)> = Vec::new();
                if !scope_billing.holdout.is_empty() {
                    let platform = scope_billing.platform.unwrap_or(PlatformId::AppLovinMax);
                    let ops = self.ops_for(platform)?;
                    let mut holdout_changes = Vec::new();
                    for ScopeHoldout {
                        mut change,
                        holdout,
                    } in scope_billing.holdout
                    {
                        // A prior pass already signed this instance's month:
                        // reuse the signed line instead of measuring (and
                        // appending) a second fee.
                        if let Some((_, signed)) = existing_billing.get(&change.change_id) {
                            run_fees.push(fee_line_from_billing(signed));
                            continue;
                        }
                        if let Some(day) = &holdout.state.rollback_day {
                            change.rolled_back_day = Some(day.clone());
                        }
                        if !holdout.state.active() {
                            if let Some(day) = &holdout.state.rollback_day {
                                change.rolled_back_day = Some(day.clone());
                                run_skipped.push((
                                    change.change_id.clone(),
                                    apex_lift::billing::BillingStop::RolledBack,
                                ));
                            } else {
                                report.holdout_no_fee.push((
                                    change.change_id.clone(),
                                    "no fee: the holdout provider confirmation is still pending"
                                        .into(),
                                ));
                            }
                            continue;
                        }
                        let elapsed = crate::calendar::day_number(&month_end)
                            .zip(crate::calendar::day_number(&holdout.state.kept_day))
                            .map(|(end, kept)| u32::try_from((end - kept + 1).max(0)).unwrap_or(0))
                            .unwrap_or(0);
                        let evidence = ops
                            .experiment_players(
                                &holdout.state.holdout_experiment_id,
                                &change.change_id,
                            )
                            .await;
                        let (
                            estimate,
                            signed_evidence,
                            signed_roster,
                            exposed_revenue_basis,
                            actual_gain_lower,
                            signed_delta_lower,
                            signed_daily,
                        ) = match evidence {
                            Ok(evidence) => {
                                // A3-V3 E2: the measurement scope is informational.
                                // The fee runs on signed daily exposed revenue from the
                                // assigned cohort — already scoped by assignment.
                                {
                                    let measured_scope =
                                        evidence.measurement_scope.unwrap_or_default();
                                    let rows = trailing_measurement(evidence.rows, &month_end);
                                    let mut window_days: Vec<String> =
                                        rows.iter().map(|row| row.day.clone()).collect();
                                    window_days.sort();
                                    window_days.dedup();
                                    // A window day with no player rows is missing
                                    // evidence unless the platform reports zero
                                    // scope revenue for that exact day.
                                    let mut unproven_days: Vec<String> = Vec::new();
                                    if window_days.len() < apex_lift::holdout::MEASUREMENT_DAYS {
                                        let window_start = crate::calendar::day_number(&month_end)
                                            .map(|end| end - 27);
                                        for offset in 0..apex_lift::holdout::MEASUREMENT_DAYS {
                                            let day = window_start.map(|start| {
                                                crate::calendar::format_day(start + offset as i64)
                                            });
                                            let Some(day) = day else {
                                                unproven_days.push(format!(
                                                    "{month_end} does not parse as a window end"
                                                ));
                                                break;
                                            };
                                            if window_days.iter().any(|known| known == &day) {
                                                continue;
                                            }
                                            match ops
                                                .scope_revenue(
                                                    &holdout.state.scope,
                                                    &day,
                                                    &day,
                                                )
                                                .await
                                            {
                                                Ok(0) => {}
                                                Ok(_) => unproven_days.push(format!(
                                                    "{day} reports scope revenue but carries no player rows"
                                                )),
                                                Err(error) => unproven_days.push(format!(
                                                    "{day} could not be confirmed quiet: {error}"
                                                )),
                                            }
                                        }
                                    }
                                    let window_complete = unproven_days.is_empty();
                                    let roster = evidence.roster;
                                    let folded =
                                        apex_lift::holdout::fold_evidence_rows(&rows, &month_end);
                                    match folded {
                                        Err(error) => {
                                            report.holdout_no_fee.push((
                                                change.change_id.clone(),
                                                format!(
                                                    "No fee this month: the holdout feed is unusable: {error}"
                                                ),
                                            ));
                                            (
                                                None,
                                                None,
                                                None,
                                                None,
                                                None,
                                                None,
                                                std::collections::BTreeMap::new(),
                                            )
                                        }
                                        Ok(folded) => {
                                            match apex_lift::holdout::measure_from_feed(
                                                &rows,
                                                holdout.state.exposed_is_test,
                                                Some(&evidence.pretest),
                                                roster,
                                            ) {
                                                Ok(measurement) => {
                                                    // Daily exposed revenue: the fee runs on
                                                    // exactly the billed days, never on
                                                    // scope revenue times a user share.
                                                    let mut daily_exposed: std::collections::BTreeMap<String, i64> =
                                                    std::collections::BTreeMap::new();
                                                    let mut exposed_by_user: std::collections::BTreeMap<String, (bool, i64)> =
                                                    std::collections::BTreeMap::new();
                                                    for row in &rows {
                                                        let is_exposed = row.test
                                                            == holdout.state.exposed_is_test;
                                                        if is_exposed {
                                                            *daily_exposed
                                                                .entry(row.day.clone())
                                                                .or_default() += row.revenue_micros;
                                                        }
                                                        if let Some(user) =
                                                            apex_lift::player_store::player_key(row)
                                                        {
                                                            let slot = exposed_by_user
                                                                .entry(user)
                                                                .or_insert((is_exposed, 0));
                                                            slot.1 = slot
                                                                .1
                                                                .saturating_add(row.revenue_micros);
                                                        }
                                                    }
                                                    let certified_from_adapter = evidence
                                                        .certified_gain_lower
                                                        .zip(evidence.certified_delta_lower);
                                                    // The adapter bound is a fallback only when
                                                    // no per-user envelopes were supplied. An
                                                    // eligible set that fails its own checks must
                                                    // never be papered over by it.
                                                    let certificate_bounds =
                                                        match evidence.eligible.as_deref() {
                                                            Some(eligible_users) => {
                                                                certified_actual_gain(
                                                                    eligible_users,
                                                                    &exposed_by_user,
                                                                    measurement.roster,
                                                                    certificate_alpha,
                                                                )
                                                            }
                                                            None => certified_from_adapter,
                                                        };
                                                    let actual_gain_lower =
                                                        certificate_bounds.map(|bounds| bounds.0);
                                                    let delta_lower =
                                                        certificate_bounds.map(|bounds| bounds.1);
                                                    let reason = if !window_complete {
                                                    let mut detail = unproven_days
                                                        .iter()
                                                        .take(3)
                                                        .cloned()
                                                        .collect::<Vec<String>>()
                                                        .join("; ");
                                                    if unproven_days.len() > 3 {
                                                        detail.push_str(&format!(
                                                            "; (+{} more)",
                                                            unproven_days.len() - 3
                                                        ));
                                                    }
                                                    Some(format!(
                                                        "{} {}",
                                                        apex_lift::holdout::INCOMPLETE_WINDOW_REASON,
                                                        detail
                                                    ))
                                                } else if !measurement.coverage_ok() {
                                                    Some(format!(
                                                        "{} {} bp of feed revenue has no resolved player id.",
                                                        apex_lift::holdout::LOW_COVERAGE_REASON,
                                                        measurement.unidentified_share_bp
                                                    ))
                                                } else if measurement.estimate.is_none() {
                                                    Some(format!(
                                                        "{} The eligible roster or the {}% assignment-count test failed.",
                                                        apex_lift::holdout::NO_MEASUREMENT_REASON,
                                                        (apex_lift::holdout::ROLLOUT_SHARE * 100.0)
                                                            .round()
                                                    ))
                                                } else if measurement.roster.is_some_and(|roster| {
                                                    !measurement.estimate.as_ref().is_some_and(
                                                        |estimate| {
                                                            apex_lift::holdout::split_is_90_10(
                                                                estimate, roster,
                                                            )
                                                        },
                                                    )
                                                }) {
                                                    Some(
                                                        apex_lift::holdout::INVALID_SPLIT_REASON
                                                            .to_string(),
                                                    )
                                                } else {
                                                    None
                                                };
                                                    let eligible = reason.is_none();
                                                    if let Some(reason) = &reason {
                                                        report.holdout_no_fee.push((
                                                            change.change_id.clone(),
                                                            reason.clone(),
                                                        ));
                                                    }
                                                    // The billed interval starts at the
                                                    // later of the month start and the
                                                    // holdout's own open day.
                                                    let from_day =
                                                        if holdout.state.kept_day.as_str()
                                                            > month_start
                                                        {
                                                            holdout.state.kept_day.clone()
                                                        } else {
                                                            month_start.to_string()
                                                        };
                                                    let exposed_revenue: i64 = daily_exposed
                                                        .iter()
                                                        .filter(|(day, _)| {
                                                            day.as_str() >= from_day.as_str()
                                                                && day.as_str()
                                                                    <= month_end.as_str()
                                                        })
                                                        .map(|(_, revenue)| *revenue)
                                                        .sum();
                                                    let signed = eligible.then(|| {
                                                        apex_lift::holdout::HoldoutEvidence {
                                                            month_end: month_end.clone(),
                                                            exposed_is_test: holdout
                                                                .state
                                                                .exposed_is_test,
                                                            measurement_scope: measured_scope,
                                                            window_days,
                                                            rows: folded,
                                                            pretest: evidence.pretest,
                                                            unidentified_micros: measurement
                                                                .unidentified_micros,
                                                            unidentified_share_bp: measurement
                                                                .unidentified_share_bp,
                                                            assigned: measurement.roster,
                                                            daily_exposed: daily_exposed.clone(),
                                                            actual_gain_lower_micros:
                                                                actual_gain_lower,
                                                            delta_lower,
                                                        }
                                                    });
                                                    (
                                                        eligible
                                                            .then_some(measurement.estimate)
                                                            .flatten(),
                                                        signed,
                                                        roster,
                                                        Some(exposed_revenue),
                                                        actual_gain_lower,
                                                        delta_lower,
                                                        daily_exposed,
                                                    )
                                                }
                                                Err(error) => {
                                                    report.holdout_no_fee.push((
                                                    change.change_id.clone(),
                                                    format!(
                                                        "No fee this month: the holdout feed is unusable: {error}"
                                                    ),
                                                ));
                                                    (
                                                        None,
                                                        None,
                                                        None,
                                                        None,
                                                        None,
                                                        None,
                                                        std::collections::BTreeMap::new(),
                                                    )
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Err(error) => {
                                report.holdout_no_fee.push((
                                    change.change_id.clone(),
                                    format!(
                                        "No fee this month: the holdout feed is unavailable: {error}"
                                    ),
                                ));
                                (
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    std::collections::BTreeMap::new(),
                                )
                            }
                        };
                        if let Some(estimate) = &estimate {
                            if apex_lift::holdout::confidently_harmful_at(estimate, elapsed) {
                                // A confident harm rolls the change back and
                                // removes every later month from billing.
                                let reason = "holdoutHarm";
                                let mut rolled = holdout.state.clone();
                                rolled.roll_back(month_start, reason);
                                let signed =
                                    apex_lift::emit::holdout_state(&rolled).map_err(|error| {
                                        PlanError::Validation(format!("holdout state: {error}"))
                                    })?;
                                let payload = signed.record.canonical_json().map_err(|error| {
                                    PlanError::Validation(format!("holdout state: {error}"))
                                })?;
                                ledger
                                    .append(LedgerRecord::lift_record(
                                        accrual_at,
                                        &signed.sha256,
                                        &payload,
                                    ))
                                    .map_err(PlanError::Ledger)?;
                                let stop = apex_lift::emit::billing_change(
                                    &change.change_id,
                                    month_start,
                                    reason,
                                    0.0,
                                )
                                .map_err(|error| {
                                    PlanError::Validation(format!("billing change: {error}"))
                                })?;
                                let stop_payload =
                                    stop.record.canonical_json().map_err(|error| {
                                        PlanError::Validation(format!("billing change: {error}"))
                                    })?;
                                ledger
                                    .append(LedgerRecord::lift_record(
                                        accrual_at,
                                        &stop.sha256,
                                        &stop_payload,
                                    ))
                                    .map_err(PlanError::Ledger)?;
                                if let Err(error) = ops.deprecate_experiment(&scope).await {
                                    eprintln!(
                                        "apex-runner: holdout rollback for {} could not deprecate {scope}: {error}",
                                        change.change_id
                                    );
                                }
                                report
                                    .holdout_rollbacks
                                    .push((change.change_id.clone(), estimate.lift));
                                run_skipped.push((
                                    change.change_id.clone(),
                                    apex_lift::billing::BillingStop::RolledBack,
                                ));
                                continue;
                            }
                        }
                        let mut references = vec![
                            change.pre_registration_hash.clone(),
                            change.result_hash.clone(),
                            holdout.hash.clone(),
                        ];
                        references.retain(|reference| !reference.is_empty());
                        references.dedup();
                        holdout_changes.push(apex_lift::run::HoldoutMonthChange {
                            change,
                            estimate,
                            ledger_references: references,
                            evidence: signed_evidence,
                            roster: signed_roster,
                            exposed_revenue: exposed_revenue_basis,
                            actual_gain_lower,
                            delta_lower: signed_delta_lower,
                            exposed_daily: signed_daily,
                        });
                    }
                    let mut run = apex_lift::run::run_holdout_month(
                        month,
                        month_start,
                        month_days,
                        revenue,
                        &holdout_changes,
                    )
                    .map_err(|error| {
                        PlanError::Validation(format!("monthly holdout billing: {error}"))
                    })?;
                    for fee in &mut run.fees {
                        if fee.fee_micros != 0 {
                            continue;
                        }
                        let reported = report
                            .holdout_no_fee
                            .iter()
                            .find(|(id, _)| id == &fee.change_id)
                            .map(|(_, reason)| reason.clone());
                        // The signed line carries the typed reason the
                        // measurement produced, not a generic fallback.
                        if let (Some(line), Some(reason)) = (fee.holdout.as_mut(), reported) {
                            line.no_fee_reason = Some(reason);
                        }
                        if !report
                            .holdout_no_fee
                            .iter()
                            .any(|(id, _)| id == &fee.change_id)
                        {
                            let note = fee
                                .holdout
                                .as_ref()
                                .and_then(|line| line.no_fee_note())
                                .unwrap_or("No fee this month: no holdout gain was billed.")
                                .to_string();
                            report.holdout_no_fee.push((fee.change_id.clone(), note));
                        }
                    }
                    run_fees.extend(run.fees);
                    run_skipped.extend(run.skipped);
                } else if !scope_billing.legacy.is_empty() {
                    let run = apex_lift::run::run_month(
                        month,
                        month_start,
                        month_days,
                        revenue,
                        &scope_billing.legacy,
                    )
                    .map_err(|error| PlanError::Validation(format!("monthly billing: {error}")))?;
                    run_fees.extend(run.fees);
                    run_skipped.extend(run.skipped);
                }
                // A signed fee from an earlier pass must reach this invoice
                // even when the recomputation no longer produces its line
                // (restated scope revenue, a stop recorded between passes).
                // Without this the one-invoice-per-period register would
                // permanently drop an already-earned fee.
                for change_id in &scope_change_ids {
                    if run_fees.iter().any(|fee| &fee.change_id == change_id) {
                        continue;
                    }
                    if let Some((_, signed)) = existing_billing.get(change_id) {
                        run_fees.push(fee_line_from_billing(signed));
                    }
                }
                for fee in &run_fees {
                    // Mollie collects whole minor units: the signed fee is the
                    // cent-exact one, and the basis re-derives it the same way.
                    let (seq, fee) =
                        if let Some((seq, signed)) = existing_billing.get(&fee.change_id) {
                            // A crash after the ledger append and before invoice
                            // issuance replays the signed line, never a second fee.
                            (*seq, fee_line_from_billing(signed))
                        } else {
                            let mut fee = fee.clone();
                            fee.fee_micros = apex_billing::round_to_cent(fee.fee_micros);
                            if let Some(line) = &mut fee.holdout {
                                line.fee_micros = fee.fee_micros;
                            }
                            let record =
                                apex_lift::emit::monthly_billing(&fee, month).map_err(|error| {
                                    PlanError::Validation(format!("monthly billing: {error}"))
                                })?;
                            let payload = record.record.canonical_json().map_err(|error| {
                                PlanError::Validation(format!("monthly billing: {error}"))
                            })?;
                            let entry = ledger
                                .append(LedgerRecord::lift_record(
                                    accrual_at,
                                    &record.sha256,
                                    &payload,
                                ))
                                .map_err(PlanError::Ledger)?;
                            (entry.seq(), fee)
                        };
                    fees_by_change.insert(fee.change_id.clone(), fee.clone());
                    report.fees.push((fee.change_id.clone(), fee.fee_micros));
                    basis.push(apex_billing::BillingBasisRecord {
                        seq,
                        at_unix_ms: accrual_at,
                        change_id: fee.change_id.clone(),
                        scope_revenue: fee.scope_revenue,
                        incremental: fee.incremental,
                        log_share: fee.log_share,
                        rate_bp: fee.rate_bp,
                        fee_micros: fee.fee_micros,
                        // The signed fee already reflects every active interval,
                        // so the invoice must not pro-rate it a second time. The
                        // signed monthly record keeps the real billed day count.
                        billed_days: fee.month_days,
                        month_days: fee.month_days,
                        holdout: fee.holdout.as_ref().map(|line| apex_billing::HoldoutLine {
                            measured_lift: line.measured_lift,
                            lower_90: line.lower_90,
                            upper_90: line.upper_90,
                            lower_alpha: line.lower_alpha,
                            treated_revenue: line.revenue_micros,
                            ledger_references: line.ledger_references.clone(),
                            no_fee_reason: line.no_fee_note().map(str::to_string),
                        }),
                    });
                }
                // Term-ended changes stop billing: record the term end so the
                // billing-change pipeline is complete, not just skipped.
                for (change_id, stop) in &run_skipped {
                    if matches!(stop, apex_lift::billing::BillingStop::TermEnd) {
                        let record = apex_lift::emit::billing_change(
                            change_id,
                            &month_end,
                            "termEnded",
                            0.0,
                        )
                        .map_err(|error| {
                            PlanError::Validation(format!("billing change: {error}"))
                        })?;
                        let payload = record.record.canonical_json().map_err(|error| {
                            PlanError::Validation(format!("billing change: {error}"))
                        })?;
                        ledger
                            .append(LedgerRecord::lift_record(
                                accrual_at,
                                &record.sha256,
                                &payload,
                            ))
                            .map_err(PlanError::Ledger)?;
                    }
                }
                report.billed_scopes += 1;
            }

            // The fee preview goes out before the invoice is issued, and every
            // fee leaves a `fee_computed` event beside its ledger record.
            if !report.fees.is_empty() {
                let net_micros = report
                    .fees
                    .iter()
                    .map(|(_, fee)| *fee)
                    .fold(0i64, |total, fee| total.saturating_add(fee));
                if let Some(notify) = self.notify.as_ref() {
                    notify
                        .fee_preview(publisher_id, month, net_micros, now_unix_ms)
                        .await;
                }
            }
            if let Some(events) = self.events.as_ref() {
                for (change_id, _) in &report.fees {
                    if let Some(fee) = fees_by_change.get(change_id) {
                        events
                            .fee_computed(
                                change_id,
                                month,
                                apex_billing::BILLING_CURRENCY,
                                fee.scope_revenue,
                                (fee.lambda * 10_000.0).round() as i64,
                                (fee.factor * 10_000.0).round() as i64,
                                fee.rate_bp,
                                fee.billed_days,
                                vec![fee.pre_registration_hash.clone(), fee.result_hash.clone()],
                                accrual_at,
                            )
                            .await;
                    }
                }
            }

            // The invoice is re-derived from the signed billing records; the
            // only fixed-fee plan line is Managed ad ops, when recorded.
            // Monitoring, the audit and the app-ads.txt watch are free and
            // never appear on an invoice. Plan state is recorded, never
            // deleted: a cancelled plan bills its stop period and nothing
            // after.
            let mut invoice = match apex_billing::compute_invoice_from_basis(
                publisher_id,
                period.clone(),
                &basis,
            ) {
                Ok(invoice) => invoice,
                Err(error) => {
                    // A refused proof is a hold, not a charge: the affected draft
                    // is flagged, the customer is told, and nothing is issued.
                    let draft_reference = billing_error_seq(&error)
                        .and_then(|seq| {
                            basis
                                .iter()
                                .find(|record| record.seq == seq)
                                .map(|record| record.change_id.clone())
                        })
                        .unwrap_or_else(|| crate::notify::draft_reference(month));
                    if let Some(events) = self.events.as_ref() {
                        events
                            .fee_held(
                                &draft_reference,
                                month,
                                apex_billing::BILLING_CURRENCY,
                                "proof_hold",
                                accrual_at,
                            )
                            .await;
                    }
                    if let Some(notify) = self.notify.as_ref() {
                        notify
                            .proof_hold(publisher_id, &draft_reference, now_unix_ms)
                            .await;
                    }
                    return Err(PlanError::Validation(format!("invoice: {error}")));
                }
            };
            let next_seq = invoice
                .line_items
                .iter()
                .map(|line| line.seq)
                .max()
                .unwrap_or(0)
                + 1;
            if let Some(state) = self.load_managed_plan_state(publisher_id)? {
                if let Some(line) = state.monthly_line(next_seq, month) {
                    invoice
                        .push_plan_line(line)
                        .map_err(|error| PlanError::Validation(format!("invoice: {error}")))?;
                }
            }

            // The lawful VAT document: customer profile attached, treatment
            // decided against the seller profile, register number assigned,
            // document rendered. Every missing piece refuses this publisher's
            // invoice with its typed reason — and a refusal consumes no number,
            // sends nothing and prints nothing half-empty. A pass with no lines
            // at all (no lift fees, no plan lines) has no tax position to state
            // and issues nothing; a plan-only month (a monitored publisher with
            // no kept changes) still issues the plan line.
            let mut issued = false;
            if invoice.line_items.is_empty() {
                report.invoice_total_micros = invoice.total_micros;
            } else {
                match self.issue_vat_invoice(&mut invoice, &profile, publisher_id) {
                    Ok(rendered) => {
                        issued = true;
                        report.invoice_number = Some(invoice.number);
                        report.invoice_total_micros = invoice.gross_micros;
                        report.rendered_invoice = Some(rendered);
                    }
                    Err(error) => {
                        report.invoice_refusal = Some(error.to_string());
                        report.invoice_total_micros = 0;
                    }
                }
            }
            (invoice, issued)
        };
        if issued {
            if let Some(events) = self.events.as_ref().filter(|_| !recovered_existing) {
                for line in &invoice.line_items {
                    events
                        .fee_invoice_issued(
                            &line.experiment_id,
                            month,
                            apex_billing::BILLING_CURRENCY,
                            &invoice_reference,
                            line.gross_micros.max(line.fee_micros),
                            accrual_at,
                        )
                        .await;
                }
            }

            if let Some((provider, transport)) = &self.provider {
                // The card on file first: the newest stored method recorded
                // for this publisher in the collection log (the first
                // payment's webhook stores it) is charged through the
                // provider; a refusal or the absence of a stored method
                // falls back to a link with the typed reason on the line.
                // Uncertain failures (transport/auth) propagate instead of
                // falling back, so a possibly-landed charge is never
                // repeated through a link.
                let stored_credential = match &self.payment_events_path {
                    Some(path) => apex_billing::stored_mandate_for_publisher(path, publisher_id)
                        .map_err(|error| {
                            PlanError::Validation(format!("collection log: {error}"))
                        })?,
                    None => None,
                };
                // The durable intent journal is opened before the provider
                // call; every line is recorded there (fsynced) first.
                let intents_path = self.payment_intents_path.clone().unwrap_or_else(|| {
                    register_path
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."))
                        .join("payment-intents.jsonl")
                });
                let intents = apex_billing::PaymentIntentStore::open(&intents_path)
                    .map_err(PlanError::Billing)?;
                let receipt = apex_billing::collect_invoice_with_intents(
                    provider.as_ref(),
                    transport.as_ref(),
                    &intents,
                    &invoice,
                    stored_credential.as_deref(),
                    now_unix_ms,
                )
                .await
                .map_err(|error| PlanError::Validation(format!("collection: {error}")))?;
                report.payment_links = receipt
                    .link_urls()
                    .into_iter()
                    .map(str::to_string)
                    .collect();
                // E7: the invoice email carries the URL the customer must
                // complete. A first payment's checkout URL is the one that
                // stores the mandate later months charge — the durable
                // invoice-level payment link does not — so when collection
                // opened a first payment its URL is the one sent. Otherwise
                // the durable payment link (Payment Links API) goes out; it
                // is also durable in the intent journal, so the console and
                // the dunning sweep can hand it out again. A line whose
                // mandate charge was refused falls back to its own checkout
                // link; with no invoice-level link it is the paying URL.
                let first_payment_url = receipt.lines.iter().find_map(|line| {
                    line.first_payment
                        .then_some(line.payment_link.as_ref())
                        .flatten()
                        .map(|link| link.url.clone())
                });
                let payment_url = first_payment_url
                    .or_else(|| receipt.invoice_link.as_ref().map(|link| link.url.clone()))
                    .or_else(|| {
                        receipt.lines.iter().find_map(|line| {
                            line.payment_link.as_ref().map(|link| link.url.clone())
                        })
                    });
                if let Some(url) = payment_url {
                    if let Some(notify) = self.notify.as_ref() {
                        notify
                            .invoice_ready(
                                publisher_id,
                                &invoice_reference,
                                invoice.gross_micros.max(invoice.total_micros),
                                Some(apex_billing::BILLING_CURRENCY),
                                &url,
                                1,
                                now_unix_ms,
                            )
                            .await;
                    }
                }
                // A recovered checkout link carries a payment id too; only a
                // stored-mandate charge is a card charge.
                report.card_charges = receipt
                    .lines
                    .iter()
                    .filter(|line| line.channel == apex_billing::CollectionChannel::StoredMandate)
                    .filter_map(|line| line.payment_id.clone())
                    .collect();
                report.collected = true;
                // The collected references are recorded so a later received
                // payment can state a real remaining balance.
                if let Some(ledger_path) = &self.payment_ledger_path {
                    for line in &invoice.line_items {
                        let Ok(item) = apex_billing::InvoiceLineItem::from_invoice_line(
                            line,
                            publisher_id,
                            month,
                        ) else {
                            continue;
                        };
                        if item.amount_minor_units <= 0 {
                            continue;
                        }
                        let gross_micros = line.gross_micros.max(line.fee_micros);
                        let row = crate::payment_events::PaymentLedgerRow {
                            reference: item.reference.clone(),
                            amount_micros: gross_micros,
                            currency: item.currency.clone(),
                            at_unix_ms: now_unix_ms,
                        };
                        crate::payment_events::append_payment_ledger_row(ledger_path, &row)
                            .map_err(|error| PlanError::Validation(error.to_string()))?;
                    }
                }
            }
        }
        Ok(report)
    }

    /// Prepare and issue the VAT invoice for one publisher: attach the
    /// customer profile read from `<connections root>/<publisher>.billing.json`
    /// (exactly the `CustomerProfile` JSON shape), decide the treatment
    /// against `profile`, assign the next gapless register number through the
    /// invoice register and render the lawful document. The register line is
    /// the durable issue record; the rendered document is returned for the
    /// report. Every refusal is typed and leaves the register untouched.
    fn issue_vat_invoice(
        &self,
        invoice: &mut apex_billing::Invoice,
        profile: &apex_billing::BillingProfile,
        publisher_id: &str,
    ) -> Result<String, apex_billing::BillingError> {
        let root = self
            .connections_root
            .as_ref()
            .ok_or(apex_billing::BillingError::MissingCustomerProfile)?;
        let path = root.join(format!(
            "{publisher_id}{}",
            crate::registry::CUSTOMER_PROFILE_SUFFIX
        ));
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(apex_billing::BillingError::MissingCustomerProfile)
            }
            Err(error) => {
                return Err(apex_billing::BillingError::Malformed(format!(
                    "customer billing profile at {} could not be read: {error}",
                    path.display()
                )))
            }
        };
        let customer: apex_billing::CustomerProfile =
            serde_json::from_str(&raw).map_err(|error| {
                apex_billing::BillingError::Malformed(format!(
                    "customer billing profile at {} is not valid JSON: {error}",
                    path.display()
                ))
            })?;
        customer.validate()?;
        let prepared = invoice.clone().with_customer(customer);
        let mut prepared = match &self.vies {
            Some(vies) => prepared.apply_vat_treatment_with_vies(profile, vies.as_ref())?,
            None => prepared.apply_vat_treatment(profile)?,
        };
        let register_path = self
            .register_path
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from(apex_billing::DEFAULT_REGISTER_PATH));
        let register = apex_billing::InvoiceRegister::open(&register_path)?;
        match register.issue(&mut prepared) {
            Ok(_) => {}
            Err(apex_billing::BillingError::InvoicePeriodAlreadyIssued { number, .. }) => {
                // A concurrent pass issued the same (publisher, period) first.
                // The register is the authority: continue the existing
                // invoice instead of reporting a refusal.
                let existing = register
                    .stored_invoice(&prepared.publisher_id, &prepared.period.year_month)?
                    .ok_or(apex_billing::BillingError::RegisterCorrupt {
                        detail: format!(
                            "register reported invoice {number} for {} {} but it cannot be read back",
                            prepared.publisher_id, prepared.period.year_month
                        ),
                    })?;
                let rendered = existing.render_vat_invoice(profile)?;
                *invoice = existing;
                return Ok(rendered);
            }
            Err(error) => return Err(error),
        }
        let rendered = prepared.render_vat_invoice(profile)?;
        *invoice = prepared;
        Ok(rendered)
    }
}

impl Runner {
    /// Start an A/A (null) experiment and record it: the same configuration on
    /// both arms, so the split and pipeline can be checked before any real
    /// change rides on them. Returns the platform experiment id.
    pub async fn start_aa(
        &self,
        ledger: &mut dyn LedgerLog,
        platform: apex_core::PlatformId,
        spec: &apex_core::ExperimentSpec,
        now_unix_ms: i64,
    ) -> Result<String, PlanError> {
        let mut spec = spec.clone();
        if spec.instance_id.trim().is_empty() {
            let attempt = ledger
                .records()
                .iter()
                .filter(|record| {
                    matches!(
                        record,
                        LedgerRecord::ExperimentStarted { spec: started, .. }
                            if started.name == spec.name
                    )
                })
                .count() as u64;
            spec.instance_id =
                crate::keys::experiment_instance_id(&spec.name, now_unix_ms, "aa", attempt);
        }
        let experiment_id = self
            .ops
            .start_null_experiment(&spec)
            .await
            .map_err(|source| PlanError::ApplyFailed { source })?;
        ledger
            .append(LedgerRecord::experiment_started(
                now_unix_ms,
                platform,
                &experiment_id,
                spec,
            ))
            .map_err(PlanError::Ledger)?;
        // The platform start is confirmed: the experiment is running. The
        // A/A path has no pre-registration, so only the started event exists.
        if let Some(events) = self.events.as_ref() {
            events
                .experiment_started(&experiment_id, "running", now_unix_ms)
                .await;
        }
        Ok(experiment_id)
    }

    /// Record a completed A/A run as this publisher's qualification for
    /// `platform`: the signed `aaQualification` record the monthly pass gates
    /// holdout billing on. The figures come from the run's own measurement;
    /// the qualification expires after [`crate::aa_qualification::VALIDITY_DAYS`],
    /// so the null run is re-run monthly. Fails closed on a non-finite
    /// measurement: a figure that cannot be recorded exactly never qualifies.
    pub fn record_aa_qualification(
        &self,
        ledger: &mut dyn LedgerLog,
        platform: apex_core::PlatformId,
        srm_chi_square: f64,
        label_consistency: f64,
        estimated_null_bias: f64,
        now_unix_ms: i64,
    ) -> Result<LedgerRecord, PlanError> {
        crate::aa_qualification::record(
            ledger,
            &self.publisher_id,
            platform,
            srm_chi_square,
            label_consistency,
            estimated_null_bias,
            now_unix_ms,
        )
        .map_err(PlanError::Validation)
    }

    /// Evaluate a completed A/A run through the same date alignment, finality
    /// and clean-window path as production measurement.
    pub async fn aa_conclusion(
        &self,
        experiment_id: &str,
        planned_split: f64,
    ) -> Result<apex_lift::calibration::AaConclusion, PlanError> {
        self.aa_conclusion_from(experiment_id, planned_split, None, None)
            .await
    }

    /// Evaluate an A/A run aligned to the day it started, exactly like the
    /// production settle path. `start_day` is the registered experiment start;
    /// days before it are platform history, not A/A traffic. `instance` is the
    /// pre-registered experiment name, which keys the durable evidence store
    /// because the platform id may be reused (MAX returns the ad unit id).
    pub async fn aa_conclusion_from(
        &self,
        experiment_id: &str,
        planned_split: f64,
        start_day: Option<&str>,
        instance: Option<&str>,
    ) -> Result<apex_lift::calibration::AaConclusion, PlanError> {
        let instance_key = match (instance, start_day) {
            (Some(name), Some(day)) => format!("{name}@{day}"),
            (Some(name), None) => name.to_string(),
            _ => experiment_id.to_string(),
        };
        let mut days = self
            .ops
            .experiment_days_for(experiment_id, &instance_key)
            .await
            .map_err(|source| PlanError::ApplyFailed { source })?;
        if let Some(start_day) = start_day {
            days.control.retain(|day| day.day.as_str() >= start_day);
            days.test.retain(|day| day.day.as_str() >= start_day);
        }
        Ok(apex_lift::calibration::evaluate_aa_days(
            &days.control,
            &days.test,
            planned_split,
            apex_lift::calibration::AA_BURN_IN_DAYS,
        ))
    }
}

/// A kept change whose month-6 (or 30%-eCPM-move) re-check is due.
#[derive(Debug, Clone, PartialEq)]
pub struct RecheckDue {
    pub change_id: String,
    pub scope: String,
    pub months_since_keep: u32,
    pub plan: apex_lift::recheck::ReversalPlan,
}

impl Runner {
    /// The kept changes whose re-check is due at `now_unix_ms` by the scheduled
    /// month-6 trigger. The reversal test itself is a normal cycle; this
    /// planner tells the operator which one to start.
    pub fn due_rechecks(
        &self,
        ledger: &dyn LedgerLog,
        now_unix_ms: i64,
    ) -> Result<Vec<RecheckDue>, PlanError> {
        let today = now_unix_ms.div_euclid(86_400_000);
        let mut due = Vec::new();
        // A3: the monthly holdout replaces the month-6 reversal re-check. A
        // change with persisted holdout state never schedules one. One
        // `records()` read feeds both facts.
        let records = ledger.records();
        let holdouts = ledger_holdouts_records(&records)?;
        for (scope, platform, change) in kept_changes_records(&records)? {
            if holdouts.contains_key(&change.change_id) {
                continue;
            }
            let Some(months) = months_between(&change.kept_day, today) else {
                continue;
            };
            // A3: the early re-check reads the actual eCPM drift from the
            // persistent 28-day baseline. It is 0.0 only when no baseline or
            // no measured eCPM exists, never as a hardcoded placeholder.
            let drift = match &self.account_watch {
                Some(watch) => watch
                    .ecpm_drift(platform.as_str(), &scope)
                    .map_err(PlanError::Validation)?,
                None => 0.0,
            };
            if apex_lift::recheck::recheck_due(months, drift) {
                due.push(RecheckDue {
                    change_id: change.change_id.clone(),
                    scope,
                    months_since_keep: months,
                    plan: apex_lift::recheck::reversal_plan(),
                });
            }
        }
        Ok(due)
    }
}

/// Whole months from a `YYYY-MM-DD` day to a day number since the epoch.
fn months_between(kept_day: &str, today_day: i64) -> Option<u32> {
    let kept = crate::calendar::day_number(kept_day)?;
    let today = crate::calendar::format_day(today_day);
    let (ky, km, kd) = parse_ymd(kept_day)?;
    let (ty, tm, td) = parse_ymd(&today)?;
    if today_day < kept {
        return Some(0);
    }
    let mut months = (ty - ky) * 12 + (tm - km);
    if td < kd {
        months -= 1;
    }
    u32::try_from(months.max(0)).ok()
}

fn parse_ymd(day: &str) -> Option<(i64, i64, i64)> {
    let mut parts = day.split('-');
    let y = parts.next()?.parse().ok()?;
    let m = parts.next()?.parse().ok()?;
    let d = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((y, m, d))
}

/// The last calendar day of a month, `YYYY-MM-DD`.
fn month_end(month_start: &str, month_days: u32) -> Result<String, PlanError> {
    let start = crate::calendar::day_number(month_start).ok_or_else(|| {
        PlanError::Validation(format!("month start '{month_start}' is not a valid day"))
    })?;
    Ok(crate::calendar::format_day(
        start + i64::from(month_days.max(1)) - 1,
    ))
}

/// One durable holdout state read from the ledger, with the signed payload
/// digest that pins it.
struct LedgerHoldout {
    state: apex_lift::holdout::HoldoutState,
    hash: String,
}

/// Every holdout state in the ledger, keyed by immutable instance. The last
/// record wins so a rollback supersedes the opening state.
fn ledger_holdouts(
    ledger: &dyn LedgerLog,
) -> Result<std::collections::BTreeMap<String, LedgerHoldout>, PlanError> {
    let records = ledger.records();
    ledger_holdouts_records(&records)
}

/// [`ledger_holdouts`] over an already-read record slice.
fn ledger_holdouts_records(
    records: &[&LedgerRecord],
) -> Result<std::collections::BTreeMap<String, LedgerHoldout>, PlanError> {
    let mut out = std::collections::BTreeMap::new();
    for record in records.iter().copied() {
        let LedgerRecord::LiftRecord {
            payload,
            payload_sha256,
            seq,
            ..
        } = record
        else {
            continue;
        };
        let parsed =
            parse_lift_record(payload).map_err(|error| PlanError::MalformedLiftRecord {
                seq: record.seq(),
                detail: error.to_string(),
            })?;
        if let apex_lift::records::LiftRecord::HoldoutState(state) = parsed {
            let _ = seq;
            out.insert(
                state.change_id.clone(),
                LedgerHoldout {
                    state,
                    hash: payload_sha256.clone(),
                },
            );
        }
    }
    Ok(out)
}

/// The signed monthly billing records already written for `month`, keyed by
/// change id. A pass that crashed after the ledger append but before invoice
/// issuance reuses them instead of appending a second fee.
fn ledger_monthly_billing(
    ledger: &dyn LedgerLog,
    month: &str,
) -> Result<std::collections::BTreeMap<String, (u64, apex_lift::records::BillingRecord)>, PlanError>
{
    let records = ledger.records();
    ledger_monthly_billing_records(&records, month)
}

/// [`ledger_monthly_billing`] over an already-read record slice.
fn ledger_monthly_billing_records(
    records: &[&LedgerRecord],
    month: &str,
) -> Result<std::collections::BTreeMap<String, (u64, apex_lift::records::BillingRecord)>, PlanError>
{
    let mut out = std::collections::BTreeMap::new();
    for record in records.iter().copied() {
        let LedgerRecord::LiftRecord { seq, payload, .. } = record else {
            continue;
        };
        let parsed =
            parse_lift_record(payload).map_err(|error| PlanError::MalformedLiftRecord {
                seq: record.seq(),
                detail: error.to_string(),
            })?;
        if let apex_lift::records::LiftRecord::MonthlyBilling(billing) = parsed {
            if billing.month == month {
                out.entry(billing.change_id.clone())
                    .or_insert_with(|| (*seq, billing));
            }
        }
    }
    Ok(out)
}

/// Keep only the trailing 28-day player rows ending on `month_end`.
fn trailing_measurement(
    rows: Vec<apex_lift::player_store::PlayerDay>,
    month_end: &str,
) -> Vec<apex_lift::player_store::PlayerDay> {
    let Some(end) = crate::calendar::day_number(month_end) else {
        return Vec::new();
    };
    let start = end - (apex_lift::holdout::MEASUREMENT_DAYS as i64 - 1);
    rows.into_iter()
        .filter(|row| {
            crate::calendar::day_number(&row.day).is_some_and(|day| day >= start && day <= end)
        })
        .collect()
}

/// Convert an already-signed month record back into the fee line the invoice
/// basis and report use.
fn fee_line_from_billing(
    record: &apex_lift::records::BillingRecord,
) -> apex_lift::billing::FeeLine {
    apex_lift::billing::FeeLine {
        change_id: record.change_id.clone(),
        scope_revenue: record.scope_revenue,
        treated_revenue: if record.treated_revenue > 0 {
            record.treated_revenue
        } else {
            record
                .holdout
                .as_ref()
                .map_or(record.scope_revenue, |line| line.revenue_micros)
        },
        holdout: record.holdout.clone(),
        lambda: record.lambda,
        factor: record.factor,
        incremental: record.incremental,
        log_share: record.log_share,
        rate_bp: record.rate_bp,
        fee_micros: record.fee_micros,
        billed_days: record.billed_days,
        month_days: record.month_days,
        pre_registration_hash: record.pre_registration_hash.clone(),
        result_hash: record.result_hash.clone(),
    }
}

/// Every kept change in the ledger, paired with its pre-registered scope and
/// the platform its experiment ran on (the fee-eligibility gate reads the
/// platform).
///
/// One pass builds the per-experiment index (the first pre-registration and
/// the first keep-result, matching the pre-index `find_map` semantics); a
/// second pass walks the `experimentStarted` records in ledger order. The old
/// shape rescanned — and re-parsed — the whole ledger for every start, which
/// is quadratic in experiments times records. An unparseable lift payload
/// refuses the pass with the record's sequence instead of being skipped:
/// failing open would silently shrink the fee preview.
fn kept_changes(
    ledger: &dyn LedgerLog,
) -> Result<
    Vec<(
        String,
        apex_core::PlatformId,
        apex_lift::billing::KeptChange,
    )>,
    PlanError,
> {
    let records = ledger.records();
    kept_changes_records(&records)
}

/// [`kept_changes`] over an already-read record slice.
fn kept_changes_records(
    records: &[&LedgerRecord],
) -> Result<
    Vec<(
        String,
        apex_core::PlatformId,
        apex_lift::billing::KeptChange,
    )>,
    PlanError,
> {
    struct RawStart {
        platform: apex_core::PlatformId,
        spec: apex_core::ExperimentSpec,
        change_seq: Option<u64>,
    }

    let mut starts: Vec<RawStart> = Vec::new();
    let mut last_change_seq: Option<u64> = None;
    let mut pre_registrations: std::collections::BTreeMap<String, (String, u16)> =
        std::collections::BTreeMap::new();
    let mut keep_results: std::collections::BTreeMap<String, (i64, String, f64)> =
        std::collections::BTreeMap::new();
    let mut rollback_days: std::collections::BTreeMap<u64, String> =
        std::collections::BTreeMap::new();
    let mut billing_change_days: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();

    for record in records.iter().copied() {
        match record {
            LedgerRecord::ChangeApplied { seq, .. } => {
                last_change_seq = Some(last_change_seq.map_or(*seq, |seen| seen.max(*seq)));
            }
            LedgerRecord::ExperimentStarted {
                at_unix_ms,
                platform,
                experiment_id,
                spec,
                ..
            } => {
                let _ = at_unix_ms;
                let _ = experiment_id;
                starts.push(RawStart {
                    platform: *platform,
                    spec: spec.clone(),
                    change_seq: last_change_seq,
                });
            }
            LedgerRecord::RollbackPerformed {
                at_unix_ms,
                rolled_back_seq,
                ..
            } => {
                rollback_days.entry(*rolled_back_seq).or_insert_with(|| {
                    crate::calendar::format_day(at_unix_ms.div_euclid(86_400_000))
                });
            }
            LedgerRecord::LiftRecord {
                at_unix_ms,
                payload_sha256,
                payload,
                ..
            } => {
                let parsed =
                    parse_lift_record(payload).map_err(|error| PlanError::MalformedLiftRecord {
                        seq: record.seq(),
                        detail: error.to_string(),
                    })?;
                match parsed {
                    apex_lift::records::LiftRecord::PreRegistration {
                        experiment_id,
                        instance_id,
                        rate_bp,
                        ..
                    } => {
                        pre_registrations
                            .entry(instance_or_name(&instance_id, &experiment_id))
                            .or_insert_with(|| (payload_sha256.clone(), rate_bp));
                    }
                    apex_lift::records::LiftRecord::Result {
                        experiment_id,
                        instance_id,
                        decision,
                        lambda,
                        ..
                    } if decision == "keep" && lambda > 0.0 => {
                        keep_results
                            .entry(instance_or_name(&instance_id, &experiment_id))
                            .or_insert_with(|| (*at_unix_ms, payload_sha256.clone(), lambda));
                    }
                    apex_lift::records::LiftRecord::BillingChange(change) => {
                        billing_change_days
                            .entry(change.change_id)
                            .or_insert(change.day);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    for start in &starts {
        let instance_id = instance_or_name(&start.spec.instance_id, &start.spec.name);
        let Some((pre_hash, rate_bp)) = pre_registrations.get(&instance_id) else {
            continue;
        };
        let Some((at_unix_ms, result_hash, lambda)) = keep_results.get(&instance_id) else {
            continue;
        };
        // An out-of-band rate can never bill: the invoice's arithmetic
        // re-verification refuses it, so the fee preview must not present
        // it either. Skip loudly instead of diverging from the invoice.
        if !(apex_core::money::MIN_LIFT_RATE_BP..=apex_core::money::MAX_LIFT_RATE_BP)
            .contains(rate_bp)
            && *rate_bp != apex_core::money::RESULTS_RATE_BP
        {
            eprintln!(
                "apex-runner: keeping change {instance_id} out of the fee preview: rate {rate_bp} bp is neither a legacy contract rate nor the A3 results rate and the invoice would refuse it"
            );
            continue;
        }
        let rolled_back_day = start
            .change_seq
            .and_then(|seq| rollback_days.get(&seq))
            .cloned();
        let overridden_day = billing_change_days
            .get(&instance_id)
            .or_else(|| billing_change_days.get(&start.spec.name))
            .cloned();
        out.push((
            start.spec.ad_unit_id.clone(),
            start.platform,
            apex_lift::billing::KeptChange {
                // The billing identity is the immutable experiment instance,
                // never the reusable platform experiment name.
                change_id: instance_id,
                lambda: *lambda,
                rate_bp: *rate_bp,
                kept_day: crate::calendar::format_day(at_unix_ms.div_euclid(86_400_000)),
                overridden_day,
                rolled_back_day,
                pre_registration_hash: pre_hash.clone(),
                result_hash: result_hash.clone(),
            },
        ));
    }
    Ok(out)
}

/// The instance id a record pins, falling back to the experiment name only
/// for legacy records written before instance ids existed.
fn instance_or_name(instance_id: &str, experiment_id: &str) -> String {
    if instance_id.trim().is_empty() {
        experiment_id.to_string()
    } else {
        instance_id.to_string()
    }
}

/// Signed lift payload parses since the last reset; the scan tests count
/// parse work through the shared ledger-index seam, so a payload parsed
/// while building the index is not parsed again by the scan.
#[cfg(test)]
fn parse_call_count() -> u64 {
    crate::ledger_index::parse_call_count()
}

#[cfg(test)]
fn reset_parse_call_count() {
    crate::ledger_index::reset_parse_call_count();
}

/// Whether two settings encodings describe the same value, comparing parsed
/// JSON when both parse (so key order or spacing never reads as an edit).
fn same_settings(a: &str, b: &str) -> bool {
    match (
        serde_json::from_str::<serde_json::Value>(a),
        serde_json::from_str::<serde_json::Value>(b),
    ) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The invoice display reference the events and messages quote:
/// `INV-<YYYY-MM>-<publisher>` (the ledger has no invoice registry, so the
/// reference is derived from the invoicing facts).
pub fn invoice_reference(publisher_id: &str, month: &str) -> String {
    format!("INV-{month}-{publisher_id}")
}

/// The ledger sequence a billing refusal names, when it names one.
fn billing_error_seq(error: &apex_billing::BillingError) -> Option<u64> {
    match error {
        apex_billing::BillingError::OutOfPeriod { seq }
        | apex_billing::BillingError::WrongPublisher { seq }
        | apex_billing::BillingError::TamperedProof { seq, .. }
        | apex_billing::BillingError::Duplicate { seq, .. }
        | apex_billing::BillingError::SubCentAmount { seq }
        | apex_billing::BillingError::UnsafeIdentifier { seq, .. } => Some(*seq),
        _ => None,
    }
}

/// The bounded error class of a platform failure, for pull events.
fn error_class_of(source: &apex_core::PlatformError) -> &'static str {
    match source {
        apex_core::PlatformError::Auth { .. } => "auth_refusal",
        apex_core::PlatformError::RateLimited { .. } => "rate_limit",
        apex_core::PlatformError::Schema { .. } => "schema_failure",
        apex_core::PlatformError::Network { .. } => "transport_failure",
        apex_core::PlatformError::Http { .. } => "transport_failure",
        apex_core::PlatformError::Unsupported { .. } | apex_core::PlatformError::Validation(_) => {
            "validation_failure"
        }
    }
}

/// How a fired guardrail's observed input reads in the event payload.
fn guardrail_input(event: &apex_lift::guardrails::GuardrailEvent) -> &'static str {
    use apex_lift::guardrails::GuardrailEvent;
    match event {
        GuardrailEvent::FastHarm { .. } => "point lift below the registered harm threshold",
        GuardrailEvent::ConfirmedHarm { .. } => "upper bound below zero",
        GuardrailEvent::FillRewardedInterstitial { .. } => {
            "fill below the registered threshold on two consecutive days"
        }
        GuardrailEvent::FillBanner { .. } => {
            "banner fill below the registered threshold on two consecutive days"
        }
        GuardrailEvent::PublisherEdit { .. } => "settings changed outside Apex",
        GuardrailEvent::SplitMismatch { .. } => {
            "arm split past the registered chi-square threshold"
        }
        GuardrailEvent::ReportingGap { .. } => "days still not final past 72 hours",
        GuardrailEvent::PlatformIncident { .. } => {
            "both arms' revenue fell past the incident threshold"
        }
    }
}

fn status_label(outcome: &SettleOutcome) -> String {
    match outcome {
        SettleOutcome::Kept { .. } => "kept".into(),
        SettleOutcome::RolledBack { .. } => "rolled_back".into(),
        SettleOutcome::Voided { .. } => "voided".into(),
        SettleOutcome::PromotionPending { .. } => "promotion_pending".into(),
        SettleOutcome::Pending { .. } => "pending".into(),
    }
}

/// One running experiment: its platform id and the ledger sequence of the
/// change it tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningExperiment {
    pub experiment_id: String,
    /// The immutable experiment instance id carried by the signed
    /// pre-registration and start record.
    pub instance_id: String,
    /// The durable pull-evidence key: the immutable experiment instance. MAX
    /// reuses the ad unit id as the platform id, so the platform id alone is
    /// not unique per experiment.
    pub instance_key: String,
    pub change_seq: u64,
    /// The platform its applied change belongs to. Reporting and settlement
    /// use this platform's own adapter.
    pub platform: PlatformId,
    /// The day the experiment started, taken from its own start record rather
    /// than a platform-id-keyed map (two runs can share a platform id).
    pub started_day: String,
    pub ad_unit_id: String,
    /// The scope's value as pre-registered; the check flags a publisher edit
    /// when the scope's current settings differ.
    pub expected_settings: String,
    /// The applied change the experiment tests: the edit check compares the
    /// specific setting this change modifies, never the whole scope document.
    pub change: Option<ChangeKind>,
    /// A signed terminal result exists but its remote side effect was not
    /// completed before a crash. The pass reconciles it instead of pulling
    /// reporting again.
    pub pending_terminal: bool,
}

/// The running experiments plus the per-experiment start days the same scan
/// observed, so one ledger read serves both the pass's settle loop and its
/// pull range events.
///
/// Experiments that have started but have no terminal record yet are running:
/// a settled experiment leaves an `experimentObserved` record (keep) or a
/// `rollbackPerformed` record (rollback); a voided one leaves neither, so a
/// void is re-checked until the window closes and the engine voids again. An
/// experiment with a signed lift `Result` but no observation (the
/// result-before-pre-registration shape) is reported as already settled and
/// never pulled, so repeated passes append nothing for it.
#[derive(Debug)]
struct RunningScan {
    experiments: Vec<RunningExperiment>,
    /// Experiment ids the ledger already holds a signed result for, in start
    /// order. The settle path refuses these; the pass reports the refusal
    /// without consuming a pull.
    settled_by_result: Vec<String>,
    #[allow(dead_code)]
    started_days: std::collections::BTreeMap<String, String>,
}

/// One `records()` read, folded once into a [`LedgerIndex`]: the index holds
/// every per-id fact (pre-registered names with their `beforeValue`, settled
/// ids, rolled-back sequences, start days, and each start paired with the
/// last applied change before it) — a terminal record can appear after the
/// start it settles, so these must reflect the whole ledger before any start
/// is judged. The old shape rescanned the whole ledger per started
/// experiment. An unparseable lift payload refuses the scan with the record's
/// sequence instead of being skipped: failing open would silently drop the
/// experiment it belongs to.
#[cfg(test)]
fn running_scan(ledger: &dyn LedgerLog) -> Result<RunningScan, PlanError> {
    let index = LedgerIndex::build(ledger);
    running_scan_indexed(&index)
}

/// The running scan over an index the caller already built (the daily pass
/// shares one index across the scan and every settle).
fn running_scan_indexed(index: &LedgerIndex) -> Result<RunningScan, PlanError> {
    if let Some((seq, detail)) = index.parse_failure() {
        return Err(PlanError::MalformedLiftRecord {
            seq,
            detail: detail.to_string(),
        });
    }
    // Only experiments with a signed pre-registration are settled here; an
    // A/A (null) run has none and is evaluated through `aa_conclusion`
    // instead, so it never gets mis-attributed to another change's sequence.
    let mut experiments = Vec::new();
    let mut settled_by_result = Vec::new();
    for start in index.starts() {
        if !index.is_pre_registered(&start.instance_id) {
            continue;
        }
        let Some(change_seq) = start.change_seq else {
            continue;
        };
        if index.is_rolled_back(change_seq) {
            continue;
        }
        let pending_terminal =
            index.pending_terminal(&start.experiment_id, &start.instance_id, change_seq);
        if index.has_result(&start.instance_id) && !pending_terminal {
            // A completed result is a finished experiment: skip it silently.
            // A result that predates its start/registration is the hostile
            // shape the pass reports.
            if !index.is_observed(&start.experiment_id)
                && index
                    .settled_refusal(&start.experiment_id, change_seq)
                    .is_some()
            {
                settled_by_result.push(start.experiment_id.clone());
            }
            continue;
        }
        let expected_settings = index
            .before_value(&start.instance_id)
            .unwrap_or_default()
            .to_string();
        let platform = index
            .change(change_seq)
            .map(|change| change.platform)
            .unwrap_or(PlatformId::AppLovinMax);
        let change = index
            .change(change_seq)
            .map(|applied| applied.change.clone());
        experiments.push(RunningExperiment {
            experiment_id: start.experiment_id.clone(),
            instance_id: start.instance_id.clone(),
            instance_key: start.instance_id.clone(),
            change_seq,
            platform,
            started_day: start.started_day.clone(),
            ad_unit_id: start.spec.ad_unit_id.clone(),
            expected_settings,
            change,
            pending_terminal,
        });
    }
    Ok(RunningScan {
        experiments,
        settled_by_result,
        started_days: index.started_days().clone(),
    })
}

/// The prior bucket a settled change belongs to: a country-floor move on its
/// platform. The ad format is not carried by the change record, so the bucket
/// pools across formats for the platform and tier. This is an index lookup:
/// with priors configured the learning step previously rescanned the whole
/// ledger once per settled experiment.
fn change_bucket(index: &LedgerIndex, change_seq: u64) -> Option<apex_lift::priors::Bucket> {
    let applied = index.change(change_seq)?;
    match &applied.change {
        ChangeKind::CountryFloorSet { country, .. } => Some(apex_lift::priors::Bucket {
            lever: "floor".into(),
            ad_format: "unknown".into(),
            country_tier: country.clone(),
            platform: applied.platform.as_str().to_string(),
        }),
        _ => None,
    }
}

/// One cohort's share of the certificate family error: 5% split over two
/// one-sided statements per cohort and every simultaneous cohort.
fn certificate_family_alpha(cohorts: usize) -> f64 {
    0.05 / (2.0 * cohorts.max(1) as f64)
}

/// Turn the adapter's envelope evidence into certificate users, joining the
/// observed per-user revenue. A user whose envelope arm contradicts the
/// observed feed arm yields no certificate.
fn certified_actual_gain(
    eligible: &[crate::ops::EligibleUserEvidence],
    exposed_by_user: &std::collections::BTreeMap<String, (bool, i64)>,
    roster: Option<apex_lift::holdout::AssignedRoster>,
    alpha: f64,
) -> Option<(i64, f64)> {
    let roster = roster?;
    // The certificate and the lift estimate must describe one cohort.
    if eligible.len() as u64 != roster.total()
        || eligible.iter().filter(|user| user.exposed).count() as u64 != roster.exposed
    {
        return None;
    }
    let mut seen_users = std::collections::BTreeSet::new();
    if !eligible
        .iter()
        .all(|user| seen_users.insert(user.user_id.as_str()))
    {
        return None;
    }
    let mut users = Vec::with_capacity(eligible.len());
    for user in eligible {
        // Missing outcomes are imputed at the conservative limit: the lower
        // limit for an exposed user, the upper envelope for a control user.
        let present = exposed_by_user.get(&user.user_id).copied();
        let (observed_exposed, revenue) = present.unwrap_or((
            user.exposed,
            if user.exposed {
                0
            } else {
                user.envelope_micros
            },
        ));
        if observed_exposed != user.exposed {
            return None;
        }
        users.push(apex_lift::certificate::CertifiedUser {
            test: user.exposed,
            outcome: revenue as f64,
            prior: user.prior_micros as f64,
            envelope: user.envelope_micros as f64,
            observed: present.is_some(),
        });
    }
    apex_lift::certificate::certify(&users, apex_lift::holdout::ROLLOUT_SHARE, alpha).map(
        |certificate| {
            (
                certificate.actual_gain_lower.floor() as i64,
                certificate.delta_lower,
            )
        },
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_core::{ExperimentResult, ExperimentState};
    use apex_proof::{InMemoryLedger, Keypair};
    use async_trait::async_trait;

    use crate::model::Action;
    use crate::ops::{ExperimentDays, PlatformOps, RecordingOps, UnmeasuredConclusion};
    use crate::plan::run_cycle;

    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    fn ledger() -> InMemoryLedger {
        InMemoryLedger::new(Keypair::from_bytes(&[9u8; 32]))
    }

    fn floor_action() -> Action {
        Action::SetCountryFloor {
            ad_unit_id: "u1".into(),
            country: "ee".into(),
            floor_ecpm: 2_500_000,
        }
    }

    fn experiment_spec(name: &str, unit: &str) -> apex_core::ExperimentSpec {
        apex_core::ExperimentSpec {
            name: name.to_string(),
            instance_id: String::new(),
            ad_unit_id: unit.to_string(),
            metric: apex_core::ExperimentMetric::Ecpm,
            variants: 2,
            change_description: "raise the EE floor".into(),
            test_group_allocation: 25,
        }
    }

    fn append_lift(
        ledger: &mut InMemoryLedger,
        record: apex_lift::records::LiftRecord,
        at_unix_ms: i64,
    ) {
        let payload = record.canonical_json().expect("canonical json");
        let sha = record.sha256_hex().expect("sha");
        ledger
            .append(LedgerRecord::lift_record(at_unix_ms, &sha, &payload))
            .expect("append lift record");
    }

    fn instance_spec(name: &str, instance_id: &str, unit: &str) -> apex_core::ExperimentSpec {
        apex_core::ExperimentSpec {
            name: name.to_string(),
            instance_id: instance_id.to_string(),
            ad_unit_id: unit.to_string(),
            metric: apex_core::ExperimentMetric::Ecpm,
            variants: 2,
            change_description: "reuse the same platform name".into(),
            test_group_allocation: 25,
        }
    }

    fn instance_prereg(
        name: &str,
        instance_id: &str,
        unit: &str,
    ) -> apex_lift::records::LiftRecord {
        apex_lift::records::LiftRecord::PreRegistration {
            experiment_id: name.to_string(),
            instance_id: instance_id.to_string(),
            scope: unit.to_string(),
            before_value: serde_json::json!({"floor": 2_500_000}),
            after_value: serde_json::json!({"change": "raise the EE floor"}),
            split: 0.25,
            metric: "revenue_per_user".into(),
            burn_in_days: 3,
            window_days: 14,
            looks: 1,
            extension_allowed: false,
            guardrail_thresholds: serde_json::json!({
                "harmPoint": -0.05,
                "fillRewarded": 0.02,
                "fillBanner": 0.05,
                "splitChiSquare": 10.8276,
            }),
            code_version_hash: "deadbeef".into(),
            rate_bp: 2_000,
            min_point_lift: 0.01,
        }
    }

    fn instance_result(
        name: &str,
        instance_id: &str,
        decision: &str,
        lambda: f64,
    ) -> apex_lift::records::LiftRecord {
        apex_lift::records::LiftRecord::Result {
            experiment_id: name.to_string(),
            instance_id: instance_id.to_string(),
            daily_log_ratios: vec![0.01, 0.02],
            mean_log: 0.015,
            lower_90: 0.005,
            lower_99: 0.002,
            bootstrap_lower: 0.004,
            decision: decision.into(),
            lambda,
        }
    }

    fn append_instance_attempt(
        ledger: &mut InMemoryLedger,
        name: &str,
        instance_id: &str,
        unit: &str,
        key: &str,
        at: i64,
    ) {
        append_lift(ledger, instance_prereg(name, instance_id, unit), at);
        ledger
            .append(LedgerRecord::change_applied(
                at,
                apex_core::PlatformId::AppLovinMax,
                unit,
                ChangeKind::CountryFloorSet {
                    country: "EE".into(),
                    floor_ecpm: 2_500_000,
                },
                key,
                "",
            ))
            .expect("change");
        ledger
            .append(LedgerRecord::experiment_started(
                at + 1,
                apex_core::PlatformId::AppLovinMax,
                name,
                instance_spec(name, instance_id, unit),
            ))
            .expect("start");
    }

    fn arm_day(day: &str, revenue: i64) -> apex_lift::window::ArmDay {
        apex_lift::window::ArmDay {
            day: day.to_string(),
            revenue,
            users: None,
            second_pull_revenue: Some(revenue),
            second_pull_users: None,
            fill: None,
            second_pull_fill: None,
            first_observed_at_unix_s: Some(1_700_000_000),
            second_observed_at_unix_s: Some(
                1_700_000_000 + apex_lift::window::SECOND_PULL_MIN_SECS,
            ),
            first_source_sha256: Some("a".repeat(64)),
            second_source_sha256: Some("b".repeat(64)),
        }
    }

    fn constant_days(control: i64, test: i64) -> ExperimentDays {
        ExperimentDays {
            control: (0..10)
                .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), control / 4 * 3))
                .collect(),
            test: (0..10)
                .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), test / 4))
                .collect(),
        }
    }

    fn policy() -> AutopilotPolicy {
        AutopilotPolicy {
            window_days: 7,
            interim_look: false,
            extension_allowed: false,
            ..AutopilotPolicy::conservative()
        }
    }

    /// A scripted adapter whose reporting read sleeps, so the test can see how
    /// many pulls were in flight and how long the pass took. Only
    /// `experiment_days` is delayed; every other call delegates.
    struct DelayedOps {
        inner: Arc<RecordingOps>,
        delay: std::time::Duration,
        fail_id: Option<String>,
        in_flight: std::sync::atomic::AtomicUsize,
        max_in_flight: std::sync::atomic::AtomicUsize,
    }

    impl DelayedOps {
        fn new(
            inner: Arc<RecordingOps>,
            delay: std::time::Duration,
            fail_id: Option<String>,
        ) -> Self {
            Self {
                inner,
                delay,
                fail_id,
                in_flight: std::sync::atomic::AtomicUsize::new(0),
                max_in_flight: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn max_in_flight(&self) -> usize {
            self.max_in_flight.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl PlatformOps for DelayedOps {
        fn supports_player_feed(&self) -> bool {
            false
        }

        fn platform(&self) -> apex_core::PlatformId {
            self.inner.platform()
        }

        async fn writable(&self, operation: &str) -> bool {
            self.inner.writable(operation).await
        }

        async fn start_experiment(
            &self,
            request: &crate::ops::ExperimentRequest,
        ) -> Result<String, apex_core::PlatformError> {
            self.inner.start_experiment(request).await
        }

        async fn experiment(
            &self,
            experiment_id: &str,
        ) -> Result<ExperimentResult, apex_core::PlatformError> {
            // The timing test measures the reporting pulls; a stopped result
            // makes the settle step return immediately after its own platform
            // read, so settlement cost cannot mask the pull concurrency.
            Ok(ExperimentResult {
                experiment_id: experiment_id.to_string(),
                state: ExperimentState::Stopped,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            })
        }

        fn measures_variants(&self) -> bool {
            self.inner.measures_variants()
        }

        async fn conclude_unmeasured_experiment(
            &self,
            experiment_id: &str,
        ) -> Result<UnmeasuredConclusion, apex_core::PlatformError> {
            self.inner
                .conclude_unmeasured_experiment(experiment_id)
                .await
        }

        async fn experiment_days(
            &self,
            experiment_id: &str,
        ) -> Result<ExperimentDays, apex_core::PlatformError> {
            use std::sync::atomic::Ordering;
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            let result = if self.fail_id.as_deref() == Some(experiment_id) {
                Err(apex_core::PlatformError::Validation(format!(
                    "injected pull failure for {experiment_id}"
                )))
            } else {
                Ok(ExperimentDays {
                    control: Vec::new(),
                    test: Vec::new(),
                })
            };
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            result
        }

        async fn promote_experiment(
            &self,
            experiment_id: &str,
        ) -> Result<(), apex_core::PlatformError> {
            self.inner.promote_experiment(experiment_id).await
        }

        async fn deprecate_experiment(
            &self,
            experiment_id: &str,
        ) -> Result<(), apex_core::PlatformError> {
            self.inner.deprecate_experiment(experiment_id).await
        }

        async fn apply(&self, action: &Action, key: &str) -> Result<(), apex_core::PlatformError> {
            self.inner.apply(action, key).await
        }

        async fn rollback(&self, action: &Action) -> Result<(), apex_core::PlatformError> {
            self.inner.rollback(action).await
        }

        fn call_log(&self) -> Vec<String> {
            self.inner.call_log()
        }

        async fn current_settings(
            &self,
            ad_unit_id: &str,
        ) -> Result<String, apex_core::PlatformError> {
            self.inner.current_settings(ad_unit_id).await
        }

        async fn scope_revenue(
            &self,
            ad_unit_id: &str,
            start: &str,
            end: &str,
        ) -> Result<apex_core::Micros, apex_core::PlatformError> {
            self.inner.scope_revenue(ad_unit_id, start, end).await
        }
    }

    /// A signature-free append-only ledger for the concurrency timing test.
    /// It assigns sequence numbers and preserves the exact `LedgerRecord`
    /// order, so ledger order/outcomes can be compared without Ed25519
    /// signing (which dominates a debug-build timing test) masking the pull
    /// measurements.
    #[derive(Default)]
    struct FastLedger {
        records: Vec<LedgerRecord>,
    }

    impl FastLedger {
        fn with_seq(record: LedgerRecord, seq: u64) -> LedgerRecord {
            match record {
                LedgerRecord::ChangeApplied {
                    at_unix_ms,
                    platform,
                    ad_unit_id,
                    change,
                    idempotency_key,
                    prev_value,
                    ..
                } => LedgerRecord::ChangeApplied {
                    seq,
                    at_unix_ms,
                    platform,
                    ad_unit_id,
                    change,
                    idempotency_key,
                    prev_value,
                    prev_hash: [0u8; 32],
                    hash: [0u8; 32],
                },
                LedgerRecord::ExperimentStarted {
                    at_unix_ms,
                    platform,
                    experiment_id,
                    spec,
                    ..
                } => LedgerRecord::ExperimentStarted {
                    seq,
                    at_unix_ms,
                    platform,
                    experiment_id,
                    spec,
                    prev_hash: [0u8; 32],
                    hash: [0u8; 32],
                },
                LedgerRecord::LiftRecord {
                    at_unix_ms,
                    payload_sha256,
                    payload,
                    ..
                } => LedgerRecord::LiftRecord {
                    seq,
                    at_unix_ms,
                    payload_sha256,
                    payload,
                    prev_hash: [0u8; 32],
                    hash: [0u8; 32],
                },
                other => other,
            }
        }
    }

    impl LedgerLog for FastLedger {
        fn append(&mut self, draft: LedgerRecord) -> Result<LedgerRecord, apex_proof::LedgerError> {
            let record = Self::with_seq(draft, self.records.len() as u64);
            self.records.push(record.clone());
            Ok(record)
        }

        fn append_rollback(
            &mut self,
            at_unix_ms: i64,
            platform: apex_core::PlatformId,
            ad_unit_id: &str,
            rolled_back_seq: u64,
            reason: &str,
        ) -> Result<LedgerRecord, apex_proof::LedgerError> {
            self.append(LedgerRecord::rollback_performed(
                at_unix_ms,
                platform,
                ad_unit_id,
                rolled_back_seq,
                reason,
            ))
        }

        fn records(&self) -> Vec<&LedgerRecord> {
            self.records.iter().collect()
        }

        fn len(&self) -> usize {
            self.records.len()
        }

        fn get(&self, index: usize) -> Option<&LedgerRecord> {
            self.records.get(index)
        }

        fn is_empty(&self) -> bool {
            self.records.is_empty()
        }
    }

    /// Seed `count` running experiments into `ledger`.
    async fn seed_running_ledger(
        ops: &Arc<RecordingOps>,
        ledger: &mut dyn LedgerLog,
        count: usize,
    ) {
        ops.set_change_in_test_arm(true);
        ops.script_start((0..count).map(|index| Ok(format!("exp-{index}"))).collect());
        for index in 0..count {
            ops.put_experiment(
                &format!("exp-{index}"),
                ExperimentResult {
                    experiment_id: format!("exp-{index}"),
                    state: ExperimentState::Running,
                    results: vec![],
                    lift_percent_bp: None,
                    confidence: None,
                    winner_variant_id: None,
                },
            );
        }
        let policy = AutopilotPolicy {
            max_actions_per_day: count as u32,
            ..policy()
        };
        let proposals = (0..count)
            .map(|index| crate::model::Proposal {
                action: Action::SetCountryFloor {
                    ad_unit_id: format!("u{index}"),
                    country: "ee".into(),
                    floor_ecpm: 2_500_000,
                },
                rationale: "seed a running experiment".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            })
            .collect();
        run_cycle(ops.as_ref(), ledger, &policy, &[], proposals, NOW)
            .await
            .expect("seed running experiments");
    }

    #[tokio::test]
    async fn reporting_pulls_are_bounded_and_the_ledger_matches_the_serial_order() {
        const EXPERIMENTS: usize = 50;
        const DELAY_MS: u64 = 150;
        let mut runs = Vec::new();
        for concurrency in [1usize, 4usize] {
            let inner = Arc::new(RecordingOps::new());
            let mut l = FastLedger::default();
            seed_running_ledger(&inner, &mut l, EXPERIMENTS).await;
            let ops = Arc::new(DelayedOps::new(
                inner,
                std::time::Duration::from_millis(DELAY_MS),
                Some("exp-25".into()),
            ));
            let runner = Runner::new(
                ops.clone(),
                AutopilotPolicy {
                    max_actions_per_day: EXPERIMENTS as u32,
                    ..policy()
                },
                FillFormat::Rewarded,
            )
            .with_reporting_concurrency(concurrency);
            let started = std::time::Instant::now();
            let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
            let elapsed = started.elapsed();
            runs.push((
                concurrency,
                elapsed,
                ops.max_in_flight(),
                report,
                l.records().into_iter().cloned().collect::<Vec<_>>(),
            ));
        }
        let (_, serial_elapsed, serial_max, serial_report, serial_records) = &runs[0];
        let (_, concurrent_elapsed, concurrent_max, concurrent_report, concurrent_records) =
            &runs[1];
        eprintln!(
            "reporting pulls: serial={serial_elapsed:?} concurrent={concurrent_elapsed:?} \
             max_in_flight={concurrent_max} experiments={EXPERIMENTS} delay={DELAY_MS}ms"
        );
        assert_eq!(*serial_max, 1, "the serial run pulls one at a time");
        assert!(
            *concurrent_max <= 4 && *concurrent_max > 1,
            "bounded concurrency 4, saw {concurrent_max}"
        );
        // Serial: 50 * 150ms = 7.5s. Concurrent: ceil(50/4) * 150ms = 1.95s.
        assert!(
            concurrent_elapsed.as_millis() < 3_000,
            "concurrent pass took {concurrent_elapsed:?}, expected near 1.95s"
        );
        assert!(
            serial_elapsed.as_millis() >= 6_500,
            "serial baseline took {serial_elapsed:?}, expected near 7.5s"
        );
        // One failed pull is a value, not a cancellation: 49 siblings pulled
        // and the failure list is exactly the same in both runs.
        assert_eq!(serial_report.pull_failures, 1);
        assert_eq!(concurrent_report.pull_failures, 1);
        assert_eq!(concurrent_report.pulled, 49);
        assert_eq!(concurrent_report.failures, serial_report.failures);
        assert_eq!(concurrent_report.outcomes, serial_report.outcomes);
        // Byte-identical ledger records in the same order, including the
        // one data-pull record per successful experiment.
        assert_eq!(concurrent_records, serial_records);
        assert_eq!(concurrent_records.len(), serial_records.len());
    }

    #[tokio::test]
    async fn concurrent_reporting_produces_the_same_signed_ledger_as_serial() {
        const EXPERIMENTS: usize = 6;
        let mut runs: Vec<Vec<LedgerRecord>> = Vec::new();
        for concurrency in [1usize, 4usize] {
            let inner = Arc::new(RecordingOps::new());
            let mut l = ledger();
            seed_running_ledger(&inner, &mut l, EXPERIMENTS).await;
            let ops = Arc::new(DelayedOps::new(
                inner,
                std::time::Duration::from_millis(10),
                None,
            ));
            let runner = Runner::new(ops, policy(), FillFormat::Rewarded)
                .with_reporting_concurrency(concurrency);
            runner.run_daily(&mut l, NOW + 1).await.expect("daily");
            runs.push(l.records().into_iter().cloned().collect());
        }
        // The real signed chain (Ed25519 hashes included) is identical: the
        // bounded reads only change when the pull completes, never the order
        // in which the ledger records are appended.
        assert_eq!(runs[0], runs[1]);
    }

    #[tokio::test]
    async fn daily_pass_pulls_records_and_settles_a_complete_window() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));

        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{\"floor\":1000000}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");

        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(report.pulled, 1);
        assert_eq!(report.settled, 1);
        assert_eq!(
            report.outcomes,
            vec![("exp-1".to_string(), "kept".to_string())]
        );
        // A data-pull record precedes the signed result.
        let pulls = l
            .records()
            .iter()
            .filter(|r| match r {
                LedgerRecord::LiftRecord { payload, .. } => payload.contains("dataPull"),
                _ => false,
            })
            .count();
        assert_eq!(pulls, 1);
    }

    #[tokio::test]
    async fn daily_concludes_an_ended_admob_test_without_a_reporting_failure() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_platform(apex_core::PlatformId::Admob);
        ops.set_change_in_test_arm(true);
        ops.set_measures_variants(false);
        ops.script_start(vec![Ok("exp-1".into())]);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: Action::SetAdmobMediationFloor {
                    group_id: "g1".into(),
                    floor_micros: 1_200_000,
                },
                rationale: "raise the mediation group floor".into(),
                expected_effect_bp: 500,
                prev_value: "{\"mediationGroupId\":\"g1\",\"ecpm\":1000000}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        ops.put_conclusion(
            "exp-1",
            UnmeasuredConclusion {
                experiment_state: "ENDED".into(),
                variant_leader: Some("TREATMENT".into()),
                keep_treatment: true,
            },
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        // The 3 burn-in plus 7 analysis days are complete on day 11.
        let report = runner
            .run_daily(&mut l, NOW + 10 * 86_400_000)
            .await
            .expect("daily");
        assert_eq!(report.settled, 1);
        assert_eq!(report.pending, 0);
        assert_eq!(report.pulled, 0, "AdMob is never pulled");
        assert_eq!(report.pull_failures, 0);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(
            report.outcomes,
            vec![("exp-1".to_string(), "kept".to_string())]
        );
        let log = ops.call_log();
        assert!(
            !log.iter().any(|entry| entry.starts_with("days:")),
            "no experiment_days call for AdMob"
        );
        assert_eq!(
            log.iter()
                .filter(|entry| entry.starts_with("conclude:"))
                .count(),
            1
        );
        assert_eq!(
            log.iter()
                .filter(|entry| entry.starts_with("promote:"))
                .count(),
            0
        );

        // A second pass has nothing left to settle and stays exit-0.
        let second = runner
            .run_daily(&mut l, NOW + 11 * 86_400_000)
            .await
            .expect("second daily");
        assert_eq!(second.settled, 0);
        assert!(second.failures.is_empty());
        assert_eq!(
            ops.call_log()
                .iter()
                .filter(|entry| entry.starts_with("conclude:"))
                .count(),
            1,
            "the ended test is not stopped again"
        );
    }

    #[tokio::test]
    async fn the_floor_pin_verifies_inside_a_full_unit_config() {
        // N2: a MAX floor change pins the floor JSON, while the unit read
        // carries the full ad-unit config (frequency capping included). The
        // comparison scopes to the floor ECPM values, so a matching config
        // verifies, a moved floor is an edit, and a config without floors
        // cannot verify at all.
        let change = ChangeKind::CountryFloorSet {
            country: "ee".into(),
            floor_ecpm: 2_500_000,
        };
        let pinned = r#"[{"country":"ee","floorEcpm":1}]"#;
        let matching_config = r#"{"frequencyCappingSettings":[{"capType":"session","sessionLimit":5}],"countryFloors":[{"country":"ee","floorEcpm":1}]}"#;
        assert_eq!(
            changed_setting_agrees(&change, matching_config, pinned),
            Some(true),
            "a full config with the pinned floors is not a publisher edit"
        );
        let moved = r#"{"countryFloors":[{"country":"ee","floorEcpm":9}]}"#;
        assert_eq!(
            changed_setting_agrees(&change, moved, pinned),
            Some(false),
            "a moved floor is still an edit"
        );
        let reordered = r#"[{"floorEcpm":1,"country":"ee"}]"#;
        assert_eq!(
            changed_setting_agrees(&change, reordered, pinned),
            Some(true),
            "encoding differences are not edits"
        );
        assert_eq!(
            changed_setting_agrees(&change, "[]", pinned),
            Some(false),
            "emptied floors are still an edit of the pinned floors"
        );
        assert_eq!(
            changed_setting_agrees(&change, r#"{"frequencyCappingSettings":[]}"#, pinned),
            None,
            "a read without a floor list cannot verify the pin"
        );
        // The registered baseline of a cap test never compares against the
        // floors read: the check is unarmed for FrequencyCapSet entirely.
        let cap_change = ChangeKind::FrequencyCapSet {
            ad_unit_id: "u1".into(),
            cap_type: "session".into(),
            countries: Vec::new(),
            session_limit: Some(4),
            day_limit: None,
            minute_frequency: None,
        };
        assert_eq!(
            changed_setting_agrees(
                &cap_change,
                "[]",
                r#"{"adLoadBaseline":{"sessionLimit":5}}"#
            ),
            None,
            "a caps baseline has no floors to compare against"
        );
    }

    #[tokio::test]
    async fn an_ad_load_test_is_not_voided_by_the_floor_read() {
        // N2: every ad-load test used to void because the unit read (floors)
        // was compared with the registered ad-load baseline. The check is now
        // unarmed for cap changes: the first pass settles pending instead of
        // voiding.
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.set_supports_settings_read(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        // The unit read exists and carries floors: with the old whole-document
        // comparison this mismatch voided the test on the first pass.
        ops.put_current_settings("u1", "[]");
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: Action::SetMaxFrequencyCap {
                    ad_unit_id: "u1".into(),
                    cap_type: crate::model::MaxCapType::Session,
                    countries: Vec::new(),
                    session_limit: Some(4),
                    day_limit: None,
                    minute_frequency: None,
                },
                rationale: "cap the ad load".into(),
                expected_effect_bp: 500,
                prev_value:
                    r#"{"adLoadBaseline":{"sessionLimit":5,"dayLimit":20,"minuteFrequency":60}}"#
                        .into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert!(
            !report.outcomes.iter().any(|(_, status)| status == "voided"),
            "an ad-load test never voids on the floors read: {:?}",
            report.outcomes
        );
        let deprecated = ops
            .call_log()
            .iter()
            .filter(|entry| entry.starts_with("deprecate:"))
            .count();
        assert_eq!(deprecated, 0, "the check is unarmed, nothing is stopped");
    }

    #[tokio::test]
    async fn a_levelplay_capping_test_verifies_the_variant_group_not_the_waterfall() {
        // N2: a capping change's scope is the variant group, which is not a
        // waterfall entry. The group document verifies the pin; a missing
        // entry reads as Unknown (held pending), never as a verified edit.
        let ops = Arc::new(RecordingOps::new());
        ops.set_platform(PlatformId::LevelPlay);
        ops.set_change_in_test_arm(true);
        ops.set_supports_settings_read(true);
        ops.script_start(vec![
            Ok("gB::aged-capping".into()),
            Ok("gB::aged-capping-2".into()),
        ]);
        ops.put_experiment(
            "gB::aged-capping",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("gB::aged-capping", constant_days(4_000_000, 4_400_000));
        ops.put_experiment(
            "gB::aged-capping-2",
            ExperimentResult {
                experiment_id: "gB::aged-capping-2".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("gB::aged-capping-2", constant_days(4_000_000, 4_400_000));
        ops.put_current_settings("gB", r#"{"groupId":"gB","abTest":"B","sessionLimit":5}"#);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: Action::SetLevelplayGroupCapping {
                    group_id: "gB".into(),
                    session_limit: Some(4),
                    interstitial_limit: None,
                    day_limit: None,
                },
                rationale: "cap group B".into(),
                expected_effect_bp: 500,
                prev_value: r#"{"group":"gB"}"#.into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert!(
            !report.outcomes.iter().any(|(_, status)| status == "voided"),
            "the intact variant group verifies: {:?}",
            report.outcomes
        );
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        // The group document is gone from the read: a failure to look, held
        // pending by the retry (LL), never a VerifiedChanged void.
        ops.clear_current_settings("gB");
        let mut l2 = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l2,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: Action::SetLevelplayGroupCapping {
                    group_id: "gB".into(),
                    session_limit: Some(4),
                    interstitial_limit: None,
                    day_limit: None,
                },
                rationale: "cap group B".into(),
                expected_effect_bp: 500,
                prev_value: r#"{"group":"gB"}"#.into(),
            }],
            NOW,
        )
        .await
        .expect("cycle 2");
        let runner2 = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner2.run_daily(&mut l2, NOW + 1).await.expect("daily 2");
        assert_eq!(
            report.pending, 1,
            "a failed capping lookup is Unknown, retried tomorrow"
        );
        assert!(
            !report.outcomes.iter().any(|(_, status)| status == "voided"),
            "{:?}",
            report.outcomes
        );
        let deprecated = ops
            .call_log()
            .iter()
            .filter(|entry| entry.starts_with("deprecate:"))
            .count();
        assert_eq!(deprecated, 0);
    }

    #[tokio::test]
    async fn a_publisher_edit_voids_the_test() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.set_supports_settings_read(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        // The scope's settings no longer match the pre-registered value.
        ops.put_current_settings("u1", "[{\"country\":\"ee\",\"floorEcpm\":9}]");
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "[{\"country\":\"ee\",\"floorEcpm\":1}]".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(
            report.outcomes,
            vec![("exp-1".to_string(), "voided".to_string())]
        );
        let deprecated = ops
            .call_log()
            .iter()
            .filter(|entry| entry.starts_with("deprecate:"))
            .count();
        assert_eq!(deprecated, 1, "a publisher edit deprecates the test");
    }

    #[tokio::test]
    async fn a_failed_settings_read_retries_two_passes_then_voids() {
        // The adapter declares the settings capability, so the edit check is
        // armed — but the read fails (no scripted settings for the unit). LL:
        // one failed read holds the test pending instead of voiding it; only
        // the third consecutive failed daily pass escalates to Unknown.
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.set_supports_settings_read(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "[{\"country\":\"ee\",\"floorEcpm\":1}]".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        // First failed read: the pass records the failure and stays pending.
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily 1");
        assert_eq!(report.pending, 1, "a single failed read is not a void");
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(settings_read_failures(&l, "exp-1"), 1);
        assert!(
            ops.call_log()
                .iter()
                .all(|entry| !entry.starts_with("deprecate:")),
            "no test is stopped on the first failed read"
        );
        // Second consecutive failed pass: still pending, still running.
        let report = runner
            .run_daily(&mut l, NOW + 1 + 86_400_000)
            .await
            .expect("daily 2");
        assert_eq!(report.pending, 1);
        assert_eq!(settings_read_failures(&l, "exp-1"), 2);
        // Third consecutive failed pass: the check escalates to Unknown and
        // the settlement voids.
        let report = runner
            .run_daily(&mut l, NOW + 1 + 2 * 86_400_000)
            .await
            .expect("daily 3");
        assert_eq!(
            report.outcomes,
            vec![("exp-1".to_string(), "voided".to_string())]
        );
        assert_eq!(settings_read_failures(&l, "exp-1"), 3);
        let deprecated = ops
            .call_log()
            .iter()
            .filter(|entry| entry.starts_with("deprecate:"))
            .count();
        assert_eq!(deprecated, 1, "an unverifiable edit check voids the test");
    }

    #[tokio::test]
    async fn a_successful_settings_read_resets_the_retry_streak() {
        // Two failures void only when three passes fail in a row: a pass that
        // reads fine settles normally, and the streak never carries into a
        // later failure.
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.set_supports_settings_read(true);
        ops.script_start(vec![Ok("exp-1".into()), Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        ops.put_current_settings("u1", "[{\"country\":\"ee\",\"floorEcpm\":1}]");
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "[{\"country\":\"ee\",\"floorEcpm\":1}]".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        // The read fails on the first pass: pending, one failure on record.
        ops.clear_current_settings("u1");
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily 1");
        assert_eq!(report.pending, 1);
        assert_eq!(settings_read_failures(&l, "exp-1"), 1);
        // The provider recovers: the pass reads the pinned value and settles
        // the test on its own evidence instead of voiding it.
        ops.put_current_settings("u1", "[{\"country\":\"ee\",\"floorEcpm\":1}]");
        let report = runner
            .run_daily(&mut l, NOW + 1 + 86_400_000)
            .await
            .expect("daily 2");
        assert!(
            !report.outcomes.iter().any(|(_, status)| status == "voided"),
            "a recovered read never voids: {:?}",
            report.outcomes
        );
        assert_eq!(settings_read_failures(&l, "exp-1"), 1);
        let deprecated = ops
            .call_log()
            .iter()
            .filter(|entry| entry.starts_with("deprecate:"))
            .count();
        assert_eq!(deprecated, 0, "a recovered read never stops the test");
    }

    /// The `settingsReadFailed` records one experiment carries in the ledger.
    fn settings_read_failures(l: &InMemoryLedger, experiment_id: &str) -> usize {
        l.records()
            .iter()
            .filter(|record| matches!(record, LedgerRecord::SettingsReadFailed { experiment_id: id, .. } if id == experiment_id))
            .count()
    }

    #[tokio::test]
    async fn an_unarmed_settings_capability_never_voids_on_a_missing_read() {
        // Without the capability flag the check is not armed: the simulator's
        // missing settings read is a capability question, not a read failure,
        // and the test settles on its own evidence.
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "[{\"country\":\"ee\",\"floorEcpm\":1}]".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(
            report.outcomes,
            vec![("exp-1".to_string(), "kept".to_string())]
        );
    }

    #[tokio::test]
    async fn an_aa_run_starts_and_is_recorded() {
        let ops = Arc::new(RecordingOps::new());
        let mut l = ledger();
        let spec = apex_core::ExperimentSpec {
            name: "apex-aa-u1".into(),
            instance_id: String::new(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "A/A calibration".into(),
            test_group_allocation: 50,
        };
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let id = runner
            .start_aa(&mut l, apex_core::PlatformId::AppLovinMax, &spec, NOW)
            .await
            .expect("aa");
        assert_eq!(id, "apex-aa-u1");
        assert!(
            l.records().iter().any(|r| matches!(
                r,
                LedgerRecord::ExperimentStarted { experiment_id, .. } if experiment_id == "apex-aa-u1"
            )),
            "the A/A start is recorded"
        );
    }

    #[tokio::test]
    async fn aa_calibration_aligns_to_the_start_day_like_production() {
        use apex_lift::calibration::AaConclusion;
        let ops = Arc::new(RecordingOps::new());
        let mut control = Vec::new();
        let mut test = Vec::new();
        for i in 1..=13 {
            let day = format!("2026-10-{i:02}");
            control.push(arm_day(&day, 1_000_000));
            test.push(arm_day(&day, if i % 2 == 0 { 1_010_000 } else { 990_000 }));
        }
        ops.put_experiment_days("apex-aa-u1", ExperimentDays { control, test });
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        // The start day itself is A/A traffic: 10-04..10-13 is ten days,
        // three burn-in days and seven accepted analysis days. Dropping the
        // boundary day would leave only six accepted days.
        let conclusion = runner
            .aa_conclusion_from("apex-aa-u1", 0.5, Some("2026-10-04"), Some("apex-aa-u1"))
            .await
            .expect("conclusion");
        assert!(
            matches!(conclusion, AaConclusion::Calibrated { .. }),
            "{conclusion:?}"
        );
    }

    #[tokio::test]
    async fn a_short_aa_run_is_audit_only() {
        use apex_lift::calibration::AaConclusion;
        let ops = Arc::new(RecordingOps::new());
        ops.put_experiment_days(
            "apex-aa-u1",
            ExperimentDays {
                control: (0..3)
                    .map(|i| arm_day(&format!("2026-10-0{}", i + 1), 1_000_000))
                    .collect(),
                test: (0..3)
                    .map(|i| arm_day(&format!("2026-10-0{}", i + 1), 1_000_000))
                    .collect(),
            },
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let conclusion = runner
            .aa_conclusion("apex-aa-u1", 0.5)
            .await
            .expect("conclusion");
        assert!(
            matches!(conclusion, AaConclusion::AuditOnly { .. }),
            "{conclusion:?}"
        );
    }

    /// A transport that answers the Mollie checkout-payment call and records it.
    #[derive(Default)]
    struct MollieFake {
        seen: std::sync::Mutex<Vec<apex_connectors::transport::HttpRequest>>,
        next: std::sync::atomic::AtomicU64,
    }

    #[async_trait]
    impl apex_connectors::transport::Transport for MollieFake {
        async fn request(
            &self,
            req: apex_connectors::transport::HttpRequest,
        ) -> Result<
            apex_connectors::transport::HttpResponse,
            apex_connectors::transport::TransportError,
        > {
            let is_get = req.method == apex_connectors::transport::HttpMethod::Get;
            let id = if is_get {
                req.url.rsplit('/').next().unwrap_or("tr_1").to_string()
            } else {
                format!(
                    "tr_{}",
                    self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
                )
            };
            self.seen.lock().unwrap().push(req);
            Ok(apex_connectors::transport::HttpResponse {
                status: 200,
                headers: vec![],
                body: format!(
                    r#"{{"resource":"payment","id":"{id}","status":"open","amount":{{"currency":"USD","value":"1.00"}},"metadata":{{"reference":"apex-test"}},"_links":{{"checkout":{{"href":"https://pay.mollie.com/{id}"}}}}}}"#
                )
                .into_bytes(),
            })
        }
    }

    /// Script the holdout player feed a kept instance's monthly pass reads:
    /// three exposed players at +4% and three holdout players, across the
    /// trailing 28 days of the billing month.
    fn script_holdout_players(ops: &Arc<RecordingOps>, ledger: &InMemoryLedger) {
        script_holdout_lift(ops, ledger, 104_008, 100_000);
    }

    /// Record an A/A calibration run for the fixture publisher ("pub1") on
    /// MAX, valid from `at` for the standard validity window. A3-V4 E2: the
    /// qualification is an audit record only — monthly holdout billing no
    /// longer requires it.
    fn grant_aa(ledger: &mut InMemoryLedger, at: i64) {
        grant_aa_for(ledger, at, "pub1", apex_core::PlatformId::AppLovinMax);
    }

    /// The same, for an explicit publisher id.
    fn grant_aa_for(
        ledger: &mut InMemoryLedger,
        at: i64,
        publisher: &str,
        platform: apex_core::PlatformId,
    ) {
        crate::aa_qualification::record(ledger, publisher, platform, 0.5, 1.0, 0.0, at)
            .expect("a valid A/A qualification");
    }

    /// The complete 28-day trailing window ending `month_end`, the only
    /// window a billable holdout reading may cover.
    fn holdout_window_days(month_end: &str) -> Vec<String> {
        let end = crate::calendar::day_number(month_end).expect("month end day");
        (0..apex_lift::holdout::MEASUREMENT_DAYS)
            .map(|offset| crate::calendar::format_day(end - 27 + offset as i64))
            .collect()
    }

    #[test]
    fn a_certificate_cohort_must_match_the_roster() {
        let eligible = vec![crate::ops::EligibleUserEvidence {
            user_id: "u1".to_string(),
            exposed: true,
            prior_micros: 500,
            envelope_micros: 1_200,
        }];
        let observed = std::collections::BTreeMap::new();
        assert!(certified_actual_gain(
            &eligible,
            &observed,
            Some(apex_lift::holdout::AssignedRoster {
                exposed: 2,
                control: 1
            }),
            0.01
        )
        .is_none());
    }

    #[test]
    fn a_certificate_with_the_right_size_but_wrong_arm_counts_is_refused() {
        let eligible = vec![
            crate::ops::EligibleUserEvidence {
                user_id: "u1".to_string(),
                exposed: true,
                prior_micros: 500,
                envelope_micros: 1_200,
            },
            crate::ops::EligibleUserEvidence {
                user_id: "u2".to_string(),
                exposed: false,
                prior_micros: 500,
                envelope_micros: 1_200,
            },
        ];
        let mut observed = std::collections::BTreeMap::new();
        observed.insert("u1".to_string(), (true, 1_000));
        observed.insert("u2".to_string(), (false, 1_000));
        assert!(certified_actual_gain(
            &eligible,
            &observed,
            Some(apex_lift::holdout::AssignedRoster {
                exposed: 0,
                control: 2
            }),
            0.01
        )
        .is_none());
    }

    #[test]
    fn a_missing_control_outcome_is_imputed_at_the_upper_envelope() {
        let roster = apex_lift::holdout::AssignedRoster {
            exposed: 270,
            control: 30,
        };
        let mut eligible = Vec::new();
        for index in 0..270 {
            eligible.push(crate::ops::EligibleUserEvidence {
                user_id: format!("e{index}"),
                exposed: true,
                prior_micros: 0,
                envelope_micros: 1_000,
            });
        }
        for index in 0..30 {
            eligible.push(crate::ops::EligibleUserEvidence {
                user_id: format!("c{index}"),
                exposed: false,
                prior_micros: 0,
                envelope_micros: 1_000,
            });
        }
        let mut observed = std::collections::BTreeMap::new();
        for index in 0..270 {
            observed.insert(format!("e{index}"), (true, 1_000));
        }
        // c0 is missing; c1.. are observed at zero.
        for index in 1..30 {
            observed.insert(format!("c{index}"), (false, 0));
        }
        let missing =
            certified_actual_gain(&eligible, &observed, Some(roster), 0.01).expect("certificate");
        // Observing the missing control at zero must yield a strictly more
        // generous bound; envelope imputation lowers the absolute gain.
        let mut completed = observed.clone();
        completed.insert("c0".to_string(), (false, 0));
        let zero =
            certified_actual_gain(&eligible, &completed, Some(roster), 0.01).expect("certificate");
        assert!(
            missing.0 < zero.0,
            "missing control must be charged at its envelope: {missing:?} vs {zero:?}"
        );
    }

    #[test]
    fn the_certificate_alpha_budget_divides_across_simultaneous_cohorts() {
        assert!((certificate_family_alpha(1) - 0.025).abs() < 1e-15);
        assert!((certificate_family_alpha(2) - 0.0125).abs() < 1e-15);
        assert!((certificate_family_alpha(4) - 0.00625).abs() < 1e-15);
        assert!((certificate_family_alpha(0) - 0.025).abs() < 1e-15);
    }

    #[test]
    fn a_contradictory_envelope_arm_yields_no_certificate() {
        let mut observed = std::collections::BTreeMap::new();
        observed.insert("u1".to_string(), (true, 1_000));
        let eligible = vec![crate::ops::EligibleUserEvidence {
            user_id: "u1".to_string(),
            exposed: false,
            prior_micros: 500,
            envelope_micros: 1_200,
        }];
        assert!(certified_actual_gain(
            &eligible,
            &observed,
            Some(apex_lift::holdout::AssignedRoster {
                exposed: 0,
                control: 1
            }),
            0.01
        )
        .is_none());
    }

    /// Script the holdout feed with chosen exposed/control revenue per player.
    fn script_holdout_lift(
        ops: &Arc<RecordingOps>,
        ledger: &InMemoryLedger,
        exposed_revenue: i64,
        control_revenue: i64,
    ) {
        let Ok(kept) = kept_changes(ledger) else {
            return;
        };
        let Some((_, _, change)) = kept.first() else {
            return;
        };
        let id = apex_lift::holdout::holdout_experiment_id(&change.change_id);
        let mut rows = Vec::new();
        for day in holdout_window_days("2023-11-30") {
            for player in 0..270 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("exposed-{player}")),
                    idfv: None,
                    idfa: None,
                    test: false,
                    revenue_micros: exposed_revenue,
                });
                if player >= 30 {
                    continue;
                }
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("holdout-{player}")),
                    idfv: None,
                    idfa: None,
                    test: true,
                    revenue_micros: control_revenue,
                });
            }
        }
        ops.put_experiment_players(
            &id,
            crate::ops::PlayerEvidence {
                rows,
                pretest: std::collections::BTreeMap::new(),
                measurement_scope: Some("u1".to_string()),
                roster: Some(apex_lift::holdout::AssignedRoster {
                    exposed: 270,
                    control: 30,
                }),
                eligible: None,
                certified_gain_lower: Some(900_000_000),
                certified_delta_lower: Some(0.04),
                assigned_test_control: None,
            },
        );
    }

    async fn kept_ledger(ops: &Arc<RecordingOps>, at: i64) -> InMemoryLedger {
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            at,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, at + 1).await.expect("daily");
        script_holdout_players(ops, &l);
        l
    }

    fn kept_ops() -> Arc<RecordingOps> {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        ops.put_scope_revenue("u1", 1_000_000_000);
        ops
    }

    /// The test VIES stub: every shape-valid customer number is confirmed.
    #[derive(Debug)]
    struct ValidVies;

    impl apex_billing::ViesClient for ValidVies {
        fn check_vat_number(
            &self,
            _country: &str,
            number: &str,
        ) -> Result<apex_billing::ViesCheck, apex_billing::ViesError> {
            Ok(apex_billing::ViesCheck::Valid {
                evidence: format!("VIES confirmation: {number} is valid (test stub)"),
                vat_number: number.to_string(),
            })
        }
    }

    /// The seller profile the monthly tests bill against (domestic EE).
    fn seller_profile() -> apex_billing::BillingProfile {
        apex_billing::BillingProfile {
            legal_name: "Apex Media OÜ".into(),
            address_lines: vec!["Sepapaja tn 6".into()],
            country: "EE".into(),
            vat_number: "EE123456789".into(),
            vat_rate_bp: 2_000,
        }
    }

    /// A per-test connections root (customer profiles) and register file.
    fn billing_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "apex-monthly-billing-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_customer(
        dir: &std::path::Path,
        publisher: &str,
        customer: &apex_billing::CustomerProfile,
    ) {
        std::fs::write(
            dir.join(format!("{publisher}.billing.json")),
            serde_json::to_string_pretty(customer).unwrap(),
        )
        .unwrap();
    }

    fn customer(country: &str, vat: Option<&str>) -> apex_billing::CustomerProfile {
        apex_billing::CustomerProfile {
            legal_name: "Buyer OÜ".into(),
            address_lines: vec!["Telliskivi 1".into()],
            country: country.into(),
            vat_number: vat.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn the_monthly_pass_collects_through_mollie_and_bills_only_lift() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("mollie");
        write_customer(&dir, "pub1", &customer("EE", Some("EE987654321")));
        let mollie = Arc::new(MollieFake::default());
        let client = Arc::new(
            apex_billing::MollieClient::new(
                "key",
                "https://api.mollie.test",
                "https://apex.test/v1/mollie/webhook",
                "https://apex.test/watch/return",
            )
            .expect("client"),
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_vat_rate(2_000)
            .with_payment_provider(client, mollie.clone())
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert!(report.collected, "the Mollie client collects");
        // A3: the only invoice line is the measured lift fee. Monitoring is
        // free and has no plan line.
        assert_eq!(report.payment_links.len(), 1, "{:?}", report.payment_links);
        assert!(
            report
                .payment_links
                .iter()
                .all(|url| url.starts_with("https://pay.mollie.com/tr_")),
            "{:?}",
            report.payment_links
        );
        assert!(
            report.card_charges.is_empty(),
            "no stored method is recorded"
        );
        assert!(
            !dir.join("pub1.monitoring.json").exists(),
            "free monitoring has no plan state"
        );
        assert!(report.invoice_total_micros > 0);
        // The invoice was issued through the register and rendered as the
        // lawful domestic document.
        let number = report.invoice_number.expect("issued");
        assert!(number > 0);
        let rendered = report.rendered_invoice.expect("rendered");
        assert!(
            rendered.starts_with(&format!("VAT invoice No. {number} for 2023-11\n")),
            "{rendered}"
        );
        assert!(rendered.contains("VAT (2000 bp): "), "{rendered}");
        assert!(report.invoice_refusal.is_none());
        let calls = mollie.seen.lock().unwrap();
        // One first payment (its customer, then the payment), plus the
        // invoice-level payment link the invoice email delivers (E7).
        assert_eq!(calls.len(), 3, "customer + first payment + payment link");
        assert!(
            calls.iter().any(|call| call.url.ends_with("/v2/customers")),
            "the first payment creates the mandate's customer"
        );
        assert!(
            calls.iter().any(|call| call.url.ends_with("/v2/payments")),
            "{}",
            calls[0].url
        );
        assert!(
            calls
                .iter()
                .any(|call| call.url.ends_with("/v2/payment-links")),
            "the invoice total gets a durable payment link"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Mollie transport whose every endpoint answers a distinct, labelled
    /// shopper URL, so a test can assert exactly which URL the customer was
    /// sent.
    #[derive(Debug, Default)]
    struct LabelledMollie;

    #[async_trait]
    impl apex_connectors::transport::Transport for LabelledMollie {
        async fn request(
            &self,
            req: apex_connectors::transport::HttpRequest,
        ) -> Result<
            apex_connectors::transport::HttpResponse,
            apex_connectors::transport::TransportError,
        > {
            let (id, href) = if req.url.ends_with("/v2/customers") {
                ("cst_first".to_string(), String::new())
            } else if req.url.ends_with("/v2/payment-links") {
                (
                    "pl_total".to_string(),
                    "https://pay.mollie.test/link".to_string(),
                )
            } else {
                (
                    "tr_first".to_string(),
                    "https://checkout.mollie.test/first".to_string(),
                )
            };
            let sequence = match req.body.as_ref() {
                Some(apex_connectors::transport::HttpBody::Json(value)) => value
                    .get("sequenceType")
                    .and_then(|item| item.as_str())
                    .unwrap_or("oneoff")
                    .to_string(),
                _ => "oneoff".to_string(),
            };
            Ok(apex_connectors::transport::HttpResponse {
                status: 200,
                headers: vec![],
                body: format!(
                    r#"{{"resource":"payment","id":"{id}","status":"open","sequenceType":"{sequence}","amount":{{"currency":"USD","value":"1.00"}},"metadata":{{"reference":"apex-test"}},"_links":{{"checkout":{{"href":"{href}"}}}}}}"#
                )
                .into_bytes(),
            })
        }
    }

    #[tokio::test]
    async fn the_invoice_email_sends_the_first_payment_url_that_stores_the_mandate() {
        // E7: the mandate exists only after the first payment completes, so
        // the invoice email carries the first payment's checkout URL — not
        // the durable invoice link, which pays without storing a mandate.
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("first-payment-email");
        write_customer(&dir, "pub1", &customer("EE", Some("EE987654321")));
        let sink = Arc::new(apex_notify::sink::FileSink::new(dir.join("outbox.jsonl")));
        let log =
            apex_notify::state::DeliveryLog::open(Some(&dir.join("delivery.jsonl"))).expect("log");
        let notifier = Arc::new(apex_notify::notifier::Notifier::new(
            sink,
            log,
            Arc::new(apex_metrics::Registry::new()),
        ));
        let book =
            apex_notify::contacts::ContactBook::from_json(r#"{"pub1": ["ops@example.com"]}"#)
                .expect("book");
        let notify = Arc::new(crate::notify::NotifyPlan::new(
            notifier,
            book,
            "https://console.apexmediation.ee",
        ));
        let client = Arc::new(
            apex_billing::MollieClient::new(
                "key",
                "https://api.mollie.test",
                "https://apex.test/v1/mollie/webhook",
                "https://apex.test/watch/return",
            )
            .expect("client"),
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_vat_rate(2_000)
            .with_payment_provider(client, Arc::new(LabelledMollie))
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"))
            .with_notifications(notify);
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert!(report.collected);
        let raw = std::fs::read_to_string(dir.join("outbox.jsonl")).expect("outbox");
        let rows: Vec<serde_json::Value> = raw
            .lines()
            .map(|line| serde_json::from_str(line).expect("row"))
            .collect();
        let invoice_mail = rows
            .iter()
            .find(|row| row["template_class"] == "invoice_ready")
            .expect("the invoice email went out");
        let text = invoice_mail["text"].as_str().expect("text");
        assert!(
            text.contains("https://checkout.mollie.test/first"),
            "the first-payment checkout URL is sent: {text}"
        );
        assert!(
            !text.contains("https://pay.mollie.test/link"),
            "the invoice-total link must not replace the first-payment URL: {text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_invoice_issued_before_collection_is_collected_on_the_next_pass() {
        // Pass one records the invoice with no Mollie client (the crash after
        // issuance, before submission). Pass two must find the issued invoice
        // and continue it through collection instead of returning early.
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("issued-recovery");
        write_customer(&dir, "pub1", &customer("EE", Some("EE987654321")));
        let first = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_vat_rate(2_000)
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let first_report = first
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("issue");
        assert!(!first_report.collected);
        let number = first_report.invoice_number.expect("issued");

        let mollie = Arc::new(MollieFake::default());
        let client = Arc::new(
            apex_billing::MollieClient::new(
                "key",
                "https://api.mollie.test",
                "https://apex.test/v1/mollie/webhook",
                "https://apex.test/watch/return",
            )
            .expect("client"),
        );
        let second = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_vat_rate(2_000)
            .with_payment_provider(client, mollie.clone())
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let second_report = second
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 3)
            .await
            .expect("recover and collect");
        assert!(second_report.collected, "the existing invoice collected");
        assert_eq!(second_report.invoice_number, Some(number));
        assert_eq!(second_report.payment_links.len(), 1);
        assert_eq!(
            mollie
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|req| req.method == apex_connectors::transport::HttpMethod::Post)
                .count(),
            3,
            "one provider payment for the lift line, plus its customer and the invoice link"
        );

        // A third pass finds the durable intents and fetches instead of
        // creating another charge.
        let third = Runner::new(ops, policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_vat_rate(2_000)
            .with_payment_provider(
                Arc::new(
                    apex_billing::MollieClient::new(
                        "key",
                        "https://api.mollie.test",
                        "https://apex.test/v1/mollie/webhook",
                        "https://apex.test/watch/return",
                    )
                    .expect("client"),
                ),
                mollie.clone(),
            )
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let third_report = third
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 4)
            .await
            .expect("rerun");
        assert!(third_report.collected);
        assert_eq!(
            mollie
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|req| req.method == apex_connectors::transport::HttpMethod::Post)
                .count(),
            3,
            "the rerun must not create another provider payment or link"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("invoices.jsonl"))
                .expect("register")
                .lines()
                .count(),
            1,
            "monthly reruns never issue a duplicate invoice"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_month_without_a_managed_plan_or_fees_issues_nothing() {
        // Free monitoring never creates a billable position. Without lift
        // fees and without a recorded Managed plan there is no invoice.
        let ops = kept_ops();
        let mut l = ledger();
        let dir = billing_dir("free-only");
        write_customer(&dir, "pub1", &customer("EE", Some("EE987654321")));
        let runner = Runner::new(ops, policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_vat_rate(2_000)
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert!(report.fees.is_empty(), "no lift fees this month");
        assert!(report.invoice_number.is_none(), "nothing to invoice");
        assert!(report.rendered_invoice.is_none());
        assert_eq!(report.invoice_total_micros, 0);
        assert!(
            !dir.join("pub1.monitoring.json").exists(),
            "monitoring has no plan state and no price"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_recorded_managed_plan_bills_its_own_line_and_stops_after_cancel() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("managed");
        write_customer(&dir, "pub1", &customer("EE", Some("EE987654321")));
        // The sales order recorded the managed plan at the top price point.
        let state =
            apex_billing::ManagedPlan::default().activate(2_000_000 * 1_000_000, "2023-11-01");
        assert_eq!(state.monthly_fee_micros, 5_000_000_000);
        state.save(&dir.join("pub1.managed.json")).unwrap();
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_vat_rate(2_000)
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        let rendered = report.rendered_invoice.expect("rendered");
        assert!(
            rendered.contains("managed:2023-11: net 5000000000 micros"),
            "the managed line is its own line: {rendered}"
        );

        // Cancelled: the current period only. The stop period line records
        // the cancellation; the next period has no managed line.
        let mut cancelled = apex_billing::PlanState::load(&dir.join("pub1.managed.json"))
            .unwrap()
            .unwrap();
        cancelled.cancel("2023-11");
        cancelled.save(&dir.join("pub1.managed.json")).unwrap();
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-12", "2023-12-01", 31, NOW + 40)
            .await
            .expect("monthly");
        if let Some(rendered) = report.rendered_invoice {
            assert!(
                !rendered.contains("managed:2023-12"),
                "the next period is zero for the managed plan: {rendered}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_monthly_pass_without_mollie_records_but_does_not_collect() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("no-mollie");
        write_customer(&dir, "pub1", &customer("EE", None));
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert!(!report.collected);
        assert!(report.payment_links.is_empty());
        assert!(
            report.invoice_total_micros > 0,
            "the invoice is still computed"
        );
        assert!(report.invoice_number.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_monthly_pass_requires_a_publisher() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let err = runner
            .run_monthly(&mut l, "", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect_err("publisher required");
        assert!(matches!(err, PlanError::Validation(_)), "{err}");
    }

    /// Serializes the few runner tests that touch process billing env vars.
    /// The guard is deliberately held across the await: the pass under test
    /// must see the process env exactly as the deployment would, and this is
    /// the only env-mutating test in the binary.
    static BILLING_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn a_missing_billing_profile_refuses_the_whole_monthly_pass() {
        let _guard = BILLING_ENV_LOCK.lock().unwrap();
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let saved_profile = std::env::var(apex_billing::profile::PROFILE_ENV).ok();
        let saved_rate = std::env::var(apex_billing::profile::VAT_RATE_ENV).ok();
        std::env::remove_var(apex_billing::profile::PROFILE_ENV);
        std::env::remove_var(apex_billing::profile::VAT_RATE_ENV);

        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let err = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect_err("no billing profile refuses the pass");
        assert!(
            matches!(
                err,
                PlanError::Billing(apex_billing::BillingError::MissingProfileField {
                    party: "seller",
                    field: "APEX_BILLING_PROFILE",
                })
            ),
            "the refusal is the typed missing-field error: {err}"
        );
        assert!(
            l.records().iter().all(|record| !matches!(
                record,
                LedgerRecord::LiftRecord { payload, .. } if payload.contains("monthlyBilling")
            )),
            "a refused pass appends no billing record"
        );

        match saved_profile {
            Some(value) => std::env::set_var(apex_billing::profile::PROFILE_ENV, value),
            None => std::env::remove_var(apex_billing::profile::PROFILE_ENV),
        }
        match saved_rate {
            Some(value) => std::env::set_var(apex_billing::profile::VAT_RATE_ENV, value),
            None => std::env::remove_var(apex_billing::profile::VAT_RATE_ENV),
        }
    }

    #[tokio::test]
    async fn a_missing_customer_profile_refuses_the_invoice_and_consumes_no_number() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("missing-customer");
        // No `<publisher>.billing.json` was written: the invoice must refuse.
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("the pass completes with the refusal recorded");
        assert_eq!(report.fees.len(), 1, "the fee itself was still computed");
        assert_eq!(
            report.invoice_refusal.as_deref(),
            Some("the invoice carries no customer profile, so no VAT treatment can be stated"),
        );
        assert!(report.invoice_number.is_none());
        assert!(report.rendered_invoice.is_none());
        assert!(!report.collected, "a refused invoice is never collected");
        assert!(report.payment_links.is_empty());
        // No register number was consumed: the register is empty (or absent).
        let register = apex_billing::InvoiceRegister::open(&dir.join("invoices.jsonl"))
            .expect("register opens");
        assert_eq!(register.highest().expect("highest"), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The byte-exact render for the two-publisher monthly run below, in the
    /// billing crate's own golden style.
    const GOLDEN_MONTHLY_DOMESTIC: &str = "\
VAT invoice No. {} for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
EE
VAT number: EE123456789
Customer:
Nurme OÜ
Narva mnt 5
10117 Tallinn
EE
VAT number: EE987654321
Line items:
  #11 max-experiment:autopilot-floor-u1-ee-2-5:u1: net 5520000 micros, VAT 1100000 micros, gross 6620000 micros
Net total: 5520000 micros
VAT (2000 bp): 1100000 micros
Gross total: 6620000 micros
";
    const GOLDEN_MONTHLY_REVERSE_CHARGE: &str = "\
VAT invoice No. {} for 2023-11
Seller:
Apex Media OÜ
Sepapaja tn 6
EE
VAT number: EE123456789
Customer:
Beispiel GmbH
Kurfürstendamm 21
10719 Berlin
DE
VAT number: DE123456789
Line items:
  #11 max-experiment:autopilot-floor-u1-ee-2-5:u1: net 5520000 micros, VAT 0 micros, gross 5520000 micros
Net total: 5520000 micros
VAT: reverse charge
VIES: VIES confirmation: DE123456789 is valid (test stub)
Gross total: 5520000 micros
";

    #[tokio::test]
    async fn two_publishers_get_gapless_numbers_and_byte_exact_vat_documents() {
        let ops1 = kept_ops();
        let mut first_ledger = kept_ledger(&ops1, NOW).await;
        let ops2 = kept_ops();
        let mut second_ledger = kept_ledger(&ops2, NOW).await;
        let dir = billing_dir("two-publishers");
        write_customer(
            &dir,
            "northstar",
            &apex_billing::CustomerProfile {
                legal_name: "Nurme OÜ".into(),
                address_lines: vec!["Narva mnt 5".into(), "10117 Tallinn".into()],
                country: "EE".into(),
                vat_number: Some("EE987654321".into()),
            },
        );
        write_customer(
            &dir,
            "aurora",
            &apex_billing::CustomerProfile {
                legal_name: "Beispiel GmbH".into(),
                address_lines: vec!["Kurfürstendamm 21".into(), "10719 Berlin".into()],
                country: "DE".into(),
                vat_number: Some("DE123456789".into()),
            },
        );
        let runner = Runner::new(ops1.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile())
            .with_vies(Arc::new(ValidVies))
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));

        grant_aa_for(
            &mut first_ledger,
            NOW + 1,
            "northstar",
            apex_core::PlatformId::AppLovinMax,
        );
        let first = runner
            .run_monthly(
                &mut first_ledger,
                "northstar",
                "2023-11",
                "2023-11-01",
                30,
                NOW + 2,
            )
            .await
            .expect("northstar monthly");
        grant_aa_for(
            &mut second_ledger,
            NOW + 1,
            "aurora",
            apex_core::PlatformId::AppLovinMax,
        );
        let second = runner
            .run_monthly(
                &mut second_ledger,
                "aurora",
                "2023-11",
                "2023-11-01",
                30,
                NOW + 2,
            )
            .await
            .expect("aurora monthly");
        let first_number = first.invoice_number.expect("issued");
        let second_number = second.invoice_number.expect("issued");
        assert_eq!(
            second_number,
            first_number + 1,
            "the register numbers are gapless and consecutive"
        );
        assert_eq!(
            first.rendered_invoice.as_deref(),
            Some(
                GOLDEN_MONTHLY_DOMESTIC
                    .replace("{}", &first_number.to_string())
                    .replace(
                        "max-experiment:autopilot-floor-u1-ee-2-5:u1",
                        &first.fees[0].0,
                    )
                    .as_str()
            ),
            "the domestic render is byte-exact"
        );
        assert_eq!(
            second.rendered_invoice.as_deref(),
            Some(
                GOLDEN_MONTHLY_REVERSE_CHARGE
                    .replace("{}", &second_number.to_string())
                    .replace(
                        "max-experiment:autopilot-floor-u1-ee-2-5:u1",
                        &second.fees[0].0,
                    )
                    .as_str()
            ),
            "the reverse-charge render is byte-exact"
        );
        // Both register lines landed in file order and the file carries the
        // whole gapless sequence.
        let raw = std::fs::read_to_string(dir.join("invoices.jsonl")).expect("register");
        let numbers: Vec<u64> = raw
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("line")["number"]
                    .as_u64()
                    .expect("number")
            })
            .collect();
        assert_eq!(numbers.len(), 2, "{raw}");
        assert_eq!(numbers[1], numbers[0] + 1);
        assert!(raw.contains("\"vatTreatment\":\"domestic\""), "{raw}");
        assert!(raw.contains("\"vatTreatment\":\"reverseCharge\""), "{raw}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_daily_pass_learns_and_persists_the_bucket_model() {
        let ops = kept_ops();
        let dir = std::env::temp_dir().join(format!("apex-priors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let priors_path = dir.join("priors.json");
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_priors(apex_lift::priors::LiftPriors::default(), &priors_path);
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        let saved = std::fs::read_to_string(&priors_path).expect("saved");
        assert!(saved.contains("floor"), "the bucket is learned: {saved}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn twenty_settled_experiments_clone_once_and_persist_priors_once() {
        const N: usize = 20;
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        let mut pass_policy = policy();
        pass_policy.max_actions_per_day = 32;
        let mut l = ledger();
        let countries = [
            "ee", "fi", "se", "de", "fr", "es", "it", "nl", "be", "at", "pl", "pt", "ie", "dk",
            "no", "cz", "gr", "ro", "hu", "bg",
        ];
        let mut proposals = Vec::new();
        for (i, country) in countries.iter().enumerate() {
            let action = Action::SetCountryFloor {
                ad_unit_id: "u1".into(),
                country: (*country).to_string(),
                floor_ecpm: 2_500_000 + i as i64,
            };
            proposals.push(crate::model::Proposal {
                action,
                rationale: format!("raise {country}"),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            });
        }
        run_cycle(
            ops.as_ref(),
            &mut l,
            &pass_policy,
            &[],
            proposals.clone(),
            NOW,
        )
        .await
        .expect("cycle");
        for proposal in &proposals {
            let name = crate::keys::experiment_name(&proposal.action);
            ops.put_experiment(
                &name,
                ExperimentResult {
                    experiment_id: name.clone(),
                    state: ExperimentState::Running,
                    results: vec![],
                    lift_percent_bp: None,
                    confidence: None,
                    winner_variant_id: None,
                },
            );
            ops.put_experiment_days(&name, constant_days(4_000_000, 4_400_000));
        }
        let dir = std::env::temp_dir().join(format!("apex-priors-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let priors_path = dir.join("priors.json");
        let runner = Runner::new(ops.clone(), pass_policy, FillFormat::Rewarded)
            .with_priors(apex_lift::priors::LiftPriors::default(), &priors_path);
        reset_prior_io();
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(report.settled, N as u32, "all twenty experiments settle");
        assert_eq!(prior_writes(), 1, "the pass writes the batched priors once");
        assert_eq!(prior_clones(), 1, "the pass clones the model once");
        let saved = std::fs::read_to_string(&priors_path).expect("saved");
        let model: apex_lift::priors::LiftPriors =
            serde_json::from_str(&saved).expect("saved priors parse");
        assert_eq!(
            model.total_observations(),
            N as u64,
            "every settled outcome reached the one persisted model"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_aa_run_is_not_swept_into_the_daily_settle_pass() {
        // An A/A run has no pre-registration; the daily pass must leave it
        // alone rather than mis-attribute it to another change's sequence.
        let ops = Arc::new(RecordingOps::new());
        let mut l = ledger();
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let spec = apex_core::ExperimentSpec {
            name: "apex-aa-u1".into(),
            instance_id: String::new(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "A/A calibration".into(),
            test_group_allocation: 50,
        };
        runner
            .start_aa(&mut l, apex_core::PlatformId::AppLovinMax, &spec, NOW)
            .await
            .expect("aa");
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(report.pulled, 0, "the A/A run is not pulled for settling");
        assert!(report.failures.is_empty());
    }

    #[tokio::test]
    async fn a_first_look_that_offers_an_extension_waits_for_its_week() {
        // A positive point lift with a wide interval offers an extension; with
        // only ten days of pulls the extended window is incomplete, so the
        // experiment waits rather than rolling back.
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        // Alternating strong/weak days: a positive mean with a wide spread.
        let days = ExperimentDays {
            control: (0..10)
                .map(|i| arm_day(&format!("2026-10-{:02}", i + 1), 4_000_000))
                .collect(),
            test: (0..10)
                .map(|i| {
                    let value = if i % 2 == 0 { 4_900_000 } else { 4_020_000 };
                    arm_day(&format!("2026-10-{:02}", i + 1), value)
                })
                .collect(),
        };
        ops.put_experiment_days("exp-1", days);
        let extension_policy = AutopilotPolicy {
            extension_allowed: true,
            ..policy()
        };
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &extension_policy,
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), extension_policy, FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(report.settled, 0, "the first look does not settle");
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(
            !l.records()
                .iter()
                .any(|r| matches!(r, LedgerRecord::RollbackPerformed { .. })),
            "an offered extension never rolls the test back"
        );
    }

    #[tokio::test]
    async fn a_guardrail_stop_alerts_the_customer() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        // Negative lift trips the fast-harm guardrail and rolls the test back.
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 3_600_000));
        let alerts = Arc::new(crate::alerts::RecordingAlertSink::new());
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner =
            Runner::new(ops.clone(), policy(), FillFormat::Rewarded).with_alerts(alerts.clone());
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        let emitted = alerts.alerts();
        assert_eq!(emitted.len(), 1, "{emitted:?}");
        assert!(
            matches!(emitted[0], Alert::GuardrailStop { .. }),
            "{emitted:?}"
        );
    }

    #[tokio::test]
    async fn a_failed_pull_alerts_the_customer() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        // No scripted days: the pull fails.
        let alerts = Arc::new(crate::alerts::RecordingAlertSink::new());
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner =
            Runner::new(ops.clone(), policy(), FillFormat::Rewarded).with_alerts(alerts.clone());
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("pass");
        assert_eq!(
            report.failures.len(),
            1,
            "the failure is recorded, not fatal"
        );
        assert!(matches!(
            alerts.alerts().as_slice(),
            [Alert::PullFailure { .. }]
        ));
    }

    #[tokio::test]
    async fn the_same_settings_in_a_different_encoding_do_not_void() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.set_supports_settings_read(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        // The same floors, keys in a different order: not a publisher edit.
        ops.put_current_settings("u1", r#"[{"floorEcpm":1,"country":"ee"}]"#);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: r#"[{"country":"ee","floorEcpm":1}]"#.into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(
            report.outcomes,
            vec![("exp-1".to_string(), "kept".to_string())],
            "a re-ordered encoding must not read as an edit"
        );
    }

    #[test]
    fn same_settings_compares_parsed_json_then_falls_back_to_strings() {
        assert!(same_settings("[1, 2]", "[1,2]"));
        assert!(!same_settings("[1, 2]", "[1, 3]"));
        assert!(same_settings("not json", "not json"));
        assert!(!same_settings("not json", "other"));
    }

    #[tokio::test]
    async fn a_second_daily_pass_does_not_re_settle() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, NOW + 1).await.expect("first");
        let second = runner.run_daily(&mut l, NOW + 2).await.expect("second");
        assert_eq!(
            second.pulled, 0,
            "a settled experiment is no longer running"
        );
        assert_eq!(second.settled, 0);
    }

    #[tokio::test]
    async fn a_settled_pending_experiment_stays_running_until_the_window_closes() {
        // Nine days is short of the ten a seven-day window needs: the engine
        // returns Pending and the experiment is retried next pass.
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        let mut short = constant_days(4_000_000, 4_400_000);
        short.control.truncate(9);
        short.test.truncate(9);
        ops.put_experiment_days("exp-1", short);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(report.pulled, 1);
        assert_eq!(report.settled, 0, "the window is not complete yet");
    }

    #[tokio::test]
    async fn aa_restarts_with_the_same_name_get_distinct_instance_ids() {
        let ops = std::sync::Arc::new(crate::ops::RecordingOps::new());
        ops.script_start(vec![Ok("exp-aa".into()), Ok("exp-aa".into())]);
        let runner = Runner::new(ops, policy(), FillFormat::Rewarded);
        let mut l = ledger();
        let spec = experiment_spec("exp-aa", "u1");
        runner
            .start_aa(&mut l, apex_core::PlatformId::AppLovinMax, &spec, NOW)
            .await
            .expect("first aa");
        runner
            .start_aa(&mut l, apex_core::PlatformId::AppLovinMax, &spec, NOW)
            .await
            .expect("second aa");

        let ids: Vec<String> = l
            .records()
            .iter()
            .filter_map(|record| match record {
                LedgerRecord::ExperimentStarted { spec, .. } => Some(spec.instance_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.iter().all(|id| !id.trim().is_empty()));
        assert_ne!(ids[0], ids[1], "a restart is a new experiment instance");
    }

    #[tokio::test]
    async fn monthly_pass_bills_a_kept_change_for_its_scope() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        ops.put_scope_revenue("u1", 1_000_000_000);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        assert_eq!(
            kept_changes(&l).expect("kept changes").len(),
            1,
            "one kept change is on the ledger"
        );
        script_holdout_players(&ops, &l);

        let dir = billing_dir("kept-scope");
        write_customer(&dir, "pub1", &customer("EE", None));
        let runner = runner
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.billed_scopes, 1);
        assert_eq!(report.fees.len(), 1);
        assert!(report.fees[0].1 > 0, "a proven lift bills a positive fee");
        let billing = l
            .records()
            .iter()
            .filter(|r| match r {
                LedgerRecord::LiftRecord { payload, .. } => payload.contains("monthlyBilling"),
                _ => false,
            })
            .count();
        assert_eq!(billing, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_holdout_bills_without_an_aa_qualification() {
        // A3-V4 E2: the A/A gate is gone from the monthly billing path. No
        // qualification is recorded at all, yet the measured holdout bills.
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let runner = Runner::new(ops, policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_billing_profile(seller_profile());
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.fees.len(), 1, "{:?}", report.fees);
        assert!(
            report.fees[0].1 > 0,
            "the point-estimate fee bills without a qualification"
        );
        assert!(
            report
                .holdout_no_fee
                .iter()
                .all(|(_, reason)| !reason.contains("A/A")),
            "no A/A gate reason is recorded: {:?}",
            report.holdout_no_fee
        );
        assert_eq!(monthly_billing_records(&l).len(), 1);
    }

    #[tokio::test]
    async fn a_stale_skewed_or_foreign_aa_record_does_not_gate_billing() {
        // Expired, skewed-split and wrong-publisher qualifications are audit
        // records now: none of them holds the month, and none of them is
        // needed for it either.
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        grant_aa_for_with_srm(
            &mut l,
            NOW - 40 * 86_400_000,
            crate::aa_qualification::MAX_SRM_CHI_SQUARE + 1.0,
        );
        grant_aa_for(
            &mut l,
            NOW + 1,
            "someone-else",
            apex_core::PlatformId::LevelPlay,
        );
        let runner = Runner::new(ops, policy(), FillFormat::Rewarded)
            .with_publisher("pub1")
            .with_billing_profile(seller_profile());
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.fees.len(), 1, "{:?}", report.fees);
        assert!(report.fees[0].1 > 0);
    }

    /// [`grant_aa`], with the split chi-square under test.
    fn grant_aa_for_with_srm(ledger: &mut InMemoryLedger, at: i64, srm_chi_square: f64) {
        crate::aa_qualification::record(
            ledger,
            "pub1",
            apex_core::PlatformId::AppLovinMax,
            srm_chi_square,
            1.0,
            0.0,
            at,
        )
        .expect("a qualification record");
    }

    fn monthly_billing_records(ledger: &InMemoryLedger) -> Vec<apex_lift::records::BillingRecord> {
        ledger
            .records()
            .iter()
            .filter_map(|record| match record {
                LedgerRecord::LiftRecord { payload, .. } => {
                    match serde_json::from_str::<apex_lift::records::LiftRecord>(payload) {
                        Ok(apex_lift::records::LiftRecord::MonthlyBilling(billing)) => {
                            Some(billing)
                        }
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect()
    }

    fn holdout_states(ledger: &InMemoryLedger) -> Vec<apex_lift::holdout::HoldoutState> {
        ledger
            .records()
            .iter()
            .filter_map(|record| match record {
                LedgerRecord::LiftRecord { payload, .. } => {
                    match serde_json::from_str::<apex_lift::records::LiftRecord>(payload) {
                        Ok(apex_lift::records::LiftRecord::HoldoutState(state)) => Some(state),
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn the_holdout_invoice_line_carries_gain_bounds_revenue_and_signed_refs() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("holdout-provenance");
        write_customer(&dir, "pub1", &customer("EE", None));
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.fees.len(), 1);
        let fee = report.fees[0].1;
        assert!(fee > 0);
        let records = monthly_billing_records(&l);
        assert_eq!(records.len(), 1);
        let billing = &records[0];
        assert_eq!(billing.rate_bp, apex_core::RESULTS_RATE_BP);
        assert!(billing.treated_revenue > 0);
        let line = billing.holdout.as_ref().expect("holdout reading");
        assert!(line.measured_lift > 0.0);
        assert!(line.lower_90 <= line.measured_lift);
        assert!(line.measured_lift <= line.upper_90);
        assert_eq!(line.revenue_micros, billing.treated_revenue);
        assert_eq!(line.fee_micros, fee);
        assert!(
            line.ledger_references.len() >= 3,
            "{:?}",
            line.ledger_references
        );
        assert!(!billing.pre_registration_hash.is_empty());
        assert!(!billing.result_hash.is_empty());
        assert!(line
            .ledger_references
            .iter()
            .any(|reference| reference == &billing.pre_registration_hash));
        assert!(line
            .ledger_references
            .iter()
            .any(|reference| reference == &billing.result_hash));
        let evidence = line.evidence.as_ref().expect("signed raw rows");
        assert_eq!(evidence.month_end, "2023-11-30");
        assert_eq!(evidence.measurement_scope, "u1");
        assert_eq!(
            evidence.window_days.len(),
            apex_lift::holdout::MEASUREMENT_DAYS
        );
        // One folded row per identified player, not one per player-day: the
        // signed evidence stays ledger-sized while reproducing the estimate.
        assert_eq!(evidence.rows.len(), 300, "{}", evidence.rows.len());
        assert!(evidence.pretest.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_flat_holdout_bills_nothing_and_states_why() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        script_holdout_lift(&ops, &l, 100_000, 100_000);
        let dir = billing_dir("holdout-flat");
        write_customer(&dir, "pub1", &customer("EE", None));
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.fees.len(), 1);
        assert_eq!(report.fees[0].1, 0);
        assert!(!report.holdout_no_fee.is_empty());
        let records = monthly_billing_records(&l);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].fee_micros, 0);
        assert_eq!(records[0].incremental, 0);
        let line = records[0].holdout.as_ref().expect("holdout");
        assert_eq!(line.measured_lift, 0.0);
        assert!(line.no_fee_note().is_some());
        assert_eq!(report.invoice_total_micros, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_incomplete_window_bills_nothing_and_never_rolls_back() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let kept = kept_changes(&l).expect("kept");
        let id = apex_lift::holdout::holdout_experiment_id(&kept[0].2.change_id);
        // A 7-day fragment whose numbers look confidently harmful: a short
        // window must not size a fee and must not drive a rollback either.
        let mut rows = Vec::new();
        for day in holdout_window_days("2023-11-30").into_iter().take(7) {
            for index in 0..270 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("exposed-{index}")),
                    idfv: None,
                    idfa: None,
                    test: false,
                    revenue_micros: 80_000,
                });
            }
            for index in 0..30 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("holdout-{index}")),
                    idfv: None,
                    idfa: None,
                    test: true,
                    revenue_micros: 100_000,
                });
            }
        }
        ops.put_experiment_players(
            &id,
            crate::ops::PlayerEvidence {
                rows,
                pretest: std::collections::BTreeMap::new(),
                measurement_scope: Some("u1".to_string()),
                roster: Some(apex_lift::holdout::AssignedRoster {
                    exposed: 270,
                    control: 30,
                }),
                eligible: None,
                certified_gain_lower: Some(900_000_000),
                certified_delta_lower: Some(0.04),
                assigned_test_control: None,
            },
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile());
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.fees.len(), 1);
        assert_eq!(report.fees[0].1, 0, "a short window never bills");
        assert!(
            report.holdout_rollbacks.is_empty(),
            "a short window never drives a rollback: {:?}",
            report.holdout_rollbacks
        );
        assert!(report
            .holdout_no_fee
            .iter()
            .any(|(_, reason)| reason.contains("28-day window")));
    }

    #[tokio::test]
    async fn a_confirmed_quiet_day_keeps_the_window_complete() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let kept = kept_changes(&l).expect("kept");
        let id = apex_lift::holdout::holdout_experiment_id(&kept[0].2.change_id);
        let days = holdout_window_days("2023-11-30");
        // The feed is silent on 2023-11-15; the platform reports zero scope
        // revenue for that exact day, so it is a quiet day, not a data hole.
        let mut rows = Vec::new();
        for day in &days {
            if day == "2023-11-15" {
                continue;
            }
            for index in 0..270 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("exposed-{index}")),
                    idfv: None,
                    idfa: None,
                    test: false,
                    revenue_micros: 104_000,
                });
            }
            for index in 0..30 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("holdout-{index}")),
                    idfv: None,
                    idfa: None,
                    test: true,
                    revenue_micros: 100_000,
                });
            }
        }
        ops.put_day_scope_revenue("u1", "2023-11-15", 0);
        ops.put_experiment_players(
            &id,
            crate::ops::PlayerEvidence {
                rows,
                pretest: std::collections::BTreeMap::new(),
                measurement_scope: Some("u1".to_string()),
                roster: Some(apex_lift::holdout::AssignedRoster {
                    exposed: 270,
                    control: 30,
                }),
                eligible: None,
                certified_gain_lower: Some(900_000_000),
                certified_delta_lower: Some(0.04),
                assigned_test_control: None,
            },
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile());
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.fees.len(), 1);
        assert!(report.fees[0].1 > 0, "a quiet day bills: {:?}", report.fees);
        assert!(
            !report
                .holdout_no_fee
                .iter()
                .any(|(_, reason)| reason
                    .contains("reports scope revenue but carries no player rows")),
            "{:?}",
            report.holdout_no_fee
        );
    }

    #[tokio::test]
    async fn a_day_with_scope_revenue_but_no_player_rows_blocks_the_fee() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let kept = kept_changes(&l).expect("kept");
        let id = apex_lift::holdout::holdout_experiment_id(&kept[0].2.change_id);
        let mut rows = Vec::new();
        for day in &holdout_window_days("2023-11-30") {
            if day == "2023-11-15" {
                continue;
            }
            for index in 0..270 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("exposed-{index}")),
                    idfv: None,
                    idfa: None,
                    test: false,
                    revenue_micros: 104_000,
                });
            }
            for index in 0..30 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("holdout-{index}")),
                    idfv: None,
                    idfa: None,
                    test: true,
                    revenue_micros: 100_000,
                });
            }
        }
        // No day-level override: the scripted month revenue (1e9) stands for
        // that day, so the missing feed day is a real evidence hole.
        ops.put_experiment_players(
            &id,
            crate::ops::PlayerEvidence {
                rows,
                pretest: std::collections::BTreeMap::new(),
                measurement_scope: Some("u1".to_string()),
                roster: Some(apex_lift::holdout::AssignedRoster {
                    exposed: 270,
                    control: 30,
                }),
                eligible: None,
                certified_gain_lower: Some(900_000_000),
                certified_delta_lower: Some(0.04),
                assigned_test_control: None,
            },
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile());
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.fees.len(), 1);
        assert_eq!(report.fees[0].1, 0, "an unproven day never bills");
        assert!(report
            .holdout_no_fee
            .iter()
            .any(|(_, reason)| reason.contains("scope revenue but carries no player rows")));
    }

    #[tokio::test]
    async fn an_observed_split_outside_ninety_ten_bills_nothing() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let kept = kept_changes(&l).expect("kept");
        let id = apex_lift::holdout::holdout_experiment_id(&kept[0].2.change_id);
        let mut rows = Vec::new();
        for day in holdout_window_days("2023-11-30") {
            for index in 0..90 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("exposed-{index}")),
                    idfv: None,
                    idfa: None,
                    test: false,
                    revenue_micros: 104_000,
                });
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("holdout-{index}")),
                    idfv: None,
                    idfa: None,
                    test: true,
                    revenue_micros: 100_000,
                });
            }
        }
        ops.put_experiment_players(
            &id,
            crate::ops::PlayerEvidence {
                rows,
                pretest: std::collections::BTreeMap::new(),
                measurement_scope: Some("u1".to_string()),
                roster: Some(apex_lift::holdout::AssignedRoster {
                    exposed: 270,
                    control: 30,
                }),
                eligible: None,
                certified_gain_lower: Some(900_000_000),
                certified_delta_lower: Some(0.04),
                assigned_test_control: None,
            },
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile());
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.fees.len(), 1);
        assert_eq!(report.fees[0].1, 0, "a 50/50 feed is not the sold split");
        assert!(report
            .holdout_no_fee
            .iter()
            .any(|(_, reason)| reason.contains("assignment-count")));
    }

    #[tokio::test]
    async fn unidentified_revenue_above_the_coverage_floor_bills_nothing() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let kept = kept_changes(&l).expect("kept");
        let id = apex_lift::holdout::holdout_experiment_id(&kept[0].2.change_id);
        let mut rows = Vec::new();
        for day in holdout_window_days("2023-11-30") {
            for index in 0..270 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("exposed-{index}")),
                    idfv: None,
                    idfa: None,
                    test: false,
                    revenue_micros: 104_000,
                });
            }
            for index in 0..30 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: Some(format!("holdout-{index}")),
                    idfv: None,
                    idfa: None,
                    test: true,
                    revenue_micros: 100_000,
                });
            }
            for _ in 0..100 {
                rows.push(apex_lift::player_store::PlayerDay {
                    day: day.clone(),
                    user_id: None,
                    idfv: None,
                    idfa: None,
                    test: false,
                    revenue_micros: 100_000,
                });
            }
        }
        ops.put_experiment_players(
            &id,
            crate::ops::PlayerEvidence {
                rows,
                pretest: std::collections::BTreeMap::new(),
                measurement_scope: Some("u1".to_string()),
                roster: Some(apex_lift::holdout::AssignedRoster {
                    exposed: 270,
                    control: 30,
                }),
                eligible: None,
                certified_gain_lower: Some(900_000_000),
                certified_delta_lower: Some(0.04),
                assigned_test_control: None,
            },
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile());
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.fees.len(), 1);
        assert_eq!(report.fees[0].1, 0, "uncovered revenue is not extrapolated");
        assert!(report
            .holdout_no_fee
            .iter()
            .any(|(_, reason)| reason.contains("no resolved player id")));
    }

    #[tokio::test]
    async fn a_confident_harm_rolls_back_and_removes_future_billing() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        script_holdout_lift(&ops, &l, 80_000, 100_000);
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile());
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.holdout_rollbacks.len(), 1);
        assert!(report.fees.is_empty(), "harm bills nothing");
        let states = holdout_states(&l);
        let last = states.last().expect("state");
        assert!(!last.active());
        assert_eq!(last.rollback_day.as_deref(), Some("2023-11-01"));
        assert!(l.records().iter().any(|record| match record {
            LedgerRecord::LiftRecord { payload, .. } => {
                payload.contains("billingChange") && payload.contains("holdoutHarm")
            }
            _ => false,
        }));

        // The next month never bills the rolled-back instance.
        let next = runner
            .run_monthly(
                &mut l,
                "pub1",
                "2023-12",
                "2023-12-01",
                31,
                NOW + 40 * 86_400_000,
            )
            .await
            .expect("next month");
        assert!(next.fees.is_empty(), "{:?}", next.fees);
        assert_eq!(monthly_billing_records(&l).len(), 0);
    }

    #[tokio::test]
    async fn a_restart_never_writes_the_same_holdout_fee_twice() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("holdout-restart");
        // No customer profile: the ledger line is written but issuance is
        // refused, which is exactly the crash window before the invoice.
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile())
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let first = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("first pass");
        assert!(first.invoice_refusal.is_some());
        let count_after_first = monthly_billing_records(&l).len();
        assert_eq!(count_after_first, 1);
        grant_aa(&mut l, NOW + 1);
        let second = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 3)
            .await
            .expect("restart pass");
        assert_eq!(monthly_billing_records(&l).len(), 1, "no second fee line");
        assert_eq!(second.fees, first.fees);
        assert_eq!(second.invoice_refusal, first.invoice_refusal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_signed_legacy_fee_survives_a_revenue_restatement_before_issuance() {
        let ops = kept_ops();
        // A keep from before holdouts existed: no platform holdout is
        // opened, so this change bills through the legacy monthly path.
        ops.set_holdout_supported(false);
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("legacy-replay");
        // Pass one signs the fee; the missing customer refuses the invoice
        // and consumes no register number — the exact crash window.
        let first = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"))
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("the pass completes with the refusal recorded");
        assert_eq!(first.fees.len(), 1, "the legacy fee was signed");
        let signed_amount = first.fees[0].1;
        assert!(signed_amount > 0);
        assert!(first.invoice_number.is_none());
        assert!(first.invoice_refusal.is_some());

        // The platform restates the month to zero revenue before the retry:
        // the recomputation now produces no line for this change at all.
        ops.put_scope_revenue("u1", 0);
        write_customer(&dir, "pub1", &customer("EE", Some("EE987654321")));
        let second = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_vat_rate(2_000)
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"))
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 3)
            .await
            .expect("the retry issues");
        assert_eq!(
            second.fees.len(),
            1,
            "the signed fee reaches the retry even though revenue is now zero"
        );
        assert_eq!(second.fees[0].1, signed_amount, "replayed, not recomputed");
        let number = second.invoice_number.expect("issued");
        assert!(number > 0);
        assert!(
            second
                .rendered_invoice
                .as_deref()
                .is_some_and(|text| text.contains("VAT invoice No.")),
            "the invoice document carries the signed line"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restart_with_the_same_name_never_inherits_another_instances_keep() {
        // Two attempts reuse one platform experiment name and one platform
        // id (MAX returns the ad unit id). Only the first instance later gets
        // a keep: the restart must not become billable from it.
        let name = "max-experiment:reused";
        let unit = "u1";
        let mut l = ledger();
        append_instance_attempt(&mut l, name, "inst-old", unit, "key-old", NOW);
        append_instance_attempt(&mut l, name, "inst-new", unit, "key-new", NOW + 10);
        append_lift(
            &mut l,
            instance_result(name, "inst-old", "keep", 0.03),
            NOW + 20,
        );

        let kept = kept_changes(&l).expect("kept changes");
        assert_eq!(kept.len(), 1, "only the instance with the keep bills");
        assert_eq!(kept[0].2.change_id, "inst-old");
        assert!(
            kept.iter()
                .all(|(_, _, change)| change.change_id != "inst-new"),
            "the restart never inherits the later keep"
        );
    }

    #[test]
    fn two_instances_with_the_same_name_bill_as_two_distinct_instances() {
        let name = "max-experiment:reused";
        let unit = "u1";
        let mut l = ledger();
        append_instance_attempt(&mut l, name, "inst-a", unit, "key-a", NOW);
        append_lift(
            &mut l,
            instance_result(name, "inst-a", "keep", 0.03),
            NOW + 3,
        );
        append_instance_attempt(&mut l, name, "inst-b", unit, "key-b", NOW + 10);
        append_lift(
            &mut l,
            instance_result(name, "inst-b", "keep", 0.02),
            NOW + 20,
        );

        let kept = kept_changes(&l).expect("kept changes");
        assert_eq!(kept.len(), 2, "both instances bill independently");
        let mut ids: Vec<&str> = kept
            .iter()
            .map(|(_, _, change)| change.change_id.as_str())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, ["inst-a", "inst-b"]);
        assert!(
            kept.iter().all(|(_, _, change)| change.change_id != name),
            "the reusable name is never a billing identity"
        );
    }

    #[tokio::test]
    async fn the_signed_monthly_line_preserves_the_prereg_and_result_hashes() {
        let ops = kept_ops();
        let l = kept_ledger(&ops, NOW).await;

        let mut prereg_hash = None;
        let mut result_hash = None;
        let mut prereg_instance = None;
        let mut result_instance = None;
        for record in l.records() {
            let LedgerRecord::LiftRecord {
                payload_sha256,
                payload,
                ..
            } = record
            else {
                continue;
            };
            match serde_json::from_str::<apex_lift::records::LiftRecord>(payload) {
                Ok(apex_lift::records::LiftRecord::PreRegistration { instance_id, .. }) => {
                    prereg_hash = Some(payload_sha256.clone());
                    prereg_instance = Some(instance_id);
                }
                Ok(apex_lift::records::LiftRecord::Result { instance_id, .. }) => {
                    result_hash = Some(payload_sha256.clone());
                    result_instance = Some(instance_id);
                }
                _ => {}
            }
        }
        let prereg_hash = prereg_hash.expect("signed pre-registration");
        let result_hash = result_hash.expect("signed result");
        let prereg_instance = prereg_instance.expect("pre-registration instance");
        let result_instance = result_instance.expect("result instance");
        assert!(!prereg_instance.trim().is_empty());
        assert_eq!(prereg_instance, result_instance);

        let kept = kept_changes(&l).expect("kept changes");
        assert_eq!(kept.len(), 1);
        let change = &kept[0].2;
        assert_eq!(change.change_id, prereg_instance);
        assert_eq!(change.pre_registration_hash, prereg_hash);
        assert_eq!(change.result_hash, result_hash);

        let run = apex_lift::run::run_month(
            "2023-11",
            "2023-11-01",
            30,
            1_000_000_000,
            std::slice::from_ref(change),
        )
        .expect("month");
        let emitted =
            apex_lift::emit::monthly_billing(&run.fees[0], "2023-11").expect("monthly billing");
        let apex_lift::records::LiftRecord::MonthlyBilling(billing) = emitted.record else {
            panic!("expected a monthly billing record");
        };
        assert_eq!(billing.change_id, change.change_id);
        assert_eq!(billing.pre_registration_hash, prereg_hash);
        assert_eq!(billing.result_hash, result_hash);
    }

    #[tokio::test]
    async fn an_out_of_band_rate_never_reaches_the_fee_preview() {
        use apex_core::{ExperimentMetric, PlatformId};
        use apex_lift::records::LiftRecord;

        // A hand-built ledger: one experiment with an out-of-band
        // pre-registered rate and a kept result. The invoice would refuse
        // this rate, so the preview path must skip it.
        let mut tampered = ledger();
        tampered
            .append(LedgerRecord::ExperimentStarted {
                seq: 0,
                at_unix_ms: NOW,
                platform: PlatformId::AppLovinMax,
                experiment_id: "exp_ob".into(),
                spec: apex_core::ExperimentSpec {
                    name: "exp_ob".into(),
                    instance_id: String::new(),
                    ad_unit_id: "u1".into(),
                    metric: ExperimentMetric::Ecpm,
                    variants: 2,
                    change_description: "rewarded floor".into(),
                    test_group_allocation: 25,
                },
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            })
            .expect("append started");
        let prereg = LiftRecord::PreRegistration {
            experiment_id: "exp_ob".into(),
            instance_id: String::new(),
            scope: "u1/ee".into(),
            before_value: serde_json::json!(2_500_000),
            after_value: serde_json::json!(2_700_000),
            split: 0.25,
            metric: "ecpm".into(),
            burn_in_days: 3,
            window_days: 14,
            looks: 1,
            extension_allowed: false,
            guardrail_thresholds: serde_json::json!({}),
            code_version_hash: "deadbeef".into(),
            rate_bp: 2_501,
            min_point_lift: 0.01,
        };
        let payload = prereg.canonical_json().expect("canonical json");
        let sha = prereg.sha256_hex().expect("sha");
        tampered
            .append(LedgerRecord::lift_record(NOW, &sha, &payload))
            .expect("append prereg");
        let result = LiftRecord::Result {
            experiment_id: "exp_ob".into(),
            instance_id: String::new(),
            daily_log_ratios: vec![0.1, 0.2],
            mean_log: 0.15,
            lower_90: 0.05,
            lower_99: 0.02,
            bootstrap_lower: 0.01,
            decision: "keep".into(),
            lambda: 1.2,
        };
        let payload = result.canonical_json().expect("canonical json");
        let sha = result.sha256_hex().expect("sha");
        tampered
            .append(LedgerRecord::lift_record(NOW + 1, &sha, &payload))
            .expect("append result");

        assert!(
            kept_changes(&tampered).expect("kept changes").is_empty(),
            "an out-of-band rate is excluded from the preview set"
        );

        // The same scenario with an in-band rate stays included.
        let mut in_band = ledger();
        for record in tampered.records() {
            match record {
                LedgerRecord::LiftRecord {
                    payload,
                    payload_sha256,
                    ..
                } if payload.contains("preRegistration") => {
                    let fixed = payload.replace("2501", "2000");
                    in_band
                        .append(LedgerRecord::lift_record(NOW, payload_sha256, &fixed))
                        .expect("append fixed prereg");
                }
                other => {
                    in_band.append(other.clone()).expect("append clone");
                }
            }
        }
        assert_eq!(
            kept_changes(&in_band).expect("kept changes").len(),
            1,
            "the in-band equivalent stays included"
        );
    }

    #[tokio::test]
    async fn a_kept_admob_experiment_is_fee_ineligible_and_never_billed() {
        use apex_core::{ExperimentMetric, PlatformId};
        use apex_lift::billing::{FeeIneligible, ADMOB_NO_MEASUREMENT_REASON};
        use apex_lift::records::LiftRecord;

        // A hand-built AdMob ledger: one experiment whose result is a keep
        // with a positive lambda (the only shape that could ever produce a
        // fee). AdMob documents no per-variant measurement, so the fee path
        // must exclude it with the typed reason — while the keep itself (the
        // monitoring/managed outcome) stays recorded.
        let mut admob = ledger();
        admob
            .append(LedgerRecord::ExperimentStarted {
                seq: 0,
                at_unix_ms: NOW,
                platform: PlatformId::Admob,
                experiment_id: "exp_ab".into(),
                spec: apex_core::ExperimentSpec {
                    name: "exp_ab".into(),
                    instance_id: String::new(),
                    ad_unit_id: "u1".into(),
                    metric: ExperimentMetric::Revenue,
                    variants: 2,
                    change_description: "mediation eCPM".into(),
                    test_group_allocation: 25,
                },
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            })
            .expect("append started");
        let prereg = LiftRecord::PreRegistration {
            experiment_id: "exp_ab".into(),
            instance_id: String::new(),
            scope: "g1/u1".into(),
            before_value: serde_json::json!(1_000_000),
            after_value: serde_json::json!(1_200_000),
            split: 0.25,
            metric: "revenue_per_user".into(),
            burn_in_days: 3,
            window_days: 14,
            looks: 1,
            extension_allowed: false,
            guardrail_thresholds: serde_json::json!({
                "harmPoint": -0.05,
                "fillRewarded": 0.02,
                "fillBanner": 0.05,
                "splitChiSquare": 10.8276,
            }),
            code_version_hash: "deadbeef".into(),
            rate_bp: 2_000,
            min_point_lift: 0.01,
        };
        let payload = prereg.canonical_json().expect("canonical json");
        let sha = prereg.sha256_hex().expect("sha");
        admob
            .append(LedgerRecord::lift_record(NOW, &sha, &payload))
            .expect("append prereg");
        let result = LiftRecord::Result {
            experiment_id: "exp_ab".into(),
            instance_id: String::new(),
            daily_log_ratios: vec![0.1, 0.2],
            mean_log: 0.15,
            lower_90: 0.05,
            lower_99: 0.02,
            bootstrap_lower: 0.01,
            decision: "keep".into(),
            lambda: 0.03,
        };
        let payload = result.canonical_json().expect("canonical json");
        let sha = result.sha256_hex().expect("sha");
        admob
            .append(LedgerRecord::lift_record(NOW + 1, &sha, &payload))
            .expect("append result");
        let kept = kept_changes(&admob).expect("kept changes");
        assert_eq!(kept.len(), 1, "the keep is on the ledger");

        // A different billing change (a stop recorded earlier) for the SAME
        // change: the ineligible dedup must match change AND reason together,
        // never either alone.
        let planted = apex_lift::emit::billing_change(
            &kept[0].2.change_id,
            "2023-10-31",
            "manualOverride",
            0.0,
        )
        .expect("planted change");
        let planted_payload = planted.record.canonical_json().expect("canonical");
        admob
            .append(LedgerRecord::lift_record(
                NOW + 1,
                &planted.sha256,
                &planted_payload,
            ))
            .expect("planted");

        let ops = Arc::new(RecordingOps::new());
        ops.put_scope_revenue("u1", 1_000_000_000);
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile());
        let report = runner
            .run_monthly(&mut admob, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(
            report.fee_ineligible,
            vec![FeeIneligible {
                platform: PlatformId::Admob,
                reason: ADMOB_NO_MEASUREMENT_REASON.to_string(),
            }]
        );
        assert!(report.fees.is_empty(), "no fee is ever produced");
        assert_eq!(report.billed_scopes, 0);
        assert_eq!(report.invoice_total_micros, 0, "no charge of any kind");
        // The platform was never asked for scope revenue: the change is
        // excluded before any fee computation.
        assert!(
            !ops.call_log()
                .iter()
                .any(|line| line.starts_with("scope_revenue")),
            "{:?}",
            ops.call_log()
        );
        // The typed decision is recorded on the ledger as a billing change
        // carrying the exact reason; no monthly billing record exists.
        let mut ineligible_records = 0;
        let mut billing_records = 0;
        for record in admob.records() {
            let LedgerRecord::LiftRecord { payload, .. } = record else {
                continue;
            };
            if payload.contains("monthlyBilling") {
                billing_records += 1;
            }
            if payload.contains("billingChange") && payload.contains(ADMOB_NO_MEASUREMENT_REASON) {
                ineligible_records += 1;
            }
        }
        assert_eq!(billing_records, 0, "never billed");
        assert_eq!(ineligible_records, 1, "the decision is recorded");

        // The next month reports the same typed decision to the operator but
        // never appends the identical record to the append-only ledger again.
        let second = runner
            .run_monthly(&mut admob, "pub1", "2023-12", "2023-12-01", 31, NOW + 3)
            .await
            .expect("second monthly");
        assert_eq!(
            second
                .fee_ineligible
                .iter()
                .map(|entry| entry.reason.as_str())
                .collect::<Vec<_>>(),
            vec![ADMOB_NO_MEASUREMENT_REASON],
            "the operator still sees why every pass"
        );
        let mut ineligible_records = 0;
        for record in admob.records() {
            let LedgerRecord::LiftRecord { payload, .. } = record else {
                continue;
            };
            if payload.contains("billingChange") && payload.contains(ADMOB_NO_MEASUREMENT_REASON) {
                ineligible_records += 1;
            }
        }
        assert_eq!(ineligible_records, 1, "recorded once, not once per month");
    }

    #[tokio::test]
    async fn a_rolled_back_change_is_never_billed() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 3_600_000));
        ops.put_scope_revenue("u1", 1_000_000_000);
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");

        let dir = billing_dir("rolled-back");
        let runner = runner
            .with_billing_profile(seller_profile())
            .with_register_path(dir.join("invoices.jsonl"));
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        assert_eq!(report.billed_scopes, 0, "no kept change, no scope");
        assert!(report.fees.is_empty());
        assert!(
            report.invoice_number.is_none(),
            "no fee means no invoice to issue"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_holdout_retires_the_month_six_recheck() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");

        // The kept day is late 2023; six months on, the old reversal re-check
        // would be due, but the persisted 90/10 holdout replaces it.
        let later = crate::calendar::day_number("2024-06-01").expect("day") * 86_400_000;
        let due = runner.due_rechecks(&l, later).expect("due rechecks");
        assert!(due.is_empty(), "H4 retires the month-6 recheck: {due:?}");
        assert!(!holdout_states(&l).is_empty());
    }

    #[tokio::test]
    async fn the_holdout_retires_the_early_ecpm_recheck() {
        let ops = Arc::new(RecordingOps::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        runner.run_daily(&mut l, NOW + 1).await.expect("daily");

        // Persist a 28-day baseline of $5 eCPM and a latest $3 eCPM: the
        // actual drift is -40%, above the 30% trigger.
        let dir = std::env::temp_dir().join(format!("apex-recheck-drift-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("watch.json");
        let watch = crate::account_watch::AccountWatch::new(&path, 0.2).expect("watch");
        let sink = crate::alerts::RecordingAlertSink::new();
        let snapshot = |revenue: i64| apex_audit::Snapshot {
            platform: apex_core::PlatformId::AppLovinMax,
            fetched_at_unix_ms: NOW,
            ad_units: Vec::new(),
            waterfall: Vec::new(),
            mediation_groups: Vec::new(),
            bidder_accounts: Vec::new(),
            country_revenue: vec![apex_audit::CountryRevenueRow {
                country: "EE".into(),
                ad_unit_id: "u1".into(),
                revenue_30d: revenue,
                impressions_30d: 1_000,
            }],
            daily_revenue: Vec::new(),
            app_ads_txt: apex_audit::AppAdsTxt {
                present: true,
                lines: vec!["a,1,DIRECT".into()],
                expected_networks: Vec::new(),
            },
            platform_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: revenue,
                impressions_30d: 1_000,
            },
            mediation_totals_30d: apex_audit::RevenueTotals {
                revenue_30d: revenue,
                impressions_30d: 1_000,
            },
            mediation_totals_reported: true,
        };
        watch
            .observe("pub1", "applovin_max", &snapshot(5_000_000), NOW, &sink)
            .await
            .expect("baseline");
        watch
            .observe("pub1", "applovin_max", &snapshot(3_000_000), NOW + 1, &sink)
            .await
            .expect("compare");

        let runner = runner.with_account_watch(&path, 0.2).expect("watch");
        // Only one month has passed and the drift is -40%, but the persisted
        // 90/10 holdout replaces the early re-check as well.
        let due = runner
            .due_rechecks(&l, NOW + 30 * 86_400_000)
            .expect("due rechecks");
        assert!(due.is_empty(), "the holdout replaces the recheck: {due:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn with_priors_accepts_owned_and_borrowed_paths() {
        let ops = Arc::new(RecordingOps::new());
        let owned = Runner::new(ops.clone(), policy(), FillFormat::Rewarded).with_priors(
            apex_lift::priors::LiftPriors::default(),
            String::from("/tmp/apex-priors-owned.json"),
        );
        let borrowed_path = std::path::PathBuf::from("/tmp/apex-priors-borrowed.json");
        let borrowed = Runner::new(ops, policy(), FillFormat::Rewarded)
            .with_priors(apex_lift::priors::LiftPriors::default(), &borrowed_path);
        assert!(owned.runner_priors().is_some());
        assert!(borrowed.runner_priors().is_some());
    }

    #[test]
    fn change_bucket_is_none_for_non_floor_changes() {
        let mut l = ledger();
        l.append(LedgerRecord::change_applied(
            1,
            apex_core::PlatformId::Admob,
            "u1",
            ChangeKind::EcpmSet {
                mediation_group_id: "g".into(),
                ad_unit_id: "u1".into(),
                ecpm: 1,
            },
            "k",
            "p",
        ))
        .expect("append");
        let index = LedgerIndex::build(&l);
        assert!(change_bucket(&index, 0).is_none());
    }

    #[tokio::test]
    async fn policy_rechecks_and_debug_are_all_reachable() {
        let ops = Arc::new(RecordingOps::new());
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        assert_eq!(runner.policy().window_days, 7);
        assert!(format!("{runner:?}").contains("Runner"));

        let mut l = ledger();
        // A months-later clock makes the month-6 re-check due for a kept change.
        let later = crate::calendar::day_number("2024-06-01").expect("day") * 86_400_000;
        assert!(
            runner
                .due_rechecks(&l, later)
                .expect("due rechecks")
                .is_empty(),
            "nothing kept yet"
        );

        // Exercise the null adapter so its trait surface is covered too.
        let null = NullOps;
        assert!(!null.writable("op").await);
        assert!(null.experiment("x").await.is_err());
        assert!(null.apply(&floor_action(), "k").await.is_ok());
        assert!(null.rollback(&floor_action()).await.is_ok());
        assert!(null
            .start_experiment(&crate::ops::ExperimentRequest {
                experiment_id: "e".into(),
                spec: apex_core::ExperimentSpec {
                    name: "e".into(),
                    instance_id: String::new(),
                    ad_unit_id: "u".into(),
                    metric: apex_core::ExperimentMetric::Revenue,
                    variants: 2,
                    change_description: "c".into(),
                    test_group_allocation: 50,
                },
                change: apex_proof::ChangeKind::CountryFloorSet {
                    country: "EE".into(),
                    floor_ecpm: 1,
                },
                prev_value: "{}".into(),
            })
            .await
            .is_ok());
        assert!(null.promote_experiment("x").await.is_ok());
        assert!(null.deprecate_experiment("x").await.is_ok());
        assert!(null.call_log().is_empty());
        assert!(!null.change_in_test_arm());
        assert!(null.scope_revenue("u", "a", "b").await.is_err());
        assert!(null.experiment_days("x").await.is_err());
        assert!(null.current_settings("u").await.is_err());
        assert!(null
            .start_null_experiment(&apex_core::ExperimentSpec {
                name: "e".into(),
                instance_id: String::new(),
                ad_unit_id: "u".into(),
                metric: apex_core::ExperimentMetric::Revenue,
                variants: 2,
                change_description: "c".into(),
                test_group_allocation: 50,
            })
            .await
            .is_err());
        let _ = &mut l;
    }

    /// A minimal adapter used to prove the runner is generic over PlatformOps.
    struct NullOps;

    #[async_trait]
    impl PlatformOps for NullOps {
        fn supports_player_feed(&self) -> bool {
            false
        }

        async fn writable(&self, _operation: &str) -> bool {
            false
        }
        async fn start_experiment(
            &self,
            _request: &crate::ops::ExperimentRequest,
        ) -> Result<String, apex_core::PlatformError> {
            Ok(String::new())
        }
        async fn experiment(
            &self,
            _id: &str,
        ) -> Result<ExperimentResult, apex_core::PlatformError> {
            Err(apex_core::PlatformError::Validation("none".into()))
        }
        async fn promote_experiment(&self, _id: &str) -> Result<(), apex_core::PlatformError> {
            Ok(())
        }
        async fn deprecate_experiment(&self, _id: &str) -> Result<(), apex_core::PlatformError> {
            Ok(())
        }
        async fn apply(
            &self,
            _action: &Action,
            _key: &str,
        ) -> Result<(), apex_core::PlatformError> {
            Ok(())
        }
        async fn rollback(&self, _action: &Action) -> Result<(), apex_core::PlatformError> {
            Ok(())
        }
        fn call_log(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn a_malformed_customer_profile_refuses_the_invoice_typed() {
        let ops = kept_ops();
        let mut l = kept_ledger(&ops, NOW).await;
        let dir = billing_dir("malformed-customer");
        std::fs::write(dir.join("pub1.billing.json"), "{ not json").unwrap();
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_billing_profile(seller_profile())
            .with_connections_root(&dir)
            .with_register_path(dir.join("invoices.jsonl"));
        grant_aa(&mut l, NOW + 1);
        let report = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 2)
            .await
            .expect("monthly");
        let refusal = report.invoice_refusal.expect("refused");
        assert!(
            refusal.contains("is not valid JSON"),
            "the typed reason names the parse failure: {refusal}"
        );
        assert!(report.invoice_number.is_none());
        // The refusal path also covers a profile path that is not a file.
        let _ = std::fs::remove_file(dir.join("pub1.billing.json"));
        std::fs::create_dir_all(dir.join("pub1.billing.json")).unwrap();
        grant_aa(&mut l, NOW + 2);
        let unreadable = runner
            .run_monthly(&mut l, "pub1", "2023-11", "2023-11-01", 30, NOW + 3)
            .await
            .expect("monthly");
        assert!(unreadable
            .invoice_refusal
            .unwrap()
            .contains("could not be read"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn owner_scoped_operator_jobs_claim_only_their_own_targets() {
        let dir =
            std::env::temp_dir().join(format!("apex-owner-scoped-jobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (id, kind, target) in [
            ("opj_000000000021", "retry_read", "exp-1"),
            ("opj_000000000022", "retry_read", "exp-other"),
        ] {
            let job = crate::operator_jobs::OperatorJob {
                job_id: id.to_string(),
                kind: kind.to_string(),
                requested_by: "maya".to_string(),
                requested_at_ms: NOW,
                target: target.to_string(),
                payload: serde_json::json!({}),
                idempotency_key: format!("k-{id}"),
                state: "queued".to_string(),
                state_history: vec![],
                outcome: serde_json::Value::Null,
                correlation_id: "corr_1".to_string(),
                owner: None,
                lease_expires_at_ms: None,
                attempts: 0,
            };
            std::fs::write(
                dir.join(format!("{id}.json")),
                serde_json::to_string_pretty(&job).unwrap(),
            )
            .unwrap();
        }
        let ops = kept_ops();
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        // The pre-registered experiment is the script's `exp-1`; the other
        // target belongs to no chain in this ledger.
        let runner = Runner::new(ops, policy(), FillFormat::Rewarded)
            .with_operator_jobs(dir.clone())
            .with_owner_scoped_operator_jobs(true);
        let report = runner.run_daily(&mut l, NOW + 1).await.expect("daily");
        let jobs = report.operator_jobs.expect("jobs");
        assert_eq!(jobs.completed, 1, "only the owned recheck ran");
        assert_eq!(jobs.skipped, 1, "the foreign target stays queued");
        let foreign: crate::operator_jobs::OperatorJob = serde_json::from_str(
            &std::fs::read_to_string(dir.join("opj_000000000022.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(foreign.state, "queued");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn months_between_counts_whole_months() {
        let day = |text: &str| crate::calendar::day_number(text).expect("valid day");
        assert_eq!(months_between("2023-11-14", day("2023-11-20")), Some(0));
        assert_eq!(months_between("2023-11-01", day("2023-12-01")), Some(1));
        assert_eq!(months_between("2023-05-01", day("2023-11-01")), Some(6));
        assert_eq!(months_between("2023-05-20", day("2023-11-01")), Some(5));
        assert_eq!(months_between("bad", day("2023-11-01")), None);
    }

    #[tokio::test]
    async fn an_empty_ledger_is_a_no_op() {
        let runner = Runner::new(Arc::new(NullOps), policy(), FillFormat::Rewarded);
        let mut l = ledger();
        let report = runner.run_daily(&mut l, NOW).await.expect("daily");
        assert_eq!(report, DailyReport::default());
    }

    /// The full instrumented runner for one test: file sink outbox, delivery
    /// log, registry, analytics store and the recording ops.
    struct Rig {
        runner: Runner,
        dir: std::path::PathBuf,
        store: Arc<apex_analytics::MemoryStore>,
        registry: Arc<apex_metrics::Registry>,
    }

    async fn rig(ops: Arc<RecordingOps>, label: &str) -> Rig {
        let dir =
            std::env::temp_dir().join(format!("apex-runner-wire-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let registry = Arc::new(apex_metrics::Registry::new());
        let sink = Arc::new(apex_notify::sink::FileSink::new(dir.join("outbox.jsonl")));
        let log =
            apex_notify::state::DeliveryLog::open(Some(&dir.join("delivery.jsonl"))).expect("log");
        let notifier = Arc::new(apex_notify::notifier::Notifier::new(
            sink,
            log,
            registry.clone(),
        ));
        let book =
            apex_notify::contacts::ContactBook::from_json(r#"{"northstar": ["ops@example.com"]}"#)
                .expect("book");
        let notify = Arc::new(crate::notify::NotifyPlan::new(
            notifier,
            book,
            "https://console.apexmediation.ee",
        ));
        let store = Arc::new(apex_analytics::MemoryStore::new());
        {
            use apex_analytics::EventStore as _;
            store.ensure_schema().await.expect("schema");
        }
        let client = apex_analytics::AnalyticsClient::new(store.clone(), "dev", "apex-runner-test");
        let events = Arc::new(
            crate::events::RunnerEvents::new(Arc::new(client), "northstar").with_environment("dev"),
        );
        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded)
            .with_publisher("northstar")
            .with_notifications(notify)
            .with_events(events)
            .with_telemetry(crate::telemetry::RunnerTelemetry::new(registry.clone()));
        Rig {
            runner,
            dir,
            store,
            registry,
        }
    }

    fn outbox(rig: &Rig) -> Vec<serde_json::Value> {
        std::fs::read_to_string(rig.dir.join("outbox.jsonl"))
            .map(|raw| {
                raw.lines()
                    .map(|line| serde_json::from_str(line).expect("row"))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn stored_types(rig: &Rig) -> Vec<String> {
        use apex_analytics::store::ProjectionStore as _;
        rig.store
            .load_events()
            .await
            .expect("events")
            .into_iter()
            .map(|event| event.event_type.as_str().to_string())
            .collect()
    }

    fn metric_counter(
        rig: &Rig,
        metric: &'static apex_metrics::MetricSpec,
        labels: &[(&'static str, &str)],
    ) -> u64 {
        rig.registry.counter_value(metric, labels).expect("metric")
    }

    #[tokio::test]
    async fn an_auth_refused_pull_notifies_reporting_access_and_records_failed_events() {
        use crate::ops::RecordingOps as Ops;
        let ops = Arc::new(Ops::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.script_experiment_days(vec![Err(apex_core::PlatformError::Auth {
            platform: apex_core::PlatformId::AppLovinMax,
            operation: "experimentDays".into(),
            status: 401,
        })]);
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        let rig = rig(ops.clone(), "auth").await;
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{\"floor\":1000000}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let report = rig
            .runner
            .run_daily(&mut l, NOW + 8 * 86_400_000)
            .await
            .expect("daily");
        assert_eq!(report.pull_failures, 1);
        assert_eq!(report.failures.len(), 1);
        let rows = outbox(&rig);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["template_class"], "reporting_access");
        let text = rows[0]["text"].as_str().expect("text");
        assert!(text.contains("Reconnect MAX reporting for Northstar"));
        assert!(text.contains("The latest reporting request was refused."));
        let types = stored_types(&rig).await;
        assert!(types.contains(&"report_pull_started".to_string()));
        assert!(types.contains(&"report_pull_failed".to_string()));
        assert!(!types.contains(&"report_pull_completed".to_string()));
        let _ = std::fs::remove_dir_all(&rig.dir);
    }

    #[tokio::test]
    async fn a_rollback_notifies_the_stop_and_records_the_restore_state() {
        use crate::ops::RecordingOps as Ops;
        let ops = Arc::new(Ops::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![Ok("exp-1".into())]);
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 3_600_000));
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        let rig = rig(ops.clone(), "stop").await;
        let mut l = ledger();
        run_cycle(
            ops.as_ref(),
            &mut l,
            &policy(),
            &[],
            vec![crate::model::Proposal {
                action: floor_action(),
                rationale: "raise EE".into(),
                expected_effect_bp: 500,
                prev_value: "{\"floor\":1000000}".into(),
            }],
            NOW,
        )
        .await
        .expect("cycle");
        let report = rig
            .runner
            .run_daily(&mut l, NOW + 8 * 86_400_000)
            .await
            .expect("daily");
        assert!(report
            .outcomes
            .iter()
            .any(|(_, status)| status == "rolled_back"));
        assert!(report.failures.is_empty());
        // The stop message and the unconfirmed restore both reached the
        // contacts, with a stable incident reference.
        let rows = outbox(&rig);
        let classes: Vec<&str> = rows
            .iter()
            .map(|row| row["template_class"].as_str().unwrap_or_default())
            .collect();
        assert!(classes.contains(&"guardrail_stop"), "{classes:?}");
        assert!(classes.contains(&"restore_failed"), "{classes:?}");
        let restore = rows
            .iter()
            .find(|row| row["template_class"] == "restore_failed")
            .expect("restore row");
        let text = restore["text"].as_str().expect("text");
        assert!(text.contains("incident inc_"));
        assert!(text.contains("What is known:"));
        let incident = crate::notify::incident_id_for("exp-1");
        assert!(text.contains(&incident));
        let types = stored_types(&rig).await;
        for expected in [
            "experiment_decided",
            "guardrail_fired",
            "guardrail_restore_requested",
            "guardrail_restore_failed",
        ] {
            assert!(types.contains(&expected.to_string()), "missing {expected}");
        }
        assert_eq!(
            metric_counter(
                &rig,
                &apex_metrics::catalog::RESTORE_OPERATIONS_TOTAL,
                &[
                    ("platform", "applovin_max"),
                    ("operation", "restore"),
                    ("outcome", "failed"),
                ],
            ),
            1
        );
        assert_eq!(
            metric_counter(
                &rig,
                &apex_metrics::catalog::EXPERIMENT_DECISIONS_TOTAL,
                &[
                    ("platform", "applovin_max"),
                    ("decision", "rollback"),
                    ("reason_class", "harm"),
                ],
            ),
            1
        );
        let _ = std::fs::remove_dir_all(&rig.dir);
    }

    #[tokio::test]
    async fn pull_outcomes_set_the_accounts_gauge() {
        use crate::ops::RecordingOps as Ops;
        let ops = Arc::new(Ops::new());
        ops.set_change_in_test_arm(true);
        ops.script_start(vec![
            Ok("exp-1".into()),
            Ok("exp-2".into()),
            Ok("exp-3".into()),
            Ok("exp-4".into()),
        ]);
        for id in ["exp-1", "exp-2", "exp-3", "exp-4"] {
            ops.put_experiment(
                id,
                ExperimentResult {
                    experiment_id: id.into(),
                    state: ExperimentState::Running,
                    results: vec![],
                    lift_percent_bp: None,
                    confidence: None,
                    winner_variant_id: None,
                },
            );
        }
        let now = NOW + 1;
        let range_end = crate::calendar::format_day(now.div_euclid(86_400_000) - 1);
        let earlier =
            crate::calendar::format_day(crate::calendar::day_number(&range_end).expect("day") - 1);
        let partial_days = ExperimentDays {
            control: vec![arm_day(&earlier, 4_000_000)],
            test: vec![arm_day(&earlier, 4_400_000)],
        };
        // Pull order is ledger order: final coverage, an auth refusal (the
        // credential is disconnected), a schema failure (the account stays
        // stale), and coverage that stops before the last closed day. The
        // daily pass performs exactly one reporting read per experiment; the
        // settlement reuses that read.
        ops.script_experiment_days(vec![
            Ok(constant_days(4_000_000, 4_400_000)),
            Err(apex_core::PlatformError::Auth {
                platform: apex_core::PlatformId::AppLovinMax,
                operation: "experimentDays".into(),
                status: 401,
            }),
            Err(apex_core::PlatformError::Schema {
                platform: apex_core::PlatformId::AppLovinMax,
                operation: "experimentDays".into(),
                detail: "malformed".into(),
            }),
            Ok(partial_days.clone()),
        ]);
        let proposals = ["ee", "fr", "de", "se"]
            .into_iter()
            .map(|country| crate::model::Proposal {
                action: crate::model::Action::SetCountryFloor {
                    ad_unit_id: "u1".into(),
                    country: country.into(),
                    floor_ecpm: 2_500_000,
                },
                rationale: format!("raise {country}"),
                expected_effect_bp: 500,
                prev_value: "{}".into(),
            })
            .collect();
        let cycle_policy = AutopilotPolicy {
            max_actions_per_day: 8,
            ..policy()
        };
        let mut l = ledger();
        run_cycle(ops.as_ref(), &mut l, &cycle_policy, &[], proposals, NOW)
            .await
            .expect("cycle");

        let rig = rig(ops.clone(), "readiness").await;
        let report = rig.runner.run_daily(&mut l, now).await.expect("daily");
        assert!(report.pulled >= 1);
        let gauge = |class: &str| {
            rig.registry
                .gauge_value(
                    &apex_metrics::catalog::REPORT_ACCOUNTS,
                    &[("platform", "applovin_max"), ("readiness_class", class)],
                )
                .expect("gauge")
        };
        assert_eq!(gauge("final"), 1.0, "coverage through the last closed day");
        assert_eq!(gauge("partial"), 1.0, "coverage stops earlier");
        assert_eq!(
            gauge("stale"),
            1.0,
            "schema failure leaves the account stale"
        );
        assert_eq!(gauge("disconnected"), 1.0, "auth refusal disconnects");
        let _ = std::fs::remove_dir_all(&rig.dir);
    }

    #[tokio::test]
    async fn the_aa_start_emits_experiment_started() {
        use apex_analytics::store::{EventStore as _, ProjectionStore as _};
        let ops = Arc::new(RecordingOps::new());
        let store = Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        let client = apex_analytics::AnalyticsClient::new(store.clone(), "dev", "apex-runner-test");
        let events = Arc::new(
            crate::events::RunnerEvents::new(Arc::new(client), "northstar").with_environment("dev"),
        );
        let runner = Runner::new(ops, policy(), FillFormat::Rewarded).with_events(events);
        let mut l = ledger();
        let spec = apex_core::ExperimentSpec {
            name: "apex-aa-u1".into(),
            instance_id: String::new(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "A/A calibration".into(),
            test_group_allocation: 50,
        };
        runner
            .start_aa(&mut l, apex_core::PlatformId::AppLovinMax, &spec, NOW)
            .await
            .expect("aa");
        let stored = store.load_events().await.expect("events");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].event_type.as_str(), "experiment_started");
        assert_eq!(stored[0].publisher_id.as_deref(), Some("northstar"));
        assert_eq!(
            stored[0].payload.experiment_id.as_deref(),
            Some("apex-aa-u1")
        );
        assert_eq!(stored[0].payload.state.as_deref(), Some("running"));
    }

    #[tokio::test]
    async fn the_daily_payment_pass_consumes_only_new_events() {
        let dir = std::env::temp_dir().join(format!("apex-runner-payments-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let collection = dir.join("collection.jsonl");
        let event = |psp: &str| apex_billing::CollectionEvent {
            at_unix_ms: NOW,
            payment_id: psp.into(),
            reference: "apex-fee:northstar:ch_0246:2026-10".into(),
            status: "paid".into(),
            amount_minor_units: 300,
            amount_currency: "USD".into(),
            stored_customer_id: String::new(),
            stored_mandate_id: String::new(),
        };
        let append = |events: &[apex_billing::CollectionEvent]| {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&collection)
                .unwrap();
            for event in events {
                writeln!(file, "{}", serde_json::to_string(event).unwrap()).unwrap();
            }
        };
        append(&[event("psp_1"), event("psp_2")]);
        let ops = Arc::new(RecordingOps::new());
        let runner = Runner::new(ops, policy(), FillFormat::Rewarded)
            .with_publisher("northstar")
            .with_payment_events(collection.clone(), None, None);
        let mut l = ledger();
        let first = runner.run_daily(&mut l, NOW).await.expect("first");
        assert_eq!(first.payments.expect("payments").received, 2);
        let cursor = crate::payment_events::default_cursor_path(&collection);
        assert!(cursor.exists(), "the offset cursor advanced");
        assert_eq!(
            crate::payment_events::read_consumed_offset(&cursor).expect("cursor"),
            std::fs::metadata(&collection).expect("metadata").len()
        );

        append(&[event("psp_3")]);
        let second = runner.run_daily(&mut l, NOW + 1).await.expect("second");
        assert_eq!(
            second.payments.expect("payments").received,
            1,
            "only the new event was processed"
        );
        let third = runner.run_daily(&mut l, NOW + 2).await.expect("third");
        assert_eq!(third.payments.expect("payments").received, 0);

        // A rewound cursor re-processes the file safely (the emits are
        // dedup-keyed; consuming nothing would be the real bug).
        crate::payment_events::write_consumed_offset(&cursor, 0).expect("rewind");
        let rewound = runner.run_daily(&mut l, NOW + 3).await.expect("rewound");
        assert_eq!(rewound.payments.expect("payments").received, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A ledger wrapper that counts `records()` scans, for structural
    /// assertions that a pass does not rescan the ledger per job.
    #[derive(Debug)]
    struct CountingLedger {
        inner: InMemoryLedger,
        scans: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl apex_proof::LedgerLog for CountingLedger {
        fn append(&mut self, draft: LedgerRecord) -> Result<LedgerRecord, apex_proof::LedgerError> {
            self.inner.append(draft)
        }

        fn append_rollback(
            &mut self,
            at_unix_ms: i64,
            platform: apex_core::PlatformId,
            ad_unit_id: &str,
            rolled_back_seq: u64,
            reason: &str,
        ) -> Result<LedgerRecord, apex_proof::LedgerError> {
            self.inner
                .append_rollback(at_unix_ms, platform, ad_unit_id, rolled_back_seq, reason)
        }

        fn records(&self) -> Vec<&LedgerRecord> {
            self.scans.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.records()
        }

        fn len(&self) -> usize {
            self.inner.len()
        }

        fn get(&self, index: usize) -> Option<&LedgerRecord> {
            self.inner.get(index)
        }

        fn is_empty(&self) -> bool {
            self.inner.is_empty()
        }
    }

    #[tokio::test]
    async fn the_due_recheck_scan_runs_once_per_pass() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = std::env::temp_dir().join(format!("apex-runner-due-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (id, target) in [("opj_00000000000d", "foot"), ("opj_00000000000e", "bar")] {
            let job = crate::operator_jobs::OperatorJob {
                job_id: id.to_string(),
                kind: "recheck_request".to_string(),
                requested_by: "maya".to_string(),
                requested_at_ms: NOW,
                target: target.to_string(),
                payload: serde_json::json!({}),
                idempotency_key: format!("k-{id}"),
                state: "queued".to_string(),
                state_history: vec![],
                outcome: serde_json::Value::Null,
                correlation_id: "corr_1".to_string(),
                owner: None,
                lease_expires_at_ms: None,
                attempts: 0,
            };
            std::fs::write(
                dir.join(format!("{id}.json")),
                serde_json::to_string_pretty(&job).unwrap(),
            )
            .unwrap();
        }

        // Baseline: a pass without jobs scans the ledger once (running
        // experiments only).
        let baseline_scans = std::sync::Arc::new(AtomicUsize::new(0));
        let mut baseline = CountingLedger {
            inner: ledger(),
            scans: baseline_scans.clone(),
        };
        let ops = Arc::new(RecordingOps::new());
        Runner::new(ops, policy(), FillFormat::Rewarded)
            .run_daily(&mut baseline, NOW)
            .await
            .expect("baseline");
        assert_eq!(baseline_scans.load(Ordering::SeqCst), 1);

        // Two recheck jobs add exactly one scan (the shared due set), never
        // one per job.
        let scans = std::sync::Arc::new(AtomicUsize::new(0));
        let mut with_jobs = CountingLedger {
            inner: ledger(),
            scans: scans.clone(),
        };
        let ops = Arc::new(RecordingOps::new());
        let runner =
            Runner::new(ops, policy(), FillFormat::Rewarded).with_operator_jobs(dir.clone());
        let report = runner.run_daily(&mut with_jobs, NOW).await.expect("daily");
        let jobs = report.operator_jobs.expect("jobs");
        assert_eq!(jobs.completed, 2, "both recheck jobs ran");
        assert_eq!(
            scans.load(Ordering::SeqCst),
            baseline_scans.load(Ordering::SeqCst) + 1,
            "the due set is computed once and reused for every job"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- scan-scaling regressions -------------------------------------------
    //
    // A read-only ledger double over directly constructed records: the
    // synthetic fixtures below hold ~10k records, and sealing that many
    // through `InMemoryLedger` (which re-verifies the full chain per append)
    // would cost minutes. The scans under test only read record fields.

    struct VecLedger {
        records: Vec<LedgerRecord>,
    }

    impl apex_proof::LedgerLog for VecLedger {
        fn append(
            &mut self,
            _draft: LedgerRecord,
        ) -> Result<LedgerRecord, apex_proof::LedgerError> {
            panic!("read-only test ledger")
        }

        fn append_rollback(
            &mut self,
            _at_unix_ms: i64,
            _platform: apex_core::PlatformId,
            _ad_unit_id: &str,
            _rolled_back_seq: u64,
            _reason: &str,
        ) -> Result<LedgerRecord, apex_proof::LedgerError> {
            panic!("read-only test ledger")
        }

        fn records(&self) -> Vec<&LedgerRecord> {
            self.records.iter().collect()
        }

        fn len(&self) -> usize {
            self.records.len()
        }

        fn get(&self, index: usize) -> Option<&LedgerRecord> {
            self.records.get(index)
        }

        fn is_empty(&self) -> bool {
            self.records.is_empty()
        }
    }

    /// A writable variant of [`VecLedger`]: appends get the next sequence
    /// without sealing a chain, and `records()` counts reads. The settle-loop
    /// scaling test needs both a 10k-record fixture and per-pass append
    /// accounting; `InMemoryLedger` re-verifies the whole chain per append.
    struct CountedVecLedger {
        records: Vec<LedgerRecord>,
        next_seq: u64,
        scans: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl CountedVecLedger {
        fn new(records: Vec<LedgerRecord>) -> Self {
            let next_seq = records
                .iter()
                .map(|record| record.seq() + 1)
                .max()
                .unwrap_or(0);
            Self {
                records,
                next_seq,
                scans: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }

        fn scans(&self) -> usize {
            self.scans.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    fn reseq(record: LedgerRecord, seq: u64) -> LedgerRecord {
        match record {
            LedgerRecord::PromotionPending {
                at_unix_ms,
                platform,
                experiment_id,
                ad_unit_id,
                change_seq,
                deadline_unix_ms,
                instructions,
                ..
            } => LedgerRecord::PromotionPending {
                seq,
                at_unix_ms,
                platform,
                experiment_id,
                ad_unit_id,
                change_seq,
                deadline_unix_ms,
                instructions,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::ChangeApplied {
                at_unix_ms,
                platform,
                ad_unit_id,
                change,
                idempotency_key,
                prev_value,
                ..
            } => LedgerRecord::ChangeApplied {
                seq,
                at_unix_ms,
                platform,
                ad_unit_id,
                change,
                idempotency_key,
                prev_value,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::ExperimentStarted {
                at_unix_ms,
                platform,
                experiment_id,
                spec,
                ..
            } => LedgerRecord::ExperimentStarted {
                seq,
                at_unix_ms,
                platform,
                experiment_id,
                spec,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::ExperimentObserved {
                at_unix_ms,
                platform,
                experiment_id,
                result,
                ..
            } => LedgerRecord::ExperimentObserved {
                seq,
                at_unix_ms,
                platform,
                experiment_id,
                result,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::LiftRecord {
                at_unix_ms,
                payload_sha256,
                payload,
                ..
            } => LedgerRecord::LiftRecord {
                seq,
                at_unix_ms,
                payload_sha256,
                payload,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::RollbackPerformed {
                at_unix_ms,
                platform,
                ad_unit_id,
                rolled_back_seq,
                reason,
                ..
            } => LedgerRecord::RollbackPerformed {
                seq,
                at_unix_ms,
                platform,
                ad_unit_id,
                rolled_back_seq,
                reason,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::BillingProof {
                at_unix_ms,
                publisher_id,
                experiment_id,
                proven_lift,
                rate_bp,
                fee_micros,
                ..
            } => LedgerRecord::BillingProof {
                seq,
                at_unix_ms,
                publisher_id,
                experiment_id,
                proven_lift,
                rate_bp,
                fee_micros,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::ExperimentCreateIntent {
                at_unix_ms,
                platform,
                experiment_id,
                ad_unit_id,
                idempotency_key,
                change,
                prev_value,
                spec,
                ..
            } => LedgerRecord::ExperimentCreateIntent {
                seq,
                at_unix_ms,
                platform,
                experiment_id,
                ad_unit_id,
                idempotency_key,
                change,
                prev_value,
                spec,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::ExperimentCreated {
                at_unix_ms,
                platform,
                experiment_id,
                external_id,
                ..
            } => LedgerRecord::ExperimentCreated {
                seq,
                at_unix_ms,
                platform,
                experiment_id,
                external_id,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::ExperimentCreateAbandoned {
                at_unix_ms,
                platform,
                experiment_id,
                reason,
                ..
            } => LedgerRecord::ExperimentCreateAbandoned {
                seq,
                at_unix_ms,
                platform,
                experiment_id,
                reason,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::AaQualification {
                at_unix_ms,
                publisher,
                platform,
                srm_chi_square_micros,
                label_consistency_micros,
                estimated_null_bias_micros,
                expires_at_unix_ms,
                ..
            } => LedgerRecord::AaQualification {
                seq,
                at_unix_ms,
                publisher,
                platform,
                srm_chi_square_micros,
                label_consistency_micros,
                estimated_null_bias_micros,
                expires_at_unix_ms,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
            LedgerRecord::SettingsReadFailed {
                at_unix_ms,
                platform,
                experiment_id,
                ad_unit_id,
                ..
            } => LedgerRecord::SettingsReadFailed {
                seq,
                at_unix_ms,
                platform,
                experiment_id,
                ad_unit_id,
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
        }
    }

    impl apex_proof::LedgerLog for CountedVecLedger {
        fn append(&mut self, draft: LedgerRecord) -> Result<LedgerRecord, apex_proof::LedgerError> {
            let seq = self.next_seq;
            self.next_seq += 1;
            let record = reseq(draft, seq);
            self.records.push(record.clone());
            Ok(record)
        }

        fn append_rollback(
            &mut self,
            at_unix_ms: i64,
            platform: apex_core::PlatformId,
            ad_unit_id: &str,
            rolled_back_seq: u64,
            reason: &str,
        ) -> Result<LedgerRecord, apex_proof::LedgerError> {
            let seq = self.next_seq;
            self.next_seq += 1;
            let record = LedgerRecord::RollbackPerformed {
                seq,
                at_unix_ms,
                platform,
                ad_unit_id: ad_unit_id.to_string(),
                rolled_back_seq,
                reason: reason.to_string(),
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            };
            self.records.push(record.clone());
            Ok(record)
        }

        fn records(&self) -> Vec<&LedgerRecord> {
            self.scans.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.records.iter().collect()
        }

        fn len(&self) -> usize {
            self.records.len()
        }

        fn get(&self, index: usize) -> Option<&LedgerRecord> {
            self.records.get(index)
        }

        fn is_empty(&self) -> bool {
            self.records.is_empty()
        }
    }

    fn vec_lift(seq: u64, record: apex_lift::records::LiftRecord) -> LedgerRecord {
        LedgerRecord::LiftRecord {
            seq,
            at_unix_ms: NOW + seq as i64,
            payload_sha256: record.sha256_hex().expect("sha"),
            payload: record.canonical_json().expect("canonical"),
            prev_hash: [0u8; 32],
            hash: [0u8; 32],
        }
    }

    fn vec_prereg(
        experiment_id: &str,
        ad_unit_id: &str,
        before_value: i64,
        rate_bp: u16,
    ) -> apex_lift::records::LiftRecord {
        apex_lift::records::LiftRecord::PreRegistration {
            experiment_id: experiment_id.to_string(),
            instance_id: String::new(),
            scope: ad_unit_id.to_string(),
            before_value: serde_json::json!({ "floor": before_value }),
            after_value: serde_json::json!({ "change": "raise the EE floor" }),
            split: 0.25,
            metric: "revenue_per_user".into(),
            burn_in_days: 3,
            window_days: 14,
            looks: 1,
            extension_allowed: false,
            guardrail_thresholds: serde_json::json!({}),
            code_version_hash: "deadbeef".into(),
            rate_bp,
            min_point_lift: 0.01,
        }
    }

    fn vec_result(
        experiment_id: &str,
        decision: &str,
        lambda: f64,
    ) -> apex_lift::records::LiftRecord {
        apex_lift::records::LiftRecord::Result {
            experiment_id: experiment_id.to_string(),
            instance_id: String::new(),
            daily_log_ratios: vec![0.01, 0.02],
            mean_log: 0.015,
            lower_90: 0.005,
            lower_99: 0.002,
            bootstrap_lower: 0.004,
            decision: decision.into(),
            lambda,
        }
    }

    /// A synthetic ledger of `experiments` experiments and `pulls` data-pull
    /// records each: change, pre-registration, start, pulls, and — for every
    /// second experiment — an observation plus a keep result. Every seventh
    /// experiment's change is rolled back, one experiment carries a duplicate
    /// pre-registration (first wins) and one an out-of-band rate (skipped by
    /// the fee preview). Total records scale as `experiments * (pulls + 3)`.
    fn synthetic_ledger(experiments: usize, pulls: usize) -> VecLedger {
        let mut records = Vec::new();
        let mut seq = 0u64;
        macro_rules! push {
            ($record:expr) => {{
                records.push($record);
                seq += 1;
            }};
        }
        for e in 0..experiments {
            let name = format!("floors-ee-u{e}");
            let unit = format!("u{e}");
            let change_seq = seq;
            push!(LedgerRecord::ChangeApplied {
                seq,
                at_unix_ms: NOW,
                platform: apex_core::PlatformId::AppLovinMax,
                ad_unit_id: unit.clone(),
                change: ChangeKind::CountryFloorSet {
                    country: "EE".into(),
                    floor_ecpm: 2_500_000,
                },
                idempotency_key: format!("key-{e}"),
                prev_value: String::new(),
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            });
            let rate = if e == 5 { 3_000 } else { 2_000 };
            push!(vec_lift(seq, vec_prereg(&name, &unit, e as i64, rate)));
            if e == 5 {
                // Duplicate registration with an in-band rate: the scans must
                // keep the first (out-of-band) one, exactly like `find_map`.
                push!(vec_lift(seq, vec_prereg(&name, &unit, e as i64, 2_000)));
            }
            push!(LedgerRecord::ExperimentStarted {
                seq,
                at_unix_ms: NOW + 86_400_000,
                platform: apex_core::PlatformId::AppLovinMax,
                experiment_id: name.clone(),
                spec: apex_core::ExperimentSpec {
                    name: name.clone(),
                    instance_id: String::new(),
                    ad_unit_id: unit.clone(),
                    metric: apex_core::ExperimentMetric::Ecpm,
                    variants: 2,
                    test_group_allocation: 25,
                    change_description: "raise the EE floor".into(),
                },
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            });
            for pull in 0..pulls {
                push!(vec_lift(
                    seq,
                    apex_lift::records::LiftRecord::DataPull {
                        day: format!("2026-{:02}-{:02}", 1 + pull / 28, 1 + pull % 28),
                        rows_sha256: "0".repeat(64),
                        rows: serde_json::json!([]),
                    },
                ));
            }
            if e % 2 == 0 {
                push!(LedgerRecord::ExperimentObserved {
                    seq,
                    at_unix_ms: NOW + 90 * 86_400_000,
                    platform: apex_core::PlatformId::AppLovinMax,
                    experiment_id: name.clone(),
                    result: ExperimentResult {
                        experiment_id: name.clone(),
                        state: ExperimentState::Completed,
                        results: vec![],
                        lift_percent_bp: None,
                        confidence: None,
                        winner_variant_id: None,
                    },
                    prev_hash: [0u8; 32],
                    hash: [0u8; 32],
                });
                push!(vec_lift(seq, vec_result(&name, "keep", 0.02)));
                if e == 12 {
                    // First keep result wins, exactly like `find_map`.
                    push!(vec_lift(seq, vec_result(&name, "keep", 0.09)));
                }
            }
            if e % 7 == 3 {
                push!(LedgerRecord::RollbackPerformed {
                    seq,
                    at_unix_ms: NOW + 90 * 86_400_000,
                    platform: apex_core::PlatformId::AppLovinMax,
                    ad_unit_id: unit,
                    rolled_back_seq: change_seq,
                    reason: "regression".into(),
                    prev_hash: [0u8; 32],
                    hash: [0u8; 32],
                });
            }
        }
        VecLedger { records }
    }

    /// The pre-index shape of `running_scan`'s two helpers, kept verbatim as
    /// the equivalence reference.
    fn naive_running_experiments(ledger: &dyn LedgerLog) -> Vec<RunningExperiment> {
        let records = ledger.records();
        let pre_registered: std::collections::HashSet<String> = records
            .iter()
            .filter_map(|record| match record {
                LedgerRecord::LiftRecord { payload, .. } => parse_lift_record(payload)
                    .ok()
                    .and_then(|parsed| match parsed {
                        apex_lift::records::LiftRecord::PreRegistration {
                            experiment_id, ..
                        } => Some(experiment_id),
                        _ => None,
                    }),
                _ => None,
            })
            .collect();
        let settled: std::collections::HashSet<String> = records
            .iter()
            .filter_map(|record| match record {
                LedgerRecord::ExperimentObserved { experiment_id, .. } => {
                    Some(experiment_id.clone())
                }
                _ => None,
            })
            .collect();
        let rolled_back: std::collections::HashSet<u64> = records
            .iter()
            .filter_map(|record| match record {
                LedgerRecord::RollbackPerformed {
                    rolled_back_seq, ..
                } => Some(*rolled_back_seq),
                _ => None,
            })
            .collect();
        records
            .iter()
            .filter_map(|record| match record {
                LedgerRecord::ExperimentStarted {
                    seq,
                    at_unix_ms,
                    experiment_id,
                    spec,
                    ..
                } => {
                    if !pre_registered.contains(&spec.name) {
                        return None;
                    }
                    let change_seq = records
                        .iter()
                        .filter(|candidate| candidate.seq() < *seq)
                        .filter_map(|candidate| match candidate {
                            LedgerRecord::ChangeApplied { seq, .. } => Some(*seq),
                            _ => None,
                        })
                        .max()?;
                    if settled.contains(experiment_id) || rolled_back.contains(&change_seq) {
                        return None;
                    }
                    let expected_settings = records
                        .iter()
                        .find_map(|record| match record {
                            LedgerRecord::LiftRecord { payload, .. } => {
                                let parsed = parse_lift_record(payload).ok()?;
                                let prereg =
                                    apex_lift::decision::PreRegistration::from_record(&parsed)
                                        .ok()?;
                                (prereg.experiment_id == spec.name).then_some(prereg.before_value)
                            }
                            _ => None,
                        })
                        .unwrap_or_default();
                    let start_day = crate::calendar::format_day(at_unix_ms.div_euclid(86_400_000));
                    let change = records.iter().find_map(|record| match record {
                        LedgerRecord::ChangeApplied { seq, change, .. } if *seq == change_seq => {
                            Some(change.clone())
                        }
                        _ => None,
                    });
                    Some(RunningExperiment {
                        experiment_id: experiment_id.clone(),
                        instance_id: instance_or_name(&spec.instance_id, &spec.name),
                        instance_key: instance_or_name(&spec.instance_id, &spec.name),
                        change_seq,
                        started_day: start_day,
                        ad_unit_id: spec.ad_unit_id.clone(),
                        expected_settings,
                        platform: apex_core::PlatformId::AppLovinMax,
                        change,
                        pending_terminal: false,
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// The old per-experiment `started_experiment_name` scan, kept verbatim
    /// as the index equivalence reference.
    fn naive_started_experiment_name(
        ledger: &dyn LedgerLog,
        experiment_id: &str,
    ) -> Option<String> {
        ledger.records().iter().find_map(|record| match record {
            LedgerRecord::ExperimentStarted {
                experiment_id: e,
                spec,
                ..
            } if e == experiment_id => Some(spec.name.clone()),
            _ => None,
        })
    }

    /// The old per-experiment signed pre-registration scan, kept as the
    /// index equivalence reference. It resolves the immutable instance of the
    /// start paired with `change_seq`.
    fn naive_signed_pre_registration(
        ledger: &dyn LedgerLog,
        experiment_id: &str,
        change_seq: u64,
    ) -> Result<apex_lift::decision::PreRegistration, PlanError> {
        let instance =
            naive_started_instance(ledger, experiment_id, change_seq).ok_or_else(|| {
                PlanError::NoBaseline {
                    reason: format!("experiment {experiment_id} was never started in this ledger"),
                }
            })?;
        ledger
            .records()
            .iter()
            .find_map(|record| match record {
                LedgerRecord::LiftRecord { payload, .. } => {
                    let parsed = parse_lift_record(payload).ok()?;
                    let prereg = apex_lift::decision::PreRegistration::from_record(&parsed).ok()?;
                    (instance_or_name(&prereg.instance_id, &prereg.experiment_id) == instance)
                        .then_some(prereg)
                }
                _ => None,
            })
            .ok_or(PlanError::NoBaseline {
                reason: format!("no signed pre-registration for experiment instance {instance}"),
            })
    }

    /// The immutable instance of the start record for `experiment_id` whose
    /// preceding applied change is `change_seq`.
    fn naive_started_instance(
        ledger: &dyn LedgerLog,
        experiment_id: &str,
        change_seq: u64,
    ) -> Option<String> {
        let mut last_change_seq: Option<u64> = None;
        for record in ledger.records() {
            match record {
                LedgerRecord::ChangeApplied { seq, .. } => {
                    last_change_seq = Some(last_change_seq.map_or(*seq, |seen| seen.max(*seq)));
                }
                LedgerRecord::ExperimentStarted {
                    experiment_id: e,
                    spec,
                    ..
                } if e == experiment_id && last_change_seq == Some(change_seq) => {
                    return Some(instance_or_name(&spec.instance_id, &spec.name));
                }
                _ => {}
            }
        }
        None
    }

    /// The old per-experiment settled-refusal scan, kept as the index
    /// equivalence reference.
    fn naive_settled_refusal(
        ledger: &dyn LedgerLog,
        experiment_id: &str,
        change_seq: u64,
    ) -> Option<PlanError> {
        let instance = naive_started_instance(ledger, experiment_id, change_seq)
            .or_else(|| naive_started_experiment_name(ledger, experiment_id))
            .unwrap_or_default();
        let settled = ledger.records().iter().any(|record| match record {
            LedgerRecord::RollbackPerformed {
                rolled_back_seq, ..
            } => *rolled_back_seq == change_seq,
            LedgerRecord::LiftRecord { payload, .. } => {
                parse_lift_record(payload).ok().is_some_and(|record| {
                    matches!(
                        record,
                        apex_lift::records::LiftRecord::Result {
                            experiment_id,
                            instance_id,
                            ..
                        } if instance_or_name(&instance_id, &experiment_id) == instance
                    )
                })
            }
            _ => false,
        });
        settled.then(|| {
            PlanError::Validation(format!("experiment {experiment_id} is already settled"))
        })
    }

    /// The pre-index shape of `kept_changes`, kept verbatim as the equivalence
    /// reference.
    fn naive_kept_changes(
        ledger: &dyn LedgerLog,
    ) -> Vec<(
        String,
        apex_core::PlatformId,
        apex_lift::billing::KeptChange,
    )> {
        let records = ledger.records();
        let mut out = Vec::new();
        for record in &records {
            let LedgerRecord::ExperimentStarted {
                seq,
                spec,
                platform,
                ..
            } = record
            else {
                continue;
            };
            let instance = instance_or_name(&spec.instance_id, &spec.name);
            let change_seq = records
                .iter()
                .filter(|candidate| candidate.seq() < *seq)
                .filter_map(|candidate| match candidate {
                    LedgerRecord::ChangeApplied { seq, .. } => Some(*seq),
                    _ => None,
                })
                .max();
            let pre = records.iter().find_map(|candidate| match candidate {
                LedgerRecord::LiftRecord {
                    payload_sha256,
                    payload,
                    ..
                } => match parse_lift_record(payload).ok() {
                    Some(apex_lift::records::LiftRecord::PreRegistration {
                        experiment_id,
                        instance_id,
                        rate_bp,
                        ..
                    }) if instance_or_name(&instance_id, &experiment_id) == instance => {
                        Some((payload_sha256.clone(), rate_bp))
                    }
                    _ => None,
                },
                _ => None,
            });
            let result = records.iter().find_map(|candidate| match candidate {
                LedgerRecord::LiftRecord {
                    at_unix_ms,
                    payload_sha256,
                    payload,
                    ..
                } => match parse_lift_record(payload).ok() {
                    Some(apex_lift::records::LiftRecord::Result {
                        experiment_id,
                        instance_id,
                        decision,
                        lambda,
                        ..
                    }) if instance_or_name(&instance_id, &experiment_id) == instance
                        && decision == "keep"
                        && lambda > 0.0 =>
                    {
                        Some((*at_unix_ms, payload_sha256.clone(), lambda))
                    }
                    _ => None,
                },
                _ => None,
            });
            let (Some((pre_hash, rate_bp)), Some((at_unix_ms, result_hash, lambda))) =
                (pre, result)
            else {
                continue;
            };
            if !(apex_core::money::MIN_LIFT_RATE_BP..=apex_core::money::MAX_LIFT_RATE_BP)
                .contains(&rate_bp)
                && rate_bp != apex_core::money::RESULTS_RATE_BP
            {
                continue;
            }
            let rolled_back_day = change_seq.and_then(|seq| {
                records.iter().find_map(|candidate| match candidate {
                    LedgerRecord::RollbackPerformed {
                        at_unix_ms,
                        rolled_back_seq,
                        ..
                    } if *rolled_back_seq == seq => Some(crate::calendar::format_day(
                        at_unix_ms.div_euclid(86_400_000),
                    )),
                    _ => None,
                })
            });
            let overridden_day = records.iter().find_map(|candidate| match candidate {
                LedgerRecord::LiftRecord { payload, .. } => {
                    match parse_lift_record(payload).ok()? {
                        apex_lift::records::LiftRecord::BillingChange(change)
                            if change.change_id == instance || change.change_id == spec.name =>
                        {
                            Some(change.day)
                        }
                        _ => None,
                    }
                }
                _ => None,
            });
            out.push((
                spec.ad_unit_id.clone(),
                *platform,
                apex_lift::billing::KeptChange {
                    change_id: instance,
                    lambda,
                    rate_bp,
                    kept_day: crate::calendar::format_day(at_unix_ms.div_euclid(86_400_000)),
                    overridden_day,
                    rolled_back_day,
                    pre_registration_hash: pre_hash,
                    result_hash,
                },
            ));
        }
        out
    }

    fn lift_record_count(ledger: &VecLedger) -> u64 {
        ledger
            .records
            .iter()
            .filter(|record| matches!(record, LedgerRecord::LiftRecord { .. }))
            .count() as u64
    }

    #[test]
    fn running_scan_parses_each_lift_record_once_at_ten_thousand_records() {
        let ledger = synthetic_ledger(210, 45);
        assert!(ledger.len() >= 10_000, "fixture size: {}", ledger.len());
        let parses = lift_record_count(&ledger);
        reset_parse_call_count();
        let scan = running_scan(&ledger).expect("scan");
        assert_eq!(
            parse_call_count(),
            parses,
            "one parse per signed payload, never per experiment times records"
        );
        // Equivalence with the pre-index shape over the same fixture, and the
        // count that shape needs: the regression bound above excludes it.
        reset_parse_call_count();
        let naive = naive_running_experiments(&ledger);
        let naive_parses = parse_call_count();
        assert_eq!(scan.experiments, naive);
        assert!(
            naive_parses > parses * 20,
            "the pre-index shape parses {naive_parses} payloads for {parses} lift records; the scan must not"
        );
        let mut naive_days = std::collections::BTreeMap::new();
        for record in &ledger.records {
            if let LedgerRecord::ExperimentStarted {
                experiment_id,
                at_unix_ms,
                ..
            } = record
            {
                naive_days.entry(experiment_id.clone()).or_insert_with(|| {
                    crate::calendar::format_day(at_unix_ms.div_euclid(86_400_000))
                });
            }
        }
        assert_eq!(scan.started_days, naive_days);
        assert!(
            scan.experiments.len() < 200,
            "settled and rolled-back experiments are excluded: {}",
            scan.experiments.len()
        );
    }

    #[test]
    fn kept_changes_parses_each_lift_record_once_at_ten_thousand_records() {
        let ledger = synthetic_ledger(210, 45);
        let parses = lift_record_count(&ledger);
        reset_parse_call_count();
        let kept = kept_changes(&ledger).expect("kept changes");
        assert_eq!(
            parse_call_count(),
            parses,
            "one parse per signed payload, never per experiment times records"
        );
        reset_parse_call_count();
        let naive = naive_kept_changes(&ledger);
        let naive_parses = parse_call_count();
        assert_eq!(kept, naive);
        assert!(
            naive_parses > parses * 20,
            "the pre-index shape parses {naive_parses} payloads for {parses} lift records; the single pass must not"
        );
        assert!(
            !kept.is_empty(),
            "the fixture's kept experiments flow through the preview set"
        );
    }

    #[tokio::test]
    async fn settle_refuses_an_already_settled_experiment_after_a_long_history() {
        // The settled guard must see the experiment's own terminal records
        // without rescanning the ledger inside its per-record walk (the old
        // shape re-resolved the registered name once per lift record).
        let mut ledger = synthetic_ledger(210, 45);
        let ops = RecordingOps::new();
        ops.put_experiment(
            "floors-ee-u0",
            ExperimentResult {
                experiment_id: "floors-ee-u0".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        let error = crate::settle::settle_experiment(
            &ops,
            &mut ledger,
            "floors-ee-u0",
            0,
            FillFormat::Rewarded,
            &crate::settle::EditCheck::VerifiedSame,
            NOW,
        )
        .await
        .expect_err("a settled experiment is refused");
        assert!(
            matches!(&error, PlanError::Validation(reason) if reason.contains("already settled")),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn settle_loop_reads_the_ledger_once_for_five_hundred_experiments() {
        let experiments = 500usize;
        let synthetic = synthetic_ledger(experiments, 17);
        assert!(
            synthetic.len() >= 10_000,
            "the fixture holds a whole history: {}",
            synthetic.len()
        );
        let mut ledger = CountedVecLedger::new(synthetic.records);
        let ops = RecordingOps::new();
        for e in 0..experiments {
            let id = format!("floors-ee-u{e}");
            ops.put_experiment(
                &id,
                ExperimentResult {
                    experiment_id: id.clone(),
                    state: ExperimentState::Running,
                    results: vec![],
                    lift_percent_bp: None,
                    confidence: None,
                    winner_variant_id: None,
                },
            );
            ops.put_experiment_days(&id, constant_days(4_000_000, 4_400_000));
        }

        // One index build serves the whole pass; then every iteration appends
        // its pull and settles against the same index.
        let mut index = LedgerIndex::build(&ledger);
        assert_eq!(ledger.scans(), 1, "the pass builds the index once");
        let starts: Vec<(String, u64)> = index
            .starts()
            .iter()
            .map(|start| {
                (
                    start.experiment_id.clone(),
                    start.change_seq.expect("every start follows a change"),
                )
            })
            .collect();
        assert_eq!(starts.len(), experiments);
        for (experiment_id, change_seq) in starts {
            let pull = apex_lift::records::LiftRecord::DataPull {
                day: "2026-10-01".into(),
                rows_sha256: "0".repeat(64),
                rows: serde_json::json!([]),
            };
            let record = ledger.append(vec_lift(0, pull)).expect("data pull");
            index.observe(&record);
            let _ = crate::settle::settle_experiment_indexed(
                &ops,
                &mut ledger,
                &mut index,
                &experiment_id,
                change_seq,
                FillFormat::Rewarded,
                &crate::settle::EditCheck::VerifiedSame,
                None,
                NOW,
            )
            .await;
        }
        assert_eq!(
            ledger.scans(),
            1,
            "settling 500 experiments never rescans the ledger"
        );
    }

    #[test]
    fn ledger_index_matches_the_per_experiment_scans_on_a_randomized_ledger() {
        let synthetic = synthetic_ledger(210, 45);
        let mut records = synthetic.records;
        // Deterministic xorshift shuffle: record order (and therefore every
        // first-match answer) is randomized without a test-only RNG crate.
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for i in (1..records.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let j = (state as usize) % (i + 1);
            records.swap(i, j);
        }
        let ledger = VecLedger { records };
        assert!(ledger.len() >= 10_000, "fixture size: {}", ledger.len());
        let index = LedgerIndex::build(&ledger);
        let mut compared = 0usize;
        for start in index.starts() {
            let id = &start.experiment_id;
            assert_eq!(
                index.started_experiment_name(id),
                naive_started_experiment_name(&ledger, id).as_deref(),
                "started name for {id}"
            );
            match (
                index.signed_pre_registration(id, start.change_seq.unwrap_or(0)),
                naive_signed_pre_registration(&ledger, id, start.change_seq.unwrap_or(0)),
            ) {
                (Ok(actual), Ok(expected)) => assert_eq!(actual, expected, "prereg for {id}"),
                (Err(actual), Err(expected)) => {
                    assert_eq!(actual.to_string(), expected.to_string(), "prereg for {id}")
                }
                (actual, expected) => panic!("prereg for {id}: {actual:?} vs {expected:?}"),
            }
            for change_seq in [
                start.change_seq.unwrap_or(0),
                0,
                start.change_seq.unwrap_or(0) + 1,
            ] {
                let actual = index
                    .settled_refusal(id, change_seq)
                    .map(|error| error.to_string());
                let expected =
                    naive_settled_refusal(&ledger, id, change_seq).map(|error| error.to_string());
                assert_eq!(actual, expected, "settled refusal for {id} at {change_seq}");
            }
            compared += 1;
        }
        assert_eq!(compared, 210, "every start was compared");
    }

    #[test]
    fn ledger_index_build_is_deterministic_and_preserves_start_order() {
        let ledger = synthetic_ledger(60, 5);
        let first = LedgerIndex::build(&ledger);
        let second = LedgerIndex::build(&ledger);
        let ids = |index: &LedgerIndex| -> Vec<String> {
            index
                .starts()
                .iter()
                .map(|start| start.experiment_id.clone())
                .collect()
        };
        assert_eq!(ids(&first), ids(&second), "builds are deterministic");
        let expected: Vec<String> = ledger
            .records
            .iter()
            .filter_map(|record| match record {
                LedgerRecord::ExperimentStarted { experiment_id, .. } => {
                    Some(experiment_id.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(ids(&first), expected, "starts stay in ledger order");
        assert_eq!(first.started_days(), second.started_days());
        assert_eq!(
            first
                .starts()
                .iter()
                .map(|start| start.change_seq)
                .collect::<Vec<_>>(),
            second
                .starts()
                .iter()
                .map(|start| start.change_seq)
                .collect::<Vec<_>>()
        );
    }

    /// n/2n/4n evidence: the index is built once per pass and the settle loop
    /// is linear in experiments. Timings are printed, never asserted.
    // dogfood:skip: this evidence test is deliberately ignored in the default
    // suite and is run explicitly with `--ignored --nocapture`.
    #[tokio::test]
    #[ignore = "n/2n/4n ledger-index scaling evidence; run with --ignored --nocapture"]
    async fn ledger_index_scales_linearly_in_experiments_and_records() {
        for experiments in [125usize, 250, 500] {
            let synthetic = synthetic_ledger(experiments, 17);
            let records = synthetic.len();
            let lift_records = lift_record_count(&synthetic);
            let mut ledger = CountedVecLedger::new(synthetic.records);
            let ops = RecordingOps::new();
            for e in 0..experiments {
                let id = format!("floors-ee-u{e}");
                ops.put_experiment(
                    &id,
                    ExperimentResult {
                        experiment_id: id.clone(),
                        state: ExperimentState::Running,
                        results: vec![],
                        lift_percent_bp: None,
                        confidence: None,
                        winner_variant_id: None,
                    },
                );
                ops.put_experiment_days(&id, constant_days(4_000_000, 4_400_000));
            }

            crate::ledger_index::reset_index_metrics();
            let started = std::time::Instant::now();
            let mut index = LedgerIndex::build(&ledger);
            let build_ms = started.elapsed().as_secs_f64() * 1000.0;
            let (visits, parses, entries) = crate::ledger_index::index_metrics();
            assert_eq!(visits, records as u64);
            assert_eq!(parses, lift_records);

            let starts: Vec<(String, u64)> = index
                .starts()
                .iter()
                .map(|start| {
                    (
                        start.experiment_id.clone(),
                        start.change_seq.expect("change before start"),
                    )
                })
                .collect();
            crate::ledger_index::reset_index_metrics();
            let started = std::time::Instant::now();
            for (experiment_id, change_seq) in starts {
                let pull = apex_lift::records::LiftRecord::DataPull {
                    day: "2026-10-01".into(),
                    rows_sha256: "0".repeat(64),
                    rows: serde_json::json!([]),
                };
                let record = ledger.append(vec_lift(0, pull)).expect("pull");
                index.observe(&record);
                let _ = crate::settle::settle_experiment_indexed(
                    &ops,
                    &mut ledger,
                    &mut index,
                    &experiment_id,
                    change_seq,
                    FillFormat::Rewarded,
                    &crate::settle::EditCheck::VerifiedSame,
                    None,
                    NOW,
                )
                .await;
            }
            let settle_ms = started.elapsed().as_secs_f64() * 1000.0;
            let (settle_visits, settle_parses, settle_entries) =
                crate::ledger_index::index_metrics();
            eprintln!(
                "AUTOPILOT experiments={experiments:>4} records={records:>6} \
                 build={build_ms:8.1}ms settle={settle_ms:8.1}ms \
                 build[visits={visits} parses={parses} entries={entries}] \
                 loop[visits={settle_visits} parses={settle_parses} entries={settle_entries}] \
                 scans={}",
                ledger.scans()
            );
            assert_eq!(ledger.scans(), 1, "one scan for the whole pass");
        }
    }

    #[test]
    fn the_scan_fixture_pins_first_match_and_out_of_band_semantics() {
        let ledger = synthetic_ledger(16, 2);
        let kept = kept_changes(&ledger).expect("kept changes");
        // Experiment 5 registered twice with an out-of-band first rate: it is
        // skipped, not billed at the in-band duplicate's rate.
        assert!(kept
            .iter()
            .all(|(_, _, change)| change.change_id != "floors-ee-u5"));
        // Experiment 12's duplicate keep result: the first (0.02) wins.
        let twelfth = kept
            .iter()
            .find(|(_, _, change)| change.change_id == "floors-ee-u12")
            .expect("experiment 12 is kept");
        assert_eq!(twelfth.2.lambda, 0.02);
        // Experiment 10 (e % 7 == 3 and e % 2 == 0) is rolled back: the
        // running scan excludes it, the fee preview (which bills kept records
        // regardless of later rollbacks) still lists it exactly like the
        // pre-index reference did.
        assert!(kept
            .iter()
            .any(|(_, _, change)| change.change_id == "floors-ee-u10"));
        let scan = running_scan(&ledger).expect("scan");
        assert!(scan
            .experiments
            .iter()
            .all(|experiment| experiment.experiment_id != "floors-ee-u10"));
    }

    #[test]
    fn a_hostile_lift_payload_fails_running_scan_closed_at_ten_thousand_records() {
        // One hostile payload in a ~10k-record history must refuse the scan,
        // naming the record, instead of silently dropping it (and any
        // experiment whose pre-registration it was).
        let mut ledger = synthetic_ledger(210, 45);
        assert!(ledger.len() >= 10_000, "fixture size: {}", ledger.len());
        let hostile_seq = 777;
        ledger.records.insert(
            5_000,
            LedgerRecord::LiftRecord {
                seq: hostile_seq,
                at_unix_ms: NOW,
                payload_sha256: "0".repeat(64),
                payload: "{\"type\": \"preRegistration\"".into(),
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
        );
        let error = running_scan(&ledger).expect_err("a hostile payload fails closed");
        match error {
            PlanError::MalformedLiftRecord { seq, detail } => {
                assert_eq!(seq, hostile_seq, "the error names the record");
                assert!(detail.contains("line 1"), "{detail}");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn a_hostile_lift_payload_fails_kept_changes_closed_at_ten_thousand_records() {
        let mut ledger = synthetic_ledger(210, 45);
        let hostile_seq = 1_234;
        ledger.records.insert(
            7_000,
            LedgerRecord::LiftRecord {
                seq: hostile_seq,
                at_unix_ms: NOW,
                payload_sha256: "0".repeat(64),
                payload: "not json at all".into(),
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
        );
        let error = kept_changes(&ledger).expect_err("a hostile payload fails closed");
        match error {
            PlanError::MalformedLiftRecord { seq, detail } => {
                assert_eq!(seq, hostile_seq, "the error names the record");
                assert!(detail.contains("line 1"), "{detail}");
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn a_hostile_lift_payload_fails_due_rechecks_closed() {
        let mut ledger = synthetic_ledger(16, 2);
        ledger.records.insert(
            3,
            LedgerRecord::LiftRecord {
                seq: 999,
                at_unix_ms: NOW,
                payload_sha256: "0".repeat(64),
                payload: "{".into(),
                prev_hash: [0u8; 32],
                hash: [0u8; 32],
            },
        );
        let runner = Runner::new(
            Arc::new(RecordingOps::new()),
            policy(),
            FillFormat::Rewarded,
        );
        let error = runner
            .due_rechecks(&ledger, NOW)
            .expect_err("the recheck planner refuses a hostile payload");
        assert!(
            matches!(error, PlanError::MalformedLiftRecord { seq: 999, .. }),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn a_hostile_preregistration_fails_the_daily_pass_closed_without_writing() {
        // The hostile payload is the experiment's pre-registration: before the
        // fix the scan skipped it and the experiment silently vanished from
        // settlement while the pass returned Ok.
        let ops = Arc::new(RecordingOps::new());
        ops.put_experiment(
            "exp-hostile",
            ExperimentResult {
                experiment_id: "exp-hostile".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-hostile", constant_days(4_000_000, 4_400_000));

        let mut l = ledger();
        l.append(LedgerRecord::change_applied(
            NOW,
            apex_core::PlatformId::AppLovinMax,
            "u1",
            ChangeKind::CountryFloorSet {
                country: "EE".into(),
                floor_ecpm: 2_500_000,
            },
            "key-1",
            "",
        ))
        .expect("change");
        l.append(LedgerRecord::experiment_started(
            NOW,
            apex_core::PlatformId::AppLovinMax,
            "exp-hostile",
            experiment_spec("floors-ee-u1", "u1"),
        ))
        .expect("start");
        let hostile_seq = l.len() as u64;
        l.append(LedgerRecord::lift_record(
            NOW + 2,
            &"0".repeat(64),
            "{\"type\": \"preRegistration\"",
        ))
        .expect("hostile record");
        // Later history keeps the hostile payload mid-history.
        l.append(LedgerRecord::experiment_observed(
            NOW + 3,
            apex_core::PlatformId::AppLovinMax,
            "exp-hostile",
            ExperimentResult {
                experiment_id: "exp-hostile".into(),
                state: ExperimentState::Completed,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        ))
        .expect("observation");
        let before: Vec<u64> = l.records().iter().map(|record| record.seq()).collect();

        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let error = runner
            .run_daily(&mut l, NOW + 1)
            .await
            .expect_err("a hostile payload must fail the pass closed");
        match error {
            PlanError::MalformedLiftRecord { seq, detail } => {
                assert_eq!(seq, hostile_seq, "the error names the record");
                assert!(detail.contains("line 1"), "{detail}");
            }
            other => panic!("unexpected error: {other}"),
        }
        let after: Vec<u64> = l.records().iter().map(|record| record.seq()).collect();
        assert_eq!(after, before, "the refused pass appends nothing");
    }

    #[test]
    fn hostile_text_in_a_non_lift_record_is_still_ignored_by_both_scans() {
        // Only LiftRecord payloads are parsed by the scans. A hostile string
        // in a non-lift record's field is not a payload and changes nothing.
        let mut ledger = synthetic_ledger(16, 2);
        for record in &mut ledger.records {
            if let LedgerRecord::ChangeApplied { prev_value, .. } = record {
                *prev_value = "{\"hostile\":".into();
            }
        }
        let kept = kept_changes(&ledger).expect("non-lift text is not a lift payload");
        assert!(!kept.is_empty(), "the valid history still scans");
        let scan = running_scan(&ledger).expect("non-lift text is not a lift payload");
        assert!(
            !scan.experiments.is_empty(),
            "the valid history still scans"
        );
    }

    #[tokio::test]
    async fn reconciled_terminal_outcomes_stay_measured() {
        for (decision, lambda) in [("keep", 0.02), ("rollback", 0.0)] {
            let ops = Arc::new(RecordingOps::new());
            let mut l = ledger();
            let seq = l
                .append(LedgerRecord::change_applied(
                    NOW,
                    apex_core::PlatformId::AppLovinMax,
                    "u1",
                    ChangeKind::CountryFloorSet {
                        country: "EE".into(),
                        floor_ecpm: 2_500_000,
                    },
                    "key-measured",
                    "",
                ))
                .expect("change")
                .seq();
            l.append(LedgerRecord::experiment_started(
                NOW - 10 * 86_400_000,
                apex_core::PlatformId::AppLovinMax,
                "floors-ee-u1",
                experiment_spec("floors-ee-u1", "u1"),
            ))
            .expect("start");
            append_lift(&mut l, vec_prereg("floors-ee-u1", "u1", 1, 2_000), NOW + 1);
            append_lift(
                &mut l,
                vec_result("floors-ee-u1", decision, lambda),
                NOW + 2,
            );
            let mut index = LedgerIndex::build(&l);
            let outcome = settle_experiment_indexed(
                ops.as_ref(),
                &mut l,
                &mut index,
                "floors-ee-u1",
                seq,
                FillFormat::Rewarded,
                &crate::settle::EditCheck::VerifiedSame,
                None,
                NOW + 3,
            )
            .await
            .expect("reconcile");
            match outcome {
                SettleOutcome::Kept { unmeasured, .. } => {
                    assert!(!unmeasured, "a reconciled keep is still measured");
                }
                SettleOutcome::RolledBack { unmeasured, .. } => {
                    assert!(!unmeasured, "a reconciled rollback is still measured");
                }
                other => panic!("unexpected outcome: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_pending_terminal_proof_is_reconciled_after_a_crash() {
        for (decision, lambda, expected) in [
            ("keep", 0.02, "kept"),
            ("rollback", 0.0, "rolled_back"),
            ("void", 0.0, "voided"),
        ] {
            let ops = Arc::new(RecordingOps::new());
            ops.set_measures_variants(true);
            ops.put_experiment(
                "floors-ee-u1",
                ExperimentResult {
                    experiment_id: "floors-ee-u1".into(),
                    state: ExperimentState::Running,
                    results: vec![],
                    lift_percent_bp: None,
                    confidence: None,
                    winner_variant_id: None,
                },
            );
            ops.put_experiment_days("floors-ee-u1", constant_days(4_000_000, 4_400_000));
            let mut l = ledger();
            l.append(LedgerRecord::change_applied(
                NOW,
                apex_core::PlatformId::AppLovinMax,
                "u1",
                ChangeKind::CountryFloorSet {
                    country: "EE".into(),
                    floor_ecpm: 2_500_000,
                },
                "key-crash",
                "",
            ))
            .expect("change");
            l.append(LedgerRecord::experiment_started(
                NOW - 10 * 86_400_000,
                apex_core::PlatformId::AppLovinMax,
                "floors-ee-u1",
                experiment_spec("floors-ee-u1", "u1"),
            ))
            .expect("start");
            append_lift(&mut l, vec_prereg("floors-ee-u1", "u1", 1, 2_000), NOW + 1);
            // The crash shape: signed proof exists, the completion marker and
            // remote side effect do not.
            append_lift(
                &mut l,
                vec_result("floors-ee-u1", decision, lambda),
                NOW + 2,
            );

            let rig = rig(ops.clone(), &format!("reconcile-{decision}")).await;
            let runner = &rig.runner;
            let report = runner.run_daily(&mut l, NOW + 3).await.expect("pass");
            assert!(
                report.failures.is_empty(),
                "{decision}: {:?}",
                report.failures
            );
            assert_eq!(report.settled, 1, "{decision}");
            assert_eq!(
                report.outcomes.first().map(|(_, status)| status.as_str()),
                Some(expected),
                "{decision}"
            );
            let calls = ops.call_log();
            assert!(
                !calls.iter().any(|entry| entry.starts_with("days:")),
                "{decision}: a reconciled terminal proof is not re-pulled: {calls:?}"
            );
            assert!(
                !stored_types(&rig)
                    .await
                    .iter()
                    .any(|kind| kind == "report_pull_started"),
                "{decision}: no pull-start event is emitted for a pending terminal proof"
            );
            match decision {
                "keep" => {
                    assert!(calls.iter().any(|entry| entry.starts_with("promote:")));
                    assert!(l
                        .records()
                        .iter()
                        .any(|record| matches!(record, LedgerRecord::ExperimentObserved { .. })));
                }
                "rollback" => {
                    assert!(calls.iter().any(|entry| entry.starts_with("deprecate:")));
                    assert!(l
                        .records()
                        .iter()
                        .any(|record| matches!(record, LedgerRecord::RollbackPerformed { .. })));
                }
                "void" => {
                    assert!(calls.iter().any(|entry| entry.starts_with("deprecate:")));
                    assert!(l
                        .records()
                        .iter()
                        .any(|record| matches!(record, LedgerRecord::ExperimentObserved { .. })));
                }
                _ => unreachable!(),
            }

            // Once the completion marker lands the pass never reconciles it
            // again.
            let before: Vec<u64> = l.records().iter().map(|record| record.seq()).collect();
            runner
                .run_daily(&mut l, NOW + 4)
                .await
                .expect("second pass");
            let after: Vec<u64> = l.records().iter().map(|record| record.seq()).collect();
            assert_eq!(after, before, "{decision}: no duplicate completion");
            let _ = std::fs::remove_dir_all(&rig.dir);
        }
    }

    #[tokio::test]
    async fn a_refused_settle_appends_no_pull_across_repeated_daily_passes() {
        // A result that lands before its pre-registration makes the settle
        // path refuse the experiment as already settled. The refusal must not
        // record a data pull: the old shape appended one on every pass.
        let ops = Arc::new(RecordingOps::new());
        ops.put_experiment(
            "exp-1",
            ExperimentResult {
                experiment_id: "exp-1".into(),
                state: ExperimentState::Running,
                results: vec![],
                lift_percent_bp: None,
                confidence: None,
                winner_variant_id: None,
            },
        );
        ops.put_experiment_days("exp-1", constant_days(4_000_000, 4_400_000));

        let mut l = ledger();
        l.append(LedgerRecord::change_applied(
            NOW,
            apex_core::PlatformId::AppLovinMax,
            "u1",
            ChangeKind::CountryFloorSet {
                country: "EE".into(),
                floor_ecpm: 2_500_000,
            },
            "key-1",
            "",
        ))
        .expect("change");
        l.append(LedgerRecord::experiment_started(
            NOW,
            apex_core::PlatformId::AppLovinMax,
            "exp-1",
            experiment_spec("floors-ee-u1", "u1"),
        ))
        .expect("start");
        append_lift(&mut l, vec_result("floors-ee-u1", "keep", 0.02), NOW + 2);
        append_lift(&mut l, vec_prereg("floors-ee-u1", "u1", 1, 2_000), NOW + 3);
        let before: Vec<u64> = l.records().iter().map(|record| record.seq()).collect();

        let runner = Runner::new(ops.clone(), policy(), FillFormat::Rewarded);
        let first = runner.run_daily(&mut l, NOW + 1).await.expect("first pass");
        assert_eq!(
            l.records()
                .iter()
                .map(|record| record.seq())
                .collect::<Vec<_>>(),
            before,
            "the refused settle appends no pull record"
        );
        assert!(
            first
                .failures
                .iter()
                .any(|(id, reason)| id == "exp-1" && reason.contains("already settled")),
            "{first:?}"
        );

        let second = runner
            .run_daily(&mut l, NOW + 2)
            .await
            .expect("second pass");
        assert_eq!(
            l.records()
                .iter()
                .map(|record| record.seq())
                .collect::<Vec<_>>(),
            before,
            "the second pass appends nothing either"
        );
        assert_eq!(
            second.failures, first.failures,
            "the typed outcome is unchanged"
        );
    }

    /// A dunning invoice written into the intent journal by "the monthly
    /// run": one line issued, submitted and definitively failed, plus the
    /// recorded invoice payment link, all older than the dunning grace.
    fn dunning_journal(intents: &apex_billing::PaymentIntentStore, created_unix_ms: i64) -> String {
        use apex_billing::VatTreatment;
        let invoice = apex_billing::Invoice {
            number: 1,
            publisher_id: "northstar".into(),
            period: apex_billing::BillingPeriod {
                year_month: "2026-09".into(),
                start_unix_ms: 0,
                end_unix_ms: 1_000,
            },
            line_items: vec![apex_billing::InvoiceLine {
                seq: 1,
                experiment_id: "ch_1".into(),
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
        };
        let item = apex_billing::mollie::InvoiceLineItem {
            seq: 1,
            experiment_id: "ch_1".into(),
            amount_minor_units: 125,
            currency: "USD".into(),
            customer_reference: "publisher-northstar".into(),
            reference: "apex-fee:northstar:ch_1:2026-09".into(),
            description: "Apex proven-lift fee".into(),
        };
        let issued = intents
            .ensure_issued(&invoice, "INV-2026-09-northstar", &item, created_unix_ms)
            .expect("issued");
        intents
            .begin_submission(
                &issued.intent_id,
                apex_billing::PaymentIntentChannel::PaymentLink,
                None,
                None,
                created_unix_ms,
            )
            .expect("begin");
        intents
            .record_provider(
                &issued.intent_id,
                Some("tr_1"),
                "failed",
                created_unix_ms + 1,
            )
            .expect("failed");
        intents
            .record_invoice_link(
                "INV-2026-09-northstar",
                &invoice,
                &apex_billing::mollie::PaymentLinkResponse {
                    id: "pl_1".into(),
                    url: "https://pay.mollie.com/pl_1".into(),
                    customer_id: None,
                },
                125,
                created_unix_ms + 2,
            )
            .expect("link");
        issued.intent_id
    }

    fn dunning_runner(dir: &std::path::Path) -> Runner {
        let registry = Arc::new(apex_metrics::Registry::new());
        let sink = Arc::new(apex_notify::sink::FileSink::new(dir.join("outbox.jsonl")));
        let log =
            apex_notify::state::DeliveryLog::open(Some(&dir.join("delivery.jsonl"))).expect("log");
        let notifier = Arc::new(apex_notify::notifier::Notifier::new(sink, log, registry));
        let book =
            apex_notify::contacts::ContactBook::from_json(r#"{"northstar": ["ops@example.com"]}"#)
                .expect("book");
        let notify = Arc::new(crate::notify::NotifyPlan::new(
            notifier,
            book,
            "https://console.apexmediation.ee",
        ));
        Runner::new(
            Arc::new(crate::ops::RecordingOps::new()),
            policy(),
            FillFormat::Rewarded,
        )
        .with_publisher("northstar")
        .with_notifications(notify)
        .with_payment_intents(dir.join("payment-intents.jsonl"))
        .with_payment_events(
            dir.join("collection.jsonl"),
            None,
            Some(dir.join("retries.jsonl")),
        )
    }

    fn dunning_outbox(dir: &std::path::Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(dir.join("outbox.jsonl"))
            .map(|raw| {
                raw.lines()
                    .map(|line| serde_json::from_str(line).expect("row"))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn the_dunning_sweep_writes_retry_rows_and_resends_the_payment_link() {
        let dir = std::env::temp_dir().join(format!("apex-runner-dunning-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let intents = apex_billing::PaymentIntentStore::open(&dir.join("payment-intents.jsonl"))
            .expect("open");
        let created = 1_000_000;
        let intent_id = dunning_journal(&intents, created);
        let now = created + apex_billing::intent::DUNNING_GRACE_MS + 5_000;
        drop(intents);

        let runner = dunning_runner(&dir);
        let report = runner
            .dunning_sweep(now)
            .await
            .expect("sweep")
            .expect("the journal exists");
        assert_eq!(report.unpaid_invoices, 1);
        assert_eq!(report.retry_rows, 1);
        assert_eq!(report.reminders, 1);

        // The retry schedule has rows now: the reader the retry messages
        // consume finally sees something (GG).
        let rows =
            crate::payment_events::read_scheduled_retries(dir.join("retries.jsonl").as_path())
                .expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].reference, "apex-fee:northstar:ch_1:2026-09");
        assert_eq!(rows[0].invoice_reference, "INV-2026-09-northstar");
        assert_eq!(rows[0].state, "scheduled");
        assert_eq!(rows[0].attempt, 1);
        assert_eq!(
            rows[0].retry_on.as_deref(),
            Some(
                crate::calendar::format_day(
                    now.div_euclid(86_400_000) + Runner::DUNNING_RETRY_IN_DAYS
                )
                .as_str()
            )
        );

        // The payment link email went out with the durable link.
        let outbox = dunning_outbox(&dir);
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0]["template_class"], "invoice_ready");
        assert_eq!(
            outbox[0]["dedup_key"],
            "invoice_ready:INV-2026-09-northstar:1"
        );
        assert!(outbox[0]["text"]
            .as_str()
            .expect("text")
            .contains("https://pay.mollie.com/pl_1"));

        // The same-day sweep is idempotent: no duplicate rows and the
        // reminder's dedup identity absorbs the repeat.
        let second = runner
            .dunning_sweep(now)
            .await
            .expect("sweep")
            .expect("swept");
        assert_eq!(second.retry_rows, 0);
        assert_eq!(second.unpaid_invoices, 1);
        assert_eq!(
            crate::payment_events::read_scheduled_retries(dir.join("retries.jsonl").as_path())
                .expect("rows")
                .len(),
            1
        );
        assert_eq!(
            dunning_outbox(&dir).len(),
            1,
            "the reminder never duplicates"
        );

        // A later collected payment on the line stops the chase entirely.
        let intents = apex_billing::PaymentIntentStore::open(&dir.join("payment-intents.jsonl"))
            .expect("reopen");
        intents
            .record_provider(&intent_id, Some("tr_1"), "paid", now + 1)
            .expect("paid");
        let report = runner
            .dunning_sweep(now + 2)
            .await
            .expect("sweep")
            .expect("swept");
        assert_eq!(report.unpaid_invoices, 0);
        assert_eq!(report.reminders, 0, "a paid invoice is not chased");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_missing_intent_journal_sweeps_nothing() {
        let dir =
            std::env::temp_dir().join(format!("apex-runner-dunning-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let runner = dunning_runner(&dir);
        let report = runner.dunning_sweep(1_000_000).await.expect("sweep");
        assert_eq!(report, None, "no journal is no sweep, not an error");
        assert!(!dir.join("retries.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
