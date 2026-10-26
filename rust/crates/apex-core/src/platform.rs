//! Platform domain types. Serde field naming mirrors the TypeScript reference
//! contract (`backend/src/services/platforms/types.ts`) so fixtures port 1:1.

use serde::{Deserialize, Serialize};

use crate::money::Micros;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlatformId {
    #[serde(rename = "applovin_max")]
    AppLovinMax,
    #[serde(rename = "levelplay")]
    LevelPlay,
    #[serde(rename = "admob")]
    Admob,
    /// The billing/payments domain (Mollie), not an ad-network mediation
    /// platform. Billing calls must carry this label: mislabeling them as an
    /// ad platform pollutes the connector metrics and error text.
    #[serde(rename = "billing")]
    Billing,
}

impl PlatformId {
    pub fn as_str(self) -> &'static str {
        match self {
            PlatformId::AppLovinMax => "applovin_max",
            PlatformId::LevelPlay => "levelplay",
            PlatformId::Admob => "admob",
            PlatformId::Billing => "billing",
        }
    }
}

impl std::fmt::Display for PlatformId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdFormat {
    Banner,
    Interstitial,
    Rewarded,
    Native,
    AppOpen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountryFloor {
    pub country: String,
    pub floor_ecpm: i64,
    /// MAX country-group name this floor came from, when the platform models
    /// floors as named groups of countries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdUnit {
    pub platform_unit_id: String,
    pub name: String,
    pub format: AdFormat,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country_floors: Option<Vec<CountryFloor>>,
    /// Platform enum value the unit was created for, e.g. `ios` or `android`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Store package / bundle id, required by write endpoints that replace
    /// the whole resource.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaterfallEntry {
    pub entry_id: String,
    pub ad_unit_id: String,
    pub network: String,
    pub instance_name: String,
    pub position: i32,
    pub price_ecpm: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub ad_unit_id: String,
    pub network: String,
    pub ecpm: i64,
}

pub type MediationMapping = Mapping;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediationGroup {
    pub mediation_group_id: String,
    pub name: String,
    pub format: AdFormat,
    pub mappings: Vec<Mapping>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentMetric {
    Ecpm,
    Revenue,
    Arpdau,
}

fn default_test_allocation() -> u8 {
    50
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentSpec {
    pub name: String,
    pub ad_unit_id: String,
    pub metric: ExperimentMetric,
    pub variants: u8,
    /// Human-auditable description of the single change under test.
    pub change_description: String,
    /// Percentage of users in the test group; the platform only accepts its
    /// documented set (MAX: 5, 10, 25 or 50).
    #[serde(default = "default_test_allocation")]
    pub test_group_allocation: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentState {
    Running,
    Stopped,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentVariantResult {
    pub variant_id: String,
    pub impressions: u64,
    pub revenue: i64,
    pub ecpm: i64,
}

/// One raw reporting row: what the platform's reporting API returns, before
/// any aggregation. Numbers arrive as strings on every platform and are
/// parsed to exact micros here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRow {
    /// `YYYY-MM-DD`, platform local day.
    pub day: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ad_unit_id: Option<String>,
    /// The experiment arm label, e.g. MAX `max_ad_unit_test` values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_group: Option<String>,
    pub revenue: Micros,
    pub impressions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentResult {
    pub experiment_id: String,
    pub state: ExperimentState,
    pub results: Vec<ExperimentVariantResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lift_percent_bp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<i32>, // basis points of confidence, e.g. 9800 = 98.00%
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner_variant_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_ids_name_and_display_consistently() {
        assert_eq!(PlatformId::AppLovinMax.as_str(), "applovin_max");
        assert_eq!(PlatformId::LevelPlay.as_str(), "levelplay");
        assert_eq!(PlatformId::Admob.as_str(), "admob");
        assert_eq!(PlatformId::AppLovinMax.to_string(), "applovin_max");
        assert_eq!(format!("{}", PlatformId::LevelPlay), "levelplay");
    }

    #[test]
    fn a_spec_without_an_allocation_defaults_to_fifty() {
        // The serde default keeps older specs readable.
        let spec: ExperimentSpec = serde_json::from_value(serde_json::json!({
            "name": "n",
            "adUnitId": "u",
            "metric": "revenue",
            "variants": 2,
            "changeDescription": "c"
        }))
        .expect("spec");
        assert_eq!(spec.test_group_allocation, 50);
    }

    #[test]
    fn platform_id_serde_matches_ts_contract() {
        for (id, s) in [
            (PlatformId::AppLovinMax, "\"applovin_max\""),
            (PlatformId::LevelPlay, "\"levelplay\""),
            (PlatformId::Admob, "\"admob\""),
        ] {
            let json = serde_json::to_string(&id).expect("serialize");
            assert_eq!(json, s);
            let back: PlatformId = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, id);
        }
    }

    #[test]
    fn ad_unit_roundtrips_camel_case() {
        let unit = AdUnit {
            platform_unit_id: "u1".into(),
            name: "Banner".into(),
            format: AdFormat::Banner,
            enabled: true,
            platform: None,
            package_name: None,
            country_floors: Some(vec![CountryFloor {
                country: "EE".into(),
                floor_ecpm: 2_500_000,
                group: None,
            }]),
        };
        let json = serde_json::to_string(&unit).expect("serialize");
        assert!(json.contains("\"platformUnitId\":\"u1\""), "got {json}");
        assert!(json.contains("\"floorEcpm\":2500000"), "got {json}");
        let back: AdUnit = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, unit);
    }

    #[test]
    fn optional_fields_are_omitted_when_absent() {
        let unit = AdUnit {
            platform_unit_id: "u".into(),
            name: "n".into(),
            format: AdFormat::Rewarded,
            enabled: false,
            platform: None,
            package_name: None,
            country_floors: None,
        };
        let json = serde_json::to_string(&unit).expect("serialize");
        assert!(!json.contains("countryFloors"));
    }

    #[test]
    fn experiment_state_parses_ts_strings() {
        let s: ExperimentState = serde_json::from_str("\"completed\"").expect("parse");
        assert_eq!(s, ExperimentState::Completed);
        let r: ExperimentState = serde_json::from_str("\"running\"").expect("parse");
        assert_eq!(r, ExperimentState::Running);
    }
}
