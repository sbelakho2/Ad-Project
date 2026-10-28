# Independent verification of committed Apex lift code

Verified commit: `685742ea22bc40f3c78404e4095c1cbf53806bad`.
Comparison base: `f490599659ca7f4b8952ff29206567c5e51bcc3f`.
Scope: read-only statistical and source verification, no production changes.

## 1. Critical: initial-keep HT variance is understated

`rust/crates/apex-lift/src/stats.rs:253` computes a planned-probability
Horvitz–Thompson ratio. Lines 287–291 do not estimate its randomization
variance correctly.

Let q be treatment probability, p=1−q, and X_t/X_c the observed arm revenue
totals. For independent Bernoulli assignment with fixed potential revenues,
the HT total is X_t/q. Its true variance is p/q times the **population** sum
of squared treatment potential outcomes. An unbiased observed-data estimate
is p/q² times the **observed treatment** sum of squares. Code uses p/q
times the observed sum, missing a further inverse-probability factor. The
two HT totals also have covariance −Σ_i Y_i(1)Y_i(0); subtracting twice this
covariance in the log-ratio delta variance increases the variance. Code
omits this term entirely.

A conservative first-order bound from observed data uses

    A_t = p Σ_t y_i² / X_t²
    A_c = q Σ_c y_i² / X_c²
    V_bound = (sqrt(A_t) + sqrt(A_c))².

This is a plug-in asymptotic variance bound, not an exact finite-sample
confidence guarantee. A full eligible-user roster permits the usual
randomized arm-mean estimator with a user-clustered variance instead.

Exact null counterexample: N=20,000, every player's revenue is one in
either arm. Then N_t~Binomial(N,q), and the implemented log ratio is
log[(N_t/q)/(N_c/p)]. Near expected counts, code SE=1/sqrt(N), while true
first-order randomization SE=1/sqrt(Npq).

| Split q | Exact false positive, nominal 90% one-look bound | After ≥1% point lift and SRM gate |
|---|---:|---:|
| .25 | 29.184441% | 26.934115% |
| .50 | 25.996101% | 24.144878% |

The right column uses `split_chi_square <= 10.8276` and the production
`min_point_lift=.01`. One million independent binomial replicates per split
confirm the exact binomial sums. See `verify_ht.py` and
`verify_ht_results.json`. These are one-final-look operating characteristics,
not a complete runner lifecycle replay. With constant per-user daily ARPDAU,
daily harm checks do not fire; the per-player decision path skips the daily
bootstrap. This witness requires no assumptions about an ad auction model.

The new Python reference repeats the same variance (`tools/lift-validation/
apex_lift_reference.py:273–279`), so twin agreement cannot detect the error.

## 2. Critical: ongoing holdout averages select on observed impressions

`holdout.rs:401–430` creates players only from ids in the impression feed;
`holdout.rs:300–365` averages over those players. Users generating no
impressions across the whole window are not in the denominator. If a
configuration changes the probability of any monetization, this is a
post-treatment selected population and does not measure revenue per assigned
player.

Deterministic zero-true-lift witness: among every 100 eligible players,
40 have potential revenues (Y0,Y1)=(1,1.25), 10 have (1,0), and 50 have
(0,0). Thus both potential means are .50. A balanced realized 90/10 split
of 100,000 users gives 90,000 exposed users and 10,000 controls, but only
36,000 exposed users and 5,000 controls appear in the impression export.

The current estimator reports +25% lift with standard error zero. Observed
exposure share is 36,000/41,000=87.804878%, within the code's 90%±3 percentage
point tolerance (`holdout.rs:192–203`). Both arms exceed 30 users. Revenue
identity coverage is 100%. Spread those observed users across all 28 days
and set a matching scope: completeness, identity, split, sample-size and harm
gates all permit a fee. A valid active confirmed holdout and ledger are
assumed for this calculation; the full runner was source checked, not run.

The **shipped Python publisher rerun**, actually executed on the complete
41,000-row fixture, reports a $2,700 fee when passed the actual exposed
revenue of $45,000. True aggregate gain is zero. See
`verify_holdout_selection.py`, `holdout_zero_true_lift.json`, and
`verify_holdout_selection_results.json`.

The monthly Rust run uses a different revenue base: `run.rs:149–159` obtains
the observed player fraction; lines 422–425 multiply total scope revenue by
that fraction. For $50,000 total scope revenue, the fee base is
$43,902.439024, and the source-derived all-month fee is **$2,634.146342**.
That full-month formula was computed independently but not executed in Rust.
Do not call $2,700 the exact full-run invoice.

Counterexample variant with Y1=1.20 for the 40% group is truly harmful
(−4% assigned-user revenue), but appears +20% in the impression-only means.

Fix: explicitly define eligibility before treatment, include eligible users
with zero revenue, and cluster by those users. If no eligible roster is
available, use a justified planned-split total-revenue estimator with correct
variance, rather than positive-impression-user means. Use only assignment
rosters for SRM. Revenue coverage alone does not establish user coverage.

## 3. CUPED is not currently available in the live adapters

Both `live_max.rs:431` and `live_levelplay.rs:608` return
`pretest: BTreeMap::new()`. They create a fresh in-memory `PlayerFeed` for
each call. Therefore live operation currently cannot realize claimed CUPED
power, despite unit-test and twin support.

`player_store.rs:232–250` can legitimately generate negative CUPED-adjusted
outcomes, but `stats.rs:264–267` rejects all negative inputs. Example pre
[0,1,2,3] and post [0,0,0,100] gives theta=30 and adjusted values
[45,15,−15,55]. `estimate_window` then returns None, while
`settle.rs:399–422` falls back to the daily estimator. Invalid precise
evidence must pause/refuse; it must not silently select a less conservative
statistic. Similarly, `experiment_players(...).await.ok()` at
`settle.rs:1030–1032` erases unsupported-feed, schema and transport failures
into the same no-evidence fallback.

## 4. Earlier defects are partly fixed but the old paths remain reachable

- Repeated player-days are folded to users (`player_window`).
- Users are aligned to retained analysis days (`settle.rs:310–314`).
- Calendar burn-in is dropped by the rerun kit.
- Harm is only checked at day 7/14 for confirmed harm, but those estimates
  still come from daily ratios (`guardrails.rs:128–143`), independently of
  whether a player estimate was supplied.
- Without unique-player evidence, `guardrails.rs:202–208` still pools daily
  counts and treats them as an ordinary binomial SRM denominator.

## 5. Current live fees may be unavailable rather than statistically safe

`live_levelplay.rs:564–608` starts a fresh feed and retains only 14 days;
ongoing billing requires 28 complete days. No persistent historical player
store is wired into that method. Also MAX returns measurement scope
`application:<package>` (`live_max.rs:435`) and LevelPlay returns
`application:unknown` (`live_levelplay.rs:609`), while
`runner.rs:1806–1818` requires the registered causal scope. These gates can
correctly produce no fee on current live integrations. They do not repair
the holdout estimator; once scope/history capability is added, the selection
counterexample becomes billable unless the denominator is fixed too.

## Verification limits

No production files were changed. This is independent mathematical/source
verification of committed code plus exact binomial calculation and execution
of the repository's Python holdout rerun. No Rust compiler was on the agent's
PATH, and no native Rust unit, mutation or complete live/runner suite was
executed by this verifier. Therefore this document is not a claim that the
AGENTS.md implementation finish gates passed. No real publisher account was
accessed, and no real-world lift claim follows from the synthetic witnesses.
