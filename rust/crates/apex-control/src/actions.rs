//! The action engine: every POST passes the same gate before anything runs.
//!
//! In order: the CSRF token must match the session's, the capability must be
//! held by the role (deny by default), sensitive capabilities must be inside
//! the recent-authentication window, the typed scoped confirmation and the
//! mandatory reason/source-document fields must be present, the idempotency
//! key claims one submission at a time (a duplicate waits for the original
//! and returns its result page), and the stale-state token must still match
//! the object. Only then does one of two things happen: a
//! durable operator job is written for the runner (retry, recheck, restore
//! reconcile, job retry), or the action's record is appended to the journal
//! and the governance registers (acknowledge, resolve, owner, invoice, fleet,
//! policy, session revoke, review). The control plane never performs a
//! platform operation itself.

use serde_json::json;

use crate::capabilities::{Capability, CapabilityCheck};
use crate::copy::{
    RESULT_CONFIRMED, RESULT_FAILED, RESULT_REQUESTED, RESULT_RUNNING, RESULT_STATE_UNKNOWN,
};
use crate::governance::{InterventionRecord, PolicyRecord};
use crate::jobs::{JobKind, OperatorJob};
use crate::journal::ActionJournalEntry;
use crate::signing::{constant_time_eq, idempotency_fingerprint, is_random_hex_32, random_hex_32};
use crate::state::ControlState;

/// The form fields every action POST carries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActionRequest {
    /// The object identity the action targets.
    pub target: String,
    /// The operator's reason.
    pub reason: String,
    /// Source document reference (manual reconcile).
    pub source_document: String,
    /// Recovery evidence (incident resolve).
    pub evidence: String,
    /// The typed scoped confirmation (destructive or broad actions).
    pub confirmation: String,
    /// The hidden idempotency key.
    pub idempotency_key: String,
    /// The hidden stale-state token.
    pub version_token: String,
    /// The hidden CSRF token.
    pub csrf_token: String,
    /// The object route to return to on denied/conflict pages.
    pub return_path: String,
    /// Bounded job payload extras (platform, account, ...).
    pub payload: serde_json::Value,
    /// Customer population (fleet interventions).
    pub customer_population: String,
    /// Expected effect (fleet interventions).
    pub expected_effect: String,
    /// Platform (fleet interventions), when scoped.
    pub platform: Option<String>,
    /// The pending policy id (policy activation).
    pub policy_id: String,
}

/// The five result states of page 64.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionState {
    /// The request is recorded and queued.
    Requested,
    /// The durable job is running.
    Running,
    /// A confirming observation is recorded.
    Confirmed,
    /// The operation failed.
    Failed,
    /// The platform outcome is not known yet.
    StateUnknown,
}

impl ActionState {
    /// The exact page-64 label.
    pub fn label(self) -> &'static str {
        match self {
            ActionState::Requested => RESULT_REQUESTED,
            ActionState::Running => RESULT_RUNNING,
            ActionState::Confirmed => RESULT_CONFIRMED,
            ActionState::Failed => RESULT_FAILED,
            ActionState::StateUnknown => RESULT_STATE_UNKNOWN,
        }
    }

    /// The journal outcome token.
    pub fn outcome(self) -> &'static str {
        match self {
            ActionState::Requested => "requested",
            ActionState::Running => "running",
            ActionState::Confirmed => "confirmed",
            ActionState::Failed => "failed",
            ActionState::StateUnknown => "state_unknown",
        }
    }
}

/// The engine result the route renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// A duplicate POST: the original result page id.
    Duplicate {
        /// The remembered result id.
        result_id: String,
    },
    /// The CSRF token did not match; nothing was requested.
    CsrfRejected,
    /// The role does not hold the capability (page 97 copy).
    Denied {
        /// The capability attempted.
        capability: Capability,
        /// The target.
        target: String,
        /// The object route to return to.
        return_path: String,
    },
    /// A sensitive capability needs a fresh sign-in first.
    StepUpRequired {
        /// The safe view to return to after re-authentication.
        next: String,
    },
    /// The stale-state check failed; nothing was requested (409).
    Conflict {
        /// Why the token failed, safe to render.
        reason: String,
        /// The object route to reload.
        return_path: String,
    },
    /// A required field is missing or malformed; the form must be re-shown.
    FormProblem {
        /// The safe message for the form.
        message: String,
    },
    /// The action executed and recorded a result.
    Result {
        /// The result id (`res_<12 hex>`).
        result_id: String,
        /// The state to display.
        state: ActionState,
        /// The durable operation or job id, when one exists.
        operation_id: Option<String>,
        /// The object route that confirms the state (a read).
        confirming_read: String,
        /// A bounded note for the result page.
        note: String,
    },
}

/// Whether a capability writes a durable operator job.
pub fn writes_job(capability: Capability) -> bool {
    matches!(
        capability,
        Capability::RetryRead
            | Capability::RecheckRequest
            | Capability::RestoreReconcileRequest
            | Capability::JobRetry
    )
}

fn job_kind(capability: Capability) -> Option<JobKind> {
    match capability {
        Capability::RetryRead => Some(JobKind::RetryRead),
        Capability::RecheckRequest => Some(JobKind::RecheckRequest),
        Capability::RestoreReconcileRequest => Some(JobKind::RestoreReconcileRequest),
        Capability::JobRetry => Some(JobKind::JobRetry),
        _ => None,
    }
}

/// The bounded correlation id for one submission.
fn correlation_id(capability: Capability) -> String {
    format!("ctl-{}-{}", capability.as_str(), &random_hex_32()[..12])
}

/// The engine entry point. The role, `session_csrf` and
/// `session_recent_auth` come from the live session view, never from the
/// form, so a forged field cannot widen authority.
pub async fn submit(
    state: &ControlState,
    capability: Capability,
    request: &ActionRequest,
    actor: &str,
    role: crate::capabilities::Role,
    session_csrf: &str,
    session_recent_auth: bool,
) -> SubmitOutcome {
    // 1. CSRF first: nothing happens for a request that cannot prove it came
    // from this session's form.
    if !constant_time_eq(request.csrf_token.as_bytes(), session_csrf.as_bytes()) {
        return SubmitOutcome::CsrfRejected;
    }
    if !is_random_hex_32(&request.idempotency_key) {
        return SubmitOutcome::FormProblem {
            message: "The form's request handle was missing. Reload the object and try again."
                .to_string(),
        };
    }
    // 2. The idempotency key is claimed before anything else runs: the
    // duplicate check, the durable job write and the journal append are one
    // critical section per key, so two concurrent submissions of the same
    // form cannot both execute. A duplicate waits here for the original and
    // then returns its result page with no new journal entry.
    let _submission_claim = state.claim_submission(&request.idempotency_key).await;
    if let Some(result_id) = state.remembered_result(&request.idempotency_key) {
        return SubmitOutcome::Duplicate { result_id };
    }
    // 3. Capability check, deny by default.
    if crate::capabilities::check(role, capability) == CapabilityCheck::Denied {
        let correlation = correlation_id(capability);
        let entry = journal_entry(
            state,
            actor,
            role,
            capability,
            request,
            "denied",
            None,
            None,
            &correlation,
        );
        let _ = state.journal.append(&entry);
        crate::telemetry::record_action(&state.metrics, capability, "denied");
        if let Some(emitter) = &state.emitter {
            emitter
                .emit_action(
                    actor,
                    capability.as_str(),
                    &request.target,
                    "denied",
                    &correlation,
                )
                .await;
        }
        return SubmitOutcome::Denied {
            capability,
            target: request.target.clone(),
            return_path: request.return_path.clone(),
        };
    }
    // 4. Recent authentication for sensitive capabilities.
    if capability.requires_step_up() && !session_recent_auth {
        let correlation = correlation_id(capability);
        let entry = journal_entry(
            state,
            actor,
            role,
            capability,
            request,
            "step_up_required",
            None,
            None,
            &correlation,
        );
        let _ = state.journal.append(&entry);
        crate::telemetry::record_action(&state.metrics, capability, "step_up_required");
        if let Some(emitter) = &state.emitter {
            emitter
                .emit_action(
                    actor,
                    capability.as_str(),
                    &request.target,
                    "step_up_required",
                    &correlation,
                )
                .await;
        }
        return SubmitOutcome::StepUpRequired {
            next: safe_return(&request.return_path),
        };
    }
    // 5. Typed scoped confirmation.
    if let Some(phrase) = capability.typed_confirmation() {
        if request.confirmation.trim() != phrase {
            return SubmitOutcome::FormProblem {
                message: format!("This action needs the typed confirmation \"{phrase}\" to run."),
            };
        }
    }
    // 6. Mandatory reason and source document.
    if capability.requires_reason() && request.reason.trim().is_empty() {
        return SubmitOutcome::FormProblem {
            message: "A reason is required for this action.".to_string(),
        };
    }
    if capability.requires_source_document() && request.source_document.trim().is_empty() {
        return SubmitOutcome::FormProblem {
            message: "A source document reference is required for this action.".to_string(),
        };
    }
    if capability == Capability::IncidentResolve && request.evidence.trim().is_empty() {
        return SubmitOutcome::FormProblem {
            message: "Resolution requires evidence of recovery (a confirming read or proof)."
                .to_string(),
        };
    }
    // 7. Stale-state check against the object as it is now.
    let Some((object_key, current_state)) = object_state(state, capability, request).await else {
        return SubmitOutcome::Conflict {
            reason: "the object is not present in this snapshot".to_string(),
            return_path: safe_return(&request.return_path),
        };
    };
    let Some(token) = crate::versions::VersionToken::parse(&request.version_token) else {
        return SubmitOutcome::Conflict {
            reason: "the form's version token is not valid".to_string(),
            return_path: safe_return(&request.return_path),
        };
    };
    if let crate::versions::VersionCheck::Stale { reason } =
        state.versions.check(&object_key, &token, &current_state)
    {
        let correlation = correlation_id(capability);
        let entry = journal_entry(
            state,
            actor,
            role,
            capability,
            request,
            "rejected",
            None,
            None,
            &correlation,
        );
        let _ = state.journal.append(&entry);
        return SubmitOutcome::Conflict {
            reason: reason.to_string(),
            return_path: safe_return(&request.return_path),
        };
    }
    // 8. Execute: a durable job for the runner, or a recorded governance
    // decision. The control plane never performs the platform operation.
    let now_ms = state.now_ms();
    let correlation = correlation_id(capability);
    let result_id = format!("res_{}", &random_hex_32()[..12]);
    let mut job_id = None;
    let note;
    let mut state_outcome = ActionState::Confirmed;
    if let Some(kind) = job_kind(capability) {
        let job = OperatorJob::queued(
            kind,
            actor,
            now_ms,
            &request.target,
            request.payload.clone(),
            &request.idempotency_key,
            &correlation,
        );
        match state.jobs.write(&job) {
            Ok(written) => {
                // An idempotent write returns the existing job when the key
                // already claimed one (for example after a restart between
                // the job write and the journal append).
                job_id = Some(written.job.job_id.clone());
                state_outcome = ActionState::Requested;
                note = format!(
                    "One operator request created one job ({}); duplicate submits reuse it.",
                    written.job.job_id
                );
            }
            Err(error) => {
                let entry = journal_entry(
                    state,
                    actor,
                    role,
                    capability,
                    request,
                    "failed",
                    Some(&result_id),
                    None,
                    &correlation,
                );
                let _ = state.journal.append(&entry);
                crate::telemetry::record_action(&state.metrics, capability, "failed");
                state.remember_result(&request.idempotency_key, &result_id);
                return SubmitOutcome::Result {
                    result_id,
                    state: ActionState::Failed,
                    operation_id: None,
                    confirming_read: safe_return(&request.return_path),
                    note: format!("the durable job could not be written: {}", error),
                };
            }
        }
    } else {
        match execute_local(state, capability, request, actor, now_ms, &correlation) {
            Ok(local_note) => {
                note = local_note;
            }
            Err(problem) => {
                let entry = journal_entry(
                    state,
                    actor,
                    role,
                    capability,
                    request,
                    "failed",
                    Some(&result_id),
                    None,
                    &correlation,
                );
                let _ = state.journal.append(&entry);
                crate::telemetry::record_action(&state.metrics, capability, "failed");
                state.remember_result(&request.idempotency_key, &result_id);
                return SubmitOutcome::Result {
                    result_id,
                    state: ActionState::Failed,
                    operation_id: None,
                    confirming_read: safe_return(&request.return_path),
                    note: problem,
                };
            }
        }
    }
    let entry = journal_entry(
        state,
        actor,
        role,
        capability,
        request,
        state_outcome.outcome(),
        Some(&result_id),
        job_id.as_deref(),
        &correlation,
    );
    let _ = state.journal.append(&entry);
    // The recorded outcome is the action's concrete result state (page 64):
    // `requested` for a queued durable job, `confirmed` for a recorded
    // governance decision.
    crate::telemetry::record_action(&state.metrics, capability, state_outcome.outcome());
    if let Some(emitter) = &state.emitter {
        emitter
            .emit_action(
                actor,
                capability.as_str(),
                &request.target,
                state_outcome.outcome(),
                &correlation,
            )
            .await;
    }
    state.versions.bump(&object_key);
    state.remember_result(&request.idempotency_key, &result_id);
    SubmitOutcome::Result {
        result_id,
        state: state_outcome,
        operation_id: job_id,
        confirming_read: safe_return(&request.return_path),
        note,
    }
}

/// Execute the actions whose durable record is the control plane's own
/// governance/journal state. Returns a bounded note, or a failure message.
fn execute_local(
    state: &ControlState,
    capability: Capability,
    request: &ActionRequest,
    actor: &str,
    now_ms: i64,
    correlation: &str,
) -> Result<String, String> {
    match capability {
        Capability::FleetPauseScheduling | Capability::FleetResumeScheduling => {
            let kind = if capability == Capability::FleetPauseScheduling {
                "pause_scheduling"
            } else {
                "resume_scheduling"
            };
            let record = InterventionRecord::new(
                kind,
                request.platform.as_deref(),
                if request.customer_population.trim().is_empty() {
                    "the deployment's full publisher population"
                } else {
                    request.customer_population.trim()
                },
                request.reason.trim(),
                if request.expected_effect.trim().is_empty() {
                    "no new scheduling starts until resume; reads continue independently"
                } else {
                    request.expected_effect.trim()
                },
                actor,
                now_ms,
                correlation,
            );
            state
                .governance
                .append_intervention(&record)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Intervention {} recorded. Reads continue independently of scheduling.",
                record.intervention_id
            ))
        }
        Capability::PolicyDraft => {
            let version = state
                .governance
                .next_version()
                .map_err(|error| error.to_string())?;
            let record = PolicyRecord::draft(
                actor,
                now_ms,
                json!({"source": "operator draft"}),
                json!({"note": request.target}),
                json!({"note": request.reason}),
                "Future proposals drafted after activation; active experiments keep their registered policy and no signed fee basis changes.",
                version,
                correlation,
            );
            state
                .governance
                .append_policy(&record)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Draft {} (version {version}) recorded as pending; activation is a separate step.",
                record.policy_id
            ))
        }
        Capability::PolicyActivate => {
            let pending = state
                .governance
                .pending_policies()
                .map_err(|error| error.to_string())?;
            let draft = pending
                .iter()
                .find(|record| record.policy_id == request.policy_id.trim())
                .ok_or_else(|| {
                    format!(
                        "no pending policy {} is on file",
                        request.policy_id.trim()
                    )
                })?;
            let mut activated = draft.clone();
            activated.state = "active".to_string();
            activated.activated_by = Some(actor.to_string());
            activated.activated_at_ms = Some(now_ms);
            state
                .governance
                .append_policy(&activated)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Policy {} activated by {actor}; the draft, preview and activation time stay traceable.",
                activated.policy_id
            ))
        }
        Capability::SessionRevoke => {
            let prefix = request.target.trim();
            let Some(id) = state
                .sessions
                .session_id_for_prefix(prefix, now_ms)
            else {
                return Err(
                    "no single live session matches this reference; reload the access page"
                        .to_string(),
                );
            };
            if state.sessions.close(&id) {
                Ok(format!("Session {prefix} revoked."))
            } else {
                Err("the session was already ended".to_string())
            }
        }
        Capability::IncidentAcknowledge
        | Capability::OwnerAssign
        | Capability::IncidentResolve
        | Capability::RequestReconnection
        | Capability::InvoiceHold
        | Capability::InvoiceRelease
        | Capability::CollectionRetry
        | Capability::DisputeMark
        | Capability::ManualReconcile
        | Capability::OperatorActionReview => Ok(format!(
            "Recorded against {}. The action journal holds the before/after digests and the reason.",
            request.target.trim()
        )),
        Capability::RetryRead
        | Capability::RecheckRequest
        | Capability::RestoreReconcileRequest
        | Capability::JobRetry => Err("this capability writes a durable job".to_string()),
    }
}

/// The current state of the object an action targets, as the stale-state
/// check sees it. `None` when the object cannot be found.
pub async fn object_state(
    state: &ControlState,
    capability: Capability,
    request: &ActionRequest,
) -> Option<(String, serde_json::Value)> {
    let now = state.now_ms();
    let target = request.target.trim();
    match capability {
        Capability::IncidentAcknowledge | Capability::IncidentResolve | Capability::OwnerAssign => {
            let queues = state.data.queues(state.as_of()).await.ok()?;
            let row = queues
                .incidents
                .iter()
                .find(|row| row.object_id == target)?;
            // Age is presentation time, not object state; the digest would
            // change every second if it were included.
            Some((
                format!("incident:{target}"),
                json!({
                    "object_id": row.object_id,
                    "publisher_id": row.publisher_id,
                    "severity": row.severity,
                    "owner": row.owner,
                    "next_action": row.next_action,
                }),
            ))
        }
        Capability::RetryRead => {
            let rows = state.data.ingestion_health(state.as_of()).await.ok()?;
            let (publisher, platform, account) = split3(target)?;
            let row = rows.iter().find(|row| {
                row.publisher_id == publisher
                    && row.platform == platform
                    && row.platform_account_id == account
            })?;
            Some((
                format!("account:{target}"),
                json!({
                    "publisher_id": row.publisher_id,
                    "platform": row.platform,
                    "platform_account_id": row.platform_account_id,
                    "last_final_day": row.last_final_day,
                    "status": row.status.as_str(),
                }),
            ))
        }
        Capability::RequestReconnection => Some((
            format!("customer:{target}"),
            json!({"publisher_id": target}),
        )),
        Capability::RestoreReconcileRequest => {
            let experiments = state.data.durable_experiments().ok()?;
            let experiment = experiments
                .iter()
                .find(|experiment| experiment.experiment_id == target)?;
            Some((
                format!("experiment:{target}"),
                json!({
                    "experiment_id": experiment.experiment_id,
                    "publisher": experiment.publisher,
                    "started_at_ms": experiment.started_at_ms,
                    "state": experiment.state,
                }),
            ))
        }
        Capability::RecheckRequest => {
            Some((format!("change:{target}"), json!({"change_id": target})))
        }
        Capability::JobRetry => {
            let rows = state.jobs.list().ok()?;
            let job = rows
                .iter()
                .find(|row| row.sort_key() == format!("{target}.json"))
                .or_else(|| rows.iter().find(|row| row.sort_key() == target))?;
            Some((
                format!("job:{target}"),
                json!({"job_id": target, "state": job.display_state()}),
            ))
        }
        Capability::InvoiceHold
        | Capability::InvoiceRelease
        | Capability::CollectionRetry
        | Capability::DisputeMark
        | Capability::ManualReconcile => {
            let invoices = state.data.invoices().ok()?;
            let invoice = invoices.iter().find(|invoice| invoice.id() == target)?;
            Some((
                format!("invoice:{target}"),
                json!({
                    "invoice_id": invoice.id(),
                    "period": invoice.period,
                    "total_micros": invoice.total_micros,
                    "gross_micros": invoice.gross_micros,
                    "lines": invoice.line_items.len(),
                }),
            ))
        }
        Capability::SessionRevoke => {
            let prefix = target;
            let rows = state.sessions.list(now);
            let row = rows.iter().find(|row| row.id_prefix == prefix)?;
            Some((
                format!("session:{prefix}"),
                json!({
                    "prefix": row.id_prefix,
                    "operator_id": row.operator_id,
                    "issued_at_ms": row.issued_at_ms,
                    "expires_at_ms": row.expires_at_ms,
                }),
            ))
        }
        Capability::FleetPauseScheduling | Capability::FleetResumeScheduling => {
            let paused = state.governance.fleet_scheduling_paused().ok()?;
            Some(("fleet:scheduling".to_string(), json!({"paused": paused})))
        }
        Capability::PolicyDraft => {
            let pending = state.governance.pending_policies().ok()?.len();
            let active = state.governance.active_policies().ok()?.len();
            Some((
                "policy:draft".to_string(),
                json!({"pending": pending, "active": active}),
            ))
        }
        Capability::PolicyActivate => {
            let pending = state.governance.pending_policies().ok()?;
            let draft = pending
                .iter()
                .find(|record| record.policy_id == request.policy_id.trim())
                .or_else(|| pending.iter().find(|record| record.policy_id == target))?;
            Some((
                format!("policy:{}", draft.policy_id),
                json!({
                    "policy_id": draft.policy_id,
                    "version": draft.version,
                    "state": draft.state,
                }),
            ))
        }
        Capability::OperatorActionReview => {
            let read = state.journal.read().ok()?;
            let entry = read
                .entries
                .iter()
                .find(|entry| action_target_key(entry) == target)?;
            Some((
                format!("action:{target}"),
                json!({
                    "at_ms": entry.at_ms,
                    "actor": entry.actor,
                    "capability": entry.capability,
                    "outcome": entry.outcome,
                    "target": entry.target,
                }),
            ))
        }
    }
}

/// Split `<publisher>:<platform>:<account>`.
fn split3(target: &str) -> Option<(&str, &str, &str)> {
    let mut parts = target.splitn(3, ':');
    let publisher = parts.next()?;
    let platform = parts.next()?;
    let account = parts.next()?;
    if publisher.is_empty() || platform.is_empty() || account.is_empty() {
        return None;
    }
    Some((publisher, platform, account))
}

/// The stable key of a journal entry for review actions.
pub fn action_target_key(entry: &ActionJournalEntry) -> String {
    format!("{}:{}:{}", entry.at_ms, entry.capability, entry.actor)
}

/// Only control-plane GET paths are accepted as a return target.
fn safe_return(path: &str) -> String {
    crate::login::safe_next(path)
        .unwrap_or("/control/")
        .to_string()
}

#[allow(clippy::too_many_arguments)]
fn journal_entry(
    state: &ControlState,
    actor: &str,
    role: crate::capabilities::Role,
    capability: Capability,
    request: &ActionRequest,
    outcome: &str,
    result_id: Option<&str>,
    job_id: Option<&str>,
    correlation: &str,
) -> ActionJournalEntry {
    let reason = crate::redact::redact_text(&format!(
        "{}{}{}",
        request.reason.trim(),
        if request.evidence.trim().is_empty() {
            String::new()
        } else {
            format!(" evidence: {}", request.evidence.trim())
        },
        if request.source_document.trim().is_empty() {
            String::new()
        } else {
            format!(" source document: {}", request.source_document.trim())
        },
    ));
    ActionJournalEntry {
        at_ms: state.now_ms(),
        actor: actor.to_string(),
        role: role.as_str().to_string(),
        capability: capability.as_str().to_string(),
        capability_class: capability.capability_class().to_string(),
        target: crate::redact::redact_text(request.target.trim()),
        outcome: outcome.to_string(),
        reason,
        before_digest: crate::signing::sha256_hex(request.version_token.as_bytes()),
        after_digest: crate::signing::sha256_hex(
            format!("{outcome}:{}", request.target.trim()).as_bytes(),
        ),
        idempotency_key: idempotency_fingerprint(&request.idempotency_key),
        result_id: result_id.map(str::to_string),
        job_id: job_id.map(str::to_string),
        correlation_id: correlation.to_string(),
    }
}
