//! The one decision gate every email passes, twice.
//!
//! [`decide`] evaluates every hard constraint in a fixed order and records
//! **every** failure, not just the first, so the decision log explains a
//! denial completely. Expected value never overrides a hard gate. The same
//! function runs again immediately before the send (re-validation), so an
//! unsubscribe, reply, complaint breaker or legal-table change that arrived
//! after planning still stops the message.

use crate::health::SenderState;
use crate::model::UnixSecs;
use crate::policy::{AccountVerdict, LegalVerdict, SuppressionReason};

/// How much the engine may do on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autonomy {
    /// Kill switch: plan nothing, send nothing. Inbound processing continues.
    Off,
    /// Run everything and log what would be sent; send nothing.
    Shadow,
    /// Send only messages an operator approved.
    Approval,
    /// Send what passes the gate.
    Autonomous,
}

impl Autonomy {
    /// Parse `off`, `shadow`, `approval` or `autonomous`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Autonomy::Off),
            "shadow" => Some(Autonomy::Shadow),
            "approval" => Some(Autonomy::Approval),
            "autonomous" => Some(Autonomy::Autonomous),
            _ => None,
        }
    }

    /// Stable string form.
    pub fn as_str(self) -> &'static str {
        match self {
            Autonomy::Off => "off",
            Autonomy::Shadow => "shadow",
            Autonomy::Approval => "approval",
            Autonomy::Autonomous => "autonomous",
        }
    }
}

/// Everything the gate reads.
#[derive(Debug, Clone, PartialEq)]
pub struct GateInput {
    /// Operating mode.
    pub autonomy: Autonomy,
    /// Suppression, if any.
    pub suppression: Option<SuppressionReason>,
    /// Legal verdict.
    pub legal: LegalVerdict,
    /// Account-coordination verdict.
    pub account: AccountVerdict,
    /// Publisher is in the randomized holdout.
    pub held_out: bool,
    /// Sending-domain state.
    pub sender: SenderState,
    /// Sends left under today's cap.
    pub cap_left: u32,
    /// The message passed grounding validation.
    pub grounded: bool,
    /// A safe message variant exists.
    pub variant_available: bool,
    /// Expected value of this touch (USD).
    pub expected_value: f64,
}

/// What to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// Send.
    Send,
    /// Would send, but the engine is in shadow mode.
    Shadow,
    /// Would send once an operator approves.
    NeedsApproval,
    /// Try again at the given time.
    Defer(UnixSecs),
    /// Do not send.
    Deny,
}

impl Verdict {
    /// Stable string form.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Send => "send",
            Verdict::Shadow => "shadow",
            Verdict::NeedsApproval => "needs_approval",
            Verdict::Defer(_) => "defer",
            Verdict::Deny => "deny",
        }
    }
}

/// A decision with all its reasons.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// Verdict.
    pub verdict: Verdict,
    /// Every reason a constraint failed, in evaluation order.
    pub reasons: Vec<String>,
}

/// Evaluate the gate.
pub fn decide(input: &GateInput) -> Decision {
    let mut reasons = Vec::new();
    let mut deny = false;
    let mut defer_until: Option<UnixSecs> = None;
    if input.autonomy == Autonomy::Off {
        reasons.push("kill switch is on".to_string());
        deny = true;
    }
    if let Some(r) = input.suppression {
        reasons.push(format!("suppressed ({r:?})"));
        deny = true;
    }
    match &input.legal {
        LegalVerdict::Allowed => {}
        LegalVerdict::ConsentRequired(why) => {
            reasons.push(format!("consent required: {why}"));
            deny = true;
        }
        LegalVerdict::Review(why) => {
            reasons.push(format!("legal review: {why}"));
            deny = true;
        }
    }
    match &input.account {
        AccountVerdict::Allowed => {}
        AccountVerdict::Deferred(until, why) => {
            reasons.push(format!("account: {why}"));
            defer_until = Some(*until);
        }
        AccountVerdict::Denied(why) => {
            reasons.push(format!("account: {why}"));
            deny = true;
        }
    }
    if input.held_out {
        reasons.push("publisher is in the randomized holdout".to_string());
        deny = true;
    }
    if !input.sender.may_send() {
        reasons.push(format!("sending domain is {}", input.sender.as_str()));
        deny = true;
    } else if input.cap_left == 0 {
        reasons.push("daily cap reached".to_string());
        deny = true;
    }
    if !input.grounded {
        reasons.push("message makes a claim without evidence".to_string());
        deny = true;
    }
    if !input.variant_available {
        reasons.push("no message variant passes the complaint safety rule".to_string());
        deny = true;
    }
    if input.expected_value.is_nan() || input.expected_value <= 0.0 {
        reasons.push(format!(
            "expected value {:.2} is not positive",
            input.expected_value
        ));
        deny = true;
    }
    let verdict = if deny {
        Verdict::Deny
    } else if let Some(t) = defer_until {
        Verdict::Defer(t)
    } else {
        match input.autonomy {
            Autonomy::Off => Verdict::Deny,
            Autonomy::Shadow => Verdict::Shadow,
            Autonomy::Approval => Verdict::NeedsApproval,
            Autonomy::Autonomous => Verdict::Send,
        }
    };
    Decision { verdict, reasons }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> GateInput {
        GateInput {
            autonomy: Autonomy::Autonomous,
            suppression: None,
            legal: LegalVerdict::Allowed,
            account: AccountVerdict::Allowed,
            held_out: false,
            sender: SenderState::Healthy,
            cap_left: 10,
            grounded: true,
            variant_available: true,
            expected_value: 3.0,
        }
    }

    #[test]
    fn a_clean_input_sends_per_mode() {
        assert_eq!(decide(&ok()).verdict, Verdict::Send);
        for (mode, v) in [
            (Autonomy::Shadow, Verdict::Shadow),
            (Autonomy::Approval, Verdict::NeedsApproval),
            (Autonomy::Off, Verdict::Deny),
        ] {
            let d = decide(&GateInput {
                autonomy: mode,
                ..ok()
            });
            assert_eq!(d.verdict, v);
        }
        assert!(decide(&ok()).reasons.is_empty());
    }

    #[test]
    fn every_failure_is_recorded() {
        let input = GateInput {
            autonomy: Autonomy::Off,
            suppression: Some(SuppressionReason::OptOut),
            legal: LegalVerdict::ConsentRequired("DE".into()),
            account: AccountVerdict::Denied("busy".into()),
            held_out: true,
            sender: SenderState::Quarantined,
            cap_left: 0,
            grounded: false,
            variant_available: false,
            expected_value: f64::NAN,
        };
        let d = decide(&input);
        assert_eq!(d.verdict, Verdict::Deny);
        assert_eq!(d.reasons.len(), 9, "{:?}", d.reasons);
        let review = decide(&GateInput {
            legal: LegalVerdict::Review("x".into()),
            ..ok()
        });
        assert_eq!(review.verdict, Verdict::Deny);
        let capped = decide(&GateInput {
            cap_left: 0,
            ..ok()
        });
        assert_eq!(capped.reasons, vec!["daily cap reached".to_string()]);
    }

    #[test]
    fn deferral_only_when_nothing_denies() {
        let d = decide(&GateInput {
            account: AccountVerdict::Deferred(99, "cool-off".into()),
            ..ok()
        });
        assert_eq!(d.verdict, Verdict::Defer(99));
        let d = decide(&GateInput {
            account: AccountVerdict::Deferred(99, "cool-off".into()),
            held_out: true,
            ..ok()
        });
        assert_eq!(d.verdict, Verdict::Deny);
        assert_eq!(Autonomy::parse(" Shadow "), Some(Autonomy::Shadow));
        assert_eq!(Autonomy::parse("x"), None);
        for v in [
            Verdict::Send,
            Verdict::Shadow,
            Verdict::NeedsApproval,
            Verdict::Defer(1),
            Verdict::Deny,
        ] {
            assert!(!v.as_str().is_empty());
        }
        for a in [
            Autonomy::Off,
            Autonomy::Shadow,
            Autonomy::Approval,
            Autonomy::Autonomous,
        ] {
            assert_eq!(Autonomy::parse(a.as_str()), Some(a));
        }
    }
}
