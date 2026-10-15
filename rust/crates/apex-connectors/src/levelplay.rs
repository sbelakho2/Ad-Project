//! LevelPlay (Unity / ironSource) management client.
//!
//! Authentication is two-credential: a **secret key** and a **refresh token**
//! are exchanged at `/partners/publisher/auth` for a short-lived bearer token,
//! which every management call then carries. The token is cached and refreshed
//! before it expires; a management call never mints a token per request.
//!
//! The read/write surface is the documented v4 Instances and Groups APIs under
//! `/partners/publisher/mediation/{instances,groups}/v4`, keyed by the app key.
//! Parsing is tolerant of the field spellings the API has used across versions
//! (`instanceId`/`id`, `instanceName`/`name`, `adapterNetwork`/`network`,
//! `price`/`priceEcpm`/`floorPrice`) but never invents data: a missing required
//! field is a typed refusal.

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
/// The v4 Instances API.
pub const LEVELPLAY_INSTANCES_V4: &str = "/partners/publisher/mediation/instances/v4";
/// The v4 Groups API.
pub const LEVELPLAY_GROUPS_V4: &str = "/partners/publisher/mediation/groups/v4";
/// Bearer lifetime assumed for a freshly minted token, refreshed this many
/// seconds before expiry.
const TOKEN_LIFETIME_SECS: i64 = 1_800;
const TOKEN_REFRESH_SKEW_SECS: i64 = 60;

pub const LEVELPLAY_ENDPOINTS: [EndpointMeta; 5] = [
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

    fn instances_path(&self) -> String {
        format!(
            "{LEVELPLAY_INSTANCES_V4}?appKey={}",
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
        let path = format!(
            "{LEVELPLAY_INSTANCES_V4}?appKey={}",
            crate::encode_query_component(&self.app_key)
        );
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
        let path = format!(
            "{LEVELPLAY_INSTANCES_V4}?appKey={}",
            crate::encode_query_component(&self.app_key)
        );
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
            "{LEVELPLAY_INSTANCES_V4}?appKey={}&instanceId={}",
            crate::encode_query_component(&self.app_key),
            crate::encode_query_component(entry_id)
        );
        let key = levelplay_remove_key(ad_unit_id, entry_id);
        caller
            .call("removeEntry", HttpMethod::Delete, &path, None, Some(&key))
            .await?;
        Ok(())
    }

    /// The app's mediation groups (v4). Returned as raw JSON: the audit reads
    /// `floorPrice` and `countriesRate` from each group.
    pub async fn list_groups(&self) -> Result<serde_json::Value, PlatformError> {
        let caller = self.caller().await?;
        let path = format!(
            "{LEVELPLAY_GROUPS_V4}?appKey={}",
            crate::encode_query_component(&self.app_key)
        );
        caller
            .call("listGroups", HttpMethod::Get, &path, None, None)
            .await
    }
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
