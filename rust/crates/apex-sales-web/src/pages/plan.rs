//! `/plan`: the engine's plan for today, the gate's refusals with every
//! reason, and the approval actions when the mode requires them.

use apex_sales::decision::{Autonomy, Verdict};
use apex_sales::engine::{plan, PlanItem};
use apex_ui::components::{csrf_hidden, data_table, notice, panel, Cell, EmptyState};
use apex_ui::format::{or_not_available, percent, utc_datetime};
use apex_ui::status::StatusKind;

use crate::data::{self, RequestData, Transport};
use crate::pages::shared::{self, PageMeta};
use crate::signing::random_hex_32;

/// Render `/plan`.
pub fn render(meta: &PageMeta, data: &RequestData, transport: &Transport) -> String {
    let items = plan(&data.snap, &data.learned, &data.config, data.now);
    // One lookup set for the whole page: a per-item scan of the approval
    // list would make the plan quadratic in items × approvals.
    let approvals: std::collections::HashSet<&str> =
        data.snap.approvals.iter().map(String::as_str).collect();
    let mode = mode_banner(data, transport);
    let body = format!(
        "{mode}\n{}{}{}{}{}{}",
        notices(data),
        planned_table(data, &items, &approvals),
        approvals_panel(meta, &items, &approvals),
        not_planned_table(&items),
        decisions_table(data),
        empty_explainer(&items)
    );
    shared::page(meta, &body)
}

fn mode_banner(data: &RequestData, transport: &Transport) -> String {
    let transport_now = data::transport_label(transport);
    match data.config.autonomy {
        Autonomy::Off => notice(
            StatusKind::Failure,
            "Kill switch is on",
            "APEX_SALES_MODE=off plans nothing and sends nothing through the web surface; inbound processing continues in the CLI.",
            None,
        ),
        Autonomy::Shadow => notice(
            StatusKind::Neutral,
            "Shadow mode: nothing is sent.",
            &format!(
                "Every touch that passes the gate becomes an .eml preview in the file outbox when the runner executes (apex-sales run); nothing leaves the machine. This deployment's transport is {transport_now}."
            ),
            Some(("Open the outbox and previews", "/sends")),
        ),
        Autonomy::Approval => notice(
            StatusKind::Attention,
            "Approval mode: operator approval is required",
            &format!(
                "A touch with the gate's approval verdict is sent by the runner on its schedule only after an operator approves it below. This deployment's transport is {transport_now}."
            ),
            Some(("Open the send history", "/sends")),
        ),
        Autonomy::Autonomous => notice(
            StatusKind::Neutral,
            "Autonomous mode",
            &format!(
                "Every touch that passes the gate is sent by the runner on its schedule (apex-sales run, hourly from its timer). This deployment's transport is {transport_now}."
            ),
            Some(("Open the send history", "/sends")),
        ),
    }
}

fn notices(data: &RequestData) -> String {
    if data.snap.skipped_lines == 0 {
        return String::new();
    }
    format!(
        "{}\n",
        notice(
            StatusKind::Attention,
            "Some state lines could not be read",
            &format!(
                "{} unreadable lines were skipped while loading {}; the engine's report prints the same count.",
                data.snap.skipped_lines,
                data.dir.display()
            ),
            None,
        )
    )
}

/// The verdict chip for a plan item.
pub fn verdict_cell(item: &PlanItem, approved: bool) -> Cell {
    match item.decision.verdict {
        Verdict::Send => Cell::Status("Send".to_string(), StatusKind::Verified),
        Verdict::Shadow => Cell::Status("Shadow: would send".to_string(), StatusKind::Neutral),
        Verdict::NeedsApproval => {
            if approved {
                Cell::Status("Approved".to_string(), StatusKind::Completed)
            } else {
                Cell::Status("Needs approval".to_string(), StatusKind::Attention)
            }
        }
        Verdict::Defer(until) => Cell::Status(
            format!("Deferred to {}", utc_datetime(until * 1000)),
            StatusKind::Queued,
        ),
        Verdict::Deny => Cell::Status("Not planned".to_string(), StatusKind::Failure),
    }
}

fn planned_table(
    data: &RequestData,
    items: &[PlanItem],
    approvals: &std::collections::HashSet<&str>,
) -> String {
    let planned: Vec<&PlanItem> = items
        .iter()
        .filter(|item| item.decision.verdict != Verdict::Deny)
        .collect();
    let multi = data.config.senders.len() > 1;
    let rows: Vec<Vec<Cell>> = planned
        .iter()
        .map(|item| {
            let approved = approvals.contains(item.touch_id.as_str());
            let mut row = vec![
                Cell::Mono(item.touch_id.clone()),
                Cell::Mono(item.email.clone()),
                Cell::Link(
                    item.publisher.clone(),
                    format!("/publishers/{}", item.publisher),
                ),
            ];
            if multi {
                row.push(match &item.sender {
                    Some(domain) => Cell::Mono(domain.clone()),
                    None => Cell::Muted(or_not_available("no sending domain")),
                });
            }
            row.push(Cell::Num((item.step + 1).to_string()));
            row.push(match &item.arm {
                Some(arm) => Cell::Mono(arm.clone()),
                None => Cell::Muted(or_not_available("no grounded variant")),
            });
            row.push(Cell::Num(format!("${:.2}", item.expected_value)));
            row.push(Cell::Num(percent(item.p_interest)));
            row.push(Cell::Text(utc_datetime(item.send_at * 1000)));
            row.push(verdict_cell(item, approved));
            row
        })
        .collect();
    let empty = EmptyState {
        reason: "No touch is planned right now: no contact is due, or every due contact is denied by the gate (the Not planned table below lists the reasons).".to_string(),
        action: None,
    };
    let mut headers = vec!["Touch", "Contact", "Publisher"];
    if multi {
        headers.push("From");
    }
    headers.extend([
        "Step",
        "Arm",
        "Expected value",
        "P(interest)",
        "Send at",
        "Verdict",
    ]);
    data_table(
        Some("Planned and deferred touches"),
        &headers,
        &rows,
        Some(empty),
        Some(&shared::count_note(
            planned.len(),
            items.len(),
            "plan items",
        )),
    )
}

/// The approve forms: one real form per touch waiting for operator approval,
/// below the table because a table cell never carries a form.
fn approvals_panel(
    meta: &PageMeta,
    items: &[PlanItem],
    approvals: &std::collections::HashSet<&str>,
) -> String {
    let waiting: Vec<&PlanItem> = items
        .iter()
        .filter(|item| {
            item.decision.verdict == Verdict::NeedsApproval
                && !approvals.contains(item.touch_id.as_str())
        })
        .collect();
    if waiting.is_empty() {
        return String::new();
    }
    let mut forms = String::new();
    for item in &waiting {
        forms.push_str(&format!(
            "<form method=\"post\" action=\"/actions/approve\">\n{}{}\n{}\n<p><strong>{}</strong> at {publisher} — step {step}, expected value ${ev:.2}. Send at {at} (UTC), local hour {hour}.</p>\n<p class=\"form-actions\"><button class=\"action action-primary\" type=\"submit\">Approve</button></p></form>\n",
            csrf_hidden(&meta.csrf),
            hidden_idem(),
            hidden_field("touch_id", &item.touch_id),
            apex_ui::escape::html_escape(&item.email),
            publisher = apex_ui::escape::html_escape(&item.publisher),
            step = item.step + 1,
            ev = item.expected_value,
            at = utc_datetime(item.send_at * 1000),
            hour = item.local_hour,
        ));
    }
    panel(
        "Approve planned touches",
        &format!(
            "{forms}<p class=\"table-note\">{} planned touches wait for operator approval. Approving writes the same durable approval the CLI's approve writes; a replayed form changes nothing.</p>",
            waiting.len()
        ),
        None,
    )
}

fn hidden_idem() -> String {
    format!(
        "<input type=\"hidden\" name=\"idempotency_key\" value=\"{}\">",
        random_hex_32()
    )
}

fn hidden_field(name: &str, value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
        apex_ui::escape::attr(name),
        apex_ui::escape::attr(value)
    )
}

fn not_planned_table(items: &[PlanItem]) -> String {
    let denied: Vec<&PlanItem> = items
        .iter()
        .filter(|item| item.decision.verdict == Verdict::Deny)
        .collect();
    let rows: Vec<Vec<Cell>> = denied
        .iter()
        .map(|item| {
            vec![
                Cell::Mono(item.email.clone()),
                Cell::Link(
                    item.publisher.clone(),
                    format!("/publishers/{}", item.publisher),
                ),
                Cell::Num((item.step + 1).to_string()),
                Cell::Text(item.decision.reasons.join("; ")),
            ]
        })
        .collect();
    let empty = EmptyState {
        reason: "No target is denied by the gate right now.".to_string(),
        action: None,
    };
    let note = if denied.is_empty() {
        "the gate denied nothing".to_string()
    } else {
        format!(
            "{} denied targets; every failure the gate recorded is listed per row",
            denied.len()
        )
    };
    data_table(
        Some("Not planned (every gate reason)"),
        &["Contact", "Publisher", "Step", "Reasons"],
        &rows,
        Some(empty),
        Some(&note),
    )
}

fn decisions_table(data: &RequestData) -> String {
    let (mut rows, skipped) = data::read_decisions(&data.dir);
    rows.reverse();
    rows.truncate(10);
    let table_rows: Vec<Vec<Cell>> = rows
        .iter()
        .map(|row| {
            vec![
                match row.at {
                    Some(at) => Cell::Text(utc_datetime(at * 1000)),
                    None => Cell::Muted(or_not_available("no timestamp")),
                },
                Cell::Mono(row.touch_id.clone()),
                Cell::Mono(row.email.clone()),
                Cell::Text(row.verdict.clone()),
                match row.expected_value {
                    Some(value) => Cell::Num(format!("${value:.2}")),
                    None => Cell::Muted(or_not_available("not recorded")),
                },
            ]
        })
        .collect();
    let empty = EmptyState {
        reason: "No gate decisions are recorded yet; the log fills when the runner sends (apex-sales run writes a decisions.jsonl line per send).".to_string(),
        action: None,
    };
    let note = if skipped > 0 {
        format!(
            "{skipped} unreadable line(s) skipped; the rows above are shown exactly as recorded"
        )
    } else {
        "the last recorded decisions, exactly as the send loop wrote them".to_string()
    };
    data_table(
        Some("Recent engine decisions"),
        &["At", "Touch", "Contact", "Verdict", "Expected value"],
        &table_rows,
        Some(empty),
        Some(&note),
    )
}

fn empty_explainer(items: &[PlanItem]) -> String {
    if !items.is_empty() {
        return String::new();
    }
    panel(
        "There is no plan yet",
        "<p>The plan is empty. The CLI produces the same plan this page renders, in this order:</p>\
         <p><code>apex-sales import-seeds seeds.csv</code> or <code>apex-sales import-leads leads.jsonl</code> to fill the store, then <code>apex-sales discover</code> to fetch public signals, then <code>apex-sales plan</code> prints the plan with every gate reason.</p>",
        Some("import the first leads"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_sales::model::DAY;

    const NOW: i64 = 20_742 * DAY + 8 * 3600;

    fn fixture(dir: &std::path::Path, mode: &str) -> RequestData {
        let store = apex_sales::store::StateDir::open(dir).unwrap();
        let mut publisher = apex_sales::model::Publisher::new("studio.example", "Studio");
        publisher.domain = Some("studio.example".into());
        publisher.country = Some("FI".into());
        publisher.apps.push(apex_sales::model::App {
            store: apex_sales::model::Store::AppStore,
            id: "1".into(),
            name: "Tile Quest".into(),
            genre: Some("Games".into()),
            rating_count: Some(9_000),
            free: Some(true),
            days_since_update: Some(5),
        });
        publisher.evidence.push(apex_sales::model::Evidence {
            id: "e1".into(),
            kind: apex_sales::model::EvidenceKind::AppAdsTxt,
            source: "s".into(),
            statement: "https://studio.example/app-ads.txt was not found".into(),
            observed_at: NOW - DAY,
        });
        publisher.ads_txt_missing = true;
        store.put_publisher(&publisher).unwrap();
        store
            .put_contact(&apex_sales::model::Contact {
                email: "maria@studio.example".into(),
                name: Some("Maria".into()),
                role: None,
                publisher_key: "studio.example".into(),
                kind: apex_sales::model::ContactKind::Named,
                source: "seed list".into(),
                consent: false,
            })
            .unwrap();
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        env.insert("APEX_SALES_MODE".to_string(), mode.to_string());
        env.insert("APEX_SALES_HOLDOUT".to_string(), "0".to_string());
        crate::data::load_from(&env, NOW).unwrap()
    }

    fn meta(dir: &std::path::Path, csrf: &str) -> PageMeta {
        PageMeta::new(
            "Plan",
            "Plan",
            Vec::new(),
            NOW,
            dir.display().to_string(),
            csrf,
        )
    }

    #[test]
    fn approval_mode_shows_the_approve_form_and_shadow_does_not() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = fixture(&dir, "approval");
        let html = render(&meta(&dir, &"a".repeat(32)), &data, &Transport::Sendmail);
        assert!(html.contains("Approval mode"));
        assert!(html.contains("action=\"/actions/approve\""));
        assert!(html.contains("Needs approval"));
        assert!(!html.contains("shadow previews need no approval"));
        let _ = std::fs::remove_dir_all(&dir);

        let dir = std::env::temp_dir().join(format!("apex-sales-web-plan2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = fixture(&dir, "shadow");
        let html = render(
            &meta(&dir, ""),
            &data,
            &Transport::Outbox(dir.join("outbox")),
        );
        assert!(html.contains("Shadow mode: nothing is sent."));
        assert!(!html.contains("action=\"/actions/approve\""));
        assert!(html.contains("Not planned (every gate reason)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autonomous_cap_produces_ranked_out_denials_with_reasons() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-plan3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // 30 publishers, cap 20: ten denials, each with the cap reason.
        let store = apex_sales::store::StateDir::open(&dir).unwrap();
        for i in 0..30 {
            let key = format!("studio{i}.example");
            let mut publisher = apex_sales::model::Publisher::new(&key, &key);
            publisher.domain = Some(key.clone());
            publisher.apps.push(apex_sales::model::App {
                store: apex_sales::model::Store::AppStore,
                id: format!("{i}"),
                name: format!("Game {i}"),
                genre: Some("Games".into()),
                rating_count: Some(10_000 + i),
                free: Some(true),
                days_since_update: Some(3),
            });
            publisher.evidence.push(apex_sales::model::Evidence {
                id: "e".into(),
                kind: apex_sales::model::EvidenceKind::AppAdsTxt,
                source: "s".into(),
                statement: "missing".into(),
                observed_at: NOW,
            });
            publisher.ads_txt_missing = true;
            store.put_publisher(&publisher).unwrap();
            store
                .put_contact(&apex_sales::model::Contact {
                    email: format!("ceo@{key}"),
                    name: None,
                    role: None,
                    publisher_key: key.clone(),
                    kind: apex_sales::model::ContactKind::Named,
                    source: "seed list".into(),
                    consent: false,
                })
                .unwrap();
        }
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        env.insert("APEX_SALES_MODE".to_string(), "autonomous".to_string());
        let data = crate::data::load_from(&env, NOW).unwrap();
        let html = render(&meta(&dir, ""), &data, &Transport::Sendmail);
        assert!(
            html.contains("ranked below today&#39;s cap")
                || html.contains("ranked below today's cap")
        );
        assert!(html.contains("denied targets"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_plan_explains_the_cli_command() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-plan4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        let data = crate::data::load_from(&env, NOW).unwrap();
        let html = render(&meta(&dir, ""), &data, &Transport::Sendmail);
        assert!(html.contains("There is no plan yet"));
        assert!(html.contains("apex-sales import-seeds"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The page's approval lookup is a set; it must answer exactly like the
    /// linear `Vec::contains` it replaced, over 10_000 recorded approvals and
    /// 10_000 probes (members, duplicates and near misses included).
    ///
    /// The pre-fix page compared `data.snap.approvals.contains(&item.touch_id)`
    /// per planned item (O(items x approvals)); this pins the replacement
    /// predicate to that exact answer set, and the measured before/after
    /// delta lives in the perf harness table.
    #[test]
    fn approval_set_lookup_matches_the_linear_scan_at_ten_thousand_entries() {
        use std::collections::HashSet;
        let approvals: Vec<String> = (0..10_000)
            .map(|i| apex_sales::engine::touch_id(&format!("lead{i}@p{i}.example"), 0, 20_745))
            .chain((0..5).map(|i| {
                // Duplicates are legal inputs and must not change answers.
                apex_sales::engine::touch_id(&format!("lead{i}@p{i}.example"), 0, 20_745)
            }))
            .collect();
        let set: HashSet<&str> = approvals.iter().map(String::as_str).collect();

        let probes: Vec<String> = (0..10_000)
            .map(|i| {
                if i % 2 == 0 {
                    apex_sales::engine::touch_id(&format!("lead{i}@p{i}.example"), 0, 20_745)
                } else {
                    apex_sales::engine::touch_id(
                        &format!("someone-else{i}@elsewhere{i}.example"),
                        0,
                        20_745,
                    )
                }
            })
            .collect();
        let mut members = 0;
        for probe in &probes {
            let from_set = set.contains(probe.as_str());
            let from_scan = approvals.contains(probe);
            assert_eq!(from_set, from_scan, "{probe}");
            members += usize::from(from_scan);
        }
        assert_eq!(members, 5_000, "the probes really hit both outcomes");
    }
}
