//! Endpoint metadata: every platform operation carries how its URL was established.
//! `VerifyLive` operations may be exercised in read-only audit mode but MUST be
//! confirmed by the live-integration lane before the autopilot is allowed to write
//! through them (the autopilot enforces this at propose-time).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceVerification {
    /// Path is verifiable from public platform documentation.
    Documented,
    /// Path must be confirmed against the platform sandbox before writes are enabled.
    VerifyLive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointMeta {
    pub name: &'static str,
    pub method: HttpMethod,
    pub verification: SourceVerification,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl EndpointMeta {
    pub const fn new(
        name: &'static str,
        method: HttpMethod,
        verification: SourceVerification,
        idempotent: bool,
    ) -> Self {
        Self {
            name,
            method,
            verification,
            idempotent,
        }
    }

    /// Autopilot writes are only allowed on operations whose URL has been verified
    /// against the live sandbox; reads may proceed with `VerifyLive` paths because a
    /// failed read is harmless and its failure is recorded.
    pub fn writable(&self) -> bool {
        self.method != HttpMethod::Get && self.verification == SourceVerification::Documented
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_blocked_until_documented() {
        let read_live = EndpointMeta::new(
            "listWaterfall",
            HttpMethod::Get,
            SourceVerification::VerifyLive,
            true,
        );
        assert!(!read_live.writable());
        let write_live = EndpointMeta::new(
            "updateEntryPrice",
            HttpMethod::Patch,
            SourceVerification::VerifyLive,
            false,
        );
        assert!(!write_live.writable());
        let write_doc = EndpointMeta::new(
            "setCountryFloor",
            HttpMethod::Put,
            SourceVerification::Documented,
            false,
        );
        assert!(write_doc.writable());
    }

    #[test]
    fn serde_renders_kebab_case() {
        let j = serde_json::to_string(&SourceVerification::VerifyLive).expect("ser");
        assert_eq!(j, "\"verify-live\"");
        let j = serde_json::to_string(&SourceVerification::Documented).expect("ser");
        assert_eq!(j, "\"documented\"");
    }
}
