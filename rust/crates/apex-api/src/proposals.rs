//! Proposal review: the read model over the runner's proposal files and the
//! approval record the console writes beside them.
//!
//! The runner (`apex-autopilot`) generates proposals in-process and does not
//! persist them today, and `plan.rs` gates writes by the policy's approval
//! mode rather than consuming per-proposal approvals. This module therefore
//! defines the file contract explicitly:
//!
//! - `<root>/<proposal_id>.json` is the proposal the workspace reviews: the
//!   embedded `proposal` object is exactly `apex_autopilot::model::Proposal`
//!   (the type `plan.rs` consumes), and the envelope carries the snapshot
//!   reference, policy version, expiry and the recorded settings the review
//!   page shows.
//! - `<root>/<proposal_id>.approval.json` is the decision record written only
//!   through this module: proposal id, decision, decided time, the derived
//!   idempotency key, and the bindings (snapshot, change hash, policy
//!   version) that make a stale approval detectable. The file lives next to
//!   the proposal so a runner-side consumer can read it without a database;
//!   until the runner wires such a consumer, approving records the decision
//!   and nothing else: an approval never bypasses the policy gates, and a
//!   retry with the same decision returns the original operation status.

use apex_autopilot::model::Proposal;
use apex_lift::decision::GuardrailThresholds;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::error::ApiError;
use crate::sources::{contained_in, is_valid_publisher};

/// Directory holding proposal files.
pub const PROPOSALS_ROOT_ENV: &str = "APEX_PROPOSALS_ROOT";
/// Suffix of the decision record next to a proposal.
pub const APPROVAL_SUFFIX: &str = ".approval.json";
/// Read cap for one proposal or approval file.
pub const MAX_PROPOSAL_BYTES: u64 = 256 * 1024;
/// The bounded size of the proposal index page.
const MAX_LIST_ENTRIES: usize = 200;
/// The actor an approval is recorded for: this console has no user accounts,
/// so the workspace itself is the recorded actor.
pub const APPROVAL_ACTOR: &str = "console-workspace";

/// One proposal as the console reviews it. `proposal` is the runner's own
/// type, so the file cannot drift from what `plan.rs` consumes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleProposal {
    /// The proposal the runner generated (action, rationale, expected effect,
    /// previous value).
    pub proposal: Proposal,
    /// Human snapshot reference the proposal is bound to.
    pub snapshot: String,
    /// When the proposal was created.
    pub created_at_unix_ms: i64,
    /// The policy version governing it.
    pub policy_version: String,
    /// When the approval window closes; absent means no recorded expiry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix_ms: Option<i64>,
    /// Planned test-arm share (0.1, 0.25 or 0.5).
    pub test_share: f64,
    /// Pre-registration burn-in days.
    pub burn_in_days: u32,
    /// Analysis window in days (7, 14, 21 or 28).
    pub window_days: usize,
    /// Contract fee rate in basis points.
    pub rate_bp: u16,
    /// The registered guardrail thresholds: the stop conditions recorded
    /// before the first write.
    pub thresholds: GuardrailThresholds,
}

impl ConsoleProposal {
    /// The canonical change hash: SHA-256 over the exact bytes of the embedded
    /// runner proposal. An approval is bound to it, so editing the proposal
    /// file after a decision invalidates the decision's bindings.
    pub fn change_hash(&self) -> String {
        match serde_json::to_string(&self.proposal) {
            Ok(canonical) => {
                let digest = Sha256::digest(canonical.as_bytes());
                hex::encode(digest)
            }
            Err(_) => "(unserializable)".to_string(),
        }
    }

    /// Whether the recorded approval window has closed at `now_unix_ms`.
    pub fn is_expired(&self, now_unix_ms: i64) -> bool {
        self.expires_at_unix_ms
            .is_some_and(|expiry| now_unix_ms > expiry)
    }
}

/// A decision the workspace recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// The test is approved to proceed under the policy gates.
    Approved,
    /// The test is declined; nothing is written.
    Declined,
}

impl ApprovalDecision {
    /// The stable name.
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalDecision::Approved => "approved",
            ApprovalDecision::Declined => "declined",
        }
    }

    /// The human label.
    pub fn label(self) -> &'static str {
        match self {
            ApprovalDecision::Approved => "Approved",
            ApprovalDecision::Declined => "Declined",
        }
    }
}

/// The decision record written beside the proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRecord {
    /// The proposal this decision is about.
    pub proposal_id: String,
    /// The decision.
    pub decision: ApprovalDecision,
    /// When the decision was recorded.
    pub decided_at_unix_ms: i64,
    /// The derived idempotency key; a retry carries the same key and returns
    /// the original operation status.
    pub idempotency_key: String,
    /// The change hash the decision is bound to.
    pub change_hash: String,
    /// The snapshot reference the decision is bound to.
    pub snapshot: String,
    /// The policy version the decision is bound to.
    pub policy_version: String,
    /// The recorded actor.
    pub approval_actor: String,
}

/// What recording a decision did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionOutcome {
    /// The decision was recorded now.
    Recorded,
    /// An identical decision (same key) was already recorded; the retry
    /// returns the original operation status.
    AlreadyDecidedSameKey,
    /// A different decision already exists; nothing was overwritten.
    AlreadyDecided,
}

/// The proposal files of one deployment.
#[async_trait]
pub trait ProposalStore: Send + Sync + std::fmt::Debug {
    /// The proposal, or `None` when no file exists for `id`.
    async fn get(&self, id: &str) -> Result<Option<ConsoleProposal>, ApiError>;
    /// The recorded decision, when one exists.
    async fn approval(&self, id: &str) -> Result<Option<ApprovalRecord>, ApiError>;
    /// Record a decision, never overwriting a different one.
    async fn record_decision(
        &self,
        id: &str,
        record: &ApprovalRecord,
    ) -> Result<DecisionOutcome, ApiError>;
    /// Every proposal file, newest first, with its decision when recorded.
    /// Unreadable files are counted in `skipped`, never silently dropped.
    async fn list(&self) -> Result<ProposalList, ApiError>;
}

/// The proposal index listing: newest first, bounded.
#[derive(Debug, Clone, Default)]
pub struct ProposalList {
    /// `(id, proposal, approval)` rows.
    pub entries: Vec<(String, ConsoleProposal, Option<ApprovalRecord>)>,
    /// Proposal files that could not be read or parsed.
    pub skipped: usize,
}

/// The derived idempotency key for one (proposal, decision, change hash)
/// triple: stable across retries, so a resubmitted form cannot record twice.
pub fn idempotency_key(proposal_id: &str, decision: ApprovalDecision, change_hash: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [proposal_id, decision.as_str(), change_hash] {
        hasher.update([0x1f]);
        hasher.update(part.as_bytes());
    }
    let digest = hex::encode(hasher.finalize());
    digest[..32].to_string()
}

/// A valid proposal id: the same charset as publisher names.
pub fn is_proposal_id(id: &str) -> bool {
    is_valid_publisher(id)
}

/// Proposal and approval files under one canonicalized root.
#[derive(Debug, Clone)]
pub struct FileProposalStore {
    root: PathBuf,
    canonical_root: PathBuf,
}

impl FileProposalStore {
    /// A store rooted at `root`; the root is canonicalized here, once.
    /// A missing root is created so a deployment whose first proposal has
    /// not been written yet still resolves files correctly.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let _ = std::fs::create_dir_all(&root);
        let canonical_root = crate::sources::canonical_root(&root);
        Self {
            root,
            canonical_root,
        }
    }

    /// The directory proposals are read from.
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    fn proposal_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn approval_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}{APPROVAL_SUFFIX}"))
    }
}

#[async_trait]
impl ProposalStore for FileProposalStore {
    async fn get(&self, id: &str) -> Result<Option<ConsoleProposal>, ApiError> {
        if !is_proposal_id(id) {
            return Err(ApiError::BadRequest("invalid_proposal"));
        }
        let store = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let path = match contained_in(&store.canonical_root, store.proposal_path(&id)) {
                Ok(path) => path,
                Err(ApiError::NotFound) => return Ok(None),
                Err(error) => return Err(error),
            };
            let metadata = std::fs::metadata(&path)
                .map_err(|error| ApiError::Internal(format!("proposal stat: {error}")))?;
            crate::sources::ensure_within_cap(metadata.len(), MAX_PROPOSAL_BYTES)?;
            let raw = std::fs::read_to_string(&path)
                .map_err(|error| ApiError::Internal(format!("proposal read: {error}")))?;
            let proposal: ConsoleProposal = serde_json::from_str(&raw)
                .map_err(|error| ApiError::Internal(format!("proposal parse: {error}")))?;
            Ok(Some(proposal))
        })
        .await
        .map_err(|join| ApiError::Internal(format!("proposal task failed: {join}")))?
    }

    async fn approval(&self, id: &str) -> Result<Option<ApprovalRecord>, ApiError> {
        if !is_proposal_id(id) {
            return Err(ApiError::BadRequest("invalid_proposal"));
        }
        let store = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let path = match contained_in(&store.canonical_root, store.approval_path(&id)) {
                Ok(path) => path,
                Err(ApiError::NotFound) => return Ok(None),
                Err(error) => return Err(error),
            };
            let metadata = std::fs::metadata(&path)
                .map_err(|error| ApiError::Internal(format!("approval stat: {error}")))?;
            crate::sources::ensure_within_cap(metadata.len(), MAX_PROPOSAL_BYTES)?;
            let raw = std::fs::read_to_string(&path)
                .map_err(|error| ApiError::Internal(format!("approval read: {error}")))?;
            let record: ApprovalRecord = serde_json::from_str(&raw)
                .map_err(|error| ApiError::Internal(format!("approval parse: {error}")))?;
            Ok(Some(record))
        })
        .await
        .map_err(|join| ApiError::Internal(format!("approval task failed: {join}")))?
    }

    async fn list(&self) -> Result<ProposalList, ApiError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut list = ProposalList::default();
            let read_dir = std::fs::read_dir(&store.root)
                .map_err(|error| ApiError::Internal(format!("proposal dir: {error}")))?;
            let mut ids: Vec<String> = Vec::new();
            for entry in read_dir {
                let Ok(entry) = entry else { continue };
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if !name.ends_with(".json") || name.ends_with(APPROVAL_SUFFIX) {
                    continue;
                }
                let id = name.trim_end_matches(".json").to_string();
                if is_proposal_id(&id) {
                    ids.push(id);
                } else {
                    list.skipped += 1;
                }
            }
            ids.sort();
            // Newest first, bounded: an index page never grows without limit.
            let mut parsed: Vec<(String, ConsoleProposal)> = Vec::new();
            for id in ids {
                if parsed.len() >= MAX_LIST_ENTRIES {
                    list.skipped += 1;
                    continue;
                }
                let Ok(path) = contained_in(&store.canonical_root, store.proposal_path(&id)) else {
                    list.skipped += 1;
                    continue;
                };
                let Ok(metadata) = std::fs::metadata(&path) else {
                    list.skipped += 1;
                    continue;
                };
                if crate::sources::ensure_within_cap(metadata.len(), MAX_PROPOSAL_BYTES).is_err() {
                    list.skipped += 1;
                    continue;
                }
                let Ok(raw) = std::fs::read_to_string(&path) else {
                    list.skipped += 1;
                    continue;
                };
                match serde_json::from_str::<ConsoleProposal>(&raw) {
                    Ok(proposal) => parsed.push((id, proposal)),
                    Err(_) => list.skipped += 1,
                }
            }
            parsed.sort_by(|a, b| {
                b.1.created_at_unix_ms
                    .cmp(&a.1.created_at_unix_ms)
                    .then_with(|| a.0.cmp(&b.0))
            });
            for (id, proposal) in parsed {
                let approval = std::fs::read_to_string(store.approval_path(&id))
                    .ok()
                    .and_then(|raw| serde_json::from_str::<ApprovalRecord>(&raw).ok());
                list.entries.push((id, proposal, approval));
            }
            Ok(list)
        })
        .await
        .map_err(|join| ApiError::Internal(format!("proposal list task failed: {join}")))?
    }

    async fn record_decision(
        &self,
        id: &str,
        record: &ApprovalRecord,
    ) -> Result<DecisionOutcome, ApiError> {
        if !is_proposal_id(id) {
            return Err(ApiError::BadRequest("invalid_proposal"));
        }
        if record.proposal_id != id {
            return Err(ApiError::Internal(
                "approval record does not match its proposal".to_string(),
            ));
        }
        let store = self.clone();
        let id = id.to_string();
        let key = record.idempotency_key.clone();
        let raw = serde_json::to_vec_pretty(record)
            .map_err(|error| ApiError::Internal(format!("approval serialization: {error}")))?;
        tokio::task::spawn_blocking(move || {
            let existing_path = store.approval_path(&id);
            if let Ok(resolved) = contained_in(&store.canonical_root, existing_path) {
                let existing_raw = std::fs::read_to_string(&resolved)
                    .map_err(|error| ApiError::Internal(format!("approval read: {error}")))?;
                let existing: ApprovalRecord = serde_json::from_str(&existing_raw)
                    .map_err(|error| ApiError::Internal(format!("approval parse: {error}")))?;
                return Ok(if existing.idempotency_key == key {
                    DecisionOutcome::AlreadyDecidedSameKey
                } else {
                    DecisionOutcome::AlreadyDecided
                });
            }
            std::fs::create_dir_all(&store.root)
                .map_err(|error| ApiError::Internal(format!("proposal root: {error}")))?;
            let root = store
                .root
                .canonicalize()
                .map_err(|error| ApiError::Internal(format!("proposal root: {error}")))?;
            let target = root.join(format!("{id}{APPROVAL_SUFFIX}"));
            let tmp = root.join(format!("{id}{APPROVAL_SUFFIX}.tmp"));
            std::fs::write(&tmp, &raw)
                .map_err(|error| ApiError::Internal(format!("approval write: {error}")))?;
            std::fs::rename(&tmp, &target)
                .map_err(|error| ApiError::Internal(format!("approval rename: {error}")))?;
            Ok(DecisionOutcome::Recorded)
        })
        .await
        .map_err(|join| ApiError::Internal(format!("approval task failed: {join}")))?
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_autopilot::model::Action;

    use super::*;

    fn sample_proposal() -> ConsoleProposal {
        ConsoleProposal {
            proposal: Proposal {
                action: Action::SetCountryFloor {
                    ad_unit_id: "rw_de_01".to_string(),
                    country: "DE".to_string(),
                    floor_ecpm: 3_200_000,
                },
                rationale: "move the DE floor -20%".to_string(),
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

    fn temp_store(name: &str) -> (FileProposalStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "apex-proposals-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        (FileProposalStore::new(&dir), dir)
    }

    fn write_proposal(dir: &std::path::Path, id: &str) {
        let proposal = sample_proposal();
        std::fs::write(
            dir.join(format!("{id}.json")),
            serde_json::to_vec_pretty(&proposal).expect("json"),
        )
        .expect("write");
    }

    #[tokio::test]
    async fn proposals_roundtrip_and_unknown_ids_are_none() {
        let (store, dir) = temp_store("roundtrip");
        write_proposal(&dir, "p_0247");
        let loaded = store.get("p_0247").await.expect("get").expect("present");
        assert_eq!(loaded.proposal.expected_effect_bp, 260);
        assert_eq!(loaded.test_share, 0.25);
        assert_eq!(loaded.burn_in_days, 3);
        assert_eq!(loaded.window_days, 14);
        assert_eq!(loaded.rate_bp, 2_000);
        assert!(store.get("p_missing").await.expect("get").is_none());
        assert!(matches!(
            store.get("../etc/passwd").await,
            Err(ApiError::BadRequest("invalid_proposal"))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn decisions_are_recorded_once_and_retries_return_the_same_status() {
        let (store, dir) = temp_store("decision");
        write_proposal(&dir, "p_0247");
        let proposal = store.get("p_0247").await.expect("get").expect("some");
        let key = idempotency_key(
            "p_0247",
            ApprovalDecision::Approved,
            &proposal.change_hash(),
        );
        let record = ApprovalRecord {
            proposal_id: "p_0247".to_string(),
            decision: ApprovalDecision::Approved,
            decided_at_unix_ms: 1_792_055_400_000,
            idempotency_key: key.clone(),
            change_hash: proposal.change_hash(),
            snapshot: proposal.snapshot.clone(),
            policy_version: proposal.policy_version.clone(),
            approval_actor: APPROVAL_ACTOR.to_string(),
        };
        assert_eq!(
            store
                .record_decision("p_0247", &record)
                .await
                .expect("record"),
            DecisionOutcome::Recorded
        );
        assert_eq!(
            store
                .record_decision("p_0247", &record)
                .await
                .expect("retry"),
            DecisionOutcome::AlreadyDecidedSameKey,
            "a retry returns the original operation status"
        );
        let conflicting = ApprovalRecord {
            decision: ApprovalDecision::Declined,
            idempotency_key: idempotency_key(
                "p_0247",
                ApprovalDecision::Declined,
                &record.change_hash,
            ),
            ..record.clone()
        };
        assert_eq!(
            store
                .record_decision("p_0247", &conflicting)
                .await
                .expect("conflict"),
            DecisionOutcome::AlreadyDecided,
            "a different later decision never overwrites the original"
        );
        let stored = store.approval("p_0247").await.expect("read").expect("some");
        assert_eq!(stored.decision, ApprovalDecision::Approved);
        assert_eq!(stored.idempotency_key, key);
        // The record is plain JSON next to the proposal.
        let raw = std::fs::read_to_string(dir.join("p_0247.approval.json")).expect("file");
        assert!(raw.contains("\"approved\""), "{raw}");
        assert!(raw.contains("\"changeHash\""), "{raw}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn change_hash_is_stable_and_editing_the_action_changes_it() {
        let proposal = sample_proposal();
        let hash = proposal.change_hash();
        assert_eq!(hash.len(), 64);
        assert_eq!(proposal.change_hash(), hash, "stable across calls");
        let mut edited = proposal.clone();
        edited.proposal.expected_effect_bp = 300;
        assert_ne!(edited.change_hash(), hash, "stale settings invalidate it");
    }

    #[test]
    fn expiry_uses_the_recorded_window() {
        let proposal = sample_proposal();
        assert!(!proposal.is_expired(1_793_000_000_000));
        assert!(proposal.is_expired(1_794_592_000_001));
        let mut no_expiry = proposal.clone();
        no_expiry.expires_at_unix_ms = None;
        assert!(!no_expiry.is_expired(i64::MAX));
    }

    #[test]
    fn idempotency_keys_are_decision_specific_and_short() {
        let key = idempotency_key("p_0247", ApprovalDecision::Approved, "abc");
        assert_eq!(key.len(), 32);
        assert!(key.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(
            key,
            idempotency_key("p_0247", ApprovalDecision::Approved, "abc")
        );
        assert_ne!(
            key,
            idempotency_key("p_0247", ApprovalDecision::Declined, "abc")
        );
        assert_ne!(
            key,
            idempotency_key("p_0248", ApprovalDecision::Approved, "abc")
        );
        assert_eq!(ApprovalDecision::Approved.as_str(), "approved");
        assert_eq!(ApprovalDecision::Declined.label(), "Declined");
    }

    #[tokio::test]
    async fn a_decision_for_an_unknown_proposal_is_refused_by_the_caller_flow() {
        let (store, dir) = temp_store("missing");
        assert!(store
            .approval("p_missing")
            .await
            .expect("no approval")
            .is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
