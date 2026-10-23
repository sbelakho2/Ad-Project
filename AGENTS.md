# Agent Rules

These rules are repository policy, not suggestions.

## Implementation and verification are separate

- The agent that changes code may not approve or declare its own change
  verified.
- Every change needs a verification pass by a **separate testing agent** (or
  the separate `independent-verifier` CI step) that starts from the committed
  diff and does not reuse the implementer's process state.
- The verifier runs the diff-scoped checks, the changed-file mutation gate,
  and the live/static gates that the diff selects. It records the commit SHA,
  the selected paths/crates, the commands, and the outcome.

## Mutation is mandatory for changed production Rust

- `make mutation-changed` must pass. Every changed production hunk that carries
  a comparison or boolean decision must produce a compiling mutant that the
  owning crate's tests kill.
- A hunk that only changes comments, copy, strings or module wiring is reported
  as `no-decision`; it cannot be mutated meaningfully. A survivor in a decision
  hunk is a test gap: add the test or change the production code; do not delete
  the mutant gate or update a floor to hide it.
- Only files under `rust/crates/*/src/` (excluding `src/bin/` and tests) are
  mutation targets. A test-only or documentation-only diff says so explicitly.

## Only relevant checks

- Use `make changed-plan` first. Run `make check-changed` (tests, clippy, mutation, secret scan, relevant gates) for the diff; do not
  rerun the full workspace to compensate for a weak diff selection.
- A change to `rust/Cargo.toml`, `Cargo.lock`, or the toolchain selects the
  full workspace. Everything else uses the reverse-dependency closure.
- An unavailable diff falls back to the full workspace; it never silently
  skips checks.

## Finish criteria

A change is finished only when the implementer's checks, the independent
verifier, the changed mutation gate, and the relevant live/static gates are
green, the tree is clean, and no generated artifact or local state is staged.
