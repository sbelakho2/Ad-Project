//! `/models`: read-only model reports — calibration, message arms, funnel
//! posteriors, cadence and the holdout effect. Nothing here is computed in
//! this crate: every number is an engine function's return value.

use apex_sales::bandit::report as arm_report;
use apex_sales::cadence::HOURS;
use apex_sales::compose::templates_for_step;
use apex_sales::ops::{calibration_pairs, holdout_groups};
use apex_sales::rng::Rng;
use apex_sales::signals::SizeBand;
use apex_sales::stats::{beta_quantile, brier, log_loss, reliability};
use apex_ui::charts::{line_chart, ChartColor, ChartSeries, ChartSpec};
use apex_ui::components::{data_table, notice, panel, Cell, EmptyState};
use apex_ui::format::{or_not_available, percent};
use apex_ui::status::StatusKind;

use crate::data::RequestData;
use crate::pages::shared::{self, rate, PageMeta};

/// Monte Carlo draws for the arm report (the CLI's report uses 4000; the
/// page uses fewer for render latency, same estimator).
pub const ARM_DRAWS: usize = 2_000;

/// Reliability bins.
pub const RELIABILITY_BINS: usize = 10;

/// Render `/models`.
pub fn render(meta: &PageMeta, data: &RequestData) -> String {
    let body = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        calibration(data),
        arms(data),
        funnel(data),
        cadence(data),
        holdout(data),
        "<p class=\"table-note\">Every table on this page renders the engine's own public functions: apex_sales::ops for the settled pairs and holdout groups, stats for Brier/log loss/reliability/Beta quantiles, bandit::report for the arm summaries, funnel, cadence and holdout for the rest. Nothing is interpolated or fabricated; a model without data says so with its reason.</p>"
    );
    shared::page(meta, &body)
}

fn calibration(data: &RequestData) -> String {
    let pairs = calibration_pairs(&data.learned, &data.config, data.now);
    if pairs.is_empty() {
        let body = format!(
            "{}\n{}",
            notice(
                StatusKind::Unavailable,
                "Calibration",
                &format!(
                    "Not available (no settled first-touch predictions yet). A lead settles when it became interested or {} days passed since its last touch; {} sends are recorded so far.",
                    data.config.settle_days,
                    data.learned
                        .progress
                        .values()
                        .map(|progress| progress.touches.len())
                        .sum::<usize>()
                ),
                None,
            ),
            "<p class=\"table-note\">Calibration is measured out of sample: the prediction recorded at send time against what actually happened, never a refit on the same data.</p>"
        );
        return panel("Calibration", &body, None);
    }
    let brier = brier(&pairs);
    let logloss = log_loss(&pairs);
    let (bins, ece) = reliability(&pairs, RELIABILITY_BINS);
    let summary = [
        (
            "Brier score",
            brier
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| or_not_available("no pairs")),
        ),
        (
            "Log loss",
            logloss
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| or_not_available("no pairs")),
        ),
        (
            "Expected calibration error",
            ece.map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| or_not_available("no pairs")),
        ),
        ("Settled first-touch leads", pairs.len().to_string()),
    ];
    let summary_rows: Vec<Vec<Cell>> = summary
        .into_iter()
        .map(|(name, value)| vec![Cell::Text(name.to_string()), Cell::Num(value)])
        .collect();
    let bin_rows: Vec<Vec<Cell>> = bins
        .iter()
        .map(|bin| {
            vec![
                Cell::Text(format!("{} to {}", percent(bin.lower), percent(bin.upper))),
                Cell::Num(bin.n.to_string()),
                Cell::Num(percent(bin.mean_predicted)),
                Cell::Num(percent(bin.observed)),
                Cell::Num(format!(
                    "{} to {}",
                    percent(bin.interval.low),
                    percent(bin.interval.high)
                )),
            ]
        })
        .collect();
    let spec = ChartSpec {
        title: "Reliability: observed vs predicted".to_string(),
        desc: "Observed interest rate per equal-width predicted-probability bin, against the perfectly calibrated reference (the bin's mean prediction). Empty bins are gaps.".to_string(),
        width: 720,
        height: 240,
        y_label: "Rate".to_string(),
        x_labels: bins
            .iter()
            .map(|bin| format!("{:.0}", ((bin.lower + bin.upper) / 2.0) * 100.0))
            .collect(),
        zero_line: false,
    };
    let observed = ChartSeries {
        label: "Observed".to_string(),
        color: ChartColor::Test,
        points: bins.iter().map(|bin| Some(bin.observed)).collect(),
    };
    let predicted = ChartSeries {
        label: "Perfect calibration (mean prediction)".to_string(),
        color: ChartColor::Control,
        points: bins.iter().map(|bin| Some(bin.mean_predicted)).collect(),
    };
    let chart = line_chart(&spec, &[predicted, observed], 0.0, 1.0);
    let body = format!(
        "{}\n{}\n{}\n{}",
        data_table(
            Some("Calibration summary"),
            &["Metric", "Value"],
            &summary_rows,
            None,
            Some(&format!(
                "measured on {} settled leads / as of the page's as-of time",
                pairs.len()
            )),
        ),
        data_table(
            Some("Reliability bins"),
            &["Predicted bin", "n", "Mean prediction", "Observed rate", "95% interval"],
            &bin_rows,
            Some(EmptyState {
                reason: "No pairs landed in any bin.".to_string(),
                action: None,
            }),
            Some(&format!(
                "{} bins over {} settled leads; the interval is the Jeffreys interval of the observed rate",
                RELIABILITY_BINS,
                pairs.len()
            )),
        ),
        chart,
        "<p class=\"table-note\">The chart is the bins table as a line: x is the bin's midpoint predicted probability, the control series is perfect calibration, the cobalt series is the observed rate with empty bins as gaps.</p>"
    );
    panel("Calibration", &body, None)
}

fn arms(data: &RequestData) -> String {
    let mut sections = String::new();
    let mut any_sends = false;
    for step in 0..data.config.max_steps {
        let arms: Vec<apex_sales::bandit::Arm> = templates_for_step(step)
            .iter()
            .filter_map(|template| data.learned.arms.get(template.id).cloned())
            .collect();
        if arms.is_empty() {
            continue;
        }
        any_sends |= arms.iter().any(|arm| arm.sends > 0.0);
        let mut rng = Rng::seeded((data.now as u64) ^ (step as u64));
        let (report, loss) = arm_report(&arms, &data.config.bandit, ARM_DRAWS, &mut rng);
        let rows: Vec<Vec<Cell>> = report
            .iter()
            .map(|row| {
                let state = arms
                    .iter()
                    .find(|arm| arm.id == row.id)
                    .map(|arm| arm.value)
                    .unwrap_or((1.0, 1.0));
                let low = beta_quantile(0.025, state.0, state.1);
                let high = beta_quantile(0.975, state.0, state.1);
                let sends = arms
                    .iter()
                    .find(|arm| arm.id == row.id)
                    .map(|arm| arm.sends)
                    .unwrap_or(0.0);
                vec![
                    Cell::Text(format!("Step {}", step + 1)),
                    Cell::Mono(row.id.clone()),
                    Cell::Num(format!("{sends:.1}")),
                    Cell::Num(format!("{:.5}", row.mean_value)),
                    Cell::Num(format!("{low:.5} to {high:.5}")),
                    Cell::Num(format!("{:.2}", row.p_best)),
                    if row.safe {
                        Cell::Status("Safe".to_string(), StatusKind::Verified)
                    } else {
                        Cell::Status("Unsafe: complaint ceiling".to_string(), StatusKind::Failure)
                    },
                ]
            })
            .collect();
        sections.push_str(&data_table(
            Some(&format!("Step {} message arms", step + 1)),
            &["Step", "Arm", "n (discounted sends)", "Posterior mean value", "95% credible interval", "P(best)", "Safety"],
            &rows,
            Some(EmptyState {
                reason: "No template is registered for this step.".to_string(),
                action: None,
            }),
            Some(&format!(
                "Monte Carlo with {ARM_DRAWS} joint posterior draws; the expected loss of committing to the current leader is {loss:.6} (value per send)"
            )),
        ));
        sections.push('\n');
    }
    let note = if any_sends {
        "<p class=\"table-note\">n counts settled sends; the posterior mean and the credible interval are the engine's Beta posterior of the arm's reward per send (a value-to-go imputation, not a measured invoice).</p>".to_string()
    } else {
        "<p class=\"table-note\">No settled sends are recorded yet, so every arm shows its prior: the numbers are the documented priors, not observations.</p>".to_string()
    };
    panel("Message arms", &format!("{sections}{note}"), None)
}

fn funnel(data: &RequestData) -> String {
    let funnel = &data.learned.funnel;
    let transitions = ["Reached the audit", "Connected a platform", "Paid"];
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    for band in SizeBand::ALL {
        for (index, label) in transitions.iter().enumerate() {
            let counts = funnel.by_band[band_index(band)][index];
            let posterior = funnel.band_posterior(band, index);
            let mean = if posterior.0 + posterior.1 > 0.0 {
                percent(posterior.0 / (posterior.0 + posterior.1))
            } else {
                or_not_available("no posterior")
            };
            rows.push(vec![
                Cell::Text(band.as_str().to_string()),
                Cell::Text((*label).to_string()),
                if counts.n > 0.0 {
                    Cell::Num(format!("{:.0}/{:.0} settled", counts.k, counts.n))
                } else {
                    Cell::Muted("Not available (no settled transitions)".to_string())
                },
                Cell::Num(mean),
            ]);
        }
    }
    let value_rows: Vec<Vec<Cell>> = SizeBand::ALL
        .iter()
        .map(|band| {
            vec![
                Cell::Text(band.as_str().to_string()),
                Cell::Num(format!("${:.0}", funnel.value_of_interest(*band))),
            ]
        })
        .collect();
    let body = format!(
        "{}\n{}\n{}",
        data_table(
            Some("Funnel posteriors by size band"),
            &["Size band", "Transition", "Settled n/N", "Posterior mean"],
            &rows,
            Some(EmptyState {
                reason: "No size band exists without settled transitions; this table always has its rows because the bands are fixed.".to_string(),
                action: None,
            }),
            Some("each band's posterior is partially pooled toward the global rate by empirical-Bayes shrinkage"),
        ),
        data_table(
            Some("Value of an interested reply"),
            &["Size band", "Expected value of interest"],
            &value_rows,
            None,
            Some("P(paid | interested) times the band's lifetime value, from the funnel model's own posteriors"),
        ),
        "<p class=\"table-note\">The funnel counts only settled transitions: a stage reached counts as yes, and a stage not reached within the configured window counts as no; anything younger is excluded rather than counted as failure.</p>"
    );
    panel("Funnel posteriors", &body, None)
}

fn band_index(band: SizeBand) -> usize {
    match band {
        SizeBand::Unknown => 0,
        SizeBand::Small => 1,
        SizeBand::Core => 2,
        SizeBand::Large => 3,
    }
}

fn cadence(data: &RequestData) -> String {
    let model = &data.learned.send_time;
    let rows: Vec<Vec<Cell>> = HOURS
        .clone()
        .map(|hour| {
            let slot = usize::try_from(hour.saturating_sub(8)).unwrap_or(0);
            let (k, n) = model.counts.get(slot).copied().unwrap_or((0.0, 0.0));
            let posterior = model.slot_posterior(slot);
            vec![
                Cell::Num(format!("{hour:02}:00")),
                Cell::Num(format!("{n:.0}")),
                Cell::Num(format!("{k:.0}")),
                Cell::Num(rate(u32_to_usize(k), u32_to_usize(n))),
                Cell::Num(percent(posterior.0 / (posterior.0 + posterior.1))),
            ]
        })
        .collect();
    let with_data: Vec<(u32, f64)> = HOURS
        .clone()
        .filter_map(|hour| {
            let slot = usize::try_from(hour.saturating_sub(8)).ok()?;
            let (_, n) = model.counts.get(slot).copied()?;
            if n <= 0.0 {
                return None;
            }
            let (a, b) = model.slot_posterior(slot);
            Some((hour, a / (a + b)))
        })
        .collect();
    let best = if with_data.is_empty() {
        format!(
            "Not available (no settled sends yet); the runner uses the default hour {:02}:00 until data arrives.",
            model.default_hour
        )
    } else {
        let mut sorted = with_data;
        sorted.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        sorted
            .iter()
            .take(3)
            .map(|(hour, rate)| format!("{hour:02}:00 ({})", percent(*rate)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let table = data_table(
        Some("Send-time model (local business hours)"),
        &[
            "Local hour",
            "Sends n",
            "Interested k",
            "Rate n/N",
            "Posterior mean",
        ],
        &rows,
        Some(EmptyState {
            reason: "The hour range is fixed (08:00 to 18:00); rows always exist.".to_string(),
            action: None,
        }),
        Some(&format!(
            "each hour's Beta posterior is pooled toward the all-hours rate with strength {:.0}",
            model.pooling
        )),
    );
    let note = format!(
        "<p class=\"table-note\">Best local hours by posterior mean: {best}. The schedule is chosen per contact's local time; buckets outside business hours are never used.</p>"
    );
    panel("Cadence", &format!("{table}\n{note}"), None)
}

fn u32_to_usize(value: f64) -> usize {
    if value.is_finite() && value >= 0.0 {
        usize::try_from(value.round() as u64).unwrap_or(usize::MAX)
    } else {
        0
    }
}

fn holdout(data: &RequestData) -> String {
    let groups = holdout_groups(&data.snap, &data.learned, &data.config);
    match apex_sales::holdout::effect(
        groups.treated_conv,
        groups.treated_n,
        groups.holdout_conv,
        groups.holdout_n,
    ) {
        Some(effect) => {
            let rows = vec![
                vec![
                    Cell::Text("Emailed group".to_string()),
                    Cell::Num(rate(
                        u32_to_usize(effect.treated_rate * groups.treated_n),
                        u32_to_usize(groups.treated_n),
                    )),
                    Cell::Text(format!(
                        "{:.0} of {:.0} publishers reached an audit request",
                        groups.treated_conv, groups.treated_n
                    )),
                ],
                vec![
                    Cell::Text("Held-out group".to_string()),
                    Cell::Num(rate(
                        u32_to_usize(effect.holdout_rate * groups.holdout_n),
                        u32_to_usize(groups.holdout_n),
                    )),
                    Cell::Text(format!(
                        "{:.0} of {:.0} publishers reached an audit request",
                        groups.holdout_conv, groups.holdout_n
                    )),
                ],
                vec![
                    Cell::Text("Difference".to_string()),
                    Cell::Num(format!("{:+.2} points", effect.difference * 100.0)),
                    Cell::Text(format!(
                        "95% Newcombe interval {:+.2} to {:+.2} points",
                        effect.interval.low * 100.0,
                        effect.interval.high * 100.0
                    )),
                ],
                vec![
                    Cell::Text("Incremental conversions".to_string()),
                    Cell::Num(format!("{:.1}", effect.incremental_conversions)),
                    Cell::Text(
                        "extra audits attributable to outreach, scaled to the emailed group"
                            .to_string(),
                    ),
                ],
            ];
            let body = format!(
                "{}\n{}",
                data_table(
                    Some("Outreach effect (randomized holdout)"),
                    &["Group", "Rate", "Basis"],
                    &rows,
                    None,
                    Some("publisher-level audit requests; the interval is Newcombe's hybrid score interval of the difference"),
                ),
                "<p class=\"table-note\">The holdout is assigned by a salted hash of the publisher key; changing APEX_SALES_HOLDOUT_SALT re-randomizes everyone. Attribution by “who replied” overstates the effect, which is why the held-out group exists.</p>"
            );
            panel("Holdout effect", &body, None)
        }
        None => {
            let note = format!(
                "<p class=\"table-note\">The current holdout share is {}; held-out publishers are never emailed, so their conversions measure what would have happened anyway.</p>",
                percent(data.config.holdout.share)
            );
            let notice_html = notice(
                    StatusKind::Unavailable,
                    "Holdout effect",
                    &format!(
                        "Not yet estimable ({} emailed-group and {} held-out publishers; the difference needs both groups non-empty).",
                        fmt_count(groups.treated_n),
                        fmt_count(groups.holdout_n)
                    ),
                    None,
                );
            panel("Holdout effect", &format!("{notice_html}\n{note}"), None)
        }
    }
}

fn fmt_count(value: f64) -> String {
    if (value - value.round()).abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_sales::model::DAY;

    const NOW: i64 = 20_745 * DAY + 10 * 3600;

    fn empty(dir: &std::path::Path) -> RequestData {
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        crate::data::load_from(&env, NOW).unwrap()
    }

    #[test]
    fn an_empty_store_says_not_available_with_reasons() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-mod-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = empty(&dir);
        let meta = PageMeta::new(
            "Models",
            "Models",
            Vec::new(),
            NOW,
            dir.display().to_string(),
            "",
        );
        let html = render(&meta, &data);
        assert!(html.contains("Not available (no settled first-touch predictions yet)"));
        assert!(html.contains("Not yet estimable"));
        assert!(html.contains("every arm shows its prior"));
        assert_eq!(html.matches("<h1").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settled_history_produces_calibration_and_arms_numbers() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-mod2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = apex_sales::store::StateDir::open(&dir).unwrap();
        let mut publisher = apex_sales::model::Publisher::new("studio.example", "Studio");
        publisher.domain = Some("studio.example".into());
        store.put_publisher(&publisher).unwrap();
        store
            .put_contact(&apex_sales::model::Contact {
                email: "maria@studio.example".into(),
                name: None,
                role: None,
                publisher_key: "studio.example".into(),
                kind: apex_sales::model::ContactKind::Named,
                source: "seed list".into(),
                consent: false,
            })
            .unwrap();
        store
            .put_event(&apex_sales::store::Event::Touch {
                at: NOW - 30 * DAY,
                email: "maria@studio.example".into(),
                publisher: "studio.example".into(),
                step: 0,
                arm: "t0-finding".into(),
                message_id: "m".into(),
                local_hour: 10,
                predicted: Some(0.05),
            })
            .unwrap();
        store
            .put_event(&apex_sales::store::Event::Outcome {
                at: NOW - 29 * DAY,
                email: Some("maria@studio.example".into()),
                publisher: None,
                outcome: apex_sales::model::Outcome::Reply(apex_sales::model::ReplyClass::Positive),
                note: None,
            })
            .unwrap();
        let data = empty(&dir);
        let meta = PageMeta::new(
            "Models",
            "Models",
            Vec::new(),
            NOW,
            dir.display().to_string(),
            "",
        );
        let html = render(&meta, &data);
        assert!(html.contains("Brier score"));
        assert!(html.contains("Reliability bins"));
        assert!(html.contains("Perfect calibration (mean prediction)"));
        assert!(html.contains("Step 1 message arms"));
        assert!(!html.contains("no settled first-touch predictions yet"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
