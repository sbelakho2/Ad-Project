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

- The monthly estimator prices the two arm revenue totals. With an eligible
  assignment roster (`AssignedRoster`) it computes revenue per assigned user,
  so eligible zeros stay in the denominator; without one it prices the
  planned-share totals, where the registered 90/10 split supplies the
  denominators. With a roster the assignment counts drive the 90/10
  chi-square; without one the chi-square over labelled rows is a feed-shape
  diagnostic, not a proof of the original assignment law. The 100,000-user
  zero-gain selection witness (and its -4% variant) produces no positive bound
  and no fee (`holdout.rs` tests).
- Revenue with no player id is kept **by arm** and enters that arm's total
  (`UnidentifiedRevenue`). Differential identity loss then leaves a null
  reading at a null: one lost control unit in a ten-unit control total reads
  as zero lift rather than an eleven percent gain. The signed evidence carries
  the per-arm unidentified revenue. The identity share is a signed diagnostic
  only.
- Fees run on **matched exposed revenue for the billed days**, summed by day
  from the signed daily evidence and restricted to the billable range
  (`range.first..=range.last`), never on observed-player share times scope
  revenue. A term-ending or overridden month does not price the days after its
  stop. Direct `monthly_fee_on` and `run_holdout_month` call the same
  single-round function, so they agree to the micro.
- An eligible-user roster and a bounded-outcome certificate, when the platform
  supplies them, are signed audit records: fixed envelopes and priors,
  augmented IPW contrast, deterministic and empirical-Bernstein radii
  union-budgeted, a baseline upper bound and the absolute lower gain
  `R_obs - N*U0`. Multiple cohorts split a family error budget (`0.05/(2K)`
  per certificate). The certificate cohort must match the eligible roster when
  a roster exists. Neither the roster nor the certificate gates or caps the
  fee; the fee is exactly the bound-derived lift fee that the invoice verifier
  re-derives from `lower_alpha`.

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

- Live MAX and LevelPlay adapters persist the per-player feed (MAX 45 days,
  LevelPlay 14 days merged into a durable spool). They do not supply an
  eligible roster, per-user envelopes or an A/A pre-period, so the monthly fee
  runs on the planned-share totals and keeps a complete 28-day window and a
  positive registered bound. Roster and envelope evidence are optional audit
  context, not a precondition for the fee.
- Harm checks still use the daily clustered ratios; the PDF's clustered harm
  recommendation remains an open statistical extension.

## Rerun

`apex-tools claims`, the Rust/Python twin and the repository unit tests cover
the corrected estimator. The external `apex-validation` kit remains the
independent harness; its revision-2 finite certificate is the reference for
the native `certificate.rs` implementation.
