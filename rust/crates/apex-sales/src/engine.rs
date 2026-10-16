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

use std::collections::BTreeMap;

use crate::bandit::{choose, Arm, BanditConfig};
use crate::cadence::{SendTimeModel, StepModel};
use crate::calendar::{country_offset_minutes, local_parts, next_weekday_hour};
use crate::compose::{render, templates_for_step, Rendered, Sender, TEMPLATES};
use crate::decision::{decide, Autonomy, Decision, GateInput, Verdict};
use crate::funnel::{FunnelModel, Stage};
use crate::health::{HealthConfig, SenderHealth};
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
    /// Who sends.
    pub sender: Sender,
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
    /// Sending-domain health.
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
}

impl Config {
    /// Defaults around a sender identity.
    pub fn new(sender: Sender) -> Self {
        Self {
            sender,
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
        }
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
    /// Sending-domain health.
    pub health: SenderHealth,
    /// Messages sent today.
    pub sent_today: u32,
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
    let mut day_sends: BTreeMap<i64, u32> = BTreeMap::new();
    let mut day_complaints: BTreeMap<i64, u32> = BTreeMap::new();
    let mut day_bounces: BTreeMap<i64, u32> = BTreeMap::new();
    let mut resets: Vec<i64> = Vec::new();
    let publisher_of = |email: &str| snap.contacts.get(email).map(|c| c.publisher_key.clone());

    for e in &snap.events {
        match e {
            Event::Touch {
                at,
                email,
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
                *day_sends.entry(at.div_euclid(DAY)).or_default() += 1;
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
                        *day_complaints.entry(at.div_euclid(DAY)).or_default() += 1;
                        if let Some(em) = email {
                            let pr = progress.entry(em.clone()).or_default();
                            pr.complained_at.get_or_insert(*at);
                        }
                    }
                    Outcome::HardBounce => {
                        *day_bounces.entry(at.div_euclid(DAY)).or_default() += 1;
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
                    Outcome::AuditRequested | Outcome::Connected | Outcome::Paid => {
                        if let Some(pk) = &pubkey {
                            let st = stages.entry(pk.clone()).or_default();
                            let slot = match outcome {
                                Outcome::AuditRequested => &mut st.audit,
                                Outcome::Connected => &mut st.connected,
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
            Event::HealthReset { at } => resets.push(at.div_euclid(DAY)),
        }
    }

    // Sending-domain health, replayed day by day.
    let today = now.div_euclid(DAY);
    let mut health = SenderHealth::new(&config.health);
    let first_day = day_sends
        .keys()
        .chain(day_complaints.keys())
        .chain(day_bounces.keys())
        .copied()
        .min()
        .unwrap_or(today);
    let mut sent_today = 0;
    for day in first_day..=today {
        if resets.contains(&day) {
            health.reset(&config.health);
        }
        let n = day_sends.get(&day).copied().unwrap_or(0);
        let c = day_complaints.get(&day).copied().unwrap_or(0);
        for _ in 0..n {
            health.record_send();
        }
        for _ in 0..n.saturating_sub(c) {
            health.record_complaint_outcome(false, &config.health);
        }
        for _ in 0..c {
            health.record_complaint_outcome(true, &config.health);
        }
        for _ in 0..day_bounces.get(&day).copied().unwrap_or(0) {
            health.record_hard_bounce();
        }
        if day < today {
            health.end_day(day, n, &config.health);
        } else {
            sent_today = n;
        }
    }

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
        sent_today,
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
/// [`Verdict::Deny`] come first, by expected value; the daily cap is applied
/// across them.
pub fn plan(snap: &Snapshot, learned: &Learned, config: &Config, now: UnixSecs) -> Vec<PlanItem> {
    let today = now.div_euclid(DAY);
    let hazard_total: f64 = learned
        .steps
        .interest
        .iter()
        .map(|(a, b)| a / (a + b))
        .sum::<f64>()
        .max(1e-12);
    let cap_left = learned.health.cap.saturating_sub(learned.sent_today);
    let mut items = Vec::new();
    for (email, contact) in &snap.contacts {
        let Some(publisher) = snap.publishers.get(&contact.publisher_key) else {
            continue;
        };
        let empty = Progress::default();
        let pr = learned.progress.get(email).unwrap_or(&empty);
        let k = pr.touches.len();
        if k >= config.max_steps || pr.reply.is_some() {
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
        let band = SizeBand::of(publisher.max_rating_count());
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
        let candidates: Vec<&crate::compose::Template> = templates_for_step(k);
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
            for i in order {
                let Some(t) = candidates.iter().find(|t| t.id == arm_states[i].id) else {
                    continue;
                };
                if !arm_states[i].is_safe(&config.bandit) {
                    continue;
                }
                if let Ok(r) = render(t, publisher, contact, &config.sender) {
                    chosen = Some((t.id.to_string(), r));
                    break;
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
            sender: learned.health.state,
            cap_left,
            grounded: chosen.is_some(),
            variant_available: picked.is_some(),
            expected_value: ev,
        };
        let decision = decide(&input);
        let offset = offset_for(publisher, contact);
        let hour = learned.send_time.choose_hour(&mut rng);
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
            step: k,
            arm,
            send_at,
            local_hour,
            expected_value: ev,
            p_interest: p_mean,
            score_reasons: learned
                .scoring
                .reasons(&fv)
                .iter()
                .map(ToString::to_string)
                .collect(),
            decision,
            rendered,
        });
    }
    // Rank, then spend the cap on the best non-denied items. A publisher gets
    // at most one touch per plan.
    items.sort_by(|a, b| {
        let live = |i: &PlanItem| u8::from(i.decision.verdict != Verdict::Deny);
        live(b)
            .cmp(&live(a))
            .then(b.expected_value.total_cmp(&a.expected_value))
            .then(a.touch_id.cmp(&b.touch_id))
    });
    let mut used = 0u32;
    let mut publishers_used: Vec<String> = Vec::new();
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
        } else if used >= cap_left {
            item.decision.verdict = Verdict::Deny;
            item.decision
                .reasons
                .push("ranked below today's cap".into());
        } else {
            used += 1;
            publishers_used.push(item.publisher.clone());
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

    const NOW: UnixSecs = 20_742 * DAY + 8 * 3600; // Fri 16 Oct 2026 08:00 UTC

    #[test]
    fn a_fresh_state_plans_first_touches_within_the_cap() {
        let s = snap(40);
        let c = config();
        let l = learn(&s, &c, NOW);
        assert_eq!(l.health.cap, c.health.start_cap);
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
            step: 0,
            arm: "t0-short".into(),
            message_id: "m".into(),
            local_hour: 9,
            predicted: None,
        });
        let l = learn(&s, &c, NOW);
        assert_eq!(l.sent_today, 0);
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
