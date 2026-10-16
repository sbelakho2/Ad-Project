//! An end-to-end simulator: the real engine against a synthetic market.
//!
//! A world of publishers is drawn with hidden "true" response parameters
//! that deliberately differ from the engine's priors (so the engine has to
//! learn), true per-template effects, true per-step interest shares and
//! complaint rates, and a true funnel. Each simulated day the policy plans
//! with the production [`crate::engine::learn`] and [`crate::engine::plan`]
//! (or the naive baseline), "sends", and the world answers with replies,
//! complaints and funnel progress as events, exactly as production would
//! record them. Nothing in the engine knows it is being simulated.
//!
//! The baseline is what a typical outbound tool does: contacts in random
//! order, one fixed template per step, every follow-up sent, the same daily
//! cap and the same gates.

use std::collections::BTreeMap;

use crate::compose::Sender;
use crate::decision::{Autonomy, Verdict};
use crate::engine::{learn, plan, Config};
use crate::model::{
    App, Contact, ContactKind, Evidence, EvidenceKind, Outcome, Publisher, ReplyClass, Store, DAY,
};
use crate::rng::Rng;
use crate::signals::SizeBand;
use crate::store::{Event, Snapshot};

/// Which policy drives the sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// The engine.
    Engine,
    /// Random order, fixed templates, every follow-up.
    Baseline,
}

/// Simulation size.
#[derive(Debug, Clone, PartialEq)]
pub struct SimConfig {
    /// Publishers in the world.
    pub publishers: usize,
    /// Days simulated.
    pub days: i64,
    /// Seed.
    pub seed: u64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            publishers: 1500,
            days: 90,
            seed: 1,
        }
    }
}

/// What a run produced.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SimResult {
    /// Messages sent.
    pub sends: u32,
    /// Interested replies.
    pub interested: u32,
    /// Spam complaints.
    pub complaints: u32,
    /// Publishers that paid.
    pub paid: u32,
    /// Expected paid revenue of the interested replies (USD), using the true
    /// funnel.
    pub expected_revenue: f64,
    /// Expected paid revenue of every send under the true response model,
    /// whether or not that send happened to draw a reply: the low-noise
    /// measure of the policy's choices.
    pub value_of_sends: f64,
}

#[derive(Debug, Clone)]
struct Truth {
    p_sequence: f64,
    complaint_scale: f64,
    band: SizeBand,
}

const TRUE_STEP_SHARE: [f64; 4] = [0.55, 0.25, 0.12, 0.08];
const TRUE_FUNNEL: [f64; 3] = [0.5, 0.35, 0.6];

fn template_effect(arm: &str) -> f64 {
    match arm {
        "t0-finding" => 1.35,
        "t0-proof" => 1.0,
        "t0-short" => 0.75,
        _ => 1.0,
    }
}

fn ltv(band: SizeBand) -> f64 {
    match band {
        SizeBand::Unknown => 2_400.0,
        SizeBand::Small => 1_200.0,
        SizeBand::Core => 6_000.0,
        SizeBand::Large => 18_000.0,
    }
}

fn build_world(cfg: &SimConfig, rng: &mut Rng) -> (Snapshot, BTreeMap<String, Truth>) {
    let mut snap = Snapshot::default();
    let mut truth = BTreeMap::new();
    for i in 0..cfg.publishers {
        let key = format!("pub{i}.example");
        let mut p = Publisher::new(&key, format!("Publisher {i}"));
        p.domain = Some(key.clone());
        p.country = Some(["US", "GB", "FI", "TR", "VN", "FR"][rng.below(6) as usize].into());
        let ratings = (10f64.powf(1.5 + 3.5 * rng.uniform())) as u64;
        let game = rng.uniform() < 0.7;
        p.apps.push(App {
            store: Store::AppStore,
            id: i.to_string(),
            name: format!("App {i}"),
            genre: Some(if game { "Games" } else { "Utilities" }.into()),
            rating_count: Some(ratings),
            free: Some(rng.uniform() < 0.85),
            days_since_update: Some(rng.below(200) as u32),
        });
        let missing = rng.uniform() < 0.25;
        p.ads_txt_missing = missing;
        if !missing {
            let errors = rng.below(4);
            let mut text = String::from(
                "applovin.com, 1, DIRECT\ngoogle.com, pub-1, DIRECT, f08c47fec0942fa0\n",
            );
            for _ in 0..errors {
                text.push_str("broken line\n");
            }
            p.ads_txt_text = Some(text);
        }
        p.evidence.push(Evidence {
            id: "ev".into(),
            kind: EvidenceKind::AppAdsTxt,
            source: format!("https://{key}/app-ads.txt"),
            statement: if missing {
                format!("https://{key}/app-ads.txt was not found")
            } else {
                format!("https://{key}/app-ads.txt was checked")
            },
            observed_at: 0,
        });
        let kind_roll = rng.uniform();
        let local = if kind_roll < 0.5 {
            "maria"
        } else if kind_roll < 0.65 {
            "bizdev"
        } else {
            "info"
        };
        let email = format!("{local}@{key}");
        let contact = Contact {
            email: email.clone(),
            name: None,
            role: None,
            publisher_key: key.clone(),
            kind: ContactKind::of_email(&email),
            source: "sim".into(),
            consent: false,
        };
        // The true response differs from the engine's priors: size and a
        // missing app-ads.txt matter more, generic mailboxes are worse, games
        // matter less.
        let size = ((ratings as f64 + 1.0).log10() - 3.0) / 1.5;
        let z = -4.0
            + 0.5 * size
            + if missing { 0.7 } else { 0.0 }
            + match contact.kind {
                ContactKind::Named => 0.5,
                ContactKind::Role => 0.2,
                ContactKind::Generic => -1.5,
            };
        let band = SizeBand::of(Some(ratings));
        truth.insert(
            email.clone(),
            Truth {
                p_sequence: crate::stats::sigmoid(z),
                complaint_scale: if contact.kind == ContactKind::Generic {
                    2.0
                } else {
                    1.0
                },
                band,
            },
        );
        snap.publishers.insert(key, p);
        snap.contacts.insert(email, contact);
    }
    (snap, truth)
}

fn sim_config() -> Config {
    let mut c = Config::new(Sender {
        name: "Sim".into(),
        email: "sim@apex.example".into(),
        company: "Apex".into(),
        postal_address: "Tallinn".into(),
        unsubscribe_url: None,
    });
    c.autonomy = Autonomy::Autonomous;
    c.holdout.share = 0.0;
    c
}

/// Run one policy.
pub fn run(policy: Policy, cfg: &SimConfig) -> SimResult {
    let mut rng = Rng::seeded(cfg.seed);
    let (mut snap, truth) = build_world(cfg, &mut rng);
    let mut world_rng = Rng::seeded(cfg.seed ^ 0x5eed);
    let config = sim_config();
    let start = 20_000 * DAY + 7 * 3600;
    let mut result = SimResult::default();
    let mut pending: Vec<Event> = Vec::new();
    let mut baseline_order: Vec<String> = snap.contacts.keys().cloned().collect();
    for i in (1..baseline_order.len()).rev() {
        let j = rng.below((i + 1) as u64) as usize;
        baseline_order.swap(i, j);
    }
    let mut answered: BTreeMap<String, bool> = BTreeMap::new();
    let mut reset_scheduled = false;
    for day in 0..cfg.days {
        let now = start + day * DAY;
        // Deliver world events due by now.
        pending.sort_by_key(Event::at);
        let (due, later): (Vec<Event>, Vec<Event>) =
            pending.into_iter().partition(|e| e.at() <= now);
        pending = later;
        snap.events.extend(due);
        snap.events.sort_by_key(Event::at);
        let learned = learn(&snap, &config, now);
        // An operator investigates a quarantine and resets the domain a week
        // later, as the runbook says.
        if learned.health.state == crate::health::SenderState::Quarantined {
            if !reset_scheduled {
                pending.push(Event::HealthReset { at: now + 7 * DAY });
                reset_scheduled = true;
            }
        } else {
            reset_scheduled = false;
        }
        let items = plan(&snap, &learned, &config, now);
        let mut sends: Vec<(String, usize, String)> = Vec::new();
        match policy {
            Policy::Engine => {
                for it in items.iter().filter(|i| i.decision.verdict == Verdict::Send) {
                    if let Some(arm) = &it.arm {
                        sends.push((it.email.clone(), it.step, arm.clone()));
                    }
                }
            }
            Policy::Baseline => {
                let cap = learned.health.cap.saturating_sub(learned.sent_today) as usize;
                let suppressed = |e: &str| {
                    snap.contacts
                        .get(e)
                        .is_some_and(|c| learned.suppressions.check(c).is_some())
                };
                // Follow-ups first (every one is sent), then new contacts.
                let mut followups = Vec::new();
                for (email, pr) in &learned.progress {
                    let k = pr.touches.len();
                    if k == 0 || k >= config.max_steps || pr.reply.is_some() || suppressed(email) {
                        continue;
                    }
                    let gap = config.step_gap_days.get(k).copied().unwrap_or(7);
                    if pr.touches.last().is_some_and(|t| now >= t.at + gap * DAY) {
                        followups.push((
                            email.clone(),
                            k,
                            ["t0-short", "t1-bump", "t2-value", "t3-close"][k].to_string(),
                        ));
                    }
                }
                sends.extend(followups.into_iter().take(cap));
                for email in &baseline_order {
                    if sends.len() >= cap {
                        break;
                    }
                    if !learned.progress.contains_key(email) && !suppressed(email) {
                        sends.push((email.clone(), 0, "t0-short".into()));
                    }
                }
            }
        }
        for (email, step, arm) in sends {
            let Some(t) = truth.get(&email) else { continue };
            let Some(contact) = snap.contacts.get(&email) else {
                continue;
            };
            let at = now + 2 * 3600;
            snap.events.push(Event::Touch {
                at,
                email: email.clone(),
                publisher: contact.publisher_key.clone(),
                step,
                arm: arm.clone(),
                message_id: format!("{email}-{step}"),
                local_hour: 10,
                predicted: None,
            });
            result.sends += 1;
            if answered.get(&email).copied().unwrap_or(false) {
                continue;
            }
            // Interest hazard at this step, conditional on no earlier reply.
            let before: f64 = TRUE_STEP_SHARE.iter().take(step).sum::<f64>() * t.p_sequence;
            let effect = if step == 0 {
                template_effect(&arm)
            } else {
                1.0
            };
            let h = (t.p_sequence * TRUE_STEP_SHARE.get(step).copied().unwrap_or(0.05) * effect
                / (1.0 - before).max(1e-9))
            .min(1.0);
            let complaint_p = 0.0004 * t.complaint_scale * (1.0 + step as f64);
            result.value_of_sends +=
                (1.0 - complaint_p) * h * TRUE_FUNNEL.iter().product::<f64>() * ltv(t.band);
            if world_rng.uniform() < complaint_p {
                result.complaints += 1;
                answered.insert(email.clone(), true);
                pending.push(Event::Outcome {
                    at: at + 3600,
                    email: Some(email.clone()),
                    publisher: None,
                    outcome: Outcome::Complaint,
                    note: None,
                });
            } else if world_rng.uniform() < h {
                result.interested += 1;
                answered.insert(email.clone(), true);
                result.expected_revenue += TRUE_FUNNEL.iter().product::<f64>() * ltv(t.band);
                let r_at = at + DAY + (world_rng.below(2) as i64) * DAY;
                pending.push(Event::Outcome {
                    at: r_at,
                    email: Some(email.clone()),
                    publisher: None,
                    outcome: Outcome::Reply(ReplyClass::Positive),
                    note: None,
                });
                let pk = Some(contact.publisher_key.clone());
                let mut stage_at = r_at;
                for (s, outcome) in [Outcome::AuditRequested, Outcome::Connected, Outcome::Paid]
                    .into_iter()
                    .enumerate()
                {
                    if world_rng.uniform() >= TRUE_FUNNEL[s] {
                        break;
                    }
                    stage_at += [3, 10, 30][s] * DAY;
                    if outcome == Outcome::Paid && stage_at <= start + cfg.days * DAY {
                        result.paid += 1;
                    }
                    pending.push(Event::Outcome {
                        at: stage_at,
                        email: None,
                        publisher: pk.clone(),
                        outcome,
                        note: None,
                    });
                }
            }
        }
    }
    result
}

/// Run both policies on the same world and seed.
pub fn compare(cfg: &SimConfig) -> (SimResult, SimResult) {
    (run(Policy::Engine, cfg), run(Policy::Baseline, cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_study_is_larger_than_a_test() {
        let d = SimConfig::default();
        assert!(d.publishers >= 1000 && d.days >= 30, "{d:?}");
    }

    #[test]
    fn the_engine_beats_the_baseline_on_the_same_world() {
        let cfg = SimConfig {
            publishers: 1200,
            days: 50,
            seed: 1,
        };
        let (engine, baseline) = compare(&cfg);
        assert!(
            engine.sends > 1000 && baseline.sends > 1000,
            "{engine:?} {baseline:?}"
        );
        let per_send = |r: &SimResult| r.value_of_sends / f64::from(r.sends.max(1));
        assert!(
            engine.value_of_sends > 1.1 * baseline.value_of_sends,
            "engine {engine:?} baseline {baseline:?}"
        );
        assert!(per_send(&engine) > per_send(&baseline));
    }
}
