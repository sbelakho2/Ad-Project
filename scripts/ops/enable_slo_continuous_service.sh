#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "$0")/../.." && pwd)
UNIT_SRC="$ROOT_DIR/scripts/ops/systemd/slo-continuous.service.example"

if [[ ! -f "$UNIT_SRC" ]]; then
  echo "[err] missing unit template: $UNIT_SRC" >&2
  exit 1
fi

# Upload unit content to test server and enable service
cat "$UNIT_SRC" | "$ROOT_DIR/scripts/ops/hetzner_ssh.sh" test -- "sudo tee /etc/systemd/system/slo-continuous.service >/dev/null"

cat <<'BASH' | "$ROOT_DIR/scripts/ops/hetzner_ssh.sh" test -- bash -s
set -euo pipefail

if [[ ! -f /opt/adproject/.env.slo ]]; then
  echo "[warn] /opt/adproject/.env.slo is missing. Create it before enabling the service." >&2
fi

sudo systemctl daemon-reload
sudo systemctl enable --now slo-continuous.service
sudo systemctl status slo-continuous.service --no-pager -n 10
BASH
