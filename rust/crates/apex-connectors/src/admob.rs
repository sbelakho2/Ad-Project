//! AdMob management client (OAuth2 bearer; mediation groups + mappings + eCPM).

use std::sync::Arc;

use apex_core::endpoint::HttpMethod as CoreMethod;
use apex_core::{
    admob_ecpm_key, validate_ecpm, AdFormat, EndpointMeta, Mapping, MediationGroup, PlatformError,
    PlatformId, SourceVerification,
};

use crate::transport::{HttpMethod, PlatformCaller, Transport};

const MAX_PAGES: usize = 50;

pub const ADMOB_ENDPOINTS: [EndpointMeta; 3] = [
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
        "updateMediationGroup",
        CoreMethod::Patch,
        SourceVerification::VerifyLive,
        false,
    ),
];

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
                None => format!("/v1/{}/mediationGroups", self.account_id),
                Some(token) => format!(
                    "/v1/{}/mediationGroups?pageToken={}",
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
            "/v1/{}/mediationGroups/{}",
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
}
