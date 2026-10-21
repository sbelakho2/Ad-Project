# Woodpecker CI: the gate stack

Each `*.yml` file in this directory is one Woodpecker pipeline: six lanes
plus the aggregate verdict in `coverage-gate.yml` (eight configs). Trigger:
`push` and `pull_request` on `main` and `master`.

Each lane carries a `when` block with an `event` list and, for the lanes that
watch inputs, a `path` list. A change wakes only the lanes whose inputs it
touched. The `path` entries inside one `when` item are OR-ed, and a `when` item
with no `path` entry is unrestricted, so `manual` and `tag` runs still reach
the live lane. `coverage-gate.yml` keeps no `path` entry because it aggregates
every lane.

| Lane | Paths that wake it |
| --- | --- |
| `apex-rs-coverage.yml` | `rust/**` |
| `dogfood-audit.yml` | `rust/**`, `tools/coverage/**`, `tools/dogfood/**`, `coverage-manifest.json`, `Makefile` |
| `dogfood-live.yml` | `rust/**`, `tools/dogfood/**`, `infrastructure/clickhouse/**`, `.woodpecker/dogfood-live.yml`, plus `manual` and `tag` events |
| `contract-coverage.yml` | `monitoring/**`, `infrastructure/**`, `tools/coverage/**`, `.woodpecker/**`, `Makefile` |
| `site-perf.yml` | `site/**`, `.woodpecker/site-perf.yml` |
| `ui-ux.yml` | `rust/crates/apex-api/**`, `rust/crates/apex-control/**`, `rust/crates/apex-sales-web/**`, `rust/crates/apex-ui/**`, `rust/crates/apex-metrics/**`, `rust/Cargo.toml`, `rust/Cargo.lock` |
| `changed.yml` | pull requests only; it computes the diff itself and runs tests and clippy only for the affected crates |
| `coverage-gate.yml` | none; it always runs |

Lanes:

- `apex-rs-coverage.yml`: one matrix entry per Rust crate for the build
  domain (tests, clippy, fmt). Coverage comes from one workspace-instrumented
  `cargo llvm-cov --workspace` run in the `apex-core` entry; the 18 floors are
  gated directly against that merged profile, with `lcov-gate --crate`
  filtering each domain's records.
- `dogfood-audit.yml`: manifest, walker parity, orphan ceiling, floor ratchet
  and sync, assertion presence, stub ban, policy bans, mutation smoke and
  floor, docs links, prose ratchet.
- `dogfood-live.yml`: the live half of the adversarial dogfooding system: the
  three services booted against a real ClickHouse `service:` container, the
  live-gated `apex-analytics`/`apex-api` suites finally executing, then
  `tools/dogfood/live.py` running every required probe cell from
  `tools/dogfood/surfaces.json` and the ratchet baseline. The static catalog
  gate (`apex-tools dogfood-plan`) runs here too, so the lane is not
  redundant in a store-less build.
- `contract-coverage.yml`: monitoring rules and their tests, the edge config
  (validate plus `infrastructure/caddy/contract.sh`), CI reference integrity,
  pipeline paths, content guard, docs references, stylesheet lint.
- `site-perf.yml`: Zola build, claims contract, byte budgets, structural
  checks on the built output.
- `ui-ux.yml`: the rendered publisher console pages and the rendered
  operator control-plane pages checked by the structural accessibility
  audit.
- `coverage-gate.yml`: the aggregate verdict. It depends on every lane and
  replays the manifest, orphan, ratchet, sync, walker, mutation and pipeline
  inventory invariants before reporting.

## Relevant dogfood

`dogfood-live.yml` splits its work in two. On a `push` or `pull_request` that
touches the Rust tree, the dogfood catalog or the ClickHouse infra, and on
every `manual` or `tag` run, the full probe-cell run executes
(`dogfood-plan`, `live-rust-suites`, `run-harness`). A `dogfood subset` step
then runs on every wake of the lane. It computes the changed paths with
`git diff --name-only "$CI_COMMIT_BEFORE_SHA" "$CI_COMMIT_SHA"` (falling back
to the last commit) and passes them to
`python3 tools/dogfood/live.py --changed`, so only the probe cells whose
surfaces those paths touched run. The step prints the number of changed paths
and the harness prints the number of cells it selected. The step is report
only and exits 0, so the full `run-harness` step stays the blocking gate on the
paths that wake it.

A manual run of the lane, or a tag run, always runs the full set. Two ways to
trigger it: the Woodpecker user interface, or `manual` and `tag` in the lane's
`when` block already listed above.

`tools/ci/local.sh --changed <path>` applies the same filters on a
workstation. A lane or step runs when any changed path matches its `path`
list, and a lane or step with no `path` list always runs. `--list --changed
<path>` prints the lanes and steps that would run and executes nothing.

Floors are single-sourced from `tools/coverage/coverage-floor.json` (ratchet
only) and mirrored to `tools/coverage/floors.txt` by `apex-tools sync-floors`;
the dogfood audit fails when they drift or when a code domain has no floor.

The Rust floors are workspace-instrumented: they were calibrated from one
llvm-cov run whose profile contains every crate's tests, and each domain is
the subset of the merged lcov under `crates/<domain>/`. A per-crate
`cargo llvm-cov -p <crate>` run is not comparable: it misses the hits that
other crates' tests make into the crate and lands below floors the workspace
run clears. The lane therefore gates the merged profile, never a standalone
per-crate llvm-cov run; `apex-tools lcov-gate --crate` filters the merged
records to that domain before applying the floor.

The local equivalent of the audit family is `make dogfood`. Every gate is a
subcommand of the `apex-tools` binary, and the pipeline inventory check fails
on both directions: unknown subcommands and subcommands with no caller.
