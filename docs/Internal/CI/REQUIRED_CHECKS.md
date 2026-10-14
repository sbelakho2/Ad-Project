# Required PR Checks and CI Artifacts

_Last updated: 2026-10-14_
_Owner: Platform Engineering_
_Review Cycle: When CI lanes change_

## Overview

Woodpecker runs the gate stack in `.woodpecker/`. Configure these three lanes
as required status checks in branch protection, mirroring
`.github/required-checks.json`:

- `coverage-gate`: the aggregate verdict over every lane.
- `dogfood-audit`: the repository invariants (manifest, ratchet, bans, docs).
- `contract-coverage`: the infrastructure, contract and content gates.

Per-lane gates (`apex-rs-coverage`, `site-perf`, `ui-ux`) feed
`coverage-gate` and must pass for it to pass.

## Artifacts

- Rust crates: per-crate lcov files gated by `apex-tools lcov-gate` against
  `tools/coverage/floors.txt`.
- The site lane gates the built output; the console lane gates the rendered
  fixture pages; the contract lane gates the monitoring rules, the edge
  config and the documentation contracts.
