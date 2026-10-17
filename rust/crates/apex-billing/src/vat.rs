//! EU membership and the VAT treatment of an invoice.
//!
//! EU membership is derived from the small constant table below — the 27
//! member states as ISO 3166-1 alpha-2 codes, sorted (Greece is `GR`, the ISO
//! code, not the `EL` VAT-number prefix). The table is tested: 27 unique
//! entries, sorted, known members present, known non-members absent. Nothing
//! else is consulted.
//!
//! **No live VIES (or any other) call is ever made.** A VAT number is only
//! checked for *shape*: non-empty, ASCII alphanumeric, 4..=14 characters,
//! optional country prefix, no separators. "Valid" in this module therefore
//! means "plausible on its face and eligible to state the reverse-charge
//! wording" — a live registry check is out of scope and is documented as such
//! wherever the treatment is stated.
//!
//! The treatment rules per customer are:
//!
//! - outside the EU: no VAT is stated at all (no VAT line);
//! - same country as the seller: the profile's domestic rate
//!   (`vat_rate_bp`, configured via [`crate::profile::VAT_RATE_ENV`]);
//! - EU cross-border with a customer VAT number present (and observed to
//!   validate): reverse charge — the words "reverse charge" stand where the
//!   tax would otherwise be stated;
//! - EU cross-border without a VAT number, or a non-EU seller: the domestic
//!   rate is charged as the documented fallback (no reverse charge without a
//!   number to cite).

use serde::{Deserialize, Serialize};

use crate::errors::BillingError;
use crate::profile::{BillingProfile, CustomerProfile};

/// The highest VAT rate a profile may carry, in basis points (100%).
pub const MAX_VAT_RATE_BP: u16 = 10_000;

/// The 27 EU member states as ISO 3166-1 alpha-2 codes, sorted.
pub const EU_MEMBER_STATES: [&str; 27] = [
    "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GR", "HR", "HU", "IE", "IT",
    "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI", "SK",
];

/// Whether `country` is an EU member state (case-insensitive; surrounding
/// whitespace ignored). Membership is decided by [`EU_MEMBER_STATES`] alone.
#[must_use]
pub fn is_eu_member(country: &str) -> bool {
    let code = country.trim();
    EU_MEMBER_STATES
        .iter()
        .any(|member| member.eq_ignore_ascii_case(code))
}

/// Whether `country` is exactly two ASCII letters. Codes are canonicalised to
/// uppercase at profile load; comparisons here and in [`treatment_for`] are
/// case-insensitive.
#[must_use]
pub fn is_valid_country_code(country: &str) -> bool {
    let code = country.trim();
    code.chars().count() == 2 && code.chars().all(|c| c.is_ascii_alphabetic())
}

/// The minimal VAT-number shape check: non-empty after trimming, at least 4
/// and at most 14 ASCII alphanumeric characters, no separators. The country
/// prefix (for example `EE` in `EE123456789`) is optional; nothing is
/// verified against VIES or any registry.
#[must_use]
pub fn is_valid_vat_number(number: &str) -> bool {
    let trimmed = number.trim();
    let length = trimmed.chars().count();
    (4..=14).contains(&length) && trimmed.chars().all(|c| c.is_ascii_alphanumeric())
}

/// The VAT treatment of an invoice, decided by [`treatment_for`] and recorded
/// on the invoice so the renderer can state (or refuse to state) tax without
/// guessing. Serialized camelCase for the invoice register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VatTreatment {
    /// No treatment applied yet (a draft; the renderer and the register
    /// refuse it).
    #[default]
    NotApplied,
    /// Seller and customer in the same country: the profile's domestic rate.
    Domestic,
    /// EU cross-border business with a customer VAT number: the invoice
    /// states the words "reverse charge" where tax would otherwise appear.
    ReverseCharge,
    /// Customer outside the EU: no VAT is stated at all.
    OutsideEu,
}

impl std::fmt::Display for VatTreatment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotApplied => "not applied",
            Self::Domestic => "domestic",
            Self::ReverseCharge => "reverse charge",
            Self::OutsideEu => "outside the EU",
        })
    }
}

/// Derive the VAT treatment for `customer` from `seller`'s profile.
///
/// Both profiles are validated first, so a missing or shaped-wrong field is a
/// typed refusal naming the field; an invalid customer VAT number is refused
/// (never silently downgraded — charging VAT to a business citing a
/// malformed number is worse than refusing). See the module docs for the
/// exact rules and the documented EU-cross-border fallback.
pub fn treatment_for(
    seller: &BillingProfile,
    customer: &CustomerProfile,
) -> Result<VatTreatment, BillingError> {
    seller.validate()?;
    customer.validate()?;
    let seller_country = seller.country.trim().to_ascii_uppercase();
    let customer_country = customer.country.trim().to_ascii_uppercase();
    if !is_eu_member(&customer_country) {
        return Ok(VatTreatment::OutsideEu);
    }
    if seller_country == customer_country {
        return Ok(VatTreatment::Domestic);
    }
    if is_eu_member(&seller_country) && customer.vat_number.is_some() {
        return Ok(VatTreatment::ReverseCharge);
    }
    Ok(VatTreatment::Domestic)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn seller() -> BillingProfile {
        BillingProfile {
            legal_name: "Apex Media OÜ".into(),
            address_lines: vec!["Sepapaja tn 6".into(), "15551 Tallinn".into()],
            country: "EE".into(),
            vat_number: "EE123456789".into(),
            vat_rate_bp: 1_900,
        }
    }

    fn customer(country: &str, vat_number: Option<&str>) -> CustomerProfile {
        CustomerProfile {
            legal_name: "Buyer OÜ".into(),
            address_lines: vec!["Telliskivi 1".into()],
            country: country.into(),
            vat_number: vat_number.map(str::to_string),
        }
    }

    #[test]
    fn the_eu_table_is_the_27_sorted_unique_members() {
        assert_eq!(EU_MEMBER_STATES.len(), 27);
        let mut sorted = EU_MEMBER_STATES;
        sorted.sort_unstable();
        assert_eq!(EU_MEMBER_STATES, sorted, "must stay sorted");
        let unique: std::collections::HashSet<&str> = EU_MEMBER_STATES.into_iter().collect();
        assert_eq!(unique.len(), 27, "no duplicates");
        for member in [
            "AT", "BE", "BG", "CY", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GR", "HR", "HU",
            "IE", "IT", "LT", "LU", "LV", "MT", "NL", "PL", "PT", "RO", "SE", "SI", "SK",
        ] {
            assert!(is_eu_member(member), "{member} must be a member");
        }
        for non_member in ["GB", "CH", "NO", "TR", "UA", "US", "LI", "IS"] {
            assert!(!is_eu_member(non_member), "{non_member} is not in the EU");
        }
        // Membership is case-insensitive and trims surrounding whitespace.
        assert!(is_eu_member("de"));
        assert!(is_eu_member(" FR "));
        assert!(!is_eu_member(""));
        assert!(!is_eu_member("EEE"));
    }

    #[test]
    fn country_codes_are_two_ascii_letters() {
        for valid in ["EE", "ee", "De", " AA "] {
            assert!(is_valid_country_code(valid), "{valid:?}");
        }
        for invalid in ["", "E", "EEE", "E1", "1E", "ÜÜ", "E €"] {
            assert!(!is_valid_country_code(invalid), "{invalid:?}");
        }
    }

    #[test]
    fn vat_numbers_are_shape_checked_only() {
        for valid in ["EE123456789", " DE123456789 ", "12345678", "ABCdef1234"] {
            assert!(is_valid_vat_number(valid), "{valid:?}");
        }
        for invalid in [
            "",
            "   ",
            "ABC",
            "AB-12",
            "DE 1234",
            "123456789012345",
            "DE_1234",
        ] {
            assert!(!is_valid_vat_number(invalid), "{invalid:?}");
        }
        // 4 and 14 are the inclusive bounds.
        assert!(is_valid_vat_number("1234"));
        assert!(is_valid_vat_number("12345678901234"));
        assert!(!is_valid_vat_number("123456789012345"));
    }

    #[test]
    fn treatment_is_domestic_when_countries_match() {
        // Same EU country, even with different case on the customer side.
        assert_eq!(
            treatment_for(&seller(), &customer("EE", None)).unwrap(),
            VatTreatment::Domestic
        );
        assert_eq!(
            treatment_for(&seller(), &customer("ee", Some("EE987654321"))).unwrap(),
            VatTreatment::Domestic
        );
        // Outside the EU the customer is never charged EU VAT, even in the
        // seller's own country: "no VAT outside the EU" wins over "domestic".
        let mut non_eu_seller = seller();
        non_eu_seller.country = "US".into();
        assert_eq!(
            treatment_for(&non_eu_seller, &customer("US", None)).unwrap(),
            VatTreatment::OutsideEu
        );
    }

    #[test]
    fn treatment_is_reverse_charge_only_for_eu_cross_border_with_a_number() {
        assert_eq!(
            treatment_for(&seller(), &customer("DE", Some("DE123456789"))).unwrap(),
            VatTreatment::ReverseCharge
        );
        // Without a VAT number the documented fallback charges domestic VAT.
        assert_eq!(
            treatment_for(&seller(), &customer("DE", None)).unwrap(),
            VatTreatment::Domestic
        );
        // A non-EU seller cannot state EU reverse charge.
        let mut non_eu_seller = seller();
        non_eu_seller.country = "US".into();
        assert_eq!(
            treatment_for(&non_eu_seller, &customer("DE", Some("DE123456789"))).unwrap(),
            VatTreatment::Domestic
        );
    }

    #[test]
    fn treatment_has_no_vat_outside_the_eu() {
        assert_eq!(
            treatment_for(&seller(), &customer("US", None)).unwrap(),
            VatTreatment::OutsideEu
        );
        assert_eq!(
            treatment_for(&seller(), &customer("GB", Some("GB123456789"))).unwrap(),
            VatTreatment::OutsideEu
        );
        assert_eq!(
            treatment_for(&seller(), &customer("CH", None)).unwrap(),
            VatTreatment::OutsideEu
        );
    }

    #[test]
    fn treatment_refuses_broken_profiles() {
        let mut broken_seller = seller();
        broken_seller.vat_number = "x".into();
        assert!(matches!(
            treatment_for(&broken_seller, &customer("EE", None)),
            Err(BillingError::InvalidVatNumber {
                party: "seller",
                ..
            })
        ));
        // An invalid customer VAT number is never silently ignored.
        assert!(matches!(
            treatment_for(&seller(), &customer("DE", Some("DE-123"))),
            Err(BillingError::InvalidVatNumber {
                party: "customer",
                ..
            })
        ));
        // Outside the EU, a malformed number is still refused when present.
        assert!(matches!(
            treatment_for(&seller(), &customer("US", Some("!!"))),
            Err(BillingError::InvalidVatNumber {
                party: "customer",
                ..
            })
        ));
        let mut empty_vat = customer("DE", Some("DE123456789"));
        empty_vat.vat_number = Some("  ".into());
        assert_eq!(
            treatment_for(&seller(), &empty_vat),
            Err(BillingError::MissingProfileField {
                party: "customer",
                field: "vatNumber"
            })
        );
    }

    #[test]
    fn treatments_render_and_default_to_not_applied() {
        assert_eq!(VatTreatment::default(), VatTreatment::NotApplied);
        assert_eq!(VatTreatment::NotApplied.to_string(), "not applied");
        assert_eq!(VatTreatment::Domestic.to_string(), "domestic");
        assert_eq!(VatTreatment::ReverseCharge.to_string(), "reverse charge");
        assert_eq!(VatTreatment::OutsideEu.to_string(), "outside the EU");
        // The register serialization is camelCase.
        assert_eq!(
            serde_json::to_string(&VatTreatment::ReverseCharge).unwrap(),
            "\"reverseCharge\""
        );
        assert_eq!(
            serde_json::from_str::<VatTreatment>("\"outsideEu\"").unwrap(),
            VatTreatment::OutsideEu
        );
    }
}
