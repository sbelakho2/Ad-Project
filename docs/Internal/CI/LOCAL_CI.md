# Kiwi CI and Local Verification

CI is Kiwi CI. The only pipeline is `.kiwi/pipeline.yaml`; `.woodpecker/` is
deleted. The pipeline has 13 jobs.

## Pull requests

`changed` and `independent-verifier` are separate Kiwi jobs. Both run from the
diff:

- `changed` runs `python3 tools/ci/changed.py --test --clippy --mutation
  --security --gates` on the reverse-dependency closure of the changed crates
  and the relevant static gates.
- `independent-verifier` runs in its own job with
  `CARGO_TARGET_DIR=target/verifier` and the same diff gate, without a
  `needs:` edge, so the implementer's job cannot suppress it.

`tools/ci/changed.py` maps paths to crates, selects only relevant gates, runs
`mutation-diff` on changed production lines, and scans changed files for
credentials. A workspace manifest change selects the whole workspace; an
unavailable diff falls back to the full workspace, never to an empty check.

## Push jobs

Pushes to `main` run `quality`, `coverage`, `dogfood-audit`, `ui-a11y`,
`site-perf`, `contracts`, `monitoring-rules`, `systemd-units`, `caddy-edge`,
`dogfood-live`, and `store-portfolio`. Each job has a `when.path` filter; a
site-only change does not compile Rust and a crate-only change does not build
the site.

## Local commands

```sh
kiwi validate
kiwi run --job quality
kiwi run --job changed --changed-file rust/crates/apex-watch/src/apps.rs
make changed-plan
make check-changed
make mutation-changed
make dogfood
make dogfood-live
```

`make dogfood-live` needs the `clickhouse` binary; the harness starts and stops
its own server and proves cleanup. `store-portfolio` runs the live App Store and
Play catalogue tests with `APEX_LIVE_WATCH=1`.

`tools/ci/ship.sh` remains the deploy helper; its pre-flight and rollback tests
run in `tools/ci/tests/run-tests.sh`.
