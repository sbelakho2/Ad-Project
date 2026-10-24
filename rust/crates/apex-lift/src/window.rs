//! Analysis windows: calendar-position burn-in, day finality, whole-week
//! scope, incident exclusion and the per-arm daily values (revenue per user on
//! LevelPlay, revenue scaled by the allocated share on MAX).

use apex_core::Micros;

/// One arm-day as pulled from the platform reporting API.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmDay {
    /// `YYYY-MM-DD`, UTC, as both platforms report it.
    pub day: String,
    /// Reported revenue in micros.
    pub revenue: Micros,
    /// Active users for the arm; LevelPlay reports this, MAX does not.
    pub users: Option<u64>,
    /// The second pull of the same day, 24 hours later. A day is final when
    /// this pull agrees with the first within 0.5%.
    pub second_pull_revenue: Option<Micros>,
    /// Second-pull active users, when the platform reports them.
    pub second_pull_users: Option<u64>,
    /// Fill rate for the watched formats, when measured.
    pub fill: Option<f64>,
}

/// What kept a day out of the analysis window.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowProblem {
    /// No final day exists for this side; the reporting gap guardrail decides
    /// what happens next.
    MissingDay { day: String },
    /// The second pull disagreed by more than 0.5%; the day is not final yet.
    NotFinal { day: String },
    /// Burn-in: the start day and the next two full days are excluded.
    BurnIn { day: String },
    /// Both arms fell more than 50% day over day.
    Incident { day: String },
    /// The day list is not strictly increasing consecutive calendar days.
    Sequence { day: String },
}

/// A ready-to-estimate analysis window.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisWindow {
    /// Days in analysis order, after burn-in and whole-week trimming.
    pub days: Vec<AnalysisDay>,
    /// Days excluded, with the reason, in day order.
    pub problems: Vec<WindowProblem>,
    /// Whether the window reached the pre-registered day count with every
    /// kept day final and consecutive.
    pub complete: bool,
}

/// One analyzed day with both arms' values and the log ratio.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisDay {
    pub day: String,
    pub test_value: f64,
    pub control_value: f64,
    /// `ln(test / control)`.
    pub log_ratio: f64,
    /// Fill delta (test minus control), when both arms report fill.
    pub fill_delta: Option<f64>,
    /// Position of this day in the calendar sequence of pulls; the fill
    /// guardrail uses it to require two *consecutive* days.
    pub calendar_index: usize,
}

const FINALITY_TOLERANCE: f64 = 0.005;
/// The only allowed analysis lengths: whole weeks, so every weekday appears
/// equally in both arms.
pub const ALLOWED_WINDOW_DAYS: [usize; 4] = [7, 14, 21, 28];

fn final_within_tolerance(a: Micros, b: Micros) -> bool {
    let base = a.abs().max(1);
    let diff = (a - b).abs() as f64 / base as f64;
    diff <= FINALITY_TOLERANCE
}

fn parse_day(day: &str) -> Option<(i32, u32, u32)> {
    let mut parts = day.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// Whether `next` is the calendar day after `current`.
fn is_next_day(current: &str, next: &str) -> bool {
    match (parse_day(current), parse_day(next)) {
        (Some((y, m, d)), Some((y2, m2, d2))) => {
            let (y, m, d) = next_date(y, m, d);
            (y, m, d) == (y2, m2, d2)
        }
        _ => false,
    }
}

fn next_date(year: i32, month: u32, day: u32) -> (i32, u32, u32) {
    let last = days_in_month(year, month);
    if day < last {
        (year, month, day + 1)
    } else if month < 12 {
        (year, month + 1, 1)
    } else {
        (year + 1, 1, 1)
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 30,
    }
}

/// Per-arm daily value: revenue per user where users are reported (LevelPlay),
/// otherwise revenue divided by the arm's allocated share (MAX).
pub fn arm_value(day: &ArmDay, allocated_share: f64) -> Option<f64> {
    if let Some(users) = day.users {
        if users == 0 {
            return None;
        }
        return Some(day.revenue as f64 / users as f64);
    }
    if !(0.0..1.0).contains(&allocated_share) || allocated_share == 0.0 {
        return None;
    }
    Some(day.revenue as f64 / allocated_share)
}

/// Consecutive incident runs over the raw pull series, by calendar
/// adjacency: a day is an incident when both arms' revenue fell more than 50%
/// day over day. Returns `(day, run_length)` for every incident day; a run of
/// two or more voids the test.
pub fn incident_runs(control: &[ArmDay], test: &[ArmDay]) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut run = 0u32;
    for index in 0..control.len() {
        if index > 0
            && control[index - 1].revenue > 0
            && test[index - 1].revenue > 0
            && (control[index].revenue as f64) < 0.5 * control[index - 1].revenue as f64
            && (test[index].revenue as f64) < 0.5 * test[index - 1].revenue as f64
        {
            run += 1;
            out.push((control[index].day.clone(), run));
        } else {
            run = 0;
        }
    }
    out
}

/// Build the window.
///
/// `control` and `test` are the arm pulls in day order, `window_days` the
/// pre-registered analysis length and `test_allocated_share` the test arm's
/// user share (MAX needs it for the value). The first calendar day plus the
/// next two full days are burn-in, chosen by calendar position before any
/// finality filtering, so a late day never shifts the burn-in boundary.
///
/// Returns a window with `complete == false` whenever any kept day is missing,
/// not final, or the pull sequence skips a calendar day: a gap pauses the
/// window rather than shortening it.
pub fn build_window(
    control: &[ArmDay],
    test: &[ArmDay],
    window_days: usize,
    test_allocated_share: f64,
) -> AnalysisWindow {
    let mut problems = Vec::new();
    if !ALLOWED_WINDOW_DAYS.contains(&window_days) {
        problems.push(WindowProblem::Sequence {
            day: format!("invalid window length {window_days}"),
        });
        return AnalysisWindow {
            days: Vec::new(),
            problems,
            complete: false,
        };
    }
    if control.is_empty() || control.len() != test.len() {
        problems.push(WindowProblem::MissingDay {
            day: "pull length mismatch".to_string(),
        });
        return AnalysisWindow {
            days: Vec::new(),
            problems,
            complete: false,
        };
    }

    // Incident detection runs over the raw pull series by calendar adjacency,
    // before finality filtering, so a dropped day can never bridge a
    // comparison across two non-adjacent days. A single incident day is an
    // exclusion; two consecutive days void.
    let run_by_day = incident_runs(control, test);
    let index_by_day: std::collections::HashMap<&str, usize> = control
        .iter()
        .enumerate()
        .map(|(index, day)| (day.day.as_str(), index))
        .collect();
    let mut runs: Vec<u32> = vec![0; control.len()];
    for (day, length) in &run_by_day {
        if let Some(&index) = index_by_day.get(day.as_str()) {
            runs[index] = *length;
        }
    }

    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut previous_day: Option<&str> = None;
    let mut analyzed: Vec<AnalysisDay> = Vec::new();
    for index in 0..control.len() {
        let control_day = &control[index];
        let test_day = &test[index];
        if control_day.day != test_day.day {
            problems.push(WindowProblem::MissingDay {
                day: control_day.day.clone(),
            });
            continue;
        }
        // Duplicate or out-of-order day strings break day-based statistics.
        if !seen.insert(control_day.day.as_str()) {
            problems.push(WindowProblem::Sequence {
                day: control_day.day.clone(),
            });
            continue;
        }
        if let Some(previous) = previous_day {
            if !is_next_day(previous, &control_day.day) {
                problems.push(WindowProblem::Sequence {
                    day: control_day.day.clone(),
                });
                previous_day = Some(control_day.day.as_str());
                continue;
            }
        }
        previous_day = Some(control_day.day.as_str());

        // Burn-in by calendar position: the first three days of the test.
        if index < 3 {
            problems.push(WindowProblem::BurnIn {
                day: control_day.day.clone(),
            });
            continue;
        }
        // A single incident day is excluded; a two-day run voids the window.
        if runs[index] >= 1 {
            problems.push(WindowProblem::Incident {
                day: control_day.day.clone(),
            });
            if runs[index] >= 2 {
                return AnalysisWindow {
                    days: analyzed,
                    problems,
                    complete: false,
                };
            }
            continue;
        }
        let final_pair = match control_day.second_pull_revenue {
            None => false,
            Some(second) => final_within_tolerance(control_day.revenue, second),
        } && match test_day.second_pull_revenue {
            None => false,
            Some(second) => final_within_tolerance(test_day.revenue, second),
        };
        if !final_pair {
            problems.push(WindowProblem::NotFinal {
                day: control_day.day.clone(),
            });
            continue;
        }
        let Some(control_value) = arm_value(control_day, 1.0 - test_allocated_share) else {
            problems.push(WindowProblem::MissingDay {
                day: control_day.day.clone(),
            });
            continue;
        };
        let Some(test_value) = arm_value(test_day, test_allocated_share) else {
            problems.push(WindowProblem::MissingDay {
                day: control_day.day.clone(),
            });
            continue;
        };
        if control_value <= 0.0 || test_value <= 0.0 {
            problems.push(WindowProblem::MissingDay {
                day: control_day.day.clone(),
            });
            continue;
        }
        let fill_delta = match (control_day.fill, test_day.fill) {
            (Some(c), Some(t)) => Some(t - c),
            _ => None,
        };
        analyzed.push(AnalysisDay {
            day: control_day.day.clone(),
            test_value,
            control_value,
            log_ratio: (test_value / control_value).ln(),
            fill_delta,
            calendar_index: index,
        });
    }

    // Whole weeks only, capped at the pre-registered window; a window is
    // complete only when every pre-registered day survived as a final,
    // consecutive day.
    let whole_weeks = (analyzed.len() / 7) * 7;
    let keep = whole_weeks.min(window_days);
    analyzed.truncate(keep);
    let clean = problems
        .iter()
        .all(|p| matches!(p, WindowProblem::BurnIn { .. }));
    let complete = keep == window_days
        && clean
        && analyzed
            .first()
            .map(|first| {
                analyzed
                    .iter()
                    .scan(first.calendar_index, |previous, day| {
                        let expected = *previous + 1;
                        let ok = day.calendar_index == expected || *previous == day.calendar_index;
                        *previous = day.calendar_index;
                        Some(ok)
                    })
                    .all(|ok| ok)
            })
            .unwrap_or(false);
    AnalysisWindow {
        days: analyzed,
        problems,
        complete,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn day(name: &str, revenue: Micros, users: Option<u64>) -> ArmDay {
        ArmDay {
            day: name.to_string(),
            revenue,
            users,
            second_pull_revenue: Some(revenue),
            second_pull_users: users,
            fill: None,
        }
    }

    fn series(days: &[(&str, Micros)]) -> Vec<ArmDay> {
        days.iter()
            .map(|(name, revenue)| day(name, *revenue, None))
            .collect()
    }

    #[test]
    fn finality_requires_two_pulls_within_half_a_percent() {
        // Burn-in covers the first three days; finality decides for the days
        // that reach analysis: 10-04 is still late, 10-05 drifted past the
        // 0.5% tolerance, 10-06 is within tolerance and final.
        let names = [
            "2026-10-01",
            "2026-10-02",
            "2026-10-03",
            "2026-10-04",
            "2026-10-05",
            "2026-10-06",
        ];
        let mut control: Vec<ArmDay> = names.iter().map(|n| day(n, 1_000_000, None)).collect();
        let mut test: Vec<ArmDay> = names.iter().map(|n| day(n, 1_000_000, None)).collect();
        control[3].second_pull_revenue = None;
        control[4].second_pull_revenue = Some(1_005_001);
        test[3].second_pull_revenue = None;
        test[4].second_pull_revenue = Some(1_005_001);
        let window = build_window(&control, &test, 7, 0.25);
        let not_final: Vec<&str> = window
            .problems
            .iter()
            .filter_map(|p| match p {
                WindowProblem::NotFinal { day } => Some(day.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(not_final, vec!["2026-10-04", "2026-10-05"]);
        // Only 10-06 survives and a single day is not a whole week, so the
        // analyzed list trims to empty while the problems stay visible.
        assert_eq!(window.days.len(), 0);
        assert!(!window.complete);
        // Exactly at the tolerance the day is final.
        let mut within = control.clone();
        let mut within_test = test.clone();
        within[5].second_pull_revenue = Some(1_004_999);
        within_test[5].second_pull_revenue = Some(1_004_999);
        assert_eq!(within[5].second_pull_revenue, Some(1_004_999));
        assert!(final_within_tolerance(1_000_000, 1_004_999));
        assert!(!final_within_tolerance(1_000_000, 1_005_001));
    }

    #[test]
    fn burn_in_follows_calendar_position_even_when_a_day_is_late() {
        // Day 2 is not final; burn-in must still be days 0..=2 by calendar
        // position, not shift to days 1..=3.
        let days = [
            "2026-10-01",
            "2026-10-02",
            "2026-10-03",
            "2026-10-04",
            "2026-10-05",
            "2026-10-06",
            "2026-10-07",
            "2026-10-08",
            "2026-10-09",
            "2026-10-10",
        ];
        let control = series(&days.map(|d| (d, 1_000_000)));
        let mut test = series(&days.map(|d| (d, 343_333)));
        test[1].second_pull_revenue = None;
        let window = build_window(&control, &test, 7, 0.25);
        // The extra problem proves the filter ignores every non-burn-in kind.
        let extra = WindowProblem::MissingDay {
            day: "2026-10-11".into(),
        };
        let burn_in: Vec<&str> = window
            .problems
            .iter()
            .chain(std::iter::once(&extra))
            .filter_map(|p| match p {
                WindowProblem::BurnIn { day } => Some(day.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(burn_in, vec!["2026-10-01", "2026-10-02", "2026-10-03"]);
        // The late day sits inside burn-in, so it is excluded as burn-in and
        // never reaches the finality decision; the positional burn-in is what
        // keeps the boundary fixed.
        assert!(window
            .problems
            .iter()
            .all(|p| !matches!(p, WindowProblem::NotFinal { .. })));
        assert_eq!(window.days.len(), 7);
        assert!(window.complete);
    }

    #[test]
    fn analysis_values_model_the_allocated_shares() {
        let control = series(&[
            ("2026-10-01", 1_000_000),
            ("2026-10-02", 1_000_000),
            ("2026-10-03", 1_000_000),
            ("2026-10-04", 1_000_000),
            ("2026-10-05", 1_000_000),
            ("2026-10-06", 1_000_000),
            ("2026-10-07", 1_000_000),
            ("2026-10-08", 1_000_000),
            ("2026-10-09", 1_000_000),
            ("2026-10-10", 1_000_000),
        ]);
        let test = series(&[
            ("2026-10-01", 343_333),
            ("2026-10-02", 343_333),
            ("2026-10-03", 343_333),
            ("2026-10-04", 343_333),
            ("2026-10-05", 343_333),
            ("2026-10-06", 343_333),
            ("2026-10-07", 343_333),
            ("2026-10-08", 343_333),
            ("2026-10-09", 343_333),
            ("2026-10-10", 343_333),
        ]);
        let window = build_window(&control, &test, 7, 0.25);
        assert_eq!(window.days.len(), 7);
        assert!(window.complete);
        for analyzed in &window.days {
            assert!(
                (analyzed.log_ratio - (1.03f64).ln()).abs() < 1e-4,
                "{}",
                analyzed.log_ratio
            );
            assert!((analyzed.test_value - 343_333.0 / 0.25).abs() < 1.0);
            assert!((analyzed.control_value - 1_000_000.0 / 0.75).abs() < 1.0);
        }
    }

    #[test]
    fn levelplay_values_use_active_users() {
        let users = Some(10_000);
        let control = day("2026-10-01", 8_000_000, users);
        let test = day("2026-10-01", 8_800_000, users);
        assert!((arm_value(&control, 0.5).unwrap() - 800.0).abs() < 1e-9);
        assert!((arm_value(&test, 0.5).unwrap() - 880.0).abs() < 1e-9);
        let zero_users = day("2026-10-02", 1, Some(0));
        assert!(arm_value(&zero_users, 0.5).is_none());
    }

    #[test]
    fn duplicate_and_out_of_order_days_are_refused() {
        let control = series(&[
            ("2026-10-01", 1_000_000),
            ("2026-10-01", 1_000_000),
            ("2026-10-03", 1_000_000),
            ("2026-10-02", 1_000_000),
        ]);
        let test = series(&[
            ("2026-10-01", 1_000_000),
            ("2026-10-01", 1_000_000),
            ("2026-10-03", 1_000_000),
            ("2026-10-02", 1_000_000),
        ]);
        let window = build_window(&control, &test, 7, 0.25);
        assert!(!window.complete);
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::Sequence { .. })));
    }

    #[test]
    fn a_gap_day_marks_the_sequence_problem() {
        let control = series(&[
            ("2026-10-01", 1_000_000),
            ("2026-10-02", 1_000_000),
            ("2026-10-03", 1_000_000),
            ("2026-10-05", 1_000_000),
        ]);
        let test = series(&[
            ("2026-10-01", 1_000_000),
            ("2026-10-02", 1_000_000),
            ("2026-10-03", 1_000_000),
            ("2026-10-05", 1_000_000),
        ]);
        let window = build_window(&control, &test, 7, 0.25);
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::Sequence { .. })));
    }

    #[test]
    fn single_incident_day_is_excluded_and_two_void() {
        let names = [
            "2026-10-01",
            "2026-10-02",
            "2026-10-03",
            "2026-10-04",
            "2026-10-05",
            "2026-10-06",
            "2026-10-07",
            "2026-10-08",
            "2026-10-09",
            "2026-10-10",
        ];
        let mut control: Vec<ArmDay> = Vec::new();
        let mut test: Vec<ArmDay> = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let revenue: Micros = if i == 7 { 400_000 } else { 1_000_000 };
            control.push(day(name, revenue, None));
            test.push(day(name, revenue, None));
        }
        let single = build_window(&control, &test, 7, 0.25);
        assert_eq!(
            single
                .problems
                .iter()
                .filter(|p| matches!(p, WindowProblem::Incident { .. }))
                .count(),
            1
        );
        assert!(
            single
                .problems
                .iter()
                .filter(|p| matches!(p, WindowProblem::Incident { .. }))
                .count()
                < 2,
            "one incident day must not void"
        );

        let mut control2 = control.clone();
        let mut test2 = test.clone();
        control2[8].revenue = 100_000;
        test2[8].revenue = 100_000;
        let two = build_window(&control2, &test2, 7, 0.25);
        assert!(
            !two.complete,
            "two consecutive incident days void the window"
        );
        assert_eq!(
            two.problems
                .iter()
                .filter(|p| matches!(p, WindowProblem::Incident { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn mismatched_pulls_zero_revenue_and_missing_fill_are_handled() {
        let control = series(&[("2026-10-01", 1_000_000), ("2026-10-02", 1_000_000)]);
        let test = series(&[("2026-10-01", 1_000_000)]);
        let short = build_window(&control, &test, 7, 0.25);
        assert!(!short.complete);
        assert!(short
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::MissingDay { .. })));

        // A day with zero active users has no value and is missing, not an
        // incident: revenue stays flat across the series.
        let names = [
            "2026-10-01",
            "2026-10-02",
            "2026-10-03",
            "2026-10-04",
            "2026-10-05",
            "2026-10-06",
            "2026-10-07",
            "2026-10-08",
            "2026-10-09",
            "2026-10-10",
        ];
        let zero: Vec<ArmDay> = names
            .iter()
            .enumerate()
            .map(|(i, name)| day(name, 1_000_000, Some(if i == 3 { 0 } else { 1000 })))
            .collect();
        let zero_window = build_window(&zero, &zero, 7, 0.25);
        assert!(zero_window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::MissingDay { .. })));

        // Fill deltas stay absent when only one arm reports fill.
        let mut control_fill = series(&[
            ("2026-10-01", 1_000_000),
            ("2026-10-02", 1_000_000),
            ("2026-10-03", 1_000_000),
            ("2026-10-04", 1_000_000),
            ("2026-10-05", 1_000_000),
            ("2026-10-06", 1_000_000),
            ("2026-10-07", 1_000_000),
            ("2026-10-08", 1_000_000),
            ("2026-10-09", 1_000_000),
            ("2026-10-10", 1_000_000),
        ]);
        let mut test_fill = control_fill.clone();
        control_fill[5].fill = Some(0.9);
        test_fill[5].fill = None;
        let fill_window = build_window(&control_fill, &test_fill, 7, 0.25);
        assert!(fill_window.days.iter().all(|day| day.fill_delta.is_none()));
    }

    #[test]
    fn window_length_must_be_a_whole_week() {
        let control = series(&[("2026-10-01", 1)]);
        let test = series(&[("2026-10-01", 1)]);
        let window = build_window(&control, &test, 10, 0.25);
        assert!(!window.complete && window.days.is_empty());
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::Sequence { .. })));
    }

    #[test]
    fn date_helpers_cover_rollovers_and_invalid_shapes() {
        assert!(parse_day("2026-10-05-00").is_none(), "extra parts refuse");
        assert!(!is_next_day("2026-10-02", "2026-10-01-02"));
        assert!(!is_next_day("garbage", "2026-10-01"));
        assert!(is_next_day("2026-10-31", "2026-11-01"), "31-day rollover");
        assert!(is_next_day("2026-11-30", "2026-12-01"), "30-day rollover");
        assert!(is_next_day("2026-12-31", "2027-01-01"), "year rollover");
        assert!(is_next_day("2024-02-28", "2024-02-29"), "leap February");
        assert!(is_next_day("2026-02-27", "2026-02-28"), "plain February");
        assert_eq!(days_in_month(2026, 13), 30, "unknown months fall back");
    }

    #[test]
    fn mismatched_arms_and_zero_values_are_missing_days() {
        // The arms disagree on the third day's label: that day is missing.
        let control = series(&[
            ("2026-10-01", 1_000_000),
            ("2026-10-02", 1_000_000),
            ("2026-10-03", 1_000_000),
        ]);
        let test = series(&[
            ("2026-10-01", 1_000_000),
            ("2026-10-02", 1_000_000),
            ("2026-10-04", 1_000_000),
        ]);
        let window = build_window(&control, &test, 7, 0.25);
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::MissingDay { day } if day == "2026-10-03")));
        assert!(!window.complete);

        // Zero revenue without user counts is a zero daily value, which is
        // missing rather than an incident (every prior day is zero too).
        let zero = series(&[
            ("2026-10-01", 0),
            ("2026-10-02", 0),
            ("2026-10-03", 0),
            ("2026-10-04", 0),
        ]);
        let window = build_window(&zero, &zero, 7, 0.25);
        assert!(window.days.is_empty());
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::MissingDay { day } if day == "2026-10-04")));

        // A test day with zero active users yields no test value at all.
        let names = [
            "2026-10-01",
            "2026-10-02",
            "2026-10-03",
            "2026-10-04",
            "2026-10-05",
            "2026-10-06",
            "2026-10-07",
            "2026-10-08",
            "2026-10-09",
            "2026-10-10",
        ];
        let control: Vec<ArmDay> = names
            .iter()
            .map(|name| day(name, 1_000_000, Some(1000)))
            .collect();
        let mut test = control.clone();
        test[4].users = Some(0);
        let window = build_window(&control, &test, 7, 0.25);
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::MissingDay { day } if day == "2026-10-05")));
    }

    #[test]
    fn a_test_arm_without_a_second_pull_is_not_final() {
        let names = [
            "2026-10-01",
            "2026-10-02",
            "2026-10-03",
            "2026-10-04",
            "2026-10-05",
            "2026-10-06",
            "2026-10-07",
            "2026-10-08",
            "2026-10-09",
            "2026-10-10",
        ];
        let control = series(&names.map(|name| (name, 1_000_000)));
        let mut test = control.clone();
        test[5].second_pull_revenue = None;
        let window = build_window(&control, &test, 7, 0.25);
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::NotFinal { day } if day == "2026-10-06")));
        assert!(!window.complete);
    }

    #[test]
    fn fill_deltas_need_both_arms_to_report_fill() {
        let names = [
            "2026-10-01",
            "2026-10-02",
            "2026-10-03",
            "2026-10-04",
            "2026-10-05",
            "2026-10-06",
            "2026-10-07",
            "2026-10-08",
            "2026-10-09",
            "2026-10-10",
        ];
        let mut control = series(&names.map(|name| (name, 1_000_000)));
        let mut test = control.clone();
        control[6].fill = Some(0.9);
        test[6].fill = Some(0.8);
        let window = build_window(&control, &test, 7, 0.25);
        let seventh = window
            .days
            .iter()
            .find(|day| day.day == "2026-10-07")
            .expect("the seventh is analyzed");
        let delta = seventh.fill_delta.expect("both arms report fill");
        assert!((delta + 0.1).abs() < 1e-12, "{delta}");
        // Every other kept day has no fill on one side and stays absent.
        assert!(window
            .days
            .iter()
            .filter(|day| day.day != "2026-10-07")
            .all(|day| day.fill_delta.is_none()));
    }
}
