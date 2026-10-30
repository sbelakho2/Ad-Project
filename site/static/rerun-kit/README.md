# Apex Lift Engine re-run kit

Prove the fee yourself from your own dashboard export, with or without a Rust
toolchain.

Export the same report from your MAX or LevelPlay dashboard and shape it as
`day,arm,revenue,users` (see `apex-lift-rerun-template.csv`). `arm` is
`control` or `test`; `users` may be blank on MAX, where the allocated share is
supplied instead (the template includes the three burn-in days the engine
drops).

Two ways to re-run it, both giving the same lift and 90% lower bound to the
rounding:

1. **No toolchain** — `python3 rerun.py export.csv` (or `--split 0.25` when
   there is no `users` column; `--burn-in 0` when the export already starts
   after burn-in). The script mirrors the engine exactly: the
   per-day log ratio of the two arms, the sample standard deviation, and the
   one-sided 90% Student-t lower bound, printed as JSON with every daily log
   ratio.
2. **The engine** — `apex-lift` is open source (Apache-2.0). Feed the same CSV
   to its `rerun` function; it returns the same numbers with the same daily
   logs.

The billed amount is the lower bound, never the point estimate, so a re-run
that reproduces the lower bound reproduces the fee.

## Monthly holdout fee (A3)

The monthly results fee is reconstructed from the signed `monthlyBilling`
record — its `holdout.evidence` rows plus the record's own billed numbers —
not from the daily experiment export:

```sh
python3 holdout_rerun.py monthlyBilling.json
```

The fee is production's exact expression, on the record's own numbers:

```
fee = round(0.30 * treatedRevenue * (1 - 1/(1 + max(lowerAlpha, 0))))
```

- `treatedRevenue` is read from the record (`treatedRevenue`, then the
  holdout line's `revenueMicros`) and must be a real positive number of
  micros. The script refuses to invent a fee from a missing or placeholder
  revenue; `--treated-revenue MICROS` overrides it explicitly.
- `lowerAlpha` is recomputed from the signed evidence and compared against
  the signed value; a disagreement refuses the fee. The signed bound is what
  the fee is then sized from, so the kit proves the number rather than
  replaying it. `--bound-alpha LOWER_ALPHA` supplies the signed value
  explicitly and is verified the same way. A non-positive bound bills zero;
  there is no second cap: a dimensionless bound and an absolute certificate
  gain are never `min`-ed together.

Before any fee the evidence gates apply, and each one refuses instead of
relaxing:

- exactly **28 contiguous** measurement days (`windowDays`, or the distinct
  days the rows carry) — a 27-day or gapped window produces no fee;
- at least 30 unique players in either arm, and no player in both arms;
- revenue with no resolvable player id is kept **by arm** and priced in that
  arm's total, as production does. Its share is reported as a diagnostic and
  is not a fee gate;
- when the evidence carries the eligible assignment roster (`assigned`),
  its **assignment counts are the denominators** of the per-eligible-user
  means — zero-revenue assigned users the impression feed never shows
  still count — and the assignment-count (SRM) chi-square must pass at
  p = 0.001. Without a roster the script prices the **planned-share totals**
  with the registered 90/10 denominators and the design-aware Cauchy bound;
  the chi-square over labelled rows is then a feed-shape diagnostic, not a
  proof of the original assignment law.

The script resolves player ids in the documented order, folds repeated
player-days, applies CUPED with the estimated `cov(pre,post)/var(pre)`
coefficient (only on a fully observed roster), computes the ratio
delta-method standard error, and prints the recomputed lift, bounds and
the billed `billingLift` side by side so you can see the bound the fee
came from.

Feed the script the whole signed `monthlyBilling` record, its `holdout`
line, or the bare `evidence` object. With a bare evidence object no
`lowerAlpha` or treated revenue exists, so the flags are the only way to pin
the billed lift. The output names the estimator, echoes the effective values
(`boundAlpha`, `treatedRevenueMicros`) next to `billingLift`, reports whether
the signed bound matched the independent recomputation
(`lowerAlphaVerified`), and keeps the per-arm unidentified revenue so the
arm totals can be checked by hand.
