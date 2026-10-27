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

pub const ADMOB_ENDPOINTS: [EndpointMeta; 7] = [
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
        "getMediationAbExperiment",
        CoreMethod::Get,
        SourceVerification::Documented,
        true,
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
///
/// The documented v1beta enum is `RUNNING`, `ENDED`, `EXPIRED` (a stopped
/// experiment is `ENDED`). `STOPPED` is accepted as a compatibility spelling
/// for recorded fixtures that used it before the canonical enum was pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediationAbExperimentState {
    Running,
    Stopped,
    Ended,
    Expired,
}

impl MediationAbExperimentState {
    /// The platform's own spelling, for records and outcomes.
    pub fn as_str(&self) -> &'static str {
        match self {
            MediationAbExperimentState::Running => "RUNNING",
            MediationAbExperimentState::Stopped => "STOPPED",
            MediationAbExperimentState::Ended => "ENDED",
            MediationAbExperimentState::Expired => "EXPIRED",
        }
    }

    /// Whether the experiment has ended (every state but `RUNNING`).
    pub fn is_ended(&self) -> bool {
        !matches!(self, MediationAbExperimentState::Running)
    }
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
    /// AdMob cannot determine a leader (legacy spelling `UNKNOWN`).
    Unknown,
    /// The control arm leads.
    Control,
    /// The variant/treatment arm leads.
    Variant,
    /// The documented `TREATMENT` spelling (the variant arm leads).
    Treatment,
    /// Collected impressions are insufficient to determine a leader.
    InsufficientData,
    /// The experiment has not run long enough to call a leader.
    TooEarlyToCall,
    /// Neither variant is a decisive winner.
    NoVariantLeader,
}

impl MediationAbVariantLeader {
    /// The documented v1beta spelling (legacy fixtures keep their spelling).
    pub fn as_str(&self) -> &'static str {
        match self {
            MediationAbVariantLeader::Unspecified => "VARIANT_LEADER_UNSPECIFIED",
            MediationAbVariantLeader::Unknown => "UNKNOWN",
            MediationAbVariantLeader::Control => "CONTROL",
            MediationAbVariantLeader::Variant => "VARIANT",
            MediationAbVariantLeader::Treatment => "TREATMENT",
            MediationAbVariantLeader::InsufficientData => "INSUFFICIENT_DATA",
            MediationAbVariantLeader::TooEarlyToCall => "TOO_EARLY_TO_CALL",
            MediationAbVariantLeader::NoVariantLeader => "NO_VARIANT_LEADER",
        }
    }

    /// Whether the verdict keeps the treatment arm. Only `TREATMENT` (and its
    /// legacy `VARIANT` spelling) is a treatment win; every other verdict
    /// leaves the original control lines.
    pub fn keeps_treatment(&self) -> bool {
        matches!(
            self,
            MediationAbVariantLeader::Treatment | MediationAbVariantLeader::Variant
        )
    }
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
    /// Experiment id -> parent mediation group id, recorded when the
    /// experiment is created. The documented `:stop` custom method is nested
    /// under the mediation group; the adapter's ledger stores the resource
    /// name but its stop path trims to the trailing id, so the mapping keeps
    /// the stop call on the documented route for a live-created experiment.
    experiments: Mutex<BTreeMap<String, String>>,
}

impl std::fmt::Debug for AdmobClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmobClient")
            .field("platform", &self.caller.platform)
            .field("account_id", &self.account_id)
            .finish()
    }
}

/// Exchange a Google OAuth refresh token for a fresh AdMob access token.
///
/// AdMob access tokens expire after about an hour; the runner is a
/// short-lived process per job, so exchanging once at startup is enough.
pub async fn refresh_access_token(
    transport: std::sync::Arc<dyn crate::transport::Transport>,
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<String, PlatformError> {
    if token_url.trim().is_empty()
        || client_id.trim().is_empty()
        || client_secret.trim().is_empty()
        || refresh_token.trim().is_empty()
    {
        return Err(PlatformError::Validation(
            "AdMob token refresh requires the token URL, client id, client secret and refresh token"
                .into(),
        ));
    }
    let form = format!(
        "grant_type=refresh_token&client_id={}&client_secret={}&refresh_token={}",
        crate::encode_query_component(client_id),
        crate::encode_query_component(client_secret),
        crate::encode_query_component(refresh_token),
    );
    let request = crate::transport::HttpRequest {
        platform: apex_core::PlatformId::Admob,
        operation: "oauthRefresh".to_string(),
        method: crate::transport::HttpMethod::Post,
        url: token_url.to_string(),
        headers: vec![(
            "Content-Type".to_string(),
            crate::transport::FORM_CONTENT_TYPE.to_string(),
        )],
        body: Some(crate::transport::HttpBody::Form(form)),
        timeout_ms: 15_000,
    };
    let response = match transport.request(request).await {
        Ok(response) => response,
        Err(crate::transport::TransportError::Platform(error)) => return Err(error),
        Err(error) => {
            return Err(PlatformError::Network {
                platform: apex_core::PlatformId::Admob,
                operation: "oauthRefresh".to_string(),
                detail: error.to_string(),
            })
        }
    };
    if !(200..300).contains(&response.status) {
        return Err(match response.status {
            401 | 403 => PlatformError::Auth {
                platform: apex_core::PlatformId::Admob,
                operation: "oauthRefresh".to_string(),
                status: response.status,
            },
            429 => PlatformError::RateLimited {
                platform: apex_core::PlatformId::Admob,
                operation: "oauthRefresh".to_string(),
                retry_after: None,
            },
            status => PlatformError::Http {
                platform: apex_core::PlatformId::Admob,
                operation: "oauthRefresh".to_string(),
                status,
            },
        });
    }
    let data: serde_json::Value =
        serde_json::from_slice(&response.body).map_err(|error| PlatformError::Schema {
            platform: apex_core::PlatformId::Admob,
            operation: "oauthRefresh".to_string(),
            detail: format!("token response is not JSON: {error}"),
        })?;
    data.get("access_token")
        .and_then(|value| value.as_str())
        .filter(|token| !token.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| PlatformError::Schema {
            platform: apex_core::PlatformId::Admob,
            operation: "oauthRefresh".to_string(),
            detail: "token response has no access_token".to_string(),
        })
}

/// Synchronous wrapper for the runner builder, which is not async. The
/// exchange runs on its own current-thread runtime; the caller's runtime is
/// never entered from inside it.
pub fn refresh_access_token_blocking(
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<String, PlatformError> {
    let token_url = token_url.to_string();
    let client_id = client_id.to_string();
    let client_secret = client_secret.to_string();
    let refresh_token = refresh_token.to_string();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| PlatformError::Validation(format!("token runtime: {error}")))?;
        runtime.block_on(async move {
            let transport = crate::transport::ReqwestTransport::new()
                .map_err(|error| PlatformError::Validation(format!("token transport: {error}")))?;
            refresh_access_token(
                std::sync::Arc::new(transport),
                &token_url,
                &client_id,
                &client_secret,
                &refresh_token,
            )
            .await
        })
    })
    .join()
    .map_err(|_| PlatformError::Validation("token exchange thread panicked".to_string()))?
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
            experiments: Mutex::new(BTreeMap::new()),
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

    /// Read one mediation group by id (the `getMediationGroup` endpoint the
    /// capability table declares). The response uses the same resource shape
    /// as the list, so both the canonical `mediationGroupLines` form and the
    /// recorded legacy form parse through [`Self::parse_group`].
    pub async fn get_mediation_group(
        &self,
        mediation_group_id: &str,
    ) -> Result<MediationGroup, PlatformError> {
        if mediation_group_id.trim().is_empty() {
            return Err(PlatformError::Validation(
                "AdMob getMediationGroup requires a mediation group id".into(),
            ));
        }
        let path = format!(
            "/v1beta/{}/mediationGroups/{}",
            self.account_id,
            crate::encode_query_component(mediation_group_id)
        );
        let data = self
            .caller
            .call("getMediationGroup", HttpMethod::Get, &path, None, None)
            .await?;
        self.parse_group(&data)
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
        // Recorded fixtures use the pre-v1beta `adUnitMappings` array; the
        // documented v1beta resource carries a `mediationGroupLines` map keyed
        // by line id, with `cpmMicros` as integer micros. Both are read into
        // the same typed mapping list. A response with neither fails closed.
        let mut mappings = Vec::new();
        if let Some(mappings_raw) = item.get("adUnitMappings").and_then(|v| v.as_array()) {
            mappings.reserve(mappings_raw.len());
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
        } else if let Some(lines) = item.get("mediationGroupLines").and_then(|v| v.as_object()) {
            // Deterministic order: BTreeMap iteration by line id, then ad unit
            // id. A line groups every unit that shares its cpmMicros.
            for (line_id, line) in lines {
                if line.get("state").and_then(|v| v.as_str()) == Some("REMOVED") {
                    continue;
                }
                let network = line
                    .get("network")
                    .or_else(|| line.get("adSourceId"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        self.schema(
                            "listMediationGroups",
                            format!("line '{line_id}' missing network/adSourceId: {line}"),
                        )
                    })?;
                let ecpm = match line.get("cpmMicros") {
                    Some(raw) => parse_cpm_micros(raw).ok_or_else(|| {
                        self.schema(
                            "listMediationGroups",
                            format!("line '{line_id}' has an invalid cpmMicros: {raw}"),
                        )
                    })?,
                    None => line
                        .get("ecpm")
                        .or_else(|| line.get("eCPM"))
                        .and_then(crate::max::number_to_micros)
                        .ok_or_else(|| {
                            self.schema(
                                "listMediationGroups",
                                format!("line '{line_id}' missing cpmMicros/ecpm: {line}"),
                            )
                        })?,
                };
                let units = line
                    .get("adUnitMappings")
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| {
                        self.schema(
                            "listMediationGroups",
                            format!("line '{line_id}' missing adUnitMappings: {line}"),
                        )
                    })?;
                for ad_unit_id in units.keys() {
                    mappings.push(Mapping {
                        ad_unit_id: ad_unit_id.clone(),
                        network: network.to_string(),
                        ecpm,
                    });
                }
            }
        } else {
            return Err(self.schema(
                "listMediationGroups",
                format!("group missing adUnitMappings/mediationGroupLines: {item}"),
            ));
        }
        // Missing 'format' defaults to banner for parity with the TypeScript
        // reference; an unknown string fails closed instead (the TS cast did not).
        // The documented resource keeps the format under `targeting.format`;
        // recorded fixtures keep it at the top level.
        let format_raw = item.get("format").and_then(|v| v.as_str()).or_else(|| {
            item.get("targeting")
                .and_then(|targeting| targeting.get("format"))
                .and_then(|v| v.as_str())
        });
        let format = match format_raw.map(str::to_ascii_lowercase).as_deref() {
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
                // The wire eCPM is a currency amount ("1.75"), not integer
                // micros. Writing the micros integer would read back as
                // $1,750,000.00 on the next list (and the live API would see
                // the same 10^6 inflation), so format it exactly.
                serde_json::json!({
                    "adUnitId": m.ad_unit_id,
                    "network": m.network,
                    "ecpm": format_ecpm(value),
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

    /// The account's estimated AdMob-network earnings over the inclusive
    /// window, in micros.
    ///
    /// This is the documented v1beta `networkReport:generate` read. It is the
    /// platform-side total, independent of the mediation report: the audit's
    /// reporting-gap rule compares the two sources for the same dates.
    pub async fn network_revenue(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<i64, PlatformError> {
        let start = report_date(start_date, "start_date")?;
        let end = report_date(end_date, "end_date")?;
        let path = format!("/v1beta/{}/networkReport:generate", self.account_id);
        let data = self
            .caller
            .call(
                "networkReport",
                HttpMethod::Post,
                &path,
                Some(serde_json::json!({
                    "startDate": { "year": start.0, "month": start.1, "day": start.2 },
                    "endDate": { "year": end.0, "month": end.1, "day": end.2 },
                    "dimensions": ["DATE"],
                    "metrics": ["ESTIMATED_EARNINGS"],
                    "currencyCode": "USD",
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
                "networkReport",
                "response carries nextPageToken; refusing a truncated revenue total",
            ));
        }
        let rows = data
            .get("rows")
            .and_then(serde_json::Value::as_array)
            .or_else(|| {
                data.get("networkReportRows")
                    .and_then(serde_json::Value::as_array)
            })
            .ok_or_else(|| self.schema("networkReport", "response has no 'rows' array"))?;
        let mut total: i64 = 0;
        for row in rows {
            let metrics = row.get("metricValues").ok_or_else(|| {
                self.schema("networkReport", format!("row has no metrics: {row}"))
            })?;
            let raw = metrics
                .get("ESTIMATED_EARNINGS")
                .and_then(|m| {
                    m.get("microsValue")
                        .or_else(|| m.get("integerValue"))
                        .or_else(|| m.get("doubleValue"))
                })
                .ok_or_else(|| {
                    self.schema(
                        "networkReport",
                        format!("row has no ESTIMATED_EARNINGS: {row}"),
                    )
                })?;
            let value = match raw {
                serde_json::Value::String(text) => text
                    .parse::<i64>()
                    .map_err(|e| self.schema("networkReport", format!("micros '{text}': {e}")))?,
                serde_json::Value::Number(number) => number.as_i64().ok_or_else(|| {
                    self.schema(
                        "networkReport",
                        format!("micros is not an integer: {number}"),
                    )
                })?,
                other => {
                    return Err(self.schema(
                        "networkReport",
                        format!("unsupported micros value: {other}"),
                    ))
                }
            };
            total = total
                .checked_add(value)
                .ok_or_else(|| PlatformError::Validation("revenue total overflow".into()))?;
        }
        Ok(total)
    }

    /// Start a mediation A/B experiment on one mediation group (v1beta
    /// `mediationAbExperiments` create). The returned typed experiment carries
    /// [`ADMOB_REVENUE_PER_VARIANT`] = `false`: AdMob publishes only a
    /// `variantLeader` verdict, never per-variant revenue.
    /// The raw `mediationGroupLines` values for one group, in the documented
    /// v1beta shape. The typed group drops fields (targeting, line ids), so a
    /// treatment line must come from the raw document.
    pub async fn mediation_group_lines(
        &self,
        mediation_group_id: &str,
    ) -> Result<Vec<serde_json::Value>, PlatformError> {
        let path = format!(
            "/v1beta/{}/mediationGroups/{}",
            self.account_id,
            crate::encode_query_component(mediation_group_id)
        );
        let data = self
            .caller
            .call("getMediationGroup", HttpMethod::Get, &path, None, None)
            .await?;
        if let Some(lines) = data
            .get("mediationGroupLines")
            .and_then(|value| value.as_object())
        {
            let values: Vec<serde_json::Value> = lines.values().cloned().collect();
            if !values.is_empty() {
                return Ok(values);
            }
        }
        // The legacy shape carries the mappings directly; wrap them so the
        // treatment still names a real line.
        if let Some(mappings) = data
            .get("adUnitMappings")
            .and_then(|value| value.as_array())
        {
            if !mappings.is_empty() {
                return Ok(vec![serde_json::json!({
                    "adUnitMappings": mappings,
                    "cpmMicros": data
                        .get("cpmMicros")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                })]);
            }
        }
        Err(self.schema(
            "getMediationGroup",
            format!("mediation group '{mediation_group_id}' has no lines"),
        ))
    }

    pub async fn create_mediation_ab_experiment(
        &self,
        mediation_group_id: &str,
        display_name: &str,
        treatment_lines: Option<&serde_json::Value>,
        test_share: Option<u8>,
    ) -> Result<MediationAbExperiment, PlatformError> {
        if mediation_group_id.trim().is_empty() || display_name.trim().is_empty() {
            return Err(PlatformError::Validation(
                "AdMob createMediationAbExperiment requires a mediation group id and a display name"
                    .into(),
            ));
        }
        // The documented v1beta resource nests the experiment under its
        // mediation group and carries the treatment lines that serve the
        // proposed change. The treatment percentage is the policy's test
        // share expressed as an integer 1..=99.
        let path = format!(
            "/v1beta/{}/mediationGroups/{}/mediationAbExperiments",
            self.account_id,
            crate::encode_query_component(mediation_group_id)
        );
        let mut body = serde_json::json!({ "displayName": display_name });
        if let Some(share) = test_share {
            let percent = share.clamp(1, 99);
            body["treatmentTrafficPercentage"] = serde_json::json!(percent.to_string());
        }
        if let Some(lines) = treatment_lines {
            body["treatmentMediationLines"] = lines.clone();
        }
        let data = self
            .caller
            .call(
                "createMediationAbExperiment",
                HttpMethod::Post,
                &path,
                Some(body),
                None,
            )
            .await?;
        let experiment = self.parse_experiment("createMediationAbExperiment", &data)?;
        // Remember which group owns the experiment so a later stop by bare id
        // can address the documented nested `:stop` route.
        if let Some(id) = experiment.name.rsplit('/').next() {
            if let Ok(mut known) = self.experiments.lock() {
                known.insert(id.to_string(), mediation_group_id.to_string());
            }
        }
        Ok(experiment)
    }

    /// Read one mediation A/B experiment (the v1beta get method).
    ///
    /// The documented resource nests under its mediation group. A full
    /// resource name supplies the group; a bare id uses the group learned at
    /// create time or falls back to the collection route. The returned typed
    /// resource carries the state and the `variantLeader` verdict only —
    /// [`ADMOB_REVENUE_PER_VARIANT`] stays `false`.
    pub async fn get_mediation_ab_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<MediationAbExperiment, PlatformError> {
        let trimmed = experiment_id.trim();
        if trimmed.is_empty() {
            return Err(PlatformError::Validation(
                "AdMob getMediationAbExperiment requires an experiment id".into(),
            ));
        }
        let (group_id, short_id) = match experiment_route(trimmed) {
            Some(route) => route,
            None => (
                self.experiments
                    .lock()
                    .ok()
                    .and_then(|known| known.get(trimmed).cloned()),
                trimmed.to_string(),
            ),
        };
        let path = match group_id {
            Some(group_id) => format!(
                "/v1beta/{}/mediationGroups/{}/mediationAbExperiments/{}",
                self.account_id,
                crate::encode_query_component(&group_id),
                crate::encode_query_component(&short_id)
            ),
            None => format!(
                "/v1beta/{}/mediationAbExperiments/{}",
                self.account_id,
                crate::encode_query_component(&short_id)
            ),
        };
        let data = self
            .caller
            .call(
                "getMediationAbExperiment",
                HttpMethod::Get,
                &path,
                None,
                None,
            )
            .await?;
        self.parse_experiment("getMediationAbExperiment", &data)
    }

    /// Stop a mediation A/B experiment (the v1beta `:stop` custom method).
    ///
    /// The documented route nests the experiment under its mediation group.
    /// Callers pass either the full resource name (preferred) or the bare
    /// experiment id; for a bare id the group learned at create time is used,
    /// and only an experiment the client never created falls back to the
    /// legacy collection path. The returned verdict is still only a
    /// `variantLeader`; see [`Self::create_mediation_ab_experiment`].
    pub async fn stop_mediation_ab_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<MediationAbExperiment, PlatformError> {
        let trimmed = experiment_id.trim();
        if trimmed.is_empty() {
            return Err(PlatformError::Validation(
                "AdMob stopMediationAbExperiment requires an experiment id".into(),
            ));
        }
        let (group_id, short_id) = match experiment_route(trimmed) {
            Some(route) => route,
            None => (
                self.experiments
                    .lock()
                    .ok()
                    .and_then(|known| known.get(trimmed).cloned()),
                trimmed.to_string(),
            ),
        };
        let path = match group_id {
            Some(group_id) => format!(
                "/v1beta/{}/mediationGroups/{}/mediationAbExperiments/{}:stop",
                self.account_id,
                crate::encode_query_component(&group_id),
                crate::encode_query_component(&short_id)
            ),
            None => format!(
                "/v1beta/{}/mediationAbExperiments/{}:stop",
                self.account_id,
                crate::encode_query_component(&short_id)
            ),
        };
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
            // The documented lifecycle is RUNNING -> ENDED -> EXPIRED; STOPPED
            // remains readable for recorded fixtures.
            Some("STOPPED") => MediationAbExperimentState::Stopped,
            Some("ENDED") => MediationAbExperimentState::Ended,
            Some("EXPIRED") => MediationAbExperimentState::Expired,
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
                "NO_VARIANT_LEADER" => MediationAbVariantLeader::NoVariantLeader,
                "CONTROL" => MediationAbVariantLeader::Control,
                "VARIANT" => MediationAbVariantLeader::Variant,
                "TREATMENT" => MediationAbVariantLeader::Treatment,
                "INSUFFICIENT_DATA" => MediationAbVariantLeader::InsufficientData,
                "TOO_EARLY_TO_CALL" => MediationAbVariantLeader::TooEarlyToCall,
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

/// Parse a `YYYY-MM-DD` into the `(year, month, day)` triple the AdMob report
/// request body carries. A malformed or impossible date (e.g. `2026-02-31`)
/// is a typed validation refusal before any request goes out.
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
                (Ok(year), Ok(month), Ok(day)) if valid_calendar_date(year, month, day) => {
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

/// Whether `(year, month, day)` is a real Gregorian calendar date.
fn valid_calendar_date(year: i64, month: i64, day: i64) -> bool {
    if !(1..=12).contains(&month) || day < 1 {
        return false;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    day <= days_in_month
}

/// Integer micros -> the exact decimal currency string the AdMob management
/// API uses for an eCPM. `1_750_000` -> `"1.75"`.
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

/// Canonical `cpmMicros` is an integer string (the unit is micros). Bare
/// numbers are accepted for recorded fixtures; negatives are refused.
fn parse_cpm_micros(value: &serde_json::Value) -> Option<i64> {
    if let Some(raw) = value.as_str() {
        return raw.trim().parse::<i64>().ok().filter(|micros| *micros >= 0);
    }
    value.as_i64().filter(|micros| *micros >= 0)
}

/// Recover the mediation group and experiment id from a v1beta experiment
/// resource name. The documented resource is
/// `accounts/{publisher}/mediationGroups/{group}/mediationAbExperiment/{id}`;
/// recordings also carry the plural collection segment.
fn experiment_route(resource: &str) -> Option<(Option<String>, String)> {
    let segments: Vec<&str> = resource.split('/').filter(|s| !s.is_empty()).collect();
    for (index, segment) in segments.iter().enumerate() {
        if *segment == "mediationGroups"
            && segments.get(index + 2).is_some_and(|kind| {
                matches!(*kind, "mediationAbExperiment" | "mediationAbExperiments")
            })
        {
            let group = segments.get(index + 1)?.to_string();
            let id = segments.get(index + 3)?.to_string();
            return Some((Some(group), id));
        }
    }
    if resource.contains('/') {
        return resource
            .rsplit('/')
            .next()
            .filter(|id| !id.is_empty())
            .map(|id| (None, id.to_string()));
    }
    None
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
