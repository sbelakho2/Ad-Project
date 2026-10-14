//! Candidate floor changes with their worst-case loss. MAX does not fill a
//! request when no ad clears the floor, so the share of revenue from
//! impressions priced below the new floor is the money a raise could lose.

use apex_core::Micros;

/// One candidate floor move.
#[derive(Debug, Clone, PartialEq)]
pub struct FloorCandidate {
    /// Country group the change applies to.
    pub group: String,
    /// The floor in force now.
    pub current_ecpm: Micros,
    /// The proposed floor.
    pub proposed_ecpm: Micros,
    /// The move in percent: -20, -10, +10, +20 or a percentile candidate.
    pub move_pct: f64,
    /// Share of the group's revenue from impressions priced below the new
    /// floor, when winning prices are known.
    pub worst_case_loss: Option<f64>,
}

/// Generates the spec's candidates: the current floor moved ±10% and ±20%,
/// plus the 5th, 10th and 20th percentiles of winning prices when impression
/// postbacks are available (percentiles need them).
pub fn floor_candidates(
    group: &str,
    current_ecpm: Micros,
    winning_prices: Option<&[Micros]>,
) -> Vec<FloorCandidate> {
    let mut out = Vec::new();
    for move_pct in [-20.0f64, -10.0, 10.0, 20.0] {
        let proposed = ((current_ecpm as f64) * (1.0 + move_pct / 100.0)).round() as Micros;
        if proposed <= 0 {
            continue;
        }
        out.push(FloorCandidate {
            group: group.to_string(),
            current_ecpm,
            proposed_ecpm: proposed,
            move_pct,
            worst_case_loss: winning_prices.map(|prices| worst_case_loss(proposed, prices)),
        });
    }
    if let Some(prices) = winning_prices {
        for percentile in [5.0f64, 10.0, 20.0] {
            if let Some(proposed) = percentile_of(prices, percentile) {
                if proposed > 0 {
                    out.push(FloorCandidate {
                        group: group.to_string(),
                        current_ecpm,
                        proposed_ecpm: proposed,
                        move_pct: ((proposed as f64 / current_ecpm as f64) - 1.0) * 100.0,
                        worst_case_loss: Some(worst_case_loss(proposed, prices)),
                    });
                }
            }
        }
    }
    out
}

/// `WorstLoss(r) = sum(p_i < r) / sum(p_i)`: the revenue share from
/// impressions priced below the proposed floor.
pub fn worst_case_loss(proposed_ecpm: Micros, winning_prices: &[Micros]) -> f64 {
    let total: i64 = winning_prices.iter().copied().sum();
    if total <= 0 {
        return 0.0;
    }
    let below: i64 = winning_prices
        .iter()
        .copied()
        .filter(|p| *p < proposed_ecpm)
        .sum();
    below as f64 / total as f64
}

/// The spec's split rules: candidates whose worst case exceeds 5% are
/// dropped; those under 1% may use 50/50, everything else uses 25%. Percentile
/// candidates without postback data cannot compute a worst case and always
/// use 25%.
pub fn candidate_split(candidate: &FloorCandidate) -> Option<f64> {
    match candidate.worst_case_loss {
        Some(loss) if loss > 0.05 => None,
        Some(loss) if loss < 0.01 => Some(0.5),
        Some(_) => Some(0.25),
        None => Some(0.25),
    }
}

/// Nearest-rank percentile: the smallest value such that at least
/// `percentile`% of the sample is at or below it.
fn percentile_of(values: &[Micros], percentile: f64) -> Option<Micros> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<Micros> = values.to_vec();
    sorted.sort_unstable();
    let rank = ((percentile / 100.0) * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted.get(index).copied()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn moves_are_ten_and_twenty_percent_both_ways() {
        let generated = floor_candidates("t1", 1_000_000, None);
        let moves: Vec<f64> = generated.iter().map(|c| c.move_pct).collect();
        assert_eq!(moves, vec![-20.0, -10.0, 10.0, 20.0]);
        assert!(generated.iter().all(|c| c.worst_case_loss.is_none()));
        assert_eq!(generated[2].proposed_ecpm, 1_100_000);
    }

    #[test]
    fn worst_case_loss_is_the_below_floor_revenue_share() {
        // Prices: 100, 200, 300, 400; floor 350 -> below = 100+200+300=600 of
        // 1000 -> 60%.
        let prices = [100, 200, 300, 400];
        assert!((worst_case_loss(350, &prices) - 0.6).abs() < 1e-9);
        assert_eq!(worst_case_loss(50, &prices), 0.0);
        assert!((worst_case_loss(1000, &prices) - 1.0).abs() < 1e-9);
        assert_eq!(worst_case_loss(350, &[]), 0.0);
    }

    #[test]
    fn percentiles_need_postbacks_and_are_ranked_right() {
        let without = floor_candidates("t1", 1_000_000, None);
        assert_eq!(
            without.len(),
            4,
            "no percentile candidates without postbacks"
        );
        let prices: Vec<Micros> = (1..=100).map(|i| i * 10_000).collect();
        let with = floor_candidates("t1", 1_000_000, Some(&prices));
        assert_eq!(with.len(), 7);
        let percentiles: Vec<Micros> = with[4..].iter().map(|c| c.proposed_ecpm).collect();
        // Nearest rank over 100 values: ceil(5) -> index 4 -> 50_000,
        // ceil(10) -> index 9 -> 100_000, ceil(20) -> index 19 -> 200_000.
        assert_eq!(percentiles, vec![50_000, 100_000, 200_000]);
    }

    #[test]
    fn split_rules_drop_the_risky_and_favour_the_cheap() {
        let cheap = FloorCandidate {
            group: "t1".into(),
            current_ecpm: 1_000_000,
            proposed_ecpm: 1_050_000,
            move_pct: 5.0,
            worst_case_loss: Some(0.005),
        };
        assert_eq!(candidate_split(&cheap), Some(0.5));
        let mid = FloorCandidate {
            worst_case_loss: Some(0.03),
            ..cheap.clone()
        };
        assert_eq!(candidate_split(&mid), Some(0.25));
        let risky = FloorCandidate {
            worst_case_loss: Some(0.06),
            ..cheap.clone()
        };
        assert_eq!(candidate_split(&risky), None);
        let unknown = FloorCandidate {
            worst_case_loss: None,
            ..cheap
        };
        assert_eq!(candidate_split(&unknown), Some(0.25));
    }
}
