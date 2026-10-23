#!/bin/sh
# Hermetic tests for tools/ci: fake docker, ssh, rsync and uname on PATH, so
# nothing needs a container runtime, a container registry or a network.
# DOGFOOD_DRY_RUN is pinned to 0 here: these tests must exercise the real
# execution paths of the scripts under test.
set -eu

TEST_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
ROOT=$(CDPATH='' cd -- "$TEST_DIR/../../.." && pwd)
SHIP="$ROOT/tools/ci/ship.sh"
FAKEBIN="$TEST_DIR/fakebin"
CHANGED_FIXTURES="$TEST_DIR/fixtures/changed"

WORK=$(mktemp -d "${TMPDIR:-/tmp}/apex-ci-tests.XXXXXX")
CAP="$WORK/cap"
mkdir -p "$CAP"

export FAKE_CAPTURE_DIR="$CAP"
export DOGFOOD_DRY_RUN=0
PATH="$FAKEBIN:$PATH"
export PATH

PASS=0
FAIL=0
RC=0

cleanup() {
  rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 130' INT HUP TERM

pass() {
  PASS=$((PASS + 1))
  printf 'PASS: %s\n' "$1"
}

fail() {
  FAIL=$((FAIL + 1))
  printf 'FAIL: %s\n' "$1"
}

capture() {
  if "$@" > "$WORK/out.txt" 2>&1; then
    RC=0
  else
    RC=$?
  fi
}

eq() {
  if [ "$2" = "$3" ]; then
    pass "$1"
  else
    fail "$1 (got '$2', want '$3')"
  fi
}

contains() {
  if grep -F -q -- "$3" "$2" 2>/dev/null; then
    pass "$1"
  else
    fail "$1 (missing '$3' in $2)"
  fi
}

matches() {
  if grep -E -q -- "$3" "$2" 2>/dev/null; then
    pass "$1"
  else
    fail "$1 (no match '$3' in $2)"
  fi
}

contains_re() {
  if grep -E -q -- "$3" "$2" 2>/dev/null; then
    pass "$1"
  else
    fail "$1 (no line matching '$3' in $2)"
  fi
}

not_contains() {
  if grep -F -q -- "$3" "$2" 2>/dev/null; then
    fail "$1 (found '$3' in $2)"
  else
    pass "$1"
  fi
}

count_prefix() {
  awk -v p="$1" 'index($0, p) == 1 { n++ } END { print n + 0 }' "$2"
}

count_containing() {
  awk -v s="$1" 'index($0, s) > 0 { n++ } END { print n + 0 }' "$2"
}

line_of_first() {
  awk -v s="$1" 'index($0, s) > 0 { print NR; exit }' "$2"
}

reset_cap() {
  rm -f "$CAP/docker.argv" "$CAP/ssh.argv" "$CAP/rsync.argv"
}

order_before() {
  awk -v a="$2" -v b="$3" 'index($0, a) > 0 && !na { na = NR } index($0, b) > 0 && !nb { nb = NR } END { exit !(na && nb && na < nb) }' "$1"
}

step_rows_without_image() {
  awk '/^  step / && !index($0, " image=") { n++ } END { print n + 0 }' "$1"
}

mkdir -p "$WORK/art" "$WORK/empty-art"
printf '#!/bin/sh\nexit 0\n' > "$WORK/art/apex-api"
chmod +x "$WORK/art/apex-api"
NO_ENV="$WORK/no-such-env"

printf '%s\n' "== ship.sh: pre-flight refusal paths =="
reset_cap
capture env APEX_SHIP_ARTIFACT_DIR="$WORK/empty-art" APEX_SHIP_ENV="$NO_ENV" sh "$SHIP" --no-build --service apex-api --host fake@deploy
if [ "$RC" -ne 0 ]; then pass "T8 missing binary refuses"; else fail "T8 missing binary did not refuse"; fi
contains "T8 refusal names the artifact dir" "$WORK/out.txt" "missing or not executable"
contains "T8 refusal names the service" "$WORK/out.txt" "apex-api"
if [ ! -f "$CAP/ssh.argv" ]; then pass "T8 no ssh before the binary check"; else fail "T8 ssh ran before the binary check"; fi

reset_cap
capture env APEX_SHIP_ARTIFACT_DIR="$WORK/art" APEX_SHIP_ENV="$NO_ENV" FAKE_SSH_PROBE_FAIL=1 sh "$SHIP" --no-build --service apex-api --host fake@deploy
if [ "$RC" -ne 0 ]; then pass "T9 unreachable host refuses"; else fail "T9 unreachable host did not refuse"; fi
contains "T9 refusal explains reachability" "$WORK/out.txt" "cannot reach fake@deploy"
contains "T9 the probe ran" "$CAP/ssh.argv" " true"
not_contains "T9 no restart attempt on the unreachable host" "$CAP/ssh.argv" "systemctl restart"

printf '%s\n' "== ship.sh: happy path (rsync + backup + restart) =="
reset_cap
capture env APEX_SHIP_ARTIFACT_DIR="$WORK/art" APEX_SHIP_ENV="$NO_ENV" sh "$SHIP" --no-build --service apex-api --host fake@deploy
eq "T10 ship exits 0" "$RC" 0
SF="$CAP/ssh.argv"
RF="$CAP/rsync.argv"
contains "T10 unit pre-flight" "$SF" "systemctl cat apex-api.service"
contains "T10 timestamped backup dir created" "$SF" "mkdir -p /usr/local/bin/apex-ship-backups/"
contains_re "T10 backup timestamp is UTC ISO" "$SF" "apex-ship-backups/[0-9]{8}T[0-9]{6}Z"
contains "T10 previous binary backed up remotely" "$SF" "cp -a /usr/local/bin/apex-api /usr/local/bin/apex-ship-backups/"
contains "T10 rsync ships the binary" "$RF" "rsync -a $WORK/art/apex-api fake@deploy:/usr/local/bin/"
contains "T10 unit restarted" "$SF" "systemctl restart apex-api.service"
contains_re "T10 backup precedes the restart" "$SF" "apex-ship-backups/[0-9]{8}T"
contains "T10 summary names units" "$WORK/out.txt" "restarted: apex-api.service"

printf '%s\n' "== ship.sh: rollback =="
reset_cap
capture env FAKE_SSH_STDOUT="/usr/local/bin/apex-ship-backups/20260101T000000Z/" APEX_SHIP_ENV="$NO_ENV" sh "$SHIP" --rollback --service apex-api --host fake@deploy
eq "T11 rollback exits 0" "$RC" 0
contains "T11 backup file is checked before any restore" "$CAP/ssh.argv" "test -f /usr/local/bin/apex-ship-backups/20260101T000000Z/apex-api"
contains "T11 restore uses the backup path" "$CAP/ssh.argv" "cp -a /usr/local/bin/apex-ship-backups/20260101T000000Z/apex-api /usr/local/bin/apex-api"
contains "T11 units restarted after restore" "$CAP/ssh.argv" "systemctl restart apex-api.service"
contains "T11 summary" "$WORK/out.txt" "rolled back apex-api from /usr/local/bin/apex-ship-backups/20260101T000000Z"
reset_cap
capture env APEX_SHIP_ENV="$NO_ENV" sh "$SHIP" --rollback --service apex-api --host fake@deploy
if [ "$RC" -ne 0 ]; then pass "T12 rollback without a backup refuses"; else fail "T12 rollback without a backup did not refuse"; fi
contains "T12 refusal explains the missing backup" "$WORK/out.txt" "no backup found"

printf '%s\n' "== secrets never reach argv =="
SENTINEL="s3cr3t-sentinel-9f2c"
reset_cap
capture env APEX_SHIP_ARTIFACT_DIR="$WORK/art" APEX_SHIP_ENV="$NO_ENV" APEX_SHIP_PASSWORD="$SENTINEL" APEX_SHIP_SSH_KEY="$SENTINEL" APEX_SHIP_PRIVATE_KEY="$SENTINEL" sh "$SHIP" --no-build --service apex-api --host fake@deploy
eq "T13 ship still works with secret-looking env set" "$RC" 0
for f in "$CAP"/ssh.argv "$CAP"/rsync.argv; do
  not_contains "T13 no secret in $f" "$f" "$SENTINEL"
done
not_contains "T13 no secret in the ship output" "$WORK/out.txt" "$SENTINEL"

printf '%s\n' "== ship.sh: --dry-run prints and executes nothing =="
reset_cap
capture env APEX_SHIP_ARTIFACT_DIR="$WORK/empty-art" APEX_SHIP_ENV="$NO_ENV" sh "$SHIP" --dry-run --service apex-api --host fake@deploy
eq "T14 dry-run exits 0" "$RC" 0
contains "T14 build printed" "$WORK/out.txt" "cargo build --release -p apex-api"
contains "T14 probe printed" "$WORK/out.txt" "ssh -o BatchMode=yes -o ConnectTimeout=10 fake@deploy true"
contains "T14 unit check printed" "$WORK/out.txt" "systemctl cat apex-api.service"
contains "T14 rsync printed" "$WORK/out.txt" "rsync -a"
contains "T14 restart printed" "$WORK/out.txt" "systemctl restart apex-api.service"
if [ ! -f "$CAP/ssh.argv" ]; then pass "T14 no ssh executed"; else fail "T14 dry-run executed ssh"; fi
if [ ! -f "$CAP/rsync.argv" ]; then pass "T14 no rsync executed"; else fail "T14 dry-run executed rsync"; fi

printf '%s\n' "== live.py bookkeeping: equivalence and n vs 2n benchmark =="
EXPECTED_CELLS=$(python3 -c 'import json, sys; surfaces = json.load(open(sys.argv[1])); print(sum(len(cell["classes"]) for service in surfaces["services"].values() for cell in service["required"]))' "$ROOT/tools/dogfood/surfaces.json")
capture python3 "$TEST_DIR/live-bookkeeping.py"
eq "T15 live bookkeeping check exits 0" "$RC" 0
matches "T15 equivalence vs the pre-fix scan" "$WORK/out.txt" "cells_of matches the pre-fix scan for [0-9]+ class/service pairs"
contains "T15 n=$EXPECTED_CELLS benchmark printed" "$WORK/out.txt" "cells_of cells=$EXPECTED_CELLS "
contains "T15 n=$((EXPECTED_CELLS * 2)) benchmark printed" "$WORK/out.txt" "cells_of cells=$((EXPECTED_CELLS * 2)) "
matches "T15 response-contract table cache evidence" "$WORK/out.txt" "response-contract tables \([0-9]+ cells x 200 rounds\)"
contains "T15 one parity process, not one per cell" "$WORK/out.txt" "one dogfood-plan process for"
contains "T15 macOS ClickHouse restart env" "$WORK/out.txt" "the restart carries the macOS fork-safety switch"

printf '%s\n' "== changed.py: diff-scoped crate and gate selection =="
capture python3 "$ROOT/tools/ci/changed.py" --list --paths rust/crates/apex-api/src/routes.rs
eq "T16 changed planner exits 0" "$RC" 0
contains "T16 apex-api selected" "$WORK/out.txt" "apex-api,"
contains "T16 UI gate selected" "$WORK/out.txt" "gates: a11y"
capture python3 "$ROOT/tools/ci/changed.py" --list --paths rust/crates/apex-core/src/lib.rs
contains "T16 core change selects reverse dependents" "$WORK/out.txt" "apex-api"
if grep -q "apex-sales" "$WORK/out.txt"; then fail "T16 unrelated apex-sales selected for a core change"; else pass "T16 unrelated apex-sales not selected"; fi
if grep -q "apex-ui" "$WORK/out.txt"; then fail "T16 unrelated apex-ui selected for a core change"; else pass "T16 unrelated apex-ui not selected"; fi
capture python3 "$ROOT/tools/ci/changed.py" --list --paths .github/required-checks.json
contains "T16 github gate selected" "$WORK/out.txt" "github"
capture python3 "$ROOT/tools/ci/changed.py" --list --paths docs/INDEX.md --paths infrastructure/caddy/Caddyfile --paths site/content/pricing.md
eq "T16 multi-area planner exits 0" "$RC" 0
contains "T16 docs gate selected" "$WORK/out.txt" "docs"
contains "T16 caddy gate selected" "$WORK/out.txt" "caddy"
contains "T16 site gate selected" "$WORK/out.txt" "site"

printf '\nrun-tests: %d passed, %d failed\n' "$PASS" "$FAIL"
if [ "$FAIL" -ne 0 ]; then
  printf '%s\n' "captured argv for debugging:"
  for f in "$CAP"/*.argv; do
    printf '%s\n' "-- $f"
    cat "$f"
  done
  exit 1
fi

# Kiwi is the CI engine; validate the tracked pipeline when it is installed.
if command -v kiwi >/dev/null 2>&1; then
  if kiwi validate -f "$ROOT/.kiwi/pipeline.yaml" >/dev/null 2>&1; then
    pass "T99 Kiwi pipeline validates"
  else
    fail "T99 Kiwi pipeline validates"
  fi
else
  pass "T99 Kiwi pipeline validation skipped (kiwi not installed)"
fi
