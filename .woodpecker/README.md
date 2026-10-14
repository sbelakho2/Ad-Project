# Woodpecker CI: the gate stack

Each `*.yml` file in this directory is one Woodpecker pipeline (five lanes plus the aggregate verdict
in `coverage-gate.yml`; six configs in total). Trigger: `push` and `pull_request` on `main` and
`master`, with no path filters (deliberate: see `docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md`
section 7.2).

Lanes:

- `apex-rs-coverage.yml`: one matrix entry per Rust crate, plus the mail-server workspace. Each entry
  runs `cargo llvm-cov`, the crate tests, clippy and fmt, then the `lcov-gate` floor check.
- `dogfood-audit.yml`: manifest, walker parity, orphan ceiling, floor ratchet and sync, assertion
  presence, stub ban, policy bans, mutation smoke and floor, docs links, prose ratchet.
- `contract-coverage.yml`: monitoring rules and their tests, terraform, proxies and compose, CI
  reference integrity, pipeline paths, content guard, docs references, i18n parity, stylesheet lint.
- `site-perf.yml`: Zola build, claims contract, byte budgets, structural checks on the built output.
- `ui-ux.yml`: the rendered console pages checked by the structural accessibility audit.
- `coverage-gate.yml`: the aggregate verdict. It depends on the five lanes and replays the manifest,
  orphan, ratchet, sync, walker and mutation invariants before reporting.

Floors are single-sourced from `tools/coverage/coverage-floor.json` (ratchet-only) and mirrored to
`tools/coverage/floors.txt` by `apex-tools sync-floors`; the dogfood audit fails if they drift or if
history regresses. Server and agent deployment design: `infrastructure/woodpecker/README.md`.

The local equivalent of the audit family is `make dogfood`. Every gate is a subcommand of the
`apex-tools` binary; the retired JavaScript tooling has no counterpart left in this tree.
