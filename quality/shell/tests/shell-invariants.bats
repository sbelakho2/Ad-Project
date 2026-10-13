// Adversarial shell contract tests (dogfood: these run in scripts-shell-coverage.yml under kcov).
// They verify repo-wide shell invariants WITHOUT executing deploy scripts.

setup() {
  ROOT="${BATS_TEST_DIRNAME}/../../.."
}

@test "every entry script declares set -eu (or set -euo pipefail) early" {
  skipped="infrastructure/hetzner/"
  fail_count=0
  while IFS= read -r script; do
    case "$script" in "$skipped"*) continue ;; esac
    if ! head -c 2000 "$script" | grep -Eq 'set[[:space:]]+-eu(o[[:space:]]+pipefail)?'; then
      echo "missing set -eu: $script"
      fail_count=$((fail_count + 1))
    fi
  done < <(find "$ROOT/scripts" "$ROOT/tools" "$ROOT/infrastructure" -name '*.sh' -type f | sort)
  [ "$fail_count" -eq 0 ]
}

@test "destructive scripts honor DOGFOOD_DRY_RUN guard" {
  fail_count=0
  while IFS= read -r script; do
    if grep -Eq 'deploy|kubectl|pm2|systemctl|terraform|helm|migrate' "$script" && ! grep -q 'DOGFOOD_DRY_RUN' "$script"; then
      echo "destructive without DOGFOOD_DRY_RUN: $script"
      fail_count=$((fail_count + 1))
    fi
  done < <(find "$ROOT/scripts" -name '*.sh' -type f | sort)
  [ "$fail_count" -eq 0 ]
}
