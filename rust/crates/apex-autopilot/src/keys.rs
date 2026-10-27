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
            instance_name,
            price_ecpm,
        } => {
            format!(
                "wfc-{}-{}-{}-{}",
                slug(ad_unit_id),
                slug(network),
                slug(instance_name),
                slug(&apex_core::format_micros_short(*price_ecpm))
            )
        }
        Action::RemoveWaterfallEntry {
            ad_unit_id,
            entry_id,
        } => {
            format!("wfr-{}-{}", slug(ad_unit_id), slug(entry_id))
        }
        Action::SetLevelplayInstancePrice {
            instance_id,
            price_micros,
        } => {
            format!(
                "ip-{}-{}",
                slug(instance_id),
                slug(&apex_core::format_micros_short(*price_micros))
            )
        }
        Action::SetAdmobMediationFloor {
            group_id,
            floor_micros,
        } => {
            format!(
                "mf-{}-{}",
                slug(group_id),
                slug(&apex_core::format_micros_short(*floor_micros))
            )
        }
        Action::SetMaxFrequencyCap {
            ad_unit_id,
            cap_type,
            session_limit,
            day_limit,
            minute_frequency,
            ..
        } => format!(
            "cap-{}-{}-{}-{}-{}",
            slug(ad_unit_id),
            cap_type.as_str(),
            session_limit.unwrap_or(0),
            day_limit.unwrap_or(0),
            minute_frequency.unwrap_or(0)
        ),
        Action::SetLevelplayGroupCapping {
            group_id,
            session_limit,
            interstitial_limit,
            day_limit,
        } => format!(
            "lpc-{}-{}-{}-{}",
            slug(group_id),
            session_limit.unwrap_or(0),
            interstitial_limit.unwrap_or(0),
            day_limit.unwrap_or(0)
        ),
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
        Action::SetLevelplayInstancePrice {
            instance_id,
            price_micros,
        } => levelplay_price_key(instance_id, instance_id, *price_micros),
        Action::SetAdmobMediationFloor {
            group_id,
            floor_micros,
        } => admob_ecpm_key(group_id, group_id, *floor_micros),
        Action::SetMaxFrequencyCap {
            ad_unit_id,
            cap_type,
            countries,
            session_limit,
            day_limit,
            minute_frequency,
        } => format!(
            "max-frequency-cap:{}:{}:{}:{}:{}:{}",
            ad_unit_id,
            cap_type.as_str(),
            session_limit.unwrap_or(0),
            day_limit.unwrap_or(0),
            minute_frequency.unwrap_or(0),
            countries.join(",")
        ),
        Action::SetLevelplayGroupCapping {
            group_id,
            session_limit,
            interstitial_limit,
            day_limit,
        } => format!(
            "levelplay-group-capping:{}:{}:{}:{}",
            group_id,
            session_limit.unwrap_or(0),
            interstitial_limit.unwrap_or(0),
            day_limit.unwrap_or(0)
        ),
    })
}

/// Deterministic platform-native experiment name for an action.
pub fn experiment_name(action: &Action) -> String {
    let name = format!("autopilot-{}", action_slug(action));
    max_experiment_key(&name, action.ad_unit_id())
}

/// A unique, immutable identity for one experiment instance.
///
/// The platform experiment name is reusable (a restart on the same action
/// derives the same name), so it can never be the billing identity. Every
/// attempt gets a fresh instance id before its pre-registration is signed;
/// the pre-registration, start record, result, fee line and monthly billing
/// line all carry this one id.
///
/// Generation is deterministic from the logical attempt: same inputs and the
/// same ledger state replay the same id (so concurrent and serial passes
/// produce the same signed ledger), while a later attempt with the same name
/// counts prior attempts and gets a different id.
pub fn experiment_instance_id(
    name: &str,
    now_unix_ms: i64,
    idempotency_key: &str,
    attempt: u64,
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for field in [
        name,
        idempotency_key,
        &now_unix_ms.to_string(),
        &attempt.to_string(),
    ] {
        for byte in field.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // Keep the id usable as a billing reference (the collection layer caps
    // references at 80 characters). The hash covers the full name, so a
    // truncated prefix can never collide.
    let prefix = name.get(..name.len().min(24)).unwrap_or(name);
    format!("{prefix}@{hash:016x}-{attempt:08x}")
}

pub fn platform_of(action: &Action) -> PlatformId {
    match action {
        Action::SetCountryFloor { .. } => PlatformId::AppLovinMax,
        Action::SetMappingEcpm { .. } | Action::SetAdmobMediationFloor { .. } => PlatformId::Admob,
        Action::SetWaterfallPrice { .. }
        | Action::CreateWaterfallEntry { .. }
        | Action::RemoveWaterfallEntry { .. }
        | Action::SetLevelplayInstancePrice { .. } => PlatformId::LevelPlay,
        Action::SetMaxFrequencyCap { .. } => PlatformId::AppLovinMax,
        Action::SetLevelplayGroupCapping { .. } => PlatformId::LevelPlay,
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
        Action::SetLevelplayInstancePrice { .. } => "updateEntryPrice",
        Action::SetAdmobMediationFloor { .. } => "updateMediationGroup",
        Action::SetMaxFrequencyCap { .. } => "setFrequencyCap",
        Action::SetLevelplayGroupCapping { .. } => "updateGroupCapping",
    }
}
