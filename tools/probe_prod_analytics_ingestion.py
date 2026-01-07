#!/usr/bin/env python3
from __future__ import annotations

import argparse
import os
import re
import subprocess
from pathlib import Path
from urllib.parse import unquote, urlparse


def read_database_url(env_path: Path) -> str:
    text = env_path.read_text(encoding="utf-8", errors="replace")
    m = re.search(r"^DATABASE_URL=(.*)$", text, flags=re.M)
    if not m:
        raise SystemExit(f"DATABASE_URL not found in {env_path}")
    raw = m.group(1).strip()
    if (raw.startswith('"') and raw.endswith('"')) or (raw.startswith("'") and raw.endswith("'")):
        raw = raw[1:-1]
    return raw


def psql_query(database_url: str, sql: str) -> str:
    url = urlparse(database_url)
    user = unquote(url.username or "")
    password = unquote(url.password or "")
    host = url.hostname or "localhost"
    port = str(url.port or 5432)
    dbname = (url.path or "/").lstrip("/")

    env = dict(os.environ)
    env["PGPASSWORD"] = password

    cmd = [
        "psql",
        "-h",
        host,
        "-p",
        port,
        "-U",
        user,
        "-d",
        dbname,
        "-At",
        "-c",
        sql,
    ]
    res = subprocess.run(cmd, env=env, check=True, capture_output=True, text=True)
    return res.stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description="Probe prod analytics ingestion freshness for a publisher")
    parser.add_argument("--publisher-id", required=True)
    parser.add_argument("--env", default="/opt/adproject/backend/.env")
    args = parser.parse_args()

    database_url = read_database_url(Path(args.env))
    pid = args.publisher_id

    sql = f"""
SELECT 'server_now' AS k, now()::timestamptz AS v;

SELECT 'analytics_impressions_all' AS table, max(observed_at) AS max_observed_at,
       count(*) FILTER (WHERE observed_at > now() - interval '1 hour') AS last_1h
FROM analytics_impressions;
SELECT 'analytics_clicks_all' AS table, max(observed_at) AS max_observed_at,
       count(*) FILTER (WHERE observed_at > now() - interval '1 hour') AS last_1h
FROM analytics_clicks;
SELECT 'analytics_revenue_events_all' AS table, max(observed_at) AS max_observed_at,
       count(*) FILTER (WHERE observed_at > now() - interval '1 hour') AS last_1h
FROM analytics_revenue_events;

-- If ingestion uses staging tables, these can indicate stuck buffers.
SELECT 'analytics_clicks_stage' AS table, count(*) AS rows FROM analytics_clicks_stage;
SELECT 'analytics_revenue_events_stage' AS table, count(*) AS rows FROM analytics_revenue_events_stage;

SELECT 'analytics_impressions' AS table, max(observed_at) AS max_observed_at,
       count(*) FILTER (WHERE observed_at > now() - interval '1 hour') AS last_1h,
       count(*) FILTER (WHERE observed_at > now() - interval '24 hours') AS last_24h
FROM analytics_impressions WHERE publisher_id='{pid}';
SELECT 'analytics_clicks' AS table, max(observed_at) AS max_observed_at,
       count(*) FILTER (WHERE observed_at > now() - interval '1 hour') AS last_1h,
       count(*) FILTER (WHERE observed_at > now() - interval '24 hours') AS last_24h
FROM analytics_clicks WHERE publisher_id='{pid}';
SELECT 'analytics_revenue_events' AS table, max(observed_at) AS max_observed_at,
       count(*) FILTER (WHERE observed_at > now() - interval '1 hour') AS last_1h,
       count(*) FILTER (WHERE observed_at > now() - interval '24 hours') AS last_24h
FROM analytics_revenue_events WHERE publisher_id='{pid}';
"""

    out = psql_query(database_url, sql)
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
