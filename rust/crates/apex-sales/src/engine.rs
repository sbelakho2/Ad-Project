//! The engine: rebuild every model from history, then plan the next sends.
//!
//! [`learn`] is a pure fold over the [`Snapshot`]: it derives suppressions,
//! account states and funnel stages, replays sending-domain health day by
//! day, and fits the lead score, the funnel, the message bandit, the
//! send-time model and the per-step hazards from settled observations only.
//! A touch is *settled* when it led to interest or a complaint, when a later
//! touch followed it, or when [`Config::settle_days`] have passed; unsettled
//! touches never count as failures.
//!
//! [`plan`] scores every due contact and ranks by expected value:
//!
//! ```text
//! EV(step k) = p̃ · s_k · V_interest(band) − cost_per_touch − c_k · C_complaint
//! ```
//!
//! where `p̃` is a Thompson draw of the contact's sequence-level interest
//! probability, `s_k = h_k / Σ h` the share of interest the population sees
//! at step *k*, `V_interest` the expected paid revenue of an interested reply
//! for the publisher's size band, and `c_k` the step's complaint
//! probability. Ranking on a posterior draw rather than the mean is what
//! makes uncertain-but-promising publishers get contacted and learned from.
//! Every candidate then passes [`crate::decision::decide`]; the daily cap
//! goes to the highest-value survivors.
//!
//! Random draws for a contact are seeded from its touch id, so the plan for
//! one day is stable across the hourly runs that execute it.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::bandit::{choose, Arm, BanditConfig};
use crate::cadence::{SendTimeModel, StepModel};
use crate::calendar::{country_offset_minutes, local_parts, next_weekday_hour};
use crate::compose::{available_templates_for_step, render, Rendered, Sender, TEMPLATES};
use crate::decision::{decide, Autonomy, Decision, GateInput, Verdict};
use crate::funnel::{FunnelModel, Stage};
use crate::health::{HealthBook, HealthConfig};
use crate::holdout::{fnv1a, Holdout};
use crate::model::{Contact, Outcome, Publisher, ReplyClass, UnixSecs, DAY};
use crate::policy::{
    cctld_country, AccountRules, AccountState, LegalTable, SuppressionReason, Suppressions,
};
use crate::rng::Rng;
use crate::scoring::{features, LogisticModel};
use crate::signals::{mediation_features, MediationModel, Platform, SizeBand};
use crate::store::{implied_suppression, Event, Snapshot};

/// Engine configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// The sending identities, one per sending domain, in spread order.
    pub senders: Vec<Sender>,
    /// Operating mode.
    pub autonomy: Autonomy,
    /// Use top-two Thompson sampling to identify the best message faster.
    pub experiment: bool,
    /// Randomized holdout.
    pub holdout: Holdout,
    /// Legal table.
    pub legal: LegalTable,
    /// Account coordination.
    pub account: AccountRules,
    /// Message bandit.
    pub bandit: BanditConfig,
    /// Sending-domain health (per domain; caps warm up independently).
    pub health: HealthConfig,
    /// Touches per sequence (hard cap).
    pub max_steps: usize,
    /// Minimum days between touch `k − 1` and touch `k`.
    pub step_gap_days: Vec<i64>,
    /// Cost of one touch (USD): founder time and tooling.
    pub cost_per_touch: f64,
    /// Cost of one spam complaint (USD): reputation.
    pub complaint_cost: f64,
    /// Days after which an unanswered touch counts as "no interest".
    pub settle_days: i64,
    /// Days to settle each funnel transition (interest→audit,
    /// audit→connected, connected→paid).
    pub funnel_window_days: [i64; 3],
    /// Pin the local send hour (0–23) instead of letting the cadence model
    /// choose one. `None` keeps the learned/default hour. Operators use it to
    /// run a pass inside a chosen business hour; the run's one-hour early
    /// window still applies.
    pub send_hour: Option<u32>,
}

impl Config {
    /// Defaults around one sender identity.
    pub fn new(sender: Sender) -> Self {
        Self::with_senders(vec![sender])
    }

    /// Defaults around several sender identities (one per sending domain).
    pub fn with_senders(senders: Vec<Sender>) -> Self {
        Self {
            senders,
            autonomy: Autonomy::Shadow,
            experiment: true,
            holdout: Holdout::default(),
            legal: LegalTable::default(),
            account: AccountRules::default(),
            bandit: BanditConfig::default(),
            health: HealthConfig::default(),
            max_steps: 4,
            step_gap_days: vec![0, 3, 5, 7],
            cost_per_touch: 0.5,
            complaint_cost: 200.0,
            settle_days: 14,
            funnel_window_days: [30, 30, 45],
            send_hour: None,
        }
    }

    /// The identity that sends from `domain`.
    pub fn sender_for(&self, domain: &str) -> Option<&Sender> {
        self.senders.iter().find(|s| s.domain() == domain)
    }

    /// The first configured identity: the one single-domain deployments use
    /// and the one history without a recorded domain belongs to.
    pub fn primary_sender(&self) -> Option<&Sender> {
        self.senders.first()
    }
}

/// One sent touch, as the learner sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchRecord {
    /// When.
    pub at: UnixSecs,
    /// Step.
    pub step: usize,
    /// Arm id.
    pub arm: String,
    /// Local hour.
    pub local_hour: u32,
    /// Interest probability predicted at send time.
    pub predicted: Option<f64>,
}

/// What happened with one contact.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Progress {
    /// Touches in time order.
    pub touches: Vec<TouchRecord>,
    /// Latest classified human reply.
    pub reply: Option<(ReplyClass, UnixSecs)>,
    /// First interested reply.
    pub interested_at: Option<UnixSecs>,
    /// First complaint.
    pub complained_at: Option<UnixSecs>,
}

/// When a publisher reached each funnel stage.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stages {
    /// First interested reply.
    pub interested: Option<UnixSecs>,
    /// Audit requested.
    pub audit: Option<UnixSecs>,
    /// Platform connected.
    pub connected: Option<UnixSecs>,
    /// Paid.
    pub paid: Option<UnixSecs>,
}

impl Stages {
    /// Furthest stage reached.
    pub fn reached(&self) -> Stage {
        if self.paid.is_some() {
            Stage::Paid
        } else if self.connected.is_some() {
            Stage::Connected
        } else if self.audit.is_some() {
            Stage::AuditRequested
        } else if self.interested.is_some() {
            Stage::Interested
        } else {
            Stage::Contacted
        }
    }
}

/// Everything learned from history.
#[derive(Debug, Clone, PartialEq)]
pub struct Learned {
    /// Lead score.
    pub scoring: LogisticModel,
    /// Funnel after interest.
    pub funnel: FunnelModel,
    /// Message arms by template id.
    pub arms: BTreeMap<String, Arm>,
    /// Send-time model.
    pub send_time: SendTimeModel,
    /// Per-step hazards.
    pub steps: StepModel,
    /// Sending-domain health, one entry per domain keyed by from-domain.
    pub health: HealthBook,
    /// Mediation inference.
    pub mediation: MediationModel,
    /// Suppressions.
    pub suppressions: Suppressions,
    /// Account states by publisher.
    pub accounts: BTreeMap<String, AccountState>,
    /// Contact progress by email.
    pub progress: BTreeMap<String, Progress>,
    /// Funnel stages by publisher.
    pub stages: BTreeMap<String, Stages>,
    /// Settled scoring observations used.
    pub scoring_observations: usize,
}

fn band_of(p: Option<&Publisher>) -> SizeBand {
    if p.is_some_and(Publisher::in_owner_queue) {
        return SizeBand::Large;
    }
    SizeBand::of(p.and_then(Publisher::max_rating_count))
}

/// Offset in minutes for a contact: publisher country, else email ccTLD,
/// else UTC.
pub fn offset_for(publisher: &Publisher, contact: &Contact) -> i32 {
    publisher
        .country
        .clone()
        .or_else(|| cctld_country(contact.email.rsplit('@').next().unwrap_or_default()))
        .and_then(|c| country_offset_minutes(&c))
        .unwrap_or(0)
}

// The replay seam: one visit per (domain, day) and one indexed reset-set
// probe per set consulted. A regression to the old per-domain-day reset scan
// makes the reset probe count grow with the reset count and fail the
// scaling test.
#[cfg(test)]
thread_local! {
    static REPLAY_DAY_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static REPLAY_RESET_PROBES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_replay_day() {
    REPLAY_DAY_VISITS.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn note_reset_probes(probes: u64) {
    REPLAY_RESET_PROBES.with(|count| count.set(count.get() + probes));
}

#[cfg(test)]
fn reset_replay_counters() {
    REPLAY_DAY_VISITS.with(|count| count.set(0));
    REPLAY_RESET_PROBES.with(|count| count.set(0));
}

#[cfg(test)]
fn replay_day_visits() -> u64 {
    REPLAY_DAY_VISITS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn replay_reset_probes() -> u64 {
    REPLAY_RESET_PROBES.with(std::cell::Cell::get)
}

/// Raw per-domain daily counts and the operator resets, already grouped by
/// day so the replay applies them in one ordered pass instead of scanning the
/// whole reset list at every (domain, day) pair.
struct HealthDays<'a> {
    sends: &'a BTreeMap<String, BTreeMap<i64, u32>>,
    complaints: &'a BTreeMap<String, BTreeMap<i64, u32>>,
    bounces: &'a BTreeMap<String, BTreeMap<i64, u32>>,
    global_resets: &'a BTreeSet<i64>,
    domain_resets: &'a BTreeMap<String, BTreeSet<i64>>,
    first_day: i64,
    today: i64,
}

/// Replay the health of every configured domain over `first_day..=today`.
///
/// The domains are independent, so each is walked once with its own running
/// [`HealthBook`] entry; for a day, the named and global resets are one
/// bounded set lookup each. The order of operations inside a day is exactly
/// the original fold: reset, record the day's raw counts, then close the day
/// (or record today's send count).
fn replay_health(config: &HealthConfig, domains: &[String], days: &HealthDays<'_>) -> HealthBook {
    let mut health = HealthBook::new(config, domains);
    let ordered = health.domains().to_vec();
    for domain in ordered {
        let own_resets = days.domain_resets.get(&domain);
        for day in days.first_day..=days.today {
            #[cfg(test)]
            note_replay_day();
            #[cfg(test)]
            note_reset_probes(if own_resets.is_some() { 2 } else { 1 });
            if days.global_resets.contains(&day)
                || own_resets.is_some_and(|resets| resets.contains(&day))
            {
                health.reset(Some(&domain), config);
            }
            let n = days
                .sends
                .get(&domain)
                .and_then(|m| m.get(&day))
                .copied()
                .unwrap_or(0);
            let c = days
                .complaints
                .get(&domain)
                .and_then(|m| m.get(&day))
                .copied()
                .unwrap_or(0);
            let b = days
                .bounces
                .get(&domain)
                .and_then(|m| m.get(&day))
                .copied()
                .unwrap_or(0);
            health.record_day(&domain, n, c, b, config);
            if day < days.today {
                if let Some(h) = health.health_mut(&domain) {
                    h.end_day(day, n, config);
                }
            } else {
                health.set_sent_today(&domain, n);
            }
        }
    }
    health
}

/// Rebuild every model from `snap` as of `now`.
pub fn learn(snap: &Snapshot, config: &Config, now: UnixSecs) -> Learned {
    let mut mediation = MediationModel::default();
    for p in snap.publishers.values() {
        if let (Some(platform), Some(report)) = (
            p.declared_platforms.iter().find_map(|s| Platform::parse(s)),
            p.ads_txt(),
        ) {
            mediation.observe(mediation_features(&report), platform);
        }
    }

    let mut suppressions = Suppressions::default();
    let mut progress: BTreeMap<String, Progress> = BTreeMap::new();
    let mut stages: BTreeMap<String, Stages> = BTreeMap::new();
    // Health replay, keyed first by from-domain. The first configured sender
    // is the default: history recorded without a domain (single-domain
    // deployments, or events older than domain recording) belongs to it.
    let default_domain = config
        .primary_sender()
        .map(|s| s.domain().to_string())
        .unwrap_or_default();
    let mut day_sends: BTreeMap<String, BTreeMap<i64, u32>> = BTreeMap::new();
    let mut day_complaints: BTreeMap<String, BTreeMap<i64, u32>> = BTreeMap::new();
    let mut day_bounces: BTreeMap<String, BTreeMap<i64, u32>> = BTreeMap::new();
    let mut global_resets: BTreeSet<i64> = BTreeSet::new();
    let mut domain_resets: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    let mut email_domain: BTreeMap<String, String> = BTreeMap::new();
    let mut publisher_domain: BTreeMap<String, String> = BTreeMap::new();
    let publisher_of = |email: &str| snap.contacts.get(email).map(|c| c.publisher_key.clone());

    for e in &snap.events {
        match e {
            Event::Touch {
                at,
                email,
                publisher,
                from,
                step,
                arm,
                local_hour,
                predicted,
                ..
            } => {
                progress
                    .entry(email.clone())
                    .or_default()
                    .touches
                    .push(TouchRecord {
                        at: *at,
                        step: *step,
                        arm: arm.clone(),
                        local_hour: *local_hour,
                        predicted: *predicted,
                    });
                let domain = from.clone().unwrap_or_else(|| default_domain.clone());
                *day_sends
                    .entry(domain.clone())
                    .or_default()
                    .entry(at.div_euclid(DAY))
                    .or_default() += 1;
                email_domain.insert(email.clone(), domain.clone());
                publisher_domain.insert(publisher.clone(), domain);
            }
            Event::Outcome {
                at,
                email,
                publisher,
                outcome,
                ..
            } => {
                let pubkey = publisher
                    .clone()
                    .or_else(|| email.as_deref().and_then(publisher_of));
                // The sending domain of this outcome: the domain of the
                // contact's latest touch, else the publisher's, else the
                // default domain.
                let domain = email
                    .as_deref()
                    .and_then(|em| email_domain.get(em))
                    .cloned()
                    .or_else(|| {
                        pubkey
                            .as_ref()
                            .and_then(|pk| publisher_domain.get(pk))
                            .cloned()
                    })
                    .unwrap_or_else(|| default_domain.clone());
                if let Some(reason) = implied_suppression(*outcome) {
                    if let Some(em) = email {
                        suppressions.suppress_email(em, reason);
                    }
                    if reason == SuppressionReason::Complaint {
                        if let Some(pk) = &pubkey {
                            suppressions.suppress_publisher(pk, reason);
                        }
                    }
                }
                match outcome {
                    Outcome::Complaint => {
                        *day_complaints
                            .entry(domain.clone())
                            .or_default()
                            .entry(at.div_euclid(DAY))
                            .or_default() += 1;
                        if let Some(em) = email {
                            let pr = progress.entry(em.clone()).or_default();
                            pr.complained_at.get_or_insert(*at);
                        }
                    }
                    Outcome::HardBounce => {
                        *day_bounces
                            .entry(domain)
                            .or_default()
                            .entry(at.div_euclid(DAY))
                            .or_default() += 1;
                    }
                    Outcome::Reply(class) => {
                        if let Some(em) = email {
                            let pr = progress.entry(em.clone()).or_default();
                            pr.reply = Some((*class, *at));
                            if class.is_interest() {
                                pr.interested_at.get_or_insert(*at);
                            }
                        }
                        if class.is_interest() {
                            if let Some(pk) = &pubkey {
                                stages
                                    .entry(pk.clone())
                                    .or_default()
                                    .interested
                                    .get_or_insert(*at);
                            }
                        }
                    }
                    Outcome::AuditRequested
                    | Outcome::Connected
                    | Outcome::Paid
                    | Outcome::WatchSignup => {
                        if let Some(pk) = &pubkey {
                            let st = stages.entry(pk.clone()).or_default();
                            // A watch signup is a free monitoring
                            // subscriber: for the ladder it counts on the
                            // connected rung, while the event log keeps the
                            // distinct outcome.
                            let slot = match outcome {
                                Outcome::AuditRequested => &mut st.audit,
                                Outcome::Connected | Outcome::WatchSignup => &mut st.connected,
                                _ => &mut st.paid,
                            };
                            slot.get_or_insert(*at);
                        }
                    }
                    _ => {}
                }
            }
            Event::Suppress {
                email, publisher, ..
            } => {
                if let Some(em) = email {
                    suppressions.suppress_email(em, SuppressionReason::Operator);
                }
                if let Some(pk) = publisher {
                    suppressions.suppress_publisher(pk, SuppressionReason::Operator);
                }
            }
            Event::HealthReset { at, domain } => {
                let day = at.div_euclid(DAY);
                match domain {
                    Some(domain) => {
                        domain_resets.entry(domain.clone()).or_default().insert(day);
                    }
                    None => {
                        global_resets.insert(day);
                    }
                }
            }
            // The outbox saga's intent marker is not settled history: the
            // models learn from the Touch (or nothing, while the send is
            // unresolved). A marker left pending without its Touch is
            // applied by the run loop, which suppresses the resend.
            Event::TouchPlanned { .. } => {}
        }
    }

    // Sending-domain health, replayed once per domain with its own running
    // state. Days and resets are applied in one ordered pass: a reset is a
    // set-membership test, so daily resets never turn the replay into a
    // domain-day-by-reset scan. One domain's complaints never move another
    // domain's state.
    let today = now.div_euclid(DAY);
    let domains: Vec<String> = config
        .senders
        .iter()
        .map(|s| s.domain().to_string())
        .collect();
    let first_day = day_sends
        .values()
        .flat_map(|m| m.keys())
        .chain(day_complaints.values().flat_map(|m| m.keys()))
        .chain(day_bounces.values().flat_map(|m| m.keys()))
        .copied()
        .min()
        .unwrap_or(today);
    let health = replay_health(
        &config.health,
        &domains,
        &HealthDays {
            sends: &day_sends,
            complaints: &day_complaints,
            bounces: &day_bounces,
            global_resets: &global_resets,
            domain_resets: &domain_resets,
            first_day,
            today,
        },
    );

    // Funnel transitions, settled only.
    let mut funnel = FunnelModel::default();
    for (pk, st) in &stages {
        let band = band_of(snap.publishers.get(pk));
        let chain = [
            (st.interested, st.audit),
            (st.audit, st.connected),
            (st.connected, st.paid),
        ];
        for (t, (from, to)) in chain.iter().enumerate() {
            if let Some(start) = from {
                if to.is_some() {
                    funnel.observe(band, t, true);
                } else if now - start > config.funnel_window_days[t] * DAY {
                    funnel.observe(band, t, false);
                }
            }
        }
    }

    // Touch-level learning.
    let mut steps = StepModel::with_steps(config.max_steps);
    let mut send_time = SendTimeModel::default();
    let first_event_day = snap
        .events
        .first()
        .map_or(today, |e| e.at().div_euclid(DAY));
    let mut arms: BTreeMap<String, Arm> = TEMPLATES
        .iter()
        .map(|t| {
            (
                t.id.to_string(),
                Arm::new(t.id, &config.bandit, first_event_day),
            )
        })
        .collect();
    let mut scoring_batch = Vec::new();
    let mut settled_touches: Vec<(UnixSecs, String, f64, bool)> = Vec::new();
    for (email, pr) in &progress {
        let Some(contact) = snap.contacts.get(email) else {
            continue;
        };
        let publisher = snap.publishers.get(&contact.publisher_key);
        let band = band_of(publisher);
        let reward_if_interest = stages
            .get(&contact.publisher_key)
            .map_or(Stage::Interested, Stages::reached)
            .max(Stage::Interested);
        let reward = funnel.p_paid_given(band, reward_if_interest);
        for (i, t) in pr.touches.iter().enumerate() {
            let next = pr.touches.get(i + 1).map(|n| n.at);
            let caused =
                |x: Option<UnixSecs>| x.is_some_and(|x| x >= t.at && next.is_none_or(|n| x < n));
            let interest = caused(pr.interested_at);
            let complaint = caused(pr.complained_at);
            let settled =
                interest || complaint || next.is_some() || now - t.at > config.settle_days * DAY;
            if !settled {
                continue;
            }
            steps.observe(t.step, interest, complaint);
            send_time.observe(t.local_hour, interest);
            settled_touches.push((
                t.at,
                t.arm.clone(),
                if interest { reward } else { 0.0 },
                complaint,
            ));
        }
        let last = pr.touches.last().map_or(0, |t| t.at);
        let y = pr.interested_at.is_some();
        if !pr.touches.is_empty() && (y || now - last > config.settle_days * DAY) {
            if let Some(p) = publisher {
                scoring_batch.push((features(p, contact, &mediation).x, y));
            }
        }
    }
    settled_touches.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    for (at, arm, reward, complaint) in settled_touches {
        if let Some(a) = arms.get_mut(&arm) {
            a.record(reward, complaint, at.div_euclid(DAY), &config.bandit);
        }
    }
    let mut scoring = LogisticModel::default();
    scoring.update(&scoring_batch);

    // Account states.
    let mut accounts: BTreeMap<String, AccountState> = BTreeMap::new();
    for (email, pr) in &progress {
        let Some(contact) = snap.contacts.get(email) else {
            continue;
        };
        let acc = accounts.entry(contact.publisher_key.clone()).or_default();
        if let Some((class, at)) = pr.reply {
            if acc.last_reply.is_none_or(|(_, t)| t <= at) {
                acc.last_reply = Some((class, at));
            }
        }
        let in_sequence = pr.reply.is_none()
            && suppressions.check(contact).is_none()
            && !pr.touches.is_empty()
            && pr.touches.len() < config.max_steps
            && pr
                .touches
                .last()
                .is_some_and(|t| now - t.at <= config.settle_days * DAY);
        if in_sequence {
            acc.active_contacts.push(email.clone());
        }
    }

    Learned {
        scoring_observations: scoring_batch.len(),
        scoring,
        funnel,
        arms,
        send_time,
        steps,
        health,
        mediation,
        suppressions,
        accounts,
        progress,
        stages,
    }
}

/// One planned (or refused) touch.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanItem {
    /// Deterministic id: contact, step and day.
    pub touch_id: String,
    /// Recipient.
    pub email: String,
    /// Publisher key.
    pub publisher: String,
    /// The from-domain this touch is planned to be sent from, once it
    /// survives the gates and the per-domain caps. `None` on denied or
    /// deferred items.
    pub sender: Option<String>,
    /// Step.
    pub step: usize,
    /// Chosen template id, when one rendered.
    pub arm: Option<String>,
    /// Scheduled send time.
    pub send_at: UnixSecs,
    /// Recipient's local hour at `send_at`.
    pub local_hour: u32,
    /// Expected value (USD) on the posterior draw.
    pub expected_value: f64,
    /// Posterior-mean interest probability over the sequence.
    pub p_interest: f64,
    /// Score reason codes.
    pub score_reasons: Vec<String>,
    /// Gate decision.
    pub decision: Decision,
    /// A3 S2: this publisher is in the owner queue (above $1M/month) and the
    /// owner writes personally after the engine's first email.
    pub owner_queue: bool,
    /// Rendered message, when one rendered.
    pub rendered: Option<Rendered>,
}

/// The touch id for `email` at `step` on `day`.
pub fn touch_id(email: &str, step: usize, day: i64) -> String {
    format!(
        "t{:016x}",
        fnv1a(format!("{email}|{step}|{day}").as_bytes())
    )
}

/// Plan the next touches as of `now`. Items whose gate verdict is not
/// [`Verdict::Deny`] come first, by expected value; the day's touched are
/// then spread across the sending domains, each up to its own cap, in the
/// configured order and round-robin (the spread rule in the module docs).
pub fn plan(snap: &Snapshot, learned: &Learned, config: &Config, now: UnixSecs) -> Vec<PlanItem> {
    let today = now.div_euclid(DAY);
    let hazard_total: f64 = learned
        .steps
        .interest
        .iter()
        .map(|(a, b)| a / (a + b))
        .sum::<f64>()
        .max(1e-12);
    // The gate sees the whole engine: the cap left across every domain that
    // may send, and a summary of the domain states. The per-domain split
    // happens when the ranked items are placed.
    let cap_left = learned.health.cap_left();
    let sender_state = learned.health.summary_state();
    let sender_reason: Option<String> = if config.senders.is_empty() {
        Some("no sending domain is configured".to_string())
    } else if config.senders.len() > 1 && !learned.health.may_send() {
        let states: Vec<String> = learned
            .health
            .domains()
            .iter()
            .filter_map(|domain| {
                learned
                    .health
                    .get(domain)
                    .map(|h| format!("{domain}: {}", h.state.as_str()))
            })
            .collect();
        Some(format!(
            "no sending domain may send ({})",
            states.join("; ")
        ))
    } else {
        None
    };
    let mut items = Vec::new();
    for (email, contact) in &snap.contacts {
        let Some(publisher) = snap.publishers.get(&contact.publisher_key) else {
            continue;
        };
        let empty = Progress::default();
        let pr = learned.progress.get(email).unwrap_or(&empty);
        let k = pr.touches.len();
        let owner_queue = publisher.in_owner_queue();
        if k >= config.max_steps || pr.reply.is_some() {
            continue;
        }
        // A3 S2: the owner takes over after the engine's first email. No
        // automated follow-up is sent from the owner's queue.
        if owner_queue && k > 0 {
            continue;
        }
        if learned
            .stages
            .get(&contact.publisher_key)
            .is_some_and(|s| s.reached() >= Stage::Interested)
        {
            continue;
        }
        if let Some(last) = pr.touches.last() {
            let gap = config.step_gap_days.get(k).copied().unwrap_or(7);
            if now < last.at + gap * DAY {
                continue;
            }
        }
        let id = touch_id(email, k, today);
        let mut rng = Rng::seeded(fnv1a(id.as_bytes()));
        let fv = features(publisher, contact, &learned.mediation);
        let p_mean = learned.scoring.predict(&fv.x);
        let p_draw = learned.scoring.sample(&fv.x, &mut rng);
        let band = band_of(Some(publisher));
        let v_interest = learned.funnel.value_of_interest(band);
        let (ha, hb) = learned.steps.interest.get(k).copied().unwrap_or((1.0, 1.0));
        let share = ha / (ha + hb) / hazard_total;
        let (ca, cb) = learned
            .steps
            .complaint
            .get(k)
            .copied()
            .unwrap_or((1.0, 1999.0));
        let ev = p_draw * share * v_interest
            - config.cost_per_touch
            - ca / (ca + cb) * config.complaint_cost;

        // Message variant: bandit choice among this step's templates, then the
        // others by posterior mean if the choice cannot be grounded.
        let candidates: Vec<&crate::compose::Template> = available_templates_for_step(k)
            .into_iter()
            .filter(|t| !owner_queue || t.id != "t0-watch")
            .collect();
        let arm_states: Vec<Arm> = candidates
            .iter()
            .filter_map(|t| learned.arms.get(t.id).cloned())
            .collect();
        let picked = choose(&arm_states, &config.bandit, config.experiment, &mut rng);
        let mut order: Vec<usize> = (0..arm_states.len()).collect();
        order.sort_by(|&a, &b| {
            arm_states[b]
                .mean_value()
                .total_cmp(&arm_states[a].mean_value())
        });
        if let Some((i, _)) = picked {
            order.retain(|&j| j != i);
            order.insert(0, i);
        }
        let mut chosen: Option<(String, Rendered)> = None;
        if picked.is_some() {
            if let Some(feasibility_sender) = config.primary_sender() {
                for i in order {
                    let Some(t) = candidates.iter().find(|t| t.id == arm_states[i].id) else {
                        continue;
                    };
                    if !arm_states[i].is_safe(&config.bandit) {
                        continue;
                    }
                    if let Ok(r) = render(t, publisher, contact, feasibility_sender) {
                        chosen = Some((t.id.to_string(), r));
                        break;
                    }
                }
            }
        }

        let acc = learned
            .accounts
            .get(&contact.publisher_key)
            .cloned()
            .unwrap_or_default();
        let input = GateInput {
            autonomy: config.autonomy,
            suppression: learned.suppressions.check(contact),
            legal: config.legal.evaluate(publisher.country.as_deref(), contact),
            account: config.account.evaluate(&acc, email, now),
            held_out: config.holdout.is_held_out(&contact.publisher_key),
            sender: sender_state,
            sender_reason: sender_reason.clone(),
            cap_left,
            grounded: chosen.is_some(),
            variant_available: picked.is_some(),
            expected_value: ev,
        };
        let decision = decide(&input);
        let offset = offset_for(publisher, contact);
        let hour = config
            .send_hour
            .unwrap_or_else(|| learned.send_time.choose_hour(&mut rng));
        let send_at = next_weekday_hour(now, offset, hour);
        let local_hour = u32::try_from(local_parts(send_at, offset).1).unwrap_or(hour);
        let (arm, rendered) = match chosen {
            Some((a, r)) => (Some(a), Some(r)),
            None => (None, None),
        };
        items.push(PlanItem {
            touch_id: id,
            email: email.clone(),
            publisher: contact.publisher_key.clone(),
            sender: None,
            step: k,
            arm,
            send_at,
            local_hour,
            expected_value: ev,
            p_interest: p_mean,
            decision,
            owner_queue,
            score_reasons: {
                let mut reasons: Vec<String> = learned
                    .scoring
                    .reasons(&fv)
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                if owner_queue {
                    reasons.push(
                        "owner queue: above $1M/month; the owner writes after the first email"
                            .into(),
                    );
                }
                reasons
            },
            rendered,
        });
    }
    // Rank, then spread the day's touches across the sending domains: the
    // survivors keep their rank order and go round-robin over the domains in
    // configured order, each up to its own remaining cap. A domain that
    // cannot send (paused or quarantined) or whose cap is spent is skipped,
    // so its share moves to the others. The engine's global daily cap is the
    // sum of the per-domain caps. A publisher gets at most one touch per
    // plan.
    items.sort_by(|a, b| {
        let live = |i: &PlanItem| u8::from(i.decision.verdict != Verdict::Deny);
        live(b)
            .cmp(&live(a))
            .then(b.owner_queue.cmp(&a.owner_queue))
            .then(b.expected_value.total_cmp(&a.expected_value))
            .then(a.touch_id.cmp(&b.touch_id))
    });
    let domain_order: Vec<String> = config
        .senders
        .iter()
        .map(|s| s.domain().to_string())
        .collect();
    let mut remaining: Vec<u32> = domain_order
        .iter()
        .map(|domain| {
            learned.health.get(domain).map_or(0, |h| {
                if h.state.may_send() {
                    h.cap.saturating_sub(learned.health.sent_today_for(domain))
                } else {
                    0
                }
            })
        })
        .collect();
    let mut cursor = 0usize;
    // One touch per publisher per plan: membership is constant-time while the
    // ranked list is walked in its (observable) order.
    let mut publishers_used: HashSet<String> = HashSet::new();
    for item in items.iter_mut() {
        if item.decision.verdict == Verdict::Deny
            || matches!(item.decision.verdict, Verdict::Defer(_))
        {
            continue;
        }
        if publishers_used.contains(&item.publisher) {
            item.decision.verdict = Verdict::Deny;
            item.decision
                .reasons
                .push("another contact at this publisher is planned".into());
            continue;
        }
        let pick = if domain_order.is_empty() {
            None
        } else {
            (0..domain_order.len())
                .map(|offset| (cursor + offset) % domain_order.len())
                .find(|&i| remaining[i] > 0)
        };
        match pick {
            Some(i) => {
                remaining[i] -= 1;
                cursor = (i + 1) % domain_order.len();
                publishers_used.insert(item.publisher.clone());
                item.sender = Some(domain_order[i].clone());
            }
            None => {
                item.decision.verdict = Verdict::Deny;
                item.decision.reasons.push(if domain_order.is_empty() {
                    "no sending domain is configured".into()
                } else {
                    "ranked below today's cap".into()
                });
            }
        }
    }

    // Render each planned touch with the identity of the domain that will
    // send it. With one sender this is byte-for-byte the same message the
    // feasibility render above produced.
    for item in items.iter_mut() {
        let (Some(domain), Some(arm_id)) = (item.sender.as_deref(), item.arm.as_deref()) else {
            continue;
        };
        let rendered = config.sender_for(domain).and_then(|sender| {
            let template = TEMPLATES.iter().find(|t| t.id == arm_id)?;
            let contact = snap.contacts.get(&item.email)?;
            let publisher = snap.publishers.get(&item.publisher)?;
            render(template, publisher, contact, sender).ok()
        });
        match rendered {
            Some(rendered) => item.rendered = Some(rendered),
            None => {
                item.sender = None;
                item.rendered = None;
                item.decision.verdict = Verdict::Deny;
                item.decision
                    .reasons
                    .push("the message could not be rendered for the sending domain".into());
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{App, ContactKind, Evidence, EvidenceKind, Store};

    fn sender() -> Sender {
        Sender {
            name: "Aaron".into(),
            email: "aaron@apexmediation.ee".into(),
            company: "Apex".into(),
            postal_address: "Tallinn".into(),
            unsubscribe_url: None,
        }
    }

    fn publisher(key: &str, ratings: u64) -> Publisher {
        let mut p = Publisher::new(key, key);
        p.domain = Some(key.into());
        p.country = Some("FI".into());
        p.apps.push(App {
            store: Store::AppStore,
            id: "1".into(),
            name: format!("{key} game"),
            genre: Some("Games".into()),
            rating_count: Some(ratings),
            free: Some(true),
            days_since_update: Some(5),
        });
        p.evidence.push(Evidence {
            id: "ev".into(),
            kind: EvidenceKind::AppAdsTxt,
            source: "s".into(),
            statement: format!("https://{key}/app-ads.txt was not found"),
            observed_at: 0,
        });
        p.ads_txt_missing = true;
        p
    }

    fn contact(email: &str, key: &str) -> Contact {
        Contact {
            email: email.into(),
            name: None,
            role: None,
            publisher_key: key.into(),
            kind: ContactKind::of_email(email),
            source: "t".into(),
            consent: false,
        }
    }

    fn snap(n: usize) -> Snapshot {
        let mut s = Snapshot::default();
        for i in 0..n {
            let key = format!("studio{i}.io");
            s.publishers
                .insert(key.clone(), publisher(&key, 10_000 + i as u64));
            let email = format!("ceo@studio{i}.io");
            s.contacts.insert(email.clone(), contact(&email, &key));
        }
        s
    }

    fn config() -> Config {
        let mut c = Config::new(sender());
        c.autonomy = Autonomy::Autonomous;
        c.holdout.share = 0.0;
        c
    }

    fn two_sender_config() -> Config {
        let mut c = Config::with_senders(vec![
            sender(),
            Sender {
                name: "Dana".into(),
                email: "dana@mail2.example".into(),
                company: "Apex".into(),
                postal_address: "Tallinn".into(),
                unsubscribe_url: None,
            },
        ]);
        c.autonomy = Autonomy::Autonomous;
        c.holdout.share = 0.0;
        c
    }

    const NOW: UnixSecs = 20_742 * DAY + 8 * 3600; // Fri 16 Oct 2026 08:00 UTC

    #[test]
    fn the_owner_queue_outranks_core_when_capacity_binds() {
        let mut s = snap(30);
        let key = "studio0.io".to_string();
        s.publishers
            .get_mut(&key)
            .expect("publisher")
            .monthly_revenue_micros = Some(apex_sales_model_owner_threshold() + 1);
        let mut c = config();
        // Only one send fits: the owner must take it.
        c.health.start_cap = 1;
        let l = learn(&s, &c, NOW);
        let items = plan(&s, &l, &c, NOW);
        let send: Vec<&PlanItem> = items
            .iter()
            .filter(|i| i.decision.verdict == Verdict::Send)
            .collect();
        assert_eq!(send.len(), 1, "{send:#?}");
        assert_eq!(send[0].publisher, key);
        assert!(send[0].owner_queue);
        assert!(send[0]
            .score_reasons
            .iter()
            .any(|r| r.contains("owner queue")));
        assert_ne!(send[0].arm.as_deref(), Some("t0-watch"));
        // The owner cadence has one automated touch: no follow-up item is
        // planned for the owner at a later step.
        assert!(items
            .iter()
            .filter(|i| i.publisher == key)
            .all(|i| i.step == 0));
    }

    fn apex_sales_model_owner_threshold() -> i64 {
        crate::model::OWNER_QUEUE_MIN_MONTHLY_REVENUE_MICROS
    }

    #[test]
    fn a_fresh_state_plans_first_touches_within_the_cap() {
        let s = snap(40);
        let c = config();
        let l = learn(&s, &c, NOW);
        assert_eq!(l.health.total_cap(), c.health.start_cap);
        let items = plan(&s, &l, &c, NOW);
        assert_eq!(items.len(), 40);
        let send: Vec<&PlanItem> = items
            .iter()
            .filter(|i| i.decision.verdict == Verdict::Send)
            .collect();
        assert_eq!(send.len(), 20, "cap");
        for i in &send {
            assert_eq!(i.step, 0);
            assert!(i.arm.as_deref().is_some_and(|a| a.starts_with("t0-")));
            assert!(i.expected_value > 0.0);
            assert!(i.send_at >= NOW);
            assert!(i
                .rendered
                .as_ref()
                .is_some_and(|r| r.cites == vec!["ev".to_string()]));
            assert!(i.score_reasons.iter().any(|r| r.contains("bias")));
            assert_eq!(i.sender.as_deref(), Some("apexmediation.ee"));
        }
        let ranked_out = items
            .iter()
            .filter(|i| {
                i.decision
                    .reasons
                    .iter()
                    .any(|r| r == "ranked below today's cap")
            })
            .count();
        assert_eq!(ranked_out, 20);
        // Plans are stable within a day.
        assert_eq!(plan(&s, &l, &c, NOW + 3600)[0].touch_id, items[0].touch_id);
    }

    #[test]
    fn a_pinned_send_hour_replaces_the_cadence_choice() {
        let s = snap(2);
        let mut c = config();
        c.send_hour = Some(14);
        let l = learn(&s, &c, NOW);
        let items = plan(&s, &l, &c, NOW);
        assert!(!items.is_empty());
        for item in &items {
            // FI is +120 in the offset table; 14:00 local is 12:00 UTC.
            assert_eq!(
                crate::calendar::local_parts(item.send_at, 120).1,
                14,
                "the pinned local hour is used"
            );
        }
    }

    #[test]
    fn a_single_domain_renders_and_records_exactly_as_before() {
        let s = snap(5);
        let c = config();
        let l = learn(&s, &c, NOW);
        let items = plan(&s, &l, &c, NOW);
        for item in items.iter().filter(|i| i.decision.verdict == Verdict::Send) {
            assert_eq!(item.sender.as_deref(), Some("apexmediation.ee"));
            let publisher = s.publishers.get(&item.publisher).expect("publisher");
            let contact = s.contacts.get(&item.email).expect("contact");
            let template = TEMPLATES
                .iter()
                .find(|t| Some(t.id) == item.arm.as_deref())
                .expect("template");
            let direct = render(
                template,
                publisher,
                contact,
                c.primary_sender().expect("sender"),
            )
            .expect("render");
            assert_eq!(item.rendered.as_ref(), Some(&direct));
        }
        // Single-domain touches carry no `from` field, so events.jsonl stays
        // byte-for-byte as it was.
        let touch = Event::Touch {
            at: 1,
            email: "a@b.io".into(),
            publisher: "b.io".into(),
            from: None,
            step: 0,
            arm: "t0-finding".into(),
            message_id: "m".into(),
            local_hour: 9,
            predicted: None,
        };
        assert!(!touch.to_json().to_json().contains("\"from\""));
    }

    #[test]
    fn two_domains_split_a_days_plan_deterministically() {
        let s = snap(40);
        let c = two_sender_config();
        let l = learn(&s, &c, NOW);
        assert_eq!(l.health.total_cap(), c.health.start_cap * 2);
        let items = plan(&s, &l, &c, NOW);
        let send: Vec<&PlanItem> = items
            .iter()
            .filter(|i| i.decision.verdict == Verdict::Send)
            .collect();
        assert_eq!(send.len(), 40, "sum-of-domains cap");
        let first = send
            .iter()
            .filter(|i| i.sender.as_deref() == Some("apexmediation.ee"))
            .count();
        let second = send
            .iter()
            .filter(|i| i.sender.as_deref() == Some("mail2.example"))
            .count();
        assert_eq!((first, second), (20, 20));
        // Documented spread rule: round-robin over the rank order, in the
        // configured sender order.
        for pair in send.chunks(2) {
            assert_eq!(pair[0].sender.as_deref(), Some("apexmediation.ee"));
            assert_eq!(pair[1].sender.as_deref(), Some("mail2.example"));
        }
        // The plan is the same on the next hourly run of the same day.
        let again = plan(&s, &l, &c, NOW + 3600);
        assert_eq!(
            again
                .iter()
                .map(|i| (i.touch_id.clone(), i.sender.clone()))
                .collect::<Vec<_>>(),
            items
                .iter()
                .map(|i| (i.touch_id.clone(), i.sender.clone()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_paused_domains_share_moves_to_the_others() {
        let mut s = snap(30);
        let c = two_sender_config();
        // Domain A sent 100 and drew 20 hard bounces two days ago: its
        // breaker fires and it rests; B has no history at all.
        let past = NOW - 2 * DAY;
        for i in 0..100 {
            s.events.push(Event::Touch {
                at: past + i,
                email: format!("hist{i}@x.example"),
                publisher: "hist.example".into(),
                from: Some("apexmediation.ee".into()),
                step: 0,
                arm: "t0-finding".into(),
                message_id: format!("h{i}"),
                local_hour: 9,
                predicted: None,
            });
        }
        for i in 0..20 {
            s.events.push(Event::Outcome {
                at: past + 200 + i,
                email: Some(format!("hist{i}@x.example")),
                publisher: None,
                outcome: Outcome::HardBounce,
                note: None,
            });
        }
        s.events.sort_by_key(Event::at);
        let l = learn(&s, &c, NOW);
        assert_eq!(
            l.health.get("apexmediation.ee").map(|h| h.state),
            Some(crate::health::SenderState::Paused)
        );
        assert_eq!(
            l.health.get("mail2.example").map(|h| h.state),
            Some(crate::health::SenderState::Warming)
        );
        let items = plan(&s, &l, &c, NOW);
        let send: Vec<&PlanItem> = items
            .iter()
            .filter(|i| i.decision.verdict == Verdict::Send)
            .collect();
        assert!(!send.is_empty());
        assert!(send
            .iter()
            .all(|i| i.sender.as_deref() == Some("mail2.example")));
        assert_eq!(
            send.len(),
            c.health.start_cap as usize,
            "only B's cap bounds the day"
        );
    }

    #[test]
    fn complaints_quarantine_only_the_domain_that_sent() {
        let mut s = snap(10);
        let c = two_sender_config();
        let past = NOW - DAY;
        for i in 0..40 {
            s.events.push(Event::Touch {
                at: past + i,
                email: format!("c{i}@burst.example"),
                publisher: "burst.example".into(),
                from: Some("apexmediation.ee".into()),
                step: 0,
                arm: "t0-finding".into(),
                message_id: format!("m{i}"),
                local_hour: 9,
                predicted: None,
            });
        }
        for i in 0..5 {
            s.events.push(Event::Outcome {
                at: past + 100 + i,
                email: Some(format!("c{i}@burst.example")),
                publisher: None,
                outcome: Outcome::Complaint,
                note: None,
            });
        }
        s.events.sort_by_key(Event::at);
        let l = learn(&s, &c, NOW);
        assert_eq!(
            l.health.get("apexmediation.ee").map(|h| h.state),
            Some(crate::health::SenderState::Quarantined)
        );
        assert_eq!(
            l.health.get("mail2.example").map(|h| h.state),
            Some(crate::health::SenderState::Warming)
        );
        assert_eq!(
            l.health.get("mail2.example").map(|h| h.cap),
            Some(c.health.start_cap)
        );
        // The complaints also suppress the publisher (they were sent to it),
        // so plan over publishers without that history.
        let items = plan(&s, &l, &c, NOW);
        let send: Vec<&PlanItem> = items
            .iter()
            .filter(|i| i.decision.verdict == Verdict::Send)
            .collect();
        assert!(!send.is_empty());
        assert!(send
            .iter()
            .all(|i| i.sender.as_deref() == Some("mail2.example")));
    }

    #[test]
    fn the_free_watch_offer_arm_serves_without_a_checkout() {
        let s = snap(10);
        let c = two_sender_config();
        let mut l = learn(&s, &c, NOW);
        assert!(l.arms.contains_key("t0-watch"));
        if let Some(arm) = l.arms.get_mut("t0-watch") {
            arm.value = (500.0, 1.0);
        }
        let items = plan(&s, &l, &c, NOW);
        let watch: Vec<&PlanItem> = items
            .iter()
            .filter(|i| i.arm.as_deref() == Some("t0-watch"))
            .collect();
        assert!(!watch.is_empty(), "{items:#?}");
        let item = watch[0];
        let rendered = item.rendered.as_ref().expect("rendered watch");
        assert!(rendered.body.contains("free"));
        assert!(rendered.body.contains("30%"));
        assert!(!rendered.body.contains("/watch/?ref="));
    }

    #[test]
    fn history_drives_suppression_sequencing_and_learning() {
        let mut s = snap(3);
        s.contacts.insert(
            "cto@studio0.io".into(),
            contact("cto@studio0.io", "studio0.io"),
        );
        let c = config();
        let t0 = NOW - 20 * DAY;
        for (i, email) in ["ceo@studio0.io", "ceo@studio1.io", "ceo@studio2.io"]
            .iter()
            .enumerate()
        {
            s.events.push(Event::Touch {
                at: t0 + i as i64,
                email: (*email).into(),
                publisher: format!("studio{i}.io"),
                from: None,
                step: 0,
                arm: "t0-finding".into(),
                message_id: format!("m{i}"),
                local_hour: 10,
                predicted: None,
            });
        }
        s.events.push(Event::Outcome {
            at: t0 + DAY,
            email: Some("ceo@studio1.io".into()),
            publisher: None,
            outcome: Outcome::Reply(ReplyClass::Positive),
            note: None,
        });
        s.events.push(Event::Outcome {
            at: t0 + 2 * DAY,
            email: None,
            publisher: Some("studio1.io".into()),
            outcome: Outcome::AuditRequested,
            note: None,
        });
        s.events.push(Event::Outcome {
            at: t0 + DAY,
            email: Some("ceo@studio2.io".into()),
            publisher: None,
            outcome: Outcome::Complaint,
            note: None,
        });
        s.events.sort_by_key(Event::at);
        let l = learn(&s, &c, NOW);
        assert_eq!(l.scoring_observations, 3);
        assert_eq!(l.stages["studio1.io"].reached(), Stage::AuditRequested);
        assert!(l
            .suppressions
            .check(&contact("x@studio2.io", "studio2.io"))
            .is_some());
        assert!(l.arms["t0-finding"].sends > 2.5);
        let items = plan(&s, &l, &c, NOW);
        // studio1 replied (a person takes over) and studio2 complained (the
        // publisher is suppressed), so only studio0 gets a touch: one
        // contact, chosen by expected value; the other is held back.
        let live: Vec<&PlanItem> = items
            .iter()
            .filter(|i| i.decision.verdict == Verdict::Send)
            .collect();
        assert_eq!(live.len(), 1, "{items:#?}");
        assert_eq!(live[0].publisher, "studio0.io");
        assert!(items.iter().any(|i| i.publisher == "studio0.io"
            && i.decision
                .reasons
                .iter()
                .any(|r| r == "another contact at this publisher is planned")));
        assert!(items
            .iter()
            .any(|i| i.email == "ceo@studio0.io" && i.step == 1));
        assert!(items
            .iter()
            .any(|i| i.email.ends_with("studio2.io") && i.decision.verdict == Verdict::Deny));
        assert!(!items.iter().any(|i| i.email == "ceo@studio1.io"));
    }

    #[test]
    fn gaps_steps_and_modes_are_respected() {
        let mut s = snap(1);
        let mut c = config();
        s.events.push(Event::Touch {
            at: NOW - DAY,
            email: "ceo@studio0.io".into(),
            publisher: "studio0.io".into(),
            from: None,
            step: 0,
            arm: "t0-short".into(),
            message_id: "m".into(),
            local_hour: 9,
            predicted: None,
        });
        let l = learn(&s, &c, NOW);
        assert_eq!(l.health.sent_today(), 0);
        assert!(plan(&s, &l, &c, NOW).is_empty(), "3-day gap not reached");
        let later = NOW + 3 * DAY;
        let l = learn(&s, &c, later);
        let items = plan(&s, &l, &c, later);
        assert_eq!(items[0].step, 1);
        c.autonomy = Autonomy::Shadow;
        assert_eq!(plan(&s, &l, &c, later)[0].decision.verdict, Verdict::Shadow);
        c.autonomy = Autonomy::Off;
        assert_eq!(plan(&s, &l, &c, later)[0].decision.verdict, Verdict::Deny);
        c.max_steps = 1;
        let l = learn(&s, &c, later);
        assert!(plan(&s, &l, &c, later).is_empty(), "sequence finished");
    }

    #[test]
    fn holdout_and_legal_gates_apply() {
        let mut s = snap(1);
        let mut c = config();
        c.holdout.share = 0.5;
        c.holdout.salt = (0..100)
            .map(|i| format!("salt{i}"))
            .find(|salt| {
                Holdout {
                    salt: salt.clone(),
                    share: 0.5,
                }
                .is_held_out("studio0.io")
            })
            .unwrap_or_default();
        let l = learn(&s, &c, NOW);
        let items = plan(&s, &l, &c, NOW);
        assert!(items[0]
            .decision
            .reasons
            .iter()
            .any(|r| r.contains("holdout")));
        c.holdout.share = 0.0;
        if let Some(p) = s.publishers.get_mut("studio0.io") {
            p.country = Some("DE".into());
        }
        let items = plan(&s, &learn(&s, &c, NOW), &c, NOW);
        assert!(items[0]
            .decision
            .reasons
            .iter()
            .any(|r| r.contains("consent")));
    }

    /// The pre-fix fold, kept verbatim as the equivalence oracle: a reset is
    /// discovered by scanning the whole reset list at every (domain, day).
    fn naive_replay_health(
        config: &HealthConfig,
        domains: &[String],
        days: &HealthDays<'_>,
        resets: &[(Option<String>, i64)],
    ) -> HealthBook {
        let mut health = HealthBook::new(config, domains);
        for day in days.first_day..=days.today {
            for domain in health.domains().to_vec() {
                if resets
                    .iter()
                    .any(|(d, at)| *at == day && d.as_deref().is_none_or(|d| d == domain.as_str()))
                {
                    health.reset(Some(&domain), config);
                }
                let n = days
                    .sends
                    .get(&domain)
                    .and_then(|m| m.get(&day))
                    .copied()
                    .unwrap_or(0);
                let c = days
                    .complaints
                    .get(&domain)
                    .and_then(|m| m.get(&day))
                    .copied()
                    .unwrap_or(0);
                let b = days
                    .bounces
                    .get(&domain)
                    .and_then(|m| m.get(&day))
                    .copied()
                    .unwrap_or(0);
                health.record_day(&domain, n, c, b, config);
                if day < days.today {
                    if let Some(h) = health.health_mut(&domain) {
                        h.end_day(day, n, config);
                    }
                } else {
                    health.set_sent_today(&domain, n);
                }
            }
        }
        health
    }

    fn grouped_resets(
        resets: &[(Option<String>, i64)],
    ) -> (BTreeSet<i64>, BTreeMap<String, BTreeSet<i64>>) {
        let mut global = BTreeSet::new();
        let mut per_domain: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
        for (domain, day) in resets {
            match domain {
                Some(domain) => {
                    per_domain.entry(domain.clone()).or_default().insert(*day);
                }
                None => {
                    global.insert(*day);
                }
            }
        }
        (global, per_domain)
    }

    #[test]
    fn single_pass_health_replay_matches_the_day_by_day_fold_on_ten_thousand_days() {
        const DAYS: i64 = 10_000;
        let domains: Vec<String> = ["a.example", "b.example", "c.example"]
            .iter()
            .map(|domain| domain.to_string())
            .collect();
        let base = HealthConfig::default();
        let mut rng = Rng::seeded(0xA12026);
        let mut sends: BTreeMap<String, BTreeMap<i64, u32>> = BTreeMap::new();
        let mut complaints: BTreeMap<String, BTreeMap<i64, u32>> = BTreeMap::new();
        let mut bounces: BTreeMap<String, BTreeMap<i64, u32>> = BTreeMap::new();
        let mut resets: Vec<(Option<String>, i64)> = Vec::new();
        for day in 0..DAYS {
            // Daily resets: R ≈ D, the shape that made the old lookup
            // quadratic. The named reset lands on a random domain so every
            // domain also has unreset days to warm, pause and recover; a rare
            // global reset exercises the domain-free event too.
            if day < DAYS - 10 {
                let reset_domain = &domains[(rng.below(domains.len() as u64)) as usize];
                resets.push((Some(reset_domain.clone()), day));
                if day % 997 == 0 {
                    resets.push((None, day));
                }
            }
            for domain in &domains {
                let n = rng.below(26) as u32;
                let mut c = if n > 0 && rng.below(12) == 0 {
                    rng.below(2) as u32 + 1
                } else {
                    0
                };
                let mut b = if n > 0 && rng.below(25) == 0 {
                    rng.below(2) as u32 + 1
                } else {
                    0
                };
                // Deterministic late bursts exercise quarantine (complaints)
                // and pause (bounces) in the final, reset-free window.
                if domain == "b.example" && day == DAYS - 5 {
                    c = 8;
                }
                if domain == "c.example" && day == DAYS - 4 {
                    b = 20;
                }
                if n > 0 {
                    sends.entry(domain.clone()).or_default().insert(day, n);
                }
                if c > 0 {
                    complaints.entry(domain.clone()).or_default().insert(day, c);
                }
                if b > 0 {
                    bounces.entry(domain.clone()).or_default().insert(day, b);
                }
            }
        }
        let (global, per_domain) = grouped_resets(&resets);
        let days = HealthDays {
            sends: &sends,
            complaints: &complaints,
            bounces: &bounces,
            global_resets: &global,
            domain_resets: &per_domain,
            first_day: 0,
            today: DAYS - 1,
        };
        let single_pass = replay_health(&base, &domains, &days);
        let fold = naive_replay_health(&base, &domains, &days, &resets);
        assert_eq!(
            single_pass, fold,
            "health states, pauses, caps, raw counts and today's sends must match"
        );
        println!(
            "health_replay_equivalence days={DAYS} domains={} resets={} states={:?}",
            domains.len(),
            resets.len(),
            domains
                .iter()
                .map(|d| single_pass.get(d).map(|h| h.state))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn health_replay_visits_each_domain_day_once_and_resets_are_indexed() {
        for domains_n in [1usize, 3, 10] {
            for days in [2_500i64, 5_000, 10_000] {
                let domains: Vec<String> = (0..domains_n)
                    .map(|index| format!("d{index}.example"))
                    .collect();
                let base = HealthConfig::default();
                let mut sends: BTreeMap<String, BTreeMap<i64, u32>> = BTreeMap::new();
                let mut complaints: BTreeMap<String, BTreeMap<i64, u32>> = BTreeMap::new();
                let mut bounces: BTreeMap<String, BTreeMap<i64, u32>> = BTreeMap::new();
                let mut global = BTreeSet::new();
                let mut per_domain: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
                for day in 0..days {
                    // Every day resets every domain: R = days, the worst case
                    // for the old per-domain-day reset scan.
                    global.insert(day);
                    for domain in &domains {
                        per_domain.entry(domain.clone()).or_default().insert(day);
                        sends
                            .entry(domain.clone())
                            .or_default()
                            .insert(day, 5 + (day as u32 % 15));
                        if day % 97 == 0 {
                            complaints.entry(domain.clone()).or_default().insert(day, 1);
                        }
                        if day % 131 == 0 {
                            bounces.entry(domain.clone()).or_default().insert(day, 1);
                        }
                    }
                }
                let trace = HealthDays {
                    sends: &sends,
                    complaints: &complaints,
                    bounces: &bounces,
                    global_resets: &global,
                    domain_resets: &per_domain,
                    first_day: 0,
                    today: days - 1,
                };
                reset_replay_counters();
                let health = replay_health(&base, &domains, &trace);
                let visits = replay_day_visits();
                let probes = replay_reset_probes();
                println!(
                    "health_replay domains={domains_n} days={days} visits={visits} reset_probes={probes} cap={}",
                    health.total_cap()
                );
                assert_eq!(
                    visits,
                    days as u64 * domains_n as u64,
                    "each (domain, day) is visited exactly once"
                );
                assert_eq!(
                    probes,
                    days as u64 * domains_n as u64 * 2,
                    "one global and one named reset-set probe per (domain, day)"
                );
                assert!(
                    probes < days as u64 * days as u64 * domains_n as u64 / 10,
                    "a reset scan would grow quadratically: {probes} probes"
                );
            }
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(touch_id("a@b.io", 0, 1), touch_id("a@b.io", 0, 1));
        assert_ne!(touch_id("a@b.io", 0, 1), touch_id("a@b.io", 1, 1));
        let p = publisher("x.de", 1);
        let mut q = p.clone();
        q.country = None;
        assert_eq!(offset_for(&p, &contact("a@x.de", "x.de")), 120);
        assert_eq!(offset_for(&q, &contact("a@x.de", "x.de")), 60);
        assert_eq!(offset_for(&q, &contact("a@x.io", "x.de")), 0);
        let st = Stages::default();
        assert_eq!(st.reached(), Stage::Contacted);
    }
}
