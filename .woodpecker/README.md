# Woodpecker CI: the gate stack

Each `*.yml` file in this directory is one Woodpecker pipeline: ten lanes plus
the aggregate verdict in `coverage-gate.yml` (eleven configs). Trigger: `push`
and `pull_request` on `main` and `master`, with no path filters (deliberate:
see `docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md` section 9.10).

Lanes:

- `apex-rs-coverage.yml`: one matrix entry per Rust crate, plus the mail-server
  workspace, each running llvm-cov, tests, clippy, fmt and the lcov floor.
- `dogfood-audit.yml`: manifest, walker parity, orphan ceiling, floor ratchet
  and sync, assertion presence, stub ban, policy bans, mutation smoke and
  floor, docs links, prose ratchet.
- `contract-coverage.yml`: monitoring rules and their tests, terraform,
  proxies and compose, CI reference integrity, pipeline paths, content guard,
  docs references, the samples contract, i18n parity, stylesheet lint.
- `site-perf.yml`: Zola build, claims contract, byte budgets, structural
  checks on the built output.
- `ui-ux.yml`: the rendered console pages checked by the structural
  accessibility audit.
- `sdk-node.yml`: web and react-native jest suites, lint, type checks, builds
  and the lcov floor.
- `sdk-android-coverage.yml`: Kotlin unit tests and the kover line and branch
  floor for core and TV, plus both sample app builds.
- `sdk-apple-coverage.yml`: iOS, adapter dev kit and tvOS swift tests with an
  llvm-cov lcov floor on the darwin local-backend agent.
- `sdk-unity-coverage.yml`: the tier-1 Unity runtime tests on plain dotnet,
  coverlet lcov floor and the package size constraints.
- `sdk-flutter-coverage.yml`: flutter analyze, tests and the lcov floor.
- `coverage-gate.yml`: the aggregate verdict. It depends on every other lane
  and replays the manifest, orphan, ratchet, sync, walker, mutation and
  pipeline inventory invariants before reporting.

Floors are single-sourced from `tools/coverage/coverage-floor.json` (ratchet
only) and mirrored to `tools/coverage/floors.txt` by `apex-tools sync-floors`;
the dogfood audit fails when they drift or when a code domain has no floor.
The darwin steps select the macOS agent with the `platform=darwin/arm64` label
(see `infrastructure/woodpecker/README.md`).

The local equivalent of the audit family is `make dogfood`. Every gate is a
subcommand of the `apex-tools` binary.
