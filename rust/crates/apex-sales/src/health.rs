//! Sending-domain health: when to slow down, pause or stop.
//!
//! Two detectors run on every sending domain:
//!
//! * **Bayesian rate tests.** Complaint and hard-bounce rates have Beta
//!   posteriors over an exponentially discounted window (daily factor
//!   [`HealthConfig::daily_decay`], about a month of memory). A domain is
//!   *paused* when `P(bounce rate > ceiling) ≥ breach_confidence` and
//!   *quarantined* when the same holds for complaints. A softer
//!   `warn_confidence` halves the daily cap instead.
//! * **A Bernoulli cumulative-sum (CUSUM) test** on the complaint stream (Page, 1954): `S ←
//!   max(0, S + llr)`, with `llr = ln(p₁/p₀)` per complaint and
//!   `ln[(1 − p₁)/(1 − p₀)]` per clean send. It catches a sudden jump from
//!   the in-control rate `p₀` to `p₁` within a few dozen sends, well before
//!   the windowed rate moves. Crossing `h` quarantines the domain.
//!
//! Volume follows additive-increase / multiplicative-decrease: a healthy day
//! that used at least 80% of the cap raises it by a fixed step up to the
//! ceiling, a warning halves it, a breach sets it to zero. A quarantined
//! domain stays at zero until an operator resets it; a paused one resumes at
//! half its old cap after a cool-off.
//!
//! Every sending domain is independent: one [`SenderHealth`] per from-domain,
//! held together by [`HealthBook`]. A complaint or bounce attributed to one
//! domain pauses or quarantines that domain only; the daily cap of the whole
//! engine is the sum of the per-domain caps.

use std::collections::BTreeMap;

use crate::stats::beta_tail_above;

/// Thresholds and volume policy.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthConfig {
    /// Complaint-rate ceiling (0.1%, well under Gmail's 0.3% limit).
    pub complaint_ceiling: f64,
    /// Hard-bounce-rate ceiling.
    pub bounce_ceiling: f64,
    /// Posterior probability above a ceiling that triggers a breach.
    pub breach_confidence: f64,
    /// Posterior probability above a ceiling that triggers a warning.
    pub warn_confidence: f64,
    /// Cumulative-sum test: in-control complaint rate.
    pub cusum_p0: f64,
    /// Cumulative-sum test: out-of-control complaint rate.
    pub cusum_p1: f64,
    /// Cumulative-sum test: decision threshold.
    pub cusum_h: f64,
    /// Starting daily cap for a new domain.
    pub start_cap: u32,
    /// Additive step.
    pub cap_step: u32,
    /// Maximum daily cap.
    pub max_cap: u32,
    /// Daily discount of past counts.
    pub daily_decay: f64,
    /// Days a paused domain rests before resuming.
    pub pause_days: i64,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            complaint_ceiling: 0.001,
            bounce_ceiling: 0.03,
            breach_confidence: 0.95,
            warn_confidence: 0.80,
            cusum_p0: 0.0005,
            cusum_p1: 0.005,
            cusum_h: 5.5,
            start_cap: 20,
            cap_step: 10,
            max_cap: 150,
            daily_decay: 0.97,
            pause_days: 3,
        }
    }
}

/// Breaker state of a sending domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderState {
    /// Building volume.
    Warming,
    /// At or near the maximum cap, no warnings.
    Healthy,
    /// A warning halved the cap.
    Throttled,
    /// Hard bounces breached; resting.
    Paused,
    /// Complaints breached; needs an operator reset.
    Quarantined,
}

impl SenderState {
    /// Stable string form.
    pub fn as_str(self) -> &'static str {
        match self {
            SenderState::Warming => "warming",
            SenderState::Healthy => "healthy",
            SenderState::Throttled => "throttled",
            SenderState::Paused => "paused",
            SenderState::Quarantined => "quarantined",
        }
    }

    /// Whether new sends are allowed.
    pub fn may_send(self) -> bool {
        matches!(
            self,
            SenderState::Warming | SenderState::Healthy | SenderState::Throttled
        )
    }
}

/// Rolling health of one sending domain.
#[derive(Debug, Clone, PartialEq)]
pub struct SenderHealth {
    /// Discounted sends.
    pub sent: f64,
    /// Discounted complaints.
    pub complaints: f64,
    /// Discounted hard bounces.
    pub hard_bounces: f64,
    /// Cumulative-sum statistic.
    pub cusum: f64,
    /// Daily cap.
    pub cap: u32,
    /// State.
    pub state: SenderState,
    /// Day the state last changed to paused.
    pub paused_on: Option<i64>,
}

impl SenderHealth {
    /// A new domain.
    pub fn new(config: &HealthConfig) -> Self {
        Self {
            sent: 0.0,
            complaints: 0.0,
            hard_bounces: 0.0,
            cusum: 0.0,
            // A configured ceiling below the warm-up starting cap (for
            // example APEX_SALES_DAILY_MAX=1) is the day's cap from the
            // first send; warm-up never raises a cap past the ceiling.
            cap: config.start_cap.min(config.max_cap),
            state: SenderState::Warming,
            paused_on: None,
        }
    }

    /// Record a send.
    pub fn record_send(&mut self) {
        self.sent += 1.0;
    }

    /// Record the settled outcome of one send for the cumulative-sum test: complaint or
    /// clean.
    pub fn record_complaint_outcome(&mut self, complaint: bool, config: &HealthConfig) {
        let (p0, p1) = (config.cusum_p0, config.cusum_p1);
        let llr = if complaint {
            self.complaints += 1.0;
            (p1 / p0).ln()
        } else {
            ((1.0 - p1) / (1.0 - p0)).ln()
        };
        self.cusum = (self.cusum + llr).max(0.0);
        if self.cusum > config.cusum_h {
            self.state = SenderState::Quarantined;
            self.cap = 0;
        }
    }

    /// Record a hard bounce.
    pub fn record_hard_bounce(&mut self) {
        self.hard_bounces += 1.0;
    }

    /// `P(complaint rate > ceiling)` under a Beta(0.5, 999.5) prior.
    pub fn p_complaints_over(&self, config: &HealthConfig) -> f64 {
        let n = self.sent.max(self.complaints);
        beta_tail_above(
            config.complaint_ceiling,
            0.5 + self.complaints,
            999.5 + n - self.complaints,
        )
    }

    /// `P(hard-bounce rate > ceiling)` under a Beta(1, 99) prior.
    pub fn p_bounces_over(&self, config: &HealthConfig) -> f64 {
        let n = self.sent.max(self.hard_bounces);
        beta_tail_above(
            config.bounce_ceiling,
            1.0 + self.hard_bounces,
            99.0 + n - self.hard_bounces,
        )
    }

    /// Close a day on which `sent_today` messages went out: apply the
    /// breakers, adjust the cap, then discount the counts.
    pub fn end_day(&mut self, day: i64, sent_today: u32, config: &HealthConfig) {
        if self.state == SenderState::Quarantined {
            self.cap = 0;
        } else if self.p_complaints_over(config) >= config.breach_confidence {
            self.state = SenderState::Quarantined;
            self.cap = 0;
        } else if self.state == SenderState::Paused {
            if self.paused_on.is_some_and(|d| day - d >= config.pause_days) {
                self.state = SenderState::Throttled;
                self.cap = (config.start_cap / 2).max(1);
                self.hard_bounces *= 0.5;
                self.paused_on = None;
            }
        } else if self.p_bounces_over(config) >= config.breach_confidence {
            self.state = SenderState::Paused;
            self.paused_on = Some(day);
            self.cap = 0;
        } else if self.p_complaints_over(config) >= config.warn_confidence
            || self.p_bounces_over(config) >= config.warn_confidence
        {
            self.state = SenderState::Throttled;
            self.cap = (self.cap / 2).max(1);
        } else {
            if f64::from(sent_today) >= 0.8 * f64::from(self.cap) {
                self.cap = (self.cap + config.cap_step).min(config.max_cap);
            }
            self.state = if self.cap >= config.max_cap {
                SenderState::Healthy
            } else {
                SenderState::Warming
            };
        }
        let g = config.daily_decay.clamp(0.0, 1.0);
        self.sent *= g;
        self.complaints *= g;
        self.hard_bounces *= g;
    }

    /// Operator reset after an investigation: back to warming at the
    /// starting cap with clean counters.
    pub fn reset(&mut self, config: &HealthConfig) {
        *self = Self::new(config);
    }
}

/// Raw (undiscounted) event counts for one sending domain, for the reports
/// and the workspace tables that show rates as `n / N`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RawCounts {
    /// Sends attributed to the domain.
    pub sends: u32,
    /// Complaints attributed to the domain.
    pub complaints: u32,
    /// Hard bounces attributed to the domain.
    pub hard_bounces: u32,
}

/// The health of every sending domain, keyed by from-domain.
///
/// The order is the configured sender order, which is also the spread order:
/// the engine rotates through it when it hands each planned touch to a
/// domain. `sent_today` is the per-domain count for the current day.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthBook {
    /// Domains in configured order, deduplicated.
    order: Vec<String>,
    /// One rolling health per domain.
    health: BTreeMap<String, SenderHealth>,
    /// Messages sent today, per domain.
    sent_today: BTreeMap<String, u32>,
    /// Raw event counts per domain.
    raw: BTreeMap<String, RawCounts>,
}

impl HealthBook {
    /// A new book: one warming domain per entry of `domains`, in order.
    pub fn new(config: &HealthConfig, domains: &[String]) -> Self {
        let mut order = Vec::new();
        let mut health = BTreeMap::new();
        for domain in domains {
            if health.contains_key(domain) {
                continue;
            }
            order.push(domain.clone());
            health.insert(domain.clone(), SenderHealth::new(config));
        }
        Self {
            order,
            health,
            sent_today: BTreeMap::new(),
            raw: BTreeMap::new(),
        }
    }

    /// The domains, in the configured spread order.
    pub fn domains(&self) -> &[String] {
        &self.order
    }

    /// One domain's rolling health.
    pub fn get(&self, domain: &str) -> Option<&SenderHealth> {
        self.health.get(domain)
    }

    /// One domain's rolling health, mutably (the replay).
    pub fn health_mut(&mut self, domain: &str) -> Option<&mut SenderHealth> {
        self.health.get_mut(domain)
    }

    /// Messages sent today on `domain`.
    pub fn sent_today_for(&self, domain: &str) -> u32 {
        self.sent_today.get(domain).copied().unwrap_or(0)
    }

    /// Set today's send count for `domain` (the replay).
    pub fn set_sent_today(&mut self, domain: &str, n: u32) {
        if self.health.contains_key(domain) {
            self.sent_today.insert(domain.to_string(), n);
        }
    }

    /// Raw counts for `domain`.
    pub fn raw(&self, domain: &str) -> RawCounts {
        self.raw.get(domain).copied().unwrap_or_default()
    }

    /// Replay one day of raw counts onto a domain's health: sends first, then
    /// the clean and complaining outcomes in the engine's fixed order, then
    /// the hard bounces.
    pub fn record_day(
        &mut self,
        domain: &str,
        sends: u32,
        complaints: u32,
        hard_bounces: u32,
        config: &HealthConfig,
    ) {
        let Some(h) = self.health.get_mut(domain) else {
            return;
        };
        for _ in 0..sends {
            h.record_send();
        }
        for _ in 0..sends.saturating_sub(complaints) {
            h.record_complaint_outcome(false, config);
        }
        for _ in 0..complaints {
            h.record_complaint_outcome(true, config);
        }
        for _ in 0..hard_bounces {
            h.record_hard_bounce();
        }
        let raw = self.raw.entry(domain.to_string()).or_default();
        raw.sends = raw.sends.saturating_add(sends);
        raw.complaints = raw.complaints.saturating_add(complaints);
        raw.hard_bounces = raw.hard_bounces.saturating_add(hard_bounces);
    }

    /// Operator reset: one domain when named, otherwise every domain.
    pub fn reset(&mut self, domain: Option<&str>, config: &HealthConfig) {
        match domain {
            Some(d) => {
                if let Some(h) = self.health.get_mut(d) {
                    h.reset(config);
                }
            }
            None => {
                for h in self.health.values_mut() {
                    h.reset(config);
                }
            }
        }
    }

    /// Sum of the per-domain caps: the engine's global daily cap.
    pub fn total_cap(&self) -> u32 {
        self.health.values().map(|h| h.cap).sum()
    }

    /// Messages sent today across every domain.
    pub fn sent_today(&self) -> u32 {
        self.sent_today.values().sum()
    }

    /// Sends left today across the domains that may send.
    pub fn cap_left(&self) -> u32 {
        self.health
            .iter()
            .filter(|(_, h)| h.state.may_send())
            .map(|(d, h)| h.cap.saturating_sub(self.sent_today_for(d)))
            .sum()
    }

    /// Whether any domain may send.
    pub fn may_send(&self) -> bool {
        self.health.values().any(|h| h.state.may_send())
    }

    /// The one state that summarizes the book: the best sendable state when
    /// any domain may send, otherwise the most severe state recorded. An
    /// empty book cannot send.
    pub fn summary_state(&self) -> SenderState {
        let rank = |s: SenderState| match s {
            SenderState::Healthy => 0,
            SenderState::Warming => 1,
            SenderState::Throttled => 2,
            SenderState::Paused => 3,
            SenderState::Quarantined => 4,
        };
        let states: Vec<SenderState> = self.health.values().map(|h| h.state).collect();
        if let Some(s) = states
            .iter()
            .copied()
            .filter(|s| s.may_send())
            .min_by_key(|s| rank(*s))
        {
            return s;
        }
        // No domain may send: the most severe state explains why.
        states
            .into_iter()
            .max_by_key(|s| rank(*s))
            .unwrap_or(SenderState::Paused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_days(h: &mut SenderHealth, c: &HealthConfig, days: i64, complaint_every: Option<u32>) {
        for day in 0..days {
            let n = h.cap;
            for i in 0..n {
                h.record_send();
                let complaint = complaint_every.is_some_and(|k| i % k == 0);
                h.record_complaint_outcome(complaint, c);
            }
            h.end_day(day, n, c);
        }
    }

    #[test]
    fn clean_domains_ramp_to_the_ceiling() {
        let c = HealthConfig::default();
        let mut h = SenderHealth::new(&c);
        assert_eq!(h.state, SenderState::Warming);
        run_days(&mut h, &c, 20, None);
        assert_eq!(h.cap, c.max_cap);
        assert_eq!(h.state, SenderState::Healthy);
        assert!(h.state.may_send());
    }

    #[test]
    fn a_complaint_burst_quarantines_fast_via_cusum() {
        let c = HealthConfig::default();
        let mut h = SenderHealth::new(&c);
        run_days(&mut h, &c, 10, None);
        // Three complaints in a row from a clean domain.
        for _ in 0..3 {
            h.record_send();
            h.record_complaint_outcome(true, &c);
        }
        assert_eq!(h.state, SenderState::Quarantined);
        assert!(!h.state.may_send());
        h.end_day(11, 0, &c);
        assert_eq!(h.cap, 0);
        h.reset(&c);
        assert_eq!(h.state, SenderState::Warming);
    }

    #[test]
    fn a_configured_ceiling_below_the_start_cap_is_the_first_day_cap() {
        let c = HealthConfig {
            start_cap: 20,
            max_cap: 1,
            ..HealthConfig::default()
        };
        let mut h = SenderHealth::new(&c);
        assert_eq!(h.cap, 1, "the ceiling wins from the first send");
        h.record_send();
        h.end_day(0, 1, &c);
        assert_eq!(h.cap, 1, "a full healthy day never raises past the ceiling");
        let book = HealthBook::new(&c, &["one.example".to_string()]);
        assert_eq!(book.total_cap(), 1);
        let zero = HealthConfig {
            start_cap: 20,
            max_cap: 0,
            ..HealthConfig::default()
        };
        assert_eq!(SenderHealth::new(&zero).cap, 0);
        assert_eq!(
            HealthBook::new(&zero, &["one.example".to_string()]).cap_left(),
            0
        );
    }

    #[test]
    fn steady_complaints_breach_the_rate_test_without_cusum() {
        let c = HealthConfig {
            cusum_h: f64::INFINITY,
            ..HealthConfig::default()
        };
        let mut h = SenderHealth::new(&c);
        run_days(&mut h, &c, 15, Some(250)); // one complaint a day
        assert_eq!(h.state, SenderState::Quarantined, "{h:?}");
    }

    #[test]
    fn bounces_pause_then_resume_throttled() {
        let c = HealthConfig::default();
        let mut h = SenderHealth::new(&c);
        for _ in 0..200 {
            h.record_send();
        }
        for _ in 0..20 {
            h.record_hard_bounce();
        }
        h.end_day(0, 200, &c);
        assert_eq!(h.state, SenderState::Paused);
        assert_eq!(h.cap, 0);
        h.end_day(1, 0, &c);
        assert_eq!(h.state, SenderState::Paused);
        h.end_day(3, 0, &c);
        assert_eq!(h.state, SenderState::Throttled);
        assert_eq!(h.cap, 10);
    }

    #[test]
    fn a_warning_halves_the_cap() {
        let c = HealthConfig::default();
        let mut h = SenderHealth::new(&c);
        h.cap = 100;
        for _ in 0..300 {
            h.record_send();
        }
        // Find a complaint level between the warning and the breach.
        let level = (1..400).map(|k| f64::from(k) * 0.05).find(|&k| {
            h.complaints = k;
            let p = h.p_complaints_over(&c);
            p >= c.warn_confidence && p < c.breach_confidence
        });
        assert!(level.is_some());
        h.end_day(0, 100, &c);
        assert_eq!(h.state, SenderState::Throttled);
        assert_eq!(h.cap, 50);
        for s in [
            SenderState::Warming,
            SenderState::Healthy,
            SenderState::Throttled,
            SenderState::Paused,
            SenderState::Quarantined,
        ] {
            assert!(!s.as_str().is_empty());
        }
    }

    fn domains() -> Vec<String> {
        vec!["a.example".to_string(), "b.example".to_string()]
    }

    #[test]
    fn the_book_keyes_health_by_domain_and_isolates_breaches() {
        let c = HealthConfig::default();
        let mut book = HealthBook::new(&c, &domains());
        assert_eq!(book.domains(), domains().as_slice());
        assert_eq!(book.summary_state(), SenderState::Warming);
        assert_eq!(book.total_cap(), c.start_cap * 2);
        assert_eq!(book.cap_left(), c.start_cap * 2);

        // Day one on a.example: enough complaints to quarantine it alone.
        book.record_day("a.example", 40, 5, 0, &c);
        if let Some(h) = book.health_mut("a.example") {
            h.end_day(0, 40, &c);
        }
        book.record_day("b.example", 20, 0, 0, &c);
        if let Some(h) = book.health_mut("b.example") {
            h.end_day(0, 20, &c);
        }
        assert_eq!(
            book.get("a.example").map(|h| h.state),
            Some(SenderState::Quarantined)
        );
        assert_eq!(
            book.get("b.example").map(|h| h.state),
            Some(SenderState::Warming)
        );
        assert!(book.may_send());
        assert_eq!(book.cap_left(), 30, "only the healthy domain's cap counts");
        assert_eq!(book.summary_state(), SenderState::Warming);
        // Raw counts stay per domain.
        assert_eq!(book.raw("a.example").complaints, 5);
        assert_eq!(book.raw("b.example").complaints, 0);
        assert_eq!((book.raw("a.example").sends, book.sent_today()), (40, 0));
        // Counting today on one domain.
        book.set_sent_today("b.example", 7);
        assert_eq!(book.sent_today(), 7);
        assert_eq!(book.cap_left(), 23);

        // A named reset touches one domain; a bare reset touches both.
        book.reset(Some("b.example"), &c);
        assert_eq!(
            book.get("b.example").map(|h| h.state),
            Some(SenderState::Warming)
        );
        assert_eq!(
            book.get("a.example").map(|h| h.state),
            Some(SenderState::Quarantined)
        );
        book.reset(None, &c);
        assert!(book
            .health
            .values()
            .all(|h| h.state == SenderState::Warming));

        // Pausing every domain makes the book unable to send, and the
        // summary names the most severe state.
        let mut paused = HealthBook::new(&c, &["x.example".to_string()]);
        paused.record_day("x.example", 100, 0, 20, &c);
        if let Some(h) = paused.health_mut("x.example") {
            h.end_day(0, 100, &c);
        }
        assert_eq!(paused.summary_state(), SenderState::Paused);
        assert!(!paused.may_send());
        assert_eq!(paused.cap_left(), 0);
        // An empty book cannot send either.
        let empty = HealthBook::new(&c, &[]);
        assert!(!empty.may_send());
        assert_eq!(empty.total_cap(), 0);
    }
}
