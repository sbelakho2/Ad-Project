//! AppLovin MAX management client (pubkey auth via `X-API-Key`).

use std::sync::Arc;

use apex_core::endpoint::HttpMethod as CoreMethod;
use apex_core::{
    max_experiment_key, normalize_country, validate_floor, AdFormat, AdUnit, CountryFloor,
    EndpointMeta, ExperimentMetric, ExperimentResult, ExperimentSpec, ExperimentState,
    ExperimentVariantResult, PlatformError, PlatformId, SourceVerification,
};

use crate::transport::{HttpMethod, PlatformCaller, Transport};
use crate::{fraction_to_basis_points, percent_to_basis_points};

pub const MAX_ENDPOINTS: [EndpointMeta; 5] = [
    EndpointMeta::new(
        "listAdUnits",
        CoreMethod::Get,
        SourceVerification::Documented,
        true,
    ),
    EndpointMeta::new(
        "setCountryFloor",
        CoreMethod::Put,
        SourceVerification::Documented,
        false,
    ),
    EndpointMeta::new(
        "createExperiment",
        CoreMethod::Post,
        SourceVerification::Documented,
        false,
    ),
    EndpointMeta::new(
        "getExperiment",
        CoreMethod::Get,
        SourceVerification::Documented,
        true,
    ),
    EndpointMeta::new(
        "getAdUnit",
        CoreMethod::Get,
        SourceVerification::Documented,
        false,
    ),
];

pub struct MaxClient {
    caller: PlatformCaller,
}

impl std::fmt::Debug for MaxClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaxClient")
            .field("platform", &self.caller.platform)
            .finish()
    }
}

impl MaxClient {
    pub fn new(
        api_key: &str,
        account_id: &str,
        base_url: &str,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, PlatformError> {
        if api_key.trim().is_empty() || account_id.trim().is_empty() {
            return Err(PlatformError::Validation(
                "MAX credentials require apiKey and accountId".into(),
            ));
        }
        Ok(Self {
            caller: PlatformCaller::new(
                PlatformId::AppLovinMax,
                base_url.to_string(),
                transport,
                ("X-API-Key".into(), api_key.to_string()),
            ),
        })
    }

    fn schema(&self, operation: &str, detail: impl Into<String>) -> PlatformError {
        PlatformError::Schema {
            platform: PlatformId::AppLovinMax,
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    pub async fn list_ad_units(&self) -> Result<Vec<AdUnit>, PlatformError> {
        let data = self
            .caller
            .call(
                "listAdUnits",
                HttpMethod::Get,
                "/v1/max/ad_units",
                None,
                None,
            )
            .await?;
        let units = data
            .get("ad_units")
            .and_then(|v| v.as_array())
            .ok_or_else(|| self.schema("listAdUnits", "response missing array 'ad_units'"))?;
        let mut out = Vec::with_capacity(units.len());
        for item in units {
            let id = item
                .get("ad_unit_id")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| self.schema("listAdUnits", format!("ad unit missing id: {item}")))?;
            let name = item.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
                self.schema("listAdUnits", format!("ad unit missing name: {item}"))
            })?;
            let format_raw = item.get("format").and_then(|v| v.as_str()).ok_or_else(|| {
                self.schema("listAdUnits", format!("ad unit missing format: {item}"))
            })?;
            let format = parse_ad_format(format_raw).ok_or_else(|| {
                self.schema("listAdUnits", format!("unknown format '{format_raw}'"))
            })?;
            out.push(AdUnit {
                platform_unit_id: id.to_string(),
                name: name.to_string(),
                format,
                enabled: item
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                country_floors: None,
            });
        }
        Ok(out)
    }

    pub async fn set_country_floor(
        &self,
        ad_unit_id: &str,
        country: &str,
        floor_ecpm: i64,
    ) -> Result<CountryFloor, PlatformError> {
        let normalized = normalize_country(country)?;
        let floor = validate_floor(floor_ecpm)?;
        let path = format!(
            "/v1/max/ad_units/{}/floors/{}",
            crate::encode_query_component(ad_unit_id),
            normalized
        );
        self.caller
            .call(
                "setCountryFloor",
                HttpMethod::Put,
                &path,
                Some(serde_json::json!({ "floor_ecpm": floor })),
                None,
            )
            .await?;
        Ok(CountryFloor {
            country: normalized,
            floor_ecpm: floor,
        })
    }

    pub async fn create_experiment(&self, spec: &ExperimentSpec) -> Result<String, PlatformError> {
        if !(2..=4).contains(&spec.variants) {
            return Err(self.schema(
                "createExperiment",
                format!("variants must be 2..=4, got {}", spec.variants),
            ));
        }
        if spec.change_description.trim().is_empty() {
            return Err(self.schema(
                "createExperiment",
                "changeDescription is required (auditable single change)",
            ));
        }
        let body = serde_json::json!({
            "name": spec.name,
            "ad_unit_id": spec.ad_unit_id,
            "metric": match spec.metric {
                ExperimentMetric::Ecpm => "ecpm",
                ExperimentMetric::Revenue => "revenue",
                ExperimentMetric::Arpdau => "arpdau",
            },
            "variant_count": spec.variants,
            "change_description": spec.change_description,
        });
        let key = max_experiment_key(&spec.name, &spec.ad_unit_id);
        let data = self
            .caller
            .call(
                "createExperiment",
                HttpMethod::Post,
                "/v1/max/experiments",
                Some(body),
                Some(&key),
            )
            .await?;
        let experiment_id = data
            .get("experiment_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| self.schema("createExperiment", "response missing experiment_id"))?;
        Ok(experiment_id.to_string())
    }

    pub async fn get_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<ExperimentResult, PlatformError> {
        let path = format!(
            "/v1/max/experiments/{}",
            crate::encode_query_component(experiment_id)
        );
        let data = self
            .caller
            .call("getExperiment", HttpMethod::Get, &path, None, None)
            .await?;
        let state_raw = data
            .get("state")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.schema("getExperiment", "response missing state"))?;
        let state = match state_raw {
            "completed" => ExperimentState::Completed,
            "stopped" => ExperimentState::Stopped,
            _ => ExperimentState::Running, // unknown states never count as finished
        };
        let variants = data
            .get("variants")
            .and_then(|v| v.as_array())
            .ok_or_else(|| self.schema("getExperiment", "response missing variants"))?;
        let mut results = Vec::with_capacity(variants.len());
        for v in variants {
            let variant_id = v
                .get("variant_id")
                .and_then(|x| x.as_str())
                .ok_or_else(|| {
                    self.schema("getExperiment", format!("variant missing variant_id: {v}"))
                })?;
            let impressions = v
                .get("impressions")
                .and_then(|x| x.as_u64())
                .ok_or_else(|| {
                    self.schema("getExperiment", format!("variant missing impressions: {v}"))
                })?;
            let revenue = v.get("revenue").and_then(number_to_micros).ok_or_else(|| {
                self.schema("getExperiment", format!("variant missing revenue: {v}"))
            })?;
            let ecpm = apex_core::ecpm_micros(revenue, impressions).unwrap_or(0);
            results.push(ExperimentVariantResult {
                variant_id: variant_id.to_string(),
                impressions,
                revenue,
                ecpm,
            });
        }
        let lift_percent_bp = data
            .get("lift_percent")
            .and_then(number_to_string)
            .and_then(|s| percent_to_basis_points(&s));
        let confidence = data
            .get("confidence")
            .and_then(number_to_string)
            .and_then(|s| fraction_to_basis_points(&s))
            .and_then(|bp| i32::try_from(bp).ok());
        let winner_variant_id = data
            .get("winner_variant_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(ExperimentResult {
            experiment_id: experiment_id.to_string(),
            state,
            results,
            lift_percent_bp,
            confidence,
            winner_variant_id,
        })
    }
}

fn parse_ad_format(raw: &str) -> Option<AdFormat> {
    match raw {
        "banner" => Some(AdFormat::Banner),
        "interstitial" => Some(AdFormat::Interstitial),
        "rewarded" => Some(AdFormat::Rewarded),
        "native" => Some(AdFormat::Native),
        "app_open" => Some(AdFormat::AppOpen),
        _ => None,
    }
}

/// JSON number (integer or decimal) -> micros via exact decimal parsing.
pub(crate) fn number_to_micros(v: &serde_json::Value) -> Option<i64> {
    number_to_string(v).and_then(|s| crate::parse_decimal_micros(&s).ok())
}

pub(crate) fn number_to_string(v: &serde_json::Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string()); // some platform fields are strings
    }
    let n = v.as_number()?;
    if let Some(i) = n.as_i64() {
        return Some(i.to_string());
    }
    Some(n.to_string()) // serde_json renders the exact decimal text as received
}
