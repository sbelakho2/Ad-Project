#!/usr/bin/env bash
set -euo pipefail

# Source key=value pairs from an .env file into the current shell (exported).
# Comments (# ...) and blank lines are ignored. Lines without '=' are skipped.
# Usage: source scripts/ops/source_env.sh infrastructure/production/.env.backend

FILE="${1:-}"
if [[ -z "$FILE" ]]; then
  echo "[err] usage: $0 <path-to-.env>" >&2
  exit 2
fi
if [[ ! -f "$FILE" ]]; then
  echo "[err] env file not found: $FILE" >&2
  exit 1
fi

while IFS= read -r line || [[ -n "$line" ]]; do
  # Trim leading/trailing whitespace safely
  # remove leading spaces
  line="${line#${line%%[![:space:]]*}}"
  # remove trailing spaces
  line="${line%${line##*[![:space:]]}}"
  # Ignore comments and blanks
  [[ -z "$line" ]] && continue
  [[ "$line" =~ ^# ]] && continue
  # Only process KEY=VALUE lines
  if [[ "$line" == *"="* ]]; then
    key="${line%%=*}"
    val="${line#*=}"
    # Remove possible quotes around values
    val="${val%$'\r'}"
    val="${val%$'\n'}"
    val="${val%\"}"
    val="${val#\"}"
    val="${val%\'}"
    val="${val#\'}"
    export "$key"="$val"
  fi
done < "$FILE"

echo "[ok] exported env from $FILE"
