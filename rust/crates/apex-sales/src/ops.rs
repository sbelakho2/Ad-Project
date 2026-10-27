//! Operator workflows: importing leads, discovering public signals,
//! recording replies and reporting what the models believe.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::bandit::report as arm_report;
use crate::classify::ReplyClassifier;
use crate::engine::{Config, Learned};
use crate::funnel::Stage;
use crate::holdout::effect;
use crate::io::Fetcher;
use crate::json::{self, Value};
use crate::model::{
    normalize_domain, normalize_email, App, Contact, ContactKind, Evidence, EvidenceKind, Outcome,
    Publisher, Store, UnixSecs, DAY,
};
use crate::rng::Rng;
use crate::scoring::{FEATURES, N};
use crate::signals::{ads_txt_findings, parse_app_ads_txt, parse_itunes_results, SizeBand};
use crate::stats::{brier, log_loss, reliability};
use crate::store::{Event, Snapshot};

/// Parse RFC 4180 CSV (quoted fields, doubled quotes, `\r\n` or `\n` line ends).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' if field.is_empty() => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                c => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| !(r.len() == 1 && r[0].trim().is_empty()));
    rows
}

/// What an import produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Imported {
    /// Publishers to upsert.
    pub publishers: Vec<Publisher>,
    /// Contacts to upsert.
    pub contacts: Vec<Contact>,
    /// Events to append.
    pub events: Vec<Event>,
    /// Rows refused, with the reason.
    pub refused: Vec<String>,
    /// Rows skipped because an earlier import already recorded them.
    pub already: usize,
}

// The membership probes a seed import performs. The scaling test asserts one
// index probe per accepted-but-not-refused row; the pre-fix `iter().find`
// pattern probed the growing output once per earlier publisher, which is
// quadratic in the row count.
#[cfg(test)]
thread_local! {
    static SEED_PROBES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static OUTCOME_PROBES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_seed_probe() {
    SEED_PROBES.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn seed_probe_count() -> u64 {
    SEED_PROBES.with(std::cell::Cell::get)
}

#[cfg(test)]
fn reset_seed_probes() {
    SEED_PROBES.with(|count| count.set(0));
}

#[cfg(test)]
fn note_outcome_probe() {
    OUTCOME_PROBES.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn outcome_probe_count() -> u64 {
    OUTCOME_PROBES.with(std::cell::Cell::get)
}

#[cfg(test)]
fn reset_outcome_probes() {
    OUTCOME_PROBES.with(|count| count.set(0));
}

/// Import a seed list. Header (any order, case-insensitive): `domain`,
/// `name`, `country`, `app_store_ids` (semicolon-separated), `email`,
/// `contact_name`, `role`. A row needs a domain and a valid email at it or
/// a valid email of any kind.
pub fn import_seeds(csv: &str, existing: &Snapshot) -> Imported {
    let rows = parse_csv(csv);
    let mut out = Imported::default();
    let Some(header) = rows.first() else {
        return out;
    };
    let col = |name: &str| {
        header
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };
    let (c_domain, c_name, c_country, c_ids, c_email, c_cname, c_role, c_revenue) = (
        col("domain"),
        col("name"),
        col("country"),
        col("app_store_ids"),
        col("email"),
        col("contact_name"),
        col("role"),
        col("monthly_revenue_micros"),
    );
    // One slot per distinct domain, plus `key -> slot` and the last-seen
    // order. The previous shape scanned the growing output per seed and
    // retained over it (`iter().find`, then `retain`), so `n` rows with
    // repeated domains cost O(n^2); a hash lookup and a slot update make the
    // membership probe and the merge O(1) while the final order stays the
    // order of each domain's last row, exactly as before.
    let mut positions: HashMap<String, usize> = HashMap::new();
    let mut slots: Vec<Option<Publisher>> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(1) {
        let get = |c: Option<usize>| {
            c.and_then(|c| row.get(c))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
        };
        let Some(email) = get(c_email).and_then(normalize_email) else {
            out.refused
                .push(format!("row {}: missing or invalid email", i + 1));
            continue;
        };
        let domain = get(c_domain)
            .and_then(normalize_domain)
            .or_else(|| normalize_domain(email.rsplit('@').next().unwrap_or_default()));
        let Some(domain) = domain else {
            out.refused.push(format!("row {}: no usable domain", i + 1));
            continue;
        };
        #[cfg(test)]
        note_seed_probe();
        let mut p = existing
            .publishers
            .get(&domain)
            .cloned()
            .or_else(|| positions.get(&domain).and_then(|&slot| slots[slot].clone()))
            .unwrap_or_else(|| Publisher::new(domain.clone(), get(c_name).unwrap_or(&domain)));
        p.domain = Some(domain.clone());
        if let Some(name) = get(c_name) {
            p.name = name.to_string();
        }
        if let Some(c) = get(c_country) {
            p.country = Some(c.to_ascii_uppercase());
        }
        if let Some(raw) = get(c_revenue) {
            match raw.parse::<i64>() {
                Ok(revenue) if revenue >= 0 => p.monthly_revenue_micros = Some(revenue),
                _ => {
                    out.refused.push(format!(
                        "row {}: monthly_revenue_micros must be a non-negative whole number of micros",
                        i + 1
                    ));
                    continue;
                }
            }
        }
        for id in get(c_ids)
            .unwrap_or_default()
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if id.chars().all(|c| c.is_ascii_digit()) && !p.apps.iter().any(|a| a.id == id) {
                p.apps.push(App {
                    store: Store::AppStore,
                    id: id.to_string(),
                    name: format!("App {id}"),
                    genre: None,
                    rating_count: None,
                    free: None,
                    days_since_update: None,
                });
            }
        }
        match positions.get(&domain) {
            Some(&slot) => slots[slot] = Some(p),
            None => {
                positions.insert(domain.clone(), slots.len());
                slots.push(Some(p));
            }
        }
        order.push(domain.clone());
        out.contacts.push(Contact {
            kind: ContactKind::of_email(&email),
            email,
            name: get(c_cname).map(str::to_string),
            role: get(c_role).map(str::to_string),
            publisher_key: domain,
            source: "seed list".into(),
            consent: false,
        });
    }
    // The old retain-and-push kept each domain at the position of its last
    // row. Walking the row order backwards and keeping the first sight of a
    // key reproduces that order in one linear pass.
    let mut emitted = HashSet::with_capacity(order.len());
    let mut keys = Vec::with_capacity(order.len());
    for key in order.iter().rev() {
        if emitted.insert(key) {
            keys.push(key);
        }
    }
    keys.reverse();
    out.publishers = keys
        .into_iter()
        .filter_map(|key| positions.get(key).and_then(|&slot| slots[slot].take()))
        .collect();
    out
}

/// Import the free-audit leads file written by `apex-api`
/// (`{"atUnixMs": ..., "lead": {"publisher", "email", "platforms", ...}}`
/// per line). Each lead becomes an inbound publisher with a consenting
/// contact and an `AuditRequested` outcome, which hands it to a person and
/// keeps it out of cold sequences.
pub fn import_leads(jsonl: &str, existing: &Snapshot) -> Imported {
    let mut out = Imported::default();
    // Two linear indexes replace the pre-fix scans: a set of already-recorded
    // (submission time, address) audits and a domain-slot map. The old path
    // scanned every event and retained over the growing publisher output per
    // line, which is O(n^2) on a 10k-lead file.
    let mut seen_audits: HashSet<(i64, String)> = existing
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Outcome {
                at,
                email: Some(email),
                outcome: Outcome::AuditRequested,
                ..
            } => Some((*at, email.clone())),
            _ => None,
        })
        .collect();
    let mut positions: HashMap<String, usize> = HashMap::new();
    let mut slots: Vec<Option<Publisher>> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    for (i, line) in jsonl.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((at, lead)) = json::parse(line).ok().and_then(|v| {
            let at = v.get("atUnixMs").and_then(Value::as_i64)? / 1000;
            Some((at, v.get("lead")?.clone()))
        }) else {
            out.refused
                .push(format!("line {}: not a lead record", i + 1));
            continue;
        };
        let Some(email) = lead.str_at("email").and_then(normalize_email) else {
            out.refused.push(format!("line {}: invalid email", i + 1));
            continue;
        };
        // The leads file is append-only and re-imported on every run: a lead
        // already recorded (same address and submission time) is skipped.
        if !seen_audits.insert((at, email.clone())) {
            out.already += 1;
            continue;
        }
        let email_domain = email.rsplit('@').next().unwrap_or_default().to_string();
        let free = crate::policy::FREE_MAIL.contains(&email_domain.as_str());
        let key = if free {
            format!("lead-{}", lead.str_at("publisher").unwrap_or("unknown"))
        } else {
            email_domain.clone()
        };
        let mut p = existing.publishers.get(&key).cloned().unwrap_or_else(|| {
            Publisher::new(
                key.clone(),
                lead.str_at("displayName")
                    .or_else(|| lead.str_at("publisher"))
                    .unwrap_or(&key),
            )
        });
        if !free {
            p.domain = Some(email_domain);
        }
        p.inbound = true;
        p.declared_platforms = lead
            .get("platforms")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect();
        if let Some(range) = lead.str_at("monthlyRevenueRange") {
            p.evidence.retain(|e| e.id != "inbound-revenue");
            p.evidence.push(Evidence {
                id: "inbound-revenue".into(),
                kind: EvidenceKind::InboundForm,
                source: "free-audit form".into(),
                statement: format!("you told us your monthly ad revenue is {range}"),
                observed_at: at,
            });
        }
        out.events.push(Event::Outcome {
            at,
            email: Some(email.clone()),
            publisher: Some(key.clone()),
            outcome: crate::model::Outcome::AuditRequested,
            note: lead.str_at("requestId").map(str::to_string),
        });
        out.contacts.push(Contact {
            kind: ContactKind::of_email(&email),
            email,
            name: None,
            role: None,
            publisher_key: key.clone(),
            source: "inbound".into(),
            consent: true,
        });
        match positions.get(&key) {
            Some(&slot) => slots[slot] = Some(p),
            None => {
                positions.insert(key.clone(), slots.len());
                slots.push(Some(p));
            }
        }
        order.push(key);
    }
    // Last occurrence per key wins the position, exactly like the previous
    // retain-then-push.
    let mut emitted = HashSet::with_capacity(order.len());
    let mut keys = Vec::with_capacity(order.len());
    for key in order.iter().rev() {
        if emitted.insert(key) {
            keys.push(key);
        }
    }
    keys.reverse();
    out.publishers = keys
        .into_iter()
        .filter_map(|key| positions.get(key).and_then(|&slot| slots[slot].take()))
        .collect();
    out
}

/// Build the outcome event the CLI's `record` writes, shared so the
/// outcomes import records connected, paid and watch signups through exactly
/// the same semantics: an address target becomes an email outcome, anything
/// else a publisher outcome.
pub fn record_outcome(target: &str, outcome: Outcome, note: Option<String>, at: UnixSecs) -> Event {
    let email = normalize_email(target).filter(|_| target.contains('@'));
    Event::Outcome {
        at,
        publisher: if email.is_none() {
            Some(target.to_string())
        } else {
            None
        },
        email,
        outcome,
        note,
    }
}

/// Import the frozen outcomes file appended by `apex-api`: one JSON object
/// per line,
/// `{"atUnixMs":<int>,"publisher":"<slug>","kind":"connected"|"paid"|"watch_signup","ref":"<free-form id>"}`.
///
/// Each line becomes the outcome event `record` writes, with the line's
/// timestamp and the `ref` as the note. Lines are deduplicated by
/// (publisher, kind, ref) against the store's events and against earlier
/// lines of the same file, so re-importing the append-only file is
/// idempotent. A malformed line is skipped and counted, never fatal.
pub fn import_outcomes(jsonl: &str, existing: &Snapshot) -> Imported {
    let mut out = Imported::default();
    // One membership set for the history and the lines of this file. The
    // previous shape scanned `existing.events` and the growing output for
    // every line (O(n^2) at 10k outcomes); the identity is the same triple
    // the scan compared: (publisher, outcome, reference note).
    let mut seen_pairs: HashSet<(String, Outcome, String)> = existing
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Outcome {
                publisher: Some(publisher),
                outcome,
                note: Some(note),
                ..
            } => Some((publisher.clone(), *outcome, note.clone())),
            _ => None,
        })
        .collect();
    for (i, line) in jsonl.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record = json::parse(line).ok().and_then(|v| {
            let at_ms = v.get("atUnixMs").and_then(Value::as_i64)?;
            let publisher = v.str_at("publisher")?.trim().to_string();
            let kind = v.str_at("kind")?.trim().to_string();
            let reference = v.str_at("ref")?.trim().to_string();
            if publisher.is_empty() || reference.is_empty() {
                return None;
            }
            Some((at_ms, publisher, kind, reference))
        });
        #[cfg(test)]
        note_outcome_probe();
        let Some((at_ms, publisher, kind, reference)) = record else {
            out.refused
                .push(format!("line {}: not an outcomes record", i + 1));
            continue;
        };
        let outcome = match kind.as_str() {
            "connected" => Outcome::Connected,
            "paid" => Outcome::Paid,
            "watch_signup" => Outcome::WatchSignup,
            "unsubscribe" => Outcome::Unsubscribe,
            other => {
                out.refused
                    .push(format!("line {}: unknown kind {other}", i + 1));
                continue;
            }
        };
        if !seen_pairs.insert((publisher.clone(), outcome, reference.clone())) {
            out.already += 1;
            continue;
        }
        out.events.push(record_outcome(
            &publisher,
            outcome,
            Some(reference),
            at_ms.div_euclid(1000),
        ));
    }
    out
}

/// Refresh a publisher's public signals: its `app-ads.txt` and its App
/// Store listings. Returns the updated publisher and any contacts listed in
/// `CONTACT=` lines.
pub fn discover(
    publisher: &Publisher,
    fetcher: &dyn Fetcher,
    now: UnixSecs,
) -> (Publisher, Vec<Contact>, Vec<String>) {
    let mut p = publisher.clone();
    let mut contacts = Vec::new();
    let mut notes = Vec::new();
    if let Some(domain) = p.domain.clone() {
        let url = format!("https://{domain}/app-ads.txt");
        match fetcher.get(&url) {
            Ok(f) if f.status == 200 && f.body.contains(',') => {
                let report = parse_app_ads_txt(&f.body);
                p.ads_txt_text = Some(f.body);
                p.ads_txt_missing = false;
                p.evidence.retain(|e| e.kind != EvidenceKind::AppAdsTxt);
                for (i, finding) in ads_txt_findings(&domain, Some(&report))
                    .into_iter()
                    .enumerate()
                {
                    p.evidence.push(Evidence {
                        id: format!("ads-{}-{i}", finding.code),
                        kind: EvidenceKind::AppAdsTxt,
                        source: url.clone(),
                        statement: finding.statement,
                        observed_at: now,
                    });
                }
                for email in report.contact_emails() {
                    contacts.push(Contact {
                        kind: ContactKind::of_email(&email),
                        email,
                        name: None,
                        role: None,
                        publisher_key: p.key.clone(),
                        source: url.clone(),
                        consent: false,
                    });
                }
                notes.push(format!(
                    "{url}: {} records, {} errors",
                    report.records.len(),
                    report.errors.len()
                ));
            }
            Ok(f) if f.status == 404 || f.status == 410 || f.status == 200 => {
                p.ads_txt_text = None;
                p.ads_txt_missing = true;
                p.evidence.retain(|e| e.kind != EvidenceKind::AppAdsTxt);
                for finding in ads_txt_findings(&domain, None) {
                    p.evidence.push(Evidence {
                        id: format!("ads-{}", finding.code),
                        kind: EvidenceKind::AppAdsTxt,
                        source: url.clone(),
                        statement: finding.statement,
                        observed_at: now,
                    });
                }
                notes.push(format!("{url}: not found ({})", f.status));
            }
            Ok(f) => notes.push(format!("{url}: status {} left unchanged", f.status)),
            Err(e) => notes.push(format!("{url}: {e}")),
        }
    }
    let ids: Vec<String> = p
        .apps
        .iter()
        .filter(|a| a.store == Store::AppStore)
        .map(|a| a.id.clone())
        .collect();
    if !ids.is_empty() {
        let url = format!("https://itunes.apple.com/lookup?id={}", ids.join(","));
        match fetcher.get(&url) {
            Ok(f) if f.status == 200 => {
                for fact in parse_itunes_results(&f.body, now) {
                    if let Some(app) = p.apps.iter_mut().find(|a| a.id == fact.app.id) {
                        *app = fact.app.clone();
                    }
                    if p.domain.is_none() {
                        p.domain = fact.seller_url.as_deref().and_then(normalize_domain);
                    }
                    if let Some(n) = fact.app.rating_count {
                        let id = format!("store-{}", fact.app.id);
                        p.evidence.retain(|e| e.id != id);
                        p.evidence.push(Evidence {
                            id,
                            kind: EvidenceKind::StoreListing,
                            source: format!("https://apps.apple.com/app/id{}", fact.app.id),
                            statement: format!(
                                "{} has {n} ratings on the App Store",
                                fact.app.name
                            ),
                            observed_at: now,
                        });
                    }
                }
                notes.push(format!("App Store: refreshed {} app(s)", ids.len()));
            }
            Ok(f) => notes.push(format!("App Store lookup: status {}", f.status)),
            Err(e) => notes.push(format!("App Store lookup: {e}")),
        }
    }
    (p, contacts, notes)
}

/// Classify a reply and turn it into an outcome event. The contact is the
/// sender (`From:`) when it is known; otherwise `None` (the caller asks a
/// person).
pub fn ingest_reply(
    raw: &str,
    snap: &Snapshot,
    classifier: &ReplyClassifier,
    now: UnixSecs,
) -> (Option<Event>, crate::classify::Classification) {
    let c = classifier.classify(raw);
    let (headers, _) = crate::classify::split_message(raw);
    let from = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("from"))
        .and_then(|(_, v)| {
            let inner = v.rsplit('<').next().unwrap_or(v).trim_end_matches('>');
            normalize_email(inner)
        });
    // A bounce comes from the mailer daemon; the failed recipient is in the
    // report body. Look for a known contact address there.
    let email = match c.outcome {
        crate::model::Outcome::HardBounce | crate::model::Outcome::SoftBounce => snap
            .contacts
            .keys()
            .find(|e| raw.to_ascii_lowercase().contains(e.as_str()))
            .cloned(),
        _ => from.filter(|e| snap.contacts.contains_key(e)),
    };
    let event = email.map(|e| Event::Outcome {
        at: now,
        email: Some(e),
        publisher: None,
        outcome: c.outcome,
        note: Some(c.rationale.clone()),
    });
    (event, c)
}

/// The settled first-touch prediction pairs used for out-of-sample
/// calibration: `(predicted interest probability at send time, interested)`.
///
/// A lead is settled when it became interested or [`Config::settle_days`]
/// passed since its last touch; unsettled leads are excluded, so the pairs
/// never count an unanswered touch as a failure early. This is the exact
/// pair set [`report`] prints its Brier score, log loss and reliability
/// bins for.
pub fn calibration_pairs(learned: &Learned, config: &Config, now: UnixSecs) -> Vec<(f64, bool)> {
    let mut pairs = Vec::new();
    for pr in learned.progress.values() {
        let Some(first) = pr.touches.first() else {
            continue;
        };
        let Some(p) = first.predicted else { continue };
        let last = pr.touches.last().map_or(0, |t| t.at);
        let y = pr.interested_at.is_some();
        if y || now - last > config.settle_days * DAY {
            pairs.push((p, y));
        }
    }
    pairs
}

/// The two groups of the randomized holdout and their audit-request
/// conversions, counting publishers (not contacts): `treated` is the
/// emailed group, `holdout` the never-emailed group. A publisher counts as
/// converted when it reached at least [`Stage::AuditRequested`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HoldoutGroups {
    /// Interested publishers in the treated group.
    pub treated_conv: f64,
    /// Publishers in the treated group.
    pub treated_n: f64,
    /// Interested publishers in the held-out group.
    pub holdout_conv: f64,
    /// Publishers in the held-out group.
    pub holdout_n: f64,
}

/// Count the holdout groups as of `learned`. Feed the result to
/// [`crate::holdout::effect`] for the Newcombe interval; with a zero-sized
/// group the effect is honestly not estimable yet.
pub fn holdout_groups(snap: &Snapshot, learned: &Learned, config: &Config) -> HoldoutGroups {
    let mut groups = HoldoutGroups::default();
    for key in snap.publishers.keys() {
        let converted = learned
            .stages
            .get(key)
            .is_some_and(|s| s.reached() >= Stage::AuditRequested);
        if config.holdout.is_held_out(key) {
            groups.holdout_n += 1.0;
            if converted {
                groups.holdout_conv += 1.0;
            }
        } else {
            groups.treated_n += 1.0;
            if converted {
                groups.treated_conv += 1.0;
            }
        }
    }
    groups
}

/// A plain-text report of what the engine has learned.
pub fn report(snap: &Snapshot, learned: &Learned, config: &Config, now: UnixSecs) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "apex-sales report, {}", crate::calendar::ymd(now));
    let _ = writeln!(
        out,
        "\npublishers {}  contacts {}  events {}  settled leads {}",
        snap.publishers.len(),
        snap.contacts.len(),
        snap.events.len(),
        learned.scoring_observations
    );
    let domains = learned.health.domains();
    match domains.len() {
        0 => {
            let _ = writeln!(out, "\nsending domains: none configured");
        }
        1 => {
            if let Some(h) = learned.health.get(&domains[0]) {
                let _ = writeln!(
                    out,
                    "\nsending domain: {}  cap {}  sent today {}  P(complaints > {:.2}%) = {:.2}  P(bounces > {:.0}%) = {:.2}",
                    h.state.as_str(),
                    h.cap,
                    learned.health.sent_today_for(&domains[0]),
                    config.health.complaint_ceiling * 100.0,
                    h.p_complaints_over(&config.health),
                    config.health.bounce_ceiling * 100.0,
                    h.p_bounces_over(&config.health),
                );
            }
        }
        n => {
            let _ = writeln!(
                out,
                "\nsending domains: {}  total cap {}  sent today {}",
                n,
                learned.health.total_cap(),
                learned.health.sent_today()
            );
            for domain in domains {
                if let Some(h) = learned.health.get(domain) {
                    let _ = writeln!(
                        out,
                        "  {domain}: {}  cap {}  sent today {}  P(complaints > {:.2}%) = {:.2}  P(bounces > {:.0}%) = {:.2}",
                        h.state.as_str(),
                        h.cap,
                        learned.health.sent_today_for(domain),
                        config.health.complaint_ceiling * 100.0,
                        h.p_complaints_over(&config.health),
                        config.health.bounce_ceiling * 100.0,
                        h.p_bounces_over(&config.health),
                    );
                }
            }
        }
    }

    let _ = writeln!(out, "\nlead score (log-odds, posterior mean ± sd):");
    for ((name, mean), precision) in FEATURES
        .iter()
        .zip(&learned.scoring.mean)
        .zip(&learned.scoring.precision)
        .take(N)
    {
        let _ = writeln!(
            out,
            "  {name:<20} {mean:+.2} ± {:.2}",
            1.0 / precision.sqrt()
        );
    }

    let mut rng = Rng::seeded(now as u64);
    for step in 0..config.max_steps {
        let arms: Vec<crate::bandit::Arm> = crate::compose::templates_for_step(step)
            .iter()
            .filter_map(|t| learned.arms.get(t.id).cloned())
            .collect();
        if arms.is_empty() {
            continue;
        }
        let (rows, loss) = arm_report(&arms, &config.bandit, 4000, &mut rng);
        let _ = writeln!(
            out,
            "\nstep {step} messages (value = expected paid conversions per send):"
        );
        for r in rows {
            let mut marker = String::new();
            if !r.safe {
                marker.push_str("  UNSAFE: complaint ceiling");
            }
            let _ = writeln!(
                out,
                "  {:<12} value {:.5}  P(best) {:.2}{marker}",
                r.id, r.mean_value, r.p_best,
            );
        }
        let _ = writeln!(out, "  expected loss of choosing the leader now: {loss:.6}");
    }

    let _ = writeln!(
        out,
        "\nfunnel after an interested reply (pooled by size band):"
    );
    for band in SizeBand::ALL {
        let _ = writeln!(
            out,
            "  {:<8} audit {:.2}  connect {:.2}  pay {:.2}  value of interest ${:.0}",
            band.as_str(),
            learned.funnel.band_rate(band, 0),
            learned.funnel.band_rate(band, 1),
            learned.funnel.band_rate(band, 2),
            learned.funnel.value_of_interest(band)
        );
    }

    // Out-of-sample calibration of first-touch predictions.
    let pairs = calibration_pairs(learned, config, now);
    if !pairs.is_empty() {
        let (bins, ece) = reliability(&pairs, 10);
        let _ = writeln!(
            out,
            "\ncalibration on {} settled leads: Brier {:.4}  log loss {:.4}  ECE {:.4}",
            pairs.len(),
            brier(&pairs).unwrap_or(f64::NAN),
            log_loss(&pairs).unwrap_or(f64::NAN),
            ece.unwrap_or(f64::NAN)
        );
        for b in bins {
            let _ = writeln!(
                out,
                "  predicted {:.3}  observed {:.3}  [{:.3}, {:.3}]  n={}",
                b.mean_predicted, b.observed, b.interval.low, b.interval.high, b.n
            );
        }
    }

    // Causal effect: emailed versus held-out publishers reaching the audit.
    let groups = holdout_groups(snap, learned, config);
    match effect(
        groups.treated_conv,
        groups.treated_n,
        groups.holdout_conv,
        groups.holdout_n,
    ) {
        Some(e) => {
            let _ = writeln!(
            out,
            "\noutreach effect on audit requests: emailed {:.2}% vs held out {:.2}%, difference {:+.2} points [{:+.2}, {:+.2}], about {:.1} extra audits",
            e.treated_rate * 100.0,
            e.holdout_rate * 100.0,
            e.difference * 100.0,
            e.interval.low * 100.0,
            e.interval.high * 100.0,
            e.incremental_conversions
        );
        }
        None => {
            let _ = writeln!(
                out,
                "\noutreach effect on audit requests: not measurable yet ({} emailed-group and {} held-out publishers)",
                groups.treated_n, groups.holdout_n
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::Sender;
    use crate::engine::learn;
    use crate::io::MapFetcher;

    /// A seed CSV with `domains` distinct publishers, each submitted
    /// `revisions` times in an interleaved order with different names,
    /// countries and app ids. Duplicate domains exercise the merge rule.
    fn seed_csv(domains: usize, revisions: usize) -> String {
        let mut csv = String::from("domain,name,country,app_store_ids,email,contact_name,role\n");
        for revision in 0..revisions {
            for index in 0..domains {
                let domain = format!("studio{index}.example");
                let country = ["FI", "SE", "EE"][(index + revision) % 3];
                let ids = format!("{};{}", index * 10, index * 10 + (revision % 3));
                let _ = writeln!(
                    csv,
                    "{domain},Studio {index} r{revision},{country},{ids},owner{index}@{domain},Owner {index},CEO"
                );
            }
        }
        csv
    }

    /// The pre-fix algorithm, copied shape for shape: a scan of the growing
    /// output for every seed and a retain over it. Used as the correctness
    /// oracle and as the comparison-count baseline.
    fn naive_import_seeds(csv: &str, existing: &Snapshot) -> Imported {
        let rows = parse_csv(csv);
        let mut out = Imported::default();
        let Some(header) = rows.first() else {
            return out;
        };
        let col = |name: &str| {
            header
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(name))
        };
        let (c_domain, c_name, c_country, c_ids, c_email, c_cname, c_role) = (
            col("domain"),
            col("name"),
            col("country"),
            col("app_store_ids"),
            col("email"),
            col("contact_name"),
            col("role"),
        );
        for (i, row) in rows.iter().enumerate().skip(1) {
            let get = |c: Option<usize>| {
                c.and_then(|c| row.get(c))
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
            };
            let Some(email) = get(c_email).and_then(normalize_email) else {
                out.refused
                    .push(format!("row {}: missing or invalid email", i + 1));
                continue;
            };
            let domain = get(c_domain)
                .and_then(normalize_domain)
                .or_else(|| normalize_domain(email.rsplit('@').next().unwrap_or_default()));
            let Some(domain) = domain else {
                out.refused.push(format!("row {}: no usable domain", i + 1));
                continue;
            };
            let mut p = existing
                .publishers
                .get(&domain)
                .cloned()
                .or_else(|| out.publishers.iter().find(|p| p.key == domain).cloned())
                .unwrap_or_else(|| Publisher::new(domain.clone(), get(c_name).unwrap_or(&domain)));
            p.domain = Some(domain.clone());
            if let Some(name) = get(c_name) {
                p.name = name.to_string();
            }
            if let Some(c) = get(c_country) {
                p.country = Some(c.to_ascii_uppercase());
            }
            for id in get(c_ids)
                .unwrap_or_default()
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if id.chars().all(|c| c.is_ascii_digit()) && !p.apps.iter().any(|a| a.id == id) {
                    p.apps.push(App {
                        store: Store::AppStore,
                        id: id.to_string(),
                        name: format!("App {id}"),
                        genre: None,
                        rating_count: None,
                        free: None,
                        days_since_update: None,
                    });
                }
            }
            out.publishers.retain(|q| q.key != p.key);
            out.publishers.push(p);
            out.contacts.push(Contact {
                kind: ContactKind::of_email(&email),
                email,
                name: get(c_cname).map(str::to_string),
                role: get(c_role).map(str::to_string),
                publisher_key: domain,
                source: "seed list".into(),
                consent: false,
            });
        }
        out
    }

    #[test]
    fn sixteen_thousand_seed_rows_match_the_naive_reference() {
        let csv = seed_csv(4_000, 4);
        assert_eq!(csv.lines().count(), 16_001, "16k data rows plus header");

        // A seed that already exists in the snapshot must merge the same way.
        let mut existing = Snapshot::default();
        let mut seeded = Publisher::new("studio0.example", "Existing Studio");
        seeded.country = Some("DE".to_string());
        existing
            .publishers
            .insert("studio0.example".to_string(), seeded);

        let fast = import_seeds(&csv, &existing);
        let naive = naive_import_seeds(&csv, &existing);
        assert_eq!(
            fast, naive,
            "the indexed import preserves order, merges and contacts exactly"
        );
        assert_eq!(fast.publishers.len(), 4_000);
        assert_eq!(fast.contacts.len(), 16_000);
        assert!(fast.refused.is_empty());
        assert_eq!(
            fast.publishers.last().map(|p| p.key.as_str()),
            Some("studio3999.example"),
            "the last row's domain is last"
        );
        let first = fast
            .publishers
            .iter()
            .find(|p| p.key == "studio0.example")
            .expect("merged seed");
        assert_eq!(first.name, "Studio 0 r3", "the last update wins");
        assert_eq!(
            first.apps.len(),
            1,
            "an existing snapshot row is the base for every repeat, as before"
        );
        let merged = fast
            .publishers
            .iter()
            .find(|p| p.key == "studio1.example")
            .expect("new seed");
        assert_eq!(
            merged.apps.len(),
            3,
            "app ids merge across revisions for a new seed"
        );
    }

    #[test]
    fn seed_membership_probes_are_linear_in_the_row_count() {
        for row_count in [4_000usize, 8_000, 16_000] {
            let domains = row_count / 4;
            let csv = seed_csv(domains, 4);
            reset_seed_probes();
            let out = import_seeds(&csv, &Snapshot::default());
            let probes = seed_probe_count();
            // The old `out.publishers.iter().find` scanned every distinct
            // publisher already emitted: on the first revision that is
            // d(d-1)/2 comparisons and on each later revision d*d, so this
            // baseline quadruples when the fixture doubles in both rows and
            // domains. The counter shows the fixed path at one probe per row.
            let naive_comparisons =
                (domains * (domains - 1) / 2) as u64 + ((3 * domains * domains) as u64);
            println!(
                "seed_probes rows={row_count} domains={domains} probes={probes} naive_comparisons={naive_comparisons}"
            );
            assert_eq!(out.publishers.len(), domains);
            assert_eq!(
                probes, row_count as u64,
                "rows={row_count}: one index probe per row, linear"
            );
            assert!(
                naive_comparisons > probes * 100,
                "rows={row_count}: the pre-fix scan is quadratic"
            );
        }
    }

    #[test]
    fn outcome_membership_probes_are_linear_at_ten_thousand_records() {
        let line = |index: usize| {
            format!(
                "{{\"atUnixMs\":{},\"publisher\":\"studio{index}.example\",\"kind\":\"connected\",\"ref\":\"evt_{index}\"}}",
                index * 1_000
            )
        };
        let jsonl: String = (0..10_000)
            .map(|index| format!("{}\n", line(index)))
            .collect();
        reset_outcome_probes();
        let first = import_outcomes(&jsonl, &Snapshot::default());
        let probes = outcome_probe_count();
        assert_eq!(first.events.len(), 10_000);
        assert_eq!(probes, 10_000, "one set probe per line, linear");
        // Replay: every line is found in the existing history, still one
        // probe per line rather than a scan of the growing history.
        let snapshot = Snapshot {
            events: first.events,
            ..Snapshot::default()
        };
        reset_outcome_probes();
        let replay = import_outcomes(&jsonl, &snapshot);
        let replay_probes = outcome_probe_count();
        assert_eq!(replay.already, 10_000);
        assert!(replay.events.is_empty());
        assert_eq!(replay_probes, 10_000, "the replay is linear too");
        println!(
            "outcome_probes records=10000 first={probes} replay={replay_probes} naive_scan={}",
            10_000u64 * 10_000
        );
    }

    #[test]
    fn csv_parses_quotes_and_line_endings() {
        let rows = parse_csv("a,b\r\n\"x, y\",\"say \"\"hi\"\"\"\n\nlast");
        assert_eq!(
            rows,
            vec![
                vec!["a".to_string(), "b".to_string()],
                vec!["x, y".to_string(), "say \"hi\"".to_string()],
                vec!["last".to_string()],
            ]
        );
        assert!(parse_csv("").is_empty());
    }

    #[test]
    fn seeds_import_with_refusals() {
        let csv = "domain,name,country,app_store_ids,email,contact_name,role\n\
                   www.Studio.example,Studio,fi,123;456;bad,Maria@Studio.example,Maria Lopez,CEO\n\
                   ,,,,info@other.example,,\n\
                   x.example,X,,,not-an-email,,\n";
        let out = import_seeds(csv, &Snapshot::default());
        assert_eq!(out.publishers.len(), 2);
        let p = &out.publishers[0];
        assert_eq!(
            (p.key.as_str(), p.country.as_deref(), p.apps.len()),
            ("studio.example", Some("FI"), 2)
        );
        assert_eq!(out.contacts[0].email, "maria@studio.example");
        assert_eq!(out.contacts[1].publisher_key, "other.example");
        assert_eq!(out.contacts[1].kind, ContactKind::Generic);
        assert_eq!(out.refused.len(), 1);
        assert!(import_seeds("", &Snapshot::default()).publishers.is_empty());
    }

    #[test]
    fn leads_import_as_consenting_inbound_audit_requests() {
        let jsonl = "{\"atUnixMs\":1700000000000,\"lead\":{\"publisher\":\"tiny-fox\",\"email\":\"Ana@TinyFox.io\",\"platforms\":[\"max\"],\"monthlyRevenueRange\":\"$20k-$50k\",\"displayName\":\"Tiny Fox\",\"requestId\":\"req_1\"}}\n\
                     {\"atUnixMs\":1700000001000,\"lead\":{\"publisher\":\"solo\",\"email\":\"dev@gmail.com\"}}\n\
                     garbage\n";
        let out = import_leads(jsonl, &Snapshot::default());
        assert_eq!(out.publishers.len(), 2);
        let p = out
            .publishers
            .iter()
            .find(|p| p.key == "tinyfox.io")
            .unwrap();
        assert!(p.inbound && p.declared_platforms == vec!["max".to_string()]);
        assert!(p.evidence[0].statement.contains("$20k-$50k"));
        assert!(out
            .publishers
            .iter()
            .any(|p| p.key == "lead-solo" && p.domain.is_none()));
        assert!(out.contacts.iter().all(|c| c.consent));
        assert_eq!(out.events.len(), 2);
        assert_eq!(out.refused.len(), 1);
        assert_eq!(out.already, 0);
        // Re-importing the append-only file records nothing twice.
        let mut snap = Snapshot::default();
        snap.events.extend(out.events.iter().cloned());
        let again = import_leads(jsonl, &snap);
        assert!(again.events.is_empty() && again.contacts.is_empty());
        assert_eq!(again.already, 2);
    }

    #[test]
    fn outcomes_import_idempotently_and_skip_malformed_lines() {
        let frozen = "{\"atUnixMs\":1700000000000,\"publisher\":\"tinyfox.io\",\"kind\":\"connected\",\"ref\":\"evt_1\"}\n\
                      {\"atUnixMs\":1700000001000,\"publisher\":\"tinyfox.io\",\"kind\":\"paid\",\"ref\":\"inv_9\"}\n\
                      {\"atUnixMs\":1700000002000,\"publisher\":\"solo\",\"kind\":\"watch_signup\",\"ref\":\"sub_3\"}\n\
                      {\"atUnixMs\":1700000003000,\"publisher\":\"tinyfox.io\",\"kind\":\"connected\",\"ref\":\"evt_1\"}\n\
                      garbage\n\
                      {\"atUnixMs\":1700000004000,\"publisher\":\"x.io\",\"kind\":\"exploded\",\"ref\":\"e\"}\n\
                      {\"atUnixMs\":\"soon\",\"publisher\":\"x.io\",\"kind\":\"paid\",\"ref\":\"e\"}\n\
                      {\"atUnixMs\":1700000005000,\"publisher\":\"\",\"kind\":\"paid\",\"ref\":\"e\"}\n";
        let out = import_outcomes(frozen, &Snapshot::default());
        assert_eq!(out.events.len(), 3, "{:#?}", out.events);
        assert_eq!(out.already, 1, "the repeated (publisher, kind, ref) line");
        assert_eq!(out.refused.len(), 4, "{:#?}", out.refused);
        assert_eq!(
            out.events[0],
            Event::Outcome {
                at: 1_700_000_000,
                email: None,
                publisher: Some("tinyfox.io".into()),
                outcome: Outcome::Connected,
                note: Some("evt_1".into()),
            }
        );
        assert!(matches!(
            &out.events[2],
            Event::Outcome {
                outcome: Outcome::WatchSignup,
                ..
            }
        ));
        // Replaying the append-only file records nothing twice.
        let mut snap = Snapshot::default();
        snap.events.extend(out.events.iter().cloned());
        let again = import_outcomes(frozen, &snap);
        assert!(again.events.is_empty());
        assert_eq!(again.already, 4);
        assert_eq!(again.refused.len(), 4);
    }

    #[test]
    fn an_unsubscribe_outcome_suppresses_the_contact_email() {
        let jsonl = "{\"atUnixMs\":10,\"publisher\":\"maria@studio.example\",\"kind\":\"unsubscribe\",\"ref\":\"tok\"}\n";
        let out = import_outcomes(jsonl, &Snapshot::default());
        assert_eq!(out.events.len(), 1);
        assert!(matches!(
            &out.events[0],
            Event::Outcome {
                outcome: Outcome::Unsubscribe,
                email: Some(email),
                ..
            } if email == "maria@studio.example"
        ));
    }

    #[test]
    fn record_outcome_matches_the_cli_record_semantics() {
        let by_email = record_outcome("Maria@Studio.Example", Outcome::Connected, None, 7);
        assert_eq!(
            by_email,
            Event::Outcome {
                at: 7,
                email: Some("maria@studio.example".into()),
                publisher: None,
                outcome: Outcome::Connected,
                note: None,
            }
        );
        let by_publisher = record_outcome("studio.example", Outcome::Paid, Some("inv_1".into()), 8);
        assert_eq!(
            by_publisher,
            Event::Outcome {
                at: 8,
                email: None,
                publisher: Some("studio.example".into()),
                outcome: Outcome::Paid,
                note: Some("inv_1".into()),
            }
        );
    }

    #[test]
    fn discovery_reads_app_ads_txt_and_the_app_store() {
        let mut p = Publisher::new("studio.example", "Studio");
        p.domain = Some("studio.example".into());
        p.apps.push(App {
            store: Store::AppStore,
            id: "42".into(),
            name: "App 42".into(),
            genre: None,
            rating_count: None,
            free: None,
            days_since_update: None,
        });
        let f = MapFetcher {
            pages: vec![
                (
                    "https://studio.example/app-ads.txt".into(),
                    200,
                    "applovin.com, 1, DIRECT\nbad\nCONTACT=ads@studio.example\n".into(),
                ),
                (
                    "https://itunes.apple.com/lookup?id=42".into(),
                    200,
                    r#"{"results":[{"kind":"software","trackId":42,"trackName":"Tile Quest","userRatingCount":9000,"price":0}]}"#.into(),
                ),
            ],
        };
        let (q, contacts, notes) = discover(&p, &f, 100);
        assert!(q.ads_txt_text.is_some() && !q.ads_txt_missing);
        assert!(q.evidence.iter().any(|e| e.statement.contains("1 line of")));
        assert!(q
            .evidence
            .iter()
            .any(|e| e.statement == "Tile Quest has 9000 ratings on the App Store"));
        assert_eq!(q.apps[0].rating_count, Some(9000));
        assert_eq!(contacts[0].email, "ads@studio.example");
        assert_eq!(notes.len(), 2);

        let missing = MapFetcher {
            pages: vec![(
                "https://studio.example/app-ads.txt".into(),
                404,
                String::new(),
            )],
        };
        let (q, _, notes) = discover(&p, &missing, 100);
        assert!(q.ads_txt_missing);
        assert!(q.evidence.iter().any(|e| e.id == "ads-ads_txt_missing"));
        assert!(notes.iter().any(|n| n.contains("no route")), "{notes:?}");
        let flaky = MapFetcher {
            pages: vec![(
                "https://studio.example/app-ads.txt".into(),
                503,
                String::new(),
            )],
        };
        let (q, _, notes) = discover(&p, &flaky, 100);
        assert!(!q.ads_txt_missing && notes[0].contains("503"));
    }

    #[test]
    fn replies_are_ingested_and_reports_render() {
        let mut snap = Snapshot::default();
        let mut p = Publisher::new("studio.example", "Studio");
        p.domain = Some("studio.example".into());
        snap.publishers.insert(p.key.clone(), p);
        snap.contacts.insert(
            "maria@studio.example".into(),
            Contact {
                email: "maria@studio.example".into(),
                name: None,
                role: None,
                publisher_key: "studio.example".into(),
                kind: ContactKind::Named,
                source: "t".into(),
                consent: false,
            },
        );
        let c = ReplyClassifier::default();
        let (e, cls) = ingest_reply(
            "From: Maria <Maria@studio.example>\nSubject: Re: x\n\nYes, please run the free audit.",
            &snap,
            &c,
            10,
        );
        assert!(
            matches!(
                e,
                Some(Event::Outcome {
                    outcome: crate::model::Outcome::Reply(crate::model::ReplyClass::Positive),
                    ..
                })
            ),
            "{cls:?}"
        );
        let bounce =
            "From: MAILER-DAEMON@mx.example\n\nStatus: 5.1.1\nmaria@studio.example: user unknown";
        let (e, _) = ingest_reply(bounce, &snap, &c, 10);
        assert!(matches!(
            e,
            Some(Event::Outcome {
                outcome: crate::model::Outcome::HardBounce,
                ..
            })
        ));
        let (e, _) = ingest_reply("From: stranger@else.example\n\nhello", &snap, &c, 10);
        assert!(e.is_none());

        snap.events.push(Event::Touch {
            at: 0,
            email: "maria@studio.example".into(),
            publisher: "studio.example".into(),
            from: None,
            step: 0,
            arm: "t0-finding".into(),
            message_id: "m".into(),
            local_hour: 10,
            predicted: Some(0.05),
        });
        let config = Config::new(Sender {
            name: "A".into(),
            email: "a@apex.example".into(),
            company: "Apex".into(),
            postal_address: "Tallinn".into(),
            unsubscribe_url: None,
        });
        let now = 60 * DAY;
        let learned = learn(&snap, &config, now);
        let text = report(&snap, &learned, &config, now);
        for needle in [
            "sending domain",
            "lead score",
            "step 0 messages",
            "funnel",
            "calibration on 1",
            "outreach effect",
        ] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
    }

    #[test]
    fn calibration_pairs_and_holdout_groups_are_the_reports_building_blocks() {
        let mut snap = Snapshot::default();
        let mut p = Publisher::new("studio.example", "Studio");
        p.domain = Some("studio.example".into());
        snap.publishers.insert(p.key.clone(), p);
        snap.contacts.insert(
            "maria@studio.example".into(),
            Contact {
                email: "maria@studio.example".into(),
                name: None,
                role: None,
                publisher_key: "studio.example".into(),
                kind: ContactKind::Named,
                source: "t".into(),
                consent: false,
            },
        );
        let config = Config::new(Sender {
            name: "A".into(),
            email: "a@apex.example".into(),
            company: "Apex".into(),
            postal_address: "Tallinn".into(),
            unsubscribe_url: None,
        });
        let now = 60 * DAY;
        // An unsettled touch contributes no calibration pair.
        snap.events.push(Event::Touch {
            at: now - DAY,
            email: "maria@studio.example".into(),
            publisher: "studio.example".into(),
            from: None,
            step: 0,
            arm: "t0-finding".into(),
            message_id: "m".into(),
            local_hour: 10,
            predicted: Some(0.05),
        });
        let learned = learn(&snap, &config, now);
        assert!(calibration_pairs(&learned, &config, now).is_empty());
        // A reply settles it as an interested observation.
        snap.events.push(Event::Outcome {
            at: now - DAY + 3600,
            email: Some("maria@studio.example".into()),
            publisher: None,
            outcome: crate::model::Outcome::Reply(crate::model::ReplyClass::Positive),
            note: None,
        });
        snap.events.sort_by_key(Event::at);
        let learned = learn(&snap, &config, now);
        let pairs = calibration_pairs(&learned, &config, now);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, 0.05);
        assert!(pairs[0].1);
        let groups = holdout_groups(&snap, &learned, &config);
        assert_eq!((groups.treated_n, groups.holdout_n), (1.0, 0.0));
        assert_eq!(groups.treated_conv, 0.0);
        // Reaching the audit moves the publisher into the converted count.
        snap.events.push(Event::Outcome {
            at: now - DAY + 7200,
            email: None,
            publisher: Some("studio.example".into()),
            outcome: crate::model::Outcome::AuditRequested,
            note: None,
        });
        snap.events.sort_by_key(Event::at);
        let learned = learn(&snap, &config, now);
        let groups = holdout_groups(&snap, &learned, &config);
        assert_eq!((groups.treated_conv, groups.treated_n), (1.0, 1.0));
        // A zero-sized group makes the effect not estimable, not zero.
        assert!(effect(
            groups.treated_conv,
            groups.treated_n,
            groups.holdout_conv,
            groups.holdout_n
        )
        .is_none());
    }
}
