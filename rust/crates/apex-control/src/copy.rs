//! The frozen copy of the design document for the control plane.
//!
//! Every string here is quoted from `Apex_Identity_UI_UX_Complete.pdf`
//! (section 04 pages 53 to 72 and page 97's empty and failure table, plus
//! the operator sign-in page 96). Rendering code must use these constants
//! instead of retyping the sentences, so a copy change is one edit and the
//! tests can assert the exact text.

/// Page 97, Operations / no open incidents.
pub const EMPTY_OPERATIONS_HEADLINE: &str = "No customer-impact incidents are open.";
/// The `Last evaluated at <time>` tail of the operations empty state; `{}` is
/// the as-of time in the console's UTC format.
pub const EMPTY_OPERATIONS_BODY: &str =
    "No customer-impact incidents are open. Last evaluated at {as_of}. Data freshness is shown below.";
/// Page 97 action for the operations empty state.
pub const EMPTY_OPERATIONS_ACTION: &str = "Review data readiness.";

/// Page 97, Customers / no matches.
pub const EMPTY_CUSTOMERS: &str =
    "No customers match these filters. Clear a filter or search by publisher ID.";
/// Page 97 action for the customers empty state.
pub const EMPTY_CUSTOMERS_ACTION: &str = "Clear filters.";

/// Page 97, Ingestion / delayed.
pub const DELAYED_INGESTION: &str = "Reporting is behind the expected schedule. The latest pull and latest final day are shown separately.";
/// Page 97 action for the delayed ingestion state.
pub const DELAYED_INGESTION_ACTION: &str = "Inspect affected accounts.";

/// Page 97, Audits / no inputs.
pub const AUDIT_NO_INPUTS: &str =
    "This rule could not run. The required reporting input is missing for the selected scope.";
/// Page 97 action for the audits no-inputs state.
pub const AUDIT_NO_INPUTS_ACTION: &str = "Inspect missing inputs.";

/// Page 97, Experiments / awaiting data.
pub const AWAITING_DATA: &str =
    "A decision is waiting for finalized data. The elapsed window alone does not establish a result.";
/// Page 97 action for the awaiting-data state.
pub const AWAITING_DATA_ACTION: &str = "Open source days.";

/// Page 97, Finance / proof hold.
pub const PROOF_HOLD: &str = "This fee is on hold. A required proof or reconciliation check did not pass. No collection is authorized from this state.";
/// Page 97 action for the proof-hold state.
pub const PROOF_HOLD_ACTION: &str = "Review proof.";

/// Page 97, Growth / incomplete source.
pub const INCOMPLETE_SOURCE: &str = "Conversion is not available for this cohort yet. The follow-up period is incomplete or the source events are missing.";
/// Page 97 action for the incomplete-source state.
pub const INCOMPLETE_SOURCE_ACTION: &str = "Show cohort coverage.";

/// Page 97, System / telemetry absent.
pub const TELEMETRY_ABSENT: &str = "This signal is not instrumented. No healthy or zero-error claim can be made from missing telemetry.";
/// Page 97 action for the telemetry-absent state.
pub const TELEMETRY_ABSENT_ACTION: &str = "Open collection status.";

/// Page 97, Access / denied action.
pub const DENIED_HEADLINE: &str = "Your role does not permit this action.";
/// Page 97 denied-action body.
pub const DENIED_BODY: &str = "Your role does not permit this action. No operation was requested.";
/// Page 97 denied-action recovery action.
pub const DENIED_ACTION: &str = "Return to the object.";

/// Page 96, sign-in heading.
pub const LOGIN_HEADLINE: &str = "Operator access";
/// Page 96, sign-in lead.
pub const LOGIN_LEAD: &str = "Sign in to the Apex control plane.";
/// Page 96, sign-in separation statement.
pub const LOGIN_SEPARATION: &str = "Customer account access does not grant operator access.";
/// Page 96, failed verification copy. Only safe fields are preserved.
pub const LOGIN_FAILED: &str = "We could not verify those access details.";
/// Page 96, the hint under the password step.
pub const LOGIN_NEXT_STEP: &str = "Next: authenticator code";
/// Page 96, the primary submit label.
pub const LOGIN_CONTINUE: &str = "Continue";
/// Label of the password field.
pub const LOGIN_PASSWORD_LABEL: &str = "Password";
/// Label of the operator-name field.
pub const LOGIN_OPERATOR_LABEL: &str = "Operator name";
/// Label of the code field.
pub const LOGIN_CODE_LABEL: &str = "Authenticator code";
/// Heading of the authenticator-code step.
pub const LOGIN_CODE_HEADLINE: &str = "Enter the authenticator code";
/// Body of the authenticator-code step.
pub const LOGIN_CODE_LEAD: &str = "Enter the current six-digit code from your authenticator app.";
/// Rate-limit headline (no document copy exists for this state; short and
/// factual in the same voice).
pub const LOGIN_RATE_LIMITED_HEADLINE: &str = "Too many attempts.";
/// Rate-limit body, with `{until}` the moment the lockout ends rendered in
/// the console's UTC format (`14 Oct 2026, 20:05 UTC`); the placeholder
/// already carries the `UTC` suffix.
pub const LOGIN_RATE_LIMITED_BODY: &str =
    "Sign-in is locked for this operator and client until {until}. Wait for the lockout to end.";

/// The unconfigured-deployment notices (no document copy exists; the
/// deployment must be self-describing).
pub const UNCONFIGURED_HEADLINE: &str = "This control plane is not configured.";
/// Body explaining that configuration is required and no sign-in is possible.
pub const UNCONFIGURED_BODY: &str = "No operator configuration file is present, so no session can be issued. Set APEX_CONTROL_OPERATORS to the operator configuration file and restart the service.";
/// Heading of the per-operator missing-second-factor notice.
pub const OPERATOR_NO_SECOND_FACTOR_HEADLINE: &str =
    "This operator has no authenticator configured.";
/// Body of the missing-second-factor notice.
pub const OPERATOR_NO_SECOND_FACTOR_BODY: &str = "A session can only be issued after both factors. Add totp_secret_base32 to this operator's configuration entry.";

/// The session-expired page (no document copy exists; page 96 says the page
/// returns the operator to a safe GET view).
pub const SESSION_EXPIRED_HEADLINE: &str = "Your session ended.";
/// Body of the session-expired page.
pub const SESSION_EXPIRED_BODY: &str =
    "Sign in again to continue. No operation was requested by the expired session.";

/// The expired sign-in-step page (between the two factors).
pub const LOGIN_STEP_EXPIRED_HEADLINE: &str = "This sign-in step expired.";
/// Body of the expired sign-in step.
pub const LOGIN_STEP_EXPIRED_BODY: &str =
    "Too much time passed between the password and the authenticator code. Start again.";

/// The step-up (recent authentication) page heading.
pub const STEP_UP_HEADLINE: &str = "Confirm it is still you.";
/// Body of the step-up page; page 96: sensitive actions require recent
/// authentication.
pub const STEP_UP_BODY: &str =
    "This action is sensitive and needs a fresh sign-in. Confirm your password and authenticator code to continue.";

/// The conflict page used when the stale-state check fails (page 64: the
/// check runs on the server; no console copy exists, so the page states the
/// fact plainly).
pub const CONFLICT_HEADLINE: &str = "This object changed while the form was open.";
/// Body of the conflict page.
pub const CONFLICT_BODY: &str =
    "The object no longer matches the view this form was prepared from, so no operation was requested. Reload the object and review the current state before acting.";
/// The conflict detail when one idempotency key is replayed for a different
/// target or payload (no document copy exists; short and factual).
pub const CONFLICT_KEY_REUSED: &str = "This request key was already used for a different action.";

/// The action result states (page 64).
pub const RESULT_REQUESTED: &str = "Requested";
/// Result state: the durable job is running.
pub const RESULT_RUNNING: &str = "Running";
/// Result state: a confirming read observed the change.
pub const RESULT_CONFIRMED: &str = "Confirmed";
/// Result state: the operation failed.
pub const RESULT_FAILED: &str = "Failed";
/// Result state: the platform outcome is not known yet.
pub const RESULT_STATE_UNKNOWN: &str = "State unknown";

/// Journal and result-page note: an interrupted request never implies the
/// platform did nothing (page 64).
pub const RESULT_RECONCILE_NOTE: &str = "An interrupted request never implies the platform did nothing. The next attempt reconciles state before retrying.";

/// The "no manual Keep" note shown on experiment pages (page 62).
pub const NO_MANUAL_KEEP: &str =
    "An operator cannot type a better result. A disputed or invalid test is held, re-evaluated under a recorded policy or voided.";

/// The finance definitions note (page 65).
pub const NO_MRR: &str = "Separate lift fees, monitoring, tax, credits, refunds, overdue debt and collected cash. Do not label their sum as MRR.";

/// The four clocks sentence (page 59).
pub const FOUR_CLOCKS: &str =
    "Last completed pull and latest finalized source day are different clocks.";

/// The read-denominator explanation for effectiveness (page 67).
pub const READ_THE_DENOMINATOR: &str = "Read the denominator";

/// Proof-hold note on fee rows and invoices.
pub const NO_COLLECTION_AUTHORIZED: &str = "No collection is authorized from this state.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_97_table_is_reproduced_exactly() {
        assert_eq!(
            EMPTY_OPERATIONS_BODY.replace("{as_of}", "09:10 UTC"),
            "No customer-impact incidents are open. Last evaluated at 09:10 UTC. Data freshness is shown below."
        );
        assert_eq!(
            EMPTY_CUSTOMERS,
            "No customers match these filters. Clear a filter or search by publisher ID."
        );
        assert_eq!(
            DELAYED_INGESTION,
            "Reporting is behind the expected schedule. The latest pull and latest final day are shown separately."
        );
        assert_eq!(
            AUDIT_NO_INPUTS,
            "This rule could not run. The required reporting input is missing for the selected scope."
        );
        assert_eq!(
            AWAITING_DATA,
            "A decision is waiting for finalized data. The elapsed window alone does not establish a result."
        );
        assert_eq!(
            PROOF_HOLD,
            "This fee is on hold. A required proof or reconciliation check did not pass. No collection is authorized from this state."
        );
        assert_eq!(
            INCOMPLETE_SOURCE,
            "Conversion is not available for this cohort yet. The follow-up period is incomplete or the source events are missing."
        );
        assert_eq!(
            TELEMETRY_ABSENT,
            "This signal is not instrumented. No healthy or zero-error claim can be made from missing telemetry."
        );
        assert_eq!(
            DENIED_BODY,
            "Your role does not permit this action. No operation was requested."
        );
        assert_eq!(DENIED_ACTION, "Return to the object.");
    }

    #[test]
    fn sign_in_copy_is_reproduced_exactly() {
        assert_eq!(LOGIN_FAILED, "We could not verify those access details.");
        assert_eq!(LOGIN_LEAD, "Sign in to the Apex control plane.");
        assert_eq!(
            LOGIN_SEPARATION,
            "Customer account access does not grant operator access."
        );
        assert_eq!(LOGIN_NEXT_STEP, "Next: authenticator code");
        assert_eq!(LOGIN_CONTINUE, "Continue");
    }

    #[test]
    fn the_rate_limit_body_names_utc_exactly_once() {
        let rendered = LOGIN_RATE_LIMITED_BODY
            .replace("{until}", &apex_ui::format::utc_datetime(1_791_965_400_000));
        assert_eq!(
            rendered,
            "Sign-in is locked for this operator and client until 14 Oct 2026, 08:10 UTC. Wait for the lockout to end."
        );
        assert_eq!(rendered.matches("UTC").count(), 1, "{rendered}");
    }

    #[test]
    fn no_copy_contains_an_em_dash() {
        let all = [
            EMPTY_OPERATIONS_HEADLINE,
            EMPTY_OPERATIONS_BODY,
            EMPTY_OPERATIONS_ACTION,
            EMPTY_CUSTOMERS,
            EMPTY_CUSTOMERS_ACTION,
            DELAYED_INGESTION,
            DELAYED_INGESTION_ACTION,
            AUDIT_NO_INPUTS,
            AUDIT_NO_INPUTS_ACTION,
            AWAITING_DATA,
            AWAITING_DATA_ACTION,
            PROOF_HOLD,
            PROOF_HOLD_ACTION,
            INCOMPLETE_SOURCE,
            INCOMPLETE_SOURCE_ACTION,
            TELEMETRY_ABSENT,
            TELEMETRY_ABSENT_ACTION,
            DENIED_HEADLINE,
            DENIED_BODY,
            DENIED_ACTION,
            LOGIN_HEADLINE,
            LOGIN_LEAD,
            LOGIN_SEPARATION,
            LOGIN_FAILED,
            LOGIN_NEXT_STEP,
            LOGIN_CONTINUE,
            LOGIN_CODE_HEADLINE,
            LOGIN_CODE_LEAD,
            LOGIN_RATE_LIMITED_HEADLINE,
            LOGIN_RATE_LIMITED_BODY,
            UNCONFIGURED_HEADLINE,
            UNCONFIGURED_BODY,
            OPERATOR_NO_SECOND_FACTOR_HEADLINE,
            OPERATOR_NO_SECOND_FACTOR_BODY,
            SESSION_EXPIRED_HEADLINE,
            SESSION_EXPIRED_BODY,
            STEP_UP_HEADLINE,
            STEP_UP_BODY,
            CONFLICT_HEADLINE,
            CONFLICT_BODY,
            CONFLICT_KEY_REUSED,
            RESULT_STATE_UNKNOWN,
            RESULT_RECONCILE_NOTE,
            NO_MANUAL_KEEP,
            NO_MRR,
            FOUR_CLOCKS,
            NO_COLLECTION_AUTHORIZED,
        ];
        for text in all {
            assert!(!text.contains('\u{2014}'), "em dash in: {text}");
            assert!(!text.contains("  "), "double space in: {text}");
            assert_eq!(text.trim(), text, "padded copy: {text}");
        }
    }
}
