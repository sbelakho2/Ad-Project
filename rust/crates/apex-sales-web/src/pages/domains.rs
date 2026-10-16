//! `/domains`: the sending domain's health, the rate tests, the cumulative-sum state
//! and the operator reset.

use apex_sales::health::SenderState;
use apex_sales::model::Outcome;
use apex_sales::store::Event;
use apex_ui::components::{csrf_hidden, data_table, field, input, notice, panel, Cell, EmptyState};
use apex_ui::format::{or_not_available, percent, utc_datetime};
use apex_ui::status::StatusKind;

use crate::data::RequestData;
use crate::pages::shared::{self, rate, rate_float, PageMeta};
use crate::signing::random_hex_32;

/// The exact phrase the reset form requires, mirroring the control-plane
/// destructive actions.
pub const RESET_PHRASE: &str = "RESET HEALTH";

/// Render `/domains`.
pub fn render(meta: &PageMeta, data: &RequestData) -> String {
    let domain = data
        .config
        .sender
        .email
        .rsplit('@')
        .next()
        .unwrap_or_default()
        .to_string();
    let (sends, complaints, bounces) = raw_counts(data);
    let health = &data.learned.health;
    let health_cfg = &data.config.health;
    let p_complaints = health.p_complaints_over(health_cfg);
    let p_bounces = health.p_bounces_over(health_cfg);

    let row = vec![
        Cell::Mono(domain.clone()),
        Cell::Status(
            state_label(health.state).to_string(),
            state_kind(health.state),
        ),
        Cell::Num(data.learned.sent_today.to_string()),
        Cell::Num(format!("{} (ceiling {})", health.cap, health_cfg.max_cap)),
        Cell::Text(rate(complaints, sends)),
        Cell::Text(rate_test(
            p_complaints,
            health_cfg.warn_confidence,
            health_cfg.breach_confidence,
            health_cfg.complaint_ceiling,
            "complaint",
        )),
        Cell::Text(rate(bounces, sends)),
        Cell::Text(rate_test(
            p_bounces,
            health_cfg.warn_confidence,
            health_cfg.breach_confidence,
            health_cfg.bounce_ceiling,
            "bounce",
        )),
        Cell::Text(format!(
            "S {:.2} of h {:.1} ({})",
            health.cusum,
            health_cfg.cusum_h,
            if health.cusum > health_cfg.cusum_h {
                "crossed: complaint jump detected"
            } else {
                "no jump detected"
            }
        )),
        Cell::Text(format!(
            "{} sends / {} complaints / {} hard bounces (discounted, about a month of memory)",
            rate_float(health.sent, health.sent),
            rate_float(health.complaints, health.sent),
            rate_float(health.hard_bounces, health.sent)
        )),
    ];
    let table = data_table(
        Some("Sending domain"),
        &[
            "Domain",
            "State",
            "Sends today",
            "Warm-up target",
            "Complaint rate",
            "Complaint rate test",
            "Bounce rate",
            "Bounce rate test",
            "Cumulative sum",
            "Discounted window",
        ],
        &[row],
        None,
        Some("Rates are raw event counts n/N; the rate tests use the health module's discounted counts. One sending domain is configured (APEX_SALES_FROM)."),
    );
    let body = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        banner(data),
        table,
        reset_history(data),
        reset_form(meta),
        "<p class=\"table-note\">The rate tests are the engine's own posteriors: P(rate &gt; ceiling) under the health module's priors, with the warning and breach confidences from the configuration; the cumulative-sum statistic (Page, 1954) tracks the complaint stream.</p>",
        "<p class=\"table-note\">Warm-up: a healthy day that uses at least 80% of the cap raises it by the configured step up to the ceiling; a warning halves it; a breach sets it to zero. A quarantined domain stays at zero until an operator reset.</p>"
    );
    shared::page(meta, &body)
}

fn state_label(state: SenderState) -> &'static str {
    match state {
        SenderState::Warming => "Warming",
        SenderState::Healthy => "Healthy",
        SenderState::Throttled => "Throttled",
        SenderState::Paused => "Paused",
        SenderState::Quarantined => "Quarantined",
    }
}

fn state_kind(state: SenderState) -> StatusKind {
    match state {
        SenderState::Warming => StatusKind::InProgress,
        SenderState::Healthy => StatusKind::Verified,
        SenderState::Throttled => StatusKind::Attention,
        SenderState::Paused => StatusKind::Attention,
        SenderState::Quarantined => StatusKind::Failure,
    }
}

/// Raw `(sends, complaints, hard bounces)` from the event history.
pub fn raw_counts(data: &RequestData) -> (usize, usize, usize) {
    let mut sends = 0usize;
    let mut complaints = 0usize;
    let mut bounces = 0usize;
    for event in &data.snap.events {
        match event {
            Event::Touch { .. } => sends += 1,
            Event::Outcome {
                outcome: Outcome::Complaint,
                ..
            } => complaints += 1,
            Event::Outcome {
                outcome: Outcome::HardBounce,
                ..
            } => bounces += 1,
            _ => {}
        }
    }
    (sends, complaints, bounces)
}

fn rate_test(p: f64, warn: f64, breach: f64, ceiling: f64, what: &str) -> String {
    let verdict = if p >= breach {
        "breach: the domain stops"
    } else if p >= warn {
        "warning: the cap halves"
    } else {
        "below the warning line"
    };
    format!(
        "P({what} rate > {}) = {:.3} — {verdict} (warn {warn:.2}, breach {breach:.2})",
        percent(ceiling),
        p
    )
}

/// The banner for a non-sending state, with the reasons the detectors fired.
fn banner(data: &RequestData) -> String {
    let health = &data.learned.health;
    let health_cfg = &data.config.health;
    let p_complaints = health.p_complaints_over(health_cfg);
    let p_bounces = health.p_bounces_over(health_cfg);
    let mut reasons = Vec::new();
    if p_complaints >= health_cfg.breach_confidence {
        reasons.push(format!(
            "complaints: P(complaint rate > {}) = {:.3} is at or above the breach confidence {:.2}",
            percent(health_cfg.complaint_ceiling),
            p_complaints,
            health_cfg.breach_confidence
        ));
    }
    if p_bounces >= health_cfg.breach_confidence {
        reasons.push(format!(
            "hard bounces: P(bounce rate > {}) = {:.3} is at or above the breach confidence {:.2}",
            percent(health_cfg.bounce_ceiling),
            p_bounces,
            health_cfg.breach_confidence
        ));
    }
    if health.cusum > health_cfg.cusum_h {
        reasons.push(format!(
            "complaint jump: the cumulative-sum statistic S = {:.2} crossed h = {:.1}, so the complaint stream moved from {:.3}% to {:.1}% per send within a few dozen sends",
            health.cusum,
            health_cfg.cusum_h,
            health_cfg.cusum_p0 * 100.0,
            health_cfg.cusum_p1 * 100.0
        ));
    }
    match health.state {
        SenderState::Quarantined => {
            let body = if reasons.is_empty() {
                "Quarantined: complaints breached a detector; the detector values are at their thresholds again, so inspect the send history before resetting.".to_string()
            } else {
                format!("Quarantined. {}", reasons.join("; "))
            };
            format!(
                "{}\n",
                notice(
                    StatusKind::Failure,
                    "The sending domain is quarantined",
                    &body,
                    Some(("Reset after investigating", "#reset-health")),
                )
            )
        }
        SenderState::Paused => {
            let since = health
                .paused_on
                .map(|day| utc_datetime(day * 86_400 * 1000))
                .unwrap_or_else(|| or_not_available("pause day not recorded"));
            format!(
                "{}\n",
                notice(
                    StatusKind::Attention,
                    "The sending domain is paused",
                    &format!(
                        "Hard bounces breached the rate test (paused on {since}). It resumes throttled at half the warm-up cap after {} days.",
                        health_cfg.pause_days
                    ),
                    None,
                )
            )
        }
        SenderState::Throttled => format!(
            "{}\n",
            notice(
                StatusKind::Attention,
                "The sending domain is throttled",
                "A detector is between its warning and breach confidences, so the daily cap is halved until the window clears.",
                None,
            )
        ),
        _ => String::new(),
    }
}

fn reset_history(data: &RequestData) -> String {
    let resets: Vec<i64> = data
        .snap
        .events
        .iter()
        .filter_map(|event| match event {
            Event::HealthReset { at } => Some(*at),
            _ => None,
        })
        .collect();
    let rows: Vec<Vec<Cell>> = resets
        .iter()
        .rev()
        .map(|at| {
            vec![
                Cell::Text(utc_datetime(at * 1000)),
                Cell::Text("Operator reset after an investigation".to_string()),
            ]
        })
        .collect();
    let empty = EmptyState {
        reason: "The domain has never been reset.".to_string(),
        action: None,
    };
    data_table(
        Some("Resets"),
        &["At (UTC)", "What happened"],
        &rows,
        Some(empty),
        Some(&shared::count_note(resets.len(), resets.len(), "resets")),
    )
}

fn reset_form(meta: &PageMeta) -> String {
    let form = format!(
        "<form method=\"post\" action=\"/actions/reset-health\" id=\"reset-health\">\n{}{}\n{}\n<p class=\"form-actions\"><button class=\"action action-primary\" type=\"submit\">Reset the domain</button></p></form>",
        csrf_hidden(&meta.csrf),
        hidden("idempotency_key", &random_hex_32()),
        field(
            "reset-confirmation",
            "Confirmation",
            Some(&format!(
                "Type {RESET_PHRASE} exactly. The reset writes the same durable event the CLI's reset-health writes: back to warming at the starting cap with clean counters."
            )),
            None,
            &input(
                "reset-confirmation",
                "confirmation",
                "text",
                None,
                true,
                Some("off"),
            ),
        )
    );
    panel(
        "Reset the sending domain",
        &format!("{form}\n<p class=\"table-note\">Do this only after investigating the send history and the complaints: the reset clears the detectors, not the underlying facts.</p>"),
        None,
    )
}

fn hidden(name: &str, value: &str) -> String {
    super::publisher::hidden(name, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_sales::model::DAY;

    const NOW: i64 = 20_745 * DAY + 10 * 3600;

    fn fixture(dir: &std::path::Path) -> RequestData {
        let store = apex_sales::store::StateDir::open(dir).unwrap();
        for i in 0..30 {
            store
                .put_event(&Event::Touch {
                    at: NOW - DAY + i,
                    email: format!("c{i}@studio.example"),
                    publisher: "studio.example".into(),
                    step: 0,
                    arm: "t0-finding".into(),
                    message_id: format!("m{i}"),
                    local_hour: 10,
                    predicted: None,
                })
                .unwrap();
        }
        store
            .put_event(&Event::Outcome {
                at: NOW - DAY,
                email: Some("c1@studio.example".into()),
                publisher: None,
                outcome: Outcome::HardBounce,
                note: None,
            })
            .unwrap();
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        crate::data::load_from(&env, NOW).unwrap()
    }

    #[test]
    fn the_table_reports_rates_as_counts_and_the_tests_state() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-dom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = fixture(&dir);
        let meta = PageMeta::new(
            "Domains",
            "Domains",
            Vec::new(),
            NOW,
            dir.display().to_string(),
            "a".repeat(32),
        );
        let html = render(&meta, &data);
        assert!(html.contains("Sending domain"));
        assert!(html.contains("1/30"), "bounce rate as n/N: {html}");
        assert!(html.contains("0/30"), "complaint rate as n/N");
        assert!(
            html.contains("P(bounce rate &gt; 3.0%)") || html.contains("P(bounce rate > 3.0%)")
        );
        assert!(html.contains("Cumulative sum"));
        assert!(html.contains("RESET HEALTH"));
        assert!(html.contains("action=\"/actions/reset-health\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quarantine_banner_names_the_detector() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-dom2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = apex_sales::store::StateDir::open(&dir).unwrap();
        // A complaint burst crosses the cumulative-sum threshold quickly.
        for i in 0..40 {
            store
                .put_event(&Event::Touch {
                    at: NOW - DAY + i,
                    email: format!("c{i}@s.example"),
                    publisher: "s.example".into(),
                    step: 0,
                    arm: "a".into(),
                    message_id: format!("m{i}"),
                    local_hour: 9,
                    predicted: None,
                })
                .unwrap();
        }
        for i in 0..5 {
            store
                .put_event(&Event::Outcome {
                    at: NOW - DAY + 100 + i,
                    email: Some(format!("c{i}@s.example")),
                    publisher: None,
                    outcome: Outcome::Complaint,
                    note: None,
                })
                .unwrap();
        }
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        let data = crate::data::load_from(&env, NOW).unwrap();
        assert_eq!(data.learned.health.state, SenderState::Quarantined);
        let meta = PageMeta::new(
            "Domains",
            "Domains",
            Vec::new(),
            NOW,
            dir.display().to_string(),
            "",
        );
        let html = render(&meta, &data);
        assert!(html.contains("quarantined"));
        assert!(html.contains("cumulative-sum statistic") && html.contains("crossed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_events_the_rates_say_not_available() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-dom3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        let data = crate::data::load_from(&env, NOW).unwrap();
        let meta = PageMeta::new(
            "Domains",
            "Domains",
            Vec::new(),
            NOW,
            dir.display().to_string(),
            "",
        );
        let html = render(&meta, &data);
        assert!(html.contains("Not available (no observations)"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
