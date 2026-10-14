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

Per-platform lanes (`apex-rs-coverage`, `sdk-node`, `sdk-android-coverage`,
`sdk-apple-coverage`, `sdk-unity-coverage`, `sdk-flutter-coverage`,
`site-perf`, `ui-ux`) feed `coverage-gate` and must pass for it to pass.

## Artifacts

- Rust crates and the mail-server: per-crate lcov files gated by
  `apex-tools lcov-gate`.
- SDK lanes: jest, kover, coverlet and flutter lcov or kover reports gated
  against `tools/coverage/floors.txt`.
- The site lane gates the built output; the console lane gates the rendered
  fixture pages.
