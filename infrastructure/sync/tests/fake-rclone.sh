#!/bin/sh
# Committed fake rclone for infrastructure/sync/tests/run-tests.sh (a test
# asset, not a product script). It backs the remote `name:path` syntax with a
# directory tree under FAKE_RCLONE_ROOT and supports the call shape push.sh
# and pull.sh use: global flags (--dry-run, --cache-dir DIR, --config FILE,
# --files-from FILE), then `copy` or `copyto`, then SOURCE DEST. A `copy`
# with --files-from copies exactly the listed paths (relative to SOURCE), the
# way rclone's own --files-from does.
#
# Fault injection and counting for the refusal and batching scenarios:
#   FAKE_RCLONE_FAIL     a substring; a call whose command line contains it,
#                        or a --files-from batch listing a matching object,
#                        exits 1 (a refused transfer)
#   FAKE_RCLONE_CORRUPT  a substring of the SOURCE; a matching download writes
#                        the object with its first byte changed and the size
#                        kept, so the pull side's sha256 check must refuse it
#   FAKE_RCLONE_COUNT    a file path; every accepted invocation appends one
#                        line to it, so the tests can assert invocation
#                        counts instead of wall-clock timings
set -eu

root="${FAKE_RCLONE_ROOT:?FAKE_RCLONE_ROOT is required}"

usage() {
    echo "fake-rclone: usage: rclone [--dry-run] [--cache-dir DIR] [--config FILE] [--files-from FILE] <copy|copyto> SOURCE DEST" >&2
    exit 2
}

dry_run=""
files_from=""
command=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run)
            dry_run="yes"
            shift
            ;;
        --cache-dir|--config)
            [ "$#" -ge 2 ] || usage
            shift 2
            ;;
        --files-from)
            [ "$#" -ge 2 ] || usage
            files_from="$2"
            shift 2
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
if [ -z "$command" ] || [ -z "$src" ] || [ -z "$dst" ]; then
    usage
fi

if [ -n "${FAKE_RCLONE_COUNT:-}" ]; then
    printf '%s %s -> %s\n' "$command" "$src" "$dst" >> "$FAKE_RCLONE_COUNT"
fi

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
