//! `/console/connections` and `/console/connections/{platform}`.
//!
//! The list shows all three platforms with their true capabilities; the
//! detail page carries the frozen connection copy, the credential form
//! (per-platform fields with their documented help), the masked metadata of
//! the stored bundle and the rotate/disconnect forms. Secrets are never
//! rendered: the page shows field presence, scheme and ciphertext length
//! only.

use apex_core::PlatformId;
use apex_ui::components::{
    action_button, csrf_hidden, data_table, details, field, form_error_summary, input, kpi_card,
    kpi_row, notice, panel, Cell, EmptyState,
};
use apex_ui::format::{date_short, or_not_available, utc_datetime};
use apex_ui::status::StatusKind;

use crate::connections::{CredentialState, PlatformState};
use crate::vault::VaultEntryMeta;

use super::{console_document, html_escape, ConsoleCtx};

/// The frozen introduction of the connection detail page.
pub const INTRO_HEADING: &str = "Connect for the audit.";
/// The frozen introduction body.
pub const INTRO_BODY: &str = "Apex will read the configuration and reports available to this \
                             account. This step does not authorize platform changes. Your \
                             credentials are encrypted and are not shown again after saving.";
/// Frozen success copy.
pub const RESULT_SUCCESS: &str = "Read access confirmed. Your first report is not yet available.";
/// Frozen invalid-key copy.
pub const RESULT_INVALID: &str = "We could not confirm access. Check the account and credential \
                                  details.";
/// Frozen partial copy.
pub const RESULT_PARTIAL: &str = "Configuration is available. Reporting access needs attention.";
/// Frozen expired copy.
pub const RESULT_EXPIRED: &str = "Reconnect this platform to resume reporting.";
/// Shown when this deployment has no verified platform host: nothing was
/// validated, and the page says so instead of claiming a result.
pub const RESULT_UNVALIDATED: &str = "Access is not yet confirmed. This deployment has no way to \
                                     validate these credentials yet.";
/// The audit-availability sentence shown when reporting is unconfirmed.
pub const AUDIT_AVAILABILITY_NOTE: &str = "Configuration reads do not cover revenue reporting: \
                                           the floors, bidder and reporting-gap audit groups \
                                           cannot run until reporting access is confirmed.";

/// Frozen per-platform field help.
pub const MAX_FIELDS_HELP: &str = "Use separate management and reporting credentials. Validate \
                                   both with the configured platform hosts. If reporting is \
                                   unavailable, explain which audit groups cannot run.";
/// Frozen LevelPlay help.
pub const LEVELPLAY_FIELDS_HELP: &str = "The secret key and refresh token are exchanged for a \
                                         short-lived bearer token. The bearer token is never \
                                         shown in this page and never stored.";
/// Frozen AdMob help.
pub const ADMOB_FIELDS_HELP: &str = "Current storage accepts a token. A full OAuth consent and \
                                     refresh workflow is an addition; until that flow exists this \
                                     form accepts a token only.";
/// The rotation safety note.
pub const ROTATE_NOTE: &str = "Rotation replaces the stored credential bundle. The old material \
                               stops being used immediately after the new bundle is saved.";
/// The disconnect confirmation note.
pub const DISCONNECT_NOTE: &str = "Disconnecting removes the stored credentials and validation \
                                   record for this platform. Read-only reports stop until a new \
                                   credential is validated.";

/// One platform row of the connections list.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionRow {
    /// Wire name.
    pub wire: &'static str,
    /// Console label.
    pub label: &'static str,
    /// The stored credential state.
    pub credential_state: CredentialState,
    /// Write readiness label.
    pub write_label: &'static str,
    /// Newest snapshot time for the platform, when one exists.
    pub last_report_unix_ms: Option<i64>,
}

/// The list page data.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionsListData {
    /// Workspace publisher.
    pub publisher: String,
    /// Read time of the page.
    pub as_of_unix_ms: i64,
    /// Rows in display order.
    pub rows: Vec<ConnectionRow>,
}

/// The detail page data.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionDetailData<'a> {
    /// Workspace publisher.
    pub publisher: &'a str,
    /// The platform.
    pub platform: PlatformId,
    /// Console label.
    pub label: &'a str,
    /// The recorded state.
    pub state: PlatformState,
    /// Masked evidence of the stored bundle.
    pub blob_meta: Option<&'a VaultEntryMeta>,
    /// Whether this deployment can store and validate (the vault and the
    /// live read path exist).
    pub writable: bool,
    /// CSRF token when a console session exists (the forms render without it
    /// otherwise, replaced by the access note).
    pub csrf: Option<&'a str>,
    /// Platform-specific field errors from a refused submission.
    pub errors: Vec<(String, String)>,
    /// Safe values to preserve after a refused submission (never secrets).
    pub preserved: Vec<(String, String)>,
    /// Running experiments on this platform; `None` when the change log is
    /// not readable here and the warning must say so.
    pub running_work: Option<usize>,
    /// The newest snapshot time for the platform.
    pub last_report_unix_ms: Option<i64>,
}

/// Render the connections list.
pub fn render_list(ctx: &ConsoleCtx, data: &ConnectionsListData) -> String {
    let mut body = String::from("<h1>Connections</h1>");
    body.push_str(&notice(
        StatusKind::ReadOnly,
        "Read-only on the platforms.",
        "Connecting shares read access for the audit. No platform setting is changed here.",
        None,
    ));

    let connected = data
        .rows
        .iter()
        .filter(|row| row.credential_state == CredentialState::Validated)
        .count();
    body.push_str(&kpi_row(&[
        kpi_card(
            "Connected platforms",
            &format!("{connected} of {}", data.rows.len()),
            "Read access confirmed by a platform read",
            None,
        ),
        kpi_card(
            "Platform changes",
            "Read-only",
            "The shipped console changes no platform setting",
            None,
        ),
    ]));

    let rows: Vec<Vec<Cell>> = data
        .rows
        .iter()
        .map(|row| {
            vec![
                Cell::Link(
                    row.label.to_string(),
                    format!("/console/connections/{}", row.wire),
                ),
                Cell::Status(
                    read_state_label(row.credential_state).to_string(),
                    read_state_kind(row.credential_state),
                ),
                Cell::Text(row.write_label.to_string()),
                Cell::Text(match row.last_report_unix_ms {
                    Some(at) => date_short(at),
                    None => or_not_available("no report observed"),
                }),
                Cell::Link(
                    "Open".to_string(),
                    format!("/console/connections/{}", row.wire),
                ),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Platform connections"),
        &[
            "Platform",
            "Read access",
            "Write readiness",
            "Last report",
            "Action",
        ],
        &rows,
        Some(EmptyState {
            reason: "No platforms are configured in this deployment.".to_string(),
            action: None,
        }),
        Some(&format!(
            "{} / as of {}",
            data.publisher,
            utc_datetime(data.as_of_unix_ms)
        )),
    ));
    body.push_str(&capability_panel());
    console_document(ctx, "Connections", "Connections", &body)
}

fn capability_panel() -> String {
    let body = format!(
        "<ul><li>{}</li><li>{}</li><li>{}</li></ul>",
        html_escape(crate::connections::MATRIX_MAX_NOTE),
        html_escape(crate::connections::MATRIX_OTHER_NOTE),
        html_escape(crate::connections::MATRIX_ALL_NOTE)
    );
    panel("Capability release matrix", &body, None)
}

fn read_state_label(state: CredentialState) -> &'static str {
    match state {
        CredentialState::Validated => "Connected",
        _ => "Not connected",
    }
}

fn read_state_kind(state: CredentialState) -> StatusKind {
    match state {
        CredentialState::Validated => StatusKind::Verified,
        CredentialState::Rejected | CredentialState::Expired => StatusKind::Attention,
        CredentialState::Unvalidated => StatusKind::Neutral,
    }
}

/// Render the platform detail page.
pub fn render_detail(ctx: &ConsoleCtx, data: &ConnectionDetailData<'_>) -> String {
    let mut body = format!("<h1>{} connection</h1>", html_escape(data.label));
    if !data.writable {
        body.push_str(&panel(
            INTRO_HEADING,
            &format!("<p>{}</p>", html_escape(INTRO_BODY)),
            None,
        ));
        body.push_str(&notice(
            StatusKind::Attention,
            "This view is not available in this deployment.",
            "Credential storage is not configured; the account team can arrange access.",
            None,
        ));
        body.push_str(&capability_panel());
        return console_document(
            ctx,
            &format!("{} connection", data.label),
            "Connections",
            &body,
        );
    }

    body.push_str(&notice(
        StatusKind::ReadOnly,
        "Read-only on the platform.",
        "This step does not authorize platform changes.",
        None,
    ));
    body.push_str(&status_panel(data));
    if matches!(data.state.credential_state(), CredentialState::Validated)
        && data.state.reporting == Some(false)
    {
        body.push_str(&panel(
            "Audit availability",
            &format!("<p>{}</p>", html_escape(AUDIT_AVAILABILITY_NOTE)),
            None,
        ));
    }
    body.push_str(&panel(
        INTRO_HEADING,
        &format!(
            "<p>{}</p>{}",
            html_escape(INTRO_BODY),
            field_help_list(data.platform)
        ),
        None,
    ));
    body.push_str(&metadata_panel(data));
    body.push_str(&submit_form(data, false));
    if !data.state.credential_fields.is_empty() {
        body.push_str(&rotate_form(data));
        body.push_str(&disconnect_form(data));
    }
    body.push_str(&capability_panel());
    console_document(
        ctx,
        &format!("{} connection", data.label),
        "Connections",
        &body,
    )
}

fn field_help_list(platform: PlatformId) -> String {
    let help = match platform {
        PlatformId::AppLovinMax => MAX_FIELDS_HELP,
        PlatformId::LevelPlay => LEVELPLAY_FIELDS_HELP,
        PlatformId::Admob => ADMOB_FIELDS_HELP,
    };
    format!("<p>{}</p>", html_escape(help))
}

fn status_panel(data: &ConnectionDetailData<'_>) -> String {
    let state = data.state.credential_state();
    let mut out = String::new();
    match state {
        CredentialState::Validated => {
            let copy = if data.state.reporting == Some(false) {
                RESULT_PARTIAL
            } else {
                RESULT_SUCCESS
            };
            out.push_str(&notice(
                StatusKind::Verified,
                copy,
                &validated_note(data),
                None,
            ));
        }
        CredentialState::Rejected => {
            out.push_str(&notice(StatusKind::Failure, RESULT_INVALID, "", None));
        }
        CredentialState::Expired => {
            out.push_str(&notice(StatusKind::Attention, RESULT_EXPIRED, "", None));
        }
        CredentialState::Unvalidated => {
            out.push_str(&notice(StatusKind::Neutral, RESULT_UNVALIDATED, "", None));
        }
    }
    out
}

fn validated_note(data: &ConnectionDetailData<'_>) -> String {
    match data.state.validated_at_unix_ms {
        Some(at) => format!("Validated {}.", utc_datetime(at)),
        None => "No validation time is recorded.".to_string(),
    }
}

fn metadata_panel(data: &ConnectionDetailData<'_>) -> String {
    let state = data.state.credential_state();
    let fields = if data.state.credential_fields.is_empty() {
        or_not_available("no credential is stored")
    } else {
        data.state.credential_fields.join(", ")
    };
    let reporting = match data.state.reporting {
        Some(true) => "Reporting reads confirmed".to_string(),
        Some(false) => "Reporting reads not confirmed".to_string(),
        None => or_not_available("reporting was not observed"),
    };
    let rows = vec![
        vec![
            Cell::Text("Credential bundle".to_string()),
            Cell::Text(match data.blob_meta {
                Some(meta) => format!("{} / {} bytes, encrypted", meta.scheme, meta.ciphertext_len),
                None => or_not_available("nothing stored yet"),
            }),
        ],
        vec![Cell::Text("Stored fields".to_string()), Cell::Mono(fields)],
        vec![
            Cell::Text("Validation".to_string()),
            Cell::Text(format!(
                "{} / {}",
                credential_state_word(state),
                data.state
                    .validated_at_unix_ms
                    .map(utc_datetime)
                    .unwrap_or_else(|| or_not_available("never validated"))
            )),
        ],
        vec![Cell::Text("Scope".to_string()), Cell::Text(reporting)],
        vec![
            Cell::Text("Next refresh".to_string()),
            Cell::Text(or_not_available(
                "the platform does not publish a refresh time here",
            )),
        ],
        vec![
            Cell::Text("Last report".to_string()),
            Cell::Text(match data.last_report_unix_ms {
                Some(at) => utc_datetime(at),
                None => or_not_available("no report observed"),
            }),
        ],
    ];
    data_table(
        Some("Stored credential metadata (secrets are never shown)"),
        &["Attribute", "Value"],
        &rows,
        None,
        None,
    )
}

fn credential_state_word(state: CredentialState) -> &'static str {
    match state {
        CredentialState::Validated => "Read access confirmed",
        CredentialState::Rejected => "Refused by the platform",
        CredentialState::Expired => "Expired",
        CredentialState::Unvalidated => "Not validated",
    }
}

fn platform_fields(data: &ConnectionDetailData<'_>) -> Vec<(String, String, String, &'static str)> {
    // (name, label, kind, help)
    match data.platform {
        PlatformId::AppLovinMax => vec![
            (
                "account_id".to_string(),
                "Account ID".to_string(),
                "text".to_string(),
                "The MAX account the management key belongs to.",
            ),
            (
                "api_key".to_string(),
                "Management key".to_string(),
                "password".to_string(),
                "Used for configuration reads only.",
            ),
            (
                "report_key".to_string(),
                "Reporting key".to_string(),
                "password".to_string(),
                "Used for report reads. Without it, the reporting audit groups cannot run.",
            ),
        ],
        PlatformId::LevelPlay => vec![
            (
                "account_id".to_string(),
                "App key".to_string(),
                "text".to_string(),
                "The app key the management APIs are keyed by.",
            ),
            (
                "api_key".to_string(),
                "Secret key".to_string(),
                "password".to_string(),
                "Exchanged at the documented auth path.",
            ),
            (
                "refresh_token".to_string(),
                "Refresh token".to_string(),
                "password".to_string(),
                "Mints short-lived bearer tokens; the bearer token is never stored.",
            ),
        ],
        PlatformId::Admob => vec![
            (
                "account_id".to_string(),
                "Publisher account ID".to_string(),
                "text".to_string(),
                "The AdMob publisher account (`accounts/pub-...`).",
            ),
            (
                "api_key".to_string(),
                "Authorized access token".to_string(),
                "password".to_string(),
                "A token is accepted today; an OAuth consent flow is a later addition.",
            ),
        ],
    }
}

fn submit_form(data: &ConnectionDetailData<'_>, rotate: bool) -> String {
    let action = if rotate { "rotate" } else { "submit" };
    let heading = if rotate {
        "Rotate credentials"
    } else {
        match data.state.credential_state() {
            CredentialState::Rejected | CredentialState::Expired => "Reconnect this platform",
            _ => "Connect this platform",
        }
    };
    let note = if rotate { ROTATE_NOTE } else { INTRO_BODY };
    let mut form = format!(
        "<form method=\"post\" action=\"/console/connections/{}/{}\">",
        data.platform.as_str(),
        action
    );
    if let Some(csrf) = data.csrf {
        form.push_str(&csrf_hidden(csrf));
    }
    for (name, label, kind, help) in platform_fields(data) {
        let id = format!("{}-{}", data.platform.as_str(), name);
        let error = data
            .errors
            .iter()
            .find(|(field, _)| field == &name)
            .map(|(_, message)| message.clone());
        let preserved = data
            .preserved
            .iter()
            .find(|(field, _)| field == &name)
            .map(|(_, value)| value.clone());
        let control = input(&id, &name, &kind, preserved.as_deref(), !rotate, None);
        form.push_str(&field(&id, &label, Some(help), error.as_deref(), &control));
    }
    let button_label = if rotate {
        "Save new credentials"
    } else {
        "Save and confirm access"
    };
    form.push_str(&action_button(button_label, "action", action, true));
    form.push_str("</form>");
    let mut body = format!("<p>{}</p>", html_escape(note));
    if !data.errors.is_empty() {
        body.push_str(&form_error_summary(&data.errors));
    }
    body.push_str(&form);
    panel(heading, &body, None)
}

fn rotate_form(data: &ConnectionDetailData<'_>) -> String {
    if data.csrf.is_none() {
        return String::new();
    }
    details("Rotate credentials", &submit_form(data, true), false)
}

fn disconnect_form(data: &ConnectionDetailData<'_>) -> String {
    let Some(csrf) = data.csrf else {
        return String::new();
    };
    let work = match data.running_work {
        Some(0) => "<p>No running work is recorded for this platform.</p>".to_string(),
        Some(1) => "<p>One running test is recorded for this platform. <strong>It keeps running                     until it is stopped:</strong> disconnecting stops reporting, not the test.</p>"
            .to_string(),
        Some(count) => format!(
            "<p>{count} running tests are recorded for this platform. <strong>They keep running \
             until they are stopped:</strong> disconnecting stops reporting, not the tests.</p>"
        ),
        None => "<p>The change log is not readable here, so running work could not be checked                  before disconnecting.</p>"
            .to_string(),
    };
    let body = format!(
        "<p>{}</p>{work}\
         <form method=\"post\" action=\"/console/connections/{}/disconnect\">\
         {}{}</form>",
        html_escape(DISCONNECT_NOTE),
        data.platform.as_str(),
        csrf_hidden(csrf),
        action_button("Disconnect this platform", "action", "disconnect", false)
    );
    details("Disconnect", &body, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::PlatformState;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::new("Northstar Studio", "USD", true)
    }

    fn state(kind: CredentialState) -> PlatformState {
        PlatformState {
            state: Some(kind),
            validated_at_unix_ms: Some(1_792_055_400_000),
            reporting: Some(true),
            credential_fields: vec!["account_id".to_string(), "api_key".to_string()],
            failure_class: None,
        }
    }

    fn detail<'a>(
        kind: CredentialState,
        meta: Option<&'a VaultEntryMeta>,
        csrf: Option<&'a str>,
        writable: bool,
    ) -> ConnectionDetailData<'a> {
        ConnectionDetailData {
            publisher: "northstar",
            platform: PlatformId::AppLovinMax,
            label: "AppLovin MAX",
            state: state(kind),
            blob_meta: meta,
            writable,
            csrf,
            errors: Vec::new(),
            preserved: Vec::new(),
            running_work: Some(0),
            last_report_unix_ms: Some(1_792_055_400_000),
        }
    }

    #[test]
    fn list_shows_capabilities_reports_and_links() {
        let data = ConnectionsListData {
            publisher: "northstar".to_string(),
            as_of_unix_ms: 1_792_055_400_000,
            rows: vec![
                ConnectionRow {
                    wire: "applovin_max",
                    label: "AppLovin MAX",
                    credential_state: CredentialState::Validated,
                    write_label: "Approval required",
                    last_report_unix_ms: Some(1_792_055_400_000),
                },
                ConnectionRow {
                    wire: "levelplay",
                    label: "LevelPlay",
                    credential_state: CredentialState::Rejected,
                    write_label: "Not enabled",
                    last_report_unix_ms: None,
                },
                ConnectionRow {
                    wire: "admob",
                    label: "AdMob",
                    credential_state: CredentialState::Unvalidated,
                    write_label: "Not enabled",
                    last_report_unix_ms: None,
                },
            ],
        };
        let html = render_list(&ctx(), &data);
        assert!(html.contains("<h1>Connections</h1>"));
        assert!(html.contains("<a href=\"/console/connections/applovin_max\">AppLovin MAX</a>"));
        assert!(html.contains(">Connected<"));
        assert!(html.contains("Approval required"));
        assert!(html.contains(">Not enabled<"));
        assert!(html.contains(">Not connected<"));
        assert!(html.contains("Not available (no report observed)"));
        assert!(html.contains("writes need a verified allow-list"));
        assert!(html.contains("1 of 3"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn detail_validated_shows_masked_metadata_and_forms() {
        let meta = VaultEntryMeta {
            scheme: "aes-256-gcm",
            ciphertext_len: 412,
        };
        let html = render_detail(
            &ctx(),
            &detail(CredentialState::Validated, Some(&meta), Some("tok"), true),
        );
        assert!(html.contains("<h1>AppLovin MAX connection</h1>"));
        assert!(html.contains("Connect for the audit."));
        assert!(html.contains("Read access confirmed. Your first report is not yet available."));
        assert!(html.contains("aes-256-gcm / 412 bytes, encrypted"));
        assert!(html.contains("<code>account_id, api_key</code>"));
        assert!(html.contains("Not available (the platform does not publish a refresh time here)"));
        assert!(html.contains(
            "<form method=\"post\" action=\"/console/connections/applovin_max/submit\">"
        ));
        assert!(html.contains("name=\"api_key\""));
        assert!(html.contains("type=\"password\""));
        assert!(
            !html.contains("value=\"management-secret\""),
            "no secret is echoed"
        );
        assert!(html.contains("Rotate credentials"));
        assert!(html.contains("Disconnect this platform"));
        assert!(html.contains("No running work is recorded for this platform."));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn detail_states_render_the_frozen_copy() {
        let error_state = PlatformState {
            reporting: Some(false),
            ..state(CredentialState::Validated)
        };
        let html = render_detail(
            &ctx(),
            &ConnectionDetailData {
                state: error_state,
                ..detail(CredentialState::Validated, None, Some("tok"), true)
            },
        );
        assert!(html.contains(RESULT_PARTIAL));
        assert!(html.contains("cannot run until reporting access is confirmed"));

        let rejected = render_detail(
            &ctx(),
            &detail(CredentialState::Rejected, None, Some("tok"), true),
        );
        assert!(rejected.contains(RESULT_INVALID));
        assert!(rejected.contains("Reconnect this platform"));

        let expired = render_detail(
            &ctx(),
            &detail(CredentialState::Expired, None, Some("tok"), true),
        );
        assert!(expired.contains(RESULT_EXPIRED));

        let unvalidated = render_detail(
            &ctx(),
            &detail(CredentialState::Unvalidated, None, Some("tok"), true),
        );
        assert!(unvalidated.contains(RESULT_UNVALIDATED));
    }

    #[test]
    fn detail_without_vault_or_session_is_honest() {
        let html = render_detail(
            &ctx(),
            &detail(CredentialState::Unvalidated, None, None, false),
        );
        assert!(html.contains("This view is not available in this deployment."));
        assert!(!html.contains("<form"));
        let no_csrf = render_detail(
            &ctx(),
            &detail(CredentialState::Validated, None, None, true),
        );
        assert!(!no_csrf.contains("Rotate credentials"));
        assert!(!no_csrf.contains("Disconnect this platform"));
    }

    #[test]
    fn detail_escapes_hostile_metadata() {
        let hostile = PlatformState {
            credential_fields: vec!["<script>alert(1)</script>".to_string()],
            ..state(CredentialState::Validated)
        };
        let html = render_detail(
            &ctx(),
            &ConnectionDetailData {
                state: hostile,
                ..detail(CredentialState::Validated, None, Some("tok"), true)
            },
        );
        assert!(!html.contains("<script>alert(1)"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn running_work_is_explained_before_disconnect() {
        let html = render_detail(
            &ctx(),
            &ConnectionDetailData {
                running_work: Some(2),
                ..detail(CredentialState::Validated, None, Some("tok"), true)
            },
        );
        assert!(html.contains("2 running tests are recorded for this platform."));
        assert!(html.contains("keep running until they are stopped"));

        let unknown = render_detail(
            &ctx(),
            &ConnectionDetailData {
                running_work: None,
                ..detail(CredentialState::Validated, None, Some("tok"), true)
            },
        );
        assert!(unknown.contains("running work could not be checked"));
    }
}
