.PHONY: test fmt lint sync-floors dogfood

test:
	cd rust && cargo test --workspace

fmt:
	cd rust && cargo fmt --all

lint:
	cd rust && cargo clippy --workspace --all-targets -- -D warnings

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
