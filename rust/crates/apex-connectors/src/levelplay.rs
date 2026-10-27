//! LevelPlay (Unity / ironSource) management client.
//!
//! Authentication is two-credential: a **secret key** and a **refresh token**
//! are exchanged at `/partners/publisher/auth` (a GET, per the documented
//! bearer flow) for a bearer token valid for 24 hours, which every management
//! call then carries. The token is cached and refreshed before it expires; a
//! management call never mints a token per request. A 401 on a management call
//! invalidates the cache and re-mints the token exactly once, so an early
//! revocation or expiry costs one bounded retry instead of failing every
//! subsequent call for the rest of the process.
//!
//! The read/write surface is the documented v4 Instances and Groups APIs, which
//! take the app key **in the path**: instances at
//! `/levelPlay/network/instances/v4/{appKey}/` and groups at
//! `/levelPlay/groups/v4/{appKey}`. The app key is percent-encoded as a single
//! path segment, so a hostile key cannot forge path or query structure.
//!
//! Reporting reads the v1 API at `/levelPlay/reporting/v1` with the `date` and
//! `abTest` breakdowns and the `revenue` and `activeUsers` metrics. The
//! documented response is `data` rows with `page`/`pageSize`/`totalResults`;
//! older deployments answered `code`/`count`/`results`, so both envelopes are
//! accepted. When `totalResults` announces more rows than the first page, the
//! client follows `page`/`resultsPerPage` to completion and refuses a truncated
//! read typed. Revenue follows the MAX unit convention: the API's currency
//! amount is parsed exactly into integer micros (1 unit = 1_000_000 micros),
//! never a float.
//!
//! Parsing is tolerant of the field spellings the API has used across versions
//! (`instanceId`/`id`, `instanceName`/`name`, `networkName`/`adapterNetwork`/
//! `network`, `rate`/`price`/`priceEcpm`/`floorPrice`) and of numeric or string
//! identifiers. The documented v4 read uses numeric ids and `networkName`/
//! `rate`, while older fixtures used strings and `price`; both are accepted.
//! An absent instance-level `rate` means the instance has no configured rate
//! and maps to zero, never to an invented price.

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
/// The impression-level revenue CSV API (not the reporting v1 API).
pub const LEVELPLAY_IMPRESSION_REVENUE_V1: &str = "/levelPlay/impressionLevelRevenue/v1";
/// Bearer lifetime documented for a freshly minted token (24 hours), refreshed
/// this many seconds before expiry.
const TOKEN_LIFETIME_SECS: i64 = 86_400;
const TOKEN_REFRESH_SKEW_SECS: i64 = 60;
/// The documented reporting default page size; used only when following a
/// `totalResults` continuation. The first request keeps the bare query the
/// API documents (and that older deployments answer without paging).
const REPORT_PAGE_SIZE: u64 = 5_000;
/// A hard bound on report pages so a hostile `totalResults` cannot spin.
const REPORT_MAX_PAGES: u64 = 1_000;

pub const LEVELPLAY_ENDPOINTS: [EndpointMeta; 8] = [
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
    EndpointMeta::new(
        "updateGroupCapping",
        CoreMethod::Post,
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

/// One impression-level row from the D3 CSV feed, folded per player later by
/// the engine. The AB Testing column identifies the dashboard arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpressionRevenueRow {
    pub day: String,
    pub platform: String,
    pub user_id: Option<String>,
    pub idfv: Option<String>,
    pub idfa: Option<String>,
    pub ab_test: Option<String>,
    pub revenue: i64,
    pub impressions: u64,
}

impl ImpressionRevenueRow {
    #[must_use]
    pub fn player_key(&self) -> Option<String> {
        valid_report_player(&self.user_id)
            .or_else(|| valid_report_player(&self.idfv))
            .or_else(|| valid_report_player(&self.idfa))
    }
}

fn valid_report_player(value: &Option<String>) -> Option<String> {
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

/// Parse the impression CSV. Header matching is case-insensitive and accepts
/// the documentation's `AB Testing` spelling plus the compact variants; the
/// required fields are revenue and at least one player id. Rows for the same
/// `(player, arm)` fold together.
pub fn parse_impression_revenue_csv(
    csv: &[u8],
    day: &str,
    platform: &str,
) -> Result<Vec<ImpressionRevenueRow>, String> {
    let text = std::str::from_utf8(csv).map_err(|error| format!("CSV is not UTF-8: {error}"))?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().ok_or_else(|| "CSV is empty".to_string())?;
    let headers: Vec<String> = header
        .split(',')
        .map(|field| field.trim().trim_matches('"').to_ascii_lowercase())
        .collect();
    let index = |names: &[&str]| -> Option<usize> {
        headers.iter().position(|header| {
            names.iter().any(|name| {
                header == name || header.replace([' ', '_'], "") == name.replace([' ', '_'], "")
            })
        })
    };
    let revenue_column =
        index(&["revenue"]).ok_or_else(|| "CSV has no Revenue column".to_string())?;
    let user_column = index(&["user id", "userid", "user_id"]);
    let idfv_column = index(&["idfv"]);
    let idfa_column = index(&["idfa"]);
    let ab_column = index(&["ab testing", "abtesting", "ab test", "abtest"]);
    let impressions_column = index(&["impressions"]);
    let date_column = index(&["date"]);
    let platform_column = index(&["platform", "os", "device os"]);
    if user_column.is_none() && idfv_column.is_none() && idfa_column.is_none() {
        return Err("CSV has no user id, IDFV or IDFA column".into());
    }
    let mut out: std::collections::BTreeMap<
        (Option<String>, Option<String>, String),
        ImpressionRevenueRow,
    > = std::collections::BTreeMap::new();
    for line in lines {
        let fields: Vec<String> = line
            .split(',')
            .map(|field| field.trim().trim_matches('"').to_string())
            .collect();
        let revenue_raw = fields
            .get(revenue_column)
            .ok_or_else(|| format!("CSV row missing revenue: {line}"))?;
        let revenue = parse_decimal_micros(revenue_raw)
            .map_err(|error| format!("revenue '{revenue_raw}': {error}"))?;
        let row = ImpressionRevenueRow {
            day: date_column
                .and_then(|column| fields.get(column))
                .map(|value| value.get(..10).unwrap_or(value.as_str()).to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| day.to_string()),
            platform: platform_column
                .and_then(|column| fields.get(column))
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| platform.to_string()),
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
            ab_test: ab_column
                .and_then(|column| fields.get(column))
                .cloned()
                .filter(|value| !value.trim().is_empty()),
            revenue,
            impressions: impressions_column
                .and_then(|column| fields.get(column))
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
        };
        let key = (row.player_key(), row.ab_test.clone(), row.day.clone());
        match out.entry(key) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(row);
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let entry = slot.get_mut();
                entry.revenue = entry
                    .revenue
                    .checked_add(revenue)
                    .ok_or_else(|| "impression revenue overflowed i64 micros".to_string())?;
                entry.impressions = entry
                    .impressions
                    .checked_add(row.impressions)
                    .ok_or_else(|| "impression count overflowed u64".to_string())?;
            }
        }
    }
    Ok(out.into_values().collect())
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
            method: HttpMethod::Get,
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
            if response.status == 401 || response.status == 403 {
                return Err(PlatformError::Auth {
                    platform: PlatformId::LevelPlay,
                    operation: "auth".into(),
                    status: response.status,
                });
            }
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

    fn invalidate_token(&self) {
        if let Ok(mut guard) = self.token.lock() {
            *guard = None;
        }
    }

    /// One management call with a single bounded refresh on a 401. The token
    /// can be revoked or expire earlier than the documented lifetime; reusing
    /// the dead cache would otherwise fail every later call in this process.
    /// A second 401 propagates unchanged (bad credentials are not a retry
    /// loop), and 403 is never refreshed because it is a permission refusal.
    async fn call(
        &self,
        operation: &str,
        method: HttpMethod,
        path: &str,
        body: Option<serde_json::Value>,
        idempotency_key: Option<&str>,
    ) -> Result<serde_json::Value, PlatformError> {
        let caller = self.caller().await?;
        match caller
            .call(operation, method, path, body.clone(), idempotency_key)
            .await
        {
            Err(PlatformError::Auth { status: 401, .. })
            | Err(PlatformError::Http { status: 401, .. }) => {
                self.invalidate_token();
                let caller = self.caller().await?;
                caller
                    .call(operation, method, path, body, idempotency_key)
                    .await
            }
            other => other,
        }
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
        let data = self
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
            // v4 returns numeric ids; older fixtures used strings.
            let instance_id =
                first_id(item, &["instanceId", "id", "instance_id"]).ok_or_else(|| {
                    self.schema("listWaterfall", format!("instance missing id: {item}"))
                })?;
            let network = first_str(
                item,
                &["networkName", "adapterNetwork", "network", "adapter"],
            )
            .unwrap_or("unknown")
            .to_string();
            let instance_name = first_str(item, &["instanceName", "name"])
                .unwrap_or(&instance_id)
                .to_string();
            // v4's instance-level price is `rate`; `price`/`priceEcpm`/
            // `floorPrice` are the spellings older fixtures used. An absent
            // rate is the documented "no instance-level rate" state.
            let price = match item
                .get("rate")
                .or_else(|| item.get("price"))
                .or_else(|| item.get("priceEcpm"))
                .or_else(|| item.get("floorPrice"))
            {
                Some(value) if !value.is_null() => {
                    let raw = crate::max::number_to_string(value).ok_or_else(|| {
                        self.schema("listWaterfall", format!("instance rate invalid: {item}"))
                    })?;
                    parse_decimal_micros(&raw).map_err(|error| {
                        self.schema(
                            "listWaterfall",
                            format!("instance rate '{raw}' invalid: {error}"),
                        )
                    })?
                }
                _ => 0,
            };
            let position = first_i64(item, &["position", "order"])
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
        let path = self.instances_path();
        let key = levelplay_price_key(ad_unit_id, entry_id, price);
        self.call(
            "updateEntryPrice",
            HttpMethod::Put,
            &path,
            Some(serde_json::json!([{
                "instanceId": entry_id,
                "rate": rate_units(price),
            }])),
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
        let path = self.instances_path();
        let key = levelplay_create_key(ad_unit_id, network, instance_name, price);
        let data = self
            .call(
                "createEntry",
                HttpMethod::Post,
                &path,
                Some(serde_json::json!([{
                    "instanceName": instance_name,
                    "adapterNetwork": network,
                    "rate": rate_units(price),
                }])),
                Some(&key),
            )
            .await?;
        // v4 create answers an array of created instances; older fixtures
        // answered one object. Both are accepted.
        let entry_id = first_id(&data, &["instanceId", "id", "instance_id"])
            .or_else(|| {
                data.get(0)
                    .and_then(|first| first_id(first, &["instanceId", "id", "instance_id"]))
            })
            .filter(|s| !s.is_empty())
            .ok_or_else(|| self.schema("createEntry", "response missing the new instance id"))?;
        Ok(entry_id)
    }

    pub async fn remove_entry(
        &self,
        ad_unit_id: &str,
        entry_id: &str,
    ) -> Result<(), PlatformError> {
        let path = self.instances_path();
        let key = levelplay_remove_key(ad_unit_id, entry_id);
        // v4 deletes by an `ids` array in the body; the old query form is not
        // part of the documented contract.
        self.call(
            "removeEntry",
            HttpMethod::Delete,
            &path,
            Some(serde_json::json!({ "ids": [entry_id] })),
            Some(&key),
        )
        .await?;
        Ok(())
    }

    /// Set the variant group's session/interstitial/day capping. The A5
    /// dashboard step is guided by the runner; this call is the live write
    /// surface and is gated by the verify-live allow-list. Group A is never
    /// addressed.
    pub async fn update_group_capping(
        &self,
        group_id: &str,
        session_limit: Option<u32>,
        interstitial_limit: Option<u32>,
        day_limit: Option<u32>,
    ) -> Result<(), PlatformError> {
        if group_id.trim().is_empty() {
            return Err(self.schema("updateGroupCapping", "group id is required"));
        }
        if session_limit.is_none() && interstitial_limit.is_none() && day_limit.is_none() {
            return Err(self.schema(
                "updateGroupCapping",
                "at least one of sessionLimit, interstitialLimit or dayLimit is required",
            ));
        }
        let body = serde_json::json!({
            "groupId": group_id,
            "sessionLimit": session_limit,
            "interstitialLimit": interstitial_limit,
            "dayLimit": day_limit,
        });
        self.call(
            "updateGroupCapping",
            HttpMethod::Post,
            &self.groups_path(),
            Some(body),
            None,
        )
        .await?;
        Ok(())
    }

    /// Delete mediation groups by id (v4). Ending a dashboard A/B test is
    /// the documented way to stop its variant arm.
    pub async fn delete_group(&self, group_id: &str) -> Result<(), PlatformError> {
        if group_id.trim().is_empty() {
            return Err(self.schema("deleteGroup", "group id is required"));
        }
        self.call(
            "deleteGroup",
            HttpMethod::Delete,
            &self.groups_path(),
            Some(serde_json::json!({ "ids": [group_id] })),
            None,
        )
        .await?;
        Ok(())
    }

    /// The app's mediation groups (v4), keyed by the path app key. Returned as
    /// raw JSON: the audit reads `floorPrice` and `countriesRate` from each
    /// group.
    pub async fn list_groups(&self) -> Result<serde_json::Value, PlatformError> {
        let path = self.groups_path();
        self.call("listGroups", HttpMethod::Get, &path, None, None)
            .await
    }

    /// Revenue and active users per A/B test arm per day over the inclusive
    /// window `[start_date, end_date]` (reporting v1, `date` and `abTest`
    /// breakdowns, `revenue` and `activeUsers` metrics).
    ///
    /// The documented response is a `data` array with `page`, `pageSize` and
    /// `totalResults`; older deployments answered `code`/`count`/`results`.
    /// Both are accepted. When `totalResults` announces more rows than the
    /// first page, every page is followed and a short read is a typed refusal.
    /// Revenue is parsed exactly into micros; active users must be a whole
    /// number. Anything missing or malformed is a typed refusal, never a
    /// silent zero.
    ///
    /// The operation label stays `reportAbTestActiveUsers` (the bounded metric
    /// vocabulary of the connector surface); the request itself carries both
    /// metrics and the date breakdown. The app key remains a percent-encoded
    /// query value and the call stays a GET.
    pub async fn ab_test_report(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<AbTestReportRow>, PlatformError> {
        let path = format!(
            "{LEVELPLAY_REPORTING_V1}?appKey={}&breakdowns=date,abTest&metrics=revenue,activeUsers&startDate={}&endDate={}",
            crate::encode_query_component(&self.app_key),
            crate::encode_query_component(start_date),
            crate::encode_query_component(end_date),
        );
        let results = self.report_pages("reportAbTestActiveUsers", &path).await?;
        let mut out = Vec::with_capacity(results.len());
        for row in &results {
            out.push(
                parse_ab_test_report_row(row)
                    .map_err(|detail| self.schema("reportAbTestActiveUsers", detail))?,
            );
        }
        Ok(out)
    }

    /// Read every page of one reporting query. The first request is the bare
    /// documented query; continuations append `page` and `resultsPerPage`.
    /// `totalResults` is the completion contract: a server that truncates a
    /// page (or ignores `page`) fails typed instead of returning a short
    /// measurement as if it were complete.
    async fn report_pages(
        &self,
        operation: &str,
        base_path: &str,
    ) -> Result<Vec<serde_json::Value>, PlatformError> {
        let mut collected: Vec<serde_json::Value> = Vec::new();
        let mut total: Option<usize> = None;
        let mut page: u64 = 1;
        loop {
            let path = if page == 1 {
                base_path.to_string()
            } else {
                format!("{base_path}&page={page}&resultsPerPage={REPORT_PAGE_SIZE}")
            };
            let data = self
                .call(operation, HttpMethod::Get, &path, None, None)
                .await?;
            let code = data
                .get("code")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(200);
            if code != 200 {
                return Err(PlatformError::Http {
                    platform: PlatformId::LevelPlay,
                    operation: operation.into(),
                    status: u16::try_from(code).unwrap_or(500),
                });
            }
            let rows = data
                .get("results")
                .or_else(|| data.get("data"))
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| {
                    self.schema(
                        operation,
                        "response has neither a 'data' nor a 'results' array",
                    )
                })?;
            if let Some(announced) = data
                .get("totalResults")
                .and_then(crate::max::number_to_string)
                .and_then(|raw| raw.parse::<usize>().ok())
            {
                total = Some(announced);
            }
            if rows.is_empty() {
                break;
            }
            collected.extend(rows.iter().cloned());
            let complete = total.is_none_or(|announced| collected.len() >= announced);
            if complete {
                break;
            }
            page = page
                .checked_add(1)
                .ok_or_else(|| self.schema(operation, "report pagination counter overflow"))?;
            if page > REPORT_MAX_PAGES {
                return Err(self.schema(
                    operation,
                    format!("report pagination exceeded {REPORT_MAX_PAGES} pages"),
                ));
            }
        }
        if let Some(announced) = total {
            if collected.len() < announced {
                return Err(self.schema(
                    operation,
                    format!(
                        "report pagination truncated: read {} of {} announced rows",
                        collected.len(),
                        announced
                    ),
                ));
            }
        }
        Ok(collected)
    }

    /// Impression-level revenue for one UTC day, keyed by
    /// `(day, platform, user)`. This is the D3 feed, not the reporting API:
    /// the response is the documented impression CSV and the AB Testing column
    /// rides each row so the caller can fold the arms. The feed keeps 14 days
    /// and is final at D+2, which the caller schedules.
    pub async fn impression_revenue_day(
        &self,
        day: &str,
        platform: &str,
    ) -> Result<Vec<ImpressionRevenueRow>, PlatformError> {
        if !super::max::valid_day_for_feed(day) {
            return Err(self.schema(
                "impressionRevenue",
                format!("day must be YYYY-MM-DD (given '{day}')"),
            ));
        }
        if platform.trim().is_empty() {
            return Err(self.schema("impressionRevenue", "platform is required"));
        }
        let path = format!(
            "{LEVELPLAY_IMPRESSION_REVENUE_V1}?appKey={}&date={}",
            crate::encode_query_component(&self.app_key),
            crate::encode_query_component(day),
        );
        let mut token = self.bearer().await?;
        let mut response = self
            .raw_report_request(&path, &token, "impressionRevenue")
            .await?;
        if response.0 == 401 || response.0 == 403 {
            // A revoked/expired bearer is refreshed exactly once, the same
            // bound the management calls use.
            self.invalidate_token();
            token = self.bearer().await?;
            response = self
                .raw_report_request(&path, &token, "impressionRevenue")
                .await?;
        }
        if response.0 != 200 {
            return Err(PlatformError::Http {
                platform: PlatformId::LevelPlay,
                operation: "impressionRevenue".into(),
                status: response.0,
            });
        }
        parse_impression_revenue_csv(&response.1, day, platform.trim())
            .map_err(|detail| self.schema("impressionRevenue", detail))
    }

    async fn raw_report_request(
        &self,
        path: &str,
        token: &str,
        operation: &str,
    ) -> Result<(u16, Vec<u8>), PlatformError> {
        let request = HttpRequest {
            platform: PlatformId::LevelPlay,
            operation: operation.to_string(),
            method: HttpMethod::Get,
            url: format!("{}{path}", self.base_url),
            headers: vec![
                ("Authorization".into(), format!("Bearer {token}")),
                ("Accept".into(), "text/csv".into()),
            ],
            body: None,
            timeout_ms: 60_000,
        };
        match self.transport.request(request).await {
            Ok(response) => Ok((response.status, response.body)),
            Err(TransportError::Platform(error)) => Err(error),
            Err(TransportError::Network(detail)) => Err(PlatformError::Network {
                platform: PlatformId::LevelPlay,
                operation: operation.into(),
                detail,
            }),
            Err(TransportError::Timeout(detail)) => Err(PlatformError::Network {
                platform: PlatformId::LevelPlay,
                operation: operation.into(),
                detail: format!("timeout: {detail}"),
            }),
        }
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
        let path = format!(
            "{LEVELPLAY_REPORTING_V1}?appKey={}&breakdowns=date&metrics=revenue&startDate={}&endDate={}",
            crate::encode_query_component(&self.app_key),
            crate::encode_query_component(start_date),
            crate::encode_query_component(end_date),
        );
        let results = self.report_pages("adUnitRevenue", &path).await?;
        let mut total: i64 = 0;
        for row in &results {
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

/// A micros price rendered as the documented v4 `rate` string in currency
/// units (e.g. 2_400_000 -> "2.4"), without a trailing zero train.
fn rate_units(price_micros: i64) -> String {
    let whole = price_micros / 1_000_000;
    let fraction = (price_micros % 1_000_000).abs();
    if fraction == 0 {
        return whole.to_string();
    }
    let digits = format!("{fraction:06}");
    format!("{whole}.{}", digits.trim_end_matches('0'))
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

/// An identifier as a string: the documented v4 API emits numbers, older
/// fixtures emitted strings. Numeric ids are rendered exactly (no float path).
fn first_id(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let field = value.get(*key)?;
        if let Some(text) = field.as_str() {
            return Some(text.to_string());
        }
        crate::max::number_to_string(field)
    })
}

/// An integer field that may be a JSON number or a numeric string.
fn first_i64(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let field = value.get(*key)?;
        field
            .as_i64()
            .or_else(|| field.as_str().and_then(|text| text.trim().parse().ok()))
    })
}

fn now_unix_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
