//! `/` Overview: the state of the outreach at a glance.

use std::collections::BTreeMap;

use apex_ui::charts::{bar_chart, ChartColor, ChartSpec};
use apex_ui::components::{data_table, kpi_card, kpi_row, notice, panel, Cell, EmptyState};
use apex_ui::format::{date_short, percent, utc_datetime};
use apex_ui::status::StatusKind;

use apex_sales::decision::Verdict;
use apex_sales::engine::plan;
use apex_sales::funnel::Stage;
use apex_sales::model::{Outcome, DAY};

use crate::data::{RequestData, Transport};
use crate::pages::shared::{self, PageMeta};

/// The chart window: sends per day over the last 30 UTC days.
pub const CHART_DAYS: i64 = 30;

/// Render the overview page.
pub fn render(meta: &PageMeta, data: &RequestData, transport: &Transport) -> String {
    let snap = &data.snap;
    let learned = &data.learned;
    let health = &learned.health;
    let domains = health.domains();

    let publishers = snap.publishers.len();
    let mut suppressed_contacts = 0usize;
    // One index for the whole count: a per-contact scan of the suppression
    // list would make the overview quadratic in contacts × suppressions.
    let suppressions = data.suppression_index();
    for contact in snap.contacts.values() {
        if suppressions.check(contact).is_some() {
            suppressed_contacts += 1;
        }
    }
    let contacts = snap.contacts.len();
    let active = contacts - suppressed_contacts;

    let (replies_interested, replies_classified) = reply_counts(data);
    let held_out = snap
        .publishers
        .keys()
        .filter(|key| data.config.holdout.is_held_out(key))
        .count();

    // One domain reads exactly as before; several domains summarize.
    let sending_value: String = if domains.len() <= 1 {
        state_label(health.summary_state()).to_string()
    } else {
        let can_send = domains
            .iter()
            .filter(|domain| health.get(domain).is_some_and(|h| h.state.may_send()))
            .count();
        format!("{can_send}/{} can send", domains.len())
    };
    let sending_note = if domains.len() <= 1 {
        let h = domains.first().and_then(|domain| health.get(domain));
        format!(
            "cap {}; {}; CUSUM {:.2} of {:.1}",
            h.map_or(0, |h| h.cap),
            complaints_sentence(data),
            h.map_or(0.0, |h| h.cusum),
            data.config.health.cusum_h
        )
    } else {
        format!(
            "total cap {}; each domain keeps its own cap, breakers and warm-up",
            health.total_cap()
        )
    };

    let cards = kpi_row(&[
        kpi_card(
            "Publishers tracked",
            &publishers.to_string(),
            &format!("in {}", super::shared::store_label(&data.dir)),
            Some("/leads"),
        ),
        kpi_card(
            "Active contacts",
            &active.to_string(),
            &format!("{suppressed_contacts} suppressed of {contacts} total"),
            Some("/leads"),
        ),
        kpi_card(
            "Sent today",
            &health.sent_today().to_string(),
            &format!(
                "of today's cap {} (warm-up ceiling {})",
                health.total_cap(),
                data.config.health.max_cap
            ),
            Some("/sends"),
        ),
        kpi_card(
            "Interested replies",
            &format!("{replies_interested}/{replies_classified}"),
            "classified human replies that count as interest",
            Some("/replies"),
        ),
        kpi_card(
            "Holdout share",
            &percent(data.config.holdout.share),
            &format!("{held_out} of {publishers} publishers are held out of outreach"),
            Some("/models"),
        ),
        kpi_card(
            if domains.len() <= 1 {
                "Sending domain"
            } else {
                "Sending domains"
            },
            &sending_value,
            &sending_note,
            Some("/domains"),
        ),
    ]);

    let body = format!(
        "{cards}\n{}\n{}\n{}\n{}",
        notices(data),
        chart_block(data, meta),
        next_steps(meta, data, transport),
        panel(
            "Where this page's numbers come from",
            &format!(
                "<p>The engine rebuilt every model from {} events in <code>{}</code> as of {}; this page reads only its public functions (sends and states from <code>apex_sales::engine</code>, suppression and stages from the learned models).</p>",
                snap.events.len(),
                apex_ui::escape::html_escape(&super::shared::store_label(&data.dir)),
                utc_datetime(meta.as_of_ms)
            ),
            None,
        )
    );
    shared::page(meta, &body)
}

/// `(interested, classified)` human replies from the event history: the
/// classification exactly as recorded, `reply:*` outcomes only.
pub fn reply_counts(data: &RequestData) -> (usize, usize) {
    let mut interested = 0usize;
    let mut classified = 0usize;
    for event in &data.snap.events {
        if let apex_sales::store::Event::Outcome {
            outcome: Outcome::Reply(class),
            ..
        } = event
        {
            classified += 1;
            if class.is_interest() {
                interested += 1;
            }
        }
    }
    (interested, classified)
}

fn state_label(state: apex_sales::health::SenderState) -> &'static str {
    match state {
        apex_sales::health::SenderState::Warming => "Warming",
        apex_sales::health::SenderState::Healthy => "Healthy",
        apex_sales::health::SenderState::Throttled => "Throttled",
        apex_sales::health::SenderState::Paused => "Paused",
        apex_sales::health::SenderState::Quarantined => "Quarantined",
    }
}

fn complaints_sentence(data: &RequestData) -> String {
    let p = data
        .learned
        .health
        .domains()
        .first()
        .and_then(|domain| data.learned.health.get(domain))
        .map_or(0.0, |h| h.p_complaints_over(&data.config.health));
    format!(
        "P(complaint rate > {}) = {:.2}",
        percent(data.config.health.complaint_ceiling),
        p
    )
}

/// Honest notices: skipped lines, sender breakers, empty store.
fn notices(data: &RequestData) -> String {
    let mut out = String::new();
    if data.snap.skipped_lines > 0 {
        out.push_str(&notice(
            StatusKind::Attention,
            "Some state lines could not be read",
            &format!(
                "{} unreadable lines were skipped while loading {}; the engine's report prints the same count.",
                data.snap.skipped_lines,
                data.dir.display()
            ),
            None,
        ));
        out.push('\n');
    }
    let health = &data.learned.health;
    let multi = health.domains().len() > 1;
    let mut first = true;
    for domain in health.domains() {
        let Some(h) = health.get(domain) else {
            continue;
        };
        let title = |base: &str| {
            if multi {
                format!("{base} ({domain})")
            } else {
                base.to_string()
            }
        };
        match h.state {
            apex_sales::health::SenderState::Quarantined => {
                if !first {
                    out.push('\n');
                }
                first = false;
                out.push_str(&notice(
                    StatusKind::Failure,
                    &title("The sending domain is quarantined"),
                    "Complaints breached the Bayesian rate test or the CUSUM jump test. No outreach is planned until an operator investigates and resets the domain.",
                    Some(("Open the domain", "/domains")),
                ));
            }
            apex_sales::health::SenderState::Paused => {
                if !first {
                    out.push('\n');
                }
                first = false;
                out.push_str(&notice(
                    StatusKind::Attention,
                    &title("The sending domain is paused"),
                    "Hard bounces breached the rate test. The domain rests, then resumes throttled if the window clears.",
                    Some(("Open the domain", "/domains")),
                ));
            }
            _ => {}
        }
    }
    out
}

/// Touch events per UTC day over the last [`CHART_DAYS`] days. Days before
/// the first recorded send are gaps (`None`); a day inside the observed
/// range with no sends is a real zero.
fn sends_per_day(data: &RequestData) -> Vec<Option<f64>> {
    let today = data.now.div_euclid(DAY);
    let mut counts: BTreeMap<i64, u32> = BTreeMap::new();
    for event in &data.snap.events {
        if matches!(event, apex_sales::store::Event::Touch { .. }) {
            *counts.entry(event.at().div_euclid(DAY)).or_default() += 1;
        }
    }
    let first = counts.keys().next().copied();
    (0..CHART_DAYS)
        .map(|offset| {
            let day = today - (CHART_DAYS - 1) + offset;
            match first {
                Some(first) if day >= first => {
                    Some(f64::from(counts.get(&day).copied().unwrap_or(0)))
                }
                _ => None,
            }
        })
        .collect()
}

fn chart_block(data: &RequestData, meta: &PageMeta) -> String {
    let values = sends_per_day(data);
    let today = data.now.div_euclid(DAY);
    // `date_short` takes unix milliseconds; `today` is a day count, so it
    // converts back through whole days, not seconds (days * 1000 renders
    // every label as 1970).
    let labels: Vec<String> = (0..CHART_DAYS)
        .map(|offset| {
            let day = today - (CHART_DAYS - 1) + offset;
            date_short(day.saturating_mul(DAY).saturating_mul(1000))
        })
        .collect();
    let spec = ChartSpec {
        title: "Sends per day".to_string(),
        desc: "Touch events handed to the transport per UTC day over the last 30 days. Days before the first recorded send are gaps, not zeros.".to_string(),
        width: 720,
        height: 220,
        y_label: "Sends".to_string(),
        x_labels: labels,
        zero_line: false,
    };
    let chart = bar_chart(&spec, &values, ChartColor::Ink);
    let recorded_days = values.iter().filter(|value| value.is_some()).count();
    let note = if recorded_days == 0 {
        "<p class=\"table-note\">No sends are recorded yet, so the chart has no bars; the table lists gaps with their reason.</p>".to_string()
    } else {
        format!(
            "<p class=\"table-note\">{} of the last 30 days are inside the recorded history (as of {}).</p>",
            recorded_days,
            utc_datetime(meta.as_of_ms)
        )
    };
    format!("{chart}\n{note}")
}

/// The next-steps table: only actions the state actually implies, each with
/// a real link.
fn next_steps(meta: &PageMeta, data: &RequestData, transport: &Transport) -> String {
    let mut steps: Vec<(String, String, String)> = Vec::new();
    let publishers = data.snap.publishers.len();
    if publishers == 0 && data.snap.contacts.is_empty() {
        steps.push((
            "Import the first leads".to_string(),
            "No publishers or contacts are tracked yet. apex-sales import-seeds seeds.csv or import-leads leads.jsonl fills the store; the free-audit form feeds the latter.".to_string(),
            "/leads".to_string(),
        ));
    } else {
        let items = plan(&data.snap, &data.learned, &data.config, data.now);
        let live = items
            .iter()
            .filter(|item| item.decision.verdict != Verdict::Deny)
            .count();
        let waiting = items
            .iter()
            .filter(|item| item.decision.verdict == Verdict::NeedsApproval)
            .count();
        let denied = items
            .iter()
            .filter(|item| item.decision.verdict == Verdict::Deny)
            .count();
        if waiting > 0 {
            steps.push((
                "Approve the planned touches".to_string(),
                format!("{waiting} planned touches wait for operator approval in this mode."),
                "/plan".to_string(),
            ));
        }
        if !data.learned.health.may_send() {
            let domains = data.learned.health.domains();
            let why = if domains.len() <= 1 {
                format!(
                    "The domain is {}; outreach stops until the health page's reset after an investigation.",
                    data.learned.health.summary_state().as_str()
                )
            } else {
                format!(
                    "No sending domain may send ({}); outreach stops until the health page's reset after an investigation.",
                    domains.join(", ")
                )
            };
            steps.push((
                "Investigate the sending domain".to_string(),
                why,
                "/domains".to_string(),
            ));
        }
        if live > 0 && data.learned.health.sent_today() == 0 {
            steps.push((
                "Run the sender".to_string(),
                format!(
                    "{live} live touches are planned and nothing has been sent today; apex-sales run executes them through {}.",
                    crate::data::transport_label(transport)
                ),
                "/sends".to_string(),
            ));
        }
        if denied > 0 {
            steps.push((
                "Read the gate reasons".to_string(),
                format!("{denied} targets are excluded from today's plan; the plan page lists every failure reason."),
                "/plan".to_string(),
            ));
        }
        let (interested, classified) = reply_counts(data);
        if classified > interested {
            steps.push((
                "Review the reply classifications".to_string(),
                format!(
                    "{classified} classified replies, {interested} counted as interest; check the others and record outcomes by hand.",
                ),
                "/replies".to_string(),
            ));
        }
        steps.push((
            "Check the models".to_string(),
            "Calibration, message arms, funnel posteriors and the holdout effect are on the models page; nothing there is interpolated.".to_string(),
            "/models".to_string(),
        ));
    }
    let rows: Vec<Vec<Cell>> = steps
        .into_iter()
        .map(|(action, why, href)| {
            vec![
                Cell::Text(action),
                Cell::Text(why),
                Cell::Link("Open".to_string(), href),
            ]
        })
        .collect();
    let empty = EmptyState {
        reason: "No immediate next step is implied by the current state.".to_string(),
        action: None,
    };
    data_table(
        Some("Next steps"),
        &["Action", "Why", "Where"],
        &rows,
        Some(empty),
        Some(&format!("as of {}", utc_datetime(meta.as_of_ms))),
    )
}

/// The stage displayed for a publisher, from the learned ladder.
pub fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Contacted => "Contacted",
        Stage::Interested => "Interested",
        Stage::AuditRequested => "Audit requested",
        Stage::Connected => "Connected",
        Stage::Paid => "Paid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Transport;

    fn fixture_data(dir: std::path::PathBuf) -> RequestData {
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        crate::data::load_from(&env, 20_745 * DAY + 10 * 3600).expect("load")
    }

    #[test]
    fn the_empty_store_renders_the_import_step() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-ov-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = fixture_data(dir.clone());
        let meta = PageMeta::new(
            "Overview",
            "Overview",
            Vec::new(),
            data.now,
            dir.display().to_string(),
            "",
        );
        let html = render(&meta, &data, &Transport::Sendmail);
        assert!(html.contains("Publishers tracked"));
        assert!(html.contains("Import the first leads"));
        assert!(html.contains("No sends are recorded yet"));
        assert_eq!(html.matches("<h1").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn send_counts_mark_days_before_history_as_gaps() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-ov2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        let now = 20_745 * DAY + 10 * 3600;
        {
            let store = apex_sales::store::StateDir::open(&dir).unwrap();
            store
                .put_event(&apex_sales::store::Event::Touch {
                    at: now - 2 * DAY,
                    email: "a@b.io".into(),
                    publisher: "b.io".into(),
                    from: None,
                    step: 0,
                    arm: "t0-finding".into(),
                    message_id: "m".into(),
                    local_hour: 9,
                    predicted: None,
                })
                .unwrap();
        }
        let data = crate::data::load_from(&env, now).unwrap();
        let values = sends_per_day(&data);
        assert_eq!(values.len(), CHART_DAYS as usize);
        assert_eq!(values[27], Some(1.0), "two days before today");
        assert_eq!(values[26], None, "still a gap before the first send");
        assert_eq!(values[28], Some(0.0), "a real zero after the first send");
        assert_eq!(values[29], Some(0.0), "today has no sends yet");
        assert_eq!(values[0], None, "a gap before the first recorded send");
        // The chart labels are real dates: a day count treated as seconds
        // once rendered every label as 01 Jan 1970.
        let meta = crate::pages::shared::PageMeta::new(
            "Overview",
            "Overview",
            Vec::new(),
            now,
            "a temporary store",
            "csrf",
        );
        let html = render(&meta, &data, &crate::data::Transport::Sendmail);
        assert!(
            !html.contains("01 Jan 1970"),
            "labels must not collapse to the epoch"
        );
        let scope_year = apex_ui::format::date_short(now.saturating_mul(1000));
        let year = scope_year
            .rsplit(' ')
            .next()
            .unwrap_or_default()
            .to_string();
        assert!(
            html.contains(&year) && year.len() == 4,
            "the chart carries the current year ({year})"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
