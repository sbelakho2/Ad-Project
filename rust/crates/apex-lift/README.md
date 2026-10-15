# apex-lift

The Apex Lift Engine, in the open. It is the exact analysis code that decides
whether a platform-native A/B change is kept, rolled back or voided, and the
fee that follows from a kept change.

The engine is deliberately small and dependency-light (`serde`, `sha2`) so a
publisher, an auditor or a regulator can build it and re-run a decision from
the same inputs:

- Estimation (`stats`): the mean daily log ratio of revenue per user, the
  sample standard deviation, and the one-sided 90% (and 99%) Student-t bounds.
  The billed number is the lower bound, never the point estimate.
- Window (`window`): burn-in, whole-week analysis windows, finality and
  the incident runs that void a test.
- Guardrails (`guardrails`): fast harm, confirmed harm, fill, split
  mismatch, reporting gaps, publisher edits and platform incidents.
- Decision (`decision`): the six keep conditions, the eight guardrails
  and the pre-registration that pins the test before it starts.
- Proof (`records`, `emit`): the signed, hash-chained records a decision
  leaves behind.
- Billing (`billing`, `run`): pro-rated monthly fees on the proven lift.
- Re-run kit (`rerun`): the same estimate from a dashboard CSV, so the
  fee can be reproduced without trusting a dashboard.

## Re-run a decision

```rust
let rows = apex_lift::rerun::parse_csv(csv)?;
let report = apex_lift::rerun::rerun(&rows, 7, test_share)?;
assert_eq!(report.days, 7);
```

`site/static/rerun-kit/rerun.py` implements the same estimator in Python for
callers without a Rust toolchain.

## License

Apache-2.0.
