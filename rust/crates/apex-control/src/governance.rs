//! Governance records: versioned policy drafts and fleet interventions.
//!
//! Both are append-only JSONL registers, the same durability discipline as
//! the action journal:
//!
//! - a policy change is a draft that can later be activated; activation is a
//!   second record naming the activator and the preview, and the active
//!   version history is reconstructable from the file. A policy change never
//!   rewrites an experiment result or a signed fee basis; it only constrains
//!   future proposals, and the active record states which future proposals
//!   would be affected;
//! - a fleet intervention (pause or resume scheduling) records platform,
//!   customer population, reason and expected effect. Reads continue
//!   independently: no record here stops a read.

use std::io::Write as _;
use std::path::PathBuf;

use crate::signing::random_hex_32;

/// A versioned policy draft (page 72).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRecord {
    /// `pol_<12 hex>`.
    pub policy_id: String,
    /// Monotonically increasing draft version.
    pub version: u32,
    /// Authoring operator.
    pub author: String,
    /// When the draft was recorded.
    pub created_at_ms: i64,
    /// Drafted caps (bounded JSON).
    pub caps: serde_json::Value,
    /// Drafted analysis windows (bounded JSON).
    pub windows: serde_json::Value,
    /// Drafted thresholds (bounded JSON).
    pub thresholds: serde_json::Value,
    /// Which future proposals would be affected by activation.
    pub affected_proposals: String,
    /// `pending` or `active`.
    pub state: String,
    /// Activating operator, when activated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activated_by: Option<String>,
    /// Activation time, when activated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activated_at_ms: Option<i64>,
    /// Correlation id shared with the journal.
    pub correlation_id: String,
}

/// A fleet intervention record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterventionRecord {
    /// `itv_<12 hex>`.
    pub intervention_id: String,
    /// `pause_scheduling` or `resume_scheduling`.
    pub kind: String,
    /// The platform whose scheduling is affected, when scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// The customer population affected.
    pub customer_population: String,
    /// The operator's reason.
    pub reason: String,
    /// The expected effect.
    pub expected_effect: String,
    /// The operator who recorded it.
    pub actor: String,
    /// When it was recorded.
    pub at_ms: i64,
    /// Correlation id shared with the journal.
    pub correlation_id: String,
}

/// Governance register failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GovernanceError {
    /// A record could not be written.
    #[error("governance write failed at {path}: {reason}")]
    Write {
        /// File involved.
        path: String,
        /// Underlying reason.
        reason: String,
    },
    /// A register could not be read back.
    #[error("governance read failed at {path}: {reason}")]
    Read {
        /// File involved.
        path: String,
        /// Underlying reason.
        reason: String,
    },
    /// A record could not be serialized.
    #[error("governance record is invalid: {reason}")]
    Invalid {
        /// Underlying reason.
        reason: String,
    },
}

/// A register read back, with corrupt lines counted rather than dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterRead<T> {
    /// Parsed records, in file order.
    pub records: Vec<T>,
    /// 1-based lines that did not parse.
    pub corrupt_lines: Vec<usize>,
}

impl<T> Default for RegisterRead<T> {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            corrupt_lines: Vec::new(),
        }
    }
}

/// The append-only governance registers.
#[derive(Debug, Clone)]
pub struct GovernanceStore {
    /// Policy register path.
    pub policies_path: PathBuf,
    /// Intervention register path.
    pub interventions_path: PathBuf,
}

impl GovernanceStore {
    /// A store over the two register paths.
    #[allow(clippy::too_many_arguments)]
    pub fn new(policies_path: impl Into<PathBuf>, interventions_path: impl Into<PathBuf>) -> Self {
        Self {
            policies_path: policies_path.into(),
            interventions_path: interventions_path.into(),
        }
    }

    /// Append one policy record.
    pub fn append_policy(&self, record: &PolicyRecord) -> Result<(), GovernanceError> {
        append_jsonl(&self.policies_path, record)
    }

    /// Read the policy register.
    pub fn policy_records(&self) -> Result<RegisterRead<PolicyRecord>, GovernanceError> {
        read_jsonl(&self.policies_path)
    }

    /// The active policy records, version ascending.
    pub fn active_policies(&self) -> Result<Vec<PolicyRecord>, GovernanceError> {
        Ok(self
            .policy_records()?
            .records
            .into_iter()
            .filter(|record| record.state == "active")
            .collect())
    }

    /// Pending drafts that no later activation superseded.
    pub fn pending_policies(&self) -> Result<Vec<PolicyRecord>, GovernanceError> {
        let records = self.policy_records()?.records;
        let activated: Vec<String> = records
            .iter()
            .filter(|record| record.state == "active")
            .map(|record| record.policy_id.clone())
            .collect();
        Ok(records
            .into_iter()
            .filter(|record| record.state == "pending" && !activated.contains(&record.policy_id))
            .collect())
    }

    /// The next draft version for a fresh policy id.
    pub fn next_version(&self) -> Result<u32, GovernanceError> {
        Ok(self
            .policy_records()?
            .records
            .iter()
            .map(|record| record.version)
            .max()
            .unwrap_or(0)
            .saturating_add(1))
    }

    /// Append one intervention record.
    pub fn append_intervention(&self, record: &InterventionRecord) -> Result<(), GovernanceError> {
        append_jsonl(&self.interventions_path, record)
    }

    /// Read the intervention register.
    pub fn interventions(&self) -> Result<RegisterRead<InterventionRecord>, GovernanceError> {
        read_jsonl(&self.interventions_path)
    }

    /// Whether the fleet is currently paused: the last pause/resume record
    /// wins.
    pub fn fleet_scheduling_paused(&self) -> Result<bool, GovernanceError> {
        let records = self.interventions()?.records;
        let last = records.iter().rev().find(|record| {
            matches!(
                record.kind.as_str(),
                "pause_scheduling" | "resume_scheduling"
            )
        });
        Ok(matches!(
            last.map(|record| record.kind.as_str()),
            Some("pause_scheduling")
        ))
    }
}

impl PolicyRecord {
    /// A fresh pending draft.
    #[allow(clippy::too_many_arguments)]
    pub fn draft(
        author: &str,
        created_at_ms: i64,
        caps: serde_json::Value,
        windows: serde_json::Value,
        thresholds: serde_json::Value,
        affected_proposals: &str,
        version: u32,
        correlation_id: &str,
    ) -> Self {
        Self {
            policy_id: format!("pol_{}", &random_hex_32()[..12]),
            version,
            author: author.to_string(),
            created_at_ms,
            caps,
            windows,
            thresholds,
            affected_proposals: affected_proposals.to_string(),
            state: "pending".to_string(),
            activated_by: None,
            activated_at_ms: None,
            correlation_id: correlation_id.to_string(),
        }
    }
}

impl InterventionRecord {
    /// A fresh intervention record.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: &str,
        platform: Option<&str>,
        customer_population: &str,
        reason: &str,
        expected_effect: &str,
        actor: &str,
        at_ms: i64,
        correlation_id: &str,
    ) -> Self {
        Self {
            intervention_id: format!("itv_{}", &random_hex_32()[..12]),
            kind: kind.to_string(),
            platform: platform.map(str::to_string),
            customer_population: customer_population.to_string(),
            reason: reason.to_string(),
            expected_effect: expected_effect.to_string(),
            actor: actor.to_string(),
            at_ms,
            correlation_id: correlation_id.to_string(),
        }
    }
}

fn append_jsonl<T: serde::Serialize>(
    path: &std::path::Path,
    record: &T,
) -> Result<(), GovernanceError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| GovernanceError::Write {
            path: path.display().to_string(),
            reason: format!("creating the register directory failed: {error}"),
        })?;
    }
    let mut line = serde_json::to_string(record).map_err(|error| GovernanceError::Invalid {
        reason: error.to_string(),
    })?;
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| GovernanceError::Write {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
    file.write_all(line.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|error| GovernanceError::Write {
            path: path.display().to_string(),
            reason: error.to_string(),
        })
}

fn read_jsonl<T: serde::de::DeserializeOwned>(
    path: &std::path::Path,
) -> Result<RegisterRead<T>, GovernanceError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RegisterRead::default())
        }
        Err(error) => {
            return Err(GovernanceError::Read {
                path: path.display().to_string(),
                reason: error.to_string(),
            })
        }
    };
    let mut read = RegisterRead::default();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(line) {
            Ok(record) => read.records.push(record),
            Err(_) => read.corrupt_lines.push(index + 1),
        }
    }
    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (GovernanceStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-governance-{}-{}",
            std::process::id(),
            random_hex_32()
        ));
        (
            GovernanceStore::new(dir.join("policies.jsonl"), dir.join("interventions.jsonl")),
            dir,
        )
    }

    #[test]
    fn a_policy_draft_then_activation_is_two_records() {
        let (store, dir) = store();
        assert!(store.pending_policies().expect("read").is_empty());
        let draft = PolicyRecord::draft(
            "maya",
            1_000,
            serde_json::json!({"max_step_bp": 1_000}),
            serde_json::json!({"window_days": 14}),
            serde_json::json!({"fill_drop": 0.1}),
            "proposals drafted after activation; active experiments keep their registered policy",
            1,
            "corr-policy-1",
        );
        store.append_policy(&draft).expect("append");
        let pending = store.pending_policies().expect("pending");
        assert_eq!(pending, vec![draft.clone()]);
        assert!(store.active_policies().expect("active").is_empty());
        let mut activated = draft.clone();
        activated.state = "active".to_string();
        activated.activated_by = Some("sam".to_string());
        activated.activated_at_ms = Some(2_000);
        store.append_policy(&activated).expect("append");
        assert!(store.pending_policies().expect("pending").is_empty());
        let active = store.active_policies().expect("active");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].activated_by.as_deref(), Some("sam"));
        // The file has both records, so author and activation time stay
        // traceable.
        let read = store.policy_records().expect("records");
        assert_eq!(read.records.len(), 2);
        assert_eq!(store.next_version().expect("version"), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fleet_pause_and_resume_compose_by_the_last_record() {
        let (store, dir) = store();
        assert!(!store.fleet_scheduling_paused().expect("default"));
        store
            .append_intervention(&InterventionRecord::new(
                "pause_scheduling",
                Some("applovin_max"),
                "all publishers",
                "platform incident",
                "no new experiments start; reads continue",
                "maya",
                1_000,
                "c1",
            ))
            .expect("append");
        assert!(store.fleet_scheduling_paused().expect("paused"));
        store
            .append_intervention(&InterventionRecord::new(
                "resume_scheduling",
                Some("applovin_max"),
                "all publishers",
                "platform recovered",
                "scheduling resumes",
                "maya",
                2_000,
                "c2",
            ))
            .expect("append");
        assert!(!store.fleet_scheduling_paused().expect("resumed"));
        let read = store.interventions().expect("records");
        assert_eq!(read.records.len(), 2);
        assert!(read.records[0].intervention_id.starts_with("itv_"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_register_lines_are_counted_not_dropped() {
        let (store, dir) = store();
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(store.policies_path.clone(), "{\"nope\":1}\n\n").expect("write");
        let read = store.policy_records().expect("read");
        assert_eq!(read.corrupt_lines, vec![1]);
        assert!(read.records.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_never_carry_tokens_or_credentials() {
        let record = PolicyRecord::draft(
            "maya",
            0,
            serde_json::json!({"max_step_bp": 1000}),
            serde_json::json!({}),
            serde_json::json!({}),
            "future proposals",
            1,
            "c",
        );
        let line = serde_json::to_string(&record).expect("json");
        assert!(line.contains("\"state\":\"pending\""));
        assert!(!line.contains("token"));
        assert!(!line.contains("password"));
    }
}
