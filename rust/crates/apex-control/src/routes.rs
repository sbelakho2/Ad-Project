//! The Axum router: every route, its handler and the response discipline.
//!
//! All pages are GET; the explicit POST routes are the two sign-in steps,
//! logout and `/control/actions/{kind}`. An executed action answers 303 to
//! its stable result page, a duplicate idempotency key answers 303 to the
//! original result page, a stale-state mismatch answers 409 with the
//! conflict copy and a capability denial answers 403 with the page-97 copy.
//! No handler holds customer credentials; the only credentials in the
//! process are the operators' argon2id PHC strings and TOTP secrets.
//!
//! Every response also passes [`crate::telemetry::track_requests`], which
//! records the bounded route-template request series in the metrics registry
//! (the operator origin has no public `/metrics` route; the registry is
//! exported through `APEX_METRICS_TEXTFILE`).

use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;

use crate::actions::{ActionRequest, ActionState, SubmitOutcome};
use crate::auth::{self, CodeOutcome, PasswordOutcome};
use crate::capabilities::{Capability, Role};
use crate::data::DurableExperiment;
use crate::pages::{self, ActionFormSpec, PageMeta};
use crate::session::{self, SessionView};
use crate::signing::random_hex_32;
use crate::state::ControlState;

/// Build the full operator router.
pub fn router(state: ControlState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/control", get(overview))
        .route("/control/", get(overview))
        .route("/control/customers", get(customers))
        .route("/control/customers/{id}", get(customer_detail))
        .route("/control/ingestion", get(ingestion))
        .route("/control/audits", get(audits))
        .route("/control/experiments", get(experiments))
        .route("/control/experiments/{id}", get(experiment_detail))
        .route("/control/incidents", get(incidents))
        .route("/control/incidents/{id}", get(incident_detail))
        .route("/control/finance", get(finance))
        .route("/control/finance/invoices/{id}", get(invoice_detail))
        .route("/control/finance/exports/{register}", get(finance_export))
        .route("/control/growth", get(growth))
        .route("/control/system", get(system))
        .route("/control/system/policies", get(system_policies))
        .route("/control/access", get(access))
        .route("/favicon.ico", get(crate::favicon::ico))
        .route("/favicon.svg", get(crate::favicon::svg))
        .route("/control/login", get(login_form).post(login_submit))
        .route(
            "/control/login/code",
            get(login_code_form).post(login_code_submit),
        )
        .route("/control/logout", post(logout))
        .route("/control/actions/{kind}", post(action_submit))
        .route("/control/actions/{kind}/result/{id}", get(action_result))
        .fallback(not_found)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::telemetry::track_requests,
        ))
        .with_state(state)
}

async fn root() -> Redirect {
    Redirect::to("/control/")
}

/// The page meta for a request, with or without a session.
fn meta(state: &ControlState, session: Option<&SessionView>) -> PageMeta {
    PageMeta {
        environment: state.environment().to_string(),
        as_of_ms: state.now_ms(),
        operator: session.map(|view| view.operator_name.clone()),
        role: session.map(|view| view.role),
        fixtures: state.data.is_fixtures(),
    }
}

fn html_response(status: StatusCode, body: String) -> Response {
    (status, Html(body)).into_response()
}

fn redirect(to: &str, cookie: Option<String>) -> Response {
    let mut response = Redirect::to(to).into_response();
    if let Some(cookie) = cookie {
        if let Ok(value) = cookie.parse() {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

/// The signed-in session, or the session-expired response to return.
fn require_session(
    state: &ControlState,
    headers: &HeaderMap,
) -> Result<SessionView, Box<Response>> {
    let meta = meta(state, None);
    let session = session::session_id_from_cookie(
        headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok()),
    )
    .and_then(|id| state.sessions.touch(&id, state.now_ms()));
    match session {
        Some(view) => Ok(view),
        None => Err(Box::new(html_response(
            StatusCode::UNAUTHORIZED,
            pages::login::session_expired(&meta),
        ))),
    }
}

/// The unconfigured-deployment response, when the deployment cannot serve
/// operator pages.
fn unconfigured_response(state: &ControlState) -> Option<Response> {
    let reason = state.unconfigured_reason.clone()?;
    let meta = meta(state, None);
    Some(html_response(
        StatusCode::SERVICE_UNAVAILABLE,
        pages::login::unconfigured(&meta, &reason),
    ))
}

// ---------------------------------------------------------------------------
// GET pages
// ---------------------------------------------------------------------------

async fn overview(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let queues = state
        .data
        .queues(state.as_of())
        .await
        .map_err(|error| error.reason);
    let fee_rows = state
        .data
        .fee_rows(state.as_of())
        .await
        .map_err(|error| error.reason);
    html_response(
        StatusCode::OK,
        pages::overview::render(&meta, &pages::overview::OverviewData { queues, fee_rows }),
    )
}

async fn customers(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let rows = customer_rows(&state).await;
    let filters = pages::customers::CustomerFilters::from_query(&pairs);
    html_response(
        StatusCode::OK,
        pages::customers::render_list(&meta, rows, &filters),
    )
}

async fn customer_rows(state: &ControlState) -> Result<Vec<crate::fixtures::CustomerRow>, String> {
    match state.data.as_ref() {
        crate::data::ControlData::Fixtures(demo) => Ok(demo.customers()),
        crate::data::ControlData::Live(_) => Err(
            "the live account inventory read is not configured in this deployment (no account projection source)"
                .to_string(),
        ),
    }
}

async fn customer_detail(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let rows = customer_rows(&state).await;
    let account_id = id;
    let row = match rows {
        Ok(rows) => match rows.into_iter().find(|row| row.account_id == account_id) {
            Some(row) => row,
            None => return html_response(StatusCode::NOT_FOUND, pages::errors::not_found(&meta)),
        },
        Err(reason) => {
            return html_response(
                StatusCode::SERVICE_UNAVAILABLE,
                pages::errors::not_configured(&meta, &reason),
            )
        }
    };
    let timeline = state
        .data
        .object_timeline(state.as_of(), "account", &row.account_id)
        .await
        .map_err(|error| error.reason);
    let experiments = state
        .data
        .durable_experiments()
        .map(|experiments| {
            experiments
                .into_iter()
                .filter(|experiment| experiment.publisher == row.publisher_id)
                .collect::<Vec<DurableExperiment>>()
        })
        .map_err(|error| error.reason);
    let filters = pages::customers::CustomerFilters::from_query(&pairs);
    let tab = pages::customers::DetailTab::parse(
        pairs
            .iter()
            .find(|(key, _)| key == "tab")
            .map(|(_, value)| value.as_str()),
    );
    html_response(
        StatusCode::OK,
        pages::customers::render_detail(
            &meta,
            &pages::customers::CustomerDetail {
                customer: row,
                timeline,
                experiments,
            },
            tab,
            &filters,
        ),
    )
}

async fn ingestion(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let health = state
        .data
        .ingestion_health(state.as_of())
        .await
        .map_err(|error| error.reason);
    let coverage = match state.data.as_ref() {
        crate::data::ControlData::Fixtures(demo) => Ok(demo.coverage_rows()),
        crate::data::ControlData::Live(_) => Err(
            "the per-day coverage matrix is not exposed by the current analytics read model; the freshness rows are shown instead"
                .to_string(),
        ),
    };
    let retry_forms = if session.role.allows(Capability::RetryRead) {
        match &health {
            Ok(rows) => {
                let mut forms = Vec::new();
                for row in rows
                    .iter()
                    .filter(|row| row.status != apex_analytics::queries::FreshnessStatus::Fresh)
                {
                    let target = format!(
                        "{}:{}:{}",
                        row.publisher_id, row.platform, row.platform_account_id
                    );
                    if let Some(spec) = build_form_spec(
                        &state,
                        &session,
                        Capability::RetryRead,
                        &target,
                        "/control/ingestion",
                        &[
                            ("platform", row.platform.clone()),
                            ("account", row.platform_account_id.clone()),
                        ],
                    )
                    .await
                    {
                        forms.push(pages::ingestion::AccountRetryForm { spec });
                    }
                }
                forms
            }
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    html_response(
        StatusCode::OK,
        pages::ingestion::render(
            &meta,
            &pages::ingestion::IngestionData {
                health,
                coverage,
                retry_forms,
            },
        ),
    )
}

async fn audits(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let quality = match state.data.as_ref() {
        crate::data::ControlData::Fixtures(demo) => Ok(demo.audit_quality()),
        crate::data::ControlData::Live(_) => Err(
            "audit-run eligibility is not exposed by a read model yet (the AuditCompleted and AuditFailed events are stored, but no projection is registered)"
                .to_string(),
        ),
    };
    html_response(
        StatusCode::OK,
        pages::audits::render(&meta, &pages::audits::AuditsData { quality }),
    )
}

async fn experiments(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let view = pairs
        .iter()
        .find(|(key, _)| key == "view")
        .map(|(_, value)| value.as_str());
    let cohort = pairs
        .iter()
        .find(|(key, _)| key == "cohort")
        .map(|(_, value)| value.as_str());
    let landscape = state
        .data
        .experiment_landscape(state.as_of())
        .await
        .map_err(|error| error.reason);
    let cohorts = match state.data.as_ref() {
        crate::data::ControlData::Fixtures(demo) => demo
            .growth_cohorts()
            .into_iter()
            .map(|cohort| cohort.cohort_month)
            .collect(),
        crate::data::ControlData::Live(_) => Vec::new(),
    };
    let selected = cohort
        .map(str::to_string)
        .or_else(|| cohorts.last().cloned());
    let effectiveness = if view == Some("effectiveness") {
        Some(match selected.as_deref() {
            Some(month) => state
                .data
                .experiment_effectiveness(state.as_of(), month)
                .await
                .map_err(|error| error.reason),
            None => Err("no settled start cohort is present in this deployment".to_string()),
        })
    } else {
        None
    };
    let decisions_due = match &landscape {
        Ok(rows) => pages::experiments::fleet_kpis(rows).decisions_due.0 > 0,
        Err(_) => false,
    };
    html_response(
        StatusCode::OK,
        pages::experiments::render_fleet(
            &meta,
            &pages::experiments::ExperimentsData {
                landscape,
                effectiveness,
                cohorts,
                selected_cohort: selected,
                decisions_due,
            },
        ),
    )
}

async fn experiment_detail(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let experiment = state
        .data
        .durable_experiments()
        .and_then(|experiments| {
            experiments
                .into_iter()
                .find(|experiment| experiment.experiment_id == id)
                .ok_or_else(|| {
                    crate::data::Unavailable::new(format!(
                        "no durable record names experiment {id}"
                    ))
                })
        })
        .map_err(|error| error.reason);
    let lift = state
        .data
        .lift_rows(state.as_of())
        .await
        .ok()
        .and_then(|rows| rows.into_iter().find(|row| row.experiment_id == id));
    let checks = match state.data.as_ref() {
        crate::data::ControlData::Fixtures(_) => Ok(pages::experiments::demo_checks()),
        crate::data::ControlData::Live(_) => Err(
            "registered decision checks are recorded on the signed decision reference; open the signed source for the exact rows"
                .to_string(),
        ),
    };
    let mut forms = Vec::new();
    if session.role.allows(Capability::RestoreReconcileRequest) {
        if let Some(spec) = build_form_spec(
            &state,
            &session,
            Capability::RestoreReconcileRequest,
            &id,
            &format!("/control/experiments/{id}"),
            &[],
        )
        .await
        {
            forms.push(spec);
        }
    }
    html_response(
        StatusCode::OK,
        pages::experiments::render_detail(
            &meta,
            &id,
            &pages::experiments::ExperimentDetail {
                experiment,
                lift,
                checks,
                forms,
            },
        ),
    )
}

async fn incidents(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let rows = state
        .data
        .queues(state.as_of())
        .await
        .map(|queues| queues.incidents)
        .map_err(|error| error.reason);
    html_response(StatusCode::OK, pages::incidents::render_list(&meta, rows))
}

async fn incident_detail(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let case = incident_case(&state, &id).await;
    if case.is_err() {
        if let Err(reason) = &case {
            if reason == "not-found" {
                return html_response(StatusCode::NOT_FOUND, pages::errors::not_found(&meta));
            }
        }
    }
    let preview = match &case {
        Ok(case) => build_preview(&state, case).await,
        Err(_) => None,
    };
    let mut forms = Vec::new();
    for capability in [
        Capability::IncidentAcknowledge,
        Capability::OwnerAssign,
        Capability::IncidentResolve,
    ] {
        if !session.role.allows(capability) {
            continue;
        }
        if let Some(spec) = build_form_spec(
            &state,
            &session,
            capability,
            &id,
            &format!("/control/incidents/{id}"),
            &[],
        )
        .await
        {
            forms.push(spec);
        }
    }
    let step_up_note = session.role.capabilities().iter().any(|capability| {
        capability.requires_step_up()
            && matches!(
                capability,
                Capability::IncidentResolve | Capability::RestoreReconcileRequest
            )
    }) && !session.recent_auth(state.now_ms(), state.sessions.step_up_ms());
    html_response(
        StatusCode::OK,
        pages::incidents::render_detail(
            &meta,
            &id,
            &pages::incidents::IncidentDetail {
                case,
                preview,
                forms,
                step_up_note,
            },
        ),
    )
}

async fn incident_case(
    state: &ControlState,
    id: &str,
) -> Result<crate::fixtures::IncidentCase, String> {
    match state.data.as_ref() {
        crate::data::ControlData::Fixtures(demo) => demo
            .incident_case(id)
            .ok_or_else(|| "not-found".to_string()),
        crate::data::ControlData::Live(_) => {
            let queues = state
                .data
                .queues(state.as_of())
                .await
                .map_err(|error| error.reason)?;
            let row = queues
                .incidents
                .iter()
                .find(|row| row.object_id == id)
                .ok_or_else(|| "not-found".to_string())?;
            let timeline = state
                .data
                .object_timeline(state.as_of(), "incident", id)
                .await
                .unwrap_or_default();
            Ok(crate::fixtures::IncidentCase {
                id: id.to_string(),
                severity: row.severity.clone(),
                age_seconds: row.age_seconds,
                publisher_id: row.publisher_id.clone(),
                owner: row.owner.clone(),
                headline: format!("Incident {id} is open."),
                experiment: None,
                scope: None,
                last_confirmed_state: None,
                failed_operation: None,
                occurred_display: None,
                next_update: None,
                communication_draft: format!(
                    "We are investigating an incident affecting your account. Reference {id}. The next update follows the owner's schedule."
                ),
                affects_traffic: None,
                timeline,
            })
        }
    }
}

async fn build_preview(
    state: &ControlState,
    case: &crate::fixtures::IncidentCase,
) -> Option<Result<crate::fixtures::InterventionPreview, String>> {
    match state.data.as_ref() {
        crate::data::ControlData::Fixtures(demo) => {
            let _ = demo;
            Some(Ok(crate::fixtures::InterventionPreview {
                publisher: case.publisher_id.clone(),
                operation: case
                    .failed_operation
                    .clone()
                    .unwrap_or_else(|| "review".to_string()),
                prior_state: case
                    .last_confirmed_state
                    .clone()
                    .unwrap_or_else(|| "not available".to_string()),
                target_state: "restored state confirmed by a platform read".to_string(),
                policy: "conservative defaults; actions stay unavailable without evidence"
                    .to_string(),
                permissions: "the acting operator's role and recent sign-in".to_string(),
                affects_traffic: case.affects_traffic.unwrap_or(false),
            }))
        }
        crate::data::ControlData::Live(_) => Some(Err(
            "the platform state read for this action is not wired in this deployment".to_string(),
        )),
    }
}

async fn finance(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let view = pairs
        .iter()
        .find(|(key, _)| key == "view")
        .map(|(_, value)| value.as_str());
    let currency = pairs
        .iter()
        .find(|(key, _)| key == "currency")
        .map(|(_, value)| value.clone());
    let fee_rows = state
        .data
        .fee_rows(state.as_of())
        .await
        .map_err(|error| error.reason);
    let analytics = if view == Some("analytics") {
        Some(match state.data.as_ref() {
            crate::data::ControlData::Fixtures(demo) => Ok(demo.economics()),
            crate::data::ControlData::Live(_) => Err(
                "the monthly economics series is not exposed by the current analytics read model"
                    .to_string(),
            ),
        })
    } else {
        None
    };
    html_response(
        StatusCode::OK,
        pages::finance::render(
            &meta,
            &pages::finance::FinanceData {
                fee_rows,
                currency,
                analytics,
            },
        ),
    )
}

async fn invoice_detail(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let invoices = match state.data.invoices() {
        Ok(invoices) => invoices,
        Err(error) => {
            return html_response(
                StatusCode::SERVICE_UNAVAILABLE,
                pages::errors::not_configured(&meta, &error.reason),
            )
        }
    };
    let Some(invoice) = invoices.into_iter().find(|invoice| invoice.id() == id) else {
        return html_response(StatusCode::NOT_FOUND, pages::errors::not_found(&meta));
    };
    let payments = match state.data.as_ref() {
        crate::data::ControlData::Fixtures(demo) => demo
            .payments()
            .into_iter()
            .filter(|payment| payment.invoice_id == id)
            .collect(),
        crate::data::ControlData::Live(_) => Vec::new(),
    };
    let mut forms = Vec::new();
    for capability in [
        Capability::InvoiceHold,
        Capability::InvoiceRelease,
        Capability::CollectionRetry,
        Capability::DisputeMark,
        Capability::ManualReconcile,
    ] {
        if !session.role.allows(capability) {
            continue;
        }
        if let Some(spec) = build_form_spec(
            &state,
            &session,
            capability,
            &id,
            &format!("/control/finance/invoices/{id}"),
            &[],
        )
        .await
        {
            forms.push(spec);
        }
    }
    let source_hashes = invoice
        .line_items
        .iter()
        .map(|line| {
            format!(
                "sha256:{}",
                crate::signing::sha256_hex(
                    format!("{}:{}", invoice.id(), line.experiment_id).as_bytes()
                )
            )
        })
        .collect();
    html_response(
        StatusCode::OK,
        pages::finance::render_invoice(
            &meta,
            &pages::finance::InvoiceDetail {
                invoice,
                payments,
                calculation_version: "calc-v3".to_string(),
                source_hashes,
                forms,
            },
        ),
    )
}

async fn finance_export(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(register): Path<String>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let _ = session;
    let rows = match state.data.invoices() {
        Ok(rows) => rows,
        Err(error) => {
            let meta = meta(&state, None);
            return html_response(
                StatusCode::SERVICE_UNAVAILABLE,
                pages::errors::not_configured(&meta, &error.reason),
            );
        }
    };
    match register.as_str() {
        "invoices.csv" => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
            pages::finance::invoices_csv(&rows),
        )
            .into_response(),
        "transactions.csv" => {
            let payments = match state.data.as_ref() {
                crate::data::ControlData::Fixtures(demo) => demo.payments(),
                crate::data::ControlData::Live(_) => Vec::new(),
            };
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/csv; charset=utf-8")],
                pages::finance::transactions_csv(&payments),
            )
                .into_response()
        }
        _ => {
            let meta = meta(&state, None);
            html_response(StatusCode::NOT_FOUND, pages::errors::not_found(&meta))
        }
    }
}

async fn growth(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let cohorts = state
        .data
        .growth_cohorts(state.as_of())
        .await
        .map_err(|error| error.reason);
    let selected = pairs
        .iter()
        .find(|(key, _)| key == "cohort")
        .map(|(_, value)| value.clone());
    let funnel = match selected.as_deref() {
        Some(month) => state
            .data
            .activation_funnel(state.as_of(), month)
            .await
            .map_err(|error| error.reason),
        None => Err("no cohort is selected yet".to_string()),
    };
    let site_requests = state.data.site_requests_last_30_days(state.as_of()).await;
    html_response(
        StatusCode::OK,
        pages::growth::render(
            &meta,
            &pages::growth::GrowthData {
                cohorts,
                selected_cohort: selected,
                funnel,
                site_requests,
            },
        ),
    )
}

async fn system(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let overdue_jobs = state
        .data
        .queues(state.as_of())
        .await
        .map(|queues| queues.overdue_jobs)
        .map_err(|error| error.reason);
    let component_rows = match state.data.as_ref() {
        crate::data::ControlData::Fixtures(demo) => demo.system_components(),
        crate::data::ControlData::Live(_) => Vec::new(),
    };
    html_response(
        StatusCode::OK,
        pages::system::render(
            &meta,
            &pages::system::SystemData {
                overdue_jobs,
                component_rows,
            },
        ),
    )
}

async fn system_policies(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let evidence = state.data.operations_evidence();
    let evidence_problem = evidence.as_ref().err().map(|error| error.reason.clone());
    let readiness = crate::data::readiness_rows(&evidence.unwrap_or_default());
    let pending = state
        .governance
        .pending_policies()
        .map_err(|error| error.to_string());
    let active = state
        .governance
        .active_policies()
        .map_err(|error| error.to_string());
    let interventions = state
        .governance
        .interventions()
        .map(|read| read.records)
        .map_err(|error| error.to_string());
    let fleet_paused = state.governance.fleet_scheduling_paused().unwrap_or(false);
    let release_health = match state.data.as_ref() {
        crate::data::ControlData::Fixtures(demo) => demo.release_health(),
        crate::data::ControlData::Live(_) => vec![crate::fixtures::ReleaseHealthRow {
            service: "apex-control".to_string(),
            version: crate::state::BUILD_LABEL.to_string(),
            migration_state: "No migrations (in-memory sessions, file journal)".to_string(),
            report_schema: "v1 domain event envelope".to_string(),
            metric_schema: "apex-metrics bounded registry (APEX_METRICS_TEXTFILE export)"
                .to_string(),
            data_quality: "See the audits and ingestion pages".to_string(),
            recent_errors: "See the action journal".to_string(),
        }],
    };
    let mut forms = Vec::new();
    if session.role.allows(Capability::PolicyDraft) {
        if let Some(spec) = build_form_spec(
            &state,
            &session,
            Capability::PolicyDraft,
            "policy:draft",
            "/control/system/policies",
            &[],
        )
        .await
        {
            forms.push(spec);
        }
    }
    if session.role.allows(Capability::PolicyActivate) {
        if let Ok(pending_records) = &pending {
            for record in pending_records {
                if let Some(spec) = build_form_spec(
                    &state,
                    &session,
                    Capability::PolicyActivate,
                    &record.policy_id,
                    "/control/system/policies",
                    &[("policy_id", record.policy_id.clone())],
                )
                .await
                {
                    forms.push(spec);
                }
            }
        }
    }
    if session.role.allows(Capability::FleetPauseScheduling) && !fleet_paused {
        if let Some(spec) = build_form_spec(
            &state,
            &session,
            Capability::FleetPauseScheduling,
            "fleet:scheduling",
            "/control/system/policies",
            &[],
        )
        .await
        {
            forms.push(spec);
        }
    }
    if session.role.allows(Capability::FleetResumeScheduling) && fleet_paused {
        if let Some(spec) = build_form_spec(
            &state,
            &session,
            Capability::FleetResumeScheduling,
            "fleet:scheduling",
            "/control/system/policies",
            &[],
        )
        .await
        {
            forms.push(spec);
        }
    }
    html_response(
        StatusCode::OK,
        pages::system::render_policies(
            &meta,
            &pages::system::PoliciesData {
                readiness,
                evidence_problem,
                pending,
                active,
                interventions,
                fleet_paused,
                release_health,
                forms,
            },
        ),
    )
}

async fn access(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let history = state
        .journal
        .read()
        .map(|read| read.entries)
        .map_err(|error| error.to_string());
    let sessions = state.sessions.list(state.now_ms());
    let can_revoke = session.role.allows(Capability::SessionRevoke);
    let mut revoke_forms = Vec::new();
    if can_revoke {
        for row in &sessions {
            if let Some(spec) = build_form_spec(
                &state,
                &session,
                Capability::SessionRevoke,
                &row.id_prefix,
                "/control/access",
                &[],
            )
            .await
            {
                revoke_forms.push(spec);
            }
        }
    }
    html_response(
        StatusCode::OK,
        pages::access::render(
            &meta,
            &pages::access::AccessData {
                history,
                sessions,
                can_revoke,
                revoke_forms,
            },
        ),
    )
}

// ---------------------------------------------------------------------------
// Sign-in
// ---------------------------------------------------------------------------

fn query_value(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
        .filter(|value| !value.is_empty())
}

async fn login_form(
    State(state): State<ControlState>,
    _headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let meta = meta(&state, None);
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let reauth = query_value(&pairs, "reauth").is_some();
    let next = query_value(&pairs, "next");
    html_response(
        StatusCode::OK,
        pages::login::render(
            &meta,
            &pages::login::LoginState::Password {
                problem: None,
                preserved_operator: String::new(),
                preserved_next: next,
                reauth,
            },
        ),
    )
}

async fn login_submit(
    State(state): State<ControlState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let meta = meta(&state, None);
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let fields = match parse_form(&body) {
        Ok(fields) => fields,
        Err(reason) => {
            return html_response(
                StatusCode::BAD_REQUEST,
                pages::errors::bad_request(&meta, Some(&reason)),
            )
        }
    };
    let operator = fields.get("operator").cloned().unwrap_or_default();
    let password = fields.get("password").cloned().unwrap_or_default();
    let next = fields.get("next").cloned();
    let reauth = fields.contains_key("reauth");
    let client = auth::client_identifier(
        &headers,
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok()),
    );
    let outcome = auth::password_login(
        &state.operators,
        &state.limiter,
        &state.pending,
        &operator,
        &password,
        &client,
        next.as_deref(),
        reauth,
        state.now_ms(),
    );
    match outcome {
        PasswordOutcome::Unconfigured => unconfigured_response(&state)
            .unwrap_or_else(|| html_response(StatusCode::OK, String::new())),
        PasswordOutcome::RateLimited { until_ms } => html_response(
            StatusCode::TOO_MANY_REQUESTS,
            pages::login::render(&meta, &pages::login::LoginState::RateLimited { until_ms }),
        ),
        PasswordOutcome::Rejected {
            preserved_operator,
            preserved_next,
            reauth,
        } => html_response(
            StatusCode::UNAUTHORIZED,
            pages::login::render(
                &meta,
                &pages::login::LoginState::Password {
                    problem: Some(crate::copy::LOGIN_FAILED),
                    preserved_operator,
                    preserved_next,
                    reauth,
                },
            ),
        ),
        PasswordOutcome::CodeRequired {
            pending_token,
            operator_name,
        } => html_response(
            StatusCode::OK,
            pages::login::render(
                &meta,
                &pages::login::LoginState::Code {
                    pending_token,
                    operator_name,
                    problem: None,
                },
            ),
        ),
        PasswordOutcome::NoSecondFactor { operator_name } => html_response(
            StatusCode::UNAUTHORIZED,
            pages::login::render(
                &meta,
                &pages::login::LoginState::NoSecondFactor { operator_name },
            ),
        ),
    }
}

async fn login_code_form(State(state): State<ControlState>, _headers: HeaderMap) -> Response {
    let meta = meta(&state, None);
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    html_response(
        StatusCode::OK,
        pages::login::render(&meta, &pages::login::LoginState::StepExpired),
    )
}

async fn login_code_submit(
    State(state): State<ControlState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let meta = meta(&state, None);
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let fields = match parse_form(&body) {
        Ok(fields) => fields,
        Err(reason) => {
            return html_response(
                StatusCode::BAD_REQUEST,
                pages::errors::bad_request(&meta, Some(&reason)),
            )
        }
    };
    let pending_token = fields.get("pending").cloned().unwrap_or_default();
    let code = fields.get("code").cloned().unwrap_or_default();
    let client = auth::client_identifier(
        &headers,
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok()),
    );
    let outcome = auth::code_login(
        &state.operators,
        &state.limiter,
        &state.pending,
        &pending_token,
        &code,
        &client,
        state.now_ms(),
    );
    match outcome {
        CodeOutcome::Unconfigured => unconfigured_response(&state)
            .unwrap_or_else(|| html_response(StatusCode::OK, String::new())),
        CodeOutcome::StepExpired => html_response(
            StatusCode::UNAUTHORIZED,
            pages::login::render(&meta, &pages::login::LoginState::StepExpired),
        ),
        CodeOutcome::RateLimited { until_ms } => html_response(
            StatusCode::TOO_MANY_REQUESTS,
            pages::login::render(&meta, &pages::login::LoginState::RateLimited { until_ms }),
        ),
        CodeOutcome::Rejected {
            pending_token,
            operator_name,
        } => html_response(
            StatusCode::UNAUTHORIZED,
            pages::login::render(
                &meta,
                &pages::login::LoginState::Code {
                    pending_token,
                    operator_name,
                    problem: Some(crate::copy::LOGIN_FAILED),
                },
            ),
        ),
        CodeOutcome::Verified {
            operator_id,
            operator_name,
            role,
            next,
            reauth,
        } => {
            let secure = session::request_is_secure(&headers);
            let correlation = format!("session-{}", &random_hex_32()[..12]);
            if reauth {
                let existing = session::session_id_from_cookie(
                    headers
                        .get(header::COOKIE)
                        .and_then(|value| value.to_str().ok()),
                );
                let refreshed = existing
                    .as_deref()
                    .and_then(|id| state.sessions.refresh_recent_auth(id, state.now_ms()));
                if let Some(emitter) = &state.emitter {
                    emitter
                        .emit_session(&operator_id, "step_up", &correlation)
                        .await;
                }
                if refreshed.is_some() {
                    return redirect(next.as_deref().unwrap_or("/control/"), None);
                }
                // The session ended while re-authenticating: fall through and
                // mint a fresh one rather than stranding the operator.
            }
            let opened = state.sessions.open(
                &operator_id,
                &operator_name,
                role,
                &auth::device_label(&headers),
                state.now_ms(),
            );
            if let Some(emitter) = &state.emitter {
                emitter
                    .emit_session(&operator_id, "sign_in", &correlation)
                    .await;
            }
            redirect(
                next.as_deref().unwrap_or("/control/"),
                Some(session::session_cookie(&opened.id, secure)),
            )
        }
    }
}

async fn logout(
    State(state): State<ControlState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let meta = meta(&state, None);
    let fields = parse_form(&body).unwrap_or_default();
    let Some(view) = session_id_from_headers(&state, &headers) else {
        return redirect("/control/login", None);
    };
    let csrf = fields.get("csrf_token").cloned().unwrap_or_default();
    if !crate::signing::constant_time_eq(csrf.as_bytes(), view.csrf.as_bytes()) {
        return html_response(
            StatusCode::BAD_REQUEST,
            pages::errors::bad_request(
                &meta,
                Some("The logout form's session check failed; no session was closed."),
            ),
        );
    }
    let correlation = format!("session-{}", &random_hex_32()[..12]);
    let cookie = session::session_id_from_cookie(
        headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok()),
    );
    if let Some(id) = cookie {
        state.sessions.close(&id);
    }
    if let Some(emitter) = &state.emitter {
        emitter
            .emit_session(&view.operator_id, "sign_out", &correlation)
            .await;
    }
    redirect("/control/login", Some(session::cleared_session_cookie()))
}

fn session_id_from_headers(state: &ControlState, headers: &HeaderMap) -> Option<SessionView> {
    session::session_id_from_cookie(
        headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok()),
    )
    .and_then(|id| state.sessions.touch(&id, state.now_ms()))
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

async fn action_submit(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let Some(capability) = Capability::parse(&kind) else {
        return html_response(StatusCode::NOT_FOUND, pages::errors::not_found(&meta));
    };
    let fields = match parse_form(&body) {
        Ok(fields) => fields,
        Err(reason) => {
            return html_response(
                StatusCode::BAD_REQUEST,
                pages::errors::bad_request(&meta, Some(&reason)),
            )
        }
    };
    let payload: serde_json::Value = fields
        .iter()
        .filter(|(key, _)| key.starts_with("payload_"))
        .map(|(key, value)| {
            (
                key.trim_start_matches("payload_").to_string(),
                serde_json::Value::String(value.clone()),
            )
        })
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();
    let request = ActionRequest {
        target: fields.get("target").cloned().unwrap_or_default(),
        reason: fields.get("reason").cloned().unwrap_or_default(),
        source_document: fields.get("source_document").cloned().unwrap_or_default(),
        evidence: fields.get("evidence").cloned().unwrap_or_default(),
        confirmation: fields.get("confirmation").cloned().unwrap_or_default(),
        idempotency_key: fields.get("idempotency_key").cloned().unwrap_or_default(),
        version_token: fields.get("version_token").cloned().unwrap_or_default(),
        csrf_token: fields.get("csrf_token").cloned().unwrap_or_default(),
        return_path: fields
            .get("return_path")
            .cloned()
            .unwrap_or_else(|| "/control/".to_string()),
        payload,
        customer_population: fields
            .get("customer_population")
            .cloned()
            .unwrap_or_default(),
        expected_effect: fields.get("expected_effect").cloned().unwrap_or_default(),
        platform: fields.get("platform").cloned(),
        policy_id: fields.get("policy_id").cloned().unwrap_or_default(),
    };
    let recent_auth = session.recent_auth(state.now_ms(), state.sessions.step_up_ms());
    let outcome = crate::actions::submit(
        &state,
        capability,
        &request,
        &session.operator_id,
        session.role,
        &session.csrf,
        recent_auth,
    )
    .await;
    match outcome {
        SubmitOutcome::Duplicate { result_id } => redirect(
            &format!(
                "/control/actions/{}/result/{result_id}",
                capability.as_str()
            ),
            None,
        ),
        SubmitOutcome::CsrfRejected => html_response(
            StatusCode::BAD_REQUEST,
            pages::errors::bad_request(
                &meta,
                Some("The form's session check failed; no action was requested."),
            ),
        ),
        SubmitOutcome::Denied {
            capability,
            target,
            return_path,
        } => html_response(
            StatusCode::FORBIDDEN,
            pages::actions::denied(&meta, capability, &target, &return_path),
        ),
        SubmitOutcome::StepUpRequired { next } => redirect(
            &format!("/control/login?reauth=1&next={}", percent_encode(&next)),
            None,
        ),
        SubmitOutcome::Conflict {
            reason,
            return_path,
        } => html_response(
            StatusCode::CONFLICT,
            pages::errors::conflict(&meta, &reason, &return_path),
        ),
        SubmitOutcome::FormProblem { message } => html_response(
            StatusCode::BAD_REQUEST,
            pages::errors::render(
                &meta,
                "Check the request.",
                &message,
                None,
                &[("Return to the object", &request.return_path)],
            ),
        ),
        SubmitOutcome::Result { result_id, .. } => redirect(
            &format!(
                "/control/actions/{}/result/{result_id}",
                capability.as_str()
            ),
            None,
        ),
    }
}

async fn action_result(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Path((kind, id)): Path<(String, String)>,
) -> Response {
    if let Some(response) = unconfigured_response(&state) {
        return response;
    }
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let meta = meta(&state, Some(&session));
    let Some(capability) = Capability::parse(&kind) else {
        return html_response(StatusCode::NOT_FOUND, pages::errors::not_found(&meta));
    };
    let read = match state.journal.read() {
        Ok(read) => read,
        Err(error) => {
            return html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                pages::errors::internal(&meta, &error.to_string()),
            )
        }
    };
    let Some(entry) = read
        .entries
        .iter()
        .find(|entry| entry.result_id.as_deref() == Some(id.as_str()))
    else {
        return html_response(StatusCode::NOT_FOUND, pages::errors::not_found(&meta));
    };
    let (state_outcome, job_state) = match entry.job_id.as_deref() {
        Some(job_id) => match state.jobs.read(job_id) {
            Ok(crate::jobs::JobRead::Parsed(job)) => {
                let display = job.display_state().to_string();
                let mapped = match job.state.as_str() {
                    "queued" => ActionState::Requested,
                    "running" => ActionState::Running,
                    "completed" => ActionState::Confirmed,
                    "failed" => ActionState::Failed,
                    _ => ActionState::StateUnknown,
                };
                (mapped, Some(display))
            }
            Ok(crate::jobs::JobRead::StateUnknown { reason, .. }) => (
                ActionState::StateUnknown,
                Some(format!("State unknown: {reason}")),
            ),
            Err(error) => (
                ActionState::StateUnknown,
                Some(format!("State unknown: {error}")),
            ),
        },
        None => match entry.outcome.as_str() {
            "confirmed" => (ActionState::Confirmed, None),
            "failed" => (ActionState::Failed, None),
            "requested" => (ActionState::Requested, None),
            _ => (ActionState::StateUnknown, None),
        },
    };
    html_response(
        StatusCode::OK,
        pages::actions::result(
            &meta,
            capability,
            &id,
            state_outcome,
            entry.job_id.as_deref(),
            &format!("/control/actions/{kind}/result/{id}"),
            &entry.reason,
            job_state.as_deref(),
        ),
    )
}

/// Build one pre-filled action form, with a fresh stale-state token observed
/// from the object's current state. Returns `None` when the object is not
/// present, so pages never render a form that cannot pass its own checks.
pub async fn build_form_spec(
    state: &ControlState,
    session: &SessionView,
    capability: Capability,
    target: &str,
    return_path: &str,
    extra: &[(&str, String)],
) -> Option<ActionFormSpec> {
    let request = ActionRequest {
        target: target.to_string(),
        ..ActionRequest::default()
    };
    let (object_key, object_json) = crate::actions::object_state(state, capability, &request)
        .await
        .or_else(|| {
            // Fleet scheduling state is the one object the fixtures and live
            // deployments both always have: the governance register itself.
            if matches!(
                capability,
                Capability::FleetPauseScheduling | Capability::FleetResumeScheduling
            ) {
                Some((
                    "fleet:scheduling".to_string(),
                    serde_json::json!({"paused": state.governance.fleet_scheduling_paused().unwrap_or(false)}),
                ))
            } else {
                None
            }
        })?;
    let token = state.versions.observe(&object_key, &object_json);
    Some(ActionFormSpec {
        action: format!("/control/actions/{}", capability.as_str()),
        label: capability.label().to_string(),
        target: target.to_string(),
        csrf: session.csrf.clone(),
        idempotency_key: random_hex_32(),
        version_token: token.encode(),
        return_path: return_path.to_string(),
        reason_required: capability.requires_reason(),
        source_document: capability.requires_source_document(),
        evidence: capability == Capability::IncidentResolve,
        confirmation: capability.typed_confirmation().map(str::to_string),
        extra: extra
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect(),
        problem: None,
    })
}

async fn not_found(State(state): State<ControlState>, headers: HeaderMap) -> Response {
    let session = session_id_from_headers(&state, &headers);
    let meta = meta(&state, session.as_ref());
    html_response(StatusCode::NOT_FOUND, pages::errors::not_found(&meta))
}

// ---------------------------------------------------------------------------
// Form decoding
// ---------------------------------------------------------------------------

/// Parse an `application/x-www-form-urlencoded` body into a map. A repeated
/// key keeps its first value (action forms have no multi-value fields).
fn parse_form(body: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let text = std::str::from_utf8(body).map_err(|_| "form body is not UTF-8".to_string())?;
    let mut fields = BTreeMap::new();
    for pair in text.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        fields
            .entry(percent_decode(key)?)
            .or_insert(percent_decode(value)?);
    }
    Ok(fields)
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' => {
                let high = bytes
                    .get(index + 1)
                    .and_then(|byte| hex_value(*byte))
                    .ok_or("bad percent escape")?;
                let low = bytes
                    .get(index + 2)
                    .and_then(|byte| hex_value(*byte))
                    .ok_or("bad percent escape")?;
                out.push((high << 4) | low);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "form component is not UTF-8".to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Minimal percent-encoding for a redirect query value.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'/'
            | b'?'
            | b'='
            | b'&' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The role of the actor on one route, exposed for the integration tests.
pub fn role_of(state: &ControlState, operator_id: &str) -> Option<Role> {
    state
        .operators
        .find(operator_id)
        .map(|operator| operator.role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_decoding_handles_percent_escapes_and_plus() {
        let fields = parse_form(b"operator=maya&password=a+b%21&next=%2Fcontrol%2F").expect("form");
        assert_eq!(fields.get("operator").map(String::as_str), Some("maya"));
        assert_eq!(fields.get("password").map(String::as_str), Some("a b!"));
        assert_eq!(fields.get("next").map(String::as_str), Some("/control/"));
        assert!(parse_form(b"%zz=1").is_err());
    }

    #[test]
    fn percent_encode_keeps_paths_and_escapes_spaces() {
        assert_eq!(
            percent_encode("/control/incidents/inc_018"),
            "/control/incidents/inc_018"
        );
        assert_eq!(percent_encode("/control/a b"), "/control/a%20b");
        assert_eq!(percent_encode("/control/a\nb"), "/control/a%0Ab");
    }

    #[test]
    fn duplicate_form_keys_keep_the_first_value() {
        let fields = parse_form(b"target=one&target=two").expect("form");
        assert_eq!(fields.get("target").map(String::as_str), Some("one"));
    }
}
