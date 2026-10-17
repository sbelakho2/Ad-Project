//! LevelPlay (Unity / ironSource) management client.
//!
//! Authentication is two-credential: a **secret key** and a **refresh token**
//! are exchanged at `/partners/publisher/auth` for a short-lived bearer token,
//! which every management call then carries. The token is cached and refreshed
//! before it expires; a management call never mints a token per request.
//!
//! The read/write surface is the documented v4 Instances and Groups APIs, which
//! take the app key **in the path**: instances at
//! `/levelPlay/network/instances/v4/{appKey}/` and groups at
//! `/levelPlay/groups/v4/{appKey}`. The app key is percent-encoded as a single
//! path segment, so a hostile key cannot forge path or query structure.
//!
//! Reporting reads the v1 API at `/levelPlay/reporting/v1` with the `date` and
//! `abTest` breakdowns and the `revenue` and `activeUsers` metrics, shaped like
//! the MAX reporting client (query-parameter request,
//! `code`/`count`/`results` envelope, typed rows, fail-closed parsing). Revenue
//! follows the MAX unit convention: the API's currency amount is parsed exactly
//! into integer micros (1 unit = 1_000_000 micros), never a float.
//!
//! Parsing is tolerant of the field spellings the API has used across versions
//! (`instanceId`/`id`, `instanceName`/`name`, `adapterNetwork`/`network`,
//! `price`/`priceEcpm`/`floorPrice`) but never invents data: a missing required
//! field is a typed refusal.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use apex_core::endpoint::HttpMethod as CoreMethod;
use apex_core::{
    levelplay_create_key, levelplay_price_key, levelplay_remove_key, validate_ecpm, EndpointMeta,
    PlatformError, PlatformId, SourceVerification, WaterfallEntry,
};

use crate::parse_decimal_micros;
use crate::transport::{HttpMethod, HttpRequest, PlatformCaller, Transport, TransportError};

/// Where the bearer token is minted.
pub const LEVELPLAY_AUTH_PATH: &str = "/partners/publisher/auth";
/// The v4 Instances API; the app key is the next path segment (trailing `/`).
pub const LEVELPLAY_INSTANCES_V4: &str = "/levelPlay/network/instances/v4";
/// The v4 Groups API; the app key is the next path segment.
pub const LEVELPLAY_GROUPS_V4: &str = "/levelPlay/groups/v4";
/// The v1 Reporting API (query parameters carry the app key, breakdown and
/// metric selection).
pub const LEVELPLAY_REPORTING_V1: &str = "/levelPlay/reporting/v1";
/// Bearer lifetime assumed for a freshly minted token, refreshed this many
/// seconds before expiry.
const TOKEN_LIFETIME_SECS: i64 = 1_800;
const TOKEN_REFRESH_SKEW_SECS: i64 = 60;

pub const LEVELPLAY_ENDPOINTS: [EndpointMeta; 7] = [
    EndpointMeta::new(
        "auth",
        CoreMethod::Post,
        SourceVerification::VerifyLive,
        true,
    ),
    EndpointMeta::new(
        "listWaterfall",
        CoreMethod::Get,
        SourceVerification::VerifyLive,
        true,
    ),
    EndpointMeta::new(
        "listGroups",
        CoreMethod::Get,
        SourceVerification::VerifyLive,
        true,
    ),
    EndpointMeta::new(
        "reportAbTestActiveUsers",
        CoreMethod::Get,
        SourceVerification::VerifyLive,
        true,
    ),
    EndpointMeta::new(
        "createEntry",
        CoreMethod::Post,
        SourceVerification::VerifyLive,
        false,
    ),
    EndpointMeta::new(
        "updateEntryPrice",
        CoreMethod::Put,
        SourceVerification::VerifyLive,
        false,
    ),
    EndpointMeta::new(
        "removeEntry",
        CoreMethod::Delete,
        SourceVerification::VerifyLive,
        false,
    ),
];

/// One row of the reporting v1 response: an A/B test arm's day, its revenue and
/// its active users (the `date`/`abTest` breakdowns and the
/// `revenue`/`activeUsers` metrics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbTestReportRow {
    /// `YYYY-MM-DD`, UTC, as the `date` breakdown reports it.
    pub day: String,
    pub ab_test: String,
    /// Revenue in micros. The reporting `revenue` metric is a currency amount
    /// in whole units (e.g. USD), parsed exactly at 1 unit = 1_000_000 micros
    /// — the same convention as MAX's `estimated_revenue`.
    pub revenue: i64,
    pub active_users: u64,
}

#[derive(Clone)]
struct CachedToken {
    value: String,
    expires_at_unix_s: i64,
}

pub struct LevelPlayClient {
    base_url: String,
    app_key: String,
    secret_key: String,
    refresh_token: String,
    transport: Arc<dyn Transport>,
    token: Mutex<Option<CachedToken>>,
    /// The app-level revenue already read this client's lifetime, keyed by
    /// the inclusive `(start, end)` window. One client is one app key, so
    /// every ad-unit scope bills from the same read; the monthly pass asks
    /// once per scope and would otherwise re-fetch the identical report.
    revenue: Mutex<BTreeMap<(String, String), i64>>,
}

impl std::fmt::Debug for LevelPlayClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LevelPlayClient")
            .field("app_key", &self.app_key)
            .finish()
    }
}

impl LevelPlayClient {
    pub fn new(
        secret_key: &str,
        refresh_token: &str,
        app_key: &str,
        base_url: &str,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, PlatformError> {
        if secret_key.trim().is_empty()
            || refresh_token.trim().is_empty()
            || app_key.trim().is_empty()
        {
            return Err(PlatformError::Validation(
                "LevelPlay credentials require a secret key, a refresh token and an app key".into(),
            ));
        }
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            app_key: app_key.to_string(),
            secret_key: secret_key.to_string(),
            refresh_token: refresh_token.to_string(),
            transport,
            token: Mutex::new(None),
            revenue: Mutex::new(BTreeMap::new()),
        })
    }

    fn schema(&self, operation: &str, detail: impl Into<String>) -> PlatformError {
        PlatformError::Schema {
            platform: PlatformId::LevelPlay,
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    /// The cached bearer token, minted or refreshed with the secret key and
    /// refresh token when absent or near expiry.
    async fn bearer(&self) -> Result<String, PlatformError> {
        let now = now_unix_s();
        if let Ok(guard) = self.token.lock() {
            if let Some(cached) = guard.as_ref() {
                if cached.expires_at_unix_s - TOKEN_REFRESH_SKEW_SECS > now {
                    return Ok(cached.value.clone());
                }
            }
        }
        let request = HttpRequest {
            platform: PlatformId::LevelPlay,
            operation: "auth".to_string(),
            method: HttpMethod::Post,
            url: format!("{}{LEVELPLAY_AUTH_PATH}", self.base_url),
            headers: vec![
                ("secretkey".into(), self.secret_key.clone()),
                ("refreshToken".into(), self.refresh_token.clone()),
                ("Accept".into(), "application/json".into()),
            ],
            body: None,
            timeout_ms: 30_000,
        };
        let response = match self.transport.request(request).await {
            Ok(response) => response,
            Err(TransportError::Platform(error)) => return Err(error),
            Err(TransportError::Network(detail)) => {
                return Err(PlatformError::Network {
                    platform: PlatformId::LevelPlay,
                    operation: "auth".into(),
                    detail,
                })
            }
            Err(TransportError::Timeout(detail)) => {
                return Err(PlatformError::Network {
                    platform: PlatformId::LevelPlay,
                    operation: "auth".into(),
                    detail: format!("timeout: {detail}"),
                })
            }
        };
        if response.status != 200 {
            return Err(PlatformError::Http {
                platform: PlatformId::LevelPlay,
                operation: "auth".into(),
                status: response.status,
            });
        }
        let value = parse_token(&response.body)
            .ok_or_else(|| self.schema("auth", "auth response carried no token"))?;
        if let Ok(mut guard) = self.token.lock() {
            *guard = Some(CachedToken {
                value: value.clone(),
                expires_at_unix_s: now + TOKEN_LIFETIME_SECS,
            });
        }
        Ok(value)
    }

    async fn caller(&self) -> Result<PlatformCaller, PlatformError> {
        let token = self.bearer().await?;
        Ok(PlatformCaller::new(
            PlatformId::LevelPlay,
            self.base_url.clone(),
            Arc::clone(&self.transport),
            ("Authorization".into(), format!("Bearer {token}")),
        ))
    }

    /// `/levelPlay/network/instances/v4/{appKey}/`, the app key percent-encoded
    /// as one path segment.
    fn instances_path(&self) -> String {
        format!(
            "{LEVELPLAY_INSTANCES_V4}/{}/",
            crate::encode_query_component(&self.app_key)
        )
    }

    /// `/levelPlay/groups/v4/{appKey}`, the app key percent-encoded as one path
    /// segment.
    fn groups_path(&self) -> String {
        format!(
            "{LEVELPLAY_GROUPS_V4}/{}",
            crate::encode_query_component(&self.app_key)
        )
    }

    /// The app's configured instances, mapped to the waterfall the audit reads.
    pub async fn list_waterfall(
        &self,
        ad_unit_id: &str,
    ) -> Result<Vec<WaterfallEntry>, PlatformError> {
        let caller = self.caller().await?;
        let data = caller
            .call(
                "listWaterfall",
                HttpMethod::Get,
                &self.instances_path(),
                None,
                None,
            )
            .await?;
        let items = instances_array(&data)
            .ok_or_else(|| self.schema("listWaterfall", "response has no instances array"))?;
        let mut out = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let instance_id =
                first_str(item, &["instanceId", "id", "instance_id"]).ok_or_else(|| {
                    self.schema("listWaterfall", format!("instance missing id: {item}"))
                })?;
            let network = first_str(item, &["adapterNetwork", "network", "adapter"])
                .unwrap_or("unknown")
                .to_string();
            let instance_name = first_str(item, &["instanceName", "name"])
                .unwrap_or(instance_id)
                .to_string();
            let price = item
                .get("price")
                .or_else(|| item.get("priceEcpm"))
                .or_else(|| item.get("floorPrice"))
                .and_then(crate::max::number_to_string)
                .and_then(|s| parse_decimal_micros(&s).ok())
                .ok_or_else(|| {
                    self.schema("listWaterfall", format!("instance price invalid: {item}"))
                })?;
            let position = item
                .get("position")
                .or_else(|| item.get("order"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| i64::try_from(index).unwrap_or(0) + 1);
            if position < 0 {
                return Err(self.schema(
                    "listWaterfall",
                    format!("instance position {position} is negative"),
                ));
            }
            out.push(WaterfallEntry {
                entry_id: instance_id.to_string(),
                ad_unit_id: ad_unit_id.to_string(),
                network,
                instance_name,
                position: i32::try_from(position).map_err(|_| {
                    self.schema(
                        "listWaterfall",
                        format!("position out of range: {position}"),
                    )
                })?,
                price_ecpm: price,
            });
        }
        Ok(out)
    }

    pub async fn update_entry_price(
        &self,
        ad_unit_id: &str,
        entry_id: &str,
        price_ecpm: i64,
    ) -> Result<WaterfallEntry, PlatformError> {
        let price = validate_ecpm(price_ecpm)?;
        let caller = self.caller().await?;
        let path = self.instances_path();
        let key = levelplay_price_key(ad_unit_id, entry_id, price);
        caller
            .call(
                "updateEntryPrice",
                HttpMethod::Put,
                &path,
                Some(serde_json::json!({ "instanceId": entry_id, "price": price })),
                Some(&key),
            )
            .await?;
        Ok(WaterfallEntry {
            entry_id: entry_id.to_string(),
            ad_unit_id: ad_unit_id.to_string(),
            network: "unknown-until-refetch".into(),
            instance_name: entry_id.to_string(),
            position: -1,
            price_ecpm: price,
        })
    }

    pub async fn create_entry(
        &self,
        ad_unit_id: &str,
        network: &str,
        instance_name: &str,
        price_ecpm: i64,
    ) -> Result<String, PlatformError> {
        if network.trim().is_empty() || instance_name.trim().is_empty() {
            return Err(self.schema("createEntry", "network and instanceName are required"));
        }
        let price = validate_ecpm(price_ecpm)?;
        let caller = self.caller().await?;
        let path = self.instances_path();
        let key = levelplay_create_key(ad_unit_id, network, instance_name, price);
        let data = caller
            .call(
                "createEntry",
                HttpMethod::Post,
                &path,
                Some(serde_json::json!({
                    "instanceName": instance_name,
                    "adapterNetwork": network,
                    "price": price,
                })),
                Some(&key),
            )
            .await?;
        let entry_id = first_str(&data, &["instanceId", "id", "instance_id"])
            .filter(|s| !s.is_empty())
            .ok_or_else(|| self.schema("createEntry", "response missing the new instance id"))?;
        Ok(entry_id.to_string())
    }

    pub async fn remove_entry(
        &self,
        ad_unit_id: &str,
        entry_id: &str,
    ) -> Result<(), PlatformError> {
        let caller = self.caller().await?;
        let path = format!(
            "{}?instanceId={}",
            self.instances_path(),
            crate::encode_query_component(entry_id)
        );
        let key = levelplay_remove_key(ad_unit_id, entry_id);
        caller
            .call("removeEntry", HttpMethod::Delete, &path, None, Some(&key))
            .await?;
        Ok(())
    }

    /// The app's mediation groups (v4), keyed by the path app key. Returned as
    /// raw JSON: the audit reads `floorPrice` and `countriesRate` from each
    /// group.
    pub async fn list_groups(&self) -> Result<serde_json::Value, PlatformError> {
        let caller = self.caller().await?;
        let path = self.groups_path();
        caller
            .call("listGroups", HttpMethod::Get, &path, None, None)
            .await
    }

    /// Revenue and active users per A/B test arm per day over the inclusive
    /// window `[start_date, end_date]` (reporting v1, `date` and `abTest`
    /// breakdowns, `revenue` and `activeUsers` metrics).
    ///
    /// The response follows the report envelope convention shared with the MAX
    /// reporting client: an in-body `code` that must be 200, a `results` array
    /// whose rows carry `date`, `abTest`, `revenue` and `activeUsers`. Revenue
    /// is parsed exactly into micros; active users must be a whole number.
    /// Anything missing or malformed is a typed refusal, never a silent zero.
    ///
    /// The operation label stays `reportAbTestActiveUsers` (the bounded metric
    /// vocabulary of the connector surface); the request itself now carries
    /// both metrics and the date breakdown. The app key remains a
    /// percent-encoded query value and the call stays a GET.
    pub async fn ab_test_report(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<AbTestReportRow>, PlatformError> {
        let caller = self.caller().await?;
        let path = format!(
            "{LEVELPLAY_REPORTING_V1}?appKey={}&breakdowns=date,abTest&metrics=revenue,activeUsers&startDate={}&endDate={}",
            crate::encode_query_component(&self.app_key),
            crate::encode_query_component(start_date),
            crate::encode_query_component(end_date),
        );
        let data = caller
            .call(
                "reportAbTestActiveUsers",
                HttpMethod::Get,
                &path,
                None,
                None,
            )
            .await?;
        let code = data
            .get("code")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(200);
        if code != 200 {
            return Err(PlatformError::Http {
                platform: PlatformId::LevelPlay,
                operation: "reportAbTestActiveUsers".into(),
                status: u16::try_from(code).unwrap_or(500),
            });
        }
        let results = data
            .get("results")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                self.schema("reportAbTestActiveUsers", "response has no 'results' array")
            })?;
        let mut out = Vec::with_capacity(results.len());
        for row in results {
            out.push(
                parse_ab_test_report_row(row)
                    .map_err(|detail| self.schema("reportAbTestActiveUsers", detail))?,
            );
        }
        Ok(out)
    }

    /// The app's reported revenue over the inclusive window
    /// `[start_date, end_date]`, in micros. The monthly pass bills the
    /// pre-registered scope's incremental revenue from this total.
    ///
    /// This is the app-level read the connector is bound to (one client = one
    /// app key): `breakdowns=date` and `metrics=revenue` only, so it works
    /// whether or not an A/B test is still running. Totals are checked sums;
    /// an overflow is a typed validation refusal, never a wrapped total.
    pub async fn app_revenue(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<i64, PlatformError> {
        let key = (start_date.to_string(), end_date.to_string());
        if let Some(total) = self
            .revenue
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).copied())
        {
            return Ok(total);
        }
        let caller = self.caller().await?;
        let path = format!(
            "{LEVELPLAY_REPORTING_V1}?appKey={}&breakdowns=date&metrics=revenue&startDate={}&endDate={}",
            crate::encode_query_component(&self.app_key),
            crate::encode_query_component(start_date),
            crate::encode_query_component(end_date),
        );
        let data = caller
            .call("adUnitRevenue", HttpMethod::Get, &path, None, None)
            .await?;
        let code = data
            .get("code")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(200);
        if code != 200 {
            return Err(PlatformError::Http {
                platform: PlatformId::LevelPlay,
                operation: "adUnitRevenue".into(),
                status: u16::try_from(code).unwrap_or(500),
            });
        }
        let results = data
            .get("results")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| self.schema("adUnitRevenue", "response has no 'results' array"))?;
        let mut total: i64 = 0;
        for row in results {
            let revenue =
                parse_report_revenue(row).map_err(|detail| self.schema("adUnitRevenue", detail))?;
            total = total
                .checked_add(revenue)
                .ok_or_else(|| PlatformError::Validation("revenue total overflow".into()))?;
        }
        if let Ok(mut cache) = self.revenue.lock() {
            cache.insert(key, total);
        }
        Ok(total)
    }
}

/// Parse a reporting `revenue` value exactly into micros. The metric is a
/// currency amount (e.g. USD) whose JSON may be a number or a decimal string;
/// the same convention as MAX's `estimated_revenue`.
fn parse_report_revenue(row: &serde_json::Value) -> Result<i64, String> {
    let raw = row
        .get("revenue")
        .and_then(crate::max::number_to_string)
        .ok_or_else(|| format!("report row missing 'revenue': {row}"))?;
    parse_decimal_micros(&raw).map_err(|e| format!("revenue '{raw}': {e}"))
}

/// Parse one reporting v1 row into the typed shape. Every field is required:
/// a missing `date`, `abTest`, `revenue` or `activeUsers` is a typed refusal
/// naming the field.
fn parse_ab_test_report_row(row: &serde_json::Value) -> Result<AbTestReportRow, String> {
    let day = row
        .get("date")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("report row missing 'date': {row}"))?;
    let ab_test = row
        .get("abTest")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("report row missing 'abTest': {row}"))?;
    let revenue = parse_report_revenue(row)?;
    let active_users_raw = row
        .get("activeUsers")
        .and_then(crate::max::number_to_string)
        .ok_or_else(|| format!("report row missing 'activeUsers': {row}"))?;
    let active_users = active_users_raw
        .parse::<u64>()
        .map_err(|e| format!("activeUsers '{active_users_raw}': {e}"))?;
    Ok(AbTestReportRow {
        day: day.to_string(),
        ab_test: ab_test.to_string(),
        revenue,
        active_users,
    })
}

/// Extract the token from an auth response: a bare JSON string or an object
/// with a `token` field.
fn parse_token(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    match value {
        serde_json::Value::String(token) if !token.is_empty() => Some(token),
        serde_json::Value::Object(map) => map
            .get("token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// The instances array, accepting either a top-level array or an object keyed
/// `instances`.
fn instances_array(data: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    data.as_array()
        .or_else(|| data.get("instances").and_then(|v| v.as_array()))
}

fn first_str<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|v| v.as_str()))
}

fn now_unix_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
