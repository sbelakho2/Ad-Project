//! Validation helpers and idempotency-key builders.
//! The key formats are the same strings the TypeScript reference layer used, so any
//! external system that observed keys from either implementation agrees.

use crate::errors::PlatformError;
use crate::money::{Micros, MICROS_PER_UNIT};
use crate::platform::PlatformId;

const MAX_ECPM_UNITS: i64 = 500; // platform contract upper bound, in whole units
const MAX_ECPM_MICROS: Micros = MAX_ECPM_UNITS * MICROS_PER_UNIT;

/// Trim + uppercase; reject anything that is not ISO 3166-1 alpha-2.
pub fn normalize_country(input: &str) -> Result<String, PlatformError> {
    let up = input.trim().to_uppercase();
    let ok = up.len() == 2 && up.bytes().all(|b| b.is_ascii_uppercase());
    if ok {
        Ok(up)
    } else {
        Err(PlatformError::Validation(format!(
            "invalid country code '{input}' (expected ISO 3166-1 alpha-2)"
        )))
    }
}

/// eCPM bounds for price-setting operations: (0, 500] units.
pub fn validate_ecpm(micros: Micros) -> Result<Micros, PlatformError> {
    // Note: 0 is invalid for a price, so the bound is 1..=MAX (clippy-suggested
    // `0..=MAX` would silently accept zero).
    if !(1..=MAX_ECPM_MICROS).contains(&micros) {
        return Err(PlatformError::Validation(format!(
            "eCPM {micros} micros out of range (0, {MAX_ECPM_UNITS} units]"
        )));
    }
    Ok(micros)
}

/// Floor bounds for MAX floors: [0, 500] units (zero is a legal floor = no floor).
pub fn validate_floor(micros: Micros) -> Result<Micros, PlatformError> {
    if !(0..=MAX_ECPM_MICROS).contains(&micros) {
        return Err(PlatformError::Validation(format!(
            "floor {micros} micros out of range [0, {MAX_ECPM_UNITS} units]"
        )));
    }
    Ok(micros)
}

/// Micros -> shortest decimal string, matching JS number formatting for the values
/// that occur in idempotency keys (up to 2 decimals from the TS layer).
pub fn format_micros_short(micros: Micros) -> String {
    if micros % MICROS_PER_UNIT == 0 {
        return (micros / MICROS_PER_UNIT).to_string();
    }
    let sign = if micros < 0 { "-" } else { "" };
    let abs = micros.unsigned_abs();
    let units = abs / (MICROS_PER_UNIT as u64);
    let frac = abs % (MICROS_PER_UNIT as u64);
    let mut frac_str = format!("{frac:06}");
    while frac_str.ends_with('0') {
        frac_str.pop();
    }
    format!("{sign}{units}.{frac_str}")
}

pub fn max_experiment_key(name: &str, ad_unit_id: &str) -> String {
    format!("max-experiment:{name}:{ad_unit_id}")
}

/// Price writers include the value: the same entry repriced later must not
/// collide with the earlier write's idempotency key (platforms treat a repeat
/// of a key as a duplicate of the original request, which would silently drop
/// a genuine change). This intentionally diverges from the TypeScript
/// reference, which had no value in these keys and no rerun-dedupe logic.
pub fn levelplay_price_key(ad_unit_id: &str, entry_id: &str, price_ecpm: Micros) -> String {
    format!(
        "levelplay-price:{ad_unit_id}:{entry_id}:{}",
        format_micros_short(price_ecpm)
    )
}

pub fn levelplay_create_key(
    ad_unit_id: &str,
    network: &str,
    instance_name: &str,
    price_ecpm: Micros,
) -> String {
    format!(
        "levelplay-create:{ad_unit_id}:{network}:{instance_name}:{}",
        format_micros_short(price_ecpm)
    )
}

/// MAX floors have no TypeScript-era key at all; the value is part of the key
/// for the same reason as the LevelPlay price key above. The country arrives
/// normalized, so reruns that only differ in input casing dedupe correctly.
pub fn max_floor_key(
    ad_unit_id: &str,
    country: &str,
    floor_ecpm: Micros,
) -> Result<String, PlatformError> {
    Ok(format!(
        "max-floor:{}:{}:{}",
        ad_unit_id,
        normalize_country(country)?,
        format_micros_short(floor_ecpm)
    ))
}

pub fn levelplay_remove_key(ad_unit_id: &str, entry_id: &str) -> String {
    format!("levelplay-remove:{ad_unit_id}:{entry_id}")
}

pub fn admob_ecpm_key(mediation_group_id: &str, ad_unit_id: &str, ecpm: Micros) -> String {
    format!(
        "admob-ecpm:{mediation_group_id}:{ad_unit_id}:{}",
        format_micros_short(ecpm)
    )
}

/// The platform each key family belongs to (used by audit checks that verify the
/// autopilot never mixes keys across platforms).
pub fn key_platform(key: &str) -> Option<PlatformId> {
    if key.starts_with("max-experiment:") {
        Some(PlatformId::AppLovinMax)
    } else if key.starts_with("levelplay-") {
        Some(PlatformId::LevelPlay)
    } else if key.starts_with("admob-") {
        Some(PlatformId::Admob)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_normalization() {
        assert_eq!(normalize_country(" ee ").expect("ok"), "EE");
        assert_eq!(normalize_country("ee").expect("ok"), "EE");
        assert!(normalize_country("EST").is_err());
        assert!(normalize_country("1!").is_err());
        assert!(normalize_country("").is_err());
        assert!(normalize_country("ÉÉ").is_err());
    }

    #[test]
    fn ecpm_bounds() {
        assert!(validate_ecpm(0).is_err());
        assert!(validate_ecpm(-1).is_err());
        assert!(validate_ecpm(MAX_ECPM_MICROS + 1).is_err());
        assert_eq!(validate_ecpm(1).expect("ok"), 1);
        assert_eq!(validate_ecpm(MAX_ECPM_MICROS).expect("ok"), MAX_ECPM_MICROS);
    }

    #[test]
    fn floor_bounds_allow_zero() {
        assert_eq!(validate_floor(0).expect("ok"), 0);
        assert!(validate_floor(-1).is_err());
        assert!(validate_floor(MAX_ECPM_MICROS + 1).is_err());
    }

    #[test]
    fn micros_formatting_matches_ts_number_rendering() {
        assert_eq!(format_micros_short(2_000_000), "2");
        assert_eq!(format_micros_short(1_750_000), "1.75");
        assert_eq!(format_micros_short(2_350_000), "2.35");
        assert_eq!(format_micros_short(2_345_000), "2.345");
        assert_eq!(format_micros_short(-1_500_000), "-1.5");
        assert_eq!(format_micros_short(1), "0.000001");
    }

    #[test]
    fn idempotency_key_formats_are_pinned() {
        assert_eq!(
            max_experiment_key("floors-ee", "u1"),
            "max-experiment:floors-ee:u1"
        );
        assert_eq!(
            levelplay_price_key("u1", "e1", 2_350_000),
            "levelplay-price:u1:e1:2.35"
        );
        assert_eq!(
            levelplay_create_key("u1", "mintegral", "MTG_1", 750_000),
            "levelplay-create:u1:mintegral:MTG_1:0.75"
        );
        assert_eq!(levelplay_remove_key("u1", "e9"), "levelplay-remove:u1:e9");
        assert_eq!(
            admob_ecpm_key("g1", "u2", 1_750_000),
            "admob-ecpm:g1:u2:1.75"
        );
        assert_eq!(admob_ecpm_key("g1", "u2", 2_000_000), "admob-ecpm:g1:u2:2");
        assert_eq!(
            max_floor_key("u1", " ee ", 2_500_000).unwrap(),
            "max-floor:u1:EE:2.5"
        );
        assert!(max_floor_key("u1", "EST", 1).is_err());
    }

    #[test]
    fn value_bearing_keys_never_collide_across_values() {
        // The repricing falsifier: same entry, different price must produce a
        // different key, or the platform would treat the new write as a
        // duplicate of the first and silently drop the change.
        assert_ne!(
            levelplay_price_key("u1", "e1", 1_100_000),
            levelplay_price_key("u1", "e1", 1_300_000)
        );
        assert_ne!(
            max_floor_key("u1", "EE", 2_000_000).unwrap(),
            max_floor_key("u1", "ee", 3_000_000).unwrap()
        );
        assert_ne!(
            admob_ecpm_key("g1", "u2", 1_000_000),
            admob_ecpm_key("g1", "u2", 1_500_000)
        );
    }

    #[test]
    fn key_platform_classification() {
        assert_eq!(
            key_platform("max-experiment:n:u"),
            Some(PlatformId::AppLovinMax)
        );
        assert_eq!(
            key_platform("levelplay-price:u:e"),
            Some(PlatformId::LevelPlay)
        );
        assert_eq!(key_platform("admob-ecpm:g:u:1"), Some(PlatformId::Admob));
        assert_eq!(key_platform("other"), None);
    }
}
