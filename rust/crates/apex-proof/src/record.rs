//! Ledger records: the tamper-evident payload chain.
//!
//! [`LedgerRecord`] is a camelCase, internally tagged (`"type"`) enum. Every
//! variant carries the chain envelope `seq`, `atUnixMs`, `platform`,
//! `prevHash` and `hash` plus its variant-specific fields. `prevHash` and
//! `hash` are 32-byte SHA-256 digests serialized as lowercase hex strings.
//!
//! Records are built with the constructors on this type (or by
//! [`crate::build_billing_proof`]) in an *unsealed* state: `seq` is 0 and both
//! hashes are zeros. [`crate::Ledger`] / [`crate::InMemoryLedger`] seal them on
//! append by assigning the next sequence number, linking `prevHash` to the
//! previous record and computing `hash` over the canonical JSON of the record
//! with the `hash` field removed (see [`crate::chain`]).

use apex_core::{ExperimentResult, ExperimentSpec, PlatformId};
use serde::{Deserialize, Serialize};

/// Serde adapter for `[u8; 32]` digests: lowercase hex strings in JSON,
/// validated for length and hex-ness on the way back in.
pub(crate) mod hash_hex {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(
        value: &[u8; 32],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(value))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u8; 32], D::Error> {
        let encoded = String::deserialize(deserializer)?;
        let bytes = hex::decode(&encoded).map_err(D::Error::custom)?;
        let digest: [u8; 32] = bytes
            .try_into()
            .map_err(|_| D::Error::custom("expected a 32-byte hex digest"))?;
        Ok(digest)
    }
}

/// The single platform mutation an autopilot applied, as recorded in a
/// `changeApplied` entry. Field names mirror the connector idempotency keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ChangeKind {
    /// A country floor was set on an ad unit (AppLovin MAX).
    CountryFloorSet { country: String, floor_ecpm: i64 },
    /// An AdMob mediation-group eCPM was set.
    EcpmSet {
        mediation_group_id: String,
        ad_unit_id: String,
        ecpm: i64,
    },
    /// A LevelPlay waterfall entry price was updated.
    WaterfallEntryPriceSet {
        ad_unit_id: String,
        entry_id: String,
        price_ecpm: i64,
    },
    /// A LevelPlay waterfall entry was created.
    WaterfallEntryCreated {
        ad_unit_id: String,
        network: String,
        instance_name: String,
        price_ecpm: i64,
    },
    /// A LevelPlay waterfall entry was removed.
    WaterfallEntryRemoved {
        ad_unit_id: String,
        entry_id: String,
    },
    /// A MAX frequency-capping setting was applied to the experiment's test
    /// arm. The country list is the exact set the setting covers.
    FrequencyCapSet {
        ad_unit_id: String,
        cap_type: String,
        countries: Vec<String>,
        session_limit: Option<u32>,
        day_limit: Option<u32>,
        minute_frequency: Option<u32>,
    },
    /// A LevelPlay group-B session/interstitial/day cap was applied.
    LevelplayGroupCapping {
        group_id: String,
        session_limit: Option<u32>,
        interstitial_limit: Option<u32>,
        day_limit: Option<u32>,
    },
}

/// One link of the tamper-evident ledger.
///
/// The envelope fields (`seq`, `atUnixtMs` as `atUnixMs`, `platform`,
/// `prevHash`, `hash`) are present on every variant so the hash chain is
/// self-contained in each JSON line. `prevHash` of the genesis record is all
/// zeros.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LedgerRecord {
    /// A platform mutation was applied by the autopilot. `prev_value` is the
    /// setting's value as read from the platform immediately before the
    /// write (principle 6: every change is reversible).
    ChangeApplied {
        seq: u64,
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: String,
        change: ChangeKind,
        idempotency_key: String,
        #[serde(default)]
        prev_value: String,
        #[serde(with = "hash_hex")]
        prev_hash: [u8; 32],
        #[serde(with = "hash_hex")]
        hash: [u8; 32],
    },
    /// A MIN/MAX experiment was started.
    ExperimentStarted {
        seq: u64,
        at_unix_ms: i64,
        platform: PlatformId,
        experiment_id: String,
        spec: ExperimentSpec,
        #[serde(with = "hash_hex")]
        prev_hash: [u8; 32],
        #[serde(with = "hash_hex")]
        hash: [u8; 32],
    },
    /// An experiment was observed (interim or final result).
    ExperimentObserved {
        seq: u64,
        at_unix_ms: i64,
        platform: PlatformId,
        experiment_id: String,
        result: ExperimentResult,
        #[serde(with = "hash_hex")]
        prev_hash: [u8; 32],
        #[serde(with = "hash_hex")]
        hash: [u8; 32],
    },
    /// A signed Apex Lift Engine record: the canonical JSON payload plus its
    /// SHA-256, so a publisher can verify the exact bytes without trusting
    /// this ledger. Chained and signed like every other record.
    LiftRecord {
        seq: u64,
        at_unix_ms: i64,
        payload_sha256: String,
        payload: String,
        #[serde(with = "hash_hex")]
        prev_hash: [u8; 32],
        #[serde(with = "hash_hex")]
        hash: [u8; 32],
    },
    /// A previously applied change was rolled back.
    RollbackPerformed {
        seq: u64,
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: String,
        rolled_back_seq: u64,
        reason: String,
        #[serde(with = "hash_hex")]
        prev_hash: [u8; 32],
        #[serde(with = "hash_hex")]
        hash: [u8; 32],
    },
    /// A kept change whose platform requires the publisher to promote it
    /// manually (LevelPlay ends its A/B test only from the dashboard). No
    /// fee may be built until a later observe confirms the change now serves
    /// everyone; `deadline_unix_ms` bounds the wait.
    PromotionPending {
        seq: u64,
        at_unix_ms: i64,
        platform: PlatformId,
        experiment_id: String,
        ad_unit_id: String,
        change_seq: u64,
        deadline_unix_ms: i64,
        instructions: String,
        #[serde(with = "hash_hex")]
        prev_hash: [u8; 32],
        #[serde(with = "hash_hex")]
        hash: [u8; 32],
    },
    /// A billing proof for platform-proven lift (the billing entry of the
    /// ledger; the fee arithmetic is pinned in [`crate::build_billing_proof`]).
    BillingProof {
        seq: u64,
        at_unix_ms: i64,
        publisher_id: String,
        experiment_id: String,
        proven_lift: i64,
        rate_bp: u16,
        fee_micros: i64,
        #[serde(with = "hash_hex")]
        prev_hash: [u8; 32],
        #[serde(with = "hash_hex")]
        hash: [u8; 32],
    },
}

/// Unsealed chain envelope: genesis linkage with no hash yet.
const UNSEALED: [u8; 32] = [0u8; 32];

impl LedgerRecord {
    /// Build an unsealed `changeApplied` record (sealed by the ledger on append).
    pub fn change_applied(
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: &str,
        change: ChangeKind,
        idempotency_key: &str,
        prev_value: &str,
    ) -> Self {
        LedgerRecord::ChangeApplied {
            seq: 0,
            at_unix_ms,
            platform,
            ad_unit_id: ad_unit_id.to_string(),
            change,
            idempotency_key: idempotency_key.to_string(),
            prev_value: prev_value.to_string(),
            prev_hash: UNSEALED,
            hash: UNSEALED,
        }
    }

    /// Build an unsealed `experimentStarted` record.
    pub fn experiment_started(
        at_unix_ms: i64,
        platform: PlatformId,
        experiment_id: &str,
        spec: ExperimentSpec,
    ) -> Self {
        LedgerRecord::ExperimentStarted {
            seq: 0,
            at_unix_ms,
            platform,
            experiment_id: experiment_id.to_string(),
            spec,
            prev_hash: UNSEALED,
            hash: UNSEALED,
        }
    }

    /// Build an unsealed `experimentObserved` record.
    pub fn experiment_observed(
        at_unix_ms: i64,
        platform: PlatformId,
        experiment_id: &str,
        result: ExperimentResult,
    ) -> Self {
        LedgerRecord::ExperimentObserved {
            seq: 0,
            at_unix_ms,
            platform,
            experiment_id: experiment_id.to_string(),
            result,
            prev_hash: UNSEALED,
            hash: UNSEALED,
        }
    }

    /// Build an unsealed `liftRecord`: the signed payload is the engine's
    /// canonical JSON and the SHA-256 pins it byte for byte.
    pub fn lift_record(at_unix_ms: i64, payload_sha256: &str, payload: &str) -> Self {
        LedgerRecord::LiftRecord {
            seq: 0,
            at_unix_ms,
            payload_sha256: payload_sha256.to_string(),
            payload: payload.to_string(),
            prev_hash: UNSEALED,
            hash: UNSEALED,
        }
    }

    /// Build an unsealed `promotionPending` record.
    pub fn promotion_pending(
        at_unix_ms: i64,
        platform: PlatformId,
        experiment_id: &str,
        ad_unit_id: &str,
        change_seq: u64,
        deadline_unix_ms: i64,
        instructions: &str,
    ) -> Self {
        LedgerRecord::PromotionPending {
            seq: 0,
            at_unix_ms,
            platform,
            experiment_id: experiment_id.to_string(),
            ad_unit_id: ad_unit_id.to_string(),
            change_seq,
            deadline_unix_ms,
            instructions: instructions.to_string(),
            prev_hash: UNSEALED,
            hash: UNSEALED,
        }
    }

    /// Build an unsealed `rollbackPerformed` record. The ledger's
    /// `append_rollback` enforces the rollback rules (known target, no
    /// rollback-of-rollback, no double rollback).
    pub fn rollback_performed(
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: &str,
        rolled_back_seq: u64,
        reason: &str,
    ) -> Self {
        LedgerRecord::RollbackPerformed {
            seq: 0,
            at_unix_ms,
            platform,
            ad_unit_id: ad_unit_id.to_string(),
            rolled_back_seq,
            reason: reason.to_string(),
            prev_hash: UNSEALED,
            hash: UNSEALED,
        }
    }

    /// Build an unsealed `billingProof` record from already-computed values.
    /// Prefer [`crate::build_billing_proof`], which performs the arithmetic
    /// and the refusal checks.
    pub fn billing_proof(
        at_unix_ms: i64,
        publisher_id: &str,
        experiment_id: &str,
        proven_lift: i64,
        rate_bp: u16,
        fee_micros: i64,
    ) -> Self {
        LedgerRecord::BillingProof {
            seq: 0,
            at_unix_ms,
            publisher_id: publisher_id.to_string(),
            experiment_id: experiment_id.to_string(),
            proven_lift,
            rate_bp,
            fee_micros,
            prev_hash: UNSEALED,
            hash: UNSEALED,
        }
    }

    /// Chain sequence number (0-based; the genesis record is 0).
    pub fn seq(&self) -> u64 {
        match self {
            LedgerRecord::ChangeApplied { seq, .. }
            | LedgerRecord::ExperimentStarted { seq, .. }
            | LedgerRecord::ExperimentObserved { seq, .. }
            | LedgerRecord::RollbackPerformed { seq, .. }
            | LedgerRecord::BillingProof { seq, .. }
            | LedgerRecord::LiftRecord { seq, .. }
            | LedgerRecord::PromotionPending { seq, .. } => *seq,
        }
    }

    /// Timestamp carried in the record.
    pub fn at_unix_ms(&self) -> i64 {
        match self {
            LedgerRecord::ChangeApplied { at_unix_ms, .. }
            | LedgerRecord::ExperimentStarted { at_unix_ms, .. }
            | LedgerRecord::ExperimentObserved { at_unix_ms, .. }
            | LedgerRecord::RollbackPerformed { at_unix_ms, .. }
            | LedgerRecord::BillingProof { at_unix_ms, .. }
            | LedgerRecord::LiftRecord { at_unix_ms, .. }
            | LedgerRecord::PromotionPending { at_unix_ms, .. } => *at_unix_ms,
        }
    }

    /// Platform this record belongs to (`billingProof` has none; billing is
    /// platform-agnostic and returns [`PlatformId::Admob`] only for the
    /// envelope check, which never inspects it for billing records).
    pub fn platform(&self) -> Option<PlatformId> {
        match self {
            LedgerRecord::ChangeApplied { platform, .. }
            | LedgerRecord::ExperimentStarted { platform, .. }
            | LedgerRecord::ExperimentObserved { platform, .. }
            | LedgerRecord::RollbackPerformed { platform, .. }
            | LedgerRecord::PromotionPending { platform, .. } => Some(*platform),
            LedgerRecord::BillingProof { .. } | LedgerRecord::LiftRecord { .. } => None,
        }
    }

    /// Hash of the previous record (zeros for genesis).
    pub fn prev_hash(&self) -> [u8; 32] {
        match self {
            LedgerRecord::ChangeApplied { prev_hash, .. }
            | LedgerRecord::ExperimentStarted { prev_hash, .. }
            | LedgerRecord::ExperimentObserved { prev_hash, .. }
            | LedgerRecord::RollbackPerformed { prev_hash, .. }
            | LedgerRecord::BillingProof { prev_hash, .. }
            | LedgerRecord::LiftRecord { prev_hash, .. }
            | LedgerRecord::PromotionPending { prev_hash, .. } => *prev_hash,
        }
    }

    /// This record's chain hash.
    pub fn hash(&self) -> [u8; 32] {
        match self {
            LedgerRecord::ChangeApplied { hash, .. }
            | LedgerRecord::ExperimentStarted { hash, .. }
            | LedgerRecord::ExperimentObserved { hash, .. }
            | LedgerRecord::RollbackPerformed { hash, .. }
            | LedgerRecord::BillingProof { hash, .. }
            | LedgerRecord::LiftRecord { hash, .. }
            | LedgerRecord::PromotionPending { hash, .. } => *hash,
        }
    }

    fn set_seq(&mut self, seq: u64) {
        match self {
            LedgerRecord::ChangeApplied { seq: s, .. }
            | LedgerRecord::ExperimentStarted { seq: s, .. }
            | LedgerRecord::ExperimentObserved { seq: s, .. }
            | LedgerRecord::RollbackPerformed { seq: s, .. }
            | LedgerRecord::BillingProof { seq: s, .. }
            | LedgerRecord::LiftRecord { seq: s, .. }
            | LedgerRecord::PromotionPending { seq: s, .. } => *s = seq,
        }
    }

    fn set_prev_hash(&mut self, prev_hash: [u8; 32]) {
        match self {
            LedgerRecord::ChangeApplied { prev_hash: p, .. }
            | LedgerRecord::ExperimentStarted { prev_hash: p, .. }
            | LedgerRecord::ExperimentObserved { prev_hash: p, .. }
            | LedgerRecord::RollbackPerformed { prev_hash: p, .. }
            | LedgerRecord::BillingProof { prev_hash: p, .. }
            | LedgerRecord::LiftRecord { prev_hash: p, .. }
            | LedgerRecord::PromotionPending { prev_hash: p, .. } => *p = prev_hash,
        }
    }

    fn set_hash(&mut self, hash: [u8; 32]) {
        match self {
            LedgerRecord::ChangeApplied { hash: h, .. }
            | LedgerRecord::ExperimentStarted { hash: h, .. }
            | LedgerRecord::ExperimentObserved { hash: h, .. }
            | LedgerRecord::RollbackPerformed { hash: h, .. }
            | LedgerRecord::BillingProof { hash: h, .. }
            | LedgerRecord::LiftRecord { hash: h, .. }
            | LedgerRecord::PromotionPending { hash: h, .. } => *h = hash,
        }
    }

    /// Seal this record into the chain: assign `seq`, link `prev_hash` and
    /// compute `hash` over the canonical JSON without the `hash` field.
    /// Ledgers call this on every append; it is not part of the public API.
    pub(crate) fn seal(&mut self, seq: u64, prev_hash: [u8; 32]) -> Result<(), crate::LedgerError> {
        self.set_seq(seq);
        self.set_prev_hash(prev_hash);
        self.set_hash(UNSEALED);
        let digest = crate::chain::compute_hash(self)?;
        self.set_hash(digest);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use apex_core::platform::ExperimentMetric;
    use apex_core::{ExperimentState, ExperimentVariantResult};

    use super::*;
    use crate::chain;
    use crate::testutil::seal_record;

    fn spec() -> ExperimentSpec {
        ExperimentSpec {
            name: "floors-ee".into(),
            instance_id: String::new(),
            ad_unit_id: "u1".into(),
            metric: ExperimentMetric::Ecpm,
            variants: 2,
            test_group_allocation: 50,
            change_description: "raise EE floor to 2.50".into(),
        }
    }

    fn result() -> ExperimentResult {
        ExperimentResult {
            experiment_id: "exp-1".into(),
            state: ExperimentState::Completed,
            results: vec![ExperimentVariantResult {
                variant_id: "control".into(),
                impressions: 10_000,
                revenue: 10_000_000,
                ecpm: 1_000_000,
            }],
            lift_percent_bp: Some(2_500),
            confidence: Some(9_800),
            winner_variant_id: Some("variant-a".into()),
        }
    }

    /// One sealed record per variant, covering every field of every variant.
    fn all_variants() -> Vec<LedgerRecord> {
        let records = vec![
            LedgerRecord::change_applied(
                1_700_000_000_000,
                PlatformId::AppLovinMax,
                "u1",
                ChangeKind::CountryFloorSet {
                    country: "EE".into(),
                    floor_ecpm: 2_500_000,
                },
                "max-experiment:floors-ee:u1",
                "prev-value",
            ),
            LedgerRecord::change_applied(
                1_700_000_001_000,
                PlatformId::Admob,
                "u2",
                ChangeKind::EcpmSet {
                    mediation_group_id: "g1".into(),
                    ad_unit_id: "u2".into(),
                    ecpm: 1_750_000,
                },
                "admob-ecpm:g1:u2:1.75",
                "prev-value",
            ),
            LedgerRecord::change_applied(
                1_700_000_002_000,
                PlatformId::LevelPlay,
                "u3",
                ChangeKind::WaterfallEntryPriceSet {
                    ad_unit_id: "u3".into(),
                    entry_id: "e1".into(),
                    price_ecpm: 2_350_000,
                },
                "levelplay-price:u3:e1",
                "prev-value",
            ),
            LedgerRecord::change_applied(
                1_700_000_003_000,
                PlatformId::LevelPlay,
                "u3",
                ChangeKind::WaterfallEntryCreated {
                    ad_unit_id: "u3".into(),
                    network: "mintegral".into(),
                    instance_name: "MTG_1".into(),
                    price_ecpm: 2_000_000,
                },
                "levelplay-create:u3:mintegral:MTG_1",
                "prev-value",
            ),
            LedgerRecord::change_applied(
                1_700_000_004_000,
                PlatformId::LevelPlay,
                "u3",
                ChangeKind::WaterfallEntryRemoved {
                    ad_unit_id: "u3".into(),
                    entry_id: "e9".into(),
                },
                "levelplay-remove:u3:e9",
                "prev-value",
            ),
            LedgerRecord::experiment_started(
                1_700_000_005_000,
                PlatformId::AppLovinMax,
                "exp-1",
                spec(),
            ),
            LedgerRecord::experiment_observed(
                1_700_000_006_000,
                PlatformId::AppLovinMax,
                "exp-1",
                result(),
            ),
            LedgerRecord::rollback_performed(
                1_700_000_007_000,
                PlatformId::AppLovinMax,
                "u1",
                0,
                "regression on EU traffic",
            ),
            LedgerRecord::billing_proof(
                1_700_000_008_000,
                "pub-1",
                "exp-1",
                2_500_000,
                2_000,
                500_000,
            ),
        ];
        records
            .into_iter()
            .enumerate()
            .map(|(index, record)| seal_record(record, index as u64, [0u8; 32]))
            .collect()
    }

    #[test]
    fn canonical_roundtrip_all_variants() {
        // The canonical form is pinned by byte equality: serialize ->
        // deserialize -> serialize must reproduce the exact same bytes.
        for record in all_variants() {
            let first = serde_json::to_vec(&record).expect("serialize");
            let parsed: LedgerRecord = serde_json::from_slice(&first).expect("deserialize");
            assert_eq!(parsed, record);
            let second = serde_json::to_vec(&parsed).expect("re-serialize");
            assert_eq!(first, second, "canonical bytes changed for {record:?}");
        }
    }

    #[test]
    fn variant_and_field_names_are_camel_case() {
        for record in all_variants() {
            let json = serde_json::to_string(&record).expect("serialize");
            assert!(json.contains("\"type\":"), "missing tag in {json}");
            assert!(json.contains("\"atUnixMs\":"), "missing atUnixMs in {json}");
            assert!(
                json.contains("\"prevHash\":\""),
                "missing prevHash in {json}"
            );
            assert!(!json.contains("at_unix_ms"), "snake_case leaked in {json}");
            assert!(!json.contains("prev_hash"), "snake_case leaked in {json}");
        }
    }

    #[test]
    fn hashes_serialize_as_lowercase_hex() {
        let record = seal_record(
            LedgerRecord::change_applied(
                1_700_000_000_000,
                PlatformId::Admob,
                "u1",
                ChangeKind::EcpmSet {
                    mediation_group_id: "g1".into(),
                    ad_unit_id: "u1".into(),
                    ecpm: 1_750_000,
                },
                "admob-ecpm:g1:u1:1.75",
                "prev-value",
            ),
            0,
            [0u8; 32],
        );
        let json = serde_json::to_string(&record).expect("serialize");
        let hash_hex = hex::encode(record.hash());
        assert_eq!(hash_hex.len(), 64);
        assert_eq!(hash_hex, hash_hex.to_lowercase());
        assert!(json.contains(&format!("\"hash\":\"{hash_hex}\"")), "{json}");
        assert!(
            json.contains(&format!("\"prevHash\":\"{}\"", "00".repeat(32))),
            "{json}"
        );
        let parsed: LedgerRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.hash(), record.hash());
    }

    #[test]
    fn bad_hash_hex_is_rejected() {
        let record = all_variants().remove(0);
        let json = serde_json::to_string(&record).expect("serialize");
        let bad = json.replace(&hex::encode(record.hash()), "zz");
        assert!(serde_json::from_str::<LedgerRecord>(&bad).is_err());
        let short = json.replace(&hex::encode(record.hash()), "abcd");
        assert!(serde_json::from_str::<LedgerRecord>(&short).is_err());
    }

    #[test]
    fn pinned_hash_and_canonical_bytes_for_worked_record() {
        // Pins the canonicalization contract: sorted-key compact JSON with the
        // `hash` field removed, hashed with SHA-256. If this test starts
        // failing, the canonical form changed and must be versioned.
        let record = seal_record(
            LedgerRecord::change_applied(
                1_700_000_000_000,
                PlatformId::Admob,
                "u1",
                ChangeKind::CountryFloorSet {
                    country: "EE".into(),
                    floor_ecpm: 2_500_000,
                },
                "admob-floor:u1:EE",
                "prev-value",
            ),
            0,
            [0u8; 32],
        );
        let canonical =
            String::from_utf8(chain::canonical_bytes(&record).expect("canonical")).expect("utf8");
        assert_eq!(
            canonical,
            concat!(
                r#"{"adUnitId":"u1","atUnixMs":1700000000000,"#,
                r#""change":{"country":"EE","floorEcpm":2500000,"type":"countryFloorSet"},"#,
                r#""idempotencyKey":"admob-floor:u1:EE","platform":"admob","#,
                r#""prevHash":"0000000000000000000000000000000000000000000000000000000000000000","#,
                r#""prevValue":"prev-value","seq":0,"#,
                r#""type":"changeApplied"}"#
            )
        );
        assert_eq!(
            hex::encode(record.hash()),
            "dd9ee5327fcdc776f5400d37db60bce945594feb84778b4c30bba3cd70109ca2"
        );
    }
}
