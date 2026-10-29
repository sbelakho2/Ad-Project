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

The monthly results fee is reconstructed from the `holdout.evidence` object in
the signed `monthlyBilling` record, not from the daily experiment export:

```sh
python3 holdout_rerun.py evidence.json --treated-revenue 900000000 \
    --bound-alpha 0.0123 --actual-gain-lower 0.011
```

The evidence rows are folded to one row per identified player (plus one
aggregated row for revenue with no resolvable id), so the signed record stays
ledger-sized; `windowDays` carries the distinct measurement days and the
script refuses to derive a fee from evidence covering fewer than 28 days. The script resolves player ids in
the documented order, folds repeated
player-days, applies CUPED with the estimated `cov(pre,post)/var(pre)`
coefficient, computes the ratio delta-method standard error, and prints the
conservative 90% lower bound. The fee is 30% of the treated revenue times
`1 - 1/(1 + bound)`; a non-positive bound bills nothing. It refuses
fewer than 30 unique players in either arm and reports the unidentified revenue
share so the identity-coverage gate is visible.

Two optional flags override the values the enclosing record already carries:

- `--bound-alpha LOWER_ALPHA` — the billing bound. Without it the script
  defaults to the line's own `lowerAlpha` — the registered bound production
  bills from — so a plain run reproduces the invoiced fee exactly (OO).
- `--actual-gain-lower GAIN_LOWER` — the certificate cap. Without it the
  script defaults to the line's own `actualGainLower`; the billed lift is
  `min(bound, actual_gain_lower)`, never above the gain the bounded-outcome
  certificate actually proved.

Feed the script the whole signed `monthlyBilling` record, its `holdout` line,
or the bare `evidence` object. With a bare evidence object neither default
exists (the line carries both values), so the flags are the only way to pin
the billed lift — the output echoes the effective values (`boundAlpha`,
`actualGainLower`, null when absent) next to `billingLift`, the lift the fee
was computed from.
