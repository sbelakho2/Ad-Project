//! Lead scoring: a Bayesian logistic model of "this contact replies with
//! interest during the sequence".
//!
//! # Model
//!
//! `P(interest | x) = σ(w · x)` with a diagonal Gaussian over `w`. Each
//! weight starts from a documented prior (mean and standard deviation in
//! log-odds), so the engine is sensible on day one and learns from every
//! settled outcome after that.
//!
//! Updates follow Chapelle and Li (2011, "An Empirical Evaluation of Thompson
//! Sampling", Algorithm 3): for a batch, `w = argmin ½ Σ qᵢ (wᵢ − mᵢ)² +
//! Σⱼ log[1 + exp(−ỹⱼ w·xⱼ)]`, then `m ← w` and `qᵢ ← qᵢ + Σⱼ xᵢⱼ² pⱼ(1 − pⱼ)`
//! (the Laplace approximation's diagonal). The minimization is a
//! diagonal-Newton iteration with backtracking, so it never increases the
//! objective.
//!
//! Predictions integrate over weight uncertainty with MacKay's probit
//! approximation, `P ≈ σ(μ / √[1 + πσ²/8])`, where `μ = m·x` and
//! `σ² = Σ xᵢ²/qᵢ`; an uncertain lead is pulled toward 50% log-odds rather
//! than reported with false confidence. [`LogisticModel::sample`] draws
//! weights for Thompson sampling.
//!
//! # Reason codes
//!
//! Every score is explained in log-odds: contribution `mᵢxᵢ` per feature,
//! with the evidence behind it. The contributions sum exactly to the
//! posterior-mean log-odds ([`LogisticModel::logit_mean`]).

use crate::model::{Contact, ContactKind, Publisher};
use crate::rng::Rng;
use crate::signals::{mediation_features, MediationModel, Platform};
use crate::stats::sigmoid;

/// Number of features.
pub const N: usize = 15;

/// Feature names, in vector order.
pub const FEATURES: [&str; N] = [
    "bias",
    "size",
    "game",
    "free",
    "ads_txt_missing",
    "ads_txt_errors",
    "mediation_max",
    "mediation_levelplay",
    "mediation_admob",
    "networks",
    "fresh",
    "named_contact",
    "generic_contact",
    "inbound",
    "no_store_data",
];

/// Prior `(mean, standard deviation)` per feature, in log-odds.
///
/// The bias is the log-odds of a contact with every feature at zero; a
/// typical cold lead (a free, recently updated game with a citable
/// `app-ads.txt` problem and a named contact) lands near 4% interest over the
/// sequence, in line with the per-step hazards in
/// [`crate::cadence::StepModel`]. Inbound requests are
/// far warmer (+3.0). Concrete, citable problems (a missing or broken
/// `app-ads.txt`) and fresh, free, game apps make interest more likely;
/// generic mailboxes make it less likely. Standard deviations say how far
/// data may move each weight.
pub const PRIOR: [(f64, f64); N] = [
    (-4.3, 1.0),
    (0.15, 0.4),
    (0.2, 0.5),
    (0.1, 0.5),
    (0.2, 0.5),
    (0.15, 0.4),
    (0.2, 0.5),
    (0.1, 0.5),
    (0.0, 0.5),
    (0.1, 0.3),
    (0.2, 0.5),
    (0.3, 0.5),
    (-0.5, 0.5),
    (3.0, 1.0),
    (-0.3, 0.5),
];

/// A feature vector with a note per feature explaining its value.
#[derive(Debug, Clone, PartialEq)]
pub struct FeatureVector {
    /// Values.
    pub x: [f64; N],
    /// Human-readable basis of each value.
    pub notes: [String; N],
}

/// Build the features of `contact` at `publisher`.
pub fn features(
    publisher: &Publisher,
    contact: &Contact,
    mediation: &MediationModel,
) -> FeatureVector {
    let mut x = [0.0; N];
    let mut notes: [String; N] = Default::default();
    x[0] = 1.0;
    notes[0] = "baseline for a cold contact".into();

    match publisher.max_rating_count() {
        Some(n) => {
            x[1] = (((n as f64) + 1.0).log10() - 3.0) / 1.5;
            notes[1] = format!("largest app has {n} ratings");
        }
        None => notes[1] = "no ratings data".into(),
    }
    if publisher
        .apps
        .iter()
        .any(|a| a.genre.as_deref().is_some_and(|g| g.contains("Game")))
    {
        x[2] = 1.0;
        notes[2] = "ships games".into();
    }
    if publisher.apps.iter().any(|a| a.free == Some(true)) {
        x[3] = 1.0;
        notes[3] = "has free apps".into();
    }
    if publisher.ads_txt_missing {
        x[4] = 1.0;
        notes[4] = "app-ads.txt not found".into();
    }
    let probs = match publisher
        .declared_platforms
        .iter()
        .find_map(|p| Platform::parse(p))
    {
        Some(p) => {
            let mut one_hot = [0.0; 4];
            one_hot[p.index()] = 1.0;
            notes[6] = "declared on the audit form".into();
            one_hot
        }
        None => match &publisher.ads_txt() {
            Some(r) => {
                notes[6] = "inferred from app-ads.txt DIRECT lines".into();
                mediation.posterior(mediation_features(r))
            }
            None => {
                notes[6] = "platform shares (no app-ads.txt)".into();
                mediation.prior
            }
        },
    };
    x[6] = probs[0];
    x[7] = probs[1];
    x[8] = probs[2];
    notes[7] = notes[6].clone();
    notes[8] = notes[6].clone();
    if let Some(r) = &publisher.ads_txt() {
        x[5] = (1.0 + r.errors.len() as f64).ln();
        notes[5] = format!("{} invalid app-ads.txt lines", r.errors.len());
        let nets = r.networks().len();
        x[9] = (1.0 + nets as f64).ln();
        notes[9] = format!("{nets} known networks authorized");
    }
    if publisher
        .apps
        .iter()
        .filter_map(|a| a.days_since_update)
        .min()
        .is_some_and(|d| d <= 60)
    {
        x[10] = 1.0;
        notes[10] = "updated in the last 60 days".into();
    }
    match contact.kind {
        ContactKind::Named => {
            x[11] = 1.0;
            notes[11] = "named person".into();
        }
        ContactKind::Generic => {
            x[12] = 1.0;
            notes[12] = "generic mailbox".into();
        }
        ContactKind::Role => {}
    }
    if publisher.inbound {
        x[13] = 1.0;
        notes[13] = "requested the free audit".into();
    }
    if publisher.apps.is_empty() {
        x[14] = 1.0;
        notes[14] = "no store listing known".into();
    }
    FeatureVector { x, notes }
}

/// One signed log-odds contribution.
#[derive(Debug, Clone, PartialEq)]
pub struct Reason {
    /// Feature name.
    pub feature: &'static str,
    /// Contribution in log-odds.
    pub log_odds: f64,
    /// Basis.
    pub note: String,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:+.2} {}: {}", self.log_odds, self.feature, self.note)
    }
}

/// Diagonal-Gaussian Bayesian logistic regression.
#[derive(Debug, Clone, PartialEq)]
pub struct LogisticModel {
    /// Posterior means.
    pub mean: [f64; N],
    /// Posterior precisions (1 / variance).
    pub precision: [f64; N],
}

impl Default for LogisticModel {
    fn default() -> Self {
        let mut mean = [0.0; N];
        let mut precision = [0.0; N];
        for (i, (m, sd)) in PRIOR.iter().enumerate() {
            mean[i] = *m;
            precision[i] = 1.0 / (sd * sd);
        }
        Self { mean, precision }
    }
}

fn dot(a: &[f64; N], b: &[f64; N]) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl LogisticModel {
    /// Posterior-mean log-odds.
    pub fn logit_mean(&self, x: &[f64; N]) -> f64 {
        dot(&self.mean, x)
    }

    /// Posterior variance of the log-odds.
    pub fn logit_variance(&self, x: &[f64; N]) -> f64 {
        x.iter()
            .zip(&self.precision)
            .map(|(xi, qi)| xi * xi / qi)
            .sum()
    }

    /// Predictive probability, integrating weight uncertainty (probit
    /// approximation).
    pub fn predict(&self, x: &[f64; N]) -> f64 {
        let mu = self.logit_mean(x);
        let var = self.logit_variance(x);
        sigmoid(mu / (1.0 + std::f64::consts::PI * var / 8.0).sqrt())
    }

    /// A Thompson draw: probability under weights sampled from the posterior.
    pub fn sample(&self, x: &[f64; N], rng: &mut Rng) -> f64 {
        let z: f64 = self
            .mean
            .iter()
            .zip(&self.precision)
            .zip(x)
            .map(|((m, q), xi)| (m + rng.normal() / q.sqrt()) * xi)
            .sum();
        sigmoid(z)
    }

    /// Reason codes; their `log_odds` sum to [`Self::logit_mean`].
    pub fn reasons(&self, fv: &FeatureVector) -> Vec<Reason> {
        (0..N)
            .filter(|&i| fv.x[i] != 0.0)
            .map(|i| Reason {
                feature: FEATURES[i],
                log_odds: self.mean[i] * fv.x[i],
                note: fv.notes[i].clone(),
            })
            .collect()
    }

    fn objective(&self, prior: &LogisticModel, w: &[f64; N], batch: &[([f64; N], bool)]) -> f64 {
        let mut total: f64 = prior
            .precision
            .iter()
            .zip(&prior.mean)
            .zip(w)
            .map(|((q, m), wi)| 0.5 * q * (wi - m).powi(2))
            .sum();
        for (x, y) in batch {
            let z = dot(w, x);
            let s = if *y { z } else { -z };
            // log[1 + exp(−s)], stably.
            total += if s > 0.0 {
                (-s).exp().ln_1p()
            } else {
                -s + s.exp().ln_1p()
            };
        }
        total
    }

    /// Bayesian update with a batch of `(features, interested)` outcomes.
    pub fn update(&mut self, batch: &[([f64; N], bool)]) {
        if batch.is_empty() {
            return;
        }
        let prior = self.clone();
        let mut w = prior.mean;
        let mut current = self.objective(&prior, &w, batch);
        for _ in 0..100 {
            let mut grad = [0.0; N];
            let mut hess = prior.precision;
            for ((g, wi), (q, m)) in grad
                .iter_mut()
                .zip(&w)
                .zip(prior.precision.iter().zip(&prior.mean))
            {
                *g = q * (wi - m);
            }
            for (x, y) in batch {
                let p = sigmoid(dot(&w, x));
                let residual = p - if *y { 1.0 } else { 0.0 };
                for ((g, h), xi) in grad.iter_mut().zip(hess.iter_mut()).zip(x) {
                    *g += residual * xi;
                    *h += p * (1.0 - p) * xi * xi;
                }
            }
            let mut step = 1.0;
            let mut improved = false;
            for _ in 0..30 {
                let mut trial = w;
                for ((t, g), h) in trial.iter_mut().zip(&grad).zip(&hess) {
                    *t -= step * g / h;
                }
                let value = self.objective(&prior, &trial, batch);
                if value <= current {
                    improved = current - value > 1e-12;
                    w = trial;
                    current = value;
                    break;
                }
                step *= 0.5;
            }
            if !improved {
                break;
            }
        }
        let mut precision = prior.precision;
        for (x, _) in batch {
            let p = sigmoid(dot(&w, x));
            for (q, xi) in precision.iter_mut().zip(x) {
                *q += p * (1.0 - p) * xi * xi;
            }
        }
        self.mean = w;
        self.precision = precision;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{App, Store};

    fn publisher() -> Publisher {
        let mut p = Publisher::new("studio.example", "Studio");
        p.apps.push(App {
            store: Store::AppStore,
            id: "1".into(),
            name: "Tile Quest".into(),
            genre: Some("Games".into()),
            rating_count: Some(12_000),
            free: Some(true),
            days_since_update: Some(12),
        });
        p.ads_txt_text =
            Some("applovin.com, 1, DIRECT\ngoogle.com, pub-1, DIRECT\nbad line\n".into());
        p
    }

    fn contact(email: &str) -> Contact {
        Contact {
            email: email.into(),
            name: None,
            role: None,
            publisher_key: "studio.example".into(),
            kind: ContactKind::of_email(email),
            source: "test".into(),
            consent: false,
        }
    }

    #[test]
    fn features_read_the_evidence() {
        let fv = features(
            &publisher(),
            &contact("maria@studio.example"),
            &MediationModel::default(),
        );
        assert_eq!(fv.x[0], 1.0);
        assert!(fv.x[1] > 0.5 && fv.x[1] < 1.0);
        assert_eq!((fv.x[2], fv.x[3], fv.x[10], fv.x[11]), (1.0, 1.0, 1.0, 1.0));
        assert!((fv.x[5] - 3f64.ln()).abs() < 1e-12, "two errors → ln 3");
        assert!(fv.x[6] > 0.5, "MAX likely: {}", fv.x[6]);
        assert_eq!(fv.x[13], 0.0);
        assert!(fv.notes[5].contains("2 invalid"));

        let mut bare = Publisher::new("x.io", "X");
        bare.ads_txt_missing = true;
        bare.inbound = true;
        bare.declared_platforms = vec!["LevelPlay".into()];
        let fv = features(&bare, &contact("info@x.io"), &MediationModel::default());
        assert_eq!(
            (fv.x[4], fv.x[7], fv.x[12], fv.x[13], fv.x[14]),
            (1.0, 1.0, 1.0, 1.0, 1.0)
        );
        assert_eq!(fv.x[6], 0.0);
        let no_txt = Publisher::new("y.io", "Y");
        let fv = features(&no_txt, &contact("bizdev@y.io"), &MediationModel::default());
        assert!((fv.x[6] - 0.5).abs() < 1e-12, "prior share");
    }

    #[test]
    fn reason_codes_reconcile_exactly() {
        let model = LogisticModel::default();
        let fv = features(
            &publisher(),
            &contact("maria@studio.example"),
            &MediationModel::default(),
        );
        let reasons = model.reasons(&fv);
        let total: f64 = reasons.iter().map(|r| r.log_odds).sum();
        assert!((total - model.logit_mean(&fv.x)).abs() < 1e-12);
        assert!(reasons.iter().any(|r| r.feature == "ads_txt_errors"));
        assert!(reasons[0].to_string().starts_with("-4.30 bias"));
    }

    #[test]
    fn predictions_shrink_under_uncertainty() {
        let model = LogisticModel::default();
        let fv = features(
            &publisher(),
            &contact("maria@studio.example"),
            &MediationModel::default(),
        );
        let p = model.predict(&fv.x);
        let point = sigmoid(model.logit_mean(&fv.x));
        assert!(
            p > point,
            "uncertainty pulls a low rate up toward 0.5: {p} {point}"
        );
        assert!(p < 0.2);
        let mut rng = Rng::seeded(1);
        let draws: Vec<f64> = (0..2000).map(|_| model.sample(&fv.x, &mut rng)).collect();
        let mean = draws.iter().sum::<f64>() / draws.len() as f64;
        assert!((mean - p).abs() < 0.02, "{mean} {p}");
    }

    #[test]
    fn updates_learn_the_true_weights_and_tighten() {
        // Ground truth: generic mailboxes never reply; named ones do at 20%.
        let mut model = LogisticModel::default();
        let mut rng = Rng::seeded(9);
        let named = {
            let mut x = [0.0; N];
            x[0] = 1.0;
            x[11] = 1.0;
            x
        };
        let generic = {
            let mut x = [0.0; N];
            x[0] = 1.0;
            x[12] = 1.0;
            x
        };
        let before_var = model.logit_variance(&named);
        for _ in 0..20 {
            let mut batch = Vec::new();
            for _ in 0..100 {
                batch.push((named, rng.uniform() < 0.20));
                batch.push((generic, rng.uniform() < 0.01));
            }
            model.update(&batch);
        }
        let p_named = model.predict(&named);
        let p_generic = model.predict(&generic);
        assert!((p_named - 0.20).abs() < 0.03, "{p_named}");
        assert!((p_generic - 0.01).abs() < 0.01, "{p_generic}");
        assert!(model.logit_variance(&named) < before_var / 10.0);
        let unchanged = model.clone();
        model.update(&[]);
        assert_eq!(model, unchanged);
    }

    #[test]
    fn update_never_increases_the_objective() {
        let mut model = LogisticModel::default();
        let mut x = [0.0; N];
        x[0] = 1.0;
        x[13] = 1.0;
        let batch = vec![(x, true); 50];
        let prior = model.clone();
        let start = model.objective(&prior, &prior.mean, &batch);
        model.update(&batch);
        let end = model.objective(&prior, &model.mean, &batch);
        assert!(end <= start);
    }
}
