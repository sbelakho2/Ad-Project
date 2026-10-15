//! Roles, capabilities and the deny-by-default check.
//!
//! Five roles exist (Observer, Support, Operator, Finance, Security). Reads
//! are open to every role; each mutation is one named capability and every
//! request checks the capability server-side before anything else happens. A
//! role that does not hold the capability renders the frozen denied copy
//! ([`crate::copy::DENIED_HEADLINE`]) and the attempt is journaled and
//! counted; nothing is requested from any platform.
//!
//! There is deliberately no capability for keeping an experiment: an
//! operator cannot type a better result (page 62, "No manual keep").

/// How long a full re-authentication (password plus authenticator code)
/// stays "recent" for sensitive capabilities: 10 minutes.
pub const STEP_UP_WINDOW_MS: i64 = 10 * 60 * 1000;

/// The operator roles of the design document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// All reads only.
    Observer,
    /// Reads plus incident acknowledgement and owner assignment.
    Support,
    /// Support plus incident resolution, retries, recovery requests and
    /// fleet scheduling control.
    Operator,
    /// Reads plus invoice holds, collection retries, disputes and manual
    /// reconciliation.
    Finance,
    /// Reads plus session revocation, policy drafts/activation and action
    /// review.
    Security,
}

impl Role {
    /// Every role, in declaration order.
    pub const ALL: [Role; 5] = [
        Role::Observer,
        Role::Support,
        Role::Operator,
        Role::Finance,
        Role::Security,
    ];

    /// Stable lower-case name (configuration file and journal).
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Observer => "observer",
            Role::Support => "support",
            Role::Operator => "operator",
            Role::Finance => "finance",
            Role::Security => "security",
        }
    }

    /// Display label.
    pub fn label(self) -> &'static str {
        match self {
            Role::Observer => "Observer",
            Role::Support => "Support",
            Role::Operator => "Operator",
            Role::Finance => "Finance",
            Role::Security => "Security",
        }
    }

    /// Parse a configuration-file role name.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|role| role.as_str() == text.trim().to_ascii_lowercase())
    }

    /// The capabilities this role holds. Reads are implicit for every role;
    /// this list is exactly the mutation surface.
    pub fn capabilities(self) -> &'static [Capability] {
        match self {
            Role::Observer => &[],
            Role::Support => &[Capability::IncidentAcknowledge, Capability::OwnerAssign],
            Role::Operator => &[
                Capability::IncidentAcknowledge,
                Capability::OwnerAssign,
                Capability::IncidentResolve,
                Capability::RetryRead,
                Capability::RequestReconnection,
                Capability::RestoreReconcileRequest,
                Capability::RecheckRequest,
                Capability::JobRetry,
                Capability::FleetPauseScheduling,
                Capability::FleetResumeScheduling,
            ],
            Role::Finance => &[
                Capability::InvoiceHold,
                Capability::InvoiceRelease,
                Capability::CollectionRetry,
                Capability::DisputeMark,
                Capability::ManualReconcile,
            ],
            Role::Security => &[
                Capability::SessionRevoke,
                Capability::PolicyDraft,
                Capability::PolicyActivate,
                Capability::OperatorActionReview,
            ],
        }
    }

    /// Whether this role holds `capability`.
    pub fn allows(self, capability: Capability) -> bool {
        self.capabilities().contains(&capability)
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Every named mutation of the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// Assign responsibility for an open incident.
    IncidentAcknowledge,
    /// Assign or change the owner of a customer or incident.
    OwnerAssign,
    /// Resolve an incident with recovery evidence.
    IncidentResolve,
    /// Retry one safe platform read.
    RetryRead,
    /// Request a publisher reconnection (credentials).
    RequestReconnection,
    /// Ask the runner to reconcile a restore state.
    RestoreReconcileRequest,
    /// Request a month-6 (or triggered) re-check.
    RecheckRequest,
    /// Retry one durable operator job.
    JobRetry,
    /// Pause fleet scheduling.
    FleetPauseScheduling,
    /// Resume fleet scheduling.
    FleetResumeScheduling,
    /// Hold one invoice line for proof.
    InvoiceHold,
    /// Release an invoice hold.
    InvoiceRelease,
    /// Retry one collection attempt.
    CollectionRetry,
    /// Mark a payment dispute.
    DisputeMark,
    /// Reconcile a payment manually with reason and source document.
    ManualReconcile,
    /// Revoke one operator session.
    SessionRevoke,
    /// Draft a versioned policy change.
    PolicyDraft,
    /// Activate a drafted policy version.
    PolicyActivate,
    /// Review a flagged operator action.
    OperatorActionReview,
}

impl Capability {
    /// Every capability, in declaration order.
    pub const ALL: [Capability; 19] = [
        Capability::IncidentAcknowledge,
        Capability::OwnerAssign,
        Capability::IncidentResolve,
        Capability::RetryRead,
        Capability::RequestReconnection,
        Capability::RestoreReconcileRequest,
        Capability::RecheckRequest,
        Capability::JobRetry,
        Capability::FleetPauseScheduling,
        Capability::FleetResumeScheduling,
        Capability::InvoiceHold,
        Capability::InvoiceRelease,
        Capability::CollectionRetry,
        Capability::DisputeMark,
        Capability::ManualReconcile,
        Capability::SessionRevoke,
        Capability::PolicyDraft,
        Capability::PolicyActivate,
        Capability::OperatorActionReview,
    ];

    /// The action kind used in URLs, capability checks and journals.
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::IncidentAcknowledge => "incident_acknowledge",
            Capability::OwnerAssign => "owner_assign",
            Capability::IncidentResolve => "incident_resolve",
            Capability::RetryRead => "retry_read",
            Capability::RequestReconnection => "request_reconnection",
            Capability::RestoreReconcileRequest => "restore_reconcile_request",
            Capability::RecheckRequest => "recheck_request",
            Capability::JobRetry => "job_retry",
            Capability::FleetPauseScheduling => "fleet_pause_scheduling",
            Capability::FleetResumeScheduling => "fleet_resume_scheduling",
            Capability::InvoiceHold => "invoice_hold",
            Capability::InvoiceRelease => "invoice_release",
            Capability::CollectionRetry => "collection_retry",
            Capability::DisputeMark => "dispute_mark",
            Capability::ManualReconcile => "manual_reconcile",
            Capability::SessionRevoke => "session_revoke",
            Capability::PolicyDraft => "policy_draft",
            Capability::PolicyActivate => "policy_activate",
            Capability::OperatorActionReview => "operator_action_review",
        }
    }

    /// Human label for the action forms.
    pub fn label(self) -> &'static str {
        match self {
            Capability::IncidentAcknowledge => "Acknowledge incident",
            Capability::OwnerAssign => "Assign owner",
            Capability::IncidentResolve => "Resolve incident",
            Capability::RetryRead => "Retry read",
            Capability::RequestReconnection => "Request reconnection",
            Capability::RestoreReconcileRequest => "Request restore reconciliation",
            Capability::RecheckRequest => "Request re-check",
            Capability::JobRetry => "Retry job",
            Capability::FleetPauseScheduling => "Pause scheduling",
            Capability::FleetResumeScheduling => "Resume scheduling",
            Capability::InvoiceHold => "Hold invoice",
            Capability::InvoiceRelease => "Release hold",
            Capability::CollectionRetry => "Retry collection",
            Capability::DisputeMark => "Mark dispute",
            Capability::ManualReconcile => "Reconcile payment",
            Capability::SessionRevoke => "Revoke session",
            Capability::PolicyDraft => "Draft policy change",
            Capability::PolicyActivate => "Activate policy version",
            Capability::OperatorActionReview => "Review action",
        }
    }

    /// The bounded `capability_class` telemetry label: the family the action
    /// belongs to, never a free-form value.
    pub fn capability_class(self) -> &'static str {
        match self {
            Capability::IncidentAcknowledge
            | Capability::IncidentResolve
            | Capability::OwnerAssign => "incident",
            Capability::RetryRead | Capability::RequestReconnection => "ingestion",
            Capability::RestoreReconcileRequest | Capability::RecheckRequest => "recovery",
            Capability::JobRetry => "jobs",
            Capability::FleetPauseScheduling | Capability::FleetResumeScheduling => "fleet",
            Capability::InvoiceHold
            | Capability::InvoiceRelease
            | Capability::CollectionRetry
            | Capability::DisputeMark
            | Capability::ManualReconcile => "finance",
            Capability::SessionRevoke | Capability::OperatorActionReview => "access",
            Capability::PolicyDraft | Capability::PolicyActivate => "policy",
        }
    }

    /// Sensitive capabilities require a full re-authentication inside the
    /// step-up window before the action runs.
    pub fn requires_step_up(self) -> bool {
        matches!(
            self,
            Capability::IncidentResolve
                | Capability::RequestReconnection
                | Capability::RestoreReconcileRequest
                | Capability::FleetPauseScheduling
                | Capability::FleetResumeScheduling
                | Capability::InvoiceHold
                | Capability::InvoiceRelease
                | Capability::CollectionRetry
                | Capability::DisputeMark
                | Capability::ManualReconcile
                | Capability::SessionRevoke
                | Capability::PolicyDraft
                | Capability::PolicyActivate
                | Capability::OperatorActionReview
        )
    }

    /// Destructive or broad actions require a typed scoped confirmation: the
    /// operator types the exact phrase, which is also the target scope.
    pub fn typed_confirmation(self) -> Option<&'static str> {
        match self {
            Capability::FleetPauseScheduling => Some("PAUSE SCHEDULING"),
            Capability::FleetResumeScheduling => Some("RESUME SCHEDULING"),
            Capability::PolicyActivate => Some("ACTIVATE POLICY"),
            Capability::SessionRevoke => Some("REVOKE SESSION"),
            _ => None,
        }
    }

    /// A reason is mandatory on the action form.
    pub fn requires_reason(self) -> bool {
        matches!(
            self,
            Capability::ManualReconcile
                | Capability::IncidentResolve
                | Capability::FleetPauseScheduling
                | Capability::FleetResumeScheduling
                | Capability::PolicyDraft
                | Capability::PolicyActivate
        )
    }

    /// A source document reference is mandatory on the action form.
    pub fn requires_source_document(self) -> bool {
        matches!(self, Capability::ManualReconcile)
    }

    /// Parse an action route kind.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.as_str() == text.trim())
    }
}

/// The result of a server-side capability check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityCheck {
    /// The role holds the capability.
    Permitted,
    /// The role does not hold it; nothing was requested from any platform.
    Denied,
}

/// Check `role` against `capability`. Deny by default: any combination not
/// explicitly listed in [`Role::capabilities`] is [`CapabilityCheck::Denied`].
pub fn check(role: Role, capability: Capability) -> CapabilityCheck {
    if role.allows(capability) {
        CapabilityCheck::Permitted
    } else {
        CapabilityCheck::Denied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_matrix_is_exact() {
        // Every role x every capability: the full expected matrix, written
        // out so a regression in either direction fails loudly.
        let expected: [(Role, &str); 5] = [
            (Role::Observer, ""),
            (Role::Support, "incident_acknowledge,owner_assign"),
            (
                Role::Operator,
                "incident_acknowledge,owner_assign,incident_resolve,retry_read,request_reconnection,restore_reconcile_request,recheck_request,job_retry,fleet_pause_scheduling,fleet_resume_scheduling",
            ),
            (
                Role::Finance,
                "invoice_hold,invoice_release,collection_retry,dispute_mark,manual_reconcile",
            ),
            (
                Role::Security,
                "session_revoke,policy_draft,policy_activate,operator_action_review",
            ),
        ];
        for (role, list) in expected {
            let actual = role
                .capabilities()
                .iter()
                .map(|capability| capability.as_str())
                .collect::<Vec<_>>()
                .join(",");
            assert_eq!(actual, list, "{role}");
            // Deny by default: every capability outside this role's list is
            // an explicit denial, not an accident of the list above.
            for capability in Capability::ALL {
                let listed = role.capabilities().contains(&capability);
                let verdict = check(role, capability);
                if listed {
                    assert_eq!(verdict, CapabilityCheck::Permitted, "{role} {capability:?}");
                } else {
                    assert_eq!(verdict, CapabilityCheck::Denied, "{role} {capability:?}");
                }
            }
        }
    }

    #[test]
    fn denied_is_the_default_for_unlisted_pairs() {
        assert_eq!(
            check(Role::Observer, Capability::IncidentAcknowledge),
            CapabilityCheck::Denied
        );
        assert_eq!(
            check(Role::Support, Capability::IncidentResolve),
            CapabilityCheck::Denied
        );
        assert_eq!(
            check(Role::Finance, Capability::SessionRevoke),
            CapabilityCheck::Denied
        );
        assert_eq!(
            check(Role::Security, Capability::InvoiceHold),
            CapabilityCheck::Denied
        );
        assert_eq!(
            check(Role::Operator, Capability::ManualReconcile),
            CapabilityCheck::Denied
        );
        assert_eq!(
            check(Role::Operator, Capability::IncidentResolve),
            CapabilityCheck::Permitted
        );
    }

    #[test]
    fn no_capability_keeps_an_experiment() {
        // The lesson of page 62: an operator cannot type a better result.
        for capability in Capability::ALL {
            let name = capability.as_str();
            assert!(!name.contains("keep"), "{name}");
            assert!(!name.contains("experiment"), "{name}");
            assert_ne!(name, "manual_keep");
        }
        assert!(Capability::parse("manual_keep").is_none());
        assert!(Capability::parse("experiment_keep").is_none());
        assert!(Capability::parse("keeper").is_none());
    }

    #[test]
    fn step_up_and_confirmation_metadata_is_bounded() {
        for capability in Capability::ALL {
            if capability.requires_step_up() {
                assert!(
                    capability.capability_class() != "incident"
                        || capability == Capability::IncidentResolve,
                    "{}",
                    capability.as_str()
                );
            }
        }
        assert!(Capability::SessionRevoke.requires_step_up());
        assert_eq!(
            Capability::FleetPauseScheduling.typed_confirmation(),
            Some("PAUSE SCHEDULING")
        );
        assert_eq!(
            Capability::SessionRevoke.typed_confirmation(),
            Some("REVOKE SESSION")
        );
        assert_eq!(Capability::OwnerAssign.typed_confirmation(), None);
        assert!(Capability::ManualReconcile.requires_reason());
        assert!(Capability::ManualReconcile.requires_source_document());
        assert!(!Capability::OwnerAssign.requires_source_document());
    }

    #[test]
    fn roles_parse_case_insensitively_and_display() {
        assert_eq!(Role::parse("observer"), Some(Role::Observer));
        assert_eq!(Role::parse(" SECURITY "), Some(Role::Security));
        assert_eq!(Role::parse("admin"), None);
        assert_eq!(Role::Support.to_string(), "support");
        assert_eq!(Role::Support.label(), "Support");
    }

    #[test]
    fn capability_kinds_round_trip() {
        for capability in Capability::ALL {
            assert_eq!(Capability::parse(capability.as_str()), Some(capability));
        }
        assert_eq!(Capability::parse("retry_read"), Some(Capability::RetryRead));
        assert_eq!(Capability::parse("Retry_Read"), None);
        assert_eq!(Capability::parse(""), None);
    }
}
