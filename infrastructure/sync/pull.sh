#!/bin/sh
set -eu
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
#   outcomes/*  -> APEX_SYNC_IMPORTS_DIR (default .apex/imports), where the
#                  outcome log lands for apex-sales to read through
#                  APEX_SALES_OUTCOMES_FILE (set to
#                  .apex/imports/outcomes.jsonl in the control host's
#                  sales.env). The destination is the sync unit's own import
#                  directory: .apex/sales is owned by the apex-sales user, so
#                  a pull into it would fail on every connection, payment,
#                  sign-up or unsubscribe.
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
# Atomic generations and installs: manifest entries may name their payload
# with an "object" key under runs/<run-id>/ (the layout push.sh writes); such
# entries are downloaded by that exact key, so a later push cannot mutate the
# bytes of a manifest already fetched. Legacy entries without "object"
# resolve to their "path" key and keep working. Installation is two-phase:
# every staged file is copied to a temporary name in its destination
# directory, then the verified files are renamed into place. A copy failure
# therefore removes the temporaries and leaves every destination untouched
# (a rename is atomic, so a reader never sees a truncated file).
#
# Exit codes: 0 success, 2 configuration error (missing or malformed
# APEX_SYNC_REMOTE, remote absent from the rclone configuration, failed
# credential check, missing rclone or python3, unknown argument), 1 download,
# manifest, verification or install failure. Any nonzero exit fails the
# apex-sync-pull unit.

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
outcomes_dir="${APEX_SYNC_OUTCOMES_DIR:-.apex/imports}"
marker="${APEX_CONTROL_SYNC_MARKER:-.apex/control/sync-marker.json}"

# systemd LoadCredential= exposes the rclone configuration through
# CREDENTIALS_DIRECTORY as a service-readable file, so the deployed
# /etc/apex/rclone.conf may stay root-owned mode 0600. Manual runs keep using
# RCLONE_CONFIG (or rclone's default location).
if [ -n "${CREDENTIALS_DIRECTORY:-}" ] && [ -r "$CREDENTIALS_DIRECTORY/rclone.conf" ]; then
    RCLONE_CONFIG="$CREDENTIALS_DIRECTORY/rclone.conf"
    export RCLONE_CONFIG
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/apex-sync-pull.XXXXXX")"
install_temps=""
marker_tmp=""
cleanup_temps() {
    [ -n "$install_temps" ] && [ -f "$install_temps" ] || return 0
    while IFS="$(printf '\t')" read -r tmp dest; do
        [ -n "$tmp" ] && rm -f "$tmp"
    done < "$install_temps"
}
cleanup() {
    cleanup_temps
    rm -rf "$work"
    if [ -n "$marker_tmp" ]; then
        rm -f "$marker_tmp"
    fi
}
trap cleanup EXIT INT TERM
stage="$work/stage"
cache="$work/rclone-cache"
mkdir -p "$stage" "$cache"

rclone_base() {
    rclone --cache-dir "$cache" "$@"
}

# Configuration validation before the first read or write. A missing remote
# section, a remote syntax rclone would treat as a local path, or rejected
# credentials are configuration errors (exit 2); nothing lands in a
# destination.
case "$remote" in
    *:*) ;;
    *)
        echo "apex-sync-pull: APEX_SYNC_REMOTE must be <remote>:<bucket>[/<prefix>], got: $remote" >&2
        exit 2
        ;;
esac
remote_name="${remote%%:*}"
remote_path="${remote#*:}"
case "$remote_name" in
    ''|[!A-Za-z0-9]*|*[!A-Za-z0-9._-]*)
        echo "apex-sync-pull: invalid rclone remote name in APEX_SYNC_REMOTE: $remote_name" >&2
        exit 2
        ;;
esac
if [ -z "$remote_path" ]; then
    echo "apex-sync-pull: APEX_SYNC_REMOTE names no bucket/prefix: $remote" >&2
    exit 2
fi

if ! rclone_base listremotes 2>/dev/null | grep -Fxq "$remote_name:"; then
    echo "apex-sync-pull: rclone remote '$remote_name' is not in the configuration named by RCLONE_CONFIG" >&2
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
            echo "apex-sync-pull: credential check failed for $remote" >&2
            exit 2
            ;;
    esac
fi

# The manifest is fetched first; it is the last object push.sh publishes, so
# a fetched manifest names a payload generation that is already complete.
rclone_base copyto "$remote/manifest.json" "$work/manifest.json" || {
    echo "apex-sync-pull: could not download manifest.json from $remote" >&2
    exit 1
}

entries="$work/entries.tsv"
families="$work/families.txt"
APEX_SYNC_MANIFEST="$work/manifest.json" APEX_SYNC_ENTRIES="$entries" APEX_SYNC_FAMILIES="$families" python3 - <<'PY'
import json
import os
import re

with open(os.environ["APEX_SYNC_MANIFEST"], encoding="utf-8") as handle:
    manifest = json.load(handle)

files = manifest.get("files")
if not isinstance(files, list) or not files:
    raise SystemExit("apex-sync-pull: manifest carries no files list")

segment = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
order = ("ledger", "invoices", "leads", "outcomes")
seen = []
rows = []
invoices = 0
outcomes = 0
logical_seen = set()
object_seen = set()
for entry in files:
    path = str(entry.get("path", ""))
    size = entry.get("size")
    digest = str(entry.get("sha256", ""))
    obj = entry.get("object", path)
    parts = path.split("/")
    if len(parts) != 2 or parts[0] not in order:
        raise SystemExit(f"apex-sync-pull: unsupported manifest key: {path!r}")
    if not all(segment.match(part) for part in parts):
        raise SystemExit(f"apex-sync-pull: unsafe manifest key: {path!r}")
    if not isinstance(size, int) or size < 0 or not re.fullmatch(r"[0-9a-f]{64}", digest):
        raise SystemExit(f"apex-sync-pull: malformed manifest entry: {path!r}")
    if not isinstance(obj, str) or not obj:
        raise SystemExit(f"apex-sync-pull: malformed manifest object for: {path!r}")
    if obj == path:
        # Legacy manifest from a push before run prefixes: the payload key is
        # the logical key.
        pass
    else:
        obj_parts = obj.split("/")
        if (
            len(obj_parts) != 4
            or obj_parts[0] != "runs"
            or not all(segment.match(part) for part in obj_parts)
            or obj_parts[2:] != parts
        ):
            raise SystemExit(f"apex-sync-pull: unsupported manifest object: {obj!r}")
    if path in logical_seen:
        raise SystemExit(f"apex-sync-pull: duplicate manifest key: {path!r}")
    if obj in object_seen:
        raise SystemExit(f"apex-sync-pull: duplicate manifest object: {obj!r}")
    logical_seen.add(path)
    object_seen.add(obj)
    if parts[0] == "invoices":
        invoices += 1
    if parts[0] == "outcomes":
        outcomes += 1
    if parts[0] not in seen:
        seen.append(parts[0])
    rows.append((path, obj, size, digest))

if invoices != 1:
    raise SystemExit(f"apex-sync-pull: expected exactly one invoices object, found {invoices}")
if outcomes > 1:
    raise SystemExit(f"apex-sync-pull: expected at most one outcomes object, found {outcomes}")

with open(os.environ["APEX_SYNC_ENTRIES"], "w", encoding="utf-8") as out:
    for path, obj, size, digest in rows:
        out.write(f"{path}\t{obj}\t{size}\t{digest}\n")
with open(os.environ["APEX_SYNC_FAMILIES"], "w", encoding="utf-8") as out:
    out.write("|".join(family for family in order if family in seen))
PY

# Stage every object in one batched transfer; nothing lands in a destination
# yet. --files-from carries exactly the manifest's payload keys, so the
# transferred set is the recorded set, never a directory walk of unlisted
# objects, and the file count never multiplies the rclone calls.
files_from="$work/files-from.txt"
cut -f2 "$entries" > "$files_from"
rclone_base copy --files-from "$files_from" "$remote" "$stage" || {
    echo "apex-sync-pull: staged download failed; nothing installed" >&2
    exit 1
}

# Verify the staged copies against the manifest. Any mismatch ends the run
# here, before an install and before any marker write.
APEX_SYNC_MANIFEST="$work/manifest.json" APEX_SYNC_ENTRIES="$entries" APEX_SYNC_STAGE="$stage" python3 - <<'PY'
import hashlib
import os
import sys

stage = os.environ["APEX_SYNC_STAGE"]
problems = []
count = 0
with open(os.environ["APEX_SYNC_ENTRIES"], encoding="utf-8") as handle:
    for line in handle:
        if not line.strip():
            continue
        path, obj, size, digest = line.rstrip("\n").split("\t")
        count += 1
        try:
            with open(os.path.join(stage, obj), "rb") as payload:
                data = payload.read()
        except OSError as error:
            problems.append(f"{path}: cannot read the staged object {obj} ({error})")
            continue
        if len(data) != int(size):
            problems.append(f"{path}: size {len(data)} does not match manifest size {size}")
        elif hashlib.sha256(data).hexdigest() != digest:
            problems.append(f"{path}: sha256 does not match the manifest")
if problems:
    for problem in problems:
        print(f"apex-sync-pull: {problem}", file=sys.stderr)
    raise SystemExit("apex-sync-pull: refusing to install unverified objects")
print(f"apex-sync-pull: verified {count} object(s) against the manifest")
PY

# Reserve the marker temp before touching a destination. Creating it here
# proves the marker directory is writable; a permission failure aborts with
# every destination still untouched. The final rename stays last.
mkdir -p "$(dirname -- "$marker")"
marker_tmp="$(mktemp "$marker.tmp.XXXXXX")"

# Build the install plan from the verified logical keys. The payload key and
# the logical key share the trailing family/name, validated above.
install_plan="$work/install.tsv"
: > "$install_plan"
while IFS="$(printf '\t')" read -r path obj size digest; do
    : "$size" "$digest"
    family="${path%%/*}"
    name="${path#*/}"
    case "$family" in
        ledger) dest="$ledger_root/$name" ;;
        invoices) dest="$invoice_register" ;;
        leads) dest="$imports_dir/$name" ;;
        outcomes) dest="$outcomes_dir/$name" ;;
        *)
            echo "apex-sync-pull: unsupported install family: $family" >&2
            exit 1
            ;;
    esac
    printf '%s\t%s\n' "$stage/$obj" "$dest" >> "$install_plan"
done < "$entries"

# Phase one: copy every verified file to a temporary name in its destination
# directory. A failure here leaves all destinations byte-for-byte untouched.
install_temps="$work/install-temps.tsv"
: > "$install_temps"
while IFS="$(printf '\t')" read -r src dest; do
    dest_dir="$(dirname -- "$dest")"
    if [ -d "$dest" ]; then
        echo "apex-sync-pull: refusing to replace the directory at $dest" >&2
        exit 1
    fi
    mkdir -p "$dest_dir"
    tmp="$(mktemp "$dest_dir/.apex-sync.XXXXXX")" || {
        echo "apex-sync-pull: cannot stage an install temp beside $dest" >&2
        exit 1
    }
    if ! cp -p "$src" "$tmp"; then
        rm -f "$tmp"
        echo "apex-sync-pull: could not copy the verified object for $dest" >&2
        exit 1
    fi
    # mktemp creates 0600 and cp -p mirrors the staging mode; pin the
    # deployment data mode so group "apex" readers keep access.
    chmod 0660 "$tmp"
    printf '%s\t%s\n' "$tmp" "$dest" >> "$install_temps"
done < "$install_plan"

# Phase two: rename the verified temps into place. Rename is atomic within the
# destination directory, and only now can a reader see the new generation.
while IFS="$(printf '\t')" read -r tmp dest; do
    if ! mv -f "$tmp" "$dest"; then
        echo "apex-sync-pull: could not install $dest" >&2
        exit 1
    fi
done < "$install_temps"
: > "$install_temps"

# The marker: written atomically (temp file plus rename) and only now, after
# a complete verified install.
sync_families="$(cat "$families")"
now_ms=$(( $(date +%s) * 1000 ))
printf '{"synced_at_unix_ms":%s,"source":"%s"}\n' "$now_ms" "$sync_families" > "$marker_tmp"
chmod 0640 "$marker_tmp"
mv "$marker_tmp" "$marker"
marker_tmp=""

count="$(wc -l < "$entries" | tr -d ' ')"
echo "apex-sync-pull: installed $count object(s) from $remote; marker written to $marker"
