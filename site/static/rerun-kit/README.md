# Apex Lift Engine re-run kit

Prove the fee yourself from your own dashboard export, with or without a Rust
toolchain.

Export the same report from your MAX or LevelPlay dashboard and shape it as
`day,arm,revenue,users` (see `apex-lift-rerun-template.csv`). `arm` is
`control` or `test`; `users` may be blank on MAX, where the allocated share is
supplied instead.

Two ways to re-run it, both giving the same lift and 90% lower bound to the
rounding:

1. **No toolchain** — `python3 rerun.py export.csv` (or `--split 0.25` when
   there is no `users` column). The script mirrors the engine exactly: the
   per-day log ratio of the two arms, the sample standard deviation, and the
   one-sided 90% Student-t lower bound, printed as JSON with every daily log
   ratio.
2. **The engine** — `apex-lift` is open source (Apache-2.0). Feed the same CSV
   to its `rerun` function; it returns the same numbers with the same daily
   logs.

The billed amount is the lower bound, never the point estimate, so a re-run
that reproduces the lower bound reproduces the fee.
