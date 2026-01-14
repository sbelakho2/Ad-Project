#!/usr/bin/env python3
"""Print non-sensitive fingerprints of selected env vars.

- Shows full values only for non-secret identifiers (e.g., URLs, IDs).
- For secrets, prints: kind (jwt/key), prefix (first 8), last4, length.

Usage:
  python3 print_env_key_fingerprints.py /path/to/.env
"""

from __future__ import annotations

from pathlib import Path
import re
import sys


PUBLIC_KEYS = {
    "API_URL",
    "PUBLISHER_ID",
    "CONSOLE_BASE_URL",
    "CONTROL_PLANE_BASE_URL",
}

SECRET_KEYS = {
    "SDK_KEY",
    "API_KEY",
    "TOKEN",
    "AUTH_TOKEN",
    "BEARER_TOKEN",
}


def read_kv(text: str, key: str) -> str | None:
    m = re.search(rf"^(?:export\s+)?{re.escape(key)}=(.*)$", text, flags=re.M)
    if not m:
        return None
    v = m.group(1).strip()
    if (v.startswith('"') and v.endswith('"')) or (v.startswith("'") and v.endswith("'")):
        v = v[1:-1]
    return v


def fingerprint(v: str) -> str:
    prefix = v[:8]
    last4 = v[-4:]
    is_jwt = v.startswith("eyJ") and v.count(".") == 2
    kind = "jwt" if is_jwt else "key"
    return f"{kind} prefix={prefix} last4={last4} len={len(v)}"


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: print_env_key_fingerprints.py /path/to/.env", file=sys.stderr)
        return 2

    env_path = Path(argv[1])
    if not env_path.exists():
        print(f"missing {env_path}")
        return 0

    text = env_path.read_text(encoding="utf-8", errors="replace")

    keys = sorted(PUBLIC_KEYS | SECRET_KEYS)
    print(f"# {env_path}")
    for key in keys:
        v = read_kv(text, key)
        if v is None or v == "":
            print(f"{key}=missing")
            continue

        if key in PUBLIC_KEYS:
            print(f"{key}={v}")
        else:
            print(f"{key}={fingerprint(v)}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
