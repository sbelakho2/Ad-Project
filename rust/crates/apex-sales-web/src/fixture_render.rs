//! The fixture renderer: every page and every state of the sales workspace,
//! rendered from a seeded engine state directory into one directory so the
//! `console-a11y` gate can scan real HTML.
//!
//! `render-fixtures` calls [`render_all`]; the integration tests sweep the
//! output: every file carries exactly one `<h1>`, no `<script>`, no
//! `<style>` block and no `style="` attribute. The fixture state is seeded
//! through the engine's own model and store APIs only, so the pages render
//! the same way they render over a live directory.

use std::path::{Path, PathBuf};

use apex_sales::cli::Env;
use apex_sales::decision::Verdict;
use apex_sales::engine::plan;
use apex_sales::model::{
    App, Contact, ContactKind, Evidence, EvidenceKind, Outcome, Publisher, ReplyClass, Store, DAY,
};
use apex_sales::store::{Event, StateDir};

use crate::data::{self, RequestData};
use crate::pages::{self, leads::Filters, shared::PageMeta};

/// The fixture "now": Monday 19 October 2026, 10:00 UTC.
pub const FIXTURE_NOW: i64 = 20_745 * DAY + 10 * 3600;
/// The fixture session's CSRF token (hex-shaped, non-secret).
pub const FIXTURE_CSRF: &str = "0123456789abcdef0123456789abcdef";

/// Render every fixture page; returns the written paths.
pub fn render_all(out: &Path) -> Result<Vec<PathBuf>, String> {
    let work = fixture_work_dir();
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|error| format!("fixture dir: {error}"))?;
    let mut files = Vec::new();

    let rich_dir = work.join("rich");
    seed_rich(&rich_dir)?;
    let rich = load(&rich_dir, &[], FIXTURE_NOW)?;
    let csrf = FIXTURE_CSRF.to_string();

    // Overview, leads, sends, replies, models, publisher pages.
    let meta = page_meta("Overview", "Overview", Vec::new(), &rich, &csrf);
    write(
        out,
        &mut files,
        "overview.html",
        pages::overview::render(&meta, &rich, &data::transport(&Env::new(), &rich.dir)),
    )?;
    let meta = page_meta("Leads", "Leads", Vec::new(), &rich, &csrf);
    write(
        out,
        &mut files,
        "leads.html",
        pages::leads::render(&meta, &rich, &Filters::default()),
    )?;
    let filtered = Filters::from_query(&[
        ("source".to_string(), "seed".to_string()),
        ("band".to_string(), "core".to_string()),
    ]);
    let meta = page_meta("Leads", "Leads", Vec::new(), &rich, &csrf);
    write(
        out,
        &mut files,
        "leads-filtered.html",
        pages::leads::render(&meta, &rich, &filtered),
    )?;
    let meta = page_meta(
        "studio.example",
        "Leads",
        vec!["studio.example".to_string()],
        &rich,
        &csrf,
    );
    write(
        out,
        &mut files,
        "publisher.html",
        pages::publisher::render(&meta, &rich, "studio.example"),
    )?;
    let meta = page_meta(
        "bad.example",
        "Leads",
        vec!["bad.example".to_string()],
        &rich,
        &csrf,
    );
    write(
        out,
        &mut files,
        "publisher-suppressed.html",
        pages::publisher::render(&meta, &rich, "bad.example"),
    )?;
    let shadow_meta = page_meta("Plan", "Plan", Vec::new(), &rich, &csrf);
    let shadow_transport = data::Transport::Outbox(rich.dir.join("outbox"));
    write(
        out,
        &mut files,
        "plan-shadow.html",
        pages::plan::render(&shadow_meta, &rich, &shadow_transport),
    )?;
    let meta = page_meta("Sends", "Sends", Vec::new(), &rich, &csrf);
    write(
        out,
        &mut files,
        "sends.html",
        pages::sends::render(&meta, &rich, &shadow_transport),
    )?;
    let meta = page_meta(
        "Preview 20261019-step1-studio.example.eml",
        "Sends",
        vec!["20261019-step1-studio.example.eml".to_string()],
        &rich,
        &csrf,
    );
    write(
        out,
        &mut files,
        "sends-preview.html",
        pages::sends::preview(
            &meta,
            &shadow_transport,
            "20261019-step1-studio.example.eml",
        ),
    )?;
    let meta = page_meta("Sends", "Sends", Vec::new(), &rich, &csrf);
    write(
        out,
        &mut files,
        "sends-sendmail.html",
        pages::sends::render(&meta, &rich, &data::Transport::Sendmail),
    )?;
    let meta = page_meta("Replies", "Replies", Vec::new(), &rich, &csrf);
    write(
        out,
        &mut files,
        "replies.html",
        pages::replies::render(&meta, &rich),
    )?;
    let meta = page_meta("Domains", "Domains", Vec::new(), &rich, &csrf);
    write(
        out,
        &mut files,
        "domains.html",
        pages::domains::render(&meta, &rich),
    )?;
    let meta = page_meta("Models", "Models", Vec::new(), &rich, &csrf);
    write(
        out,
        &mut files,
        "models.html",
        pages::models::render(&meta, &rich),
    )?;

    // Approval mode: the page with approve forms.
    let approval = load(&rich_dir, &[("APEX_SALES_MODE", "approval")], FIXTURE_NOW)?;
    let meta = page_meta("Plan", "Plan", Vec::new(), &approval, &csrf);
    write(
        out,
        &mut files,
        "plan-approval.html",
        pages::plan::render(&meta, &approval, &data::Transport::Sendmail),
    )?;

    // Autonomous mode.
    let autonomous = load(&rich_dir, &[("APEX_SALES_MODE", "autonomous")], FIXTURE_NOW)?;
    let meta = page_meta("Plan", "Plan", Vec::new(), &autonomous, &csrf);
    write(
        out,
        &mut files,
        "plan-autonomous.html",
        pages::plan::render(&meta, &autonomous, &data::Transport::Sendmail),
    )?;

    // Quarantined sending domain.
    let quarantine_dir = work.join("quarantine");
    seed_complaint_burst(&quarantine_dir)?;
    let quarantine = load(&quarantine_dir, &[], FIXTURE_NOW)?;
    let meta = page_meta("Overview", "Overview", Vec::new(), &quarantine, &csrf);
    write(
        out,
        &mut files,
        "overview-quarantine.html",
        pages::overview::render(&meta, &quarantine, &data::Transport::Sendmail),
    )?;
    let meta = page_meta("Domains", "Domains", Vec::new(), &quarantine, &csrf);
    write(
        out,
        &mut files,
        "domains-quarantine.html",
        pages::domains::render(&meta, &quarantine),
    )?;

    // Paused sending domain.
    let paused_dir = work.join("paused");
    seed_bounce_burst(&paused_dir)?;
    let paused = load(&paused_dir, &[], FIXTURE_NOW)?;
    let meta = page_meta("Domains", "Domains", Vec::new(), &paused, &csrf);
    write(
        out,
        &mut files,
        "domains-paused.html",
        pages::domains::render(&meta, &paused),
    )?;

    // Multi-domain configuration: two sending domains, one quarantined, the
    // other clean, with the day's plan spread across them.
    let multi_dir = work.join("multi");
    seed_multi(&multi_dir)?;
    let multi = load(
        &multi_dir,
        &[(
            "APEX_SALES_SENDERS",
            "aaron@one.example|Aaron at Apex|Apex OÜ|Tallinn, Estonia;dana@two.example|Dana at Apex|Apex OÜ|Tallinn, Estonia",
        )],
        FIXTURE_NOW,
    )?;
    let meta = page_meta("Domains", "Domains", Vec::new(), &multi, &csrf);
    write(
        out,
        &mut files,
        "domains-multi.html",
        pages::domains::render(&meta, &multi),
    )?;
    let meta = page_meta("Plan", "Plan", Vec::new(), &multi, &csrf);
    write(
        out,
        &mut files,
        "plan-multi.html",
        pages::plan::render(&meta, &multi, &data::Transport::Sendmail),
    )?;

    // Empty state: every page with an empty store.
    let empty_dir = work.join("empty");
    let empty = load(&empty_dir, &[], FIXTURE_NOW)?;
    for (name, html) in empty_pages(&empty, &csrf) {
        write(out, &mut files, &name, html)?;
    }

    // Sign-in surfaces.
    write(
        out,
        &mut files,
        "login.html",
        pages::login::form(FIXTURE_NOW, ".apex/sales (fixture)", None),
    )?;
    write(
        out,
        &mut files,
        "login-failed.html",
        pages::login::form(
            FIXTURE_NOW,
            ".apex/sales (fixture)",
            Some(pages::login::LOGIN_FAILED),
        ),
    )?;
    write(
        out,
        &mut files,
        "login-unconfigured.html",
        pages::login::unconfigured(FIXTURE_NOW, "APEX_SALES_WEB_TOKEN is not set"),
    )?;
    write(
        out,
        &mut files,
        "login-session-required.html",
        pages::login::session_required(FIXTURE_NOW, ".apex/sales (fixture)"),
    )?;
    write(
        out,
        &mut files,
        "login-signed-out.html",
        pages::login::signed_out(FIXTURE_NOW, ".apex/sales (fixture)"),
    )?;

    // Error and result pages.
    write(
        out,
        &mut files,
        "error-not-found.html",
        pages::errors::not_found(FIXTURE_NOW, ".apex/sales (fixture)"),
    )?;
    write(
        out,
        &mut files,
        "error-bad-request.html",
        pages::errors::bad_request(
            FIXTURE_NOW,
            ".apex/sales (fixture)",
            "The confirmation must be exactly RESET HEALTH; nothing was written.",
        ),
    )?;
    write(
        out,
        &mut files,
        "error-conflict.html",
        pages::errors::conflict(FIXTURE_NOW, ".apex/sales (fixture)"),
    )?;
    write(
        out,
        &mut files,
        "error-data-unavailable.html",
        pages::errors::data_unavailable(
            FIXTURE_NOW,
            ".apex/sales (fixture)",
            "permission denied opening the state directory",
        ),
    )?;
    let record = crate::state::ActionResultRecord {
        operation_id: "op_0123456789ab".to_string(),
        action: "suppress".to_string(),
        target: "spam@evil.example".to_string(),
        detail: "Suppressed spam@evil.example (spam complaint); the engine rebuilds the suppression from this event on its next run".to_string(),
        at_ms: FIXTURE_NOW * 1000,
        confirmed_link: "/publishers/evil.example".to_string(),
    };
    write(
        out,
        &mut files,
        "result-confirmed.html",
        pages::result::confirmed(FIXTURE_NOW, ".apex/sales (fixture)", &record),
    )?;
    write(
        out,
        &mut files,
        "result-failed.html",
        pages::result::failed(
            FIXTURE_NOW,
            ".apex/sales (fixture)",
            "the state directory refused the write (read-only file system)",
        ),
    )?;

    // The traversal-refused preview shows the not-found page the route
    // answers with for a hostile file name.
    write(
        out,
        &mut files,
        "sends-preview-refused.html",
        pages::errors::not_found(FIXTURE_NOW, ".apex/sales (fixture)"),
    )?;

    Ok(files)
}

/// Every fixture page this renderer publishes, in the order the a11y gate
/// scans them. [`run_cli`] refuses to report success unless the renderer
/// wrote exactly this list.
pub const PAGE_NAMES: [&str; 38] = [
    "domains-empty.html",
    "domains-multi.html",
    "domains-paused.html",
    "domains-quarantine.html",
    "domains.html",
    "error-bad-request.html",
    "error-conflict.html",
    "error-data-unavailable.html",
    "error-not-found.html",
    "leads-empty.html",
    "leads-filtered.html",
    "leads.html",
    "login-failed.html",
    "login-session-required.html",
    "login-signed-out.html",
    "login-unconfigured.html",
    "login.html",
    "models-empty.html",
    "models.html",
    "overview-empty.html",
    "overview-quarantine.html",
    "overview.html",
    "plan-approval.html",
    "plan-autonomous.html",
    "plan-empty.html",
    "plan-multi.html",
    "plan-shadow.html",
    "publisher-suppressed.html",
    "publisher.html",
    "replies-empty.html",
    "replies.html",
    "result-confirmed.html",
    "result-failed.html",
    "sends-empty.html",
    "sends-preview-refused.html",
    "sends-preview.html",
    "sends-sendmail.html",
    "sends.html",
];

/// The output directory a `render-fixtures` invocation writes to: the first
/// argument when present, the documented default otherwise.
pub fn out_dir(args: &[String]) -> PathBuf {
    args.first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rust/target/ui-fixtures-sales"))
}

/// One completed `render-fixtures` run.
#[derive(Debug, Clone, PartialEq)]
pub struct FixtureRun {
    /// The directory the pages were written to.
    pub out_dir: PathBuf,
    /// The written page paths, in render order.
    pub files: Vec<PathBuf>,
}

/// The whole `render-fixtures` command: resolve the output directory, render
/// every page, and refuse anything that does not match [`PAGE_NAMES`]. The
/// binary prints the returned summary; every check is unit-tested here.
pub fn run_cli(args: &[String]) -> Result<FixtureRun, String> {
    let out = out_dir(args);
    let files = render_all(&out)?;
    if files.len() != PAGE_NAMES.len() {
        return Err(format!(
            "the renderer produced {} pages but PAGES lists {}",
            files.len(),
            PAGE_NAMES.len()
        ));
    }
    for path in &files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("page {} is not a UTF-8 file name", path.display()))?;
        if !PAGE_NAMES.contains(&name) {
            return Err(format!("page {name} is not listed in PAGES"));
        }
    }
    Ok(FixtureRun {
        out_dir: out,
        files,
    })
}

fn empty_pages(empty: &RequestData, csrf: &str) -> Vec<(String, String)> {
    let transport = data::Transport::Outbox(empty.dir.join("outbox"));
    let meta = page_meta("Overview", "Overview", Vec::new(), empty, csrf);
    let overview = pages::overview::render(&meta, empty, &transport);
    let meta = page_meta("Leads", "Leads", Vec::new(), empty, csrf);
    let leads = pages::leads::render(&meta, empty, &Filters::default());
    let meta = page_meta("Plan", "Plan", Vec::new(), empty, csrf);
    let plan = pages::plan::render(&meta, empty, &transport);
    let meta = page_meta("Sends", "Sends", Vec::new(), empty, csrf);
    let sends = pages::sends::render(&meta, empty, &transport);
    let meta = page_meta("Replies", "Replies", Vec::new(), empty, csrf);
    let replies = pages::replies::render(&meta, empty);
    let meta = page_meta("Domains", "Domains", Vec::new(), empty, csrf);
    let domains = pages::domains::render(&meta, empty);
    let meta = page_meta("Models", "Models", Vec::new(), empty, csrf);
    let models = pages::models::render(&meta, empty);
    vec![
        ("overview-empty.html".to_string(), overview),
        ("leads-empty.html".to_string(), leads),
        ("plan-empty.html".to_string(), plan),
        ("sends-empty.html".to_string(), sends),
        ("replies-empty.html".to_string(), replies),
        ("domains-empty.html".to_string(), domains),
        ("models-empty.html".to_string(), models),
    ]
}

fn page_meta(
    title: &str,
    nav: &'static str,
    trail: Vec<String>,
    data: &RequestData,
    csrf: &str,
) -> PageMeta {
    PageMeta::new(
        title,
        nav,
        trail,
        data.now,
        data.dir.display().to_string(),
        csrf,
    )
}

fn write(out: &Path, files: &mut Vec<PathBuf>, name: &str, html: String) -> Result<(), String> {
    std::fs::create_dir_all(out).map_err(|error| format!("{}: {error}", out.display()))?;
    let path = out.join(name);
    std::fs::write(&path, html).map_err(|error| format!("{}: {error}", path.display()))?;
    files.push(path);
    Ok(())
}

/// A fresh fixture working directory under the system temp dir. The name is
/// deterministic per process; [`render_all`] clears it before seeding, so
/// repeated calls in one test process start clean.
pub fn fixture_work_dir() -> PathBuf {
    std::env::temp_dir().join(format!("apex-sales-web-fixtures-{}", std::process::id()))
}

fn load(dir: &Path, overrides: &[(&str, &str)], now: i64) -> Result<RequestData, String> {
    let mut env = Env::new();
    env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
    for (key, value) in overrides {
        env.insert((*key).to_string(), (*value).to_string());
    }
    data::load_from(&env, now)
}

fn base_publisher(key: &str, name: &str, rating: Option<u64>) -> Publisher {
    let mut publisher = Publisher::new(key, name);
    publisher.domain = Some(key.to_string());
    publisher.country = Some("FI".to_string());
    if let Some(rating) = rating {
        publisher.apps.push(App {
            store: Store::AppStore,
            id: format!("{key}-app"),
            name: format!("{name} Game"),
            genre: Some("Games".to_string()),
            rating_count: Some(rating),
            free: Some(true),
            days_since_update: Some(7),
        });
    }
    publisher
}

fn put_contact(
    store: &StateDir,
    email: &str,
    name: Option<&str>,
    key: &str,
    kind: ContactKind,
    source: &str,
) -> Result<(), String> {
    store
        .put_contact(&Contact {
            email: email.to_string(),
            name: name.map(str::to_string),
            role: None,
            publisher_key: key.to_string(),
            kind,
            source: source.to_string(),
            consent: false,
        })
        .map_err(|error| error.to_string())
}

fn seed_rich(dir: &Path) -> Result<(), String> {
    let store = StateDir::open(dir).map_err(|error| error.to_string())?;
    let evidence = |id: &str, key: &str, kind: EvidenceKind, statement: String| Evidence {
        id: id.to_string(),
        kind,
        source: format!("https://{key}/app-ads.txt"),
        statement,
        observed_at: FIXTURE_NOW - 3 * DAY,
    };
    let mut studio = base_publisher("studio.example", "Studio Example", Some(24_000));
    studio.ads_txt_text = Some("applovin.com, 12345, DIRECT\n".to_string());
    studio.evidence.push(evidence(
        "ads-found",
        "studio.example",
        EvidenceKind::AppAdsTxt,
        "app-ads.txt names AppLovin (DIRECT)".to_string(),
    ));
    studio.evidence.push(Evidence {
        id: "store-listing".to_string(),
        kind: EvidenceKind::StoreListing,
        source: "https://apps.apple.com/app/id42".to_string(),
        statement: "App Store listing shows 24,000 ratings and an active release".to_string(),
        observed_at: FIXTURE_NOW - 5 * DAY,
    });
    store
        .put_publisher(&studio)
        .map_err(|error| error.to_string())?;
    let mut bright = base_publisher("brightapps.example", "Bright Apps", Some(3_400));
    bright.ads_txt_missing = true;
    bright.evidence.push(evidence(
        "ads-missing",
        "brightapps.example",
        EvidenceKind::AppAdsTxt,
        "https://brightapps.example/app-ads.txt was not found".to_string(),
    ));
    store
        .put_publisher(&bright)
        .map_err(|error| error.to_string())?;
    store
        .put_publisher(&base_publisher("indie.example", "Indie Dev", None))
        .map_err(|error| error.to_string())?;
    store
        .put_publisher(&base_publisher(
            "evil.example",
            "Evil Spam Co",
            Some(1_200_000),
        ))
        .map_err(|error| error.to_string())?;
    store
        .put_publisher(&base_publisher("quiet.example", "Quiet Studio", Some(800)))
        .map_err(|error| error.to_string())?;
    for index in 0..6 {
        store
            .put_publisher(&base_publisher(
                &format!("band{index}.example"),
                &format!("Band {index}"),
                Some(2_000 + index as u64 * 500),
            ))
            .map_err(|error| error.to_string())?;
    }

    put_contact(
        &store,
        "maria@studio.example",
        Some("Maria"),
        "studio.example",
        ContactKind::Named,
        "seed list",
    )?;
    put_contact(
        &store,
        "hello@brightapps.example",
        None,
        "brightapps.example",
        ContactKind::Role,
        "inbound",
    )?;
    put_contact(
        &store,
        "contact@indie.example",
        None,
        "indie.example",
        ContactKind::Generic,
        "seed list",
    )?;
    put_contact(
        &store,
        "spam@evil.example",
        None,
        "evil.example",
        ContactKind::Generic,
        "https://evil.example/app-ads.txt",
    )?;
    for index in 0..6 {
        put_contact(
            &store,
            &format!("ceo@band{index}.example"),
            None,
            &format!("band{index}.example"),
            ContactKind::Named,
            "seed list",
        )?;
    }

    // Events over the past days.
    let touch =
        |at: i64, email: &str, key: &str, step: usize, predicted: Option<f64>| Event::Touch {
            at,
            email: email.to_string(),
            publisher: key.to_string(),
            from: None,
            step,
            arm: if step == 0 { "t0-finding" } else { "t1-offer" }.to_string(),
            message_id: format!("{key}-{step}-{at}"),
            local_hour: 10,
            predicted,
        };
    let day = |offset: i64| FIXTURE_NOW - offset * DAY + 3600;
    store
        .put_event(&touch(
            day(6),
            "maria@studio.example",
            "studio.example",
            0,
            Some(0.05),
        ))
        .map_err(|error| error.to_string())?;
    store
        .put_event(&touch(
            day(5),
            "ceo@band0.example",
            "band0.example",
            0,
            Some(0.04),
        ))
        .map_err(|error| error.to_string())?;
    store
        .put_event(&touch(
            day(4),
            "ceo@band1.example",
            "band1.example",
            0,
            Some(0.03),
        ))
        .map_err(|error| error.to_string())?;
    store
        .put_event(&touch(
            day(3),
            "hello@brightapps.example",
            "brightapps.example",
            0,
            Some(0.02),
        ))
        .map_err(|error| error.to_string())?;
    store
        .put_event(&touch(
            day(2),
            "spam@evil.example",
            "evil.example",
            0,
            Some(0.01),
        ))
        .map_err(|error| error.to_string())?;
    store
        .put_event(&touch(
            day(6),
            "maria@studio.example",
            "studio.example",
            1,
            None,
        ))
        .map_err(|error| error.to_string())?;
    store
        .put_event(&Event::Outcome {
            at: day(5),
            email: Some("maria@studio.example".to_string()),
            publisher: None,
            outcome: Outcome::Reply(ReplyClass::Positive),
            note: Some("classification: the sender asked for a call next week".to_string()),
        })
        .map_err(|error| error.to_string())?;
    store
        .put_event(&Event::Outcome {
            at: day(4),
            email: Some("ceo@band0.example".to_string()),
            publisher: None,
            outcome: Outcome::Reply(ReplyClass::Unclear),
            note: Some("classification: ambiguous, needs a person".to_string()),
        })
        .map_err(|error| error.to_string())?;
    store
        .put_event(&Event::Outcome {
            at: day(3),
            email: Some("ceo@band1.example".to_string()),
            publisher: None,
            outcome: Outcome::Reply(ReplyClass::OptOut),
            note: Some("classification: asked not to be contacted again".to_string()),
        })
        .map_err(|error| error.to_string())?;
    store
        .put_event(&Event::Outcome {
            at: day(2),
            email: Some("hello@brightapps.example".to_string()),
            publisher: None,
            outcome: Outcome::AutoReply,
            note: Some("out of office until Monday".to_string()),
        })
        .map_err(|error| error.to_string())?;
    store
        .put_event(&Event::Outcome {
            at: day(2),
            email: Some("spam@evil.example".to_string()),
            publisher: None,
            outcome: Outcome::SoftBounce,
            note: None,
        })
        .map_err(|error| error.to_string())?;
    store
        .put_event(&Event::Suppress {
            at: day(2),
            email: Some("spam@evil.example".to_string()),
            publisher: None,
        })
        .map_err(|error| error.to_string())?;
    store
        .put_event(&Event::HealthReset {
            at: day(20),
            domain: None,
        })
        .map_err(|error| error.to_string())?;
    // One recorded approval so the approval page shows an approved row.
    let items = {
        let data = load(dir, &[], FIXTURE_NOW)?;
        plan(&data.snap, &data.learned, &data.config, data.now)
    };
    for item in items.iter().take(2) {
        store
            .put_approval(&item.touch_id, FIXTURE_NOW - 3600)
            .map_err(|error| error.to_string())?;
    }
    // A decisions.jsonl log and one outbox preview.
    let decisions = items
        .iter()
        .filter(|item| item.decision.verdict != Verdict::Deny)
        .take(3)
        .map(|item| {
            format!(
                "{{\"at\":{},\"touchId\":\"{}\",\"email\":\"{}\",\"verdict\":\"sent\",\"expectedValue\":{:.4}}}",
                item.send_at, item.touch_id, item.email, item.expected_value
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !decisions.is_empty() {
        std::fs::write(dir.join("decisions.jsonl"), format!("{decisions}\n"))
            .map_err(|error| error.to_string())?;
    }
    let outbox = dir.join("outbox");
    std::fs::create_dir_all(&outbox).map_err(|error| error.to_string())?;
    std::fs::write(
        outbox.join("20261019-step1-studio.example.eml"),
        "From: alexis@apexmediation.ee\nTo: maria@studio.example\nSubject: Studio Example: what AppLovin leaves on the table\n\nHi Maria,\n\nyour App Store listing shows 24,000 ratings and no app-ads.txt entry.\n\nAlexis\n\nUnsubscribe: https://apexmediation.ee/unsubscribe?u=maria\n",
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn seed_complaint_burst(dir: &Path) -> Result<(), String> {
    let store = StateDir::open(dir).map_err(|error| error.to_string())?;
    for index in 0..40 {
        store
            .put_event(&Event::Touch {
                at: FIXTURE_NOW - DAY + index,
                email: format!("c{index}@burst.example"),
                publisher: "burst.example".to_string(),
                from: None,
                step: 0,
                arm: "t0-finding".to_string(),
                message_id: format!("m{index}"),
                local_hour: 9,
                predicted: Some(0.04),
            })
            .map_err(|error| error.to_string())?;
    }
    for index in 0..5 {
        store
            .put_event(&Event::Outcome {
                at: FIXTURE_NOW - DAY + 100 + index,
                email: Some(format!("c{index}@burst.example")),
                publisher: None,
                outcome: Outcome::Complaint,
                note: None,
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn seed_bounce_burst(dir: &Path) -> Result<(), String> {
    let store = StateDir::open(dir).map_err(|error| error.to_string())?;
    for index in 0..50 {
        store
            .put_event(&Event::Touch {
                at: FIXTURE_NOW - DAY + index,
                email: format!("b{index}@bounce.example"),
                publisher: "bounce.example".to_string(),
                from: None,
                step: 0,
                arm: "t0-finding".to_string(),
                message_id: format!("m{index}"),
                local_hour: 9,
                predicted: None,
            })
            .map_err(|error| error.to_string())?;
    }
    for index in 0..3 {
        store
            .put_event(&Event::Outcome {
                at: FIXTURE_NOW - DAY + 200 + index,
                email: Some(format!("b{index}@bounce.example")),
                publisher: None,
                outcome: Outcome::HardBounce,
                note: None,
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// A two-domain fixture: domain one carries a complaint burst (quarantined),
/// domain two is clean, and the plan spreads across both.
fn seed_multi(dir: &Path) -> Result<(), String> {
    let store = StateDir::open(dir).map_err(|error| error.to_string())?;
    for index in 0..6 {
        let key = format!("multi{index}.example");
        let mut publisher = base_publisher(
            &key,
            &format!("Multi {index}"),
            Some(5_000 + index as u64 * 1_000),
        );
        publisher.ads_txt_missing = true;
        publisher.evidence.push(Evidence {
            id: "ads-missing".to_string(),
            kind: EvidenceKind::AppAdsTxt,
            source: format!("https://{key}/app-ads.txt"),
            statement: format!("https://{key}/app-ads.txt was not found"),
            observed_at: FIXTURE_NOW - DAY,
        });
        store
            .put_publisher(&publisher)
            .map_err(|error| error.to_string())?;
        put_contact(
            &store,
            &format!("ceo@{key}"),
            None,
            &key,
            ContactKind::Named,
            "seed list",
        )?;
    }
    for index in 0..40 {
        store
            .put_event(&Event::Touch {
                at: FIXTURE_NOW - DAY + index,
                email: format!("c{index}@burst.example"),
                publisher: "burst.example".to_string(),
                from: Some("one.example".to_string()),
                step: 0,
                arm: "t0-finding".to_string(),
                message_id: format!("m{index}"),
                local_hour: 9,
                predicted: Some(0.04),
            })
            .map_err(|error| error.to_string())?;
    }
    for index in 0..5 {
        store
            .put_event(&Event::Outcome {
                at: FIXTURE_NOW - DAY + 100 + index,
                email: Some(format!("c{index}@burst.example")),
                publisher: None,
                outcome: Outcome::Complaint,
                note: None,
            })
            .map_err(|error| error.to_string())?;
    }
    for index in 0..4 {
        store
            .put_event(&Event::Touch {
                at: FIXTURE_NOW - DAY + 200 + index,
                email: format!("k{index}@second.example"),
                publisher: "second.example".to_string(),
                from: Some("two.example".to_string()),
                step: 0,
                arm: "t0-finding".to_string(),
                message_id: format!("n{index}"),
                local_hour: 9,
                predicted: Some(0.03),
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_set_renders_at_least_thirty_eight_pages() {
        // The entry point resolves the default output directory and rejects
        // an argument-free run only by its successful default, which is
        // checked here without rendering twice in one process.
        assert_eq!(out_dir(&[]), PathBuf::from("rust/target/ui-fixtures-sales"));
        let out =
            std::env::temp_dir().join(format!("apex-sales-web-fixtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        assert_eq!(out_dir(&[out.display().to_string()]), out);
        let run = run_cli(&[out.display().to_string()]).expect("run");
        assert_eq!(run.out_dir, out);
        let files = run.files;
        assert!(files.len() >= 38, "{} pages", files.len());
        assert_eq!(files.len(), PAGE_NAMES.len(), "the listing stays exact");
        for path in &files {
            assert!(
                PAGE_NAMES.contains(&path.file_name().and_then(|n| n.to_str()).unwrap()),
                "{} is not listed",
                path.display()
            );
            let html = std::fs::read_to_string(path).unwrap();
            assert_eq!(html.matches("<h1").count(), 1, "{}", path.display());
            assert!(!html.contains("<script"), "{}", path.display());
            assert!(!html.contains("<style"), "{}", path.display());
            assert!(!html.contains("style=\""), "{}", path.display());
            assert_eq!(
                html.matches("<link rel=\"stylesheet\" href=\"/assets/apex.css\">")
                    .count(),
                1,
                "{}",
                path.display()
            );
        }
        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::remove_dir_all(fixture_work_dir());
    }
}
