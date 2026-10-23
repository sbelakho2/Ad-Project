# Local CI

CI is Kiwi CI (`.kiwi/pipeline.yaml`); pull requests run the diff-scoped `changed` and independent verifier jobs, and pushes run the full lanes. `tools/ci/local.sh` is a legacy local harness that can replay the retired Woodpecker lane files on a workstation; it is not a CI definition. `tools/ci/ship.sh` then ships the release binaries to the deploy host with `rsync` and restarts the systemd units. The hermetic tests live in `tools/ci/tests/run-tests.sh`.

## The host architecture answer

Both x86-64 Linux and Apple Silicon Mac hosts are supported. `local.sh` reads `uname -m` and maps it to a docker platform: `linux/amd64` on Intel, `linux/arm64` on Apple Silicon. The value is exported as `DOCKER_DEFAULT_PLATFORM` and passed to every `docker run` as `--platform`, so multi-arch images run natively on both hosts. Set `APEX_CI_PLATFORM` to override the mapping, for example `linux/amd64` for an image that publishes no arm64 build; docker then uses emulation for that image.

Release artifacts are always built for the deploy target platform. On a Linux deploy host the plain `cargo build --release` already produces Linux binaries. From a Mac, set `APEX_SHIP_TARGET=x86_64-unknown-linux-gnu` so artifacts land in `rust/target/x86_64-unknown-linux-gnu/release`, and `ship.sh` refuses before any `ssh` when a selected binary is missing.

## Running the lanes

```
sh tools/ci/local.sh --list
sh tools/ci/local.sh
sh tools/ci/local.sh --lane dogfood-audit
sh tools/ci/local.sh --lane ui-ux --lane site-perf --dry-run
make ci-local
```

Flags:

- `--lane <name>` runs one lane; repeat for several; the default is all lanes.
- `--list` prints lanes, steps, services, images and matrix variables; nothing runs.
- `--keep` keeps containers, the per-lane network and step scripts for debugging.
- `--dry-run` prints every docker command, with the step script as a heredoc; nothing runs.
- `DOGFOOD_DRY_RUN=1` equals `--dry-run`.

`make ci-local` forwards flags through `ARGS`, for example `make ci-local ARGS="--lane site-perf --dry-run"`.

## Diff-scoped checks

`tools/ci/changed.py` maps the working tree (or a `--base` range) to the
Rust crates it can affect, including reverse-dependency dependents, and to the
non-Rust gates whose inputs changed. The `changed` lane runs it on pull
requests; `make changed-plan` prints the selection, `make test-changed` and
`make lint-changed` run only the selected crates, and `make check-changed`
adds the relevant docs, site, edge, sync, dogfood, accessibility and mutation
gates. A workspace manifest change selects the whole workspace; a source change
to `apex-core` selects only the reverse-dependency closure. An unavailable
diff falls back to the full workspace rather than skipping checks.

`make mutation-changed` runs `mutation-diff` on every changed production Rust
source file. It copies the repository to a throwaway tree, generates one
compiling comparison/boolean mutant per changed decision, and requires the
owning crate's tests to kill it. A survivor or an uncompilable mutant fails the
gate; a hunk that only changes comments, copy or wiring is reported as
`no-decision`. This is deliberately different from rerunning the test suite:
it measures whether the tests can detect a changed decision.

## Independent verification

Implementation and verification are separate roles. The implementer may not
mark its own change verified. A separate testing agent (or the separate
`independent-verifier` Kiwi job) reruns the diff-scoped checks, mutation, and
the live/static gates in its own process and with `CARGO_TARGET_DIR` pointed
at a verifier-only directory, then records the exact commit and commands. The
policy is in the repository `AGENTS.md`. The Kiwi job has no `needs:` edge on
`changed`, so a failing implementer job cannot suppress the independent run.

The repository is mounted read-write at `$CI_WORKSPACE` (`/woodpecker/src/local`), and every step's `commands:` are delivered to `sh -es` inside its own `docker run`, the same shape as a lane step's shell script. Steps with `depends_on` run after their dependencies; independent steps run sequentially in file order. A step-level `depends_on` that names a lane instead of a step (the coverage gate does this) is reported on stderr and does not change local order. Lanes run in their own `depends_on` order.

Lane service containers start the same way the lanes declare them. `dogfood-live` uses a ClickHouse service; `local.sh` creates a per-lane docker network, starts the service with its declared `image` and `environment`, and attaches every step container to that network with `--network-alias <service>`, so `http://clickhouse:8123` resolves as it does in CI.

A lane with a `matrix:` block expands like Woodpecker: each matrix entry becomes `-e` environment on the step. `apex-rs-coverage` therefore runs one entry per crate.

The first failing step stops the run with the lane name, the step name, the exit code and the tail of the step output; the script exits with the step's code. A real run needs `docker` with a reachable daemon and `python3`; `--list` and `--dry-run` need neither docker nor a daemon.

## Lanes covered

Every lane is covered. Images are parsed at runtime from the lane files, so a pin change needs no edit here.

- `apex-rs-coverage`: `rust:1.88-bookworm`, one matrix entry per crate.
- `dogfood-audit`: `rust:1.88-bookworm`.
- `dogfood-live`: `rust:1.88-bookworm` plus the `clickhouse/clickhouse-server:26.8` service container.
- `contract-coverage`: `rust:1.88-bookworm`, `alpine:3.20`, `debian:bookworm`.
- `site-perf`: `ghcr.io/getzola/zola:v0.19.2`, `rust:1.88-bookworm`.
- `ui-ux`: `rust:1.88-bookworm`.
- `coverage-gate`: `rust:1.88-bookworm`; run it after the lanes it aggregates.

## Shipping releases

Copy `tools/ci/ship.env.sample` to `tools/ci/ship.env` and set the host. The sample documents `APEX_SHIP_HOST`, `APEX_SHIP_BIN_DIR`, `APEX_SHIP_UNITS`, `APEX_SHIP_TARGET` and `APEX_SHIP_ARTIFACT_DIR`. The file holds no secrets; `ssh` uses your config and agent, and no key path or password ever appears on a command line.

Ship flow:

1. `cargo build --release` for the selected services; skip with `--no-build`.
2. Local pre-flight: every selected binary must exist and be executable.
3. Remote pre-flight: `ssh` reachability, then `systemctl cat` for every unit.
4. Remote backup under `$APEX_SHIP_BIN_DIR/apex-ship-backups/<UTC timestamp>/`.
5. `rsync -a` of each binary into `$APEX_SHIP_BIN_DIR`.
6. `systemctl restart` of each unit.

Any failure before step 6 stops with a non-zero exit and no restart. Flags: `--service` (repeatable), `--host`, `--dry-run`, `--no-build`, `--rollback`.

`--rollback` checks every unit and every backup file first, restores the newest backup for the selected services, then restarts the units.

| Service | Cargo package | Units restarted |
| --- | --- | --- |
| `apex-api` | `apex-api` | `apex-api.service` |
| `apex-control` | `apex-control` | `apex-control.service` |
| `apex-sales-web` | `apex-sales-web` | `apex-sales-web.service` |
| `apex-watch` | `apex-watch` | `apex-watch.service` |
| `apex-runner` | `apex-autopilot` | `apex-runner-audit.service`, `apex-runner-cycle.service`, `apex-runner-daily.service`, `apex-runner-monthly.service` |
| `apex-ingest` | `apex-analytics` | `apex-ingest.service` |

## Relationship to the CI lanes

Same images: parsed from the `image:` values in `.woodpecker/*.yml`, never hardcoded. Same commands: taken verbatim from `commands:`. Same environment: `CI_WORKSPACE` is set and the repository is mounted at that path. Same meaning of failure: a non-zero step fails the run.

Differences, by design: steps run one at a time instead of in parallel; matrix entries run sequentially; lane triggers (`when:`) are not evaluated, so running a lane runs it now.

## Running the tests

`sh tools/ci/tests/run-tests.sh` needs no docker, no ssh and no network. Fake `docker`, `ssh`, `rsync` and `uname` binaries on `PATH` capture argv. The suite checks lane parsing against the real `.woodpecker` files and against fixtures (dependency ordering, multi-command steps, service containers). It also checks the platform mapping on both simulated architectures, the ship pre-flight refusals and the rollback paths. One assertion greps every captured argv for a secret-looking value and requires it to be absent.
