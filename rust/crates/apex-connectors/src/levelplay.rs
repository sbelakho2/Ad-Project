//! LevelPlay (Unity / ironSource) management client.

use std::sync::Arc;

use apex_core::endpoint::HttpMethod as CoreMethod;
use apex_core::{
    levelplay_create_key, levelplay_price_key, levelplay_remove_key, validate_ecpm, EndpointMeta,
    PlatformError, PlatformId, SourceVerification, WaterfallEntry,
};

use crate::parse_decimal_micros;
use crate::transport::{HttpMethod, PlatformCaller, Transport};

pub const LEVELPLAY_ENDPOINTS: [EndpointMeta; 4] = [
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
        CoreMethod::Patch,
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

pub struct LevelPlayClient {
    caller: PlatformCaller,
}

impl std::fmt::Debug for LevelPlayClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LevelPlayClient")
            .field("platform", &self.caller.platform)
            .finish()
    }
}

impl LevelPlayClient {
    pub fn new(
        api_key: &str,
        account_id: &str,
        base_url: &str,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, PlatformError> {
        if api_key.trim().is_empty() || account_id.trim().is_empty() {
            return Err(PlatformError::Validation(
                "LevelPlay credentials require apiKey and accountId".into(),
            ));
        }
        Ok(Self {
            caller: PlatformCaller::new(
                PlatformId::LevelPlay,
                base_url.to_string(),
                transport,
                ("Authorization".into(), format!("Bearer {api_key}")),
            ),
        })
    }

    fn schema(&self, operation: &str, detail: impl Into<String>) -> PlatformError {
        PlatformError::Schema {
            platform: PlatformId::LevelPlay,
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    pub async fn list_waterfall(
        &self,
        ad_unit_id: &str,
    ) -> Result<Vec<WaterfallEntry>, PlatformError> {
        let path = format!(
            "/v1/adunits/{}/waterfall",
            crate::encode_query_component(ad_unit_id)
        );
        let data = self
            .caller
            .call("listWaterfall", HttpMethod::Get, &path, None, None)
            .await?;
        let entries = data
            .get("waterfall")
            .and_then(|v| v.as_array())
            .ok_or_else(|| self.schema("listWaterfall", "response missing array 'waterfall'"))?;
        let mut out = Vec::with_capacity(entries.len());
        for item in entries {
            let entry_id = item
                .get("entry_id")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| self.schema("listWaterfall", format!("entry missing id: {item}")))?;
            let network = item
                .get("network")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    self.schema("listWaterfall", format!("entry missing network: {item}"))
                })?;
            let instance_name = item
                .get("instance_name")
                .and_then(|v| v.as_str())
                .unwrap_or(network)
                .to_string();
            let position = item
                .get("position")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| {
                    self.schema("listWaterfall", format!("entry missing position: {item}"))
                })?;
            let price_raw = item.get("price_ecpm").ok_or_else(|| {
                self.schema("listWaterfall", format!("entry missing price_ecpm: {item}"))
            })?;
            let price_ecpm = crate::max::number_to_string(price_raw)
                .and_then(|s| parse_decimal_micros(&s).ok())
                .ok_or_else(|| {
                    self.schema("listWaterfall", format!("entry price_ecpm invalid: {item}"))
                })?;
            out.push(WaterfallEntry {
                entry_id: entry_id.to_string(),
                ad_unit_id: ad_unit_id.to_string(),
                network: network.to_string(),
                instance_name,
                position: i32::try_from(position).map_err(|_| {
                    self.schema(
                        "listWaterfall",
                        format!("position out of range: {position}"),
                    )
                })?,
                price_ecpm,
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
        let path = format!(
            "/v1/adunits/{}/waterfall/{}",
            crate::encode_query_component(ad_unit_id),
            crate::encode_query_component(entry_id)
        );
        let key = levelplay_price_key(ad_unit_id, entry_id, price);
        self.caller
            .call(
                "updateEntryPrice",
                HttpMethod::Patch,
                &path,
                Some(serde_json::json!({ "price_ecpm": price })),
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
        let path = format!(
            "/v1/adunits/{}/waterfall",
            crate::encode_query_component(ad_unit_id)
        );
        let key = levelplay_create_key(ad_unit_id, network, instance_name, price);
        let data = self
            .caller
            .call(
                "createEntry",
                HttpMethod::Post,
                &path,
                Some(serde_json::json!({
                    "network": network,
                    "instance_name": instance_name,
                    "price_ecpm": price,
                })),
                Some(&key),
            )
            .await?;
        let entry_id = data
            .get("entry_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| self.schema("createEntry", "response missing entry_id"))?;
        Ok(entry_id.to_string())
    }

    pub async fn remove_entry(
        &self,
        ad_unit_id: &str,
        entry_id: &str,
    ) -> Result<(), PlatformError> {
        let path = format!(
            "/v1/adunits/{}/waterfall/{}",
            crate::encode_query_component(ad_unit_id),
            crate::encode_query_component(entry_id)
        );
        let key = levelplay_remove_key(ad_unit_id, entry_id);
        self.caller
            .call("removeEntry", HttpMethod::Delete, &path, None, Some(&key))
            .await?;
        Ok(())
    }
}
