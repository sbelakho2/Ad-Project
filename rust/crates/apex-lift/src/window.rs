//! Analysis windows: calendar-position burn-in, day finality, whole-week
//! scope, incident exclusion and the per-arm daily values (revenue per user on
//! LevelPlay, revenue scaled by the allocated share on MAX).

use apex_core::Micros;

/// One reporting read of one arm-day: the raw measured values, when the read
/// happened and the hash of the raw source it came from.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullObservation {
    /// Reported revenue in micros.
    pub revenue: Micros,
    /// Active users for the arm; LevelPlay reports this, MAX does not.
    pub users: Option<u64>,
    /// Fill rate for the watched formats, when measured.
    pub fill: Option<f64>,
    /// When this reporting read happened (Unix seconds).
    pub observed_at_unix_s: i64,
    /// SHA-256 of the raw arm-day source this observation was folded from.
    pub source_sha256: String,
}

/// One arm-day carrying the two latest reporting reads. The durable pull
/// evidence keeps every read; once more than two reads exist, the pair here is
/// the two **latest** (earlier, latest), never the very first read, so a day
/// can become final after any two consecutive later reads agree.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmDay {
    /// `YYYY-MM-DD`, UTC, as both platforms report it.
    pub day: String,
    /// Reported revenue in micros (the earlier of the two latest reads).
    pub revenue: Micros,
    /// Active users for the arm; LevelPlay reports this, MAX does not.
    pub users: Option<u64>,
    /// The latest read of the same day. A day is final when this read is from
    /// a distinct read at least 24 hours after the earlier one and agrees on
    /// the metric numerator and denominator within 0.5%.
    pub second_pull_revenue: Option<Micros>,
    /// Latest-read active users, when the platform reports them.
    pub second_pull_users: Option<u64>,
    /// Fill rate for the watched formats (the earlier of the two latest reads).
    pub fill: Option<f64>,
    /// Fill from the latest read, when measured.
    pub second_pull_fill: Option<f64>,
    /// When the earlier of the two latest reporting reads observed this
    /// arm-day (Unix seconds).
    pub first_observed_at_unix_s: Option<i64>,
    /// When the latest reporting read observed it (Unix seconds).
    pub second_observed_at_unix_s: Option<i64>,
    /// SHA-256 of the earlier read's raw arm-day source.
    pub first_source_sha256: Option<String>,
    /// SHA-256 of the latest read's raw arm-day source.
    pub second_source_sha256: Option<String>,
}

/// The minimum gap between the two reporting reads that make a day final.
pub const SECOND_PULL_MIN_SECS: i64 = 86_400;

/// The canonical SHA-256 of one raw arm-day observation, used as its source
/// hash in the durable pull-evidence store and the signed DataPull record.
pub fn arm_day_source_sha256(
    day: &str,
    arm: &str,
    revenue: Micros,
    users: Option<u64>,
    fill: Option<f64>,
) -> String {
    crate::records::rows_sha256(&serde_json::json!({
        "day": day,
        "arm": arm,
        "revenue": revenue,
        "users": users,
        "fill": fill,
    }))
}

impl ArmDay {
    /// The first observation, when the full provenance is present.
    pub fn first_observation(&self) -> Option<PullObservation> {
        Some(PullObservation {
            revenue: self.revenue,
            users: self.users,
            fill: self.fill,
            observed_at_unix_s: self.first_observed_at_unix_s?,
            source_sha256: self.first_source_sha256.clone()?,
        })
    }

    /// The accepted second observation, when one exists.
    pub fn second_observation(&self) -> Option<PullObservation> {
        Some(PullObservation {
            revenue: self.second_pull_revenue?,
            users: self.second_pull_users,
            fill: self.second_pull_fill,
            observed_at_unix_s: self.second_observed_at_unix_s?,
            source_sha256: self.second_source_sha256.clone()?,
        })
    }
}

/// Whether the arm-day is final: both latest reads are present, they come
/// from distinct reads at least 24 hours apart, and the revenue (numerator),
/// users (denominator), revenue per user and fill (where the rule uses it)
/// agree within 0.5%. The pair is always the two latest reads (A3-V4 E6), so
/// the decision no longer rides on the first read, which never changes.
pub fn day_is_final(day: &ArmDay) -> bool {
    let (Some(first_at), Some(second_at)) =
        (day.first_observed_at_unix_s, day.second_observed_at_unix_s)
    else {
        return false;
    };
    let (Some(first_source), Some(second_source)) = (
        day.first_source_sha256.as_deref(),
        day.second_source_sha256.as_deref(),
    ) else {
        return false;
    };
    // The second pull must be a later read, not the first read relabelled.
    if second_at.saturating_sub(first_at) < SECOND_PULL_MIN_SECS {
        return false;
    }
    if first_at == second_at && first_source == second_source {
        return false;
    }
    let Some(second_revenue) = day.second_pull_revenue else {
        return false;
    };
    // Numerator: revenue.
    if !final_within_tolerance(day.revenue, second_revenue) {
        return false;
    }
    // Denominator: users, when the platform reports them.
    match (day.users, day.second_pull_users) {
        (None, None) => {}
        (Some(first), Some(second)) if units_within_tolerance(first, second) => {}
        _ => return false,
    }
    // The metric actually used: revenue per user. A zero-user day is final
    // evidence but has no value; `arm_value` marks it missing afterwards.
    match (day.users, day.second_pull_users) {
        (Some(first_users), Some(second_users)) if first_users > 0 && second_users > 0 => {
            let first_value = day.revenue as f64 / first_users as f64;
            let second_value = second_revenue as f64 / second_users as f64;
            if !relative_within_tolerance(first_value, second_value) {
                return false;
            }
        }
        (Some(0), Some(0)) => {}
        (None, None) => {}
        _ => return false,
    }
    // Fill, where the fill guardrails would read it.
    match (day.fill, day.second_pull_fill) {
        (None, None) => {}
        (Some(first), Some(second)) if (first - second).abs() <= FINALITY_TOLERANCE + 1e-12 => {}
        _ => return false,
    }
    true
}

/// What kept a day out of the analysis window.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowProblem {
    /// No final day exists for this side; the reporting gap guardrail decides
    /// what happens next. Temporarily incomplete: it can resolve.
    MissingDay { day: String },
    /// The second pull disagreed by more than 0.5%; the day is not final yet.
    /// Temporarily incomplete: it can resolve.
    NotFinal { day: String },
    /// Burn-in: the start day and the next two full days are excluded.
    BurnIn { day: String },
    /// Both arms fell more than 50% day over day. Excluded informationally.
    Incident { day: String },
    /// The day list is not strictly increasing consecutive calendar days.
    /// Fatal: waiting cannot repair a broken sequence.
    Sequence { day: String },
    /// The window shape itself is unusable (for example an unregistered
    /// length). Fatal: waiting cannot repair it.
    Schema { day: String },
}

impl WindowProblem {
    /// Whether the problem is fatal (waiting cannot make the window usable).
    pub fn is_fatal(&self) -> bool {
        matches!(self, Self::Sequence { .. } | Self::Schema { .. })
    }

    /// Whether the problem is a temporary incompleteness that later clean
    /// days can resolve.
    pub fn is_temporarily_incomplete(&self) -> bool {
        matches!(self, Self::MissingDay { .. } | Self::NotFinal { .. })
    }

    /// Whether the problem only records an exclusion and never blocks the
    /// window by itself.
    pub fn is_informational(&self) -> bool {
        matches!(self, Self::BurnIn { .. } | Self::Incident { .. })
    }
}

/// A ready-to-estimate analysis window.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisWindow {
    /// Days in analysis order, after burn-in and whole-week trimming.
    pub days: Vec<AnalysisDay>,
    /// Days excluded, with the reason, in day order.
    pub problems: Vec<WindowProblem>,
    /// Whether the window reached the pre-registered day count with enough
    /// accepted clean analysis days. Incident and burn-in exclusions are
    /// informational and do not by themselves keep this false.
    pub complete: bool,
    /// Whether a temporary problem (not-final or missing day) occurred before
    /// the window had accumulated enough clean days. It can resolve later.
    pub incomplete: bool,
    /// Whether a fatal problem (sequence or schema) makes the window
    /// unusable.
    pub fatal: bool,
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
    /// Users in the test arm on this analysis day, when the feed reports it.
    pub test_users: Option<u64>,
    /// Users in the control arm on this analysis day.
    pub control_users: Option<u64>,
}

const FINALITY_TOLERANCE: f64 = 0.005;
/// The only allowed analysis lengths: whole weeks, so every weekday appears
/// equally in both arms.
pub const ALLOWED_WINDOW_DAYS: [usize; 4] = [7, 14, 21, 28];

fn scaled_within_tolerance(a: i128, b: i128) -> bool {
    let base = a.abs().max(1);
    (a - b).abs() * 1_000 <= base * 5
}

fn final_within_tolerance(a: Micros, b: Micros) -> bool {
    scaled_within_tolerance(i128::from(a), i128::from(b))
}

fn units_within_tolerance(a: u64, b: u64) -> bool {
    scaled_within_tolerance(i128::from(a), i128::from(b))
}

fn relative_within_tolerance(a: f64, b: f64) -> bool {
    if !a.is_finite() || !b.is_finite() {
        return false;
    }
    let base = a.abs().max(1e-12);
    (a - b).abs() / base <= FINALITY_TOLERANCE
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
/// Returns a window with `complete == false` when the accepted clean days do
/// not yet reach the pre-registered count, when a not-final or missing day
/// sits before the window (a gap pauses rather than shortens it), or when the
/// pull sequence/schema is fatal. Incident and burn-in days are excluded
/// informationally: a single incident day never prevents later clean days
/// from completing the window.
pub fn build_window(
    control: &[ArmDay],
    test: &[ArmDay],
    window_days: usize,
    test_allocated_share: f64,
    burn_in_days: usize,
) -> AnalysisWindow {
    let mut problems = Vec::new();
    if !ALLOWED_WINDOW_DAYS.contains(&window_days) {
        problems.push(WindowProblem::Schema {
            day: format!("invalid window length {window_days}"),
        });
        return AnalysisWindow {
            days: Vec::new(),
            problems,
            complete: false,
            incomplete: false,
            fatal: true,
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
            incomplete: true,
            fatal: false,
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

        // Burn-in by calendar position: the pre-registered count.
        if index < burn_in_days {
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
                    incomplete: false,
                    fatal: false,
                };
            }
            continue;
        }
        // Finality compares both the numerator (revenue) and the denominator
        // (users) of the measured value, plus fill where the rule uses it.
        if !day_is_final(control_day) || !day_is_final(test_day) {
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
            test_users: test_day.users,
            control_users: control_day.users,
        });
    }

    // Whole weeks only, capped at the pre-registered window. `complete` means
    // enough accepted clean analysis days accumulated: incident and burn-in
    // exclusions are informational and never block it by themselves. A
    // temporary problem blocks only while it sits before the analysis window
    // (a gap pauses the window rather than shortening it); a fatal problem
    // always blocks.
    let whole_weeks = (analyzed.len() / 7) * 7;
    let keep = whole_weeks.min(window_days);
    analyzed.truncate(keep);
    let keep_index = analyzed.last().map(|day| day.calendar_index);
    let within_window = |problem: &WindowProblem| -> bool {
        match problem {
            WindowProblem::BurnIn { .. } | WindowProblem::Incident { .. } => false,
            WindowProblem::Sequence { .. } | WindowProblem::Schema { .. } => true,
            WindowProblem::NotFinal { day } | WindowProblem::MissingDay { day } => index_by_day
                .get(day.as_str())
                .map(|index| keep_index.is_none_or(|keep| *index <= keep))
                .unwrap_or(true),
        }
    };
    let fatal = problems.iter().any(WindowProblem::is_fatal);
    let incomplete = problems
        .iter()
        .any(|problem| problem.is_temporarily_incomplete() && within_window(problem));
    let complete = keep == window_days && !fatal && !incomplete;
    AnalysisWindow {
        days: analyzed,
        problems,
        complete,
        incomplete,
        fatal,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn first_at() -> i64 {
        1_700_000_000
    }

    fn first_hash() -> String {
        "a".repeat(64)
    }

    fn second_hash() -> String {
        "b".repeat(64)
    }

    fn day(name: &str, revenue: Micros, users: Option<u64>) -> ArmDay {
        ArmDay {
            day: name.to_string(),
            revenue,
            users,
            second_pull_revenue: Some(revenue),
            second_pull_users: users,
            fill: None,
            second_pull_fill: None,
            first_observed_at_unix_s: Some(first_at()),
            second_observed_at_unix_s: Some(first_at() + SECOND_PULL_MIN_SECS),
            first_source_sha256: Some(first_hash()),
            second_source_sha256: Some(second_hash()),
        }
    }

    fn day_with_fill(name: &str, revenue: Micros, users: Option<u64>, fill: Option<f64>) -> ArmDay {
        ArmDay {
            fill,
            second_pull_fill: fill,
            ..day(name, revenue, users)
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
        let window = build_window(&control, &test, 7, 0.25, 3);
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
        let window = build_window(&control, &test, 7, 0.25, 3);
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
        let window = build_window(&control, &test, 7, 0.25, 3);
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
        let window = build_window(&control, &test, 7, 0.25, 3);
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
        let window = build_window(&control, &test, 7, 0.25, 3);
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
        let single = build_window(&control, &test, 7, 0.25, 3);
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
        let two = build_window(&control2, &test2, 7, 0.25, 3);
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
        let short = build_window(&control, &test, 7, 0.25, 3);
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
        let zero_window = build_window(&zero, &zero, 7, 0.25, 3);
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
        let fill_window = build_window(&control_fill, &test_fill, 7, 0.25, 3);
        assert!(fill_window.days.iter().all(|day| day.fill_delta.is_none()));
    }

    #[test]
    fn window_length_must_be_a_whole_week() {
        let control = series(&[("2026-10-01", 1)]);
        let test = series(&[("2026-10-01", 1)]);
        let window = build_window(&control, &test, 10, 0.25, 3);
        assert!(!window.complete && window.days.is_empty());
        assert!(window.fatal);
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::Schema { .. })));
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
        let window = build_window(&control, &test, 7, 0.25, 3);
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
        let window = build_window(&zero, &zero, 7, 0.25, 3);
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
        test[4].second_pull_users = Some(0);
        let window = build_window(&control, &test, 7, 0.25, 3);
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
        let window = build_window(&control, &test, 7, 0.25, 3);
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
        control[6].second_pull_fill = Some(0.9);
        test[6].fill = Some(0.8);
        test[6].second_pull_fill = Some(0.8);
        let window = build_window(&control, &test, 7, 0.25, 3);
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

    #[test]
    fn the_same_read_relabelled_as_a_second_pull_is_not_final() {
        let mut control = day("2026-10-01", 1_000_000, None);
        control.second_observed_at_unix_s = control.first_observed_at_unix_s;
        control.second_source_sha256 = control.first_source_sha256.clone();
        let test = day("2026-10-01", 1_000_000, None);
        assert!(!day_is_final(&control));
        let window = build_window(
            std::slice::from_ref(&control),
            std::slice::from_ref(&test),
            7,
            0.25,
            0,
        );
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::NotFinal { .. })));
    }

    #[test]
    fn missing_observation_provenance_is_not_final() {
        let mut no_time = day("2026-10-01", 1_000_000, None);
        no_time.second_observed_at_unix_s = None;
        assert!(!day_is_final(&no_time));
        let mut no_source = day("2026-10-01", 1_000_000, None);
        no_source.second_source_sha256 = None;
        assert!(!day_is_final(&no_source));
        let mut no_first_source = day("2026-10-01", 1_000_000, None);
        no_first_source.first_source_sha256 = None;
        assert!(!day_is_final(&no_first_source));
    }

    #[test]
    fn a_second_pull_less_than_twenty_four_hours_later_is_not_final() {
        let mut control = day("2026-10-01", 1_000_000, None);
        control.second_observed_at_unix_s = Some(first_at() + SECOND_PULL_MIN_SECS - 1);
        assert!(!day_is_final(&control));
        // Exactly 24 hours later is a distinct, late enough read.
        control.second_observed_at_unix_s = Some(first_at() + SECOND_PULL_MIN_SECS);
        assert!(day_is_final(&control));
    }

    #[test]
    fn finality_tolerance_is_half_a_percent_not_fifty() {
        let mut control = day("2026-10-01", 1_000_000, Some(1_000));
        control.second_pull_revenue = Some(1_010_000);
        control.second_pull_users = Some(1_000);
        assert!(!day_is_final(&control), "1% revenue drift is not final");
        control.second_pull_revenue = Some(1_004_000);
        assert!(day_is_final(&control), "0.4% revenue drift is final");
    }

    #[test]
    fn revenue_stable_but_users_changed_is_not_final() {
        let mut control = day("2026-10-01", 1_000_000, Some(1_000));
        control.second_pull_revenue = Some(1_000_000);
        control.second_pull_users = Some(900);
        assert!(
            !day_is_final(&control),
            "a revenue-stable but users-changing day is not final"
        );
        // Both revenue and users stable is final.
        control.second_pull_users = Some(1_000);
        assert!(day_is_final(&control));
        // A users drop with a proportional revenue drop changes RPU only
        // slightly; the denominator rule still refuses it.
        control.second_pull_revenue = Some(900_000);
        control.second_pull_users = Some(900);
        assert!(!day_is_final(&control));
    }

    #[test]
    fn a_fill_change_between_pulls_is_not_final() {
        let mut control = day_with_fill("2026-10-01", 1_000_000, None, Some(0.90));
        control.second_pull_fill = Some(0.85);
        assert!(!day_is_final(&control));
        control.second_pull_fill = Some(0.904);
        assert!(day_is_final(&control));
    }

    #[test]
    fn one_incident_day_informational_and_later_clean_days_complete_the_window() {
        // Burn-in days 0..=2, one excluded incident on day 4, then clean days.
        // After enough clean days the window completes: a recorded incident
        // never blocks it forever.
        let names: Vec<String> = (1..=12).map(|day| format!("2026-10-{day:02}")).collect();
        let mut control: Vec<ArmDay> = names
            .iter()
            .map(|name| day(name, 1_000_000, None))
            .collect();
        let mut test = control.clone();
        control[4].revenue = 400_000;
        control[4].second_pull_revenue = Some(400_000);
        test[4].revenue = 400_000;
        test[4].second_pull_revenue = Some(400_000);
        let window = build_window(&control, &test, 7, 0.25, 3);
        assert!(
            window
                .problems
                .iter()
                .any(|p| matches!(p, WindowProblem::Incident { day } if day == "2026-10-05")),
            "the incident is recorded informationally: {:?}",
            window.problems
        );
        assert!(!window.fatal);
        assert!(!window.incomplete);
        assert_eq!(window.days.len(), 7);
        assert!(
            window.complete,
            "later clean days complete the window: {:?}",
            window.problems
        );
    }

    #[test]
    fn a_fatal_sequence_or_schema_problem_never_completes() {
        let names = [
            "2026-10-01",
            "2026-10-02",
            "2026-10-03",
            "2026-10-01",
            "2026-10-05",
            "2026-10-06",
            "2026-10-07",
            "2026-10-08",
            "2026-10-09",
            "2026-10-10",
        ];
        let control = series(&names.map(|name| (name, 1_000_000)));
        let test = control.clone();
        let window = build_window(&control, &test, 7, 0.25, 3);
        assert!(window.fatal);
        assert!(!window.complete);
        assert!(window
            .problems
            .iter()
            .any(|p| matches!(p, WindowProblem::Sequence { .. })));
        assert!(WindowProblem::Schema { day: "x".into() }.is_fatal());
        assert!(WindowProblem::MissingDay { day: "x".into() }.is_temporarily_incomplete());
        assert!(WindowProblem::Incident { day: "x".into() }.is_informational());
    }
}
