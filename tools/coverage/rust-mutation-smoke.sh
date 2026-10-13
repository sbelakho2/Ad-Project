#!/bin/sh
# rust-mutation-smoke.sh — sample-based mutation testing for apex-core.
#
# Phase 3 of the dogfooding plan starts with a bounded, honest sample instead of
# a full mutation platform: six hand-picked mutants across the money and
# validation modules are applied to a throwaway copy of the crate (the repo is
# never mutated), the crate's own test suite must kill each one, and the kill
# rate is written to tools/coverage/mutation-results.json where the
# mutation-floor gate enforces the ratcheting minimum.
#
# Killed = the crate's tests fail (or panic) under the mutant. A surviving
# mutant is printed loudly and lowers the score.
set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$WORK/crates"
cp -R "$REPO_ROOT/rust/crates/apex-core" "$WORK/crates/apex-core"

cat > "$WORK/Cargo.toml" <<'TOML'
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "proprietary"
publish = false

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"

[workspace.lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "deny"

[workspace.lints.clippy]
all = "deny"
TOML

cp "$WORK/crates/apex-core/src/money.rs" "$WORK/money.rs.orig"
cp "$WORK/crates/apex-core/src/validation.rs" "$WORK/validation.rs.orig"

run_tests() {
  ( cd "$WORK" && cargo test -p apex-core --offline --quiet >/dev/null 2>&1 )
}

# Baseline must be green, otherwise the kill counting is meaningless.
if ! run_tests; then
  echo "mutation-smoke: baseline test run failed; aborting" >&2
  exit 1
fi
echo "mutation-smoke: baseline green"

TOTAL=0
KILLED=0
RESULTS=""

mutate() {
  # $1 = file, $2 = sed expression, $3 = human description
  cp "$WORK/$1.orig" "$WORK/crates/apex-core/src/$1"
  sed -i '' -e "$2" "$WORK/crates/apex-core/src/$1" 2>/dev/null || sed -i -e "$2" "$WORK/crates/apex-core/src/$1"
  TOTAL=$((TOTAL + 1))
  if run_tests; then
    STATUS=survived
    echo "SURVIVED: $3" >&2
  else
    STATUS=killed
    KILLED=$((KILLED + 1))
  fi
  RESULTS="$RESULTS${RESULTS:+,}
    { \"description\": \"$3\", \"status\": \"$STATUS\" }"
}

mutate money.rs 's/MIN_LIFT_RATE_BP: u16 = 1_500/MIN_LIFT_RATE_BP: u16 = 1_400/' 'min lift rate 1500 -> 1400'
mutate money.rs 's/MAX_LIFT_RATE_BP: u16 = 2_000/MAX_LIFT_RATE_BP: u16 = 2_100/' 'max lift rate 2000 -> 2100'
mutate money.rs 's/checked_mul(10_000)/checked_mul(100)/' 'lift basis-points scale 10_000 -> 100'
mutate money.rs 's/if impressions == 0 {/if impressions == 1 {/' 'zero-impressions guard == 0 -> == 1'
mutate validation.rs 's/!(1..=MAX_ECPM_MICROS).contains(\&micros)/!(0..=MAX_ECPM_MICROS).contains(\&micros)/' 'ecpm lower bound 1 -> 0'
mutate validation.rs 's/trimmed.len() == 2/trimmed.len() != 2/' 'country shape check inverted'

cp "$WORK/money.rs.orig" "$WORK/crates/apex-core/src/money.rs"
cp "$WORK/validation.rs.orig" "$WORK/crates/apex-core/src/validation.rs"

SCORE=$((KILLED * 100 / TOTAL))
cat > "$REPO_ROOT/tools/coverage/mutation-results.json" <<JSON
{
  "__comment": "Sample-based mutation smoke for apex-core (money.rs + validation.rs). Phase 3: the mutant set and the floor ratchet upward together; a survived mutant must be investigated, not silently dropped from the sample.",
  "generatedAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "target": "apex-core: money.rs, validation.rs",
  "mutants": [$RESULTS
  ],
  "killed": $KILLED,
  "total": $TOTAL,
  "killRatePercent": $SCORE
}
JSON

echo "mutation-smoke: killed $KILLED/$TOTAL (${SCORE}%) -> tools/coverage/mutation-results.json"
exit 0
