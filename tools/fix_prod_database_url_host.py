#!/usr/bin/env python3
"""Fix DATABASE_URL hostname/port in prod .env files on the app server.

- Reads /opt/adproject/.env (and backend/.env if present)
- Rewrites DATABASE_URL to use a provided host/port while preserving user/pass/db/query
- Writes a .bak.<timestamp> backup next to the original
- Prints only sanitized targets (host/port/db)

Intended to be copied to the app server and executed there.
"""

from __future__ import annotations

import re
import time
from pathlib import Path
from typing import Optional
from urllib.parse import ParseResult, urlparse, urlunparse

DB_HOST = "77.42.44.35"
DB_PORT = 5432


def _sanitize(db_url: str) -> str:
    u = urlparse(db_url)
    dbname = (u.path or "").lstrip("/")
    return f"host={u.hostname} port={u.port} db={dbname or '<none>'}"


def _read_database_url(path: Path) -> Optional[str]:
    if not path.exists():
        return None
    text = path.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^(?:export\s+)?DATABASE_URL=(.*)$", text, flags=re.M)
    if not m:
        return None
    raw = m.group(1).strip()
    if raw.startswith('"') and raw.endswith('"') and len(raw) >= 2:
        raw = raw[1:-1]
    return raw.strip()


def _replace_database_url_line(text: str, new_url: str) -> str:
    def repl(match: re.Match[str]) -> str:
        prefix = match.group(1)
        return f"{prefix}\"{new_url}\""

    # Force a stable quoting style: DATABASE_URL="..."
    pattern = r"^((?:export\s+)?DATABASE_URL=)(.*)$"
    if not re.search(pattern, text, flags=re.M):
        raise RuntimeError("DATABASE_URL line not found")
    return re.sub(pattern, repl, text, flags=re.M)


def _rewrite_url(db_url: str) -> str:
    u: ParseResult = urlparse(db_url)
    if not u.scheme:
        raise ValueError("DATABASE_URL missing scheme")

    # Preserve username/password, dbname, query, fragment.
    username = u.username or ""
    password = u.password or ""

    auth = username
    if password:
        auth = f"{username}:{password}"

    if auth:
        netloc = f"{auth}@{DB_HOST}:{DB_PORT}"
    else:
        netloc = f"{DB_HOST}:{DB_PORT}"

    rewritten = u._replace(netloc=netloc)
    return urlunparse(rewritten)


def _fix_file(path: Path) -> bool:
    old = _read_database_url(path)
    if not old:
        print(f"{path}: DATABASE_URL missing; skipping")
        return False

    new = _rewrite_url(old)
    if _sanitize(old) == _sanitize(new):
        print(f"{path}: already ok ({_sanitize(old)})")
        return False

    text = path.read_text(encoding="utf-8", errors="replace")
    updated = _replace_database_url_line(text, new)

    ts = time.strftime("%Y%m%d-%H%M%S")
    bak = path.with_suffix(path.suffix + f".bak.{ts}")
    bak.write_text(text, encoding="utf-8")
    path.write_text(updated, encoding="utf-8")

    print(f"{path}: {_sanitize(old)} -> {_sanitize(new)}")
    print(f"{path}: backup={bak}")
    return True


def main() -> int:
    changed = False
    for p in [Path("/opt/adproject/.env"), Path("/opt/adproject/backend/.env")]:
        try:
            if _fix_file(p):
                changed = True
        except Exception as e:
            print(f"{p}: ERROR {e.__class__.__name__}: {e}")

    return 0 if changed else 0


if __name__ == "__main__":
    raise SystemExit(main())
