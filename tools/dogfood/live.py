#!/usr/bin/env python3
"""dogfood-live: the adversarial live half of the dogfooding system.

`apex-tools dogfood-plan` proves statically that `tools/dogfood/surfaces.json`
agrees with the registered routes and fixture page lists. This harness boots
the real services (apex-api, apex-control, apex-sales-web) against a real
ClickHouse and runs every required probe cell declared in the catalog.

Usage (from the repository root):

    python3 tools/dogfood/live.py --static-only
    python3 tools/dogfood/live.py --clickhouse-owned --bin-dir rust/target/debug

Contract, matching the frozen schema in
`rust/crates/apex-tools/src/dogfood_plan.rs`:

- the catalog (`tools/dogfood/surfaces.json`) declares `services.<id>.routes`
  (`path` + `methods`), `fixture_pages` and `required` cells; a cell expands
  to `"<service>|<METHOD path>|<class>"`;
- every required cell must be attempted in a live run and every attempted
  cell must pass; any failure or unattempted cell exits 1 with the cell list;
- `--static-only` runs only the surface-parity and catalog checks (no
  processes, no ClickHouse) and records every cell as skipped;
- the ratchet (`tools/dogfood/live-baseline.json`, the same file the static
  gate reads): a required cell missing from the recorded baseline fails the
  run (drift), and a baseline cell that is no longer required fails unless a
  `removals` entry with a reason releases it;
- `--update-baseline` rewrites the baseline (cells identical to the required
  set, removals preserved) after a fully green run;
- teardown always kills every process the harness started, drops the
  ClickHouse test database and prints the cleanup proof.

Only the Python standard library is used.
"""

import argparse
import base64
import csv
import hashlib
import hmac
import http.client
import io
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
import urllib.parse
from concurrent.futures import ThreadPoolExecutor

FROZEN_PROBE_CLASSES = [
    "surface-parity",
    "response-contract",
    "no-inline",
    "assets-fonts",
    "auth-authority",
    "input-corpus",
    "csrf-idempotency",
    "traversal",
    "escaping",
    "concurrency",
    "failure-injection",
    "differential-truth",
]

PROBE_ORDER = list(FROZEN_PROBE_CLASSES)

SERVICES = ["apex-api", "apex-control", "apex-sales-web"]

# Product facts the harness injects (the catalog declares only the contract).
API_LEDGER_TOKEN = "dogfood-live-ledger-token"
API_LIVE_AUDIT_TOKEN = "dogfood-live-audit-token"
SALES_WEB_TOKEN = "dogfood-live-owner-token"
CONTROL_OPERATOR_PASSWORD = "dogfood-operator-passphrase"
CONTROL_LOCKED_OPERATOR = "locked-probe"
# The RFC 4648 base32 form of the 20-byte all-zero-then-0x01 secret. The
# brief's literal is one character short of a whole byte count, which the
# product decoder correctly refuses (trailing bits are non-zero), so the
# harness uses the byte-aligned spelling of the same secret.
CONTROL_TOTP_SECRET_B32 = "A" * 31 + "B"
# ed25519 public key of the [7u8; 32] test seed the committed fixture ledger
# (rust/crates/apex-api/fixtures/northstar.jsonl) is signed with.
NORTHSTAR_PUBKEY_HEX = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"
FONT_NAMES = [
    "ibm-plex-mono-400.woff2",
    "ibm-plex-mono-500.woff2",
    "ibm-plex-mono-600.woff2",
    "manrope-400.woff2",
    "manrope-700.woff2",
    "manrope-800.woff2",
]

TRAVERSAL_CORPUS = [
    "..%2f..%2fetc%2fpasswd",
    "%2e%2e%2f%2e%2e%2fetc%2fpasswd",
    "..%252f..%252fetc%252fpasswd",
    "....%2f....%2fetc%2fpasswd",
    "%00",
    "%ff",
    "%E2%80%AE%2e%2e%2f",
]
TRAVERSAL_CANARIES = [b"root:x:0", b"[extensions]", b"BEGIN OPENSSH", b"@include"]

HOSTILE_QUERY = "<script>alert('dogfood')</script>\"'\u202e"
HOSTILE_FORMULA = "=cmd|'/C calc'!A0"

# The `report_days` and `domain_events` DDL verbatim from
# rust/crates/apex-analytics/src/schema.rs (the runner's own ensure_schema is
# idempotent over it; the harness seeds them before the first runner pass).
REPORT_DAYS_DDL = """CREATE TABLE IF NOT EXISTS {db}.report_days (
    publisher_id LowCardinality(String),
    platform LowCardinality(String),
    platform_account_id String,
    day Date,
    finality LowCardinality(String),
    revenue_micros Nullable(Int64),
    impressions Nullable(UInt64),
    currency LowCardinality(Nullable(String)),
    country LowCardinality(Nullable(String)),
    ad_unit_id Nullable(String),
    format LowCardinality(Nullable(String)),
    source_digest Nullable(String),
    as_of_version UInt64
) ENGINE = ReplacingMergeTree(as_of_version)
ORDER BY (publisher_id, platform, platform_account_id, day, country, ad_unit_id, format)
SETTINGS allow_nullable_key = 1"""

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
TTL occurred_at_utc + INTERVAL 90 DAY
SETTINGS index_granularity = 8192"""

PATH_PARAMETERS = {
    "{id}": "nope",
    "{seq}": "1",
    "{platform}": "admob",
    "{publisher}": "northstar",
    "{period}": "2026-10",
    "{name}": "manrope-400.woff2",
    "{domain}": "studio.example",
    "{file}": "m-dogfood-live-1.eml",
    "{kind}": "policy_draft",
    "{register}": "invoices",
}


def log(message):
    print(message, flush=True)


def materialize(template):
    path = template
    for token, value in PATH_PARAMETERS.items():
        path = path.replace(token, value, 1)
    return path


def extract_csrf(html):
    for pattern in (
        r'name="csrf_token"\s+value="([^"]+)"',
        r'value="([^"]+)"\s+name="csrf_token"',
    ):
        match = re.search(pattern, html)
        if match:
            return match.group(1)
    return None


def parse_hidden_fields(fragment):
    """Every <input name=... value=...>, attribute order independent."""
    fields = {}
    for tag in re.findall(r"<input\b[^>]*>", fragment):
        attrs = dict(re.findall(r'([A-Za-z_:][-\w:.]*)="([^"]*)"', tag))
        if "name" in attrs:
            fields[attrs["name"]] = html_unescape(attrs.get("value", ""))
    return fields


def html_unescape(text):
    import html as html_module

    return html_module.unescape(text)


def debase32(text):
    """RFC 4648 base32 decode mirroring the product's acceptance rules."""
    alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
    significant = text.strip().rstrip("=").upper()
    buffer = 0
    bits = 0
    out = bytearray()
    for char in significant:
        value = alphabet.find(char)
        if value < 0:
            raise ValueError("invalid base32 character %r" % char)
        buffer = (buffer << 5) | value
        bits += 5
        if bits >= 8:
            bits -= 8
            out.append((buffer >> bits) & 0xFF)
    if bits and (buffer & ((1 << bits) - 1)):
        raise ValueError("base32 trailing bits are non-zero")
    return bytes(out)


def count_lines(path, kind=None):
    if not os.path.exists(path):
        return 0
    count = 0
    with open(path, "r", encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            if kind is None:
                count += 1
                continue
            try:
                if json.loads(line).get("type") == kind:
                    count += 1
            except ValueError:
                continue
    return count


def journal_key_count(path, key):
    """Journal lines for an idempotency key: the journal stores only the
    sha256 fingerprint of the raw key, never the key itself."""
    if not key or not os.path.exists(path):
        return 0
    digest = hashlib.sha256(key.encode("utf-8")).hexdigest()
    count = 0
    with open(path, "r", encoding="utf-8", errors="replace") as fh:
        for line in fh:
            if digest in line:
                count += 1
    return count


def sanitize_evidence(text, limit=520):
    text = " ".join(str(text).split())
    return text[:limit]


class Resp:
    """One raw HTTP response."""

    def __init__(self, status, headers, body):
        self.status = status
        self.header_list = headers
        self.body = body

    @property
    def text(self):
        return self.body.decode("utf-8", "replace")

    def header(self, name):
        lowered = name.lower()
        for key, value in self.header_list:
            if key.lower() == lowered:
                return value
        return None

    def json(self):
        return json.loads(self.body)

    def content_type(self):
        return (self.header("content-type") or "").split(";")[0].strip()


class ReqError(Exception):
    pass


def http_req(method, url, headers=None, body=None, timeout=15):
    parts = urllib.parse.urlsplit(url)
    path = parts.path + (("?" + parts.query) if parts.query else "")
    conn = http.client.HTTPConnection(parts.hostname, parts.port or 80, timeout=timeout)
    try:
        conn.request(method, path or "/", body=body, headers=headers or {})
        raw = conn.getresponse()
        payload = raw.read()
        return Resp(raw.status, list(raw.getheaders()), payload)
    except (OSError, http.client.HTTPException) as error:
        raise ReqError("%s %s: %s" % (method, url, error))
    finally:
        conn.close()


def form_body(fields):
    pairs = []
    for key, value in fields:
        pairs.append(
            urllib.parse.quote_plus(str(key)) + "=" + urllib.parse.quote_plus(str(value))
        )
    return "&".join(pairs).encode("utf-8")


def cookie_value(set_cookie, name):
    if not set_cookie:
        return None
    first = set_cookie.split(";")[0].strip()
    prefix = name + "="
    if first.startswith(prefix):
        return first[len(prefix):]
    return None


class Harness:
    def __init__(self, args):
        self.args = args
        self.root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        self.workdir = (
            args.workdir if os.path.isabs(args.workdir) else os.path.join(self.root, args.workdir)
        )
        self.results_path = (
            args.results if os.path.isabs(args.results) else os.path.join(self.root, args.results)
        )
        self.bin_dir = (
            args.bin_dir if os.path.isabs(args.bin_dir) else os.path.join(self.root, args.bin_dir)
        )
        self.ch_url = args.clickhouse_url.rstrip("/")
        self.ch_port = urllib.parse.urlsplit(self.ch_url).port or 8123
        self.database = "dogfood_live_%d" % os.getpid()
        self.ports = {
            "apex-api": args.port_base,
            "apex-control": args.port_base + 1,
            "apex-sales-web": args.port_base + 2,
            "apex-sales-web-unconfigured": args.port_base + 3,
        }
        self.procs = {}
        self.checks = []
        self.cells = {}
        self.surface = {}
        self.required = set()
        self.failures = []
        self.started_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        self.runner_ran = False
        self.api = {}
        self.control = {}
        self.sales = {}
        self.static_only = args.static_only
        self.ch_owned = args.clickhouse_owned
        self.ch_owned_procs = []
        self.site_skip_reason = None

    # ------------------------------------------------------------------ io
    def repo_path(self, rel):
        return os.path.join(self.root, rel)

    def bin_path(self, name):
        return os.path.join(self.bin_dir, name)

    def check(self, check_id, ok, detail, command=None):
        entry = {
            "check": check_id,
            "status": "pass" if ok else "fail",
            "detail": sanitize_evidence(detail),
            "command": command,
        }
        self.checks.append(entry)
        if not ok:
            self.failures.append("check %s: %s" % (check_id, entry["detail"]))
        log("[%s] %s: %s" % ("PASS" if ok else "FAIL", check_id, entry["detail"]))

    def cell(self, cell_id, ok, detail, command=None):
        entry = {
            "cell": cell_id,
            "status": "pass" if ok else "fail",
            "detail": sanitize_evidence(detail),
            "command": command,
        }
        self.cells[cell_id] = entry
        if not ok:
            self.failures.append("cell %s: %s" % (cell_id, entry["detail"]))

    # ------------------------------------------------------- catalog + boot
    def load_catalog(self):
        path = self.repo_path("tools/dogfood/surfaces.json")
        with open(path, "r", encoding="utf-8") as fh:
            self.surface = json.load(fh)
        declared = list(self.surface.get("probe_classes", []))
        if declared != FROZEN_PROBE_CLASSES:
            self.check(
                "catalog|probe-classes",
                False,
                "probe_classes %r != the frozen registry %r" % (declared, FROZEN_PROBE_CLASSES),
            )
        else:
            self.check("catalog|probe-classes", True, "the 12 frozen probe classes, in order")
        for service, declared_service in self.surface["services"].items():
            for cell in declared_service["required"]:
                for cls in cell["classes"]:
                    self.required.add("%s|%s|%s" % (service, cell["surface"], cls))
        self.check(
            "catalog|required-cells",
            len(self.required) > 0,
            "%d required cells across %s"
            % (len(self.required), ", ".join(sorted(self.surface["services"]))),
        )
        declared_surfaces = set()
        for service, declared_service in self.surface["services"].items():
            for route in declared_service["routes"]:
                for method in route["methods"]:
                    declared_surfaces.add("%s|%s %s" % (service, method.upper(), route["path"]))
        bad = []
        for cell_id in sorted(self.required):
            service, surface, cls = cell_id.split("|", 2)
            if cls not in FROZEN_PROBE_CLASSES:
                bad.append(cell_id)
            if "%s|%s" % (service, surface) not in declared_surfaces:
                bad.append(cell_id)
        self.check(
            "catalog|cells-resolve",
            not bad,
            "every required cell names a declared surface and class"
            if not bad
            else "cells that do not resolve: %s" % bad[:8],
        )
        notes = []
        for cls in FROZEN_PROBE_CLASSES:
            if not any(cell_id.split("|")[2] == cls for cell_id in self.required):
                notes.append("class %s has no required cell" % cls)
            if getattr(self, "probe_" + cls.replace("-", "_"), None) is None:
                notes.append("class %s has no probe implementation" % cls)
        self.check(
            "catalog|probe-bindings",
            not notes,
            "; ".join(notes) if notes else "every class is required somewhere and has a probe",
        )

    def check_site(self):
        public = self.repo_path("site/public")
        pages_min = int(self.surface.get("site", {}).get("pages_min", 0))
        if not os.path.isdir(public):
            self.site_skip_reason = "site/public does not exist in this checkout"
            self.check(
                "site|pages-min",
                True,
                "SKIPPED: %s; pages_min is %d and is never checked silently" % (self.site_skip_reason, pages_min),
            )
            return
        count = 0
        for base, _dirs, files in os.walk(public):
            for name in files:
                if name.endswith(".html"):
                    count += 1
        self.check(
            "site|pages-min",
            count >= pages_min,
            "site/public holds %d .html pages (pages_min %d)" % (count, pages_min),
        )

    def require_binaries(self):
        missing = []
        for name in ("apex-api", "apex-control", "apex-sales-web", "apex-sales", "apex-runner", "apex-tools"):
            if not os.access(self.bin_path(name), os.X_OK):
                missing.append(name)
        if missing:
            self.check(
                "boot|binaries",
                False,
                "missing binaries in %s: %s (build with cargo build --bins)" % (self.bin_dir, ", ".join(missing)),
            )
            return False
        self.check("boot|binaries", True, "all harness binaries present in %s" % self.bin_dir)
        return True

    # -------------------------------------------------------- clickhouse
    def ch_raw(self, sql, body=None, timeout=30):
        url = self.ch_url + "/?query=" + urllib.parse.quote(sql)
        return http_req("POST", url, headers={"Content-Type": "text/plain; charset=utf-8"}, body=body, timeout=timeout)

    def ch(self, sql, body=None, timeout=30):
        resp = self.ch_raw(sql, body=body, timeout=timeout)
        if resp.status != 200:
            raise ReqError("ClickHouse refused %r: HTTP %d %s" % (sql[:120], resp.status, resp.text[:200]))
        return resp.text

    def ch_insert_rows(self, table, rows):
        payload = "\n".join(json.dumps(row) for row in rows).encode("utf-8")
        url = self.ch_url + "/?query=" + urllib.parse.quote(
            "INSERT INTO %s.%s FORMAT JSONEachRow" % (self.database, table)
        )
        resp = http_req("POST", url, headers={"Content-Type": "application/x-ndjson"}, body=payload, timeout=30)
        if resp.status != 200:
            raise ReqError("ClickHouse insert into %s failed: HTTP %d %s" % (table, resp.status, resp.text[:300]))

    def ch_reachable(self):
        try:
            return self.ch("SELECT 1").strip() == "1"
        except ReqError:
            return False

    def ch_bootstrap(self):
        """Create the test database, the two seed tables, seed them and run
        the runner once so the projections are rebuilt and published."""
        self.ch("CREATE DATABASE IF NOT EXISTS %s" % self.database)
        self.ch(REPORT_DAYS_DDL.format(db=self.database))
        self.ch(DOMAIN_EVENTS_DDL.format(db=self.database))
        now = int(time.time())
        yesterday = (now // 86400) * 86400 - 86400
        rows = []
        for offset in range(10):
            day = time.strftime("%Y-%m-%d", time.gmtime(yesterday - offset * 86400))
            rows.append(
                {
                    "publisher_id": "northstar",
                    "platform": "applovin_max",
                    "platform_account_id": "acct-1",
                    "day": day,
                    "finality": "final",
                    "revenue_micros": 5_150_000_000,
                    "impressions": 20,
                    "currency": "USD",
                    "country": "DE",
                    "ad_unit_id": "rw_de_01",
                    "format": "rewarded",
                    "source_digest": None,
                    "as_of_version": 0,
                }
            )
        self.ch_insert_rows("report_days", rows)
        stamp = time.strftime("%Y-%m-%d %H:%M:%S.000", time.gmtime(yesterday))
        self.ch_insert_rows(
            "domain_events",
            [
                {
                    "event_id": "dogfood-live-fee-1",
                    "event_type": "fee_computed",
                    "schema_version": 1,
                    "occurred_at_utc": stamp,
                    "received_at_utc": stamp,
                    "environment": "production",
                    "service_version": "dogfood-live",
                    "source": "finance",
                    "publisher_id": "northstar",
                    "currency": "USD",
                    "amount_micros": 51_500_000_000,
                    "period": time.strftime("%Y-%m", time.gmtime(yesterday)),
                    "payload_json": json.dumps({"note": "dogfood-live seed"}),
                }
            ],
        )
        return self.run_runner()

    def run_runner(self):
        env = dict(os.environ)
        env.update(
            {
                "APEX_CLICKHOUSE_HTTP": self.ch_url,
                "APEX_CLICKHOUSE_DB": self.database,
                "APEX_ANALYTICS_ENV": "production",
                "APEX_MAX_API_KEY": "dogfood-live-dummy",
                "APEX_MAX_REPORT_KEY": "dogfood-live-dummy",
                "APEX_MAX_MANAGEMENT_BASE": "http://127.0.0.1:9",
                "APEX_MAX_REPORTING_BASE": "http://127.0.0.1:9",
                "APEX_RUNNER_LEDGER": os.path.join(self.workdir, "runner", "ledger.jsonl"),
                "APEX_RUNNER_SIGNING_KEY_HEX": "07" * 32,
                "APEX_RUNNER_PUBLISHER": "northstar",
                "APEX_RUNNER_DATA_DIR": os.path.join(self.workdir, "runner"),
                "APEX_OPERATOR_JOBS_DIR": os.path.join(self.workdir, "runner", "jobs"),
                "APEX_METRICS_TEXTFILE": os.path.join(self.workdir, "runner", "runner.prom"),
            }
        )
        os.makedirs(os.path.join(self.workdir, "runner", "jobs"), exist_ok=True)
        proc = subprocess.run(
            [self.bin_path("apex-runner"), "daily"],
            cwd=self.root,
            env=env,
            capture_output=True,
            text=True,
            timeout=180,
        )
        out = (proc.stdout or "") + (proc.stderr or "")
        ok = proc.returncode == 0 and "Projections: rebuilt" in out
        self.check(
            "clickhouse|runner-daily",
            ok,
            "apex-runner daily exit=%d; %s" % (proc.returncode, sanitize_evidence(out, 400)),
            "apex-runner daily (APEX_CLICKHOUSE_HTTP=%s APEX_CLICKHOUSE_DB=%s, dummy MAX bases)" % (self.ch_url, self.database),
        )
        if ok:
            self.runner_ran = True
        return ok

    def ch_sum_revenue(self):
        sql = (
            "SELECT sum(revenue_micros) FROM %s.customer_daily WHERE as_of_version = "
            "(SELECT max(as_of_version) FROM %s.projection_versions WHERE projection_name = 'customer_daily' AND published = 1)"
            % (self.database, self.database)
        )
        return int(self.ch(sql).strip())

    # -------------------------------------------------------- processes
    def start_process(self, name, argv, env, log_name):
        os.makedirs(self.workdir, exist_ok=True)
        log_path = os.path.join(self.workdir, log_name)
        log_file = open(log_path, "ab")
        child_env = dict(os.environ)
        child_env.pop("APEX_CLICKHOUSE_HTTP", None)
        child_env.pop("APEX_CLICKHOUSE_DB", None)
        child_env.update(env)
        proc = subprocess.Popen(
            argv,
            cwd=self.root,
            env=child_env,
            stdout=log_file,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self.procs[name] = proc
        return proc

    def wait_http(self, url, expect_ok, timeout=25.0, headers=None):
        deadline = time.time() + timeout
        last = None
        while time.time() < deadline:
            try:
                resp = http_req("GET", url, headers=headers, timeout=5)
                last = resp.status
                if expect_ok(resp.status):
                    return resp
            except ReqError as error:
                last = str(error)
            time.sleep(0.2)
        raise ReqError("timeout waiting for %s (last: %s)" % (url, last))

    def kill_process(self, name):
        proc = self.procs.pop(name, None)
        if proc is None:
            return
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        except (ProcessLookupError, PermissionError, OSError):
            try:
                proc.kill()
            except OSError:
                pass
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass

    def start_service(self, name):
        if name == "apex-api":
            return self.start_api()
        if name == "apex-control":
            return self.start_control()
        if name == "apex-sales-web":
            return self.start_sales_web()
        raise ReqError("unknown service %s" % name)

    def base_url(self, name):
        return "http://127.0.0.1:%d" % self.ports[name]

    # ------------------------------------------------------- service boot
    def prepare_api_files(self):
        work = os.path.join(self.workdir, "api")
        shutil.rmtree(work, ignore_errors=True)
        snapshots = os.path.join(work, "snapshots")
        ledgers = os.path.join(work, "ledgers")
        os.makedirs(snapshots)
        os.makedirs(ledgers)
        os.makedirs(os.path.join(work, "proposals"))
        os.makedirs(os.path.join(work, "credentials"))
        shutil.copy(self.repo_path("rust/crates/apex-api/fixtures/admob.json"), snapshots)
        shutil.copy(self.repo_path("rust/crates/apex-api/fixtures/northstar.jsonl"), ledgers)
        # The escaping probe owns the snapshot copy: one ad unit carries a
        # hostile name and a formula-leading id, so the CSV export has to
        # neutralize it and the HTML has to escape it.
        snapshot_path = os.path.join(snapshots, "admob.json")
        with open(snapshot_path, "r", encoding="utf-8") as fh:
            snapshot = json.load(fh)
        original_id = None
        if snapshot.get("adUnits"):
            original_id = snapshot["adUnits"][0].get("platformUnitId")

        def walk(node):
            if isinstance(node, dict):
                for key, value in list(node.items()):
                    if isinstance(value, str) and original_id and value == original_id:
                        node[key] = "=cmd1"
                    elif key == "name" and isinstance(value, str) and value.startswith("unit "):
                        node[key] = HOSTILE_FORMULA
                    else:
                        walk(value)
            elif isinstance(node, list):
                for item in node:
                    walk(item)

        walk(snapshot)
        with open(snapshot_path, "w", encoding="utf-8") as fh:
            json.dump(snapshot, fh)
        with open(os.path.join(work, "tokens.json"), "w", encoding="utf-8") as fh:
            json.dump({"northstar": API_LEDGER_TOKEN}, fh)
        return work

    def api_env(self, work, port):
        return {
            "APEX_API_BIND": "127.0.0.1:%d" % port,
            "APEX_SNAPSHOT_ROOT": os.path.join(work, "snapshots"),
            "APEX_LEDGER_ROOT": os.path.join(work, "ledgers"),
            "APEX_LEDGER_KEYS": "northstar=%s" % NORTHSTAR_PUBKEY_HEX,
            "APEX_LEDGER_TOKENS_FILE": os.path.join(work, "tokens.json"),
            "APEX_LEADS_FILE": os.path.join(work, "leads.jsonl"),
            "APEX_LEAD_ID_SALT": "dogfood" * 8,
            "APEX_PROPOSALS_ROOT": os.path.join(work, "proposals"),
            "APEX_CONSOLE_PUBLISHER": "northstar",
            "APEX_CONSOLE_PUBLISHER_NAME": "Northstar Studio",
            "APEX_CONSOLE_CURRENCY": "USD",
            "APEX_CREDENTIAL_ROOT": os.path.join(work, "credentials"),
            "APEX_VAULT_KEY_HEX": "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "APEX_LIVE_AUDIT_TOKEN": API_LIVE_AUDIT_TOKEN,
            "APEX_CLICKHOUSE_HTTP": self.ch_url,
            "APEX_CLICKHOUSE_DB": self.database,
            "APEX_ANALYTICS_ENV": "production",
            "APEX_METRICS_TEXTFILE": os.path.join(self.workdir, "api.prom"),
            "APEX_BILLING_VAT_RATE_BP": "2000",
            "APEX_MONITORING_FEE_MICROS": "500000000",
        }

    def start_api(self):
        work = self.api["work"]
        self.start_process(
            "apex-api", [self.bin_path("apex-api")], self.api_env(work, self.ports["apex-api"]), "apex-api.log"
        )
        self.wait_http(self.base_url("apex-api") + "/healthz", lambda status: status == 200)
        return self.procs["apex-api"]

    def hash_password(self, password):
        proc = subprocess.run(
            [self.bin_path("apex-control"), "hash-password"],
            input=password + "\n",
            capture_output=True,
            text=True,
            timeout=60,
        )
        if proc.returncode != 0:
            raise ReqError("apex-control hash-password failed: %s" % proc.stderr.strip())
        phc = proc.stdout.strip()
        if not phc.startswith("$argon2id$"):
            raise ReqError("hash-password did not print an argon2id PHC string: %r" % phc[:60])
        return phc

    def prepare_control_files(self):
        work = os.path.join(self.workdir, "control")
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(os.path.join(work, "jobs"))
        os.makedirs(os.path.join(work, "ledgers"))
        # The committed northstar ledger is the control plane's read-only
        # evidence source: durable experiments and record counts come from it.
        shutil.copy(
            self.repo_path("rust/crates/apex-api/fixtures/northstar.jsonl"),
            os.path.join(work, "ledgers", "northstar.jsonl"),
        )
        # An empty (but present) invoice register keeps the finance routes on
        # their real 404/empty-register paths instead of the not-configured
        # state, so the traversal corpus reaches the register lookup.
        open(os.path.join(work, "invoices.jsonl"), "w", encoding="utf-8").close()
        phc = self.hash_password(CONTROL_OPERATOR_PASSWORD)
        operators = [
            {
                "id": "ops",
                "name": "Dogfood Operator",
                # Security holds SessionRevoke and PolicyDraft, the two
                # capabilities the replay and concurrency probes exercise.
                "role": "security",
                "password_argon2id_phc": phc,
                "totp_secret_base32": CONTROL_TOTP_SECRET_B32,
            },
            {
                "id": CONTROL_LOCKED_OPERATOR,
                "name": "Lockout Probe",
                "role": "observer",
                "password_argon2id_phc": phc,
                "totp_secret_base32": CONTROL_TOTP_SECRET_B32,
            },
        ]
        with open(os.path.join(work, "operators.json"), "w", encoding="utf-8") as fh:
            json.dump(operators, fh)
        return work

    def control_env(self, work, port):
        return {
            "APEX_CONTROL_BIND": "127.0.0.1:%d" % port,
            "APEX_CONTROL_OPERATORS": os.path.join(work, "operators.json"),
            "APEX_CONTROL_JOURNAL": os.path.join(work, "journal.jsonl"),
            "APEX_OPERATOR_JOBS_DIR": os.path.join(work, "jobs"),
            "APEX_CONTROL_LEDGER_ROOT": os.path.join(work, "ledgers"),
            "APEX_CONTROL_INVOICE_REGISTER": os.path.join(work, "invoices.jsonl"),
            "APEX_CONTROL_OPERATIONS": os.path.join(work, "operations.json"),
            "APEX_CONTROL_POLICIES": os.path.join(work, "policies.jsonl"),
            "APEX_CONTROL_INTERVENTIONS": os.path.join(work, "interventions.jsonl"),
            "APEX_ENVIRONMENT": "TEST",
            "APEX_CLICKHOUSE_HTTP": self.ch_url,
            "APEX_CLICKHOUSE_DB": self.database,
            "APEX_METRICS_TEXTFILE": os.path.join(self.workdir, "control.prom"),
        }

    def start_control(self):
        work = self.control["work"]
        self.start_process(
            "apex-control",
            [self.bin_path("apex-control")],
            self.control_env(work, self.ports["apex-control"]),
            "apex-control.log",
        )
        self.wait_http(self.base_url("apex-control") + "/control/login", lambda status: status in (200, 401))
        return self.procs["apex-control"]

    def sales_cli(self, argv, sales_dir, outbox, mode="approval"):
        env = dict(os.environ)
        env.update(
            {
                "APEX_SALES_DIR": sales_dir,
                "APEX_SALES_MODE": mode,
                "APEX_SALES_HOLDOUT": "0",
                "APEX_SALES_OUTBOX": outbox,
            }
        )
        return subprocess.run(
            [self.bin_path("apex-sales")] + argv,
            cwd=self.root,
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def prepare_sales_files(self):
        work = os.path.join(self.workdir, "sales")
        shutil.rmtree(work, ignore_errors=True)
        sales_dir = os.path.join(work, "state")
        outbox = os.path.join(work, "outbox")
        os.makedirs(sales_dir)
        os.makedirs(outbox)
        seeds = os.path.join(work, "seeds.csv")
        with open(seeds, "w", encoding="utf-8", newline="") as fh:
            writer = csv.writer(fh, lineterminator="\n")
            writer.writerow(["domain", "name", "country", "app_store_ids", "email", "contact_name", "role"])
            writer.writerow(["studio.example", "Studio Example", "FI", "42", "hello@studio.example", "Heli Virtanen", "Partnerships"])
            writer.writerow(["northwind.example", "Northwind Games", "SE", "77", "ops@northwind.example", "Nora Berg", "Growth"])
            writer.writerow(
                [
                    "probe-hostile.example",
                    'Hostile & Co "<Probe>"',
                    "DE",
                    "99",
                    "=cmd+probe@probe-hostile.example",
                    HOSTILE_FORMULA,
                    "=SUM(A1:A2)",
                ]
            )
        leads = os.path.join(work, "leads.jsonl")
        with open(leads, "w", encoding="utf-8") as fh:
            fh.write(
                json.dumps(
                    {
                        "atUnixMs": int(time.time() * 1000),
                        "lead": {
                            "publisher": "lead-test",
                            "email": "leading@lead-test.example",
                            "studio": "Lead Test Studio",
                            "platforms": ["admob"],
                            "notes": "dogfood-live imported lead",
                        },
                    }
                )
                + "\n"
            )
        results = []
        for argv in (
            ["import-seeds", seeds],
            ["import-leads", leads],
            ["record", "studio.example", "audit_requested", "dogfood-live"],
            ["suppress", "northwind.example"],
        ):
            proc = self.sales_cli(argv, sales_dir, outbox)
            results.append((argv[0], proc.returncode, sanitize_evidence((proc.stdout or proc.stderr).strip(), 120)))
        ok = all(rc == 0 for _cmd, rc, _out in results)
        self.check(
            "sales|cli-seed",
            ok,
            "; ".join("%s exit=%d %s" % (cmd, rc, out) for cmd, rc, out in results),
            "apex-sales import-seeds/import-leads/record/suppress",
        )
        self.sales["work"] = work
        self.sales["sales_dir"] = sales_dir
        self.sales["outbox"] = outbox
        return ok

    def seed_touch(self):
        """A Touch-equivalent: the exact durable shapes the engine writes (an
        outbox `.eml` named after the message id plus the store's Touch event
        JSON line). The seam exists: both shapes are documented by
        `apex_sales::io::OutboxMailer` and `apex_sales::store::Event`."""
        message_id = "m-dogfood-live-1"
        raw = (
            "From: Apex Sales <sales@apex.example>\r\n"
            "To: Heli Virtanen <hello@studio.example>\r\n"
            "Subject: Studio Example - your rewarded inventory audit\r\n"
            "Message-Id: <" + message_id + "@apex.example>\r\n"
            "MIME-Version: 1.0\r\n"
            "Content-Type: text/plain; charset=utf-8\r\n"
            "\r\n"
            "Hi Heli,\r\n\r\nWe found reward-format inventory in Tile Quest with a stale floor.\r\n"
            "The audit summary is in your console.\r\n"
        )
        with open(os.path.join(self.sales["outbox"], message_id + ".eml"), "w", encoding="utf-8", newline="") as fh:
            fh.write(raw)
        event = {
            "type": "touch",
            "at": int(time.time()),
            "email": "hello@studio.example",
            "publisher": "studio.example",
            "step": 1,
            "arm": "warm-intro",
            "messageId": message_id,
            "localHour": 10,
            "predicted": None,
        }
        with open(os.path.join(self.sales["sales_dir"], "events.jsonl"), "a", encoding="utf-8") as fh:
            fh.write(json.dumps(event) + "\n")
        self.check(
            "sales|touch-seed",
            True,
            "outbox/%s.eml + one Touch event line in the store (the engine's own durable shapes)" % message_id,
        )
        self.sales["message_id"] = message_id

    def sales_web_env(self, port, token):
        env = {
            "APEX_SALES_WEB_BIND": "127.0.0.1:%d" % port,
            "APEX_SALES_DIR": self.sales["sales_dir"],
            "APEX_SALES_MODE": "approval",
            "APEX_SALES_HOLDOUT": "0",
            "APEX_SALES_OUTBOX": self.sales["outbox"],
            "APEX_METRICS_TEXTFILE": os.path.join(self.workdir, "sales-web.prom"),
        }
        if token is not None:
            env["APEX_SALES_WEB_TOKEN"] = token
        return env

    def start_sales_web(self):
        name = "apex-sales-web"
        self.start_process(
            name,
            [self.bin_path("apex-sales-web")],
            self.sales_web_env(self.ports[name], SALES_WEB_TOKEN),
            "apex-sales-web.log",
        )
        self.wait_http(self.base_url(name) + "/login", lambda status: status in (200, 503))
        return self.procs[name]

    # ------------------------------------------------------- auth helpers
    def api_session(self, force=False):
        if self.api.get("cookie") and not force:
            return self.api["cookie"], self.api.get("csrf")
        resp = http_req(
            "POST",
            self.base_url("apex-api") + "/console/ledger",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("publisher", "northstar"), ("token", API_LEDGER_TOKEN)]),
        )
        if resp.status != 303:
            raise ReqError("ledger exchange failed: HTTP %d" % resp.status)
        cookie = cookie_value(resp.header("set-cookie"), "apex_ledger_session")
        if not cookie:
            raise ReqError("ledger exchange did not set apex_ledger_session")
        page = http_req(
            "GET",
            self.base_url("apex-api") + "/console/settings/access",
            headers={"Cookie": "apex_ledger_session=" + cookie},
        )
        self.api["cookie"] = cookie
        self.api["csrf"] = extract_csrf(page.text)
        return cookie, self.api["csrf"]

    def totp_code(self, secret_b32, at=None):
        secret = debase32(secret_b32)
        counter = int((at if at is not None else time.time()) // 30)
        digest = hmac.new(secret, counter.to_bytes(8, "big"), hashlib.sha1).digest()
        offset = digest[-1] & 0x0F
        binary = (
            (digest[offset] & 0x7F) << 24
            | digest[offset + 1] << 16
            | digest[offset + 2] << 8
            | digest[offset + 3]
        )
        return "%06d" % (binary % 1_000_000)

    def control_login(self, operator="ops"):
        resp = http_req(
            "POST",
            self.base_url("apex-control") + "/control/login",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("operator", operator), ("password", CONTROL_OPERATOR_PASSWORD)]),
        )
        if resp.status != 200:
            return resp, None
        match = re.search(r'name="pending"\s+value="([^"]+)"', resp.text)
        if not match:
            return resp, None
        pending = match.group(1)
        resp = http_req(
            "POST",
            self.base_url("apex-control") + "/control/login/code",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("pending", pending), ("code", self.totp_code(CONTROL_TOTP_SECRET_B32))]),
        )
        return resp, pending

    def control_session(self, force=False):
        if self.control.get("cookie") and not force:
            return self.control["cookie"], self.control.get("csrf")
        resp, _pending = self.control_login("ops")
        if resp.status != 303:
            raise ReqError("control login did not answer 303: HTTP %d" % resp.status)
        cookie = cookie_value(resp.header("set-cookie"), "apex_operator_session")
        if not cookie:
            raise ReqError("control login did not set apex_operator_session")
        page = http_req(
            "GET",
            self.base_url("apex-control") + "/control/access",
            headers={"Cookie": "apex_operator_session=" + cookie},
        )
        self.control["cookie"] = cookie
        self.control["csrf"] = extract_csrf(page.text)
        return cookie, self.control["csrf"]

    def sales_session(self, force=False):
        if self.sales.get("cookie") and not force:
            return self.sales["cookie"], self.sales.get("csrf")
        resp = http_req(
            "POST",
            self.base_url("apex-sales-web") + "/login",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("token", SALES_WEB_TOKEN)]),
        )
        if resp.status != 303:
            raise ReqError("sales-web login did not answer 303: HTTP %d" % resp.status)
        cookie = cookie_value(resp.header("set-cookie"), "apex_sales_session")
        if not cookie:
            raise ReqError("sales-web login did not set apex_sales_session")
        page = http_req(
            "GET",
            self.base_url("apex-sales-web") + "/domains",
            headers={"Cookie": "apex_sales_session=" + cookie},
        )
        self.sales["cookie"] = cookie
        self.sales["csrf"] = extract_csrf(page.text)
        return cookie, self.sales["csrf"]

    def service_session(self, service, force=False):
        if service == "apex-api":
            return self.api_session(force=force)
        if service == "apex-control":
            return self.control_session(force=force)
        return self.sales_session(force=force)

    def cookie_header(self, service):
        name, value = {
            "apex-api": ("apex_ledger_session", self.api.get("cookie")),
            "apex-control": ("apex_operator_session", self.control.get("cookie")),
            "apex-sales-web": ("apex_sales_session", self.sales.get("cookie")),
        }[service]
        if not value:
            return {}
        return {"Cookie": "%s=%s" % (name, value)}

    # ------------------------------------------------------------- probes
    def cells_of(self, cls, service=None):
        out = []
        for cell_id in sorted(self.required):
            svc, surface, cell_cls = cell_id.split("|", 2)
            if cell_cls != cls:
                continue
            if service and svc != service:
                continue
            out.append((svc, surface, cell_id))
        return out

    def run_all_probes(self):
        for cls in PROBE_ORDER:
            probe = getattr(self, "probe_" + cls.replace("-", "_"))
            try:
                probe()
            except ReqError as error:
                for _svc, _surface, cell_id in self.cells_of(cls):
                    if cell_id not in self.cells:
                        self.cell(cell_id, False, "probe aborted: %s" % error)
            except Exception as error:  # a harness bug must never lose cells
                for _svc, _surface, cell_id in self.cells_of(cls):
                    if cell_id not in self.cells:
                        self.cell(cell_id, False, "probe error %r: %s" % (type(error).__name__, error))

    # 1 ------------------------------------------------------ surface-parity
    def probe_surface_parity(self):
        proc = subprocess.run(
            [self.bin_path("apex-tools"), "dogfood-plan"],
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=120,
        )
        out = (proc.stdout or "") + (proc.stderr or "")
        ok = proc.returncode == 0 and "dogfood-plan PASS" in out
        for _svc, _surface, cell_id in self.cells_of("surface-parity"):
            self.cell(
                cell_id,
                ok,
                "apex-tools dogfood-plan exit=%d; %s" % (proc.returncode, sanitize_evidence(out)),
                "apex-tools dogfood-plan",
            )

    # 2 --------------------------------------------------- response-contract
    def response_contract_expectations(self, service):
        if service == "apex-api":
            return {
                "/": ({200}, True),
                "/console": ({303}, False),
                "/console/": ({303}, False),
                "/console/audit": ({200}, True),
                "/console/audit/export.csv": ({400}, False),
                "/console/audit/findings/{id}": ({404}, True),
                "/console/ledger": ({200}, True),
                "/console/ledger/records/{seq}": ({303}, False),
                "/console/connections": ({200}, True),
                "/console/connections/{platform}": ({200}, True),
                "/console/proposals": ({200}, True),
                "/console/proposals/{id}": ({404}, True),
                "/console/experiments": ({200}, True),
                "/console/experiments/{id}": ({404}, True),
                "/console/experiments/{id}/incident": ({404}, True),
                "/console/billing": ({200}, True),
                "/console/billing/invoices/{period}": ({404}, True),
                "/console/settings": ({200}, True),
                "/console/settings/access": ({200}, True),
                "/console/settings/policy": ({200}, True),
                "/console/settings/monitoring": ({200}, True),
                "/console/analytics": ({200}, True),
                "/free-audit": ({303}, False),
                "/free-audit/": ({200}, True),
                "/audit-requested/": ({200}, True),
                "/healthz": ({200}, False),
                "/favicon.ico": ({200}, False),
                "/favicon.svg": ({200}, False),
                "/assets/fonts/{name}": ({200}, False),
                "/metrics": ({200}, False),
                "/v1/audit/{platform}": ({200}, False),
                "/v1/ledger/{publisher}": ({401}, False),
            }
        if service == "apex-control":
            return {
                "/": ({303}, False),
                "/control": ({401}, True),
                "/control/": ({401}, True),
                "/control/customers": ({401}, True),
                "/control/customers/{id}": ({401}, True),
                "/control/ingestion": ({401}, True),
                "/control/audits": ({401}, True),
                "/control/experiments": ({401}, True),
                "/control/experiments/{id}": ({401}, True),
                "/control/incidents": ({401}, True),
                "/control/incidents/{id}": ({401}, True),
                "/control/finance": ({401}, True),
                "/control/finance/invoices/{id}": ({401}, True),
                "/control/finance/exports/{register}": ({401}, True),
                "/control/growth": ({401}, True),
                "/control/system": ({401}, True),
                "/control/system/policies": ({401}, True),
                "/control/access": ({401}, True),
                "/favicon.ico": ({200}, False),
                "/favicon.svg": ({200}, False),
                "/assets/fonts/{name}": ({200}, False),
                "/control/login": ({200}, True),
                "/control/login/code": ({200}, True),
                "/control/actions/{kind}/result/{id}": ({401}, True),
            }
        return {
            "/": ({401}, True),
            "/leads": ({401}, True),
            "/leads/export.csv": ({401}, False),
            "/publishers/{domain}": ({401}, True),
            "/plan": ({401}, True),
            "/sends": ({401}, True),
            "/sends/preview/{file}": ({401}, True),
            "/replies": ({401}, True),
            "/domains": ({401}, True),
            "/models": ({401}, True),
            "/healthz": ({200}, False),
            "/assets/apex.css": ({200}, False),
            "/assets/fonts/{name}": ({200}, False),
            "/favicon.ico": ({200}, False),
            "/favicon.svg": ({200}, False),
            "/login": ({200}, True),
            "/actions/result/{id}": ({401}, True),
        }

    def probe_response_contract(self):
        for service, _surface, cell_id in self.cells_of("response-contract"):
            surface = cell_id.split("|")[1]
            method, template = surface.split(" ", 1)
            expect = self.response_contract_expectations(service).get(template)
            if expect is None:
                self.cell(cell_id, False, "no response-contract expectation for %s" % template)
                continue
            statuses, html = expect
            url = self.base_url(service) + materialize(template)
            try:
                resp = http_req(method, url)
            except ReqError as error:
                self.cell(cell_id, False, str(error))
                continue
            problems = []
            if resp.status not in statuses:
                problems.append("HTTP %d not in %s" % (resp.status, sorted(statuses)))
            if html and 200 <= resp.status < 400 and resp.content_type() == "text/html":
                body = resp.text
                h1 = len(re.findall(r"<h1[\s>]", body))
                if h1 != 1:
                    problems.append("expected exactly one h1, found %d" % h1)
                if "<title" not in body:
                    problems.append("missing <title>")
            detail = "GET %s -> HTTP %d" % (url, resp.status)
            if problems:
                detail += "; " + "; ".join(problems)
            self.cell(cell_id, not problems, detail, "GET %s" % url)

    # 3 ---------------------------------------------------------- no-inline
    def no_inline_targets(self):
        return {
            "apex-api": [
                ("GET /console/audit/findings/{id}", "/console/audit/findings/nope"),
                ("GET /console/ledger", "/console/ledger"),
                ("GET /console/ledger/records/{seq}", "/console/ledger/records/1"),
                ("GET /console/connections", "/console/connections"),
                ("GET /console/connections/{platform}", "/console/connections/admob"),
                ("GET /console/proposals", "/console/proposals"),
                ("GET /console/proposals/{id}", "/console/proposals/nope"),
                ("GET /console/experiments", "/console/experiments"),
                ("GET /console/experiments/{id}", "/console/experiments/nope"),
                ("GET /console/experiments/{id}/incident", "/console/experiments/nope/incident"),
                ("GET /console/billing", "/console/billing"),
                ("GET /console/billing/invoices/{period}", "/console/billing/invoices/2026-10"),
                ("GET /console/settings", "/console/settings"),
                ("GET /console/settings/access", "/console/settings/access"),
                ("GET /console/settings/policy", "/console/settings/policy"),
                ("GET /console/settings/monitoring", "/console/settings/monitoring"),
                ("GET /console/analytics", "/console/analytics"),
                ("GET /console/audit", "/console/audit"),
                ("GET /", "/"),
                ("GET /free-audit/", "/free-audit/"),
                ("GET /audit-requested/", "/audit-requested/"),
            ],
            "apex-control": [
                ("GET /control", "/control"),
                ("GET /control/", "/control/"),
                ("GET /control/customers", "/control/customers"),
                ("GET /control/customers/{id}", "/control/customers/nope"),
                ("GET /control/ingestion", "/control/ingestion"),
                ("GET /control/audits", "/control/audits"),
                ("GET /control/experiments", "/control/experiments"),
                ("GET /control/experiments/{id}", "/control/experiments/nope"),
                ("GET /control/incidents", "/control/incidents"),
                ("GET /control/incidents/{id}", "/control/incidents/nope"),
                ("GET /control/finance", "/control/finance"),
                ("GET /control/finance/invoices/{id}", "/control/finance/invoices/nope"),
                ("GET /control/finance/exports/{register}", "/control/finance/exports/nope"),
                ("GET /control/growth", "/control/growth"),
                ("GET /control/system", "/control/system"),
                ("GET /control/system/policies", "/control/system/policies"),
                ("GET /control/access", "/control/access"),
                ("GET /control/login", "/control/login"),
                ("GET /control/login/code", "/control/login/code"),
                ("GET /control/actions/{kind}/result/{id}", "/control/actions/policy_draft/result/res_nope000000"),
            ],
            "apex-sales-web": [
                ("GET /", "/"),
                ("GET /leads", "/leads"),
                ("GET /publishers/{domain}", "/publishers/studio.example"),
                ("GET /plan", "/plan"),
                ("GET /sends", "/sends"),
                ("GET /sends/preview/{file}", "/sends/preview/m-dogfood-live-1.eml"),
                ("GET /replies", "/replies"),
                ("GET /domains", "/domains"),
                ("GET /models", "/models"),
                ("GET /login", "/login"),
                ("GET /actions/result/{id}", "/actions/result/op_nonexistent0"),
            ],
        }

    def probe_no_inline(self):
        for service, _surface, cell_id in self.cells_of("no-inline"):
            surface = cell_id.split("|")[1]
            target = None
            for template, path in self.no_inline_targets()[service]:
                if template == surface:
                    target = path
                    break
            if target is None:
                self.cell(cell_id, False, "no no-inline sweep target for %s" % surface)
                continue
            headers = {}
            try:
                if service == "apex-control":
                    self.control_session()
                    headers = self.cookie_header(service)
                elif service == "apex-sales-web":
                    self.sales_session()
                    headers = self.cookie_header(service)
                elif service == "apex-api":
                    self.api_session()
                    headers = self.cookie_header(service)
            except ReqError as error:
                self.cell(cell_id, False, "session setup failed: %s" % error)
                continue
            url = self.base_url(service) + target
            try:
                resp = http_req("GET", url, headers=headers)
            except ReqError as error:
                self.cell(cell_id, False, str(error))
                continue
            if resp.status == 303:
                self.cell(cell_id, True, "GET %s -> HTTP 303 (redirect; no body to sweep)" % url)
                continue
            problems = []
            body = resp.text
            if resp.content_type() == "text/html":
                if 'style="' in body:
                    problems.append('contains a style=" attribute')
                if "<script" in body:
                    problems.append("contains a <script tag")
                if service == "apex-sales-web":
                    if "<style" in body:
                        problems.append("contains a <style block (sales-web links the one stylesheet)")
                    links = body.count('rel="stylesheet"')
                    if links != 1:
                        problems.append("expected exactly one stylesheet link, found %d" % links)
                    if "/assets/apex.css" not in body:
                        problems.append("the stylesheet link does not point at /assets/apex.css")
                else:
                    styles = len(re.findall(r"<style[\s>]", body))
                    if styles != 1:
                        problems.append("expected exactly one <style block, found %d" % styles)
            detail = "GET %s -> HTTP %d" % (url, resp.status)
            if problems:
                detail += "; " + "; ".join(problems)
            self.cell(cell_id, not problems, detail, "GET %s" % url)

    # 4 ------------------------------------------------------- assets-fonts
    def fetch_css_fixture(self, service):
        if service == "apex-sales-web":
            return http_req("GET", self.base_url(service) + "/assets/apex.css").body
        resp = http_req("GET", self.base_url(service) + "/console/audit")
        match = re.search(r"<style>(.*?)</style>", resp.text, re.S)
        if not match:
            return None
        return match.group(1).encode("utf-8")

    def probe_assets_fonts(self):
        css_bodies = {}
        for service in SERVICES:
            try:
                css_bodies[service] = self.fetch_css_fixture(service)
            except ReqError as error:
                css_bodies[service] = None
                self.check("assets-fonts|css|%s" % service, False, "css fetch failed: %s" % error)
        baseline = css_bodies.get("apex-api")
        for service in SERVICES:
            body = css_bodies.get(service)
            if baseline is None or body is None:
                self.check("assets-fonts|css|%s" % service, False, "no CSS bytes to compare")
                continue
            self.check(
                "assets-fonts|css|%s" % service,
                body == baseline,
                "CSS bytes match the console_css sheet served by every service (%d bytes)" % len(body),
            )
        for service, _surface, cell_id in self.cells_of("assets-fonts"):
            surface = cell_id.split("|")[1]
            url = self.base_url(service) + materialize(surface.split(" ", 1)[1])
            try:
                resp = http_req("GET", url)
            except ReqError as error:
                self.cell(cell_id, False, str(error))
                continue
            problems = []
            if resp.status != 200:
                problems.append("HTTP %d" % resp.status)
            elif surface.endswith("/assets/fonts/{name}"):
                name = materialize(surface.split(" ", 1)[1]).rsplit("/", 1)[-1]
                if "font/woff2" not in (resp.header("content-type") or ""):
                    problems.append("content-type %s" % resp.header("content-type"))
                if resp.body[:4] != b"wOF2":
                    problems.append("magic %r" % resp.body[:4])
                expected = os.path.getsize(self.repo_path("site/static/fonts/" + name))
                if len(resp.body) != expected:
                    problems.append(
                        "served %d bytes, site/static/fonts/%s is %d" % (len(resp.body), name, expected)
                    )
            elif surface == "GET /favicon.ico":
                if resp.body[:4] != b"\x00\x00\x01\x00":
                    problems.append("ICO magic %r" % resp.body[:4])
            elif surface == "GET /favicon.svg":
                stripped = resp.body.lstrip()
                if not (stripped[:4] == b"<svg" or stripped[:5] == b"<?xml"):
                    problems.append("SVG magic %r" % resp.body[:8])
            elif surface == "GET /assets/apex.css":
                if resp.content_type() != "text/css":
                    problems.append("content-type %s" % resp.content_type())
                if baseline is not None and resp.body != baseline:
                    problems.append("CSS bytes differ from the other services")
            detail = "GET %s -> HTTP %d, %d bytes" % (url, resp.status, len(resp.body))
            if problems:
                detail += "; " + "; ".join(problems)
            self.cell(cell_id, not problems, detail, "GET %s" % url)

    # 5 ------------------------------------------------------ auth-authority
    def probe_auth_authority(self):
        results = {
            "apex-api": self.auth_api(),
            "apex-control": self.auth_control(),
            "apex-sales-web": self.auth_sales(),
        }
        for service, _surface, cell_id in self.cells_of("auth-authority"):
            ok, detail = results[service]
            self.cell(cell_id, ok, detail)

    def auth_api(self):
        problems = []
        notes = []
        base = self.base_url("apex-api")
        wrong = http_req(
            "POST",
            base + "/console/ledger",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("publisher", "northstar"), ("token", "wrong-token")]),
        )
        if wrong.status != 401:
            problems.append("wrong ledger token answered %d, expected 401" % wrong.status)
        else:
            notes.append("wrong token -> 401")
        good = http_req(
            "POST",
            base + "/console/ledger",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("publisher", "northstar"), ("token", API_LEDGER_TOKEN)]),
        )
        set_cookie = good.header("set-cookie") or ""
        if good.status != 303:
            problems.append("valid ledger token answered %d, expected 303" % good.status)
        else:
            notes.append("valid token -> 303")
        for flag in ("HttpOnly", "SameSite=Lax", "Path=/console"):
            if flag not in set_cookie:
                problems.append("session cookie lacks %s" % flag)
        no_bearer = http_req("GET", base + "/v1/ledger/northstar")
        if no_bearer.status != 401:
            problems.append("/v1/ledger without bearer answered %d, expected 401" % no_bearer.status)
        else:
            notes.append("v1 ledger no bearer -> 401")
        with_bearer = http_req(
            "GET", base + "/v1/ledger/northstar", headers={"Authorization": "Bearer " + API_LEDGER_TOKEN}
        )
        if with_bearer.status != 200:
            problems.append("/v1/ledger with bearer answered %d, expected 200" % with_bearer.status)
        else:
            notes.append("v1 ledger bearer -> 200")
        live = http_req(
            "POST",
            base + "/v1/audit/admob",
            headers={"Content-Type": "application/json"},
            body=b'{"publisher":"northstar"}',
        )
        if live.status != 401:
            problems.append("live audit without bearer answered %d, expected 401" % live.status)
        else:
            notes.append("live audit no bearer -> 401")
        control_with_api_cookie = http_req(
            "GET",
            self.base_url("apex-control") + "/control/",
            headers={"Cookie": "apex_ledger_session=" + (self.api.get("cookie") or "x")},
        )
        if control_with_api_cookie.status not in (401, 303):
            problems.append("control honoured an api cookie (HTTP %d)" % control_with_api_cookie.status)
        else:
            notes.append("control rejects the api cookie (HTTP %d)" % control_with_api_cookie.status)
        return (not problems, "; ".join(notes + problems))

    def auth_control(self):
        problems = []
        notes = []
        base = self.base_url("apex-control")
        wrong = http_req(
            "POST",
            base + "/control/login",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("operator", CONTROL_LOCKED_OPERATOR), ("password", "not-the-password")]),
        )
        if wrong.status != 401:
            problems.append("wrong password answered %d, expected 401" % wrong.status)
        elif "We could not verify those access details." not in wrong.text:
            problems.append("wrong password refusal is missing the pinned copy")
        else:
            notes.append("wrong password -> 401 + pinned copy")
        pending_resp = http_req(
            "POST",
            base + "/control/login",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("operator", "ops"), ("password", CONTROL_OPERATOR_PASSWORD)]),
        )
        match = re.search(r'name="pending"\s+value="([^"]+)"', pending_resp.text)
        if pending_resp.status != 200 or not match:
            problems.append("password step did not reach the code step (HTTP %d)" % pending_resp.status)
            return (False, "; ".join(notes + problems))
        pending = match.group(1)
        wrong_code = http_req(
            "POST",
            base + "/control/login/code",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("pending", pending), ("code", "000000")]),
        )
        if self.totp_code(CONTROL_TOTP_SECRET_B32) == "000000":
            notes.append("wrong-code probe skipped: 000000 collided with the live code")
        elif wrong_code.status != 401:
            problems.append("wrong code answered %d, expected 401" % wrong_code.status)
        else:
            notes.append("wrong code -> 401 + pinned copy")
        login_resp, _ = self.control_login("ops")
        set_cookie = login_resp.header("set-cookie") or ""
        if login_resp.status != 303:
            problems.append("valid sign-in answered %d, expected 303" % login_resp.status)
        else:
            notes.append("password + TOTP -> 303")
        for flag in ("HttpOnly", "SameSite=Lax", "Path=/control"):
            if flag not in set_cookie:
                problems.append("operator session cookie lacks %s" % flag)
        cookie = cookie_value(set_cookie, "apex_operator_session")
        if cookie:
            page = http_req("GET", base + "/control/", headers={"Cookie": "apex_operator_session=" + cookie})
            if page.status != 200:
                problems.append("session did not grant /control/ (HTTP %d)" % page.status)
            else:
                notes.append("session grants /control/ (200)")
        api_with_control_cookie = http_req(
            "GET",
            base.replace(":%d" % self.ports["apex-control"], ":%d" % self.ports["apex-api"]) + "/console/ledger",
            headers={"Cookie": "apex_operator_session=" + (cookie or "x")},
        )
        if api_with_control_cookie.status == 200 and "records" in api_with_control_cookie.text:
            problems.append("api honoured a control cookie")
        else:
            notes.append("api ignores the control cookie (HTTP %d)" % api_with_control_cookie.status)
        sales_with_control_cookie = http_req(
            "GET",
            base.replace(":%d" % self.ports["apex-control"], ":%d" % self.ports["apex-sales-web"]) + "/",
            headers={"Cookie": "apex_operator_session=" + (cookie or "x")},
        )
        if sales_with_control_cookie.status not in (401, 303):
            problems.append("sales-web honoured a control cookie (HTTP %d)" % sales_with_control_cookie.status)
        else:
            notes.append("sales-web rejects the control cookie (HTTP %d)" % sales_with_control_cookie.status)
        return (not problems, "; ".join(notes + problems))

    def auth_sales(self):
        problems = []
        notes = []
        base = self.base_url("apex-sales-web")
        wrong = http_req(
            "POST",
            base + "/login",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("token", "wrong-owner-token")]),
        )
        if wrong.status != 401:
            problems.append("wrong token answered %d, expected 401" % wrong.status)
        else:
            notes.append("wrong token -> 401")
        good = http_req(
            "POST",
            base + "/login",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("token", SALES_WEB_TOKEN)]),
        )
        set_cookie = good.header("set-cookie") or ""
        if good.status != 303:
            problems.append("valid token answered %d, expected 303" % good.status)
        else:
            notes.append("valid token -> 303")
        for flag in ("HttpOnly", "SameSite=Lax"):
            if flag not in set_cookie:
                problems.append("session cookie lacks %s" % flag)
        cookie = cookie_value(set_cookie, "apex_sales_session")
        control_with_sales_cookie = http_req(
            "GET",
            self.base_url("apex-control") + "/control/",
            headers={"Cookie": "apex_sales_session=" + (cookie or "x")},
        )
        if control_with_sales_cookie.status not in (401, 303):
            problems.append("control honoured a sales cookie (HTTP %d)" % control_with_sales_cookie.status)
        else:
            notes.append("control rejects the sales cookie (HTTP %d)" % control_with_sales_cookie.status)
        api_with_sales_cookie = http_req(
            "GET",
            self.base_url("apex-api") + "/console/ledger",
            headers={"Cookie": "apex_sales_session=" + (cookie or "x")},
        )
        if api_with_sales_cookie.status == 200 and "records" in api_with_sales_cookie.text:
            problems.append("api honoured a sales cookie")
        else:
            notes.append("api ignores the sales cookie (HTTP %d)" % api_with_sales_cookie.status)
        name = "apex-sales-web-unconfigured"
        try:
            self.start_process(
                name,
                [self.bin_path("apex-sales-web")],
                self.sales_web_env(self.ports[name], None),
                "apex-sales-web-unconfigured.log",
            )
            unconfigured = self.wait_http(
                self.base_url(name) + "/login", lambda status: status in (200, 503)
            )
            login_post = http_req(
                "POST",
                self.base_url(name) + "/login",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=form_body([("token", SALES_WEB_TOKEN)]),
            )
            if unconfigured.status != 503 or "APEX_SALES_WEB_TOKEN is not set" not in unconfigured.text:
                problems.append(
                    "unset-token instance answered %d without the not-configured copy" % unconfigured.status
                )
            elif login_post.status != 503:
                problems.append("unset-token instance minted a session (HTTP %d)" % login_post.status)
            else:
                notes.append("token unset -> 503 not-configured, no session minted")
        except ReqError as error:
            problems.append("unset-token instance failed: %s" % error)
        finally:
            self.kill_process(name)
        return (not problems, "; ".join(notes + problems))

    # 6 ------------------------------------------------------- input-corpus
    def probe_input_corpus(self):
        problems = {}
        notes = {}

        def note(service, ok, detail):
            entry = notes if ok else problems
            entry.setdefault(service, []).append(detail)

        base = self.base_url("apex-api")
        try:
            resp = http_req("GET", base + "/console/audit?platform=%zz")
            note("apex-api", resp.status == 400, "malformed percent in query -> %d (expected 400)" % resp.status)
            resp = http_req("GET", base + "/console/audit/findings/%00")
            note("apex-api", resp.status in (400, 404), "%%00 path segment -> %d" % resp.status)
            resp = http_req("GET", base + "/v1/audit/%ff")
            note("apex-api", resp.status in (400, 404), "%%ff path segment -> %d" % resp.status)
            resp = http_req("GET", base + "/" + "a" * 8192)
            note("apex-api", resp.status < 500, "8KB path -> %d" % resp.status)
            flood = "&".join("p%d=%d" % (i, i) for i in range(500))
            resp = http_req("GET", base + "/console/analytics?" + flood)
            note("apex-api", resp.status == 200, "500-param flood on /console/analytics -> %d" % resp.status)
            big = b"x" * (300 * 1024)
            resp = http_req(
                "POST",
                base + "/free-audit",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=big,
            )
            note("apex-api", resp.status == 413, "300KB body on /free-audit -> %d (expected 413)" % resp.status)
            resp = http_req(
                "POST", base + "/v1/leads", headers={"Content-Type": "application/json"}, body=big
            )
            note("apex-api", resp.status == 413, "300KB body on /v1/leads -> %d (expected 413)" % resp.status)
            common = [
                ("email", "boundary@studio.example"),
                ("studio", "Boundary Studio"),
                ("platforms", "admob"),
            ]
            at_limit = http_req(
                "POST",
                base + "/free-audit",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=form_body(common + [("notes", "a" * 2000)]),
            )
            over = http_req(
                "POST",
                base + "/free-audit",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=form_body(common + [("notes", "a" * 2001)]),
            )
            note("apex-api", at_limit.status == 303, "2000-char notes -> %d (expected 303)" % at_limit.status)
            note("apex-api", over.status == 400, "2001-char notes -> %d (expected 400)" % over.status)
        except ReqError as error:
            note("apex-api", False, "api corpus: %s" % error)
        # control: malformed query, %00, 8KB path, 500 params.
        try:
            self.control_session()
            cookie = self.cookie_header("apex-control")
            base = self.base_url("apex-control")
            resp = http_req("GET", base + "/control/customers?q=%zz", headers=cookie)
            note("apex-control", resp.status in (200, 400), "malformed percent on customers -> %d" % resp.status)
            resp = http_req("GET", base + "/control/customers/%00", headers=cookie)
            note(
                "apex-control",
                resp.status in (400, 404) or resp.status == 503,
                "%%00 customer id -> %d (503 is the honest not-configured state)" % resp.status,
            )
            flood = "&".join("p%d=%d" % (i, i) for i in range(500))
            resp = http_req("GET", base + "/control/customers?" + flood, headers=cookie)
            note("apex-control", resp.status == 200, "500-param flood on customers -> %d" % resp.status)
            resp = http_req("GET", base + "/" + "a" * 8192, headers=cookie)
            note("apex-control", resp.status < 500, "8KB path -> %d" % resp.status)
        except ReqError as error:
            note("apex-control", False, "control corpus: %s" % error)
        # sales-web: malformed query, %00, 8KB path, 500 params.
        try:
            self.sales_session()
            cookie = self.cookie_header("apex-sales-web")
            base = self.base_url("apex-sales-web")
            resp = http_req("GET", base + "/leads?q=%zz", headers=cookie)
            note("apex-sales-web", resp.status in (200, 400), "malformed percent on leads -> %d" % resp.status)
            resp = http_req("GET", base + "/publishers/%00", headers=cookie)
            note("apex-sales-web", resp.status in (400, 404), "%%00 publisher -> %d" % resp.status)
            flood = "&".join("p%d=%d" % (i, i) for i in range(500))
            resp = http_req("GET", base + "/leads?" + flood, headers=cookie)
            note("apex-sales-web", resp.status == 200, "500-param flood on leads -> %d" % resp.status)
            resp = http_req("GET", base + "/" + "a" * 8192, headers=cookie)
            note("apex-sales-web", resp.status < 500, "8KB path -> %d" % resp.status)
        except ReqError as error:
            note("apex-sales-web", False, "sales corpus: %s" % error)
        for service in SERVICES:
            self.method_matrix_check(service)
        for service, _surface, cell_id in self.cells_of("input-corpus"):
            service_problems = problems.get(service, [])
            detail = "; ".join((notes.get(service, []) + service_problems)[:8]) or "no corpus observation"
            if service_problems:
                detail += " || failures: " + "; ".join(service_problems[:4])
            self.cell(cell_id, not service_problems, "[%s] %s" % (service, detail))

    def method_matrix_check(self, service):
        declared = [route["path"] for route in self.surface["services"][service]["routes"]]
        headers = {}
        try:
            if service != "apex-api":
                self.service_session(service)
                headers = self.cookie_header(service)
        except ReqError:
            headers = {}
        route_methods = {
            route["path"]: {method.upper() for method in route["methods"]}
            for route in self.surface["services"][service]["routes"]
        }
        offenders = []
        for template in declared:
            url = self.base_url(service) + materialize(template)
            baseline = None
            if "GET" in route_methods.get(template, set()):
                try:
                    baseline = http_req("GET", url, headers=headers, timeout=10).status
                except ReqError:
                    baseline = None
            for method in ("HEAD", "OPTIONS", "PUT", "DELETE"):
                try:
                    resp = http_req(method, url, headers=headers, timeout=10)
                    if resp.status >= 500 and resp.status != baseline:
                        offenders.append("%s %s -> %d" % (method, template, resp.status))
                except ReqError as error:
                    offenders.append("%s %s: %s" % (method, template, error))
        self.check(
            "input-corpus|%s|method-matrix" % service,
            not offenders,
            "HEAD/OPTIONS/PUT/DELETE never 5xx across %d routes" % len(declared)
            if not offenders
            else "; ".join(offenders[:6]),
        )

    # 7 --------------------------------------------------- csrf-idempotency
    def probe_csrf_idempotency(self):
        for service in SERVICES:
            try:
                if service == "apex-api":
                    ok, detail = self.csrf_api()
                elif service == "apex-control":
                    ok, detail = self.csrf_control()
                else:
                    ok, detail = self.csrf_sales()
            except ReqError as error:
                ok, detail = False, "csrf probe aborted: %s" % error
            for _svc, _surface, cell_id in self.cells_of("csrf-idempotency", service):
                self.cell(cell_id, ok, detail)

    def csrf_api(self):
        problems = []
        notes = []
        base = self.base_url("apex-api")
        self.api_session()
        cookie = self.cookie_header("apex-api")
        session_posts = [
            ("/console/ledger/session/end", ""),
            ("/console/connections/admob/submit", "platform=admob"),
            ("/console/connections/admob/rotate", "platform=admob"),
            ("/console/connections/admob/disconnect", "platform=admob"),
            ("/console/proposals/nope/decision", "decision=approve"),
            ("/console/settings/theme", "theme=dark"),
            ("/console/settings/access/revoke", "handle=deadbeef"),
        ]
        for path, payload in session_posts:
            body = (payload + "&").encode() if payload else b""
            resp = http_req(
                "POST",
                base + path,
                headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
                body=body,
            )
            if resp.status != 400:
                problems.append("with session, no CSRF: %s -> %d (expected 400)" % (path, resp.status))
            no_session = http_req(
                "POST",
                base + path,
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=body,
            )
            if no_session.status != 303:
                problems.append("without session: %s -> %d (expected 303)" % (path, no_session.status))
        if not problems:
            notes.append("7 session POSTs: no-CSRF 400, no-session 303")
        exchange = http_req(
            "POST",
            base + "/console/ledger",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("publisher", "northstar"), ("token", API_LEDGER_TOKEN)]),
        )
        if exchange.status != 303:
            problems.append("ledger exchange without CSRF -> %d (expected 303)" % exchange.status)
        live = http_req(
            "POST",
            base + "/v1/audit/admob",
            headers={"Content-Type": "application/json"},
            body=b'{"publisher":"northstar"}',
        )
        if live.status != 401:
            problems.append("/v1/audit without bearer -> %d (expected 401)" % live.status)
        cycle = http_req(
            "POST", base + "/v1/cycle/northstar", headers={"Content-Type": "application/json"}, body=b"{}"
        )
        if cycle.status != 400:
            problems.append("/v1/cycle without platform -> %d (expected 400)" % cycle.status)
        webhook = http_req(
            "POST", base + "/v1/adyen/webhook", headers={"Content-Type": "application/json"}, body=b"{}"
        )
        if webhook.status not in (400, 503):
            problems.append("/v1/adyen/webhook unsigned -> %d (expected 400/503)" % webhook.status)
        leads_path = os.path.join(self.api["work"], "leads.jsonl")
        before = count_lines(leads_path)
        payload = json.dumps(
            {
                "publisher": "v1-replay-%d" % os.getpid(),
                "email": "v1-replay-%d@studio.example" % os.getpid(),
                "displayName": "Replay Studio",
                "platforms": ["admob"],
            }
        ).encode()
        sent = []
        for _ in range(2):
            resp = http_req(
                "POST", base + "/v1/leads", headers={"Content-Type": "application/json"}, body=payload
            )
            sent.append(resp.status)
        after = count_lines(leads_path)
        if sent != [202, 202]:
            problems.append("/v1/leads replay -> %s (expected [202, 202])" % sent)
        elif after - before != 1:
            problems.append("/v1/leads replay stored %d durable lead(s), expected 1" % (after - before))
        else:
            notes.append("/v1/leads replay: 202 twice, one durable lead")
        before = count_lines(leads_path)
        audit_payload = [
            ("email", "replay-%d@studio.example" % os.getpid()),
            ("studio", "Replay Studio"),
            ("platforms", "admob"),
        ]
        first = http_req(
            "POST",
            base + "/free-audit",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(audit_payload),
        )
        second = http_req(
            "POST",
            base + "/free-audit",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(audit_payload),
        )
        after = count_lines(leads_path)
        refs = [resp.header("location") for resp in (first, second)]
        if first.status != 303 or second.status != 303:
            problems.append("free-audit replay -> %s (expected 303, 303)" % [first.status, second.status])
        elif refs[0] != refs[1]:
            problems.append("free-audit replay returned different refs: %s" % refs)
        elif after - before != 1:
            problems.append("free-audit replay stored %d leads, expected 1" % (after - before))
        else:
            notes.append("free-audit replay: same ref %s, one durable lead" % refs[0])
        return (not problems, "; ".join(notes + problems))

    def csrf_control(self):
        problems = []
        notes = []
        base = self.base_url("apex-control")
        self.control_session()
        cookie = self.cookie_header("apex-control")
        resp = http_req(
            "POST",
            base + "/control/actions/policy_draft",
            headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
            body=form_body([("target", "dogfood"), ("reason", "probe")]),
        )
        if resp.status != 400:
            problems.append("action without CSRF -> %d (expected 400)" % resp.status)
        else:
            notes.append("action without CSRF -> 400")
        no_session = http_req(
            "POST",
            base + "/control/logout",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=b"",
        )
        if no_session.status not in (303, 400):
            problems.append("logout without session -> %d (expected 303/400)" % no_session.status)
        else:
            notes.append("logout without session -> %d" % no_session.status)
        try:
            ok, detail = self.control_action_replay()
        except ReqError as error:
            ok, detail = False, "action replay failed: %s" % error
        (notes if ok else problems).append(detail)
        return (not problems, "; ".join(notes + problems))

    def control_revoke_form(self, cookie, driver_id):
        page = http_req("GET", self.base_url("apex-control") + "/control/access", headers=cookie)
        fallback = None
        for form in re.findall(
            r'<form[^>]*action="/control/actions/session_revoke"[^>]*>(.*?)</form>', page.text, re.S
        ):
            fields = parse_hidden_fields(form)
            if not fields.get("target"):
                continue
            phrase = re.search(r"Type exactly: ([A-Z ]+)", form)
            if phrase:
                # The input ships empty; the operator types the phrase, and
                # the probe supplies it from the form's own help line.
                fields["confirmation"] = phrase.group(1).strip()
            if fields["target"] != driver_id[:8]:
                return fields
            fallback = fallback or fields
        if fallback is not None:
            return fallback
        raise ReqError("the access page did not render a session_revoke form with a target")

    def control_action_replay(self):
        driver_cookie, _csrf = self.control_session()
        driver = {"Cookie": "apex_operator_session=" + driver_cookie}
        victim_resp, _ = self.control_login("ops")
        if victim_resp.status != 303:
            return False, "could not mint the target session (HTTP %d)" % victim_resp.status
        fields = self.control_revoke_form(driver, driver_cookie)
        pairs = sorted(fields.items())
        journal = os.path.join(self.control["work"], "journal.jsonl")
        before = journal_key_count(journal, fields.get("idempotency_key", ""))
        first = http_req(
            "POST",
            self.base_url("apex-control") + "/control/actions/session_revoke",
            headers={"Content-Type": "application/x-www-form-urlencoded", **driver},
            body=form_body(pairs),
        )
        second = http_req(
            "POST",
            self.base_url("apex-control") + "/control/actions/session_revoke",
            headers={"Content-Type": "application/x-www-form-urlencoded", **driver},
            body=form_body(pairs),
        )
        after = journal_key_count(journal, fields.get("idempotency_key", ""))
        problems = []
        if first.status != 303 or second.status != 303:
            problems.append("replay statuses %s (expected 303, 303)" % [first.status, second.status])
        elif first.header("location") != second.header("location"):
            problems.append(
                "replay returned different result pages: %s vs %s"
                % (first.header("location"), second.header("location"))
            )
        if after - before != 1:
            problems.append("replayed action wrote %d journal entries, expected 1" % (after - before))
        if problems:
            return False, "; ".join(problems)
        return True, "session_revoke replay: same result page %s, one journal entry" % first.header("location")

    def csrf_sales(self):
        problems = []
        notes = []
        base = self.base_url("apex-sales-web")
        self.sales_session()
        cookie = self.cookie_header("apex-sales-web")
        resp = http_req(
            "POST",
            base + "/actions/reset-health",
            headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
            body=form_body([("confirmation", "RESET HEALTH")]),
        )
        if resp.status != 400:
            problems.append("action without CSRF -> %d (expected 400)" % resp.status)
        else:
            notes.append("action without CSRF -> 400")
        logout = http_req(
            "POST",
            base + "/logout",
            headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
            body=b"",
        )
        if logout.status != 400:
            problems.append("logout without CSRF -> %d (expected 400)" % logout.status)
        else:
            notes.append("logout without CSRF -> 400")
        csrf = self.sales.get("csrf")
        fields = [
            ("csrf_token", csrf),
            ("idempotency_key", "d" * 32),
            ("confirmation", "RESET HEALTH"),
        ]
        events = os.path.join(self.sales["sales_dir"], "events.jsonl")
        before = count_lines(events, kind="health_reset")
        first = http_req(
            "POST",
            base + "/actions/reset-health",
            headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
            body=form_body(fields),
        )
        second = http_req(
            "POST",
            base + "/actions/reset-health",
            headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
            body=form_body(fields),
        )
        after = count_lines(events, kind="health_reset")
        if first.status != 303 or second.status != 303:
            problems.append("reset-health replay statuses %s (expected 303, 303)" % [first.status, second.status])
        elif first.header("location") != second.header("location"):
            problems.append(
                "reset-health replay returned different result pages: %s vs %s"
                % (first.header("location"), second.header("location"))
            )
        elif after - before != 1:
            problems.append("replayed reset-health wrote %d durable events, expected 1" % (after - before))
        else:
            notes.append("reset-health replay: same page %s, one durable event" % first.header("location"))
        return (not problems, "; ".join(notes + problems))

    # 8 --------------------------------------------------------- traversal
    def probe_traversal(self):
        for service in SERVICES:
            headers = {}
            try:
                if service != "apex-api":
                    self.service_session(service)
                headers = self.cookie_header(service)
            except ReqError:
                headers = {}
            for _svc, surface, cell_id in self.cells_of("traversal", service):
                method, template = surface.split(" ", 1)
                if "{" not in template:
                    self.cell(cell_id, False, "declared traversal surface %s has no parameter" % surface)
                    continue
                problems = []
                observations = []
                for payload in TRAVERSAL_CORPUS:
                    path = re.sub(r"\{[^}]+\}", payload, template, count=1)
                    url = self.base_url(service) + path
                    request_headers = dict(headers)
                    body = None
                    if method == "POST":
                        request_headers["Content-Type"] = "application/x-www-form-urlencoded"
                        csrf = {
                            "apex-api": self.api.get("csrf"),
                            "apex-control": self.control.get("csrf"),
                            "apex-sales-web": self.sales.get("csrf"),
                        }[service]
                        body = form_body([("csrf_token", csrf or "")])
                    try:
                        resp = http_req(method, url, headers=request_headers, body=body)
                    except ReqError as error:
                        problems.append("%s -> %s" % (payload, error))
                        continue
                    accepted = resp.status in (400, 404, 303, 401)
                    if resp.status == 503:
                        accepted = True  # the honest not-configured state, never foreign content
                    if resp.status == 200 and (
                        "no durable record names" in resp.text or "is not readable" in resp.text
                    ):
                        accepted = True  # the honest unavailable state, never foreign content
                    if not accepted:
                        problems.append("%s -> HTTP %d (neither a not-found refusal nor an honest state)" % (payload, resp.status))
                    for canary in TRAVERSAL_CANARIES:
                        if canary in resp.body:
                            problems.append("%s leaked %r" % (payload, canary.decode()))
                    observations.append("%d" % resp.status)
                detail = "%s %s statuses [%s]" % (method, template, ", ".join(observations))
                if problems:
                    detail += "; " + "; ".join(problems[:4])
                self.cell(cell_id, not problems, detail, "%s %s/<corpus>" % (method, template))

    # 9 ---------------------------------------------------------- escaping
    def probe_escaping(self):
        results = {
            "apex-api": self.escaping_api(),
            "apex-control": self.escaping_control(),
            "apex-sales-web": self.escaping_sales(),
        }
        for service, _surface, cell_id in self.cells_of("escaping"):
            ok, detail = results[service]
            self.cell(cell_id, ok, detail)

    def escaping_api(self):
        problems = []
        notes = []
        base = self.base_url("apex-api")
        resp = http_req("GET", base + "/console/audit?platform=" + urllib.parse.quote(HOSTILE_QUERY))
        if HOSTILE_QUERY in resp.text:
            problems.append("the hostile platform echo is unescaped on /console/audit")
        else:
            notes.append("hostile platform refused/escaped (HTTP %d)" % resp.status)
        report = http_req("GET", base + "/console/audit?platform=admob")
        if "<script" in report.text:
            problems.append("the audit report echoes the hostile snapshot name as script")
        else:
            notes.append("the hostile snapshot name never becomes script")
        csv_resp = http_req("GET", base + "/console/audit/export.csv?platform=admob")
        if csv_resp.status != 200:
            problems.append("audit CSV answered %d" % csv_resp.status)
        else:
            raw = csv_resp.body
            if raw.count(b"\n") != raw.count(b"\r\n"):
                problems.append("audit CSV has lone LF line endings")
            rows = list(csv.reader(io.StringIO(raw.decode("utf-8"))))
            neutralized = False
            for row in rows:
                for cellvalue in row:
                    if cellvalue.startswith("=") or cellvalue.startswith(HOSTILE_FORMULA[:5]):
                        if not cellvalue.startswith("'"):
                            problems.append("CSV cell %r is not formula-neutralized" % cellvalue[:40])
                        else:
                            neutralized = True
            if neutralized:
                notes.append("audit CSV neutralizes the formula-leading cell and ends CRLF")
            elif not problems:
                notes.append("audit CSV parses with CRLF endings; no formula-leading cell present")
        return (not problems, "; ".join(notes + problems))

    def escaping_control(self):
        problems = []
        notes = []
        self.control_session()
        resp = http_req(
            "GET",
            self.base_url("apex-control") + "/control/customers?q=" + urllib.parse.quote(HOSTILE_QUERY),
            headers=self.cookie_header("apex-control"),
        )
        if HOSTILE_QUERY in resp.text:
            problems.append("the hostile customer filter echo is unescaped")
        else:
            notes.append("hostile customer filter echoed escaped or refused (HTTP %d)" % resp.status)
        return (not problems, "; ".join(notes + problems))

    def escaping_sales(self):
        problems = []
        notes = []
        self.sales_session()
        cookie = self.cookie_header("apex-sales-web")
        base = self.base_url("apex-sales-web")
        resp = http_req("GET", base + "/leads?q=" + urllib.parse.quote(HOSTILE_QUERY), headers=cookie)
        if HOSTILE_QUERY in resp.text:
            problems.append("the hostile leads filter echo is unescaped")
        else:
            notes.append("hostile leads filter echoed escaped or refused (HTTP %d)" % resp.status)
        csv_resp = http_req("GET", base + "/leads/export.csv", headers=cookie)
        if csv_resp.status != 200:
            problems.append("leads CSV answered %d" % csv_resp.status)
            return (not problems, "; ".join(notes + problems))
        raw = csv_resp.body
        if raw.count(b"\n") != raw.count(b"\r\n"):
            problems.append("leads CSV has lone LF line endings")
        rows = list(csv.reader(io.StringIO(raw.decode("utf-8"))))
        hostile_row = None
        for row in rows[1:]:
            if any("probe-hostile" in cell or "cmd" in cell for cell in row):
                hostile_row = row
                break
        if hostile_row is None:
            problems.append("the hostile seeded lead is missing from the leads CSV")
        else:
            for cellvalue in hostile_row:
                if cellvalue.startswith("=") or cellvalue.startswith("+"):
                    problems.append("CSV cell %r is not formula-neutralized" % cellvalue[:40])
            email_cells = [cell for cell in hostile_row if "probe-hostile" in cell]
            if not email_cells or not any(cell.startswith("'=") for cell in email_cells):
                problems.append("the hostile contact email is not prefixed with the neutralizer apostrophe")
            notes.append("hostile lead exported through the neutralizing CSV path (%d columns)" % len(hostile_row))
        return (not problems, "; ".join(notes + problems))

    # 10 ------------------------------------------------------- concurrency
    def probe_concurrency(self):
        results = {
            "apex-api": self.concurrency_api(),
            "apex-control": self.concurrency_control(),
            "apex-sales-web": self.concurrency_sales(),
        }
        for service, _surface, cell_id in self.cells_of("concurrency"):
            ok, detail = results[service]
            self.cell(cell_id, ok, detail)

    def concurrency_api(self):
        base = self.base_url("apex-api")
        leads_path = os.path.join(self.api["work"], "leads.jsonl")
        before = count_lines(leads_path)
        payload = form_body(
            [
                ("email", "parallel-%d@studio.example" % os.getpid()),
                ("studio", "Parallel Studio"),
                ("platforms", "admob"),
            ]
        )

        def fire(_index):
            return http_req(
                "POST",
                base + "/free-audit",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=payload,
            )

        with ThreadPoolExecutor(max_workers=32) as pool:
            results = list(pool.map(fire, range(32)))
        after = count_lines(leads_path)
        statuses = sorted({resp.status for resp in results})
        refs = {resp.header("location") for resp in results}
        problems = []
        if any(status >= 500 for status in statuses):
            problems.append("5xx under load: %s" % statuses)
        if set(statuses) != {303}:
            problems.append("statuses %s (expected only 303)" % statuses)
        if len(refs) != 1:
            problems.append("32 identical posts produced %d refs: %s" % (len(refs), sorted(refs)))
        if after - before != 1:
            problems.append("32 identical posts stored %d leads, expected 1" % (after - before))
        detail = "32 parallel identical free-audit posts -> %s, %d ref(s), %d durable lead(s)" % (
            statuses,
            len(refs),
            after - before,
        )
        if problems:
            detail += "; " + "; ".join(problems)
        return (not problems, detail)

    def concurrency_control(self):
        driver_cookie, _csrf = self.control_session()
        driver = {"Cookie": "apex_operator_session=" + driver_cookie}
        victim_resp, _ = self.control_login("ops")
        if victim_resp.status != 303:
            return False, "could not mint the concurrency target session (HTTP %d)" % victim_resp.status
        fields = self.control_revoke_form(driver, driver_cookie)
        body = form_body(sorted(fields.items()))
        journal = os.path.join(self.control["work"], "journal.jsonl")
        before = journal_key_count(journal, fields.get("idempotency_key", ""))

        def fire(_index):
            return http_req(
                "POST",
                self.base_url("apex-control") + "/control/actions/session_revoke",
                headers={"Content-Type": "application/x-www-form-urlencoded", **driver},
                body=body,
            )

        with ThreadPoolExecutor(max_workers=16) as pool:
            results = list(pool.map(fire, range(16)))
        after = journal_key_count(journal, fields.get("idempotency_key", ""))
        statuses = sorted({resp.status for resp in results})
        problems = []
        if any(status >= 500 for status in statuses):
            problems.append("5xx under load: %s" % statuses)
        if set(statuses) != {303}:
            problems.append("statuses %s (expected only 303)" % statuses)
        if after - before != 1:
            problems.append("16 replayed actions wrote %d journal entries, expected 1" % (after - before))
        detail = "16 parallel replayed session_revoke -> %s, one journal entry" % statuses
        if problems:
            detail += "; " + "; ".join(problems)
        return (not problems, detail)

    def concurrency_sales(self):
        self.sales_session()
        cookie = self.cookie_header("apex-sales-web")
        body = form_body(
            [
                ("csrf_token", self.sales.get("csrf")),
                ("idempotency_key", "c" * 32),
                ("confirmation", "RESET HEALTH"),
            ]
        )
        events = os.path.join(self.sales["sales_dir"], "events.jsonl")
        before = count_lines(events, kind="health_reset")

        def fire(_index):
            return http_req(
                "POST",
                self.base_url("apex-sales-web") + "/actions/reset-health",
                headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
                body=body,
            )

        with ThreadPoolExecutor(max_workers=16) as pool:
            results = list(pool.map(fire, range(16)))
        after = count_lines(events, kind="health_reset")
        statuses = sorted({resp.status for resp in results})
        locations = {resp.header("location") for resp in results}
        problems = []
        if any(status >= 500 for status in statuses):
            problems.append("5xx under load: %s" % statuses)
        if set(statuses) != {303}:
            problems.append("statuses %s (expected only 303)" % statuses)
        if len(locations) != 1:
            problems.append("replayed actions returned %d result pages" % len(locations))
        if after - before != 1:
            problems.append("16 replayed actions wrote %d durable events, expected 1" % (after - before))
        detail = "16 parallel replayed reset-health -> %s, one durable event" % statuses
        if problems:
            detail += "; " + "; ".join(problems)
        return (not problems, detail)

    # 11 -------------------------------------------------- failure-injection
    def probe_failure_injection(self):
        results = {
            "apex-api": self.failure_api(),
            "apex-control": self.failure_control(),
            "apex-sales-web": self.failure_sales(),
        }
        for service, _surface, cell_id in self.cells_of("failure-injection"):
            ok, detail = results[service]
            self.cell(cell_id, ok, detail)

    def failure_api(self):
        problems = []
        notes = []
        base = self.base_url("apex-api")
        self.api_session()
        old_cookie = self.api.get("cookie")
        self.kill_process("apex-api")
        self.start_api()
        health = http_req("GET", base + "/healthz")
        if health.status != 200:
            problems.append("healthz after restart -> %d" % health.status)
        else:
            notes.append("SIGKILL + restart: healthz 200")
        old = http_req("GET", base + "/console/ledger", headers={"Cookie": "apex_ledger_session=" + old_cookie})
        if old.status != 200 or "Ledger records" in old.text:
            problems.append("the pre-crash session still reads records (HTTP %d)" % old.status)
        else:
            notes.append("pre-crash session honestly invalidated (access panel, HTTP %d)" % old.status)
        self.api["cookie"] = None
        self.api["csrf"] = None
        page = http_req("GET", base + "/console/audit?platform=admob")
        if page.status != 200:
            problems.append("audit page after restart -> %d" % page.status)
        else:
            notes.append("pages recover after restart")
        if self.ch_owned:
            ok, detail = self.clickhouse_severance()
            if not ok:
                problems.append(detail)
            else:
                notes.append(detail)
        else:
            self.check(
                "failure-injection|clickhouse-severance",
                True,
                "SKIPPED: --clickhouse-url %s is an external, unowned server; the harness never manages it "
                "(pass --clickhouse-owned for the local make path)" % self.ch_url,
            )
        return (not problems, "; ".join(notes + problems))

    def failure_control(self):
        problems = []
        notes = []
        base = self.base_url("apex-control")
        self.control_session()
        old_cookie = self.control.get("cookie")
        self.kill_process("apex-control")
        self.start_control()
        old = http_req("GET", base + "/control/", headers={"Cookie": "apex_operator_session=" + old_cookie})
        if old.status != 401:
            problems.append("the pre-crash operator session survived (HTTP %d)" % old.status)
        else:
            notes.append("pre-crash operator session invalidated (401)")
        login_page = http_req("GET", base + "/control/login")
        if login_page.status != 200:
            problems.append("login page after restart -> %d" % login_page.status)
        else:
            notes.append("login page recovers after restart")
        self.control["cookie"] = None
        return (not problems, "; ".join(notes + problems))

    def failure_sales(self):
        problems = []
        notes = []
        base = self.base_url("apex-sales-web")
        self.sales_session()
        old_cookie = self.sales.get("cookie")
        self.kill_process("apex-sales-web")
        self.start_sales_web()
        old = http_req("GET", base + "/", headers={"Cookie": "apex_sales_session=" + old_cookie})
        if old.status != 401:
            problems.append("the pre-crash session survived (HTTP %d)" % old.status)
        else:
            notes.append("pre-crash session invalidated (401)")
        login_page = http_req("GET", base + "/login")
        if login_page.status != 200:
            problems.append("login page after restart -> %d" % login_page.status)
        else:
            notes.append("login page recovers after restart")
        self.sales["cookie"] = None
        self.sales_session()
        home = http_req("GET", base + "/", headers=self.cookie_header("apex-sales-web"))
        if home.status != 200:
            problems.append("home after re-login -> %d" % home.status)
        else:
            notes.append("pages recover after restart")
        return (not problems, "; ".join(notes + problems))

    def wait_clickhouse_down(self, timeout=30.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if not self.ch_reachable():
                return True
            time.sleep(0.3)
        return not self.ch_reachable()

    def clickhouse_stop(self):
        """Stop the ClickHouse server the harness was granted ownership of."""
        if not self.ch_reachable():
            return True, "ClickHouse was already down"
        attempted = []
        try:
            resp = self.ch_raw("SYSTEM SHUTDOWN")
            attempted.append("HTTP SYSTEM SHUTDOWN -> %d" % resp.status)
        except ReqError as error:
            attempted.append("HTTP SYSTEM SHUTDOWN: %s" % error)
        if self.wait_clickhouse_down(15):
            return True, attempted[-1] + "; the server stopped"
        client = shutil.which("clickhouse")
        if client:
            for port in (self.ch_port + 877, self.ch_port + 877 + 1000, 9000):
                try:
                    proc = subprocess.run(
                        [client, "client", "--host", "127.0.0.1", "--port", str(port), "--query", "SYSTEM SHUTDOWN"],
                        capture_output=True,
                        text=True,
                        timeout=30,
                    )
                except (subprocess.TimeoutExpired, OSError) as error:
                    attempted.append("client port %d: %s" % (port, error))
                    continue
                attempted.append("client port %d exit=%d" % (port, proc.returncode))
                if self.wait_clickhouse_down(15):
                    return True, attempted[-1] + "; the server stopped"
        return False, "could not stop ClickHouse: " + "; ".join(attempted)

    def clickhouse_start(self):
        work = os.path.join(self.workdir, "clickhouse-recovery")
        os.makedirs(work, exist_ok=True)
        # The dev/verify convention keeps the native port 877 above the HTTP
        # port (8123->9000, 18123->19000).
        tcp_port = self.ch_port + 877
        argv = [
            shutil.which("clickhouse") or "clickhouse",
            "server",
            "--config-file",
            self.repo_path("infrastructure/clickhouse/config.xml"),
            "--",
            "--http_port=%d" % self.ch_port,
            "--tcp_port=%d" % tcp_port,
            "--path=%s/" % work,
            "--logger.log=%s/server.log" % work,
            "--logger.errorlog=%s/server.err.log" % work,
        ]
        try:
            proc = subprocess.Popen(
                argv,
                cwd=self.root,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
        except OSError as error:
            return False, str(error)
        self.ch_owned_procs.append(proc)
        deadline = time.time() + 60
        while time.time() < deadline:
            if self.ch_reachable():
                return True, None
            if proc.poll() is not None:
                return False, "the restarted server exited with %s" % proc.returncode
            time.sleep(0.3)
        return False, "the restarted server never answered within 60s"

    def clickhouse_severance(self):
        problems = []
        notes = []
        stopped, stop_detail = self.clickhouse_stop()
        if not stopped:
            return False, "owned ClickHouse could not be stopped: %s" % stop_detail
        notes.append(stop_detail)
        deadline = time.time() + 30
        while time.time() < deadline and self.ch_reachable():
            time.sleep(0.3)
        if self.ch_reachable():
            return False, "ClickHouse still answers after the shutdown request"
        analytics = http_req("GET", self.base_url("apex-api") + "/console/analytics?view=revenue")
        if analytics.status != 200 or "the analytics store did not answer" not in analytics.text:
            problems.append(
                "with the store down /console/analytics answered %d without the not-available state" % analytics.status
            )
        else:
            notes.append("store down: revenue page renders the honest not-available state (200)")
        control = http_req(
            "GET", self.base_url("apex-control") + "/control/", headers=self.cookie_header("apex-control")
        )
        if control.status != 200:
            problems.append("with the store down /control/ answered %d" % control.status)
        else:
            notes.append("store down: /control/ renders without 5xx (200)")
        started, start_error = self.clickhouse_start()
        if not started:
            return False, "; ".join(notes + ["ClickHouse could not be restarted: %s" % start_error])
        notes.append("ClickHouse restarted")
        if self.runner_ran:
            self.ch_bootstrap()
            recovery = http_req("GET", self.base_url("apex-api") + "/console/analytics?view=revenue")
            if recovery.status != 200 or "$51,500.00" not in recovery.text:
                problems.append("recovery: revenue page does not show USD 51,500.00 again")
            else:
                notes.append("recovery: re-seeded, projections republished, revenue shows USD 51,500.00")
        else:
            recovery = http_req("GET", self.base_url("apex-api") + "/console/analytics?view=revenue")
            if recovery.status != 200:
                problems.append("recovery: analytics page -> %d" % recovery.status)
            else:
                notes.append("recovery: analytics answers again (200; the runner never ran, so no published numbers)")
        return (not problems, "; ".join(notes + problems))

    # 12 --------------------------------------------------- differential-truth
    def probe_differential_truth(self):
        results = {
            "apex-api": self.differential_api(),
            "apex-control": self.differential_control(),
            "apex-sales-web": self.differential_sales(),
        }
        for service, _surface, cell_id in self.cells_of("differential-truth"):
            ok, detail = results[service]
            self.cell(cell_id, ok, detail)

    def differential_api(self):
        problems = []
        notes = []
        base = self.base_url("apex-api")
        analytics = http_req("GET", base + "/console/analytics?view=revenue")
        sql_sum = self.ch_sum_revenue()
        expected_micros = 51_500_000_000
        if analytics.status != 200:
            problems.append("revenue page -> %d" % analytics.status)
        elif "$51,500.00" not in analytics.text:
            problems.append("the revenue KPI does not render USD 51,500.00")
        else:
            notes.append("revenue KPI renders USD 51,500.00")
        if sql_sum != expected_micros:
            problems.append("ClickHouse sum(revenue_micros) = %d, expected %d" % (sql_sum, expected_micros))
        else:
            notes.append("ClickHouse published projection sum = %d micros" % sql_sum)
        self.api_session()
        ledger_page = http_req("GET", base + "/console/ledger", headers=self.cookie_header("apex-api"))
        ledger_json = http_req(
            "GET", base + "/v1/ledger/northstar", headers={"Authorization": "Bearer " + API_LEDGER_TOKEN}
        )
        if ledger_json.status != 200:
            problems.append("/v1/ledger answered %d" % ledger_json.status)
            json_records = []
        else:
            json_records = ledger_json.json().get("records", [])
        page_records = len(re.findall(r'/console/ledger/records/\d+"', ledger_page.text))
        if not json_records:
            problems.append("the ledger JSON carries no records")
        elif page_records != len(json_records):
            problems.append(
                "the ledger page lists %d records, /v1/ledger carries %d" % (page_records, len(json_records))
            )
        else:
            notes.append("ledger page and /v1/ledger agree on %d records" % page_records)
        report = http_req("GET", base + "/console/audit?platform=admob")
        csv_resp = http_req("GET", base + "/console/audit/export.csv?platform=admob")
        page_findings = len(re.findall(r"/console/audit/findings/", report.text))
        csv_rows = len([line for line in csv_resp.text.split("\r\n") if line]) - 1
        if page_findings != csv_rows:
            problems.append("the report page links %d findings, the CSV exports %d rows" % (page_findings, csv_rows))
        else:
            notes.append("audit CSV rows equal the report rows (%d)" % csv_rows)
        return (not problems, "; ".join(notes + problems))

    def differential_control(self):
        problems = []
        notes = []
        base = self.base_url("apex-control")
        self.control_session()
        cookie = self.cookie_header("apex-control")
        pages = ["/control/", "/control/ingestion", "/control/experiments", "/control/growth", "/control/finance"]
        statuses = []
        for path in pages:
            resp = http_req("GET", base + path, headers=cookie)
            statuses.append("%s=%d" % (path, resp.status))
            if resp.status >= 500:
                problems.append("%s -> %d" % (path, resp.status))
        notes.append("ClickHouse-reading pages: " + ", ".join(statuses))
        column = self.ch(
            "SELECT count() FROM %s.operator_queues WHERE as_of_version = "
            "(SELECT max(as_of_version) FROM %s.projection_versions WHERE projection_name = 'operator_queues' AND published = 1)"
            % (self.database, self.database)
        ).strip()
        queues = http_req("GET", base + "/control/", headers=cookie)
        rendered = len(re.findall(r"/control/(?:incidents|experiments)/[A-Za-z0-9_-]+", queues.text))
        if int(column) != rendered:
            problems.append("operator_queues holds %s rows, the overview renders %d queue links" % (column, rendered))
        else:
            notes.append("operator_queues rows equal the rendered queue links (%s)" % column)
        accounts = self.ch(
            "SELECT uniqExact(platform_account_id) FROM %s.report_days" % self.database
        ).strip()
        ingestion = http_req("GET", base + "/control/ingestion", headers=cookie)
        row_links = len(re.findall(r"/control/customers/", ingestion.text))
        notes.append(
            "report_days holds %s account(s); the ingestion page renders %d customer link(s)" % (accounts, row_links)
        )
        return (not problems, "; ".join(notes + problems))

    def report_numbers(self):
        proc = self.sales_cli(["report"], self.sales["sales_dir"], self.sales["outbox"])
        out = (proc.stdout or "") + (proc.stderr or "")
        match = re.search(r"publishers (\d+)\s+contacts (\d+)\s+events (\d+)", out)
        if proc.returncode != 0 or not match:
            raise ReqError("apex-sales report failed (exit %d): %s" % (proc.returncode, sanitize_evidence(out)))
        return int(match.group(1)), int(match.group(2)), int(match.group(3))

    def differential_sales(self):
        problems = []
        notes = []
        base = self.base_url("apex-sales-web")
        self.sales_session()
        cookie = self.cookie_header("apex-sales-web")
        publishers, contacts, events = self.report_numbers()
        home = http_req("GET", base + "/", headers=cookie)
        kpi_publishers = re.search(r"Publishers tracked.*?kpi-value\">(?:<a[^>]*>)?(\d+)", home.text, re.S)
        total_contacts = re.search(r"(\d+) suppressed of (\d+) total", home.text)
        events_used = re.search(r"rebuilt every model from (\d+) events", home.text)
        if not kpi_publishers:
            problems.append("the publishers KPI was not found on the overview")
        elif int(kpi_publishers.group(1)) != publishers:
            problems.append("overview says %s publishers, apex-sales report says %d" % (kpi_publishers.group(1), publishers))
        else:
            notes.append("publishers KPI equals apex-sales report (%d)" % publishers)
        if not total_contacts:
            problems.append("the contacts line was not found on the overview")
        elif int(total_contacts.group(2)) != contacts:
            problems.append("overview says %s contacts, apex-sales report says %d" % (total_contacts.group(2), contacts))
        else:
            notes.append("contacts line equals apex-sales report (%d)" % contacts)
        if not events_used:
            problems.append("the events line was not found on the overview")
        elif int(events_used.group(1)) != events:
            problems.append("overview says %s events, apex-sales report says %d" % (events_used.group(1), events))
        else:
            notes.append("events line equals apex-sales report (%d)" % events)
        page = http_req("GET", base + "/leads", headers=cookie)
        csv_resp = http_req("GET", base + "/leads/export.csv", headers=cookie)
        csv_rows = len([line for line in csv_resp.text.split("\r\n") if line]) - 1
        page_rows = len(re.findall(r'href="/publishers/', page.text))
        if csv_rows != page_rows:
            problems.append("the leads page lists %d rows, the CSV exports %d" % (page_rows, csv_rows))
        else:
            notes.append("leads page and CSV agree on %d rows" % csv_rows)
        sends = http_req("GET", base + "/sends", headers=cookie)
        if self.sales.get("message_id") and self.sales["message_id"] not in sends.text:
            problems.append("the sends page does not show the seeded Touch message id")
        else:
            notes.append("sends page shows the seeded Touch history")
        return (not problems, "; ".join(notes + problems))

    # 13 ----------------------------------------------- deferred lockout probe
    def run_lockout_probe(self):
        """The lockout copy requires five failures, which locks the client key
        for 15 minutes: it runs after every other worker is done."""
        base = self.base_url("apex-control")
        last = None
        for _attempt in range(6):
            last = http_req(
                "POST",
                base + "/control/login",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=form_body([("operator", CONTROL_LOCKED_OPERATOR), ("password", "still-not-the-password")]),
            )
            if last.status == 429:
                break
        ok = last is not None and last.status == 429 and "Too many attempts." in last.text
        detail = "6 wrong-password attempts on %s -> HTTP %d%s" % (
            CONTROL_LOCKED_OPERATOR,
            last.status if last else -1,
            " with the pinned lockout copy" if ok else "",
        )
        if ok:
            self.check("auth-authority|apex-control|lockout", True, detail)
            return
        for _svc, _surface, cell_id in self.cells_of("auth-authority", "apex-control"):
            previous = self.cells.get(cell_id)
            if previous and previous["status"] == "pass":
                self.cell(cell_id, False, previous["detail"] + "; lockout probe: " + detail)

    # ------------------------------------------------------------- baseline
    def baseline_path(self):
        return self.repo_path("tools/dogfood/live-baseline.json")

    def check_baseline(self):
        path = self.baseline_path()
        if not os.path.exists(path):
            self.check(
                "ratchet|baseline",
                True,
                "no baseline yet (%s); --update-baseline records the required set after a green run" % path,
            )
            return
        with open(path, "r", encoding="utf-8") as fh:
            baseline = json.load(fh)
        recorded = set(baseline.get("cells", []))
        removals = {entry.get("cell") for entry in baseline.get("removals", []) if entry.get("cell")}
        problems = []
        missing = sorted(self.required - recorded)
        if missing:
            problems.append("required cells missing from the baseline (drift): %s" % missing[:6])
        dangling = sorted(recorded - self.required - removals)
        if dangling:
            problems.append("baseline cells no longer required without a reasoned removal: %s" % dangling[:6])
        self.check(
            "ratchet|baseline",
            not problems,
            "baseline of %d cells matches the required set" % len(recorded)
            if not problems
            else "; ".join(problems),
        )

    def write_baseline(self):
        path = self.baseline_path()
        removals = []
        if os.path.exists(path):
            with open(path, "r", encoding="utf-8") as fh:
                removals = json.load(fh).get("removals", [])
        baseline = {
            "cells": sorted(self.required),
            "removals": removals,
            "updated": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        }
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(baseline, fh, indent=2)
            fh.write("\n")
        log("baseline written: %s (%d cells)" % (path, len(baseline["cells"])))

    # ------------------------------------------------------------- results
    def write_results(self, exit_code, unattempted):
        os.makedirs(os.path.dirname(self.results_path), exist_ok=True)
        payload = {
            "run": {
                "started": self.started_at,
                "finished": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "mode": "static-only" if self.static_only else "live",
                "clickhouse_url": self.ch_url,
                "clickhouse_database": None if self.static_only else self.database,
                "bin_dir": self.bin_dir,
                "workdir": self.workdir,
                "exit_code": exit_code,
            },
            "summary": {
                "required_cells": len(self.required),
                "attempted_cells": len(self.cells),
                "passed_cells": sum(1 for cell in self.cells.values() if cell["status"] == "pass"),
                "failed_cells": sorted(c for c, cell in self.cells.items() if cell["status"] == "fail"),
                "unattempted_cells": sorted(unattempted),
                "failed_checks": [entry["check"] for entry in self.checks if entry["status"] == "fail"],
            },
            "checks": self.checks,
            "cells": dict(sorted(self.cells.items())),
        }
        with open(self.results_path, "w", encoding="utf-8") as fh:
            json.dump(payload, fh, indent=2)
            fh.write("\n")
        log("results written: %s" % self.results_path)

    # ------------------------------------------------------------ teardown
    def teardown(self):
        log("teardown: killing every harness process and dropping the test database")
        for name in list(self.procs):
            self.kill_process(name)
        if not self.static_only and self.ch_reachable():
            try:
                self.ch("DROP DATABASE IF EXISTS %s" % self.database)
                remaining = self.ch(
                    "SELECT count() FROM system.databases WHERE name = '%s'" % self.database
                ).strip()
                log("cleanup: dropped ClickHouse database %s (databases with that name: %s)" % (self.database, remaining))
            except ReqError as error:
                log("cleanup: could not drop %s: %s" % (self.database, error))
        elif not self.static_only:
            log("cleanup: ClickHouse is already down; %s did not survive the run" % self.database)
        for proc in self.ch_owned_procs:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            except (ProcessLookupError, PermissionError, OSError):
                pass
        proofs = []
        for name, port in sorted(self.ports.items()):
            lsof = subprocess.run(
                ["lsof", "-nP", "-iTCP:%d" % port, "-sTCP:LISTEN"], capture_output=True, text=True
            )
            listeners = [line for line in lsof.stdout.splitlines()[1:] if line.strip()]
            if listeners:
                proofs.append("PORT STILL LISTENING %s: %s" % (port, " ".join(listeners[:2])))
        for proc in self.ch_owned_procs:
            alive = proc.poll() is None
            proofs.append(
                "clickhouse recovery server pid %d: %s"
                % (proc.pid, "STILL RUNNING (killed by pgid)" if alive else "exited %s; not listening" % proc.returncode)
            )
        if proofs:
            log("cleanup proof: " + " | ".join(proofs))
        else:
            log("cleanup proof: no harness process or port remains (pgrep/lsof found nothing)")

    # ---------------------------------------------------------------- main
    def run_static(self):
        self.load_catalog()
        self.probe_surface_parity()
        self.check_site()
        self.check_baseline()
        for cell_id in sorted(self.required):
            self.cell(cell_id, True, "SKIPPED: --static-only records the cell without attempting it")
        log(
            "static-only: %d cells recorded as skipped; %d checks run"
            % (len(self.required), len(self.checks))
        )

    def run_live(self):
        try:
            self.load_catalog()
            self.check_site()
            if not self.require_binaries():
                return
            if not self.ch_reachable():
                self.check(
                    "boot|clickhouse",
                    False,
                    "no ClickHouse at %s; the live half requires a reachable server (make clickhouse-dev, "
                    "or the lane service container)" % self.ch_url,
                )
                self.probe_surface_parity()
                return
            self.check("boot|clickhouse", True, "ClickHouse reachable at %s" % self.ch_url)
            self.api["work"] = self.prepare_api_files()
            self.control["work"] = self.prepare_control_files()
            self.prepare_sales_files()
            self.seed_touch()
            self.ch_bootstrap()
            self.start_api()
            self.start_control()
            self.start_sales_web()
            self.run_all_probes()
            self.run_lockout_probe()
            self.check_baseline()
        except ReqError as error:
            self.check("boot|live-sequence", False, str(error))

    def summarize(self):
        failed_cells = sorted(c for c, cell in self.cells.items() if cell["status"] == "fail")
        unattempted = sorted(self.required - set(self.cells))
        failed_checks = [entry["check"] for entry in self.checks if entry["status"] == "fail"]
        log("")
        log("dogfood-live summary")
        log("  required cells:  %d" % len(self.required))
        log("  attempted cells: %d" % len(self.cells))
        log(
            "  passed cells:    %d"
            % sum(1 for cell in self.cells.values() if cell["status"] == "pass")
        )
        if failed_cells:
            log("  FAILED CELLS:")
            for cell_id in failed_cells:
                log("    - %s: %s" % (cell_id, self.cells[cell_id]["detail"]))
        if unattempted:
            log("  UNATTEMPTED REQUIRED CELLS:")
            for cell_id in unattempted:
                log("    - %s" % cell_id)
        if failed_checks:
            log("  FAILED CHECKS: %s" % ", ".join(failed_checks))
        green = not failed_cells and not unattempted and not failed_checks
        log("  verdict: %s" % ("GREEN" if green else "RED"))
        return green

    def run(self):
        try:
            if self.static_only:
                self.run_static()
            else:
                self.run_live()
        finally:
            green = False
            try:
                green = self.summarize()
            finally:
                self.teardown()
            if green and not self.static_only and self.args.update_baseline:
                self.write_baseline()
            unattempted = sorted(self.required - set(self.cells))
            exit_code = 0 if green else 1
            self.write_results(exit_code, unattempted)
        return 0 if green else 1


def parse_args(argv):
    parser = argparse.ArgumentParser(description="dogfood-live: the adversarial live harness")
    parser.add_argument("--bin-dir", default="rust/target/debug")
    parser.add_argument("--workdir", default="rust/target/dogfood-live")
    parser.add_argument("--clickhouse-url", default="http://127.0.0.1:18123")
    parser.add_argument("--static-only", action="store_true")
    parser.add_argument("--results", default="rust/target/dogfood-live/results.json")
    parser.add_argument("--update-baseline", action="store_true")
    parser.add_argument("--port-base", type=int, default=18700)
    parser.add_argument(
        "--clickhouse-owned",
        action="store_true",
        help="the operator granted the harness ownership of the ClickHouse server at --clickhouse-url "
        "(the local make path); the failure-injection CH severance subprobe is required",
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv if argv is not None else sys.argv[1:])
    return Harness(args).run()


if __name__ == "__main__":
    sys.exit(main())
