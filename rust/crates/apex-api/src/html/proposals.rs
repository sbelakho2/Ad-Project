//! `/console/proposals/{id}`: the review page, frozen copy.
//!
//! Current versus test settings come from the proposal file's embedded
//! runner `Proposal` (the exact object, with the prior value the runner read
//! before proposing). Approval requires a console session (CSRF) and the
//! derived idempotency key; a retry returns the original operation status.

use apex_autopilot::model::Action;
use apex_ui::components::{
    action, action_button, csrf_hidden, data_table, notice, panel, Cell, EmptyState,
};
use apex_ui::format::{ecpm, or_not_available, percent, utc_datetime};
use apex_ui::status::StatusKind;

use crate::proposals::{ApprovalDecision, ApprovalRecord, ConsoleProposal};

use super::{console_document, html_escape, ConsoleCtx};

/// Frozen headline.
pub const HEADLINE: &str = "Review a proposed test";
/// Frozen standing note under the settings table.
pub const SCOPE_NOTE: &str = "Stop conditions and fee rate are recorded before the first write. \
                             Other settings stay outside this proposal.";
/// Frozen authority note.
pub const AUTHORITY_NOTE: &str = "\"Approve\" never means unlimited write authority.";
/// Frozen decision-chip label.
pub const REQUIRED_LABEL: &str = "Approval required";
/// The stale-settings warning (the frozen "stale settings invalidate it").
pub const STALE_NOTE: &str = "The settings shown must still match the platform. Stale settings \
                              invalidate it.";
/// Decline explanation (frozen financial language: Decline means no change).
pub const DECLINE_NOTE: &str = "Decline means no change: nothing is written and nothing is billed.";
/// What the approval record is for.
pub const APPROVAL_RECORD_NOTE: &str = "The decision is recorded next to the proposal for the \
                                        runner to consume; until the runner wires that consumer, \
                                        nothing is written to the platform.";

/// The page data.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalPageData<'a> {
    /// Workspace publisher.
    pub publisher: &'a str,
    /// The proposal id (the file stem).
    pub id: &'a str,
    /// The proposal.
    pub proposal: &'a ConsoleProposal,
    /// The recorded decision, when one exists.
    pub approval: Option<&'a ApprovalRecord>,
    /// Whether the recorded approval window has closed.
    pub expired: bool,
    /// The CSRF token when a console session exists.
    pub csrf: Option<&'a str>,
    /// A refusal to show (conflict, bad request) instead of a fresh form.
    pub refusal: Option<&'a str>,
}

/// Render the proposal review page.
pub fn render_detail(ctx: &ConsoleCtx, data: &ProposalPageData<'_>) -> String {
    let proposal = data.proposal;
    let (object, current, test) = setting_row(proposal, &ctx.currency);
    let mut body = format!("<h1>{HEADLINE}</h1>");
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "Proposal {} / {}",
            data.id,
            proposal.proposal.action.ad_unit_id()
        ))
    ));
    let (chip, chip_kind) = match data.approval {
        Some(record) => (record.decision.label().to_string(), StatusKind::Verified),
        None if data.expired => ("Expired".to_string(), StatusKind::Failure),
        None => (REQUIRED_LABEL.to_string(), StatusKind::Attention),
    };
    body.push_str(&notice(chip_kind, &chip, AUTHORITY_NOTE, None));

    let rows = vec![
        vec![
            Cell::Text("Country floor".to_string()),
            Cell::Text(current),
            Cell::Text(test),
        ],
        vec![
            Cell::Text("Allocation".to_string()),
            Cell::Text("Control retained".to_string()),
            Cell::Text(allocation_label(proposal.test_share)),
        ],
        vec![
            Cell::Text("Analysis window".to_string()),
            Cell::Text(format!("{}-day burn-in", proposal.burn_in_days)),
            Cell::Text(format!("{} final days", proposal.window_days)),
        ],
        vec![
            Cell::Text("Expected effect".to_string()),
            Cell::Text("Not applicable".to_string()),
            Cell::Text(format!(
                "+{} (expected)",
                percent(proposal.proposal.expected_effect_bp as f64 / 10_000.0)
            )),
        ],
        vec![
            Cell::Mono(object),
            Cell::Text("Recorded before proposing".to_string()),
            Cell::Text("Proposed".to_string()),
        ],
    ];
    body.push_str(&data_table(
        Some("Current versus test settings"),
        &["Setting", "Current", "Test configuration"],
        &rows,
        None,
        None,
    ));
    body.push_str(&panel(
        "Scope",
        &format!(
            "<p>{}</p><p>{}</p><ul><li>Reason recorded: {}</li><li>Snapshot: {}</li>\
             <li>Policy version: {}</li><li>Change hash: <code>{}</code></li>\
             <li>Fee rate: {}</li></ul>",
            html_escape(SCOPE_NOTE),
            html_escape(STALE_NOTE),
            html_escape(&proposal.proposal.rationale),
            html_escape(&proposal.snapshot),
            html_escape(&proposal.policy_version),
            html_escape(&short_hash(&proposal.change_hash())),
            html_escape(&percent(f64::from(proposal.rate_bp) / 10_000.0))
        ),
        None,
    ));
    body.push_str(&stop_conditions(proposal));
    body.push_str(&decision_area(data));
    console_document(ctx, &format!("Proposal {}", data.id), "Proposals", &body)
}

/// The current versus test values for the proposed action.
pub fn setting_row(proposal: &ConsoleProposal, currency: &str) -> (String, String, String) {
    let prior = serde_json::from_str::<serde_json::Value>(&proposal.proposal.prev_value).ok();
    let prior_micros = |key: &str| {
        prior
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_i64)
    };
    match &proposal.proposal.action {
        Action::SetCountryFloor {
            ad_unit_id,
            country,
            floor_ecpm,
        } => (
            format!("Country floor ({ad_unit_id} / {country})"),
            match prior_micros(country) {
                Some(old) => format!("{} eCPM", ecpm(old, currency)),
                None => or_raw(&proposal.proposal.prev_value),
            },
            format!("{} eCPM", ecpm(*floor_ecpm, currency)),
        ),
        Action::SetMappingEcpm {
            mediation_group_id,
            ad_unit_id,
            ecpm: value,
        } => (
            format!("Mapping eCPM ({mediation_group_id} / {ad_unit_id})"),
            match prior_micros(ad_unit_id) {
                Some(old) => format!("{} eCPM", ecpm(old, currency)),
                None => or_raw(&proposal.proposal.prev_value),
            },
            format!("{} eCPM", ecpm(*value, currency)),
        ),
        Action::SetWaterfallPrice {
            ad_unit_id,
            entry_id,
            price_ecpm,
        } => (
            format!("Waterfall price ({ad_unit_id} / {entry_id})"),
            match prior_micros(entry_id) {
                Some(old) => format!("{} eCPM", ecpm(old, currency)),
                None => or_raw(&proposal.proposal.prev_value),
            },
            format!("{} eCPM", ecpm(*price_ecpm, currency)),
        ),
        Action::CreateWaterfallEntry {
            ad_unit_id,
            network,
            instance_name,
            price_ecpm,
        } => (
            format!("Waterfall entry ({ad_unit_id} / {instance_name})"),
            "No entry is recorded".to_string(),
            format!("{} on {network}", ecpm(*price_ecpm, currency)),
        ),
        Action::RemoveWaterfallEntry {
            ad_unit_id,
            entry_id,
        } => (
            format!("Waterfall entry ({ad_unit_id} / {entry_id})"),
            or_raw(&proposal.proposal.prev_value),
            "Removed".to_string(),
        ),
        Action::SetLevelplayInstancePrice {
            instance_id,
            price_micros,
        } => (
            format!("LevelPlay instance price ({instance_id})"),
            match prior_micros(instance_id) {
                Some(old) => format!("{} eCPM", ecpm(old, currency)),
                None => or_raw(&proposal.proposal.prev_value),
            },
            format!("{} eCPM", ecpm(*price_micros, currency)),
        ),
        Action::SetAdmobMediationFloor {
            group_id,
            floor_micros,
        } => (
            format!("AdMob mediation floor ({group_id})"),
            match prior_micros(group_id) {
                Some(old) => format!("{} eCPM", ecpm(old, currency)),
                None => or_raw(&proposal.proposal.prev_value),
            },
            format!("{} eCPM", ecpm(*floor_micros, currency)),
        ),
    }
}

fn or_raw(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        or_not_available("the runner recorded no prior value")
    } else {
        trimmed.chars().take(120).collect()
    }
}

fn allocation_label(share: f64) -> String {
    if (share - 0.10).abs() < 1e-9 {
        "Exploratory 10% split".to_string()
    } else {
        format!("{}% test", (share * 100.0).round() as i64)
    }
}

fn stop_conditions(proposal: &ConsoleProposal) -> String {
    let thresholds = &proposal.thresholds;
    let rows = vec![
        vec![
            Cell::Text("Fast harm stop".to_string()),
            Cell::Text(percent(thresholds.harm_point)),
            Cell::Text("Point-lift stop threshold".to_string()),
        ],
        vec![
            Cell::Text("Rewarded fill drop".to_string()),
            Cell::Text(percent(thresholds.fill_rewarded)),
            Cell::Text("Guardrail threshold".to_string()),
        ],
        vec![
            Cell::Text("Banner fill drop".to_string()),
            Cell::Text(percent(thresholds.fill_banner)),
            Cell::Text("Guardrail threshold".to_string()),
        ],
        vec![
            Cell::Text("Split integrity".to_string()),
            Cell::Text(format!("chi-square {}", thresholds.split_chi_square)),
            Cell::Text("Traffic-split guard".to_string()),
        ],
    ];
    data_table(
        Some("Stop conditions recorded before the first write"),
        &["Condition", "Threshold", "Note"],
        &rows,
        None,
        None,
    )
}

fn decision_area(data: &ProposalPageData<'_>) -> String {
    let proposal = data.proposal;
    if let Some(record) = data.approval {
        let body = format!(
            "<p>{} decided {}. Recorded actor: {}.</p>\
             <ul><li>Change hash bound: <code>{}</code></li>\
             <li>Snapshot bound: {}</li><li>Policy version bound: {}</li>\
             <li>Idempotency key: <code>{}</code></li></ul>\
             <p>{}</p>",
            html_escape(record.decision.label()),
            html_escape(&utc_datetime(record.decided_at_unix_ms)),
            html_escape(&record.approval_actor),
            html_escape(&short_hash(&record.change_hash)),
            html_escape(&record.snapshot),
            html_escape(&record.policy_version),
            html_escape(record.idempotency_key.as_str()),
            html_escape(STALE_NOTE)
        );
        return panel("Already decided", &body, None);
    }
    if data.expired {
        return panel(
            "Expired",
            &format!(
                "<p>This proposal passed its recorded approval window before a decision. \
                 Nothing is written for an expired proposal.</p><p>{}</p>",
                html_escape(STALE_NOTE)
            ),
            None,
        );
    }
    let mut body = String::new();
    if let Some(refusal) = data.refusal {
        body.push_str(&notice(StatusKind::Failure, refusal, "", None));
    }
    match data.csrf {
        Some(csrf) => {
            let approve_key = crate::proposals::idempotency_key(
                data.id,
                ApprovalDecision::Approved,
                &proposal.change_hash(),
            );
            let decline_key = crate::proposals::idempotency_key(
                data.id,
                ApprovalDecision::Declined,
                &proposal.change_hash(),
            );
            // One form per decision: each decision has its own idempotency
            // key, so the button a person presses always submits the key the
            // handler expects.
            body.push_str(&format!(
                "<form method=\"post\" action=\"/console/proposals/{}/decision\">\
                 {}\
                 <input type=\"hidden\" name=\"idempotency_key\" value=\"{}\">\
                 {}</form>\
                 <form method=\"post\" action=\"/console/proposals/{}/decision\">\
                 {}\
                 <input type=\"hidden\" name=\"idempotency_key\" value=\"{}\">\
                 {}</form>",
                html_escape(data.id),
                csrf_hidden(csrf),
                html_escape(&approve_key),
                action_button("Approve this test", "decision", "approve", true),
                html_escape(data.id),
                csrf_hidden(csrf),
                html_escape(&decline_key),
                action_button("Decline", "decision", "decline", false),
            ));
        }
        None => {
            body.push_str(&format!(
                "<p>{}</p>",
                html_escape(
                    "Open the change log with your access token to start a console session; \
                     decisions are submitted with that session's verification token."
                )
            ));
            body.push_str(&action("Open the change log", "/console/ledger", false));
        }
    }
    body.push_str(&format!("<p>{}</p>", html_escape(APPROVAL_RECORD_NOTE)));
    body.push_str(&format!("<p>{}</p>", html_escape(DECLINE_NOTE)));
    panel(REQUIRED_LABEL, &body, None)
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(16).collect()
}

/// The proposal index headline (the rail label and breadcrumb area).
pub const INDEX_HEADLINE: &str = "Proposals";
/// The standing note under the index table.
pub const INDEX_NOTE: &str =
    "Approvals are bound to the proposal's snapshot, scope and change hash.";

/// One row of the proposal index.
#[derive(Debug, Clone)]
pub struct IndexEntry<'a> {
    /// The proposal id (the file stem).
    pub id: &'a str,
    /// The proposal.
    pub proposal: &'a ConsoleProposal,
    /// The recorded decision, when one exists.
    pub approval: Option<&'a ApprovalRecord>,
    /// Whether the recorded approval window has closed.
    pub expired: bool,
}

/// Render the proposal index: every proposed change with its decision state.
pub fn render_index(ctx: &ConsoleCtx, entries: &[IndexEntry<'_>], skipped: usize) -> String {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    for entry in entries {
        let (chip, kind) = match entry.approval {
            Some(record) => (record.decision.label().to_string(), StatusKind::Verified),
            None if entry.expired => ("Expired".to_string(), StatusKind::Failure),
            None => (REQUIRED_LABEL.to_string(), StatusKind::Attention),
        };
        rows.push(vec![
            Cell::Mono(entry.id.to_string()),
            Cell::Text(setting_row(entry.proposal, &ctx.currency).0),
            Cell::Text(utc_datetime(entry.proposal.created_at_unix_ms)),
            Cell::Status(chip, kind),
            Cell::Link(
                "Review proposal".to_string(),
                format!("/console/proposals/{}", entry.id),
            ),
        ]);
    }
    let mut count_note = match rows.len() {
        0 => "No proposals".to_string(),
        1 => "1 proposal".to_string(),
        n => format!("{n} proposals"),
    };
    if skipped > 0 {
        count_note.push_str(&format!(
            " / the newest 200 are shown; {skipped} older or unreadable file(s) are not listed"
        ));
    }
    let mut body = format!("<h1>{}</h1>", html_escape(INDEX_HEADLINE));
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(INDEX_NOTE)
    ));
    body.push_str(&data_table(
        None,
        &["Proposal", "Setting", "Created", "State", "Action"],
        &rows,
        Some(EmptyState {
            reason: "No proposal is open.".to_string(),
            action: Some((
                "Open the experiment list".to_string(),
                "/console/experiments".to_string(),
            )),
        }),
        Some(&count_note),
    ));
    console_document(ctx, "Proposals", "Proposals", &body)
}

/// The proposal-unknown page.
pub fn render_not_found(ctx: &ConsoleCtx) -> String {
    let body = format!(
        "<h1>Proposal not found</h1>{}",
        notice(
            StatusKind::Failure,
            "No proposal matches this address.",
            "The proposal is not available in this workspace. Check the link from the change log \
             or the experiment overview.",
            Some(("Open experiments", "/console/experiments")),
        )
    );
    console_document(ctx, "Proposal not found", "Proposals", &body)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_lift::decision::GuardrailThresholds;

    use super::*;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::new("Northstar Studio", "USD", true)
    }

    fn proposal() -> ConsoleProposal {
        ConsoleProposal {
            publisher: Some("northstar".to_string()),
            proposal: apex_autopilot::model::Proposal {
                action: Action::SetCountryFloor {
                    ad_unit_id: "rw_de_01".to_string(),
                    country: "DE".to_string(),
                    floor_ecpm: 3_200_000,
                },
                rationale: "Raise the DE rewarded floor by 8%".to_string(),
                expected_effect_bp: 260,
                prev_value: "{\"DE\":4000000}".to_string(),
            },
            snapshot: "14 Oct 2026, 08:10 UTC".to_string(),
            created_at_unix_ms: 1_792_000_000_000,
            policy_version: "apex-autopilot-0.1.0".to_string(),
            expires_at_unix_ms: Some(1_794_592_000_000),
            test_share: 0.25,
            burn_in_days: 3,
            window_days: 14,
            rate_bp: 2_000,
            thresholds: GuardrailThresholds::default(),
        }
    }

    fn page<'a>(
        proposal: &'a ConsoleProposal,
        approval: Option<&'a ApprovalRecord>,
        csrf: Option<&'a str>,
    ) -> ProposalPageData<'a> {
        ProposalPageData {
            publisher: "northstar",
            id: "p_0247",
            proposal,
            approval,
            expired: false,
            csrf,
            refusal: None,
        }
    }

    #[test]
    fn the_page_renders_the_frozen_copy_and_the_exact_object() {
        let proposal = proposal();
        let html = render_detail(&ctx(), &page(&proposal, None, Some("tok")));
        assert!(html.contains("<h1>Review a proposed test</h1>"));
        assert!(html.contains("Approval required"));
        assert!(html.contains("never means unlimited write authority."));
        assert!(html.contains("Country floor (rw_de_01 / DE)"));
        assert!(html.contains("$4.00 eCPM"));
        assert!(html.contains("$3.20 eCPM"));
        assert!(html.contains("25% test"));
        assert!(html.contains("3-day burn-in"));
        assert!(html.contains("14 final days"));
        assert!(html.contains("Stop conditions and fee rate are recorded before the first write."));
        assert!(html.contains("Approve this test"));
        assert!(html.contains(">Decline<"));
        assert!(html.contains("name=\"idempotency_key\""));
        assert!(html.contains("Stale settings invalidate it."));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn decided_proposals_show_the_binding_and_hide_the_form() {
        let proposal = proposal();
        let record = ApprovalRecord {
            proposal_id: "p_0247".to_string(),
            decision: ApprovalDecision::Approved,
            decided_at_unix_ms: 1_792_055_400_000,
            idempotency_key: crate::proposals::idempotency_key(
                "p_0247",
                ApprovalDecision::Approved,
                &proposal.change_hash(),
            ),
            change_hash: proposal.change_hash(),
            snapshot: proposal.snapshot.clone(),
            policy_version: proposal.policy_version.clone(),
            approval_actor: crate::proposals::APPROVAL_ACTOR.to_string(),
        };
        let html = render_detail(&ctx(), &page(&proposal, Some(&record), Some("tok")));
        assert!(html.contains("Already decided"));
        assert!(html.contains(">Approved"));
        assert!(html.contains("console-workspace"));
        assert!(!html.contains("<form"));
        assert!(!html.contains("Approve this test"));
    }

    #[test]
    fn expired_and_refused_proposals_show_no_decision_form() {
        let proposal = proposal();
        let mut data = page(&proposal, None, Some("tok"));
        data.expired = true;
        let html = render_detail(&ctx(), &data);
        assert!(html.contains(">Expired<"));
        assert!(html.contains("Nothing is written for an expired proposal."));
        assert!(!html.contains("<form"));

        let mut refused = page(&proposal, None, Some("tok"));
        refused.refusal = Some("This proposal was already decided; the recorded decision stands.");
        let html = render_detail(&ctx(), &refused);
        assert!(html.contains("the recorded decision stands"));
    }

    #[test]
    fn without_a_session_the_page_offers_the_access_panel() {
        let proposal = proposal();
        let html = render_detail(&ctx(), &page(&proposal, None, None));
        assert!(!html.contains("<form"));
        assert!(html.contains("Open the change log"));
        assert!(html.contains("href=\"/console/ledger\""));
    }

    #[test]
    fn unknown_proposals_render_the_not_found_page() {
        let html = render_not_found(&ctx());
        assert!(html.contains("<h1>Proposal not found</h1>"));
        assert!(html.contains("No proposal matches this address."));
    }

    #[test]
    fn hostile_values_are_escaped() {
        let mut proposal = proposal();
        proposal.proposal.rationale = "<script>alert(1)</script>".to_string();
        proposal.proposal.prev_value = "<script>alert(2)</script>".to_string();
        let html = render_detail(&ctx(), &page(&proposal, None, Some("tok")));
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn levelplay_and_admob_actions_render_their_ids_and_micros() {
        let mut proposal = proposal();

        proposal.proposal.action = Action::SetLevelplayInstancePrice {
            instance_id: "inst_lp_1".to_string(),
            price_micros: 1_250_000,
        };
        proposal.proposal.prev_value = "{\"inst_lp_1\":1000000}".to_string();
        let (object, current, test) = setting_row(&proposal, "USD");
        assert_eq!(object, "LevelPlay instance price (inst_lp_1)");
        assert_eq!(current, "$1.00 eCPM");
        assert_eq!(test, "$1.25 eCPM");

        proposal.proposal.action = Action::SetAdmobMediationFloor {
            group_id: "g_admob_1".to_string(),
            floor_micros: 2_500_000,
        };
        proposal.proposal.prev_value = "{\"g_admob_1\":2000000}".to_string();
        let (object, current, test) = setting_row(&proposal, "USD");
        assert_eq!(object, "AdMob mediation floor (g_admob_1)");
        assert_eq!(current, "$2.00 eCPM");
        assert_eq!(test, "$2.50 eCPM");
    }

    #[test]
    fn every_action_kind_renders_current_and_test() {
        let cases: Vec<Action> = vec![
            Action::SetMappingEcpm {
                mediation_group_id: "g1".to_string(),
                ad_unit_id: "u1".to_string(),
                ecpm: 1_000_000,
            },
            Action::SetWaterfallPrice {
                ad_unit_id: "u1".to_string(),
                entry_id: "e1".to_string(),
                price_ecpm: 2_000_000,
            },
            Action::CreateWaterfallEntry {
                ad_unit_id: "u1".to_string(),
                network: "n1".to_string(),
                instance_name: "i1".to_string(),
                price_ecpm: 3_000_000,
            },
            Action::RemoveWaterfallEntry {
                ad_unit_id: "u1".to_string(),
                entry_id: "e1".to_string(),
            },
            Action::SetLevelplayInstancePrice {
                instance_id: "inst_lp_1".to_string(),
                price_micros: 1_250_000,
            },
            Action::SetAdmobMediationFloor {
                group_id: "g_admob_1".to_string(),
                floor_micros: 2_500_000,
            },
        ];
        for action in cases {
            let mut proposal = proposal();
            proposal.proposal.action = action;
            let (object, current, test) = setting_row(&proposal, "USD");
            assert!(!object.is_empty());
            assert!(!current.is_empty());
            assert!(!test.is_empty());
        }
    }
}
