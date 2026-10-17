//! The re-run kit: a publisher exports the same report from their MAX or
//! LevelPlay dashboard as CSV and gets the same lift and bound to the
//! rounding. The CSV columns are `day,arm,revenue,users`; `arm` is `control`
//! or `test`; `users` may be blank on MAX, where the allocated share is
//! supplied on the command line instead.

use apex_core::Micros;

use crate::error::{LiftError, LiftResult};
use crate::stats::{estimate, Estimate};
use crate::window::ALLOWED_WINDOW_DAYS;

/// One parsed arm-day row.
#[derive(Debug, Clone, PartialEq)]
pub struct RerunRow {
    pub day: String,
    pub test: bool,
    pub revenue: Micros,
    pub users: Option<u64>,
}

/// The full re-run report, ready to print as JSON next to the invoice.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RerunReport {
    pub days: usize,
    pub mean_log: f64,
    pub lift: f64,
    pub lower_90: f64,
    pub upper_90: f64,
    /// The per-day log ratios, so the publisher can check every number.
    pub daily_log_ratios: Vec<f64>,
}

/// Parse the CSV export. Header required; unknown arms and malformed numbers
/// are refused with the row number.
pub fn parse_csv(csv: &str) -> LiftResult<Vec<RerunRow>> {
    let mut lines = csv.lines().filter(|line| !line.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| LiftError::Malformed("empty CSV".into()))?;
    let columns: Vec<&str> = header.split(',').map(str::trim).collect();
    let index = |name: &str| columns.iter().position(|c| *c == name);
    let (day_at, arm_at, revenue_at, users_at) = (
        index("day").ok_or_else(|| LiftError::Malformed("CSV needs a 'day' column".into()))?,
        index("arm").ok_or_else(|| LiftError::Malformed("CSV needs an 'arm' column".into()))?,
        index("revenue")
            .ok_or_else(|| LiftError::Malformed("CSV needs a 'revenue' column".into()))?,
        index("users"),
    );
    let mut rows = Vec::new();
    for (offset, line) in lines.enumerate() {
        let cells: Vec<&str> = line.split(',').map(str::trim).collect();
        let row_number = offset + 2;
        let day = cells
            .get(day_at)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| LiftError::Malformed(format!("row {row_number}: missing day")))?;
        let arm = cells
            .get(arm_at)
            .ok_or_else(|| LiftError::Malformed(format!("row {row_number}: missing arm")))?;
        let test = match *arm {
            "control" => false,
            "test" => true,
            other => {
                return Err(LiftError::Malformed(format!(
                    "row {row_number}: arm '{other}' is not control or test"
                )))
            }
        };
        let revenue_raw = cells
            .get(revenue_at)
            .ok_or_else(|| LiftError::Malformed(format!("row {row_number}: missing revenue")))?;
        let revenue = parse_csv_revenue(revenue_raw, row_number)?;
        let users = match users_at.and_then(|at| cells.get(at)).copied() {
            Some("") | None => None,
            Some(value) => Some(value.parse::<u64>().map_err(|_| {
                LiftError::Malformed(format!("row {row_number}: users '{value}' is not a number"))
            })?),
        };
        rows.push(RerunRow {
            day: day.to_string(),
            test,
            revenue,
            users,
        });
    }
    if rows.is_empty() {
        return Err(LiftError::Malformed("CSV has no data rows".into()));
    }
    Ok(rows)
}

/// Compute the report from parsed rows. `window_days` must be one of the
/// engine's whole-week lengths and `allocated_share` is the test arm's user
/// share, used when the export has no user counts.
pub fn rerun(
    rows: &[RerunRow],
    window_days: usize,
    allocated_share: f64,
) -> LiftResult<RerunReport> {
    if !ALLOWED_WINDOW_DAYS.contains(&window_days) {
        return Err(LiftError::Malformed(format!(
            "window must be one of {ALLOWED_WINDOW_DAYS:?}"
        )));
    }
    let mut days: Vec<String> = Vec::new();
    for row in rows {
        if !days.contains(&row.day) {
            days.push(row.day.clone());
        }
    }
    days.sort();
    let mut ratios = Vec::new();
    for day in &days {
        let control = rows.iter().find(|r| &r.day == day && !r.test);
        let test = rows.iter().find(|r| &r.day == day && r.test);
        let (Some(control), Some(test)) = (control, test) else {
            continue;
        };
        let control_value = match control.users {
            Some(users) if users > 0 => control.revenue as f64 / users as f64,
            Some(_) => continue,
            None => control.revenue as f64 / (1.0 - allocated_share),
        };
        let test_value = match test.users {
            Some(users) if users > 0 => test.revenue as f64 / users as f64,
            Some(_) => continue,
            None => test.revenue as f64 / allocated_share,
        };
        if control_value > 0.0 && test_value > 0.0 {
            ratios.push((test_value / control_value).ln());
        }
    }
    if ratios.len() > window_days {
        let keep = (window_days / 7) * 7;
        ratios.truncate(keep);
    }
    let estimate: Estimate = estimate(&ratios)
        .ok_or_else(|| LiftError::Malformed("the export has fewer than two usable days".into()))?;
    Ok(RerunReport {
        days: estimate.days,
        mean_log: estimate.mean_log,
        lift: estimate.lift,
        lower_90: estimate.lower_90,
        upper_90: estimate.upper_90,
        daily_log_ratios: ratios,
    })
}

/// Parse one revenue cell: plain decimal string in currency units.
pub(crate) fn parse_csv_revenue(raw: &str, row: usize) -> LiftResult<Micros> {
    crate::parse_revenue_micros(raw)
        .ok_or_else(|| LiftError::Malformed(format!("row {row}: revenue '{raw}' is not a number")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const CSV: &str = "day,arm,revenue,users\n2026-10-01,control,100.00,1000\n2026-10-01,test,106.00,1000\n2026-10-02,control,100.00,1000\n2026-10-02,test,106.00,1000\n2026-10-03,control,100.00,1000\n2026-10-03,test,106.00,1000\n2026-10-04,control,100.00,1000\n2026-10-04,test,106.00,1000\n2026-10-05,control,100.00,1000\n2026-10-05,test,106.00,1000\n2026-10-06,control,100.00,1000\n2026-10-06,test,106.00,1000\n2026-10-07,control,100.00,1000\n2026-10-07,test,106.00,1000\n";

    #[test]
    fn the_kit_reruns_the_bound_from_a_publisher_export() {
        let rows = parse_csv(CSV).expect("rows");
        assert_eq!(rows.len(), 14);
        let report = rerun(&rows, 7, 0.5).expect("report");
        assert_eq!(report.days, 7);
        assert!((report.lift - 0.06).abs() < 1e-9, "{}", report.lift);
        assert!((report.lower_90 - 0.06).abs() < 1e-9);
        let json = serde_json::to_string(&report).expect("json");
        assert!(json.contains("lower90"), "{json}");
    }

    #[test]
    fn share_based_rows_work_when_users_are_absent() {
        let csv = "day,arm,revenue\n2026-10-01,control,75.00\n2026-10-01,test,26.50\n2026-10-02,control,75.00\n2026-10-02,test,26.50\n";
        let rows = parse_csv(csv).expect("rows");
        let report = rerun(&rows, 7, 0.25).expect("report");
        // control 75/0.75 = 100, test 26.50/0.25 = 106 -> +6%.
        assert!((report.lift - 0.06).abs() < 1e-9, "{}", report.lift);
    }

    #[test]
    fn malformed_exports_are_refused_with_their_row() {
        let bad_arm = "day,arm,revenue\n2026-10-01,other,1\n";
        assert!(parse_csv(bad_arm)
            .unwrap_err()
            .to_string()
            .contains("row 2"));
        let bad_revenue = "day,arm,revenue\n2026-10-01,control,abc\n";
        assert!(parse_csv(bad_revenue).is_err());
        let bad_users = "day,arm,revenue,users\n2026-10-01,control,1,ten\n";
        assert!(parse_csv(bad_users).is_err());
        assert!(parse_csv("").is_err());
        assert!(parse_csv("day,arm,revenue\n").is_err());
        let missing_day = "day,arm,revenue\n,control,1\n";
        assert!(parse_csv(missing_day).is_err());
    }

    #[test]
    fn partial_days_and_zero_users_are_skipped() {
        let csv = "day,arm,revenue,users\n2026-10-01,control,100,1000\n2026-10-02,control,100,1000\n2026-10-02,test,106,1000\n2026-10-03,control,100,0\n2026-10-03,test,106,1000\n2026-10-04,control,100,1000\n2026-10-04,test,106,1000\n2026-10-05,control,100,1000\n2026-10-05,test,106,1000\n";
        let rows = parse_csv(csv).expect("rows");
        let report = rerun(&rows, 7, 0.5).expect("report");
        // Day one has no test partner and day three has zero users: three
        // usable days remain.
        assert_eq!(report.days, 3);
        assert!(report.daily_log_ratios.len() == 3);
    }

    #[test]
    fn invalid_windows_refuse() {
        let rows = parse_csv(CSV).expect("rows");
        assert!(rerun(&rows, 10, 0.5).is_err());
    }

    #[test]
    fn a_zero_user_test_row_is_skipped_and_extra_days_truncate() {
        // Ten clean days but the fifth test row reports zero users: that day
        // is skipped, leaving nine ratios for a seven-day window.
        let mut csv = String::from("day,arm,revenue,users\n");
        for day in 1..=10 {
            let users = if day == 5 { "0" } else { "1000" };
            csv.push_str(&format!("2026-10-{day:02},control,100.00,1000\n"));
            csv.push_str(&format!("2026-10-{day:02},test,106.00,{users}\n"));
        }
        let rows = parse_csv(&csv).expect("rows");
        let report = rerun(&rows, 7, 0.5).expect("report");
        // Nine usable days truncate to the whole-week window of seven.
        assert_eq!(report.days, 7);
        assert_eq!(report.daily_log_ratios.len(), 7);
        assert!((report.lift - 0.06).abs() < 1e-9, "{}", report.lift);
    }
}
