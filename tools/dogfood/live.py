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
  (`path` + `methods`, plus `"session_required": true` on the publisher-console
  GETs that refuse anonymous reads), `fixture_pages` and `required` cells; a
  cell expands to `"<service>|<METHOD path>|<class>"`;
- every route declared `session_required` must answer the recorded gate
  without a session (exactly `303 -> /console/ledger` on a deployment with
  tokens configured) and must render under a live session; the documented
  public console exceptions stay public and are asserted as such;
- every required cell must be attempted in a live run and every attempted
  cell must pass; any failure or unattempted cell exits 1 with the cell list;
- `--static-only` runs only the surface-parity and catalog checks (no
  processes, no ClickHouse) and records every cell as skipped;
- the ratchet (`tools/dogfood/live-baseline.json`, the same file the static
  gate reads): a required cell missing from the recorded baseline fails the
  run (drift), and a baseline cell that is no longer required fails unless a
  `removals` entry with a reason releases it; `--update-baseline` records pure
  additions during the green recording run and the ratchet is strict again on
  every run that does not carry the flag;
- teardown always kills every process the harness started, drops the
  ClickHouse test database and prints the cleanup proof.

Only the Python standard library is used; the vaulted credential bundle and
the tamper-evident ledger lines the harness seeds are written by the
pure-stdlib AES-256-GCM and ed25519 implementations at the top of this file,
pinned against the published NIST/RFC vectors at boot.
"""

import argparse
import base64
import csv
import hashlib
import hmac
import http.client
import http.server
import io
import json
import os
import re
import shutil
import socket
import signal
import subprocess
import sys
import itertools
import threading
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
    "tenant-isolation",
    "auth-surface",
    "signin-ownership",
    "payment-activation",
]

# The dispatch order is not the registry order: `failure-injection` restarts
# the owned ClickHouse server on a fresh data path, so every
# ClickHouse-dependent class (`differential-truth`, which asserts the seeded
# projections and the runner's billing truth) must run before it. The frozen
# class registry above stays unchanged; only the execution schedule moves.
PROBE_ORDER = list(FROZEN_PROBE_CLASSES)
PROBE_ORDER.remove("differential-truth")
PROBE_ORDER.insert(PROBE_ORDER.index("failure-injection"), "differential-truth")

# The always-run core set the relevance filter adds to every selection. Must
# stay in lockstep with `CORE_CELLS` in
# rust/crates/apex-tools/src/dogfood_plan.rs: the published `/healthz`
# contract (which also runs the static parity gate), the console session gate
# and the two-publisher tenant isolation of the overview and the ledger.
CORE_CELLS = [
    "apex-api|GET /healthz|surface-parity",
    "apex-api|GET /healthz|response-contract",
    "apex-api|GET /console/audit|auth-surface",
    "apex-api|GET /|tenant-isolation",
    "apex-api|GET /console/ledger|tenant-isolation",
]

SERVICES = ["apex-api", "apex-control", "apex-sales-web"]

# Product facts the harness injects (the catalog declares only the contract).
API_LEDGER_TOKEN = "dogfood-live-ledger-token"
# The primary apex-api's absolute public origins: the console link base is the
# instance's own loopback origin (loopback http is a valid public URL), and the
# Help destination is a distinct absolute URL so a relative fallback is
# observable on every console page.
API_SUPPORT_URL = "https://support.dogfood.example/help/"
# The recorded contact of the links-disabled instance's existing workspace.
# The instance keeps the full lead/workspace/email wiring and removes only
# APEX_CONSOLE_URL, so a successful free audit proves links alone are off.
LINKS_OFF_CONTACT = "links-off@dogfood.example"
# The workspace key the free-audit form derives from that address's proven
# domain. It must already exist with the contact recorded, so the disabled
# link path is reached instead of a domain-proof refusal.
LINKS_OFF_WORKSPACE = "dogfood-example"
# An emailed link: the absolute base, the sign-in path and a 32-hex token.
SIGNIN_LINK_PATTERN = re.compile(
    r"https?://[^\s<>\"']+/console/signin\?token=([0-9a-f]{32})(?![0-9a-f])"
)
API_LIVE_AUDIT_TOKEN = "dogfood-live-audit-token"
API_OPERATOR_JOBS_TOKEN = "dogfood-live-operator-jobs-token"
API_VAULT_KEY_HEX = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
SALES_WEB_TOKEN = "dogfood-live-owner-token"
CONTROL_OPERATOR_PASSWORD = "dogfood-operator-passphrase"
CONTROL_LOCKED_OPERATOR = "locked-probe"
# The setup-watch sign-up lane: one domain/email pair the harness submits
# through the public form and bills through the product's own billing run.
WATCH_DOMAIN = "dogfood-watch.example"
WATCH_PUBLISHER = "dogfood-watch-example"
WATCH_EMAIL = "watch@dogfood-watch.example"
# The second publisher the tenant-isolation probe seeds: a distinct snapshot,
# a distinct ledger and its own bearer token, so a leak in either direction is
# observable in the bytes.
RIVAL_PUBLISHER = "rival"
RIVAL_TOKEN = "dogfood-live-rival-token"
RIVAL_LEDGER_RECORDS = 5
# A third publisher whose ledger is the pristine 24-record fixture. The shared
# northstar ledger receives the harness's own kept-change and runner records
# before the probes run, so the pagination contract is asserted against a
# ledger that is still exactly the committed fixture (24 records, one page).
PAGINATION_PUBLISHER = "fixture24"
PAGINATION_TOKEN = "dogfood-live-pagination-token"
PAGINATION_LEDGER_RECORDS = 24
# The publisher the console's own session is minted for (the token file's
# primary entry).
API_PUBLISHER = "northstar"
RIVAL_DOMAIN = "rival-only.example"
RIVAL_UNIT = "rival-only-unit"
NORTHSTAR_UNIT = "northstar-only-unit"
# Markers that appear on the rendered audit page as the missing-network
# finding's message: the northstar (hostile) snapshot is missing mintegral.com,
# the rival snapshot is missing rival-missing.example. The audit page is what
# the tenant-isolation probe compares, so the markers must be finding text,
# not snapshot fields (ad-unit names are only rendered when a rule cites them).
NORTHSTAR_AUDIT_MARKER = "mintegral.com"
RIVAL_AUDIT_MARKER = "rival-missing.example"
# The marker the DNS-rebinding probe's stub curl emits when the fetch did NOT
# pin the guard-approved address: the re-resolution landed elsewhere. The
# fixed fetch passes `--resolve host:port:address`, so the marker never
# reaches a recorded finding.
REBIND_CANARY = "REBOUND-TO-LOOPBACK"
# The Adyen webhook reference the payment-activation probe settles.
WATCH_SETTLEMENT_REFERENCE = "dogfood-watch-settlement"
# The Adyen HMAC key (hex) the harness signs the settlement notification with
# and hands the API through APEX_ADYEN_HMAC_KEY.
API_ADYEN_HMAC_KEY_HEX = "646f67666f6f642d616479656e2d686d61632d6b6579"
# The loopback Adyen checkout stub the API and the watch bill charge through:
# a successful payment link and an accepted card-on-file charge. The public
# developer site is a real HTTPS domain whose `/app-ads.txt` is a 404, so the
# live audit's app-ads read is an honest "absent" without reaching a private
# address (the guard refuses loopback names).
API_ADYEN_MERCHANT_ACCOUNT = "DogfoodECOM"
API_ADYEN_CHECKOUT_LINK = "https://checkout.dogfood.example/pay/DOGFOOD-LINK"
API_LIVE_DEVELOPER_SITE = "digitalturbine.com"
# The vaulted LevelPlay key set the tenant/LevelPlay probes onboard: the base
# URL is repointed at the harness's loopback stub so the reporting request is
# recorded.
LEVELPLAY_SECRET_KEY = "dogfood-levelplay-secret"
LEVELPLAY_REFRESH_TOKEN = "dogfood-levelplay-refresh"
LEVELPLAY_APP_KEY = "dogfood-levelplay-app"
LEVELPLAY_ACCOUNT_ID = "dogfood-levelplay-account"
# The cross-border EU customer the VIES probe bills: Estonia, so a Finnish
# seller is out of country and a VAT number is required for reverse charge.
VIES_CUSTOMER_VAT = "EE123456789"
VIES_CUSTOMER_COUNTRY = "EE"
# The label the fixed VAT treatment records as its VIES evidence. The probe
# avoids the substring in its own publisher/customer names (a bare "vies"
# marker would match "vies-probe" and mask the defect), so the markers are the
# serialized evidence *key* and the confirmation word.
VIES_EVIDENCE_MARKERS = ('"vies', "confirmation")
VIES_PROBE_DOMAIN = "cross-border-probe.example"
VIES_PROBE_PUBLISHER = "cross-border-probe"
# The seeded monitoring band: USD 350,000 of reported 30-day revenue, inside
# the Mid band ($100k..$500k) so the chosen tier ($1,500/month) is provably
# band-selected and not a default.
MONITORING_BAND_REVENUE_30D_MICROS = 350_000 * 1_000_000
MONITORING_MID_FEE_MICROS = 1_500 * 1_000_000
MONITORING_TOP_FEE_MICROS = 3_000 * 1_000_000
MONITORING_BASE_FEE_MICROS = 500 * 1_000_000
MONITORING_MID_FROM_REVENUE_MICROS = 100_000 * 1_000_000
MONITORING_TOP_ABOVE_REVENUE_MICROS = 500_000 * 1_000_000
# The three platform revenue seam the fleet audit must fill and the tier each
# seeded 30-day revenue selects. MAX's total is the sum of its country rows;
# LevelPlay's is the app-level reporting call; AdMob's is its mediation report.
MONITORING_PLATFORM_TRUTH = {
    "applovin_max": {
        "revenue_30d": 120_000 * 1_000_000,
        "tier": "Mid",
        "fee_micros": MONITORING_MID_FEE_MICROS,
    },
    "levelplay": {
        "revenue_30d": 600_000 * 1_000_000,
        "tier": "Top",
        "fee_micros": MONITORING_TOP_FEE_MICROS,
    },
    "admob": {
        "revenue_30d": 100_000 * 1_000_000,
        "tier": "Mid",
        "fee_micros": MONITORING_MID_FEE_MICROS,
    },
}
# The proven-domain workspace key the corrected free-audit flow must derive
# from the email domain, with the studio name kept only as the display label.
WORKSPACE_PROVEN_DOMAIN = "digitalturbine.com"
WORKSPACE_DOMAIN_SLUG = "digitalturbine-com"


def workspace_key_matches(key, slug):
    """A self-serve workspace key is the legacy slug or its hashed form
    (`<slug>-<8 hex of SHA-256(domain)>`), which is collision-resistant."""
    return key == slug or (key.startswith(slug + "-") and len(key) == len(slug) + 9)
# Setup Watch's evidence tier: at least ten distinct apps the subscriber names.
WATCH_APPS_CORE_MIN = 10
WATCH_APPS_MAX = 50
# The typed reason the monthly pass records when monitoring is on but no
# connections root is configured (the legacy-mode skip the defect cell pins).
MONITORING_ROOT_UNCONFIGURED_REASON = "monitoring state root not configured"
# The seeded kept AdMob change: the experiment name and the monthly fee the
# fee-eligibility gate must refuse to ever charge.
ADMOB_KEPT_EXPERIMENT = "exp_dogfood_admob"
SELLER_VAT_RATE_BP = 2000
WATCH_FEE_MICROS = 99 * 1_000_000
# The RFC 4648 base32 form of the 20-byte all-zero-then-0x01 secret. The
# brief's literal is one character short of a whole byte count, which the
# product decoder correctly refuses (trailing bits are non-zero), so the
# harness uses the byte-aligned spelling of the same secret.
CONTROL_TOTP_SECRET_B32 = "A" * 31 + "B"
# ed25519 public key of the [7u8; 32] test seed the committed fixture ledger
# (rust/crates/apex-api/fixtures/northstar.jsonl) is signed with.
NORTHSTAR_PUBKEY_HEX = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"
RUNNER_SIGNING_SEED_HEX = "07" * 32
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
    "{job_id}": "opj_000000000000",
}

# Patterns used inside the per-cell probe loops are compiled once here.
CSRF_FIELD_PATTERNS = (
    re.compile(r'name="csrf_token"\s+value="([^"]+)"'),
    re.compile(r'value="([^"]+)"\s+name="csrf_token"'),
)
INPUT_TAG_PATTERN = re.compile(r"<input\b[^>]*>")
INPUT_ATTR_PATTERN = re.compile(r'([A-Za-z_:][-\w:.]*)="([^"]*)"')
PENDING_INPUT_PATTERN = re.compile(r'name="pending"\s+value="([^"]+)"')
TRAVERSAL_TOKEN_PATTERN = re.compile(r"\{[^}]+\}")
H1_TAG_PATTERN = re.compile(r"<h1[\s>]")
STYLE_TAG_PATTERN = re.compile(r"<style[\s>]")
STYLE_BLOCK_PATTERN = re.compile(r"<style>(.*?)</style>", re.S)
FINDING_LINK_PATTERN = re.compile(r"/console/audit/findings/([A-Za-z0-9_.-]+)")
REVOKE_FORM_PATTERN = re.compile(
    r'<form[^>]*action="/control/actions/session_revoke"[^>]*>(.*?)</form>', re.S
)
CONFIRM_PHRASE_PATTERN = re.compile(r"Type exactly: ([A-Z ]+)")


def log(message):
    print(message, flush=True)


def source_matches(changed, source):
    """Whether a changed repo path matches one declared surface source. A
    source naming a directory (trailing slash) matches it and anything under
    it; otherwise the match is exact. Mirrors `source_matches` in
    rust/crates/apex-tools/src/dogfood_plan.rs so the two relevance filters
    agree byte-for-byte."""
    if source.endswith("/"):
        return changed == source[:-1] or changed.startswith(source)
    return changed == source


def select_relevant_cells(required, surface_sources, changed, core=CORE_CELLS):
    """The pure relevance selection, mirroring `select_cells` in
    dogfood_plan.rs: with no changed paths return every cell; otherwise select
    the cells whose surface `sources` intersect a changed path, add the
    always-run core, and report the classes with no selected cell and the
    changed paths that matched no declared source. Deterministic."""
    all_cells = set(required)
    if not changed:
        return all_cells, set(), []
    selected = set()
    for cell_id in sorted(all_cells):
        service, surface, _cls = cell_id.split("|", 2)
        path = surface.split(" ", 1)[1] if " " in surface else surface
        sources = surface_sources.get("%s|%s" % (service, path), [])
        if any(source_matches(change, source) for change in changed for source in sources):
            selected.add(cell_id)
    unmatched = [
        change
        for change in changed
        if not any(
            source_matches(change, source)
            for sources in surface_sources.values()
            for source in sources
        )
    ]
    for cell in core:
        if cell in all_cells:
            selected.add(cell)
    required_classes = set(cell.split("|", 2)[2] for cell in all_cells)
    selected_classes = set(cell.split("|", 2)[2] for cell in selected)
    return selected, required_classes - selected_classes, unmatched


def relevance_self_test():
    """The four documented relevance cases, against a miniature catalog. Kept
    beside the frozen core so the always-run set stays honest."""
    required = [
        "apex-api|GET /console/audit|response-contract",
        "apex-api|GET /console/audit|tenant-isolation",
        "apex-api|GET /console/audit|auth-surface",
        "apex-api|GET /|tenant-isolation",
        "apex-api|GET /console/ledger|tenant-isolation",
        "apex-api|GET /healthz|surface-parity",
        "apex-api|GET /healthz|response-contract",
        "apex-api|POST /watch|failure-injection",
        "apex-sales-web|GET /leads|response-contract",
    ]
    sources = {
        "apex-api|/console/audit": [
            "rust/crates/apex-api/src/routes.rs",
            "rust/crates/apex-api/src/html/",
        ],
        "apex-api|/": ["rust/crates/apex-api/src/html/overview.rs"],
        "apex-api|/console/ledger": ["rust/crates/apex-api/src/html/ledger.rs"],
        "apex-api|/healthz": ["rust/crates/apex-api/src/routes.rs"],
        "apex-api|/watch": [
            "rust/crates/apex-watch/src/fetch.rs",
            "rust/crates/apex-watch/src/guard.rs",
        ],
        "apex-sales-web|/leads": ["rust/crates/apex-sales-web/src/routes.rs"],
    }

    def cell_present(selected, needle):
        return any(needle == cell for cell in selected)

    problems = []
    # 1. A changed console page selects the console + tenant cells, skips sales.
    selected, skipped, unmatched = select_relevant_cells(
        required, sources, ["rust/crates/apex-api/src/html/audit.rs"]
    )
    if not cell_present(selected, "apex-api|GET /console/audit|response-contract"):
        problems.append("console change did not select the console response-contract cell")
    if not cell_present(selected, "apex-api|GET /console/audit|tenant-isolation"):
        problems.append("console change did not select the tenant cell")
    if not cell_present(selected, "apex-api|GET /|tenant-isolation"):
        problems.append("console change did not add the core tenant cell")
    if cell_present(selected, "apex-sales-web|GET /leads|response-contract"):
        problems.append("console change selected a sales cell")
    if "failure-injection" not in skipped:
        problems.append("console change did not skip the fetch class")
    if unmatched:
        problems.append("console change reported an unmatched path: %s" % unmatched)
    # 2. A changed watch fetch selects the fetch/SSRF cell.
    selected, _skipped, _unmatched = select_relevant_cells(
        required, sources, ["rust/crates/apex-watch/src/fetch.rs"]
    )
    if not cell_present(selected, "apex-api|POST /watch|failure-injection"):
        problems.append("fetch change did not select the SSRF cell")
    # 3. No paths selects everything, nothing skipped.
    selected, skipped, unmatched = select_relevant_cells(required, sources, [])
    if selected != set(required) or skipped or unmatched:
        problems.append("no paths did not select everything")
    # 4. An unknown path selects the core set only and reports the path.
    selected, _skipped, unmatched = select_relevant_cells(
        required, sources, ["docs/nowhere.md"]
    )
    core = set(cell for cell in CORE_CELLS if cell in required)
    if selected != core:
        problems.append("unknown path did not select the core only: %s" % sorted(selected))
    if unmatched != ["docs/nowhere.md"]:
        problems.append("unknown path not reported: %s" % unmatched)
    return problems


def adyen_sign(hmac_key_hex, item):
    """The base64 HMAC-SHA256 Adyen puts in `additionalData.hmacSignature`:
    the eight documented fields colon-joined, in order (mirrors
    apex_billing::adyen::NotificationItem::signed_message)."""
    key = bytes.fromhex(hmac_key_hex)

    def escape(value):
        # Adyen escapes backslashes then colons inside every field before
        # joining, so a merchant reference containing ':' signs correctly.
        return str(value).replace("\\", "\\\\").replace(":", "\\:")

    message = ":".join(
        escape(field)
        for field in [
            item["pspReference"],
            item.get("originalReference", ""),
            item["merchantAccountCode"],
            item["merchantReference"],
            str(item["amount"]["value"]),
            item["amount"]["currency"],
            item["eventCode"],
            item["success"],
        ]
    )
    return base64.b64encode(hmac.new(key, message.encode("utf-8"), hashlib.sha256).digest()).decode()


class _StubHandler(http.server.BaseHTTPRequestHandler):
    """One request against a `LocalStub`, recorded then answered."""

    def log_message(self, *args):  # keep the harness output clean
        return

    def _serve(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        self.server.record(self.command, self.path, dict(self.headers), body)
        status, payload = self.server.answer(self.command, self.path)
        data = payload if isinstance(payload, (bytes, bytearray)) else json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_GET = _serve
    do_POST = _serve


class LocalStub:
    """A loopback HTTP stub the product talks to through a configured base
    URL. Every request is recorded so a probe can assert the exact request the
    product issued (for example the LevelPlay reporting query string)."""

    def __init__(self, responder):
        self.responder = responder
        self.requests = []
        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _StubHandler)
        self.httpd.record = self._record
        self.httpd.answer = self._answer
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def _record(self, method, path, headers, body):
        self.requests.append(
            {
                "method": method,
                "path": path,
                "headers": headers,
                "body": body.decode("utf-8", "replace"),
            }
        )

    def _answer(self, method, path):
        return self.responder(method, path, self.requests)

    def start(self):
        self.thread.start()
        return self

    @property
    def port(self):
        return self.httpd.server_address[1]

    def base_url(self):
        return "http://127.0.0.1:%d" % self.port

    def stop(self):
        try:
            self.httpd.shutdown()
        except Exception:
            pass
        try:
            self.httpd.server_close()
        except Exception:
            pass


def materialize(template):
    path = template
    for token, value in PATH_PARAMETERS.items():        path = path.replace(token, value, 1)
    return path


def extract_csrf(html):
    for pattern in CSRF_FIELD_PATTERNS:
        match = pattern.search(html)
        if match:
            return match.group(1)
    return None


def parse_hidden_fields(fragment):
    """Every <input name=... value=...>, attribute order independent."""
    fields = {}
    for tag in INPUT_TAG_PATTERN.findall(fragment):
        attrs = dict(INPUT_ATTR_PATTERN.findall(tag))
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


# ---------------------------------------------------------------------------
# Pure-stdlib crypto for the harness's own provisioning seams.
#
# Two product formats are written by the harness, both owned by the deployment
# side: the vaulted credential bundle (AES-256-GCM, `nonce-hex.ct-hex`, see
# `apex-vault`) and the tamper-evident ledger line (`{"record":..,
# "signature":hex,"pubkey":hex}`, ed25519 over the record hash, see
# `apex-proof`). Python's standard library ships neither cipher, so both live
# here, pinned by the product's own published vectors (NIST AES-GCM, RFC 8032
# ed25519 test 1) in `crypto_self_check`, which the harness runs at boot.
# ---------------------------------------------------------------------------

_SBOX = bytes.fromhex(
    "637c777bf26b6fc53001672bfed7ab76ca82c97dfa5947f0add4a2af9ca472c0"
    "b7fd9326363ff7cc34a5e5f171d8311504c723c31896059a071280e2eb27b275"
    "09832c1a1b6e5aa0523bd6b329e32f8453d100ed20fcb15b6acbbe394a4c58cf"
    "d0efaafb434d338545f9027f503c9fa851a3408f929d38f5bcb6da2110fff3d2"
    "cd0c13ec5f974417c4a77e3d645d197360814fdc222a908846eeb814de5e0bdb"
    "e0323a0a4906245cc2d3ac629195e479e7c8376d8dd54ea96c56f4ea657aae08"
    "ba78252e1ca6b4c6e8dd741f4bbd8b8a703eb5664803f60e613557b986c11d9e"
    "e1f8981169d98e949b1e87e9ce5528df8ca1890dbfe6426841992d0fb054bb16"
)


def _aes_expand_key(key):
    nk = len(key) // 4
    nr = nk + 6
    words = [list(key[4 * i : 4 * i + 4]) for i in range(nk)]
    rcon = 1
    for i in range(nk, 4 * (nr + 1)):
        temp = list(words[i - 1])
        if i % nk == 0:
            temp = temp[1:] + temp[:1]
            temp = [_SBOX[b] for b in temp]
            temp[0] ^= rcon
            rcon = ((rcon << 1) ^ (0x11B if rcon & 0x80 else 0)) & 0xFF
        elif nk > 6 and i % nk == 4:
            temp = [_SBOX[b] for b in temp]
        words.append([words[i - nk][j] ^ temp[j] for j in range(4)])
    return [sum(words[4 * r : 4 * r + 4], []) for r in range(nr + 1)], nr


def _aes_encrypt_block(round_keys, block):
    state = [block[i] ^ round_keys[0][i] for i in range(16)]

    def xtime(a):
        return ((a << 1) ^ 0x1B) & 0xFF if a & 0x80 else a << 1

    for rnd in range(1, len(round_keys)):
        state = [_SBOX[b] for b in state]
        # ShiftRows (column-major state index = 4*col + row).
        shifted = state[:]
        for col in range(4):
            for row in range(4):
                shifted[4 * col + row] = state[4 * ((col + row) % 4) + row]
        state = shifted
        if rnd != len(round_keys) - 1:
            mixed = []
            for col in range(4):
                a = state[4 * col : 4 * col + 4]
                t = a[0] ^ a[1] ^ a[2] ^ a[3]
                mixed.extend(
                    [
                        a[i] ^ t ^ xtime(a[i] ^ a[(i + 1) % 4])
                        for i in range(4)
                    ]
                )
            state = mixed
        state = [state[i] ^ round_keys[rnd][i] for i in range(16)]
    return bytes(state)


def _gf_mul(x, y):
    result = 0
    for bit in range(128):
        if (y >> (127 - bit)) & 1:
            result ^= x
        if x & 1:
            x = (x >> 1) ^ 0xE1000000000000000000000000000000
        else:
            x >>= 1
    return result


def _ghash(h, data):
    y = 0
    for offset in range(0, len(data), 16):
        block = data[offset : offset + 16].ljust(16, b"\x00")
        y = _gf_mul(y ^ int.from_bytes(block, "big"), h)
    return y


def aes_gcm_encrypt(key, nonce, plaintext, aad=b""):
    """AES-GCM (12-byte nonce) -> (ciphertext, 16-byte tag)."""
    round_keys, _nr = _aes_expand_key(key)
    h = int.from_bytes(_aes_encrypt_block(round_keys, b"\x00" * 16), "big")
    counter = int.from_bytes(nonce + b"\x00\x00\x00\x01", "big")
    ciphertext = bytearray()
    for offset in range(0, len(plaintext), 16):
        counter = (counter + 1) & ((1 << 128) - 1)
        keystream = _aes_encrypt_block(round_keys, counter.to_bytes(16, "big"))
        chunk = plaintext[offset : offset + 16]
        ciphertext.extend(a ^ b for a, b in zip(chunk, keystream))
    lengths = (len(aad) * 8).to_bytes(8, "big") + (len(ciphertext) * 8).to_bytes(8, "big")
    padded = aad + b"\x00" * (-len(aad) % 16)
    padded += bytes(ciphertext) + b"\x00" * (-len(ciphertext) % 16)
    tag = _ghash(h, padded + lengths) ^ int.from_bytes(
        _aes_encrypt_block(round_keys, nonce + b"\x00\x00\x00\x01"), "big"
    )
    return bytes(ciphertext), tag.to_bytes(16, "big")


_ED_P = 2**255 - 19
_ED_L = 2**252 + 27742317777372353535851937790883648493
_ED_D = (-121665 * pow(121666, _ED_P - 2, _ED_P)) % _ED_P
_ED_I = pow(2, (_ED_P - 1) // 4, _ED_P)


def _ed_xrecover(y):
    xx = (y * y - 1) * pow(_ED_D * y * y + 1, _ED_P - 2, _ED_P)
    x = pow(xx, (_ED_P + 3) // 8, _ED_P)
    if (x * x - xx) % _ED_P != 0:
        x = (x * _ED_I) % _ED_P
    if x % 2 != 0:
        x = _ED_P - x
    return x


_ED_BY = 4 * pow(5, _ED_P - 2, _ED_P) % _ED_P
_ED_BX = _ed_xrecover(_ED_BY)
_ED_B = (_ED_BX, _ED_BY, 1, (_ED_BX * _ED_BY) % _ED_P)


def _ed_add(p, q):
    x1, y1, z1, t1 = p
    x2, y2, z2, t2 = q
    a = (y1 - x1) * (y2 - x2) % _ED_P
    b = (y1 + x1) * (y2 + x2) % _ED_P
    c = 2 * t1 * t2 * _ED_D % _ED_P
    d = z1 * 2 * z2 % _ED_P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % _ED_P, g * h % _ED_P, f * g % _ED_P, e * h % _ED_P)


def _ed_scalarmult(point, scalar):
    result = (0, 1, 1, 0)
    addend = point
    while scalar:
        if scalar & 1:
            result = _ed_add(result, addend)
        addend = _ed_add(addend, addend)
        scalar >>= 1
    return result


def _ed_encode(point):
    x, y, z, _t = point
    zi = pow(z, _ED_P - 2, _ED_P)
    x = x * zi % _ED_P
    y = y * zi % _ED_P
    encoded = y | ((x & 1) << 255)
    return encoded.to_bytes(32, "little")


def _ed_clamp(seed):
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, h[32:]


def ed25519_public_key(seed):
    a, _prefix = _ed_clamp(seed)
    return _ed_encode(_ed_scalarmult(_ED_B, a))


def ed25519_sign(seed, message):
    a, prefix = _ed_clamp(seed)
    public = _ed_encode(_ed_scalarmult(_ED_B, a))
    r = int.from_bytes(hashlib.sha512(prefix + message).digest(), "little") % _ED_L
    big_r = _ed_encode(_ed_scalarmult(_ED_B, r))
    k = int.from_bytes(hashlib.sha512(big_r + public + message).digest(), "little") % _ED_L
    s = (r + k * a) % _ED_L
    return big_r + s.to_bytes(32, "little")


def vault_encrypt(key_hex, plaintext):
    """`<nonce hex>.<ciphertext hex>` exactly as apex-vault writes it."""
    key = bytes.fromhex(key_hex)
    nonce = os.urandom(12)
    ciphertext, tag = aes_gcm_encrypt(key, nonce, plaintext.encode("utf-8"))
    return nonce.hex() + "." + (ciphertext + tag).hex()


def crypto_self_check():
    """Pin both primitives against their published vectors and raise on any
    mismatch, so a broken cipher can never silently corrupt the boot."""
    key128 = bytes(16)
    ciphertext, tag = aes_gcm_encrypt(key128, bytes(12), bytes(16))
    assert ciphertext.hex() == "0388dace60b6a392f328c2b971b2fe78", ciphertext.hex()
    assert tag.hex() == "ab6e47d42cec13bdf53a67b21257bddf", tag.hex()
    ciphertext, tag = aes_gcm_encrypt(bytes(32), bytes(12), bytes(16))
    assert ciphertext.hex() == "cea7403d4d606b6e074ec5d3baf39d18", ciphertext.hex()
    assert tag.hex() == "d0d1c8a799996bf0265b98b5d48ab919", tag.hex()
    seed = bytes.fromhex(
        "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"
    )
    assert ed25519_public_key(seed).hex() == (
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
    )
    assert ed25519_sign(seed, b"").hex() == (
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb88215"
        "90a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
    )
    # The runner's deployment seed must produce the fixture ledger's public key
    # (the same key the api verifies the sealed chain with).
    assert ed25519_public_key(bytes.fromhex(RUNNER_SIGNING_SEED_HEX)).hex() == (
        NORTHSTAR_PUBKEY_HEX
    )


def canonical_json(value):
    """Compact JSON with recursively sorted keys: apex-proof's hash input."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def ledger_hashed_record(record):
    """The 32-byte record hash: sha256 of the canonical record without `hash`."""
    body = {key: value for key, value in record.items() if key != "hash"}
    return hashlib.sha256(canonical_json(body).encode("utf-8")).digest()


def ledger_line(record, seed):
    """One byte-exact persisted line: `{"record":..,"signature":hex,
    "pubkey":hex}` with the signature over the record hash (apex-proof)."""
    signature = ed25519_sign(seed, ledger_hashed_record(record))
    line = {"record": record, "signature": signature.hex(), "pubkey": ed25519_public_key(seed).hex()}
    return json.dumps(line, separators=(",", ":"), ensure_ascii=False)


def seal_ledger_record(record, seq, prev_hash):
    """Rebuild the record in serde's declaration order, fill the envelope,
    compute the chain hash and return the finished record."""
    ordered = {"type": record["type"], "seq": seq}
    for key, value in record.items():
        if key not in ("type", "seq", "prevHash", "hash"):
            ordered[key] = value
    ordered["prevHash"] = prev_hash
    ordered["hash"] = "0" * 64
    ordered["hash"] = ledger_hashed_record(ordered).hex()
    return ordered


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


# Per-run HTTP counters. `requests` and `connects` make the keep-alive effect
# measurable without timing: before the pool every request opened a socket,
# so connects == requests; the pool reuses one connection per service and
# thread for the idempotent methods.
_HTTP_STATS = {"requests": 0, "connects": 0, "reused": 0, "retries": 0, "drops": 0}
_HTTP_STATS_LOCK = threading.Lock()
# Every pooled connection, so a server restart can close stale sockets from
# the harness thread. Thread-local dictionaries keep one connection per
# (host, port, timeout) without sharing a connection across threads.
_HTTP_LOCAL = threading.local()
_HTTP_LIVE = {}

# POSTs are never replayed: a connection failure after the request left the
# socket could duplicate a sign-up, a webhook or a job. Only the idempotent
# methods use the pool and the one retry.
_HTTP_IDEMPOTENT = frozenset(("GET", "HEAD", "OPTIONS"))


def http_stats():
    with _HTTP_STATS_LOCK:
        return dict(_HTTP_STATS)


def reset_http_stats():
    with _HTTP_STATS_LOCK:
        for name in _HTTP_STATS:
            _HTTP_STATS[name] = 0


def _http_count(name, delta=1):
    with _HTTP_STATS_LOCK:
        _HTTP_STATS[name] += delta


def _drop_pooled_connection(key):
    conns = getattr(_HTTP_LOCAL, "conns", None)
    if conns is not None:
        conn = conns.pop(key, None)
        if conn is not None:
            try:
                conn.close()
            except OSError:
                pass
            with _HTTP_STATS_LOCK:
                _HTTP_LIVE.pop(id(conn), None)


def drop_http_pool():
    """Close every pooled connection (a service restarted, so its sockets are
    stale). Thread-local dictionaries are cleared for the calling thread; a
    worker's stale entry fails once and is retried for the idempotent methods."""
    with _HTTP_STATS_LOCK:
        for conn in list(_HTTP_LIVE.values()):
            try:
                conn.close()
            except OSError:
                pass
        _HTTP_LIVE.clear()
        _HTTP_STATS["drops"] += 1
    _HTTP_LOCAL.conns = {}


def _pooled_connection(host, port, timeout):
    key = (host, port, timeout)
    conns = getattr(_HTTP_LOCAL, "conns", None)
    if conns is None:
        conns = {}
        _HTTP_LOCAL.conns = conns
    conn = conns.get(key)
    if conn is not None and conn.sock is not None:
        _http_count("reused")
        return key, conn
    if conn is not None:
        _drop_pooled_connection(key)
    conn = http.client.HTTPConnection(host, port, timeout=timeout)
    conns[key] = conn
    with _HTTP_STATS_LOCK:
        _HTTP_LIVE[id(conn)] = conn
    _http_count("connects")
    return key, conn


_HTTP_CLIENT_SEQ = itertools.count(1)
_HTTP_CLIENT_LOCK = threading.Lock()


def _client_forwarded_for():
    """A distinct edge client per harness request.

    The public rate limiter now counts requests without an X-Forwarded-For
    under one direct bucket. The live harness is an external client, so it
    must present distinct clients like the edge does; probes that test the
    limiter pass an explicit header.
    """
    with _HTTP_CLIENT_LOCK:
        value = next(_HTTP_CLIENT_SEQ)
    return "198.18.%d.%d" % ((value // 250) % 250, value % 250 + 1)


def http_req(method, url, headers=None, body=None, timeout=15):
    method = method.upper()
    headers = dict(headers or {})
    if not any(key.lower() == "x-forwarded-for" for key in headers):
        headers["X-Forwarded-For"] = _client_forwarded_for()
    parts = urllib.parse.urlsplit(url)
    path = parts.path + (("?" + parts.query) if parts.query else "")
    host = parts.hostname
    port = parts.port or 80
    _http_count("requests")
    if method not in _HTTP_IDEMPOTENT:
        conn = http.client.HTTPConnection(host, port, timeout=timeout)
        _http_count("connects")
        try:
            conn.request(method, path or "/", body=body, headers=headers or {})
            raw = conn.getresponse()
            payload = raw.read()
            return Resp(raw.status, list(raw.getheaders()), payload)
        except (OSError, http.client.HTTPException) as error:
            raise ReqError("%s %s: %s" % (method, url, error))
        finally:
            conn.close()
    # A pooled idempotent request: retry once on a stale keep-alive socket.
    last_error = None
    for attempt in range(2):
        key, conn = _pooled_connection(host, port, timeout)
        try:
            conn.request(method, path or "/", body=body, headers=headers or {})
            raw = conn.getresponse()
            payload = raw.read()
            will_close = raw.will_close
            response = Resp(raw.status, list(raw.getheaders()), payload)
            if will_close:
                _drop_pooled_connection(key)
            return response
        except (OSError, http.client.HTTPException) as error:
            last_error = error
            _drop_pooled_connection(key)
            if attempt == 0:
                _http_count("retries")
                continue
    raise ReqError("%s %s: %s" % (method, url, last_error))


def http_pool_self_test():
    """A real loopback server proves the pool semantics: idempotent requests
    reuse one connection, a server that closes the socket is reconnected, and
    a POST always uses a fresh connection (never replayed)."""
    import socketserver

    accepted = []
    state = {"close_after": False}

    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *args):
            pass

        def _answer(self, with_body=True):
            body = b"ok" if with_body else b""
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            if state["close_after"]:
                self.send_header("Connection", "close")
                self.close_connection = True
            self.end_headers()
            if body:
                self.wfile.write(body)

        def do_GET(self):
            self._answer()

        def do_POST(self):
            length = int(self.headers.get("content-length") or 0)
            if length:
                self.rfile.read(length)
            self._answer()

    class Server(socketserver.ThreadingTCPServer):
        allow_reuse_address = True
        daemon_threads = True

    def counting_setup(handler):
        accepted.append(1)
        Handler.setup(handler)

    handler_cls = type("CountingHandler", (Handler,), {"setup": counting_setup})
    server = Server(("127.0.0.1", 0), handler_cls)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base = "http://127.0.0.1:%d" % server.server_address[1]
    problems = []
    try:
        drop_http_pool()
        reset_http_stats()
        for _ in range(12):
            if http_req("GET", base + "/x").text != "ok":
                problems.append("pooled GET did not return the body")
                break
        stats = http_stats()
        if stats["connects"] > 1 or stats["reused"] < 11:
            problems.append(
                "12 GETs opened %d connection(s) and reused %d; expected 1 and 11"
                % (stats["connects"], stats["reused"])
            )
        if len(accepted) != 1:
            problems.append(
                "the server accepted %d connections for a keep-alive GET loop"
                % len(accepted)
            )
        # A server that closes the socket must be reconnected, not failed.
        state["close_after"] = True
        if http_req("GET", base + "/close").text != "ok":
            problems.append("the close-after-response GET failed")
        state["close_after"] = False
        if http_req("GET", base + "/again").text != "ok":
            problems.append("the GET after a closed connection failed")
        if len(accepted) < 2:
            problems.append("the closed connection was not re-established")
        # POSTs never share a connection.
        before = http_stats()["connects"]
        for _ in range(4):
            http_req("POST", base + "/p", body=b"x", headers={"Content-Type": "text/plain"})
        after = http_stats()["connects"]
        if after - before != 4:
            problems.append("4 POSTs opened %d connection(s); each must be fresh" % (after - before))
        drop_http_pool()
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        drop_http_pool()
    return problems


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


def cookie_path_matches(cookie_path, request_path):
    """RFC 6265 section 5.1.4 path matching (the rule a browser applies)."""
    if request_path == cookie_path:
        return True
    return request_path.startswith(cookie_path) and (
        cookie_path.endswith("/")
        or request_path[len(cookie_path):len(cookie_path) + 1] == "/"
    )


class BrowserCookieJar:
    """The smallest path-scoped cookie jar the auth probes need.

    The product sets a session `Path`; a browser only sends the cookie to a
    request whose path matches it. The pre-fix `Path=/console` therefore left
    the overview at `/` anonymous (303) even though a manual `Cookie` header
    would have been honoured by the server. Minting into this jar and reading
    `/` through `header("/")` makes that browser rule part of the assertion.
    """

    def __init__(self):
        self.entries = []

    def store(self, set_cookie):
        if not set_cookie:
            return
        parts = [part.strip() for part in set_cookie.split(";")]
        name, _, value = parts[0].partition("=")
        attrs = {}
        for part in parts[1:]:
            key, _, attr_value = part.partition("=")
            attrs[key.strip().lower()] = attr_value.strip()
        self.entries.append(
            {
                "name": name.strip(),
                "value": value.strip(),
                "path": attrs.get("path") or "/",
                "secure": "secure" in attrs,
            }
        )

    def header(self, request_path):
        pairs = [
            entry
            for entry in self.entries
            if cookie_path_matches(entry["path"], request_path)
        ]
        return "; ".join("%s=%s" % (entry["name"], entry["value"]) for entry in pairs)


def console_session_cookie_problems(set_cookie, mint_path):
    """The console session cookie contract shared by both mint paths.

    The browser only sends a cookie to a request whose path matches the
    cookie's `Path` (RFC 6265 section 5.1.4). The product scopes the session to
    `/` so the overview at `/` and every `/console` page receive it; the old
    `Path=/console` scope left `/` unreachable. `Path=/;` (with the delimiter)
    is asserted so a substring match cannot accept `Path=/console;`.
    """
    if not set_cookie:
        return ["%s set no Set-Cookie" % mint_path]
    problems = []
    for flag in ("Path=/;", "HttpOnly", "SameSite=Lax"):
        if flag not in set_cookie:
            problems.append("%s cookie lacks %s" % (mint_path, flag))
    if "Path=/console" in set_cookie:
        problems.append("%s cookie is still scoped to /console" % mint_path)
    if "Domain=" in set_cookie:
        problems.append("%s cookie is not host-only (carries Domain=)" % mint_path)
    return problems


def overview_problems(resp, mint_path):
    """The overview at `/` must render under the session the mint path set."""
    problems = []
    if resp.status != 200:
        problems.append(
            "%s session GET / -> HTTP %d, expected the overview 200"
            % (mint_path, resp.status)
        )
    elif "<h1>Your account at a glance</h1>" not in resp.text:
        problems.append("%s session GET / rendered without the overview heading" % mint_path)
    elif "Verified records" not in resp.text:
        problems.append("%s session GET / rendered without the verified-records KPI" % mint_path)
    return problems


class Harness:
    def __init__(self, args):
        self.args = args
        self.root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
        self.workdir = (
            args.workdir if os.path.isabs(args.workdir) else os.path.join(self.root, args.workdir)
        )
        # Every run starts from empty service state: fixed settlement
        # references, subscriber ids and sign-in rows are replay-protected,
        # so a reused workdir would make a second run answer "already
        # recorded" everywhere. Logs and artifacts from the run stay in the
        # directory; only the previous run's state is removed.
        if not getattr(args, "keep_workdir", False):
            shutil.rmtree(self.workdir, ignore_errors=True)
        os.makedirs(self.workdir, exist_ok=True)
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
            "apex-api-unconfigured": args.port_base + 4,
            "apex-api-links-off": args.port_base + 5,
        }
        self.procs = {}
        self.checks = []
        self.cells = {}
        self.surface = {}
        self.required = set()
        # Relevance: `surface_sources` is the per-surface repo-path list read
        # from the catalog, `selected` is the filtered cell set (== required
        # when unfiltered), and `skipped_classes` lists the classes the filter
        # left untouched.
        self.surface_sources = {}
        self.selected = set()
        self.filtered = False
        self.changed_paths = []
        self.skipped_classes = set()
        self.selection_notes = []
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
        # Declared publisher-console GETs that must refuse anonymous reads
        # (`"session_required": true` in the catalog): the anonymous sweep and
        # the with-session sweep both read this set.
        self.session_required = set()
        self.fleet = {}
        self.monetization = {}
        # Per-run caches for the per-cell loops: the class index over the
        # required cells, the response-contract tables and the no-inline
        # sweep targets are built once instead of once per cell.
        self._cell_index_cache = None
        self._contract_expectations = {}
        self._no_inline_targets = None
        # Per-class wall time (measurement, never asserted) and the run start
        # for the total. Reporting only: the probes and their order are
        # unchanged.
        self.class_timings = {}
        self.run_started_monotonic = time.monotonic()
        # Deterministic work counters (call counts, never timings): the
        # bookkeeping checks assert one catalog read, one baseline read, one
        # results write and one dogfood-plan process per run, and the results
        # document carries the per-program spawn counts.
        self.counters = {
            "surfaces_reads": 0,
            "baseline_reads": 0,
            "baseline_writes": 0,
            "results_writes": 0,
            "dogfood_plan_spawns": 0,
            "cell_index_builds": 0,
            "cells_of_calls": 0,
            "subprocess_spawns": 0,
            "server_starts": 0,
            "server_kills": 0,
        }

    # ------------------------------------------------------------------ io
    def spawn(self, argv, **kwargs):
        """`subprocess.run` with a deterministic spawn counter. A program name
        is part of the counter key, so the per-run result can prove e.g. one
        `dogfood-plan` process for the whole catalog."""
        self.counters["subprocess_spawns"] += 1
        key = "spawn:%s" % os.path.basename(str(argv[0] if argv else "?"))
        self.counters[key] = self.counters.get(key, 0) + 1
        return subprocess.run(argv, **kwargs)

    def spawn_popen(self, argv, **kwargs):
        """`subprocess.Popen` with the same deterministic spawn counter."""
        self.counters["subprocess_spawns"] += 1
        key = "spawn:%s" % os.path.basename(str(argv[0] if argv else "?"))
        self.counters[key] = self.counters.get(key, 0) + 1
        return subprocess.Popen(argv, **kwargs)

    def runtime_counters(self):
        counters = dict(sorted(self.counters.items()))
        counters["http"] = http_stats()
        return counters

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
        self.counters["surfaces_reads"] += 1
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
            self.check(
                "catalog|probe-classes",
                True,
                "the %d frozen probe classes, in order" % len(FROZEN_PROBE_CLASSES),
            )
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
                sources = route.get("sources") or []
                if not sources:
                    self.check(
                        "catalog|sources",
                        False,
                        "%s %s declares no sources; the relevance filter needs them"
                        % (service, route["path"]),
                    )
                self.surface_sources["%s|%s" % (service, route["path"])] = list(sources)
        self.selected = set(self.required)
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
        # The recorded session gate: every `"session_required": true` route is
        # an apex-api console GET, and the documented public exceptions are
        # declared as plain routes. The anonymous sweep and the with-session
        # sweep both drive from this set.
        self.session_required = set()
        bad_markers = []
        declared_get = set()
        for service, declared_service in self.surface["services"].items():
            for route in declared_service["routes"]:
                if "GET" in [m.upper() for m in route["methods"]]:
                    declared_get.add("%s|GET %s" % (service, route["path"]))
                if route.get("session_required"):
                    cell_surface = "%s|GET %s" % (service, route["path"])
                    if service != "apex-api" or cell_surface not in declared_get:
                        bad_markers.append(cell_surface)
                        continue
                    self.session_required.add(cell_surface)
        public = [
            "/console",
            "/console/",
            "/console/ledger",
            "/console/signin",
            "/free-audit",
            "/free-audit/",
            "/audit-requested/",
            "/watch",
            "/watch/",
            "/watch/requested",
        ]
        collisions = sorted(
            surface for surface in self.session_required if surface.split("|", 1)[1][4:] in public
        )
        self.check(
            "catalog|session-markers",
            not bad_markers and not collisions and len(self.session_required) == 20,
            "%d session_required console GETs recorded; public exceptions stay unmarked"
            % len(self.session_required)
            if not bad_markers and not collisions and len(self.session_required) == 20
            else "bad markers %s; public-route collisions %s; total %d (expected 20)"
            % (bad_markers, collisions, len(self.session_required)),
        )

    # ------------------------------------------------------------ relevance
    def git_diff_paths(self, spec):
        proc = self.spawn(
            ["git", "diff", "--name-only", spec],
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=120,
        )
        if proc.returncode != 0:
            raise ReqError(
                "git diff --name-only %s failed (exit %d): %s"
                % (spec, proc.returncode, (proc.stderr or "").strip())
            )
        return [line.strip() for line in (proc.stdout or "").splitlines() if line.strip()]

    def resolve_changed_paths(self):
        changed = list(self.args.changed or [])
        if self.args.git_diff:
            changed.extend(self.git_diff_paths(self.args.git_diff))
        return sorted(set(changed))

    def apply_selection(self):
        """Resolve `--changed` / `--git-diff` into the cell subset to run. With
        neither flag the run is unfiltered (every declared cell). Otherwise
        only the cells whose surface `sources` intersect a changed path are
        selected, plus the always-run core set; classes with no selected cell
        are skipped and reported."""
        changed = self.resolve_changed_paths()
        if not changed:
            self.selected = set(self.required)
            self.filtered = False
            return
        self.filtered = True
        self.changed_paths = changed
        selected, skipped, unmatched = select_relevant_cells(
            self.required, self.surface_sources, changed
        )
        self.selected = selected
        self.skipped_classes = skipped
        if unmatched:
            self.selection_notes.append(
                "%d changed path(s) matched no declared source: %s"
                % (len(unmatched), ", ".join(unmatched))
            )

    def site_selected(self):
        if not self.filtered:
            return True
        return any(change.startswith("site/") for change in self.changed_paths)

    def check_site(self):
        if not self.site_selected():
            self.check(
                "site|pages-min",
                True,
                "SKIPPED: no changed path touches site/; the static half is not in this selection",
            )
            return
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
        for name in ("apex-api", "apex-control", "apex-sales-web", "apex-sales", "apex-runner", "apex-watch", "apex-tools"):
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

    def runner_env(self):
        """The fleet-mode runner environment: the console-validated vaulted
        connection registry (`APEX_CONNECTIONS_ROOT`) drives which publishers
        run, exactly the deployment shape the product documents. The legacy
        single-publisher variables are deliberately absent."""
        env = dict(os.environ)
        for name in (
            "APEX_CLICKHOUSE_HTTP",
            "APEX_CLICKHOUSE_DB",
            "APEX_RUNNER_PUBLISHER",
            "APEX_RUNNER_LEDGER",
            "APEX_MAX_API_KEY",
            "APEX_MAX_REPORT_KEY",
            "APEX_MAX_MANAGEMENT_BASE",
            "APEX_MAX_REPORTING_BASE",
        ):
            env.pop(name, None)
        env.update(
            {
                "APEX_CLICKHOUSE_HTTP": self.ch_url,
                "APEX_CLICKHOUSE_DB": self.database,
                "APEX_ANALYTICS_ENV": "production",
                "APEX_RUNNER_SIGNING_KEY_HEX": RUNNER_SIGNING_SEED_HEX,
                "APEX_RUNNER_DATA_DIR": self.fleet["runner_data"],
                "APEX_CONNECTIONS_ROOT": self.fleet["credentials"],
                "APEX_RUNNER_SNAPSHOT": self.fleet["snapshots"],
                "APEX_VAULT_KEY_HEX": API_VAULT_KEY_HEX,
                "APEX_RUNNER_LEDGER_ROOT": self.fleet["ledgers"],
                "APEX_BILLING_PROFILE": self.fleet["seller"],
                "APEX_BILLING_VAT_RATE_BP": str(SELLER_VAT_RATE_BP),
                "APEX_OPERATOR_JOBS_DIR": self.fleet["jobs_dir"],
                "APEX_METRICS_TEXTFILE": os.path.join(self.workdir, "runner", "runner.prom"),
            }
        )
        os.makedirs(self.fleet["jobs_dir"], exist_ok=True)
        return env

    def run_runner_command(self, argv, cwd=None, extra_env=None, timeout=180):
        env = self.runner_env()
        if extra_env:
            env.update(extra_env)
        return self.spawn(
            [self.bin_path("apex-runner")] + argv,
            cwd=cwd or self.root,
            env=env,
            capture_output=True,
            text=True,
            timeout=timeout,
        )

    def run_runner(self):
        proc = self.run_runner_command(["daily"])
        out = (proc.stdout or "") + (proc.stderr or "")
        ok = proc.returncode == 0 and "Projections: rebuilt" in out
        self.check(
            "clickhouse|runner-daily",
            ok,
            "apex-runner daily exit=%d; %s" % (proc.returncode, sanitize_evidence(out, 400)),
            "apex-runner daily (fleet: APEX_CONNECTIONS_ROOT=%s, APEX_CLICKHOUSE_DB=%s)"
            % (self.fleet["credentials"], self.database),
        )
        if ok:
            self.runner_ran = True
        return ok

    def bootstrap_monitoring(self):
        """Activate the monitoring plan the way the product does: the `cycle`
        command consumes a snapshot and records the tier chosen from its
        reported 30-day revenue (the seeded $350k band -> Mid)."""
        work = os.path.join(self.workdir, "billing-run")
        os.makedirs(work, exist_ok=True)
        proc = self.run_runner_command(
            ["cycle", "--snapshot", self.fleet["snapshot"], "--json"], cwd=work
        )
        out = (proc.stdout or "") + (proc.stderr or "")
        ok = proc.returncode == 0
        tier = None
        if ok:
            try:
                document = json.loads(proc.stdout)
                tier = document.get("cycle", {}).get("monitoring_activated")
            except ValueError:
                ok = False
        self.monetization["cycle"] = {
            "exit": proc.returncode,
            "activated": tier,
            "human": sanitize_evidence(out, 400),
        }
        self.check(
            "boot|monitoring-activation",
            ok and tier == "Mid",
            "apex-runner cycle exit=%d; activated=%r; %s"
            % (proc.returncode, tier, sanitize_evidence(out, 300)),
            "apex-runner cycle --snapshot band-snapshot.json (30-day revenue $350k)",
        )

    def seed_admob_kept_change(self):
        """Append the signed records of one kept AdMob change to the shared
        fleet ledger: the exact envelope records a settled AdMob mediation A/B
        experiment produces (experimentStarted + preRegistration + result
        keep). Signed with the deployment's runner seed, so the product's own
        ledger verifier accepts the chain byte-for-byte."""
        seed = bytes.fromhex(RUNNER_SIGNING_SEED_HEX)
        ledger_path = os.path.join(self.fleet["ledgers"], "northstar.jsonl")
        with open(ledger_path, "r", encoding="utf-8") as fh:
            lines = [line for line in fh.read().splitlines() if line.strip()]
        previous = json.loads(lines[-1])["record"]["hash"] if lines else "0" * 64
        seq = len(lines)
        now_ms = int(time.time() * 1000)
        started = {
            "type": "experimentStarted",
            "atUnixMs": now_ms - 3 * 86_400_000,
            "platform": "admob",
            "experimentId": ADMOB_KEPT_EXPERIMENT,
            "spec": {
                "name": ADMOB_KEPT_EXPERIMENT,
                "adUnitId": "rw_de_01",
                "metric": "revenue",
                "variants": 2,
                "changeDescription": "AdMob mediation-group eCPM",
                "testGroupAllocation": 25,
            },
        }
        observed = {
            "type": "experimentObserved",
            "atUnixMs": now_ms - 2 * 86_400_000,
            "platform": "admob",
            "experimentId": ADMOB_KEPT_EXPERIMENT,
            "result": {
                "experimentId": ADMOB_KEPT_EXPERIMENT,
                "state": "completed",
                "results": [],
                "winnerVariantId": "test",
            },
        }
        preregistration = {
            "type": "preRegistration",
            "experiment_id": ADMOB_KEPT_EXPERIMENT,
            "scope": "g1/rw_de_01",
            "before_value": 1_000_000,
            "after_value": 1_200_000,
            "split": 0.25,
            "metric": "revenue_per_user",
            "burn_in_days": 3,
            "window_days": 14,
            "looks": 1,
            "extension_allowed": False,
            "guardrail_thresholds": {},
            "code_version_hash": "deadbeef",
            "rate_bp": 2_000,
        }
        result = {
            "type": "result",
            "experiment_id": ADMOB_KEPT_EXPERIMENT,
            "daily_log_ratios": [0.1],
            "mean_log": 0.1,
            "lower_90": 0.05,
            "lower_99": 0.02,
            "bootstrap_lower": 0.01,
            "decision": "keep",
            "lambda": 0.03,
        }
        lift_records = []
        for payload in (preregistration, result):
            payload_json = json.dumps(payload, separators=(",", ":"), ensure_ascii=False)
            lift_records.append(
                {
                    "type": "liftRecord",
                    "atUnixMs": now_ms - 2 * 86_400_000,
                    "payloadSha256": hashlib.sha256(payload_json.encode("utf-8")).hexdigest(),
                    "payload": payload_json,
                }
            )
        out_lines = []
        for record in [started, observed] + lift_records:
            sealed = seal_ledger_record(record, seq, previous)
            out_lines.append(ledger_line(sealed, seed))
            previous = sealed["hash"]
            seq += 1
        with open(ledger_path, "a", encoding="utf-8") as fh:
            fh.write("\n".join(out_lines) + "\n")
        self.check(
            "boot|admob-kept-change",
            True,
            "seeded one kept AdMob change (%s) as %d signed ledger records continuing seq %d"
            % (ADMOB_KEPT_EXPERIMENT, len(out_lines), seq - len(out_lines)),
        )

    # ------------------------------------------- runner defect regression cells
    def run_runner_regression_cells(self):
        """The two runner-level regression cells the extended live dogfood
        found, each running the real `apex-runner` binary: the legacy-mode
        monthly with monitoring on (the historical abort) and the cycle
        against the committed admob fixture snapshot (the historical
        "not writable" abort). Recorded as checks like the neighbouring
        runner probes (`boot|monitoring-activation`,
        `clickhouse|runner-daily`); both are bounded local invocations."""
        self.legacy_monthly_monitoring_cell()
        self.admob_cycle_capability_cell()

    def legacy_monthly_monitoring_cell(self):
        """Cell `runner|legacy-monthly-monitoring`: legacy-mode monthly with
        monitoring env defaults and the seeded seller billing profile.

        Without a connections root the pass must exit 0 and record the typed
        monitoring-skip reason while billing no plan line — the historical
        abort on this exact env is the defect. With the root seeded the same
        command bills the chosen tier line; the real binary carries a root
        only in the single-publisher fleet shape, so the root-seeded variant
        runs that shape and asserts the recorded Mid tier line."""
        problems = []
        notes = []
        period = time.strftime("%Y-%m", time.gmtime())
        start = period + "-01"
        work = os.path.join(self.workdir, "runner-monthly-check")
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(work)

        # Variant 1, the exact defect: legacy mode (publisher env set, no
        # connections root), monitoring on by default, seller profile seeded.
        legacy_env = self.runner_env()
        for name in (
            "APEX_CONNECTIONS_ROOT",
            "APEX_CREDENTIAL_ROOT",
            "APEX_CLICKHOUSE_HTTP",
            "APEX_CLICKHOUSE_DB",
            "APEX_RUNNER_WRITABLE",
        ):
            legacy_env.pop(name, None)
        legacy_env.update(
            {
                "APEX_RUNNER_PUBLISHER": "northstar",
                "APEX_RUNNER_LEDGER": os.path.join(work, "legacy-ledger.jsonl"),
                "APEX_MAX_API_KEY": "dogfood-live-legacy-key",
                "APEX_MAX_REPORT_KEY": "dogfood-live-legacy-report-key",
            }
        )
        proc = self.spawn(
            [
                self.bin_path("apex-runner"),
                "monthly",
                "--month",
                period,
                "--start",
                start,
                "--json",
            ],
            cwd=work,
            env=legacy_env,
            capture_output=True,
            text=True,
            timeout=60,
        )
        out = (proc.stdout or "") + (proc.stderr or "")
        if proc.returncode != 0:
            problems.append(
                "legacy monthly exit=%d: %s" % (proc.returncode, sanitize_evidence(out, 240))
            )
        else:
            document = None
            try:
                document = json.loads(proc.stdout)
            except ValueError as error:
                problems.append("legacy monthly stdout is not JSON: %s" % error)
            if document is not None:
                billing = document.get("billing", {})
                if billing.get("monitoring_skipped") != MONITORING_ROOT_UNCONFIGURED_REASON:
                    problems.append(
                        "no typed monitoring skip reason (got %r)"
                        % billing.get("monitoring_skipped")
                    )
                elif billing.get("fees") != 0 or billing.get("rendered_invoice"):
                    problems.append(
                        "the skipped plan still billed a line (fees=%r, rendered=%r)"
                        % (billing.get("fees"), bool(billing.get("rendered_invoice")))
                    )
                else:
                    notes.append(
                        "no root: exit 0, typed skip %r, no plan line billed"
                        % MONITORING_ROOT_UNCONFIGURED_REASON
                    )
        # The human report carries the same typed reason verbatim.
        proc = self.spawn(
            [self.bin_path("apex-runner"), "monthly", "--month", period, "--start", start],
            cwd=work,
            env=legacy_env,
            capture_output=True,
            text=True,
            timeout=60,
        )
        human = (proc.stdout or "") + (proc.stderr or "")
        if (
            proc.returncode != 0
            or "Monitoring: skipped (monitoring state root not configured)" not in human
        ):
            problems.append(
                "the human report lacks the typed skip line (exit %d): %s"
                % (proc.returncode, sanitize_evidence(human, 200))
            )

        # Variant 2, the tier assertion: run with the seeded root (the
        # single-publisher fleet shape) and require the Mid tier line chosen
        # from the seeded $350k band.
        fleet_cwd = os.path.join(work, "fleet")
        os.makedirs(fleet_cwd)
        proc = self.run_runner_command(
            ["monthly", "--month", period, "--start", start, "--json"], cwd=fleet_cwd
        )
        out = (proc.stdout or "") + (proc.stderr or "")
        if proc.returncode != 0:
            problems.append(
                "root-seeded monthly exit=%d: %s" % (proc.returncode, sanitize_evidence(out, 240))
            )
        else:
            document = None
            try:
                document = json.loads(proc.stdout)
            except ValueError as error:
                problems.append("root-seeded monthly stdout is not JSON: %s" % error)
            if document is not None:
                rendered = document.get("billing", {}).get("rendered_invoice") or ""
                tier_line = "monitoring:%s: net %d micros" % (period, MONITORING_MID_FEE_MICROS)
                if tier_line not in rendered:
                    problems.append(
                        "the root-seeded monthly run lacks the chosen tier line %r"
                        % tier_line
                    )
                else:
                    notes.append("root seeded: the record-and-bill path renders %r" % tier_line)
        self.check(
            "runner|legacy-monthly-monitoring",
            not problems,
            "; ".join(notes + problems) if (notes or problems) else "no observation",
            "apex-runner monthly (legacy, no root; then the root-seeded singleton) "
            "with monitoring env defaults and the seeded billing profile",
        )

    def admob_cycle_capability_cell(self):
        """Cell `runner|admob-cycle-capability`: cycle against the committed
        admob fixture snapshot in fleet/AdMob mode.

        The MAX-style country-floor proposals are not in the AdMob adapter's
        verified write set: the pass must exit 0 with the typed
        awaiting-live-verification record, and the canary refuses the
        historical hard "not writable" abort text."""
        problems = []
        notes = []
        work = os.path.join(self.workdir, "runner-cycle-check")
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(work)
        env = self.runner_env()
        env.pop("APEX_RUNNER_WRITABLE", None)
        snapshot = self.repo_path("rust/crates/apex-api/fixtures/admob.json")
        proc = self.spawn(
            [self.bin_path("apex-runner"), "cycle", "--snapshot", snapshot, "--json"],
            cwd=work,
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )
        out = (proc.stdout or "") + (proc.stderr or "")
        typed = "awaiting live verification for Admob: setCountryFloor"
        if proc.returncode != 0:
            problems.append("cycle exit=%d: %s" % (proc.returncode, sanitize_evidence(out, 240)))
        else:
            document = None
            try:
                document = json.loads(proc.stdout)
            except ValueError as error:
                problems.append("cycle stdout is not JSON: %s" % error)
            if document is not None:
                cycle = document.get("cycle", {})
                holds = cycle.get("awaiting_verification") or []
                if cycle.get("proposed", 0) < 1:
                    problems.append("the fixture proposed nothing to hold")
                elif cycle.get("applied") != 0:
                    problems.append("held operations were applied (%r)" % cycle.get("applied"))
                elif not holds or any(reason != typed for reason in holds):
                    problems.append("the typed awaiting record is missing or wrong: %r" % holds)
                else:
                    notes.append(
                        "exit 0: %d proposal(s) held with the typed record %r"
                        % (len(holds), typed)
                    )
        if "not writable" in out:
            # The canary: the historical hard error text must never return.
            problems.append(
                "the historical hard abort text is present: %s" % sanitize_evidence(out, 240)
            )
        self.check(
            "runner|admob-cycle-capability",
            not problems,
            "; ".join(notes + problems) if (notes or problems) else "no observation",
            "apex-runner cycle --snapshot rust/crates/apex-api/fixtures/admob.json "
            "(fleet AdMob, no verified write set)",
        )

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
        self.counters["server_starts"] += 1
        self.counters["start:%s" % name] = self.counters.get("start:%s" % name, 0) + 1
        drop_http_pool()
        log_path = os.path.join(self.workdir, log_name)
        log_file = open(log_path, "ab")
        child_env = dict(os.environ)
        child_env.pop("APEX_CLICKHOUSE_HTTP", None)
        child_env.pop("APEX_CLICKHOUSE_DB", None)
        child_env.update(env)
        proc = self.spawn_popen(
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
        self.counters["server_kills"] += 1
        self.counters["kill:%s" % name] = self.counters.get("kill:%s" % name, 0) + 1
        drop_http_pool()
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

        # One loopback stub serves the two checkout/AdMob seams the corrected
        # live probes exercise: the Adyen payment-link/card-on-file calls and
        # the AdMob mediation-group read. Every request is recorded.
        def stub_responder(method, path, requests):
            if path.startswith("/v1beta/") and "mediationGroups" in path:
                return (200, {"mediationGroups": []})
            if path.startswith("/v71/paymentLinks"):
                return (200, {"id": "pl_dogfood_live", "url": API_ADYEN_CHECKOUT_LINK})
            if path.startswith("/v71/payments"):
                return (
                    200,
                    {
                        "pspReference": "PSPCARDOGFOODLIVE",
                        "resultCode": "Authorised",
                        "amount": {"value": int(WATCH_FEE_MICROS // 10_000), "currency": "USD"},
                    },
                )
            return (404, {"error": "unexpected stub path"})

        self.api["stub"] = LocalStub(stub_responder).start()
        os.makedirs(os.path.join(work, "proposals"))
        os.makedirs(os.path.join(work, "credentials"))
        shutil.copy(self.repo_path("rust/crates/apex-api/fixtures/admob.json"), snapshots)
        shutil.copy(self.repo_path("rust/crates/apex-api/fixtures/northstar.jsonl"), ledgers)
        self.normalize_fixture_lift_records(os.path.join(ledgers, "northstar.jsonl"))
        # Keep one pristine copy of the normalized 24-record fixture for the
        # pagination probe: the shared northstar ledger is mutated below by the
        # harness's own kept-change and runner records, while this copy stays
        # exactly the committed fixture so "Showing 1-24 of 24" is a real
        # assertion about the fixture, not a coincidence of the seed order.
        shutil.copy(
            os.path.join(ledgers, API_PUBLISHER + ".jsonl"),
            os.path.join(ledgers, PAGINATION_PUBLISHER + ".jsonl"),
        )
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
        # A northstar-only marker unit so the tenant-isolation probe can prove
        # which publisher's snapshot a page rendered.
        snapshot.setdefault("adUnits", []).append(
            {
                "platformUnitId": "northstar-only-1",
                "name": NORTHSTAR_UNIT,
                "format": "rewarded",
                "enabled": True,
                "countryFloors": [],
            }
        )
        with open(snapshot_path, "w", encoding="utf-8") as fh:
            json.dump(snapshot, fh)
        # The fixed contract keys snapshots by publisher and platform
        # (`<root>/<publisher>/<platform>.json`) while the current tree reads
        # the flat `<root>/<platform>.json`. Both layouts are seeded: the flat
        # hostile copy keeps the escaping/assets probes fed on either tree, the
        # per-publisher copies are what the tenant-isolation probe compares.
        for publisher in (API_PUBLISHER, RIVAL_PUBLISHER):
            publisher_dir = os.path.join(snapshots, publisher)
            os.makedirs(publisher_dir, exist_ok=True)
            with open(os.path.join(publisher_dir, "admob.json"), "w", encoding="utf-8") as fh:
                json.dump(snapshot, fh)
        # The rival's snapshot carries its own markers so a shared-snapshot read
        # is visible in the bytes of the other publisher's pages.
        rival_snapshot = {
            "platform": "admob",
            "fetchedAtUnixMs": int(time.time() * 1000),
            "adUnits": [
                {
                    "platformUnitId": "rival-only-1",
                    "name": RIVAL_UNIT,
                    "format": "banner",
                    "enabled": True,
                    "countryFloors": [],
                }
            ],
            "waterfall": [],
            "mediationGroups": [],
            "bidderAccounts": [],
            "countryRevenue": [],
            "appAdsTxt": {
                "present": True,
                "lines": ["rival-only.example, pub-rival, DIRECT"],
                "expectedNetworks": ["rival-only.example", RIVAL_AUDIT_MARKER],
            },
            "platformTotals30d": {"revenue30d": 0, "impressions30d": 0},
            "mediationTotals30d": {"revenue30d": 0, "impressions30d": 0},
        }
        with open(
            os.path.join(snapshots, RIVAL_PUBLISHER, "admob.json"), "w", encoding="utf-8"
        ) as fh:
            json.dump(rival_snapshot, fh)
        # The rival ledger: the first five verified records of the fixture check
        # under the same deployment key, so the overview's "Verified records"
        # total is 24 for northstar and 5 for rival. The two must never be
        # summed across a session.
        with open(self.repo_path("rust/crates/apex-api/fixtures/northstar.jsonl"), "r", encoding="utf-8") as fh:
            fixture_lines = [line for line in fh.read().splitlines() if line.strip()]
        with open(os.path.join(ledgers, RIVAL_PUBLISHER + ".jsonl"), "w", encoding="utf-8") as fh:
            for line in fixture_lines[:RIVAL_LEDGER_RECORDS]:
                fh.write(line + "\n")
        with open(os.path.join(work, "tokens.json"), "w", encoding="utf-8") as fh:
            json.dump(
                {
                    "northstar": API_LEDGER_TOKEN,
                    RIVAL_PUBLISHER: RIVAL_TOKEN,
                    PAGINATION_PUBLISHER: PAGINATION_TOKEN,
                },
                fh,
            )
        # The stub sendmail: appends every composed message to a file and marks
        # its end, so the signin-ownership probe can read the `To:` and the
        # one-time link of each delivery.
        mail_outbox = os.path.join(work, "mail.outbox")
        with open(mail_outbox, "w", encoding="utf-8"):
            pass
        sendmail_stub = os.path.join(work, "sendmail-stub.sh")
        with open(sendmail_stub, "w", encoding="utf-8") as fh:
            fh.write(
                "#!/bin/sh\n"
                'cat >> "%s"\n'
                'printf \'\\n===DOGFOOD-MESSAGE-END===\\n\' >> "%s"\n'
                "exit 0\n" % (mail_outbox, mail_outbox)
            )
        os.chmod(sendmail_stub, 0o755)
        self.api["mail_outbox"] = mail_outbox
        self.api["sendmail_stub"] = sendmail_stub
        self.api["rival_token"] = RIVAL_TOKEN
        self.seed_signin_rows(work)
        self.seed_billing_stores(work)
        return work

    def normalize_fixture_lift_records(self, path):
        """Re-seal the committed ledger copy so every `liftRecord` payload uses
        a shape the product's current `LiftRecord` parser accepts.

        The committed fixture still carries one legacy `{"type":"lift"}`
        payload; the runner's daily pass fails closed on it ("an unparseable
        lift payload refuses the scan"). The fixture itself is product data,
        so the harness normalizes only its own copy: the legacy measurement is
        rewritten as an equivalent `dataPull` record (semantically inert for
        the fee scan) and the whole chain from that point is re-sealed with the
        same [7u8; 32] seed the fixture already uses. The API's own ledger
        verifier remains the final oracle."""
        accepted = {
            "preRegistration",
            "dataPull",
            "guardrail",
            "result",
            "monthlyBilling",
            "billingChange",
        }
        seed = bytes.fromhex(RUNNER_SIGNING_SEED_HEX)
        with open(path, "r", encoding="utf-8") as fh:
            entries = [json.loads(line) for line in fh if line.strip()]
        previous = "0" * 64
        changed = 0
        out = []
        for entry in entries:
            record = entry["record"]
            reseal = False
            if record.get("type") == "liftRecord":
                try:
                    payload = json.loads(record.get("payload") or "{}")
                except ValueError:
                    payload = {}
                if payload.get("type") not in accepted:
                    at_ms = int(record.get("atUnixMs") or int(time.time() * 1000))
                    replacement = {
                        "type": "dataPull",
                        "day": time.strftime("%Y-%m-%d", time.gmtime(at_ms / 1000.0)),
                        "rows_sha256": hashlib.sha256(b"[]").hexdigest(),
                        "rows": [],
                    }
                    text = canonical_json(replacement)
                    record["payload"] = text
                    record["payloadSha256"] = hashlib.sha256(text.encode("utf-8")).hexdigest()
                    reseal = True
                    changed += 1
            if changed:
                record = seal_ledger_record(record, record["seq"], previous)
                entry = json.loads(ledger_line(record, seed))
            previous = record["hash"]
            out.append(json.dumps(entry, separators=(",", ":"), ensure_ascii=False))
        if changed:
            with open(path, "w", encoding="utf-8") as fh:
                fh.write("\n".join(out) + "\n")
            self.check(
                "boot|ledger-lift-shape",
                True,
                "normalized %d legacy lift payload(s) in the harness ledger copy and re-sealed the chain "
                "with the fixture seed (the product's parser is the oracle)" % changed,
            )

    def seed_signin_rows(self, work):
        """Hash-only `signin.jsonl` rows beside the leads file: one valid link,
        one already-used link and one expired link. The raw tokens exist only
        in the harness; the product stores SHA-256 hex digests (its own
        format), so the seeds are indistinguishable from rows the product
        itself would append."""
        now_ms = int(time.time() * 1000)
        tokens = {"valid": "a1" * 16, "used": "b2" * 16, "expired": "c3" * 16}

        def row(token, at_ms, expires_at_ms, used_at_ms):
            return {
                "at": at_ms,
                "publisher": "northstar",
                "token_hash": hashlib.sha256(token.encode("utf-8")).hexdigest(),
                "expires_at": expires_at_ms,
                "used_at": used_at_ms,
            }

        rows = [
            row(tokens["valid"], now_ms - 60_000, now_ms + 72 * 3_600_000, None),
            row(tokens["used"], now_ms - 3_600_000, now_ms + 3_600_000, now_ms - 1_800_000),
            row(tokens["expired"], now_ms - 10 * 86_400_000, now_ms - 86_400_000, None),
        ]
        with open(os.path.join(work, "signin.jsonl"), "w", encoding="utf-8") as fh:
            for entry in rows:
                fh.write(json.dumps(entry) + "\n")
        self.api["signin"] = tokens
        self.check(
            "boot|signin-seed",
            True,
            "signin.jsonl seeded (hash-only rows): one valid, one used, one expired link",
        )

    def seed_billing_stores(self, work):
        """The billing seams the fleet runner and the console share:
        - the vaulted AdMob connection bundle plus its validated meta record
          (exactly the shape apex-autopilot's own fleet tests provision);
        - the customer billing sidecars (`<root>/<publisher>.billing.json`)
          the monthly pass and the watch bill load;
        - the recorded managed plan state the console's cancel action needs;
        - the seller billing profile and the revenue-band snapshot the
          monitoring tier is chosen from."""
        credentials = os.path.join(work, "credentials")
        os.makedirs(credentials, exist_ok=True)
        now_ms = int(time.time() * 1000)
        account = {
            # A real public HTTPS site whose `/app-ads.txt` is a 404: the live
            # audit records an honest absent file instead of failing on a
            # private address (the app-ads guard refuses loopback names).
            "developer_site": API_LIVE_DEVELOPER_SITE,
            "expected_networks": [],
            "ad_unit_ids": [],
            "bidder_accounts": [],
            "platforms": {
                "admob": {
                    "api_key": "dogfood-live-admob-key",
                    "account_id": "pub-1234567890",
                    "base_url": self.api["stub"].base_url(),
                    "report_key": None,
                    "reporting_base_url": None,
                    "refresh_token": None,
                }
            },
        }
        with open(os.path.join(credentials, "northstar.blob"), "w", encoding="utf-8") as fh:
            fh.write(vault_encrypt(API_VAULT_KEY_HEX, json.dumps(account)))
        meta = {
            "platforms": {
                "admob": {
                    "state": "validated",
                    "validated_at_unix_ms": now_ms,
                    "reporting": True,
                    "credential_fields": ["account_id", "api_key"],
                    "failure_class": None,
                }
            }
        }
        with open(
            os.path.join(credentials, "northstar.connections.json"), "w", encoding="utf-8"
        ) as fh:
            json.dump(meta, fh)
        customer = {
            "legalName": "Northstar Studio Oy",
            "addressLines": ["Rantatie 4"],
            "country": "FI",
        }
        with open(
            os.path.join(credentials, "northstar.billing.json"), "w", encoding="utf-8"
        ) as fh:
            json.dump(customer, fh)
        managed = {
            "plan": "managed",
            "active": True,
            "since": "2026-10-01",
            "monthlyFeeMicros": 2_500 * 1_000_000,
            "currency": "USD",
            "revenue30d": 60_000 * 1_000_000,
        }
        with open(
            os.path.join(credentials, "northstar.managed.json"), "w", encoding="utf-8"
        ) as fh:
            json.dump(managed, fh)
        seller = {
            "legalName": "Apex Ops Oy",
            "addressLines": ["Testikatu 1"],
            "country": "FI",
            "vatNumber": "FI12345678",
            "vatRateBp": SELLER_VAT_RATE_BP,
        }
        seller_path = os.path.join(self.workdir, "seller.json")
        with open(seller_path, "w", encoding="utf-8") as fh:
            json.dump(seller, fh)
        # The band snapshot: a valid, empty AdMob snapshot whose only signal is
        # the reported 30-day revenue the tier is chosen from. Empty because
        # the cycle's proposal pipeline must stay out of the way: the tier
        # activation is the seam under test, not the audit.
        band_snapshot = {
            "platform": "admob",
            "fetchedAtUnixMs": now_ms,
            "adUnits": [],
            "waterfall": [],
            "mediationGroups": [],
            "bidderAccounts": [],
            "countryRevenue": [],
            "appAdsTxt": {"present": False, "lines": [], "expectedNetworks": []},
            "platformTotals30d": {
                "revenue30d": MONITORING_BAND_REVENUE_30D_MICROS,
                "impressions30d": 0,
            },
            "mediationTotals30d": {
                "revenue30d": MONITORING_BAND_REVENUE_30D_MICROS,
                "impressions30d": 0,
            },
        }
        snapshot_dir = os.path.join(self.workdir, "billing")
        os.makedirs(snapshot_dir, exist_ok=True)
        snapshot_path = os.path.join(snapshot_dir, "band-snapshot.json")
        with open(snapshot_path, "w", encoding="utf-8") as fh:
            json.dump(band_snapshot, fh)
        # The fleet cycle now consumes each publisher's own
        # `<snapshot_root>/<publisher>/<wire>.json` (its `--snapshot`
        # arguments are replaced), so the runner root holds the committed
        # AdMob fixture with the band's reported 30-day revenue: the cycle
        # still has real proposals to hold AND the monitoring tier is chosen
        # from the seeded $350k band.
        runner_snapshots = os.path.join(self.workdir, "runner-snapshots")
        shutil.rmtree(runner_snapshots, ignore_errors=True)
        runner_publisher = os.path.join(runner_snapshots, API_PUBLISHER)
        os.makedirs(runner_publisher)
        with open(self.repo_path("rust/crates/apex-api/fixtures/admob.json"), "r", encoding="utf-8") as fh:
            runner_snapshot = json.load(fh)
        runner_snapshot["platformTotals30d"] = {
            "revenue30d": MONITORING_BAND_REVENUE_30D_MICROS,
            "impressions30d": runner_snapshot.get("platformTotals30d", {}).get("impressions30d", 0),
        }
        runner_snapshot["mediationTotals30d"] = {
            "revenue30d": MONITORING_BAND_REVENUE_30D_MICROS,
            "impressions30d": runner_snapshot.get("mediationTotals30d", {}).get("impressions30d", 0),
        }
        with open(os.path.join(runner_publisher, "admob.json"), "w", encoding="utf-8") as fh:
            json.dump(runner_snapshot, fh)
        self.fleet = {
            "credentials": credentials,
            "ledgers": os.path.join(work, "ledgers"),
            "seller": seller_path,
            "snapshot": snapshot_path,
            "snapshots": runner_snapshots,
            "runner_data": os.path.join(self.workdir, "runner"),
            "jobs_dir": os.path.join(self.workdir, "runner", "jobs"),
        }
        self.check(
            "boot|billing-seed",
            True,
            "vaulted AdMob bundle + validated meta, customer billing sidecars, managed plan state, "
            "seller profile and the $350k band snapshot seeded",
        )

    def api_env(self, work, port):
        return {
            "APEX_API_BIND": "127.0.0.1:%d" % port,
            # The emailed one-time link is absolute: the primary instance
            # names its own loopback origin, and the console Help link is the
            # configured absolute destination (the marketing pages keep the
            # relative on-site /support/).
            "APEX_CONSOLE_URL": "http://127.0.0.1:%d" % port,
            "APEX_SUPPORT_URL": API_SUPPORT_URL,
            "APEX_SNAPSHOT_ROOT": os.path.join(work, "snapshots"),
            "APEX_LEDGER_ROOT": os.path.join(work, "ledgers"),
            "APEX_LEDGER_KEYS": "northstar=%s,%s=%s,%s=%s"
            % (
                NORTHSTAR_PUBKEY_HEX,
                RIVAL_PUBLISHER,
                NORTHSTAR_PUBKEY_HEX,
                PAGINATION_PUBLISHER,
                NORTHSTAR_PUBKEY_HEX,
            ),
            "APEX_LEDGER_TOKENS_FILE": os.path.join(work, "tokens.json"),
            "APEX_LEADS_FILE": os.path.join(work, "leads.jsonl"),
            "APEX_LEAD_ID_SALT": "dogfood" * 8,
            "APEX_PROPOSALS_ROOT": os.path.join(work, "proposals"),
            "APEX_CONSOLE_PUBLISHER": API_PUBLISHER,
            "APEX_CONSOLE_PUBLISHER_NAME": "Northstar Studio",
            "APEX_CONSOLE_CURRENCY": "USD",
            # The transactional sink, pointed at the harness's stub sendmail so
            # the signin-ownership probe can read where a link was delivered.
            "APEX_NOTIFY_SINK": "email",
            "APEX_NOTIFY_FROM": "ops@dogfood.example",
            "APEX_NOTIFY_SENDMAIL": self.api["sendmail_stub"],
            "APEX_DOGFOOD_MAIL_OUTBOX": self.api["mail_outbox"],
            "APEX_CREDENTIAL_ROOT": os.path.join(work, "credentials"),
            "APEX_VAULT_KEY_HEX": API_VAULT_KEY_HEX,
            "APEX_LIVE_AUDIT_TOKEN": API_LIVE_AUDIT_TOKEN,
            "APEX_OPERATOR_JOBS_TOKEN": API_OPERATOR_JOBS_TOKEN,
            "APEX_OPERATOR_JOBS_DIR": os.path.join(work, "jobs"),
            "APEX_WATCH_ROOT": os.path.join(work, "watch"),
            "APEX_ADYEN_HMAC_KEY": API_ADYEN_HMAC_KEY_HEX,
            # The checkout client the setup-watch sign-up records a link
            # through (the harness stub answers with a stable link).
            "APEX_ADYEN_API_KEY": "dogfood-live-adyen-key",
            "APEX_ADYEN_MERCHANT_ACCOUNT": API_ADYEN_MERCHANT_ACCOUNT,
            "APEX_ADYEN_CHECKOUT_BASE": self.api["stub"].base_url(),
            "APEX_COLLECTION_FILE": os.path.join(work, "collection.jsonl"),
            "APEX_CLICKHOUSE_HTTP": self.ch_url,
            "APEX_CLICKHOUSE_DB": self.database,
            "APEX_ANALYTICS_ENV": "production",
            "APEX_METRICS_TEXTFILE": os.path.join(self.workdir, "api.prom"),
            "APEX_BILLING_VAT_RATE_BP": "2000",
        }

    def start_api(self):
        work = self.api["work"]
        self.start_process(
            "apex-api", [self.bin_path("apex-api")], self.api_env(work, self.ports["apex-api"]), "apex-api.log"
        )
        self.wait_http(self.base_url("apex-api") + "/healthz", lambda status: status == 200)
        return self.procs["apex-api"]

    def start_api_unconfigured(self):
        """The not-configured deployment: no ledger tokens, no leads store, no
        live-audit or operator-jobs token. The documented 503 panels and the
        machine routes' 503 refusals are asserted against this instance."""
        self.start_process(
            "apex-api-unconfigured",
            [self.bin_path("apex-api")],
            {
                "APEX_API_BIND": "127.0.0.1:%d" % self.ports["apex-api-unconfigured"],
                "APEX_CONSOLE_PUBLISHER": "northstar",
                "APEX_METRICS_TEXTFILE": os.path.join(self.workdir, "api-unconfigured.prom"),
            },
            "apex-api-unconfigured.log",
        )
        self.wait_http(
            self.base_url("apex-api-unconfigured") + "/healthz", lambda status: status == 200
        )
        return self.procs["apex-api-unconfigured"]

    def prepare_links_off_files(self):
        """The links-disabled deployment: full lead, workspace and email
        wiring, with only `APEX_CONSOLE_URL` absent. It owns its mutable state
        (leads, token file, ownership sidecar, sign-in log, outbox) so its
        zero-row and zero-delivery assertions cannot be confused with the
        primary instance's traffic."""
        work = os.path.join(self.workdir, "links-off")
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(work)
        with open(os.path.join(work, "leads.jsonl"), "w", encoding="utf-8"):
            pass
        with open(os.path.join(work, "tokens.json"), "w", encoding="utf-8") as fh:
            # Both the current domain key and the retired studio-slug key are
            # present, so this probe also exercises the same deployment shape
            # against an older binary during the pre-change proof.
            json.dump(
                {LINKS_OFF_WORKSPACE: API_LEDGER_TOKEN, API_PUBLISHER: API_LEDGER_TOKEN}, fh
            )
        with open(
            os.path.join(work, "tokens.json.workspaces.json"), "w", encoding="utf-8"
        ) as fh:
            json.dump(
                {
                    LINKS_OFF_WORKSPACE: {
                        "ownerEmail": LINKS_OFF_CONTACT,
                        "contacts": [LINKS_OFF_CONTACT],
                    },
                    API_PUBLISHER: {
                        "ownerEmail": LINKS_OFF_CONTACT,
                        "contacts": [LINKS_OFF_CONTACT],
                    },
                },
                fh,
            )
        outbox = os.path.join(work, "mail.outbox")
        with open(outbox, "w", encoding="utf-8"):
            pass
        sendmail = os.path.join(work, "sendmail-stub.sh")
        with open(sendmail, "w", encoding="utf-8") as fh:
            fh.write(
                "#!/bin/sh\n"
                'cat >> "%s"\n'
                'printf \'\\n===DOGFOOD-MESSAGE-END===\\n\' >> "%s"\n'
                "exit 0\n" % (outbox, outbox)
            )
        os.chmod(sendmail, 0o755)
        return work

    def links_off_env(self, work, port):
        return {
            "APEX_API_BIND": "127.0.0.1:%d" % port,
            "APEX_SNAPSHOT_ROOT": os.path.join(self.api["work"], "snapshots"),
            "APEX_LEDGER_ROOT": os.path.join(self.api["work"], "ledgers"),
            "APEX_LEDGER_KEYS": "northstar=%s,%s=%s,%s=%s"
            % (
                NORTHSTAR_PUBKEY_HEX,
                RIVAL_PUBLISHER,
                NORTHSTAR_PUBKEY_HEX,
                PAGINATION_PUBLISHER,
                NORTHSTAR_PUBKEY_HEX,
            ),
            "APEX_LEDGER_TOKENS_FILE": os.path.join(work, "tokens.json"),
            "APEX_LEADS_FILE": os.path.join(work, "leads.jsonl"),
            "APEX_LEAD_ID_SALT": "dogfood-links-off" * 4,
            "APEX_PROPOSALS_ROOT": os.path.join(self.api["work"], "proposals"),
            "APEX_CONSOLE_PUBLISHER": API_PUBLISHER,
            "APEX_CONSOLE_PUBLISHER_NAME": "Northstar Studio",
            "APEX_CONSOLE_CURRENCY": "USD",
            "APEX_NOTIFY_SINK": "email",
            "APEX_NOTIFY_FROM": "ops@dogfood.example",
            "APEX_NOTIFY_SENDMAIL": os.path.join(work, "sendmail-stub.sh"),
            "APEX_DOGFOOD_MAIL_OUTBOX": os.path.join(work, "mail.outbox"),
            "APEX_WATCH_ROOT": os.path.join(work, "watch"),
            "APEX_CLICKHOUSE_HTTP": self.ch_url,
            "APEX_CLICKHOUSE_DB": self.database,
            "APEX_ANALYTICS_ENV": "production",
            "APEX_METRICS_TEXTFILE": os.path.join(self.workdir, "api-links-off.prom"),
            # Blank is the documented unset form: it overrides any inherited
            # value so this instance is always links-disabled, and the Help
            # destination stays the documented default.
            "APEX_CONSOLE_URL": "",
            "APEX_SUPPORT_URL": "",
        }

    def start_api_links_off(self):
        work = self.prepare_links_off_files()
        self.api["links_off_work"] = work
        self.api["links_off_outbox"] = os.path.join(work, "mail.outbox")
        self.api["links_off_signin"] = os.path.join(work, "signin.jsonl")
        self.api["links_off_log"] = os.path.join(self.workdir, "apex-api-links-off.log")
        try:
            os.remove(self.api["links_off_log"])
        except FileNotFoundError:
            pass
        self.start_process(
            "apex-api-links-off",
            [self.bin_path("apex-api")],
            self.links_off_env(work, self.ports["apex-api-links-off"]),
            "apex-api-links-off.log",
        )
        self.wait_http(
            self.base_url("apex-api-links-off") + "/healthz", lambda status: status == 200
        )
        return self.procs["apex-api-links-off"]

    def hash_password(self, password):
        proc = self.spawn(
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
            "APEX_CONTROL_TOTP_STATE": os.path.join(work, "totp-steps.json"),
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
        return self.spawn(
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
                    "probe@probe-hostile.example",
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
        # A TOTP code is single-use per step (the control plane refuses a
        # replay). The harness logs in many times in one run, so it waits for
        # the next step instead of reusing the current one.
        last_step = self.control.get("last_code_step")
        if last_step is not None:
            step = int(time.time() // 30)
            if step <= last_step:
                time.sleep((last_step + 1) * 30 - time.time() + 0.5)
        resp = http_req(
            "POST",
            self.base_url("apex-control") + "/control/login",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("operator", operator), ("password", CONTROL_OPERATOR_PASSWORD)]),
        )
        if resp.status != 200:
            return resp, None
        match = PENDING_INPUT_PATTERN.search(resp.text)
        if not match:
            return resp, None
        pending = match.group(1)
        resp = http_req(
            "POST",
            self.base_url("apex-control") + "/control/login/code",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("pending", pending), ("code", self.totp_code(CONTROL_TOTP_SECRET_B32))]),
        )
        if resp.status == 303:
            self.control["last_code_step"] = int(time.time() // 30)
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
        """The selected cells of one probe class, in sorted order, from the
        class index built once per run. Under a relevance selection only the
        selected cells are returned, so a probe asserts exactly the subset. A
        harness stand-in without a `selected` set (the CI bookkeeping check)
        gets the unfiltered class."""
        cells = self._cell_index().get(cls)
        counters = getattr(self, "counters", None)
        if counters is not None:
            counters["cells_of_calls"] += 1
        if not cells:
            return []
        selected = getattr(self, "selected", None)
        if selected is not None:
            cells = [entry for entry in cells if entry[2] in selected]
        if service is None:
            return cells
        return [entry for entry in cells if entry[0] == service]

    def _cell_index(self):
        if self._cell_index_cache is None:
            counters = getattr(self, "counters", None)
            if counters is not None:
                counters["cell_index_builds"] += 1
            index = {}
            for cell_id in sorted(self.required):
                svc, surface, cell_cls = cell_id.split("|", 2)
                index.setdefault(cell_cls, []).append((svc, surface, cell_id))
            self._cell_index_cache = index
        return self._cell_index_cache

    def run_all_probes(self):
        for cls in PROBE_ORDER:
            if not self.cells_of(cls):
                # Filtered run: nothing in this class is selected, so the
                # probe (and its side work) is skipped entirely.
                continue
            probe = getattr(self, "probe_" + cls.replace("-", "_"))
            started = time.monotonic()
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
            finally:
                self.class_timings[cls] = self.class_timings.get(cls, 0.0) + (
                    time.monotonic() - started
                )

    # 1 ------------------------------------------------------ surface-parity
    def probe_surface_parity(self):
        argv = [self.bin_path("apex-tools"), "dogfood-plan"]
        counters = getattr(self, "counters", None)
        if counters is not None:
            counters["dogfood_plan_spawns"] += 1
        if self.filtered:
            for change in self.changed_paths:
                argv += ["--changed", change]
        proc = self.spawn(
            argv,
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
                " ".join(argv),
            )

    # 2 --------------------------------------------------- response-contract
    def response_contract_expectations(self, service):
        """(anonymous statuses, with-session statuses or None, html check).

        For every route declared `session_required` the anonymous answer must
        be the recorded gate (303 to /console/ledger, asserted in the probe);
        the with-session status is the product's documented answer for the
        harness seed (200 for the pages the seed can render, 404 for the
        routes whose identifiers deliberately do not exist, 400 for the CSV
        without its platform parameter). The table is built once per service
        per run; the per-cell loop used to rebuild it for every cell."""
        table = self._contract_expectations.get(service)
        if table is None:
            table = self._response_contract_table(service)
            self._contract_expectations[service] = table
        return table

    def _response_contract_table(self, service):
        if service == "apex-api":
            return {
                "/": ({303}, {200}, True),
                "/console": ({303}, None, False),
                "/console/": ({303}, None, False),
                "/console/audit": ({303}, {200}, True),
                "/console/audit/export.csv": ({303}, {400}, False),
                "/console/audit/findings/{id}": ({303}, {200}, True),
                "/console/ledger": ({200}, None, True),
                "/console/ledger/records/{seq}": ({303}, {200}, True),
                "/console/signin": ({400}, None, False),
                "/console/connections": ({303}, {200}, True),
                "/console/connections/{platform}": ({303}, {200}, True),
                "/console/proposals": ({303}, {200}, True),
                "/console/proposals/{id}": ({303}, {404}, True),
                "/console/experiments": ({303}, {200}, True),
                "/console/experiments/{id}": ({303}, {404}, True),
                "/console/experiments/{id}/incident": ({303}, {404}, True),
                "/console/billing": ({303}, {200}, True),
                "/console/billing/invoices/{period}": ({303}, {404}, True),
                "/console/settings": ({303}, {200}, True),
                "/console/settings/access": ({303}, {200}, True),
                "/console/settings/policy": ({303}, {200}, True),
                "/console/settings/monitoring": ({303}, {200}, True),
                "/console/settings/billing": ({303}, {200}, True),
                "/console/analytics": ({303}, {200}, True),
                "/free-audit": ({303}, None, False),
                "/free-audit/": ({200}, None, True),
                "/audit-requested/": ({200}, None, True),
                "/watch": ({303}, None, False),
                "/watch/": ({200}, None, True),
                "/watch/requested": ({404}, None, True),
                "/watch/confirm": ({400}, None, False),
                "/watch/billing-profile/recorded": ({404}, None, True),
                "/healthz": ({200}, None, False),
                "/favicon.ico": ({200}, None, False),
                "/favicon.svg": ({200}, None, False),
                "/assets/fonts/{name}": ({200}, None, False),
                "/metrics": ({200}, None, False),
                "/v1/audit/{platform}": ({200}, None, False),
                "/v1/ledger/{publisher}": ({401}, None, False),
                "/v1/operator/jobs/{job_id}": ({401}, None, False),
            }
        if service == "apex-control":
            return {
                "/": ({303}, None, False),
                "/control": ({401}, None, True),
                "/control/": ({401}, None, True),
                "/control/customers": ({401}, None, True),
                "/control/customers/{id}": ({401}, None, True),
                "/control/ingestion": ({401}, None, True),
                "/control/audits": ({401}, None, True),
                "/control/experiments": ({401}, None, True),
                "/control/experiments/{id}": ({401}, None, True),
                "/control/incidents": ({401}, None, True),
                "/control/incidents/{id}": ({401}, None, True),
                "/control/finance": ({401}, None, True),
                "/control/finance/invoices/{id}": ({401}, None, True),
                "/control/finance/exports/{register}": ({401}, None, True),
                "/control/growth": ({401}, None, True),
                "/control/system": ({401}, None, True),
                "/control/system/policies": ({401}, None, True),
                "/control/access": ({401}, None, True),
                "/favicon.ico": ({200}, None, False),
                "/favicon.svg": ({200}, None, False),
                "/assets/fonts/{name}": ({200}, None, False),
                "/control/login": ({200}, None, True),
                "/control/login/code": ({200}, None, True),
                "/control/actions/{kind}/result/{id}": ({401}, None, True),
            }
        return {
            "/": ({401}, None, True),
            "/leads": ({401}, None, True),
            "/leads/export.csv": ({401}, None, False),
            "/publishers/{domain}": ({401}, None, True),
            "/plan": ({401}, None, True),
            "/sends": ({401}, None, True),
            "/sends/preview/{file}": ({401}, None, True),
            "/replies": ({401}, None, True),
            "/domains": ({401}, None, True),
            "/models": ({401}, None, True),
            "/healthz": ({200}, None, False),
            "/assets/apex.css": ({200}, None, False),
            "/assets/fonts/{name}": ({200}, None, False),
            "/favicon.ico": ({200}, None, False),
            "/favicon.svg": ({200}, None, False),
            "/login": ({200}, None, True),
            "/actions/result/{id}": ({401}, None, True),
        }

    def finding_id(self):
        """A real finding id from the seeded hostile report, so the session
        probe of `/console/audit/findings/{id}` renders a real page. The id
        lives on the report page's finding links; no id means the probe must
        fail loudly, not skip."""
        self.api_session()
        try:
            resp = http_req(
                "GET",
                self.base_url("apex-api") + "/console/audit?platform=admob",
                headers=self.cookie_header("apex-api"),
                timeout=10,
            )
        except ReqError:
            return None
        match = FINDING_LINK_PATTERN.search(resp.text)
        return match.group(1) if match else None

    def probe_response_contract(self):
        """Every response-contract cell, both directions for the recorded
        session gate: the anonymous request must answer the documented status
        (and, for `session_required` routes, the exact `303 ->
        /console/ledger`), and the with-session request must answer the
        product's documented page status."""
        session = self.api_session()
        session = session[0] if session else None
        finding_id = None
        cells = self.cells_of("response-contract")
        expectations = {
            service: self.response_contract_expectations(service)
            for service, _surface, _cell_id in cells
        }
        for service, _surface, cell_id in cells:
            surface = cell_id.split("|")[1]
            method, template = surface.split(" ", 1)
            if service == "apex-api" and template == "/watch/confirm":
                problems = []
                refused_copy = "This confirmation link is no longer valid."
                url = self.base_url(service) + template
                missing = http_req("GET", url)
                if missing.status != 400 or refused_copy not in missing.text:
                    problems.append(
                        "missing token -> %d (expected the 400 refusal page)" % missing.status
                    )
                garbage = http_req("GET", url + "?token=not-a-confirmation-token")
                if garbage.status != 400 or refused_copy not in garbage.text:
                    problems.append(
                        "garbage token -> %d (expected the 400 refusal page)" % garbage.status
                    )
                self.clear_mail_outbox()
                signup = http_req(
                    "POST",
                    self.base_url(service) + "/watch",
                    headers={"Content-Type": "application/x-www-form-urlencoded"},
                    body=form_body(
                        [
                            ("domain", "confirm-probe.example"),
                            ("email", "confirm@confirm-probe.example"),
                            ("accounts", "admob.com = pub-confirm-probe"),
                        ]
                    ),
                )
                token = None
                if signup.status == 303:
                    tokens = re.findall(
                        r"/watch/confirm\?token=([0-9a-f]{32})", self.read_mail_outbox()
                    )
                    token = tokens[-1] if tokens else None
                if token is None:
                    problems.append(
                        "the sign-up did not send a confirmation token (HTTP %d)" % signup.status
                    )
                    first = second = confirmed = spent = None
                else:
                    # GET is inert: a mail scanner may open the link any
                    # number of times without consuming the token. Only the
                    # recipient's POST confirms, and a second POST refuses.
                    first = http_req("GET", url + "?token=" + token)
                    second = http_req("GET", url + "?token=" + token)
                    if first.status != 200 or "Confirm your watch email." not in first.text:
                        problems.append(
                            "a fresh token GET -> %d (expected the 200 confirm button)" % first.status
                        )
                    if second.status != 200 or "Confirm your watch email." not in second.text:
                        problems.append(
                            "a second GET -> %d (a scanner must not consume the token)" % second.status
                        )
                    confirmed = http_req(
                        "POST",
                        url,
                        headers={"Content-Type": "application/x-www-form-urlencoded"},
                        body=form_body([("token", token)]),
                    )
                    if confirmed.status != 200 or "Confirm your billing details." not in confirmed.text:
                        problems.append(
                            "the recipient POST -> %d (expected the 200 billing-profile form)"
                            % confirmed.status
                        )
                    spent = http_req(
                        "POST",
                        url,
                        headers={"Content-Type": "application/x-www-form-urlencoded"},
                        body=form_body([("token", token)]),
                    )
                    if spent.status != 200 or refused_copy not in spent.text:
                        problems.append(
                            "a spent token POST -> %d (expected the 200 refusal page)" % spent.status
                        )
                detail = (
                    "missing=%d garbage=%d signup=%d fresh_get=%s second_get=%s post=%s spent=%s"
                    % (
                        missing.status,
                        garbage.status,
                        signup.status,
                        first.status if first else "n/a",
                        second.status if second else "n/a",
                        confirmed.status if confirmed else "n/a",
                        spent.status if spent else "n/a",
                    )
                )
                if problems:
                    detail += "; " + "; ".join(problems)
                self.cell(
                    cell_id,
                    not problems,
                    detail,
                    "GET/POST /watch/confirm (missing, garbage, inert GET, consuming POST, spent)",
                )
                continue
            if service == "apex-api" and template == "/v1/audit/{platform}":
                problems = []
                path = materialize(template)
                anonymous = http_req("GET", self.base_url(service) + path)
                if anonymous.status != 401:
                    problems.append(
                        "anonymous HTTP %d (expected 401: the bearer gate is first)" % anonymous.status
                    )
                authorized = http_req(
                    "GET",
                    self.base_url(service) + path + "?publisher=" + API_PUBLISHER,
                    headers={"Authorization": "Bearer " + API_LIVE_AUDIT_TOKEN},
                )
                if authorized.status != 200:
                    problems.append(
                        "bearer + ?publisher=%s -> %d (expected 200)" % (API_PUBLISHER, authorized.status)
                    )
                elif NORTHSTAR_AUDIT_MARKER not in authorized.text:
                    problems.append("the report does not carry the named publisher's own data")
                elif RIVAL_AUDIT_MARKER in authorized.text:
                    problems.append("the report carries another publisher's data")
                detail = "anonymous=%d bearer+publisher=%d" % (
                    anonymous.status,
                    authorized.status,
                )
                if problems:
                    detail += "; " + "; ".join(problems)
                self.cell(
                    cell_id,
                    not problems,
                    detail,
                    "GET /v1/audit/admob (anonymous; then bearer + ?publisher=%s)" % API_PUBLISHER,
                )
                continue
            expect = expectations[service].get(template)
            if expect is None:
                self.cell(cell_id, False, "no response-contract expectation for %s" % template)
                continue
            anonymous_statuses, session_statuses, html = expect
            path = materialize(template)
            if template == "/console/audit/findings/{id}" and self.session_required:
                if finding_id is None:
                    finding_id = self.finding_id() or ""
                if finding_id:
                    path = path.replace("/console/audit/findings/nope", "/console/audit/findings/" + finding_id)
            url = self.base_url(service) + path
            problems = []
            try:
                anonymous = http_req(method, url)
            except ReqError as error:
                self.cell(cell_id, False, str(error))
                continue
            if anonymous.status not in anonymous_statuses:
                problems.append("anonymous HTTP %d not in %s" % (anonymous.status, sorted(anonymous_statuses)))
            gate = ("%s|%s" % (service, surface)) in self.session_required
            if gate:
                # The recorded expectation: without a live session the console
                # answers exactly 303 to the access panel.
                if anonymous.status != 303:
                    problems.append("session gate answered HTTP %d, not 303" % anonymous.status)
                location = anonymous.header("location")
                if location != "/console/ledger":
                    problems.append("session gate Location %r, not /console/ledger" % location)
            detail = "GET %s -> anonymous HTTP %d" % (url, anonymous.status)
            final = anonymous
            if session_statuses is not None:
                if session is None:
                    problems.append("could not open a console session")
                else:
                    headers = {"Cookie": "apex_ledger_session=%s" % session}
                    try:
                        final = http_req(method, url, headers=headers)
                    except ReqError as error:
                        problems.append("with session: %s" % error)
                        final = anonymous
                    else:
                        if final.status not in session_statuses:
                            problems.append(
                                "with session HTTP %d not in %s" % (final.status, sorted(session_statuses))
                            )
                        detail += "; with session HTTP %d" % final.status
            if html and 200 <= final.status < 400 and final.content_type() == "text/html":
                body = final.text
                h1 = len(H1_TAG_PATTERN.findall(body))
                if h1 != 1:
                    problems.append("expected exactly one h1, found %d" % h1)
                if "<title" not in body:
                    problems.append("missing <title>")
                # A valid receipt carries the pinned confirmation copy; an
                # unknown reference is an honest 404, never a success page.
                if template == "/watch/requested":
                    if final.status == 404 and "This link is not valid or has expired." not in body:
                        problems.append("the invalid-reference page is not the honest 404 copy")
                    elif final.status == 200 and "Your watch is requested." not in body:
                        problems.append("the confirmation page does not carry its pinned copy")
            if problems:
                detail += "; " + "; ".join(problems)
            self.cell(cell_id, not problems, detail, "GET %s (anonymous%s)" % (url, " + session" if session_statuses else ""))
        # The explicit documented exception list: these console paths stay
        # public, so none may ever be declared session_required (their public
        # anonymous statuses are their own response-contract cells above).
        exceptions = [
            "/console/ledger",
            "/console",
            "/console/",
            "/console/signin",
            "/free-audit",
            "/free-audit/",
            "/audit-requested/",
            "/watch",
            "/watch/",
            "/watch/requested",
        ]
        leaked = sorted(
            surface
            for surface in ("apex-api|GET " + p for p in exceptions)
            if surface in self.session_required
        )
        self.check(
            "response-contract|public-exceptions",
            not leaked,
            "documented public console paths stay public; none is session_required"
            if not leaked
            else "public paths marked session_required: %s" % leaked,
        )

    # 3 ---------------------------------------------------------- no-inline
    def no_inline_targets(self):
        if self._no_inline_targets is None:
            self._no_inline_targets = self._build_no_inline_targets()
        return self._no_inline_targets

    def _build_no_inline_targets(self):
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
                ("GET /console/settings/billing", "/console/settings/billing"),
                ("GET /console/analytics", "/console/analytics"),
                ("GET /console/audit", "/console/audit"),
                ("GET /", "/"),
                ("GET /free-audit/", "/free-audit/"),
                ("GET /audit-requested/", "/audit-requested/"),
                ("GET /watch/confirm", "/watch/confirm?token=not-a-confirmation-token"),
                (
                    "GET /watch/billing-profile/recorded",
                    "/watch/billing-profile/recorded?ref=sub_0123456789",
                ),
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
        targets = {
            (service, template): path
            for service, pairs in self.no_inline_targets().items()
            for template, path in pairs
        }
        for service, _surface, cell_id in self.cells_of("no-inline"):
            surface = cell_id.split("|")[1]
            target = targets.get((service, surface))
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
                # Every surface now links one external stylesheet so the
                # edge's style-src 'self' policy holds; no page embeds CSS.
                if "<style" in body:
                    problems.append("contains a <style block (pages link /assets/apex.css)")
                links = body.count('rel="stylesheet"')
                if links != 1:
                    problems.append("expected exactly one stylesheet link, found %d" % links)
                if "/assets/apex.css" not in body:
                    problems.append("the stylesheet link does not point at /assets/apex.css")
            detail = "GET %s -> HTTP %d" % (url, resp.status)
            if problems:
                detail += "; " + "; ".join(problems)
            self.cell(cell_id, not problems, detail, "GET %s" % url)

    # 4 ------------------------------------------------------- assets-fonts
    def fetch_css_fixture(self, service):
        # All three surfaces serve the same external stylesheet.
        resp = http_req("GET", self.base_url(service) + "/assets/apex.css")
        if resp.status != 200 or "text/css" not in resp.header("content-type").lower():
            return None
        return resp.body

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
        cookie_problems = console_session_cookie_problems(set_cookie, "ledger exchange")
        problems.extend(cookie_problems)
        if not cookie_problems:
            notes.append("ledger exchange cookie is Path=/; HttpOnly; SameSite=Lax; host-only")
        cookie = cookie_value(set_cookie, "apex_ledger_session")
        if not cookie:
            problems.append("ledger exchange did not set apex_ledger_session")
        else:
            jar = BrowserCookieJar()
            jar.store(set_cookie)
            if not jar.header("/"):
                problems.append(
                    "the ledger-minted cookie jar does not send the session to / (Path scope)"
                )
            if not jar.header("/console/ledger"):
                problems.append(
                    "the ledger-minted cookie jar does not send the session to /console/ledger"
                )
            overview = http_req("GET", base + "/", headers={"Cookie": jar.header("/")} if jar.header("/") else {})
            overview_issues = overview_problems(overview, "ledger-minted")
            problems.extend(overview_issues)
            if not overview_issues:
                notes.append("ledger-minted jar GET / -> overview 200")
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
        # POST /v1/cycle/{publisher}: the live-auditor bearer gates the write
        # cycle. Missing and wrong bearers are 401; a bearer without a
        # platform query is the documented 400 behind the gate, proving the
        # authority check runs first without masking the validation.
        cycle_anon = http_req("POST", base + "/v1/cycle/northstar", headers={"Content-Type": "application/json"}, body=b"{}")
        if cycle_anon.status != 401:
            problems.append("cycle without bearer answered %d, expected 401" % cycle_anon.status)
        else:
            notes.append("cycle no bearer -> 401")
        cycle_wrong = http_req(
            "POST",
            base + "/v1/cycle/northstar",
            headers={"Content-Type": "application/json", "Authorization": "Bearer wrong-audit-token"},
            body=b"{}",
        )
        if cycle_wrong.status != 401:
            problems.append("cycle with wrong bearer answered %d, expected 401" % cycle_wrong.status)
        else:
            notes.append("cycle wrong bearer -> 401")
        cycle_ok = http_req(
            "POST",
            base + "/v1/cycle/northstar",
            headers={"Content-Type": "application/json", "Authorization": "Bearer " + API_LIVE_AUDIT_TOKEN},
            body=b"{}",
        )
        if cycle_ok.status != 400:
            problems.append("cycle with bearer answered %d, expected the 400 platform refusal" % cycle_ok.status)
        else:
            notes.append("cycle bearer -> 400 platform_required")
        # GET /console/signin: one-time links. No token is 400; used or
        # expired links are the typed 200 refusal; a valid link 303s and mints
        # the console session; the same link replayed is refused (single use).
        signin = self.api.get("signin") or {}
        bare = http_req("GET", base + "/console/signin")
        if bare.status != 400:
            problems.append("signin without token answered %d, expected 400" % bare.status)
        else:
            notes.append("signin without token -> 400")
        for label in ("used", "expired"):
            token = signin.get(label)
            resp = http_req("GET", base + "/console/signin?token=" + token)
            if resp.status != 200 or "Sign-in refused." not in resp.text:
                problems.append("signin %s link answered %d without the one-time refusal copy" % (label, resp.status))
            else:
                notes.append("signin %s -> 200 refusal" % label)
        valid_token = signin.get("valid", "")
        fresh = http_req("GET", base + "/console/signin?token=" + valid_token)
        fresh_cookie = cookie_value(fresh.header("set-cookie") or "", "apex_ledger_session")
        hidden = parse_hidden_fields(fresh.text)
        form_action = re.search(
            r'<form[^>]*action="/console/signin"', fresh.text, re.I
        )
        if fresh.status != 200 or fresh_cookie or not form_action or hidden.get("token") != valid_token:
            problems.append(
                "signin valid link answered %d (cookie %r, form=%s, hidden=%s), expected the inert 200 form"
                % (
                    fresh.status,
                    fresh_cookie,
                    bool(form_action),
                    hidden.get("token") == valid_token,
                )
            )
        else:
            notes.append("signin valid link -> inert 200 form")
        posted = http_req(
            "POST",
            base + "/console/signin",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("token", valid_token)]),
        )
        fresh_cookie = cookie_value(posted.header("set-cookie") or "", "apex_ledger_session")
        if posted.status != 303 or not fresh_cookie:
            problems.append(
                "signin POST answered %d (cookie %r), expected 303 + session"
                % (posted.status, fresh_cookie)
            )
        else:
            notes.append("signin POST -> 303 + session")
        replay = http_req(
            "POST",
            base + "/console/signin",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("token", valid_token)]),
        )
        if replay.status != 200 or "Sign-in refused." not in replay.text:
            problems.append("signin replay POST answered %d, expected the 200 refusal" % replay.status)
        else:
            notes.append("signin replay POST -> 200 refusal")
        if fresh_cookie:
            granted = http_req(
                "GET",
                base + "/console/audit?platform=admob",
                headers={"Cookie": "apex_ledger_session=" + fresh_cookie},
            )
            if granted.status != 200:
                problems.append("sign-in session did not grant the console (HTTP %d)" % granted.status)
            else:
                notes.append("sign-in session grants /console/audit (200)")
        # POST/GET /v1/operator/jobs: the bearer gates both; the unconfigured
        # deployment keeps the 503 refusal.
        jobs_anon = http_req(
            "POST", base + "/v1/operator/jobs", headers={"Content-Type": "application/json"}, body=b"{}"
        )
        if jobs_anon.status != 401:
            problems.append("operator jobs POST without bearer answered %d, expected 401" % jobs_anon.status)
        else:
            notes.append("operator jobs no bearer -> 401")
        jobs_wrong = http_req(
            "POST",
            base + "/v1/operator/jobs",
            headers={"Content-Type": "application/json", "Authorization": "Bearer wrong-jobs-token"},
            body=b"{}",
        )
        if jobs_wrong.status != 401:
            problems.append("operator jobs POST with wrong bearer answered %d, expected 401" % jobs_wrong.status)
        else:
            notes.append("operator jobs wrong bearer -> 401")
        jobs_show_anon = http_req("GET", base + "/v1/operator/jobs/opj_aaaaaaaaaaa1")
        if jobs_show_anon.status != 401:
            problems.append("operator jobs GET without bearer answered %d, expected 401" % jobs_show_anon.status)
        else:
            notes.append("operator jobs GET no bearer -> 401")
        unconfigured = self.base_url("apex-api-unconfigured")
        for label, method, path, body in (
            ("cycle", "POST", "/v1/cycle/northstar", b"{}"),
            ("operator jobs POST", "POST", "/v1/operator/jobs", b"{}"),
            ("operator jobs GET", "GET", "/v1/operator/jobs/opj_aaaaaaaaaaa1", None),
            ("signin", "GET", "/console/signin?token=" + signin.get("valid", ""), None),
        ):
            headers = {"Authorization": "Bearer " + (API_OPERATOR_JOBS_TOKEN if "jobs" in label else API_LIVE_AUDIT_TOKEN)}
            if method == "POST":
                headers["Content-Type"] = "application/json"
            resp = http_req(method, unconfigured + path, headers=headers, body=body)
            if resp.status != 503:
                problems.append("unconfigured api %s answered %d, expected 503" % (label, resp.status))
            else:
                notes.append("unconfigured api %s -> 503" % label)
        panel = http_req("GET", unconfigured + "/console/connections")
        if panel.status != 503 or "not available in this deployment" not in panel.text:
            problems.append("unconfigured console panel answered %d without the typed copy" % panel.status)
        else:
            notes.append("unconfigured console panel -> 503 typed copy")
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
        wrong_refused = (
            wrong.status == 401
            and "We could not verify those access details." in wrong.text
        ) or (wrong.status == 429 and "Too many attempts." in wrong.text)
        if not wrong_refused:
            problems.append(
                "wrong password answered %d, expected 401 (pinned copy) or 429 (lockout)"
                % wrong.status
            )
        else:
            notes.append(
                "wrong password -> %d (%s)"
                % (wrong.status, "pinned copy" if wrong.status == 401 else "lockout copy")
            )
        pending_resp = http_req(
            "POST",
            base + "/control/login",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("operator", "ops"), ("password", CONTROL_OPERATOR_PASSWORD)]),
        )
        match = PENDING_INPUT_PATTERN.search(pending_resp.text)
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
        elif wrong_code.status not in (401, 429):
            problems.append("wrong code answered %d, expected 401 or 429" % wrong_code.status)
        else:
            notes.append("wrong code -> %d" % wrong_code.status)
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
    def ledger_pagination_probe(self):
        """The paginated change log: one page for the 24-record fixture, stable
        links, and the typed safe 400 for every rejected page value.

        The fixture the harness seeds carries 24 verified records and the page
        size is 100, so the default and `page=1` render one page with no
        next/previous links and the "Showing 1-24 of 24 records" note. `page=2`
        is beyond the last page and must be the typed 400, as are non-numeric,
        zero, negative, exponent and overflow values. The error page never
        echoes the rejected value and links only the stable first page.
        """
        problems = []
        notes = []
        base = self.base_url("apex-api")
        exchange = http_req(
            "POST",
            base + "/console/ledger",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(
                [("publisher", PAGINATION_PUBLISHER), ("token", PAGINATION_TOKEN)]
            ),
        )
        session = cookie_value(exchange.header("set-cookie") or "", "apex_ledger_session")
        if exchange.status != 303 or not session:
            return (
                False,
                "could not mint the pristine-fixture session (HTTP %d, cookie=%s)"
                % (exchange.status, bool(session)),
            )
        cookie = {"Cookie": "apex_ledger_session=" + session}
        fixture_path = os.path.join(
            self.api["work"], "ledgers", PAGINATION_PUBLISHER + ".jsonl"
        )
        fixture_records = count_lines(fixture_path)
        if fixture_records != PAGINATION_LEDGER_RECORDS:
            problems.append(
                "the pristine fixture ledger carries %d records, expected %d"
                % (fixture_records, PAGINATION_LEDGER_RECORDS)
            )
        try:
            overview, total = self.overview_records(session)
            if overview.status != 200:
                problems.append("fixture overview -> %d (expected 200)" % overview.status)
            elif total != PAGINATION_LEDGER_RECORDS:
                problems.append(
                    "fixture overview verified-records KPI is %r, expected %d"
                    % (total, PAGINATION_LEDGER_RECORDS)
                )
            else:
                notes.append("fixture overview KPI still reports 24 verified records")
            first = http_req("GET", base + "/console/ledger", headers=cookie)
            if first.status != 200:
                problems.append("default page -> %d (expected 200)" % first.status)
            elif "Showing 1-24 of 24 records" not in first.text:
                problems.append("default page is missing the pinned 'Showing 1-24 of 24 records' note")
            elif 'rel="next"' in first.text or 'rel="prev"' in first.text:
                problems.append("the single-page fixture rendered a pagination link")
            else:
                notes.append("default page: Showing 1-24 of 24 records, no prev/next link")
            explicit = http_req("GET", base + "/console/ledger?page=1", headers=cookie)
            if explicit.status != 200:
                problems.append("page=1 -> %d (expected 200)" % explicit.status)
            elif "Showing 1-24 of 24 records" not in explicit.text:
                problems.append("page=1 is missing the pinned 'Showing 1-24 of 24 records' note")
            elif 'rel="next"' in explicit.text or 'rel="prev"' in explicit.text:
                problems.append("page=1 rendered a pagination link on the single-page fixture")
            else:
                notes.append("page=1 is the same stable single page")
            rejected = ("2", "abc", "0", "-1", "1e3", "9" * 40)
            for value in rejected:
                resp = http_req("GET", base + "/console/ledger?page=" + urllib.parse.quote(value), headers=cookie)
                where = "page=%s" % value
                if resp.status != 400:
                    problems.append("%s -> %d (expected the typed 400)" % (where, resp.status))
                    continue
                if "Check the request." not in resp.text:
                    problems.append("%s -> 400 without the typed error headline" % where)
                if "The page parameter must be a whole number from 1 to the last page" not in resp.text:
                    problems.append("%s -> 400 without the typed page copy" % where)
                if value != "1" and ("?page=" + value) in resp.text:
                    problems.append("%s -> the error page echoed the rejected value" % where)
                if resp.text.count('href="/console/ledger?page=') != 1 or (
                    'href="/console/ledger?page=1"' not in resp.text
                ):
                    problems.append("%s -> the error page does not link only the stable first page" % where)
                if "<script" in resp.text:
                    problems.append("%s -> the error page carries a script tag" % where)
            notes.append("typed 400 for page=2, abc, 0, -1, 1e3 and an overflowing page")
        except ReqError as error:
            problems.append("pagination probe: %s" % error)
        detail = "; ".join(problems + notes)
        return (not problems, detail)

    def probe_input_corpus(self):
        problems = {}
        notes = {}

        def note(service, ok, detail):
            entry = notes if ok else problems
            entry.setdefault(service, []).append(detail)

        base = self.base_url("apex-api")
        try:
            self.api_session()
            cookie = self.cookie_header("apex-api")
            resp = http_req("GET", base + "/console/audit?platform=%zz", headers=cookie)
            note("apex-api", resp.status == 400, "malformed percent in query -> %d (expected 400)" % resp.status)
            resp = http_req("GET", base + "/console/audit/findings/%00", headers=cookie)
            note("apex-api", resp.status in (400, 404), "%%00 path segment -> %d" % resp.status)
            # Auth runs before path decoding: anonymous is the gate's 401;
            # behind the bearer the malformed platform is the typed 400/404.
            resp = http_req("GET", base + "/v1/audit/%ff")
            note(
                "apex-api",
                resp.status == 401,
                "%%ff path segment anonymous -> %d (expected the auth-first 401)" % resp.status,
            )
            resp = http_req(
                "GET",
                base + "/v1/audit/%ff?publisher=" + API_PUBLISHER,
                headers={"Authorization": "Bearer " + API_LIVE_AUDIT_TOKEN},
            )
            note(
                "apex-api",
                resp.status in (400, 404),
                "%%ff path segment with bearer -> %d (expected 400/404 for the malformed platform)" % resp.status,
            )
            resp = http_req("GET", base + "/" + "a" * 8192)
            note("apex-api", resp.status < 500, "8KB path -> %d" % resp.status)
            flood = "&".join("p%d=%d" % (i, i) for i in range(500))
            resp = http_req("GET", base + "/console/analytics?" + flood, headers=cookie)
            note("apex-api", resp.status == 200, "500-param flood on /console/analytics -> %d" % resp.status)
            # The public setup-watch form: a malformed submission is the typed
            # 400 with its pinned field copy, never a stored subscriber.
            invalid_watch = http_req(
                "POST",
                base + "/watch",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=form_body([("domain", "not a domain"), ("email", "nope"), ("accounts", "===")]),
            )
            note(
                "apex-api",
                invalid_watch.status == 400
                and "Enter the domain where your app-ads.txt lives." in invalid_watch.text
                and "Enter an email address we can reply to." in invalid_watch.text
                and "<script" not in invalid_watch.text,
                "invalid watch form -> %d with the pinned field copy" % invalid_watch.status,
            )
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
            # Method confusion on every public surface: no method may 5xx.
            for method in ("OPTIONS", "TRACE", "PUT", "DELETE", "PATCH"):
                for path in ("/", "/console/audit", "/free-audit", "/watch", "/v1/leads", "/metrics"):
                    resp = http_req(method, base + path, headers=cookie)
                    note(
                        "apex-api",
                        resp.status < 500,
                        "%s %s -> %d (no 5xx)" % (method, path, resp.status),
                    )
            # Content-type confusion: JSON where a form is parsed and the
            # reverse are typed refusals, never a parser crash.
            json_to_form = http_req(
                "POST",
                base + "/free-audit",
                headers={"Content-Type": "application/json"},
                body=b'{"email":"x@studio.example"}',
            )
            note("apex-api", json_to_form.status in (400, 415, 303), "JSON body on a form route -> %d" % json_to_form.status)
            form_to_json = http_req(
                "POST",
                base + "/v1/leads",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=b"email=x@studio.example",
            )
            note("apex-api", form_to_json.status in (400, 415), "form body on a JSON route -> %d" % form_to_json.status)
            # Duplicate and conflicting parameters must not flip a decision
            # into a 5xx or an unauthenticated success.
            dup = http_req(
                "GET",
                base + "/console/audit?platform=admob&platform=levelplay&platform=",
                headers=cookie,
            )
            note("apex-api", dup.status < 500, "conflicting platform params -> %d" % dup.status)
            # Header abuse: a hostile Host, a spoofed forwarded chain and a
            # tampered cookie are answered, never crashed.
            for headers, label in (
                ({"Host": "evil.example"}, "Host override"),
                ({"X-Forwarded-For": "6.6.6.6, 203.0.113.9"}, "forwarded chain"),
                ({"X-Forwarded-Proto": "http, https"}, "forwarded proto chain"),
                ({"Cookie": "apex_ledger_session=" + "0" * 64}, "tampered session"),
                ({"X-Apex-Probe": "x" * 8192}, "8KB header"),
            ):
                merged = dict(cookie)
                merged.update(headers)
                resp = http_req("GET", base + "/console/audit", headers=merged)
                note("apex-api", resp.status < 500, "%s -> %d" % (label, resp.status))
            # A deeply nested JSON body is refused or bounded, never a stack
            # overflow: 4000 nested arrays.
            depth = b"[" * 4000 + b"]" * 4000
            resp = http_req(
                "POST",
                base + "/v1/leads",
                headers={"Content-Type": "application/json"},
                body=depth,
            )
            note("apex-api", resp.status in (400, 413), "4000-deep JSON -> %d" % resp.status)
            # NUL, CRLF and unicode in every text field.
            for field, value in (
                ("email", "nul%00@studio.example"),
                ("email", "crlf%0d%0aX-Evil%3a1@studio.example"),
                ("studio", "emoji+%F0%9F%98%80+studio"),
            ):
                resp = http_req(
                    "POST",
                    base + "/free-audit",
                    headers={"Content-Type": "application/x-www-form-urlencoded"},
                    body=form_body([("email", "ok@studio.example"), ("studio", "S"), ("platforms", "admob"), (field, value)]),
                )
                note("apex-api", resp.status in (303, 400), "hostile %s -> %d" % (field, resp.status))
            # A malformed asset path never escapes the font directory.
            for path in (
                "/assets/fonts/..%2f..%2fetc%2fpasswd",
                "/assets/apex.css%00",
                "/console/%2e%2e/%2e%2e/etc/passwd",
            ):
                resp = http_req("GET", base + path, headers=cookie)
                note("apex-api", resp.status < 500, "asset/path abuse %s -> %d" % (path, resp.status))
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
        ledger_pagination = None
        for service, surface, cell_id in self.cells_of("input-corpus"):
            if service == "apex-api" and surface == "GET /console/ledger":
                if ledger_pagination is None:
                    ledger_pagination = self.ledger_pagination_probe()
                ok, detail = ledger_pagination
                self.cell(cell_id, ok, detail, "GET /console/ledger?page=<1|2|abc|0|...>")
                continue
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

    def ensure_monitoring_plan_state(self):
        """Write the monitoring `PlanState` the console cancel action loads,
        in `apex-billing`'s exact camelCase shape, atomically (temp file then
        rename), when the cycle has not already recorded it. The publisher is
        the session publisher and the plan id is Monitoring."""
        root = os.path.join(self.api["work"], "credentials")
        path = os.path.join(root, "%s.monitoring.json" % API_PUBLISHER)
        if os.path.exists(path):
            try:
                with open(path, "r", encoding="utf-8") as fh:
                    if json.load(fh).get("plan") == "monitoring":
                        return path
            except ValueError:
                pass
        period = time.strftime("%Y-%m", time.gmtime())
        state = {
            "plan": "monitoring",
            "active": True,
            "since": period + "-01",
            "monthlyFeeMicros": MONITORING_MID_FEE_MICROS,
            "currency": "USD",
            "tier": "mid",
            "revenue30d": MONITORING_BAND_REVENUE_30D_MICROS,
        }
        os.makedirs(root, exist_ok=True)
        tmp = path + ".tmp"
        with open(tmp, "w", encoding="utf-8") as fh:
            json.dump(state, fh, indent=2)
        os.replace(tmp, path)
        return path

    def csrf_api(self):
        problems = []
        notes = []
        base = self.base_url("apex-api")
        self.api_session()
        cookie = self.cookie_header("apex-api")
        self.ensure_monitoring_plan_state()
        session_posts = [
            ("/console/ledger/session/end", ""),
            ("/console/connections/admob/submit", "platform=admob"),
            ("/console/connections/admob/rotate", "platform=admob"),
            ("/console/connections/admob/disconnect", "platform=admob&confirm=yes"),
            ("/console/proposals/nope/decision", "decision=approve"),
            ("/console/settings/theme", "theme=dark"),
            ("/console/settings/access/revoke", "handle=deadbeef&confirm=yes"),
            ("/console/settings/monitoring/cancel", "confirm=yes"),
            ("/console/settings/billing/managed/cancel", "confirm=yes"),
        ]
        for path, payload in session_posts:
            body = (payload + "&").encode() if payload else b""
            resp = http_req(
                "POST",
                base + path,
                headers={
                    "Content-Type": "application/x-www-form-urlencoded",
                    **cookie,
                },
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
            notes.append("9 session POSTs: no-CSRF 400, no-session 303")
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
            "POST",
            base + "/v1/cycle/northstar",
            headers={"Content-Type": "application/json", "Authorization": "Bearer " + API_LIVE_AUDIT_TOKEN},
            body=b"{}",
        )
        if cycle.status != 400:
            problems.append("/v1/cycle without platform answered %d (expected 400 behind the bearer)" % cycle.status)
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
        # POST /watch: the public setup-watch form. A repeat submission inside
        # the dedup window is the same subscription: same confirmation ref,
        # exactly one durable subscriber row. The valid submission is the seed
        # the billing run later turns into the $99 line.
        watch_root = os.path.join(self.api["work"], "watch")
        subscribers_path = os.path.join(watch_root, "subscribers.jsonl")
        signup = form_body(
            [
                ("domain", WATCH_DOMAIN),
                ("email", WATCH_EMAIL),
                ("accounts", "admob.com = pub-1234567890\napplovin.com = dogfood-live"),
            ]
        )
        watch_before = len(self.read_watch_subscribers(watch_root))
        watch_first = http_req(
            "POST",
            base + "/watch",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=signup,
        )
        watch_second = http_req(
            "POST",
            base + "/watch",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=signup,
        )
        watch_after = len(self.read_watch_subscribers(watch_root))
        watch_refs = [resp.header("location") for resp in (watch_first, watch_second)]
        self.monetization["watch_ref"] = None
        if watch_first.status != 303 or watch_second.status != 303:
            problems.append("watch submit -> %s (expected 303, 303)" % [watch_first.status, watch_second.status])
        elif watch_refs[0] != watch_refs[1] or not (watch_refs[0] or "").startswith("/watch/requested?ref=sub_"):
            problems.append("watch replay returned refs %s (expected one /watch/requested?ref=sub_...)" % watch_refs)
        elif watch_after - watch_before != 1:
            problems.append("watch replay stored %d subscriber(s), expected 1" % (watch_after - watch_before))
        else:
            self.monetization["watch_ref"] = watch_refs[0].split("ref=", 1)[-1]
            notes.append("watch replay: same ref %s, one durable subscriber" % self.monetization["watch_ref"])
        # The cancel actions: no CSRF is 400; with CSRF the first and repeated
        # cancellations both 303 and the recorded stop period is never
        # re-dated (idempotent, recorded-forever state).
        csrf = form_body([("csrf_token", self.api.get("csrf") or ""), ("confirm", "yes")])
        # The no-plan state is an honest 400, separately from the CSRF gate:
        # park the seeded monitoring file for one request, then restore it.
        monitoring_state = self.ensure_monitoring_plan_state()
        parked = monitoring_state + ".dogfood-parked"
        os.replace(monitoring_state, parked)
        try:
            no_plan = http_req(
                "POST",
                base + "/console/settings/monitoring/cancel",
                headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
                body=csrf,
            )
        finally:
            os.replace(parked, monitoring_state)
        if no_plan.status != 400:
            problems.append(
                "cancel without a plan state -> %d (expected the honest 400)" % no_plan.status
            )
        else:
            notes.append("no plan state: cancel -> 400")
        for path in ("/console/settings/monitoring/cancel", "/console/settings/billing/managed/cancel"):
            plan_name = "monitoring" if "monitoring" in path else "managed"
            state_path = os.path.join(self.api["work"], "credentials", "northstar.%s.json" % plan_name)
            cancel_first = http_req(
                "POST",
                base + path,
                headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
                body=csrf,
            )
            stop_first = json.load(open(state_path)).get("stopsAfter") if os.path.exists(state_path) else None
            cancel_second = http_req(
                "POST",
                base + path,
                headers={"Content-Type": "application/x-www-form-urlencoded", **cookie},
                body=csrf,
            )
            stop_second = json.load(open(state_path)).get("stopsAfter") if os.path.exists(state_path) else None
            if cancel_first.status != 303 or cancel_second.status != 303:
                problems.append("cancel %s -> %s (expected 303, 303)" % (path, [cancel_first.status, cancel_second.status]))
            elif not stop_first or stop_first != stop_second:
                problems.append("cancel %s stop period %r was re-dated to %r" % (path, stop_first, stop_second))
            else:
                notes.append("cancel %s: 303 twice, stop period %s unchanged" % (plan_name, stop_first))
        # POST /v1/operator/jobs: the bearer-gated receiver persists the exact
        # request bytes, a duplicate job_id returns the existing recording
        # unchanged, and GET reads the same bytes back.
        job = {
            "job_id": "opj_d0f00d000001",
            "kind": "recheck_request",
            "requested_by": "dogfood-live-console",
            "requested_at_ms": int(time.time() * 1000),
            "target": "northstar",
            "payload": {"reason": "dogfood-live idempotency probe"},
            "idempotency_key": "dogfood-live-opj-1",
            "state": "queued",
            "state_history": [
                {"state": "queued", "at_ms": int(time.time() * 1000), "note": "submitted by the dogfood console probe"}
            ],
            "outcome": {},
            "correlation_id": "dogfood-live-opj-correlation",
        }
        job_bytes = json.dumps(job, separators=(",", ":")).encode()
        auth = {"Authorization": "Bearer " + API_OPERATOR_JOBS_TOKEN, "Content-Type": "application/json"}
        jobs_dir = os.path.join(self.api["work"], "jobs")
        jobs_before = sorted(os.listdir(jobs_dir)) if os.path.isdir(jobs_dir) else []
        jobs_first = http_req("POST", base + "/v1/operator/jobs", headers=auth, body=job_bytes)
        jobs_second = http_req("POST", base + "/v1/operator/jobs", headers=auth, body=job_bytes)
        jobs_after = sorted(os.listdir(jobs_dir)) if os.path.isdir(jobs_dir) else []
        jobs_get = http_req(
            "GET", base + "/v1/operator/jobs/" + job["job_id"], headers={"Authorization": "Bearer " + API_OPERATOR_JOBS_TOKEN}
        )
        job_file = os.path.join(jobs_dir, job["job_id"] + ".json")
        if jobs_first.status != 200 or jobs_second.status != 200:
            problems.append("operator jobs POST -> %s (expected 200, 200)" % [jobs_first.status, jobs_second.status])
        elif jobs_first.body != job_bytes or jobs_second.body != job_bytes:
            problems.append("operator jobs POST did not return the request bytes verbatim")
        elif jobs_after != sorted(jobs_before + [job["job_id"] + ".json"]):
            problems.append("operator jobs dir gained %s, expected exactly %s" % (jobs_after, job["job_id"] + ".json"))
        elif not os.path.exists(job_file) or open(job_file, "rb").read() != job_bytes:
            problems.append("operator job file does not hold the exact request bytes")
        elif jobs_get.status != 200 or jobs_get.body != job_bytes:
            problems.append("operator jobs GET answered %d/%.40r, expected the stored bytes" % (jobs_get.status, jobs_get.body))
        else:
            self.monetization["job_id"] = job["job_id"]
            notes.append("operator jobs: duplicate idempotent, GET reads the stored bytes")
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
        for form in REVOKE_FORM_PATTERN.findall(page.text):
            fields = parse_hidden_fields(form)
            if not fields.get("target"):
                continue
            phrase = CONFIRM_PHRASE_PATTERN.search(form)
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
                if service == "apex-api" and template == "/watch/billing-profile/recorded":
                    # The stable recorded page takes its identifier in the
                    # `ref` query parameter. A malformed reference is filtered
                    # out and the honest recorded page renders without a
                    # confirmation id; no foreign bytes may reach the body.
                    problems = []
                    observations = []
                    for payload in TRAVERSAL_CORPUS:
                        url = (
                            self.base_url(service)
                            + "/watch/billing-profile/recorded?ref="
                            + urllib.parse.quote(payload, safe="")
                        )
                        try:
                            resp = http_req("GET", url, headers=headers)
                        except ReqError as error:
                            problems.append("%s -> %s" % (payload, error))
                            continue
                        accepted = resp.status in (400, 404, 303, 401)
                        # A 200 is only honest when it is the generic
                        # invalid-reference page, never a recorded success.
                        if resp.status == 200 and (
                            "Your billing details are recorded." not in resp.text
                        ):
                            accepted = False
                        if not accepted:
                            problems.append(
                                "%s -> HTTP %d (neither a typed refusal nor the honest recorded page)"
                                % (payload, resp.status)
                            )
                        for canary in TRAVERSAL_CANARIES:
                            if canary in resp.body:
                                problems.append("%s leaked %r" % (payload, canary.decode()))
                        observations.append("%d" % resp.status)
                    detail = (
                        "GET /watch/billing-profile/recorded?ref=<corpus> statuses [%s]"
                        % ", ".join(observations)
                    )
                    if problems:
                        detail += "; " + "; ".join(problems[:4])
                    self.cell(
                        cell_id,
                        not problems,
                        detail,
                        "GET /watch/billing-profile/recorded?ref=<corpus>",
                    )
                    continue
                if service == "apex-api" and template == "/watch/confirm":
                    # The email-ownership route takes its hostile input in the
                    # `token` query parameter. A malformed token is the 400
                    # refusal page; a well-shaped unknown one is the same
                    # refusal rendered at 200. Neither may carry foreign bytes.
                    problems = []
                    observations = []
                    for payload in TRAVERSAL_CORPUS:
                        url = (
                            self.base_url(service)
                            + "/watch/confirm?token="
                            + urllib.parse.quote(payload, safe="")
                        )
                        try:
                            resp = http_req("GET", url, headers=headers)
                        except ReqError as error:
                            problems.append("%s -> %s" % (payload, error))
                            continue
                        accepted = resp.status in (400, 404, 303, 401)
                        if resp.status == 200 and (
                            "This confirmation link is no longer valid." in resp.text
                            or "Confirm your billing details." in resp.text
                        ):
                            accepted = True
                        if not accepted:
                            problems.append(
                                "%s -> HTTP %d (neither a typed refusal nor an honest state)"
                                % (payload, resp.status)
                            )
                        for canary in TRAVERSAL_CANARIES:
                            if canary in resp.body:
                                problems.append("%s leaked %r" % (payload, canary.decode()))
                        observations.append("%d" % resp.status)
                    detail = "GET /watch/confirm?token=<corpus> statuses [%s]" % ", ".join(
                        observations
                    )
                    if problems:
                        detail += "; " + "; ".join(problems[:4])
                    self.cell(
                        cell_id,
                        not problems,
                        detail,
                        "GET /watch/confirm?token=<corpus>",
                    )
                    continue
                if "{" not in template:
                    self.cell(cell_id, False, "declared traversal surface %s has no parameter" % surface)
                    continue
                problems = []
                observations = []
                for payload in TRAVERSAL_CORPUS:
                    path = TRAVERSAL_TOKEN_PATTERN.sub(payload, template, count=1)
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
        self.api_session()
        cookie = self.cookie_header("apex-api")
        resp = http_req(
            "GET",
            base + "/console/audit?platform=" + urllib.parse.quote(HOSTILE_QUERY),
            headers=cookie,
        )
        if HOSTILE_QUERY in resp.text:
            problems.append("the hostile platform echo is unescaped on /console/audit")
        else:
            notes.append("hostile platform refused/escaped (HTTP %d)" % resp.status)
        report = http_req("GET", base + "/console/audit?platform=admob", headers=cookie)
        if "<script" in report.text:
            problems.append("the audit report echoes the hostile snapshot name as script")
        else:
            notes.append("the hostile snapshot name never becomes script")
        csv_resp = http_req("GET", base + "/console/audit/export.csv?platform=admob", headers=cookie)
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
            if not any(cell.endswith("@probe-hostile.example") for cell in email_cells):
                problems.append("the hostile contact email is missing from the CSV row")
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

        def fire(index):
            return http_req(
                "POST",
                base + "/free-audit",
                headers={
                    "Content-Type": "application/x-www-form-urlencoded",
                    "X-Forwarded-For": "198.51.100.%d" % (index % 250 + 1),
                },
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
        rate_headers = {
            "Content-Type": "application/x-www-form-urlencoded",
            "X-Forwarded-For": "203.0.113.250",
        }
        rate_statuses = sorted(
            {
                http_req(
                    "POST",
                    base + "/free-audit",
                    headers=rate_headers,
                    body=form_body(
                        [
                            ("email", "rate-%d@studio.example" % index),
                            ("studio", "Rate Studio"),
                            ("platforms", "admob"),
                        ]
                    ),
                ).status
                for index in range(35)
            }
        )
        if 429 not in rate_statuses:
            problems.append(
                "35 posts from one client were not rate limited: %s" % rate_statuses
            )
        detail = (
            "32 parallel identical free-audit posts -> %s, %d ref(s), %d durable lead(s); "
            "35 same-client posts -> %s"
            % (statuses, len(refs), after - before, rate_statuses)
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

        def fire(index):
            return http_req(
                "POST",
                self.base_url("apex-control") + "/control/actions/session_revoke",
                headers={
                    "Content-Type": "application/x-www-form-urlencoded",
                    "X-Forwarded-For": "198.51.100.%d" % (index % 250 + 1),
                    **driver,
                },
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

        def fire(index):
            return http_req(
                "POST",
                self.base_url("apex-sales-web") + "/actions/reset-health",
                headers={
                    "Content-Type": "application/x-www-form-urlencoded",
                    **cookie,
                },
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
        # The watch sub-probes run after `failure_api` has restarted apex-api.
        watch_results = {}

        def watch_result(surface):
            if surface not in watch_results:
                if surface == "POST /watch":
                    watch_results[surface] = self.failure_dns_rebinding()
                elif surface == "GET /watch":
                    watch_results[surface] = self.failure_unpaid_subscriber()
                else:
                    watch_results[surface] = results["apex-api"]
            return watch_results[surface]

        for service, surface, cell_id in self.cells_of("failure-injection"):
            if service == "apex-api" and surface in ("POST /watch", "GET /watch"):
                ok, detail = watch_result(surface)
            else:
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
        self.api_session(force=True)
        page = http_req("GET", base + "/console/audit?platform=admob", headers=self.cookie_header("apex-api"))
        if page.status != 200:
            problems.append("audit page after restart -> %d" % page.status)
        else:
            notes.append("pages recover after restart (session re-minted)")
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
        self.counters["server_kills"] += 1
        self.counters["kill:clickhouse"] = self.counters.get("kill:clickhouse", 0) + 1
        drop_http_pool()
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
                    proc = self.spawn(
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
        self.counters["server_starts"] += 1
        self.counters["start:clickhouse-recovery"] = (
            self.counters.get("start:clickhouse-recovery", 0) + 1
        )
        drop_http_pool()
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
        # ClickHouse resolves DNS after forking its resolver thread. On macOS
        # the Objective-C runtime aborts the forked child (SIGABRT/exit 134)
        # unless this is set, which would turn the severance subprobe's own
        # restart into a failure. Inherit a caller-set value when present.
        recovered_env = dict(os.environ)
        recovered_env.setdefault("OBJC_DISABLE_INITIALIZE_FORK_SAFETY", "YES")
        try:
            proc = self.spawn_popen(
                argv,
                cwd=self.root,
                env=recovered_env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
        except OSError as error:
            return False, str(error)
        self.ch_owned_procs.append(proc)
        # The severance probe stops the server and immediately restarts it.
        # The listening socket can linger for a moment, in which case
        # ClickHouse exits 210 (address in use) before it binds; retry a
        # bounded number of times instead of reporting a flaky severance.
        last_failure = "the restarted server never answered within 60s"
        for attempt in range(10):
            if attempt:
                # The old process can hold ClickHouse's status lock briefly
                # after its listening socket goes away. Ten two-second
                # attempts keep the severance probe from flaking on that
                # hand-off without hiding a real failure.
                time.sleep(2.0)
                try:
                    proc = self.spawn_popen(
                        argv,
                        cwd=self.root,
                        env=recovered_env,
                        stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL,
                        start_new_session=True,
                    )
                except OSError as error:
                    last_failure = str(error)
                    continue
                self.ch_owned_procs.append(proc)
            deadline = time.time() + 60
            while time.time() < deadline:
                if self.ch_reachable():
                    return True, None
                if proc.poll() is not None:
                    last_failure = "the restarted server exited with %s" % proc.returncode
                    break
                time.sleep(0.3)
            else:
                last_failure = "the restarted server never answered within 60s"
        return False, last_failure

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
        self.api_session()
        api_cookie = self.cookie_header("apex-api")
        analytics = http_req(
            "GET", self.base_url("apex-api") + "/console/analytics?view=revenue", headers=api_cookie
        )
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
            recovery = http_req(
                "GET",
                self.base_url("apex-api") + "/console/analytics?view=revenue",
                headers=self.cookie_header("apex-api"),
            )
            if recovery.status != 200 or "$51,500.00" not in recovery.text:
                problems.append("recovery: revenue page does not show USD 51,500.00 again")
            else:
                notes.append("recovery: re-seeded, projections republished, revenue shows USD 51,500.00")
        else:
            recovery = http_req(
                "GET",
                self.base_url("apex-api") + "/console/analytics?view=revenue",
                headers=self.cookie_header("apex-api"),
            )
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
        # The ApAudit expectations that ride differential-truth get their own
        # sub-result so the failing cell names the exact assertion.
        levelplay = (True, "")
        vies = (True, "")
        platform_monitoring = (True, "")
        platform_proposals = (True, "")
        if self.cells_of("differential-truth", "apex-api"):
            surfaces = {surface for _svc, surface, _cid in self.cells_of("differential-truth", "apex-api")}
            if "POST /v1/audit/{platform}" in surfaces:
                levelplay = self.differential_levelplay()
            if "GET /console/settings/billing" in surfaces:
                vies = self.differential_vies()
            if (
                "GET /console/settings" in surfaces
                or "GET /console/settings/monitoring" in surfaces
                or "GET /console/proposals" in surfaces
            ):
                truth = self.platform_truth()
                platform_monitoring = (truth[0], truth[1])
                platform_proposals = (truth[2], truth[3])
        for service, surface, cell_id in self.cells_of("differential-truth"):
            ok, detail = results[service]
            if service == "apex-api" and surface == "POST /v1/audit/{platform}":
                ok = ok and levelplay[0]
                detail = "; ".join(part for part in ("LevelPlay: " + levelplay[1], detail) if part)
            elif service == "apex-api" and surface == "GET /console/settings/billing":
                ok = ok and vies[0]
                detail = "; ".join(part for part in ("VIES: " + vies[1], detail) if part)
            elif service == "apex-api" and surface == "GET /console/settings":
                ok = ok and platform_monitoring[0]
                detail = "; ".join(
                    part for part in ("Monitoring tiers: " + platform_monitoring[1], detail) if part
                )
            elif service == "apex-api" and surface == "GET /console/settings/monitoring":
                ok = ok and platform_monitoring[0]
                detail = "; ".join(
                    part for part in ("Monitoring tiers: " + platform_monitoring[1], detail) if part
                )
            elif service == "apex-api" and surface == "GET /console/proposals":
                ok = ok and platform_proposals[0]
                detail = "; ".join(
                    part for part in ("Platform proposals: " + platform_proposals[1], detail) if part
                )
            self.cell(cell_id, ok, detail)

    # 12a ------------------------------ platform audit truth (defects I and J)
    def write_platform_registry(self, root, publisher, platform, credentials):
        """One publisher in the runner's vaulted registry shape, validated for
        exactly one platform. The transport base URLs point at the harness
        stub so the audit's reads are observed, never sent to a vendor."""
        os.makedirs(root, exist_ok=True)
        account = {
            "developer_site": WORKSPACE_PROVEN_DOMAIN,
            "expected_networks": [],
            "ad_unit_ids": ["unit-b"] if platform == "levelplay" else [],
            "bidder_accounts": [],
            "platforms": {platform: credentials},
        }
        with open(os.path.join(root, publisher + ".blob"), "w", encoding="utf-8") as fh:
            fh.write(vault_encrypt(API_VAULT_KEY_HEX, json.dumps(account)))
        meta = {
            "platforms": {
                platform: {
                    "state": "validated",
                    "validated_at_unix_ms": int(time.time() * 1000),
                    "reporting": True,
                    "credential_fields": sorted(credentials),
                    "failure_class": None,
                }
            }
        }
        with open(
            os.path.join(root, publisher + ".connections.json"), "w", encoding="utf-8"
        ) as fh:
            json.dump(meta, fh)

    def platform_truth_responder(self, method, path, requests):
        """The platform management/reporting reads the fleet audit performs.
        MAX reports country rows, LevelPlay reports app revenue and the B-group
        waterfall instance, and AdMob reports its mediation group plus the
        mediation report. The shapes carry every plausible field spelling so
        the probe observes the product's own parser, not a harness guess."""
        del method, requests
        lower = path.lower()
        if "/partners/publisher/auth" in lower:
            return (200, {"token": "dogfood-platform-truth-bearer"})
        if "/maxreport" in lower:
            return (
                200,
                {
                    "code": 200,
                    "count": 1,
                    "results": [
                        {
                            "day": time.strftime("%Y-%m-%d", time.gmtime(time.time() - 86_400)),
                            "country": "US",
                            "max_ad_unit_id": "unit-max",
                            "estimated_revenue": "120000.00",
                            "impressions": "1000",
                        }
                    ],
                },
            )
        if "/ad_units" in lower:
            return (200, [])
        if "/levelplay/network/instances/v4" in lower:
            return (
                200,
                {
                    "instances": [
                        {
                            "instanceId": "inst_b",
                            "id": "inst_b",
                            "adapterNetwork": "admob",
                            "network": "admob",
                            "instanceName": "group B instance",
                            "name": "group B instance",
                            "price": "2.50",
                            "priceEcpm": "2.50",
                            "position": 1,
                            "order": 1,
                            "group": "B",
                            "instanceGroup": "B",
                            "abTest": "B",
                            "testGroup": "B",
                            "variant": "B",
                        }
                    ]
                },
            )
        if "/levelplay/groups/v4" in lower:
            return (
                200,
                {
                    "groups": [
                        {
                            "groupId": "grp_b",
                            "abTest": "B",
                            "name": "group B",
                            "floorPrice": "2.50",
                            "price": "2.50",
                            "format": "rewarded",
                            "countriesRate": 1.0,
                            "instances": [
                                {
                                    "instanceId": "inst_b",
                                    "adapterNetwork": "admob",
                                    "instanceName": "group B instance",
                                    "price": "2.50",
                                    "format": "rewarded",
                                    "position": 1,
                                }
                            ],
                        }
                    ]
                },
            )
        if "/levelplay/reporting/v1" in lower:
            return (
                200,
                {
                    "code": 200,
                    "results": [
                        {
                            "date": time.strftime("%Y-%m-%d", time.gmtime(time.time() - 86_400)),
                            "abTest": "B",
                            "revenue": "600000.00",
                            "activeUsers": "1000",
                        }
                    ],
                },
            )
        admob_group = {
            "mediationGroupId": "grp_admob",
            "name": "AdMob mediation group",
            "format": "rewarded",
            "adUnitMappings": [
                {
                    "adUnitId": "unit-admob",
                    "network": "admob",
                    "ecpm": "1.50",
                    "eCPM": "1.50",
                    "revenue": "100000.00",
                    "estimatedRevenue": "100000.00",
                },
                {
                    "adUnitId": "unit-admob-2",
                    "network": "applovin",
                    "ecpm": "2.00",
                    "eCPM": "2.00",
                },
            ],
            "revenue": "100000.00",
            "estimatedRevenue": "100000.00",
            "estimatedEarnings": "100000.00",
        }
        if "report" in lower or "networkreport" in lower:
            day = time.strftime("%Y-%m-%d", time.gmtime(time.time() - 86_400))
            row = {
                "date": day,
                "mediationGroupId": "grp_admob",
                "revenue": "100000.00",
                "estimatedRevenue": "100000.00",
                "estimatedEarnings": "100000.00",
                "impressions": "1000",
            }
            # The documented v1beta mediationReport:generate row shape the
            # product's own parser (and its recorded fixture) reads: the
            # metric lives in the metricValues map, money as an exact
            # microsValue string. One row for one day: $100,000.00.
            documented_row = {
                "dimensionValues": {
                    "DATE": {"value": day},
                    "MEDIATION_GROUP_ID": {"value": "grp_admob"},
                },
                "metricValues": {
                    "ESTIMATED_EARNINGS": {"microsValue": "100000000000"},
                    "IMPRESSIONS": {"integerValue": "1000"},
                },
            }
            return (
                200,
                {
                    "code": 200,
                    "results": [row],
                    "rows": [documented_row],
                    "networkReport": {
                        "header": {
                            "dimensionHeaders": [
                                {"name": "DATE"},
                                {"name": "MEDIATION_GROUP_ID"},
                            ],
                            "metricHeaders": [
                                {"name": "ESTIMATED_EARNINGS"},
                                {"name": "IMPRESSIONS"},
                            ],
                        },
                        "rows": [
                            {
                                "dimensionValues": [
                                    {"value": day},
                                    {"value": "grp_admob"},
                                ],
                                "metricValues": [
                                    {"value": "100000.00"},
                                    {"value": "1000"},
                                ],
                            }
                        ],
                    },
                    "mediationGroups": [admob_group],
                },
            )
        if "mediationgroups" in lower:
            return (200, {"mediationGroups": [admob_group]})
        return (404, {"error": "unexpected platform-truth path", "path": path})

    def platform_truth(self):
        """Run the real fleet audit against the three platform seams and the
        real cycle over each produced snapshot. Returns
        `(monitoring_ok, monitoring_detail, proposals_ok, proposals_detail)`.

        Defect J: the audit must fill `platform_totals_30d` from MAX country
        revenue, LevelPlay app revenue and AdMob mediation revenue, and the
        activation seam must then select the matching tier. Defect I: the same
        LevelPlay (B-group waterfall) and AdMob (mediation group) snapshots
        must yield proposals. On the pre-fix tree the totals stay zero and the
        proposed counts are zero, which is the adversarial failure this cell
        records."""
        cached = self.monetization.get("platform_truth")
        if cached is not None:
            return cached
        monitoring_problems = []
        proposal_problems = []
        notes = []
        stub = LocalStub(self.platform_truth_responder).start()
        work = os.path.join(self.workdir, "platform-truth")
        shutil.rmtree(work, ignore_errors=True)
        os.makedirs(work, exist_ok=True)
        try:
            for platform in ("applovin_max", "levelplay", "admob"):
                publisher = "truth-" + platform.replace("_", "-")
                root = os.path.join(work, "credentials", platform)
                snapshots = os.path.join(work, "snapshots", platform)
                ledgers = os.path.join(work, "ledgers", platform)
                data = os.path.join(work, "data", platform)
                for path in (ledgers, data):
                    os.makedirs(path, exist_ok=True)
                base = stub.base_url()
                if platform == "applovin_max":
                    credentials = {
                        "api_key": "dogfood-truth-max-key",
                        "account_id": "",
                        "base_url": base,
                        "report_key": "dogfood-truth-report-key",
                        "reporting_base_url": base,
                        "refresh_token": None,
                    }
                elif platform == "levelplay":
                    credentials = {
                        "api_key": "dogfood-truth-levelplay-key",
                        "account_id": "dogfood-truth-app-key",
                        "base_url": base,
                        "report_key": None,
                        "reporting_base_url": base,
                        "refresh_token": "dogfood-truth-refresh",
                    }
                else:
                    credentials = {
                        "api_key": "dogfood-truth-admob-token",
                        "account_id": "accounts/pub-1234567890",
                        "base_url": base,
                        "report_key": "dogfood-truth-admob-report",
                        "reporting_base_url": base,
                        "refresh_token": None,
                    }
                self.write_platform_registry(root, publisher, platform, credentials)
                env = self.runner_env()
                env.update(
                    {
                        "APEX_CONNECTIONS_ROOT": root,
                        "APEX_RUNNER_SNAPSHOT": snapshots,
                        "APEX_RUNNER_LEDGER_ROOT": ledgers,
                        "APEX_RUNNER_DATA_DIR": data,
                        "APEX_METRICS_TEXTFILE": os.path.join(work, platform + ".prom"),
                    }
                )
                audit = self.spawn(
                    [self.bin_path("apex-runner"), "audit", "--out", snapshots, "--json"],
                    cwd=work,
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=180,
                )
                audit_out = (audit.stdout or "") + (audit.stderr or "")
                if audit.returncode != 0:
                    monitoring_problems.append(
                        "%s audit exit=%d: %s"
                        % (platform, audit.returncode, sanitize_evidence(audit_out, 240))
                    )
                    proposal_problems.append("%s audit failed" % platform)
                    continue
                snapshot_path = os.path.join(snapshots, publisher, platform + ".json")
                if not os.path.exists(snapshot_path):
                    monitoring_problems.append("%s audit wrote no snapshot at %s" % (platform, snapshot_path))
                    proposal_problems.append("%s snapshot missing" % platform)
                    continue
                with open(snapshot_path, "r", encoding="utf-8") as fh:
                    snapshot = json.load(fh)
                revenue = int(snapshot.get("platformTotals30d", {}).get("revenue30d", 0) or 0)
                expected = MONITORING_PLATFORM_TRUTH[platform]
                if revenue < expected["revenue_30d"]:
                    monitoring_problems.append(
                        "%s platformTotals30d=%d, expected at least %d from the platform's own revenue read"
                        % (platform, revenue, expected["revenue_30d"])
                    )
                cycle = self.spawn(
                    [self.bin_path("apex-runner"), "cycle", "--json"],
                    cwd=work,
                    env=env,
                    capture_output=True,
                    text=True,
                    timeout=180,
                )
                cycle_out = (cycle.stdout or "") + (cycle.stderr or "")
                cycle_json = None
                if cycle.returncode == 0:
                    try:
                        cycle_json = json.loads(cycle.stdout).get("cycle", {})
                    except ValueError:
                        cycle_json = None
                if cycle_json is None:
                    monitoring_problems.append(
                        "%s cycle exit=%d: %s"
                        % (platform, cycle.returncode, sanitize_evidence(cycle_out, 240))
                    )
                    proposal_problems.append("%s cycle failed" % platform)
                    continue
                state_path = os.path.join(root, publisher + ".monitoring.json")
                if not os.path.exists(state_path):
                    monitoring_problems.append("%s cycle recorded no monitoring plan state" % platform)
                else:
                    with open(state_path, "r", encoding="utf-8") as fh:
                        state = json.load(fh)
                    if (
                        state.get("tier") != expected["tier"].lower()
                        or state.get("monthlyFeeMicros") != expected["fee_micros"]
                    ):
                        monitoring_problems.append(
                            "%s plan state tier=%r fee=%r, expected %s/%d from %d micros"
                            % (
                                platform,
                                state.get("tier"),
                                state.get("monthlyFeeMicros"),
                                expected["tier"].lower(),
                                expected["fee_micros"],
                                revenue,
                            )
                        )
                    else:
                        notes.append(
                            "%s revenue=%d -> %s ($%d)"
                            % (
                                platform,
                                revenue,
                                expected["tier"],
                                expected["fee_micros"] // 1_000_000,
                            )
                        )
                if platform in ("levelplay", "admob"):
                    proposed = int(cycle_json.get("proposed", 0) or 0)
                    if proposed < 1:
                        proposal_problems.append(
                            "%s cycle proposed=%d from its %s; the generator must yield a proposal"
                            % (
                                platform,
                                proposed,
                                "waterfall B-group instance" if platform == "levelplay" else "mediation group",
                            )
                        )
                    else:
                        notes.append("%s proposed=%d" % (platform, proposed))
                        awaiting = " ".join(
                            str(item)
                            for item in (cycle_json.get("awaiting_verification") or [])
                        ).lower()
                        # The held-proposal reason names the platform operation,
                        # not the resource fields: LevelPlay's new
                        # SetLevelplayInstancePrice maps to `updateEntryPrice`
                        # and AdMob's new SetAdmobMediationFloor maps to
                        # `updateMediationGroup`. The proposal's own rationale
                        # carries the instance/group and target; the operation
                        # name here proves the new variant was the one exercised.
                        operation = (
                            "updateentryprice"
                            if platform == "levelplay"
                            else "updatemediationgroup"
                        )
                        if awaiting and operation not in awaiting:
                            proposal_problems.append(
                                "%s held operation does not name %s: %s"
                                % (platform, operation, sanitize_evidence(awaiting, 160))
                            )
                        else:
                            notes.append("%s held operation=%s" % (platform, operation))
        finally:
            stub.stop()
        result = (
            not monitoring_problems,
            "; ".join(notes + monitoring_problems),
            not proposal_problems,
            "; ".join(notes + proposal_problems),
        )
        self.monetization["platform_truth"] = result
        return result

    def differential_api(self):
        problems = []
        notes = []
        base = self.base_url("apex-api")
        self.api_session()
        cookie = self.cookie_header("apex-api")
        analytics = http_req("GET", base + "/console/analytics?view=revenue", headers=cookie)
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
        ledger_page = http_req("GET", base + "/console/ledger", headers=cookie)
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
        report = http_req("GET", base + "/console/audit?platform=admob", headers=cookie)
        csv_resp = http_req("GET", base + "/console/audit/export.csv?platform=admob", headers=cookie)
        page_findings = len(re.findall(r"/console/audit/findings/", report.text))
        csv_rows = len([line for line in csv_resp.text.split("\r\n") if line]) - 1
        if page_findings != csv_rows:
            problems.append("the report page links %d findings, the CSV exports %d rows" % (page_findings, csv_rows))
        else:
            notes.append("audit CSV rows equal the report rows (%d)" % csv_rows)
        ok, detail = self.differential_monetization(base, cookie)
        (notes if ok else problems).append(detail)
        return (not problems, "; ".join(problems + notes))

    def differential_levelplay(self):
        """The LevelPlay reporting request shape: the runner's experiment pull
        must ask for `metrics=revenue,activeUsers` with
        `breakdowns=date,abTest`, so the lift value is revenue and the active
        users ride separately."""
        problems = []

        def responder(method, path, requests):
            if "/partners/publisher/auth" in path:
                return (200, {"token": "dogfood-levelplay-bearer"})
            if "/levelPlay/reporting/v1" in path:
                return (
                    200,
                    {
                        "code": 200,
                        "results": [
                            {"abTest": "A", "revenue": "12.5", "activeUsers": "100"},
                            {"abTest": "B", "revenue": "13.1", "activeUsers": "110"},
                        ],
                    },
                )
            if "/levelPlay/groups/v4" in path:
                return (
                    200,
                    {
                        "groups": [
                            {"groupId": "exp_dogfood", "abTest": "B", "name": "variant"},
                            {"groupId": "exp_dogfood_control", "abTest": "A", "name": "control"},
                        ]
                    },
                )
            if "/levelPlay/network/instances/v4" in path:
                return (200, {"instances": []})
            return (200, {})

        stub = LocalStub(responder).start()
        credentials = os.path.join(self.workdir, "levelplay-probe-registry")
        shutil.rmtree(credentials, ignore_errors=True)
        os.makedirs(credentials, exist_ok=True)
        lp_blob = os.path.join(credentials, "levelplay-probe.blob")
        lp_meta = os.path.join(credentials, "levelplay-probe.connections.json")
        try:
            account = {
                "developer_site": "example.com",
                "expected_networks": [],
                "ad_unit_ids": [],
                "bidder_accounts": [],
                "platforms": {
                    "levelplay": {
                        "api_key": LEVELPLAY_SECRET_KEY,
                        "account_id": LEVELPLAY_APP_KEY,
                        "base_url": stub.base_url(),
                        "report_key": None,
                        "reporting_base_url": None,
                        "refresh_token": LEVELPLAY_REFRESH_TOKEN,
                    }
                },
            }
            with open(lp_blob, "w", encoding="utf-8") as fh:
                fh.write(vault_encrypt(API_VAULT_KEY_HEX, json.dumps(account)))
            with open(lp_meta, "w", encoding="utf-8") as fh:
                json.dump(
                    {
                        "platforms": {
                            "levelplay": {
                                "state": "validated",
                                "validated_at_unix_ms": int(time.time() * 1000),
                                "reporting": True,
                                "credential_fields": ["account_id", "api_key", "refresh_token"],
                                "failure_class": None,
                            }
                        }
                    },
                    fh,
                )
            proc = self.run_runner_command(
                ["aa-eval", "--experiment", "exp_dogfood", "--json"],
                extra_env={"APEX_CONNECTIONS_ROOT": credentials},
                timeout=180,
            )
            reporting = [r for r in stub.requests if "/levelPlay/reporting/v1" in r["path"]]
            if not reporting:
                problems.append(
                    "the runner never called the LevelPlay reporting endpoint (exit %d): %s"
                    % (proc.returncode, sanitize_evidence((proc.stdout or "") + (proc.stderr or ""), 200))
                )
            else:
                bad = [
                    r["path"]
                    for r in reporting
                    if "metrics=revenue,activeUsers" not in r["path"]
                    or "breakdowns=date,abTest" not in r["path"]
                ]
                if bad:
                    problems.append(
                        "the LevelPlay reporting request carries the wrong metrics/breakdowns: %s" % bad[0]
                    )
                else:
                    return (
                        True,
                        "the LevelPlay reporting request carries metrics=revenue,activeUsers + breakdowns=date,abTest (%d pull(s))"
                        % len(reporting),
                    )
        finally:
            stub.stop()
            for path in (lp_blob, lp_meta):
                try:
                    os.remove(path)
                except OSError:
                    pass
        return (not problems, "; ".join(problems))

    def differential_vies(self):
        """Reverse charge is only stated with a VIES confirmation, and the
        answer is recorded as evidence. A cross-border EU customer is billed
        through the product's own watch bill; the register line's treatment
        must cite the VIES answer, and with VIES unavailable the treatment is
        not reverse charge."""
        problems = []
        watch_root = os.path.join(self.workdir, "vies-watch")
        shutil.rmtree(watch_root, ignore_errors=True)
        os.makedirs(watch_root, exist_ok=True)
        now_ms = int(time.time() * 1000)
        row = {
            "id": "sub_vies000001",
            "publisher": VIES_PROBE_PUBLISHER,
            "domain": VIES_PROBE_DOMAIN,
            "email": "billing@cross-border-probe.example",
            "tier": "normal",
            "createdMs": now_ms,
            "status": "active",
            "cancelTokenHash": hashlib.sha256(b"vies-token").hexdigest(),
            "billing": {
                "legalName": "Cross Border OU",
                "addressLines": ["Telliskivi 1"],
                "country": VIES_CUSTOMER_COUNTRY,
                "vatNumber": VIES_CUSTOMER_VAT,
            },
        }
        with open(os.path.join(watch_root, "subscribers.jsonl"), "w", encoding="utf-8") as fh:
            fh.write(json.dumps(row) + "\n")
        cwd = os.path.join(self.workdir, "vies-billing")
        shutil.rmtree(cwd, ignore_errors=True)
        os.makedirs(cwd, exist_ok=True)
        env = {
            "APEX_WATCH_ROOT": watch_root,
            "APEX_BILLING_PROFILE": self.fleet["seller"],
            "APEX_BILLING_VAT_RATE_BP": str(SELLER_VAT_RATE_BP),
        }
        result = self.watch_bill(self.current_period(), cwd, env)
        register_path = os.path.join(cwd, ".apex", "billing", "invoices.jsonl")
        line = ""
        if os.path.exists(register_path):
            with open(register_path, "r", encoding="utf-8") as fh:
                lines = [entry for entry in fh.read().splitlines() if entry.strip()]
            line = lines[0] if lines else ""
        output = (result or {}).get("output", "")
        if (result or {}).get("issued") != 1:
            # A typed hold/refusal that cites the missing VIES confirmation is
            # the other documented fixed outcome when VIES cannot confirm.
            lowered = output.lower()
            if "refused" in lowered and any(marker in lowered for marker in ("vies", "confirmation")):
                return (True, "the cross-border bill held the invoice pending the VIES confirmation")
            problems.append(
                "the cross-border VAT invoice was neither issued with evidence nor held with a VIES reason (bill=%s)"
                % (result or "failed")
            )
        reverse = "reverse charge" in line.lower() or '"reverseCharge"' in line
        if reverse and not any(marker in line.lower() for marker in VIES_EVIDENCE_MARKERS):
            problems.append(
                "the invoice states reverse charge for %s with no recorded VIES confirmation as evidence"
                % VIES_CUSTOMER_VAT
            )
        if not line and (result or {}).get("issued") == 1:
            problems.append("the billing register recorded no invoice line for the VIES probe")
        return (
            not problems,
            "; ".join(problems)
            or "the cross-border invoice cites the VIES confirmation (or refuses reverse charge without one)",
        )

    def differential_monetization(self, base, cookie):
        """The monetization truth cells, differential across the harness-seeded
        stores:
        - the monitoring tier chosen from the seeded $350k band (runner JSON,
          plan state file and the console monitoring page agree);
        - the setup-watch $99 line after the seeded sign-up (subscriber store,
          invoice register and the idempotent second billing run agree);
        - the AdMob no-lift rule (the kept AdMob change yields the typed
          ineligible record and no fee anywhere)."""
        problems = []
        notes = []
        ledger_root = self.fleet["ledgers"]
        # 1. The monthly run: billed after the cancel so the recorded stop
        # period is proven to still bill its current period.
        period = time.strftime("%Y-%m", time.gmtime())
        start = period + "-01"
        monthly_cwd = os.path.join(self.workdir, "billing-run")
        os.makedirs(monthly_cwd, exist_ok=True)
        proc = self.run_runner_command(["monthly", "--month", period, "--start", start, "--json"], cwd=monthly_cwd)
        monthly_out = (proc.stdout or "") + (proc.stderr or "")
        billing = None
        if proc.returncode == 0:
            try:
                billing = json.loads(proc.stdout).get("billing")
            except ValueError:
                billing = None
        if billing is None:
            problems.append("monthly run failed (exit %d): %s" % (proc.returncode, sanitize_evidence(monthly_out, 300)))
        else:
            rendered = billing.get("rendered_invoice") or ""
            # The cancellation preceded this run, so the stop-period line
            # records it in the id: `monitoring:<month>:cancelled:stops-*`
            # with the Mid band fee.
            monitoring_line = "monitoring:%s:cancelled:stops-%s: net %d micros" % (
                period,
                period,
                MONITORING_MID_FEE_MICROS,
            )
            if monitoring_line not in rendered:
                problems.append("the rendered invoice lacks the Mid band line %r" % monitoring_line)
            else:
                notes.append("monthly VAT render bills %s (band-selected tier, recorded stop)" % monitoring_line)
            ineligible = billing.get("fee_ineligible") or []
            admob_reasons = [
                entry.get("reason", "") for entry in ineligible if entry.get("platform") == "admob"
            ]
            if billing.get("fees") != 0 or ADMOB_KEPT_EXPERIMENT in rendered:
                problems.append(
                    "the kept AdMob change was billed (fees=%s)" % billing.get("fees")
                )
            elif not admob_reasons or "no per-variant revenue measurement" not in admob_reasons[0]:
                problems.append("the AdMob ineligible record is missing its typed reason: %s" % ineligible)
            else:
                notes.append("AdMob kept change: typed fee-ineligible (%s), zero fees" % admob_reasons[0])
            self.monetization["monthly"] = {
                "period": period,
                "rendered_invoice": rendered,
                "fee_ineligible": ineligible,
                "fees": billing.get("fees"),
                "invoice_gross_micros": billing.get("invoice_gross_micros"),
            }
        # 2. The recorded plan state: the tier and the band revenue the tier
        # was chosen from, plus the recorded cancellation.
        state_path = os.path.join(self.api["work"], "credentials", "northstar.monitoring.json")
        if not os.path.exists(state_path):
            problems.append("no monitoring plan state was recorded")
        else:
            with open(state_path, "r", encoding="utf-8") as fh:
                state = json.load(fh)
            if state.get("tier") != "mid" or state.get("monthlyFeeMicros") != MONITORING_MID_FEE_MICROS:
                problems.append(
                    "plan state tier %r fee %r, expected the Mid band" % (state.get("tier"), state.get("monthlyFeeMicros"))
                )
            elif state.get("revenue30d") != MONITORING_BAND_REVENUE_30D_MICROS:
                problems.append(
                    "plan state revenue %r does not equal the seeded band %d"
                    % (state.get("revenue30d"), MONITORING_BAND_REVENUE_30D_MICROS)
                )
            elif state.get("stopsAfter") != period:
                problems.append("plan state stop period %r, expected %s" % (state.get("stopsAfter"), period))
            else:
                notes.append(
                    "plan state: tier mid from $350,000 band revenue, cancellation stops after %s" % period
                )
        # 3. The console monitoring page renders the same store's truth.
        monitoring_page = http_req("GET", base + "/console/settings/monitoring", headers=cookie)
        if monitoring_page.status != 200:
            problems.append("monitoring page -> %d" % monitoring_page.status)
        elif ">Mid<" not in monitoring_page.text or "$1,500.00" not in monitoring_page.text:
            problems.append("the monitoring page does not render the recorded Mid tier and $1,500.00 price")
        elif ("bills %s and stops after it" % period) not in monitoring_page.text:
            problems.append("the monitoring page does not render the recorded cancellation stop period")
        else:
            notes.append("console monitoring page agrees: Mid, $1,500.00, billed through %s" % period)
        # 4. The AdMob no-lift rule: the typed ineligible record reached the
        # shared ledger, and the console billing view keeps the change out of
        # every billed artifact (no signed fee line anywhere names it).
        with open(os.path.join(ledger_root, "northstar.jsonl"), "r", encoding="utf-8") as fh:
            ledger_lines = [line for line in fh.read().splitlines() if line.strip()]
        ineligible_records = [
            line for line in ledger_lines if "no per-variant revenue measurement" in line
        ]
        if not ineligible_records or ADMOB_KEPT_EXPERIMENT not in ineligible_records[0]:
            problems.append("the typed AdMob ineligible record never reached the ledger")
        else:
            notes.append("ledger carries %d typed AdMob ineligible record(s)" % len(ineligible_records))
        billing_page = http_req("GET", base + "/console/billing", headers=cookie)
        if billing_page.status != 200:
            problems.append("billing page -> %d" % billing_page.status)
        elif "No signed monthly billing record is stored for this workspace." not in billing_page.text:
            problems.append("the console billing register does not render its no-fee state")
        elif ADMOB_KEPT_EXPERIMENT in billing_page.text:
            problems.append("the console billing page lists the fee-ineligible AdMob change as billed")
        else:
            notes.append("console billing page: no signed fee record names the ineligible AdMob change")
        # 5. The setup-watch $99 line: the subscriber the form seeded, the
        # billing run's report, the register document, and the idempotent
        # second run (one month never bills twice).
        ref = self.monetization.get("watch_ref")
        subscribers_path = os.path.join(self.api["work"], "watch", "subscribers.jsonl")
        # The store is append-only: an update writes a new row, so fold by id
        # before asserting "one subscriber" (a raw line count would count
        # every payment-link and status update).
        rows = list(self.read_watch_subscribers(os.path.dirname(subscribers_path)).values())
        target = next((row for row in rows if row.get("id") == ref), None)
        if target is not None and target.get("status") == "pending":
            # The corrected sign-up lands pending and collects the customer
            # billing profile on the mailed confirmation page before payment.
            # Confirm the address and store the profile while the row is still
            # pending, so the first payment's invoice is issued by the billing
            # run (not yet by the profile POST).
            token = self.watch_confirmation_token(WATCH_EMAIL)
            if not token:
                problems.append("the seeded watch sign-up mailed no confirmation token")
            else:
                # The confirmation link is inert on GET; the recipient's
                # POST is what consumes it and reveals the profile form.
                confirmed = http_req(
                    "POST",
                    base + "/watch/confirm",
                    headers={"Content-Type": "application/x-www-form-urlencoded"},
                    body=form_body([("token", token)]),
                )
                hidden = parse_hidden_fields(confirmed.text)
                profile_fields = {
                    "profile_token": hidden.get("profile_token") or "",
                    "reference": hidden.get("reference") or ref,
                    "legal_name": "Dogfood Watch Oy",
                    "address_lines": "Vahdontie 9",
                    "country": "FI",
                }
                profile = http_req(
                    "POST",
                    base + "/watch/billing-profile",
                    headers={"Content-Type": "application/x-www-form-urlencoded"},
                    body=form_body(sorted(profile_fields.items())),
                )
                if (
                    confirmed.status != 200
                    or not profile_fields["profile_token"]
                    or profile.status != 303
                ):
                    problems.append(
                        "the seeded watch confirmation/profile flow answered %d/%d (token=%s)"
                        % (confirmed.status, profile.status, bool(profile_fields["profile_token"]))
                    )
            # The corrected sign-up lands pending; the $99 line is billed only
            # after the payment webhook switches the subscriber on. Settle the
            # seeded row through the product's own signed webhook path.
            settlement = self.post_adyen_notification(
                "PSPDOGFOODDIFFERENTIAL", "apex-watch:" + ref, "card-dogfood-differential"
            )
            if settlement.status != 200:
                problems.append("the seeded subscriber's settlement answered %d" % settlement.status)
            else:
                try:
                    if settlement.json().get("new") != 1:
                        problems.append("the seeded subscriber's settlement was not recorded")
                except (ValueError, ReqError):
                    problems.append("the seeded subscriber's settlement returned no JSON")
            rows = list(self.read_watch_subscribers(os.path.dirname(subscribers_path)).values())
        active = [
            row
            for row in rows
            if row.get("id") == ref and row.get("publisher") == WATCH_PUBLISHER and row.get("status") == "active"
        ]
        if ref is None or len(active) != 1:
            problems.append("the seeded subscriber row (%s) is missing or duplicated" % ref)
        watch_cwd = os.path.join(self.workdir, "watch-billing")
        os.makedirs(watch_cwd, exist_ok=True)
        watch_env = {
            "APEX_WATCH_ROOT": os.path.join(self.api["work"], "watch"),
            "APEX_BILLING_PROFILE": self.fleet["seller"],
            "APEX_BILLING_VAT_RATE_BP": str(SELLER_VAT_RATE_BP),
            # The webhook recorded the card; the stub accepts the ContAuth
            # charge so the idempotent second run is the only skip.
            "APEX_ADYEN_API_KEY": "dogfood-live-adyen-key",
            "APEX_ADYEN_MERCHANT_ACCOUNT": API_ADYEN_MERCHANT_ACCOUNT,
            "APEX_ADYEN_CHECKOUT_BASE": self.api["stub"].base_url(),
            "APEX_ADYEN_HMAC_KEY": API_ADYEN_HMAC_KEY_HEX,
        }
        first_bill = self.watch_bill(period, watch_cwd, watch_env)
        second_bill = self.watch_bill(period, watch_cwd, watch_env)
        register_path = os.path.join(watch_cwd, ".apex", "billing", "invoices.jsonl")
        register_lines = []
        if os.path.exists(register_path):
            with open(register_path, "r", encoding="utf-8") as fh:
                register_lines = [line for line in fh.read().splitlines() if line.strip()]
        if first_bill is None or first_bill.get("issued") != 1:
            problems.append("watch bill run 1: %s (expected 1 issued)" % (first_bill or "failed"))
        elif second_bill is None or second_bill.get("issued") != 0 or second_bill.get("skipped") != 1:
            problems.append("watch bill run 2: %s (expected 0 issued, 1 skipped)" % (second_bill or "failed"))
        elif not register_lines or "setup-watch:%s" % period not in register_lines[0] or str(WATCH_FEE_MICROS) not in register_lines[0]:
            problems.append("the invoice register lacks the setup-watch $99 line for %s" % period)
        else:
            notes.append(
                "watch sign-up bills exactly once: setup-watch:%s net %d micros, second run skipped"
                % (period, WATCH_FEE_MICROS)
            )
        return (not problems, "; ".join(problems + notes))

    def watch_bill(self, period, cwd, env):
        merged = dict(os.environ)
        if env:
            merged.update(env)
        proc = self.spawn(
            [self.bin_path("apex-watch"), "bill", period],
            cwd=cwd,
            env=merged,
            capture_output=True,
            text=True,
            timeout=60,
        )
        out = (proc.stdout or "") + (proc.stderr or "")
        if proc.returncode != 0:
            return {
                "issued": 0,
                "refused": 0,
                "skipped": 0,
                "charged": 0,
                "returncode": proc.returncode,
                "output": out,
            }
        match = re.search(r"(\d+) issued, (\d+) refused, (\d+) skipped", out)
        charged = re.search(r"(\d+) charged", out)
        if not match:
            return {
                "issued": 0,
                "refused": 0,
                "skipped": 0,
                "charged": 0,
                "returncode": proc.returncode,
                "output": out,
            }
        return {
            "issued": int(match.group(1)),
            "refused": int(match.group(2)),
            "skipped": int(match.group(3)),
            "charged": int(charged.group(1)) if charged else 0,
            "returncode": proc.returncode,
            "output": out,
        }

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

    # 13 -------------------------------------------------- auth-surface
    def mint_console_session(self, publisher, token):
        """The ledger-token exchange, returning the raw response (its
        Set-Cookie path scope matters) and the session cookie id."""
        resp = http_req(
            "POST",
            self.base_url("apex-api") + "/console/ledger",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body([("publisher", publisher), ("token", token)]),
        )
        return resp, cookie_value(resp.header("set-cookie") or "", "apex_ledger_session")

    def open_console_session(self, publisher, token):
        """Mint a console session bound to `publisher` (the ledger-token
        exchange). Returns the session cookie id or None."""
        resp, cookie = self.mint_console_session(publisher, token)
        if resp.status != 303:
            return None
        return cookie

    def console_cookie(self, cookie):
        return {"Cookie": "apex_ledger_session=" + cookie} if cookie else {}

    def probe_auth_surface(self):
        """The `/v1/audit/{platform}` bearer gate (401 without it, 200 with it)
        and the console session gate (303 without a session, 200 with one)."""
        cells = self.cells_of("auth-surface")
        for service, surface, cell_id in cells:
            method, template = surface.split(" ", 1)
            base = self.base_url(service)
            path = materialize(template)
            problems = []
            detail = ""
            if service == "apex-api" and template == "/v1/audit/{platform}":
                is_get = method.upper() == "GET"
                scoped = base + path + "?publisher=" + API_PUBLISHER
                anon_headers = {} if is_get else {"Content-Type": "application/json"}
                anon_body = None if is_get else b"{}"
                anonymous = http_req(
                    method, scoped, headers=anon_headers, body=anon_body
                )
                if anonymous.status != 401:
                    problems.append(
                        "anonymous %s /v1/audit/admob answered %d, expected 401 (the report is behind the bearer)"
                        % (method, anonymous.status)
                    )
                headers = {"Authorization": "Bearer " + API_LIVE_AUDIT_TOKEN}
                body = None
                if not is_get:
                    headers["Content-Type"] = "application/json"
                    body = json.dumps({"publisher": API_PUBLISHER}).encode("utf-8")
                authorized = http_req(method, scoped, headers=headers, body=body)
                if authorized.status != 200:
                    problems.append(
                        "bearer %s /v1/audit/admob?publisher=%s answered %d, expected 200"
                        % (method, API_PUBLISHER, authorized.status)
                    )
                elif RIVAL_AUDIT_MARKER in authorized.text:
                    problems.append("the named publisher's report carries the rival's data")
                elif is_get and NORTHSTAR_AUDIT_MARKER not in authorized.text:
                    problems.append("the named publisher's report does not carry its own data")
                detail = "%s /v1/audit/admob?publisher=%s anon=%d bearer=%d" % (
                    method,
                    API_PUBLISHER,
                    anonymous.status,
                    authorized.status,
                )
                if is_get:
                    rival = http_req(
                        "GET",
                        base + path + "?publisher=" + RIVAL_PUBLISHER,
                        headers={"Authorization": "Bearer " + API_LIVE_AUDIT_TOKEN},
                    )
                    if rival.status != 200:
                        problems.append(
                            "the second publisher read answered %d, expected 200" % rival.status
                        )
                    elif RIVAL_AUDIT_MARKER not in rival.text:
                        problems.append("the second publisher read does not carry its own data")
                    elif NORTHSTAR_AUDIT_MARKER in rival.text:
                        problems.append("the second publisher read carries northstar's data")
                    detail += "; rival=%d" % rival.status
            elif service == "apex-api" and template == "/":
                anonymous = http_req("GET", base + path)
                gate = ("%s|%s" % (service, surface)) in self.session_required
                if gate:
                    if anonymous.status != 303:
                        problems.append(
                            "the overview answered %d without a session, expected 303" % anonymous.status
                        )
                    elif anonymous.header("location") != "/console/ledger":
                        problems.append(
                            "the overview's session gate Location %r, expected /console/ledger"
                            % anonymous.header("location")
                        )
                else:
                    problems.append("the overview is not recorded as session_required")
                minted, cookie = self.mint_console_session(API_PUBLISHER, API_LEDGER_TOKEN)
                if minted.status != 303 or not cookie:
                    problems.append("could not mint a console session for the overview")
                else:
                    jar = BrowserCookieJar()
                    jar.store(minted.header("set-cookie") or "")
                    jar_path = jar.header("/")
                    if not jar_path:
                        problems.append(
                            "the minted cookie jar does not send the session to / (Path scope)"
                        )
                    overview = http_req(
                        "GET", base + "/", headers={"Cookie": jar_path} if jar_path else {}
                    )
                    overview_issues = overview_problems(overview, "minted")
                    problems.extend(overview_issues)
                    if not overview_issues:
                        detail = "GET / anon=%d -> 303; jar session -> overview 200" % anonymous.status
                if not detail:
                    detail = "GET / anon=%d (session gate %s)" % (
                        anonymous.status,
                        "recorded" if gate else "missing",
                    )
            elif service == "apex-api" and template.startswith("/console"):
                anonymous = http_req("GET", base + path)
                gate = ("%s|%s" % (service, surface)) in self.session_required
                if gate:
                    if anonymous.status != 303:
                        problems.append(
                            "session-gated %s answered %d without a session, expected 303"
                            % (path, anonymous.status)
                        )
                    elif anonymous.header("location") != "/console/ledger":
                        problems.append(
                            "session gate %s Location %r, expected /console/ledger"
                            % (path, anonymous.header("location"))
                        )
                elif template in ("/console/ledger",):
                    if anonymous.status != 200:
                        problems.append(
                            "public access panel %s answered %d, expected 200" % (path, anonymous.status)
                        )
                elif anonymous.status not in (200, 303, 401):
                    problems.append("%s answered %d anonymously" % (path, anonymous.status))
                detail = "GET %s anon=%d (session gate %s)" % (
                    path,
                    anonymous.status,
                    "recorded" if gate else "public",
                )
            elif service == "apex-api" and template.startswith("/v1/"):
                headers = {"Content-Type": "application/json"}
                anonymous = http_req(method, base + path, headers=headers, body=b"")
                if anonymous.status != 401:
                    problems.append(
                        "%s %s answered %d without a bearer, expected 401"
                        % (method, path, anonymous.status)
                    )
                detail = "%s %s anon=%d" % (method, path, anonymous.status)
            else:
                anonymous = http_req(method, base + path)
                if anonymous.status not in (200, 303, 401):
                    problems.append("%s %s answered %d anonymously" % (method, path, anonymous.status))
                detail = "%s %s anon=%d" % (method, path, anonymous.status)
            if problems:
                detail += "; " + "; ".join(problems)
            self.cell(cell_id, not problems, detail, "%s %s" % (method, base + path))

    # 14 ------------------------------------------------- tenant-isolation
    def publisher_audit_text(self, cookie, platform="admob"):
        resp = http_req(
            "GET",
            self.base_url("apex-api") + "/console/audit?platform=" + platform,
            headers=self.console_cookie(cookie),
        )
        return resp

    def overview_records(self, cookie):
        resp = http_req("GET", self.base_url("apex-api") + "/", headers=self.console_cookie(cookie))
        match = re.search(
            r'kpi-label">Verified records</p><p class="kpi-value">(?:<a[^>]*>)?(\d+)',
            resp.text,
        )
        return resp, (int(match.group(1)) if match else None)

    def ledger_record_count(self, cookie):
        resp = http_req(
            "GET", self.base_url("apex-api") + "/console/ledger", headers=self.console_cookie(cookie)
        )
        links = len(re.findall(r"/console/ledger/records/\d+", resp.text))
        return resp, links

    def probe_tenant_isolation(self):
        """Seed two publishers and prove each session sees only its own
        snapshot and its own ledger totals."""
        base = self.base_url("apex-api")
        northstar = self.open_console_session(API_PUBLISHER, API_LEDGER_TOKEN)
        rival = self.open_console_session(RIVAL_PUBLISHER, RIVAL_TOKEN)
        if not northstar or not rival:
            for _service, _surface, cell_id in self.cells_of("tenant-isolation"):
                self.cell(
                    cell_id,
                    False,
                    "could not mint both publisher sessions (northstar=%r rival=%r)"
                    % (bool(northstar), bool(rival)),
                )
            return
        ns_audit = self.publisher_audit_text(northstar)
        rv_audit = self.publisher_audit_text(rival)
        ns_csv = http_req(
            "GET",
            base + "/console/audit/export.csv?platform=admob",
            headers=self.console_cookie(northstar),
        )
        rv_csv = http_req(
            "GET",
            base + "/console/audit/export.csv?platform=admob",
            headers=self.console_cookie(rival),
        )
        ns_over, ns_total = self.overview_records(northstar)
        rv_over, rv_total = self.overview_records(rival)
        ns_ledger, ns_links = self.ledger_record_count(northstar)
        rv_ledger, rv_links = self.ledger_record_count(rival)
        v1_audit = http_req(
            "GET",
            base + "/v1/audit/admob?publisher=" + API_PUBLISHER,
            headers={"Authorization": "Bearer " + API_LIVE_AUDIT_TOKEN},
        )
        v1_audit_rival = http_req(
            "GET",
            base + "/v1/audit/admob?publisher=" + RIVAL_PUBLISHER,
            headers={"Authorization": "Bearer " + API_LIVE_AUDIT_TOKEN},
        )
        v1_ledger_ns = http_req(
            "GET", base + "/v1/ledger/" + API_PUBLISHER, headers={"Authorization": "Bearer " + API_LEDGER_TOKEN}
        )
        v1_ledger_rv = http_req(
            "GET", base + "/v1/ledger/" + RIVAL_PUBLISHER, headers={"Authorization": "Bearer " + RIVAL_TOKEN}
        )

        def snapshot_problems(label, session_text, own, other):
            problems = []
            if own not in session_text:
                problems.append("%s does not show its own snapshot marker %s" % (label, own))
            if other in session_text:
                problems.append("%s carries the other publisher's snapshot marker %s" % (label, other))
            return problems

        northstar_records = count_lines(
            os.path.join(self.fleet["ledgers"], API_PUBLISHER + ".jsonl")
        )
        rival_records = count_lines(os.path.join(self.fleet["ledgers"], RIVAL_PUBLISHER + ".jsonl"))

        for service, surface, cell_id in self.cells_of("tenant-isolation"):
            problems = []
            detail = ""
            if surface == "GET /console/audit":
                problems += snapshot_problems(
                    "northstar session", ns_audit.text, NORTHSTAR_AUDIT_MARKER, RIVAL_AUDIT_MARKER
                )
                problems += snapshot_problems(
                    "rival session", rv_audit.text, RIVAL_AUDIT_MARKER, NORTHSTAR_AUDIT_MARKER
                )
                detail = "northstar/audit=%d rival/audit=%d" % (ns_audit.status, rv_audit.status)
            elif surface == "GET /console/audit/export.csv":
                problems += snapshot_problems(
                    "northstar CSV", ns_csv.text, NORTHSTAR_AUDIT_MARKER, RIVAL_AUDIT_MARKER
                )
                problems += snapshot_problems(
                    "rival CSV", rv_csv.text, RIVAL_AUDIT_MARKER, NORTHSTAR_AUDIT_MARKER
                )
                detail = "northstar/csv=%d rival/csv=%d" % (ns_csv.status, rv_csv.status)
            elif surface == "GET /":
                if ns_total != northstar_records:
                    problems.append(
                        "northstar overview shows %r verified records, expected its own %d"
                        % (ns_total, northstar_records)
                    )
                if rv_total != rival_records:
                    problems.append(
                        "rival overview shows %r verified records, expected its own %d"
                        % (rv_total, rival_records)
                    )
                detail = "northstar/overview=%r (own %d) rival/overview=%r (own %d)" % (
                    ns_total,
                    northstar_records,
                    rv_total,
                    rival_records,
                )
            elif surface == "GET /console/ledger":
                if ns_links != northstar_records:
                    problems.append(
                        "northstar ledger page links %d records, expected its own %d"
                        % (ns_links, northstar_records)
                    )
                if rv_links != rival_records:
                    problems.append(
                        "rival ledger page links %d records, expected its own %d"
                        % (rv_links, rival_records)
                    )
                detail = "northstar/ledger=%d rival/ledger=%d" % (ns_links, rv_links)
            elif surface == "GET /v1/audit/{platform}":
                if v1_audit.status != 200 or v1_audit_rival.status != 200:
                    problems.append(
                        "/v1/audit/admob answered northstar=%d rival=%d"
                        % (v1_audit.status, v1_audit_rival.status)
                    )
                else:
                    if NORTHSTAR_AUDIT_MARKER not in v1_audit.text:
                        problems.append("the northstar read does not carry northstar's own data")
                    if RIVAL_AUDIT_MARKER in v1_audit.text:
                        problems.append("the northstar read exposes the rival snapshot")
                    if RIVAL_AUDIT_MARKER not in v1_audit_rival.text:
                        problems.append("the rival read does not carry rival's own data")
                    if NORTHSTAR_AUDIT_MARKER in v1_audit_rival.text:
                        problems.append("the rival read exposes the northstar snapshot")
                detail = "/v1/audit northstar=%d rival=%d" % (
                    v1_audit.status,
                    v1_audit_rival.status,
                )
            elif surface == "POST /v1/audit/{platform}":
                # The write route is live and would overwrite the read-only
                # snapshot the isolation cells compare, so its cell pins the
                # two contract refusals that never write: the auth gate and
                # the required publisher in the body.
                anonymous_post = http_req(
                    "POST",
                    base + "/v1/audit/admob",
                    headers={"Content-Type": "application/json"},
                    body=b'{"publisher":"%s"}' % API_PUBLISHER.encode(),
                )
                missing_publisher = http_req(
                    "POST",
                    base + "/v1/audit/admob",
                    headers={
                        "Content-Type": "application/json",
                        "Authorization": "Bearer " + API_LIVE_AUDIT_TOKEN,
                    },
                    body=b"{}",
                )
                if anonymous_post.status != 401:
                    problems.append(
                        "anonymous POST /v1/audit/admob -> %d (expected 401)"
                        % anonymous_post.status
                    )
                if missing_publisher.status != 400:
                    problems.append(
                        "bearer POST without a publisher -> %d (expected 400)"
                        % missing_publisher.status
                    )
                detail = "POST /v1/audit/admob anon=%d no-publisher=%d" % (
                    anonymous_post.status,
                    missing_publisher.status,
                )
            elif surface == "GET /v1/ledger/{publisher}":
                if v1_ledger_ns.status != 200 or v1_ledger_rv.status != 200:
                    problems.append(
                        "/v1/ledger readers answered %d/%d"
                        % (v1_ledger_ns.status, v1_ledger_rv.status)
                    )
                else:
                    ns_names = set(re.findall(r'"publisher":"([^"]+)"', v1_ledger_ns.text))
                    rv_names = set(re.findall(r'"publisher":"([^"]+)"', v1_ledger_rv.text))
                    if ns_names and ns_names != {API_PUBLISHER}:
                        problems.append("northstar bearer read named %s" % sorted(ns_names))
                    if rv_names and rv_names != {RIVAL_PUBLISHER}:
                        problems.append("rival bearer read named %s" % sorted(rv_names))
                detail = "/v1/ledger northstar=%d rival=%d" % (
                    v1_ledger_ns.status,
                    v1_ledger_rv.status,
                )
            if problems:
                detail += "; " + "; ".join(problems)
            self.cell(cell_id, not problems, detail)

    # 15 ------------------------------------------------- signin-ownership
    def read_mail_outbox(self):
        path = self.api.get("mail_outbox")
        if not path or not os.path.exists(path):
            return ""
        with open(path, "r", encoding="utf-8", errors="replace") as fh:
            return fh.read()

    def clear_mail_outbox(self):
        path = self.api.get("mail_outbox")
        if path:
            with open(path, "w", encoding="utf-8"):
                pass

    def append_signin_row(self, token, publisher="northstar", ttl_ms=72 * 3_600_000):
        """Append one hash-only sign-in row in the product's own format.
        The raw token lives only in the harness, exactly like a mailed link."""
        now_ms = int(time.time() * 1000)
        row = {
            "at": now_ms,
            "publisher": publisher,
            "token_hash": hashlib.sha256(token.encode("utf-8")).hexdigest(),
            "expires_at": now_ms + ttl_ms,
            "used_at": None,
        }
        path = os.path.join(self.api["work"], "signin.jsonl")
        with open(path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(row) + "\n")

    def probe_signin_ownership(self):
        """The single-sign-on model: the absolute one-time link, the inert
        GET, the POST exchange, the session at `/`, the configured absolute
        Help destination on console pages, the absolute public-site Help on
        the marketing forms, and the links-disabled deployment (a saved lead
        with no sign-in row, no email and the typed warning)."""
        results = {
            "GET /console/signin": {"problems": [], "notes": []},
            "POST /console/signin": {"problems": [], "notes": []},
            "POST /free-audit": {"problems": [], "notes": []},
            "GET /free-audit/": {"problems": [], "notes": []},
            "GET /watch": {"problems": [], "notes": []},
            "GET /console/audit": {"problems": [], "notes": []},
        }

        def add(surface, ok, detail):
            entry = results.setdefault(surface, {"problems": [], "notes": []})
            (entry["notes"] if ok else entry["problems"]).append(detail)

        base = self.base_url("apex-api")

        # 1. An existing workspace never sends its link to a form-supplied
        # attacker address.
        self.clear_mail_outbox()
        attacker = "attacker@evil.example"
        resp = http_req(
            "POST",
            base + "/free-audit",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(
                [
                    ("studio", "Northstar"),
                    ("email", attacker),
                    ("platforms", "admob"),
                    ("revenue", "10k-50k"),
                    ("notes", "dogfood ownership probe"),
                ]
            ),
        )
        if resp.status not in (303, 200):
            add("POST /free-audit", False, "free-audit for the own attacker answered %d" % resp.status)
        outbox = self.read_mail_outbox()
        add(
            "POST /free-audit",
            attacker not in outbox,
            "an existing workspace delivered its link to the form-supplied attacker"
            if attacker in outbox
            else "existing workspace refused the attacker",
        )

        # 2. A new workspace still needs the email domain's app-ads proof.
        self.clear_mail_outbox()
        intruder = "intruder@evil.example"
        http_req(
            "POST",
            base + "/free-audit",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(
                [
                    ("studio", "Brand New Studio"),
                    ("email", intruder),
                    ("platforms", "admob"),
                    ("revenue", "10k-50k"),
                    ("notes", "dogfood ownership probe"),
                ]
            ),
        )
        outbox = self.read_mail_outbox()
        add(
            "POST /free-audit",
            intruder not in outbox,
            "a brand-new workspace delivered a link to an unproven-domain email %s"
            if intruder in outbox
            else "new workspace refused the unproven domain",
        )

        # 3. Two different studio names sharing one proven email domain map
        # to ONE workspace keyed by that domain (the studio stays a label).
        self.clear_mail_outbox()
        studios = [
            ("Alpha Studio", "alpha@" + WORKSPACE_PROVEN_DOMAIN),
            ("Beta Studio", "beta@" + WORKSPACE_PROVEN_DOMAIN),
        ]
        confirmations = []
        submissions = []
        for studio, email in studios:
            submitted = http_req(
                "POST",
                base + "/free-audit",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=form_body(
                    [
                        ("studio", studio),
                        ("email", email),
                        ("platforms", "admob"),
                        ("revenue", "10k-50k"),
                        ("notes", "dogfood workspace keying probe"),
                    ]
                ),
            )
            location = submitted.header("location") or ""
            submissions.append((submitted.status, location))
            if location:
                confirmation = http_req("GET", base + location)
                confirmations.append(confirmation.status)
            else:
                confirmations.append(None)
        add(
            "POST /free-audit",
            all(
                status == 303 and location.startswith("/audit-requested/?ref=")
                for status, location in submissions
            ),
            "domain submissions -> %s" % [status for status, _location in submissions],
        )
        add(
            "POST /free-audit",
            all(status == 200 for status in confirmations) and len(confirmations) == 2,
            "confirmations -> %s" % confirmations,
        )

        tokens_path = os.path.join(self.api["work"], "tokens.json")
        tokens = {}
        if os.path.exists(tokens_path):
            try:
                with open(tokens_path, "r", encoding="utf-8") as fh:
                    tokens = json.load(fh)
            except ValueError:
                tokens = {}
        domain_keys = [
            key
            for key in tokens
            if workspace_key_matches(key, WORKSPACE_DOMAIN_SLUG)
            or workspace_key_matches(key, WORKSPACE_PROVEN_DOMAIN)
        ]
        studio_keys = [key for key in tokens if key in ("alpha-studio", "beta-studio")]
        add(
            "POST /free-audit",
            len(domain_keys) == 1 and not studio_keys,
            "workspace keys domain=%s studio=%s"
            % (domain_keys, studio_keys),
        )
        workspace_key = domain_keys[0] if len(domain_keys) == 1 else None

        leads_path = os.path.join(self.api["work"], "leads.jsonl")
        leads = []
        if os.path.exists(leads_path):
            with open(leads_path, "r", encoding="utf-8") as fh:
                for line in fh:
                    if not line.strip():
                        continue
                    try:
                        leads.append(json.loads(line))
                    except ValueError:
                        continue
        workspace_leads = [
            row.get("lead", {})
            for row in leads
            if (row.get("lead") or {}).get("publisher") == (workspace_key or WORKSPACE_DOMAIN_SLUG)
        ]
        labels = {
            row.get("displayName", row.get("display_name"))
            for row in workspace_leads
        }
        add(
            "POST /free-audit",
            len(workspace_leads) >= 2 and {"Alpha Studio", "Beta Studio"}.issubset(labels),
            "domain leads=%d labels=%s"
            % (len(workspace_leads), sorted(label for label in labels if label)),
        )

        signin_path = os.path.join(self.api["work"], "signin.jsonl")
        signin_publishers = []
        if os.path.exists(signin_path):
            with open(signin_path, "r", encoding="utf-8") as fh:
                for line in fh:
                    if not line.strip():
                        continue
                    try:
                        row = json.loads(line)
                    except ValueError:
                        continue
                    if row.get("publisher"):
                        signin_publishers.append(row["publisher"])
        add(
            "POST /free-audit",
            workspace_key is not None
            and workspace_key in signin_publishers
            and not any(key in signin_publishers for key in ("alpha-studio", "beta-studio")),
            "sign-in rows name %r (studio slugs absent)"
            % workspace_key,
        )

        # 3a. The primary instance's link is absolute and exactly
        # `{base}/console/signin?token=<32 hex>`: the base is the instance's
        # own origin, never a relative path a mail client cannot resolve.
        outbox = self.read_mail_outbox()
        link_matches = list(SIGNIN_LINK_PATTERN.finditer(outbox))
        expected_link_prefix = base + "/console/signin?token="
        absolute_links = [match.group(0) for match in link_matches]
        distinct_tokens = sorted({match.group(1) for match in link_matches})
        link_problems = []
        if not link_matches:
            link_problems.append("no absolute /console/signin?token=<32 hex> link was delivered")
        for match in link_matches:
            if not match.group(0).startswith(expected_link_prefix):
                link_problems.append(
                    "link %r does not start with the configured base %r"
                    % (match.group(0), expected_link_prefix)
                )
            if len(match.group(1)) != 32:
                link_problems.append("link token is %d hex characters, expected 32" % len(match.group(1)))
        if 'href="/console/signin?token=' in outbox or '"href": "/console/signin?token=' in outbox:
            link_problems.append("the delivered message carries a relative sign-in link")
        add(
            "GET /console/signin",
            not link_problems,
            "emailed link token(s) %d -> %s"
            % (
                len(distinct_tokens),
                "; ".join(link_problems) if link_problems else absolute_links[-1],
            ),
        )

        # 3b. The marketing shell keeps the on-site relative Help link: only
        # the authenticated console uses the configured absolute URL.
        for surface, path in (("GET /free-audit/", "/free-audit/"), ("GET /watch", "/watch")):
            page = http_req("GET", base + path)
            initial_status = page.status
            if page.status == 303 and page.header("location"):
                page = http_req(
                    "GET", urllib.parse.urljoin(base + path, page.header("location"))
                )
            # Marketing-shell pages are rendered by apex-api on the API and
            # console origins, where relative site paths are dead. They link
            # the public site absolutely; the configured console Help URL is
            # for authenticated console pages only.
            site_support = 'href="https://apexmediation.ee/support/"'
            marketing_ok = (
                page.status == 200
                and site_support in page.text
                and 'href="/support/"' not in page.text
                and API_SUPPORT_URL not in page.text
            )
            add(
                surface,
                marketing_ok,
                "%s -> HTTP %d%s, absolute site Help=%s, relative Help=%s, console Help=%s"
                % (
                    path,
                    initial_status,
                    (" then %d" % page.status) if page.status != initial_status else "",
                    site_support in page.text,
                    'href="/support/"' in page.text,
                    API_SUPPORT_URL in page.text,
                ),
            )

        # 4. Scanner-safe GET: repeated reads render the sign-in button and
        # never consume the token; the POST consumes exactly once.
        scanner_token = "e5" * 16
        self.append_signin_row(scanner_token)
        get_pages = [
            http_req("GET", base + "/console/signin?token=" + scanner_token)
            for _ in range(3)
        ]
        get_ok = True
        submitted_fields = {}
        for index, page in enumerate(get_pages, 1):
            hidden = parse_hidden_fields(page.text)
            form_action = re.search(
                r'<form[^>]*method="post"[^>]*action="/console/signin"', page.text, re.I
            ) or re.search(
                r'<form[^>]*action="/console/signin"[^>]*method="post"', page.text, re.I
            )
            button = "sign in" in page.text.lower()
            hidden_token = hidden.get("token") or hidden.get("signin_token")
            if hidden_token == scanner_token and not submitted_fields:
                submitted_fields = hidden
            if (
                page.status != 200
                or not form_action
                or hidden_token != scanner_token
                or not button
                or page.header("set-cookie")
            ):
                get_ok = False
            add(
                "GET /console/signin",
                page.status == 200
                and bool(form_action)
                and hidden_token == scanner_token
                and bool(button)
                and not page.header("set-cookie"),
                "GET #%d -> HTTP %d, form=%s, hidden token=%s, button=%s, cookie=%s"
                % (
                    index,
                    page.status,
                    bool(form_action),
                    hidden_token == scanner_token,
                    bool(button),
                    bool(page.header("set-cookie")),
                ),
            )
        add(
            "GET /console/signin",
            get_ok,
            "repeated GETs render the sign-in page without consuming the token",
        )

        posted = http_req(
            "POST",
            base + "/console/signin",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(sorted(submitted_fields.items())),
        )
        set_cookie = posted.header("set-cookie") or ""
        session = cookie_value(set_cookie, "apex_ledger_session")
        cookie_problems = console_session_cookie_problems(set_cookie, "sign-in exchange")
        if "Max-Age=" not in set_cookie:
            cookie_problems.append("sign-in exchange cookie lacks Max-Age=")
        add(
            "POST /console/signin",
            posted.status == 303
            and (posted.header("location") or "") == "/"
            and bool(session)
            and not cookie_problems,
            "POST -> HTTP %d %s, session=%s, cookie=%s%s"
            % (
                posted.status,
                posted.header("location"),
                bool(session),
                set_cookie,
                ("; " + "; ".join(cookie_problems)) if cookie_problems else "",
            ),
        )
        if session:
            jar = BrowserCookieJar()
            jar.store(set_cookie)
            jar_path = jar.header("/")
            if not jar_path:
                add(
                    "POST /console/signin",
                    False,
                    "the sign-in-minted cookie jar does not send the session to / (Path scope)",
                )
            else:
                overview = http_req("GET", base + "/", headers={"Cookie": jar_path})
                overview_issues = overview_problems(overview, "sign-in-minted")
                add(
                    "POST /console/signin",
                    not overview_issues,
                    "sign-in-minted jar GET / -> HTTP %d%s"
                    % (
                        overview.status,
                        ("; " + "; ".join(overview_issues)) if overview_issues else " (overview)",
                    ),
                )
        # The authenticated console uses the configured absolute Help
        # destination on every page; the marketing shell above keeps the
        # relative on-site link. If the sign-in exchange could not mint a
        # session (a pre-change binary answers the GET instead), the existing
        # ledger-token exchange supplies one so the page itself is still
        # exercised rather than skipped.
        support_cookie = session
        support_header = set_cookie
        if not support_cookie:
            minted, support_cookie = self.mint_console_session(API_PUBLISHER, API_LEDGER_TOKEN)
            support_header = minted.header("set-cookie") or ""
        support_jar_path = None
        if support_cookie:
            support_jar = BrowserCookieJar()
            support_jar.store(support_header)
            support_jar_path = support_jar.header("/")
        if support_jar_path:
            support_overview = http_req(
                "GET", base + "/", headers={"Cookie": support_jar_path}
            )
            audit_page = http_req(
                "GET", base + "/console/audit", headers={"Cookie": support_jar_path}
            )
            support_ok = (
                audit_page.status == 200
                and API_SUPPORT_URL in audit_page.text
                and 'href="/support/"' not in audit_page.text
                and API_SUPPORT_URL in support_overview.text
            )
            add(
                "GET /console/audit",
                support_ok,
                "session GET /console/audit -> HTTP %d, absolute Help=%s, relative Help=%s (overview absolute=%s)"
                % (
                    audit_page.status,
                    API_SUPPORT_URL in audit_page.text,
                    'href="/support/"' in audit_page.text,
                    API_SUPPORT_URL in support_overview.text,
                ),
            )
        else:
            add(
                "GET /console/audit",
                False,
                "no console session could be minted, so the configured Help destination was not checked",
            )

        second = http_req(
            "POST",
            base + "/console/signin",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(sorted(submitted_fields.items())),
        )
        second_cookie = second.header("set-cookie") or ""
        refused_copy = "Sign-in refused." in second.text or "refused" in second.text.lower()
        add(
            "POST /console/signin",
            "apex_ledger_session=" not in second_cookie
            and (refused_copy or second.status >= 400),
            "a replayed POST -> HTTP %d, refusal copy=%s, cookie=%s"
            % (
                second.status,
                refused_copy,
                bool(second_cookie),
            ),
        )
        after = http_req("GET", base + "/console/signin?token=" + scanner_token)
        add(
            "GET /console/signin",
            after.status in (200, 400) and not after.header("set-cookie"),
            "GET after the POST -> HTTP %d, cookie=%s (the token is spent, no session minted)"
            % (after.status, bool(after.header("set-cookie"))),
        )

        # 5. The links-disabled deployment: the same full lead/workspace/email
        # wiring with only APEX_CONSOLE_URL absent. The audit still accepts and
        # saves the lead, but no sign-in row is created and no email is sent;
        # the startup warning and the typed MissingBase refusal are both
        # recorded in the operator log.
        off_base = self.base_url("apex-api-links-off")
        off_signin = self.api["links_off_signin"]
        off_outbox = self.api["links_off_outbox"]
        off_log = self.api["links_off_log"]

        def read_file(path):
            if not os.path.exists(path):
                return ""
            with open(path, "r", encoding="utf-8", errors="replace") as fh:
                return fh.read()

        startup_log = read_file(off_log)
        startup_warning = "sign-in links are disabled: APEX_CONSOLE_URL is not configured"
        add(
            "POST /free-audit",
            startup_warning in startup_log,
            "links off: startup warning=%s" % (startup_warning in startup_log),
        )
        rows_before = count_lines(off_signin)
        off_submitted = http_req(
            "POST",
            off_base + "/free-audit",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(
                [
                    ("studio", "Northstar"),
                    ("email", LINKS_OFF_CONTACT),
                    ("platforms", "admob"),
                    ("revenue", "10k-50k"),
                    ("notes", "dogfood links-disabled probe"),
                ]
            ),
        )
        off_location = off_submitted.header("location") or ""
        rows_after = count_lines(off_signin)
        off_outbox_text = read_file(off_outbox)
        off_log_text = read_file(off_log)
        disabled_problems = []
        if off_submitted.status != 303 or not off_location.startswith("/audit-requested/?ref="):
            disabled_problems.append(
                "free audit answered %d %s, expected the saved-lead 303"
                % (off_submitted.status, off_location)
            )
        if rows_before != 0 or rows_after != 0:
            disabled_problems.append(
                "signin.jsonl rows before=%d after=%d, expected zero" % (rows_before, rows_after)
            )
        if "===DOGFOOD-MESSAGE-END===" in off_outbox_text:
            disabled_problems.append("a sign-in email was delivered with the base unset")
        typed_refusal = (
            "sign-in link base is not configured" in off_log_text
            or "missing_base" in off_log_text
        )
        if not typed_refusal:
            disabled_problems.append(
                "the typed MissingBase refusal (\"sign-in link base is not configured\") was not recorded"
            )
        add(
            "POST /free-audit",
            not disabled_problems,
            "links off: audit -> %d %s; rows %d->%d; email=%s; MissingBase=%s%s"
            % (
                off_submitted.status,
                off_location,
                rows_before,
                rows_after,
                "===DOGFOOD-MESSAGE-END===" in off_outbox_text,
                typed_refusal,
                ("; " + "; ".join(disabled_problems)) if disabled_problems else "",
            ),
        )

        for _service, cell_surface, cell_id in self.cells_of("signin-ownership"):
            entry = results.get(cell_surface)
            if entry is None:
                self.cell(
                    cell_id,
                    False,
                    "no signin-ownership probe covers %s" % cell_surface,
                )
                continue
            detail = "; ".join(entry["notes"] + entry["problems"]) or "no observation"
            self.cell(cell_id, not entry["problems"], detail)

    # 16 ------------------------------------------------ payment-activation
    def read_watch_subscribers(self, watch_root):
        path = os.path.join(watch_root, "subscribers.jsonl")
        folded = {}
        if os.path.exists(path):
            with open(path, "r", encoding="utf-8") as fh:
                for line in fh:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        row = json.loads(line)
                    except ValueError:
                        continue
                    if row.get("id"):
                        folded[row["id"]] = row
        return folded

    def watch_signup(self, domain, email, accounts, apps=""):
        # The form fields the frozen contract names: `apps` is the tier
        # evidence; there is no tier or app-count input.
        fields = [
            ("domain", domain),
            ("email", email),
            ("accounts", accounts),
        ]
        if apps:
            fields.append(("apps", apps))
        return http_req(
            "POST",
            self.base_url("apex-api") + "/watch",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(fields),
        )

    def watch_confirmation_token(self, email):
        """The newest one-time confirmation token mailed to `email`.

        A repeat submission inside the dedup window keeps the subscriber row
        but mints a fresh token and invalidates the previous one, so the last
        mailed link is the live one."""
        outbox = self.read_mail_outbox()
        token = None
        for block in outbox.split("===DOGFOOD-MESSAGE-END==="):
            if email not in block:
                continue
            match = re.search(r"/watch/confirm\?token=([0-9a-f]{32})", block)
            if match:
                token = match.group(1)
        return token

    def post_adyen_notification(self, psp_reference, merchant_reference, stored_card):
        """Sign and post one Adyen AUTHORISATION notification through the
        product's own HMAC-verified webhook route."""
        item = {
            "pspReference": psp_reference,
            "originalReference": "",
            "merchantAccountCode": API_ADYEN_MERCHANT_ACCOUNT,
            "merchantReference": merchant_reference,
            "amount": {"value": WATCH_FEE_MICROS // 10_000, "currency": "USD"},
            "eventCode": "AUTHORISATION",
            "success": "true",
        }
        body = {
            "live": "false",
            "notificationItems": [
                {
                    "NotificationRequestItem": {
                        **item,
                        "additionalData": {
                            "hmacSignature": adyen_sign(API_ADYEN_HMAC_KEY_HEX, item),
                            "storedPaymentMethodId": stored_card,
                        },
                    }
                }
            ],
        }
        return http_req(
            "POST",
            self.base_url("apex-api") + "/v1/adyen/webhook",
            headers={"Content-Type": "application/json"},
            body=json.dumps(body).encode("utf-8"),
        )

    def run_watch_daily(self, watch_root, extra_env=None, path_dir=None):
        env = dict(os.environ)
        env.update(
            {
                "APEX_WATCH_ROOT": watch_root,
                "APEX_NOTIFY_SINK": "file",
                "APEX_NOTIFY_FILE": os.path.join(self.workdir, "watch-alerts.jsonl"),
            }
        )
        if extra_env:
            env.update(extra_env)
        if path_dir:
            env["PATH"] = path_dir + os.pathsep + env.get("PATH", "")
        return self.spawn(
            [self.bin_path("apex-watch"), "daily"],
            cwd=self.root,
            env=env,
            capture_output=True,
            text=True,
            timeout=180,
        )

    def watch_checked_count(self, proc):
        out = (proc.stdout or "") + (proc.stderr or "")
        match = re.search(r"(\d+) checked", out)
        return int(match.group(1)) if match else None

    def probe_payment_activation(self):
        """The corrected setup-watch flow end to end: the APPS tier boundary,
        the one-time billing-profile collection on the confirmation page, the
        first invoiced payment, the monthly renewal charge, hostile profile
        refusals, and the no-profile typed refusal."""
        results = {
            surface: {"problems": [], "notes": [], "checks": []}
            for surface in (
                "GET /watch/",
                "POST /watch",
                "GET /watch/confirm",
                "POST /watch/billing-profile",
                "GET /watch/requested",
                "POST /v1/adyen/webhook",
            )
        }

        def add(surface, ok, detail):
            entry = results[surface]
            entry["checks"].append((ok, detail))
            if ok:
                entry["notes"].append(detail)
            else:
                entry["problems"].append(detail)

        base = self.base_url("apex-api")
        watch_root = os.path.join(self.api["work"], "watch")
        os.makedirs(watch_root, exist_ok=True)

        # 1. The sign-up form names optional apps, not a tier/account count.
        form = http_req("GET", base + "/watch/")
        has_apps = re.search(r'<textarea[^>]*\bname="apps"', form.text) or re.search(
            r'<input[^>]*\bname="apps"', form.text
        )
        forbidden = re.search(r'name="(?:tier|app_count)"', form.text)
        add(
            "GET /watch/",
            form.status == 200 and bool(has_apps) and not forbidden,
            "sign-up form HTTP %d; apps field=%s; tier/app_count field=%s"
            % (form.status, bool(has_apps), bool(forbidden)),
        )

        # 2. The 9/10 distinct-app boundary. Account ids are sent in both
        # directions: many accounts with nine apps stays Normal, no accounts
        # with ten apps is Core.
        tier_rows = {}
        for domain, app_count, account_count, expected_tier in (
            ("tier-nine.example", WATCH_APPS_CORE_MIN - 1, 20, "normal"),
            # Self-reported app ids cannot buy the Core tier: verification
            # against the public store listing decides, and these fixture
            # URLs have no listing, so the honest tier stays normal.
            ("tier-ten.example", WATCH_APPS_CORE_MIN, 0, "normal"),
        ):
            email = "owner@" + domain
            apps = "\n".join(
                "https://apps.example/%s/app-%d" % (domain, index)
                for index in range(app_count)
            )
            accounts = "\n".join(
                "network%d.example = pub-%d" % (index, index)
                for index in range(account_count)
            )
            resp = self.watch_signup(domain, email, accounts, apps=apps)
            rows = self.read_watch_subscribers(watch_root)
            row = next((r for r in rows.values() if r.get("domain") == domain), None)
            tier_rows[domain] = row
            if resp.status != 303:
                add("POST /watch", False, "app-tier sign-up %s answered %d (expected 303)" % (domain, resp.status))
            elif row is None:
                add("POST /watch", False, "app-tier sign-up %s recorded no subscriber" % domain)
            elif row.get("tier") != expected_tier:
                add(
                    "POST /watch",
                    False,
                    "%d distinct apps with %d account ids -> tier %r, expected %r"
                    % (app_count, account_count, row.get("tier"), expected_tier),
                )
            else:
                add(
                    "POST /watch",
                    True,
                    "%d apps / %d account ids -> %s" % (app_count, account_count, expected_tier),
                )

        # 2b. The optional apps textarea is capped at 50 lines.
        over_domain = "tier-over.example"
        over_apps = "\n".join(
            "https://apps.example/%s/app-%d" % (over_domain, index)
            for index in range(WATCH_APPS_MAX + 1)
        )
        over = self.watch_signup(over_domain, "owner@" + over_domain, "", apps=over_apps)
        over_rows = [
            row
            for row in self.read_watch_subscribers(watch_root).values()
            if row.get("domain") == over_domain
        ]
        add(
            "POST /watch",
            over.status in (400, 422) and not over_rows,
            "%d apps -> HTTP %d, stored=%s (the form caps apps at %d)"
            % (WATCH_APPS_MAX + 1, over.status, bool(over_rows), WATCH_APPS_MAX),
        )

        # 3. Two ordinary sign-ups for the profile/renewal flow.
        domain_a = "pending-a.example"
        email_a = "owner@pending-a.example"
        domain_b = "pending-b.example"
        email_b = "owner@pending-b.example"
        self.clear_mail_outbox()
        resp_a = self.watch_signup(domain_a, email_a, "admob.com = pub-1")
        resp_b = self.watch_signup(domain_b, email_b, "admob.com = pub-2")
        if resp_a.status != 303:
            add("POST /watch", False, "watch sign-up answered %d, expected 303" % resp_a.status)
        if resp_b.status != 303:
            add("POST /watch", False, "watch sign-up (b) answered %d, expected 303" % resp_b.status)
        rows = self.read_watch_subscribers(watch_root)
        by_domain = {
            row.get("domain"): row
            for row in rows.values()
            if row.get("domain") in (domain_a, domain_b)
        }
        for domain in (domain_a, domain_b):
            row = by_domain.get(domain)
            if row is None:
                add("POST /watch", False, "no subscriber row was recorded for %s" % domain)
                continue
            if row.get("status") != "pending":
                add(
                    "POST /watch",
                    False,
                    "the sign-up for %s landed %r; it must be pending until payment"
                    % (domain, row.get("status")),
                )
            if row.get("lastCheckAt"):
                add("POST /watch", False, "the pending sign-up for %s already ran a check" % domain)
            if row.get("tier") != "normal":
                add(
                    "POST /watch",
                    False,
                    "the tier for %s is %r; no apps evidence must be normal" % (domain, row.get("tier")),
                )
            if row.get("paymentLink") is not None:
                add(
                    "POST /watch",
                    False,
                    "the sign-up for %s already records payment link %r; the profile must come first"
                    % (domain, row.get("paymentLink")),
                )
            if row.get("shopperReference") != row.get("id"):
                add(
                    "POST /watch",
                    False,
                    "the sign-up for %s does not record its id as the shopper reference" % domain,
                )

        # 4. The confirmation page renders the checkout link and the
        # one-time billing-profile form (hidden profile_token + reference).
        requested = http_req(
            "GET",
            base + "/watch/requested?ref=" + str((by_domain.get(domain_a) or {}).get("id", "")),
        )
        add(
            "GET /watch/requested",
            requested.status == 200
            and API_ADYEN_CHECKOUT_LINK not in requested.text
            and "Your watch is requested." in requested.text,
            "watch sign-up redirect=%d; the checkout link must wait for the profile; page=%d"
            % (resp_a.status, requested.status),
        )
        tokens = {
            domain_a: self.watch_confirmation_token(email_a),
            domain_b: self.watch_confirmation_token(email_b),
        }
        confirmations = {}
        profile_pages = {}
        for domain in (domain_a, domain_b):
            token = tokens[domain]
            if not token:
                add("GET /watch/confirm", False, "no confirmation token was mailed for %s" % domain)
                continue
            first = http_req(
                "POST",
                base + "/watch/confirm",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=form_body([("token", token)]),
            )
            confirmations[domain] = first
            if first.status != 200 or "Confirm your billing details." not in first.text:
                add(
                    "POST /watch/confirm",
                    False,
                    "confirming %s answered %d without the billing-profile form" % (domain, first.status),
                )
            ref = (by_domain.get(domain) or {}).get("id") or ""
            page = first
            hidden = parse_hidden_fields(page.text) if "billing-profile" in page.text else {}
            if not hidden.get("profile_token"):
                # The fixed flow may place the profile form on the requested
                # receipt instead of the confirmation page; both are honest
                # states of the same confirmation flow.
                fallback = http_req("GET", base + "/watch/requested?ref=" + ref)
                if "billing-profile" in fallback.text:
                    page = fallback
                    hidden = parse_hidden_fields(page.text)
            profile_pages[domain] = (page, hidden)
            if not hidden.get("profile_token"):
                add(
                    "GET /watch/confirm",
                    False,
                    "the confirmation page for %s renders no POST /watch/billing-profile form with a hidden profile_token"
                    % domain,
                )
            elif not hidden.get("reference"):
                add(
                    "GET /watch/confirm",
                    False,
                    "the profile form for %s carries no reference" % domain,
                )
            else:
                add(
                    "GET /watch/confirm",
                    True,
                    "%s confirmed; profile form carries profile_token + reference" % domain,
                )
        if tokens[domain_a]:
            replay = http_req(
                "POST",
                base + "/watch/confirm",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=form_body([("token", tokens[domain_a])]),
            )
            refused = (
                "This confirmation link is no longer valid." in replay.text
                or "no longer valid" in replay.text.lower()
                or "refused" in replay.text.lower()
            )
            add(
                "GET /watch/confirm",
                replay.status in (200, 400) and refused,
                "a spent confirmation token answered %d%s"
                % (
                    replay.status,
                    ""
                    if refused
                    else " without the refusal copy (%s)" % sanitize_evidence(replay.text, 160),
                ),
            )

        # 5. Submit the billing profile once; a replay is refused and never
        # overwrites the stored profile.
        def profile_fields(domain, overrides=None):
            page, hidden = profile_pages.get(domain, (None, {}))
            fields = {
                "profile_token": hidden.get("profile_token") or "",
                "reference": hidden.get("reference") or (by_domain.get(domain) or {}).get("id", ""),
                "legal_name": "Pending %s Oy" % domain,
                "address_lines": "Testikatu 1\n00100 Helsinki",
                "country": "FI",
            }
            if overrides:
                fields.update(overrides)
            return fields

        profile_a = profile_fields(domain_a)
        submitted = http_req(
            "POST",
            base + "/watch/billing-profile",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(sorted(profile_a.items())),
        )
        rows = self.read_watch_subscribers(watch_root)
        row_a = rows.get((by_domain.get(domain_a) or {}).get("id") or "", {})
        billing = row_a.get("billingProfile") or row_a.get("billing") or {}
        if submitted.status not in (200, 303):
            add(
                "POST /watch/billing-profile",
                False,
                "the billing profile POST answered %d (expected 200/303)" % submitted.status,
            )
        elif (
            billing.get("legalName") != profile_a["legal_name"]
            or billing.get("country") != "FI"
            or not billing.get("addressLines")
        ):
            add(
                "POST /watch/billing-profile",
                False,
                "the profile was not stored on the subscriber: %r" % (billing or None),
            )
        else:
            add(
                "POST /watch/billing-profile",
                True,
                "profile stored: %s / %s / %s"
                % (billing.get("legalName"), billing.get("addressLines"), billing.get("country")),
            )
        replay_profile = dict(profile_a)
        replay_profile["legal_name"] = "Replay Overwrite Oy"
        replayed = http_req(
            "POST",
            base + "/watch/billing-profile",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(sorted(replay_profile.items())),
        )
        rows = self.read_watch_subscribers(watch_root)
        row_after_replay = rows.get((by_domain.get(domain_a) or {}).get("id") or "", {})
        replay_billing = row_after_replay.get("billingProfile") or row_after_replay.get("billing") or {}
        refusal_copy = any(
            marker in replayed.text.lower()
            for marker in ("refused", "no longer", "already", "used", "invalid")
        )
        add(
            "POST /watch/billing-profile",
            replayed.status in (400, 409, 410, 413, 422)
            or (replayed.status == 200 and refusal_copy),
            "a replayed profile token answered %d%s"
            % (replayed.status, " with a refusal" if refusal_copy else ""),
        )
        add(
            "POST /watch/billing-profile",
            replay_billing.get("legalName") == billing.get("legalName"),
            "a replayed profile token changed the stored legal name to %r"
            % replay_billing.get("legalName"),
        )
        # The second settled subscriber needs its own profile through its own
        # one-time form: the first bill then issues both first payments with
        # gapless numbers.
        profile_b = profile_fields(domain_b)
        submitted_b = http_req(
            "POST",
            base + "/watch/billing-profile",
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            body=form_body(sorted(profile_b.items())),
        )
        rows_b = self.read_watch_subscribers(watch_root)
        row_b = rows_b.get((by_domain.get(domain_b) or {}).get("id") or "", {})
        billing_b = row_b.get("billingProfile") or row_b.get("billing") or {}
        add(
            "POST /watch/billing-profile",
            submitted_b.status in (200, 303)
            and billing_b.get("legalName") == profile_b["legal_name"],
            "the second subscriber's profile POST answered %d, stored=%s"
            % (submitted_b.status, billing_b.get("legalName")),
        )

        # 6. Hostile profile inputs are typed refusals and store nothing.
        hostile_cases = [
            ("country", {"country": "E1"}),
            ("address", {"address_lines": "A" * 70_000}),
            ("vat", {"vat_number": "not-a-vat"}),
        ]
        for label, overrides in hostile_cases:
            domain = "hostile-%s.example" % label
            email = "owner@" + domain
            signup = self.watch_signup(domain, email, "admob.com = pub-hostile")
            token = self.watch_confirmation_token(email)
            if signup.status != 303 or not token:
                add(
                    "POST /watch/billing-profile",
                    False,
                    "the hostile-%s sign-up could not reach confirmation (HTTP %d, token=%s)"
                    % (label, signup.status, bool(token)),
                )
                continue
            confirmed = http_req("GET", base + "/watch/confirm?token=" + token)
            hidden = parse_hidden_fields(confirmed.text)
            ref = next(
                (
                    row.get("id")
                    for row in self.read_watch_subscribers(watch_root).values()
                    if row.get("domain") == domain
                ),
                "",
            )
            fields = {
                "profile_token": hidden.get("profile_token") or "",
                "reference": hidden.get("reference") or ref,
                "legal_name": "Hostile %s Oy" % label,
                "address_lines": "Testikatu 2",
                "country": "FI",
            }
            fields.update(overrides)
            hostile = http_req(
                "POST",
                base + "/watch/billing-profile",
                headers={"Content-Type": "application/x-www-form-urlencoded"},
                body=form_body(sorted(fields.items())),
            )
            stored = next(
                (
                    row.get("billingProfile") or row.get("billing")
                    for row in self.read_watch_subscribers(watch_root).values()
                    if row.get("domain") == domain
                ),
                None,
            )
            add(
                "POST /watch/billing-profile",
                hostile.status in (400, 413, 422) and not stored,
                "hostile %s profile -> %d, stored=%s" % (label, hostile.status, bool(stored)),
            )

        # 7. The settlement webhook activates both rows and stores the card.
        ids = {domain: (by_domain.get(domain) or {}).get("id") for domain in (domain_a, domain_b)}
        cards = {domain_a: "card-dogfood-a", domain_b: "card-dogfood-b"}
        for domain in (domain_a, domain_b):
            reference = ids[domain] or ""
            merchant = reference if domain == domain_a else "apex-watch:" + reference
            resp = self.post_adyen_notification(
                "PSP" + WATCH_SETTLEMENT_REFERENCE.upper() + ("A" if domain == domain_a else "B"),
                merchant,
                cards[domain],
            )
            new = None
            if resp.status == 200:
                try:
                    new = resp.json().get("new")
                except (ValueError, ReqError):
                    new = None
            add(
                "POST /v1/adyen/webhook",
                resp.status == 200 and new == 1,
                "settlement for %s -> HTTP %s new=%r (expected 200/1)" % (domain, resp.status, new),
            )
        rows = self.read_watch_subscribers(watch_root)
        for domain in (domain_a, domain_b):
            row = rows.get(ids[domain] or "", {})
            add(
                "POST /v1/adyen/webhook",
                row.get("status") == "active"
                and row.get("storedPaymentMethodId") == cards[domain]
                and bool(row.get("paidAt"))
                and row.get("confirmed") is True,
                "settled %s status=%r card=%r paid=%s confirmed=%s"
                % (
                    domain,
                    row.get("status"),
                    row.get("storedPaymentMethodId"),
                    bool(row.get("paidAt")),
                    row.get("confirmed"),
                ),
            )
        # The shared watch store already holds the differential-truth
        # subscriber, so a daily pass over it would check that paid row too.
        # Copy only the two rows this cell settled into an isolated root: the
        # assertion is about both of them, not about who else the shared store
        # happens to contain.
        daily_root = os.path.join(self.workdir, "payment-daily-watch")
        shutil.rmtree(daily_root, ignore_errors=True)
        os.makedirs(daily_root)
        with open(os.path.join(daily_root, "subscribers.jsonl"), "w", encoding="utf-8") as fh:
            for domain in (domain_a, domain_b):
                row = rows.get(ids[domain] or "")
                if row:
                    fh.write(json.dumps(row) + "\n")
        daily = self.run_watch_daily(daily_root)
        checked = self.watch_checked_count(daily)
        add(
            "POST /v1/adyen/webhook",
            daily.returncode == 0 and checked == 2,
            "daily pass exit=%d checked=%r (expected both confirmed active rows)"
            % (daily.returncode, checked),
        )
        replay = self.post_adyen_notification(
            "PSP" + WATCH_SETTLEMENT_REFERENCE.upper() + "A", ids[domain_a] or "", cards[domain_a]
        )
        try:
            replay_new = replay.json().get("new") if replay.status == 200 else None
        except (ValueError, ReqError):
            replay_new = None
        add(
            "POST /v1/adyen/webhook",
            replay.status == 200 and replay_new == 0,
            "a replayed settlement -> HTTP %s new=%r (expected 200/0)" % (replay.status, replay_new),
        )

        # 8. Bill the isolated copy of the two settled rows: the first
        # payment is invoiced with gapless numbers, and the next month charges
        # the stored card again (the renewal defect).
        bill_root = os.path.join(self.workdir, "payment-watch")
        shutil.rmtree(bill_root, ignore_errors=True)
        os.makedirs(bill_root)
        rows = self.read_watch_subscribers(watch_root)
        with open(os.path.join(bill_root, "subscribers.jsonl"), "w", encoding="utf-8") as fh:
            for domain in (domain_a, domain_b):
                row = rows.get(ids[domain] or "")
                if row:
                    fh.write(json.dumps(row) + "\n")
        bill_cwd = os.path.join(self.workdir, "payment-billing")
        shutil.rmtree(bill_cwd, ignore_errors=True)
        os.makedirs(bill_cwd, exist_ok=True)
        bill_env = {
            "APEX_WATCH_ROOT": bill_root,
            "APEX_BILLING_PROFILE": self.fleet["seller"],
            "APEX_BILLING_VAT_RATE_BP": str(SELLER_VAT_RATE_BP),
            "APEX_ADYEN_API_KEY": "dogfood-live-adyen-key",
            "APEX_ADYEN_MERCHANT_ACCOUNT": API_ADYEN_MERCHANT_ACCOUNT,
            "APEX_ADYEN_CHECKOUT_BASE": self.api["stub"].base_url(),
            "APEX_ADYEN_HMAC_KEY": API_ADYEN_HMAC_KEY_HEX,
        }
        current = self.current_period()
        first = self.watch_bill(current, bill_cwd, bill_env)
        register_path = os.path.join(bill_cwd, ".apex", "billing", "invoices.jsonl")

        def register_numbers():
            numbers = []
            if os.path.exists(register_path):
                with open(register_path, "r", encoding="utf-8") as fh:
                    for line in fh:
                        if not line.strip():
                            continue
                        try:
                            row = json.loads(line)
                        except ValueError:
                            continue
                        if isinstance(row.get("number"), int):
                            numbers.append(row["number"])
            return numbers

        first_numbers = register_numbers()
        add(
            "POST /watch/billing-profile",
            first.get("issued") == 2
            and first.get("charged") == 0
            and first.get("refused") == 0
            and first_numbers == [1, 2],
            "first bill %s: issued=%s charged=%s refused=%s numbers=%s (expected 2/0/0 [1, 2]: the first payment was already collected, so the bill issues without a new charge)"
            % (
                current,
                first.get("issued"),
                first.get("charged"),
                first.get("refused"),
                first_numbers,
            ),
        )
        def add_months(period, count):
            year, month = (int(part) for part in period.split("-"))
            index = (year * 12 + (month - 1)) + count
            return "%04d-%02d" % (index // 12, index % 12 + 1)

        next_month = add_months(current, 1)
        renewal = self.watch_bill(next_month, bill_cwd, bill_env)
        renewal_numbers = register_numbers()
        add(
            "POST /watch/billing-profile",
            renewal.get("issued") == 2
            and renewal.get("charged") == 2
            and renewal_numbers == [1, 2, 3, 4],
            "renewal bill %s: issued=%s charged=%s numbers=%s (expected 2/2 [1, 2, 3, 4])"
            % (
                next_month,
                renewal.get("issued"),
                renewal.get("charged"),
                renewal_numbers,
            ),
        )
        again = self.watch_bill(next_month, bill_cwd, bill_env)
        add(
            "POST /watch/billing-profile",
            again.get("issued") == 0
            and again.get("charged") == 0
            and again.get("skipped", 0) >= 2,
            "a rerun of %s: issued=%s charged=%s skipped=%s (expected 0/0/2)"
            % (next_month, again.get("issued"), again.get("charged"), again.get("skipped")),
        )

        # 9. A subscriber with no profile is refused with the documented
        # reason (never silently skipped).
        no_profile = {
            "id": "sub_noprofile1",
            "publisher": "no-profile-probe",
            "domain": "no-profile-probe.example",
            "email": "owner@no-profile-probe.example",
            "tier": "normal",
            "createdMs": int(time.time() * 1000),
            "status": "active",
            "confirmed": True,
            "paidAt": int(time.time() * 1000),
            "storedPaymentMethodId": "card-no-profile",
            "cancelTokenHash": hashlib.sha256(b"no-profile-token").hexdigest(),
        }
        with open(os.path.join(bill_root, "subscribers.jsonl"), "a", encoding="utf-8") as fh:
            fh.write(json.dumps(no_profile) + "\n")
        third_month = add_months(current, 2)
        proc = self.spawn(
            [self.bin_path("apex-watch"), "bill", third_month],
            cwd=bill_cwd,
            env={**os.environ, **bill_env},
            capture_output=True,
            text=True,
            timeout=60,
        )
        refusal_out = (proc.stdout or "") + (proc.stderr or "")
        add(
            "POST /watch/billing-profile",
            "no-profile-probe: refused:" in refusal_out
            and "no customer billing profile" in refusal_out,
            "the no-profile subscriber was refused with the documented reason",
        )

        for _service, surface, cell_id in self.cells_of("payment-activation"):
            entry = results.get(surface)
            if entry is None:
                self.cell(cell_id, False, "no payment-activation expectation for %s" % surface)
                continue
            detail = "; ".join((entry["notes"] + entry["problems"])[:10]) or "no observation"
            self.cell(cell_id, not entry["problems"], detail)

    def current_period(self):
        return time.strftime("%Y-%m", time.gmtime())

    # 13b ------------------------------------------ failure-injection (watch)
    def failure_dns_rebinding(self):
        """The fetch must pin the guard-approved address with
        `curl --resolve host:port:address`, so a DNS answer that changes
        between the guard and the fetch cannot redirect it."""
        problems = []
        watch_root = os.path.join(self.workdir, "rebind-watch")
        shutil.rmtree(watch_root, ignore_errors=True)
        os.makedirs(watch_root, exist_ok=True)
        now_ms = int(time.time() * 1000)
        token = "rebind-token"
        row = {
            "id": "sub_rebind0001",
            "publisher": "dogfood-watch-example",
            "domain": "example.com",
            "email": WATCH_EMAIL,
            "tier": "normal",
            "createdMs": now_ms,
            "status": "active",
            # Q: a checkable subscriber has a recorded billing profile.
            "billingProfile": {
                "legalName": "Rebind Control Oy",
                "addressLines": ["Testikatu 1", "00100 Helsinki"],
                "country": "FI",
                "vatNumber": None,
            },
            "cancelTokenHash": hashlib.sha256(token.encode("utf-8")).hexdigest(),
        }
        with open(os.path.join(watch_root, "subscribers.jsonl"), "w", encoding="utf-8") as fh:
            fh.write(json.dumps(row) + "\n")
        stub_dir = os.path.join(self.workdir, "curl-stub")
        shutil.rmtree(stub_dir, ignore_errors=True)
        os.makedirs(stub_dir)
        argv_log = os.path.join(self.workdir, "curl-argv.log")
        verdict_log = os.path.join(self.workdir, "curl-verdict.log")
        for path in (argv_log, verdict_log):
            with open(path, "w", encoding="utf-8"):
                pass
        stub = os.path.join(stub_dir, "curl")
        with open(stub, "w", encoding="utf-8") as fh:
            fh.write(
                "#!/usr/bin/env python3\n"
                "import os, sys\n"
                "args = sys.argv[1:]\n"
                "with open(%r, 'a') as fh:\n"
                "    fh.write('\\t'.join(args) + '\\n')\n"
                "resolve = []\n"
                "url = ''\n"
                "i = 0\n"
                "while i < len(args):\n"
                "    if args[i] == '--resolve' and i + 1 < len(args):\n"
                "        resolve.append(args[i + 1]); i += 2; continue\n"
                "    if args[i].startswith('http'):\n"
                "        url = args[i]\n"
                "    i += 1\n"
                "host = url.split('//', 1)[-1].split('/', 1)[0].split(':')[0]\n"
                "pinned = any(entry.split(':', 1)[0] == host for entry in resolve)\n"
                "with open(%r, 'a') as fh:\n"
                "    fh.write(('PINNED ' if pinned else 'UNPINNED ') + (resolve[0] if resolve else '') + '\\n')\n"
                "if pinned:\n"
                "    body = 'admob.com, pub-northstar, DIRECT\\n'\n"
                "else:\n"
                "    body = 'REBOUND-TO-LOOPBACK.example, pub-evil, DIRECT\\n'\n"
                "sys.stdout.write(body + '\\n@@apex-watch-status@@200|')\n"
                % (argv_log, verdict_log)
            )
        os.chmod(stub, 0o755)
        proc = self.run_watch_daily(
            watch_root,
            extra_env={
                "APEX_DOGFOOD_CURL_LOG": argv_log,
                "APEX_DOGFOOD_CURL_VERDICT": verdict_log,
            },
            path_dir=stub_dir,
        )
        with open(verdict_log, "r", encoding="utf-8") as fh:
            verdicts = [line.strip() for line in fh if line.strip()]
        if not verdicts:
            problems.append(
                "the watch fetch never called curl (no request observed); daily exit=%d" % proc.returncode
            )
        else:
            unpinned = [line for line in verdicts if line.startswith("UNPINNED")]
            pinned = [line for line in verdicts if line.startswith("PINNED")]
            if unpinned:
                problems.append(
                    "the fetch re-resolved the host instead of pinning the guard-approved address (%d unpinned call(s))"
                    % len(unpinned)
                )
            if not pinned:
                problems.append("no `--resolve host:port:address` call was observed")
        rows = self.read_watch_subscribers(watch_root)
        blob = json.dumps(rows)
        if REBIND_CANARY in blob or "REBOUND-TO-LOOPBACK" in blob:
            problems.append("the rebound loopback body reached a recorded finding")
        return (not problems, "; ".join(problems) or "watch fetch pinned the guard-approved address")

    def failure_unpaid_subscriber(self):
        """A subscriber that has not paid runs no daily check.

        The shared API watch store also holds the paid subscriber seeded by
        differential-truth, so copying it would make the daily pass check a
        paid row and prove nothing about the unpaid ones. The run therefore
        uses an isolated store with two unpaid rows (pending and active but
        unconfirmed) plus one paid control: exactly the control may run a
        check, and neither unpaid row may gain a check timestamp."""
        problems = []
        watch_root = os.path.join(self.workdir, "unpaid-watch")
        shutil.rmtree(watch_root, ignore_errors=True)
        os.makedirs(watch_root)
        now_ms = int(time.time() * 1000)

        def unpaid_row(row_id, publisher, domain, status, confirmed, paid):
            row = {
                "id": row_id,
                "publisher": publisher,
                "domain": domain,
                "email": "owner@" + domain,
                "tier": "normal",
                "createdMs": now_ms,
                "status": status,
                "confirmed": confirmed,
                "cancelTokenHash": hashlib.sha256(row_id.encode("utf-8")).hexdigest(),
            }
            if paid:
                row["paidAt"] = now_ms
                # Q: an active paid row is only checkable with a recorded
                # billing profile.
                row["billingProfile"] = {
                    "legalName": "Paid Control Oy",
                    "addressLines": ["Testikatu 1", "00100 Helsinki"],
                    "country": "FI",
                    "vatNumber": None,
                }
            return row

        rows = [
            unpaid_row("sub_unpaid_pending1", "unpaid-pending", "pending-unpaid.example", "pending", True, False),
            unpaid_row(
                "sub_unpaid_confirm1",
                "unpaid-unconfirmed",
                "unconfirmed-unpaid.example",
                "active",
                False,
                True,
            ),
            unpaid_row("sub_paid_control1", "paid-control", "example.com", "active", True, True),
        ]
        with open(os.path.join(watch_root, "subscribers.jsonl"), "w", encoding="utf-8") as fh:
            for row in rows:
                fh.write(json.dumps(row) + "\n")
        proc = self.run_watch_daily(watch_root)
        checked = self.watch_checked_count(proc)
        if checked is None:
            problems.append(
                "apex-watch daily produced no checked count (exit %d): %s"
                % (proc.returncode, sanitize_evidence((proc.stdout or "") + (proc.stderr or ""), 200))
            )
        elif checked != 1:
            problems.append(
                "the unpaid run checked %d subscriber(s); only the paid control may run a check (expected 1)"
                % checked
            )
        folded = self.read_watch_subscribers(watch_root)
        for row_id in ("sub_unpaid_pending1", "sub_unpaid_confirm1"):
            if (folded.get(row_id) or {}).get("lastCheckAt"):
                problems.append("the unpaid subscriber %s ran a daily check" % row_id)
        if not (folded.get("sub_paid_control1") or {}).get("lastCheckAt"):
            problems.append("the paid control subscriber ran no daily check")
        return (
            not problems,
            "; ".join(problems)
            or "only the paid control ran a check; both unpaid rows ran none (1 checked)",
        )

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
        self.counters["baseline_reads"] += 1
        with open(path, "r", encoding="utf-8") as fh:
            baseline = json.load(fh)
        recorded = set(baseline.get("cells", []))
        removals = {entry.get("cell") for entry in baseline.get("removals", []) if entry.get("cell")}
        problems = []
        # Under a relevance selection the run is deliberately partial, so the
        # "new required cell not yet recorded" direction is not enforced (a
        # filtered run cannot record the whole catalog); the ratchet still
        # refuses a baseline cell that is no longer required.
        missing = [] if self.filtered else sorted(self.required - recorded)
        dangling = sorted(recorded - self.required - removals)
        if self.args.update_baseline and not dangling and missing:
            # The recording run itself: pure additions are what `--update-baseline`
            # exists to record, so the drift passes once and the green gate still
            # has to hold everywhere else. The ratchet is strict again on every
            # run that does not carry the flag.
            self.check(
                "ratchet|baseline",
                True,
                "recording %d new cell(s) with --update-baseline (baseline %d -> required %d)"
                % (len(missing), len(recorded), len(self.required)),
            )
            return
        if self.args.update_baseline and dangling:
            self.check(
                "ratchet|baseline",
                False,
                "baseline cells no longer required without a reasoned removal: %s" % dangling[:6],
            )
            return
        if missing:
            problems.append("required cells missing from the baseline (drift): %s" % missing[:6])
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
            self.counters["baseline_reads"] += 1
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
        self.counters["baseline_writes"] += 1
        log("baseline written: %s (%d cells)" % (path, len(baseline["cells"])))

    # ------------------------------------------------------------- results
    def write_results(self, exit_code, unattempted):
        os.makedirs(os.path.dirname(self.results_path), exist_ok=True)
        self.counters["results_writes"] += 1
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
                "elapsed_seconds": getattr(self, "elapsed_seconds", None),
                "class_seconds": dict(sorted(self.class_timings.items())),
                "counters": self.runtime_counters(),
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
        for key in ("stub",):
            stub = self.api.get(key)
            if stub is not None:
                stub.stop()
                self.api[key] = None
        proofs = []
        for name, port in sorted(self.ports.items()):
            lsof = self.spawn(
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
        self.apply_selection()
        self.report_selection()
        self.probe_surface_parity()
        self.check_site()
        self.check_baseline()
        for cell_id in sorted(self.selected):
            if cell_id in self.cells:
                # The static parity cell was actually attempted above; do
                # not overwrite its pass/fail with the skipped placeholder.
                continue
            self.cell(cell_id, True, "SKIPPED: --static-only records the cell without attempting it")
        skipped = sum(1 for cell_id in self.selected if cell_id not in self.cells)
        log(
            "static-only: %d cells recorded as skipped; %d attempted; %d checks run"
            % (skipped, len(self.cells), len(self.checks))
        )

    def report_selection(self):
        if not self.filtered:
            return
        log(
            "relevance: selected %d of %d cells (skipped %d unrelated); skipped classes: %s"
            % (
                len(self.selected),
                len(self.required),
                len(self.required) - len(self.selected),
                ", ".join(sorted(self.skipped_classes)) or "(none)",
            )
        )
        for note in self.selection_notes:
            log("relevance: %s" % note)

    def run_live(self):
        try:
            self.load_catalog()
            self.apply_selection()
            self.report_selection()
            self.check_site()
            if not self.require_binaries():
                return
            if not self.ch_reachable() and self.ch_owned:
                # `--clickhouse-owned` is the local make path: the harness
                # owns the server, so it must start it rather than requiring
                # the operator to run `make clickhouse-dev` first.
                started, detail = self.clickhouse_start()
                if not started:
                    self.check(
                        "boot|clickhouse",
                        False,
                        "the owned ClickHouse could not start: %s" % detail,
                    )
                    self.probe_surface_parity()
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
            self.check(
                "boot|clickhouse",
                True,
                "ClickHouse reachable at %s%s"
                % (self.ch_url, " (started by the harness)" if self.ch_owned else ""),
            )
            crypto_self_check()
            self.check(
                "boot|crypto",
                True,
                "AES-256-GCM and ed25519 pinned against the published vectors "
                "(NIST GCM, RFC 8032 test 1); the runner seed derives the fixture pubkey",
            )
            busy = []
            for service_name, port in self.ports.items():
                probe = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                probe.settimeout(0.2)
                try:
                    if probe.connect_ex(("127.0.0.1", port)) == 0:
                        busy.append("%s:%d" % (service_name, port))
                finally:
                    probe.close()
            if busy:
                self.check(
                    "boot|ports",
                    False,
                    "harness ports already in use (a previous run is still alive): "
                    + ", ".join(busy),
                )
                return
            self.check("boot|ports", True, "harness ports free on 127.0.0.1")
            self.api["work"] = self.prepare_api_files()
            # The runner-, billing- and watch-run working trees hold durable
            # product state (registers, job files, projections); a stale tree
            # from an earlier run would leak into this one, so the harness
            # resets them before seeding its own stores.
            for stale in ("runner", "billing-run", "watch-billing"):
                shutil.rmtree(os.path.join(self.workdir, stale), ignore_errors=True)
            self.control["work"] = self.prepare_control_files()
            self.prepare_sales_files()
            self.seed_touch()
            self.ch_bootstrap()
            self.bootstrap_monitoring()
            self.seed_admob_kept_change()
            self.run_runner_regression_cells()
            self.start_api()
            self.start_api_unconfigured()
            self.start_api_links_off()
            self.start_control()
            self.start_sales_web()
            self.run_all_probes()
            self.run_lockout_probe()
            self.check_baseline()
        except ReqError as error:
            self.check("boot|live-sequence", False, str(error))

    def summarize(self):
        failed_cells = sorted(c for c, cell in self.cells.items() if cell["status"] == "fail")
        unattempted = sorted(self.selected - set(self.cells))
        failed_checks = [entry["check"] for entry in self.checks if entry["status"] == "fail"]
        log("")
        log("dogfood-live summary")
        if self.filtered:
            log(
                "  relevance:       selected %d of %d cells (skipped %d unrelated)"
                % (len(self.selected), len(self.required), len(self.required) - len(self.selected))
            )
        log("  required cells:  %d" % len(self.selected))
        log("  attempted cells: %d" % len(self.cells))
        log(
            "  passed cells:    %d"
            % sum(1 for cell in self.cells.values() if cell["status"] == "pass")
        )
        if self.class_timings:
            log("  class timings (wall seconds, measurement only):")
            for cls in PROBE_ORDER:
                seconds = self.class_timings.get(cls)
                if seconds is not None:
                    log("    %-22s %7.2fs" % (cls, seconds))
        http = http_stats()
        log(
            "  work counters:   http %d request(s) over %d connect(s) (%d reused, %d retries, %d pool drop(s)); "
            "%d subprocess spawn(s); dogfood-plan %d; surfaces %d; baseline %d read/%d write; results %d"
            % (
                http["requests"],
                http["connects"],
                http["reused"],
                http["retries"],
                http["drops"],
                self.counters["subprocess_spawns"],
                self.counters["dogfood_plan_spawns"],
                self.counters["surfaces_reads"],
                self.counters["baseline_reads"],
                self.counters["baseline_writes"],
                # The summary prints before write_results increments this.
                self.counters["results_writes"] + 1,
            )
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
        reset_http_stats()
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
            self.elapsed_seconds = time.monotonic() - self.run_started_monotonic
            log("  total wall time: %.2fs" % self.elapsed_seconds)
            unattempted = sorted(self.selected - set(self.cells))
            exit_code = 0 if green else 1
            self.write_results(exit_code, unattempted)
        return 0 if green else 1


def parse_args(argv):
    parser = argparse.ArgumentParser(description="dogfood-live: the adversarial live harness")
    parser.add_argument("--bin-dir", default="rust/target/debug")
    parser.add_argument("--workdir", default="rust/target/dogfood-live")
    parser.add_argument(
        "--keep-workdir",
        action="store_true",
        help="keep the previous run's service state (the default starts clean)",
    )
    parser.add_argument("--clickhouse-url", default="http://127.0.0.1:18123")
    parser.add_argument("--static-only", action="store_true")
    parser.add_argument("--results", default="rust/target/dogfood-live/results.json")
    parser.add_argument("--update-baseline", action="store_true")
    parser.add_argument("--port-base", type=int, default=18700)
    parser.add_argument(
        "--changed",
        action="extend",
        nargs="+",
        default=[],
        metavar="PATH",
        help="run only the cells whose surface sources intersect this repo-relative path "
        "(repeatable); adds the always-run core set",
    )
    parser.add_argument(
        "--git-diff",
        default=None,
        metavar="BASE..HEAD",
        help="run only the cells whose surface sources intersect the files changed between "
        "two git revisions",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run the relevance-selection self-test (the four documented cases) and exit",
    )
    parser.add_argument(
        "--clickhouse-owned",
        action="store_true",
        help="the operator granted the harness ownership of the ClickHouse server at --clickhouse-url; "
        "the local make path starts one when none is reachable, and the failure-injection CH "
        "severance subprobe is required",
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv if argv is not None else sys.argv[1:])
    if args.self_test:
        problems = relevance_self_test()
        if problems:
            print("relevance self-test FAILED:")
            for problem in problems:
                print("  - %s" % problem)
            return 1
        print("relevance self-test PASS: the four documented cases")
        pool_problems = http_pool_self_test()
        if pool_problems:
            print("http pool self-test FAILED:")
            for problem in pool_problems:
                print("  - %s" % problem)
            return 1
        print(
            "http pool self-test PASS: idempotent requests reuse one connection, "
            "a closed socket reconnects, POSTs stay fresh"
        )
        return 0
    return Harness(args).run()


if __name__ == "__main__":
    sys.exit(main())
