//! `/sends`: the send history, today versus the cap, and the `.eml` outbox
//! with its preview links.

use apex_sales::store::Event;
use apex_ui::components::{data_table, notice, panel, Cell, EmptyState};
use apex_ui::format::{date_short, or_not_available, percent, utc_datetime};
use apex_ui::status::StatusKind;

use crate::data::{self, RequestData, Transport};
use crate::pages::shared::{self, PageMeta};

/// The most history rows shown at once.
pub const HISTORY_LIMIT: usize = 200;

/// Render `/sends`.
pub fn render(meta: &PageMeta, data: &RequestData, transport: &Transport) -> String {
    let body = format!(
        "{}\n{}\n{}\n{}\n{}",
        progress_block(data),
        history_table(data, transport),
        outbox_table(transport),
        "<p class=\"table-note\">History rows come from the durable Touch events; the event records the recipient, step, arm and message id, but not the transport, so the transport column shows this deployment's current transport instead.</p>",
        skip_notice(data)
    );
    shared::page(meta, &body)
}

fn skip_notice(data: &RequestData) -> String {
    if data.snap.skipped_lines == 0 {
        return String::new();
    }
    notice(
        StatusKind::Attention,
        "Some state lines could not be read",
        &format!(
            "{} unreadable lines were skipped while loading {}.",
            data.snap.skipped_lines,
            super::shared::store_label(&data.dir)
        ),
        None,
    )
}

/// Today's sends versus the current daily cap, as a native progress element
/// with its text equivalent.
fn progress_block(data: &RequestData) -> String {
    let sent = data.learned.health.sent_today();
    let cap = data.learned.health.total_cap();
    let ceiling = data.config.health.max_cap;
    let state = data.learned.health.summary_state().as_str();
    let domains = data.config.senders.len();
    let (value, max) = if cap == 0 { (0, 1) } else { (sent, cap) };
    let mut out = String::from("<section class=\"panel\"><h2 class=\"panel-heading\">Today versus the cap</h2><div class=\"panel-body\">");
    out.push_str(&format!(
        "<progress value=\"{value}\" max=\"{max}\">{sent} of {cap}</progress>"
    ));
    if domains <= 1 {
        out.push_str(&format!(
            "<p class=\"table-note\">Sent today: {sent} of today's cap {cap} (warm-up ceiling {ceiling}; domain {state}).</p>"
        ));
    } else {
        out.push_str(&format!(
            "<p class=\"table-note\">Sent today: {sent} of today's cap {cap} across {domains} sending domains (warm-up ceiling {ceiling} per domain; {state}).</p>"
        ));
    }
    if cap == 0 {
        out.push_str(&format!(
            "<p class=\"table-note\">{}</p>",
            apex_ui::escape::html_escape(&apex_ui::format::or_not_available(
                "the daily cap is zero while the domain cannot send"
            ))
        ));
    } else if sent >= cap {
        out.push_str("<p class=\"table-note\">Today's cap is fully spent; the next touch is planned for the next UTC day.</p>");
    } else {
        out.push_str(&format!(
            "<p class=\"table-note\">{sent} of {cap} used ({}) — the remainder may still be planned if the gate passes.</p>",
            percent(f64::from(sent) / f64::from(cap))
        ));
    }
    out.push_str("</div></section>");
    out
}

fn history_table(data: &RequestData, transport: &Transport) -> String {
    let mut sends: Vec<&Event> = data
        .snap
        .events
        .iter()
        .filter(|event| matches!(event, Event::Touch { .. }))
        .collect();
    let total = sends.len();
    sends.sort_by_key(|event| std::cmp::Reverse(event.at()));
    sends.truncate(HISTORY_LIMIT);
    let transport_name = data::transport_label(transport);
    let multi = data.config.senders.len() > 1;
    let rows: Vec<Vec<Cell>> = sends
        .iter()
        .map(|event| {
            let Event::Touch {
                at,
                email,
                publisher,
                from,
                step,
                arm,
                ..
            } = event
            else {
                return Vec::new();
            };
            let mut row = vec![
                Cell::Text(date_short(at * 1000)),
                Cell::Link(publisher.clone(), format!("/publishers/{publisher}")),
                Cell::Mono(email.clone()),
            ];
            if multi {
                row.push(match from {
                    Some(domain) => Cell::Mono(domain.clone()),
                    None => Cell::Muted(or_not_available("no domain recorded")),
                });
            }
            row.push(Cell::Num((step + 1).to_string()));
            row.push(Cell::Mono(arm.clone()));
            row.push(Cell::Text(transport_name.to_string()));
            row
        })
        .collect();
    let empty = EmptyState {
        reason: "No sends are recorded yet. In shadow mode the runner writes .eml previews instead of sending; in approval or autonomous mode a send becomes a Touch event.".to_string(),
        action: Some(("Open today's plan".to_string(), "/plan".to_string())),
    };
    let mut headers = vec!["Day (UTC)", "Publisher", "Contact"];
    if multi {
        headers.push("From");
    }
    headers.extend(["Step", "Arm", "Transport"]);
    data_table(
        Some("Send history"),
        &headers,
        &rows,
        Some(empty),
        Some(&shared::count_note(rows.len(), total, "sends")),
    )
}

fn outbox_table(transport: &Transport) -> String {
    match transport {
        Transport::Outbox(dir) => {
            let files = match data::outbox_files(dir) {
                Ok(files) => files,
                Err(reason) => {
                    return format!(
                        "{}\n",
                        notice(
                            StatusKind::Unavailable,
                            "The outbox directory could not be listed",
                            &format!("Not available ({reason})"),
                            None,
                        )
                    )
                }
            };
            let rows: Vec<Vec<Cell>> = files
                .iter()
                .map(|file| {
                    vec![
                        Cell::Link(
                            file.name.clone(),
                            format!("/sends/preview/{}", file.name),
                        ),
                        Cell::Num(file.size.to_string()),
                        match file.modified_ms {
                            Some(ms) => Cell::Text(utc_datetime(ms)),
                            None => Cell::Muted(or_not_available("no modification time")),
                        },
                    ]
                })
                .collect();
            let empty = EmptyState {
                reason: format!(
                    "No previews are written yet in {}. The runner writes one .eml per send attempt in this transport (apex-sales run).",
                    super::shared::store_label(dir)
                ),
                action: Some(("Open today's plan".to_string(), "/plan".to_string())),
            };
            data_table(
                Some("Outbox previews (.eml)"),
                &["File", "Size (bytes)", "Modified"],
                &rows,
                Some(empty),
                Some(&shared::count_note(rows.len(), files.len(), "preview files")),
            )
        }
        Transport::Sendmail => panel(
            "Outbox",
            "<p>This deployment sends through the local MTA's sendmail; there is no file outbox to preview. Set <code>APEX_SALES_OUTBOX</code> to a directory, or run in shadow or off mode, to collect .eml previews instead.</p>",
            None,
        ),
    }
}

/// Render the preview page for one outbox file.
pub fn preview(meta: &PageMeta, transport: &Transport, name: &str) -> String {
    let dir = match transport {
        Transport::Outbox(dir) => dir.clone(),
        Transport::Sendmail => {
            let body = notice(
                StatusKind::Unavailable,
                "No file outbox",
                "This deployment sends through sendmail, so there is nothing to preview.",
                Some(("Back to sends", "/sends")),
            );
            return shared::page(meta, &body);
        }
    };
    match data::read_outbox_file(&dir, name) {
        Ok(text) => {
            let body = format!(
                "<p class=\"table-note\">{} / {} bytes, shown exactly as written, escaped as text.</p>\n<pre>{}</pre>",
                apex_ui::escape::html_escape(name),
                text.len(),
                apex_ui::escape::html_escape(&text)
            );
            shared::page(meta, &body)
        }
        Err(reason) => {
            let body = format!(
                "{}\n{}",
                notice(
                    StatusKind::Unavailable,
                    "Preview not available",
                    &reason,
                    None
                ),
                apex_ui::components::action("Back to sends", "/sends", false)
            );
            shared::page(meta, &body)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 20_745 * apex_sales::model::DAY + 10 * 3600;

    #[test]
    fn preview_escapes_the_file_entirely() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-prev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("outbox")).unwrap();
        std::fs::write(
            dir.join("outbox/m1.eml"),
            "Subject: <script>alert(1)</script>\n\nbody & more",
        )
        .unwrap();
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        let _ = crate::data::load_from(&env, NOW).unwrap();
        let meta = PageMeta::new(
            "Preview m1.eml",
            "Sends",
            Vec::new(),
            NOW,
            super::shared::store_label(&dir).to_string(),
            "",
        );
        let html = preview(&meta, &Transport::Outbox(dir.join("outbox")), "m1.eml");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("body &amp; more"));
        let missing = preview(&meta, &Transport::Outbox(dir.join("outbox")), "nope.eml");
        assert!(missing.contains("Preview not available"));
        assert!(missing.contains("Not available (no outbox file named nope.eml)"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
