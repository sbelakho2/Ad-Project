//! The Axum router: every route, its handler and the response discipline.
//!
//! All pages are GET. The explicit POST routes are `/login`, `/logout` and
//! `/actions/{kind}`; an executed action answers `303` to its stable result
//! page, a replayed idempotency key answers `303` to the original result
//! page, and a conflicting key answers `409`. With `APEX_SALES_WEB_TOKEN`
//! unset every page renders the not-configured state and no session can be
//! minted; without a live session the protected pages answer `401` with the
//! sign-in surface. Every response passes
//! [`crate::telemetry::track_requests`], which records the bounded
//! route-template series in the metrics registry.

use axum::extract::{Path, Query, State};
use axum::http::uri::Authority;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;

use apex_sales::store::StateDir;

use crate::actions::{self, ActionError, ActionKind};
use crate::data::{self, RequestData, Target, Transport};
use crate::pages::shared::PageMeta;
use crate::pages::{self, leads::Filters};
use crate::session::{self, SessionView};
use crate::state::WebState;

/// Build the full sales workspace router.
pub fn router(state: WebState) -> Router {
    Router::new()
        .route("/", get(overview))
        .route("/leads", get(leads))
        .route("/leads/export.csv", get(leads_csv))
        .route("/publishers/{domain}", get(publisher))
        .route("/plan", get(plan_page))
        .route("/sends", get(sends))
        .route("/sends/preview/{file}", get(sends_preview))
        .route("/replies", get(replies))
        .route("/domains", get(domains))
        .route("/models", get(models_page))
        .route("/healthz", get(healthz))
        .route("/assets/apex.css", get(crate::assets::css))
        .route("/assets/fonts/{name}", get(crate::assets::font))
        .route("/favicon.ico", get(crate::favicon::ico))
        .route("/favicon.svg", get(crate::favicon::svg))
        .route("/login", get(login_form).post(login_submit))
        .route("/logout", post(logout))
        .route("/actions/{kind}", post(action_submit))
        .route("/actions/result/{id}", get(action_result))
        .fallback(not_found)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::telemetry::track_requests,
        ))
        .with_state(state)
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

/// Where the engine state lives, as every page's meta line states it.
fn location(state: &WebState) -> String {
    format!(
        "{} (publishers.jsonl, contacts.jsonl, events.jsonl, approvals.jsonl, decisions.jsonl)",
        crate::pages::shared::store_label(&state.config.sales_dir)
    )
}

/// The signed-in session, or the response to return instead.
fn require_session(state: &WebState, headers: &HeaderMap) -> Result<SessionView, Box<Response>> {
    if let Some(reason) = &state.unconfigured_reason {
        return Err(Box::new(html_response(
            StatusCode::SERVICE_UNAVAILABLE,
            pages::login::unconfigured(state.now_secs(), reason),
        )));
    }
    let session = session_id_from_headers(state, headers);
    match session {
        Some(view) => Ok(view),
        None => Err(Box::new(html_response(
            StatusCode::UNAUTHORIZED,
            pages::login::session_required(state.now_secs(), &location(state)),
        ))),
    }
}

fn session_id_from_headers(state: &WebState, headers: &HeaderMap) -> Option<SessionView> {
    session::session_id_from_cookie(
        headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok()),
    )
    .and_then(|id| state.sessions.touch(&id, state.now_ms()))
}

/// Load the engine state or the data-unavailable response.
fn load(state: &WebState) -> Result<RequestData, Box<Response>> {
    data::load(state).map_err(|reason| {
        Box::new(html_response(
            StatusCode::SERVICE_UNAVAILABLE,
            pages::errors::data_unavailable(state.now_secs(), &location(state), &reason),
        ))
    })
}

fn meta(
    state: &WebState,
    session: &SessionView,
    title: &str,
    nav: &'static str,
    trail: Vec<String>,
) -> PageMeta {
    PageMeta::new(
        title,
        nav,
        trail,
        state.now_secs(),
        location(state),
        session.csrf.clone(),
    )
}

fn transport(state: &WebState, data: &RequestData) -> Transport {
    data::transport(&state.config.env, &data.dir)
}

// ---------------------------------------------------------------------------
// GET pages
// ---------------------------------------------------------------------------

async fn overview(State(state): State<WebState>, headers: HeaderMap) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    let meta = meta(&state, &session, "Overview", "Overview", Vec::new());
    let transport = transport(&state, &data);
    html_response(
        StatusCode::OK,
        pages::overview::render(&meta, &data, &transport),
    )
}

async fn leads(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    let filters = Filters::from_query(&pairs);
    let meta = meta(&state, &session, "Leads", "Leads", Vec::new());
    html_response(StatusCode::OK, pages::leads::render(&meta, &data, &filters))
}

async fn leads_csv(
    State(state): State<WebState>,
    headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let _ = session;
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    let filters = Filters::from_query(&pairs);
    let csv = pages::leads::export_csv(&data, &filters);
    let mut response = (StatusCode::OK, csv).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"leads.csv\""),
    );
    response
}

async fn publisher(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(domain): Path<String>,
) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    if !data::valid_domain(&domain) || !data.snap.publishers.contains_key(&domain) {
        return html_response(
            StatusCode::NOT_FOUND,
            pages::errors::not_found(state.now_secs(), &location(&state)),
        );
    }
    let meta = meta(&state, &session, &domain, "Leads", vec![domain.clone()]);
    html_response(
        StatusCode::OK,
        pages::publisher::render(&meta, &data, &domain),
    )
}

async fn plan_page(State(state): State<WebState>, headers: HeaderMap) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    let meta = meta(&state, &session, "Plan", "Plan", Vec::new());
    let transport = transport(&state, &data);
    html_response(
        StatusCode::OK,
        pages::plan::render(&meta, &data, &transport),
    )
}

async fn sends(State(state): State<WebState>, headers: HeaderMap) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    let meta = meta(&state, &session, "Sends", "Sends", Vec::new());
    let transport = transport(&state, &data);
    html_response(
        StatusCode::OK,
        pages::sends::render(&meta, &data, &transport),
    )
}

async fn sends_preview(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(file): Path<String>,
) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    if !data::valid_outbox_name(&file) {
        return html_response(
            StatusCode::NOT_FOUND,
            pages::errors::not_found(state.now_secs(), &location(&state)),
        );
    }
    let transport = transport(&state, &data);
    let meta = meta(
        &state,
        &session,
        &format!("Preview {file}"),
        "Sends",
        vec![file.clone()],
    );
    html_response(
        StatusCode::OK,
        pages::sends::preview(&meta, &transport, &file),
    )
}

async fn replies(State(state): State<WebState>, headers: HeaderMap) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    let meta = meta(&state, &session, "Replies", "Replies", Vec::new());
    html_response(StatusCode::OK, pages::replies::render(&meta, &data))
}

async fn domains(State(state): State<WebState>, headers: HeaderMap) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    let meta = meta(&state, &session, "Domains", "Domains", Vec::new());
    html_response(StatusCode::OK, pages::domains::render(&meta, &data))
}

async fn models_page(State(state): State<WebState>, headers: HeaderMap) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let data = match load(&state) {
        Ok(data) => data,
        Err(response) => return *response,
    };
    let meta = meta(&state, &session, "Models", "Models", Vec::new());
    html_response(StatusCode::OK, pages::models::render(&meta, &data))
}

async fn healthz() -> Response {
    let mut response = (StatusCode::OK, "{\"status\":\"ok\"}").into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

async fn not_found(State(state): State<WebState>, headers: HeaderMap) -> Response {
    // A 404 never leaks state: without a session it is the sign-in surface,
    // with one the neutral not-found page.
    let session = session_id_from_headers(&state, &headers);
    match session {
        Some(_) => html_response(
            StatusCode::NOT_FOUND,
            pages::errors::not_found(state.now_secs(), &location(&state)),
        ),
        None => html_response(
            StatusCode::NOT_FOUND,
            pages::login::session_required(state.now_secs(), &location(&state)),
        ),
    }
}

// ---------------------------------------------------------------------------
// Login / logout
// ---------------------------------------------------------------------------

async fn login_form(
    State(state): State<WebState>,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    if let Some(reason) = &state.unconfigured_reason {
        return html_response(
            StatusCode::SERVICE_UNAVAILABLE,
            pages::login::unconfigured(state.now_secs(), reason),
        );
    }
    let signed_out = pairs.iter().any(|(key, _)| key == "signed_out");
    let body = if signed_out {
        pages::login::signed_out(state.now_secs(), &location(&state))
    } else {
        pages::login::form(state.now_secs(), &location(&state), None)
    };
    html_response(StatusCode::OK, body)
}

fn normalized_authority(value: &str, default_port: u16) -> Option<(String, u16)> {
    let authority: Authority = value.trim().parse().ok()?;
    Some((
        authority.host().to_ascii_lowercase(),
        authority.port_u16().unwrap_or(default_port),
    ))
}

/// Same-origin guard for every state-changing POST (`/login`, `/logout` and
/// `/actions/{kind}`).
///
/// Absent `Origin` (tests, curl) is allowed. A present `Origin` must name the
/// same authority as the request's `Host`, must be `https` over TLS, and a
/// `Sec-Fetch-Site: cross-site` hint is always refused. The per-session CSRF
/// token still protects the action forms; this is the independent origin
/// check the browser also supplies.
fn same_origin_allowed(headers: &HeaderMap) -> bool {
    if headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("cross-site"))
    {
        return false;
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let origin = origin.trim();
    if origin.is_empty() || origin.eq_ignore_ascii_case("null") {
        return false;
    }
    let Ok(uri) = origin.parse::<axum::http::Uri>() else {
        return false;
    };
    let Some(scheme) = uri.scheme_str() else {
        return false;
    };
    let default_port = if scheme.eq_ignore_ascii_case("https") {
        443
    } else if scheme.eq_ignore_ascii_case("http") {
        80
    } else {
        return false;
    };
    if session::request_is_secure(headers) && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let Some(origin_authority) = uri.authority() else {
        return false;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    match (
        normalized_authority(origin_authority.as_str(), default_port),
        normalized_authority(host, default_port),
    ) {
        (Some(origin), Some(host)) => origin == host,
        _ => false,
    }
}

async fn login_submit(
    State(state): State<WebState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !same_origin_allowed(&headers) {
        let location = location(&state);
        return html_response(
            StatusCode::FORBIDDEN,
            pages::errors::bad_request(
                state.now_secs(),
                &location,
                "Cross-origin sign-in is not allowed.",
            ),
        );
    }
    let Some(expected) = state.config.token.clone() else {
        let reason = state
            .unconfigured_reason
            .clone()
            .unwrap_or_else(|| "APEX_SALES_WEB_TOKEN is not set".to_string());
        return html_response(
            StatusCode::SERVICE_UNAVAILABLE,
            pages::login::unconfigured(state.now_secs(), &reason),
        );
    };
    let fields = match actions::parse_form(&body) {
        Ok(fields) => fields,
        Err(reason) => {
            return html_response(
                StatusCode::BAD_REQUEST,
                pages::errors::bad_request(state.now_secs(), &location(&state), &reason),
            )
        }
    };
    let token = fields.get("token").cloned().unwrap_or_default();
    if !crate::signing::constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        return html_response(
            StatusCode::UNAUTHORIZED,
            pages::login::form(
                state.now_secs(),
                &location(&state),
                Some(pages::login::LOGIN_FAILED),
            ),
        );
    }
    let opened = state.sessions.open(state.now_ms());
    let secure = session::request_is_secure(&headers);
    redirect("/", Some(session::session_cookie(&opened.id, secure)))
}

async fn logout(
    State(state): State<WebState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(view) = session_id_from_headers(&state, &headers) else {
        return redirect("/login", None);
    };
    if !same_origin_allowed(&headers) {
        return html_response(
            StatusCode::FORBIDDEN,
            pages::errors::bad_request(
                state.now_secs(),
                &location(&state),
                "Cross-origin sign-out is not allowed; the session was not closed.",
            ),
        );
    }
    let fields = actions::parse_form(&body).unwrap_or_default();
    let csrf = fields.get("csrf_token").cloned().unwrap_or_default();
    if !crate::signing::constant_time_eq(csrf.as_bytes(), view.csrf.as_bytes()) {
        return html_response(
            StatusCode::BAD_REQUEST,
            pages::errors::bad_request(
                state.now_secs(),
                &location(&state),
                "The logout form's session check failed; no session was closed.",
            ),
        );
    }
    if let Some(id) = session::session_id_from_cookie(
        headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok()),
    ) {
        state.sessions.close(&id);
    }
    redirect(
        "/login?signed_out=1",
        Some(session::cleared_session_cookie()),
    )
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

async fn action_submit(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(kind): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    if !same_origin_allowed(&headers) {
        return html_response(
            StatusCode::FORBIDDEN,
            pages::errors::bad_request(
                state.now_secs(),
                &location(&state),
                "Cross-origin form submission is not allowed; nothing was written.",
            ),
        );
    }
    let Some(kind) = ActionKind::parse(&kind) else {
        return html_response(
            StatusCode::NOT_FOUND,
            pages::errors::not_found(state.now_secs(), &location(&state)),
        );
    };
    let fields = match actions::parse_form(&body) {
        Ok(fields) => fields,
        Err(reason) => {
            return html_response(
                StatusCode::BAD_REQUEST,
                pages::errors::bad_request(state.now_secs(), &location(&state), &reason),
            )
        }
    };
    let csrf = fields.get("csrf_token").cloned().unwrap_or_default();
    if !crate::signing::constant_time_eq(csrf.as_bytes(), session.csrf.as_bytes()) {
        return html_response(
            StatusCode::BAD_REQUEST,
            pages::errors::bad_request(
                state.now_secs(),
                &location(&state),
                "The form's session check (CSRF token) did not match; nothing was written.",
            ),
        );
    }
    match actions::execute(&state, kind, &fields) {
        Ok((record, _duplicate)) => {
            redirect(&format!("/actions/result/{}", record.operation_id), None)
        }
        Err(ActionError::BadRequest(reason)) => html_response(
            StatusCode::BAD_REQUEST,
            pages::errors::bad_request(state.now_secs(), &location(&state), &reason),
        ),
        Err(ActionError::Conflict) => html_response(
            StatusCode::CONFLICT,
            pages::errors::conflict(state.now_secs(), &location(&state)),
        ),
        Err(ActionError::Io(reason)) => html_response(
            StatusCode::SERVICE_UNAVAILABLE,
            pages::result::failed(state.now_secs(), &location(&state), &reason),
        ),
    }
}

async fn action_result(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let session = match require_session(&state, &headers) {
        Ok(session) => session,
        Err(response) => return *response,
    };
    let _ = session;
    if !crate::signing::is_operation_id(&id) {
        return html_response(
            StatusCode::NOT_FOUND,
            pages::errors::not_found(state.now_secs(), &location(&state)),
        );
    }
    let record = state.with_actions(|memory| memory.outcome(&id).cloned());
    match record {
        Some(record) => html_response(
            StatusCode::OK,
            pages::result::confirmed(state.now_secs(), &location(&state), &record),
        ),
        None => html_response(
            StatusCode::NOT_FOUND,
            pages::errors::not_found(state.now_secs(), &location(&state)),
        ),
    }
}

/// The confirming read link for a written target: the publisher page when
/// the state directory knows the target, else the closest listing.
pub fn confirming_link_for(dir: &StateDir, target: &Target) -> String {
    let snap = dir.load().ok();
    match target {
        Target::Publisher(key) => format!("/publishers/{key}"),
        Target::Email(email) => snap
            .and_then(|snap| {
                snap.contacts
                    .get(email)
                    .map(|contact| format!("/publishers/{}", contact.publisher_key))
            })
            .unwrap_or_else(|| "/replies".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirming_links_point_at_the_publisher_when_known() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-lnk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = StateDir::open(&dir).unwrap();
        store
            .put_contact(&apex_sales::model::Contact {
                email: "a@studio.example".into(),
                name: None,
                role: None,
                publisher_key: "studio.example".into(),
                kind: apex_sales::model::ContactKind::Named,
                source: "s".into(),
                consent: false,
            })
            .unwrap();
        assert_eq!(
            confirming_link_for(&store, &Target::Email("a@studio.example".into())),
            "/publishers/studio.example"
        );
        assert_eq!(
            confirming_link_for(&store, &Target::Email("x@nowhere.example".into())),
            "/replies"
        );
        assert_eq!(
            confirming_link_for(&store, &Target::Publisher("studio.example".into())),
            "/publishers/studio.example"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
