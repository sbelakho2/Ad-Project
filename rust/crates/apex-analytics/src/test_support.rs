//! Shared test fixtures: one valid payload and event per catalog entry, plus
//! the acceptance known-value rows ($51,500 / 3% / $300).
//!
//! This module is test-only (`cfg(test)`); production code never sees it.

use crate::envelope::{DomainEvent, DomainEventBuilder, EventPayload, EventSource, EventType};
use crate::store::ReportDayRow;
use crate::timestamp::UtcMillis;

/// The fixed occurrence time every fixture uses.
pub(crate) fn occurred() -> UtcMillis {
    UtcMillis::parse_rfc3339("2026-10-15T09:10:00.000Z").expect("fixture time")
}

/// One valid payload per catalog entry.
pub(crate) fn payload_for(event_type: EventType) -> EventPayload {
    let mut p = EventPayload::new();
    match event_type {
        EventType::PublicPageRequested => {
            p.route_template = Some("/pricing".into());
            p.status = Some(200);
            p.bot_class = Some("unclassified".into());
            p.referrer_host = Some("example.com".into());
            p.campaign_code = Some("launch".into());
            p.query_keys = Some(vec!["utm_source".into(), "utm_campaign".into()]);
        }
        EventType::AuditRequestAccepted => {
            p.lead_id = Some("lead_01".into());
            p.platforms = Some(vec!["applovin_max".into()]);
            p.range_start = Some("2026-09-15".into());
            p.range_end = Some("2026-10-14".into());
            p.source_code = Some("seo".into());
        }
        EventType::AuditRequestRejected => {
            p.lead_id = Some("lead_02".into());
            p.source_code = Some("seo".into());
            p.validation_reason = Some("invalid contact".into());
        }
        EventType::LeadForwardAttempted => {
            p.lead_id = Some("lead_01".into());
            p.destination_type = Some("webhook".into());
            p.attempt = Some(1);
        }
        EventType::LeadForwardAcknowledged | EventType::LeadForwardFailed => {
            p.lead_id = Some("lead_01".into());
            p.destination_type = Some("webhook".into());
            p.attempt = Some(2);
            p.response_class = Some("2xx".into());
        }
        EventType::AccountStageChanged => {
            p.previous_stage = Some("audit_requested".into());
            p.next_stage = Some("audit_delivered".into());
            p.owner = Some("maya".into());
            p.effective_at = Some("2026-10-15T09:00:00Z".into());
        }
        EventType::ConnectionValidated
        | EventType::ConnectionExpired
        | EventType::ConnectionRevoked => {
            p.platform_account_id = Some("max_acct_1".into());
            p.read_capability = Some(true);
            p.write_capability = Some(false);
            p.validator_version = Some("v3".into());
            if event_type != EventType::ConnectionValidated {
                p.failure_class = Some(
                    if event_type == EventType::ConnectionExpired {
                        "expired"
                    } else {
                        "revoked"
                    }
                    .into(),
                );
            }
        }
        EventType::ReportPullStarted => {
            p.job_id = Some("job_77".into());
            p.platform_operation = Some("pull_reports".into());
            p.range_start = Some("2026-10-01".into());
            p.range_end = Some("2026-10-14".into());
            p.attempt = Some(1);
        }
        EventType::ReportPullCompleted => {
            p.job_id = Some("job_77".into());
            p.platform_operation = Some("pull_reports".into());
            p.range_start = Some("2026-10-01".into());
            p.range_end = Some("2026-10-14".into());
            p.rows = Some(30);
            p.finality = Some("final".into());
            p.duration_ms = Some(1_200);
        }
        EventType::ReportPullFailed => {
            p.job_id = Some("job_78".into());
            p.platform_operation = Some("pull_reports".into());
            p.range_start = Some("2026-10-01".into());
            p.range_end = Some("2026-10-14".into());
            p.attempt = Some(3);
            p.error_class = Some("rate_limited".into());
        }
        EventType::AuditCompleted => {
            p.snapshot_digest = Some("a".repeat(64));
            p.rule_version = Some("v0.1.1".into());
            p.eligible_rule_groups = Some(4);
            p.expected_rule_groups = Some(5);
            p.findings_critical = Some(1);
            p.findings_warning = Some(3);
            p.findings_info = Some(1);
            p.missing_inputs = Some(vec!["fill counts".into()]);
            p.duration_ms = Some(800);
        }
        EventType::AuditFailed => {
            p.rule_version = Some("v0.1.1".into());
            p.error_class = Some("snapshot_unreadable".into());
        }
        EventType::ProposalCreated | EventType::ProposalDeclined | EventType::ProposalExpired => {
            p.proposal_id = Some("p_0247".into());
            p.snapshot = Some("snap_09".into());
            p.change_hash = Some("b".repeat(64));
            p.policy_version = Some("policy_3".into());
            if event_type == EventType::ProposalExpired {
                p.expiry = Some("2026-10-20T00:00:00Z".into());
            }
        }
        EventType::ProposalApproved => {
            p.proposal_id = Some("p_0247".into());
            p.snapshot = Some("snap_09".into());
            p.change_hash = Some("b".repeat(64));
            p.policy_version = Some("policy_3".into());
            p.approval_actor = Some("maya".into());
        }
        EventType::ExperimentRegistered => {
            p.experiment_id = Some("exp_0247".into());
            p.source_digest = Some("c".repeat(64));
            p.registered_metric = Some("revenue_per_user".into());
            p.window_days = Some(14);
        }
        EventType::ExperimentStarted => {
            p.experiment_id = Some("exp_0247".into());
            p.state = Some("running".into());
        }
        EventType::ExperimentPull => {
            p.experiment_id = Some("exp_0247".into());
            p.source_digest = Some("c".repeat(64));
            p.range_start = Some("2026-10-01".into());
            p.range_end = Some("2026-10-14".into());
        }
        EventType::ExperimentDecided => {
            p.experiment_id = Some("exp_0246".into());
            p.decision = Some("keep".into());
            p.point_estimate_bp = Some(540);
            p.decision_bound_bp = Some(300);
        }
        EventType::GuardrailFired => {
            p.guard_type = Some("fill_rewarded".into());
            p.observed_input = Some("-3.2%".into());
            p.threshold = Some("-2.0%".into());
        }
        EventType::GuardrailRestoreRequested => {
            p.guard_type = Some("fill_rewarded".into());
            p.operation_id = Some("op_31".into());
            p.target_state = Some("floor_restored".into());
        }
        EventType::GuardrailRestoreConfirmed => {
            p.guard_type = Some("fill_rewarded".into());
            p.operation_id = Some("op_31".into());
            p.target_state = Some("floor_restored".into());
            p.confirming_read = Some("platform read at 09:05".into());
        }
        EventType::GuardrailRestoreFailed => {
            p.guard_type = Some("fill_rewarded".into());
            p.operation_id = Some("op_31".into());
            p.target_state = Some("floor_restored".into());
            p.error_class = Some("platform_refused".into());
        }
        EventType::LedgerVerified => {
            p.highest_sequence = Some(24);
            p.verification_version = Some("v2".into());
            p.duration_ms = Some(150);
        }
        EventType::LedgerVerificationFailed => {
            p.highest_sequence = Some(24);
            p.verification_version = Some("v2".into());
            p.failure_line = Some(18);
            p.duration_ms = Some(150);
        }
        EventType::FeeComputed => {
            p.change_id = Some("ch_0246".into());
            p.period = Some("2026-10".into());
            p.currency = Some("USD".into());
            p.source_amount_micros = Some(51_500_000_000);
            p.bound_bp = Some(300);
            p.factor_bp = Some(10_300);
            p.rate_bp = Some(2_000);
            p.active_days = Some(31);
            p.proof_hashes = Some(vec!["d".repeat(64)]);
        }
        EventType::FeeHeld => {
            p.change_id = Some("ch_0246".into());
            p.period = Some("2026-10".into());
            p.currency = Some("USD".into());
        }
        EventType::FeeInvoiceIssued | EventType::FeeCredited => {
            p.change_id = Some("ch_0246".into());
            p.period = Some("2026-10".into());
            p.currency = Some("USD".into());
            p.invoice_id = Some("INV-2026-104".into());
            p.amount_micros = Some(300_000_000);
        }
        EventType::PaymentReceived => {
            p.psp_event_key = Some("psp_evt_88".into());
            p.invoice_id = Some("INV-2026-104".into());
            p.currency = Some("USD".into());
            p.amount_micros = Some(300_000_000);
        }
        EventType::PaymentReconciled | EventType::PaymentRefunded | EventType::PaymentDisputed => {
            p.invoice_id = Some("INV-2026-104".into());
            p.currency = Some("USD".into());
            p.amount_micros = Some(300_000_000);
        }
        EventType::JobRun => {
            p.job_id = Some("job_daily".into());
            p.duration_ms = Some(4_200);
        }
        EventType::IncidentChanged => {
            p.incident_id = Some("inc_018".into());
            p.owner = Some("maya".into());
            p.impact = Some("1 test state unresolved".into());
        }
        EventType::OperatorAction => {
            p.action_actor = Some("sam".into());
            p.target = Some("invoice INV-2026-104".into());
            p.authorization = Some("finance".into());
            p.result = Some("confirmed".into());
        }
    }
    p
}

/// A builder for a valid event of `event_type`, with every catalog minimum
/// satisfied and every envelope label the family requires set.
pub(crate) fn valid_event_builder(event_type: EventType) -> DomainEventBuilder {
    let source = match event_type.family() {
        crate::envelope::EventFamily::PageRequest => EventSource::AccessLog,
        crate::envelope::EventFamily::Fee | crate::envelope::EventFamily::Payment => {
            EventSource::Finance
        }
        crate::envelope::EventFamily::Operations => EventSource::Operator,
        _ => EventSource::Worker,
    };
    let mut builder = DomainEvent::builder(event_type, source)
        .with_occurred_at(occurred())
        .with_received_at(occurred())
        .with_environment("production")
        .with_service_version("apex-0.1.0")
        .with_payload(payload_for(event_type));
    if !matches!(
        event_type.family(),
        crate::envelope::EventFamily::PageRequest
            | crate::envelope::EventFamily::AuditRequest
            | crate::envelope::EventFamily::LeadForward
            | crate::envelope::EventFamily::Operations
    ) {
        builder = builder
            .with_publisher_id("northstar")
            .with_platform("applovin_max");
    }
    match event_type {
        EventType::AuditRequestRejected => {
            builder = builder
                .with_reason_code("validation")
                .with_outcome("rejected")
        }
        EventType::AccountStageChanged => builder = builder.with_reason_code("audit_delivered"),
        EventType::LeadForwardAcknowledged => builder = builder.with_outcome("acknowledged"),
        EventType::LeadForwardFailed => builder = builder.with_outcome("failed"),
        EventType::ReportPullFailed => builder = builder.with_outcome("failed"),
        EventType::AuditFailed => builder = builder.with_outcome("failed"),
        EventType::FeeHeld => builder = builder.with_reason_code("proof_hold"),
        EventType::GuardrailRestoreFailed => builder = builder.with_reason_code("platform_refused"),
        EventType::JobRun => builder = builder.with_outcome("completed"),
        EventType::OperatorAction => builder = builder.with_outcome("confirmed"),
        EventType::IncidentChanged => builder = builder.with_publisher_id("northstar"),
        _ => {}
    }
    builder
}

/// A built, valid event with a stable explicit identity.
pub(crate) fn valid_event(event_type: EventType) -> DomainEvent {
    valid_event_builder(event_type)
        .with_event_id(format!("evt_{}", event_type.as_str()))
        .build()
        .unwrap_or_else(|error| panic!("{event_type} fixture must build: {error}"))
}

/// The acceptance known-value source rows: US$ 51,500.00 of finalized
/// platform revenue for October 2026 (ten final days of 5,150.00 each),
/// under tenant `northstar`, platform `applovin_max`. Two countries split
/// the rows six to four so breakdowns have a known shape.
pub(crate) fn known_value_report_days() -> Vec<ReportDayRow> {
    (1..=10)
        .map(|day| ReportDayRow {
            publisher_id: "northstar".into(),
            platform: "applovin_max".into(),
            platform_account_id: "max_acct_1".into(),
            day: format!("2026-10-{day:02}"),
            finality: "final".into(),
            revenue_micros: Some(5_150_000_000),
            impressions: Some(2_500_000),
            currency: Some("USD".into()),
            country: Some(if day <= 6 { "DE" } else { "GB" }.into()),
            ad_unit_id: Some("rw_de_01".into()),
            format: Some("rewarded".into()),
            source_digest: Some(format!("{:064x}", day)),
        })
        .collect()
}

/// The acceptance known-value fee event: billable lower bound of 3 percent
/// on the 51,500.00 scope, agreed rate 20 percent, fee 300.00 before tax.
pub(crate) fn known_value_fee_event() -> DomainEvent {
    let mut payload = payload_for(EventType::FeeComputed);
    payload.change_id = Some("ch_0246".into());
    payload.period = Some("2026-10".into());
    payload.currency = Some("USD".into());
    payload.source_amount_micros = Some(51_500_000_000);
    payload.bound_bp = Some(300);
    payload.factor_bp = Some(10_300);
    payload.rate_bp = Some(2_000);
    payload.active_days = Some(31);
    payload.proof_hashes = Some(vec![format!("{:064x}", 246)]);
    valid_event_builder(EventType::FeeComputed)
        .with_event_id("fee_evt_known_value")
        .with_object("change", "ch_0246")
        .with_payload(payload)
        .build()
        .expect("known-value fee event")
}
