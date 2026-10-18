//! AdMob management client (OAuth2 bearer; mediation groups + mappings + eCPM).
//!
//! Mediation groups and mappings live on the **v1beta** API under
//! `/v1beta/{account}/mediationGroups`. Mediation A/B experiments use
//! `mediationAbExperiments` create and `:stop` on the same v1beta surface.
//!
//! AdMob documents **no revenue (or any other per-variant metric) for
//! mediation A/B experiments** — only a [`MediationAbVariantLeader`] verdict.
//! [`ADMOB_REVENUE_PER_VARIANT`] is therefore `false` and every parsed
//! experiment carries that same flag, so no caller can mistake AdMob for a
//! measurable-lift platform. The fee exclusion this implies is the runner's
//! lift-path job, not the connector's.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use apex_core::endpoint::HttpMethod as CoreMethod;
use apex_core::{
    admob_ecpm_key, validate_ecpm, AdFormat, EndpointMeta, Mapping, MediationGroup, PlatformError,
    PlatformId, SourceVerification,
};

use crate::transport::{HttpMethod, PlatformCaller, Transport};

const MAX_PAGES: usize = 50;

pub const ADMOB_ENDPOINTS: [EndpointMeta; 6] = [
    EndpointMeta::new(
        "listMediationGroups",
        CoreMethod::Get,
        SourceVerification::Documented,
        true,
    ),
    EndpointMeta::new(
        "getMediationGroup",
        CoreMethod::Get,
        SourceVerification::Documented,
        true,
    ),
    EndpointMeta::new(
        "mediationReport",
        CoreMethod::Post,
        SourceVerification::VerifyLive,
        true,
    ),
    EndpointMeta::new(
        "updateMediationGroup",
        CoreMethod::Patch,
        SourceVerification::VerifyLive,
        false,
    ),
    EndpointMeta::new(
        "createMediationAbExperiment",
        CoreMethod::Post,
        SourceVerification::VerifyLive,
        false,
    ),
    EndpointMeta::new(
        "stopMediationAbExperiment",
        CoreMethod::Post,
        SourceVerification::VerifyLive,
        false,
    ),
];

/// AdMob exposes a `variantLeader` verdict but **no revenue per variant** for
/// mediation A/B experiments, so AdMob can never yield a measured lift. This
/// is the typed capability flag the lift path consults; the fee exclusion it
/// implies is enforced in the runner wave, not here.
pub const ADMOB_REVENUE_PER_VARIANT: bool = false;

/// State of a mediation A/B experiment as AdMob reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediationAbExperimentState {
    Running,
    Stopped,
}

/// The leader verdict AdMob publishes for a mediation A/B experiment.
///
/// This is the only performance signal the API documents for an experiment: no
/// revenue, impressions or any other per-variant metric is exposed. Converting
/// a verdict into measured lift would fabricate data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediationAbVariantLeader {
    /// AdMob has not set a leader (e.g. the experiment is still running).
    Unspecified,
    /// AdMob cannot determine a leader.
    Unknown,
    /// The control arm leads.
    Control,
    /// The variant arm leads.
    Variant,
}

/// One mediation A/B experiment as the v1beta API reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediationAbExperiment {
    /// Full resource name, `accounts/{publisher}/mediationAbExperiments/{id}`.
    pub name: String,
    pub mediation_group_id: String,
    pub display_name: String,
    pub state: MediationAbExperimentState,
    /// The verdict; `None` when the API carries no `variantLeader` field yet.
    pub variant_leader: Option<MediationAbVariantLeader>,
    /// Always [`ADMOB_REVENUE_PER_VARIANT`] (`false`): no per-variant revenue
    /// is exposed, so no measured lift can be computed from this platform.
    pub revenue_per_variant: bool,
}

pub fn normalize_account_id(account_id: &str) -> Result<String, PlatformError> {
    let trimmed = account_id.trim();
    let with_prefix = if trimmed.starts_with("accounts/") {
        trimmed.to_string()
    } else {
        format!("accounts/{trimmed}")
    };
    let valid = with_prefix
        .strip_prefix("accounts/pub-")
        .map(|digits| digits.len() >= 6 && digits.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or(false);
    if valid {
        Ok(with_prefix)
    } else {
        Err(PlatformError::Validation(format!(
            "invalid AdMob account id '{account_id}' (expected accounts/pub-<digits>)"
        )))
    }
}

pub struct AdmobClient {
    caller: PlatformCaller,
    account_id: String,
    /// One mediation report total per inclusive date window. The audit and the
    /// billing path can both ask for the same 30-day window; the cache keeps
    /// that one platform read, exactly like LevelPlay's `app_revenue`.
    revenue: Mutex<BTreeMap<(String, String), i64>>,
}

impl std::fmt::Debug for AdmobClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmobClient")
            .field("platform", &self.caller.platform)
            .field("account_id", &self.account_id)
            .finish()
    }
}

impl AdmobClient {
    pub fn new(
        access_token: &str,
        account_id: &str,
        base_url: &str,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, PlatformError> {
        if access_token.trim().is_empty() {
            return Err(PlatformError::Validation(
                "AdMob credentials require accessToken and accountId".into(),
            ));
        }
        Ok(Self {
            account_id: normalize_account_id(account_id)?,
            caller: PlatformCaller::new(
                PlatformId::Admob,
                base_url.to_string(),
                transport,
                ("Authorization".into(), format!("Bearer {access_token}")),
            ),
            revenue: Mutex::new(BTreeMap::new()),
        })
    }

    fn schema(&self, operation: &str, detail: impl Into<String>) -> PlatformError {
        PlatformError::Schema {
            platform: PlatformId::Admob,
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    pub async fn list_mediation_groups(&self) -> Result<Vec<MediationGroup>, PlatformError> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        for _page in 0..MAX_PAGES {
            let path = match &page_token {
                None => format!("/v1beta/{}/mediationGroups", self.account_id),
                Some(token) => format!(
                    "/v1beta/{}/mediationGroups?pageToken={}",
                    self.account_id,
                    crate::encode_query_component(token)
                ),
            };
            let data = self
                .caller
                .call("listMediationGroups", HttpMethod::Get, &path, None, None)
                .await?;
            let groups = data
                .get("mediationGroups")
                .and_then(|v| v.as_array())
                .ok_or_else(|| {
                    self.schema(
                        "listMediationGroups",
                        "response missing array 'mediationGroups'",
                    )
                })?;
            for item in groups {
                out.push(self.parse_group(item)?);
            }
            match data.get("nextPageToken").and_then(|v| v.as_str()) {
                None | Some("") => return Ok(out),
                Some(token) => page_token = Some(token.to_string()),
            }
        }
        Err(self.schema(
            "listMediationGroups",
            format!("pagination exceeded {MAX_PAGES} pages (runaway nextPageToken)"),
        ))
    }

    fn parse_group(&self, item: &serde_json::Value) -> Result<MediationGroup, PlatformError> {
        let id = item
            .get("mediationGroupId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                self.schema("listMediationGroups", format!("group missing id: {item}"))
            })?;
        let name = item.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            self.schema("listMediationGroups", format!("group missing name: {item}"))
        })?;
        let mappings_raw = item
            .get("adUnitMappings")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                self.schema(
                    "listMediationGroups",
                    format!("group missing adUnitMappings: {item}"),
                )
            })?;
        let mut mappings = Vec::with_capacity(mappings_raw.len());
        for m in mappings_raw {
            let ad_unit_id = m.get("adUnitId").and_then(|v| v.as_str()).ok_or_else(|| {
                self.schema(
                    "listMediationGroups",
                    format!("mapping missing adUnitId: {m}"),
                )
            })?;
            let network = m.get("network").and_then(|v| v.as_str()).ok_or_else(|| {
                self.schema(
                    "listMediationGroups",
                    format!("mapping missing network: {m}"),
                )
            })?;
            let ecpm = m
                .get("ecpm")
                .or_else(|| m.get("eCPM"))
                .and_then(crate::max::number_to_micros)
                .ok_or_else(|| {
                    self.schema("listMediationGroups", format!("mapping missing ecpm: {m}"))
                })?;
            mappings.push(Mapping {
                ad_unit_id: ad_unit_id.to_string(),
                network: network.to_string(),
                ecpm,
            });
        }
        // Missing 'format' defaults to banner for parity with the TypeScript
        // reference; an unknown string fails closed instead (the TS cast did not).
        let format = match item.get("format").and_then(|v| v.as_str()) {
            Some("banner") | None => AdFormat::Banner,
            Some("interstitial") => AdFormat::Interstitial,
            Some("rewarded") => AdFormat::Rewarded,
            Some("native") => AdFormat::Native,
            Some("app_open") => AdFormat::AppOpen,
            Some(other) => {
                return Err(self.schema("listMediationGroups", format!("unknown format '{other}'")))
            }
        };
        // Duplicate adUnitId entries make the read-modify-write ambiguous
        // (which mapping does "the" update target, and the platform would
        // receive both changed). Refuse rather than guess.
        let mut seen_ids = std::collections::BTreeSet::new();
        for mapping in &mappings {
            if !seen_ids.insert(mapping.ad_unit_id.as_str()) {
                return Err(self.schema(
                    "listMediationGroups",
                    format!(
                        "duplicate adUnitId '{}' inside group '{id}'",
                        mapping.ad_unit_id
                    ),
                ));
            }
        }
        Ok(MediationGroup {
            mediation_group_id: id.to_string(),
            name: name.to_string(),
            format,
            mappings,
        })
    }

    /// Read-modify-write of the full mapping list (the API replaces the whole list).
    pub async fn update_mapping_ecpm(
        &self,
        mediation_group_id: &str,
        ad_unit_id: &str,
        ecpm: i64,
    ) -> Result<Mapping, PlatformError> {
        let new_ecpm = validate_ecpm(ecpm)?;
        let groups = self.list_mediation_groups().await?;
        let group = groups
            .iter()
            .find(|g| g.mediation_group_id == mediation_group_id)
            .ok_or_else(|| {
                self.schema(
                    "updateMediationGroup",
                    format!("mediation group '{mediation_group_id}' not found"),
                )
            })?;
        let target = group
            .mappings
            .iter()
            .find(|m| m.ad_unit_id == ad_unit_id)
            .ok_or_else(|| {
                self.schema(
                    "updateMediationGroup",
                    format!("ad unit '{ad_unit_id}' not in group '{mediation_group_id}'"),
                )
            })?;
        let updated: Vec<serde_json::Value> = group
            .mappings
            .iter()
            .map(|m| {
                let value = if m.ad_unit_id == ad_unit_id {
                    new_ecpm
                } else {
                    m.ecpm
                };
                serde_json::json!({
                    "adUnitId": m.ad_unit_id,
                    "network": m.network,
                    "ecpm": value,
                })
            })
            .collect();
        let path = format!(
            "/v1beta/{}/mediationGroups/{}",
            self.account_id,
            crate::encode_query_component(mediation_group_id)
        );
        let key = admob_ecpm_key(mediation_group_id, ad_unit_id, new_ecpm);
        self.caller
            .call(
                "updateMediationGroup",
                HttpMethod::Patch,
                &path,
                Some(serde_json::json!({ "adUnitMappings": updated })),
                Some(&key),
            )
            .await?;
        Ok(Mapping {
            ad_unit_id: ad_unit_id.to_string(),
            network: target.network.clone(),
            ecpm: new_ecpm,
        })
    }

    /// The app's estimated mediation earnings over the inclusive window
    /// `[start_date, end_date]`, in micros.
    ///
    /// This is the documented v1beta `mediationReport:generate` read with the
    /// `ESTIMATED_EARNINGS` metric only. The report returns money in
    /// `microsValue` (a string of integer micros); the parser also accepts the
    /// other documented metric-value spellings so a recorded fixture from
    /// either surface parses without losing exactness. A missing `rows` array
    /// or a row without the metric is a typed refusal, never a silent zero;
    /// an empty `rows` array is a real zero. Totals are checked sums.
    pub async fn mediation_revenue(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<i64, PlatformError> {
        let start = report_date(start_date, "start_date")?;
        let end = report_date(end_date, "end_date")?;
        let key = (start_date.to_string(), end_date.to_string());
        if let Some(total) = self
            .revenue
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).copied())
        {
            return Ok(total);
        }
        let path = format!("/v1beta/{}/mediationReport:generate", self.account_id);
        let data = self
            .caller
            .call(
                "mediationReport",
                HttpMethod::Post,
                &path,
                Some(serde_json::json!({
                    "startDate": { "year": start.0, "month": start.1, "day": start.2 },
                    "endDate": { "year": end.0, "month": end.1, "day": end.2 },
                    "dimensions": ["DATE"],
                    "metrics": ["ESTIMATED_EARNINGS"],
                    "currencyCode": "USD",
                    // The DATE dimension is one row per day (at most 31 in the
                    // audit window); the page is explicit so a short default
                    // can never truncate the total.
                    "pageSize": 1000,
                })),
                None,
            )
            .await?;
        if data
            .get("nextPageToken")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|token| !token.is_empty())
        {
            return Err(self.schema(
                "mediationReport",
                "response carries nextPageToken; refusing a truncated revenue total",
            ));
        }
        let rows = data
            .get("rows")
            .and_then(serde_json::Value::as_array)
            .or_else(|| {
                data.get("mediationReportRows")
                    .and_then(serde_json::Value::as_array)
            })
            .ok_or_else(|| self.schema("mediationReport", "response has no 'rows' array"))?;
        let mut total: i64 = 0;
        for row in rows {
            let value = row
                .get("metricValues")
                .and_then(serde_json::Value::as_object)
                .and_then(|values| values.get("ESTIMATED_EARNINGS"))
                .ok_or_else(|| {
                    self.schema(
                        "mediationReport",
                        format!("report row missing ESTIMATED_EARNINGS: {row}"),
                    )
                })?;
            let revenue = report_metric_micros(value).ok_or_else(|| {
                self.schema(
                    "mediationReport",
                    format!("report row has an unparseable ESTIMATED_EARNINGS: {value}"),
                )
            })?;
            total = total
                .checked_add(revenue)
                .ok_or_else(|| PlatformError::Validation("revenue total overflow".into()))?;
        }
        if let Ok(mut cache) = self.revenue.lock() {
            cache.insert(key, total);
        }
        Ok(total)
    }

    /// Start a mediation A/B experiment on one mediation group (v1beta
    /// `mediationAbExperiments` create). The returned typed experiment carries
    /// [`ADMOB_REVENUE_PER_VARIANT`] = `false`: AdMob publishes only a
    /// `variantLeader` verdict, never per-variant revenue.
    pub async fn create_mediation_ab_experiment(
        &self,
        mediation_group_id: &str,
        display_name: &str,
    ) -> Result<MediationAbExperiment, PlatformError> {
        if mediation_group_id.trim().is_empty() || display_name.trim().is_empty() {
            return Err(PlatformError::Validation(
                "AdMob createMediationAbExperiment requires a mediation group id and a display name"
                    .into(),
            ));
        }
        let path = format!("/v1beta/{}/mediationAbExperiments", self.account_id);
        let data = self
            .caller
            .call(
                "createMediationAbExperiment",
                HttpMethod::Post,
                &path,
                Some(serde_json::json!({
                    "mediationGroupId": mediation_group_id,
                    "displayName": display_name,
                })),
                None,
            )
            .await?;
        self.parse_experiment("createMediationAbExperiment", &data)
    }

    /// Stop a mediation A/B experiment by its id (the v1beta `:stop` custom
    /// method). The returned verdict is still only a `variantLeader`; see
    /// [`Self::create_mediation_ab_experiment`] for the capability flag.
    pub async fn stop_mediation_ab_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<MediationAbExperiment, PlatformError> {
        if experiment_id.trim().is_empty() {
            return Err(PlatformError::Validation(
                "AdMob stopMediationAbExperiment requires an experiment id".into(),
            ));
        }
        let path = format!(
            "/v1beta/{}/mediationAbExperiments/{}:stop",
            self.account_id,
            crate::encode_query_component(experiment_id)
        );
        let data = self
            .caller
            .call(
                "stopMediationAbExperiment",
                HttpMethod::Post,
                &path,
                None,
                None,
            )
            .await?;
        self.parse_experiment("stopMediationAbExperiment", &data)
    }

    /// Parse the shared v1beta experiment resource shape. A missing required
    /// field or an unknown enum value is a typed refusal; the verdict stays
    /// `None` when the API carries no `variantLeader` field, and
    /// `revenue_per_variant` is pinned to [`ADMOB_REVENUE_PER_VARIANT`].
    fn parse_experiment(
        &self,
        operation: &str,
        data: &serde_json::Value,
    ) -> Result<MediationAbExperiment, PlatformError> {
        let name = data
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| self.schema(operation, format!("experiment missing name: {data}")))?;
        let mediation_group_id = data
            .get("mediationGroupId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                self.schema(
                    operation,
                    format!("experiment missing mediationGroupId: {data}"),
                )
            })?;
        let display_name = data
            .get("displayName")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                self.schema(operation, format!("experiment missing displayName: {data}"))
            })?;
        let state = match data.get("state").and_then(|v| v.as_str()) {
            Some("RUNNING") => MediationAbExperimentState::Running,
            Some("STOPPED") => MediationAbExperimentState::Stopped,
            Some(other) => {
                return Err(self.schema(operation, format!("unknown experiment state '{other}'")))
            }
            None => return Err(self.schema(operation, format!("experiment missing state: {data}"))),
        };
        let variant_leader = match data.get("variantLeader") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(raw)) => Some(match raw.as_str() {
                "VARIANT_LEADER_UNSPECIFIED" => MediationAbVariantLeader::Unspecified,
                "UNKNOWN" => MediationAbVariantLeader::Unknown,
                "CONTROL" => MediationAbVariantLeader::Control,
                "VARIANT" => MediationAbVariantLeader::Variant,
                other => {
                    return Err(self.schema(operation, format!("unknown variantLeader '{other}'")))
                }
            }),
            Some(other) => {
                return Err(
                    self.schema(operation, format!("variantLeader is not a string: {other}"))
                )
            }
        };
        Ok(MediationAbExperiment {
            name: name.to_string(),
            mediation_group_id: mediation_group_id.to_string(),
            display_name: display_name.to_string(),
            state,
            variant_leader,
            revenue_per_variant: ADMOB_REVENUE_PER_VARIANT,
        })
    }
}

/// Parse `YYYY-MM-DD` into the `(year, month, day)` triple the AdMob report
/// request body carries. A malformed date is a typed validation refusal before
/// any request goes out.
fn report_date(date: &str, field: &str) -> Result<(i64, i64, i64), PlatformError> {
    let mut parts = date.split('-');
    let year_raw = parts.next();
    let month_raw = parts.next();
    let day_raw = parts.next();
    let tail = parts.next();
    let parsed = match (year_raw, month_raw, day_raw) {
        (Some(year), Some(month), Some(day))
            if year.len() == 4 && month.len() == 2 && day.len() == 2 =>
        {
            match (
                year.parse::<i64>(),
                month.parse::<i64>(),
                day.parse::<i64>(),
            ) {
                (Ok(year), Ok(month), Ok(day))
                    if (1..=12).contains(&month) && (1..=31).contains(&day) =>
                {
                    Some((year, month, day))
                }
                _ => None,
            }
        }
        _ => None,
    };
    match (parsed, tail) {
        (Some((year, month, day)), None) => Ok((year, month, day)),
        _ => Err(PlatformError::Validation(format!(
            "AdMob mediationReport {field} '{date}' is not YYYY-MM-DD"
        ))),
    }
}

/// Parse one AdMob report metric value exactly into micros.
///
/// The documented report value is `{"microsValue": "<integer micros>"}`. The
/// other spellings the same v1beta surface uses are accepted without guessing:
/// `integerValue` is integer micros, `doubleValue` is a currency amount
/// rounded to the nearest micro, and a bare string is the currency-string
/// convention shared with the MAX and LevelPlay reports (parsed exactly into
/// micros). A bare number is read as whole micros.
fn report_metric_micros(value: &serde_json::Value) -> Option<i64> {
    if let Some(raw) = value.as_str() {
        return crate::parse_decimal_micros(raw).ok();
    }
    if let Some(micros) = value.as_i64() {
        return Some(micros);
    }
    if let Some(amount) = value.as_f64() {
        if amount.is_finite() {
            return Some((amount * 1_000_000.0).round() as i64);
        }
    }
    let object = value.as_object()?;
    if let Some(raw) = object
        .get("microsValue")
        .and_then(serde_json::Value::as_str)
    {
        return raw.parse::<i64>().ok();
    }
    if let Some(micros) = object
        .get("microsValue")
        .and_then(serde_json::Value::as_i64)
    {
        return Some(micros);
    }
    if let Some(raw) = object
        .get("integerValue")
        .and_then(serde_json::Value::as_str)
    {
        return raw.parse::<i64>().ok();
    }
    if let Some(micros) = object
        .get("integerValue")
        .and_then(serde_json::Value::as_i64)
    {
        return Some(micros);
    }
    if let Some(amount) = object
        .get("doubleValue")
        .and_then(serde_json::Value::as_f64)
    {
        if amount.is_finite() {
            return Some((amount * 1_000_000.0).round() as i64);
        }
    }
    None
}
