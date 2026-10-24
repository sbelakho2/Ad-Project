#!/bin/sh
set -eu
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
#   outcomes/outcomes.jsonl  APEX_SYNC_OUTCOMES_FILE (default
#                       .apex/sales/outcomes.jsonl)
#   manifest.json       uploaded LAST: size and sha256 for every object above,
#                       so the pull side can refuse a corrupted or partial
#                       copy (pull.sh verifies before installing anything).
#
# Atomic generations: payload objects are uploaded under an immutable
# runs/<run-id>/ prefix and manifest.json names them with its "object" field.
# manifest.json is the only mutable pointer and is replaced only after every
# payload object is uploaded and re-hashed. A transfer that fails midway
# therefore leaves the previous manifest and the previous run's objects
# untouched, so the control host can still reconstruct the last complete
# generation. The newest APEX_SYNC_RUNS_RETAIN (default 4) run prefixes are
# kept, so a pull that fetched the previous manifest still finds its objects;
# older prefixes are pruned best-effort after the new manifest lands.
#
# Source families that do not exist yet are skipped with a note on stdout;
# when no family exists at all the run fails, so a typo in the sync.env paths
# cannot pass silently. Object names are validated against the same closed
# grammar pull.sh enforces: refusing an unsafe or nested source name here
# stops a manifest the control host could never verify.
#
# Exit codes: 0 success, 2 configuration error (missing or malformed
# APEX_SYNC_REMOTE, remote absent from the rclone configuration, failed
# credential check, missing rclone or python3, unsafe source name, unknown
# argument), 1 upload or manifest failure. Any nonzero exit fails the
# apex-sync-push unit.
#
# --dry-run prints the rclone actions and uploads nothing (the credential
# check is read-only).


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
case "$ledger_root" in
    /) ;;
    *) ledger_root="${ledger_root%/}" ;;
esac
invoice_register="${APEX_SYNC_INVOICE_REGISTER:-.apex/billing/invoices.jsonl}"
leads_file="${APEX_SYNC_LEADS_FILE:-.apex/leads.jsonl}"
outcomes_file="${APEX_SYNC_OUTCOMES_FILE:-.apex/sales/outcomes.jsonl}"
signin_file="$(dirname -- "$leads_file")/signin.jsonl"

# systemd LoadCredential= exposes the rclone configuration through
# CREDENTIALS_DIRECTORY as a service-readable file, so the deployed
# /etc/apex/rclone.conf may stay root-owned mode 0600. Manual runs keep using
# RCLONE_CONFIG (or rclone's default location).
if [ -n "${CREDENTIALS_DIRECTORY:-}" ] && [ -r "$CREDENTIALS_DIRECTORY/rclone.conf" ]; then
    RCLONE_CONFIG="$CREDENTIALS_DIRECTORY/rclone.conf"
    export RCLONE_CONFIG
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/apex-sync-push.XXXXXX")"
cleanup() {
    rm -rf "$work"
}
trap cleanup EXIT INT TERM
cache="$work/rclone-cache"
mkdir -p "$cache"

# The rclone call shape is pinned here so the tests can shim it: global flags
# first, then the command, then the operands.
rclone_base() {
    rclone --cache-dir "$cache" "$@"
}
run_rclone() {
    if [ -n "$dry_run" ]; then
        rclone_base --dry-run "$@"
    else
        rclone_base "$@"
    fi
}

# Configuration validation. A missing remote section, a remote syntax that
# rclone would treat as a local path, or rejected credentials are all
# configuration errors (exit 2) and are detected before any upload starts.
case "$remote" in
    *:*) ;;
    *)
        echo "apex-sync-push: APEX_SYNC_REMOTE must be <remote>:<bucket>[/<prefix>], got: $remote" >&2
        exit 2
        ;;
esac
remote_name="${remote%%:*}"
remote_path="${remote#*:}"
case "$remote_name" in
    ''|[!A-Za-z0-9]*|*[!A-Za-z0-9._-]*)
        echo "apex-sync-push: invalid rclone remote name in APEX_SYNC_REMOTE: $remote_name" >&2
        exit 2
        ;;
esac
if [ -z "$remote_path" ]; then
    echo "apex-sync-push: APEX_SYNC_REMOTE names no bucket/prefix: $remote" >&2
    exit 2
fi

if ! rclone_base listremotes 2>/dev/null | grep -Fxq "$remote_name:"; then
    echo "apex-sync-push: rclone remote '$remote_name' is not in the configuration named by RCLONE_CONFIG" >&2
    exit 2
fi

preflight_err="$work/preflight.err"
: > "$preflight_err"
status=0
rclone_base lsd "$remote_name:" >/dev/null 2>"$preflight_err" || status=$?
if [ "$status" -ne 0 ]; then
    # A bucket-scoped credential may list its bucket but not all buckets; an
    # empty bucket may not exist yet. lsf separates those from a bad key.
    status=0
    rclone_base lsf --max-depth 1 "$remote" >/dev/null 2>>"$preflight_err" || status=$?
    case "$status" in
        0|3) ;;
        *)
            cat "$preflight_err" >&2
            echo "apex-sync-push: credential check failed for $remote" >&2
            exit 2
            ;;
    esac
fi

# The closed object-name grammar shared with pull.sh.
check_name() {
    name=$1
    case "$name" in
        [A-Za-z0-9]*) ;;
        *)
            echo "apex-sync-push: unsafe object name (must start with a letter or digit): $name" >&2
            exit 2
            ;;
    esac
    case "$name" in
        *[!A-Za-z0-9._-]*)
            echo "apex-sync-push: unsafe object name (only A-Z a-z 0-9 . _ - are allowed, files stay flat): $name" >&2
            exit 2
            ;;
    esac
}

# A timestamp plus the PID plus random bytes keeps concurrent consoles from
# choosing the same immutable prefix.
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$-$(od -An -N3 -tx1 /dev/urandom | tr -d ' \n')"
run_prefix="runs/$run_id"
files_tsv="$work/files.tsv"
: > "$files_tsv"

add_file() {
    printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$files_tsv"
}

ledger_batch="$work/ledger-batch.txt"
: > "$ledger_batch"
if [ -d "$ledger_root" ]; then
    ledger_paths="$work/ledger-paths.txt"
    find "$ledger_root" -type f | sort > "$ledger_paths"
    while IFS= read -r path; do
        name="${path#"$ledger_root"/}"
        check_name "$name"
        printf '%s\n' "$name" >> "$ledger_batch"
        add_file "ledger/$name" "$run_prefix/ledger/$name" "$path"
    done < "$ledger_paths"
    if [ ! -s "$ledger_batch" ]; then
        echo "apex-sync-push: ledger root is empty, skipping: $ledger_root"
    fi
else
    echo "apex-sync-push: ledger root not present, skipping: $ledger_root"
fi

invoice_name=""
if [ -f "$invoice_register" ]; then
    invoice_name="$(basename -- "$invoice_register")"
    check_name "$invoice_name"
    add_file "invoices/$invoice_name" "$run_prefix/invoices/$invoice_name" "$invoice_register"
else
    echo "apex-sync-push: invoice register not present, skipping: $invoice_register"
fi

# The leads family uploads in ONE batched transfer: the leads file and the
# sign-in log beside it are named with --files-from relative to their shared
# directory, so the file count never multiplies the rclone calls.
leads_batch="$work/leads-batch.txt"
: > "$leads_batch"
if [ -f "$leads_file" ]; then
    leads_name="$(basename -- "$leads_file")"
    check_name "$leads_name"
    if [ "$leads_name" = "signin.jsonl" ]; then
        echo "apex-sync-push: APEX_SYNC_LEADS_FILE must not be named signin.jsonl (that key is reserved for the sign-in log)" >&2
        exit 2
    fi
    printf '%s\n' "$leads_name" >> "$leads_batch"
    add_file "leads/$leads_name" "$run_prefix/leads/$leads_name" "$leads_file"
fi
if [ -f "$signin_file" ]; then
    check_name "signin.jsonl"
    printf 'signin.jsonl\n' >> "$leads_batch"
    add_file "leads/signin.jsonl" "$run_prefix/leads/signin.jsonl" "$signin_file"
fi
if [ ! -s "$leads_batch" ]; then
    echo "apex-sync-push: leads file not present, skipping: $leads_file"
fi
if [ -f "$outcomes_file" ]; then
    add_file "outcomes/outcomes.jsonl" "$run_prefix/outcomes/outcomes.jsonl" "$outcomes_file"
else
    echo "apex-sync-push: outcomes file not present, skipping: $outcomes_file"
fi

if [ ! -s "$files_tsv" ]; then
    echo "apex-sync-push: no sync sources exist (ledger root, invoice register, leads file); check the APEX_SYNC_* paths in /etc/apex/sync.env" >&2
    exit 1
fi

# Hash every source BEFORE the transfer. After the transfer the same files are
# hashed again; a source that changed in between aborts the run before the
# manifest pointer moves, so manifest.json never describes bytes the remote
# does not hold.
pre_hashes="$work/pre-hashes.tsv"
APEX_SYNC_FILES_TSV="$files_tsv" APEX_SYNC_PRE_HASHES="$pre_hashes" python3 - <<'PY'
import hashlib
import os

with open(os.environ["APEX_SYNC_FILES_TSV"], encoding="utf-8") as handle:
    rows = [line.rstrip("\n").split("\t", 2) for line in handle if line.strip()]

with open(os.environ["APEX_SYNC_PRE_HASHES"], "w", encoding="utf-8") as out:
    for key, obj, path in rows:
        digest = hashlib.sha256()
        try:
            with open(path, "rb") as handle:
                for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                    digest.update(chunk)
            size = os.path.getsize(path)
        except OSError as error:
            raise SystemExit(f"apex-sync-push: cannot read source {path}: {error}")
        out.write(f"{key}\t{obj}\t{size}\t{digest.hexdigest()}\n")
PY

# Upload the payload under this run's immutable prefix. Any failure exits
# before manifest.json is replaced; the previous generation stays complete.
if [ -s "$ledger_batch" ]; then
    run_rclone copy --files-from "$ledger_batch" "$ledger_root" "$remote/$run_prefix/ledger" || {
        echo "apex-sync-push: ledger upload failed; manifest left unchanged" >&2
        exit 1
    }
fi

if [ -n "$invoice_name" ]; then
    run_rclone copyto "$invoice_register" "$remote/$run_prefix/invoices/$invoice_name" || {
        echo "apex-sync-push: invoice upload failed; manifest left unchanged" >&2
        exit 1
    }
fi
if [ -f "$outcomes_file" ]; then
    run_rclone copyto "$outcomes_file" "$remote/$run_prefix/outcomes/outcomes.jsonl" || {
        echo "apex-sync-push: outcomes upload failed; manifest left unchanged" >&2
        exit 1
    }
fi

if [ -s "$leads_batch" ]; then
    run_rclone copy --files-from "$leads_batch" "$(dirname -- "$leads_file")" "$remote/$run_prefix/leads" || {
        echo "apex-sync-push: leads upload failed; manifest left unchanged" >&2
        exit 1
    }
fi

# Re-hash, compare with the pre-transfer snapshot, and only then write the
# manifest from the verified pre-transfer values.
manifest="$work/manifest.json"
APEX_SYNC_FILES_TSV="$files_tsv" APEX_SYNC_PRE_HASHES="$pre_hashes" APEX_SYNC_MANIFEST="$manifest" python3 - <<'PY'
import hashlib
import json
import os
import sys
import time

entries = []
with open(os.environ["APEX_SYNC_PRE_HASHES"], encoding="utf-8") as handle:
    for line in handle:
        if not line.strip():
            continue
        key, obj, size, digest = line.rstrip("\n").split("\t")
        entries.append({"path": key, "object": obj, "size": int(size), "sha256": digest})

# Map object back to its local source for the post-transfer check.
sources = {}
with open(os.environ["APEX_SYNC_FILES_TSV"], encoding="utf-8") as handle:
    for line in handle:
        if not line.strip():
            continue
        key, obj, path = line.rstrip("\n").split("\t", 2)
        sources[obj] = path

problems = []
for entry in entries:
    path = sources[entry["object"]]
    if path is None:
        problems.append(f"{entry['path']}: missing source mapping")
        continue
    digest = hashlib.sha256()
    try:
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
        size = os.path.getsize(path)
    except OSError as error:
        problems.append(f"{entry['path']}: source changed during transfer ({error})")
        continue
    if size != entry["size"] or digest.hexdigest() != entry["sha256"]:
        problems.append(f"{entry['path']}: source changed during transfer")
if problems:
    for problem in problems:
        print(f"apex-sync-push: {problem}", file=sys.stderr)
    raise SystemExit("apex-sync-push: refusing to publish a manifest over changed sources")

payload = {
    "created_at_unix_ms": int(time.time() * 1000),
    "files": entries,
}
with open(os.environ["APEX_SYNC_MANIFEST"], "w", encoding="utf-8") as out:
    json.dump(payload, out, indent=2, sort_keys=True)
    out.write("\n")
PY

run_rclone copyto "$manifest" "$remote/manifest.json" || {
    echo "apex-sync-push: manifest upload failed; the previous manifest is still live" >&2
    exit 1
}

# Best-effort retention: keep the newest run prefixes so a pull that fetched
# a recent manifest still finds its payload. A prune failure never fails a
# sync that already published a complete generation.
retain="${APEX_SYNC_RUNS_RETAIN:-4}"
case "$retain" in
    ''|*[!0-9]*) retain=4 ;;
esac
[ "$retain" -ge 1 ] || retain=1
if [ -z "$dry_run" ]; then
    if listing=$(rclone_base lsf --dirs-only "$remote/runs" 2>/dev/null); then
        seen=0
        for entry in $(printf '%s\n' "$listing" | sort -r); do
            seen=$((seen + 1))
            [ "$seen" -le "$retain" ] && continue
            old="${entry%/}"
            case "$old" in
                ''|*[!A-Za-z0-9._-]*) continue ;;
            esac
            rclone_base purge "$remote/runs/$old" >/dev/null 2>&1 || \
                echo "apex-sync-push: warning: could not prune runs/$old" >&2
        done
    fi
fi

count="$(wc -l < "$files_tsv" | tr -d ' ')"
if [ -n "$dry_run" ]; then
    echo "apex-sync-push: dry run: would upload $count object(s) under $run_prefix and the manifest to $remote"
else
    echo "apex-sync-push: uploaded $count object(s) under $run_prefix and the manifest to $remote"
fi
