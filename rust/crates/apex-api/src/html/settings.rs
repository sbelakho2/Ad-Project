//! `/console/settings`, `/console/settings/access`, `/console/settings/policy`,
//! `/console/settings/monitoring` and `/console/settings/billing`.
//!
//! The settings surfaces state authority instead of hiding it: account facts
//! come from explicit deployment configuration, the policy values from the
//! recorded policy file and the newest signed pre-registration, the session
//! list from the live session store, and reporting health from observed
//! snapshots and connection records. Plan facts come from the recorded plan
//! state files the runner bills from. Mutations exist only where they are
//! real: the theme preference, session revocation and plan cancellation;
//! everything else is read-only with the signed-order note.

use apex_ui::components::{
    action, action_button, csrf_hidden, data_table, notice, panel, Cell, EmptyState,
};
use apex_ui::format::{date_short, money_fixed, or_not_available, percent, utc_datetime};
use apex_ui::status::StatusKind;
use apex_ui::theme::{theme_form, Theme};

use crate::billing_view::{ManagedView, MonitoringView};
use crate::connections::CredentialState;
use crate::settings::{AccountView, MonitoringRow, PolicyView};

use super::{checkbox_with_value, console_document, html_escape, ConsoleCtx};

/// The frozen session note on the access page.
pub const TOKEN_NOTE: &str = "The ledger access token is never stored in the browser. The session \
                             cookie carries only an opaque id, and the session ends after 15 \
                             minutes of inactivity, when the workspace restarts, or when you \
                             sign out.";
/// The frozen identity boundary note.
pub const IDENTITY_NOTE: &str = "A customer account owner is never an operator. Team invitations \
                                 and granular roles need a new identity system; they are not \
                                 offered here.";
/// Frozen policy note.
pub const POLICY_NOTE: &str = "The signed order is the authority: a UI preference cannot widen a \
                               stop condition or bypass an approval.";
/// The monitoring ownership note.
pub const MONITORING_NOTE: &str = "Monitoring is included at connection; you can cancel any \
                                   time and the plan bills the current month only.";
/// The managed plan authority note.
pub const MANAGED_NOTE: &str = "The written scope is part of the plan, not an estimate. The \
                                account team records the plan; you can cancel it here.";

/// The account page data.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountPageData<'a> {
    /// The account view.
    pub account: &'a AccountView,
    /// Read time of the page.
    pub as_of_unix_ms: i64,
    /// The active theme.
    pub theme: Theme,
    /// The CSRF token when a console session exists.
    pub csrf: Option<&'a str>,
    /// A validation refusal for the theme form.
    pub theme_error: Option<&'a str>,
}

/// The access page data.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessPageData<'a> {
    /// Workspace publisher.
    pub publisher: &'a str,
    /// Sessions of the viewing session's publisher, newest first.
    pub sessions: Vec<crate::session::SessionSummary>,
    /// The handle of the viewing session itself.
    pub current_handle: Option<&'a str>,
    /// The CSRF token when a console session exists.
    pub csrf: Option<&'a str>,
    /// Read time of the page.
    pub as_of_unix_ms: i64,
}

/// The policy page data.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyPageData<'a> {
    /// Workspace publisher.
    pub publisher: &'a str,
    /// The recorded policy, when the deployment records one.
    pub policy: Option<&'a apex_autopilot::model::AutopilotPolicy>,
    /// Whether the recorded policy holds proposals for approval right now.
    pub approval_required_now: Option<bool>,
    /// The newest signed registration.
    pub view: &'a PolicyView,
    /// Read time of the page.
    pub as_of_unix_ms: i64,
}

/// The monitoring page data.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitoringPageData<'a> {
    /// Workspace publisher.
    pub publisher: &'a str,
    /// The billing monitoring plan.
    pub plan: &'a MonitoringView,
    /// Per-platform reporting rows.
    pub rows: Vec<MonitoringRow>,
    /// Whether the runner's alert log is configured for the console.
    pub alert_log_configured: bool,
    /// Read time of the page.
    pub as_of_unix_ms: i64,
    /// The CSRF token when a console session exists.
    pub csrf: Option<&'a str>,
}

/// The billing settings page data.
#[derive(Debug, Clone, PartialEq)]
pub struct BillingSettingsPageData<'a> {
    /// Workspace publisher.
    pub publisher: &'a str,
    /// The managed ad-ops plan.
    pub managed: &'a ManagedView,
    /// Read time of the page.
    pub as_of_unix_ms: i64,
    /// The CSRF token when a console session exists.
    pub csrf: Option<&'a str>,
}

/// Render the account settings page.
pub fn render_account(ctx: &ConsoleCtx, data: &AccountPageData<'_>) -> String {
    let account = data.account;
    let mut body = String::from("<h1>Settings</h1>");
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "Effective from {}. Ask your account team to change these values.",
            date_short(data.as_of_unix_ms)
        ))
    ));
    body.push_str(&notice(
        StatusKind::ReadOnly,
        "Read-only account facts.",
        "Changing the display currency never converts historical source amounts silently.",
        None,
    ));
    let rows =
        vec![
            vec![
                Cell::Text("Workspace".to_string()),
                Cell::Text(account.workspace.clone()),
            ],
            vec![
                Cell::Text("Contact".to_string()),
                Cell::Text(account.contact.clone().unwrap_or_else(|| {
                    or_not_available("no contact is recorded for this workspace")
                })),
            ],
            vec![
                Cell::Text("Timezone label (timestamps are UTC)".to_string()),
                Cell::Text(account.timezone.clone()),
            ],
            vec![
                Cell::Text("Reporting currency".to_string()),
                Cell::Text(account.currency.clone()),
            ],
            vec![
                Cell::Text("Retention".to_string()),
                Cell::Text(match account.retention_months {
                    Some(months) => format!("{months} months of signed records"),
                    None => or_not_available("no retention window is configured"),
                }),
            ],
            vec![
                Cell::Text("Platform sessions".to_string()),
                Cell::Text(
                    "Bound to credentials: revoking a connection ends the platform sessions the \
                 runner holds"
                        .to_string(),
                ),
            ],
        ];
    body.push_str(&data_table(
        Some("Account"),
        &["Attribute", "Value"],
        &rows,
        None,
        None,
    ));
    body.push_str(&theme_panel(data));
    body.push_str(&setting_links());
    console_document(ctx, "Settings", "Settings", &body)
}

fn theme_panel(data: &AccountPageData<'_>) -> String {
    let mut body = String::new();
    if let Some(error) = data.theme_error {
        body.push_str(&notice(StatusKind::Failure, error, "", None));
    }
    match data.csrf {
        Some(csrf) => {
            body.push_str(&theme_form_with_csrf(
                "/console/settings/theme",
                data.theme,
                csrf,
            ));
        }
        None => {
            body.push_str(&format!(
                "<p>{}</p>",
                html_escape(
                    "Open the change log with your access token to start a console session; \
                     the theme preference is saved with that session."
                )
            ));
            body.push_str(&action("Open the change log", "/console/ledger", false));
        }
    }
    panel("Display preference", &body, None)
}

/// The frozen theme form carries no CSRF field (the shell cookie is the only
/// state), so the settings mutant adds the session's CSRF token right after
/// the opening tag. The insertion point is the deterministic first tag of the
/// frozen component; a failure to insert degrades to appending before
/// `</form>` so the form always carries the token.
pub fn theme_form_with_csrf(action: &str, current: Theme, csrf: &str) -> String {
    let rendered = theme_form(action, current);
    let hidden = csrf_hidden(csrf);
    let mutated = match rendered.find('>') {
        Some(position) => format!(
            "{}{hidden}{}",
            &rendered[..=position],
            &rendered[position + 1..]
        ),
        None => format!("{rendered}{hidden}"),
    };
    mutated
}

fn setting_links() -> String {
    let rows = vec![
        vec![
            Cell::Text("Access".to_string()),
            Cell::Text("Session status, recent access and a sign-out action.".to_string()),
            Cell::Link(
                "Open access".to_string(),
                "/console/settings/access".to_string(),
            ),
        ],
        vec![
            Cell::Text("Operating policy".to_string()),
            Cell::Text("Approval mode, caps, burn-in, window and guardrail limits.".to_string()),
            Cell::Link(
                "Open policy".to_string(),
                "/console/settings/policy".to_string(),
            ),
        ],
        vec![
            Cell::Text("Monitoring".to_string()),
            Cell::Text("Reporting health per platform and the monitoring plan.".to_string()),
            Cell::Link(
                "Open monitoring".to_string(),
                "/console/settings/monitoring".to_string(),
            ),
        ],
        vec![
            Cell::Text("Billing".to_string()),
            Cell::Text("The managed ad-ops line, its written scope and cancellation.".to_string()),
            Cell::Link(
                "Open billing".to_string(),
                "/console/settings/billing".to_string(),
            ),
        ],
    ];
    data_table(
        Some("Settings sections"),
        &["Section", "Purpose", "Action"],
        &rows,
        None,
        None,
    )
}

/// Render the access settings page.
pub fn render_access(ctx: &ConsoleCtx, data: &AccessPageData<'_>) -> String {
    let mut body = String::from("<h1>Access</h1>");
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "{} / as of {}",
            data.publisher,
            utc_datetime(data.as_of_unix_ms)
        ))
    ));
    body.push_str(&notice(
        StatusKind::Neutral,
        IDENTITY_NOTE,
        TOKEN_NOTE,
        None,
    ));
    body.push_str(&workspace_key_table(data));
    if data.sessions.is_empty() {
        body.push_str(&panel(
            "Sessions",
            &format!(
                "<p>{}</p>",
                html_escape(
                    "No live session is recorded. Open the change log with your access token to \
                     start one."
                )
            ),
            None,
        ));
        body.push_str(&action("Open the change log", "/console/ledger", true));
        return console_document(ctx, "Access", "Settings", &body);
    }
    let rows: Vec<Vec<Cell>> = data
        .sessions
        .iter()
        .map(|session| {
            let is_current = data.current_handle == Some(session.handle.as_str());
            vec![
                Cell::Mono(session.handle.clone()),
                Cell::Text(format!(
                    "{}{}",
                    utc_datetime(session.created_ms),
                    if is_current { " (this session)" } else { "" }
                )),
                Cell::Text(utc_datetime(session.last_seen_ms)),
                Cell::Text(if is_current {
                    "Current session".to_string()
                } else {
                    "Live".to_string()
                }),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Active console sessions"),
        &["Session", "Opened", "Last seen", "State"],
        &rows,
        None,
        Some("Session ids are never shown; the handle is a separate public identifier."),
    ));
    body.push_str(&session_actions(data));
    console_document(ctx, "Access", "Settings", &body)
}

/// The exact workspace key this session reads, so an operator can provision
/// the runner's ledger under it. The value is the session's publisher — the
/// same key every page of this workspace is scoped to, not a second identity —
/// and it is escaped by the table cell renderer.
fn workspace_key_table(data: &AccessPageData<'_>) -> String {
    let rows = vec![vec![
        Cell::Text("Workspace key".to_string()),
        Cell::Mono(data.publisher.to_string()),
    ]];
    data_table(
        Some("Workspace"),
        &["Attribute", "Value"],
        &rows,
        None,
        Some("Provision the runner's ledger under this key."),
    )
}

fn session_actions(data: &AccessPageData<'_>) -> String {
    let mut body = String::new();
    match data.csrf {
        None => {
            body.push_str(&format!(
                "<p>{}</p>",
                html_escape("Revoking a session needs a live session of this workspace.")
            ));
        }
        Some(csrf) => {
            if let Some(current) = data
                .current_handle
                .and_then(|handle| data.sessions.iter().find(|s| s.handle == handle))
            {
                body.push_str(&format!(
                    "<p>Sign out of this session ({}).</p>\
                     <form method=\"post\" action=\"/console/ledger/session/end\">{}{}</form>",
                    html_escape(&utc_datetime(current.created_ms)),
                    csrf_hidden(csrf),
                    action_button("Sign out", "action", "sign_out", true)
                ));
            }
            for session in data
                .sessions
                .iter()
                .filter(|session| data.current_handle != Some(session.handle.as_str()))
            {
                body.push_str(&format!(
                    "<form method=\"post\" action=\"/console/settings/access/revoke\">{}\
                     <input type=\"hidden\" name=\"handle\" value=\"{}\">\
                     <input type=\"hidden\" name=\"idempotency_key\" value=\"{}\">\
                     {}{}</form>",
                    csrf_hidden(csrf),
                    html_escape(&session.handle),
                    html_escape(&crate::proposals::idempotency_key(
                        "session-revoke",
                        crate::proposals::ApprovalDecision::Declined,
                        &session.handle,
                    )),
                    checkbox_with_value(
                        &format!("confirm-revoke-{}", session.handle),
                        "confirm",
                        "yes",
                        &format!("Yes, revoke session {}.", session.handle),
                        false
                    ),
                    action_button(
                        &format!("Revoke session {}", session.handle),
                        "action",
                        "revoke",
                        false
                    )
                ));
            }
        }
    }
    panel("Session actions", &body, None)
}

/// The policy source the page displays.
///
/// When `APEX_EFFECTIVE_POLICY` is configured, that file (written by the
/// control plane's `policy_set`) is authoritative and the page recomputes the
/// approval context from it. A configured but invalid file yields `None`, not
/// the handler's legacy policy. Without the variable the handler's legacy
/// `APEX_POLICY_FILE` view is used unchanged.
fn effective_page_policy(
    passed: Option<&apex_autopilot::model::AutopilotPolicy>,
    passed_approval: Option<bool>,
    as_of_unix_ms: i64,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> (Option<apex_autopilot::model::AutopilotPolicy>, Option<bool>) {
    let configured =
        lookup(crate::settings::EFFECTIVE_POLICY_ENV).is_some_and(|value| !value.trim().is_empty());
    if !configured {
        return (passed.cloned(), passed_approval);
    }
    let policy = crate::settings::configured_policy(lookup).ok().flatten();
    let approval = policy
        .as_ref()
        .map(|policy| policy.approval_required(as_of_unix_ms));
    (policy, approval)
}

/// Render the policy settings page.
pub fn render_policy(ctx: &ConsoleCtx, data: &PolicyPageData<'_>) -> String {
    let mut body = String::from("<h1>Approval policy</h1>");
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "{} / as of {}",
            data.publisher,
            utc_datetime(data.as_of_unix_ms)
        ))
    ));
    // The control plane writes the effective policy file; when it is
    // configured it is the page's source in preference to the handler's
    // legacy APEX_POLICY_FILE read. An invalid effective file renders as
    // unavailable (fail closed) rather than falling back to legacy values.
    let (page_policy, approval_required_now) = effective_page_policy(
        data.policy,
        data.approval_required_now,
        data.as_of_unix_ms,
        &|name| std::env::var(name).ok(),
    );
    let policy = page_policy.as_ref();
    let registration = data.view.latest_registration.as_ref();
    let approval_label = match approval_required_now {
        Some(true) => "Approval required before the first write of each test.",
        Some(false) => "The recorded policy does not hold proposals; the gates below still run.",
        None if registration.is_some() => {
            "No effective policy file is recorded; the newest signed registration below is the authority."
        }
        None => "Not available (no policy record is stored in this workspace)",
    };
    body.push_str(&notice(
        StatusKind::Attention,
        approval_label,
        crate::settings::APPROVAL_MODE_STATEMENT,
        None,
    ));
    body.push_str(&notice(
        StatusKind::Neutral,
        POLICY_NOTE,
        "The values below are the recorded policy and the newest signed registration; this page \
         does not edit them.",
        None,
    ));
    let rows = vec![
        policy_row(
            "Action cap",
            policy.map(|policy| format!("{} actions per day", policy.max_actions_per_day)),
        ),
        policy_row(
            "Step cap",
            policy.map(|policy| percent(policy.max_step_bp as f64 / 10_000.0)),
        ),
        policy_row(
            "Test allocation",
            policy.map(|policy| format!("{}% test", (policy.test_share * 100.0).round() as i64)),
        ),
        policy_row(
            "Burn-in",
            registration
                .map(|registration| format!("{} days (signed)", registration.burn_in_days))
                .or_else(|| policy.map(|policy| format!("{} days", policy.burn_in_days))),
        ),
        policy_row(
            "Analysis window",
            registration
                .map(|registration| format!("{} final days (signed)", registration.window_days))
                .or_else(|| policy.map(|policy| format!("{} days", policy.window_days))),
        ),
        policy_row(
            "Fast harm stop",
            registration.map(|registration| threshold_value(&registration.thresholds, "harmPoint")),
        ),
        policy_row(
            "Rewarded fill drop",
            registration
                .map(|registration| threshold_value(&registration.thresholds, "fillRewarded")),
        ),
        policy_row(
            "Banner fill drop",
            registration
                .map(|registration| threshold_value(&registration.thresholds, "fillBanner")),
        ),
        policy_row(
            "Split integrity",
            registration.map(|registration| {
                let value = threshold_number(&registration.thresholds, "splitChiSquare");
                if value.starts_with("Not available") {
                    value
                } else {
                    format!("chi-square {value}")
                }
            }),
        ),
        policy_row(
            "Fee rate",
            registration.map(|registration| percent(f64::from(registration.rate_bp) / 10_000.0)),
        ),
        policy_row(
            "Engine version",
            registration.map(|registration| registration.code_version_hash.clone()),
        ),
        policy_row(
            "Who last changed",
            registration.map(|registration| {
                format!(
                    "Signed registration for {} at {} (seq {})",
                    registration.experiment_id,
                    utc_datetime(registration.at_unix_ms),
                    registration.seq
                )
            }),
        ),
    ];
    body.push_str(&data_table(
        Some("Recorded policy values"),
        &["Setting", "Recorded value"],
        &rows,
        Some(EmptyState {
            reason: "No policy or signed registration is recorded in this workspace.".to_string(),
            action: None,
        }),
        None,
    ));
    console_document(ctx, "Approval policy", "Settings", &body)
}

fn policy_row(label: &str, value: Option<String>) -> Vec<Cell> {
    vec![
        Cell::Text(label.to_string()),
        Cell::Text(
            value.unwrap_or_else(|| {
                or_not_available("no policy record is stored in this workspace")
            }),
        ),
    ]
}

/// One signed threshold as a percentage (or the raw value when it is not a
/// finite number).
fn threshold_value(thresholds: &serde_json::Value, key: &str) -> String {
    match thresholds.get(key).and_then(serde_json::Value::as_f64) {
        Some(value) => percent(value),
        None => or_not_available("the signed threshold is not a number"),
    }
}

/// One signed threshold as its recorded number.
fn threshold_number(thresholds: &serde_json::Value, key: &str) -> String {
    match thresholds.get(key).and_then(serde_json::Value::as_f64) {
        Some(value) => format!("{value}"),
        None => or_not_available("the signed threshold is not a number"),
    }
}

/// Render the monitoring settings page.
pub fn render_monitoring(ctx: &ConsoleCtx, data: &MonitoringPageData<'_>) -> String {
    let mut body = String::from("<h1>Monitoring</h1>");
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "{} / as of {}",
            data.publisher,
            utc_datetime(data.as_of_unix_ms)
        ))
    ));
    let plan_rows = vec![
        vec![
            Cell::Text("Monitoring plan".to_string()),
            Cell::Text(data.plan.selected_label().to_string()),
        ],
        vec![
            Cell::Text("Tier".to_string()),
            Cell::Text(match data.plan.tier_label() {
                Some(label) => label.to_string(),
                None => or_not_available("no tier is recorded"),
            }),
        ],
        vec![
            Cell::Text("Monthly price".to_string()),
            Cell::Text(match data.plan.fee_micros {
                Some(fee) => money_fixed(fee, &ctx.currency),
                None => or_not_available("no plan price is recorded"),
            }),
        ],
        vec![
            Cell::Text("Effective dates".to_string()),
            Cell::Text(match &data.plan.since {
                Some(since) => format!("Since {since}"),
                None => or_not_available("no effective date is recorded"),
            }),
        ],
        vec![
            Cell::Text("Next billing date".to_string()),
            Cell::Text(if data.plan.active && !data.plan.cancelled() {
                next_billing_day(data.as_of_unix_ms)
            } else {
                or_not_available("no next billing date: the plan is not billing")
            }),
        ],
        vec![
            Cell::Text("Cancellation".to_string()),
            Cell::Text(match &data.plan.stops_after {
                Some(period) => format!("Cancelled; the plan bills {period} and stops after it"),
                None if data.plan.active => {
                    "Cancels any time; the plan stops after the current month".to_string()
                }
                None => or_not_available("no plan is recorded"),
            }),
        ],
    ];
    body.push_str(&data_table(
        Some("Monitoring plan"),
        &["Fact", "Value"],
        &plan_rows,
        None,
        Some(MONITORING_NOTE),
    ));
    body.push_str(&monitoring_actions(data));
    let rows: Vec<Vec<Cell>> = data
        .rows
        .iter()
        .map(|row| {
            vec![
                Cell::Text(row.label.clone()),
                Cell::Status(
                    credential_label(row.credential_state).to_string(),
                    credential_kind(row.credential_state),
                ),
                Cell::Text(match row.reporting {
                    Some(true) => "Reporting reads confirmed".to_string(),
                    Some(false) => "Reporting reads not confirmed".to_string(),
                    None => or_not_available("reporting was not observed"),
                }),
                Cell::Text(match row.last_snapshot_unix_ms {
                    Some(at) => utc_datetime(at),
                    None => or_not_available("no report observed"),
                }),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Per-platform reporting health"),
        &["Platform", "Credential", "Reporting", "Last report"],
        &rows,
        None,
        None,
    ));
    body.push_str(&alerts_panel(data.alert_log_configured));
    console_document(ctx, "Monitoring", "Settings", &body)
}

/// The monitoring cancel action: POST + CSRF, rendered only when a plan is
/// recorded and not yet cancelled. Cancellation keeps the plan record (the
/// line bills the current period and stops), so the action is never offered
/// twice.
fn monitoring_actions(data: &MonitoringPageData<'_>) -> String {
    let mut body = String::new();
    if !data.plan.active || data.plan.cancelled() {
        body.push_str(&format!(
            "<p>{}</p>",
            html_escape(if data.plan.cancelled() {
                "This plan is cancelled. It bills its recorded stop period and nothing after it."
            } else {
                "Monitoring starts with your first validated connection; there is nothing to cancel yet."
            })
        ));
        return panel("Plan actions", &body, None);
    }
    match data.csrf {
        Some(csrf) => {
            body.push_str(&format!(
                "<p>{}</p><form method=\"post\" action=\"/console/settings/monitoring/cancel\">{}{}{}</form>",
                html_escape(
                    "Cancelling bills the current month and stops the plan after it. The record \
                     stays visible here."
                ),
                csrf_hidden(csrf),
                checkbox_with_value(
                    "confirm-monitoring-cancel",
                    "confirm",
                    "yes",
                    "Yes, cancel monitoring after this month.",
                    false
                ),
                action_button("Cancel monitoring", "action", "cancel", false)
            ));
        }
        None => {
            body.push_str(&format!(
                "<p>{}</p>",
                html_escape(
                    "Open the change log with your access token to start a console session; \
                     cancellation needs that session."
                )
            ));
            body.push_str(&action("Open the change log", "/console/ledger", false));
        }
    }
    panel("Plan actions", &body, None)
}

/// The first day of the month after `as_of_unix_ms`, the plan's next billing
/// date (plans bill monthly, so this is derived from the read time, never
/// stored).
fn next_billing_day(as_of_unix_ms: i64) -> String {
    let day = apex_analytics::UtcMillis::from_unix_millis(as_of_unix_ms).to_day_string();
    let year: i32 = day[..4].parse().unwrap_or(1970);
    let month: u32 = day[5..7].parse().unwrap_or(1);
    if month >= 12 {
        format!("{}-01-01", year + 1)
    } else {
        format!("{year}-{:02}-01", month + 1)
    }
}

/// Render the billing settings page: the managed ad-ops plan line with its
/// written scope and cancel action.
pub fn render_billing_settings(ctx: &ConsoleCtx, data: &BillingSettingsPageData<'_>) -> String {
    let mut body = String::from("<h1>Billing settings</h1>");
    body.push_str(&format!(
        "<p class=\"page-subtitle\">{}</p>",
        html_escape(&format!(
            "{} / as of {}",
            data.publisher,
            utc_datetime(data.as_of_unix_ms)
        ))
    ));
    body.push_str(&notice(
        StatusKind::Neutral,
        MANAGED_NOTE,
        "Monitoring and its tiers live on the monitoring settings page.",
        Some(("Open monitoring settings", "/console/settings/monitoring")),
    ));
    let rows = vec![
        vec![
            Cell::Text("Managed ad ops".to_string()),
            Cell::Text(data.managed.selected_label().to_string()),
        ],
        vec![
            Cell::Text("Monthly price".to_string()),
            Cell::Text(match data.managed.fee_micros {
                Some(fee) => money_fixed(fee, &ctx.currency),
                None => format!(
                    "From {} per month; $5,000 above $1M monthly revenue",
                    money_fixed(2_500 * 1_000_000, &ctx.currency)
                ),
            }),
        ],
        vec![
            Cell::Text("Effective dates".to_string()),
            Cell::Text(match &data.managed.since {
                Some(since) => format!("Since {since}"),
                None => or_not_available("no effective date is recorded"),
            }),
        ],
        vec![
            Cell::Text("Cancellation".to_string()),
            Cell::Text(match &data.managed.stops_after {
                Some(period) => format!("Cancelled; the plan bills {period} and stops after it"),
                None if data.managed.active => {
                    "Cancels any time; the plan stops after the current month".to_string()
                }
                None => or_not_available("no plan is recorded"),
            }),
        ],
    ];
    body.push_str(&data_table(
        Some("Managed ad ops plan"),
        &["Fact", "Value"],
        &rows,
        None,
        None,
    ));
    let scope_items: String = apex_billing::MANAGED_SCOPE
        .iter()
        .map(|item| format!("<li>{}</li>", html_escape(item)))
        .collect();
    body.push_str(&panel(
        "Included scope",
        &format!("<ul>{scope_items}</ul>"),
        None,
    ));
    body.push_str(&managed_actions(data));
    console_document(ctx, "Billing settings", "Settings", &body)
}

/// The managed cancel action: POST + CSRF, rendered only when the plan is
/// recorded and not yet cancelled.
fn managed_actions(data: &BillingSettingsPageData<'_>) -> String {
    let mut body = String::new();
    if !data.managed.active || data.managed.cancelled() {
        body.push_str(&format!(
            "<p>{}</p>",
            html_escape(if data.managed.cancelled() {
                "This plan is cancelled. It bills its recorded stop period and nothing after it."
            } else {
                "Managed ad ops is recorded by the account team; there is nothing to cancel yet."
            })
        ));
        return panel("Plan actions", &body, None);
    }
    match data.csrf {
        Some(csrf) => {
            body.push_str(&format!(
                "<p>{}</p><form method=\"post\" action=\"/console/settings/billing/managed/cancel\">{}{}{}</form>",
                html_escape(
                    "Cancelling bills the current month and stops the plan after it. The record \
                     stays visible here."
                ),
                csrf_hidden(csrf),
                checkbox_with_value(
                    "confirm-managed-cancel",
                    "confirm",
                    "yes",
                    "Yes, cancel the managed plan after this month.",
                    false
                ),
                action_button("Cancel managed plan", "action", "cancel", false)
            ));
        }
        None => {
            body.push_str(&format!(
                "<p>{}</p>",
                html_escape(
                    "Open the change log with your access token to start a console session; \
                     cancellation needs that session."
                )
            ));
            body.push_str(&action("Open the change log", "/console/ledger", false));
        }
    }
    panel("Plan actions", &body, None)
}

fn credential_label(state: CredentialState) -> &'static str {
    match state {
        CredentialState::Validated => "Connected",
        CredentialState::Rejected => "Refused",
        CredentialState::Expired => "Expired",
        CredentialState::Unvalidated => "Not connected",
    }
}

fn credential_kind(state: CredentialState) -> StatusKind {
    match state {
        CredentialState::Validated => StatusKind::Verified,
        CredentialState::Rejected | CredentialState::Expired => StatusKind::Attention,
        CredentialState::Unvalidated => StatusKind::Neutral,
    }
}

fn alerts_panel(log_configured: bool) -> String {
    let delivery = if log_configured {
        "The runner's alert log is connected to this console; incidents recorded there appear on \
         the experiment pages."
    } else {
        "No alert log is connected to this console; alert delivery is configured by the account \
         team."
    };
    let body = format!(
        "<p>The runner raises a customer-impact alert for exactly four conditions:</p>\
         <ul><li>A rollback that failed to revert</li><li>A guardrail that stopped a test</li>\
         <li>Two days of missing reporting</li><li>An expired platform token</li></ul>\
         <p>{}</p>",
        html_escape(delivery)
    );
    panel("Alert preferences", &body, None)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::connections::{ConnectionMeta, PlatformState};
    use crate::settings::{account_view, monitoring_rows, policy_view};

    use super::*;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::new("Northstar Studio", "USD", true)
    }

    #[test]
    fn account_page_shows_the_recorded_facts_and_theme_form() {
        let account = account_view(
            &|key| match key {
                crate::settings::CONTACT_ENV => Some("ops@northstar.example".to_string()),
                crate::settings::RETENTION_MONTHS_ENV => Some("24".to_string()),
                _ => None,
            },
            "Northstar Studio",
            "USD",
        );
        let html = render_account(
            &ctx(),
            &AccountPageData {
                account: &account,
                as_of_unix_ms: 1_792_055_400_000,
                theme: Theme::Dark,
                csrf: Some("tok"),
                theme_error: None,
            },
        );
        assert!(html.contains("<h1>Settings</h1>"));
        assert!(html.contains("Ask your account team to change these values."));
        assert!(html.contains("Northstar Studio"));
        assert!(html.contains("ops@northstar.example"));
        assert!(html.contains("UTC"));
        assert!(html.contains("24 months of signed records"));
        assert!(html.contains("revoking a connection ends the platform sessions"));
        assert!(html.contains("/console/settings/access"));
        assert!(html.contains("/console/settings/policy"));
        assert!(html.contains("/console/settings/monitoring"));
        assert!(html.contains("/console/settings/billing"));
        assert!(html.contains("<form class=\"theme-form\""));
        assert!(html.contains("value=\"dark\" checked"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn access_page_lists_sessions_by_handle_and_scopes_revocation() {
        let sessions = vec![
            crate::session::SessionSummary {
                handle: "0123456789abcdef".to_string(),
                publisher: "northstar".to_string(),
                created_ms: 1_792_000_000_000,
                last_seen_ms: 1_792_055_000_000,
            },
            crate::session::SessionSummary {
                handle: "fedcba9876543210".to_string(),
                publisher: "northstar".to_string(),
                created_ms: 1_791_900_000_000,
                last_seen_ms: 1_791_999_000_000,
            },
        ];
        let html = render_access(
            &ctx(),
            &AccessPageData {
                publisher: "northstar",
                sessions,
                current_handle: Some("0123456789abcdef"),
                csrf: Some("tok"),
                as_of_unix_ms: 1_792_055_400_000,
            },
        );
        assert!(html.contains("<h1>Access</h1>"));
        assert!(html.contains("A customer account owner is never an operator."));
        assert!(html.contains("The ledger access token is never stored in the browser."));
        assert!(html.contains("<code>0123456789abcdef</code>"));
        assert!(html.contains("(this session)"));
        assert!(html.contains("/console/ledger/session/end"));
        assert!(html.contains("/console/settings/access/revoke"));
        assert!(html.contains("name=\"handle\" value=\"fedcba9876543210\""));
        assert!(html.contains("Revoke session fedcba9876543210"));
        assert!(
            html.contains("name=\"confirm\" value=\"yes\""),
            "the revoke confirmation carries the accepted value"
        );
        assert!(html.contains("Yes, revoke session fedcba9876543210."));
        assert!(html.contains("Session ids are never shown"));
    }

    #[test]
    fn access_page_without_a_session_points_at_the_change_log() {
        let html = render_access(
            &ctx(),
            &AccessPageData {
                publisher: "northstar",
                sessions: Vec::new(),
                current_handle: None,
                csrf: None,
                as_of_unix_ms: 1_792_055_400_000,
            },
        );
        assert!(html.contains("No live session is recorded."));
        assert!(html.contains("Open the change log"));
    }

    #[test]
    fn access_page_shows_the_workspace_key_escaped() {
        let html = render_access(
            &ctx(),
            &AccessPageData {
                publisher: "a-b<com>-0123abcd",
                sessions: Vec::new(),
                current_handle: None,
                csrf: None,
                as_of_unix_ms: 1_792_055_400_000,
            },
        );
        assert!(html.contains("Workspace key"), "{html}");
        assert!(
            html.contains("a-b&lt;com&gt;-0123abcd"),
            "the key is escaped: {html}"
        );
        assert!(
            !html.contains("a-b<com>-0123abcd"),
            "the raw key never renders: {html}"
        );
        assert!(!html.contains("<script"), "{html}");
    }

    #[test]
    fn policy_page_shows_recorded_values_and_never_a_preset() {
        let html = render_policy(
            &ctx(),
            &PolicyPageData {
                publisher: "northstar",
                policy: None,
                approval_required_now: None,
                view: &policy_view(&[]),
                as_of_unix_ms: 1_792_055_400_000,
            },
        );
        assert!(html.contains("<h1>Approval policy</h1>"));
        assert!(html.contains("Not available (no policy record is stored in this workspace)"));
        assert!(html.contains(POLICY_NOTE));
        assert!(!html.contains("three actions per day"), "no preset values");
    }

    #[test]
    fn the_effective_policy_file_overrides_the_handler_policy_and_fails_closed() {
        let dir = std::env::temp_dir().join(format!("apex-api-html-policy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("effective.json");
        let mut effective = apex_autopilot::model::AutopilotPolicy::conservative();
        effective.max_actions_per_day = 1;
        effective.approval_mode = true;
        std::fs::write(&path, serde_json::to_vec(&effective).expect("json")).expect("write");
        let lookup = |name: &str| match name {
            crate::settings::EFFECTIVE_POLICY_ENV => Some(path.display().to_string()),
            _ => None,
        };
        let legacy = apex_autopilot::model::AutopilotPolicy::conservative();
        let (policy, approval) =
            effective_page_policy(Some(&legacy), Some(false), 1_700_000_000_000, &lookup);
        let policy = policy.expect("effective policy");
        assert_eq!(policy.max_actions_per_day, 1);
        assert_eq!(
            approval,
            Some(true),
            "approval is recomputed from the effective policy"
        );
        std::fs::write(&path, "{not json}").expect("write");
        let (policy, approval) =
            effective_page_policy(Some(&legacy), Some(false), 1_700_000_000_000, &lookup);
        assert!(
            policy.is_none(),
            "an invalid effective file never falls back"
        );
        assert_eq!(approval, None);
        let (policy, approval) =
            effective_page_policy(Some(&legacy), Some(false), 1_700_000_000_000, &|_| None);
        assert_eq!(
            policy.expect("legacy fallback"),
            apex_autopilot::model::AutopilotPolicy::conservative()
        );
        assert_eq!(approval, Some(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn policy_page_shows_signed_registration_values() {
        use sha2::{Digest, Sha256};
        let payload = r#"{"type":"preRegistration","experiment_id":"exp_1","scope":"u","before_value":1,"after_value":2,"split":0.25,"metric":"RevenuePerUser","burn_in_days":3,"window_days":14,"looks":1,"extension_allowed":true,"guardrail_thresholds":{"harmPoint":-0.04,"fillRewarded":0.02,"fillBanner":0.05,"splitChiSquare":10.83},"code_version_hash":"apex-lift-test","rate_bp":2000}"#;
        let digest = hex::encode(Sha256::digest(payload.as_bytes()));
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[5u8; 32]));
        let record = ledger
            .append(apex_proof::LedgerRecord::lift_record(
                1_792_000_000_000,
                &digest,
                payload,
            ))
            .expect("seal");
        let view = policy_view(&[record]);
        let policy = apex_autopilot::model::AutopilotPolicy::conservative();
        let html = render_policy(
            &ctx(),
            &PolicyPageData {
                publisher: "northstar",
                policy: Some(&policy),
                approval_required_now: Some(true),
                view: &view,
                as_of_unix_ms: 1_792_055_400_000,
            },
        );
        assert!(html.contains("Approval required before the first write of each test."));
        assert!(html.contains("3 actions per day"));
        assert!(html.contains("20.0%"));
        assert!(html.contains("3 days (signed)"));
        assert!(html.contains("14 final days (signed)"));
        assert!(html.contains("-4.0%"));
        assert!(html.contains("apex-lift-test"));
        assert!(html.contains("Signed registration for exp_1"));
    }

    #[test]
    fn monitoring_page_shows_plan_health_and_alerts() {
        let plan = MonitoringView::not_recorded();
        let mut meta = ConnectionMeta::default();
        meta.set_platform(
            "applovin_max",
            PlatformState {
                state: Some(CredentialState::Validated),
                reporting: Some(true),
                ..Default::default()
            },
        );
        let rows = monitoring_rows(
            &[(apex_core::PlatformId::AppLovinMax, 1_792_055_400_000)],
            &meta,
        );
        let html = render_monitoring(
            &ctx(),
            &MonitoringPageData {
                publisher: "northstar",
                plan: &plan,
                rows,
                alert_log_configured: true,
                as_of_unix_ms: 1_792_055_400_000,
                csrf: Some("tok"),
            },
        );
        assert!(html.contains("<h1>Monitoring</h1>"));
        assert!(html.contains("Not recorded"));
        assert!(html.contains("Not available (no plan price is recorded)"));
        assert!(html.contains("Not available (no effective date is recorded)"));
        assert!(html.contains("Not available (no next billing date: the plan is not billing)"));
        assert!(html.contains("Monitoring starts with your first validated connection"));
        assert!(!html.contains("Cancel monitoring"), "nothing to cancel");
        assert!(html.contains(">Connected<"));
        assert!(html.contains("Reporting reads confirmed"));
        assert!(html.contains("alert log is connected to this console"));
        assert!(html.contains("A rollback that failed to revert"));
        assert!(html.contains("An expired platform token"));
    }

    #[test]
    fn monitoring_page_shows_the_tier_price_next_billing_and_cancel_action() {
        // An active, recorded plan: tier, price, next billing date and the
        // POST + CSRF cancel action.
        let plan = MonitoringView::from_state(
            &apex_billing::MonitoringPlan::default().activate(350_000 * 1_000_000, "2026-05-01"),
        );
        let html = render_monitoring(
            &ctx(),
            &MonitoringPageData {
                publisher: "northstar",
                plan: &plan,
                rows: monitoring_rows(&[], &ConnectionMeta::default()),
                alert_log_configured: false,
                as_of_unix_ms: 1_792_055_400_000,
                csrf: Some("tok"),
            },
        );
        assert!(html.contains("Included"));
        assert!(html.contains(">Mid<"), "the tier is shown");
        assert!(html.contains("$1,500.00"));
        assert!(html.contains("Since 2026-05-01"));
        assert!(html.contains("2026-11-01"), "the next billing date");
        assert!(html.contains("Cancels any time; the plan stops after the current month"));
        assert!(
            html.contains("<form method=\"post\" action=\"/console/settings/monitoring/cancel\">")
        );
        assert!(
            html.contains("name=\"csrf_token\""),
            "the cancel form carries the token"
        );
        assert!(
            html.contains("name=\"confirm\" value=\"yes\""),
            "the cancel form carries the confirmation checkbox with the accepted value"
        );
        assert!(html.contains("Yes, cancel monitoring after this month."));
        assert!(html.contains("Cancel monitoring"));
        assert!(html.contains("No alert log is connected to this console"));

        // Without a session the action degrades to the change-log recovery,
        // never a CSRF-less form.
        let html = render_monitoring(
            &ctx(),
            &MonitoringPageData {
                publisher: "northstar",
                plan: &plan,
                rows: monitoring_rows(&[], &ConnectionMeta::default()),
                alert_log_configured: false,
                as_of_unix_ms: 1_792_055_400_000,
                csrf: None,
            },
        );
        assert!(!html.contains("<form"), "no form without a session");
        assert!(html.contains("Open the change log"));
        assert!(html.contains("cancellation needs that session"));
    }

    #[test]
    fn a_cancelled_monitoring_plan_renders_its_stop_period_without_a_cancel_action() {
        let mut state =
            apex_billing::MonitoringPlan::default().activate(50_000 * 1_000_000, "2026-05-01");
        state.cancel("2026-10");
        let plan = MonitoringView::from_state(&state);
        assert!(plan.cancelled());
        let html = render_monitoring(
            &ctx(),
            &MonitoringPageData {
                publisher: "northstar",
                plan: &plan,
                rows: monitoring_rows(&[], &ConnectionMeta::default()),
                alert_log_configured: false,
                as_of_unix_ms: 1_792_055_400_000,
                csrf: Some("tok"),
            },
        );
        assert!(html.contains("Cancelled"));
        assert!(html.contains("Cancelled; the plan bills 2026-10 and stops after it"));
        assert!(html.contains("bills its recorded stop period"));
        assert!(
            !html.contains("Cancel monitoring"),
            "cancelled plans are not cancelled twice"
        );
    }

    #[test]
    fn billing_settings_page_shows_the_managed_scope_and_cancel_action() {
        let managed =
            apex_billing::ManagedPlan::default().activate(2_000_000 * 1_000_000, "2026-05-01");
        let view = ManagedView::from_state(&managed);
        let html = render_billing_settings(
            &ctx(),
            &BillingSettingsPageData {
                publisher: "northstar",
                managed: &view,
                as_of_unix_ms: 1_792_055_400_000,
                csrf: Some("tok"),
            },
        );
        assert!(html.contains("<h1>Billing settings</h1>"));
        assert!(html.contains(">Active<"));
        assert!(html.contains("$5,000.00"), "the top price point is shown");
        // The written scope renders from the crate's typed const.
        for item in apex_billing::MANAGED_SCOPE {
            assert!(html.contains(item), "scope item missing: {item}");
        }
        assert!(html.contains(
            "<form method=\"post\" action=\"/console/settings/billing/managed/cancel\">"
        ));
        assert!(html.contains("name=\"csrf_token\""));
        assert!(
            html.contains("name=\"confirm\" value=\"yes\""),
            "the cancel form carries the confirmation checkbox with the accepted value"
        );
        assert!(html.contains("Yes, cancel the managed plan after this month."));
        assert!(html.contains("Cancel managed plan"));
        assert!(html.contains(MANAGED_NOTE));
        assert!(html.contains("/console/settings/monitoring"));

        // Nothing recorded: the offering with both price points, no cancel.
        let html = render_billing_settings(
            &ctx(),
            &BillingSettingsPageData {
                publisher: "northstar",
                managed: &ManagedView::not_recorded(),
                as_of_unix_ms: 1_792_055_400_000,
                csrf: Some("tok"),
            },
        );
        assert!(html.contains("Not recorded"));
        assert!(html.contains("From $2,500.00 per month; $5,000 above $1M monthly revenue"));
        assert!(html.contains("recorded by the account team"));
        assert!(!html.contains("<form"), "nothing to cancel");

        // A cancelled plan keeps its record and stops offering the action.
        let mut cancelled = managed;
        cancelled.cancel("2026-10");
        let html = render_billing_settings(
            &ctx(),
            &BillingSettingsPageData {
                publisher: "northstar",
                managed: &ManagedView::from_state(&cancelled),
                as_of_unix_ms: 1_792_055_400_000,
                csrf: Some("tok"),
            },
        );
        assert!(html.contains("Cancelled; the plan bills 2026-10 and stops after it"));
        assert!(!html.contains("Cancel managed plan"));
    }

    #[test]
    fn the_next_billing_day_rolls_over_the_year() {
        // 2026-10-15 -> 2026-11-01; a December read rolls into January.
        assert_eq!(next_billing_day(1_792_055_400_000), "2026-11-01");
        assert_eq!(next_billing_day(1_798_675_200_000), "2027-01-01");
    }
}
