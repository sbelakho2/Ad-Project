.PHONY: test fmt lint test-changed lint-changed check-changed mutation-changed mutation-file changed-plan sync-floors dogfood dogfood-live clickhouse-dev clickhouse-stop ci-local

test:
	cd rust && cargo test --workspace

fmt:
	cd rust && cargo fmt --all

lint:
	cd rust && cargo clippy --workspace --all-targets -- -D warnings

# Diff-scoped checks: only the crates and gates the working tree (or
# --base range) can affect. See tools/ci/changed.py.
changed-plan:
	python3 tools/ci/changed.py --list

test-changed:
	python3 tools/ci/changed.py --test

lint-changed:
	python3 tools/ci/changed.py --clippy

check-changed:
	python3 tools/ci/changed.py --test --clippy --gates --mutation --security

# Mutation-verify only the production Rust files in the diff.
mutation-changed:
	python3 tools/ci/changed.py --mutation

# Mutation-verify one production file (the changed lane calls this through
# mutation-diff with every changed source file).
mutation-file:
	cd rust && ./target/debug/apex-tools mutation-diff --file "$(FILE)"

# ClickHouse dev server. Runs in the FOREGROUND: run it in a terminal or a
# supervisor, not in a build step. Start from the repository root so the
# config's .apex/clickhouse paths resolve correctly.
clickhouse-dev:
	clickhouse server --config-file infrastructure/clickhouse/config.xml

# Graceful stop of the dev server over the native protocol. There is no
# process-signal fallback: on a shared host a pattern kill could take down
# another project's server. If the client cannot connect, nothing is stopped
# and the recipe says so.
clickhouse-stop:
	clickhouse client --port 9000 --query "SYSTEM SHUTDOWN" || echo "clickhouse-stop: SYSTEM SHUTDOWN was not delivered. Check the server is down; if it is not (started elsewhere), stop it wherever it runs."

# Mirror coverage-floor.json into floors.txt after a deliberate floor change.
sync-floors:
	cd rust && cargo build -q -p apex-tools
	cd rust && ./target/debug/apex-tools sync-floors

# The local audit family: every invariant the Kiwi CI jobs enforce, in one
# pass. Build the tool first if needed.
dogfood:
	cd rust && cargo build -q -p apex-tools
	cd rust && ./target/debug/apex-tools manifest --out coverage-manifest.json
	cd rust && ./target/debug/apex-tools walker-parity --manifest coverage-manifest.json
	cd rust && ./target/debug/apex-tools orphan-gate --manifest coverage-manifest.json
	cd rust && ./target/debug/apex-tools floor-ratchet
	cd rust && ./target/debug/apex-tools floors-sync
	cd rust && ./target/debug/apex-tools assertion-presence
	cd rust && ./target/debug/apex-tools stub-ban
	cd rust && ./target/debug/apex-tools yaml-policy --full
	cd rust && ./target/debug/apex-tools mutation-smoke
	cd rust && ./target/debug/apex-tools mutation-floor
	cd rust && ./target/debug/apex-tools contract ci-reference-integrity
	cd rust && ./target/debug/apex-tools contract content-guard-adversarial
	cd rust && ./target/debug/apex-tools contract pipeline-paths
	cd rust && ./target/debug/apex-tools contract docs-references
	cd rust && ./target/debug/apex-tools docs-links --baseline tools/coverage/docs-links-baseline.txt
	cd rust && ./target/debug/apex-tools docs-lint --source --baseline tools/coverage/docs-lint-baseline.txt
	cd rust && ./target/debug/apex-tools css-lint
	cd rust && ./target/debug/apex-tools aggregate-verdict --report coverage-report.md
	cd rust && ./target/debug/apex-tools dogfood-plan

# The live half of the adversarial dogfooding system: boots apex-api,
# apex-control and apex-sales-web against the ClickHouse dev server and runs
# every required probe cell in tools/dogfood/surfaces.json (full contract in
# tools/dogfood/README.md). Requires the clickhouse binary and the harness
# binaries: cargo build -p apex-api -p apex-control -p apex-sales-web
# -p apex-sales -p apex-autopilot -p apex-tools --bins.
# --clickhouse-owned starts a server when none is reachable and grants the
# harness the failure-injection ClickHouse severance subprobe (it stops and
# restarts the server at --clickhouse-url). Do not point it at a shared server.
dogfood-live:
	@command -v clickhouse >/dev/null 2>&1 || { echo "dogfood-live: the clickhouse binary is required (the harness starts its own server)"; exit 1; }
	cd rust && cargo build -q --bins
	python3 tools/dogfood/live.py --clickhouse-owned --bin-dir rust/target/debug

# Run the legacy local lane harness (not CI; see
# docs/Internal/CI/LOCAL_CI.md). Flags pass through ARGS, for example
# ARGS="--lane dogfood-audit --dry-run".
ci-local:
	sh tools/ci/local.sh $(ARGS)
