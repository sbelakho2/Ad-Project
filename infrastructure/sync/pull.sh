#!/bin/sh
# Apex sync pull: one-way download on the control host from the
# S3-compatible Object Storage bucket that push.sh uploads to. Runs as the
# ExecStart of apex-sync-pull.service with the environment file
# /etc/apex/sync.env; see infrastructure/README.md, "Hosting split".
#
# Destinations (all read by control-host services):
#   ledger/*    -> APEX_CONTROL_LEDGER_ROOT (default .apex/ledgers), the
#                  read-only verified ledger root apex-control opens
#   invoices/*  -> APEX_CONTROL_INVOICE_REGISTER (default
#                  .apex/billing/invoices.jsonl), the register the control
#                  plane reads; the manifest must carry exactly one object
#   leads/*     -> APEX_SYNC_IMPORTS_DIR (default .apex/imports), the import
#                  directory apex-sales reads through APEX_SALES_LEADS_FILE
#
# Integrity and marker semantics: every object named by the remote
# manifest.json is staged, then size and sha256 are verified against the
# manifest, and only when every object verifies are the files installed and
# the marker APEX_CONTROL_SYNC_MARKER (default
# .apex/control/sync-marker.json) written atomically LAST, as
# {"synced_at_unix_ms":N,"source":"ledger|invoices|leads"} with the joined
# families actually installed. A partial failure exits nonzero before the
# marker step, so the marker always describes a complete sync and never
# claims fresh data over a stale or half-installed tree.
#
# Exit codes: 0 success, 2 configuration error (missing APEX_SYNC_REMOTE,
# missing rclone or python3, unknown argument), 1 download, manifest,
# verification or install failure. Any nonzero exit fails the
# apex-sync-pull unit.
set -eu

if [ "$#" -gt 0 ]; then
    echo "usage: apex-sync-pull" >&2
    exit 2
fi

if [ -z "${APEX_SYNC_REMOTE:-}" ]; then
    echo "apex-sync-pull: APEX_SYNC_REMOTE is not set (rclone remote syntax, for example object-store:apex-sync)" >&2
    exit 2
fi

if ! command -v rclone >/dev/null 2>&1; then
    echo "apex-sync-pull: rclone is not on PATH" >&2
    exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "apex-sync-pull: python3 is not on PATH (manifest verification needs it)" >&2
    exit 2
fi

remote="${APEX_SYNC_REMOTE%/}"
ledger_root="${APEX_CONTROL_LEDGER_ROOT:-.apex/ledgers}"
invoice_register="${APEX_CONTROL_INVOICE_REGISTER:-.apex/billing/invoices.jsonl}"
imports_dir="${APEX_SYNC_IMPORTS_DIR:-.apex/imports}"
marker="${APEX_CONTROL_SYNC_MARKER:-.apex/control/sync-marker.json}"

work="$(mktemp -d "${TMPDIR:-/tmp}/apex-sync-pull.XXXXXX")"
marker_tmp=""
cleanup() {
    rm -rf "$work"
    if [ -n "$marker_tmp" ]; then
        rm -f "$marker_tmp"
    fi
}
trap cleanup EXIT INT TERM
stage="$work/stage"
cache="$work/rclone-cache"
mkdir -p "$stage" "$cache"

# The manifest is fetched first; it is the last object push.sh uploads, so a
# fetched manifest names objects that are already in the bucket.
rclone --cache-dir "$cache" copyto "$remote/manifest.json" "$work/manifest.json"

entries="$work/entries.tsv"
families="$work/families.txt"
APEX_SYNC_MANIFEST="$work/manifest.json" APEX_SYNC_ENTRIES="$entries" APEX_SYNC_FAMILIES="$families" python3 - <<'PY'
import json
import os
import re
import sys

with open(os.environ["APEX_SYNC_MANIFEST"], encoding="utf-8") as handle:
    manifest = json.load(handle)

files = manifest.get("files")
if not isinstance(files, list) or not files:
    raise SystemExit("apex-sync-pull: manifest carries no files list")

segment = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
order = ("ledger", "invoices", "leads")
seen = []
rows = []
invoices = 0
for entry in files:
    path = str(entry.get("path", ""))
    size = entry.get("size")
    digest = str(entry.get("sha256", ""))
    parts = path.split("/")
    if len(parts) != 2 or parts[0] not in order:
        raise SystemExit(f"apex-sync-pull: unsupported manifest key: {path!r}")
    if not all(segment.match(part) for part in parts):
        raise SystemExit(f"apex-sync-pull: unsafe manifest key: {path!r}")
    if not isinstance(size, int) or size < 0 or not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise SystemExit(f"apex-sync-pull: malformed manifest entry: {path!r}")
    if parts[0] == "invoices":
        invoices += 1
    if parts[0] not in seen:
        seen.append(parts[0])
    rows.append((path, size, digest))

if invoices != 1:
    raise SystemExit(f"apex-sync-pull: expected exactly one invoices object, found {invoices}")

with open(os.environ["APEX_SYNC_ENTRIES"], "w", encoding="utf-8") as out:
    for path, size, digest in rows:
        out.write(f"{path}\t{size}\t{digest}\n")
with open(os.environ["APEX_SYNC_FAMILIES"], "w", encoding="utf-8") as out:
    out.write("|".join(family for family in order if family in seen))
PY

# Stage every object in one batched transfer; nothing lands in a destination
# yet. --files-from carries exactly the manifest keys (relative to the remote
# root), so the transferred set is the recorded set, never a directory walk
# of unlisted objects, and the file count never multiplies the rclone calls.
files_from="$work/files-from.txt"
cut -f1 "$entries" > "$files_from"
rclone --cache-dir "$cache" copy --files-from "$files_from" "$remote" "$stage"

# Verify the staged copies against the manifest. Any mismatch ends the run
# here, before an install and before any marker write.
APEX_SYNC_MANIFEST="$work/manifest.json" APEX_SYNC_STAGE="$stage" python3 - <<'PY'
import hashlib
import json
import os
import sys

with open(os.environ["APEX_SYNC_MANIFEST"], encoding="utf-8") as handle:
    manifest = json.load(handle)

stage = os.environ["APEX_SYNC_STAGE"]
problems = []
for entry in manifest["files"]:
    path = entry["path"]
    try:
        with open(os.path.join(stage, path), "rb") as handle:
            data = handle.read()
    except OSError as error:
        problems.append(f"{path}: cannot read the staged copy ({error})")
        continue
    if len(data) != entry["size"]:
        problems.append(f"{path}: size {len(data)} does not match manifest size {entry['size']}")
    elif hashlib.sha256(data).hexdigest() != entry["sha256"]:
        problems.append(f"{path}: sha256 does not match the manifest")
if problems:
    for problem in problems:
        print(f"apex-sync-pull: {problem}", file=sys.stderr)
    raise SystemExit("apex-sync-pull: refusing to install unverified objects")
print(f"apex-sync-pull: verified {len(manifest['files'])} object(s) against the manifest")
PY

# Install only after everything verified. Each family installs with one
# batched move (find -exec ... {} +), so the file count never multiplies the
# process count; find passes the paths as arguments, so names with spaces
# stay safe.
install_ledgers() {
    [ -d "$stage/ledger" ] || return 0
    mkdir -p "$ledger_root"
    find "$stage/ledger" -type f -exec sh -c 'dest=$1; shift; mv "$@" "$dest"' sh "$ledger_root" {} +
}

install_invoices() {
    [ -d "$stage/invoices" ] || return 0
    mkdir -p "$(dirname -- "$invoice_register")"
    find "$stage/invoices" -type f -exec sh -c 'dest=$1; shift; mv "$@" "$dest"' sh "$invoice_register" {} +
}

install_leads() {
    [ -d "$stage/leads" ] || return 0
    mkdir -p "$imports_dir"
    find "$stage/leads" -type f -exec sh -c 'dest=$1; shift; mv "$@" "$dest"' sh "$imports_dir" {} +
}

install_ledgers
install_invoices
install_leads

# The marker: written atomically (temp file plus rename) and only now, after
# a complete verified install.
sync_families="$(cat "$families")"
now_ms=$(( $(date +%s) * 1000 ))
mkdir -p "$(dirname -- "$marker")"
marker_tmp="$marker.tmp.$$"
printf '{"synced_at_unix_ms":%s,"source":"%s"}\n' "$now_ms" "$sync_families" > "$marker_tmp"
mv "$marker_tmp" "$marker"
marker_tmp=""

count="$(wc -l < "$entries" | tr -d ' ')"
echo "apex-sync-pull: installed $count object(s) from $remote; marker written to $marker"
