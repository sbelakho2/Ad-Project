#!/bin/sh
# Committed fake rclone for infrastructure/sync/tests/run-tests.sh (a test
# asset, not a product script). It backs the remote `name:path` syntax with a
# directory tree under FAKE_RCLONE_ROOT and supports the call shape push.sh
# and pull.sh use: global flags (--dry-run, --cache-dir DIR, --config FILE,
# --files-from FILE, --max-depth N, --dirs-only), then the command, then the
# operands. A `copy` with --files-from copies exactly the listed paths
# (relative to SOURCE), the way rclone's own --files-from does.
#
# Product scripts also use read-only preflight calls (listremotes, lsd, lsf)
# and a best-effort retention purge. Those never count as transfer calls:
# FAKE_RCLONE_COUNT records only `copy` and `copyto`, so the tests assert the
# data-plane call shape, not configuration checks.
#
# Fault injection and counting for the refusal and batching scenarios:
#   FAKE_RCLONE_FAIL     a substring; a call whose command line contains it,
#                        or a --files-from batch listing a matching object,
#                        exits 1 (a refused transfer)
#   FAKE_RCLONE_CORRUPT  a substring of the SOURCE; a matching download writes
#                        the object with its first byte changed and the size
#                        kept, so the pull side's sha256 check must refuse it
#   FAKE_RCLONE_COUNT    a file path; every accepted copy/copyto invocation
#                        appends one line to it, so the tests can assert
#                        invocation counts instead of wall-clock timings
#   FAKE_RCLONE_REMOTES  space-separated remote names reported by listremotes
#                        (default "object-store"), used to exercise a missing
#                        remote section
#   FAKE_RCLONE_BAD_CREDS  non-empty makes lsd/lsf fail like a rejected key
set -eu

root="${FAKE_RCLONE_ROOT:?FAKE_RCLONE_ROOT is required}"

usage() {
    echo "fake-rclone: usage: rclone [flags] <copy|copyto> SOURCE DEST" >&2
    exit 2
}

dry_run=""
files_from=""
lsf_dirs=""
command=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run)
            dry_run="yes"
            shift
            ;;
        --cache-dir|--config|--files-from|--max-depth)
            [ "$#" -ge 2 ] || usage
            if [ "$1" = "--files-from" ]; then
                files_from="$2"
            fi
            shift 2
            ;;
        --dirs-only)
            lsf_dirs="yes"
            shift
            ;;
        --*)
            shift
            ;;
        *)
            if [ -n "$command" ]; then
                break
            fi
            command="$1"
            shift
            ;;
    esac
done

src="${1:-}"
dst="${2:-}"
if [ -z "$command" ]; then
    usage
fi
case "$command" in
    listremotes) ;;
    lsd|lsf|purge)
        [ -n "$src" ] || usage
        ;;
    copy|copyto)
        [ -n "$src" ] && [ -n "$dst" ] || usage
        ;;
    *)
        usage
        ;;
esac

# Remote `name:path` maps under FAKE_RCLONE_ROOT; anything else is a local path.
to_local() {
    case "$1" in
        /*|./*|../*)
            printf '%s\n' "$1"
            ;;
        *:*)
            rest="${1#*:}"
            rest="${rest#/}"
            printf '%s/%s\n' "$root" "$rest"
            ;;
        *)
            printf '%s\n' "$1"
            ;;
    esac
}

remote_known() {
    name=${1%%:*}
    for known in ${FAKE_RCLONE_REMOTES:-object-store}; do
        [ "${known%:}" = "$name" ] && return 0
    done
    return 1
}

case "$command" in
    listremotes)
        for known in ${FAKE_RCLONE_REMOTES:-object-store}; do
            printf '%s:\n' "${known%:}"
        done
        exit 0
        ;;
    lsd)
        if [ -n "${FAKE_RCLONE_BAD_CREDS:-}" ]; then
            echo "fake-rclone: injected credential refusal: lsd $src" >&2
            exit 1
        fi
        if ! remote_known "$src"; then
            echo "fake-rclone: didn't find section in config file (\"${src%%:*}\")" >&2
            exit 1
        fi
        printf '          -1 2026-01-01 00:00:00        -1 apex-sync\n'
        exit 0
        ;;
    lsf)
        if [ -n "${FAKE_RCLONE_BAD_CREDS:-}" ]; then
            echo "fake-rclone: injected credential refusal: lsf $src" >&2
            exit 1
        fi
        if ! remote_known "$src"; then
            echo "fake-rclone: didn't find section in config file (\"${src%%:*}\")" >&2
            exit 1
        fi
        src_local="$(to_local "$src")"
        if [ ! -d "$src_local" ]; then
            echo "fake-rclone: directory not found: $src" >&2
            exit 3
        fi
        if [ -n "$lsf_dirs" ]; then
            find "$src_local" -mindepth 1 -maxdepth 1 -type d -exec sh -c '
                for path do printf "%s/\n" "${path##*/}"; done
            ' sh {} +
        else
            find "$src_local" -type f -exec sh -c '
                base=$1; shift
                for path do printf "%s\n" "${path#"$base"/}"; done
            ' sh "$src_local" {} +
        fi
        exit 0
        ;;
    purge)
        if [ -n "${FAKE_RCLONE_BAD_CREDS:-}" ]; then
            echo "fake-rclone: injected credential refusal: purge $src" >&2
            exit 1
        fi
        src_local="$(to_local "$src")"
        rm -rf "$src_local"
        exit 0
        ;;
esac

if [ -n "${FAKE_RCLONE_COUNT:-}" ]; then
    printf '%s %s -> %s\n' "$command" "$src" "$dst" >> "$FAKE_RCLONE_COUNT"
fi

if [ -n "${FAKE_RCLONE_FAIL:-}" ]; then
    case "$command $src $dst" in
        *"$FAKE_RCLONE_FAIL"*)
            echo "fake-rclone: injected transfer refusal: $command $src -> $dst" >&2
            exit 1
            ;;
    esac
fi

if [ -n "$dry_run" ]; then
    echo "fake-rclone: dry run: $command $src -> $dst" >&2
    exit 0
fi

src_local="$(to_local "$src")"
dst_local="$(to_local "$dst")"

corrupt_if_wanted() {
    if [ -n "${FAKE_RCLONE_CORRUPT:-}" ]; then
        case "$2" in
            *"$FAKE_RCLONE_CORRUPT"*)
                if [ -s "$1" ]; then
                    printf 'X' | dd of="$1" bs=1 seek=0 conv=notrunc 2>/dev/null
                fi
                ;;
        esac
    fi
}

case "$command" in
    copy)
        if [ -n "$files_from" ]; then
            if [ ! -d "$src_local" ]; then
                echo "fake-rclone: copy source not found: $src" >&2
                exit 3
            fi
            mkdir -p "$dst_local"
            while IFS= read -r rel; do
                [ -n "$rel" ] || continue
                if [ -n "${FAKE_RCLONE_FAIL:-}" ]; then
                    case "$src/$rel" in
                        *"$FAKE_RCLONE_FAIL"*)
                            echo "fake-rclone: injected transfer refusal: copy $src/$rel -> $dst/$rel" >&2
                            exit 1
                            ;;
                    esac
                fi
                if [ ! -f "$src_local/$rel" ]; then
                    echo "fake-rclone: copy source not found: $src/$rel" >&2
                    exit 3
                fi
                mkdir -p "$dst_local/$(dirname -- "$rel")"
                cp "$src_local/$rel" "$dst_local/$rel"
                corrupt_if_wanted "$dst_local/$rel" "$src/$rel"
            done < "$files_from"
        elif [ -d "$src_local" ]; then
            mkdir -p "$dst_local"
            (cd "$src_local" && find . -type f -print) | while IFS= read -r rel; do
                rel="${rel#./}"
                mkdir -p "$dst_local/$(dirname -- "$rel")"
                cp "$src_local/$rel" "$dst_local/$rel"
            done
        elif [ -f "$src_local" ]; then
            mkdir -p "$dst_local"
            cp "$src_local" "$dst_local/$(basename -- "$src_local")"
        else
            echo "fake-rclone: copy source not found: $src" >&2
            exit 3
        fi
        ;;
    copyto)
        if [ ! -f "$src_local" ]; then
            echo "fake-rclone: copyto source not found: $src" >&2
            exit 3
        fi
        mkdir -p "$(dirname -- "$dst_local")"
        cp "$src_local" "$dst_local"
        corrupt_if_wanted "$dst_local" "$src"
        ;;
    *)
        echo "fake-rclone: unsupported command: $command" >&2
        exit 2
        ;;
esac

exit 0
