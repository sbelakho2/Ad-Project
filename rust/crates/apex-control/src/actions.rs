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
    CONFLICT_KEY_REUSED, RESULT_CONFIRMED, RESULT_FAILED, RESULT_REQUESTED, RESULT_RUNNING,
    RESULT_STATE_UNKNOWN,
};
use crate::governance::{InterventionRecord, PolicyRecord};
use crate::jobs::{JobKind, JobWrite, OperatorJob};
use crate::journal::ActionJournalEntry;
use crate::signing::{
    constant_time_eq, idempotency_fingerprint, is_random_hex_32, payload_fingerprint, random_hex_32,
};
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
    /// A full `AutopilotPolicy` JSON document (`policy_set`).
    pub policy_json: String,
    /// The reported 30-day revenue in micros (`managed_plan_activate`).
    pub revenue_30d_micros: String,
    /// The optional monitoring tier (`managed_plan_activate`).
    pub tier: String,
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
            | Capability::RequestReconnection
            | Capability::RecheckRequest
            | Capability::RestoreReconcileRequest
            | Capability::JobRetry
    )
}

fn job_kind(capability: Capability) -> Option<JobKind> {
    match capability {
        Capability::RetryRead => Some(JobKind::RetryRead),
        Capability::RequestReconnection => Some(JobKind::ReconnectionRequest),
        Capability::RecheckRequest => Some(JobKind::RecheckRequest),
        Capability::RestoreReconcileRequest => Some(JobKind::RestoreReconcileRequest),
        Capability::JobRetry => Some(JobKind::JobRetry),
        _ => None,
    }
}

/// The bounded correlation id for one submission.
pub(crate) fn correlation_id() -> String {
    // The correlation id doubles as the analytics event id (prefixed in
    // events.rs), so it must stay a short hex-dominant token: identifiers
    // that read like long non-hex words are refused by the envelope's
    // token-like guard (a 40+ char word would drop every action event).
    format!("ctl-{}", &random_hex_32()[..12])
}

/// Append one journal entry and add it to the in-memory index only when the
/// durable write succeeded. The result page and access forms then read the
/// index instead of the file.
fn append_entry(
    state: &ControlState,
    entry: &ActionJournalEntry,
) -> Result<(), crate::journal::JournalError> {
    let result = state.journal.append(entry);
    if result.is_ok() {
        state.observe_entry(entry);
    }
    result
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
    // form cannot both execute. A duplicate (same key, same payload
    // fingerprint) waits here for the original and then returns its result
    // page with no new journal entry; the same key for a *different* action
    // is a conflict and changes nothing.
    let fingerprint =
        payload_fingerprint(capability.as_str(), request.target.trim(), &request.payload);
    let _submission_claim = state.claim_submission(&request.idempotency_key).await;
    if let Some(remembered) = state.remembered_result(&request.idempotency_key) {
        if remembered.matches(&fingerprint) {
            return SubmitOutcome::Duplicate {
                result_id: remembered.result_id,
            };
        }
        return SubmitOutcome::Conflict {
            reason: CONFLICT_KEY_REUSED.to_string(),
            return_path: safe_return(&request.return_path),
        };
    }
    // 3. Capability check, deny by default.
    if crate::capabilities::check(role, capability) == CapabilityCheck::Denied {
        let correlation = correlation_id();
        let entry = journal_entry(
            state,
            actor,
            role,
            capability,
            request,
            &fingerprint,
            "denied",
            None,
            None,
            &correlation,
        );
        let _ = append_entry(state, &entry);
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
        let correlation = correlation_id();
        let entry = journal_entry(
            state,
            actor,
            role,
            capability,
            request,
            &fingerprint,
            "step_up_required",
            None,
            None,
            &correlation,
        );
        let _ = append_entry(state, &entry);
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
        let correlation = correlation_id();
        let entry = journal_entry(
            state,
            actor,
            role,
            capability,
            request,
            &fingerprint,
            "rejected",
            None,
            None,
            &correlation,
        );
        let _ = append_entry(state, &entry);
        return SubmitOutcome::Conflict {
            reason: reason.to_string(),
            return_path: safe_return(&request.return_path),
        };
    }
    // 8. Execute: a durable job for the runner, or a recorded governance
    // decision. The control plane never performs the platform operation.
    let now_ms = state.now_ms();
    let correlation = correlation_id();
    let result_id = format!("res_{}", &random_hex_32()[..12]);
    let mut job_id = None;
    let note;
    let mut state_outcome = ActionState::Confirmed;
    if let Some(kind) = job_kind(capability) {
        // The reconnection notification names a platform when the operator
        // supplied one; the runner maps the wire name to console copy.
        let mut job_payload = request.payload.clone();
        if capability == Capability::RequestReconnection {
            if let (Some(platform), Some(object)) =
                (request.platform.as_deref(), job_payload.as_object_mut())
            {
                object.insert(
                    "platform".to_string(),
                    serde_json::Value::String(platform.to_string()),
                );
            }
        }
        let job = OperatorJob::queued(
            kind,
            actor,
            now_ms,
            &request.target,
            job_payload,
            &request.idempotency_key,
            &correlation,
        );
        match state.jobs.write(&job).await {
            Ok(JobWrite::Created(existing) | JobWrite::Duplicate(existing)) => {
                // An idempotent write returns the existing job when the key
                // already claimed an identical action (for example after a
                // restart between the job write and the journal append).
                job_id = Some(existing.job_id.clone());
                state_outcome = ActionState::Requested;
                note = format!(
                    "One operator request created one job ({}); duplicate submits reuse it.",
                    existing.job_id
                );
            }
            Ok(JobWrite::KeyConflict(_)) => {
                // The key is already bound to a different durable job (a
                // crash between the job write and the journal append): the
                // documented conflict page, with nothing changed.
                return SubmitOutcome::Conflict {
                    reason: CONFLICT_KEY_REUSED.to_string(),
                    return_path: safe_return(&request.return_path),
                };
            }
            Err(error) => {
                let entry = journal_entry(
                    state,
                    actor,
                    role,
                    capability,
                    request,
                    &fingerprint,
                    "failed",
                    Some(&result_id),
                    None,
                    &correlation,
                );
                let _ = append_entry(state, &entry);
                crate::telemetry::record_action(&state.metrics, capability, "failed");
                state.remember_result(&request.idempotency_key, &fingerprint, &result_id);
                return SubmitOutcome::Result {
                    result_id,
                    state: ActionState::Failed,
                    operation_id: None,
                    confirming_read: safe_return(&request.return_path),
                    note: format!("the durable job could not be written: {error}"),
                };
            }
        }
    } else {
        match execute_local(state, capability, request, actor, now_ms, &correlation).await {
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
                    &fingerprint,
                    "failed",
                    Some(&result_id),
                    None,
                    &correlation,
                );
                let _ = append_entry(state, &entry);
                crate::telemetry::record_action(&state.metrics, capability, "failed");
                state.remember_result(&request.idempotency_key, &fingerprint, &result_id);
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
        &fingerprint,
        state_outcome.outcome(),
        Some(&result_id),
        job_id.as_deref(),
        &correlation,
    );
    if let Err(error) = append_entry(state, &entry) {
        // The durable journal is the action's authority. A failed append
        // must never be reported as a Confirmed or Requested result: the
        // effect may exist, but the control plane cannot prove it.
        crate::telemetry::record_action(&state.metrics, capability, "failed");
        return SubmitOutcome::Result {
            result_id,
            state: ActionState::Failed,
            operation_id: job_id,
            confirming_read: safe_return(&request.return_path),
            note: format!("the durable journal write failed: {error}"),
        };
    }
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
    state.remember_result(&request.idempotency_key, &fingerprint, &result_id);
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
/// Deliver a governance mutation to the console host when the split-host
/// channel is configured. A single-host deployment has no remote and relies
/// on the local files only.
async fn push_governance(
    state: &crate::state::ControlState,
    body: serde_json::Value,
) -> Result<(), String> {
    let Some(config) = &state.config.jobs_remote else {
        return Ok(());
    };
    let remote = crate::jobs::RemoteJobs::new(config).map_err(|error| error.to_string())?;
    remote
        .write_governance(&body)
        .await
        .map_err(|error| error.to_string())
}

/// Serialize and atomically write a plan-state file with mode `0600`.
///
/// The bytes match `apex_billing::PlanState::save` (the runner and console
/// read the same format), but the temporary file is created owner-only and
/// fsynced before the rename, so the file is never world-readable even
/// momentarily.
fn write_plan_state_0600(
    path: &std::path::Path,
    plan: &apex_billing::PlanState,
) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(plan)
        .map_err(|error| format!("plan state could not be serialized: {error}"))?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("plan state directory: {error}"))?;
        }
    }
    let temp = path.with_extension("json.control-tmp");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let write = || -> std::io::Result<()> {
        use std::io::Write as _;
        let mut file = options.open(&temp)?;
        file.write_all(&body)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("plan state: {error}"));
    }
    Ok(())
}

async fn execute_local(
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
            // Split-host: the runner reads the intervention register on the
            // console host. Deliver there first; if that fails the action
            // fails rather than reporting a pause that does not pause.
            push_governance(
                state,
                serde_json::json!({"kind": "intervention", "record": record}),
            )
            .await?;
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
            let mut caps = json!({"source": "operator draft"});
            let raw = request.policy_json.trim();
            if !raw.is_empty() {
                // A draft is the review copy of a complete effective policy.
                // Store it only after the same validation activation applies,
                // so the activated file and the reviewed document cannot
                // diverge.
                let policy: apex_autopilot::model::AutopilotPolicy = serde_json::from_str(raw)
                    .map_err(|error| format!("The policy JSON is invalid: {error}"))?;
                policy.validate().map_err(|reason| {
                    format!("The policy is outside the documented bounds: {reason}")
                })?;
                caps = json!({
                    "source": "operator draft",
                    "policy": policy,
                    "policyHash": policy.canonical_hash().map_err(|error| error.to_string())?,
                });
            }
            let record = PolicyRecord::draft(
                actor,
                now_ms,
                caps,
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
            // A draft that carries the complete policy activates the same
            // effective document the runner consumes. The write is atomic and
            // the record binds the canonical hash of exactly that document, so
            // an approval reviewed against the activation cannot drift.
            let effective = match draft.caps.get("policy") {
                Some(value) => {
                    let policy: apex_autopilot::model::AutopilotPolicy =
                        serde_json::from_value(value.clone()).map_err(|error| {
                            format!("The draft's policy JSON is invalid: {error}")
                        })?;
                    policy.validate().map_err(|reason| {
                        format!("The draft's policy is outside the documented bounds: {reason}")
                    })?;
                    push_governance(
                        state,
                        serde_json::json!({"kind": "policy", "policy": policy}),
                    )
                    .await?;
                    Some(
                        crate::governance::write_effective_policy(
                            &state.config.effective_policy_path,
                            &policy,
                        )
                        .map_err(|error| error.to_string())?,
                    )
                }
                None => None,
            };
            activated.state = "active".to_string();
            activated.activated_by = Some(actor.to_string());
            activated.activated_at_ms = Some(now_ms);
            match &effective {
                Some(written) => {
                    activated.policy_hash = Some(written.canonical_hash.clone());
                }
                None => {
                    // A legacy metadata-only draft still activates, but it
                    // binds to the currently effective document when one
                    // exists so the approval cannot silently float.
                    if state.config.effective_policy_path.exists() {
                        let policy = apex_autopilot::model::load_policy_file(
                            &state.config.effective_policy_path,
                        )
                        .map_err(|error| error.to_string())?;
                        activated.policy_hash = Some(
                            policy
                                .canonical_hash()
                                .map_err(|error| error.to_string())?,
                        );
                    }
                }
            }
            state
                .governance
                .append_policy(&activated)
                .map_err(|error| error.to_string())?;
            match effective {
                Some(written) => Ok(format!(
                    "Policy {} activated by {actor}; effective policy {} written to {} (canonical sha256 {}).",
                    activated.policy_id, written.version, written.path, written.canonical_hash
                )),
                None => Ok(format!(
                    "Policy {} activated by {actor}; the draft, preview and activation time stay traceable.",
                    activated.policy_id
                )),
            }
        }
        Capability::PolicySet => {
            let raw = request.policy_json.trim();
            if raw.is_empty() {
                return Err("A full policy JSON document is required.".to_string());
            }
            let policy: apex_autopilot::model::AutopilotPolicy =
                serde_json::from_str(raw).map_err(|error| {
                    format!("The policy JSON is not a valid AutopilotPolicy: {error}")
                })?;
            policy.validate().map_err(|reason| {
                format!("The policy is outside the documented bounds: {reason}")
            })?;
            // The console host's runner and policy page must see the same
            // document; deliver it before recording anything locally.
            push_governance(
                state,
                serde_json::json!({"kind": "policy", "policy": policy}),
            )
            .await?;
            let written = crate::governance::write_effective_policy(
                &state.config.effective_policy_path,
                &policy,
            )
            .map_err(|error| error.to_string())?;
            let version = state
                .governance
                .next_version()
                .map_err(|error| error.to_string())?;
            let mut record = PolicyRecord::draft(
                actor,
                now_ms,
                json!({
                    "policy": policy,
                    "hash": written.hash,
                    "effectivePath": written.path,
                }),
                json!({
                    "windowDays": policy.window_days,
                    "burnInDays": policy.burn_in_days,
                }),
                json!({"thresholds": policy.thresholds}),
                "Future proposals; active experiments keep their registered policy and no signed fee basis changes.",
                version,
                correlation,
            );
            record.state = "active".to_string();
            record.policy_hash = Some(written.canonical_hash.clone());
            record.activated_by = Some(actor.to_string());
            record.activated_at_ms = Some(now_ms);
            state
                .governance
                .append_policy(&record)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Effective policy {} written to {} (sha256 {}, register version {version}). The runner reads it on its next cycle.",
                written.version, written.path, written.hash
            ))
        }
        Capability::ManagedPlanActivate => {
            let publisher = request.target.trim();
            validate_publisher(publisher)?;
            let revenue = parse_revenue(&request.revenue_30d_micros)?;
            let tier = parse_tier(&request.tier)?;
            let path = apex_billing::PlanState::path_for(
                &state.config.connections_root,
                publisher,
                apex_billing::PlanId::Managed,
            );
            let existing = apex_billing::PlanState::load(&path)
                .map_err(|error| format!("plan state: {error}"))?;
            if let Some(existing) = existing {
                if existing.plan == apex_billing::PlanId::Managed
                    && existing.active
                    && !existing.cancelled()
                {
                    return Ok(format!(
                        "The managed plan for {publisher} is already active since {}; nothing changed.",
                        existing.since
                    ));
                }
            }
            let since =
                apex_analytics::UtcMillis::from_unix_millis(now_ms).to_day_string();
            let mut plan = apex_billing::ManagedPlan::default().activate(revenue, &since);
            plan.tier = tier;
            // Split-host: the console host's runner and billing view read
            // the plan state there, so deliver the same document first.
            push_governance(
                state,
                serde_json::json!({
                    "kind": "managed_plan",
                    "publisher": publisher,
                    "state": plan,
                }),
            )
            .await?;
            // The shared plan-state file is read by the runner and console.
            // `PlanState::save` is a plain write (umask-dependent), so this
            // host writes the same bytes through an owner-only atomic rename.
            write_plan_state_0600(&path, &plan)?;
            Ok(format!(
                "Managed plan activated for {publisher} at {} micros/month from {since} ({}). The runner and console read the same plan state file.",
                plan.monthly_fee_micros,
                path.display()
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
        | Capability::RequestReconnection
        | Capability::RecheckRequest
        | Capability::RestoreReconcileRequest
        | Capability::JobRetry => Err("this capability writes a durable job".to_string()),
    }
}

/// Publisher ids are path segments in the plan-state contract: refuse
/// anything that could escape the vault root or name no publisher.
fn validate_publisher(publisher: &str) -> Result<(), String> {
    if publisher.is_empty() || publisher.len() > 64 {
        return Err("publisher must be 1 to 64 characters".to_string());
    }
    let mut bytes = publisher.bytes();
    let Some(first) = bytes.next() else {
        return Err("publisher is required".to_string());
    };
    if !first.is_ascii_alphanumeric() {
        return Err("publisher must start with a letter or digit".to_string());
    }
    if !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')) {
        return Err("publisher may contain only letters, digits, '.', '-' and '_'".to_string());
    }
    Ok(())
}

fn parse_revenue(raw: &str) -> Result<i64, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(0);
    }
    let revenue = raw
        .parse::<i64>()
        .map_err(|_| "revenue_30d_micros must be a whole number of micros".to_string())?;
    if revenue < 0 {
        return Err("revenue_30d_micros cannot be negative".to_string());
    }
    Ok(revenue)
}

fn parse_tier(raw: &str) -> Result<Option<apex_billing::MonitoringTier>, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "none" => Ok(None),
        "base" => Ok(Some(apex_billing::MonitoringTier::Base)),
        "mid" => Ok(Some(apex_billing::MonitoringTier::Mid)),
        "top" => Ok(Some(apex_billing::MonitoringTier::Top)),
        other => Err(format!(
            "unknown monitoring tier '{other}' (expected base, mid, top or none)"
        )),
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
            let experiment = state.data.experiment_by_id(target).ok()??;
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
            // Read the one job by id: in the split-host mode the folder is
            // not on this host, so the transport answers from the endpoint's
            // GET instead of a local listing.
            let job_id = target.strip_suffix(".json").unwrap_or(target);
            let job = state.jobs.read(job_id).await.ok()?;
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
            let invoice = state.data.invoice_by_id(target).ok()??;
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
        Capability::PolicySet => {
            let active = state.governance.active_policies().ok()?.len();
            let effective = state.config.effective_policy_path.exists();
            Some((
                "policy:effective".to_string(),
                json!({
                    "active_registers": active,
                    "effective_file_present": effective,
                    "path": state.config.effective_policy_path.display().to_string(),
                }),
            ))
        }
        Capability::ManagedPlanActivate => Some((
            format!("publisher:{target}"),
            json!({"publisher": target, "plan": "managed"}),
        )),
        Capability::OperatorActionReview => {
            let entry = state.indexed_action(target)?;
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
    entry.action_key()
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
    payload_fingerprint: &str,
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
        payload_fingerprint: Some(payload_fingerprint.to_string()),
        result_id: result_id.map(str::to_string),
        job_id: job_id.map(str::to_string),
        correlation_id: correlation.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::Role;

    async fn submit_for_test(
        state: &ControlState,
        capability: Capability,
        request: ActionRequest,
        role: crate::capabilities::Role,
    ) -> SubmitOutcome {
        let csrf = request.csrf_token.clone();
        submit(state, capability, &request, "maya", role, &csrf, true).await
    }

    #[tokio::test]
    async fn policy_activation_writes_the_effective_file_and_binds_its_hash() {
        let state = crate::fixture_render::fixture_state().expect("fixture state");
        let policy = apex_autopilot::model::AutopilotPolicy::conservative();
        let mut draft = PolicyRecord::draft(
            "maya",
            state.now_ms(),
            serde_json::json!({"policy": policy}),
            serde_json::json!({}),
            serde_json::json!({}),
            "policy under review",
            1,
            "corr-activate-effective",
        );
        draft.state = "pending".to_string();
        state.governance.append_policy(&draft).expect("draft");
        let pending = state.governance.pending_policies().expect("pending");
        let record = pending
            .iter()
            .find(|record| record.policy_id == draft.policy_id)
            .expect("draft");
        let object = serde_json::json!({
            "policy_id": record.policy_id,
            "version": record.version,
            "state": record.state,
        });
        let token = state
            .versions
            .observe(&format!("policy:{}", draft.policy_id), &object);
        let request = ActionRequest {
            target: draft.policy_id.clone(),
            policy_id: draft.policy_id.clone(),
            reason: "activate the reviewed policy".to_string(),
            confirmation: "ACTIVATE POLICY".to_string(),
            idempotency_key: "a".repeat(32),
            version_token: token.encode(),
            csrf_token: "c".repeat(32),
            return_path: "/control/system/policies".to_string(),
            ..ActionRequest::default()
        };
        let outcome =
            submit_for_test(&state, Capability::PolicyActivate, request, Role::Security).await;
        match outcome {
            SubmitOutcome::Result {
                state: ActionState::Confirmed,
                ..
            } => {}
            other => panic!("activation must confirm: {other:?}"),
        }
        let written = apex_autopilot::model::load_policy_file(&state.config.effective_policy_path)
            .expect("effective policy");
        assert_eq!(written, policy, "the runner's file is the reviewed policy");
        let active = state.governance.active_policies().expect("active");
        let activated = active
            .iter()
            .find(|record| record.policy_id == draft.policy_id)
            .expect("activated");
        assert_eq!(
            activated.policy_hash.as_deref(),
            Some(policy.canonical_hash().expect("hash").as_str()),
            "the activation binds the canonical hash, not the version label"
        );
    }

    #[tokio::test]
    async fn a_failed_journal_append_never_reports_confirmed_or_requested() {
        let mut state = crate::fixture_render::fixture_state().expect("fixture state");
        let bad_journal = state.config.journal_path.with_extension("journal-dir");
        let _ = std::fs::remove_file(&bad_journal);
        std::fs::create_dir_all(&bad_journal).expect("dir");
        state.journal = crate::journal::ActionJournal::new(bad_journal);

        // The fleet object's version token matches its current state.
        let paused = state
            .governance
            .fleet_scheduling_paused()
            .expect("pause state");
        let object = serde_json::json!({"paused": paused});
        let token = state.versions.observe("fleet:scheduling", &object);
        let request = ActionRequest {
            target: "fleet:scheduling".to_string(),
            reason: "durable-journal failure test".to_string(),
            confirmation: "PAUSE SCHEDULING".to_string(),
            idempotency_key: "b".repeat(32),
            version_token: token.encode(),
            csrf_token: "d".repeat(32),
            return_path: "/control/system/policies".to_string(),
            ..ActionRequest::default()
        };
        let outcome = submit_for_test(
            &state,
            Capability::FleetPauseScheduling,
            request,
            Role::Operator,
        )
        .await;
        match outcome {
            SubmitOutcome::Result {
                state: ActionState::Failed,
                ..
            } => {}
            SubmitOutcome::Result {
                state: ActionState::Confirmed | ActionState::Requested,
                ..
            } => panic!("a failed durable append must never confirm or request"),
            other => panic!("unexpected outcome: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn plan_state_is_written_owner_only_and_atomically() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!(
            "apex-control-plan-{}-{}",
            std::process::id(),
            crate::signing::random_hex_32()
        ));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("northstar.managed.json");
        let plan = apex_billing::ManagedPlan::default().activate(2_000_000_000, "2026-10");
        write_plan_state_0600(&path, &plan).expect("write");
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("northstar") || text.contains("managed") || text.contains("tier"));
        assert!(!dir.join("northstar.managed.json.control-tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
