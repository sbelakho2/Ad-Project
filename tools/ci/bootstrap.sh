#!/bin/sh
# Version preflight for native Kiwi jobs. It never installs silently: it
# either proves the exact tool is present or prints the pinned install command.
set -eu

need_rust=0
need_zola=0
need_live=0
need_cov=0
for arg in "$@"; do
  case "$arg" in
    rust) need_rust=1 ;;
    site) need_zola=1 ;;
    live) need_live=1 ;;
    coverage) need_cov=1 ;;
    all) need_rust=1; need_zola=1; need_live=1; need_cov=1 ;;
    *) echo "bootstrap: unknown requirement '$arg'" >&2; exit 2 ;;
  esac
done

fail=0
version_at_least() {
  # Sort -V: the first line is the smaller version.
  [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -n 1)" = "$2" ]
}

check_minimum() {
  command_name=$1
  minimum=$2
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "bootstrap: $command_name is missing (need >= $minimum)" >&2
    fail=1
    return
  fi
  raw=$("$command_name" --version 2>/dev/null | head -n 1)
  version=$(printf '%s' "$raw" | sed -n 's/.*[^0-9]\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\).*/\1/p')
  version=${version:-0.0.0}
  if ! version_at_least "$version" "$minimum"; then
    echo "bootstrap: $command_name has '$raw', need >= $minimum" >&2
    fail=1
  fi
}

check_prefix() {
  command_name=$1
  want=$2
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "bootstrap: $command_name is missing (need $want)" >&2
    fail=1
    return
  fi
  have=$("$command_name" --version 2>/dev/null | head -n 1)
  case "$have" in
    *"$want"*) ;;
    *) echo "bootstrap: $command_name has '$have', need prefix '$want'" >&2; fail=1 ;;
  esac
}

if [ "$need_rust" = 1 ]; then
  check_minimum rustc 1.88.0
  check_minimum cargo 1.88.0
fi
if [ "$need_zola" = 1 ]; then
  check_prefix zola 0.22
fi
if [ "$need_live" = 1 ]; then
  if ! command -v clickhouse >/dev/null 2>&1; then
    echo "bootstrap: clickhouse is missing; install the pinned server/client build" >&2
    fail=1
  fi
fi
if [ "$need_cov" = 1 ]; then
  if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
    echo "bootstrap: cargo-llvm-cov is missing; run: cargo install cargo-llvm-cov --version 0.9.1 --locked" >&2
    fail=1
  fi
fi
if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "bootstrap: required native tools present"
