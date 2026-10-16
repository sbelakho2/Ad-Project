//! The `apex-sales` command line, as a testable function.
//!
//! ```text
//! apex-sales import-seeds <seeds.csv>  seed list (domain,name,country,app_store_ids,email,contact_name,role)
//! apex-sales import-leads <leads.jsonl> the apex-api free-audit leads file
//! apex-sales discover [--limit N]      fetch app-ads.txt and App Store data
//! apex-sales plan                      show today's plan with every gate reason
//! apex-sales run                       plan and send what is due (hourly from a timer)
//! apex-sales approve <touch-id>        approve a planned touch (approval mode)
//! apex-sales ingest <message.eml>     classify a reply or bounce and record it
//! apex-sales record <target> <outcome> [note]   e.g. record studio.example paid
//! apex-sales suppress <target>         never contact an address or publisher
//! apex-sales reset-health              after investigating a quarantine
//! apex-sales report                    what the models believe, with intervals
//! apex-sales simulate [--publishers N] [--days D] [--seed S]
//! ```
//!
//! Configuration comes from the environment:
//!
//! | variable | meaning | default |
//! |---|---|---|
//! | `APEX_SALES_DIR` | state directory | `.apex/sales` |
//! | `APEX_SALES_MODE` | `off`, `shadow`, `approval`, `autonomous` | `shadow` |
//! | `APEX_SALES_FROM` / `APEX_SALES_FROM_NAME` | sender | required to send |
//! | `APEX_SALES_COMPANY` / `APEX_SALES_ADDRESS` | legal entity and postal address | required to send |
//! | `APEX_SALES_UNSUBSCRIBE_URL` | HTTPS one-click base URL (token appended) | mailto only |
//! | `APEX_SALES_TOKEN_SALT` | salt for unsubscribe tokens | none |
//! | `APEX_SALES_HOLDOUT` / `APEX_SALES_HOLDOUT_SALT` | holdout share and salt | `0.10` |
//! | `APEX_SALES_EXPERIMENT` | top-two message experiments | `true` |
//! | `APEX_SALES_LEGAL_FILE` | CSV `country,rule` (`opt_out`, `consent`, `review`; `*` = unknown) | built-in table |
//! | `APEX_SALES_DAILY_MAX` | ceiling of the warm-up ramp | `150` |
//! | `APEX_SALES_FETCH_DELAY_MS` | politeness delay between fetches | `1000` |

use std::collections::BTreeMap;

use crate::classify::ReplyClassifier;
use crate::compose::{build_message, Sender};
use crate::decision::{Autonomy, Verdict};
use crate::engine::{learn, plan, Config, PlanItem};
use crate::holdout::fnv1a;
use crate::io::{Fetcher, Mailer};
use crate::model::{normalize_email, Outcome, UnixSecs};
use crate::policy::{LegalTable, Rule};
use crate::simulate::{compare, SimConfig, SimResult};
use crate::store::{Event, StateDir};

/// Environment variables, captured once.
pub type Env = BTreeMap<String, String>;

fn var<'a>(env: &'a Env, key: &str) -> Option<&'a str> {
    env.get(key)
        .map(String::as_str)
        .filter(|v| !v.trim().is_empty())
}

/// Parse a legal table CSV.
pub fn parse_legal_table(csv: &str) -> Result<LegalTable, String> {
    let mut table = LegalTable {
        version: format!("file-{:08x}", fnv1a(csv.as_bytes()) as u32),
        rules: Vec::new(),
        unknown: Rule::OptOut,
    };
    for (i, row) in crate::ops::parse_csv(csv).into_iter().enumerate() {
        let (Some(country), Some(rule)) = (row.first(), row.get(1)) else {
            return Err(format!("legal table row {}: expected country,rule", i + 1));
        };
        if i == 0 && country.eq_ignore_ascii_case("country") {
            continue;
        }
        let rule = match rule.trim() {
            "opt_out" => Rule::OptOut,
            "consent" => Rule::ConsentRequired,
            "review" => Rule::Review,
            other => return Err(format!("legal table row {}: unknown rule {other}", i + 1)),
        };
        if country.trim() == "*" {
            table.unknown = rule;
        } else {
            table
                .rules
                .push((country.trim().to_ascii_uppercase(), rule));
        }
    }
    Ok(table)
}

/// Build the engine configuration from the environment.
pub fn config_from_env(env: &Env) -> Result<Config, String> {
    let sender = Sender {
        name: var(env, "APEX_SALES_FROM_NAME")
            .unwrap_or("Apex")
            .to_string(),
        email: var(env, "APEX_SALES_FROM")
            .unwrap_or("sales@apexmediation.ee")
            .to_string(),
        company: var(env, "APEX_SALES_COMPANY")
            .unwrap_or_default()
            .to_string(),
        postal_address: var(env, "APEX_SALES_ADDRESS")
            .unwrap_or_default()
            .to_string(),
        unsubscribe_url: var(env, "APEX_SALES_UNSUBSCRIBE_URL").map(str::to_string),
    };
    let mut config = Config::new(sender);
    if let Some(mode) = var(env, "APEX_SALES_MODE") {
        config.autonomy =
            Autonomy::parse(mode).ok_or_else(|| format!("APEX_SALES_MODE: unknown mode {mode}"))?;
    }
    if let Some(share) = var(env, "APEX_SALES_HOLDOUT") {
        let s: f64 = share
            .parse()
            .map_err(|_| "APEX_SALES_HOLDOUT must be a number".to_string())?;
        if !(0.0..=0.5).contains(&s) {
            return Err("APEX_SALES_HOLDOUT must be between 0 and 0.5".into());
        }
        config.holdout.share = s;
    }
    if let Some(salt) = var(env, "APEX_SALES_HOLDOUT_SALT") {
        config.holdout.salt = salt.to_string();
    }
    if let Some(e) = var(env, "APEX_SALES_EXPERIMENT") {
        config.experiment = e != "false" && e != "0";
    }
    if let Some(max) = var(env, "APEX_SALES_DAILY_MAX") {
        config.health.max_cap = max
            .parse()
            .map_err(|_| "APEX_SALES_DAILY_MAX must be a whole number".to_string())?;
    }
    if let Some(path) = var(env, "APEX_SALES_LEGAL_FILE") {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("APEX_SALES_LEGAL_FILE: {e}"))?;
        config.legal = parse_legal_table(&text)?;
    }
    Ok(config)
}

fn require_sender(config: &Config) -> Result<(), String> {
    let s = &config.sender;
    if normalize_email(&s.email).is_none()
        || s.company.trim().is_empty()
        || s.postal_address.trim().is_empty()
    {
        return Err("sending needs APEX_SALES_FROM, APEX_SALES_COMPANY and APEX_SALES_ADDRESS (the postal address every message must carry)".into());
    }
    Ok(())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn describe(item: &PlanItem) -> String {
    format!(
        "{} {:<14} {:<32} step {} arm {:<11} EV ${:>7.2} p {:.3} at {} {}",
        item.touch_id,
        item.decision.verdict.as_str(),
        item.email,
        item.step,
        item.arm.as_deref().unwrap_or("-"),
        item.expected_value,
        item.p_interest,
        crate::calendar::rfc5322_date(item.send_at),
        if item.decision.reasons.is_empty() {
            String::new()
        } else {
            format!("| {}", item.decision.reasons.join("; "))
        }
    )
}

fn sim_line(name: &str, r: &SimResult) -> String {
    format!(
        "{name:<9} sends {:>6}  interested {:>4}  complaints {:>3}  value of sends ${:>10.0}  per send ${:.2}",
        r.sends,
        r.interested,
        r.complaints,
        r.value_of_sends,
        r.value_of_sends / f64::from(r.sends.max(1))
    )
}

/// The transport for this environment: `.eml` files in shadow or off mode
/// or when `APEX_SALES_OUTBOX` is set, otherwise the local MTA's
/// `sendmail` (`APEX_SALES_SENDMAIL`, default `/usr/sbin/sendmail`).
pub fn mailer_from_env(env: &Env) -> Box<dyn Mailer> {
    let mode = var(env, "APEX_SALES_MODE")
        .and_then(Autonomy::parse)
        .unwrap_or(Autonomy::Shadow);
    let dir = var(env, "APEX_SALES_DIR").unwrap_or(".apex/sales");
    match var(env, "APEX_SALES_OUTBOX") {
        Some(outbox) => Box::new(crate::io::OutboxMailer { dir: outbox.into() }),
        None if matches!(mode, Autonomy::Shadow | Autonomy::Off) => {
            Box::new(crate::io::OutboxMailer {
                dir: std::path::Path::new(dir).join("outbox"),
            })
        }
        None => Box::new(crate::io::SendmailMailer {
            binary: var(env, "APEX_SALES_SENDMAIL")
                .unwrap_or("/usr/sbin/sendmail")
                .to_string(),
        }),
    }
}

/// Run one command. Returns the text to print.
pub fn run(
    args: &[String],
    env: &Env,
    now: UnixSecs,
    fetcher: &dyn Fetcher,
    mailer: &dyn Mailer,
) -> Result<String, String> {
    let io = |e: std::io::Error| e.to_string();
    let config = config_from_env(env)?;
    let dir = StateDir::open(var(env, "APEX_SALES_DIR").unwrap_or(".apex/sales")).map_err(io)?;
    let command = args.first().map(String::as_str).unwrap_or("help");
    let arg = |i: usize| {
        args.get(i)
            .cloned()
            .ok_or_else(|| format!("{command}: missing argument"))
    };
    match command {
        "import-seeds" | "import-leads" => {
            let text = std::fs::read_to_string(arg(1)?).map_err(io)?;
            let snap = dir.load().map_err(io)?;
            let out = if command == "import-seeds" {
                crate::ops::import_seeds(&text, &snap)
            } else {
                crate::ops::import_leads(&text, &snap)
            };
            for p in &out.publishers {
                dir.put_publisher(p).map_err(io)?;
            }
            for c in &out.contacts {
                dir.put_contact(c).map_err(io)?;
            }
            for e in &out.events {
                dir.put_event(e).map_err(io)?;
            }
            let mut msg = format!(
                "imported {} publishers, {} contacts, {} events",
                out.publishers.len(),
                out.contacts.len(),
                out.events.len()
            );
            if out.already > 0 {
                msg.push_str(&format!(", {} already imported", out.already));
            }
            for r in out.refused {
                msg.push_str(&format!("\nrefused {r}"));
            }
            Ok(msg)
        }
        "discover" => {
            let limit: usize = flag(args, "--limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(50);
            let delay = var(env, "APEX_SALES_FETCH_DELAY_MS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000u64);
            let snap = dir.load().map_err(io)?;
            let stale = |p: &crate::model::Publisher| {
                p.domain.is_some()
                    && p.evidence
                        .iter()
                        .filter(|e| e.kind == crate::model::EvidenceKind::AppAdsTxt)
                        .map(|e| e.observed_at)
                        .max()
                        .is_none_or(|t| now - t > 30 * crate::model::DAY)
            };
            let mut lines = Vec::new();
            let mut existing: Vec<String> = snap.contacts.keys().cloned().collect();
            for p in snap.publishers.values().filter(|p| stale(p)).take(limit) {
                let (updated, contacts, notes) = crate::ops::discover(p, fetcher, now);
                dir.put_publisher(&updated).map_err(io)?;
                for c in contacts {
                    if !existing.contains(&c.email) {
                        existing.push(c.email.clone());
                        dir.put_contact(&c).map_err(io)?;
                    }
                }
                lines.push(format!("{}: {}", p.key, notes.join("; ")));
                if delay > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                }
            }
            Ok(if lines.is_empty() {
                "nothing to discover".into()
            } else {
                lines.join("\n")
            })
        }
        "plan" => {
            let snap = dir.load().map_err(io)?;
            let learned = learn(&snap, &config, now);
            let items = plan(&snap, &learned, &config, now);
            let mut out = format!(
                "mode {}  domain {}  cap {} (sent today {})\n",
                config.autonomy.as_str(),
                learned.health.state.as_str(),
                learned.health.cap,
                learned.sent_today
            );
            for item in &items {
                out.push_str(&describe(item));
                out.push('\n');
            }
            Ok(out)
        }
        "run" => {
            let snap = dir.load().map_err(io)?;
            let learned = learn(&snap, &config, now);
            let items = plan(&snap, &learned, &config, now);
            if config.autonomy != Autonomy::Shadow && config.autonomy != Autonomy::Off {
                require_sender(&config)?;
            }
            let salt = var(env, "APEX_SALES_TOKEN_SALT").unwrap_or_default();
            let mut sent = 0;
            let mut previews = 0;
            let mut waiting = 0;
            for item in &items {
                let approved = snap.approvals.contains(&item.touch_id);
                let go = match item.decision.verdict {
                    Verdict::Send => true,
                    Verdict::NeedsApproval => approved,
                    Verdict::Shadow => false,
                    _ => continue,
                };
                if item.send_at > now + 3600 {
                    waiting += 1;
                    continue;
                }
                let (Some(rendered), Some(arm), Some(contact)) =
                    (&item.rendered, &item.arm, snap.contacts.get(&item.email))
                else {
                    continue;
                };
                let token = format!(
                    "{:016x}",
                    fnv1a(format!("{salt}|{}", item.email).as_bytes())
                );
                let msg = build_message(
                    rendered,
                    contact,
                    &config.sender,
                    &item.touch_id,
                    now,
                    &token,
                );
                if !go {
                    if item.decision.verdict == Verdict::Shadow {
                        mailer.send(&msg).map_err(|e| e.to_string())?;
                        previews += 1;
                    } else {
                        waiting += 1;
                    }
                    continue;
                }
                mailer.send(&msg).map_err(|e| e.to_string())?;
                dir.put_event(&Event::Touch {
                    at: now,
                    email: item.email.clone(),
                    publisher: item.publisher.clone(),
                    step: item.step,
                    arm: arm.clone(),
                    message_id: msg.message_id.clone(),
                    local_hour: item.local_hour,
                    predicted: Some(item.p_interest),
                })
                .map_err(io)?;
                dir.put_decision(&crate::json::Value::object(vec![
                    ("at", crate::json::Value::num(now as f64)),
                    ("touchId", crate::json::Value::str(item.touch_id.clone())),
                    ("email", crate::json::Value::str(item.email.clone())),
                    ("verdict", crate::json::Value::str("sent")),
                    (
                        "expectedValue",
                        crate::json::Value::num(item.expected_value),
                    ),
                    ("pInterest", crate::json::Value::num(item.p_interest)),
                    (
                        "scoreReasons",
                        crate::json::Value::Array(
                            item.score_reasons
                                .iter()
                                .map(|r| crate::json::Value::str(r.clone()))
                                .collect(),
                        ),
                    ),
                    (
                        "cites",
                        crate::json::Value::Array(
                            rendered
                                .cites
                                .iter()
                                .map(|r| crate::json::Value::str(r.clone()))
                                .collect(),
                        ),
                    ),
                ]))
                .map_err(io)?;
                sent += 1;
            }
            Ok(format!(
                "mode {}: sent {sent}, previews {previews}, waiting {waiting}",
                config.autonomy.as_str()
            ))
        }
        "approve" => {
            let id = arg(1)?;
            dir.put_approval(&id, now).map_err(io)?;
            Ok(format!("approved {id}"))
        }
        "ingest" => {
            let raw = std::fs::read_to_string(arg(1)?).map_err(io)?;
            let snap = dir.load().map_err(io)?;
            let (event, c) =
                crate::ops::ingest_reply(&raw, &snap, &ReplyClassifier::default(), now);
            match event {
                Some(e) => {
                    dir.put_event(&e).map_err(io)?;
                    Ok(format!(
                        "recorded {} ({})",
                        c.outcome.as_string(),
                        c.rationale
                    ))
                }
                None => Ok(format!(
                    "unmatched sender: classified {} ({}); record it by hand",
                    c.outcome.as_string(),
                    c.rationale
                )),
            }
        }
        "record" => {
            let target = arg(1)?;
            let outcome = Outcome::parse(&arg(2)?).ok_or("record: unknown outcome")?;
            let email = normalize_email(&target).filter(|_| target.contains('@'));
            dir.put_event(&Event::Outcome {
                at: now,
                publisher: if email.is_none() {
                    Some(target.clone())
                } else {
                    None
                },
                email,
                outcome,
                note: args.get(3).cloned(),
            })
            .map_err(io)?;
            Ok(format!("recorded {} for {target}", outcome.as_string()))
        }
        "suppress" => {
            let target = arg(1)?;
            let email = normalize_email(&target).filter(|_| target.contains('@'));
            dir.put_event(&Event::Suppress {
                at: now,
                publisher: if email.is_none() {
                    Some(target.clone())
                } else {
                    None
                },
                email,
            })
            .map_err(io)?;
            Ok(format!("suppressed {target}"))
        }
        "reset-health" => {
            dir.put_event(&Event::HealthReset { at: now }).map_err(io)?;
            Ok("sending domain reset to warming".into())
        }
        "report" => {
            let snap = dir.load().map_err(io)?;
            let learned = learn(&snap, &config, now);
            let mut out = crate::ops::report(&snap, &learned, &config, now);
            if snap.skipped_lines > 0 {
                out.push_str(&format!(
                    "\nwarning: {} unreadable lines were skipped\n",
                    snap.skipped_lines
                ));
            }
            Ok(out)
        }
        "simulate" => {
            let cfg = SimConfig {
                publishers: flag(args, "--publishers")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1500),
                days: flag(args, "--days")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(60),
                seed: flag(args, "--seed")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1),
            };
            let (engine, baseline) = compare(&cfg);
            Ok(format!(
                "{} publishers, {} days, seed {}\n{}\n{}\nratio {:.2}",
                cfg.publishers,
                cfg.days,
                cfg.seed,
                sim_line("engine", &engine),
                sim_line("baseline", &baseline),
                engine.value_of_sends / baseline.value_of_sends.max(1e-9)
            ))
        }
        "help" | "--help" | "-h" => Ok(include_str!("cli_help.txt").to_string()),
        other => Err(format!("unknown command {other}; try `apex-sales help`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{MapFetcher, OutboxMailer};
    use crate::model::DAY;

    fn env(dir: &std::path::Path, mode: &str) -> Env {
        let mut e = Env::new();
        e.insert("APEX_SALES_DIR".into(), dir.display().to_string());
        e.insert("APEX_SALES_MODE".into(), mode.into());
        e.insert("APEX_SALES_FROM".into(), "aaron@apexmediation.ee".into());
        e.insert("APEX_SALES_FROM_NAME".into(), "Aaron".into());
        e.insert("APEX_SALES_COMPANY".into(), "Apex OÜ".into());
        e.insert("APEX_SALES_ADDRESS".into(), "Tallinn, Estonia".into());
        e.insert("APEX_SALES_HOLDOUT".into(), "0".into());
        e.insert("APEX_SALES_FETCH_DELAY_MS".into(), "0".into());
        e
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    const NOW: UnixSecs = 20_745 * DAY + 9 * 3600; // Monday 19 Oct 2026 09:00 UTC

    #[test]
    fn end_to_end_import_discover_plan_run_ingest_report() {
        let root = std::env::temp_dir().join(format!("apex-sales-cli-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let seeds = root.join("seeds.csv");
        std::fs::write(
            &seeds,
            "domain,name,country,app_store_ids,email,contact_name,role\nstudio.example,Studio,FI,42,maria@studio.example,Maria Lopez,CEO\n",
        )
        .unwrap();
        let state = root.join("state");
        let outbox = OutboxMailer {
            dir: root.join("outbox"),
        };
        let fetcher = MapFetcher {
            pages: vec![
                ("https://studio.example/app-ads.txt".into(), 404, String::new()),
                (
                    "https://itunes.apple.com/lookup?id=42".into(),
                    200,
                    r#"{"results":[{"kind":"software","trackId":42,"trackName":"Tile Quest","primaryGenreName":"Games","userRatingCount":20000,"price":0}]}"#.into(),
                ),
            ],
        };
        let e = env(&state, "autonomous");
        let out = run(
            &s(&["import-seeds", seeds.to_str().unwrap()]),
            &e,
            NOW,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(out.starts_with("imported 1 publishers, 1 contacts"));
        let out = run(&s(&["discover"]), &e, NOW, &fetcher, &outbox).unwrap();
        assert!(out.contains("not found (404)"), "{out}");
        assert_eq!(
            run(&s(&["discover"]), &e, NOW, &fetcher, &outbox).unwrap(),
            "nothing to discover"
        );
        let out = run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).unwrap();
        assert!(
            out.contains(" send ") && out.contains("maria@studio.example"),
            "{out}"
        );
        // The chosen local hour may be later today or tomorrow; run hourly
        // until it is due, as the timer would.
        let mut sent = false;
        for h in 0..48 {
            let out = run(&s(&["run"]), &e, NOW + h * 3600, &fetcher, &outbox).unwrap();
            if out.contains("sent 1") {
                sent = true;
                break;
            }
        }
        assert!(sent);
        let emls: Vec<_> = std::fs::read_dir(root.join("outbox")).unwrap().collect();
        assert_eq!(emls.len(), 1);
        let reply = root.join("reply.eml");
        std::fs::write(
            &reply,
            "From: Maria <maria@studio.example>\nSubject: Re\n\nYes, please run the free audit.\n",
        )
        .unwrap();
        let out = run(
            &s(&["ingest", reply.to_str().unwrap()]),
            &e,
            NOW + DAY,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(out.starts_with("recorded reply:positive"), "{out}");
        assert!(run(
            &s(&["record", "studio.example", "audit_requested"]),
            &e,
            NOW + 2 * DAY,
            &fetcher,
            &outbox
        )
        .is_ok());
        assert!(run(&s(&["record", "x", "nope"]), &e, NOW, &fetcher, &outbox).is_err());
        assert!(run(
            &s(&["suppress", "spam@else.example"]),
            &e,
            NOW,
            &fetcher,
            &outbox
        )
        .is_ok());
        assert!(run(&s(&["reset-health"]), &e, NOW, &fetcher, &outbox).is_ok());
        assert!(run(&s(&["approve", "t1"]), &e, NOW, &fetcher, &outbox)
            .unwrap()
            .contains("t1"));
        let rep = run(&s(&["report"]), &e, NOW + 3 * DAY, &fetcher, &outbox).unwrap();
        assert!(rep.contains("lead score"));
        // The replied publisher is no longer planned.
        let out = run(&s(&["plan"]), &e, NOW + 5 * DAY, &fetcher, &outbox).unwrap();
        assert!(!out.contains("maria@studio.example"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn modes_and_configuration_guard_sending() {
        let root =
            std::env::temp_dir().join(format!("apex-sales-cli-modes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let outbox = OutboxMailer {
            dir: root.join("outbox"),
        };
        let fetcher = MapFetcher::default();
        let mut e = env(&root.join("state"), "autonomous");
        e.insert("APEX_SALES_ADDRESS".into(), String::new());
        assert!(run(&s(&["run"]), &e, NOW, &fetcher, &outbox)
            .unwrap_err()
            .contains("postal address"));
        e.insert("APEX_SALES_MODE".into(), "shadow".into());
        assert!(run(&s(&["run"]), &e, NOW, &fetcher, &outbox)
            .unwrap()
            .starts_with("mode shadow"));
        e.insert("APEX_SALES_MODE".into(), "chaos".into());
        assert!(run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).is_err());
        e.insert("APEX_SALES_MODE".into(), "off".into());
        e.insert("APEX_SALES_HOLDOUT".into(), "0.9".into());
        assert!(run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).is_err());
        e.insert("APEX_SALES_HOLDOUT".into(), "x".into());
        assert!(run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).is_err());
        e.insert("APEX_SALES_HOLDOUT".into(), "0.2".into());
        e.insert("APEX_SALES_DAILY_MAX".into(), "x".into());
        assert!(run(&s(&["plan"]), &e, NOW, &fetcher, &outbox).is_err());
        e.remove("APEX_SALES_DAILY_MAX");
        e.insert("APEX_SALES_EXPERIMENT".into(), "false".into());
        assert!(!config_from_env(&e).unwrap().experiment);
        assert!(run(&s(&["help"]), &e, NOW, &fetcher, &outbox)
            .unwrap()
            .contains("apex-sales"));
        let probe = crate::compose::Outgoing {
            from: "-x@a.example".into(),
            to: "b@b.example".into(),
            message_id: "m".into(),
            raw: String::new(),
        };
        // Off mode writes previews; autonomous mode uses sendmail, which
        // refuses this hostile envelope sender before running anything.
        assert!(mailer_from_env(&e).send(&probe).is_ok());
        e.insert("APEX_SALES_MODE".into(), "autonomous".into());
        assert!(mailer_from_env(&e).send(&probe).is_err());
        e.insert(
            "APEX_SALES_OUTBOX".into(),
            root.join("box").display().to_string(),
        );
        assert!(mailer_from_env(&e).send(&probe).is_ok());
        e.insert("APEX_SALES_MODE".into(), "off".into());
        assert!(run(&s(&["bogus"]), &e, NOW, &fetcher, &outbox).is_err());
        assert!(run(&s(&["ingest"]), &e, NOW, &fetcher, &outbox)
            .unwrap_err()
            .contains("missing argument"));
        let out = run(
            &s(&["simulate", "--publishers", "60", "--days", "5"]),
            &e,
            NOW,
            &fetcher,
            &outbox,
        )
        .unwrap();
        assert!(out.contains("engine") && out.contains("baseline"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[derive(Debug)]
    struct BrokenMailer;

    impl Mailer for BrokenMailer {
        fn send(&self, _msg: &crate::compose::Outgoing) -> Result<(), crate::io::MailError> {
            Err(crate::io::MailError("relay down".into()))
        }
    }

    #[test]
    fn transport_and_file_failures_are_errors_and_record_nothing() {
        let root = std::env::temp_dir().join(format!("apex-sales-cli-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fetcher = MapFetcher {
            pages: vec![
                ("https://studio.example/app-ads.txt".into(), 404, String::new()),
                (
                    "https://itunes.apple.com/lookup?id=42".into(),
                    200,
                    r#"{"results":[{"kind":"software","trackId":42,"trackName":"Tile Quest","primaryGenreName":"Games","userRatingCount":20000,"price":0}]}"#.into(),
                ),
            ],
        };
        let mut e = env(&root.join("state"), "autonomous");
        let missing = root.join("missing.csv").display().to_string();
        let seeds_cmd = |path: &str| s(&["import-seeds", path]);
        assert!(run(&seeds_cmd(&missing), &e, NOW, &fetcher, &BrokenMailer).is_err());
        let seeds = root.join("seeds.csv");
        std::fs::write(
            &seeds,
            "domain,name,country,app_store_ids,email,contact_name,role\nstudio.example,Studio,FI,42,maria@studio.example,Maria Lopez,CEO\n",
        )
        .unwrap();
        let seeds = seeds.display().to_string();
        run(&seeds_cmd(&seeds), &e, NOW, &fetcher, &BrokenMailer).unwrap();
        // Without evidence there is nothing true to say, so nothing is planned
        // to send; discovery finds the app and the missing app-ads.txt.
        let out = run(&s(&["plan"]), &e, NOW, &fetcher, &BrokenMailer).unwrap();
        assert!(!out.contains(" send "), "{out}");
        run(&s(&["discover"]), &e, NOW, &fetcher, &BrokenMailer).unwrap();
        // Hourly runs until the touch is due: the failed hand-off is an
        // error, and no touch is recorded, so the contact stays planned.
        let failed = (0..48)
            .find_map(|h| run(&s(&["run"]), &e, NOW + h * 3600, &fetcher, &BrokenMailer).err());
        assert!(failed.is_some_and(|err| err.contains("relay down")));
        let out = run(&s(&["plan"]), &e, NOW, &fetcher, &BrokenMailer).unwrap();
        assert!(
            out.contains(" send ") && out.contains("maria@studio.example"),
            "{out}"
        );
        // Shadow previews go through the same transport and fail the same way.
        e.insert("APEX_SALES_MODE".into(), "shadow".into());
        let shadow_failed = (0..48)
            .any(|h| run(&s(&["run"]), &e, NOW + h * 3600, &fetcher, &BrokenMailer).is_err());
        assert!(shadow_failed);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn legal_tables_parse() {
        let t = parse_legal_table("country,rule\nDE,consent\nTR,review\n*,review\n").unwrap();
        assert_eq!(t.rule("de"), Rule::ConsentRequired);
        assert_eq!(t.rule("TR"), Rule::Review);
        assert_eq!(t.unknown, Rule::Review);
        assert!(parse_legal_table("DE,maybe\n").is_err());
        assert!(parse_legal_table("DE\n").is_err());
        let dir = std::env::temp_dir().join(format!("apex-sales-legal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legal.csv");
        std::fs::write(&path, "FR,consent\n").unwrap();
        let mut e = Env::new();
        e.insert("APEX_SALES_LEGAL_FILE".into(), path.display().to_string());
        assert_eq!(
            config_from_env(&e).unwrap().legal.rule("FR"),
            Rule::ConsentRequired
        );
        e.insert(
            "APEX_SALES_LEGAL_FILE".into(),
            dir.join("missing.csv").display().to_string(),
        );
        assert!(config_from_env(&e).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
