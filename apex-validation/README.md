# ApexMediation independent validation

Reviewed production commit: **685742ea22bc40f3c78404e4095c1cbf53806bad**.
No production source was changed. This kit tests statistical validity and
documents exact proposed changes; it is not a production fix or a publisher
revenue case study. Results are synthetic except native tests on real code.

## Reproduce

Use Python 3.11+ with NumPy, SciPy and Matplotlib, and a current Rust toolchain.
Extract this folder as `apex-validation`, beside a checkout named
`apexmediation`. Clone `https://github.com/sbelakho2/Ad-Project.git` into that
adjacent folder and detach it at the exact commit above. Do not substitute
the latest branch when comparing the supplied results.

```sh
python3 -m unittest test_benchmark.py -v
python3 benchmark.py
python3 information_validation.py
python3 switchback_check.py
cargo build --manifest-path ../apexmediation/rust/Cargo.toml -p apex-lift --example apex-lift-twin
python3 native_decision_witness.py
```

`benchmark.py` runs the frozen sensitivity grid and compiles the separate
driver against the unmodified production crate. It uses Cargo from PATH,
with this environment's toolchain path as a fallback.
The driver can also be built manually with
`cargo build --release --manifest-path driver/Cargo.toml`.
The main grid took about 200 seconds and the supplementary run about
84 seconds on this environment. Runtime varies by hardware.

`plan.json` specifies all initial seeds and profiles. The supplementary
600,000-user sample was selected after the initial grid exposed low power;
it uses an independent new seed and is explicitly a conditional feasibility
study, not a preregistered first-grid result. Each trial is a fresh population.
Six effect sizes reuse its potential outcomes. Pointwise Monte Carlo
intervals are exact Clopper-Pearson intervals over trials, not over users.
No simultaneous familywise claim is made for all displayed intervals.

The main Monte Carlo code uses independently derived sufficient-statistic
formulae. Sixty generated HT cases plus three holdout cases were compared
with the unmodified native crate; all 63 agree. The repository's separate
26-case twin also passes. Passing those comparisons does not make the
shared estimator correct; the exact-null oracle exposes its error.

Only initial HT results at 25% or 50% are reachable default initial-keep
designs. HT at90% is estimator sensitivity. Only the90% rows of
`current_holdout_full_roster` correspond to the current holdout split; its
other rows are hypothetical sensitivity comparisons. The holdout simulation
supplies a full eligible roster, more evidence than the current live feed.
The separate native billing witnesses expose the real feed-selection bug.

## Real outcomes without owning an app

With the publisher's authorization, provide an eligible-user roster with one
row per predefined assigned user, including zero-revenue users. CSV fields:

```csv
user_id,revenue
example-1,0.00
example-2,0.13
```

Then run:

```sh
python3 rerandomize.py eligible-users.csv --share .25 --splits 10000 --output aa.json
python3 rerandomize.py eligible-users.csv --share .25 --splits 10000 --effect .03 --output injected.json
```

An impression-only export does not certify eligible-player coverage. Do not
silently treat it as the complete roster. Re-splitting tests null calibration
under its assignment model. An injected3% effect is a power assumption; it
does not establish real treatment lift. Provider assignment, bidder learning,
spillover and retention need authorized real publisher measurements.

## Evidence

- `results/benchmark.json`: all initial cells, rates and pointwise95% intervals.
- `results/information-validation.json`: independent larger-sample study.
- `results/production-parity.json`: native witnesses including monthly-run fee.
- `results/native-AA-decision.json`: native keep of a truly unchanged setting.
- `results/native-AA-witness.json.gz`: exact input for that decision.
- `results/switchback.json`: complete30-schedule sharp-null audit.
- `results/*tests.log`: native and Python test logs.
- `verifier/`: independent mathematical and implementation review.
- `output/pdf/ApexMediation_Validation.pdf`: findings, changes and research.

The current implementation is not statistically cleared for arbitrary apps.
In particular, heavy-tail90/10 designs fail nominal coverage even after a
simple normal-bound repair. No real publisher account was accessed; no
actual invoice was sent; no generated traffic was sent to an ad network.
