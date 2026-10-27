# Lift validation kit

A Python reference for the `apex-lift` engine plus a twin harness that feeds
one dataset to both implementations and fails on any disagreement.

## What the reference mirrors

- `estimate`: the daily log-ratio estimator, sample standard deviation and
  one-sided 90/99% Student-t bounds.
- `horvitz_thompson_estimate`: the per-player HT log ratio with split
  probabilities and delta-method variance.
- `player_window`: per-player folding, cross-arm refusal, CUPED theta over
  paired pre/post players and the adjusted arm values.
- `build_window`: calendar-position burn-in, two-pull finality within 0.5%,
  whole-week analysis, incident runs and per-arm values (users on LevelPlay,
  allocated share on MAX).
- `evaluate_guardrails`: fast and confirmed harm, fill stops, unique-player
  split chi-square, reporting gaps, publisher edits and platform incidents.
- `decide` / `decide_with_estimate`: the six keep conditions, the interim 99%
  look, the pre-registered extension, the bootstrap cross-check and
  keep/rollback/void/pending.
- `bootstrap_lower_bound`: the engine's SplitMix64 block bootstrap, replayed
  draw for draw.
- `switchback_plan` / `switchback_out`: the weekday-balanced 14-block plan and
  the block analysis that uses `plan.blocks` to split one observed series into
  test and control arms.
- `parse_csv` / `rerun`: the publisher re-run kit, including the three
  calendar burn-in days.

The incomplete-beta continued fraction uses the Rust term
`-(a + m) * (a + b + m) * x / ...`; the old Python kit used `(b + m)` and could
drift from the engine's t bound.

## Run the twin

```sh
python3 -m unittest discover -s tools/lift-validation -p 'test_*.py'
python3 tools/lift-validation/twin.py
python3 tools/lift-validation/apex_lift_reference.py case.json
```

The twin builds `rust/crates/apex-lift/examples/apex-lift-twin.rs` on demand
and runs it once per case. It never builds the workspace. Cases are
deterministic and adversarial: keep/rollback/void/pending/extend, fast and
confirmed harm, fill stops, split mismatch, publisher edits, reporting gaps,
incidents, incomplete and fatal windows, MAX share values, per-player and
CUPED paths, switchback plans and block analysis, re-run burn-in and the fee
basis.

`twin.py --case <id>` runs a subset. The comparison covers the window shape and
problems, both estimates and all bounds, guardrails, verdict and lambda, split
evidence, switchback result, re-run report and fee factors. Only free-text
`reason` strings are excluded.
