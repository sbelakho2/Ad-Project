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

/// The ad-load levers A2 generates: caps and pacing/delay steps. The cap
/// levers map directly onto the MAX frequency-capping action; the delay lever
/// is carried for the candidate record and the dashboard-guided LevelPlay
/// path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdLoadLever {
    SessionCap,
    DayCap,
    PacingMinutes,
    FirstAdDelaySeconds,
}

impl AdLoadLever {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionCap => "session_cap",
            Self::DayCap => "day_cap",
            Self::PacingMinutes => "pacing_minutes",
            Self::FirstAdDelaySeconds => "first_ad_delay_seconds",
        }
    }
}

/// One ad-load candidate: a lever, its current value and a step.
#[derive(Debug, Clone, PartialEq)]
pub struct AdLoadCandidate {
    pub unit_id: String,
    pub country_tier: String,
    pub lever: AdLoadLever,
    pub current: u32,
    /// Proposed value; always at least one.
    pub proposed: u32,
    /// Signed move in percent.
    pub move_pct: f64,
}

/// The A2 ad-load candidates: ±10% and ±20% steps for session/day caps,
/// pacing minutes and first-ad delay seconds. Values never go below one
/// (zero would mean "no cap"), and a baseline of zero yields no candidate for
/// that lever because a move from zero is not a percentage step.
pub fn ad_load_candidates(
    unit_id: &str,
    country_tier: &str,
    current_session_cap: u32,
    current_day_cap: u32,
    current_pacing_minutes: u32,
    current_first_ad_delay_seconds: u32,
) -> Vec<AdLoadCandidate> {
    let mut out = Vec::new();
    for (lever, current) in [
        (AdLoadLever::SessionCap, current_session_cap),
        (AdLoadLever::DayCap, current_day_cap),
        (AdLoadLever::PacingMinutes, current_pacing_minutes),
        (
            AdLoadLever::FirstAdDelaySeconds,
            current_first_ad_delay_seconds,
        ),
    ] {
        if current == 0 {
            continue;
        }
        let mut seen = std::collections::BTreeSet::new();
        for move_pct in [-20.0f64, -10.0, 10.0, 20.0] {
            let proposed = ((current as f64) * (1.0 + move_pct / 100.0)).round() as u32;
            let proposed = proposed.max(1);
            if proposed == current || !seen.insert(proposed) {
                continue;
            }
            out.push(AdLoadCandidate {
                unit_id: unit_id.to_string(),
                country_tier: country_tier.to_string(),
                lever,
                current,
                proposed,
                move_pct: ((proposed as f64 / current as f64) - 1.0) * 100.0,
            });
        }
    }
    out
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

    #[test]
    fn floors_that_round_to_zero_and_zero_percentiles_are_dropped() {
        // Every +/- move of a zero floor rounds to zero and is dropped.
        assert!(floor_candidates("t1", 0, None).is_empty());
        // An empty winning-price list yields no percentile candidates.
        assert_eq!(floor_candidates("t1", 1_000_000, Some(&[])).len(), 4);
        // A list whose 5th percentile is zero produces no candidate for it
        // while the 10th and 20th percentiles still appear.
        let mut prices: Vec<Micros> = vec![0; 5];
        prices.extend((1..=95).map(|i| i * 10_000));
        let generated = floor_candidates("t1", 1_000_000, Some(&prices));
        assert_eq!(generated.len(), 6);
        assert!(generated[4..].iter().all(|c| c.proposed_ecpm > 0));
    }
    #[test]
    fn ad_load_candidates_step_caps_and_pacing_without_zero() {
        let candidates = ad_load_candidates("u1", "T1", 5, 20, 60, 0);
        assert!(!candidates.is_empty());
        assert!(candidates.iter().all(|candidate| candidate.proposed >= 1));
        let session: Vec<u32> = candidates
            .iter()
            .filter(|candidate| candidate.lever == AdLoadLever::SessionCap)
            .map(|candidate| candidate.proposed)
            .collect();
        assert_eq!(session, vec![4, 6]);
        // A zero baseline for first-ad delay yields no candidates; the other
        // levers still do.
        assert!(candidates
            .iter()
            .all(|candidate| candidate.lever != AdLoadLever::FirstAdDelaySeconds));
        let delay = ad_load_candidates("u1", "T1", 0, 0, 0, 10);
        assert_eq!(delay.len(), 4, "a positive delay gets all four steps");
    }

    #[test]
    fn ad_load_candidates_cover_every_positive_lever() {
        let candidates = ad_load_candidates("u1", "T1", 10, 30, 40, 8);
        for lever in [
            AdLoadLever::SessionCap,
            AdLoadLever::DayCap,
            AdLoadLever::PacingMinutes,
            AdLoadLever::FirstAdDelaySeconds,
        ] {
            assert!(
                candidates.iter().any(|candidate| candidate.lever == lever),
                "missing {lever:?}"
            );
        }
        assert!(candidates
            .iter()
            .all(|candidate| candidate.unit_id == "u1" && candidate.country_tier == "T1"));
    }
}
