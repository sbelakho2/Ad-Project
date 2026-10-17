#!/bin/sh
# Apex sync push: one-way upload from the console host to the S3-compatible
# Object Storage bucket the control host pulls from. Runs as the ExecStart of
# apex-sync-push.service with the environment file /etc/apex/sync.env; see
# infrastructure/README.md, "Hosting split", for the inventory.
#
# Uploads (rclone is the runtime dependency; the bucket credentials stay in
# the rclone configuration named by RCLONE_CONFIG, never on the command line):
#   ledger/<name>       the files under APEX_SYNC_LEDGER_ROOT
#                       (default .apex/ledgers), flat <publisher>.jsonl files
#   invoices/<name>     APEX_SYNC_INVOICE_REGISTER
#                       (default .apex/billing/invoices.jsonl)
#   leads/<name>        APEX_SYNC_LEADS_FILE (default .apex/leads.jsonl)
#   leads/signin.jsonl  the sign-in log beside the leads file, when present
#   manifest.json       uploaded LAST: size and sha256 for every object above,
#                       so the pull side can refuse a corrupted or partial
#                       copy (pull.sh verifies before installing anything).
#
# Source families that do not exist yet are skipped with a note on stdout;
# when no family exists at all the run fails, so a typo in the sync.env paths
# cannot pass silently.
#
# Exit codes: 0 success, 2 configuration error (missing APEX_SYNC_REMOTE,
# missing rclone, python3, sha256sum or shasum, unknown argument), 1 upload
# or manifest failure. Any nonzero exit fails the apex-sync-push unit.
#
# --dry-run prints the rclone actions and uploads nothing.
set -eu

usage() {
    echo "usage: apex-sync-push [--dry-run]" >&2
    exit 2
}

dry_run=""
if [ "$#" -gt 0 ]; then
    if [ "$#" -eq 1 ] && [ "$1" = "--dry-run" ]; then
        dry_run="yes"
    else
        usage
    fi
fi

if [ -z "${APEX_SYNC_REMOTE:-}" ]; then
    echo "apex-sync-push: APEX_SYNC_REMOTE is not set (rclone remote syntax, for example object-store:apex-sync)" >&2
    exit 2
fi

if ! command -v rclone >/dev/null 2>&1; then
    echo "apex-sync-push: rclone is not on PATH" >&2
    exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "apex-sync-push: python3 is not on PATH (the manifest step needs it)" >&2
    exit 2
fi

remote="${APEX_SYNC_REMOTE%/}"
ledger_root="${APEX_SYNC_LEDGER_ROOT:-.apex/ledgers}"
invoice_register="${APEX_SYNC_INVOICE_REGISTER:-.apex/billing/invoices.jsonl}"
leads_file="${APEX_SYNC_LEADS_FILE:-.apex/leads.jsonl}"
signin_file="$(dirname -- "$leads_file")/signin.jsonl"

work="$(mktemp -d "${TMPDIR:-/tmp}/apex-sync-push.XXXXXX")"
cleanup() {
    rm -rf "$work"
}
trap cleanup EXIT INT TERM
cache="$work/rclone-cache"
mkdir -p "$cache"

# The rclone call shape is pinned here so the tests can shim it: global flags
# first, then copy or copyto, then the source and destination operands.
run_rclone() {
    if [ -n "$dry_run" ]; then
        rclone --dry-run --cache-dir "$cache" "$@"
    else
        rclone --cache-dir "$cache" "$@"
    fi
}

files_tsv="$work/files.tsv"
: > "$files_tsv"

if [ -d "$ledger_root" ]; then
    run_rclone copy "$ledger_root" "$remote/ledger"
    find "$ledger_root" -type f | sort | while IFS= read -r path; do
        name="${path#"$ledger_root"/}"
        printf 'ledger/%s\t%s\n' "$name" "$path"
    done >> "$files_tsv"
else
    echo "apex-sync-push: ledger root not present, skipping: $ledger_root"
fi

if [ -f "$invoice_register" ]; then
    name="$(basename -- "$invoice_register")"
    run_rclone copyto "$invoice_register" "$remote/invoices/$name"
    printf 'invoices/%s\t%s\n' "$name" "$invoice_register" >> "$files_tsv"
else
    echo "apex-sync-push: invoice register not present, skipping: $invoice_register"
fi

# The leads family uploads in ONE batched transfer: the leads file and the
# sign-in log beside it are named with --files-from relative to their shared
# directory, so the file count never multiplies the rclone calls.
leads_batch="$work/leads-batch.txt"
: > "$leads_batch"
if [ -f "$leads_file" ]; then
    name="$(basename -- "$leads_file")"
    printf '%s\n' "$name" >> "$leads_batch"
    printf 'leads/%s\t%s\n' "$name" "$leads_file" >> "$files_tsv"
fi
if [ -f "$signin_file" ]; then
    printf 'signin.jsonl\n' >> "$leads_batch"
    printf 'leads/signin.jsonl\t%s\n' "$signin_file" >> "$files_tsv"
fi
if [ -s "$leads_batch" ]; then
    run_rclone copy --files-from "$leads_batch" "$(dirname -- "$leads_file")" "$remote/leads"
else
    echo "apex-sync-push: leads file not present, skipping: $leads_file"
fi

if [ ! -s "$files_tsv" ]; then
    echo "apex-sync-push: no sync sources exist (ledger root, invoice register, leads file); check the APEX_SYNC_* paths in /etc/apex/sync.env" >&2
    exit 1
fi

# The manifest: one entry per uploaded object, in the exact remote key the
# pull side downloads. It is uploaded last, so a pull that sees a new
# manifest can rely on the payload objects being already in place. The
# per-file hashes come from ONE batched sha256sum/shasum pass over the source
# list; sizes come from stat, so python never re-reads a payload file.
manifest="$work/manifest.json"
paths_list="$work/paths.txt"
hashes_file="$work/hashes.txt"
cut -f2 "$files_tsv" > "$paths_list"
if command -v sha256sum >/dev/null 2>&1; then
    tr '\n' '\0' < "$paths_list" | xargs -0 sha256sum > "$hashes_file"
elif command -v shasum >/dev/null 2>&1; then
    tr '\n' '\0' < "$paths_list" | xargs -0 shasum -a 256 > "$hashes_file"
else
    echo "apex-sync-push: neither sha256sum nor shasum is on PATH (the manifest step needs one)" >&2
    exit 2
fi
APEX_SYNC_FILES_TSV="$files_tsv" APEX_SYNC_HASHES="$hashes_file" APEX_SYNC_MANIFEST="$manifest" python3 - <<'PY'
import json
import os
import time

with open(os.environ["APEX_SYNC_FILES_TSV"], encoding="utf-8") as handle:
    keys_paths = [line.rstrip("\n").split("\t", 1) for line in handle if line.strip()]
with open(os.environ["APEX_SYNC_HASHES"], encoding="utf-8") as handle:
    hash_lines = [line.rstrip("\n") for line in handle if line.strip()]
if len(keys_paths) != len(hash_lines):
    raise SystemExit(
        "apex-sync-push: hashed %d file(s) for %d manifest entr(ies)"
        % (len(hash_lines), len(keys_paths))
    )

entries = []
for (key, path), hash_line in zip(keys_paths, hash_lines):
    digest = hash_line.split(" ", 1)[0]
    if digest.startswith("\\"):
        # GNU sha256sum prefixes the line for file names with a backslash.
        digest = digest[1:]
    if len(digest) != 64:
        raise SystemExit("apex-sync-push: unexpected hash output: %r" % hash_line[:120])
    entries.append(
        {
            "path": key,
            "size": os.path.getsize(path),
            "sha256": digest,
        }
    )

manifest = {
    "created_at_unix_ms": int(time.time() * 1000),
    "files": entries,
}
with open(os.environ["APEX_SYNC_MANIFEST"], "w", encoding="utf-8") as out:
    json.dump(manifest, out, indent=2, sort_keys=True)
    out.write("\n")
PY

run_rclone copyto "$manifest" "$remote/manifest.json"

count="$(wc -l < "$files_tsv" | tr -d ' ')"
if [ -n "$dry_run" ]; then
    echo "apex-sync-push: dry run: would upload $count object(s) and the manifest to $remote"
else
    echo "apex-sync-push: uploaded $count object(s) and the manifest to $remote"
fi
