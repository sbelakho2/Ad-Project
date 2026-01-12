#!/usr/bin/env python3
"""Restart PM2 backend with DATABASE_URL sourced from /opt/adproject/.env.

Avoids shell-quoting issues and never prints credentials.

Steps:
- Read DATABASE_URL from /opt/adproject/.env
- Run: pm2 restart backend --update-env, with that DATABASE_URL in the pm2 CLI env
- Print sanitized target from pm2 env after restart
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Optional
from urllib.parse import urlparse


def _sanitize(db_url: str) -> str:
    u = urlparse(db_url)
    dbname = (u.path or "").lstrip("/")
    return f"host={u.hostname} port={u.port} db={dbname or '<none>'}"


def _read_env_database_url(path: Path) -> str:
    text = path.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^(?:export\s+)?DATABASE_URL=(.*)$", text, flags=re.M)
    if not m:
        raise RuntimeError(f"DATABASE_URL not found in {path}")
    raw = m.group(1).strip()
    if raw.startswith('"') and raw.endswith('"') and len(raw) >= 2:
        raw = raw[1:-1]
    return raw.strip()


def _run(cmd: list[str], env: Optional[dict[str, str]] = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False, env=env)


def _pm2_backend_id() -> Optional[int]:
    p = _run(["pm2", "jlist"])
    if p.returncode != 0:
        return None
    try:
        entries = json.loads(p.stdout or "[]")
    except Exception:
        return None

    for e in entries:
        if str(e.get("name")) == "backend":
            try:
                return int(e.get("pm_id"))
            except Exception:
                return None
    return None


def _database_url_from_pm2_env(pm_id: int) -> Optional[str]:
    p = _run(["pm2", "env", str(pm_id)])
    if p.returncode != 0:
        return None
    m = re.search(r"^DATABASE_URL\s*:\s*(.+)$", p.stdout, flags=re.M)
    if not m:
        return None
    return m.group(1).strip().strip('"')


def main() -> int:
    env_file = Path("/opt/adproject/.env")
    db_url = _read_env_database_url(env_file)

    child_env = dict(**{k: v for k, v in dict(**subprocess.os.environ).items()})
    child_env["DATABASE_URL"] = db_url

    p = _run(["pm2", "restart", "backend", "--update-env"], env=child_env)
    # Print pm2 output, but it shouldn't include secrets.
    if p.stdout:
        sys.stdout.write(p.stdout)
    if p.stderr:
        sys.stderr.write(p.stderr)

    pm_id = _pm2_backend_id()
    if pm_id is None:
        print("pm2 backend id not found")
        return 1

    pm2_db_url = _database_url_from_pm2_env(pm_id)
    if not pm2_db_url:
        print("pm2 DATABASE_URL missing after restart")
        return 1

    print(f"pm2 backend DATABASE_URL now: {_sanitize(pm2_db_url)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
