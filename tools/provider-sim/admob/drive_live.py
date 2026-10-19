#!/usr/bin/env python3
"""Drive the real `apex-runner` and the real AdMob connector against the
stateful simulator in this directory.

This is a live integration harness, not a unit test. It:

1. starts `server.py` on a free loopback port;
2. writes a real AES-256-GCM vault bundle + validated connection metadata so
   the real `apex-runner` registry builds a live AdMob adapter;
3. pre-creates the two ClickHouse tables whose `DateTime64` TTL the schema
   builder emits (24.8 rejects it) so a real ClickHouse can accept events;
4. runs `apex-runner audit`, `cycle`, and `daily`, recording output;
5. injects each simulator fault and runs `audit` again to observe the client's
   retry/error behavior;
6. exercises a live mediation-floor write through the real `AdmobClient`
   (compiled in a throwaway Cargo project) and reads the value back;
7. saves the simulator state, request log, signed ledger, and ClickHouse event
   counts as evidence.

Requires: Python `cryptography` (vault encryption only), a built
`rust/target/debug/apex-runner`, and (optionally) a reachable ClickHouse. The
server itself is standard library only.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

try:
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
except ImportError:  # live harness only
    AESGCM = None

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
SERVER = HERE / "server.py"
RUNNER = REPO / "rust" / "target" / "debug" / "apex-runner"
ACCOUNT = "pub-1234567890123456"
PUBLISHER = "northstar"
VAULT_KEY = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
SIGNING_KEY = "1111111111111111111111111111111111111111111111111111111111111111"
UNIT = "ca-app-pub-1234567890123456/4444444444"

DOMAIN_EVENTS_DDL = """CREATE TABLE IF NOT EXISTS {db}.domain_events (
 event_id String,
 event_type LowCardinality(String),
 schema_version UInt16,
 occurred_at_utc DateTime64(3, 'UTC'),
 received_at_utc DateTime64(3, 'UTC'),
 version UInt64 DEFAULT toUnixTimestamp64Milli(received_at_utc),
 environment LowCardinality(String),
 service_version String,
 source LowCardinality(String),
 publisher_id Nullable(String),
 platform LowCardinality(Nullable(String)),
 object_type Nullable(String),
 object_id Nullable(String),
 correlation_id Nullable(String),
 causation_id Nullable(String),
 outcome Nullable(String),
 reason_code Nullable(String),
 currency LowCardinality(Nullable(String)),
 amount_micros Nullable(Int64),
 period Nullable(String),
 lead_id Nullable(String),
 account_id Nullable(String),
 job_id Nullable(String),
 experiment_id Nullable(String),
 change_id Nullable(String),
 proposal_id Nullable(String),
 invoice_id Nullable(String),
 incident_id Nullable(String),
 platform_operation Nullable(String),
 finality LowCardinality(Nullable(String)),
 rule_version Nullable(String),
 snapshot_digest Nullable(String),
 guard_type Nullable(String),
 payload_json String
) ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMM(occurred_at_utc)
ORDER BY (source, event_id)
SETTINGS index_granularity = 8192"""

REQUEST_LOGS_DDL = """CREATE TABLE IF NOT EXISTS {db}.request_logs (
 event_id String,
 occurred_at_utc DateTime64(3, 'UTC'),
 received_at_utc DateTime64(3, 'UTC'),
 route_template LowCardinality(String),
 status UInt16,
 status_class LowCardinality(String),
 bot_class LowCardinality(String),
 referrer_host Nullable(String),
 campaign_code Nullable(String),
 query_keys Array(String),
 environment LowCardinality(String)
) ENGINE = ReplacingMergeTree
PARTITION BY toYYYYMM(occurred_at_utc)
ORDER BY (route_template, occurred_at_utc, event_id)"""


def run(cmd, env=None, timeout=180):
    started = time.time()
    proc = subprocess.run(
        cmd,
        cwd=REPO,
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    return {
        "command": [str(part) for part in cmd],
        "exit": proc.returncode,
        "elapsed_s": round(time.time() - started, 2),
        "stdout": proc.stdout,
        "stderr": proc.stderr,
    }


def start_sim(workdir: Path):
    state_file = workdir / "sim-state.json"
    proc = subprocess.Popen(
        [
            sys.executable,
            str(SERVER),
            "--port",
            "0",
            "--account",
            ACCOUNT,
            "--state-file",
            str(state_file),
        ],
        cwd=REPO,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    line = proc.stdout.readline()
    if not line:
        raise RuntimeError(f"simulator did not start: {proc.stderr.read()}")
    banner = json.loads(line)
    return proc, banner


def stop_sim(proc):
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()


def write_vault(vault_root: Path, base_url: str, token: str, refresh_token=None):
    if AESGCM is None:
        raise RuntimeError("the `cryptography` package is required for the vault")
    vault_root.mkdir(parents=True, exist_ok=True)
    account = {
        "developer_site": "example.com",
        "expected_networks": ["admob.com", "mintegral.com"],
        "ad_unit_ids": [],
        "bidder_accounts": [],
        "platforms": {
            "admob": {
                "api_key": token,
                "account_id": ACCOUNT,
                "base_url": base_url,
                "report_key": None,
                "reporting_base_url": None,
                "refresh_token": refresh_token,
            }
        },
    }
    raw = json.dumps(account, separators=(",", ":")).encode()
    nonce = secrets.token_bytes(12)
    ciphertext = AESGCM(bytes.fromhex(VAULT_KEY)).encrypt(nonce, raw, None)
    (vault_root / f"{PUBLISHER}.blob").write_text(
        nonce.hex() + "." + ciphertext.hex(), encoding="utf-8"
    )
    (vault_root / f"{PUBLISHER}.connections.json").write_text(
        json.dumps({"platforms": {"admob": {"state": "validated"}}}),
        encoding="utf-8",
    )


def prepare_clickhouse(ch_url: str, user: str, password: str, db: str):
    """Pre-create the two DateTime64-TTL tables without TTL.

    `apex-analytics` emits `TTL occurred_at_utc + ...` on a DateTime64 column,
    which ClickHouse 24.8 rejects (BAD_TTL_EXPRESSION). `CREATE TABLE IF NOT
    EXISTS` short-circuits once the table exists, so a TTL-free table of the
    same shape lets the real schema setup and inserts proceed. This is a
    workaround for an out-of-scope schema defect, recorded in the report.
    """
    headers = {"X-ClickHouse-User": user, "X-ClickHouse-Key": password}
    statements = [
        f"CREATE DATABASE IF NOT EXISTS {db}",
        DOMAIN_EVENTS_DDL.format(db=db),
        REQUEST_LOGS_DDL.format(db=db),
    ]
    results = []
    for statement in statements:
        request = urllib.request.Request(
            ch_url, data=statement.encode(), headers=headers, method="POST"
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                results.append({"ok": True, "status": response.status})
        except urllib.error.HTTPError as error:
            results.append(
                {"ok": False, "status": error.code, "body": error.read().decode()}
            )
    return results


def runner_env(workdir: Path, banner, ch_url, ch_user, ch_password, ch_db):
    env = dict(os.environ)
    env.update(
        {
            "APEX_CONNECTIONS_ROOT": str(workdir / "vault"),
            "APEX_VAULT_KEY_HEX": VAULT_KEY,
            "APEX_RUNNER_LEDGER_ROOT": str(workdir / "ledger"),
            "APEX_RUNNER_SIGNING_KEY_HEX": SIGNING_KEY,
            "APEX_RUNNER_SNAPSHOT": str(workdir / "snapshots"),
            "APEX_RUNNER_WRITABLE": "updateMediationGroup",
            "APEX_RUNNER_DATA_DIR": str(workdir / "data"),
            "APEX_PROPOSALS_ROOT": str(workdir / "proposals"),
            "APEX_METRICS_TEXTFILE": str(workdir / "runner.prom"),
        }
    )
    if "APEX_ADMOB_CLIENT_ID" in env and "APEX_ADMOB_CLIENT_SECRET" in env:
        env.setdefault(
            "APEX_ADMOB_OAUTH_TOKEN_URL", banner["base_url"].rstrip("/") + "/token"
        )
    if ch_url:
        env.update(
            {
                "APEX_CLICKHOUSE_HTTP": ch_url,
                "APEX_CLICKHOUSE_DB": ch_db,
                "APEX_CLICKHOUSE_USER": ch_user,
                "APEX_CLICKHOUSE_PASSWORD": ch_password,
            }
        )
    else:
        for name in (
            "APEX_CLICKHOUSE_HTTP",
            "APEX_CLICKHOUSE_DB",
            "APEX_CLICKHOUSE_USER",
            "APEX_CLICKHOUSE_PASSWORD",
        ):
            env.pop(name, None)
    return env


def admin(base_url, path, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    request = urllib.request.Request(
        base_url + path,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST" if data is not None else "GET",
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.loads(response.read().decode())


def inject(base_url, target, kind, **extra):
    payload = {"target": target, "kind": kind, **extra}
    return admin(base_url, "/__admin/faults", payload)


def run_runner(workdir, banner, env, command, *args, timeout=180):
    return run([str(RUNNER), command, *args], env=env, timeout=timeout)


def write_probe_project(workdir: Path):
    project = workdir / "probe"
    (project / "src").mkdir(parents=True, exist_ok=True)
    (project / "Cargo.toml").write_text(
        f"""[package]
name = "admob-probe"
version = "0.1.0"
edition = "2021"

[dependencies]
apex-connectors = {{ path = "{REPO / 'rust' / 'crates' / 'apex-connectors'}" }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
""",
        encoding="utf-8",
    )
    (project / "src" / "main.rs").write_text(
        """use std::sync::Arc;
use apex_connectors::admob::AdmobClient;
use apex_connectors::transport::{ReqwestTransport, RetryTransport, TokioSleeper, Transport};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = std::env::args().nth(1).expect("base url");
    let token = std::env::args().nth(2).expect("access token");
    let unit = std::env::args().nth(3).expect("ad unit");
    let inner = Arc::new(ReqwestTransport::new()?);
    let transport: Arc<dyn Transport> = Arc::new(RetryTransport::new(inner, 2, Arc::new(TokioSleeper)));
    let client = AdmobClient::new(&token, "pub-1234567890123456", &base, transport)?;
    println!("before={:?}", client.list_mediation_groups().await?);
    println!("updated={:?}", client.update_mapping_ecpm("g1", &unit, 1_750_000).await?);
    for group in client.list_mediation_groups().await? {
        if group.mediation_group_id == "g1" {
            for mapping in group.mappings {
                if mapping.ad_unit_id == unit {
                    println!("after_ecpm_micros={}", mapping.ecpm);
                }
            }
        }
    }
    println!(
        "report_micros={}",
        client.mediation_revenue("2026-09-01", "2026-10-18").await?
    );
    Ok(())
}
""",
        encoding="utf-8",
    )
    return project


def clickhouse_query(ch_url, user, password, db, query):
    headers = {"X-ClickHouse-User": user, "X-ClickHouse-Key": password}
    request = urllib.request.Request(
        ch_url + f"/?database={db}", data=query.encode(), headers=headers, method="POST"
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.read().decode()
    except urllib.error.HTTPError as error:
        return f"ERROR {error.code}: {error.read().decode()}"


def main(argv=None):
    global RUNNER
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workdir", default="/tmp/apex-admob-live")
    parser.add_argument("--runner", default=str(RUNNER))
    parser.add_argument("--clickhouse", default=os.environ.get("APEX_CLICKHOUSE_HTTP", ""))
    parser.add_argument("--clickhouse-user", default=os.environ.get("APEX_CLICKHOUSE_USER", "apex"))
    parser.add_argument(
        "--clickhouse-password", default=os.environ.get("APEX_CLICKHOUSE_PASSWORD", "apex")
    )
    parser.add_argument("--clickhouse-db", default="apex_admob_sim")
    parser.add_argument("--skip-faults", action="store_true")
    parser.add_argument("--keep-sim", action="store_true")
    args = parser.parse_args(argv)

    RUNNER = Path(args.runner)
    workdir = Path(args.workdir)
    evidence = workdir / "evidence"
    if workdir.exists():
        shutil.rmtree(workdir)
    for name in ("vault", "ledger", "snapshots", "data", "proposals", "evidence"):
        (workdir / name).mkdir(parents=True, exist_ok=True)

    result = {"simulator": {}, "commands": {}, "faults": {}, "probe": {}, "clickhouse": {}}
    ch_results = []
    if args.clickhouse:
        ch_results = prepare_clickhouse(
            args.clickhouse,
            args.clickhouse_user,
            args.clickhouse_password,
            args.clickhouse_db,
        )
    result["clickhouse"]["precreate"] = ch_results

    sim_proc, banner = start_sim(workdir)
    result["simulator"]["banner"] = banner
    try:
        base = banner["base_url"]
        write_vault(workdir / "vault", base, banner["access_token"], banner.get("refresh_token"))
        env = runner_env(
            workdir,
            banner,
            args.clickhouse,
            args.clickhouse_user,
            args.clickhouse_password,
            args.clickhouse_db,
        )

        audit = run_runner(workdir, banner, env, "audit", "--json")
        (evidence / "audit.json").write_text(audit["stdout"] + audit["stderr"])
        result["commands"]["audit"] = audit

        cycle = run_runner(workdir, banner, env, "cycle", "--json")
        (evidence / "cycle.json").write_text(cycle["stdout"] + cycle["stderr"])
        result["commands"]["cycle"] = cycle
        result["simulator"]["after_cycle"] = admin(base, "/__admin/state")

        daily = run_runner(workdir, banner, env, "daily", "--json")
        (evidence / "daily.json").write_text(daily["stdout"] + daily["stderr"])
        result["commands"]["daily"] = daily

        probe_project = write_probe_project(workdir)
        probe = run(
            [
                "cargo",
                "run",
                "-q",
                "--manifest-path",
                str(probe_project / "Cargo.toml"),
                "--",
                base,
                banner["access_token"],
                UNIT,
            ],
            env={**os.environ, "CARGO_TARGET_DIR": str(REPO / "rust" / "target")},
            timeout=420,
        )
        (evidence / "probe.txt").write_text(probe["stdout"] + probe["stderr"])
        result["probe"] = probe
        result["simulator"]["after_probe"] = admin(base, "/__admin/state")

        if not args.skip_faults:
            scenarios = [
                ("401_token_expiry", "groups.list", "401", {}),
                ("429_retry_after", "groups.list", "429", {"count": 1, "retry_after": 1}),
                ("500_retry", "groups.list", "500", {"count": 1}),
                ("timeout_retry", "groups.list", "timeout", {"count": 1, "seconds": 11}),
                ("malformed_json", "report", "malformed_json", {"count": 1}),
                (
                    "groups_pagination_truncation",
                    "groups.list",
                    "pagination_truncation",
                    {"count": 1},
                ),
                (
                    "report_pagination_truncation",
                    "report",
                    "pagination_truncation",
                    {"count": 1},
                ),
            ]
            for name, target, kind, extra in scenarios:
                inject(base, target, kind, **extra)
                outcome = run_runner(workdir, banner, env, "audit", "--json", timeout=180)
                (evidence / f"fault-{name}.json").write_text(
                    outcome["stdout"] + outcome["stderr"]
                )
                result["faults"][name] = outcome

            # Token expiry is stateful: expire the bootstrap token, prove the
            # 401, exchange the refresh token, rewrite the vault and recover.
            admin(base, "/__admin/expire-tokens", {"all": True})
            expired = run_runner(workdir, banner, env, "audit", "--json")
            result["faults"]["token_expiry_state"] = expired
            request = urllib.request.Request(
                base + "/oauth2/token",
                data=(
                    "grant_type=refresh_token&refresh_token="
                    + banner["refresh_token"]
                    + "&client_id="
                    + banner["client_id"]
                    + "&client_secret="
                    + banner["client_secret"]
                ).encode(),
                method="POST",
            )
            with urllib.request.urlopen(request, timeout=30) as response:
                refreshed = json.loads(response.read().decode())
            result["faults"]["refresh_token_exchange"] = {
                "token_type": refreshed["token_type"],
                "expires_in": refreshed["expires_in"],
                "access_token_prefix": refreshed["access_token"][:12],
            }
            write_vault(workdir / "vault", base, refreshed["access_token"], banner.get("refresh_token"))
            recovered = run_runner(workdir, banner, env, "audit", "--json")
            result["faults"]["token_refresh_recovered"] = recovered

        result["simulator"]["final_state"] = admin(base, "/__admin/state")
        result["simulator"]["requests"] = admin(base, "/__admin/requests")
        (evidence / "sim-state.json").write_text(
            json.dumps(result["simulator"]["final_state"], indent=2)
        )
        (evidence / "sim-requests.json").write_text(
            json.dumps(result["simulator"]["requests"], indent=2)
        )
        ledger = workdir / "ledger" / f"{PUBLISHER}.jsonl"
        if ledger.is_file():
            shutil.copy2(ledger, evidence / "ledger.jsonl")
            result["ledger"] = {
                "path": str(ledger),
                "lines": sum(1 for _ in ledger.open()),
            }

        if args.clickhouse:
            result["clickhouse"]["events"] = clickhouse_query(
                args.clickhouse,
                args.clickhouse_user,
                args.clickhouse_password,
                args.clickhouse_db,
                "SELECT event_type, count() FROM domain_events GROUP BY event_type ORDER BY event_type FORMAT TSV",
            )
            result["clickhouse"]["tables"] = clickhouse_query(
                args.clickhouse,
                args.clickhouse_user,
                args.clickhouse_password,
                args.clickhouse_db,
                "SHOW TABLES FORMAT TSV",
            )
    finally:
        if not args.keep_sim:
            stop_sim(sim_proc)
        else:
            result["simulator"]["pid"] = sim_proc.pid

    (workdir / "summary.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
