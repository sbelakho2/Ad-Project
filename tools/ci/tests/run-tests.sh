#!/bin/sh
# Hermetic tests for tools/ci: fake docker, ssh, rsync and uname on PATH, so
# nothing needs a container runtime, a container registry or a network.
# DOGFOOD_DRY_RUN is pinned to 0 here: these tests must exercise the real
# execution paths of the scripts under test.
set -eu

TEST_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
ROOT=$(CDPATH='' cd -- "$TEST_DIR/../../.." && pwd)
LOCAL="$ROOT/tools/ci/local.sh"
SHIP="$ROOT/tools/ci/ship.sh"
FAKEBIN="$TEST_DIR/fakebin"
FIXTURES="$TEST_DIR/fixtures/lanes"
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

printf '%s\n' "== local.sh: lane, step, service and matrix parsing (real .woodpecker) =="
reset_cap
capture env FAKE_UNAME_M=arm64 sh "$LOCAL" --list
eq "T1 --list exits 0" "$RC" 0
eq "T1 lane count" "$(count_prefix "lane " "$WORK/out.txt")" 7
eq "T1 step count (matrix expanded)" "$(count_prefix "  step " "$WORK/out.txt")" 49
eq "T1 service count" "$(count_prefix "  service " "$WORK/out.txt")" 1
eq "T1 every step row has an image" "$(step_rows_without_image "$WORK/out.txt")" 0
contains "T1 dogfood-live clickhouse service" "$WORK/out.txt" "  service clickhouse image=clickhouse/clickhouse-server:26.8 env=CLICKHOUSE_SKIP_USER_SETUP=1"
contains "T1 zola image" "$WORK/out.txt" "ghcr.io/getzola/zola:v0.19.2"
contains "T1 rust image" "$WORK/out.txt" "image=rust:1.88-bookworm"
contains "T1 matrix entry expanded" "$WORK/out.txt" "  step apex-rs-crate[CRATE=apex-core,PKG=apex-core] image=rust:1.88-bookworm"
contains_re "T1 matrix env passed" "$WORK/out.txt" "env=CRATE=apex-core,PKG=apex-core"
if [ ! -f "$CAP/docker.argv" ]; then pass "T1 --list never invokes docker"; else fail "T1 --list invoked docker"; fi

printf '%s\n' "== local.sh: depends_on ordering and service fixtures =="
reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$FIXTURES" sh "$LOCAL" --list
eq "T2 fixture --list exits 0" "$RC" 0
eq "T2 fixture lane count" "$(count_prefix "lane " "$WORK/out.txt")" 3
if order_before "$WORK/out.txt" "lane zz-ordering" "lane xx-service"; then
  pass "T2 lane-level depends_on ordered before dependents"
else
  fail "T2 lane-level depends_on ordering"
fi
if order_before "$WORK/out.txt" "  step first image=" "  step second image="; then
  pass "T2 step-level depends_on orders second after first"
else
  fail "T2 step-level depends_on ordering"
fi
contains "T2 fixture service row" "$WORK/out.txt" "  service clickhouse image=clickhouse/clickhouse-server:26.8 env=CLICKHOUSE_SKIP_USER_SETUP=1"

printf '%s\n' "== local.sh: --dry-run platform mapping =="
reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$FIXTURES" sh "$LOCAL" --dry-run --lane xx-service
eq "T3 dry-run exits 0" "$RC" 0
contains "T3 Apple Silicon platform" "$WORK/out.txt" "--platform linux/arm64"
contains "T3 dry-run network create" "$WORK/out.txt" "docker network create apex-ci-xx-service"
contains "T3 dry-run service container" "$WORK/out.txt" "--network-alias clickhouse"
contains "T3 dry-run service env" "$WORK/out.txt" "--env CLICKHOUSE_SKIP_USER_SETUP=1"
eq "T3 every printed docker run carries --platform" "$(awk '/docker run/ && !index($0, "--platform") { n++ } END { print n + 0 }' "$WORK/out.txt")" 0
if [ ! -f "$CAP/docker.argv" ]; then pass "T3 --dry-run never invokes docker"; else fail "T3 --dry-run invoked docker"; fi
capture env FAKE_UNAME_M=x86_64 APEX_CI_LANES_DIR="$FIXTURES" sh "$LOCAL" --dry-run --lane yy-multi
contains "T3 x86-64 platform" "$WORK/out.txt" "--platform linux/amd64"
capture env FAKE_UNAME_M=arm64 APEX_CI_PLATFORM=linux/amd64 APEX_CI_LANES_DIR="$FIXTURES" sh "$LOCAL" --dry-run --lane yy-multi
contains "T3 APEX_CI_PLATFORM override wins over the host arch" "$WORK/out.txt" "--platform linux/amd64"

printf '%s\n' "== local.sh: multi-command steps print as one script =="
reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$FIXTURES" sh "$LOCAL" --dry-run --lane yy-multi
contains "T4 heredoc form" "$WORK/out.txt" "<<'APEX_CI_SCRIPT'"
contains "T4 shell entrypoint" "$WORK/out.txt" "sh -es"
if order_before "$WORK/out.txt" "echo one" "echo two" && order_before "$WORK/out.txt" "echo two" "echo three"; then
  pass "T4 commands keep their order"
else
  fail "T4 command order"
fi

printf '%s\n' "== local.sh: real execution through the fake docker =="
reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$FIXTURES" sh "$LOCAL" --lane xx-service
eq "T5 execute exits 0" "$RC" 0
DF="$CAP/docker.argv"
contains "T5 service started the lane way" "$DF" "docker run -d --name apex-ci-xx-service-clickhouse --network apex-ci-xx-service --network-alias clickhouse --platform linux/arm64 --env CLICKHOUSE_SKIP_USER_SETUP=1 clickhouse/clickhouse-server:26.8"
contains "T5 step joined the lane network" "$DF" "--network apex-ci-xx-service"
contains "T5 CI_WORKSPACE env" "$DF" "-e CI_WORKSPACE=/woodpecker/src/local"
contains "T5 repo mounted at CI_WORKSPACE" "$DF" "-v $ROOT:/woodpecker/src/local"
contains "T5 workdir is CI_WORKSPACE" "$DF" "-w /woodpecker/src/local"
contains "T5 step image" "$DF" "alpine:3.20"
contains "T5 step shell" "$DF" "sh -es"
contains "T5 docker info probe" "$DF" "docker info"
contains "T5 service container removed after the lane" "$DF" "docker rm -f apex-ci-xx-service-clickhouse"
contains "T5 network removed after the lane" "$DF" "docker network rm apex-ci-xx-service"
eq "T5 no --keep so every step is --rm" "$(count_containing "docker run --rm" "$DF")" 1

printf '%s\n' "== local.sh: step execution order and failure surfacing =="
reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$FIXTURES" sh "$LOCAL" --lane zz-ordering
eq "T6 ordering lane exits 0" "$RC" 0
if order_before "$CAP/docker.argv" "debian:bookworm" "alpine:3.20"; then
  pass "T6 first step runs before second despite file order"
else
  fail "T6 execution order"
fi
reset_cap
capture env FAKE_UNAME_M=arm64 FAKE_DOCKER_RUN_EXIT=7 APEX_CI_LANES_DIR="$FIXTURES" sh "$LOCAL" --lane zz-ordering
eq "T7 failing step exit code propagates" "$RC" 7
contains "T7 step name in the failure" "$WORK/out.txt" "FAILED lane 'zz-ordering' step 'first' (exit 7)"
contains "T7 output tail printed" "$WORK/out.txt" "tail of the step output"

printf '%s\n' "== local.sh: --changed relevance selection (fixtures) =="
reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$CHANGED_FIXTURES" sh "$LOCAL" --list --changed rust/crates/apex-core/src/lib.rs
eq "T16 changed rust exits 0" "$RC" 0
contains "T16 changed rust runs apex-rs-coverage" "$WORK/out.txt" "lane apex-rs-coverage"
contains "T16 changed rust lists the crate step" "$WORK/out.txt" "  step cov image=alpine:3.20"
not_contains "T16 changed rust skips site-perf" "$WORK/out.txt" "lane site-perf"
not_contains "T16 changed rust skips contract-coverage" "$WORK/out.txt" "lane contract-coverage"
if [ ! -f "$CAP/docker.argv" ]; then pass "T16 --list --changed never invokes docker"; else fail "T16 --list --changed invoked docker"; fi

reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$CHANGED_FIXTURES" sh "$LOCAL" --list --changed site/guide/index.html
eq "T17 changed site exits 0" "$RC" 0
contains "T17 changed site runs site-perf" "$WORK/out.txt" "lane site-perf"
contains "T17 changed site lists the perf step" "$WORK/out.txt" "  step perf image=alpine:3.20"
not_contains "T17 changed site skips apex-rs-coverage" "$WORK/out.txt" "lane apex-rs-coverage"
not_contains "T17 changed site skips contract-coverage" "$WORK/out.txt" "lane contract-coverage"

reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$CHANGED_FIXTURES" sh "$LOCAL" --list --changed .woodpecker/dogfood-live.yml
eq "T18 changed .woodpecker exits 0" "$RC" 0
contains "T18 changed .woodpecker runs contract-coverage" "$WORK/out.txt" "lane contract-coverage"
not_contains "T18 changed .woodpecker skips apex-rs-coverage" "$WORK/out.txt" "lane apex-rs-coverage"
not_contains "T18 changed .woodpecker skips site-perf" "$WORK/out.txt" "lane site-perf"

reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$CHANGED_FIXTURES" sh "$LOCAL" --list --changed docs/only.md
eq "T19 unmatchable change exits 0" "$RC" 0
eq "T19 unmatchable change runs no lane" "$(count_prefix "lane " "$WORK/out.txt")" 0
eq "T19 unmatchable change runs no step" "$(count_prefix "  step " "$WORK/out.txt")" 0

reset_cap
capture env FAKE_UNAME_M=arm64 APEX_CI_LANES_DIR="$CHANGED_FIXTURES" sh "$LOCAL" --list --changed rust/crates/apex-ui/src/lib.rs --changed site/index.html
eq "T20 union exits 0" "$RC" 0
contains "T20 union runs apex-rs-coverage" "$WORK/out.txt" "lane apex-rs-coverage"
contains "T20 union runs site-perf" "$WORK/out.txt" "lane site-perf"
not_contains "T20 union skips contract-coverage" "$WORK/out.txt" "lane contract-coverage"
eq "T20 union runs two lanes" "$(count_prefix "lane " "$WORK/out.txt")" 2

printf '%s\n' "== local.sh: --changed step-level filters (real dogfood-live) =="
reset_cap
capture env FAKE_UNAME_M=arm64 sh "$LOCAL" --list --changed .woodpecker/dogfood-live.yml
eq "T21 real lane-file change exits 0" "$RC" 0
contains "T21 dogfood-live wakes on its own file" "$WORK/out.txt" "lane dogfood-live"
contains "T21 subset step runs after the build" "$WORK/out.txt" "  step dogfood subset image=rust:1.88-bookworm depends_on=build"
not_contains "T21 full run-harness step is gated out" "$WORK/out.txt" "  step run-harness image="
not_contains "T21 full dogfood-plan step is gated out" "$WORK/out.txt" "  step dogfood-plan image="
contains "T21 contract-coverage wakes on .woodpecker/**" "$WORK/out.txt" "lane contract-coverage"

reset_cap
capture env FAKE_UNAME_M=arm64 sh "$LOCAL" --list --changed rust/crates/apex-billing/src/lib.rs
eq "T22 real rust change exits 0" "$RC" 0
contains "T22 rust change runs apex-rs-coverage" "$WORK/out.txt" "lane apex-rs-coverage"
contains "T22 rust change runs dogfood-audit" "$WORK/out.txt" "lane dogfood-audit"
contains "T22 rust change keeps the full run-harness step" "$WORK/out.txt" "  step run-harness image="
not_contains "T22 unrelated-crate change skips ui-ux" "$WORK/out.txt" "lane ui-ux"

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

printf '\nrun-tests: %d passed, %d failed\n' "$PASS" "$FAIL"
if [ "$FAIL" -ne 0 ]; then
  printf '%s\n' "captured argv for debugging:"
  for f in "$CAP"/*.argv; do
    printf '%s\n' "-- $f"
    cat "$f"
  done
  exit 1
fi
