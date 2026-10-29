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

/// One page of the management list (`limit`) and the reporting API. The
/// documented reporting maximum is 1000; the management list accepts a
/// `limit`/`offset` pair and can otherwise time out or answer 500 on a large
/// account.
const PAGE_LIMIT: usize = 1000;
/// Hard bound on pagination loops: a server that ignores `offset` must fail
/// typed instead of looping forever.
const MAX_PAGES: usize = 10_000;
/// The documented reporting API request window (inclusive days). Larger
/// requests are split into this-sized windows instead of being refused by the
/// provider.
pub const MAX_REPORT_WINDOW_DAYS: i64 = 45;

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

/// One MAX `frequency_capping_settings` entry. `cap_type` is the documented
/// `session` or `time`; countries are ISO alpha-2 values and must be disjoint
/// across all entries in one experiment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaxFrequencyCap {
    pub cap_type: MaxCapFrequencyType,
    pub countries: Vec<String>,
    pub session_limit: Option<u32>,
    pub day_limit: Option<u32>,
    pub minute_frequency: Option<u32>,
}

/// The two documented MAX frequency-cap styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxCapFrequencyType {
    Session,
    Time,
}

impl MaxCapFrequencyType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Time => "time",
        }
    }
}

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
    ///
    /// The documented endpoint paginates with `limit`/`offset` and warns that
    /// a large account can time out or answer `500` without them. Pagination
    /// is therefore explicit; a server that repeats a page fails typed rather
    /// than duplicating ad units or looping forever.
    pub async fn list_ad_units(&self, with_floors: bool) -> Result<Vec<AdUnit>, PlatformError> {
        let mut units = Vec::new();
        let mut ids = std::collections::HashSet::new();
        let mut offset = 0usize;
        for _ in 0..MAX_PAGES {
            let mut path = format!("/ad_units?limit={PAGE_LIMIT}&offset={offset}");
            if with_floors {
                path.push_str("&fields=bid_floors");
            }
            let data = self
                .caller
                .call("listAdUnits", HttpMethod::Get, &path, None, None)
                .await?;
            let page = data.as_array().ok_or_else(|| {
                self.schema(
                    "listAdUnits",
                    "response is not a top-level array of ad units",
                )
            })?;
            if page.len() > PAGE_LIMIT {
                return Err(self.schema(
                    "listAdUnits",
                    format!(
                        "page returned {} ad units, above the {PAGE_LIMIT} requested limit",
                        page.len()
                    ),
                ));
            }
            if page.is_empty() {
                return Ok(units);
            }
            for item in page {
                let unit = self.parse_ad_unit(item)?;
                if !ids.insert(unit.platform_unit_id.clone()) {
                    return Err(self.schema(
                        "listAdUnits",
                        format!(
                            "pagination repeated ad unit '{}' at offset {offset}; the server is not advancing",
                            unit.platform_unit_id
                        ),
                    ));
                }
                units.push(unit);
            }
            offset = offset
                .checked_add(page.len())
                .ok_or_else(|| self.schema("listAdUnits", "pagination offset overflowed usize"))?;
            if page.len() < PAGE_LIMIT {
                return Ok(units);
            }
        }
        Err(self.schema(
            "listAdUnits",
            format!("pagination exceeded {MAX_PAGES} pages"),
        ))
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
        let groups = floor_groups(floors)?;
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

    /// Create an experiment whose test group carries MAX frequency capping.
    /// The country lists must be disjoint across entries (the documented
    /// contract); the control group keeps the live unit's settings.
    pub async fn create_experiment_with_caps(
        &self,
        spec: &ExperimentSpec,
        caps: &[MaxFrequencyCap],
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
        if caps.is_empty() {
            return Err(PlatformError::Validation(
                "MAX createExperiment with frequency caps requires at least one cap".into(),
            ));
        }
        let settings = frequency_cap_settings(caps)?;
        let body = serde_json::json!({
            "experiment_name": spec.name,
            "test_group_allocation": spec.test_group_allocation,
            "frequency_capping_settings": settings,
        });
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

/// The User-Level Ad Revenue API path (the response is a signed CSV URL).
pub const MAX_USER_REVENUE_PATH: &str = "/max/userAdRevenueReport";

/// One player's revenue for one UTC day on one platform, folded across the
/// MAX ad units that served the player. This is the D2 feed shape: the
/// estimator never receives ad-unit rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRevenueRow {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    /// `android`, `fireos` or `ios`.
    pub platform: String,
    pub user_id: Option<String>,
    pub idfv: Option<String>,
    pub idfa: Option<String>,
    /// The experiment arm label, when the feed carries one. The documented
    /// user-level report has no arm column; a deployment that adds one is
    /// parsed here so the player rows can be labelled without ad-unit rows.
    pub test_group: Option<String>,
    /// Revenue in micros.
    pub revenue: i64,
    pub impressions: u64,
}

impl UserRevenueRow {
    /// The resolved player id in D4 order: User ID, IDFV, IDFA. A zeroed IDFA
    /// is missing, never a player.
    #[must_use]
    pub fn player_key(&self) -> Option<String> {
        valid_player_id(&self.user_id)
            .or_else(|| valid_player_id(&self.idfv))
            .or_else(|| valid_player_id(&self.idfa))
    }
}

fn valid_player_id(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && *value != "00000000-0000-0000-0000-000000000000"
                && *value != "00000000000000000000000000000000"
        })
        .map(str::to_string)
}

/// Parse the aggregated user-level CSV into one row per resolved player. The
/// header names are matched case-insensitively because MAX has used both
/// `User Id` and `User ID`; a missing `Revenue` column is a typed refusal.
/// Unidentified rows are kept as one row per distinct raw id tuple so the
/// caller can report the uncovered revenue share.
pub fn parse_user_revenue_csv(
    csv: &[u8],
    day: &str,
    platform: &str,
) -> Result<Vec<UserRevenueRow>, String> {
    let text = std::str::from_utf8(csv).map_err(|error| format!("CSV is not UTF-8: {error}"))?;
    let mut records = crate::csv::records(text).into_iter();
    let header_line = records.next().ok_or_else(|| "CSV is empty".to_string())?;
    let headers: Vec<String> = header_line
        .iter()
        .map(|field| field.trim().to_ascii_lowercase())
        .collect();
    let index = |names: &[&str]| -> Option<usize> {
        headers
            .iter()
            .position(|header| names.iter().any(|name| header == name))
    };
    let revenue_column =
        index(&["revenue"]).ok_or_else(|| "CSV has no Revenue column".to_string())?;
    let user_column = index(&["user id", "userid", "user_id"]);
    let idfv_column = index(&["idfv"]);
    let idfa_column = index(&["idfa"]);
    let impressions_column = index(&["impressions"]);
    let test_group_column = index(&[
        "max_ad_unit_test",
        "max ad unit test",
        "test group",
        "testgroup",
        "ad unit test",
    ]);
    if user_column.is_none() && idfv_column.is_none() && idfa_column.is_none() {
        return Err("CSV has no user id, IDFV or IDFA column".into());
    }
    // Fold per resolved player; unresolved rows keep their raw identifier
    // tuple so two different anonymous players are not merged.
    let mut resolved: std::collections::BTreeMap<String, UserRevenueRow> =
        std::collections::BTreeMap::new();
    let mut unresolved: Vec<UserRevenueRow> = Vec::new();
    for fields in records {
        let display = fields.join(",");
        let revenue_raw = fields
            .get(revenue_column)
            .ok_or_else(|| format!("CSV row missing revenue: {display}"))?;
        let revenue = parse_decimal_micros(revenue_raw.trim())
            .map_err(|error| format!("revenue '{revenue_raw}': {error}"))?;
        let impressions = match impressions_column.and_then(|column| fields.get(column)) {
            Some(raw) if !raw.trim().is_empty() => raw
                .trim()
                .parse::<u64>()
                .map_err(|error| format!("impressions '{raw}' on '{display}': {error}"))?,
            _ => 0,
        };
        let row = UserRevenueRow {
            day: day.to_string(),
            platform: platform.to_string(),
            user_id: user_column
                .and_then(|column| fields.get(column))
                .cloned()
                .filter(|value| !value.trim().is_empty()),
            idfv: idfv_column
                .and_then(|column| fields.get(column))
                .cloned()
                .filter(|value| !value.trim().is_empty()),
            idfa: idfa_column
                .and_then(|column| fields.get(column))
                .cloned()
                .filter(|value| !value.trim().is_empty()),
            test_group: test_group_column
                .and_then(|column| fields.get(column))
                .cloned()
                .filter(|value| !value.trim().is_empty()),
            revenue,
            impressions,
        };
        match row.player_key() {
            Some(key) => match resolved.entry(key) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(row);
                }
                std::collections::btree_map::Entry::Occupied(mut slot) => {
                    let player = slot.key().clone();
                    let entry = slot.get_mut();
                    if entry.test_group.as_deref().map(str::trim)
                        != row.test_group.as_deref().map(str::trim)
                    {
                        return Err(format!(
                            "player '{player}' appears under more than one experiment arm on {day}"
                        ));
                    }
                    entry.revenue = entry
                        .revenue
                        .checked_add(revenue)
                        .ok_or_else(|| "user revenue overflowed i64 micros".to_string())?;
                    entry.impressions = entry
                        .impressions
                        .checked_add(impressions)
                        .ok_or_else(|| "user impressions overflowed u64".to_string())?;
                }
            },
            None => unresolved.push(row),
        }
    }
    let mut out: Vec<UserRevenueRow> = resolved.into_values().collect();
    out.append(&mut unresolved);
    Ok(out)
}

/// CSV fields without the quoted-field grammar the report never uses; quotes
/// are stripped if a deployment wraps a field. Values are trimmed by callers.
pub(crate) fn valid_day_for_feed(day: &str) -> bool {
    valid_day(day)
}

fn valid_day(day: &str) -> bool {
    let mut parts = day.split('-');
    let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    parts.next().is_none()
        && year.len() == 4
        && year.chars().all(|c| c.is_ascii_digit())
        && month.len() == 2
        && day.len() == 2
        && month.parse::<u32>().is_ok_and(|m| (1..=12).contains(&m))
        && day.parse::<u32>().is_ok_and(|d| (1..=31).contains(&d))
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
            base_url: base_url.trim_end_matches('/').to_string(),
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
        if columns.is_empty() {
            return Err(PlatformError::Validation(
                "MAX maxReport requires at least one column".into(),
            ));
        }
        let columns = columns.join(",");
        let windows = report_windows(start, end)?;
        // A provider that accepts the whole range is asked once. Only when it
        // refuses the request because the span exceeds the documented 45-day
        // window do we split and retry the exact same query in windows. This
        // keeps the common short-range path one request while never sending a
        // range the provider documents as invalid more than once.
        if windows.len() > 1 {
            let probe = self
                .fetch_window(start, end, &columns, filter, true)
                .await?;
            if !probe.window_too_long {
                return Ok(probe.rows);
            }
        }
        let mut collected: Vec<ReportRow> = Vec::new();
        for (window_start, window_end) in windows {
            let outcome = self
                .fetch_window(&window_start, &window_end, &columns, filter, false)
                .await?;
            collected.extend(outcome.rows);
        }
        Ok(collected)
    }

    /// Fetch one inclusive window, following `count`/`limit`/`offset` until
    /// the provider reports the window complete. When `probe` is set the
    /// caller is testing whether the provider accepts a range longer than the
    /// documented window; a 400 then means "split it" rather than a failure.
    async fn fetch_window(
        &self,
        window_start: &str,
        window_end: &str,
        columns: &str,
        filter: Option<(&str, &str)>,
        probe: bool,
    ) -> Result<FetchOutcome, PlatformError> {
        self.fetch_window_with_bound(window_start, window_end, columns, filter, probe, MAX_PAGES)
            .await
    }

    /// [`fetch_window`] with an explicit page bound so the exhaustion path is
    /// testable without issuing ten thousand requests.
    async fn fetch_window_with_bound(
        &self,
        window_start: &str,
        window_end: &str,
        columns: &str,
        filter: Option<(&str, &str)>,
        probe: bool,
        max_pages: usize,
    ) -> Result<FetchOutcome, PlatformError> {
        let mut collected: Vec<ReportRow> = Vec::new();
        let mut offset = 0usize;
        let mut window_collected = 0usize;
        // `count` is documented as "the number of results returned", but some
        // deployments report the total for the whole query. Treat it as a
        // total only when it exceeds the page length, and otherwise page on
        // the page length itself.
        let mut claimed_total: Option<usize> = None;
        let mut previous_page: Option<String> = None;
        let mut completed = false;
        for _ in 0..max_pages {
            let mut url = format!(
                "{}/maxReport?api_key={}&columns={}&start={}&end={}&format=json&limit={PAGE_LIMIT}&offset={offset}",
                self.base_url,
                encode_query(&self.report_key),
                encode_query(columns),
                encode_query(window_start),
                encode_query(window_end),
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
                Err(crate::transport::TransportError::Platform(e)) => {
                    if probe && matches!(&e, PlatformError::Http { status: 400, .. }) {
                        return Ok(FetchOutcome {
                            rows: Vec::new(),
                            window_too_long: true,
                        });
                    }
                    return Err(e);
                }
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
                if probe && response.status == 400 {
                    return Ok(FetchOutcome {
                        rows: Vec::new(),
                        window_too_long: true,
                    });
                }
                return Err(report_status_error(response.status));
            }
            let parsed: serde_json::Value =
                serde_json::from_slice(&response.body).map_err(|e| PlatformError::Schema {
                    platform: PlatformId::AppLovinMax,
                    operation: "maxReport".into(),
                    detail: format!("response is not JSON: {e}"),
                })?;
            let code = parsed.get("code").and_then(|v| v.as_u64()).unwrap_or(200);
            if code != 200 {
                if probe && code == 400 {
                    return Ok(FetchOutcome {
                        rows: Vec::new(),
                        window_too_long: true,
                    });
                }
                let status = u16::try_from(code).unwrap_or(500);
                return Err(report_status_error(status));
            }
            let results = parsed
                .get("results")
                .and_then(|v| v.as_array())
                .ok_or_else(|| PlatformError::Schema {
                    platform: PlatformId::AppLovinMax,
                    operation: "maxReport".into(),
                    detail: "response has no results array".into(),
                })?;
            if results.len() > PAGE_LIMIT {
                return Err(PlatformError::Schema {
                    platform: PlatformId::AppLovinMax,
                    operation: "maxReport".into(),
                    detail: format!(
                        "page returned {} results, above the {PAGE_LIMIT} requested limit",
                        results.len()
                    ),
                });
            }
            let count = parsed.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if count > results.len() {
                // A later page may announce MORE rows than the first did:
                // keep the largest claim, never freeze on (or shrink to) an
                // earlier page's count.
                claimed_total = Some(claimed_total.map_or(count, |known| known.max(count)));
            }
            let page_len = results.len();
            if page_len == 0 {
                if let Some(total) = claimed_total {
                    if window_collected < total {
                        return Err(PlatformError::Schema {
                            platform: PlatformId::AppLovinMax,
                            operation: "maxReport".into(),
                            detail: format!(
                                "pagination truncated for {window_start}..{window_end}: collected {window_collected} of {total} rows before the provider returned an empty page"
                            ),
                        });
                    }
                }
                completed = true;
                break;
            }
            let signature = serde_json::to_string(results).unwrap_or_default();
            if previous_page.as_deref() == Some(signature.as_str()) {
                return Err(PlatformError::Schema {
                    platform: PlatformId::AppLovinMax,
                    operation: "maxReport".into(),
                    detail: format!(
                        "pagination repeated the same {page_len}-row page at offset {offset}"
                    ),
                });
            }
            previous_page = Some(signature);
            for row in results {
                collected.push(
                    parse_report_row(row).map_err(|detail| PlatformError::Schema {
                        platform: PlatformId::AppLovinMax,
                        operation: "maxReport".into(),
                        detail,
                    })?,
                );
            }
            window_collected += page_len;
            if let Some(total) = claimed_total {
                if window_collected >= total {
                    completed = true;
                    break;
                }
            }
            if page_len < PAGE_LIMIT && claimed_total.is_none() {
                completed = true;
                break;
            }
            offset = offset
                .checked_add(page_len)
                .ok_or_else(|| PlatformError::Schema {
                    platform: PlatformId::AppLovinMax,
                    operation: "maxReport".into(),
                    detail: "pagination offset overflowed usize".into(),
                })?;
        }
        if completed {
            return Ok(FetchOutcome {
                rows: collected,
                window_too_long: false,
            });
        }
        // The bound is only reached without a completeness signal: a short
        // page, an empty page or a fulfilled count breaks inside. Returning
        // the rows anyway would present a truncated window as a full one.
        if let Some(total) = claimed_total {
            return Err(PlatformError::Schema {
                platform: PlatformId::AppLovinMax,
                operation: "maxReport".into(),
                detail: format!(
                    "pagination truncated for {window_start}..{window_end}: collected {window_collected} of {total} rows after {max_pages} pages"
                ),
            });
        }
        Err(PlatformError::Schema {
            platform: PlatformId::AppLovinMax,
            operation: "maxReport".into(),
            detail: format!(
                "maxReport exceeded {max_pages} pages for {window_start}..{window_end} without a short page; the window is unread"
            ),
        })
    }

    /// The User-Level Ad Revenue API for one UTC day, aggregated per player
    /// (`aggregated=true`). The report endpoint answers a signed CSV URL; this
    /// method fetches that URL and folds the rows into one value per player,
    /// keyed by `(day, platform, user)`. It is deliberately **not** an
    /// ad-unit report: callers that need an ad-unit breakdown use the Revenue
    /// Reporting API, and the per-player estimator must never see ad-unit
    /// rows.
    ///
    /// `platform` is one of the documented `android`, `fireos` or `ios`
    /// values, and `application` is the package name / bundle id. The data is
    /// available eight hours after the UTC day; the caller schedules the pull.
    pub async fn user_revenue_day(
        &self,
        day: &str,
        platform: &str,
        application: &str,
    ) -> Result<Vec<UserRevenueRow>, PlatformError> {
        let platform = platform.trim().to_ascii_lowercase();
        if !matches!(platform.as_str(), "android" | "fireos" | "ios") {
            return Err(PlatformError::Validation(format!(
                "MAX user revenue platform must be android, fireos or ios (given '{platform}')"
            )));
        }
        if application.trim().is_empty() {
            return Err(PlatformError::Validation(
                "MAX user revenue requires the application package/bundle id".into(),
            ));
        }
        if !valid_day(day) {
            return Err(PlatformError::Validation(format!(
                "MAX user revenue day must be YYYY-MM-DD (given '{day}')"
            )));
        }
        let url = format!(
            "{}{MAX_USER_REVENUE_PATH}?api_key={}&date={}&platform={}&application={}&aggregated=true",
            self.base_url,
            encode_query(&self.report_key),
            encode_query(day),
            encode_query(&platform),
            encode_query(application),
        );
        let request = crate::transport::HttpRequest {
            platform: PlatformId::AppLovinMax,
            operation: "userAdRevenueReport".to_string(),
            method: HttpMethod::Get,
            url,
            headers: vec![],
            body: None,
            timeout_ms: 30_000,
        };
        let response = self.request_report(&request, "userAdRevenueReport").await?;
        let parsed: serde_json::Value =
            serde_json::from_slice(&response).map_err(|error| PlatformError::Schema {
                platform: PlatformId::AppLovinMax,
                operation: "userAdRevenueReport".into(),
                detail: format!("response is not JSON: {error}"),
            })?;
        let status = parsed
            .get("status")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let report_url = parsed
            .get("ad_revenue_report_url")
            .and_then(serde_json::Value::as_str)
            .filter(|url| !url.is_empty());
        if status != 200 {
            return Err(PlatformError::Schema {
                platform: PlatformId::AppLovinMax,
                operation: "userAdRevenueReport".into(),
                detail: format!("report endpoint refused with status {status}"),
            });
        }
        let report_url = report_url.ok_or_else(|| PlatformError::Schema {
            platform: PlatformId::AppLovinMax,
            operation: "userAdRevenueReport".into(),
            detail: "response carried no ad_revenue_report_url".into(),
        })?;
        let download = crate::transport::HttpRequest {
            platform: PlatformId::AppLovinMax,
            operation: "userAdRevenueReportCsv".to_string(),
            method: HttpMethod::Get,
            url: report_url.to_string(),
            headers: vec![],
            body: None,
            timeout_ms: 120_000,
        };
        let csv = self
            .request_report(&download, "userAdRevenueReportCsv")
            .await?;
        parse_user_revenue_csv(&csv, day, &platform).map_err(|detail| PlatformError::Schema {
            platform: PlatformId::AppLovinMax,
            operation: "userAdRevenueReport".into(),
            detail,
        })
    }

    /// Run one reporting request through the shared transport, mapping the
    /// transport's typed failures and non-200 statuses consistently.
    async fn request_report(
        &self,
        request: &crate::transport::HttpRequest,
        operation: &str,
    ) -> Result<Vec<u8>, PlatformError> {
        match self.transport.request(request.clone()).await {
            Ok(response) if response.status == 200 => Ok(response.body),
            Ok(response) => Err(report_status_error(response.status)),
            Err(crate::transport::TransportError::Platform(error)) => Err(error),
            Err(crate::transport::TransportError::Network(detail)) => Err(PlatformError::Network {
                platform: PlatformId::AppLovinMax,
                operation: operation.into(),
                detail,
            }),
            Err(crate::transport::TransportError::Timeout(detail)) => Err(PlatformError::Network {
                platform: PlatformId::AppLovinMax,
                operation: operation.into(),
                detail: format!("timeout: {detail}"),
            }),
        }
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

    /// The last 30 complete days of country/ad-unit revenue rows, for the
    /// floor rules. `now_unix_ms` bounds the window (the day before today is
    /// the last complete day).
    pub async fn country_revenue_30d(
        &self,
        now_unix_ms: i64,
    ) -> Result<Vec<ReportRow>, PlatformError> {
        self.country_revenue_30d_with_last_day(now_unix_ms)
            .await
            .map(|(rows, _)| rows)
    }

    /// The same one-pull read as [`Self::country_revenue_30d`], plus the last
    /// complete day's own rows (HH): the account watch stores the daily rows
    /// per unit, so its day values are the day's own numbers instead of a
    /// difference of two rolling 30-day totals. Both results fold from one
    /// reporting pull — the same call serves the floor rules and the watch.
    pub async fn country_revenue_30d_with_last_day(
        &self,
        now_unix_ms: i64,
    ) -> Result<(Vec<ReportRow>, Vec<ReportRow>), PlatformError> {
        let today = now_unix_ms.div_euclid(86_400_000);
        let start = format_day(today - 30);
        let end = format_day(today - 1);
        let rows = self.country_ad_unit_rows(&start, &end).await?;
        // The reporting API groups by every requested dimension (here: day),
        // so the same country/ad-unit pair arrives once per day. The audit
        // consumes one 30-day total per pair; returning the daily rows would
        // make it treat each day as a separate 30-day observation and double
        // count the floor rules. Fold checked, so an overflowing total fails
        // typed instead of wrapping.
        let mut totals: std::collections::BTreeMap<(Option<String>, Option<String>), (i64, u64)> =
            std::collections::BTreeMap::new();
        for row in &rows {
            let entry = totals
                .entry((row.country.clone(), row.ad_unit_id.clone()))
                .or_insert((0, 0));
            entry.0 = entry.0.checked_add(row.revenue).ok_or_else(|| {
                PlatformError::Validation("MAX 30-day revenue total overflow".into())
            })?;
            entry.1 = entry.1.checked_add(row.impressions).ok_or_else(|| {
                PlatformError::Validation("MAX 30-day impressions total overflow".into())
            })?;
        }
        let folded = totals
            .into_iter()
            .map(
                |((country, ad_unit_id), (revenue, impressions))| ReportRow {
                    day: end.clone(),
                    country,
                    ad_unit_id,
                    test_group: None,
                    revenue,
                    impressions,
                },
            )
            .collect();
        // The window's last day is the pull's own daily slice.
        let last_day = rows
            .into_iter()
            .filter(|row| row.day == end)
            .collect();
        Ok((folded, last_day))
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
        // `impressions` is requested even though only revenue is summed: the
        // shared row parser requires the column, and a missing column would
        // otherwise fail the whole monthly billing pull.
        let rows = self
            .rows(
                start,
                end,
                &["day", "max_ad_unit_id", "estimated_revenue", "impressions"],
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
        // `day` is requested because the shared row parser requires it even
        // when the caller only sums the metrics. Requesting only the two
        // numeric columns made the provider return rows the parser rejected.
        let rows = self
            .rows(
                start,
                end,
                &["day", "estimated_revenue", "impressions"],
                None,
            )
            .await?;
        let revenue: i64 = rows.iter().map(|r| r.revenue).sum();
        let impressions: u64 = rows.iter().map(|r| r.impressions).sum();
        Ok((revenue, impressions))
    }
}

/// One window's fetch result. `window_too_long` is set only for the probe
/// request that tests a range above the documented 45-day limit.
#[derive(Debug)]
struct FetchOutcome {
    rows: Vec<ReportRow>,
    window_too_long: bool,
}

/// Classify a reporting-API status the same way the retry transport does, so
/// a client built without the retry wrapper never turns a credential refusal
/// or a rate limit into a generic HTTP error.
fn report_status_error(status: u16) -> PlatformError {
    if status == 401 || status == 403 {
        return PlatformError::Auth {
            platform: PlatformId::AppLovinMax,
            operation: "maxReport".into(),
            status,
        };
    }
    if status == 429 {
        return PlatformError::RateLimited {
            platform: PlatformId::AppLovinMax,
            operation: "maxReport".into(),
            retry_after: None,
        };
    }
    PlatformError::Http {
        platform: PlatformId::AppLovinMax,
        operation: "maxReport".into(),
        status,
    }
}

/// Split `[start, end]` into inclusive windows no longer than the documented
/// 45-day reporting limit. Invalid or reversed dates are a typed validation
/// error before any request is sent.
fn report_windows(start: &str, end: &str) -> Result<Vec<(String, String)>, PlatformError> {
    let start_day = parse_day(start).ok_or_else(|| {
        PlatformError::Validation(format!("MAX maxReport start '{start}' is not YYYY-MM-DD"))
    })?;
    let end_day = parse_day(end).ok_or_else(|| {
        PlatformError::Validation(format!("MAX maxReport end '{end}' is not YYYY-MM-DD"))
    })?;
    if end_day < start_day {
        return Err(PlatformError::Validation(format!(
            "MAX maxReport end '{end}' is before start '{start}'"
        )));
    }
    let mut windows = Vec::new();
    let mut cursor = start_day;
    while cursor <= end_day {
        let window_end = (cursor + MAX_REPORT_WINDOW_DAYS - 1).min(end_day);
        windows.push((format_day(cursor), format_day(window_end)));
        cursor = window_end + 1;
    }
    Ok(windows)
}

/// `YYYY-MM-DD` -> day number since the Unix epoch, strict enough to reject
/// impossible dates (the civil-to-days inverse of [`format_day`]).
fn parse_day(text: &str) -> Option<i64> {
    let mut parts = text.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    // Round-trip catches e.g. 2026-02-30, which would otherwise roll over.
    (format_day(days) == text).then_some(days)
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

/// Build the documented `frequency_capping_settings` array and reject country
/// overlap across entries. An empty country list means all countries; it may
/// appear only once because it already covers the whole population.
fn frequency_cap_settings(
    caps: &[MaxFrequencyCap],
) -> Result<Vec<serde_json::Value>, PlatformError> {
    let mut settings = Vec::with_capacity(caps.len());
    let mut seen_countries: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut saw_global = false;
    for cap in caps {
        if saw_global && !cap.countries.is_empty() {
            return Err(PlatformError::Validation(
                "a global MAX frequency cap cannot be combined with country-specific caps".into(),
            ));
        }
        let mut countries = Vec::with_capacity(cap.countries.len());
        for country in &cap.countries {
            let normalized = apex_core::normalize_country(country)
                .map_err(|error| PlatformError::Validation(error.to_string()))?;
            let normalized = normalized.to_ascii_lowercase();
            if !seen_countries.insert(normalized.clone()) {
                return Err(PlatformError::Validation(format!(
                    "MAX frequency capping country '{normalized}' appears in more than one setting; country lists must be disjoint"
                )));
            }
            countries.push(normalized);
        }
        if countries.is_empty() {
            if saw_global || !seen_countries.is_empty() {
                return Err(PlatformError::Validation(
                    "a global MAX frequency cap (empty country list) cannot be combined with other cap country lists"
                        .into(),
                ));
            }
            saw_global = true;
        }
        let (session_limit, day_limit, minute_frequency) = match cap.cap_type {
            MaxCapFrequencyType::Session => {
                let limit = cap
                    .session_limit
                    .filter(|limit| *limit > 0)
                    .ok_or_else(|| {
                        PlatformError::Validation(
                            "a MAX session cap requires a positive session_limit".into(),
                        )
                    })?;
                (limit, 0, 0)
            }
            MaxCapFrequencyType::Time => {
                let day = cap.day_limit.unwrap_or(0);
                let minute = cap.minute_frequency.unwrap_or(0);
                if day == 0 && minute == 0 {
                    return Err(PlatformError::Validation(
                        "a MAX time cap requires day_limit or minute_frequency".into(),
                    ));
                }
                (0, day, minute)
            }
        };
        settings.push(serde_json::json!({
            "type": cap.cap_type.as_str(),
            "session_capping_settings": {"session_limit": session_limit},
            "time_capping_settings": {"day_limit": day_limit, "minute_frequency": minute_frequency},
            "countries": {"type": "INCLUDE", "values": countries},
        }));
    }
    Ok(settings)
}

/// Rebuild the platform's grouped `bid_floors` shape: one group per distinct
/// group name in first-seen order, its countries deduplicated in first-seen
/// order, and each group's `cpm` taken from its first floor.
///
/// The fold is single-pass: a `Vec::contains`/`position` over the grown
/// groups and country lists would be quadratic in the submitted floor count.
fn floor_groups(floors: &[CountryFloor]) -> Result<Vec<serde_json::Value>, PlatformError> {
    struct Group<'a> {
        name: &'a str,
        cpm_micros: i64,
        countries: Vec<&'a str>,
    }
    let mut index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut seen_countries: Vec<std::collections::HashSet<&str>> = Vec::new();
    let mut groups: Vec<Group<'_>> = Vec::new();
    for floor in floors {
        let key = floor.group.as_deref().unwrap_or("default");
        match index.get(key).copied() {
            Some(pos) => {
                if seen_countries[pos].insert(floor.country.as_str()) {
                    groups[pos].countries.push(floor.country.as_str());
                }
            }
            None => {
                index.insert(key, groups.len());
                let mut seen = std::collections::HashSet::new();
                seen.insert(floor.country.as_str());
                seen_countries.push(seen);
                groups.push(Group {
                    name: key,
                    cpm_micros: floor.floor_ecpm,
                    countries: vec![floor.country.as_str()],
                });
            }
        }
    }
    Ok(groups
        .into_iter()
        .map(|group| {
            serde_json::json!({
                "country_group_name": group.name,
                "cpm": format_ecpm(group.cpm_micros),
                "countries": {"type": "INCLUDE", "values": group.countries}
            })
        })
        .collect())
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

/// `YYYY-MM-DD` from a day number since the Unix epoch (Howard Hinnant's
/// civil-from-days algorithm).
fn format_day(day: i64) -> String {
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
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
            instance_id: String::new(),
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

    #[tokio::test]
    async fn list_ad_units_follows_limit_and_offset_until_a_short_page() {
        let mut first = Vec::new();
        for index in 0..PAGE_LIMIT {
            first.push(serde_json::json!({
                "id": format!("u{index:04}"),
                "name": "Banner",
                "platform": "ios",
                "package_name": "com.x",
                "ad_format": "BANNER"
            }));
        }
        let page1 = serde_json::to_string(&first).expect("page one");
        let page2 = r#"[{"id":"last","name":"Rewarded","platform":"android","package_name":"com.y","ad_format":"REWARD"}]"#;
        let transport = Scripted::new(vec![ok(&page1), ok(page2)]);
        let units = client(transport.clone())
            .list_ad_units(false)
            .await
            .expect("paginated units");
        assert_eq!(units.len(), PAGE_LIMIT + 1);
        assert_eq!(units.last().expect("last").platform_unit_id, "last");
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[0].url.contains("limit=1000&offset=0"),
            "{}",
            calls[0].url
        );
        assert!(
            calls[1].url.contains("limit=1000&offset=1000"),
            "{}",
            calls[1].url
        );
    }

    #[tokio::test]
    async fn list_ad_units_fails_typed_when_pagination_does_not_advance() {
        let mut page = Vec::new();
        for index in 0..PAGE_LIMIT {
            page.push(serde_json::json!({
                "id": format!("u{index:04}"),
                "name": "Banner",
                "platform": "ios",
                "package_name": "com.x",
                "ad_format": "BANNER"
            }));
        }
        let body = serde_json::to_string(&page).expect("page");
        let transport = Scripted::new(vec![ok(&body), ok(&body)]);
        let error = client(transport)
            .list_ad_units(false)
            .await
            .expect_err("a repeated page is not a complete list");
        assert!(
            matches!(error, PlatformError::Schema { .. }),
            "unexpected {error:?}"
        );
        assert!(error.to_string().contains("not advancing"), "{error}");
    }

    #[tokio::test]
    async fn list_ad_units_reports_a_response_above_the_requested_page_size() {
        let mut page = Vec::new();
        for index in 0..=PAGE_LIMIT {
            page.push(serde_json::json!({
                "id": format!("u{index:04}"),
                "name": "Banner",
                "platform": "ios",
                "package_name": "com.x",
                "ad_format": "BANNER"
            }));
        }
        let body = serde_json::to_string(&page).expect("page");
        let transport = Scripted::new(vec![ok(&body)]);
        let error = client(transport)
            .list_ad_units(false)
            .await
            .expect_err("oversized page");
        assert!(error.to_string().contains("requested limit"), "{error}");
    }

    #[tokio::test]
    async fn report_pagination_uses_the_documented_page_sized_count() {
        // The docs define `count` as "the number of results returned"; a
        // short page is the end even when count equals the page size.
        let page = r#"{"code":200,"count":1,"results":[
            {"day":"2026-10-01","country":"US","max_ad_unit_id":"u1","estimated_revenue":"1.00","impressions":"10"}
        ]}"#;
        let transport = Scripted::new(vec![ok(page)]);
        let report = MaxReportClient::new("report-key", "https://r.test", transport.clone())
            .expect("report");
        let rows = report
            .country_ad_unit_rows("2026-10-01", "2026-10-01")
            .await
            .expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn report_pagination_fails_typed_when_the_provider_truncates() {
        let page1 = r#"{"code":200,"count":5,"results":[
            {"day":"2026-10-01","country":"US","max_ad_unit_id":"u1","estimated_revenue":"1.00","impressions":"10"},
            {"day":"2026-10-02","country":"US","max_ad_unit_id":"u1","estimated_revenue":"2.00","impressions":"20"}
        ]}"#;
        let page2 = r#"{"code":200,"count":5,"results":[]}"#;
        let transport = Scripted::new(vec![ok(page1), ok(page2)]);
        let report =
            MaxReportClient::new("report-key", "https://r.test", transport).expect("report");
        let error = report
            .country_ad_unit_rows("2026-10-01", "2026-10-02")
            .await
            .expect_err("a truncated report must not look complete");
        assert!(
            matches!(error, PlatformError::Schema { .. }),
            "unexpected {error:?}"
        );
        assert!(error.to_string().contains("truncated"), "{error}");
        assert!(error.to_string().contains("collected 2 of 5"), "{error}");
    }

    #[tokio::test]
    async fn exhausting_the_page_bound_refuses_the_window_instead_of_returning_it() {
        let page = |row: usize| {
            serde_json::to_string(&serde_json::json!({
                "code": 200,
                "count": 9_999,
                "results": [{
                    "day": "2026-10-01",
                    "country": "US",
                    "max_ad_unit_id": "u1",
                    "estimated_revenue": "1.00",
                    "impressions": "10",
                    "row": row
                }]
            }))
            .expect("page")
        };
        let transport = Scripted::new(vec![ok(&page(0)), ok(&page(1)), ok(&page(2))]);
        let report =
            MaxReportClient::new("report-key", "https://r.test", transport).expect("report");
        let error = report
            .fetch_window_with_bound("2026-10-01", "2026-10-02", "day", None, false, 3)
            .await
            .expect_err("an unread window must not return as complete");
        assert!(matches!(error, PlatformError::Schema { .. }), "{error:?}");
        assert!(error.to_string().contains("after 3 pages"), "{error}");
        assert!(
            error.to_string().contains("1 of 9999") || error.to_string().contains("3 of 9999"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_later_page_cannot_shrink_the_announced_total() {
        let page1 = r#"{"code":200,"count":9,"results":[
            {"day":"2026-10-01","country":"US","max_ad_unit_id":"u1","estimated_revenue":"1.00","impressions":"10","row":1},
            {"day":"2026-10-01","country":"US","max_ad_unit_id":"u1","estimated_revenue":"2.00","impressions":"20","row":2}
        ]}"#;
        let page2 = r#"{"code":200,"count":3,"results":[
            {"day":"2026-10-02","country":"US","max_ad_unit_id":"u1","estimated_revenue":"3.00","impressions":"30","row":3},
            {"day":"2026-10-02","country":"US","max_ad_unit_id":"u1","estimated_revenue":"4.00","impressions":"40","row":4}
        ]}"#;
        let page3 = r#"{"code":200,"count":3,"results":[]}"#;
        let transport = Scripted::new(vec![ok(page1), ok(page2), ok(page3)]);
        let report =
            MaxReportClient::new("report-key", "https://r.test", transport).expect("report");
        let error = report
            .country_ad_unit_rows("2026-10-01", "2026-10-02")
            .await
            .expect_err("the first (largest) claim still governs");
        assert!(error.to_string().contains("collected 4 of 9"), "{error}");
    }

    #[tokio::test]
    async fn report_pagination_fails_typed_on_a_repeated_page_without_a_total() {
        let mut results = Vec::new();
        for index in 0..PAGE_LIMIT {
            results.push(serde_json::json!({
                "day": "2026-10-01",
                "country": "US",
                "max_ad_unit_id": "u1",
                "estimated_revenue": "1.00",
                "impressions": "10",
                "row": index
            }));
        }
        let page = serde_json::to_string(&serde_json::json!({
            "code": 200,
            "count": PAGE_LIMIT,
            "results": results
        }))
        .expect("page");
        let transport = Scripted::new(vec![ok(&page), ok(&page)]);
        let report =
            MaxReportClient::new("report-key", "https://r.test", transport).expect("report");
        let error = report
            .country_ad_unit_rows("2026-10-01", "2026-10-02")
            .await
            .expect_err("a repeated page");
        assert!(error.to_string().contains("repeated"), "{error}");
    }

    #[tokio::test]
    async fn thirty_day_country_revenue_folds_daily_rows_per_pair() {
        let page = r#"{"code":200,"count":4,"results":[
            {"day":"2026-10-01","country":"EE","max_ad_unit_id":"u1","estimated_revenue":"1.50","impressions":"10"},
            {"day":"2026-10-02","country":"EE","max_ad_unit_id":"u1","estimated_revenue":"2.25","impressions":"15"},
            {"day":"2026-10-01","country":"US","max_ad_unit_id":"u1","estimated_revenue":"4.00","impressions":"40"},
            {"day":"2026-10-01","country":"EE","max_ad_unit_id":"u2","estimated_revenue":"9.00","impressions":"90"}
        ]}"#;
        let transport = Scripted::new(vec![ok(page)]);
        let report = MaxReportClient::new("report-key", "https://r.test", transport.clone())
            .expect("report");
        let now = 1_791_000_000_000_i64; // 2026-10-03 UTC
        let rows = report.country_revenue_30d(now).await.expect("rows");
        assert_eq!(rows.len(), 3, "one row per country/ad-unit pair");
        let ee_u1 = rows
            .iter()
            .find(|row| {
                row.country.as_deref() == Some("EE") && row.ad_unit_id.as_deref() == Some("u1")
            })
            .expect("EE u1");
        assert_eq!(ee_u1.revenue, 3_750_000);
        assert_eq!(ee_u1.impressions, 25);
        let us_u1 = rows
            .iter()
            .find(|row| {
                row.country.as_deref() == Some("US") && row.ad_unit_id.as_deref() == Some("u1")
            })
            .expect("US u1");
        assert_eq!(us_u1.revenue, 4_000_000);
        assert_eq!(us_u1.impressions, 40);
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert!(
            calls[0].url.contains("start=2026-09-03"),
            "{}",
            calls[0].url
        );
        assert!(calls[0].url.contains("end=2026-10-02"), "{}", calls[0].url);

        // The same one-pull read also yields the last complete day's own
        // rows (HH): the watch's daily slice, from the same response.
        let transport = Scripted::new(vec![ok(page)]);
        let report = MaxReportClient::new("report-key", "https://r.test", transport).expect("r2");
        let (rows, last_day) = report
            .country_revenue_30d_with_last_day(now)
            .await
            .expect("rows with last day");
        assert_eq!(rows.len(), 3);
        assert_eq!(last_day.len(), 1, "only 2026-10-02 rows: {last_day:?}");
        assert_eq!(last_day[0].day, "2026-10-02");
        assert_eq!(last_day[0].country.as_deref(), Some("EE"));
        assert_eq!(last_day[0].ad_unit_id.as_deref(), Some("u1"));
        assert_eq!(last_day[0].revenue, 2_250_000);
        assert_eq!(last_day[0].impressions, 15);
    }

    #[tokio::test]
    async fn a_long_range_is_probed_and_split_only_after_a_400() {
        let refused = Ok(HttpResponse {
            status: 400,
            headers: vec![],
            body: b"request window is 60 days; the API allows 45".to_vec(),
        });
        let first = ok(r#"{"code":200,"count":1,"results":[
                {"day":"2026-01-01","country":"US","max_ad_unit_id":"u1","estimated_revenue":"1.00","impressions":"10"}
            ]}"#);
        let second = ok(r#"{"code":200,"count":1,"results":[
                {"day":"2026-02-20","country":"US","max_ad_unit_id":"u1","estimated_revenue":"2.00","impressions":"20"}
            ]}"#);
        let transport = Scripted::new(vec![refused, first, second]);
        let report = MaxReportClient::new("report-key", "https://r.test", transport.clone())
            .expect("report");
        let rows = report
            .country_ad_unit_rows("2026-01-01", "2026-03-01")
            .await
            .expect("split rows");
        assert_eq!(rows.len(), 2);
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert_eq!(calls.len(), 3, "probe, then one request per 45-day window");
        assert!(
            calls[0].url.contains("start=2026-01-01"),
            "{}",
            calls[0].url
        );
        assert!(calls[0].url.contains("end=2026-03-01"), "{}", calls[0].url);
        assert!(calls[1].url.contains("end=2026-02-14"), "{}", calls[1].url);
        assert!(
            calls[2].url.contains("start=2026-02-15"),
            "{}",
            calls[2].url
        );
    }

    #[tokio::test]
    async fn a_provider_that_accepts_the_long_range_is_asked_once() {
        let transport = Scripted::new(vec![ok(r#"{"code":200,"count":1,"results":[
                {"day":"2026-01-01","country":"US","max_ad_unit_id":"u1","estimated_revenue":"1.00","impressions":"10"}
            ]}"#)]);
        let report = MaxReportClient::new("report-key", "https://r.test", transport.clone())
            .expect("report");
        let rows = report
            .country_ad_unit_rows("2026-01-01", "2026-03-01")
            .await
            .expect("long range");
        assert_eq!(rows.len(), 1);
        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn report_windows_split_the_documented_45_day_limit() {
        let windows = report_windows("2026-01-01", "2026-03-01").expect("windows");
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0], ("2026-01-01".into(), "2026-02-14".into()));
        assert_eq!(windows[1], ("2026-02-15".into(), "2026-03-01".into()));
        for (start, end) in &windows {
            let span = parse_day(end).expect("end") - parse_day(start).expect("start") + 1;
            assert!(
                span <= MAX_REPORT_WINDOW_DAYS,
                "{start}..{end} is {span} days"
            );
        }
        assert!(report_windows("2026-03-01", "2026-01-01").is_err());
        assert!(report_windows("2026-02-30", "2026-03-01").is_err());
    }

    #[tokio::test]
    async fn totals_requests_the_day_column_its_parser_requires() {
        let transport = Scripted::new(vec![ok(r#"{"code":200,"count":1,"results":[
                {"day":"2026-10-01","estimated_revenue":"1.50","impressions":"10"}
            ]}"#)]);
        let report = MaxReportClient::new("report-key", "https://r.test", transport.clone())
            .expect("report");
        let totals = report
            .totals("2026-10-01", "2026-10-01")
            .await
            .expect("totals");
        assert_eq!(totals, (1_500_000, 10));
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert!(
            calls[0]
                .url
                .contains("columns=day%2Cestimated_revenue%2Cimpressions"),
            "{}",
            calls[0].url
        );
    }

    #[tokio::test]
    async fn report_statuses_keep_auth_and_rate_limit_classification() {
        for (status, expected) in [(401, "auth"), (403, "auth"), (429, "rate")] {
            let transport = Scripted::new(vec![Ok(HttpResponse {
                status,
                headers: vec![],
                body: Vec::new(),
            })]);
            let report =
                MaxReportClient::new("report-key", "https://r.test", transport).expect("report");
            let error = report
                .totals("2026-10-01", "2026-10-01")
                .await
                .expect_err("status error");
            match (&error, expected) {
                (PlatformError::Auth { status: seen, .. }, "auth") if *seen == status => {}
                (PlatformError::RateLimited { .. }, "rate") => {}
                _ => panic!("{status} classified as {error:?}"),
            }
            assert!(!error.is_retryable() || expected == "rate");
        }
    }

    #[tokio::test]
    async fn ids_and_keys_are_percent_encoded_in_paths_and_queries() {
        let unit = r#"{"id":"u/1 ?","name":"B","platform":"ios","package_name":"com.x","ad_format":"BANNER"}"#;
        let transport = Scripted::new(vec![ok(unit)]);
        let client = MaxClient::new("key", "https://api.test", transport.clone()).expect("client");
        let _ = client.get_ad_unit("u/1 ?").await.expect("unit");
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert!(
            calls[0].url.contains("/ad_unit/u%2F1%20%3F"),
            "{}",
            calls[0].url
        );

        let transport = Scripted::new(vec![ok(r#"{"code":200,"count":0,"results":[]}"#)]);
        let report = MaxReportClient::new("key with & space", "https://r.test/", transport.clone())
            .expect("report");
        let _ = report
            .totals("2026-10-01", "2026-10-01")
            .await
            .expect("empty totals");
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert!(
            calls[0].url.starts_with("https://r.test/maxReport?"),
            "{}",
            calls[0].url
        );
        assert!(
            calls[0].url.contains("api_key=key%20with%20%26%20space"),
            "{}",
            calls[0].url
        );
    }

    /// Live promotion driver for the provider simulator. Ignored by default:
    /// run it with `MAX_SIM_BASE`/`MAX_SIM_API_KEY` and `--ignored` after a
    /// real `apex-runner cycle` has created an experiment. It uses the real
    /// [`MaxClient`] promoted through `ReqwestTransport`, then the next audit
    /// reads the promoted floors from the management API.
    /// Live floor-write driver: read the unit, replace the whole floor list
    /// through the real [`MaxClient::update_floors`], and read it back. Run
    /// with `MAX_SIM_BASE`/`MAX_SIM_API_KEY` and `--ignored`; the simulator's
    /// state file is the external proof of the write.
    #[tokio::test]
    async fn live_update_floors_is_visible_on_the_next_read() {
        if std::env::var_os("MAX_SIM_BASE").is_none() {
            eprintln!("skipping live MAX driver: set MAX_SIM_BASE and MAX_SIM_API_KEY");
            return;
        }
        let base = std::env::var("MAX_SIM_BASE").expect("MAX_SIM_BASE");
        let key = std::env::var("MAX_SIM_API_KEY").expect("MAX_SIM_API_KEY");
        let unit_id = std::env::var("MAX_SIM_UNIT").unwrap_or_else(|_| "rw_demo_01".into());
        let target: i64 = std::env::var("MAX_SIM_FLOOR")
            .unwrap_or_else(|_| "1750000".into())
            .parse()
            .expect("MAX_SIM_FLOOR micros");
        let transport: Arc<dyn Transport> =
            Arc::new(crate::transport::ReqwestTransport::new().expect("reqwest transport"));
        let client = MaxClient::new(&key, &base, transport).expect("client");
        let unit = client.get_ad_unit(&unit_id).await.expect("unit");
        let mut floors = unit.country_floors.clone().unwrap_or_default();
        assert!(!floors.is_empty(), "the simulator unit has floors");
        floors[0].floor_ecpm = target;
        let updated = client
            .update_floors(&unit, &floors)
            .await
            .expect("floor write");
        assert_eq!(
            updated.country_floors.as_ref().expect("written floors")[0].floor_ecpm,
            target
        );
        let reread = client.get_ad_unit(&unit_id).await.expect("reread");
        assert_eq!(
            reread.country_floors.as_ref().expect("reread floors")[0].floor_ecpm,
            target
        );
    }

    #[tokio::test]
    async fn live_promote_the_simulator_experiment() {
        if std::env::var_os("MAX_SIM_BASE").is_none() {
            eprintln!("skipping live MAX driver: set MAX_SIM_BASE and MAX_SIM_API_KEY");
            return;
        }
        let base = std::env::var("MAX_SIM_BASE").expect("MAX_SIM_BASE");
        let key = std::env::var("MAX_SIM_API_KEY").expect("MAX_SIM_API_KEY");
        let unit = std::env::var("MAX_SIM_UNIT").unwrap_or_else(|_| "rw_demo_01".into());
        let transport: Arc<dyn Transport> =
            Arc::new(crate::transport::ReqwestTransport::new().expect("reqwest transport"));
        let client = MaxClient::new(&key, &base, transport).expect("client");
        let running = client.get_experiment(&unit).await.expect("experiment");
        assert_eq!(running.state, ExperimentState::Running);
        client.promote_experiment(&unit).await.expect("promote");
        let promoted = client.get_experiment(&unit).await.expect("experiment");
        assert_eq!(promoted.state, ExperimentState::Completed);
    }

    #[test]
    fn ecpm_formatting_keeps_decimal_shape() {
        assert_eq!(format_ecpm(1_000_000), "1");
        assert_eq!(format_ecpm(10_500_000), "10.5");
        assert_eq!(format_ecpm(12_345_678), "12.345678");
    }
    /// The pre-optimization floor grouping: a `Vec::contains`/`position`
    /// scan per floor. Kept verbatim as the reference the single-pass fold
    /// must equal.
    fn floor_groups_naive(floors: &[apex_core::platform::CountryFloor]) -> Vec<serde_json::Value> {
        let mut groups: Vec<serde_json::Value> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for floor in floors {
            let key = floor.group.clone().unwrap_or_else(|| "default".to_string());
            if let Some(pos) = seen.iter().position(|name| *name == key) {
                let countries = groups[pos]["countries"]["values"]
                    .as_array_mut()
                    .expect("fixture group keeps its countries list");
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
        groups
    }

    #[test]
    fn floor_grouping_equals_the_naive_scan_at_ten_thousand_floors() {
        use apex_core::platform::CountryFloor;
        // 200 groups x 50 floors each, with repeated countries and repeated
        // group names interleaved: first-seen group order, first-seen country
        // order and the first floor's cpm must all survive the single pass.
        let floors: Vec<CountryFloor> = (0..10_000)
            .map(|i| CountryFloor {
                country: format!("C{:03}", (i / 200) % 60),
                floor_ecpm: 1_000_000 + (i as i64 / 50),
                group: Some(format!("g{:03}", i % 200)),
            })
            .collect();
        // One group with unique countries: the per-country dedup axis.
        let mut wide = floors.clone();
        wide.truncate(3_000);
        for (i, floor) in wide.iter_mut().enumerate() {
            floor.group = Some("everything".to_string());
            floor.country = format!("U{i:05}");
        }
        for case in [&floors, &wide] {
            let optimized = floor_groups(case).expect("single-pass grouping");
            let reference = floor_groups_naive(case);
            assert_eq!(optimized, reference);
        }
        let grouped = floor_groups(&floors).expect("grouping");
        assert_eq!(grouped.len(), 200, "one entry per distinct group");
        let first = &grouped[0];
        assert_eq!(first["country_group_name"], "g000");
        assert_eq!(first["cpm"], "1"); // 1_000_000 micros = 1.0 unit
        let countries = first["countries"]["values"].as_array().expect("array");
        assert!(countries.len() <= 60, "countries dedup inside the group");
    }
    #[tokio::test]
    async fn user_revenue_day_fetches_the_csv_and_folds_per_player() {
        let csv = "Ad Unit ID,Placement,IDFA,IDFV,User Id,Impressions,Revenue\n\
u1,home,idfa-a,idfv-a,user-1,3,1.500000\n\
u1,home,idfa-a,idfv-a,user-1,2,0.250000\n\
u2,level,idfa-b,idfv-b,,4,0.000079\n\
u1,home,00000000-0000-0000-0000-000000000000,idfv-c,,1,0.100000\n";
        let transport = Scripted::new(vec![
            ok(r#"{"status":200,"ad_revenue_report_url":"https://reports.test/user.csv"}"#),
            ok(csv),
        ]);
        let report = MaxReportClient::new("report-key", "https://r.test", transport.clone())
            .expect("report");
        let rows = report
            .user_revenue_day("2026-10-01", "android", "com.example.game")
            .await
            .expect("rows");
        // user-1 folds across both ad-unit rows; the IDFV-only player resolves
        // through IDFV; the zeroed IDFA row resolves through its IDFV.
        assert_eq!(rows.len(), 3, "{rows:?}");
        let user = rows
            .iter()
            .find(|row| row.player_key().as_deref() == Some("user-1"))
            .expect("user");
        assert_eq!(user.revenue, 1_750_000);
        assert_eq!(user.impressions, 5);
        assert_eq!(user.day, "2026-10-01");
        assert_eq!(user.platform, "android");
        assert!(rows.iter().all(|row| row.player_key().is_some()));
        // The feed is player-keyed, never one row per ad unit.
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].url.contains("/max/userAdRevenueReport"));
        assert!(calls[0].url.contains("aggregated=true"));
        assert!(calls[0].url.contains("date=2026-10-01"));
        assert!(calls[0].url.contains("platform=android"));
        assert_eq!(calls[1].url, "https://reports.test/user.csv");
    }

    #[tokio::test]
    async fn user_revenue_day_refuses_a_non_player_key_and_bad_requests() {
        let report = MaxReportClient::new("report-key", "https://r.test", Scripted::new(vec![]))
            .expect("report");
        assert!(report
            .user_revenue_day("2026-10-01", "steam", "com.x")
            .await
            .is_err());
        assert!(report
            .user_revenue_day("2026-13-01", "android", "com.x")
            .await
            .is_err());
        assert!(parse_user_revenue_csv(b"Revenue\n1.0\n", "2026-10-01", "android").is_err());
        assert!(
            parse_user_revenue_csv(b"User Id,Revenue\n", "2026-10-01", "android")
                .expect("empty")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn user_revenue_day_rejects_a_report_without_a_url() {
        let transport = Scripted::new(vec![ok(r#"{"status":200}"#)]);
        let report =
            MaxReportClient::new("report-key", "https://r.test", transport).expect("report");
        assert!(report
            .user_revenue_day("2026-10-01", "ios", "com.x")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn max_cap_experiment_body_is_the_documented_test_group_shape() {
        let transport = Scripted::new(vec![ok(r#"{"id":"u1"}"#)]);
        let client = client(transport.clone());
        let spec = ExperimentSpec {
            name: "autopilot-cap-u1".into(),
            instance_id: "i1".into(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "cap interstitial ads".into(),
            test_group_allocation: 25,
        };
        let id = client
            .create_experiment_with_caps(
                &spec,
                &[MaxFrequencyCap {
                    cap_type: MaxCapFrequencyType::Time,
                    countries: vec!["us".into(), "ca".into()],
                    session_limit: None,
                    day_limit: Some(10),
                    minute_frequency: Some(30),
                }],
            )
            .await
            .expect("create");
        assert_eq!(id, "u1");
        let calls = transport.calls.lock().map(|c| c.clone()).unwrap();
        assert_eq!(calls.len(), 1);
        let body = match calls[0].body.as_ref().expect("body") {
            crate::transport::HttpBody::Json(value) => value.clone(),
            other => panic!("expected JSON body, got {other:?}"),
        };
        assert_eq!(body["test_group_allocation"], 25);
        let caps = body["frequency_capping_settings"].as_array().expect("caps");
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0]["type"], "time");
        assert_eq!(caps[0]["time_capping_settings"]["day_limit"], 10);
        assert_eq!(caps[0]["time_capping_settings"]["minute_frequency"], 30);
        assert_eq!(caps[0]["session_capping_settings"]["session_limit"], 0);
        assert_eq!(caps[0]["countries"]["type"], "INCLUDE");
        assert_eq!(
            caps[0]["countries"]["values"],
            serde_json::json!(["us", "ca"])
        );
    }

    #[tokio::test]
    async fn max_cap_country_lists_must_not_overlap() {
        let transport = Scripted::new(vec![]);
        let client = client(transport.clone());
        let spec = ExperimentSpec {
            name: "autopilot-cap-u1".into(),
            instance_id: "i1".into(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "cap interstitial ads".into(),
            test_group_allocation: 25,
        };
        let error = client
            .create_experiment_with_caps(
                &spec,
                &[
                    MaxFrequencyCap {
                        cap_type: MaxCapFrequencyType::Session,
                        countries: vec!["US".into()],
                        session_limit: Some(4),
                        day_limit: None,
                        minute_frequency: None,
                    },
                    MaxFrequencyCap {
                        cap_type: MaxCapFrequencyType::Time,
                        countries: vec!["us".into()],
                        session_limit: None,
                        day_limit: Some(10),
                        minute_frequency: None,
                    },
                ],
            )
            .await
            .expect_err("overlap");
        assert!(error.to_string().contains("disjoint"), "{error}");
        assert!(transport.calls.lock().unwrap().is_empty(), "no write");
    }

    #[test]
    fn user_revenue_csv_keeps_quoted_commas_from_shifting_columns() {
        // A placement name with a comma makes a naive splitter see three
        // fields in a two-column row: the revenue column would silently
        // disappear (or worse, shift onto another column).
        let csv = "User Id,Placement,Revenue\n\
u1,\"foo, bar\",0.10\n";
        let rows = parse_user_revenue_csv(csv.as_bytes(), "2026-10-01", "android")
            .expect("quoted placement parses");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].revenue, 100_000, "revenue stays in its column");

        // The same row with a doubled quote inside the field.
        let csv = "User Id,Placement,Revenue\n\
u1,\"say \"\"hi\"\"\",0.30\n";
        let rows = parse_user_revenue_csv(csv.as_bytes(), "2026-10-01", "android")
            .expect("escaped quote parses");
        assert_eq!(rows[0].revenue, 300_000);
    }

    #[test]
    fn user_revenue_csv_refuses_a_player_in_two_arms() {
        let csv = "User Id,Max Ad Unit Test,Revenue\n\
u1,Control,0.10\n\
u1,Test,0.20\n";
        let error = parse_user_revenue_csv(csv.as_bytes(), "2026-10-01", "android")
            .expect_err("cross-arm player");
        assert!(error.contains("more than one experiment arm"), "{error}");
    }
}
