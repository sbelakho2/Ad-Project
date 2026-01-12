#!/usr/bin/env python3
"""Print sanitized DATABASE_URL targets (host/port/db) from common prod env sources.

Never prints usernames/passwords/query params.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Optional
from urllib.parse import urlparse


def _sanitize_db_url(db_url: str) -> str:
    u = urlparse(db_url)
    dbname = (u.path or "").lstrip("/")
    return f"host={u.hostname} port={u.port} db={dbname or '<none>'}"


def _read_env_file(path: Path) -> Optional[str]:
    if not path.exists():
        return None
    text = path.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^(?:export\s+)?DATABASE_URL=(.*)$", text, flags=re.M)
    if not m:
        return None
    raw = m.group(1).strip()
    # Support common .env quoting; avoid complicated shell-escape behavior.
    if raw.startswith('"') and raw.endswith('"') and len(raw) >= 2:
        raw = raw[1:-1]
    return raw.strip()


def _run(cmd: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)


def _pm2_backend_id() -> Optional[int]:
    # Try to find a process that looks like the backend.
    p = _run(["pm2", "jlist"])
    if p.returncode != 0:
        return None
    try:
        entries = json.loads(p.stdout or "[]")
    except Exception:
        return None

    def score(entry: dict) -> int:
        name = str(entry.get("name") or "").lower()
        cwd = str((entry.get("pm2_env") or {}).get("pm_cwd") or "").lower()
        script = str((entry.get("pm2_env") or {}).get("pm_exec_path") or "").lower()
        s = 0
        if "backend" in name:
            s += 10
        if "/backend" in cwd:
            s += 5
        if "backend" in script:
            s += 3
        return s

    best = None
    best_score = -1
    for e in entries:
        try:
            sc = score(e)
            if sc > best_score:
                best_score = sc
                best = e
        except Exception:
            continue

    if not best or best_score <= 0:
        return None
    try:
        return int(best.get("pm_id"))
    except Exception:
        return None


def _pm2_backend_meta(pm_id: int) -> dict:
    p = _run(["pm2", "jlist"])
    if p.returncode != 0:
        return {}
    try:
        entries = json.loads(p.stdout or "[]")
    except Exception:
        return {}
    for e in entries:
        try:
            if int(e.get("pm_id")) != pm_id:
                continue
        except Exception:
            continue
        env = e.get("pm2_env") or {}
        return {
            "name": e.get("name"),
            "pm_cwd": env.get("pm_cwd"),
            "pm_exec_path": env.get("pm_exec_path"),
            "env_file": env.get("env_file"),
            "NODE_ENV": env.get("NODE_ENV"),
        }
    return {}


def _database_url_from_pm2_env(pm_id: int) -> Optional[str]:
    p = _run(["pm2", "env", str(pm_id)])
    if p.returncode != 0:
        return None
    # pm2 env output format: "KEY : value"
    m = re.search(r"^DATABASE_URL\s*:\s*(.+)$", p.stdout, flags=re.M)
    if not m:
        return None
    return m.group(1).strip().strip('"')


def main() -> int:
    sources: list[tuple[str, Optional[str]]] = []

    sources.append(("process.env.DATABASE_URL", os.environ.get("DATABASE_URL")))
    sources.append(("/opt/adproject/.env", _read_env_file(Path("/opt/adproject/.env"))))
    sources.append(("/opt/adproject/backend/.env", _read_env_file(Path("/opt/adproject/backend/.env"))))

    pm_id = _pm2_backend_id()
    if pm_id is None:
        sources.append(("pm2 backend (id)", None))
    else:
        meta = _pm2_backend_meta(pm_id)
        if meta:
            print("pm2 backend meta:")
            for k in ["name", "pm_cwd", "pm_exec_path", "env_file", "NODE_ENV"]:
                if k in meta:
                    print(f"  {k}={meta.get(k)}")
        sources.append((f"pm2 backend env (pm_id={pm_id})", _database_url_from_pm2_env(pm_id)))

    for label, value in sources:
        if not value:
            print(f"{label}: <missing>")
            continue
        try:
            print(f"{label}: {_sanitize_db_url(value)}")
        except Exception as e:
            print(f"{label}: <unparseable> ({e.__class__.__name__})")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
