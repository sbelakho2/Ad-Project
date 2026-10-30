# Executed checks and limits

> **HISTORICAL / version-pinned evidence.** Everything below was executed
> against commit `685742ea22bc40f3c78404e4095c1cbf53806bad`, **not** the
> current HEAD. The `apex-lift` API has since changed (`estimate`,
> `split_is_90_10`, `HoldoutMonthChange`, the fee path), the driver no
> longer compiles against current HEAD, and none of these numbers describe
> today's code. Do not cite this file as current verification.

Production SHA:685742ea22bc40f3c78404e4095c1cbf53806bad.
Source checkout was clean after the tests. No production patch was made.

Native checks, all against that unmodified commit:

| Command (from checkout root) | Outcome |
|---|---|
| `cargo test --manifest-path rust/Cargo.toml -p apex-lift --lib` |200passed|
| `cargo test --manifest-path rust/Cargo.toml -p apex-autopilot --lib` |390passed,1ignored|
| `cargo test --manifest-path rust/Cargo.toml -p apex-connectors --lib` |157passed|
| `cargo test --manifest-path rust/Cargo.toml -p apex-proof --lib` |61passed|
| `cargo test --manifest-path rust/Cargo.toml -p apex-billing --lib` |108passed,1ignored|
| `python3 -m unittest discover -s tools/lift-validation -p 'test_*.py'` |19passed|
| `python3 tools/provider-sim/max/selftest.py` |11passed|
| `python3 tools/lift-validation/twin.py --keep-going` |26cases agreed|

The five Rust suites total916passed and2ignored. These are the relevant
library suites, not the complete workspace or external real-provider tests.
Logs are supplied, including the ignored counts. No production code changed,
so there was no changed-hunk mutation or implementation-verifier gate to claim.

Standalone checks:

-4independent unit checks pass.
-63JSONL numerical fixtures call the unmodified native apex-lift crate.
-The native A/A example executes window, guards and decision, returning
 Keep and lower90=.017642726808801745 under true zero lift.
-Three native holdout fixtures execute both the estimate and
 `run_holdout_month`. The zero-true-lift fixture produces fee2634146342micros.
-The earlier independent verifier's selection report was written before
 those native monthly checks; its source-only caveat is superseded for the
 core monthly function by `results/production-parity.json`. A complete live
 platform-to-invoice workflow was still not executed.
-The benchmark code was independently reviewed, including its Cauchy
 bound, signed adjustment moments, alpha-spending assumptions and schedule
 enumeration. See verifier/NEW_METHOD_REVIEW.md.

Monte Carlo grid:120000independent cohorts, with multiple effect/method
evaluations reusing each cohort. Supplemental sample:2000fresh600000-user
cohorts. Sequential study:200000null trials. Switchback:6cells of100000
trials each. Auction test:1000000held-out auctions reused across3hypothetical
models; training uses200000separate auctions. Exact binomial calculations
are also supplied; independent verifier simulated1000000draws per share.

Software:Python3.12.14,NumPy2.3.5,SciPy1.17.0,Matplotlib3.10.8,
ReportLab4.4.9,Rust1.99.1,Cargo1.99.1. All intervals described in the
report's tables are pointwise Monte Carlo intervals unless explicitly stated.
They measure simulation error, not transport to real apps.

The PDF was rendered and visually checked across all13pages; selected dense
pages were inspected at full size, and no text boundary overflow was found.
No simulated revenue was represented as actual publisher revenue. No
publisher credentials were used, no app was purchased and no real invoice
or ad-network traffic was generated.
