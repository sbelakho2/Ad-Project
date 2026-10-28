# A3 validation revision 2: applied corrections

Source: `apex-validation/ApexMediation_Validation.pdf` (32 pages, revision 2,
reviewed commit `685742e`). This records what the engine now does and what it
refuses to do without evidence.

## Inference

- The per-player Horvitz-Thompson estimator uses the design-aware Cauchy
  variance bound `[sqrt(p*S_t/X_t^2) + sqrt(q*S_c/X_c^2)]^2` instead of the
  fixed-population sum that omitted assignment covariance
  (`apex-lift/src/stats.rs`, estimator id `planned_share_total_v2`).
- Initial keeps use an explicit registered one-sided bound: interim 0.5%,
  planned final 2.25%, extension final 2.25%, for a 5% family budget. The
  exact-binomial oracle (25% and 50% splits, 20,000 eligible users) and the
  native constant-revenue A/A witness are tests; the old formula is kept as a
  negative control.
- Small information is a no-keep state, not a reason to relax the bound.

## Holdout eligibility and billing

- The monthly estimator takes an **eligible assignment roster**
  (`AssignedRoster`) and computes revenue per assigned user, so eligible zeros
  are in the denominator. Assignment counts drive the 90/10 SRM chi-square;
  observed ad-user proportions are not a substitute. The 100,000-user
  zero-gain selection witness (and its -4% variant) produces no positive bound
  and no fee (`holdout.rs` tests).
- Fees run on **matched exposed revenue for the billed days**, summed by day
  from the signed daily evidence and restricted to the billable range
  (`range.first..=range.last`), never on observed-player share times scope
  revenue. A term-ending or overridden month does not price the days after its
  stop. Direct `monthly_fee_on` and `run_holdout_month` call the same
  single-round function, so they agree to the micro.
- A fee additionally requires a bounded-outcome certificate
  (`certificate.rs`): fixed envelopes and priors, augmented IPW contrast,
  deterministic and empirical-Bernstein radii union-budgeted, a baseline upper
  bound and the absolute lower gain `R_obs - N*U0`. A fee requires both the
  population `delta_lower` and the absolute `actual_gain_lower` above zero, and
  is at most 30% of the absolute lower gain. Multiple cohorts split a family
  error budget (`0.05/(2K)` per certificate). Missing outcomes are imputed at
  the conservative limit (zero for exposed, upper envelope for control) and
  contribute nothing to the observed total. The certificate cohort must match
  the eligible roster, and no certificate or roster means no fee with a typed
  reason.

## Decision integrity

- Settlement no longer swallows per-player feed errors or empty feeds on a
  platform that registers one: the decision voids instead of silently using
  daily ratios (`supports_player_feed`). It also voids when the registered
  estimator itself fails to produce a reading. A platform that documents no
  player feed keeps the daily path.
- CUPED accepts signed residuals; only non-finite values are refused. The
  coefficient is fitted only when the roster is fully observed.
- Switchback: the admissible 14-block assignment law is enumerated (exactly 30
  schedules, minimum exact one-sided p = 1/30) and the decision uses the
  registered 97.75% bound.

## Capability gates that remain closed

- Live MAX and LevelPlay adapters do not yet persist a complete eligible
  roster, per-user envelopes or an A/A pre-period, so they cannot certify an
  A3 results fee. They keep the existing scope, coverage and window gates and
  bill nothing. Enabling results billing requires authoritative roster and
  envelope evidence, not a relaxed gate.
- Harm checks still use the daily clustered ratios; the PDF's clustered harm
  recommendation remains an open statistical extension.

## Rerun

`apex-tools claims`, the Rust/Python twin and the repository unit tests cover
the corrected estimator. The external `apex-validation` kit remains the
independent harness; its revision-2 finite certificate is the reference for
the native `certificate.rs` implementation.
