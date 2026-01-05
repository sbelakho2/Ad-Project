#!/usr/bin/env bash
set -euo pipefail

# Idempotently enable Transparency API and ensure required tables exist on Hetzner prod.
# Runs on the app server (46.62.252.96) because that host has access to DATABASE_URL.

KEY="${APEX_HETZNER_SSH_KEY:-$HOME/.ssh/hetzner-db-mac}"
APP_HOST="46.62.252.96"

ssh -i "$KEY" "root@$APP_HOST" bash -s <<'BASH'
set -euo pipefail
cd /opt/adproject

# Ensure Transparency API is enabled (default is enabled in code, but we keep env aligned)
if grep -q '^TRANSPARENCY_API_ENABLED=' .env; then
  sed -i 's/^TRANSPARENCY_API_ENABLED=.*/TRANSPARENCY_API_ENABLED=true/' .env
else
  printf '\nTRANSPARENCY_API_ENABLED=true\n' >> .env
fi

# Extract DATABASE_URL (without printing it)
DATABASE_URL=$(python3 - <<'PY'
import re
from pathlib import Path
text=Path('.env').read_text(encoding='utf-8', errors='replace')
m=re.search(r'^DATABASE_URL=(.*)$', text, flags=re.M)
if not m:
    raise SystemExit(2)
v=m.group(1).strip()
if v[:1] == chr(34) and v[-1:] == chr(34):
    v=v[1:-1]
print(v)
PY
)

# Ensure gen_random_uuid available
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -Atc "CREATE EXTENSION IF NOT EXISTS pgcrypto;" >/dev/null

# Ensure team tables exist
TEAM_EXISTS=$(psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -Atc "select to_regclass('public.team_members') is not null;" | tr -d '\r\n')
if [[ "$TEAM_EXISTS" == "f" ]]; then
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f backend/migrations/035_team_and_settings.sql >/dev/null
fi

# Ensure transparency tables exist
T_EXISTS=$(psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -Atc "select to_regclass('public.transparency_auctions') is not null;" | tr -d '\r\n')
if [[ "$T_EXISTS" == "f" ]]; then
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f backend/migrations/postgres/20251201_090000_transparency_tables.up.sql >/dev/null
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f backend/migrations/postgres/20251201_134500_partition_transparency_tables.up.sql >/dev/null
  psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f backend/migrations/postgres/20251201_135600_transparency_staging_tables.up.sql >/dev/null
fi

# Restart backend so env/schema changes take effect (production uses PM2)
if command -v pm2 >/dev/null 2>&1 && pm2 describe backend >/dev/null 2>&1; then
  pm2 restart backend >/dev/null
else
  docker compose -f docker-compose.hetzner.yml up -d --force-recreate backend >/dev/null
fi

echo 'OK: transparency enabled; team+transparency tables ensured; backend restarted'
BASH
