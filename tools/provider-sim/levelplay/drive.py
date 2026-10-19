#!/usr/bin/env python3
"""Drive the real apex-runner against the LevelPlay simulator.

Phases
------
``baseline``
    Start the simulator in the documented wire shape (numeric ids,
    ``data``/``totalResults`` reporting envelope) and record the pre-patch
    connector failures: the reporting envelope and the instances fields.

``full``
    Faithful read audit (numeric ids, v1 reporting envelope), then a complete
    audited publisher in the compatibility id style the current autopilot
    adapter requires (string ids), with a real ``cycle`` write and a real
    ``daily`` report pull.  Every runner command opens and verifies the signed
    ledger; ClickHouse on 18172/19072 receives the daily analytics events.

``faults``
    One run per injected fault: 401 bearer expiry, 429 Retry-After, 500,
    timeout, malformed JSON, pagination truncation and a write-side 500.
    Each record includes the runner exit code, the simulator's request log and
    the before/after simulator state.

The runner is the compiled ``rust/target/debug/apex-runner``; the ledger is a
real ``apex-proof`` ed25519 file.  No product code is stubbed.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
RUNNER = REPO / "rust" / "target" / "debug" / "apex-runner"
SERVER = Path(__file__).resolve().parent / "server.py"
VAULT_KEY_HEX = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
SIGNING_SEED_HEX = "0707070707070707070707070707070707070707070707070707070707070707"
CLICKHOUSE_HTTP = "http://127.0.0.1:18172"
CLICKHOUSE_TCP = 19072
DEVELOPER_SITE = "playrix.com"
PUBLISHER = "levelplay-sim"


def free_port() -> int:
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    return port


def http_json(url, method="GET", payload=None, timeout=10):
    data = None
    headers = {"Accept": "application/json"}
    if payload is not None:
        data = json.dumps(payload).encode()
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, method=method, headers=headers)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        body = response.read()
        return json.loads(body) if body else None


def http_text(url, timeout=10):
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return response.read().decode("utf-8", "replace")


def wait_http(url, timeout=30):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        try:
            http_text(url, timeout=2)
            return True
        except Exception as error:  # noqa: BLE001 - polling a local service
            last = error
            time.sleep(0.1)
    raise RuntimeError(f"{url} did not become ready: {last}")


class Sim:
    def __init__(
        self,
        workdir: Path,
        tag: str,
        id_style: str,
        report_style: str,
        strict: bool,
        b_lift: float = 1.03,
        variant_group_id: str | None = None,
    ):
        self.workdir = workdir
        self.tag = tag
        self.port = free_port()
        self.state_file = workdir / f"sim-{tag}.json"
        command = [
            sys.executable,
            str(SERVER),
            "--port",
            str(self.port),
            "--id-style",
            id_style,
            "--report-style",
            report_style,
            "--state-file",
            str(self.state_file),
            "--b-lift",
            str(b_lift),
            "--quiet",
        ]
        if variant_group_id:
            command.extend(["--variant-group-id", variant_group_id])
        if strict:
            command.append("--strict-writes")
        self.log = open(workdir / f"sim-{tag}.log", "w", encoding="utf-8")
        self.proc = subprocess.Popen(
            command,
            cwd=str(REPO),
            stdout=self.log,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self.base = f"http://127.0.0.1:{self.port}"
        wait_http(self.base + "/_control/state", timeout=15)

    def control(self, action, payload=None):
        return http_json(f"{self.base}/_control/{action}", method="POST", payload=payload or {})

    def state(self):
        return http_json(self.base + "/_control/state")

    def requests(self):
        return self.state().get("requests", [])

    def stop(self):
        try:
            self.control("shutdown")
        except Exception:  # noqa: BLE001
            pass
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        self.log.close()


class ClickHouse:
    def __init__(self, workdir: Path):
        self.workdir = workdir
        self.proc = None

    def ensure(self):
        try:
            wait_http(CLICKHOUSE_HTTP + "/ping", timeout=2)
            return
        except Exception:  # noqa: BLE001 - not running yet
            pass
        binary = shutil.which("clickhouse")
        if not binary:
            raise RuntimeError("the `clickhouse` binary is required for the live daily pass")
        root = self.workdir / "clickhouse"
        for name in ("data", "tmp", "user_files", "access", "format_schemas"):
            (root / name).mkdir(parents=True, exist_ok=True)
        config = root / "config.xml"
        config.write_text(
            f"""<clickhouse>
  <listen_host>127.0.0.1</listen_host>
  <http_port>18172</http_port>
  <tcp_port>{CLICKHOUSE_TCP}</tcp_port>
  <path>{root}/data/</path>
  <tmp_path>{root}/tmp/</tmp_path>
  <user_files_path>{root}/user_files/</user_files_path>
  <access_control_path>{root}/access/</access_control_path>
  <format_schema_path>{root}/format_schemas/</format_schema_path>
  <logger>
    <level>warning</level>
    <log>{root}/clickhouse.log</log>
    <errorlog>{root}/clickhouse.err.log</errorlog>
    <size>50M</size>
    <count>1</count>
  </logger>
  <default_profile>default</default_profile>
  <default_database>default</default_database>
  <users_config>{REPO}/infrastructure/clickhouse/users.xml</users_config>
</clickhouse>
""",
            encoding="utf-8",
        )
        log = open(root / "server.out", "w", encoding="utf-8")
        # ClickHouse resolves DNS after forking its resolver thread; macOS
        # aborts that forked Objective-C child unless this is set.
        env = dict(os.environ)
        env.setdefault("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES")
        self.proc = subprocess.Popen(
            [binary, "server", "--config-file", str(config)],
            cwd=str(REPO),
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )
        wait_http(CLICKHOUSE_HTTP + "/ping", timeout=60)
        self.log = log

    def query(self, sql):
        url = CLICKHOUSE_HTTP + "/?" + urllib.parse.urlencode({"query": sql})
        return http_text(url, timeout=30).strip()

    def native(self, sql):
        binary = shutil.which("clickhouse")
        if not binary:
            return "clickhouse binary unavailable"
        proc = subprocess.run(
            [binary, "client", "--port", str(CLICKHOUSE_TCP), "--query", sql],
            cwd=str(REPO),
            capture_output=True,
            text=True,
            timeout=30,
        )
        return (proc.stdout or proc.stderr).strip()


def encrypt_vault(plaintext: str) -> str:
    try:
        from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    except ImportError as error:  # environment guard
        raise RuntimeError(
            "provisioning the runner vault needs the `cryptography` package "
            "(AES-256-GCM); the simulator itself is stdlib-only"
        ) from error
    nonce = os.urandom(12)
    ciphertext = AESGCM(bytes.fromhex(VAULT_KEY_HEX)).encrypt(nonce, plaintext.encode(), None)
    return nonce.hex() + "." + ciphertext.hex()


def provision(root: Path, sim: Sim):
    root.mkdir(parents=True, exist_ok=True)
    account = {
        "developer_site": DEVELOPER_SITE,
        "expected_networks": [],
        "ad_unit_ids": [],
        "bidder_accounts": [],
        "platforms": {
            "levelplay": {
                "api_key": "lp-secret",
                "account_id": "lp-app-key",
                "base_url": sim.base,
                "report_key": None,
                "reporting_base_url": None,
                "refresh_token": "lp-refresh",
            }
        },
    }
    (root / f"{PUBLISHER}.blob").write_text(encrypt_vault(json.dumps(account)), encoding="utf-8")
    (root / f"{PUBLISHER}.connections.json").write_text(
        json.dumps({"platforms": {"levelplay": {"state": "validated"}}}), encoding="utf-8"
    )


def runner_env(
    creds: Path,
    ledgers: Path,
    snapshots: Path,
    data: Path,
    jobs: Path,
    metrics: Path,
    policy: dict | None = None,
):
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("APEX_") and not key.startswith("CLICKHOUSE_")
    }
    env.update(
        {
            "APEX_CONNECTIONS_ROOT": str(creds),
            "APEX_VAULT_KEY_HEX": VAULT_KEY_HEX,
            "APEX_RUNNER_LEDGER_ROOT": str(ledgers),
            "APEX_RUNNER_SNAPSHOT": str(snapshots),
            "APEX_RUNNER_SIGNING_KEY_HEX": SIGNING_SEED_HEX,
            "APEX_RUNNER_WRITABLE": "updateEntryPrice,createEntry,removeEntry",
            "APEX_CLICKHOUSE_HTTP": CLICKHOUSE_HTTP,
            "APEX_CLICKHOUSE_DB": "apex_levelplay_sim",
            "APEX_ANALYTICS_ENV": "levelplay-sim",
            "APEX_RUNNER_DATA_DIR": str(data),
            "APEX_OPERATOR_JOBS_DIR": str(jobs),
            "APEX_METRICS_TEXTFILE": str(metrics),
        }
    )
    if policy is not None:
        env["APEX_RUNNER_POLICY"] = json.dumps(policy)
    return env


def run_runner(env, args, timeout=180):
    started = time.time()
    proc = subprocess.run(
        [str(RUNNER)] + args,
        cwd=str(REPO),
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    return {
        "argv": args,
        "exit": proc.returncode,
        "elapsed_s": round(time.time() - started, 3),
        "stdout": proc.stdout,
        "stderr": proc.stderr,
    }


def parse_json_stdout(record):
    text = (record.get("stdout") or "").strip()
    if not text:
        return None
    for line in reversed(text.splitlines()):
        line = line.strip()
        if not line:
            continue
        try:
            return json.loads(line)
        except json.JSONDecodeError:
            continue
    return None


def request_counts(state):
    counts = {}
    for request in state.get("requests", []):
        key = f"{request['method']} {request['path'].split('?')[0]}"
        counts[key] = counts.get(key, 0) + 1
    return counts


def summarize(record, state, expect):
    parsed = parse_json_stdout(record)
    ledger = (parsed or {}).get("ledger", {})
    return {
        "argv": record["argv"],
        "exit": record["exit"],
        "expected": expect,
        "ok": (record["exit"] == 0) == (expect == "success"),
        "elapsed_s": record["elapsed_s"],
        "stdout": (record["stdout"] or "")[-4000:],
        "stderr": (record["stderr"] or "")[-4000:],
        "json": parsed,
        "ledger": ledger,
        "requests": request_counts(state),
        "request_log": state.get("requests", [])[-80:],
        "instances": state.get("instances", []),
        "writes": state.get("writes", []),
        "faults": state.get("faults", []),
    }


def single_action_policy() -> dict:
    return {
        "maxActionsPerDay": 1,
        "maxStepBp": 2000,
        "burnInDays": 3,
        "windowDays": 14,
        "interimLook": True,
        "extensionAllowed": True,
        "testShare": 0.25,
        "minPointLift": 0.01,
        "thresholds": {
            "harmPoint": -0.05,
            "fillRewarded": 0.02,
            "fillBanner": 0.05,
            "splitChiSquare": 10.8276,
        },
        "rateBp": 2000,
        "codeVersionHash": "apex-lift-0.1.0",
        "blackoutOnCriticalFindings": True,
        "approvalMode": False,
        "onboardingUnixMs": None,
    }


class Case:
    def __init__(self, workdir: Path, tag: str, sim: Sim, policy: dict | None = None):
        self.tag = tag
        self.sim = sim
        self.root = workdir / tag
        self.root.mkdir(parents=True, exist_ok=True)
        self.creds = self.root / "connections"
        self.ledgers = self.root / "ledgers"
        self.snapshots = self.root / "snapshots"
        self.data = self.root / "data"
        self.jobs = self.root / "jobs"
        self.metrics = self.root / "runner.prom"
        for path in (self.creds, self.ledgers, self.snapshots, self.data, self.jobs):
            path.mkdir(parents=True, exist_ok=True)
        provision(self.creds, self.sim)
        self.env = runner_env(
            self.creds,
            self.ledgers,
            self.snapshots,
            self.data,
            self.jobs,
            self.metrics,
            policy,
        )

    def snapshot_path(self):
        return self.snapshots / PUBLISHER / "levelplay.json"

    def ledger_path(self):
        return self.ledgers / f"{PUBLISHER}.jsonl"


def instance_rate(state, instance_id="1001"):
    for instance in state.get("instances", []):
        if str(instance.get("instanceId")) == str(instance_id):
            return instance.get("rate")
    return None


def audit_case(case: Case):
    record = run_runner(case.env, ["audit", "--json"])
    state = case.sim.state()
    result = summarize(record, state, "success")
    result["snapshot_exists"] = case.snapshot_path().is_file()
    parsed = record.get("stdout") and parse_json_stdout(record)
    if parsed:
        publishers = parsed.get("publishers") or []
        if publishers:
            result["audit"] = publishers[0].get("snapshots", [])
    if case.snapshot_path().is_file():
        snapshot = json.loads(case.snapshot_path().read_text())
        result["snapshot"] = {
            "platform": snapshot.get("platform"),
            "waterfall": snapshot.get("waterfall"),
            "findings_inputs": {
                "ad_units": len(snapshot.get("ad_units", [])),
                "country_revenue": len(snapshot.get("country_revenue", [])),
                "platform_totals_30d": snapshot.get("platform_totals_30d"),
                "app_ads_present": snapshot.get("app_ads_txt", {}).get("present"),
            },
        }
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workdir", default="/tmp/apex-levelplay-sim")
    parser.add_argument("--phase", choices=["baseline", "full", "faults", "all"], default="all")
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--timeout-s", type=int, default=240)
    args = parser.parse_args(argv)

    workdir = Path(args.workdir)
    shutil.rmtree(workdir, ignore_errors=True)
    workdir.mkdir(parents=True, exist_ok=True)

    if not args.no_build:
        subprocess.run(
            ["cargo", "build", "-p", "apex-autopilot", "--bin", "apex-runner"],
            cwd=str(REPO),
            check=True,
        )
    if not RUNNER.is_file():
        raise SystemExit(f"runner binary missing: {RUNNER}")

    clickhouse = ClickHouse(workdir)
    clickhouse.ensure()

    report = {"phases": {}, "clickhouse": {"http": CLICKHOUSE_HTTP, "tcp": CLICKHOUSE_TCP}}

    if args.phase == "baseline":
        phase = {}
        # 1. Documented reporting envelope: the old client expects `results`.
        sim = Sim(workdir, "baseline-report", "numeric", "v1", strict=False)
        try:
            case = Case(workdir, "baseline-report", sim)
            case.env = runner_env(
                case.creds, case.ledgers, case.snapshots, case.data, case.jobs, case.metrics
            )
            record = run_runner(case.env, ["audit", "--json"])
            phase["report_envelope_v1"] = summarize(record, sim.state(), "failure")
        finally:
            sim.stop()
        # 2. Documented instances shape: numeric ids and `rate`/`networkName`.
        sim = Sim(workdir, "baseline-instances", "numeric", "legacy", strict=False)
        try:
            case = Case(workdir, "baseline-instances", sim)
            case.env = runner_env(
                case.creds, case.ledgers, case.snapshots, case.data, case.jobs, case.metrics
            )
            record = run_runner(case.env, ["audit", "--json"])
            phase["instances_shape"] = summarize(record, sim.state(), "failure")
        finally:
            sim.stop()
        report["phases"]["baseline"] = phase

    if args.phase in ("full", "all"):
        phase = {}
        # Faithful read: documented numeric ids + v1 reporting envelope.
        sim = Sim(workdir, "faithful-read", "numeric", "v1", strict=True)
        try:
            case = Case(workdir, "faithful-read", sim)
            case.env = runner_env(
                case.creds, case.ledgers, case.snapshots, case.data, case.jobs, case.metrics
            )
            phase["audit_numeric_v1"] = audit_case(case)
        finally:
            sim.stop()

        # End-to-end drive in the documented wire shape: numeric ids and the
        # strict v4 write bodies. The adapter now accepts both.
        sim = Sim(
            workdir,
            "drive",
            "numeric",
            "v1",
            strict=True,
            b_lift=1.12,
            variant_group_id="1002",
        )
        try:
            case = Case(workdir, "drive", sim, single_action_policy())
            before = sim.state()
            phase["audit"] = audit_case(case)
            cycle = run_runner(case.env, ["cycle", "--json"])
            after_cycle = sim.state()
            phase["cycle"] = summarize(cycle, after_cycle, "success")
            phase["cycle"]["rate_before"] = instance_rate(before)
            phase["cycle"]["rate_after"] = instance_rate(after_cycle)
            phase["cycle"]["target_rate"] = "2.4"
            # A real subsequent read through the same connector: the audit
            # listWaterfall call must observe the applied rate.
            phase["audit_after_cycle"] = audit_case(case)
            daily = run_runner(case.env, ["daily", "--json"], timeout=args.timeout_s)
            after_daily = sim.state()
            phase["daily"] = summarize(daily, after_daily, "success")
            daily_json = phase["daily"].get("json") or {}
            pulls = daily_json.get("reporting_pulls") or {}
            unresolved = (daily_json.get("platform_operations") or {}).get("unresolved")
            ledger_state = daily_json.get("ledger") or {}
            phase["daily"]["report_requests"] = [
                request
                for request in after_daily.get("requests", [])
                if "/levelPlay/reporting/v1" in request["path"]
            ]
            phase["daily"]["ingestion"] = {
                "pulls_completed": pulls.get("completed"),
                "pulls_failed": pulls.get("failed"),
                "unresolved_platform_operations": unresolved,
                "ledger_verified": ledger_state.get("verified"),
                "ingestion_ok": bool(pulls.get("completed"))
                and not pulls.get("failed")
                and ledger_state.get("verified") is True,
            }
            phase["ledger_path"] = str(case.ledger_path())
            if case.ledger_path().is_file():
                lines = [
                    json.loads(line)
                    for line in case.ledger_path().read_text().splitlines()
                    if line.strip()
                ]
                kinds = []
                for line in lines:
                    record = line.get("record", {})
                    kind = record.get("type")
                    if kind == "liftRecord":
                        try:
                            kind = json.loads(record.get("payload", "{}")).get(
                                "type", "liftRecord"
                            )
                        except json.JSONDecodeError:
                            kind = "liftRecord"
                    kinds.append(kind)
                phase["ledger"] = {
                    "records": len(lines),
                    "kinds": kinds,
                    "sequences": [line.get("record", {}).get("seq") for line in lines],
                    "signed": all(line.get("signature") for line in lines),
                }
            db = "apex_levelplay_sim"
            try:
                phase["clickhouse"] = {
                    "domain_events_http": clickhouse.query(
                        f"SELECT count() FROM {db}.domain_events"
                    ),
                    "domain_events_tcp": clickhouse.native(
                        f"SELECT count() FROM {db}.domain_events"
                    ),
                    "projection_versions": clickhouse.query(
                        f"SELECT count() FROM {db}.projection_versions"
                    ),
                }
            except Exception as error:  # noqa: BLE001 - evidence, not a crash
                phase["clickhouse"] = {"error": str(error)}
        finally:
            sim.stop()
        report["phases"]["full"] = phase

    if args.phase in ("faults", "all"):
        phase = {}

        def fault_case(
            name,
            target,
            command,
            expect,
            fault=None,
            expire_next=0,
            strict=False,
        ):
            sim = Sim(workdir, name, "string", "v1", strict=strict)
            try:
                case = Case(workdir, name, sim)
                case.env = runner_env(
                    case.creds, case.ledgers, case.snapshots, case.data, case.jobs, case.metrics
                )
                # A clean audit always creates the snapshot the cycle needs.
                audit = run_runner(case.env, ["audit", "--json"])
                if audit["exit"] != 0:
                    result = summarize(audit, sim.state(), "failure")
                    result["note"] = "setup audit failed"
                    phase[name] = result
                    return
                before = sim.state()
                fault_index = len(before.get("requests", []))
                if expire_next:
                    sim.control("expire_next_bearer", {"count": expire_next})
                if fault:
                    sim.control("fault", fault)
                record = run_runner(case.env, command, timeout=args.timeout_s)
                after = sim.state()
                result = summarize(record, after, expect)
                fault_requests = after.get("requests", [])[fault_index:]
                result["fault_requests"] = request_counts({"requests": fault_requests})
                result["fault_request_log"] = fault_requests
                result["rate_before"] = instance_rate(before)
                result["rate_after"] = instance_rate(after)
                result["fault"] = fault
                result["expire_next_bearer"] = expire_next
                if name in ("write-500", "strict-write-shape"):
                    # The cycle reports an honest non-application: no change is
                    # applied and the provider state is untouched. The process
                    # itself may still exit 0 because a rolled-back proposal is
                    # a recorded outcome, not a fatal error.
                    parsed = parse_json_stdout(record) or {}
                    applied = (parsed.get("cycle") or {}).get("applied")
                    result["honest_failure"] = (
                        applied == 0 and result["rate_before"] == result["rate_after"]
                    )
                    result["ok"] = result["honest_failure"]
                phase[name] = result
            finally:
                sim.stop()

        fault_case(
            "401-bearer-expiry",
            "read",
            ["audit", "--json"],
            "success",
            expire_next=1,
        )
        fault_case(
            "429-retry-after",
            "read",
            ["audit", "--json"],
            "success",
            fault={
                "endpoint": "report",
                "mode": "status",
                "status": 429,
                "retry_after": 1,
                "remaining": 1,
            },
        )
        fault_case(
            "500-bounded-failure",
            "read",
            ["audit", "--json"],
            "failure",
            fault={
                "endpoint": "report",
                "mode": "status",
                "status": 500,
                "remaining": 3,
            },
        )
        fault_case(
            "timeout-bounded-failure",
            "read",
            ["audit", "--json"],
            "failure",
            fault={
                "endpoint": "report",
                "mode": "timeout",
                "delay_ms": 10_500,
                "remaining": 3,
            },
        )
        fault_case(
            "malformed-json",
            "read",
            ["audit", "--json"],
            "failure",
            fault={"endpoint": "report", "mode": "malformed", "remaining": 1},
        )
        fault_case(
            "pagination-truncation",
            "read",
            ["audit", "--json"],
            "failure",
            fault={"endpoint": "report", "mode": "truncate", "remaining": 1},
        )
        fault_case(
            "write-500",
            "write",
            ["cycle", "--json"],
            "failure",
            fault={"endpoint": "write", "mode": "status", "status": 500, "remaining": 100},
        )
        fault_case(
            "strict-write-shape",
            "write",
            ["cycle", "--json"],
            "failure",
            strict=True,
        )
        fault_case(
            "write-429-retry",
            "write",
            ["cycle", "--json"],
            "success",
            fault={
                "endpoint": "write",
                "mode": "status",
                "status": 429,
                "retry_after": 1,
                "remaining": 1,
            },
        )
        report["phases"]["faults"] = phase

    (workdir / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(compact_report(report), indent=2))
    print(f"\nfull evidence: {workdir / 'report.json'}")


def compact_report(report):
    compact = {"clickhouse": report.get("clickhouse"), "phases": {}}
    for phase_name, phase in report.get("phases", {}).items():
        compact["phases"][phase_name] = {}
        for case_name, result in phase.items():
            if not isinstance(result, dict) or "exit" not in result:
                if case_name in ("ledger_path",):
                    compact["phases"][phase_name][case_name] = result
                continue
            compact["phases"][phase_name][case_name] = {
                "exit": result.get("exit"),
                "expected": result.get("expected"),
                "ok": result.get("ok"),
                "elapsed_s": result.get("elapsed_s"),
                "ledger_verified": (result.get("ledger") or {}).get("verified"),
                "audit": result.get("audit"),
                "ingestion": result.get("ingestion"),
                "rate_before": result.get("rate_before"),
                "rate_after": result.get("rate_after"),
                "requests": result.get("requests"),
                "fault_requests": result.get("fault_requests"),
                "honest_failure": result.get("honest_failure"),
                "writes": result.get("writes"),
                "stderr_tail": (result.get("stderr") or "").strip().splitlines()[-3:],
            }
    return compact


if __name__ == "__main__":
    main()
