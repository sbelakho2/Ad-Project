.PHONY: test fmt lint sync-floors dogfood clickhouse-dev clickhouse-stop

test:
	cd rust && cargo test --workspace

fmt:
	cd rust && cargo fmt --all

lint:
	cd rust && cargo clippy --workspace --all-targets -- -D warnings

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

# The local audit family: every invariant the Woodpecker lanes enforce, in one
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
