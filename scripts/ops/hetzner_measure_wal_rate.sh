#!/usr/bin/env bash
set -euo pipefail

DURATION_SECONDS="${1:-120}"

SSH_KEY="${SSH_KEY:-$HOME/.ssh/hetzner-db-mac}"
APP_HOST="${APP_HOST:-46.62.252.96}"
DB_HOST="${DB_HOST:-77.42.44.35}"
DB_NAME="${DB_NAME:-adproject}"

ssh_db() {
  ssh -T \
    -i "$SSH_KEY" \
    -o BatchMode=yes \
    -o ConnectTimeout=10 \
    -o ServerAliveInterval=10 \
    -o ServerAliveCountMax=3 \
    -J "root@$APP_HOST" \
    "root@$DB_HOST" "$@"
}

echo "== pg_settings (archiving/WAL) =="
ssh_db "sudo -u postgres psql -d '$DB_NAME' -P pager=off -At -F '|' -c \"select name, setting, coalesce(unit,''), coalesce(source,''), coalesce(sourcefile,'') from pg_settings where name in ('archive_mode','archive_command','archive_timeout','wal_level','wal_compression','wal_segment_size','checkpoint_timeout','max_wal_size','min_wal_size','checkpoint_completion_target') order by name;\"" || true

echo

echo "== pg_stat_wal delta over ${DURATION_SECONDS}s =="
STAT_SQL="select extract(epoch from clock_timestamp())::bigint, wal_bytes, wal_records from pg_stat_wal;"
A="$(ssh_db "sudo -u postgres psql -d '$DB_NAME' -P pager=off -At -F '|' -c \"$STAT_SQL\"")"
sleep "$DURATION_SECONDS"
B="$(ssh_db "sudo -u postgres psql -d '$DB_NAME' -P pager=off -At -F '|' -c \"$STAT_SQL\"")"

python3 - <<PY
import sys

def parse(row: str):
    parts = row.strip().split('|')
    if len(parts) != 3:
        raise SystemExit(f"unexpected pg_stat_wal row: {row!r}")
    t, b, r = map(int, parts)
    return t, b, r

t1, b1, r1 = parse("""$A""")
t2, b2, r2 = parse("""$B""")
dt = t2 - t1
if dt <= 0:
    raise SystemExit(f"bad dt={dt} from A={t1} B={t2}")

db = b2 - b1
if db < 0:
    raise SystemExit(f"wal_bytes went backwards? db={db}")

dr = r2 - r1

mib = db / 1024 / 1024

gib_per_day = (db / 1024 / 1024 / 1024) * (86400 / dt)

print(f"A={t1}|{b1}|{r1}")
print(f"B={t2}|{b2}|{r2}")
print(f"delta_seconds={dt}")
print(f"wal_mib_in_window={mib:.2f}")
print(f"wal_records_in_window={dr}")
print(f"projected_gib_per_day={gib_per_day:.2f}")
PY

echo

echo "== archived WAL files created in last ${DURATION_SECONDS}s =="
MINUTES=$(( (DURATION_SECONDS + 59) / 60 ))
# This is a coarse proxy (mtime-based); pg_stat_wal above is the authoritative rate.
ARCH_RAW="$({
    ssh_db bash -s -- "$MINUTES" <<'BASH'
set -euo pipefail
MINUTES="$1"
c=$(find /var/backups/postgresql/wal -type f -name '0000*' -mmin -"$MINUTES" | wc -l | tr -d ' ')
b=$(find /var/backups/postgresql/wal -type f -name '0000*' -mmin -"$MINUTES" -print0 | xargs -0 -r stat -c '%s' | awk '{s+=$1} END {print s+0}')
echo "$c|$b"
BASH
} 2>/dev/null || true)"

if [[ -n "$ARCH_RAW" && "$ARCH_RAW" == *'|'* ]]; then
    c="${ARCH_RAW%%|*}"
    b="${ARCH_RAW#*|}"
    echo "files_last_${MINUTES}min=${c}"
    awk -v b="$b" -v m="$MINUTES" 'BEGIN { printf("mib_last_%dmin=%.2f\n", m, b/1024/1024) }'
fi
