//! Change-log pages: the access panel, the verified records list and the
//! record detail.
//!
//! The access token is exchanged over `POST /console/ledger` for an opaque
//! session and never travels in a URL: the panel's form method is POST, the
//! session lives in an `HttpOnly`, host-only, `SameSite=Lax` cookie, and the
//! records pages carry a per-session CSRF token for the session-end action.
//! Records are only ever rendered from the single verified read of the
//! publisher's ledger.

use apex_core::platform::ExperimentMetric;
use apex_core::ExperimentState;
use apex_proof::{ChangeKind, LedgerRecord};
use apex_ui::components::{
    action, action_button, csrf_hidden, data_table, details, field, input, notice, pagination,
    panel, Cell, EmptyState,
};
use apex_ui::format::{self, ecpm, money, or_not_available, percent, utc_datetime};
use apex_ui::status::{self, StatusKind};

use super::{console_document, html_escape, ConsoleCtx};

/// How many verified records one change-log page renders. The paginated list
/// keeps one render's row work bounded regardless of the ledger's length.
pub const RECORDS_PER_PAGE: usize = 100;

/// The slice of the change log a render receives: only the current page's
/// records plus the totals the pagination note needs.
#[derive(Debug, Clone, Copy)]
pub struct RecordsPage<'a> {
    /// The records on this page, in chain order.
    pub records: &'a [LedgerRecord],
    /// Every verified record in the publisher's ledger.
    pub total: u64,
    /// The 1-based page number, already validated by the route.
    pub page: u64,
}

/// Why the access panel is being rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerAccessProblem {
    /// Wrong or missing access details (401, frozen copy).
    InvalidAccess,
    /// The session ended; enter the details again.
    SessionExpired,
    /// The publisher field is not a valid publisher id (400).
    InvalidPublisher,
}

impl LedgerAccessProblem {
    /// The HTTP status this problem answers with.
    pub fn status(&self) -> u16 {
        match self {
            LedgerAccessProblem::InvalidAccess => 401,
            _ => 400,
        }
    }
}

/// The change-log access panel (`GET /console/ledger` without a session).
pub fn render_access_panel(
    ctx: &ConsoleCtx,
    problem: Option<LedgerAccessProblem>,
    raw_publisher: Option<&str>,
) -> String {
    let mut body = String::from("<h1>Open your signed change log</h1>");
    body.push_str(
        "<p>Records are verified before they are displayed. Use the access details provided for \
         your publisher account.</p>",
    );
    body.push_str(&problem_notice(problem));
    let publisher_error = match problem {
        Some(LedgerAccessProblem::InvalidPublisher) => Some("Enter a valid publisher ID."),
        _ => None,
    };
    let form = format!(
        "<form method=\"post\" action=\"/console/ledger\">{}{}\
         <div class=\"form-actions\">\
         <button class=\"action action-primary\" type=\"submit\">Open change log</button></div>\
         </form>",
        field(
            "publisher",
            "Publisher ID",
            None,
            publisher_error,
            &input(
                "publisher",
                "publisher",
                "text",
                Some(raw_publisher.unwrap_or("")),
                false,
                Some("off"),
            ),
        ),
        field(
            "token",
            "Access token",
            Some("Sent in the request body, never in the URL."),
            None,
            &input("token", "token", "password", None, false, Some("off")),
        ),
    );
    body.push_str(&form);
    console_document(ctx, "Change log", "Change log", &body)
}

fn problem_notice(problem: Option<LedgerAccessProblem>) -> String {
    match problem {
        None => String::new(),
        Some(LedgerAccessProblem::InvalidAccess) => notice(
            StatusKind::Failure,
            "We could not open this publisher's history.",
            "Check the access details or contact support.",
            Some(("Contact support", apex_ui::shell::DEFAULT_SUPPORT_URL)),
        ),
        Some(LedgerAccessProblem::SessionExpired) => notice(
            StatusKind::Attention,
            "Your access session has ended.",
            "Enter the access details again to continue.",
            None,
        ),
        Some(LedgerAccessProblem::InvalidPublisher) => notice(
            StatusKind::Failure,
            "Check the request.",
            "A field or upload could not be accepted.",
            None,
        ),
    }
}

/// The human label of every ledger record variant.
pub fn human_label(record: &LedgerRecord) -> &'static str {
    match record {
        LedgerRecord::ChangeApplied { .. } => "Change applied",
        LedgerRecord::ExperimentStarted { .. } => "Experiment started",
        LedgerRecord::ExperimentObserved { .. } => "Experiment observed",
        LedgerRecord::RollbackPerformed { .. } => "Rollback performed",
        LedgerRecord::BillingProof { .. } => "Billing proof",
        LedgerRecord::LiftRecord { .. } => "Lift record",
        LedgerRecord::PromotionPending { .. } => "Promotion pending",
    }
}

/// The Scope cell of a record: its ad unit or experiment, `-` when the
/// variant carries neither.
pub fn record_scope(record: &LedgerRecord) -> String {
    match record {
        LedgerRecord::ChangeApplied { ad_unit_id, .. }
        | LedgerRecord::RollbackPerformed { ad_unit_id, .. } => ad_unit_id.clone(),
        LedgerRecord::ExperimentStarted { experiment_id, .. }
        | LedgerRecord::ExperimentObserved { experiment_id, .. }
        | LedgerRecord::BillingProof { experiment_id, .. }
        | LedgerRecord::PromotionPending { experiment_id, .. } => experiment_id.clone(),
        LedgerRecord::LiftRecord { .. } => "-".to_string(),
    }
}

/// The identifier after the label on the detail page (`Change applied /
/// rw_de_01`).
pub fn record_identifier(record: &LedgerRecord) -> String {
    match record {
        LedgerRecord::LiftRecord { payload_sha256, .. } => {
            format!("payload {}", short_digest(payload_sha256))
        }
        _ => record_scope(record),
    }
}

/// The first 12 characters of a hex digest.
fn short_digest(digest: &str) -> String {
    digest.chars().take(12).collect()
}

/// A one-line, human summary of a record.
pub fn record_summary(record: &LedgerRecord, currency: &str) -> String {
    match record {
        LedgerRecord::PromotionPending {
            deadline_unix_ms,
            instructions,
            ..
        } => format!(
            "Kept, waiting for the publisher to promote it by {}: {instructions}",
            apex_ui::format::utc_datetime(*deadline_unix_ms)
        ),
        LedgerRecord::ChangeApplied {
            ad_unit_id,
            change,
            prev_value,
            ..
        } => {
            let change_text = match change {
                ChangeKind::CountryFloorSet {
                    country,
                    floor_ecpm,
                } => format!(
                    "Set the {country} country floor to {} eCPM on ad unit {ad_unit_id}",
                    ecpm(*floor_ecpm, currency)
                ),
                ChangeKind::EcpmSet {
                    mediation_group_id,
                    ad_unit_id,
                    ecpm: value,
                } => format!(
                    "Set mediation group {mediation_group_id} to {} eCPM on ad unit {ad_unit_id}",
                    ecpm(*value, currency)
                ),
                ChangeKind::WaterfallEntryPriceSet {
                    ad_unit_id,
                    entry_id,
                    price_ecpm,
                } => format!(
                    "Set waterfall entry {entry_id} to {} eCPM on ad unit {ad_unit_id}",
                    ecpm(*price_ecpm, currency)
                ),
                ChangeKind::WaterfallEntryCreated {
                    ad_unit_id,
                    network,
                    instance_name,
                    price_ecpm,
                } => format!(
                    "Created waterfall entry {instance_name} for {network} at {} eCPM on ad unit \
                     {ad_unit_id}",
                    ecpm(*price_ecpm, currency)
                ),
                ChangeKind::WaterfallEntryRemoved {
                    ad_unit_id,
                    entry_id,
                } => {
                    format!("Removed waterfall entry {entry_id} on ad unit {ad_unit_id}")
                }
                ChangeKind::FrequencyCapSet {
                    ad_unit_id,
                    cap_type,
                    countries,
                    session_limit,
                    day_limit,
                    minute_frequency,
                } => format!(
                    "Set a {cap_type} frequency cap on ad unit {ad_unit_id} for [{}] (session {}, day {}, minute {})",
                    countries.join(", "),
                    session_limit.unwrap_or(0),
                    day_limit.unwrap_or(0),
                    minute_frequency.unwrap_or(0)
                ),
                ChangeKind::LevelplayGroupCapping {
                    group_id,
                    session_limit,
                    interstitial_limit,
                    day_limit,
                } => format!(
                    "Set LevelPlay group {group_id} capping (session {}, interstitial {}, day {})",
                    session_limit.unwrap_or(0),
                    interstitial_limit.unwrap_or(0),
                    day_limit.unwrap_or(0)
                ),
            };
            let previous = if prev_value.trim().is_empty() {
                or_not_available("no previous value recorded")
            } else {
                format!("previous value {prev_value}")
            };
            format!("{change_text} / {previous}")
        }
        LedgerRecord::ExperimentStarted {
            experiment_id,
            spec,
            ..
        } => format!(
            "Started experiment {experiment_id}: {} on ad unit {}, {}% test allocation",
            spec.name, spec.ad_unit_id, spec.test_group_allocation
        ),
        LedgerRecord::ExperimentObserved {
            experiment_id,
            result,
            ..
        } => {
            let lift = result
                .lift_percent_bp
                .map(lift_label)
                .unwrap_or_else(|| or_not_available("no lift estimate"));
            format!(
                "Observed experiment {experiment_id}: {}, observed lift {lift}",
                experiment_state_label(result.state)
            )
        }
        LedgerRecord::RollbackPerformed {
            ad_unit_id,
            rolled_back_seq,
            reason,
            ..
        } => format!("Rolled back change {rolled_back_seq} on ad unit {ad_unit_id}: {reason}"),
        LedgerRecord::BillingProof {
            experiment_id,
            proven_lift,
            rate_bp,
            fee_micros,
            ..
        } => format!(
            "Billing proof for {experiment_id}: fee {}, rate {}, proven lift {}",
            money(*fee_micros, currency),
            percent(f64::from(*rate_bp) / 10_000.0),
            money(*proven_lift, currency)
        ),
        LedgerRecord::LiftRecord { payload_sha256, .. } => {
            format!(
                "Lift record pinned to payload digest {}",
                short_digest(payload_sha256)
            )
        }
    }
}

/// The human label of an experiment state.
pub fn experiment_state_label(state: ExperimentState) -> &'static str {
    match state {
        ExperimentState::Running => "running",
        ExperimentState::Stopped => "stopped",
        ExperimentState::Completed => "completed",
    }
}

/// An experiment lift in basis points (`300` becomes `+3.0%`).
fn lift_label(bp: i64) -> String {
    format::delta(1.0, 1.0 + bp as f64 / 10_000.0)
}

fn metric_label(metric: ExperimentMetric) -> &'static str {
    match metric {
        ExperimentMetric::Ecpm => "eCPM",
        ExperimentMetric::Revenue => "Revenue",
        ExperimentMetric::Arpdau => "ARPDAU",
    }
}

/// The verified records list (`GET /console/ledger?page=N` with a session).
pub fn render_records(
    ctx: &ConsoleCtx,
    publisher: &str,
    page: RecordsPage<'_>,
    verified_at_unix_ms: i64,
    csrf: &str,
) -> String {
    let mut body = format!(
        "<h1>Signed change log</h1><p>{}</p>",
        status::chip("Verified", StatusKind::Verified)
    );
    let total = page.total;
    body.push_str(&format!(
        "<p class=\"table-note\">{} {} / Verified {} / Publisher {}</p>",
        grouped(total),
        if total == 1 { "record" } else { "records" },
        utc_datetime(verified_at_unix_ms),
        html_escape(publisher)
    ));
    let rows: Vec<Vec<Cell>> = page
        .records
        .iter()
        .map(|record| {
            vec![
                Cell::Text(utc_datetime(record.at_unix_ms())),
                Cell::Link(
                    human_label(record).to_string(),
                    format!("/console/ledger/records/{}", record.seq()),
                ),
                Cell::Mono(record_scope(record)),
                Cell::Text(record_summary(record, &ctx.currency)),
            ]
        })
        .collect();
    let empty = EmptyState {
        reason: "No changes have been recorded. Your verified history is empty for this publisher."
            .to_string(),
        action: Some(("Open the audit".to_string(), "/console/audit".to_string())),
    };
    body.push_str(&data_table(
        Some("Records"),
        &["When (UTC)", "Event", "Scope", "Result"],
        &rows,
        Some(empty),
        None,
    ));
    if total > 0 {
        body.push_str(&pagination_nav(page));
    }
    body.push_str(&session_end_form(csrf));
    console_document(ctx, "Change log", "Change log", &body)
}

/// The design's pagination component over stable GET URLs. The page links
/// carry only the page number, so the session cookie (the whole access
/// boundary) remains the only credential in the request.
fn pagination_nav(page: RecordsPage<'_>) -> String {
    let offset = (page.page.saturating_sub(1)).saturating_mul(RECORDS_PER_PAGE as u64);
    let first = offset.saturating_add(1);
    let last = offset.saturating_add(page.records.len() as u64);
    let note = format!(
        "Showing {}-{} of {} records",
        grouped(first),
        grouped(last),
        grouped(page.total)
    );
    let previous = (page.page > 1).then(|| format!("/console/ledger?page={}", page.page - 1));
    let last_page = page.total.div_ceil(RECORDS_PER_PAGE as u64).max(1);
    let next = (page.page < last_page).then(|| format!("/console/ledger?page={}", page.page + 1));
    pagination(previous.as_deref(), next.as_deref(), &note)
}

/// Group a count with commas: `2000` becomes `2,000`.
fn grouped(value: u64) -> String {
    format::micros_exact(i64::try_from(value).unwrap_or(i64::MAX))
}

/// The CSRF-protected session-end form.
fn session_end_form(csrf: &str) -> String {
    format!(
        "<form method=\"post\" action=\"/console/ledger/session/end\">{}{}</form>",
        csrf_hidden(csrf),
        action_button("End session", "action", "end", false)
    )
}

/// The record detail (`GET /console/ledger/records/{seq}` with a session).
pub fn render_record_detail(
    ctx: &ConsoleCtx,
    publisher: &str,
    record: &LedgerRecord,
    records_total: usize,
    csrf: &str,
) -> String {
    let label = human_label(record);
    let mut body = format!(
        "<h1>{} / {}</h1><p>{}</p>",
        html_escape(label),
        html_escape(&record_identifier(record)),
        status::chip("Verified record", StatusKind::Verified)
    );
    body.push_str(&format!(
        "<p class=\"table-note\">Record {} of {} / Recorded {} / Publisher {}</p>",
        record.seq(),
        records_total,
        utc_datetime(record.at_unix_ms()),
        html_escape(publisher)
    ));
    body.push_str(&panel(
        "Record summary",
        &format!(
            "<p>{}</p>",
            html_escape(&record_summary(record, &ctx.currency))
        ),
        None,
    ));
    body.push_str(&data_table(
        None,
        &["Field", "Value"],
        &detail_rows(record, &ctx.currency),
        None,
        None,
    ));
    body.push_str(&details_panel(record));
    body.push_str(&format!(
        "<p>{}</p>",
        action("Back to the change log", "/console/ledger", false)
    ));
    body.push_str(&session_end_form(csrf));
    console_document(ctx, "Record", "Change log", &body)
}

fn detail_rows(record: &LedgerRecord, currency: &str) -> Vec<Vec<Cell>> {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut push = |field: &str, value: String| {
        rows.push(vec![Cell::Text(field.to_string()), Cell::Text(value)]);
    };
    match record {
        LedgerRecord::PromotionPending {
            platform,
            experiment_id,
            ad_unit_id,
            deadline_unix_ms,
            instructions,
            ..
        } => {
            push("Platform", platform.as_str().to_string());
            push("Experiment", experiment_id.clone());
            push("Ad unit", ad_unit_id.clone());
            push(
                "Promotion deadline",
                apex_ui::format::utc_datetime(*deadline_unix_ms),
            );
            push("Instructions", instructions.clone());
        }
        LedgerRecord::ChangeApplied {
            platform,
            ad_unit_id,
            change,
            idempotency_key,
            prev_value,
            ..
        } => {
            push("Platform", platform.as_str().to_string());
            push("Ad unit", ad_unit_id.clone());
            push(
                "Change",
                match change {
                    ChangeKind::CountryFloorSet { country, floor_ecpm } => format!(
                        "Set the {country} country floor to {} eCPM",
                        ecpm(*floor_ecpm, currency)
                    ),
                    ChangeKind::EcpmSet {
                        mediation_group_id,
                        ad_unit_id,
                        ecpm: value,
                    } => format!(
                        "Set mediation group {mediation_group_id} to {} eCPM on ad unit {ad_unit_id}",
                        ecpm(*value, currency)
                    ),
                    ChangeKind::WaterfallEntryPriceSet {
                        ad_unit_id,
                        entry_id,
                        price_ecpm,
                    } => format!(
                        "Set waterfall entry {entry_id} to {} eCPM on ad unit {ad_unit_id}",
                        ecpm(*price_ecpm, currency)
                    ),
                    ChangeKind::WaterfallEntryCreated {
                        ad_unit_id,
                        network,
                        instance_name,
                        price_ecpm,
                    } => format!(
                        "Created waterfall entry {instance_name} for {network} at {} eCPM on ad \
                         unit {ad_unit_id}",
                        ecpm(*price_ecpm, currency)
                    ),
                    ChangeKind::WaterfallEntryRemoved { ad_unit_id, entry_id } => {
                        format!("Removed waterfall entry {entry_id} on ad unit {ad_unit_id}")
                    }
                    ChangeKind::FrequencyCapSet {
                        ad_unit_id,
                        cap_type,
                        session_limit,
                        day_limit,
                        minute_frequency,
                        ..
                    } => format!(
                        "Set a {cap_type} frequency cap on ad unit {ad_unit_id} (session {}, day {}, minute {})",
                        session_limit.unwrap_or(0),
                        day_limit.unwrap_or(0),
                        minute_frequency.unwrap_or(0)
                    ),
                    ChangeKind::LevelplayGroupCapping {
                        group_id,
                        session_limit,
                        interstitial_limit,
                        day_limit,
                    } => format!(
                        "Set LevelPlay group {group_id} capping (session {}, interstitial {}, day {})",
                        session_limit.unwrap_or(0),
                        interstitial_limit.unwrap_or(0),
                        day_limit.unwrap_or(0)
                    ),
                },
            );
            push(
                "Before",
                if prev_value.trim().is_empty() {
                    or_not_available("no previous value recorded")
                } else {
                    prev_value.clone()
                },
            );
            push("Idempotency key", idempotency_key.clone());
        }
        LedgerRecord::ExperimentStarted {
            platform,
            experiment_id,
            spec,
            ..
        } => {
            push("Platform", platform.as_str().to_string());
            push("Experiment", experiment_id.clone());
            push("Name", spec.name.clone());
            push("Ad unit", spec.ad_unit_id.clone());
            push("Metric", metric_label(spec.metric).to_string());
            push("Variants", spec.variants.to_string());
            push(
                "Test allocation",
                format!("{}% test allocation", spec.test_group_allocation),
            );
            push("Change description", spec.change_description.clone());
        }
        LedgerRecord::ExperimentObserved {
            platform,
            experiment_id,
            result,
            ..
        } => {
            push("Platform", platform.as_str().to_string());
            push("Experiment", experiment_id.clone());
            push("State", experiment_state_label(result.state).to_string());
            push(
                "Observed lift",
                result
                    .lift_percent_bp
                    .map(lift_label)
                    .unwrap_or_else(|| or_not_available("no lift estimate")),
            );
            push(
                "Confidence",
                result
                    .confidence
                    .map(|bp| percent(f64::from(bp) / 10_000.0))
                    .unwrap_or_else(|| or_not_available("no confidence interval recorded")),
            );
            push(
                "Winner",
                result
                    .winner_variant_id
                    .clone()
                    .unwrap_or_else(|| "-".to_string()),
            );
        }
        LedgerRecord::RollbackPerformed {
            platform,
            ad_unit_id,
            rolled_back_seq,
            reason,
            ..
        } => {
            push("Platform", platform.as_str().to_string());
            push("Ad unit", ad_unit_id.clone());
            push("Rolled back sequence", rolled_back_seq.to_string());
            push("Reason", reason.clone());
        }
        LedgerRecord::BillingProof {
            publisher_id,
            experiment_id,
            proven_lift,
            rate_bp,
            fee_micros,
            ..
        } => {
            push("Publisher", publisher_id.clone());
            push("Experiment", experiment_id.clone());
            push("Proven lift", money(*proven_lift, currency));
            push("Rate", percent(f64::from(*rate_bp) / 10_000.0));
            push("Fee", money(*fee_micros, currency));
        }
        LedgerRecord::LiftRecord {
            payload_sha256,
            payload,
            ..
        } => {
            push("Payload SHA-256", payload_sha256.clone());
            push("Payload", payload.clone());
        }
    }
    rows
}

/// The native details panel: sequence, UTC timestamp, previous hash, digest,
/// verification statement and the exact raw record JSON as signed.
fn details_panel(record: &LedgerRecord) -> String {
    let raw = serde_json::to_string(record).unwrap_or_default();
    let body = format!(
        "<p class=\"table-note\">Sequence {} / Recorded {} / Previous hash: <code>{}</code> / \
         Digest: <code>{}</code></p>\
         <p class=\"table-note\">The record's signature was verified as part of the whole-file \
         verification before this record was served.</p>\
         <pre>{}</pre>",
        record.seq(),
        utc_datetime(record.at_unix_ms()),
        hex::encode(record.prev_hash()),
        hex::encode(record.hash()),
        html_escape(&raw)
    );
    details("Technical details", &body, false)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_core::PlatformId;
    use apex_core::{ExperimentResult, ExperimentSpec, ExperimentState, ExperimentVariantResult};

    use super::*;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::new("Northstar Studio", "USD", true)
    }

    /// Seal drafts through the real chain engine; the returned records carry
    /// the sequences the in-memory ledger assigned.
    fn seal_all(drafts: Vec<LedgerRecord>) -> Vec<LedgerRecord> {
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[7u8; 32]));
        drafts
            .into_iter()
            .map(|draft| ledger.append(draft).expect("seal"))
            .collect()
    }

    fn change(at: i64, unit: &str, country: &str, floor: i64) -> LedgerRecord {
        LedgerRecord::change_applied(
            at,
            PlatformId::AppLovinMax,
            unit,
            ChangeKind::CountryFloorSet {
                country: country.into(),
                floor_ecpm: floor,
            },
            &format!("max-floor:{unit}:{country}"),
            "1000000",
        )
    }

    fn observed(at: i64, lift_bp: Option<i64>) -> LedgerRecord {
        LedgerRecord::experiment_observed(
            at,
            PlatformId::AppLovinMax,
            "exp_0247",
            ExperimentResult {
                experiment_id: "exp_0247".into(),
                state: ExperimentState::Completed,
                results: vec![ExperimentVariantResult {
                    variant_id: "control".into(),
                    impressions: 10_000,
                    revenue: 10_000_000,
                    ecpm: 1_000_000,
                }],
                lift_percent_bp: lift_bp,
                confidence: Some(9_800),
                winner_variant_id: Some("variant-a".into()),
            },
        )
    }

    fn started(at: i64) -> LedgerRecord {
        LedgerRecord::experiment_started(
            at,
            PlatformId::AppLovinMax,
            "exp_0247",
            ExperimentSpec {
                name: "floors-ee".into(),
                instance_id: String::new(),
                ad_unit_id: "rw_de_01".into(),
                metric: ExperimentMetric::Ecpm,
                variants: 2,
                test_group_allocation: 25,
                change_description: "raise DE floor".into(),
            },
        )
    }

    #[test]
    fn access_panel_is_a_post_form_with_no_token_in_the_url() {
        let html = render_access_panel(&ctx(), None, Some("northstar"));
        assert!(html.contains("Open your signed change log"));
        assert!(html.contains("Records are verified before they are displayed."));
        assert!(html.contains("<form method=\"post\" action=\"/console/ledger\">"));
        assert!(!html.contains("method=\"get\""));
        assert!(html.contains("value=\"northstar\""));
        assert!(html.contains("Sent in the request body, never in the URL."));
        assert!(html.contains("type=\"password\""));
        assert!(!html.contains("token="), "no token in any URL");
        assert!(!html.contains("<script"));
    }

    #[test]
    fn access_problems_render_the_frozen_copy_and_clear_secrets() {
        let invalid = render_access_panel(
            &ctx(),
            Some(LedgerAccessProblem::InvalidAccess),
            Some("northstar"),
        );
        assert_eq!(LedgerAccessProblem::InvalidAccess.status(), 401);
        assert!(invalid.contains("We could not open this publisher&#39;s history."));
        assert!(invalid.contains("Check the access details or contact support."));
        assert!(invalid.contains("Contact support"));
        // The safe publisher value is preserved; the secret is never echoed.
        assert!(invalid.contains("value=\"northstar\""));

        let expired = render_access_panel(
            &ctx(),
            Some(LedgerAccessProblem::SessionExpired),
            Some("northstar"),
        );
        assert!(expired.contains("Your access session has ended."));
        assert!(expired.contains("Enter the access details again to continue."));
        assert_eq!(LedgerAccessProblem::SessionExpired.status(), 400);

        let bad = render_access_panel(
            &ctx(),
            Some(LedgerAccessProblem::InvalidPublisher),
            Some("a/b"),
        );
        assert_eq!(LedgerAccessProblem::InvalidPublisher.status(), 400);
        assert!(bad.contains("Check the request."));
        assert!(bad.contains("Enter a valid publisher ID."));
        assert!(bad.contains("aria-invalid=\"true\""));
    }

    #[test]
    fn hostile_publisher_is_escaped_in_the_panel() {
        let hostile = "\" onmouseover=\"alert(1)";
        let html = render_access_panel(&ctx(), None, Some(hostile));
        assert!(!html.contains("onmouseover=\"alert(1)\""));
        assert!(html.contains("value=\"&quot; onmouseover=&quot;alert(1)\""));
    }

    #[test]
    fn records_page_lists_human_labels_dates_and_links() {
        let records = seal_all(vec![
            change(1_791_967_200_000, "rw_de_01", "DE", 4_000_000),
            started(1_790_842_200_000),
            observed(1_791_967_140_000, Some(300)),
        ]);
        let html = render_records(
            &ctx(),
            "northstar",
            RecordsPage {
                records: &records,
                total: records.len() as u64,
                page: 1,
            },
            1_792_055_400_000,
            "csrf-token-1",
        );
        assert!(html.contains("<h1>Signed change log</h1>"));
        assert!(html.contains(">Verified<"));
        assert!(html.contains("3 records / Verified 15 Oct 2026, 09:10 UTC / Publisher northstar"));
        for header in ["When (UTC)", "Event", "Scope", "Result"] {
            assert!(html.contains(&format!("<th scope=\"col\">{header}</th>")));
        }
        assert!(html.contains(">Change applied<"));
        assert!(html.contains(">Experiment started<"));
        assert!(html.contains(">Experiment observed<"));
        assert!(html.contains("/console/ledger/records/0"));
        assert!(html.contains("/console/ledger/records/2"));
        assert!(html.contains("14 Oct 2026, 08:40 UTC"));
        assert!(html.contains("Set the DE country floor to $4.00 eCPM on ad unit rw_de_01"));
        assert!(html.contains("25% test allocation"));
        assert!(html.contains("observed lift +3.0%"));
        assert!(html.contains("csrf-token-1"));
        assert!(html.contains("action=\"/console/ledger/session/end\""));
        assert!(html.contains("Showing 1-3 of 3 records"));
        assert!(!html.contains("rel=\"prev\""));
        assert!(!html.contains("rel=\"next\""));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn records_page_paginates_with_stable_links_and_a_remainder() {
        let records = seal_all(
            (0..250)
                .map(|index| change(1_791_967_200_000 + index, "rw_de_01", "DE", 4_000_000))
                .collect(),
        );
        let first = render_records(
            &ctx(),
            "northstar",
            RecordsPage {
                records: &records[0..RECORDS_PER_PAGE],
                total: records.len() as u64,
                page: 1,
            },
            1_792_055_400_000,
            "c",
        );
        assert!(first.contains("250 records / Verified"));
        assert!(first.contains("Showing 1-100 of 250 records"));
        assert!(first.contains("rel=\"next\" href=\"/console/ledger?page=2\""));
        assert!(!first.contains("rel=\"prev\""));

        let middle = render_records(
            &ctx(),
            "northstar",
            RecordsPage {
                records: &records[100..200],
                total: records.len() as u64,
                page: 2,
            },
            1_792_055_400_000,
            "c",
        );
        assert!(middle.contains("Showing 101-200 of 250 records"));
        assert!(middle.contains("rel=\"prev\" href=\"/console/ledger?page=1\""));
        assert!(middle.contains("rel=\"next\" href=\"/console/ledger?page=3\""));

        let last = render_records(
            &ctx(),
            "northstar",
            RecordsPage {
                records: &records[200..],
                total: records.len() as u64,
                page: 3,
            },
            1_792_055_400_000,
            "c",
        );
        assert!(last.contains("Showing 201-250 of 250 records"));
        assert!(last.contains("rel=\"prev\" href=\"/console/ledger?page=2\""));
        assert!(!last.contains("rel=\"next\""));
        assert!(last.contains("/console/ledger/records/249"));
    }

    #[test]
    fn records_page_singular_and_empty_states() {
        let one = seal_all(vec![change(1_791_967_200_000, "rw_de_01", "DE", 4_000_000)]);
        let html = render_records(
            &ctx(),
            "northstar",
            RecordsPage {
                records: &one,
                total: 1,
                page: 1,
            },
            1_792_055_400_000,
            "c",
        );
        assert!(html.contains("1 record / Verified"));
        assert!(html.contains("Showing 1-1 of 1 records"));
        let empty = render_records(
            &ctx(),
            "northstar",
            RecordsPage {
                records: &[],
                total: 0,
                page: 1,
            },
            1_792_055_400_000,
            "c",
        );
        assert!(empty.contains(
            "No changes have been recorded. Your verified history is empty for this publisher."
        ));
        assert!(empty.contains("href=\"/console/audit\""));
        assert!(!empty.contains("class=\"pagination\""));
    }

    #[test]
    fn summaries_cover_every_variant() {
        let currency = "USD";
        let applied = change(0, "rw_de_01", "DE", 4_000_000);
        assert!(
            record_summary(&applied, currency).contains("Set the DE country floor to $4.00 eCPM")
        );
        assert!(record_summary(&applied, currency).contains("previous value 1000000"));
        assert!(record_summary(&started(0), currency).contains("Started experiment exp_0247"));
        assert!(record_summary(&observed(0, Some(300)), currency).contains("+3.0%"));
        assert!(record_summary(&observed(0, Some(-120)), currency).contains("-1.2%"));
        assert!(record_summary(&observed(0, None), currency)
            .contains("Not available (no lift estimate)"));

        let rollback = LedgerRecord::rollback_performed(
            0,
            PlatformId::AppLovinMax,
            "rw_de_01",
            0,
            "regression on EU traffic",
        );
        assert!(record_summary(&rollback, currency)
            .contains("Rolled back change 0 on ad unit rw_de_01: regression on EU traffic"));

        let proof = apex_proof::build_billing_proof(
            "northstar",
            "exp_0247",
            10_000_000_000,
            10_500_000_000,
            2_000,
            0,
        )
        .expect("billing proof");
        let summary = record_summary(&proof, currency);
        assert!(summary.contains("Billing proof for exp_0247"));
        assert!(summary.contains("rate 20.0%"));
        assert!(summary.contains("USD 100.00"), "{summary}");

        let lift = LedgerRecord::lift_record(0, &"ab".repeat(32), "{\"lift\":true}");
        assert!(record_summary(&lift, currency).contains("payload digest abababababab"));
    }

    #[test]
    fn record_detail_shows_fields_verification_and_raw_json() {
        let records = seal_all(vec![change(1_791_967_200_000, "rw_de_01", "DE", 4_000_000)]);
        let html = render_record_detail(&ctx(), "northstar", &records[0], 24, "csrf-1");
        assert!(html.contains("<h1>Change applied / rw_de_01</h1>"));
        assert!(html.contains(">Verified record<"));
        assert!(
            html.contains("Record 0 of 24 / Recorded 14 Oct 2026, 08:40 UTC / Publisher northstar")
        );
        assert!(html.contains("Record summary"));
        assert!(html.contains("Field") && html.contains("Value"));
        assert!(html.contains("Platform"));
        assert!(html.contains("applovin_max"));
        assert!(html.contains("Before"));
        assert!(html.contains("Idempotency key"));
        assert!(html.contains("max-floor:rw_de_01:DE"));
        assert!(html.contains("<details class=\"details\">"));
        assert!(html.contains("Previous hash: <code>"));
        assert!(html.contains("Digest: <code>"));
        assert!(html.contains("verified as part of the whole-file verification"));
        assert!(html.contains("&quot;type&quot;:&quot;changeApplied&quot;"));
        assert!(html.contains("csrf-1"));
        assert!(html.contains("Back to the change log"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn record_detail_renders_experiment_and_lift_identifiers() {
        let records = seal_all(vec![started(1_790_842_200_000)]);
        let html = render_record_detail(&ctx(), "northstar", &records[0], 24, "c");
        assert!(html.contains("<h1>Experiment started / exp_0247</h1>"));
        assert!(html.contains("Metric"));
        assert!(html.contains("eCPM"));
        assert!(html.contains("25% test allocation"));

        let lift = LedgerRecord::lift_record(0, &"ab".repeat(32), "{\"lift\":true}");
        let lift = seal_all(vec![lift]);
        let html = render_record_detail(&ctx(), "northstar", &lift[0], 1, "c");
        assert!(html.contains("<h1>Lift record / payload abababababab</h1>"));
        assert!(html.contains("Payload SHA-256"));
    }

    #[test]
    fn labels_and_scopes_are_pinned() {
        let records = [
            change(0, "rw_de_01", "DE", 4_000_000),
            started(0),
            observed(0, Some(100)),
        ];
        assert_eq!(human_label(&records[0]), "Change applied");
        assert_eq!(human_label(&records[1]), "Experiment started");
        assert_eq!(human_label(&records[2]), "Experiment observed");
        assert_eq!(record_scope(&records[0]), "rw_de_01");
        assert_eq!(record_scope(&records[1]), "exp_0247");
    }
}
