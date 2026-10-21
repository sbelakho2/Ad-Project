//! Customers `/control/customers` and `/control/customers/{id}`
//! (pages 57 and 58): the account inventory with server-side filters, the
//! customer detail with its KPIs, the merged timeline and the context tabs.

use apex_analytics::queries::ObjectTimelineEntry;
use apex_ui::components::{action, data_table, field, input, notice, select, Cell, EmptyState};
use apex_ui::escape::{attr, html_escape};
use apex_ui::status::{chip, StatusKind};

use crate::copy;
use crate::data::{DurableExperiment, InvoiceRecord, VerifiedRecordView};
use crate::fixtures::CustomerRow;

use super::{action_form, document, header, kpis, ActionFormSpec, PageMeta};

/// The filter state of the customer list (all values are URL parameters).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomerFilters {
    /// Lifecycle stage filter.
    pub lifecycle: Option<String>,
    /// Platform filter.
    pub platform: Option<String>,
    /// Health filter.
    pub health: Option<String>,
    /// Owner filter.
    pub owner: Option<String>,
    /// Publisher search (id or name substring, case-insensitive).
    pub search: Option<String>,
}

impl CustomerFilters {
    /// Parse from query pairs.
    pub fn from_query(pairs: &[(String, String)]) -> Self {
        let get = |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Self {
            lifecycle: get("lifecycle"),
            platform: get("platform"),
            health: get("health"),
            owner: get("owner"),
            search: get("search"),
        }
    }

    /// Whether any filter is set.
    pub fn is_filtered(&self) -> bool {
        self.lifecycle.is_some()
            || self.platform.is_some()
            || self.health.is_some()
            || self.owner.is_some()
            || self.search.is_some()
    }

    /// The filters as visible query-string pairs.
    pub fn query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for (name, value) in [
            ("lifecycle", &self.lifecycle),
            ("platform", &self.platform),
            ("health", &self.health),
            ("owner", &self.owner),
            ("search", &self.search),
        ] {
            if let Some(value) = value {
                pairs.push((name.to_string(), value.clone()));
            }
        }
        pairs
    }

    /// Apply the filters.
    pub fn apply(&self, rows: &[CustomerRow]) -> Vec<CustomerRow> {
        rows.iter()
            .filter(|row| {
                self.lifecycle
                    .as_deref()
                    .is_none_or(|value| row.lifecycle == value)
            })
            .filter(|row| {
                self.platform.as_deref().is_none_or(|value| {
                    row.platform_readiness.iter().any(|line| {
                        line.to_ascii_lowercase()
                            .contains(&value.to_ascii_lowercase())
                    })
                })
            })
            .filter(|row| {
                self.health
                    .as_deref()
                    .is_none_or(|value| row.health.eq_ignore_ascii_case(value))
            })
            .filter(|row| {
                self.owner
                    .as_deref()
                    .is_none_or(|value| row.owner.as_deref() == Some(value))
            })
            .filter(|row| {
                self.search.as_deref().is_none_or(|value| {
                    let value = value.to_ascii_lowercase();
                    row.publisher_id.to_ascii_lowercase().contains(&value)
                        || row.display_name.to_ascii_lowercase().contains(&value)
                        || row.account_id.to_ascii_lowercase().contains(&value)
                })
            })
            .cloned()
            .collect()
    }
}

/// Render the customer list.
pub fn render_list(
    meta: &PageMeta,
    rows: Result<Vec<CustomerRow>, String>,
    filters: &CustomerFilters,
) -> String {
    let mut body = header(meta, "Customers");
    match rows {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The account inventory is not readable: {reason}"),
                Some((copy::TELEMETRY_ABSENT_ACTION, "/control/system")),
            ));
        }
        Ok(rows) => {
            let filtered = filters.apply(&rows);
            body.push_str(&render_filters(&rows, filters));
            body.push_str(&format!(
                "<p class=\"table-note\">{} of {} {} {} the saved filters.</p>",
                filtered.len(),
                rows.len(),
                if rows.len() == 1 {
                    "customer"
                } else {
                    "customers"
                },
                if rows.len() == 1 { "matches" } else { "match" }
            ));
            let cells: Vec<Vec<Cell>> = filtered
                .iter()
                .map(|row| {
                    vec![
                        Cell::Link(
                            row.display_name.clone(),
                            format!("/control/customers/{}", row.account_id),
                        ),
                        Cell::Mono(row.publisher_id.clone()),
                        Cell::Text(row.lifecycle.replace('_', " ")),
                        Cell::Text(row.platform_readiness.join("; ")),
                        Cell::Status(row.health.clone(), health_kind(&row.health)),
                        Cell::Text(
                            row.owner
                                .clone()
                                .unwrap_or_else(|| "Unassigned".to_string()),
                        ),
                        Cell::Num(format!("{}d", row.stage_age_days)),
                        Cell::Muted(row.contact_masked.clone().unwrap_or_else(|| {
                            apex_ui::format::or_not_available("no contact source is configured")
                        })),
                    ]
                })
                .collect();
            body.push_str(&data_table(
                Some("Account inventory"),
                &[
                    "Customer",
                    "Publisher id",
                    "Lifecycle",
                    "Platform readiness",
                    "Health",
                    "Owner",
                    "Stage age",
                    "Contact",
                ],
                &cells,
                Some(EmptyState {
                    reason: copy::EMPTY_CUSTOMERS.to_string(),
                    action: Some((
                        copy::EMPTY_CUSTOMERS_ACTION.to_string(),
                        "/control/customers".to_string(),
                    )),
                }),
                None,
            ));
            body.push_str(&format!(
                "<p class=\"table-note\">{}</p>",
                html_escape(
                    "Broad views mask contacts. Opening a customer does not impersonate them."
                )
            ));
        }
    }
    document(meta, "Customers", &["Customers"], &body)
}

fn health_kind(health: &str) -> StatusKind {
    match health.to_ascii_lowercase().as_str() {
        "restore failed" => StatusKind::Failure,
        "proof hold" | "waiting for access" => StatusKind::Attention,
        "ready for review" => StatusKind::Verified,
        _ => StatusKind::Neutral,
    }
}

fn render_filters(all: &[CustomerRow], filters: &CustomerFilters) -> String {
    let mut lifecycles: Vec<String> = all.iter().map(|row| row.lifecycle.clone()).collect();
    lifecycles.sort();
    lifecycles.dedup();
    let mut owners: Vec<String> = all.iter().filter_map(|row| row.owner.clone()).collect();
    owners.sort();
    owners.dedup();
    let options = |values: &[String]| -> Vec<(String, String)> {
        let mut out = vec![(String::new(), "All".to_string())];
        out.extend(
            values
                .iter()
                .map(|value| (value.clone(), value.replace('_', " "))),
        );
        out
    };
    let platforms: Vec<String> = vec![
        "applovin_max".to_string(),
        "levelplay".to_string(),
        "admob".to_string(),
    ];
    let healths: Vec<String> = vec![
        "Restore failed".to_string(),
        "Proof hold".to_string(),
        "Waiting for access".to_string(),
        "Ready for review".to_string(),
    ];
    format!(
        "<form class=\"filter-bar\" method=\"get\" action=\"/control/customers\">\
         {}\
         {}\
         {}\
         {}\
         <div class=\"filter-field\"><label class=\"field-label\" for=\"f-search\">Search</label>{}</div>\
         <button class=\"action action-secondary\" type=\"submit\">Apply filters</button>\
         <a class=\"filter-clear\" href=\"/control/customers\">Clear filters</a></form>",
        field(
            "f-lifecycle",
            "Lifecycle",
            None,
            None,
            &select("f-lifecycle", "lifecycle", &options(&lifecycles), filters.lifecycle.as_deref()),
        ),
        field(
            "f-platform",
            "Platform",
            None,
            None,
            &select("f-platform", "platform", &options(&platforms), filters.platform.as_deref()),
        ),
        field(
            "f-health",
            "Health",
            None,
            None,
            &select("f-health", "health", &options(&healths), filters.health.as_deref()),
        ),
        field(
            "f-owner",
            "Owner",
            None,
            None,
            &select("f-owner", "owner", &options(&owners), filters.owner.as_deref()),
        ),
        input("f-search", "search", "search", filters.search.as_deref(), false, None),
    )
}

/// The customer detail tabs (the scope is preserved: the account id and the
/// current filters travel with every tab link).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    /// The summary.
    Summary,
    /// Platform connections and credential state.
    Connections,
    /// Audit history for this account.
    Audits,
    /// Experiments for this publisher.
    Experiments,
    /// Fee and invoice state.
    Finance,
    /// The full activity timeline.
    Activity,
}

impl DetailTab {
    /// Every tab in order.
    pub const ALL: [DetailTab; 6] = [
        DetailTab::Summary,
        DetailTab::Connections,
        DetailTab::Audits,
        DetailTab::Experiments,
        DetailTab::Finance,
        DetailTab::Activity,
    ];

    /// The URL parameter value.
    pub fn slug(self) -> &'static str {
        match self {
            DetailTab::Summary => "summary",
            DetailTab::Connections => "connections",
            DetailTab::Audits => "audits",
            DetailTab::Experiments => "experiments",
            DetailTab::Finance => "finance",
            DetailTab::Activity => "activity",
        }
    }

    /// The display label.
    pub fn label(self) -> &'static str {
        match self {
            DetailTab::Summary => "Summary",
            DetailTab::Connections => "Connections",
            DetailTab::Audits => "Audits",
            DetailTab::Experiments => "Experiments",
            DetailTab::Finance => "Finance",
            DetailTab::Activity => "Activity",
        }
    }

    /// Parse a tab parameter.
    pub fn parse(value: Option<&str>) -> Self {
        DetailTab::ALL
            .into_iter()
            .find(|tab| Some(tab.slug()) == value)
            .unwrap_or(DetailTab::Summary)
    }
}

/// The customer detail data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerDetail {
    /// The customer row.
    pub customer: CustomerRow,
    /// The merged timeline by occurrence time.
    pub timeline: Result<Vec<ObjectTimelineEntry>, String>,
    /// Durable experiments for this publisher.
    pub experiments: Result<Vec<DurableExperiment>, String>,
    /// Pre-filled action forms the signed-in role may submit.
    pub forms: Vec<ActionFormSpec>,
    /// Verified ledger records, when a live ledger is the source.
    pub verified_records: Option<Result<Vec<VerifiedRecordView>, String>>,
    /// Invoice register rows for this publisher, when live billing is read.
    pub invoices: Option<Result<Vec<InvoiceRecord>, String>>,
}

/// Render the customer detail.
pub fn render_detail(
    meta: &PageMeta,
    detail: &CustomerDetail,
    tab: DetailTab,
    filters: &CustomerFilters,
) -> String {
    let row = &detail.customer;
    let mut body = header(meta, &row.display_name);
    body.push_str(&format!(
        "<p class=\"panel-body\">{} <code>{}</code> / Lifecycle: {} / {} stage age {}\
         <br>Customer since the audit request; owner and next step are named, and the source is the durable lifecycle record.</p>",
        chip(&row.health, health_kind(&row.health)),
        html_escape(&row.publisher_id),
        html_escape(&row.lifecycle.replace('_', " ")),
        html_escape(
            &row.owner
                .clone()
                .unwrap_or_else(|| "Unassigned".to_string())
        ),
        row.stage_age_days
    ));
    body.push_str(&kpis(&[
        (
            "Data through",
            detail
                .timeline
                .as_ref()
                .ok()
                .and_then(|entries| entries.last())
                .and_then(|entry| entry.occurred_at_utc.get(0..10))
                .unwrap_or("Not available"),
            "Last lifecycle occurrence",
            None,
        ),
        (
            "Active work",
            &row.running_tests.to_string(),
            "Experiments or checks in flight",
            Some("/control/experiments"),
        ),
        (
            "Fee state",
            &row.billing_state,
            "Billing and proof state",
            Some("/control/finance"),
        ),
    ]));
    body.push_str(&tabs(&row.account_id, tab, filters));
    let panel_body = match tab {
        DetailTab::Summary => render_summary(detail),
        DetailTab::Connections => render_connections(row, &detail.forms),
        DetailTab::Audits => render_audits(detail),
        DetailTab::Experiments => render_experiments(detail),
        DetailTab::Finance => render_finance(row, &detail.forms, detail.invoices.as_ref()),
        DetailTab::Activity => render_timeline(detail),
    };
    body.push_str(&panel_body);
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape("No silent impersonation: this page never opens a customer session and renders no customer credential.")
    ));
    document(
        meta,
        &row.display_name,
        &["Customers", &row.display_name],
        &body,
    )
}

fn tabs(account_id: &str, current: DetailTab, filters: &CustomerFilters) -> String {
    let mut out = String::from("<nav class=\"tabs\" aria-label=\"Customer context\">");
    let filter_suffix = {
        let pairs = filters.query_pairs();
        if pairs.is_empty() {
            String::new()
        } else {
            format!(
                "&{}",
                pairs
                    .iter()
                    .map(|(key, value)| format!("{}={}", attr(key), attr(value)))
                    .collect::<Vec<_>>()
                    .join("&")
            )
        }
    };
    for tab in DetailTab::ALL {
        let current_attr = if tab == current {
            " aria-current=\"page\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<a class=\"tab\" href=\"/control/customers/{}?tab={}{}\"{}>{}</a>",
            attr(account_id),
            tab.slug(),
            filter_suffix,
            current_attr,
            html_escape(tab.label())
        ));
    }
    out.push_str("</nav>");
    out
}

fn render_summary(detail: &CustomerDetail) -> String {
    let row = &detail.customer;
    let mut out = String::from("<section class=\"panel\"><h2 class=\"panel-heading\">Summary</h2>");
    out.push_str(&format!(
        "<p class=\"panel-body\">Last activity: {}. Billing: {}. Contact: {}.</p>",
        html_escape(&row.last_activity),
        html_escape(&row.billing_state),
        html_escape(
            row.contact_masked
                .as_deref()
                .unwrap_or("not available (no contact source is configured)")
        )
    ));
    if let Some(records) = &detail.verified_records {
        out.push_str(&render_verified_records(records));
    }
    out.push_str("</section>");
    out.push_str(&render_timeline(detail));
    out
}

fn render_verified_records(records: &Result<Vec<VerifiedRecordView>, String>) -> String {
    match records {
        Err(reason) => notice(
            StatusKind::Unavailable,
            copy::TELEMETRY_ABSENT,
            &format!("The verified ledger records are not readable: {reason}"),
            None,
        ),
        Ok(records) => {
            let cells: Vec<Vec<Cell>> = records
                .iter()
                .rev()
                .take(100)
                .map(|record| {
                    vec![
                        Cell::Num(record.seq.to_string()),
                        Cell::Text(apex_ui::format::utc_datetime(record.at_ms)),
                        Cell::Text(record.kind.clone()),
                        Cell::Text(
                            record
                                .platform
                                .clone()
                                .unwrap_or_else(|| "not platform-labelled".to_string()),
                        ),
                        Cell::Muted(record.detail.clone()),
                    ]
                })
                .collect();
            data_table(
                Some("Verified ledger records (newest first, bounded to 100)"),
                &["Seq", "When (UTC)", "Kind", "Platform", "Detail"],
                &cells,
                Some(EmptyState {
                    reason: "The ledger verified but holds no record.".to_string(),
                    action: None,
                }),
                Some("Every row passed chain and signature verification against the configured publisher key."),
            )
        }
    }
}

fn forms_for<'a>(forms: &'a [ActionFormSpec], kind: &str) -> Vec<&'a ActionFormSpec> {
    forms
        .iter()
        .filter(|form| form.action.ends_with(&format!("/{kind}")))
        .collect()
}

fn render_connections(row: &CustomerRow, forms: &[ActionFormSpec]) -> String {
    let cells: Vec<Vec<Cell>> = row
        .platform_readiness
        .iter()
        .map(|line| {
            let (platform, state) = line.split_once(':').unwrap_or((line.as_str(), ""));
            vec![
                Cell::Text(platform.trim().to_string()),
                Cell::Status(
                    state.trim().to_string(),
                    if state.to_ascii_lowercase().contains("missing") {
                        StatusKind::Failure
                    } else {
                        StatusKind::Verified
                    },
                ),
                Cell::Muted("Credential values are never rendered here.".to_string()),
            ]
        })
        .collect();
    let mut body = data_table(
        None,
        &["Platform", "Connection state", "Note"],
        &cells,
        None,
        Some("Per-platform connection state from the lifecycle records."),
    );
    for form in forms_for(forms, "request_reconnection") {
        body.push_str(&format!(
            "<div class=\"panel\"><h3 class=\"panel-heading\">Request reconnection</h3>{}</div>",
            action_form(form)
        ));
    }
    format!("<section class=\"panel\"><h2 class=\"panel-heading\">Connections</h2>{body}</section>")
}

fn render_audits(detail: &CustomerDetail) -> String {
    let done = detail
        .timeline
        .as_ref()
        .map(|entries| {
            entries
                .iter()
                .filter(|entry| {
                    matches!(
                        entry.event_type,
                        apex_analytics::EventType::AuditCompleted
                            | apex_analytics::EventType::AuditFailed
                    )
                })
                .count()
        })
        .unwrap_or(0);
    format!(
        "<section class=\"panel\"><h2 class=\"panel-heading\">Audits</h2>\
         <p class=\"panel-body\">{} recorded on this account's timeline. A rule that could not run is not a clean check: the audits page lists eligibility per rule.</p>\
         <p class=\"panel-body\">{}</p></section>",
        super::count(done, "audit run event", "audit run events"),
        action("Open audits", "/control/audits", false)
    )
}

fn render_experiments(detail: &CustomerDetail) -> String {
    match &detail.experiments {
        Err(reason) => notice(
            StatusKind::Unavailable,
            copy::TELEMETRY_ABSENT,
            &format!("The durable experiment records are not readable: {reason}"),
            None,
        ),
        Ok(experiments) => {
            let cells: Vec<Vec<Cell>> = experiments
                .iter()
                .map(|experiment| {
                    vec![
                        Cell::Link(
                            experiment.experiment_id.clone(),
                            format!("/control/experiments/{}", experiment.experiment_id),
                        ),
                        Cell::Text(experiment.change_description.clone().unwrap_or_else(|| {
                            apex_ui::format::or_not_available("no registered description")
                        })),
                        Cell::Status(
                            experiment
                                .state
                                .clone()
                                .unwrap_or_else(|| "State unknown".to_string()),
                            StatusKind::Neutral,
                        ),
                    ]
                })
                .collect();
            data_table(
                Some("Experiments (durable runner records)"),
                &["Experiment", "Registered change", "Platform state"],
                &cells,
                Some(EmptyState {
                    reason: "No experiments are recorded for this publisher.".to_string(),
                    action: None,
                }),
                None,
            )
        }
    }
}

fn render_finance(
    row: &CustomerRow,
    forms: &[ActionFormSpec],
    invoices: Option<&Result<Vec<InvoiceRecord>, String>>,
) -> String {
    let mut body = format!(
        "<p class=\"panel-body\">Billing state: {}. A fee on proof hold authorizes no collection.</p>\
         <p class=\"panel-body\">{}</p>",
        html_escape(&row.billing_state),
        action("Open finance", "/control/finance", false)
    );
    if let Some(invoices) = invoices {
        match invoices {
            Err(reason) => {
                body.push_str(&notice(
                    StatusKind::Unavailable,
                    copy::TELEMETRY_ABSENT,
                    &format!("The invoice register is not readable: {reason}"),
                    None,
                ));
            }
            Ok(rows) => {
                let cells: Vec<Vec<Cell>> = rows
                    .iter()
                    .map(|invoice| {
                        vec![
                            Cell::Link(
                                invoice.id(),
                                format!("/control/finance/invoices/{}", invoice.id()),
                            ),
                            Cell::Text(invoice.period.clone()),
                            Cell::Num(apex_ui::format::money(invoice.total_micros, "USD")),
                            Cell::Num(apex_ui::format::money(invoice.gross_micros, "USD")),
                            Cell::Num(invoice.line_items.len().to_string()),
                        ]
                    })
                    .collect();
                body.push_str(&data_table(
                    Some("Invoice register rows for this publisher"),
                    &["Invoice", "Period", "Net", "Gross", "Lines"],
                    &cells,
                    Some(EmptyState {
                        reason: "No invoice record names this publisher.".to_string(),
                        action: None,
                    }),
                    None,
                ));
            }
        }
    }
    for form in forms_for(forms, "managed_plan_activate") {
        body.push_str(&format!(
            "<div class=\"panel\"><h3 class=\"panel-heading\">Activate managed ad ops</h3>\
             <p class=\"panel-body\">Writes the plan state the runner bills from: {}.</p>{}</div>",
            html_escape(&form.target),
            action_form(form)
        ));
    }
    format!("<section class=\"panel\"><h2 class=\"panel-heading\">Finance</h2>{body}</section>")
}

fn render_timeline(detail: &CustomerDetail) -> String {
    match &detail.timeline {
        Err(reason) => notice(
            StatusKind::Unavailable,
            copy::TELEMETRY_ABSENT,
            &format!("The lifecycle timeline is not readable: {reason}"),
            None,
        ),
        Ok(entries) if entries.is_empty() => notice(
            StatusKind::Neutral,
            "No lifecycle occurrences are recorded for this account yet.",
            "The durable lifecycle record holds the account's typed events once they occur.",
            None,
        ),
        Ok(entries) => {
            let cells: Vec<Vec<Cell>> = entries
                .iter()
                .map(|entry| {
                    vec![
                        Cell::Text(entry.occurred_at_utc.clone()),
                        Cell::Text(entry.event_type.as_str().replace('_', " ")),
                        Cell::Text(
                            entry
                                .outcome
                                .clone()
                                .unwrap_or_else(|| "recorded".to_string()),
                        ),
                        Cell::Text(entry.reason_code.clone().unwrap_or_else(|| "-".to_string())),
                        Cell::Muted(entry.received_at_utc.clone()),
                    ]
                })
                .collect();
            data_table(
                Some("Timeline (merged by occurrence time; received time available)"),
                &[
                    "Occurred (UTC)",
                    "Event",
                    "Outcome",
                    "Reason",
                    "Received (UTC)",
                ],
                &cells,
                None,
                Some("Typed lifecycle events in one chronology."),
            )
        }
    }
}
