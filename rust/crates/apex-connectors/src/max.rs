//! AppLovin MAX clients against the published APIs.
//!
//! Management: `https://o.applovin.com/mediation/v1`, header `Api-Key`
//! (https://developers.applovin.com/en/max/advanced-features/ad-unit-management-api).
//! The real write shape is a full-resource POST on `/ad_unit/{id}` whose
//! `bid_floors` list replaces the whole list, so callers must read the unit
//! first (the autopilot does, and stores the previous floors for rollback).
//!
//! Experiments: `/ad_unit_experiment/{ad-unit-id}` create/read/promote/
//! deprecate. The API returns configuration only, never lift or confidence:
//! results come from the Revenue Reporting API grouped by `max_ad_unit_test`
//! (https://developers.applovin.com/en/max/reporting-apis/revenue-reporting-api).
//!
//! Reporting: `https://r.applovin.com/maxReport` with the separate Report Key
//! as the `api_key` query parameter. Every number is a string in the JSON.

use std::sync::Arc;

use apex_core::endpoint::HttpMethod as CoreMethod;
use apex_core::{
    AdFormat, AdUnit, CountryFloor, EndpointMeta, ExperimentResult, ExperimentSpec,
    ExperimentState, PlatformError, PlatformId, ReportRow, SourceVerification,
};

use crate::parse_decimal_micros;
use crate::transport::{HttpMethod, PlatformCaller, Transport};

/// Management API base from the public documentation.
pub const MAX_MANAGEMENT_BASE: &str = "https://o.applovin.com/mediation/v1";
/// Reporting API base from the public documentation.
pub const MAX_REPORTING_BASE: &str = "https://r.applovin.com";

/// All management writes are doc-matched but not yet confirmed against a live
/// sandbox, so they stay `VerifyLive`: the autopilot refuses writes until the
/// operator adds the endpoint to the verified list.
pub const MAX_ENDPOINTS: [EndpointMeta; 6] = [
    EndpointMeta::new(
        "listAdUnits",
        CoreMethod::Get,
        SourceVerification::Documented,
        true,
    ),
    EndpointMeta::new(
        "getAdUnit",
        CoreMethod::Get,
        SourceVerification::Documented,
        true,
    ),
    EndpointMeta::new(
        "updateFloors",
        CoreMethod::Post,
        SourceVerification::VerifyLive,
        false,
    ),
    EndpointMeta::new(
        "createExperiment",
        CoreMethod::Post,
        SourceVerification::VerifyLive,
        false,
    ),
    EndpointMeta::new(
        "getExperiment",
        CoreMethod::Get,
        SourceVerification::Documented,
        true,
    ),
    EndpointMeta::new(
        "promoteOrDeprecateExperiment",
        CoreMethod::Post,
        SourceVerification::VerifyLive,
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
        base_url: &str,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, PlatformError> {
        if api_key.trim().is_empty() {
            return Err(PlatformError::Validation(
                "MAX credentials require the management Api-Key".into(),
            ));
        }
        Ok(Self {
            caller: PlatformCaller::new(
                PlatformId::AppLovinMax,
                base_url.to_string(),
                transport,
                ("Api-Key".into(), api_key.to_string()),
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

    /// List ad units. The response is a top-level array; `bid_floors` only
    /// arrive when `with_floors` is requested via the `fields` parameter.
    pub async fn list_ad_units(&self, with_floors: bool) -> Result<Vec<AdUnit>, PlatformError> {
        let path = if with_floors {
            "/ad_units?fields=bid_floors".to_string()
        } else {
            "/ad_units".to_string()
        };
        let data = self
            .caller
            .call("listAdUnits", HttpMethod::Get, &path, None, None)
            .await?;
        let units = data.as_array().ok_or_else(|| {
            self.schema(
                "listAdUnits",
                "response is not a top-level array of ad units",
            )
        })?;
        units.iter().map(|item| self.parse_ad_unit(item)).collect()
    }

    /// Read one ad unit, including its floors.
    pub async fn get_ad_unit(&self, ad_unit_id: &str) -> Result<AdUnit, PlatformError> {
        let path = format!("/ad_unit/{}", encode_id(ad_unit_id));
        let path = format!("{path}?fields=bid_floors");
        let data = self
            .caller
            .call("getAdUnit", HttpMethod::Get, &path, None, None)
            .await?;
        self.parse_ad_unit(&data)
    }

    /// Replace the whole `bid_floors` list of one ad unit. The body must carry
    /// the unit's own identifying fields; build it from the unit read back by
    /// [`get_ad_unit`](Self::get_ad_unit) right before the write.
    pub async fn update_floors(
        &self,
        unit: &AdUnit,
        floors: &[CountryFloor],
    ) -> Result<AdUnit, PlatformError> {
        let platform = unit.platform.as_deref().ok_or_else(|| {
            self.schema(
                "updateFloors",
                "ad unit has no platform; read it back with getAdUnit first",
            )
        })?;
        let package_name = unit.package_name.as_deref().ok_or_else(|| {
            self.schema(
                "updateFloors",
                "ad unit has no package_name; read it back with getAdUnit first",
            )
        })?;
        // Rebuild the platform's own grouped shape: one group per distinct
        // group name, with the country list of that group.
        let mut groups: Vec<serde_json::Value> = Vec::new();
        {
            let mut seen: Vec<String> = Vec::new();
            for floor in floors {
                let key = floor.group.clone().unwrap_or_else(|| "default".to_string());
                if let Some(pos) = seen.iter().position(|name| *name == key) {
                    let countries = groups[pos]["countries"]["values"]
                        .as_array_mut()
                        .ok_or_else(|| PlatformError::Schema {
                            platform: PlatformId::AppLovinMax,
                            operation: "updateFloors".into(),
                            detail: "internal floor group lost its countries list".into(),
                        })?;
                    if !countries
                        .iter()
                        .any(|v| v.as_str() == Some(floor.country.as_str()))
                    {
                        countries.push(serde_json::Value::String(floor.country.clone()));
                    }
                    continue;
                }
                seen.push(key.clone());
                groups.push(serde_json::json!({
                    "country_group_name": key,
                    "cpm": format_ecpm(floor.floor_ecpm),
                    "countries": {"type": "INCLUDE", "values": [floor.country.clone()]}
                }));
            }
        }
        let body = serde_json::json!({
            "id": unit.platform_unit_id,
            "name": unit.name,
            "platform": platform,
            "ad_format": max_ad_format(unit.format)?,
            "package_name": package_name,
            "bid_floors": groups,
        });
        let path = format!("/ad_unit/{}", encode_id(&unit.platform_unit_id));
        let data = self
            .caller
            .call("updateFloors", HttpMethod::Post, &path, Some(body), None)
            .await?;
        self.parse_ad_unit(&data)
    }

    /// Create an experiment on one ad unit. Only the documented allocations
    /// (5, 10, 25, 50) are accepted, and a description is required so the
    /// change stays auditable.
    pub async fn create_experiment(
        &self,
        spec: &ExperimentSpec,
        floors: Option<&[CountryFloor]>,
    ) -> Result<String, PlatformError> {
        if !matches!(spec.test_group_allocation, 5 | 10 | 25 | 50) {
            return Err(PlatformError::Validation(format!(
                "MAX test_group_allocation must be 5, 10, 25 or 50, got {}",
                spec.test_group_allocation
            )));
        }
        if spec.change_description.trim().is_empty() {
            return Err(PlatformError::Validation(
                "MAX createExperiment: changeDescription is required (auditable single change)"
                    .into(),
            ));
        }
        let mut body = serde_json::json!({
            "experiment_name": spec.name,
            "test_group_allocation": spec.test_group_allocation,
        });
        if let Some(floors) = floors {
            let groups = floor_groups(floors)?;
            body["bid_floors"] = serde_json::Value::Array(groups);
        }
        let path = format!("/ad_unit_experiment/{}", encode_id(&spec.ad_unit_id));
        let data = self
            .caller
            .call(
                "createExperiment",
                HttpMethod::Post,
                &path,
                Some(body),
                None,
            )
            .await?;
        // The experiment's id equals the parent ad unit id in this API.
        let id = data
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.schema("createExperiment", "response missing ad unit id"))?;
        Ok(id.to_string())
    }

    /// Read the experiment configuration of one ad unit. The API reports no
    /// state, lift or confidence; `promote`/`deprecate` flags map to the
    /// internal state, and the results stay empty for the reporting client to
    /// fill.
    pub async fn get_experiment(
        &self,
        ad_unit_id: &str,
    ) -> Result<ExperimentResult, PlatformError> {
        let path = format!("/ad_unit_experiment/{}", encode_id(ad_unit_id));
        let data = self
            .caller
            .call(
                "getExperiment",
                HttpMethod::Get,
                &format!("{path}?fields=bid_floors"),
                None,
                None,
            )
            .await?;
        let promote = data
            .get("promote")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let deprecate = data
            .get("deprecate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let state = if promote {
            ExperimentState::Completed
        } else if deprecate {
            ExperimentState::Stopped
        } else {
            ExperimentState::Running
        };
        Ok(ExperimentResult {
            experiment_id: ad_unit_id.to_string(),
            state,
            results: Vec::new(),
            lift_percent_bp: None,
            confidence: None,
            winner_variant_id: None,
        })
    }

    /// Apply the experiment configuration to all users and end the test.
    pub async fn promote_experiment(&self, ad_unit_id: &str) -> Result<(), PlatformError> {
        self.promote_or_deprecate(ad_unit_id, true).await
    }

    /// Revert to the control configuration and end the test.
    pub async fn deprecate_experiment(&self, ad_unit_id: &str) -> Result<(), PlatformError> {
        self.promote_or_deprecate(ad_unit_id, false).await
    }

    async fn promote_or_deprecate(
        &self,
        ad_unit_id: &str,
        promote: bool,
    ) -> Result<(), PlatformError> {
        let config = self
            .caller
            .call(
                "getExperiment",
                HttpMethod::Get,
                &format!("/ad_unit_experiment/{}", encode_id(ad_unit_id)),
                None,
                None,
            )
            .await?;
        let name = config
            .get("experiment_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                self.schema(
                    "promoteOrDeprecateExperiment",
                    "response missing experiment_name",
                )
            })?;
        let body = if promote {
            serde_json::json!({
                "id": ad_unit_id,
                "experiment_name": name,
                "promote": true,
                "deprecate": false,
            })
        } else {
            serde_json::json!({
                "id": ad_unit_id,
                "experiment_name": name,
                "promote": false,
                "deprecate": true,
            })
        };
        self.caller
            .call(
                "promoteOrDeprecateExperiment",
                HttpMethod::Post,
                &format!("/ad_unit_experiment/{}", encode_id(ad_unit_id)),
                Some(body),
                None,
            )
            .await?;
        Ok(())
    }

    fn parse_ad_unit(&self, item: &serde_json::Value) -> Result<AdUnit, PlatformError> {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.schema("parseAdUnit", format!("ad unit missing id: {item}")))?;
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.schema("parseAdUnit", format!("ad unit missing name: {item}")))?;
        let format_raw = item
            .get("ad_format")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                self.schema("parseAdUnit", format!("ad unit missing ad_format: {item}"))
            })?;
        let format = parse_ad_format(format_raw).ok_or_else(|| {
            self.schema("parseAdUnit", format!("unknown ad_format '{format_raw}'"))
        })?;
        let disabled = item
            .get("disabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let floors = match item.get("bid_floors") {
            None => None,
            Some(serde_json::Value::Null) => None,
            Some(list) => {
                let groups = list
                    .as_array()
                    .ok_or_else(|| self.schema("parseAdUnit", "bid_floors is not an array"))?;
                let mut out = Vec::new();
                for group in groups {
                    let group_name = group
                        .get("country_group_name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            self.schema("parseAdUnit", "floor group missing country_group_name")
                        })?;
                    let cpm = group
                        .get("cpm")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| self.schema("parseAdUnit", "floor group missing cpm"))?;
                    let floor_ecpm = parse_decimal_micros(cpm).map_err(|e| {
                        self.schema("parseAdUnit", format!("floor cpm '{cpm}' unparsable: {e}"))
                    })?;
                    let countries = group
                        .get("countries")
                        .and_then(|c| c.get("values"))
                        .and_then(|v| v.as_array())
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect::<Vec<String>>()
                        })
                        .unwrap_or_default();
                    for country in countries {
                        out.push(CountryFloor {
                            country,
                            floor_ecpm,
                            group: Some(group_name.to_string()),
                        });
                    }
                }
                Some(out)
            }
        };
        Ok(AdUnit {
            platform_unit_id: id.to_string(),
            name: name.to_string(),
            format,
            enabled: !disabled,
            country_floors: floors,
            platform: item
                .get("platform")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            package_name: item
                .get("package_name")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    }
}

/// Reporting client. The Report Key is read-only for the reporting API and is
/// sent as the `api_key` query parameter.
pub struct MaxReportClient {
    transport: Arc<dyn Transport>,
    report_key: String,
    base_url: String,
}

impl std::fmt::Debug for MaxReportClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaxReportClient").finish()
    }
}

impl MaxReportClient {
    pub fn new(
        report_key: &str,
        base_url: &str,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, PlatformError> {
        if report_key.trim().is_empty() {
            return Err(PlatformError::Validation(
                "MAX reporting requires the Report Key".into(),
            ));
        }
        Ok(Self {
            transport,
            report_key: report_key.to_string(),
            base_url: base_url.to_string(),
        })
    }

    /// Raw rows for the requested columns. `columns` is the exact column list
    /// from the reporting docs; the standard experiment lift recipe is
    /// `day,max_ad_unit_id,max_ad_unit_test,estimated_revenue,impressions`.
    pub async fn rows(
        &self,
        start: &str,
        end: &str,
        columns: &[&str],
        filter: Option<(&str, &str)>,
    ) -> Result<Vec<ReportRow>, PlatformError> {
        let mut collected: Vec<ReportRow> = Vec::new();
        let mut offset = 0usize;
        loop {
            let mut url = format!(
                "{}/maxReport?api_key={}&columns={}&start={}&end={}&format=json&limit=1000&offset={offset}",
                self.base_url,
                encode_query(&self.report_key),
                encode_query(&columns.join(",")),
                encode_query(start),
                encode_query(end),
            );
            if let Some((column, value)) = filter {
                url.push_str(&format!(
                    "&filter_{}={}",
                    encode_query(column),
                    encode_query(value)
                ));
            }
            let request = crate::transport::HttpRequest {
                platform: PlatformId::AppLovinMax,
                operation: "maxReport".to_string(),
                method: HttpMethod::Get,
                url,
                headers: vec![],
                body: None,
                timeout_ms: 30_000,
            };
            let response = match self.transport.request(request).await {
                Ok(r) => r,
                Err(crate::transport::TransportError::Platform(e)) => return Err(e),
                Err(crate::transport::TransportError::Network(detail)) => {
                    return Err(PlatformError::Network {
                        platform: PlatformId::AppLovinMax,
                        operation: "maxReport".into(),
                        detail,
                    });
                }
                Err(crate::transport::TransportError::Timeout(detail)) => {
                    return Err(PlatformError::Network {
                        platform: PlatformId::AppLovinMax,
                        operation: "maxReport".into(),
                        detail: format!("timeout: {detail}"),
                    });
                }
            };
            if response.status != 200 {
                return Err(PlatformError::Http {
                    platform: PlatformId::AppLovinMax,
                    operation: "maxReport".into(),
                    status: response.status,
                });
            }
            let parsed: serde_json::Value =
                serde_json::from_slice(&response.body).map_err(|e| PlatformError::Schema {
                    platform: PlatformId::AppLovinMax,
                    operation: "maxReport".into(),
                    detail: format!("response is not JSON: {e}"),
                })?;
            let code = parsed.get("code").and_then(|v| v.as_u64()).unwrap_or(200);
            if code != 200 {
                return Err(PlatformError::Http {
                    platform: PlatformId::AppLovinMax,
                    operation: "maxReport".into(),
                    status: u16::try_from(code).unwrap_or(500),
                });
            }
            let count = parsed.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let results = parsed
                .get("results")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for row in &results {
                collected.push(
                    parse_report_row(row).map_err(|detail| PlatformError::Schema {
                        platform: PlatformId::AppLovinMax,
                        operation: "maxReport".into(),
                        detail,
                    })?,
                );
            }
            if collected.len() >= count || results.is_empty() {
                break;
            }
            offset += results.len();
        }
        Ok(collected)
    }

    /// Revenue rows by country and ad unit for the audit floor rules.
    pub async fn country_ad_unit_rows(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<ReportRow>, PlatformError> {
        self.rows(
            start,
            end,
            &[
                "day",
                "country",
                "max_ad_unit_id",
                "estimated_revenue",
                "impressions",
            ],
            None,
        )
        .await
    }

    /// Total reported revenue of one ad unit over `[start, end]`, in micros.
    /// Used by the monthly billing pass to bill the pre-registered scope's
    /// incremental revenue.
    pub async fn ad_unit_revenue(
        &self,
        ad_unit_id: &str,
        start: &str,
        end: &str,
    ) -> Result<i64, PlatformError> {
        let rows = self
            .rows(
                start,
                end,
                &["day", "max_ad_unit_id", "estimated_revenue"],
                None,
            )
            .await?;
        let mut total: i64 = 0;
        for row in rows {
            if row.ad_unit_id.as_deref() == Some(ad_unit_id) {
                total = total
                    .checked_add(row.revenue)
                    .ok_or_else(|| PlatformError::Validation("revenue total overflow".into()))?;
            }
        }
        Ok(total)
    }

    /// Per-arm rows for one experiment: revenue and impressions grouped by the
    /// `max_ad_unit_test` arm label.
    pub async fn experiment_rows(
        &self,
        ad_unit_id: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<ReportRow>, PlatformError> {
        self.rows(
            start,
            end,
            &[
                "day",
                "max_ad_unit_id",
                "max_ad_unit_test",
                "estimated_revenue",
                "impressions",
            ],
            Some(("max_ad_unit_id", ad_unit_id)),
        )
        .await
    }

    /// Platform-wide totals for the window.
    pub async fn totals(&self, start: &str, end: &str) -> Result<(i64, u64), PlatformError> {
        let rows = self
            .rows(start, end, &["estimated_revenue", "impressions"], None)
            .await?;
        let revenue: i64 = rows.iter().map(|r| r.revenue).sum();
        let impressions: u64 = rows.iter().map(|r| r.impressions).sum();
        Ok((revenue, impressions))
    }
}

fn parse_report_row(row: &serde_json::Value) -> Result<ReportRow, String> {
    let day = row
        .get("day")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "report row missing day".to_string())?
        .to_string();
    let revenue = row
        .get("estimated_revenue")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "report row missing estimated_revenue".to_string())?;
    let revenue = parse_decimal_micros(revenue).map_err(|e| format!("revenue '{revenue}': {e}"))?;
    let impressions_raw = row
        .get("impressions")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "report row missing impressions".to_string())?;
    let impressions = impressions_raw
        .parse::<u64>()
        .map_err(|e| format!("impressions '{impressions_raw}': {e}"))?;
    Ok(ReportRow {
        day,
        country: row
            .get("country")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        ad_unit_id: row
            .get("max_ad_unit_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        test_group: row
            .get("max_ad_unit_test")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        revenue,
        impressions,
    })
}

fn floor_groups(floors: &[CountryFloor]) -> Result<Vec<serde_json::Value>, PlatformError> {
    let mut groups: Vec<serde_json::Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for floor in floors {
        let key = floor.group.clone().unwrap_or_else(|| "default".to_string());
        if let Some(pos) = seen.iter().position(|name| *name == key) {
            let countries = groups[pos]["countries"]["values"]
                .as_array_mut()
                .ok_or_else(|| PlatformError::Schema {
                    platform: PlatformId::AppLovinMax,
                    operation: "updateFloors".into(),
                    detail: "internal floor group lost its countries list".into(),
                })?;
            if !countries
                .iter()
                .any(|v| v.as_str() == Some(floor.country.as_str()))
            {
                countries.push(serde_json::Value::String(floor.country.clone()));
            }
            continue;
        }
        seen.push(key.clone());
        groups.push(serde_json::json!({
            "country_group_name": key,
            "cpm": format_ecpm(floor.floor_ecpm),
            "countries": {"type": "INCLUDE", "values": [floor.country.clone()]}
        }));
    }
    Ok(groups)
}

fn format_ecpm(micros: i64) -> String {
    let whole = micros / 1_000_000;
    let frac = (micros.abs() % 1_000_000) as u32;
    if frac == 0 {
        format!("{whole}")
    } else {
        format!("{whole}.{frac:06}")
            .trim_end_matches('0')
            .to_string()
    }
}

fn max_ad_format(format: AdFormat) -> Result<&'static str, PlatformError> {
    match format {
        AdFormat::Interstitial => Ok("INTER"),
        AdFormat::Banner => Ok("BANNER"),
        AdFormat::Rewarded => Ok("REWARD"),
        other => Err(PlatformError::Validation(format!(
            "MAX has no ad_format mapping for {other:?}"
        ))),
    }
}

/// JSON number (integer or decimal string) -> micros via exact decimal parsing.
pub(crate) fn number_to_micros(v: &serde_json::Value) -> Option<i64> {
    number_to_string(v).and_then(|s| crate::parse_decimal_micros(&s).ok())
}

pub(crate) fn number_to_string(v: &serde_json::Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    let n = v.as_number()?;
    if let Some(i) = n.as_i64() {
        return Some(i.to_string());
    }
    Some(n.to_string())
}

fn parse_ad_format(raw: &str) -> Option<AdFormat> {
    match raw {
        "INTER" => Some(AdFormat::Interstitial),
        "BANNER" => Some(AdFormat::Banner),
        "REWARD" => Some(AdFormat::Rewarded),
        "native" => Some(AdFormat::Native),
        _ => None,
    }
}

fn encode_id(id: &str) -> String {
    urlencoding(id)
}

fn encode_query(value: &str) -> String {
    urlencoding(value)
}

/// Percent-encode with the strict set the platforms expect in paths and query
/// values (no `+` for spaces, alphanumerics and `-_.~` left alone).
fn urlencoding(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{HttpRequest, HttpResponse, Transport, TransportError};
    use std::sync::Mutex;

    struct Scripted {
        script: Mutex<std::collections::VecDeque<Result<HttpResponse, TransportError>>>,
        calls: Mutex<Vec<HttpRequest>>,
    }

    impl Scripted {
        fn new(items: Vec<Result<HttpResponse, TransportError>>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(items.into()),
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait::async_trait]
    impl Transport for Scripted {
        async fn request(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.calls
                .lock()
                .map_err(|_| TransportError::Network("poisoned".into()))?
                .push(req);
            self.script
                .lock()
                .map_err(|_| TransportError::Network("poisoned".into()))?
                .pop_front()
                .unwrap_or(Err(TransportError::Network("script exhausted".into())))
        }
    }

    fn ok(body: &str) -> Result<HttpResponse, TransportError> {
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: body.as_bytes().to_vec(),
        })
    }

    fn client(transport: Arc<dyn Transport>) -> MaxClient {
        MaxClient::new("mgmt-key", "https://api.test", transport).expect("client")
    }

    #[tokio::test]
    async fn list_ad_units_parses_the_top_level_array_and_group_floors() {
        let transport = Scripted::new(vec![ok(r#"[
                {"id":"u1","name":"Banner","platform":"ios","package_name":"com.x","ad_format":"BANNER","disabled":false,
                 "bid_floors":[{"country_group_name":"t1","cpm":"10.50","countries":{"type":"INCLUDE","values":["us","ca"]}}]},
                {"id":"u2","name":"Rewarded","platform":"android","package_name":"com.y","ad_format":"REWARD","disabled":true}
            ]"#)]);
        let units = client(transport.clone())
            .list_ad_units(true)
            .await
            .expect("units");
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].platform_unit_id, "u1");
        assert_eq!(units[0].format, AdFormat::Banner);
        assert!(units[0].enabled);
        let floors = units[0].country_floors.as_ref().expect("floors");
        assert_eq!(floors.len(), 2);
        assert_eq!(floors[0].country, "us");
        assert_eq!(floors[0].floor_ecpm, 10_500_000);
        assert_eq!(floors[0].group.as_deref(), Some("t1"));
        assert!(!units[1].enabled);
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].url.contains("/ad_units"));
        assert!(
            calls[0].url.contains("fields=bid_floors"),
            "{}",
            calls[0].url
        );
        assert!(calls[0]
            .headers
            .iter()
            .any(|(k, v)| k == "Api-Key" && v == "mgmt-key"));
    }

    #[tokio::test]
    async fn update_floors_sends_the_full_grouped_list() {
        let transport = Scripted::new(vec![ok(
            r#"{"id":"u1","name":"Banner","platform":"ios","package_name":"com.x","ad_format":"BANNER","bid_floors":[]}"#,
        )]);
        let unit = AdUnit {
            platform_unit_id: "u1".into(),
            name: "Banner".into(),
            format: AdFormat::Banner,
            enabled: true,
            country_floors: None,
            platform: Some("ios".into()),
            package_name: Some("com.x".into()),
        };
        let floors = vec![
            CountryFloor {
                country: "us".into(),
                floor_ecpm: 1_000_000,
                group: Some("t1 eng".into()),
            },
            CountryFloor {
                country: "ca".into(),
                floor_ecpm: 1_000_000,
                group: Some("t1 eng".into()),
            },
        ];
        let client = client(transport.clone());
        let _ = client.update_floors(&unit, &floors).await.expect("write");
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].url, "https://api.test/ad_unit/u1");
        let body = match calls[0].body.as_ref().expect("body") {
            crate::transport::HttpBody::Json(value) => value.clone(),
            other => panic!("unexpected body variant {other:?}"),
        };
        let groups = body["bid_floors"].as_array().expect("groups");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["country_group_name"], "t1 eng");
        assert_eq!(groups[0]["cpm"], "1");
        assert_eq!(
            groups[0]["countries"]["values"],
            serde_json::json!(["us", "ca"])
        );
    }

    #[tokio::test]
    async fn create_experiment_validates_allocation_and_uses_the_real_path() {
        let transport = Scripted::new(vec![ok(r#"{"id":"u1","experiment_name":"test"}"#)]);
        let client = client(transport.clone());
        let spec = ExperimentSpec {
            name: "test".into(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Ecpm,
            variants: 2,
            change_description: "raise t1 floor".into(),
            test_group_allocation: 50,
        };
        let id = client.create_experiment(&spec, None).await.expect("create");
        assert_eq!(id, "u1");
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert!(
            calls[0].url.contains("/ad_unit_experiment/u1"),
            "{}",
            calls[0].url
        );
        let mut bad = spec.clone();
        bad.test_group_allocation = 33;
        assert!(matches!(
            client.create_experiment(&bad, None).await,
            Err(PlatformError::Validation(_))
        ));
    }

    #[tokio::test]
    async fn experiment_state_derives_from_promote_deprecate_flags() {
        for (body, expected) in [
            (
                r#"{"id":"u1","experiment_name":"t","promote":true}"#,
                ExperimentState::Completed,
            ),
            (
                r#"{"id":"u1","experiment_name":"t","deprecate":true}"#,
                ExperimentState::Stopped,
            ),
            (
                r#"{"id":"u1","experiment_name":"t"}"#,
                ExperimentState::Running,
            ),
        ] {
            let transport = Scripted::new(vec![ok(body)]);
            let result = client(transport).get_experiment("u1").await.expect("get");
            assert_eq!(result.state, expected);
            assert!(result.results.is_empty());
            assert!(result.lift_percent_bp.is_none());
        }
    }

    #[tokio::test]
    async fn report_rows_parse_string_numbers_and_paginate() {
        let page1 = r#"{"code":200,"count":3,"results":[
            {"day":"2026-10-01","country":"US","max_ad_unit_id":"u1","max_ad_unit_test":"Control","estimated_revenue":"12.50","impressions":"100"},
            {"day":"2026-10-01","country":"US","max_ad_unit_id":"u1","max_ad_unit_test":"Test","estimated_revenue":"13.25","impressions":"99"}
        ]}"#;
        let page2 = r#"{"code":200,"count":3,"results":[
            {"day":"2026-10-02","country":"US","max_ad_unit_id":"u1","max_ad_unit_test":"Control","estimated_revenue":"1.00","impressions":"10"}
        ]}"#;
        let transport = Scripted::new(vec![ok(page1), ok(page2)]);
        let report = MaxReportClient::new("report-key", "https://r.test", transport.clone())
            .expect("report");
        let rows = report
            .experiment_rows("u1", "2026-10-01", "2026-10-14")
            .await
            .expect("rows");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].revenue, 12_500_000);
        assert_eq!(rows[0].impressions, 100);
        assert_eq!(rows[1].test_group.as_deref(), Some("Test"));
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert!(
            calls[0].url.contains("api_key=report-key"),
            "{}",
            calls[0].url
        );
        assert!(
            calls[0].url.contains("filter_max_ad_unit_id=u1"),
            "{}",
            calls[0].url
        );
        assert!(calls[1].url.contains("offset=2"), "{}", calls[1].url);
    }

    #[test]
    fn ecpm_formatting_keeps_decimal_shape() {
        assert_eq!(format_ecpm(1_000_000), "1");
        assert_eq!(format_ecpm(10_500_000), "10.5");
        assert_eq!(format_ecpm(12_345_678), "12.345678");
    }
}
