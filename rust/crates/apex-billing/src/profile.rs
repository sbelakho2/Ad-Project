//! The billing party profiles a lawful VAT invoice is made of.
//!
//! # Seller profile (deployment config, one shape)
//!
//! The seller's identity is a single JSON file named by the environment
//! variable `APEX_BILLING_PROFILE` (there is deliberately no parallel
//! `APEX_BILLING_PROFILE_*` variable set — one file, one shape):
//!
//! ```json
//! {
//!   "legalName": "Apex Media OÜ",
//!   "addressLines": ["Sepapaja tn 6", "15551 Tallinn"],
//!   "country": "EE",
//!   "vatNumber": "EE123456789",
//!   "vatRateBp": 2200
//! }
//! ```
//!
//! `vatRateBp` is the domestic VAT rate in basis points; when the environment
//! variable `APEX_BILLING_VAT_RATE_BP` is set it overrides the file's value.
//! Load through [`BillingProfile::from_env`], [`BillingProfile::load`] or
//! [`BillingProfile::from_json_str`]; every entry point validates and refuses
//! with [`BillingError::MissingProfileField`] (or the matching shaped error)
//! naming the missing field, so a deployment that is incomplete fails at
//! startup instead of printing a half-empty invoice. Values are trimmed and
//! the country is canonicalised to uppercase at load.
//!
//! # Customer profile (carried on the invoice)
//!
//! The customer's legal name, address lines, country and (optionally) VAT
//! number travel on the [`crate::invoice::Invoice`] itself
//! ([`crate::invoice::Invoice::with_customer`]); today's caller may not have
//! them yet, so the invoice field is optional and the renderer refuses with a
//! typed error rather than printing blanks. The VAT number is required only
//! when the reverse-charge wording will be stated; if present it must always
//! pass the minimal shape check of [`crate::vat::is_valid_vat_number`].

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::errors::BillingError;
use crate::vat::{is_valid_country_code, is_valid_vat_number, MAX_VAT_RATE_BP};

/// Environment variable naming the seller profile JSON file.
pub const PROFILE_ENV: &str = "APEX_BILLING_PROFILE";

/// Environment variable overriding the profile's `vatRateBp`.
pub const VAT_RATE_ENV: &str = "APEX_BILLING_VAT_RATE_BP";

/// The seller's legal identity, as required on every VAT invoice: legal name,
/// address lines, country and VAT number, plus the domestic VAT rate. Loaded
/// from the JSON file named by `APEX_BILLING_PROFILE`; see the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingProfile {
    /// Company legal name.
    pub legal_name: String,
    /// Address lines, printed in order.
    pub address_lines: Vec<String>,
    /// Country, canonicalised to an uppercase two-letter code.
    pub country: String,
    /// VAT number (shape-checked only; no VIES call).
    pub vat_number: String,
    /// Domestic VAT rate in basis points (`0..=10000`).
    pub vat_rate_bp: u16,
}

/// The billed party's legal identity, carried on the invoice input. The VAT
/// number is present only for businesses that cite one; it is required for
/// the reverse-charge treatment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerProfile {
    /// Company legal name.
    pub legal_name: String,
    /// Address lines, printed in order.
    pub address_lines: Vec<String>,
    /// Country as a two-letter code (canonical uppercase; comparisons are
    /// case-insensitive).
    pub country: String,
    /// VAT number when the customer cites one (shape-checked only).
    #[serde(default)]
    pub vat_number: Option<String>,
}

impl BillingProfile {
    /// Parse and validate a profile from JSON text. `vatRateBp` comes from
    /// the JSON; use [`Self::from_env_values`] for the environment override.
    pub fn from_json_str(json: &str) -> Result<Self, BillingError> {
        let raw: RawProfile = serde_json::from_str(json).map_err(|error| {
            BillingError::Malformed(format!("billing profile is not valid JSON: {error}"))
        })?;
        Self::from_raw(raw, None)
    }

    /// Load and validate a profile from a JSON file.
    pub fn load(path: &Path) -> Result<Self, BillingError> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            BillingError::Malformed(format!(
                "billing profile at {} could not be read: {error}",
                path.display()
            ))
        })?;
        Self::from_json_str(&text)
    }

    /// Compose a profile from raw environment values: the profile file path
    /// (`APEX_BILLING_PROFILE`) and an optional rate in basis points
    /// (`APEX_BILLING_VAT_RATE_BP`). Pure so deployments and tests can
    /// exercise it without touching the process environment.
    pub fn from_env_values(
        profile_path: Option<&str>,
        vat_rate_bp: Option<&str>,
    ) -> Result<Self, BillingError> {
        let path = profile_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or(BillingError::MissingProfileField {
                party: "seller",
                field: PROFILE_ENV,
            })?;
        let text = std::fs::read_to_string(path).map_err(|error| {
            BillingError::Malformed(format!(
                "billing profile at {path} could not be read: {error}"
            ))
        })?;
        let raw: RawProfile = serde_json::from_str(&text).map_err(|error| {
            BillingError::Malformed(format!(
                "billing profile at {path} is not valid JSON: {error}"
            ))
        })?;
        let rate_override = match vat_rate_bp.map(str::trim).filter(|rate| !rate.is_empty()) {
            Some(rate) => Some(parse_rate_bp(rate)?),
            None => None,
        };
        Self::from_raw(raw, rate_override)
    }

    /// Load the seller profile from the process environment: the JSON file
    /// named by `APEX_BILLING_PROFILE`, with `APEX_BILLING_VAT_RATE_BP`
    /// overriding the file's `vatRateBp` when set. Call this at startup and
    /// refuse to run when it errors.
    pub fn from_env() -> Result<Self, BillingError> {
        let path = std::env::var(PROFILE_ENV).ok();
        let rate = std::env::var(VAT_RATE_ENV).ok();
        Self::from_env_values(path.as_deref(), rate.as_deref())
    }

    /// Validate an already-built profile. Every entry point runs this, so an
    /// explicit call is only needed for profiles assembled in code.
    pub fn validate(&self) -> Result<(), BillingError> {
        if self.legal_name.trim().is_empty() {
            return Err(BillingError::MissingProfileField {
                party: "seller",
                field: "legalName",
            });
        }
        if self.address_lines.is_empty()
            || self.address_lines.iter().any(|line| line.trim().is_empty())
        {
            return Err(BillingError::MissingProfileField {
                party: "seller",
                field: "addressLines",
            });
        }
        if self.country.trim().is_empty() {
            return Err(BillingError::MissingProfileField {
                party: "seller",
                field: "country",
            });
        }
        if !is_valid_country_code(&self.country) {
            return Err(BillingError::InvalidCountryCode {
                party: "seller",
                value: self.country.clone(),
            });
        }
        check_vat_number("seller", &self.vat_number)?;
        checked_rate_bp(u32::from(self.vat_rate_bp))?;
        Ok(())
    }

    fn from_raw(raw: RawProfile, rate_override: Option<u32>) -> Result<Self, BillingError> {
        let legal_name = required_value(raw.legal_name, "legalName")?;
        let address_lines = required_address_lines(raw.address_lines)?;
        let country = required_value(raw.country, "country")?;
        if !is_valid_country_code(&country) {
            return Err(BillingError::InvalidCountryCode {
                party: "seller",
                value: country,
            });
        }
        let vat_number = required_value(raw.vat_number, "vatNumber")?;
        check_vat_number("seller", &vat_number)?;
        let rate_bp = match rate_override.or(raw.vat_rate_bp) {
            Some(rate_bp) => checked_rate_bp(rate_bp)?,
            None => {
                return Err(BillingError::MissingProfileField {
                    party: "seller",
                    field: "vatRateBp",
                })
            }
        };
        let profile = Self {
            legal_name: legal_name.trim().to_string(),
            address_lines: address_lines
                .iter()
                .map(|line| line.trim().to_string())
                .collect(),
            country: country.trim().to_ascii_uppercase(),
            vat_number: vat_number.trim().to_string(),
            vat_rate_bp: rate_bp,
        };
        profile.validate()?;
        Ok(profile)
    }
}

impl CustomerProfile {
    /// Validate the customer's identity before a VAT treatment (or an
    /// invoice document) is derived from it. Every missing or shaped-wrong
    /// field is a typed refusal naming it.
    pub fn validate(&self) -> Result<(), BillingError> {
        if self.legal_name.trim().is_empty() {
            return Err(BillingError::MissingProfileField {
                party: "customer",
                field: "legalName",
            });
        }
        if self.address_lines.is_empty()
            || self.address_lines.iter().any(|line| line.trim().is_empty())
        {
            return Err(BillingError::MissingProfileField {
                party: "customer",
                field: "addressLines",
            });
        }
        if self.country.trim().is_empty() {
            return Err(BillingError::MissingProfileField {
                party: "customer",
                field: "country",
            });
        }
        if !is_valid_country_code(&self.country) {
            return Err(BillingError::InvalidCountryCode {
                party: "customer",
                value: self.country.clone(),
            });
        }
        if let Some(vat_number) = &self.vat_number {
            check_vat_number("customer", vat_number)?;
        }
        Ok(())
    }
}

/// The JSON shape of `APEX_BILLING_PROFILE`, all fields optional so every
/// missing one is reported by name instead of serde's first-error-wins.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawProfile {
    #[serde(default)]
    legal_name: Option<String>,
    #[serde(default)]
    address_lines: Option<Vec<String>>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    vat_number: Option<String>,
    #[serde(default)]
    vat_rate_bp: Option<u32>,
}

fn required_value(value: Option<String>, field: &'static str) -> Result<String, BillingError> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(BillingError::MissingProfileField {
            party: "seller",
            field,
        }),
    }
}

fn required_address_lines(value: Option<Vec<String>>) -> Result<Vec<String>, BillingError> {
    match value {
        Some(lines) if !lines.is_empty() && lines.iter().all(|line| !line.trim().is_empty()) => {
            Ok(lines)
        }
        _ => Err(BillingError::MissingProfileField {
            party: "seller",
            field: "addressLines",
        }),
    }
}

fn check_vat_number(party: &'static str, value: &str) -> Result<(), BillingError> {
    if value.trim().is_empty() {
        return Err(BillingError::MissingProfileField {
            party,
            field: "vatNumber",
        });
    }
    if !is_valid_vat_number(value) {
        return Err(BillingError::InvalidVatNumber {
            party,
            detail: format!("{value:?} is not 4..=14 ASCII alphanumeric characters"),
        });
    }
    Ok(())
}

fn parse_rate_bp(raw: &str) -> Result<u32, BillingError> {
    raw.trim()
        .parse::<u32>()
        .map_err(|_| BillingError::InvalidVatRate {
            detail: format!("{raw:?} is not a number of basis points"),
        })
}

fn checked_rate_bp(rate_bp: u32) -> Result<u16, BillingError> {
    if rate_bp > u32::from(MAX_VAT_RATE_BP) {
        return Err(BillingError::InvalidVatRate {
            detail: format!("{rate_bp} bp is outside 0..={MAX_VAT_RATE_BP}"),
        });
    }
    Ok(rate_bp as u16)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use super::*;

    /// Serializes the env-mutating tests within this process; no other test
    /// in the crate reads these variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const PROFILE_JSON: &str = r#"{
        "legalName": "  Apex Media OÜ  ",
        "addressLines": ["  Sepapaja tn 6 ", "15551 Tallinn"],
        "country": "ee",
        "vatNumber": " EE123456789 ",
        "vatRateBp": 1900
    }"#;

    #[test]
    fn parses_and_normalises_a_full_profile() {
        let profile = BillingProfile::from_json_str(PROFILE_JSON).unwrap();
        assert_eq!(profile.legal_name, "Apex Media OÜ");
        assert_eq!(profile.address_lines, ["Sepapaja tn 6", "15551 Tallinn"]);
        assert_eq!(profile.country, "EE");
        assert_eq!(profile.vat_number, "EE123456789");
        assert_eq!(profile.vat_rate_bp, 1900);
    }

    #[test]
    fn every_missing_profile_field_is_named() {
        let cases: [(&str, &str); 6] = [
            (
                r#"{"addressLines":["a"],"country":"EE","vatNumber":"EE123456789","vatRateBp":1900}"#,
                "legalName",
            ),
            (
                r#"{"legalName":"A","country":"EE","vatNumber":"EE123456789","vatRateBp":1900}"#,
                "addressLines",
            ),
            (
                r#"{"legalName":"A","addressLines":[],"country":"EE","vatNumber":"EE123456789","vatRateBp":1900}"#,
                "addressLines",
            ),
            (
                r#"{"legalName":"A","addressLines":[" "],"country":"EE","vatNumber":"EE123456789","vatRateBp":1900}"#,
                "addressLines",
            ),
            (
                r#"{"legalName":"A","addressLines":["a"],"vatNumber":"EE123456789","vatRateBp":1900}"#,
                "country",
            ),
            (
                r#"{"legalName":"A","addressLines":["a"],"country":"EE","vatRateBp":1900}"#,
                "vatNumber",
            ),
        ];
        for (json, field) in cases {
            assert_eq!(
                BillingProfile::from_json_str(json),
                Err(BillingError::MissingProfileField {
                    party: "seller",
                    field,
                }),
                "{field} must be named"
            );
        }
        // An empty VAT number is a missing field, not a shape error.
        assert_eq!(
            BillingProfile::from_json_str(
                r#"{"legalName":"A","addressLines":["a"],"country":"EE","vatNumber":" ","vatRateBp":1900}"#
            ),
            Err(BillingError::MissingProfileField {
                party: "seller",
                field: "vatNumber",
            })
        );
        // The rate may come from neither the file nor the environment.
        assert_eq!(
            BillingProfile::from_json_str(
                r#"{"legalName":"A","addressLines":["a"],"country":"EE","vatNumber":"EE123456789"}"#
            ),
            Err(BillingError::MissingProfileField {
                party: "seller",
                field: "vatRateBp",
            })
        );
    }

    #[test]
    fn shaped_wrong_profile_fields_are_refused() {
        assert_eq!(
            BillingProfile::from_json_str(
                r#"{"legalName":"A","addressLines":["a"],"country":"E1","vatNumber":"EE123456789","vatRateBp":1900}"#
            ),
            Err(BillingError::InvalidCountryCode {
                party: "seller",
                value: "E1".into(),
            })
        );
        assert!(matches!(
            BillingProfile::from_json_str(
                r#"{"legalName":"A","addressLines":["a"],"country":"EE","vatNumber":"EE-123","vatRateBp":1900}"#
            ),
            Err(BillingError::InvalidVatNumber {
                party: "seller",
                ..
            })
        ));
        assert!(matches!(
            BillingProfile::from_json_str(
                r#"{"legalName":"A","addressLines":["a"],"country":"EE","vatNumber":"AB","vatRateBp":1900}"#
            ),
            Err(BillingError::InvalidVatNumber {
                party: "seller",
                ..
            })
        ));
        // Not JSON at all, an unknown field, and a wrong type all fail closed.
        assert!(matches!(
            BillingProfile::from_json_str("not json"),
            Err(BillingError::Malformed(_))
        ));
        assert!(matches!(
            BillingProfile::from_json_str(
                r#"{"legalName":"A","addressLines":["a"],"country":"EE","vatNumber":"EE123456789","vatRateBp":1900,"extra":true}"#
            ),
            Err(BillingError::Malformed(_))
        ));
        assert!(matches!(
            BillingProfile::from_json_str(
                r#"{"legalName":"A","addressLines":"a","country":"EE","vatNumber":"EE123456789","vatRateBp":1900}"#
            ),
            Err(BillingError::Malformed(_))
        ));
    }

    #[test]
    fn rate_bounds_are_enforced_everywhere() {
        // 10000 bp (100%) is the inclusive bound.
        let at_bound = BillingProfile::from_json_str(
            r#"{"legalName":"A","addressLines":["a"],"country":"EE","vatNumber":"EE123456789","vatRateBp":10000}"#,
        )
        .unwrap();
        assert_eq!(at_bound.vat_rate_bp, 10_000);
        // One basis point over is refused, including values u16 could not hold.
        for json in [
            r#"{"legalName":"A","addressLines":["a"],"country":"EE","vatNumber":"EE123456789","vatRateBp":10001}"#,
            r#"{"legalName":"A","addressLines":["a"],"country":"EE","vatNumber":"EE123456789","vatRateBp":70000}"#,
        ] {
            assert!(matches!(
                BillingProfile::from_json_str(json),
                Err(BillingError::InvalidVatRate { ref detail }) if detail.contains("10000")
            ));
        }
        // A rate assembled in code can also be validated explicitly.
        let mut profile = BillingProfile::from_json_str(PROFILE_JSON).unwrap();
        profile.vat_rate_bp = 10_001;
        assert!(matches!(
            profile.validate(),
            Err(BillingError::InvalidVatRate { .. })
        ));
        let mut named = profile.clone();
        named.legal_name = "  ".into();
        assert_eq!(
            named.validate(),
            Err(BillingError::MissingProfileField {
                party: "seller",
                field: "legalName",
            })
        );
        named = profile.clone();
        named.address_lines = vec![];
        assert_eq!(
            named.validate(),
            Err(BillingError::MissingProfileField {
                party: "seller",
                field: "addressLines",
            })
        );
        named = profile.clone();
        named.country = "x".into();
        assert!(matches!(
            named.validate(),
            Err(BillingError::InvalidCountryCode { .. })
        ));
        named = profile;
        named.vat_number = "!!!".into();
        assert!(matches!(
            named.validate(),
            Err(BillingError::InvalidVatNumber { .. })
        ));
    }

    #[test]
    fn load_reads_the_named_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("billing-profile.json");
        std::fs::write(&path, PROFILE_JSON).unwrap();
        let profile = BillingProfile::load(&path).unwrap();
        assert_eq!(profile.country, "EE");
        assert_eq!(profile.vat_rate_bp, 1900);
        assert!(matches!(
            BillingProfile::load(&dir.path().join("absent.json")),
            Err(BillingError::Malformed(_))
        ));
    }

    #[test]
    fn from_env_values_composes_file_and_rate_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("billing-profile.json");
        std::fs::write(&path, PROFILE_JSON).unwrap();
        let path = path.to_str().expect("utf-8 temp path");

        // No path at all: typed refusal naming the environment variable.
        assert_eq!(
            BillingProfile::from_env_values(None, Some("2000")),
            Err(BillingError::MissingProfileField {
                party: "seller",
                field: "APEX_BILLING_PROFILE",
            })
        );
        assert_eq!(
            BillingProfile::from_env_values(Some("  "), None),
            Err(BillingError::MissingProfileField {
                party: "seller",
                field: "APEX_BILLING_PROFILE",
            })
        );
        // Empty override strings count as unset; the file's rate is used.
        assert_eq!(
            BillingProfile::from_env_values(Some(path), Some(" "))
                .unwrap()
                .vat_rate_bp,
            1900
        );
        // A set override wins over the file.
        assert_eq!(
            BillingProfile::from_env_values(Some(path), Some("2200"))
                .unwrap()
                .vat_rate_bp,
            2200
        );
        // Unparseable and out-of-range overrides are typed refusals.
        assert!(matches!(
            BillingProfile::from_env_values(Some(path), Some("twenty")),
            Err(BillingError::InvalidVatRate { ref detail }) if detail.contains("twenty")
        ));
        assert!(matches!(
            BillingProfile::from_env_values(Some(path), Some("10001")),
            Err(BillingError::InvalidVatRate { ref detail }) if detail.contains("10000")
        ));
        assert!(matches!(
            BillingProfile::from_env_values(Some(path), Some("-1")),
            Err(BillingError::InvalidVatRate { .. })
        ));

        // An unreadable path and a malformed file both fail closed, typed.
        assert!(matches!(
            BillingProfile::from_env_values(Some("/nonexistent/apex/profile.json"), None),
            Err(BillingError::Malformed(ref detail)) if detail.contains("could not be read")
        ));
        let corrupt = dir.path().join("corrupt.json");
        std::fs::write(&corrupt, "{ not json").unwrap();
        assert!(matches!(
            BillingProfile::from_env_values(Some(corrupt.to_str().unwrap()), None),
            Err(BillingError::Malformed(ref detail)) if detail.contains("not valid JSON")
        ));
    }

    #[test]
    fn from_env_reads_the_process_environment() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("billing-profile.json");
        std::fs::write(&path, PROFILE_JSON).unwrap();
        let saved_path = std::env::var(PROFILE_ENV).ok();
        let saved_rate = std::env::var(VAT_RATE_ENV).ok();

        std::env::remove_var(PROFILE_ENV);
        std::env::remove_var(VAT_RATE_ENV);
        assert_eq!(
            BillingProfile::from_env(),
            Err(BillingError::MissingProfileField {
                party: "seller",
                field: "APEX_BILLING_PROFILE",
            })
        );

        std::env::set_var(PROFILE_ENV, &path);
        assert_eq!(BillingProfile::from_env().unwrap().vat_rate_bp, 1900);
        std::env::set_var(VAT_RATE_ENV, "2200");
        let overridden = BillingProfile::from_env().unwrap();
        assert_eq!(overridden.vat_rate_bp, 2200);
        assert_eq!(overridden.country, "EE");

        match saved_path {
            Some(value) => std::env::set_var(PROFILE_ENV, value),
            None => std::env::remove_var(PROFILE_ENV),
        }
        match saved_rate {
            Some(value) => std::env::set_var(VAT_RATE_ENV, value),
            None => std::env::remove_var(VAT_RATE_ENV),
        }
    }

    #[test]
    fn every_missing_customer_field_is_named() {
        let base = CustomerProfile {
            legal_name: "Buyer OÜ".into(),
            address_lines: vec!["Telliskivi 1".into()],
            country: "DE".into(),
            vat_number: Some("DE123456789".into()),
        };
        assert!(base.validate().is_ok());

        let mut missing_name = base.clone();
        missing_name.legal_name = " ".into();
        assert_eq!(
            missing_name.validate(),
            Err(BillingError::MissingProfileField {
                party: "customer",
                field: "legalName",
            })
        );
        let mut missing_address = base.clone();
        missing_address.address_lines = vec![];
        assert_eq!(
            missing_address.validate(),
            Err(BillingError::MissingProfileField {
                party: "customer",
                field: "addressLines",
            })
        );
        let mut empty_address_line = base.clone();
        empty_address_line.address_lines = vec!["".into()];
        assert_eq!(
            empty_address_line.validate(),
            Err(BillingError::MissingProfileField {
                party: "customer",
                field: "addressLines",
            })
        );
        let mut missing_country = base.clone();
        missing_country.country = String::new();
        assert_eq!(
            missing_country.validate(),
            Err(BillingError::MissingProfileField {
                party: "customer",
                field: "country",
            })
        );
        let mut invalid_country = base.clone();
        invalid_country.country = "D3".into();
        assert_eq!(
            invalid_country.validate(),
            Err(BillingError::InvalidCountryCode {
                party: "customer",
                value: "D3".into(),
            })
        );
        // The VAT number is optional, but an empty or malformed one is named.
        let mut no_vat = base.clone();
        no_vat.vat_number = None;
        assert!(no_vat.validate().is_ok());
        let mut empty_vat = base.clone();
        empty_vat.vat_number = Some("".into());
        assert_eq!(
            empty_vat.validate(),
            Err(BillingError::MissingProfileField {
                party: "customer",
                field: "vatNumber",
            })
        );
        let mut bad_vat = base;
        bad_vat.vat_number = Some("DE 1234".into());
        assert!(matches!(
            bad_vat.validate(),
            Err(BillingError::InvalidVatNumber {
                party: "customer",
                ..
            })
        ));
    }
}
