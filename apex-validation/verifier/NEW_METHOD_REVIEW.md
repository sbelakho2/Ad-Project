# Separate verification of the new validation kit

Reviewed starting commit `32028f79981996bef5b88bee5942a9ab9c437234` in
`apex-validation`, against production commit
`685742ea22bc40f3c78404e4095c1cbf53806bad`. No production code was changed by
the verifier. The later working-tree additions allowing an explicit `shares`
grid and calling `run_holdout_month` in the native driver were also inspected;
they do not change the main statistical estimators. This review is not a
claim that those later changes were present in the starting commit.

## What was checked

- All four `test_benchmark.py` unit checks passed independently.
- Re-executed all 63 generated native parity fixtures through the driver
  linked to the unmodified production `apex-lift` crate. All checked fields
  matched, including the three feed-selection witnesses.
- Expanded the signed CUPED moment identity to 100 randomized cases and
  compared with direct per-user adjusted values. Largest absolute error was
  `8.74e-11`; signed residuals were allowed.
- Independently enumerated the production switchback mask/filter design.
  It has the same 30 schedules as `switchback_check.py`.
- On 100 fixed outcome paths, enumerating all 30 possible assignments gave
  at most 1/30 rejections at alpha=.05 and none at alpha=.025.
- Numerically checked the full-potential-outcome Cauchy variance inequality
  on 1,000 independently generated pairs of potential-outcome populations.
- Inspected the completed main `results/benchmark.json`, the frozen grid,
  random-number generation, the sequential model and the held-out auction
  calculation. The main simulation was not rerun by this verifier.

Detailed independent numerical checks are in
`new_method_focused_checks.json`.

## Mathematical assessment

The current-HT algebra matches production. The proposed Cauchy bound is the
appropriate conservative **first-order** randomization-variance construction
for the planned-split HT log ratio: with observed arm totals X_t/X_c,

    A_t = (1−q) Σ_t Y_i² / X_t²
    A_c = q Σ_c Y_i² / X_c²
    variance_bound = (sqrt(A_t) + sqrt(A_c))².

Its empirical plug-in version combined with a normal critical value does
not become an exact finite-sample confidence bound. The benchmark correctly
states this limitation, and its heavy-tail results show why it matters.

The CUPED method uses a coefficient learned from an independent historical
sample, a pooled pre-treatment center and one outcome per eligible user.
The algebra for the adjusted sum of squares is correct. Including every
predefined eligible user, even with zero outcome, is essential. The finite
roster's centered covariate has mean zero, so constant proportional effects
retain the stated finite-population ratio target. This does not establish
coverage under arbitrary provider assignment, missing eligibility, history
drift or interference.

Sequential budgets `.005 + .0225 + .0225 = .05` are arithmetically correct.
The union-bound familywise argument does not require independent looks,
provided each constituent test is valid at its allocated level. The Gaussian
increment simulation isolates repeated looks; it does not repair invalid
heavy-tail p-values. The fixed-final Monte Carlo uses alpha=.025, whereas
the three-look schedule uses .0225 at its last two looks. Their power
estimates should not be presented as identical policies.

The exact switchback p-value correctly enumerates the uniform admissible
assignment distribution under a sharp no-effect null and no carryover.
Arbitrary fixed trends or serial correlation do not invalidate that
randomization argument. With only 30 schedules, its smallest nonrandomized
one-sided p-value is 1/30, so alpha=.025 has zero power. An exact null test is
not itself a valid billing confidence interval under heterogeneous effects.

## Main results must retain the failures

At 90/10, the proposed normal-based bounds are **not** calibrated uniformly
over the grid. In the concentrated profile, null false keeps were 5.99%
(corrected HT), 5.80% (roster unadjusted), and 5.72% (roster CUPED), against
their nominal fixed-final 2.5% test. In the extreme-tail profile these rose to
11.77%, 10.45%, and 9.52%. The current holdout, even generously supplied with
a full eligible roster, falsely bills in 10.75% of extreme-tail null cases.

These are substantive failures, not Monte Carlo noise. No all-profile
statistical-safety or universal 90/10 billing claim is supported. Refusing
underpowered or uncalibrated units is part of an honest acceptance policy;
normal-bound algebra alone is insufficient. Exact sharp-null randomization
can validate no-effect tests, but does not by itself solve heterogeneous
average-lift confidence coverage.

At 25/50, the methods perform much better, but individual empirical rates
near 2.5% need their Monte Carlo intervals. Do not mark every point estimate
below a threshold as a statistically certified pass. The provided
Clopper–Pearson intervals are per-cell intervals, not simultaneous intervals
for the entire grid.

## Reporting boundaries

- The HT comparisons are fixed-final **estimator** comparisons. Production
  initial preregistration accepts .25/.50/.10, so .90 HT rows are sensitivity
  results rather than reachable initial-keep decisions.
- The `current_holdout_full_roster` rows at .25/.50 deliberately bypass the
  production 90/10 gate; they must not be described as production invoices.
  The .90 rows are the relevant holdout-design comparisons.
- Effects share potential outcomes. Individual trial-level intervals remain
  meaningful, but differences across effects are correlated.
- Synthetic profiles are explicitly not calibrated to a real publisher.
  Great power in a stated profile is conditional feasibility, not a revenue
  forecast or proof of real bidder response.
- The supplementary larger-sample study was designed after the original
  grid exposed low power. Its independent seed is appropriate for a new
  conditional validation, but it must be labeled supplementary rather than
  represented as part of the original frozen grid.

## Holdout witness confirmed natively

The zero-true-lift selection witness has 90,000 assigned exposed users and
10,000 assigned controls. Only 36,000 exposed users with outcome 1.25 and
5,000 controls with outcome 1 appear in the impression export. Both assigned
means are .50; the feed-only method reports +25%, SE=0, and passes the
observed-user split tolerance. The native helper bills $2,700 on the actual
$45,000 exposed revenue. The native `run_holdout_month` all-month calculation
bills **$2,634.146342** on $50,000 scope revenue, confirming the expected
one-micro rounding distinction from `monthly_fee_on_scope`.

This executes the estimator and monthly calculation on an assumed usable
holdout, not a complete signed runner lifecycle or a live provider account.
Existing live scope/history refusals can block billing and must remain
distinguished from estimator correctness.

## Supplementary larger-sample result inspected

The subsequent helper additions are committed at
`58643079ca2142c95d2ee20d5ebce0f67eea1ff6`. The new-seed information study
uses 600,000 eligible players, a 50/50 split, 2,000 trials, and only the
stated concentrated synthetic profile. At +3% true lift, roster CUPED keeps
92.75% of changes (95% Monte Carlo interval 91.52–93.85%). Under the null it
keeps 3.00% (interval 2.30–3.84%). Corrected HT keeps 80.30% of +3% winners
(interval 78.49–82.02%), with null keeps 2.15% (interval 1.56–2.89%).

These results support conditional high-power feasibility with more
information. Neither null interval's upper endpoint certifies a ≤2.5% error
ceiling. CUPED's 3% point estimate is compatible with 2.5% at this Monte Carlo
precision, so it also should not be called a statistically established
miscalibration from this supplementary study alone. This study does not
resolve the original grid's severe 90/10 heavy-tail failures.
