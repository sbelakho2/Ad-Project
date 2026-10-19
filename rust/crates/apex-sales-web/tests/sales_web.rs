#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests over the real sales-workspace router: the owner gate,
//! session and CSRF discipline, the one-stylesheet rule over every route,
//! seeded snapshot pages, the mutating actions with their idempotency, the
//! CSV export, escaping and the preview refusals.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use apex_sales::model::{
    App, Contact, ContactKind, Evidence, EvidenceKind, Outcome, Publisher, ReplyClass, Store, DAY,
};
use apex_sales::store::{Event, StateDir};
use apex_sales_web::config::WebConfig;
use apex_sales_web::routes::router;
use apex_sales_web::state::{Clock, WebState};
use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use tower::ServiceExt;

/// Monday 19 October 2026, 10:00 UTC.
const T0_MS: i64 = (20_745 * DAY + 10 * 3600) * 1000;
const TOKEN: &str = "test-owner-token-0001";

/// A clock the tests can advance (session expiry needs time to move).
#[derive(Debug, Clone)]
struct TestClock(Arc<AtomicI64>);

impl TestClock {
    fn new(now_ms: i64) -> Self {
        Self(Arc::new(AtomicI64::new(now_ms)))
    }

    fn advance(&self, delta_ms: i64) {
        self.0.fetch_add(delta_ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Harness {
    state: WebState,
    clock: TestClock,
    dir: PathBuf,
}

fn harness_with_token(token: Option<&str>) -> Harness {
    let mut extra: Vec<(&str, &str)> = Vec::new();
    if let Some(token) = token {
        extra.push(("APEX_SALES_WEB_TOKEN", token));
    }
    harness_with(&extra)
}

fn harness_with(extra: &[(&str, &str)]) -> Harness {
    let dir = std::env::temp_dir().join(format!(
        "apex-sales-web-it-{}-{}",
        std::process::id(),
        apex_sales_web::signing::random_hex_32()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut env = apex_sales::cli::Env::new();
    env.insert("APEX_SALES_DIR".into(), dir.display().to_string());
    env.insert("APEX_SALES_MODE".into(), "approval".into());
    env.insert("APEX_SALES_HOLDOUT".into(), "0".into());
    env.insert(
        "APEX_SALES_OUTBOX".into(),
        dir.join("outbox").display().to_string(),
    );
    for (key, value) in extra {
        env.insert((*key).to_string(), (*value).to_string());
    }
    let clock = TestClock::new(T0_MS);
    let state =
        WebState::new(WebConfig::from_env(&env).unwrap()).with_clock(Arc::new(clock.clone()));
    Harness { state, clock, dir }
}

impl Harness {
    fn app(&self) -> Router {
        router(self.state.clone())
    }

    /// A signed-in session cookie.
    async fn sign_in(&self) -> String {
        let (status, cookie, _location, _body) =
            post(&self.app(), "/login", None, &format!("token={TOKEN}"), &[]).await;
        assert_eq!(status, StatusCode::SEE_OTHER, "sign-in must redirect");
        cookie.expect("a session cookie")
    }

    fn csrf(&self, cookie: &str) -> String {
        let view = self
            .state
            .sessions
            .touch(session_id(cookie).as_str(), self.state.now_ms())
            .expect("live session");
        view.csrf
    }
}

fn session_id(cookie: &str) -> String {
    cookie
        .split(';')
        .next()
        .and_then(|pair| pair.trim().strip_prefix("apex_sales_session="))
        .expect("session cookie pair")
        .to_string()
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn seed_demo(dir: &Path) {
    let store = StateDir::open(dir).unwrap();
    let mut studio = Publisher::new("studio.example", "Studio Example");
    studio.domain = Some("studio.example".into());
    studio.country = Some("FI".into());
    studio.apps.push(App {
        store: Store::AppStore,
        id: "42".into(),
        name: "Tile Quest".into(),
        genre: Some("Games".into()),
        rating_count: Some(24_000),
        free: Some(true),
        days_since_update: Some(3),
    });
    studio.ads_txt_text = Some("applovin.com, 1, DIRECT\n".into());
    studio.evidence.push(Evidence {
        id: "ads-found".into(),
        kind: EvidenceKind::AppAdsTxt,
        source: "https://studio.example/app-ads.txt".into(),
        statement: "app-ads.txt names AppLovin (DIRECT)".into(),
        observed_at: T0_MS / 1000 - DAY,
    });
    store.put_publisher(&studio).unwrap();

    let mut evil = Publisher::new("evil.example", "<script>Evil</script> & Co");
    evil.domain = Some("evil.example".into());
    evil.apps.push(App {
        store: Store::AppStore,
        id: "7".into(),
        name: "Shady App".into(),
        genre: None,
        rating_count: Some(900_000),
        free: Some(true),
        days_since_update: Some(1),
    });
    store.put_publisher(&evil).unwrap();

    let mut quiet = Publisher::new("quiet.example", "Quiet Studio");
    quiet.domain = Some("quiet.example".into());
    quiet.apps.push(App {
        store: Store::AppStore,
        id: "9".into(),
        name: "Quiet Game".into(),
        genre: Some("Puzzle".into()),
        rating_count: Some(5_000),
        free: Some(true),
        days_since_update: Some(12),
    });
    quiet.evidence.push(Evidence {
        id: "quiet-store".into(),
        kind: EvidenceKind::StoreListing,
        source: "https://apps.apple.com/app/id9".into(),
        statement: "App Store listing shows 5,000 ratings".into(),
        observed_at: T0_MS / 1000 - 2 * DAY,
    });
    store.put_publisher(&quiet).unwrap();

    let mut fresh = Publisher::new("fresh.example", "Fresh Studio");
    fresh.domain = Some("fresh.example".into());
    fresh.country = Some("EE".into());
    fresh.apps.push(App {
        store: Store::AppStore,
        id: "11".into(),
        name: "Fresh Game".into(),
        genre: Some("Arcade".into()),
        rating_count: Some(12_000),
        free: Some(true),
        days_since_update: Some(2),
    });
    fresh.evidence.push(Evidence {
        id: "fresh-ads".into(),
        kind: EvidenceKind::AppAdsTxt,
        source: "https://fresh.example/app-ads.txt".into(),
        statement: "app-ads.txt is present and names Unity LevelPlay (DIRECT)".into(),
        observed_at: T0_MS / 1000 - DAY,
    });
    fresh.ads_txt_text = Some(
        "unity.com, 77, DIRECT
"
        .into(),
    );
    store.put_publisher(&fresh).unwrap();

    for (email, name, role, key, kind, source) in [
        (
            "maria@studio.example",
            Some("Maria"),
            Some("CEO"),
            "studio.example",
            ContactKind::Named,
            "seed list",
        ),
        (
            "spam@evil.example",
            None,
            None,
            "evil.example",
            ContactKind::Generic,
            "https://evil.example/app-ads.txt",
        ),
        (
            "ceo@quiet.example",
            None,
            None,
            "quiet.example",
            ContactKind::Named,
            "inbound",
        ),
        (
            "lena@quiet.example",
            Some("Lena"),
            None,
            "quiet.example",
            ContactKind::Named,
            "inbound",
        ),
        (
            "ada@fresh.example",
            Some("Ada"),
            Some("Growth lead"),
            "fresh.example",
            ContactKind::Named,
            "inbound",
        ),
    ] {
        store
            .put_contact(&Contact {
                email: email.into(),
                name: name.map(str::to_string),
                role: role.map(str::to_string),
                publisher_key: key.into(),
                kind,
                source: source.into(),
                consent: false,
            })
            .unwrap();
    }

    let now = T0_MS / 1000;
    store
        .put_event(&Event::Touch {
            at: now - 3 * DAY,
            email: "maria@studio.example".into(),
            publisher: "studio.example".into(),
            from: None,
            step: 0,
            arm: "t0-finding".into(),
            message_id: "m1".into(),
            local_hour: 10,
            predicted: Some(0.05),
        })
        .unwrap();
    store
        .put_event(&Event::Touch {
            at: now - 2 * DAY,
            email: "ceo@quiet.example".into(),
            publisher: "quiet.example".into(),
            from: None,
            step: 0,
            arm: "t0-finding".into(),
            message_id: "m2".into(),
            local_hour: 11,
            predicted: Some(0.04),
        })
        .unwrap();
    store
        .put_event(&Event::Outcome {
            at: now - 2 * DAY,
            email: Some("maria@studio.example".into()),
            publisher: None,
            outcome: Outcome::Reply(ReplyClass::Positive),
            note: Some("yes please".into()),
        })
        .unwrap();
    store
        .put_event(&Event::Suppress {
            at: now - DAY,
            email: Some("spam@evil.example".into()),
            publisher: None,
        })
        .unwrap();
}

fn seed_quarantine(dir: &Path) {
    let store = StateDir::open(dir).unwrap();
    let now = T0_MS / 1000;
    for index in 0..40 {
        store
            .put_event(&Event::Touch {
                at: now - DAY + index,
                email: format!("c{index}@burst.example"),
                publisher: "burst.example".into(),
                from: None,
                step: 0,
                arm: "t0-finding".into(),
                message_id: format!("m{index}"),
                local_hour: 9,
                predicted: Some(0.04),
            })
            .unwrap();
    }
    for index in 0..5 {
        store
            .put_event(&Event::Outcome {
                at: now - DAY + 100 + index,
                email: Some(format!("c{index}@burst.example")),
                publisher: None,
                outcome: Outcome::Complaint,
                note: None,
            })
            .unwrap();
    }
}

async fn get(app: &Router, uri: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let (status, _headers, body) = get_full(app, uri, cookie, &[]).await;
    (status, body)
}

async fn get_full(
    app: &Router,
    uri: &str,
    cookie: Option<&str>,
    extra: &[(&str, &str)],
) -> (StatusCode, HeaderMap, String) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    for (name, value) in extra {
        builder = builder.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), 4 << 20)
        .await
        .expect("body");
    (status, headers, String::from_utf8_lossy(&body).to_string())
}

async fn post(
    app: &Router,
    uri: &str,
    cookie: Option<&str>,
    form: &str,
    extra: &[(&str, &str)],
) -> (StatusCode, Option<String>, Option<String>, String) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    for (name, value) in extra {
        builder = builder.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(form.to_string())).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = axum::body::to_bytes(response.into_body(), 4 << 20)
        .await
        .expect("body");
    (
        status,
        cookie,
        location,
        String::from_utf8_lossy(&body).to_string(),
    )
}

/// The invariant every HTML route must satisfy: one stylesheet link, no
/// style block, no inline style attribute, no script, exactly one h1.
fn assert_page_invariants(uri: &str, html: &str) {
    assert!(
        !html.contains("<style"),
        "{uri}: must not contain a style block"
    );
    assert!(
        !html.contains("style=\""),
        "{uri}: must not contain an inline style attribute"
    );
    assert!(
        !html.contains("<script"),
        "{uri}: must not contain a script"
    );
    assert_eq!(html.matches("<h1").count(), 1, "{uri}: one h1");
    assert_eq!(
        html.matches("<link rel=\"stylesheet\" href=\"/assets/apex.css\">")
            .count(),
        1,
        "{uri}: exactly one stylesheet link"
    );
}

// ---------------------------------------------------------------------------
// The owner gate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unset_token_fails_closed_on_every_page_and_mints_nothing() {
    let harness = harness_with_token(None);
    let app = harness.app();
    for uri in [
        "/", "/leads", "/plan", "/sends", "/replies", "/domains", "/models", "/login",
    ] {
        let (status, html) = get(&app, uri, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        assert!(
            html.contains("This deployment is not configured"),
            "{uri} must render the not-configured state"
        );
        assert!(html.contains("APEX_SALES_WEB_TOKEN"), "{uri}");
    }
    // Even the right token cannot mint a session while unset.
    let (status, cookie, _location, _body) =
        post(&app, "/login", None, &format!("token={TOKEN}"), &[]).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(cookie.is_none(), "no cookie may be minted");
    assert!(harness.state.sessions.is_empty());
}

#[tokio::test]
async fn a_wrong_token_answers_401_with_the_form_and_no_cookie() {
    let harness = harness_with_token(Some(TOKEN));
    let (status, cookie, _location, body) =
        post(&harness.app(), "/login", None, "token=wrong", &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(cookie.is_none());
    assert!(body.contains("<form method=\"post\" action=\"/login\">"));
    assert!(body.contains("That token was not accepted"));
    assert!(harness.state.sessions.is_empty());
}

#[tokio::test]
async fn login_sets_the_flagged_cookie_and_logout_requires_csrf() {
    let harness = harness_with_token(Some(TOKEN));
    let app = harness.app();
    let (status, cookie, location, _body) =
        post(&app, "/login", None, &format!("token={TOKEN}"), &[]).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/"));
    let cookie = cookie.expect("cookie");
    assert!(cookie.starts_with("apex_sales_session="));
    assert!(cookie.contains("Path=/"));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(cookie.contains("Max-Age=28800"));
    assert!(!cookie.contains("Domain="), "host-only");
    assert!(!cookie.contains("Secure"), "plain HTTP has no Secure");

    // The protected overview renders for the session.
    let (status, html) = get(&app, "/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Overview"));

    // Logout without the CSRF token is refused and the session stays live.
    let (status, _cookie, _location, body) = post(&app, "/logout", Some(&cookie), "", &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("session check failed"));
    assert!(!harness.state.sessions.is_empty());

    // With the right CSRF token the session closes and the cookie clears.
    let csrf = harness.csrf(&cookie);
    let (status, cleared, location, _body) = post(
        &app,
        "/logout",
        Some(&cookie),
        &format!("csrf_token={csrf}"),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/login?signed_out=1"));
    assert!(cleared.unwrap().contains("Max-Age=0"));
    assert!(harness.state.sessions.is_empty());
    let (status, _html) = get(&app, "/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_rejects_cross_origin_and_accepts_same_origin() {
    let harness = harness_with_token(Some(TOKEN));
    let app = harness.app();
    let form = format!("token={TOKEN}");

    let cross = [
        ("host", "sales.example"),
        ("origin", "https://evil.example"),
        ("x-forwarded-proto", "https"),
    ];
    let (status, cookie, _location, html) = post(&app, "/login", None, &form, &cross).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{html}");
    assert!(cookie.is_none(), "a refused origin mints no cookie");

    let (status, cookie, _location, _html) = post(
        &app,
        "/login",
        None,
        &form,
        &[("host", "sales.example"), ("sec-fetch-site", "cross-site")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(cookie.is_none());

    let same = [
        ("host", "sales.example"),
        ("origin", "https://sales.example"),
        ("x-forwarded-proto", "https"),
    ];
    let (status, cookie, location, html) = post(&app, "/login", None, &form, &same).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{html}");
    assert_eq!(location.as_deref(), Some("/"));
    assert!(cookie.is_some(), "the same-origin login mints the session");
}

#[tokio::test]
async fn forwarded_https_marks_the_cookie_secure() {
    let harness = harness_with_token(Some(TOKEN));
    let (status, cookie, _location, _body) = post(
        &harness.app(),
        "/login",
        None,
        &format!("token={TOKEN}"),
        &[("x-forwarded-proto", "https")],
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(cookie.unwrap().ends_with("; Secure"));
}

#[tokio::test]
async fn sessions_expire_on_idle_and_absolutely() {
    let harness = harness_with_token(Some(TOKEN));
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let (status, _html) = get(&app, "/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    harness.clock.advance(61 * 60 * 1000);
    let (status, _html) = get(&app, "/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "idle expiry");
    // A fresh session ends at the 8-hour absolute cap even with activity
    // inside the idle window (a touch every 30 minutes).
    let cookie = harness.sign_in().await;
    for _ in 0..15 {
        harness.clock.advance(30 * 60 * 1000);
        let (status, _html) = get(&app, "/", Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK, "touched within the idle window");
    }
    harness.clock.advance(30 * 60 * 1000);
    let (status, _html) = get(&app, "/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "absolute expiry");
}

#[tokio::test]
async fn protected_pages_answer_401_with_the_sign_in_surface() {
    let harness = harness_with_token(Some(TOKEN));
    let (status, html) = get(&harness.app(), "/leads", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(html.contains("Sign-in required"));
    assert!(html.contains("action=\"/login\""));
    assert_page_invariants("/leads (401)", &html);
}

// ---------------------------------------------------------------------------
// Assets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_stylesheet_is_exactly_console_css() {
    let harness = harness_with_token(Some(TOKEN));
    let (status, headers, body) = get_full(&harness.app(), "/assets/apex.css", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/css; charset=utf-8")
    );
    assert_eq!(
        headers
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("public, max-age=3600")
    );
    assert_eq!(body, apex_ui::tokens::console_css());
}

#[tokio::test]
async fn fonts_and_favicons_serve_the_committed_bytes() {
    let harness = harness_with_token(Some(TOKEN));
    let app = harness.app();
    for name in apex_ui::fonts::FONT_NAMES {
        // The stylesheet URLs carry the extension; that is the request the
        // browser actually makes, so the route must answer it.
        let (status, headers, body) =
            get_full(&app, &format!("/assets/fonts/{name}.woff2"), None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{name}.woff2");
        assert_eq!(
            headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("font/woff2")
        );
        assert!(
            body.starts_with("wOF2"),
            "{name}.woff2 lacks the WOFF2 magic"
        );
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/fonts/manrope-400")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(body.starts_with(b"wOF2"));
    let (status, _html) = get(&app, "/assets/fonts/nope.woff2", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _headers, ico) = get_full(&app, "/favicon.ico", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ico.as_bytes().starts_with(&[0x00, 0x00, 0x01, 0x00]));
    let (status, _headers, svg) = get_full(&app, "/favicon.svg", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(svg.contains("viewBox"));
}

#[tokio::test]
async fn healthz_answers_ok() {
    let harness = harness_with_token(Some(TOKEN));
    let (status, headers, body) = get_full(&harness.app(), "/healthz", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "{\"status\":\"ok\"}");
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
}

// ---------------------------------------------------------------------------
// The no-inline sweep over every route
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_route_carries_the_single_stylesheet_and_no_inline_style() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    std::fs::create_dir_all(harness.dir.join("outbox")).unwrap();
    std::fs::write(
        harness.dir.join("outbox/preview1.eml"),
        "Subject: <script>alert(1)</script>\n\nbody",
    )
    .unwrap();
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let routes = [
        "/",
        "/leads",
        "/leads?status=suppressed&source=app-ads&platform=max&band=core",
        "/publishers/studio.example",
        "/publishers/evil.example",
        "/plan",
        "/sends",
        "/sends/preview/preview1.eml",
        "/replies",
        "/domains",
        "/models",
        "/login",
        "/actions/result/op_0123456789ab",
        "/does-not-exist",
        "/leads?status=%3Cscript%3E",
    ];
    for uri in routes {
        let (status, html) = get(&app, uri, Some(&cookie)).await;
        assert!(
            status == StatusCode::OK || status == StatusCode::NOT_FOUND,
            "{uri}: {status}"
        );
        assert_page_invariants(uri, &html);
    }
    // The sign-in surface without a cookie.
    let (status, html) = get(&app, "/login", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_page_invariants("/login (anonymous)", &html);
    // The 401 surface.
    let (status, html) = get(&app, "/plan", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_page_invariants("/plan (401)", &html);
    // The not-configured surface.
    let harness = harness_with_token(None);
    let (status, html) = get(&harness.app(), "/", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_page_invariants("/ (unconfigured)", &html);
}

// ---------------------------------------------------------------------------
// Seeded snapshot pages
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_overview_shows_the_kpis_and_the_chart() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let (status, html) = get(&app, "/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Publishers tracked"));
    assert!(html.contains(">4<"), "four publishers");
    assert!(html.contains("Active contacts"));
    assert!(html.contains("1 suppressed of 5 total"));
    assert!(html.contains("Interested replies"));
    assert!(html.contains(">1/1<"), "one interested of one classified");
    assert!(html.contains("Holdout share"));
    assert!(html.contains("Sending domain"));
    assert!(html.contains("Sends per day"), "the bar chart is present");
    assert!(html.contains("chart-table"), "the adjacent data table");
    assert!(html.contains("as of 19 Oct 2026, 10:00 UTC"));
}

#[tokio::test]
async fn the_leads_page_filters_and_counts() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let (status, html) = get(&app, "/leads", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("/publishers/studio.example"));
    assert!(html.contains("/publishers/evil.example"));
    assert!(html.contains("Suppressed (operator block)"));
    assert!(html.contains("5 contacts"));
    assert!(html.contains("Not available (no app-ads.txt)"));
    assert!(html.contains("Filters") || html.contains("filter-bar"));
    // Filtering by status narrows to the suppressed contact.
    let (status, html) = get(&app, "/leads?status=suppressed", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Showing 1 of 5 contacts"));
    assert!(html.contains("/publishers/evil.example"));
    assert!(!html.contains("/publishers/studio.example"));
    // Hostile filter values fall back safely.
    let (status, html) = get(&app, "/leads?status=%3Cscript%3E", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("5 contacts"));
    assert_page_invariants("/leads (hostile filter)", &html);
}

#[tokio::test]
async fn the_csv_export_is_the_filtered_view() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let (status, headers, csv) = get_full(&app, "/leads/export.csv", Some(&cookie), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/csv; charset=utf-8")
    );
    assert!(csv.ends_with("\r\n"));
    assert_eq!(csv.matches("\r\n").count(), 6, "header + five rows");
    assert!(csv.contains("<script>Evil</script> & Co"));
    let (status, _headers, filtered) = get_full(
        &app,
        "/leads/export.csv?status=suppressed",
        Some(&cookie),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(filtered.contains("spam@evil.example"));
    assert!(!filtered.contains("maria@studio.example"));
}

#[tokio::test]
async fn the_publisher_page_shows_events_forms_and_escapes_hostile_names() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let (status, html) = get(&app, "/publishers/evil.example", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!html.contains("<script>"), "hostile name must be escaped");
    assert!(html.contains("&lt;script&gt;Evil&lt;/script&gt; &amp; Co"));
    assert!(html.contains("Some addresses at this publisher are suppressed"));
    assert!(html.contains("operator block"));
    assert!(html.contains("action=\"/actions/suppress\""));
    assert!(html.contains("action=\"/actions/record\""));
    assert!(html.contains("Suppressed by operator"));
    // The rich publisher shows the timeline and the score breakdown.
    let (status, html) = get(&app, "/publishers/studio.example", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("app-ads.txt names AppLovin"));
    assert!(html.contains("Touch sent (step 1)"));
    assert!(html.contains("Score breakdown by feature contribution"));
}

#[tokio::test]
async fn hostile_domains_are_refused_with_a_404_without_reflection() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    for uri in [
        "/publishers/%3Cscript%3Ealert(1)%3C/script%3E",
        "/publishers/a%2F..%2Fb",
        "/publishers/UPPER.example",
        "/publishers/",
    ] {
        let (status, html) = get(&app, uri, Some(&cookie)).await;
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::UNAUTHORIZED,
            "{uri}: {status}"
        );
        assert!(!html.contains("<script>"), "{uri}");
    }
}

#[tokio::test]
async fn empty_store_pages_state_their_reasons() {
    let harness = harness_with_token(Some(TOKEN));
    let app = harness.app();
    let cookie = harness.sign_in().await;
    for (uri, needle) in [
        ("/", "Import the first leads"),
        ("/leads", "No contacts are tracked yet"),
        ("/plan", "There is no plan yet"),
        ("/sends", "No sends are recorded yet"),
        ("/replies", "No responses are recorded yet"),
        (
            "/models",
            "Not available (no settled first-touch predictions yet)",
        ),
        ("/domains", "Not available (no observations)"),
    ] {
        let (status, html) = get(&app, uri, Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(
            html.contains(needle),
            "{uri} must state its reason: {needle}"
        );
    }
}

#[tokio::test]
async fn the_quarantined_domain_shows_banner_and_state() {
    let harness = harness_with_token(Some(TOKEN));
    seed_quarantine(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let (status, html) = get(&app, "/domains", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("quarantined"));
    assert!(html.contains("Quarantined"));
    assert!(html.contains("RESET HEALTH"));
    let (status, html) = get(&app, "/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("The sending domain is quarantined"));
}

#[tokio::test]
async fn a_multi_domain_deployment_renders_isolated_domains_and_the_spread() {
    let harness = harness_with(&[
        ("APEX_SALES_WEB_TOKEN", TOKEN),
        (
            "APEX_SALES_SENDERS",
            "aaron@one.example|Aaron|Apex|Tallinn;dana@two.example|Dana|Apex|Tallinn",
        ),
    ]);
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    // Both domains are listed, one row each.
    let (status, html) = get(&app, "/domains", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Sending domains"), "{html}");
    assert!(html.contains("one.example") && html.contains("two.example"));
    assert!(html.contains("Warming"), "both start warming");
    // The send history gains the From column.
    let (status, html) = get(&app, "/sends", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(">From<"), "the history shows the domain");
    // The plan spreads across the domains and shows where each touch goes.
    let (status, html) = get(&app, "/plan", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(">From<"), "the plan shows the domain");
    assert!(
        html.contains("one.example") || html.contains("two.example"),
        "a planned touch names its sending domain"
    );
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn extract_hidden(html: &str, name: &str) -> String {
    let needle = format!("name=\"{name}\" value=\"");
    let start = html.find(&needle).expect("hidden field present") + needle.len();
    let rest = &html[start..];
    let end = rest.find('"').expect("value terminator");
    rest[..end].to_string()
}

#[tokio::test]
async fn approve_writes_exactly_one_approval_and_replays_idempotently() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let (status, html) = get(&app, "/plan", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Approval mode"));
    assert!(html.contains("Approve planned touches"));
    assert!(html.contains("ada@fresh.example"));
    let touch_id = extract_hidden(&html, "touch_id");
    let idem = extract_hidden(&html, "idempotency_key");
    let csrf = harness.csrf(&cookie);
    let form = format!("touch_id={touch_id}&idempotency_key={idem}&csrf_token={csrf}");
    let (status, _cookie, location, _body) =
        post(&app, "/actions/approve", Some(&cookie), &form, &[]).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = location.expect("a result redirect");
    assert!(location.starts_with("/actions/result/op_"), "{location}");
    let approvals = std::fs::read_to_string(harness.dir.join("approvals.jsonl")).unwrap();
    assert_eq!(approvals.lines().count(), 1);
    assert!(approvals.contains(&touch_id));
    // The result page confirms with the operation id and a read link.
    let (status, html) = get(&app, &location, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Confirmed"));
    assert!(html.contains("href=\"/plan\""));
    // The replay answers the same result and writes nothing new.
    let (status, _cookie, again, _body) =
        post(&app, "/actions/approve", Some(&cookie), &form, &[]).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(again.as_deref(), Some(location.as_str()));
    assert_eq!(
        std::fs::read_to_string(harness.dir.join("approvals.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1,
        "a replay must not duplicate the approval"
    );
    // The approved touch now shows as approved on the plan page.
    let (status, html) = get(&app, "/plan", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Approved"), "the verdict flips to Approved");
}

#[tokio::test]
async fn actions_require_the_session_csrf_token() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let (status, _html) = get(&app, "/plan", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let form = format!(
        "touch_id=t0123456789abcdef&idempotency_key={}&csrf_token=wrong",
        "a".repeat(32)
    );
    let (status, _cookie, _location, body) =
        post(&app, "/actions/approve", Some(&cookie), &form, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("CSRF token"));
    assert!(!harness.dir.join("approvals.jsonl").exists());
}

#[tokio::test]
async fn suppress_writes_the_implied_event_through_the_web() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let csrf = harness.csrf(&cookie);
    let form = format!(
        "target=maria%40studio.example&reason=opt_out&idempotency_key={}&csrf_token={csrf}",
        "b".repeat(32)
    );
    let (status, _cookie, location, _body) =
        post(&app, "/actions/suppress", Some(&cookie), &form, &[]).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location.unwrap().starts_with("/actions/result/op_"));
    let events = std::fs::read_to_string(harness.dir.join("events.jsonl")).unwrap();
    assert!(events.contains("\"outcome\":\"unsubscribe\""), "{events}");
    // The publisher page now shows the suppression from the rebuilt state.
    let (status, html) = get(&app, "/publishers/studio.example", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Some addresses at this publisher are suppressed"));
    assert!(html.contains("opted out"));
}

#[tokio::test]
async fn record_outcome_is_bounded_and_idempotent() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let csrf = harness.csrf(&cookie);
    let long_note = "x".repeat(501);
    let form = format!(
        "target=maria%40studio.example&outcome=reply%3Aquestion&note={long_note}&idempotency_key={}&csrf_token={csrf}",
        "c".repeat(32)
    );
    let (status, _cookie, _location, body) =
        post(&app, "/actions/record", Some(&cookie), &form, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("longer than 500"));
    // A 500-char note is accepted once.
    let note = "y".repeat(500);
    let form = format!(
        "target=maria%40studio.example&outcome=reply%3Aquestion&note={note}&idempotency_key={}&csrf_token={csrf}",
        "d".repeat(32)
    );
    let (status, _cookie, location, _body) =
        post(&app, "/actions/record", Some(&cookie), &form, &[]).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let events_after_first = std::fs::read_to_string(harness.dir.join("events.jsonl")).unwrap();
    let count_after_first = events_after_first.lines().count();
    assert!(events_after_first.contains("reply:question"));
    // Replay: no second line.
    let (status, _cookie, again, _body) =
        post(&app, "/actions/record", Some(&cookie), &form, &[]).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(again, location);
    assert_eq!(
        std::fs::read_to_string(harness.dir.join("events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        count_after_first
    );
}

#[tokio::test]
async fn reset_health_requires_the_exact_phrase_and_links_back() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let csrf = harness.csrf(&cookie);
    let form = format!(
        "confirmation=reset+health&idempotency_key={}&csrf_token={csrf}",
        "e".repeat(32)
    );
    let (status, _cookie, _location, body) =
        post(&app, "/actions/reset-health", Some(&cookie), &form, &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("RESET HEALTH"));
    let form = format!(
        "confirmation=RESET+HEALTH&idempotency_key={}&csrf_token={csrf}",
        "f".repeat(32)
    );
    let (status, _cookie, location, _body) =
        post(&app, "/actions/reset-health", Some(&cookie), &form, &[]).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = location.unwrap();
    let (status, html) = get(&app, &location, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("href=\"/domains\""));
    let events = std::fs::read_to_string(harness.dir.join("events.jsonl")).unwrap();
    assert!(events.contains("\"type\":\"health_reset\""));
    // The domains page lists the reset in its history.
    let (status, html) = get(&app, "/domains", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Operator reset after an investigation"));
}

#[tokio::test]
async fn concurrent_same_key_submissions_write_exactly_one_event() {
    // The live dogfood harness catches this class: claim and remember used
    // to run in separate lock acquisitions, so sixteen simultaneous replays
    // of one form all claimed Fresh and wrote sixteen durable events.
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let csrf = harness.csrf(&cookie);
    let form = format!(
        "confirmation=RESET+HEALTH&idempotency_key={}&csrf_token={csrf}",
        "a".repeat(32)
    );
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let app = app.clone();
        let cookie = cookie.clone();
        let form = form.clone();
        tasks.push(tokio::spawn(async move {
            post(&app, "/actions/reset-health", Some(&cookie), &form, &[])
                .await
                .0
        }));
    }
    let mut statuses = Vec::new();
    for task in tasks {
        statuses.push(task.await.expect("join"));
    }
    assert!(
        statuses
            .iter()
            .all(|status| *status == StatusCode::SEE_OTHER),
        "every concurrent replay answers the result redirect: {statuses:?}"
    );
    let events = std::fs::read_to_string(harness.dir.join("events.jsonl")).unwrap();
    let resets = events
        .lines()
        .filter(|line| line.contains("\"type\":\"health_reset\""))
        .count();
    assert_eq!(resets, 1, "exactly one durable reset event: {events}");
}

#[tokio::test]
async fn an_idempotency_conflict_answers_409() {
    let harness = harness_with_token(Some(TOKEN));
    seed_demo(&harness.dir);
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let csrf = harness.csrf(&cookie);
    let key = "9".repeat(32);
    let first = format!("touch_id=t0123456789abcdef&idempotency_key={key}&csrf_token={csrf}");
    let (status, _cookie, _location, _body) =
        post(&app, "/actions/approve", Some(&cookie), &first, &[]).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let conflicting = format!("touch_id=tffffffffffffffff&idempotency_key={key}&csrf_token={csrf}");
    let (status, _cookie, _location, body) =
        post(&app, "/actions/approve", Some(&cookie), &conflicting, &[]).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("already used for a different action"));
}

// ---------------------------------------------------------------------------
// The outbox preview
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_preview_escapes_content_and_refuses_traversal() {
    let harness = harness_with_token(Some(TOKEN));
    let outbox = harness.dir.join("outbox");
    std::fs::create_dir_all(&outbox).unwrap();
    std::fs::write(
        outbox.join("m1.eml"),
        "Subject: <script>alert(1)</script>\n\nHi & bye",
    )
    .unwrap();
    std::fs::write(harness.dir.join("secret.eml"), "top secret").unwrap();
    let app = harness.app();
    let cookie = harness.sign_in().await;
    let (status, html) = get(&app, "/sends/preview/m1.eml", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!html.contains("<script>"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("Hi &amp; bye"));
    for uri in [
        "/sends/preview/..%2Fsecret.eml",
        "/sends/preview/.eml",
        "/sends/preview/a..b.eml",
        "/sends/preview/%2E%2E%2F%2E%2E%2Fetc%2Fpasswd",
    ] {
        let (status, html) = get(&app, uri, Some(&cookie)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(!html.contains("top secret"), "{uri}");
        assert!(!html.contains("passwd"), "{uri}");
    }
    // A missing (but well-formed) name answers the preview page's refusal.
    let (status, html) = get(&app, "/sends/preview/missing.eml", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Preview not available"));
    assert!(html.contains("Not available (no outbox file named missing.eml)"));
}

// ---------------------------------------------------------------------------
// Metrics middleware
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_router_records_bounded_route_metrics() {
    let harness = harness_with_token(Some(TOKEN));
    let app = harness.app();
    let (status, _html) = get(&app, "/", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let family = harness
        .state
        .metrics
        .family_snapshot("apex_http_requests_total")
        .expect("request series");
    let labels: Vec<(&str, &str)> = family.series[0]
        .labels
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    assert_eq!(
        labels,
        vec![
            ("service", "apex-sales-web"),
            ("route_template", "/"),
            ("method", "GET"),
            ("status_class", "4xx"),
        ]
    );
}

// ---------------------------------------------------------------------------
// The entry points the binaries call
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_fixture_entry_point_renders_and_validates_the_page_listing() {
    let out =
        std::env::temp_dir().join(format!("apex-sales-web-it-fixtures-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let run =
        apex_sales_web::fixture_render::run_cli(&[out.display().to_string()]).expect("fixture run");
    assert_eq!(run.out_dir, out);
    assert_eq!(
        run.files.len(),
        apex_sales_web::fixture_render::PAGE_NAMES.len()
    );
    for path in &run.files {
        let name = path.file_name().and_then(|n| n.to_str()).expect("name");
        assert!(
            apex_sales_web::fixture_render::PAGE_NAMES.contains(&name),
            "{name} is not listed in PAGES"
        );
        let html = std::fs::read_to_string(path).expect("read page");
        assert!(html.starts_with("<!DOCTYPE html>"), "{}", path.display());
    }
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(apex_sales_web::fixture_render::fixture_work_dir());
}

#[tokio::test]
async fn the_startup_line_and_serve_entry_are_wired_for_the_binary() {
    // The not-configured startup line keeps the fail-closed contract visible.
    let harness = harness_with_token(None);
    let line = apex_sales_web::entry::startup_line(&harness.state);
    assert!(line.contains("UNCONFIGURED"), "{line}");
    assert!(line.contains("no session can be minted"), "{line}");

    // The shared serve entry answers on a real listener until its shutdown
    // future resolves.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let addr = listener.local_addr().expect("addr");
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(apex_sales_web::entry::serve(
        listener,
        harness.state.clone(),
        async move {
            let _ = shutdown_rx.await;
        },
    ));
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("write");
    let mut response = String::new();
    stream.read_to_string(&mut response).await.expect("read");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("ok"), "{response}");
    shutdown_tx.send(()).expect("shutdown");
    server.await.expect("server task").expect("serve");
}

#[tokio::test]
async fn telemetry_helpers_export_the_registry_from_the_environment() {
    use apex_sales_web::telemetry;

    let dir =
        std::env::temp_dir().join(format!("apex-sales-web-it-metrics-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("sales-web.prom");

    std::env::remove_var("APEX_METRICS_TEXTFILE");
    assert!(!telemetry::textfile_configured());
    std::env::set_var("APEX_METRICS_TEXTFILE", path.as_os_str());
    assert!(telemetry::textfile_configured());

    let harness = harness_with_token(Some(TOKEN));
    let (status, _html) = get(&harness.app(), "/healthz", None).await;
    assert_eq!(status, StatusCode::OK);
    telemetry::export_textfile(&harness.state.metrics);
    let contents = std::fs::read_to_string(&path).expect("export");
    assert!(contents.contains("apex_http_requests_total"), "{contents}");

    std::env::remove_var("APEX_METRICS_TEXTFILE");
    let _ = std::fs::remove_dir_all(&dir);
}
