//! Idempotency keys, experiment naming and platform attribution.
//! The change keys reuse apex-core builders verbatim so the platform sees the
//! same key whether the write came from a direct call or the autopilot.

use apex_core::{
    admob_ecpm_key, levelplay_create_key, levelplay_price_key, levelplay_remove_key,
    max_experiment_key, max_floor_key, PlatformError, PlatformId,
};

use crate::model::Action;

/// Stable slug per action used in experiment names (idempotent reruns must
/// produce the identical experiment key).
fn action_slug(action: &Action) -> String {
    match action {
        Action::SetCountryFloor {
            ad_unit_id,
            country,
            floor_ecpm,
        } => {
            format!(
                "floor-{}-{}-{}",
                slug(ad_unit_id),
                slug(&country.to_lowercase()),
                slug(&apex_core::format_micros_short(*floor_ecpm))
            )
        }
        Action::SetMappingEcpm {
            mediation_group_id,
            ad_unit_id,
            ecpm,
        } => {
            format!(
                "ecpm-{}-{}-{}",
                slug(mediation_group_id),
                slug(ad_unit_id),
                slug(&apex_core::format_micros_short(*ecpm))
            )
        }
        Action::SetWaterfallPrice {
            ad_unit_id,
            entry_id,
            price_ecpm,
        } => {
            format!(
                "wfp-{}-{}-{}",
                slug(ad_unit_id),
                slug(entry_id),
                slug(&apex_core::format_micros_short(*price_ecpm))
            )
        }
        Action::CreateWaterfallEntry {
            ad_unit_id,
            network,
            instance_name: _,
            price_ecpm,
        } => {
            format!(
                "wfc-{}-{}-{}",
                slug(ad_unit_id),
                slug(network),
                slug(&apex_core::format_micros_short(*price_ecpm))
            )
        }
        Action::RemoveWaterfallEntry {
            ad_unit_id,
            entry_id,
        } => {
            format!("wfr-{}-{}", slug(ad_unit_id), slug(entry_id))
        }
    }
}

fn slug(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// The idempotency key for the platform write that applies `action`.
/// `SetCountryFloor` gets a dedicated `max-floor:` key (MAX has no core builder
/// for floors); every other variant reuses the apex-core builder verbatim.
pub fn idempotency_key(action: &Action) -> Result<String, PlatformError> {
    Ok(match action {
        Action::SetCountryFloor {
            ad_unit_id,
            country,
            floor_ecpm,
        } => max_floor_key(ad_unit_id, country, *floor_ecpm)?,
        Action::SetMappingEcpm {
            mediation_group_id,
            ad_unit_id,
            ecpm,
        } => admob_ecpm_key(mediation_group_id, ad_unit_id, *ecpm),
        Action::SetWaterfallPrice {
            ad_unit_id,
            entry_id,
            price_ecpm,
            ..
        } => levelplay_price_key(ad_unit_id, entry_id, *price_ecpm),
        Action::CreateWaterfallEntry {
            ad_unit_id,
            network,
            instance_name,
            price_ecpm,
        } => levelplay_create_key(ad_unit_id, network, instance_name, *price_ecpm),
        Action::RemoveWaterfallEntry {
            ad_unit_id,
            entry_id,
        } => levelplay_remove_key(ad_unit_id, entry_id),
    })
}

/// Deterministic platform-native experiment name for an action.
pub fn experiment_name(action: &Action) -> String {
    let name = format!("autopilot-{}", action_slug(action));
    max_experiment_key(&name, action.ad_unit_id())
}

pub fn platform_of(action: &Action) -> PlatformId {
    match action {
        Action::SetCountryFloor { .. } => PlatformId::AppLovinMax,
        Action::SetMappingEcpm { .. } => PlatformId::Admob,
        Action::SetWaterfallPrice { .. }
        | Action::CreateWaterfallEntry { .. }
        | Action::RemoveWaterfallEntry { .. } => PlatformId::LevelPlay,
    }
}

/// The platform operation name gating writability via `PlatformOps::writable`.
pub fn operation_of(action: &Action) -> &'static str {
    match action {
        Action::SetCountryFloor { .. } => "setCountryFloor",
        Action::SetMappingEcpm { .. } => "updateMediationGroup",
        Action::SetWaterfallPrice { .. } => "updateEntryPrice",
        Action::CreateWaterfallEntry { .. } => "createEntry",
        Action::RemoveWaterfallEntry { .. } => "removeEntry",
    }
}
