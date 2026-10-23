# Required PR Checks and CI Artifacts

_Last updated: 2026-10-14_
_Owner: Platform Engineering_
_Review Cycle: When CI lanes change_

## Overview

Kiwi CI is the only pipeline (`.kiwi/pipeline.yaml`). Branch protection on pull
requests requires exactly these two job names, matching
`.github/required-checks.json`:

- `changed`: diff-scoped tests, clippy, changed-file mutation, secret scan and
  the relevant static gates.
- `independent-verifier`: the same diff from a separate Kiwi job, with its own
  `CARGO_TARGET_DIR`, so the implementer cannot approve its own change.

Pushes to `main` run the full jobs: `quality`, `coverage`, `dogfood-audit`,
`ui-a11y`, `site-perf`, `contracts`, `monitoring-rules`, `systemd-units`,
`caddy-edge`, `dogfood-live` and `store-portfolio`. Each job declares its own
`when.path` filter; a site-only change does not compile Rust and a crate-only
change does not rebuild the site.

## Artifacts

- Rust crates: per-crate lcov files gated by `apex-tools lcov-gate` against
  `tools/coverage/floors.txt`.
- Changed production Rust files: `apex-tools mutation-diff` requires a killed
  mutant per file and writes no repository state.
- The site lane gates the built output; the console lane gates the rendered
  fixture pages; the contract lane gates the monitoring rules, the edge
  config and the documentation contracts.
