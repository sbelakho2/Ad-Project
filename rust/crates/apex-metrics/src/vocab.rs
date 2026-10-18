//! The declared bounded label vocabularies.
//!
//! Section 06 (pages 111 and 112) requires every metric label to be a bounded
//! dimension: "Do not generate a new metric name or label value for every
//! customer, experiment, URL, email or error message." Each enum below is one
//! declared vocabulary; [`crate::MetricSpec`] names the vocabulary a label
//! accepts, and [`crate::Registry`] refuses any value outside it.

/// Declare one bounded label vocabulary: an enum, its stable string values,
/// `as_str` and a strict `parse`.
macro_rules! bounded {
    ($(#[$meta:meta])* pub enum $name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Every value of this vocabulary, in declaration order.
            pub const VALUES: &'static [&'static str] = &[$($value),+];

            /// The stable label value.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            /// Parse a stored label value back into the enum.
            pub fn parse(text: &str) -> Option<Self> {
                $(
                    if text == $value {
                        return Some(Self::$variant);
                    }
                )+
                None
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

bounded! {
    /// The emitting service boundary.
    pub enum Service {
        Api => "apex-api",
        Control => "apex-control",
        Runner => "apex-runner",
    }
}

bounded! {
    /// HTTP methods carried by route-level metrics.
    pub enum Method {
        Get => "GET",
        Post => "POST",
        Put => "PUT",
        Patch => "PATCH",
        Delete => "DELETE",
        Head => "HEAD",
        Options => "OPTIONS",
    }
}

bounded! {
    /// Response status classes, including redirects.
    pub enum StatusClass {
        Informational => "1xx",
        Success => "2xx",
        Redirect => "3xx",
        ClientError => "4xx",
        ServerError => "5xx",
    }
}

impl StatusClass {
    /// The class of a numeric HTTP status.
    pub fn of(status: u16) -> Self {
        match status / 100 {
            1 => StatusClass::Informational,
            2 => StatusClass::Success,
            3 => StatusClass::Redirect,
            4 => StatusClass::ClientError,
            _ => StatusClass::ServerError,
        }
    }
}

bounded! {
    /// The platform a connector or job talks to.
    pub enum Platform {
        AppLovinMax => "applovin_max",
        LevelPlay => "levelplay",
        AdMob => "admob",
        Billing => "billing",
    }
}

bounded! {
    /// Connector operations: the platform-adapter call classes that may carry
    /// request/duration series. New adapter methods extend this release set;
    /// arbitrary strings are refused.
    pub enum ConnectorOperation {
        ListAdUnits => "listAdUnits",
        GetAdUnit => "getAdUnit",
        UpdateFloors => "updateFloors",
        CreateExperiment => "createExperiment",
        GetExperiment => "getExperiment",
        PromoteOrDeprecateExperiment => "promoteOrDeprecateExperiment",
        MaxReport => "maxReport",
        FetchAppAdsTxt => "fetchAppAdsTxt",
        Auth => "auth",
        CreatePaymentLink => "createPaymentLink",
        ListMediationGroups => "listMediationGroups",
        CreateMediationAbExperiment => "createMediationAbExperiment",
        StopMediationAbExperiment => "stopMediationAbExperiment",
        ListWaterfall => "listWaterfall",
        CreateEntry => "createEntry",
        UpdateEntryPrice => "updateEntryPrice",
        RemoveEntry => "removeEntry",
        ListGroups => "listGroups",
        ReportAbTestActiveUsers => "reportAbTestActiveUsers",
        CountryRevenue30d => "countryRevenue30d",
        AdUnitRevenue => "adUnitRevenue",
        ExperimentRows => "experimentRows",
    }
}

bounded! {
    /// Connector call outcomes (page 111): success, auth refusal, rate limit,
    /// transport failure or schema failure.
    pub enum OutcomeClass {
        Success => "success",
        AuthRefusal => "auth_refusal",
        RateLimit => "rate_limit",
        TransportFailure => "transport_failure",
        SchemaFailure => "schema_failure",
    }
}

bounded! {
    /// The scheduled job classes the runner executes.
    pub enum JobType {
        Audit => "audit",
        Cycle => "cycle",
        Daily => "daily",
        Monthly => "monthly",
        Recheck => "recheck",
        Aa => "aa",
        AaEval => "aa_eval",
        PaymentEvents => "payment_events",
    }
}

bounded! {
    /// Projections rebuilt and compared before readers switch.
    pub enum ProjectionName {
        CustomerDaily => "customer_daily",
        PlatformDaily => "platform_daily",
        LifecycleCohorts => "lifecycle_cohorts",
        OperatorQueues => "operator_queues",
    }
}

bounded! {
    /// Deployment environments.
    pub enum Environment {
        Production => "production",
        Staging => "staging",
        Dev => "dev",
    }
}

bounded! {
    /// Account report-readiness classes: Final, Partial, Stale or Disconnected.
    pub enum ReadinessClass {
        Final => "final",
        Partial => "partial",
        Stale => "stale",
        Disconnected => "disconnected",
    }
}

bounded! {
    /// Retained audit rule releases. The set is deliberately a release list,
    /// not a free version string.
    pub enum RuleVersion {
        V0_1_0 => "v0.1.0",
    }
}

bounded! {
    /// Experiment decision outcomes: Keep, Rollback, Void, Extend or Pending.
    pub enum Decision {
        Keep => "keep",
        Rollback => "rollback",
        Void => "void",
        Extend => "extend",
        Pending => "pending",
    }
}

bounded! {
    /// Bounded decision reason classes (the free reason text stays in events).
    pub enum ReasonClass {
        BoundPositive => "bound_positive",
        Harm => "harm",
        Fill => "fill",
        PublisherEdit => "publisher_edit",
        SplitMismatch => "split_mismatch",
        PlatformIncident => "platform_incident",
        ReportingGap => "reporting_gap",
        WindowIncomplete => "window_incomplete",
    }
}

bounded! {
    /// The registered guardrails that can fire.
    pub enum Guardrail {
        FastHarm => "fast_harm",
        ConfirmedHarm => "confirmed_harm",
        FillRewardedInterstitial => "fill_rewarded_interstitial",
        FillBanner => "fill_banner",
        PublisherEdit => "publisher_edit",
        SplitMismatch => "split_mismatch",
        ReportingGap => "reporting_gap",
        PlatformIncident => "platform_incident",
    }
}

bounded! {
    /// What a fired guardrail did to the test.
    pub enum GuardrailAction {
        Rollback => "rollback",
        Void => "void",
        Pause => "pause",
    }
}

bounded! {
    /// Restore operation classes.
    pub enum RestoreOperation {
        Restore => "restore",
        Reconcile => "reconcile",
    }
}

bounded! {
    /// Restore operation outcomes: requested, confirmed, failed, reconciled.
    pub enum RestoreOutcome {
        Requested => "requested",
        Confirmed => "confirmed",
        Failed => "failed",
        Reconciled => "reconciled",
    }
}

bounded! {
    /// Ledger verification results. Source reason codes stay bounded.
    pub enum LedgerResult {
        Verified => "verified",
        InvalidSignature => "invalid_signature",
        InvalidChain => "invalid_chain",
        Malformed => "malformed",
        Unavailable => "unavailable",
    }
}

bounded! {
    /// Retained verifier releases (a release set, not a free version).
    pub enum VerifierVersion {
        V0_1_0 => "v0.1.0",
    }
}

bounded! {
    /// Payment providers.
    pub enum PaymentProvider {
        Adyen => "adyen",
    }
}

bounded! {
    /// Payment event classes at the finance boundary. No PSP reference label
    /// is ever declared.
    pub enum PaymentEventClass {
        Authorisation => "authorisation",
        Refund => "refund",
        Dispute => "dispute",
        Reconciliation => "reconciliation",
        Cancellation => "cancellation",
        RetryScheduled => "retry_scheduled",
    }
}

bounded! {
    /// Payment processing results: accepted, invalid, duplicate, reconciled or
    /// failed provider events.
    pub enum PaymentResult {
        Accepted => "accepted",
        Invalid => "invalid",
        Duplicate => "duplicate",
        Reconciled => "reconciled",
        Failed => "failed",
    }
}

bounded! {
    /// Notification channels.
    pub enum Channel {
        Email => "email",
        Webhook => "webhook",
    }
}

bounded! {
    /// The transactional email template catalog (section 06, pages 93 to 95).
    pub enum TemplateClass {
        FeePreview => "fee_preview",
        PaymentRetry => "payment_retry",
        PaymentFailed => "payment_failed",
        PaymentReceived => "payment_received",
        AuditReady => "audit_ready",
        ReportingAccess => "reporting_access",
        GuardrailStop => "guardrail_stop",
        RestoreFailed => "restore_failed",
        ProofHold => "proof_hold",
        ApprovalRequested => "approval_requested",
    }
}

bounded! {
    /// Notification delivery attempt outcomes.
    pub enum NotificationOutcome {
        Attempted => "attempted",
        Accepted => "accepted",
        Delivered => "delivered",
        Failed => "failed",
        Duplicate => "duplicate",
    }
}

bounded! {
    /// Operator capability classes (actor and target IDs stay in the
    /// restricted action journal, never in labels).
    pub enum CapabilityClass {
        Finance => "finance",
        Connection => "connection",
        Proposal => "proposal",
        Restore => "restore",
        Experiment => "experiment",
        Reporting => "reporting",
        Incident => "incident",
        Ingestion => "ingestion",
        Recovery => "recovery",
        Jobs => "jobs",
        Fleet => "fleet",
        Access => "access",
        Policy => "policy",
    }
}

bounded! {
    /// Operator action classes.
    pub enum ActionClass {
        Approve => "approve",
        Decline => "decline",
        RetryRead => "retry_read",
        Recheck => "recheck",
        Restore => "restore",
        Credit => "credit",
        Reconcile => "reconcile",
        IncidentAcknowledge => "incident_acknowledge",
        OwnerAssign => "owner_assign",
        IncidentResolve => "incident_resolve",
        RequestReconnection => "request_reconnection",
        RestoreReconcileRequest => "restore_reconcile_request",
        RecheckRequest => "recheck_request",
        JobRetry => "job_retry",
        FleetPauseScheduling => "fleet_pause_scheduling",
        FleetResumeScheduling => "fleet_resume_scheduling",
        InvoiceHold => "invoice_hold",
        InvoiceRelease => "invoice_release",
        CollectionRetry => "collection_retry",
        DisputeMark => "dispute_mark",
        ManualReconcile => "manual_reconcile",
        SessionRevoke => "session_revoke",
        PolicyDraft => "policy_draft",
        PolicyActivate => "policy_activate",
        OperatorActionReview => "operator_action_review",
    }
}

bounded! {
    /// The generic job/audit outcome vocabulary.
    pub enum Outcome {
        Success => "success",
        Completed => "completed",
        Failed => "failed",
        Partial => "partial",
        Pending => "pending",
        Skipped => "skipped",
        Unknown => "unknown",
        Requested => "requested",
        Confirmed => "confirmed",
        Denied => "denied",
        StepUpRequired => "step_up_required",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn every_vocabulary_round_trips_through_parse() {
        assert_eq!(Platform::parse("applovin_max"), Some(Platform::AppLovinMax));
        assert_eq!(Platform::parse("northstar"), None);
        assert_eq!(Service::Api.as_str(), "apex-api");
        assert_eq!(Service::parse("apex-runner"), Some(Service::Runner));
        assert_eq!(Decision::parse("rollback"), Some(Decision::Rollback));
        assert_eq!(Decision::parse("kept"), None);
        assert_eq!(
            TemplateClass::parse("approval_requested"),
            Some(TemplateClass::ApprovalRequested)
        );
        assert_eq!(TemplateClass::parse("welcome"), None);
        assert_eq!(
            ReadinessClass::parse("disconnected"),
            Some(ReadinessClass::Disconnected)
        );
        assert_eq!(Guardrail::parse("fast_harm"), Some(Guardrail::FastHarm));
        assert_eq!(
            LedgerResult::parse("invalid_chain"),
            Some(LedgerResult::InvalidChain)
        );
        assert_eq!(RuleVersion::VALUES, &["v0.1.0"]);
        assert_eq!(VerifierVersion::VALUES, &["v0.1.0"]);
        assert_eq!(Channel::parse("email"), Some(Channel::Email));
        assert_eq!(
            NotificationOutcome::parse("duplicate"),
            Some(NotificationOutcome::Duplicate)
        );
        assert_eq!(
            ConnectorOperation::parse("createPaymentLink"),
            Some(ConnectorOperation::CreatePaymentLink)
        );
        // The adapter operations the connectors' LevelPlay/AdMob surfaces
        // meter: groups, the A/B reporting read and the v1beta experiment
        // calls are metered like every other connector call.
        for (value, operation) in [
            ("listGroups", ConnectorOperation::ListGroups),
            (
                "reportAbTestActiveUsers",
                ConnectorOperation::ReportAbTestActiveUsers,
            ),
            (
                "createMediationAbExperiment",
                ConnectorOperation::CreateMediationAbExperiment,
            ),
            (
                "stopMediationAbExperiment",
                ConnectorOperation::StopMediationAbExperiment,
            ),
            ("updateEntryPrice", ConnectorOperation::UpdateEntryPrice),
        ] {
            assert_eq!(ConnectorOperation::parse(value), Some(operation), "{value}");
            assert_eq!(operation.as_str(), value);
        }
        assert_eq!(ConnectorOperation::parse("listGroup"), None);
        assert_eq!(StatusClass::of(301), StatusClass::Redirect);
        assert_eq!(StatusClass::of(599), StatusClass::ServerError);
        assert_eq!(format!("{}", Outcome::Completed), "completed");
    }

    #[test]
    fn status_classes_cover_every_numeric_family() {
        assert_eq!(StatusClass::of(100), StatusClass::Informational);
        assert_eq!(StatusClass::of(204), StatusClass::Success);
        assert_eq!(StatusClass::of(307), StatusClass::Redirect);
        assert_eq!(StatusClass::of(404), StatusClass::ClientError);
        assert_eq!(StatusClass::of(503), StatusClass::ServerError);
        assert_eq!(StatusClass::of(99), StatusClass::ServerError);
    }

    #[test]
    fn outcome_class_vocabulary_matches_the_design() {
        assert_eq!(
            OutcomeClass::VALUES,
            &[
                "success",
                "auth_refusal",
                "rate_limit",
                "transport_failure",
                "schema_failure"
            ]
        );
    }

    #[test]
    fn no_vocabulary_is_empty_or_duplicated() {
        let vocabs: &[&[&str]] = &[
            Service::VALUES,
            Method::VALUES,
            StatusClass::VALUES,
            Platform::VALUES,
            ConnectorOperation::VALUES,
            OutcomeClass::VALUES,
            JobType::VALUES,
            ProjectionName::VALUES,
            Environment::VALUES,
            ReadinessClass::VALUES,
            RuleVersion::VALUES,
            Decision::VALUES,
            ReasonClass::VALUES,
            Guardrail::VALUES,
            GuardrailAction::VALUES,
            RestoreOperation::VALUES,
            RestoreOutcome::VALUES,
            LedgerResult::VALUES,
            VerifierVersion::VALUES,
            PaymentProvider::VALUES,
            PaymentEventClass::VALUES,
            PaymentResult::VALUES,
            Channel::VALUES,
            TemplateClass::VALUES,
            NotificationOutcome::VALUES,
            CapabilityClass::VALUES,
            ActionClass::VALUES,
            Outcome::VALUES,
        ];
        for values in vocabs {
            assert!(!values.is_empty());
            let mut sorted = values.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), values.len(), "duplicate value in {values:?}");
        }
    }
}
