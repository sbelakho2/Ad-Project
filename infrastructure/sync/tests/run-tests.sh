#!/bin/sh
# Local test harness for infrastructure/sync/push.sh and pull.sh. Runs five
# scenarios against the committed fake rclone shim (fake-rclone.sh) put on
# PATH as `rclone`:
#   1. success: push uploads the ledger, invoice, leads and signin objects
#      plus the manifest last; pull verifies every object, installs it into
#      the control paths and writes the marker; a dry run uploads nothing.
#   2. hash mismatch: an object corrupted in transit (same size, one changed
#      byte) makes pull refuse; the previous control copies and marker stay
#      byte for byte untouched.
#   3. partial copy: one refused download aborts the run before any install,
#      so stale destinations and the marker stay as they were.
#   4. empty bucket: a missing manifest refuses before anything is written.
#   5. batch scale: with dozens of objects the rclone invocation counts stay
#      flat (one batched transfer per family, one staged download batch), and
#      every installed file is byte-identical to its source.
# Invocation counts come from the shim's FAKE_RCLONE_COUNT file: assertions
# are count-based, never wall-clock.
# Usage: sh infrastructure/sync/tests/run-tests.sh
set -eu

script_dir="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
sync_dir="$(dirname -- "$script_dir")"
push_script="$sync_dir/push.sh"
pull_script="$sync_dir/pull.sh"
shim="$script_dir/fake-rclone.sh"

if ! command -v python3 >/dev/null 2>&1; then
    echo "run-tests: python3 is required for the JSON assertions" >&2
    exit 2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/apex-sync-tests.XXXXXX")"
count_file="$work/rclone-count.txt"
cleanup() {
    rm -rf "$work"
}
trap cleanup EXIT INT TERM

# Put the shim on PATH as `rclone`; the product scripts must not need edits.
mkdir -p "$work/bin"
cp "$shim" "$work/bin/rclone"
chmod +x "$work/bin/rclone"
PATH="$work/bin:$PATH"
export PATH

fail() {
    echo "run-tests: FAIL: $*" >&2
    exit 1
}

remote="object-store:apex-sync"

push_env() {
    APEX_SYNC_REMOTE="$remote" \
    APEX_SYNC_LEDGER_ROOT="$console/.apex/ledgers" \
    APEX_SYNC_INVOICE_REGISTER="$console/.apex/billing/invoices.jsonl" \
    APEX_SYNC_LEADS_FILE="$console/.apex/leads.jsonl" \
    FAKE_RCLONE_ROOT="$bucket" \
    FAKE_RCLONE_COUNT="$count_file" \
    sh "$push_script" "$@"
}

pull_env() {
    APEX_SYNC_REMOTE="$remote" \
    APEX_CONTROL_LEDGER_ROOT="$control/.apex/ledgers" \
    APEX_CONTROL_INVOICE_REGISTER="$control/.apex/billing/invoices.jsonl" \
    APEX_SYNC_IMPORTS_DIR="$control/.apex/imports" \
    APEX_CONTROL_SYNC_MARKER="$marker_path" \
    FAKE_RCLONE_ROOT="$bucket" \
    FAKE_RCLONE_COUNT="$count_file" \
    sh "$pull_script"
}

pull_corrupt() {
    FAKE_RCLONE_CORRUPT="$1" \
    APEX_SYNC_REMOTE="$remote" \
    APEX_CONTROL_LEDGER_ROOT="$control/.apex/ledgers" \
    APEX_CONTROL_INVOICE_REGISTER="$control/.apex/billing/invoices.jsonl" \
    APEX_SYNC_IMPORTS_DIR="$control/.apex/imports" \
    APEX_CONTROL_SYNC_MARKER="$marker_path" \
    FAKE_RCLONE_ROOT="$bucket" \
    FAKE_RCLONE_COUNT="$count_file" \
    sh "$pull_script"
}

pull_partial() {
    FAKE_RCLONE_FAIL="$1" \
    APEX_SYNC_REMOTE="$remote" \
    APEX_CONTROL_LEDGER_ROOT="$control/.apex/ledgers" \
    APEX_CONTROL_INVOICE_REGISTER="$control/.apex/billing/invoices.jsonl" \
    APEX_SYNC_IMPORTS_DIR="$control/.apex/imports" \
    APEX_CONTROL_SYNC_MARKER="$marker_path" \
    FAKE_RCLONE_ROOT="$bucket" \
    FAKE_RCLONE_COUNT="$count_file" \
    sh "$pull_script"
}

new_sandbox() {
    sandbox="$work/$1"
    console="$sandbox/console"
    control="$sandbox/control"
    bucket="$sandbox/bucket"
    marker_path="$control/.apex/control/sync-marker.json"
    mkdir -p "$console/.apex/ledgers" "$console/.apex/billing" "$control"
    printf '{"publisher":"acme","amount_micros":100}\n' > "$console/.apex/ledgers/acme.jsonl"
    printf '{"publisher":"globex","amount_micros":200}\n' > "$console/.apex/ledgers/globex.jsonl"
    printf '{"invoice":"inv-1"}\n' > "$console/.apex/billing/invoices.jsonl"
    printf '{"email":"lead@example.test"}\n' > "$console/.apex/leads.jsonl"
    printf '{"token_hash":"aa"}\n' > "$console/.apex/signin.jsonl"
}

assert_file_eq() {
    cmp -s "$1" "$2" || fail "expected identical files: $1 and $2"
}

assert_call_count() {
    expected="$1"
    actual=$(wc -l < "$count_file" | tr -d ' ')
    [ "$actual" = "$expected" ] || fail "expected $expected rclone invocation(s), counted $actual"
}

assert_absent() {
    [ ! -e "$1" ] || fail "expected nothing at: $1"
}

assert_marker() {
    APEX_TEST_MARKER="$marker_path" APEX_TEST_SOURCE="$1" python3 - <<'PY'
import json
import os

with open(os.environ["APEX_TEST_MARKER"], encoding="utf-8") as handle:
    marker = json.load(handle)
if not isinstance(marker.get("synced_at_unix_ms"), int) or marker["synced_at_unix_ms"] <= 0:
    raise SystemExit("marker synced_at_unix_ms is not a positive integer")
if marker.get("source") != os.environ["APEX_TEST_SOURCE"]:
    raise SystemExit(f"marker source mismatch: {marker.get('source')!r}")
PY
}

assert_manifest_matches_sources() {
    APEX_TEST_MANIFEST="$bucket/apex-sync/manifest.json" \
    APEX_TEST_CONSOLE="$console" python3 - <<'PY'
import hashlib
import json
import os

console = os.environ["APEX_TEST_CONSOLE"]
expected = {
    "ledger/acme.jsonl": f"{console}/.apex/ledgers/acme.jsonl",
    "ledger/globex.jsonl": f"{console}/.apex/ledgers/globex.jsonl",
    "invoices/invoices.jsonl": f"{console}/.apex/billing/invoices.jsonl",
    "leads/leads.jsonl": f"{console}/.apex/leads.jsonl",
    "leads/signin.jsonl": f"{console}/.apex/signin.jsonl",
}
with open(os.environ["APEX_TEST_MANIFEST"], encoding="utf-8") as handle:
    manifest = json.load(handle)
entries = {entry["path"]: entry for entry in manifest["files"]}
if set(entries) != set(expected):
    raise SystemExit(f"manifest keys mismatch: {sorted(entries)}")
for key, path in expected.items():
    with open(path, "rb") as handle:
        data = handle.read()
    entry = entries[key]
    if entry["size"] != len(data) or entry["sha256"] != hashlib.sha256(data).hexdigest():
        raise SystemExit(f"manifest entry does not match the source: {key}")
if not isinstance(manifest.get("created_at_unix_ms"), int):
    raise SystemExit("manifest created_at_unix_ms is not an integer")
print(f"manifest verified: {len(entries)} entries against the console sources")
PY
}

echo "== scenario 1: success, dry run, install and marker =="
new_sandbox success
: > "$count_file"
push_env
assert_call_count 4
echo "push used 4 batched rclone call(s): ledger copy, invoice, leads batch, manifest"
assert_manifest_matches_sources
for object in ledger/acme.jsonl ledger/globex.jsonl invoices/invoices.jsonl leads/leads.jsonl leads/signin.jsonl; do
    [ -f "$bucket/apex-sync/$object" ] || fail "missing bucket object: $object"
done

bucket="$sandbox/bucket-dry"
: > "$count_file"
push_env --dry-run
assert_call_count 4
assert_absent "$bucket"
echo "dry run uploaded nothing (same 4 batched call shape)"
bucket="$sandbox/bucket"

: > "$count_file"
pull_env
assert_call_count 2
echo "pull used 2 batched rclone call(s): manifest, staged download batch"
assert_marker "ledger|invoices|leads"
assert_file_eq "$console/.apex/ledgers/acme.jsonl" "$control/.apex/ledgers/acme.jsonl"
assert_file_eq "$console/.apex/ledgers/globex.jsonl" "$control/.apex/ledgers/globex.jsonl"
assert_file_eq "$console/.apex/billing/invoices.jsonl" "$control/.apex/billing/invoices.jsonl"
assert_file_eq "$console/.apex/leads.jsonl" "$control/.apex/imports/leads.jsonl"
assert_file_eq "$console/.apex/signin.jsonl" "$control/.apex/imports/signin.jsonl"
pull_env
echo "scenario 1 (success): PASS"

echo
echo "== scenario 2: hash mismatch refusal =="
new_sandbox hash-mismatch
push_env >/dev/null
pull_env >/dev/null
cp "$control/.apex/ledgers/acme.jsonl" "$sandbox/before-acme.jsonl"
cp "$marker_path" "$sandbox/before-marker.json"
: > "$count_file"
if pull_corrupt "ledger/acme.jsonl" > "$sandbox/refusal.out" 2>&1; then
    fail "a corrupted object must refuse the pull"
fi
assert_call_count 2
cat "$sandbox/refusal.out"
grep -q "sha256 does not match" "$sandbox/refusal.out" || fail "refusal must name the sha256 mismatch"
assert_file_eq "$sandbox/before-acme.jsonl" "$control/.apex/ledgers/acme.jsonl"
assert_file_eq "$sandbox/before-marker.json" "$marker_path"
echo "scenario 2 (hash mismatch): PASS"

echo
echo "== scenario 3: partial copy, no install, no marker =="
new_sandbox partial-copy
push_env >/dev/null
pull_env >/dev/null
printf 'stale ledger\n' > "$control/.apex/ledgers/acme.jsonl"
printf 'stale leads\n' > "$control/.apex/imports/leads.jsonl"
cp "$marker_path" "$sandbox/before-marker.json"
: > "$count_file"
if pull_partial "leads/signin.jsonl" > "$sandbox/refusal.out" 2>&1; then
    fail "a refused download must fail the pull"
fi
assert_call_count 2
cat "$sandbox/refusal.out"
grep -q "injected transfer refusal" "$sandbox/refusal.out" || fail "transcript must show the refused transfer"
assert_file_eq "$sandbox/before-marker.json" "$marker_path"
grep -q "stale ledger" "$control/.apex/ledgers/acme.jsonl" || fail "a failed run must not install staged objects"
grep -q "stale leads" "$control/.apex/imports/leads.jsonl" || fail "a failed run must not install staged objects"
echo "scenario 3 (partial copy): PASS"

echo
echo "== scenario 4: empty bucket refusal =="
new_sandbox empty-bucket
: > "$count_file"
if pull_env > "$sandbox/refusal.out" 2>&1; then
    fail "a missing manifest must refuse the pull"
fi
assert_call_count 1
cat "$sandbox/refusal.out"
assert_absent "$marker_path"
assert_absent "$control/.apex/ledgers/acme.jsonl"
echo "scenario 4 (empty bucket): PASS"

echo
echo "== scenario 5: batched transfers stay flat as the object count grows =="
new_sandbox batch-scale
index=1
while [ "$index" -le 64 ]; do
    printf '{"publisher":"batch-%03d","amount_micros":%d}\n' "$index" "$index" \
        > "$console/.apex/ledgers/batch-$index.jsonl"
    index=$((index + 1))
done
: > "$count_file"
push_env >/dev/null
assert_call_count 4
bucket_objects=$(find "$bucket/apex-sync/ledger" -type f | wc -l | tr -d ' ')
[ "$bucket_objects" = "66" ] || fail "expected 66 uploaded ledger objects, found $bucket_objects"
: > "$count_file"
pull_env >/dev/null
assert_call_count 2
assert_marker "ledger|invoices|leads"
installed=0
for source_file in "$console/.apex/ledgers"/*.jsonl; do
    name="${source_file##*/}"
    cmp -s "$source_file" "$control/.apex/ledgers/$name" || fail "batched install differs: $name"
    installed=$((installed + 1))
done
[ "$installed" = "66" ] || fail "expected 66 installed ledger files, found $installed"
echo "scenario 5 (batch scale): PASS: 66 ledger objects through 4 push and 2 pull rclone calls"

echo
echo "run-tests: 5 scenarios PASS"
