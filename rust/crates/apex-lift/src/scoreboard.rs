//! The public scoreboard: three publisher-name-free numbers published
//! monthly. With consent and no names.

/// One month of community results.
#[derive(Debug, Clone, PartialEq)]
pub struct Scoreboard {
    pub month: String,
    pub tests_total: u64,
    pub tests_kept: u64,
    /// Billed lifts of the kept tests, for the median.
    pub billed_lifts: Vec<f64>,
    pub rechecks_total: u64,
    pub rechecks_confirmed: u64,
}

impl Scoreboard {
    /// The share of tests kept.
    pub fn kept_share(&self) -> f64 {
        if self.tests_total == 0 {
            0.0
        } else {
            self.tests_kept as f64 / self.tests_total as f64
        }
    }

    /// The median billed lift (0 with no kept tests).
    pub fn median_billed_lift(&self) -> f64 {
        if self.billed_lifts.is_empty() {
            return 0.0;
        }
        let mut sorted: Vec<f64> = self
            .billed_lifts
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .collect();
        if sorted.is_empty() {
            return 0.0;
        }
        sorted.sort_by(f64::total_cmp);
        let middle = sorted.len() / 2;
        if sorted.len().is_multiple_of(2) {
            (sorted[middle - 1] + sorted[middle]) / 2.0
        } else {
            sorted[middle]
        }
    }

    /// How often re-checks confirmed the original result.
    pub fn recheck_confirmation_rate(&self) -> f64 {
        if self.rechecks_total == 0 {
            0.0
        } else {
            self.rechecks_confirmed as f64 / self.rechecks_total as f64
        }
    }

    /// The published JSON: three numbers and the month, no names.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "month": self.month,
            "keptShare": self.kept_share(),
            "medianBilledLift": self.median_billed_lift(),
            "recheckConfirmationRate": self.recheck_confirmation_rate(),
        })
    }
}

impl Scoreboard {
    /// Build the monthly scoreboard from recorded outcomes: one entry per
    /// finished test `(kept, billed_lambda_when_kept)` plus one per finished
    /// re-check `(confirmed)`. No publisher identifiers pass through.
    pub fn from_outcomes(month: &str, tests: &[(bool, f64)], rechecks: &[bool]) -> Scoreboard {
        Scoreboard {
            month: month.to_string(),
            tests_total: tests.len() as u64,
            tests_kept: tests.iter().filter(|(kept, _)| *kept).count() as u64,
            billed_lifts: tests
                .iter()
                .filter(|(kept, _)| *kept)
                .map(|(_, lambda)| *lambda)
                .collect(),
            rechecks_total: rechecks.len() as u64,
            rechecks_confirmed: rechecks.iter().filter(|confirmed| **confirmed).count() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn outcomes_build_a_name_free_scoreboard() {
        let board = Scoreboard::from_outcomes(
            "2026-10",
            &[(true, 0.02), (false, 0.0), (true, 0.04)],
            &[true, false, true],
        );
        assert_eq!(board.tests_total, 3);
        assert_eq!(board.tests_kept, 2);
        assert_eq!(board.billed_lifts, vec![0.02, 0.04]);
        assert!((board.median_billed_lift() - 0.03).abs() < 1e-12);
        assert!((board.recheck_confirmation_rate() - 2.0 / 3.0).abs() < 1e-12);
        let json = board.to_json().to_string();
        assert!(!json.contains("publisher"), "{json}");
    }

    #[test]
    fn medians_handle_odd_even_and_empty() {
        let odd = Scoreboard {
            month: "2026-10".into(),
            tests_total: 3,
            tests_kept: 2,
            billed_lifts: vec![0.01, 0.03, 0.02],
            rechecks_total: 2,
            rechecks_confirmed: 1,
        };
        assert!((odd.median_billed_lift() - 0.02).abs() < 1e-12);
        assert!((odd.kept_share() - 2.0 / 3.0).abs() < 1e-12);
        assert!((odd.recheck_confirmation_rate() - 0.5).abs() < 1e-12);
        let even = Scoreboard {
            billed_lifts: vec![0.04, 0.02],
            ..odd.clone()
        };
        assert!((even.median_billed_lift() - 0.03).abs() < 1e-12);
        let empty = Scoreboard {
            month: "2026-09".into(),
            tests_total: 0,
            tests_kept: 0,
            billed_lifts: vec![],
            rechecks_total: 0,
            rechecks_confirmed: 0,
        };
        assert_eq!(empty.median_billed_lift(), 0.0);
        assert_eq!(empty.kept_share(), 0.0);
        let json = odd.to_json();
        assert_eq!(json["month"], "2026-10");
        assert!(json.get("publisher").is_none());
    }
}
