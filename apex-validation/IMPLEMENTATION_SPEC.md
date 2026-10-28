# Production change specification

Target: sbelakho2/Ad-Project, commit685742ea22bc40f3c78404e4095c1cbf53806bad.
This document proposes changes. It does not claim they have been implemented.
Preserve current capability, scope and no-fee gates while making them.

## 1. Name and register the actual causal measurement

Add a versioned measurement contract to pre-registration and proof evidence:

- immutable experiment/assignment epoch and code/source hash;
- causal scope, outcome currency and exact outcome/billing dates;
- assignment law, planned probability and randomization unit;
- eligibility definition fixed before treatment, with unique assignment counts;
- `eligible_user_mean` or `planned_share_revenue_total` estimator identity;
- interval method and one-sided alpha, with the full allowed look schedule;
- completeness/finality/identity requirements and typed refusal reasons.

Do not use the generic label `revenue_per_user` for both a daily ARPDAU
ratio and a planned-share revenue-total ratio: their denominators and causal
interpretations differ. A 28-day estimate does not automatically certify an
unmeasured extra three days of a 31-day invoice.

## 2. Correct the initial planned-split HT variance

In `apex-lift/src/stats.rs::horvitz_thompson_estimate`, after summing one
whole-window raw outcome per independent user, replace the implemented
variance with this conservative, first-order observed-data construction:

```rust
// q = planned treatment probability; p = 1.0 - q.
let arm_t = p * test_squares / test_total.powi(2);
let arm_c = q * control_squares / control_total.powi(2);
let variance = (arm_t.sqrt() + arm_c.sqrt()).powi(2);
```

This includes an upper bound for the unobserved cross-potential-outcome
covariance; simply adding the two marginal terms is insufficient. It is a
plug-in asymptotic bound, not a finite-sample promise under arbitrary tails.
Update the Python reference AND add an independent exact-binomial oracle.
Keep `lower90` meaning90% if retained; add a generic explicit-alpha bound
rather than hiding a97.5% bound in a90%-named field.

Acceptance: `native_decision_witness.py` currently produces a positive
1.7643% lower bound and Keep at true zero lift. The corrected method must
refuse that positive billing/keep conclusion. Check the entire null law,
not just the one selected realization. Validate both25% and50% initial splits.

## 3. Fix the ongoing holdout population and fee basis

In `holdout.rs::from_player_feed` / `estimate`, an id in an impression file
means a person generated an impression. It does not mean all assigned people
are represented. Introduce explicit eligibility/assignment evidence. Include
eligible users with zero outcome. If a complete roster is unavailable, use a
justified planned-share TOTAL-revenue estimator with its design-aware
uncertainty, or return unavailable. Do not silently use positive-ad-user means.

Use unique assigned users for SRM. Revenue identity coverage is a different
quantity from coverage of eligible users. A fixed ±3percentage-point tolerance
does not replace an assignment-count test and can pass outcome-selection bias.

In `run.rs::run_holdout_month` / `attributed_by_change`, pass actual
scoped exposed revenue over the billable interval. Do not multiply whole
scope revenue by the observed positive-ad-user fraction. For a valid
dimensionless lower lift lambda and matched treatment revenue R, the intended
gain conversion is R*lambda/(1+lambda), followed by the30% fee. Revenue basis,
dates, scope and eligibility must all match the evidence contract.

Acceptance: zero-gain potential population40%(1,1.25),10%(1,0),50%(0,0)
cannot produce a positive gain invoice. Current native core month function
charges$2,634.146342 on the witness. Harmful variant(1,1.20) cannot bill either.
The direct and monthly reruns must reproduce exactly the same basis to micros.

## 4. Prohibit data-dependent estimator fallback

In `settle.rs::decide_window`, `player_estimate == None` after registered
player evidence must become a typed Pending/Void result. It must not silently
select `stats::estimate(daily_ratios)`. Replace the broad `.await.ok()` on
the player feed with explicit capability/transport/schema distinctions.
An intentionally registered daily-only exploratory method is separate and
cannot automatically become lift-billable.

## 5. Make CUPED evidence and mathematics consistent

Both live adapters currently return empty `pretest` maps. Persist the actual
pre-period, joining by stable user identity and assignment epoch. Treat a
known zero pre-outcome differently from an unknown identity. Include new,
nonmonetized and departed eligible users according to the registered cohort.

Learn the covariate adjustment independently or use a justified treatment-
interacted / cross-fitted analysis. Center consistently across the eligible
population. Use a signed-outcome-capable variance calculation: adjusted
outcomes can legitimately be negative. Validate nonnegative RAW monetary
inputs separately. Negative residuals must not select a different estimator.
Report realized variance reduction rather than assuming the Bing50% result.

## 6. Calibrate tails and look schedules jointly

Initial keeps: use one final look, or explicitly allocate a family budget
across interim/final/extension. The tested illustrative schedule
(.005,.0225,.0225) sums to.05. Its union-bound argument requires each
underlying test to be valid; it cannot fix a bad90/10 heavy-tail interval.

Ongoing bills and harm: predefine the longitudinal claim and error budget.
Twelve per-month intervals do not imply one95% yearly guarantee. Daily
retesting after day14 also consumes error. Use one cluster-valid method for
the keep, harm and financial estimands, rather than valid per-user keeps with
daily-ratio harm tests that retain the original correlation defect.

Replace the fixed30-user adequacy criterion with an information plan and
independent calibration. Record small-arm size, concentration, largest-user
share and covariance predictability. No universal ESS threshold was proved
in this review. Outcome-dependent adequacy gates must themselves be included
in operating-characteristic validation. Preserve No measurement/No fee when
the configured method has not been validated for the evidence regime.

## 7. Persist and scope the provider evidence

LevelPlay exposes14days; the holdout requires28 complete days. Pull daily,
reconcile D-2 restatements and persist enough stable history plus pre-period.
Do not create a fresh14-day-only store and expect to bill a28-day window.

Retain ad-unit scope in source parsing, not merely application scope. Keep
MAX's Waterfall-name semantics as an unverified capability until a real
authorized experiment establishes its mapping. Namespace identifier types
and verify stable alias resolution across pre/post windows. Missing ids,
duplicate pages, mixed apps, cross-arm users and split edits must refuse.

## 8. Use independent release oracles

Retain the existing26-case Rust/Python twin for mechanical parity. Add the
exact constant-outcome binomial law, zero-user financial witness, signed
CUPED cases, and source-to-invoice chaos replay with a simulated clock.

Switchback: enumerate the actual30 allowed schedules for sharp-null tests.
Minimum exact one-sided p=1/30; expand the assignment space ifalpha=.025 is
required. Do not claim six hours is a validated bidder washout. A sharp-null
test is not automatically an average-effect billing confidence interval.

Release operating-curve gates should use Monte Carlo confidence bounds,
with multiplicity adjustment when claiming every cell passes. Validate on
fresh seeds after tuning; compare paired methods using paired outcomes.

Any production implementation must satisfy AGENTS.md: `make changed-plan`,
diff-scoped checks, changed Rust mutation gates and the independent verifier.
This review left production source clean and does not substitute its
standalone tests for those implementation gates.
