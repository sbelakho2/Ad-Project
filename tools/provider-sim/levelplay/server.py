#!/usr/bin/env python3
"""Stateful LevelPlay (Unity / ironSource) provider simulator.

The simulator speaks the documented LevelPlay APIs the Rust connector uses:

* ``GET /partners/publisher/auth`` with the ``secretkey`` and ``refreshToken``
  headers, returning a quoted bearer token (valid for ``--token-ttl`` seconds).
* ``/levelPlay/network/instances/v4/{appKey}/`` CRUD (bare JSON array shapes).
* ``/levelPlay/groups/v4/{appKey}`` reads and floor writes (bare array).
* ``/levelPlay/reporting/v1`` with ``date``/``abTest``/``country``/``adUnit``
  breakdowns, ``revenue``/``activeUsers`` metrics, and ``page`` /
  ``resultsPerPage`` pagination (``data``/``page``/``pageSize``/``totalResults``
  envelope).

Everything is mutable and in-memory, optionally mirrored to ``--state-file``
so an external driver can inspect exactly what the product wrote.  A small
``/_control/*`` surface injects faults and reports state; it is not part of the
provider contract.

Only the Python standard library is used.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import signal
import sys
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

AUTH_PATH = "/partners/publisher/auth"
INSTANCES_PREFIX = "/levelPlay/network/instances/v4/"
GROUPS_PREFIX = "/levelPlay/groups/v4/"
REPORTING_PATH = "/levelPlay/reporting/v1"
DAY_SECS = 86_400

INSTANCES_RE = re.compile(r"^/levelPlay/network/instances/v4/([^/]+)/?$")
GROUPS_RE = re.compile(r"^/levelPlay/groups/v4/([^/]+)/?$")


def utc_day(now: float | None = None) -> int:
    return int((time.time() if now is None else now) // DAY_SECS)


def format_day(day_number: int) -> str:
    return time.strftime("%Y-%m-%d", time.gmtime(day_number * DAY_SECS))


def decimal_units(micros: int) -> str:
    """Micros -> shortest exact units string (2500000 -> '2.5')."""
    sign = "-" if micros < 0 else ""
    value = abs(micros)
    whole, frac = divmod(value, 1_000_000)
    if frac == 0:
        return f"{sign}{whole}"
    text = f"{frac:06d}".rstrip("0")
    return f"{sign}{whole}.{text}"


def parse_units(raw) -> float:
    if isinstance(raw, bool) or raw is None:
        raise ValueError("price/rate must be a number")
    if isinstance(raw, (int, float)):
        return float(raw)
    text = str(raw).strip()
    if not text:
        raise ValueError("price/rate is empty")
    return float(text)


def parse_price_micros(raw) -> int:
    units = parse_units(raw)
    return int(round(units * 1_000_000))


def id_value(raw, style: str):
    """Render an identifier in the configured wire style."""
    if style == "string":
        return str(raw)
    try:
        return int(raw)
    except (TypeError, ValueError):
        return raw


class State:
    def __init__(self, args):
        self.args = args
        self.lock = threading.RLock()
        self.secret_key = args.secret_key
        self.refresh_token = args.refresh_token
        self.app_key = args.app_key
        self.token_ttl_s = args.token_ttl
        self.id_style = args.id_style
        self.report_style = args.report_style
        self.strict_writes = args.strict_writes
        self.default_page_size = args.page_size
        self.seed_days = args.seed_days
        self.b_lift = args.b_lift
        self.variant_group_id = args.variant_group_id
        self.state_file = args.state_file
        self.started_at = time.time()
        self.token_seq = 0
        self.tokens: dict[str, float] = {}
        self.expire_next_bearer = 0
        self.instances: dict[str, dict] = {}
        self.instance_seq = 1000
        self.groups: list[dict] = []
        self.group_seq = 2000
        self.reports: list[dict] = []
        self.idempotency: dict[str, dict] = {}
        self.faults: list[dict] = []
        self.writes: list[dict] = []
        self.requests: list[dict] = []
        self.counters: dict[str, int] = {}
        self._seed()

    # ---------------------------------------------------------------- seed
    def _seed(self):
        self.instances = {}
        self.instance_seq = 1000
        self.groups = []
        self.group_seq = 2000
        self.reports = []
        self.idempotency = {}
        self.faults = []
        self.writes = []
        self.requests = []
        self.counters = {}
        self.token_seq = 0
        self.tokens = {}
        self.expire_next_bearer = 0

        first = self._next_instance_id()
        second = self._next_instance_id()
        self.instances[str(first)] = {
            "instanceId": first,
            "instanceName": "Rewarded_High",
            "adUnit": "rewardedVideo",
            "adFormat": "rewarded",
            "networkName": "unityAds",
            "isBidder": False,
            "groups": [2001, 2002],
            "isLive": True,
            "rate": "2.00",
            "appConfig1": "appcfg",
            "instanceConfig1": "instcfg",
        }
        self.instances[str(second)] = {
            "instanceId": second,
            "instanceName": "Rewarded_Backfill",
            "adUnit": "rewardedVideo",
            "adFormat": "rewarded",
            "networkName": "admob",
            "isBidder": False,
            "groups": [2001, 2002],
            "isLive": True,
            "rate": "0.80",
            "appConfig1": "appcfg",
            "instanceConfig1": "instcfg2",
        }
        members = [
            {
                "id": first,
                "name": "Rewarded_High",
                "networkName": "unityAds",
                "isBidder": False,
                "groupRate": 2.0,
                "countriesRate": [{"countryCode": "US", "rate": 2.0}],
            },
            {
                "id": second,
                "name": "Rewarded_Backfill",
                "networkName": "admob",
                "isBidder": False,
                "groupRate": 0.8,
                "countriesRate": [{"countryCode": "US", "rate": 0.8}],
            },
        ]
        for ab in ("A", "B"):
            gid = self._next_group_id()
            if ab == "B" and self.variant_group_id:
                gid = self.variant_group_id
            group_members = copy.deepcopy(members)
            if ab == "B":
                # A/B isolation: the variant arm must reference its own
                # instance rows, not the control's, or a floor write would
                # mutate both arms.
                for member in group_members:
                    clone_id = self._next_instance_id()
                    member["id"] = clone_id
                    self.instances[str(clone_id)] = {
                        "instanceId": clone_id,
                        "instanceName": member.get("name"),
                        "adUnit": "lp-adunit-1",
                        "adFormat": "rewarded",
                        "networkName": member.get("networkName"),
                        "isBidder": bool(member.get("isBidder")),
                        "groups": [gid],
                        "isLive": True,
                        "rate": decimal_units(int(round(float(member.get("groupRate", 0)) * 1_000_000))),
                        "appConfig1": "appcfg",
                        "instanceConfig1": "instcfg",
                    }
            self.groups.append(
                {
                    "groupId": gid,
                    "groupName": "Rewarded_Main",
                    "mediationAdUnitId": "lp-adunit-1",
                    "mediationAdUnitName": "rewarded-main",
                    "adFormat": "rewarded",
                    "abTest": ab,
                    "position": 1 if ab == "A" else 2,
                    "floorPrice": 2.0,
                    "countries": ["US", "GB"],
                    "segments": [],
                    "instances": group_members,
                }
            )
        self._seed_reports()

    def _next_instance_id(self):
        self.instance_seq += 1
        return self.instance_seq

    def _next_group_id(self):
        self.group_seq += 1
        return self.group_seq

    def _seed_reports(self):
        today = utc_day()
        for offset in range(self.seed_days, 0, -1):
            day = format_day(today - offset)
            for arm in ("A", "B"):
                base = 12.50 + (offset % 7) * 0.03
                revenue = round(base * (self.b_lift if arm == "B" else 1.0), 6)
                users = 5000 + (offset % 11) * 13
                self.reports.append(
                    {
                        "date": day,
                        "abTest": arm,
                        "country": "US" if offset % 2 == 0 else "GB",
                        "mediationAdUnitId": "lp-adunit-1",
                        "adUnit": "rewardedVideo",
                        "adFormat": "rewarded",
                        "networkName": "unityAds" if arm == "A" else "admob",
                        "revenue": revenue,
                        "activeUsers": users,
                        "impressions": 100_000 + offset * 10,
                    }
                )

    # --------------------------------------------------------------- helpers
    def public_instance(self, instance: dict) -> dict:
        copy_ = copy.deepcopy(instance)
        copy_["instanceId"] = id_value(copy_["instanceId"], self.id_style)
        copy_["groups"] = [
            id_value(group_id, self.id_style) for group_id in copy_.get("groups", [])
        ]
        return copy_

    def public_group(self, group: dict) -> dict:
        copy_ = copy.deepcopy(group)
        copy_["groupId"] = id_value(copy_["groupId"], self.id_style)
        for member in copy_.get("instances", []):
            member["id"] = id_value(member["id"], self.id_style)
        return copy_

    def bump(self, name: str):
        self.counters[name] = self.counters.get(name, 0) + 1

    def persist(self):
        if not self.state_file:
            return
        payload = self.control_state(include_requests=True, include_reports=True)
        directory = os.path.dirname(os.path.abspath(self.state_file))
        os.makedirs(directory, exist_ok=True)
        temp = f"{self.state_file}.{os.getpid()}.tmp"
        with open(temp, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, indent=2, sort_keys=True)
        os.replace(temp, self.state_file)

    def control_state(
        self, include_requests: bool = True, include_reports: bool = False
    ) -> dict:
        payload = {
            "config": {
                "app_key": self.app_key,
                "token_ttl_s": self.token_ttl_s,
                "id_style": self.id_style,
                "report_style": self.report_style,
                "strict_writes": self.strict_writes,
                "seed_days": self.seed_days,
                "b_lift": self.b_lift,
                "variant_group_id": self.variant_group_id,
            },
            "instances": [self.public_instance(i) for i in self.instances.values()],
            "groups": [self.public_group(g) for g in self.groups],
            "report_facts": len(self.reports),
            "report_facts_data": self.reports if include_reports else None,
            "tokens_active": len(self.tokens),
            "writes": self.writes,
            "idempotency_keys": list(self.idempotency.keys()),
            "faults": self.faults,
            "counters": self.counters,
            "uptime_s": round(time.time() - self.started_at, 3),
        }
        if include_requests:
            payload["requests"] = self.requests[-300:]
        return payload


class Handler(BaseHTTPRequestHandler):
    server_version = "LevelPlaySim/1.0"
    protocol_version = "HTTP/1.1"

    # ------------------------------------------------------------- plumbing
    def log_message(self, fmt, *args):
        if not self.server.state.args.quiet:
            sys.stderr.write("levelplay-sim: " + (fmt % args) + "\n")

    def _read_body(self) -> bytes:
        length = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(length) if length else b""

    def _record(self, method, path, body, status, summary):
        state = self.server.state

        def redact(value):
            return "[redacted]" if value else value

        with state.lock:
            state.requests.append(
                {
                    "at": round(time.time() - state.started_at, 6),
                    "method": method,
                    "path": path,
                    "authorization": redact(self.headers.get("Authorization")),
                    "secretkey": redact(self.headers.get("secretkey")),
                    "refreshToken": redact(self.headers.get("refreshToken")),
                    "idempotency_key": self.headers.get("Idempotency-Key"),
                    "body": body.decode("utf-8", "replace") if body else "",
                    "status": status,
                    "summary": summary,
                }
            )
            if len(state.requests) > 1000:
                del state.requests[: len(state.requests) - 1000]

    def _send(self, status, payload=None, headers=None, raw=None, summary=None):
        body = b""
        content_type = "application/json"
        if raw is not None:
            body = raw
            content_type = "application/octet-stream"
        elif payload is not None:
            body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        try:
            if body:
                self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass

    # -------------------------------------------------------------- routing
    def do_GET(self):
        self._dispatch("GET")

    def do_POST(self):
        self._dispatch("POST")

    def do_PUT(self):
        self._dispatch("PUT")

    def do_DELETE(self):
        self._dispatch("DELETE")

    def _dispatch(self, method):
        parsed = urllib.parse.urlsplit(self.path)
        path = parsed.path
        query = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
        body = self._read_body()
        state = self.server.state

        if path.startswith("/_control/"):
            self._control(method, path, query, body)
            return

        if path == AUTH_PATH:
            self._auth(method, body)
            return
        if path == REPORTING_PATH:
            self._reporting(method, query, body)
            return
        match = INSTANCES_RE.match(path)
        if match:
            self._instances(method, match.group(1), query, body)
            return
        match = GROUPS_RE.match(path)
        if match:
            self._groups(method, match.group(1), query, body)
            return

        self._record(method, self.path, body, 404, "unknown path")
        self._send(404, {"error": "not found"})

    # ---------------------------------------------------------------- faults
    def _endpoint_for(self, method, path):
        if path == AUTH_PATH:
            return "auth"
        if path == REPORTING_PATH:
            return "report"
        if INSTANCES_RE.match(path):
            return "instances" if method == "GET" else "write"
        if GROUPS_RE.match(path):
            return "groups" if method == "GET" else "write"
        return "any"

    def _take_fault(self, endpoint, method, path):
        state = self.server.state
        with state.lock:
            for index, fault in enumerate(state.faults):
                targets = fault.get("endpoint", "any")
                if isinstance(targets, str):
                    targets = [targets]
                if "any" not in targets and endpoint not in targets:
                    continue
                remaining = fault.get("remaining")
                if remaining is not None:
                    if remaining <= 0:
                        state.faults.pop(index)
                        return None
                    fault["remaining"] = remaining - 1
                    if fault["remaining"] <= 0:
                        state.faults.pop(index)
                copied = copy.deepcopy(fault)
                state.persist()
                return copied
        return None

    # ----------------------------------------------------------------- auth
    def _auth(self, method, body):
        state = self.server.state
        endpoint = "auth"
        fault = self._take_fault(endpoint, method, AUTH_PATH)
        if fault:
            self._apply_fault(fault, method, self.path, body)
            if fault.get("mode") in ("status", "timeout", "malformed"):
                return
        if method not in ("GET", "POST"):
            self._record(method, self.path, body, 405, "method not allowed")
            self._send(405, {"error": "method not allowed"})
            return
        secret = self.headers.get("secretkey", "")
        refresh = self.headers.get("refreshToken", "")
        with state.lock:
            state.bump("auth")
            valid = secret == state.secret_key and refresh == state.refresh_token
            if not valid:
                state.persist()
                self._record(method, self.path, body, 401, "invalid credentials")
                self._send(
                    401,
                    {"error": "invalid secret key or refresh token"},
                    headers={"WWW-Authenticate": "Bearer"},
                )
                return
            state.token_seq += 1
            token = f"lp-bearer-{state.token_seq}"
            state.tokens[token] = time.time() + state.token_ttl_s
            state.persist()
        self._record(method, self.path, body, 200, "minted bearer token")
        # The documented answer is a quoted token string.  Accepting a JSON
        # object with `token` is left to the client's parser.
        self._send(200, token)

    def _check_bearer(self, method, path, body) -> bool:
        state = self.server.state
        with state.lock:
            if state.expire_next_bearer > 0:
                state.expire_next_bearer -= 1
                state.persist()
                self._record(method, path, body, 401, "forced bearer expiry")
                self._send(
                    401,
                    {"error": "bearer token expired"},
                    headers={"WWW-Authenticate": "Bearer"},
                )
                return False
            header = self.headers.get("Authorization", "")
            if not header.lower().startswith("bearer "):
                self._record(method, path, body, 401, "missing bearer")
                self._send(
                    401,
                    {"error": "missing bearer token"},
                    headers={"WWW-Authenticate": "Bearer"},
                )
                return False
            token = header[7:].strip()
            expires = state.tokens.get(token)
            if expires is None or expires <= time.time():
                state.tokens.pop(token, None)
                state.persist()
                self._record(method, path, body, 401, "invalid or expired bearer")
                self._send(
                    401,
                    {"error": "invalid or expired bearer token"},
                    headers={"WWW-Authenticate": "Bearer"},
                )
                return False
            return True

    # ------------------------------------------------------------ instances
    def _instances(self, method, raw_app_key, query, body):
        app_key = urllib.parse.unquote(raw_app_key)
        state = self.server.state
        if app_key != state.app_key:
            self._record(method, self.path, body, 404, "unknown app key")
            self._send(404, {"error": "unknown app key"})
            return
        endpoint = "instances" if method == "GET" else "write"
        fault = self._take_fault(endpoint, method, self.path)
        if fault and self._apply_fault(fault, method, self.path, body):
            return
        if not self._check_bearer(method, self.path, body):
            return

        if method == "GET":
            with state.lock:
                state.bump("instances_get")
                instances = [state.public_instance(i) for i in state.instances.values()]
            self._record(method, self.path, body, 200, f"{len(instances)} instances")
            self._send(200, instances)
            return
        if method == "POST":
            self._write_create(body)
            return
        if method == "PUT":
            self._write_update(body)
            return
        if method == "DELETE":
            self._write_delete(query, body)
            return
        self._record(method, self.path, body, 405, "method not allowed")
        self._send(405, {"error": "method not allowed"})

    def _idempotent(self, body):
        key = self.headers.get("Idempotency-Key")
        if not key:
            return None
        state = self.server.state
        fingerprint = hashlib.sha256(
            (self.command + self.path + body.decode("utf-8", "replace")).encode()
        ).hexdigest()
        with state.lock:
            seen = state.idempotency.get(key)
            if seen is None:
                return None
            if seen["fingerprint"] != fingerprint:
                return ("conflict", None)
            state.bump("idempotent_replay")
            return ("replay", seen)

    def _remember_idempotent(self, body, status, payload):
        key = self.headers.get("Idempotency-Key")
        if not key:
            return
        fingerprint = hashlib.sha256(
            (self.command + self.path + body.decode("utf-8", "replace")).encode()
        ).hexdigest()
        with self.server.state.lock:
            self.server.state.idempotency[key] = {
                "fingerprint": fingerprint,
                "status": status,
                "body": payload,
            }

    def _parse_json_body(self, body):
        if not body:
            return None
        try:
            return json.loads(body.decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            return None

    def _write_create(self, body):
        state = self.server.state
        idem = self._idempotent(body)
        if idem and idem[0] == "conflict":
            self._record("POST", self.path, body, 409, "idempotency key conflict")
            self._send(409, {"error": "idempotency key reused with a different payload"})
            return
        if idem and idem[0] == "replay":
            cached = idem[1]
            self._record("POST", self.path, body, cached["status"], "idempotent replay")
            self._send(cached["status"], cached["body"])
            return
        parsed = self._parse_json_body(body)
        if parsed is None:
            self._record("POST", self.path, body, 400, "malformed json")
            self._send(400, {"error": "malformed JSON"})
            return
        legacy = isinstance(parsed, dict)
        if legacy and state.strict_writes:
            self._record("POST", self.path, body, 400, "legacy create shape refused")
            self._send(
                400,
                {
                    "errorsArray": [
                        {
                            "code": "ERR-CONTRACT",
                            "errorMessage": "Instances v4 expects a JSON array; "
                            "the legacy object shape is not the provider contract",
                        }
                    ],
                    "code": 400,
                },
            )
            return
        items = [parsed] if legacy else parsed
        if not isinstance(items, list) or not items:
            self._record("POST", self.path, body, 400, "empty create")
            self._send(400, {"error": "create expects an array of instances"})
            return
        created = []
        with state.lock:
            for item in items:
                if not isinstance(item, dict):
                    continue
                instance_id = state._next_instance_id()
                name = item.get("instanceName") or item.get("name") or f"Instance_{instance_id}"
                network = (
                    item.get("networkName")
                    or item.get("adapterNetwork")
                    or item.get("network")
                    or "unknown"
                )
                rate_raw = item.get("rate")
                if rate_raw is None and "price" in item:
                    # Legacy client: `price` carries micros.
                    rate_raw = decimal_units(int(item["price"]))
                    legacy = True
                try:
                    rate = decimal_units(parse_price_micros(rate_raw)) if rate_raw is not None else "0"
                except (TypeError, ValueError):
                    rate = "0"
                record = {
                    "instanceId": instance_id,
                    "instanceName": str(name),
                    "adUnit": item.get("adUnit", "rewardedVideo"),
                    "adFormat": item.get("adFormat", "rewarded"),
                    "networkName": str(network),
                    "isBidder": bool(item.get("isBidder", False)),
                    "groups": list(item.get("groups", [])),
                    "isLive": bool(item.get("isLive", True)),
                    "rate": rate,
                    "appConfig1": item.get("appConfig1", "appcfg"),
                    "instanceConfig1": item.get("instanceConfig1", "instcfg"),
                }
                state.instances[str(instance_id)] = record
                created.append(record)
                state.writes.append(
                    {
                        "at": round(time.time() - state.started_at, 6),
                        "method": "POST",
                        "operation": "createInstance",
                        "legacy_shape": legacy,
                        "instanceId": instance_id,
                        "rate": rate,
                    }
                )
            state.persist()
        response = {"instanceId": id_value(created[0]["instanceId"], state.id_style)}
        if not legacy:
            response = [response]
        status = 200
        self._remember_idempotent(body, status, response)
        self._record("POST", self.path, body, status, f"created {len(created)}")
        self._send(status, response)

    def _write_update(self, body):
        state = self.server.state
        idem = self._idempotent(body)
        if idem and idem[0] == "conflict":
            self._record("PUT", self.path, body, 409, "idempotency key conflict")
            self._send(409, {"error": "idempotency key reused with a different payload"})
            return
        if idem and idem[0] == "replay":
            cached = idem[1]
            self._record("PUT", self.path, body, cached["status"], "idempotent replay")
            self._send(cached["status"], cached["body"])
            return
        parsed = self._parse_json_body(body)
        if parsed is None:
            self._record("PUT", self.path, body, 400, "malformed json")
            self._send(400, {"error": "malformed JSON"})
            return
        legacy = isinstance(parsed, dict)
        if legacy and state.strict_writes:
            self._record("PUT", self.path, body, 400, "legacy update shape refused")
            self._send(
                400,
                {
                    "errorsArray": [
                        {
                            "code": "ERR-CONTRACT",
                            "errorMessage": "Instances v4 expects an array of "
                            "{instanceId, rate} updates; the legacy object shape "
                            "is not the provider contract",
                        }
                    ],
                    "code": 400,
                },
            )
            return
        updates = [parsed] if legacy else parsed
        if not isinstance(updates, list) or not updates:
            self._record("PUT", self.path, body, 400, "empty update")
            self._send(400, {"error": "update expects an array"})
            return
        errors = []
        applied = []
        with state.lock:
            for item in updates:
                if not isinstance(item, dict):
                    errors.append({"code": "ERR-CONTRACT", "errorMessage": "not an object"})
                    continue
                raw_id = item.get("instanceId", item.get("id", item.get("instance_id")))
                record = state.instances.get(str(raw_id))
                if record is None:
                    errors.append(
                        {
                            "code": "ERR-404",
                            "errorMessage": f"instance {raw_id} not found",
                        }
                    )
                    continue
                rate_raw = item.get("rate")
                legacy_item = False
                if "price" in item and rate_raw is None:
                    # Legacy client: `price` is integer micros, not units.
                    rate_raw = decimal_units(int(item["price"]))
                    legacy_item = True
                if rate_raw is None:
                    continue
                try:
                    units = parse_units(rate_raw)
                except (TypeError, ValueError):
                    errors.append(
                        {
                            "code": "ERR-RATE",
                            "errorMessage": f"rate {rate_raw!r} is not a number",
                        }
                    )
                    continue
                if units < 0.01 or units > 3000:
                    errors.append(
                        {
                            "code": "ERR-RATE",
                            "errorMessage": "rate must be between 0.01 and 3000",
                        }
                    )
                    continue
                record["rate"] = decimal_units(parse_price_micros(rate_raw))
                applied.append((str(raw_id), record["rate"]))
                state.writes.append(
                    {
                        "at": round(time.time() - state.started_at, 6),
                        "method": "PUT",
                        "operation": "updateInstanceRate",
                        "legacy_shape": legacy or legacy_item,
                        "instanceId": raw_id,
                        "rate": record["rate"],
                    }
                )
            if errors:
                state.persist()
                self._record("PUT", self.path, body, 400, f"{len(errors)} rejected")
                self._send(400, {"errorsArray": errors, "code": 400})
                return
            state.persist()
            result = [state.public_instance(i) for i in state.instances.values()]
        self._remember_idempotent(body, 200, result)
        self._record("PUT", self.path, body, 200, f"updated {len(applied)}")
        self._send(200, result)

    def _write_delete(self, query, body):
        state = self.server.state
        idem = self._idempotent(body)
        if idem and idem[0] == "conflict":
            self._record("DELETE", self.path, body, 409, "idempotency key conflict")
            self._send(409, {"error": "idempotency key reused with a different payload"})
            return
        if idem and idem[0] == "replay":
            cached = idem[1]
            self._record("DELETE", self.path, body, cached["status"], "idempotent replay")
            self._send(cached["status"], cached["body"])
            return
        parsed = self._parse_json_body(body)
        legacy = False
        ids = []
        if isinstance(parsed, dict) and isinstance(parsed.get("ids"), list):
            ids = parsed["ids"]
        elif "instanceId" in query:
            ids = query["instanceId"]
            legacy = True
        else:
            self._record("DELETE", self.path, body, 400, "missing ids")
            self._send(400, {"error": "delete expects ids"})
            return
        with state.lock:
            removed = []
            for raw_id in ids:
                if state.instances.pop(str(raw_id), None) is not None:
                    removed.append(raw_id)
                    state.writes.append(
                        {
                            "at": round(time.time() - state.started_at, 6),
                            "method": "DELETE",
                            "operation": "deleteInstance",
                            "legacy_shape": legacy,
                            "instanceId": raw_id,
                        }
                    )
            state.persist()
            result = [state.public_instance(i) for i in state.instances.values()]
        self._remember_idempotent(body, 200, result)
        self._record("DELETE", self.path, body, 200, f"deleted {len(removed)}")
        self._send(200, result)

    # --------------------------------------------------------------- groups
    def _groups(self, method, raw_app_key, query, body):
        app_key = urllib.parse.unquote(raw_app_key)
        state = self.server.state
        if app_key != state.app_key:
            self._record(method, self.path, body, 404, "unknown app key")
            self._send(404, {"error": "unknown app key"})
            return
        endpoint = "groups" if method == "GET" else "write"
        fault = self._take_fault(endpoint, method, self.path)
        if fault and self._apply_fault(fault, method, self.path, body):
            return
        if not self._check_bearer(method, self.path, body):
            return
        if method == "GET":
            with state.lock:
                state.bump("groups_get")
                groups = [state.public_group(g) for g in state.groups]
            self._record(method, self.path, body, 200, f"{len(groups)} groups")
            self._send(200, groups)
            return
        if method == "PUT":
            self._groups_update(body)
            return
        if method == "POST":
            self._groups_create(body)
            return
        if method == "DELETE":
            self._groups_delete(body)
            return
        self._record(method, self.path, body, 405, "method not allowed")
        self._send(405, {"error": "method not allowed"})

    def _groups_update(self, body):
        state = self.server.state
        idem = self._idempotent(body)
        if idem and idem[0] == "replay":
            cached = idem[1]
            self._record("PUT", self.path, body, cached["status"], "idempotent replay")
            self._send(cached["status"], cached["body"])
            return
        if idem and idem[0] == "conflict":
            self._record("PUT", self.path, body, 409, "idempotency key conflict")
            self._send(409, {"error": "idempotency key reused with a different payload"})
            return
        parsed = self._parse_json_body(body)
        updates = [parsed] if isinstance(parsed, dict) else parsed
        if not isinstance(updates, list) or not updates:
            self._record("PUT", self.path, body, 400, "empty group update")
            self._send(400, {"error": "group update expects an array"})
            return
        errors = []
        with state.lock:
            for item in updates:
                if not isinstance(item, dict):
                    continue
                raw_id = item.get("groupId")
                group = next(
                    (g for g in state.groups if str(g["groupId"]) == str(raw_id)), None
                )
                if group is None:
                    errors.append({"code": "ERR-404", "errorMessage": "group not found"})
                    continue
                if "floorPrice" in item:
                    try:
                        floor = parse_units(item["floorPrice"])
                    except (TypeError, ValueError):
                        errors.append({"code": "ERR-FLOOR", "errorMessage": "bad floor"})
                        continue
                    group["floorPrice"] = floor
                    state.writes.append(
                        {
                            "at": round(time.time() - state.started_at, 6),
                            "method": "PUT",
                            "operation": "updateGroupFloor",
                            "groupId": raw_id,
                            "floorPrice": floor,
                        }
                    )
            if errors:
                state.persist()
                self._record("PUT", self.path, body, 400, f"{len(errors)} rejected")
                self._send(400, {"errorsArray": errors, "code": 400})
                return
            state.persist()
            result = [state.public_group(g) for g in state.groups]
        self._remember_idempotent(body, 200, result)
        self._record("PUT", self.path, body, 200, "group floor updated")
        self._send(200, result)

    def _groups_create(self, body):
        state = self.server.state
        parsed = self._parse_json_body(body)
        items = [parsed] if isinstance(parsed, dict) else parsed
        if not isinstance(items, list) or not items:
            self._record("POST", self.path, body, 400, "empty group create")
            self._send(400, {"error": "group create expects an array"})
            return
        created = []
        with state.lock:
            for item in items:
                if not isinstance(item, dict):
                    continue
                group_id = state._next_group_id()
                record = {
                    "groupId": group_id,
                    "groupName": item.get("groupName", f"Group_{group_id}"),
                    "mediationAdUnitId": item.get("mediationAdUnitId", "lp-adunit-1"),
                    "mediationAdUnitName": "rewarded-main",
                    "adFormat": item.get("adFormat", "rewarded"),
                    "abTest": item.get("abTest"),
                    "position": item.get("position", len(state.groups) + 1),
                    "floorPrice": item.get("floorPrice", 0),
                    "countries": item.get("countries", []),
                    "segments": item.get("segments", []),
                    "instances": item.get("instances", []),
                }
                state.groups.append(record)
                created.append(state.public_group(record))
            state.persist()
        self._record("POST", self.path, body, 200, f"created {len(created)} groups")
        self._send(200, created)

    def _groups_delete(self, body):
        state = self.server.state
        parsed = self._parse_json_body(body)
        ids = parsed.get("ids", []) if isinstance(parsed, dict) else []
        with state.lock:
            before = len(state.groups)
            state.groups = [
                g for g in state.groups if str(g["groupId"]) not in {str(i) for i in ids}
            ]
            state.persist()
            result = [state.public_group(g) for g in state.groups]
        self._record("DELETE", self.path, body, 200, f"deleted {before - len(state.groups)}")
        self._send(200, result)

    # ------------------------------------------------------------- reporting
    def _reporting(self, method, query, body):
        state = self.server.state
        fault = self._take_fault("report", method, self.path)
        if fault and self._apply_fault(fault, method, self.path, body):
            return
        if not self._check_bearer(method, self.path, body):
            return
        if method != "GET":
            self._record(method, self.path, body, 405, "method not allowed")
            self._send(405, {"error": "method not allowed"})
            return

        def first(name, default=None):
            values = query.get(name)
            return values[0] if values else default

        app_key = first("appKey")
        if app_key and state.app_key not in [v.strip() for v in app_key.split(",")]:
            self._record(method, self.path, body, 403, "app key not authorized")
            self._send(403, {"error": "app key not authorized"})
            return
        start = first("startDate")
        end = first("endDate")
        if not start or not end:
            self._record(method, self.path, body, 400, "missing date range")
            self._send(400, {"error": "startDate and endDate are required"})
            return
        breakdowns = [
            part.strip() for part in (first("breakdowns", "date") or "").split(",") if part.strip()
        ]
        metrics = [
            part.strip() for part in (first("metrics", "revenue") or "").split(",") if part.strip()
        ]
        try:
            page = max(1, int(first("page", "1")))
        except ValueError:
            page = 1
        try:
            results_per_page = max(1, int(first("resultsPerPage", str(state.default_page_size))))
        except ValueError:
            results_per_page = state.default_page_size

        rows = self._report_rows(start, end, breakdowns, metrics)
        total = len(rows)
        start_index = (page - 1) * results_per_page
        page_rows = rows[start_index : start_index + results_per_page]

        truncate = fault and fault.get("mode") == "truncate"
        if truncate and page_rows:
            page_rows = page_rows[:1]

        state.bump("report")
        if state.report_style == "legacy":
            payload = {"code": 200, "count": len(page_rows), "results": page_rows}
        else:
            payload = {
                "data": page_rows,
                "page": page,
                "pageSize": len(page_rows),
                "totalResults": total,
            }
        note = f"{len(page_rows)}/{total} rows page {page}"
        if truncate:
            note += " (truncated)"
        self._record(method, self.path, body, 200, note)
        self._send(200, payload)

    def _report_rows(self, start, end, breakdowns, metrics):
        state = self.server.state
        canonical = {
            "date": "date",
            "abTest": "abTest",
            "country": "country",
            "adUnit": "mediationAdUnitId",
            "mediationAdUnitId": "mediationAdUnitId",
            "mediationAdUnit": "mediationAdUnitId",
            "adFormat": "adFormat",
            "adNetwork": "networkName",
            "networkName": "networkName",
        }
        unknown = [b for b in breakdowns if b not in canonical]
        if unknown:
            return []
        keys = [canonical[b] for b in breakdowns]
        output_names = list(breakdowns)
        groups: dict[tuple, dict] = {}
        with state.lock:
            facts = list(state.reports)
        for fact in facts:
            if fact["date"] < start or fact["date"] > end:
                continue
            group_key = tuple(str(fact.get(key, "")) for key in keys)
            bucket = groups.setdefault(
                group_key,
                {"revenue": 0.0, "activeUsers": 0, "impressions": 0},
            )
            bucket["revenue"] += float(fact["revenue"])
            bucket["activeUsers"] += int(fact["activeUsers"])
            bucket["impressions"] += int(fact["impressions"])
        rows = []
        for group_key in sorted(groups):
            bucket = groups[group_key]
            row = {}
            for name, value in zip(output_names, group_key):
                if name == "adUnit":
                    row["mediationAdUnitId"] = value
                elif name == "adNetwork":
                    row["networkName"] = value
                elif name == "mediationAdUnit":
                    row["mediationAdUnitId"] = value
                else:
                    row[name] = value
            if "revenue" in metrics or not metrics:
                row["revenue"] = round(bucket["revenue"], 6)
            if "activeUsers" in metrics:
                row["activeUsers"] = bucket["activeUsers"]
            if "impressions" in metrics:
                row["impressions"] = bucket["impressions"]
            rows.append(row)
        return rows

    # --------------------------------------------------------------- control
    def _control(self, method, path, query, body):
        state = self.server.state
        parsed = self._parse_json_body(body) if body else {}
        if parsed is None:
            parsed = {}
        action = path[len("/_control/") :].strip("/")

        if action == "state" and method == "GET":
            with state.lock:
                payload = state.control_state(include_requests=True, include_reports=False)
            self._send(200, payload)
            return
        if action == "reports" and method == "GET":
            with state.lock:
                payload = state.control_state(include_requests=False, include_reports=True)
            self._send(200, payload)
            return
        if action == "requests" and method == "GET":
            since = int(query.get("since", ["0"])[0])
            with state.lock:
                payload = state.requests[since:]
            self._send(200, payload)
            return
        if action == "reset" and method == "POST":
            with state.lock:
                state._seed()
                state.persist()
            self._send(200, {"ok": True})
            return
        if action == "expire_tokens" and method == "POST":
            with state.lock:
                state.tokens.clear()
                state.persist()
            self._send(200, {"ok": True, "tokens_active": 0})
            return
        if action == "expire_next_bearer" and method == "POST":
            with state.lock:
                state.expire_next_bearer += int(parsed.get("count", 1))
                state.persist()
            self._send(200, {"ok": True, "expire_next_bearer": state.expire_next_bearer})
            return
        if action == "fault" and method == "POST":
            fault = dict(parsed)
            if "mode" not in fault:
                self._send(400, {"error": "fault needs a mode"})
                return
            if "remaining" not in fault:
                fault["remaining"] = 1
            with state.lock:
                state.faults.append(fault)
                state.persist()
            self._send(200, {"ok": True, "faults": state.faults})
            return
        if action == "clear_faults" and method == "POST":
            with state.lock:
                state.faults = []
                state.persist()
            self._send(200, {"ok": True})
            return
        if action == "instance" and method == "POST":
            raw_id = parsed.get("instanceId")
            with state.lock:
                record = state.instances.get(str(raw_id))
                if record is None:
                    self._send(404, {"error": "instance not found"})
                    return
                if "rate" in parsed:
                    record["rate"] = decimal_units(parse_price_micros(parsed["rate"]))
                state.persist()
                payload = state.public_instance(record)
            self._send(200, payload)
            return
        if action == "group_floor" and method == "POST":
            raw_id = parsed.get("groupId")
            with state.lock:
                group = next(
                    (g for g in state.groups if str(g["groupId"]) == str(raw_id)), None
                )
                if group is None:
                    self._send(404, {"error": "group not found"})
                    return
                group["floorPrice"] = parse_units(parsed.get("floorPrice"))
                state.persist()
                payload = state.public_group(group)
            self._send(200, payload)
            return
        if action == "report" and method == "POST":
            facts = parsed.get("facts")
            if facts is None:
                facts = [parsed]
            if not isinstance(facts, list):
                self._send(400, {"error": "facts must be an array"})
                return
            with state.lock:
                state.reports.extend(facts)
                state.persist()
            self._send(200, {"ok": True, "report_facts": len(state.reports)})
            return
        if action == "shutdown" and method == "POST":
            self._send(200, {"ok": True})
            threading.Thread(target=self.server.shutdown, daemon=True).start()
            return

        self._send(404, {"error": "unknown control action"})

    # ---------------------------------------------------------------- faults
    def _apply_fault(self, fault, method, path, body) -> bool:
        """Apply a non-truncate fault. Returns True when the request is done."""
        mode = fault.get("mode")
        if mode == "status":
            status = int(fault.get("status", 500))
            headers = {}
            if "retry_after" in fault:
                headers["Retry-After"] = str(fault["retry_after"])
            payload = fault.get("body")
            if payload is None:
                payload = {"error": f"injected status {status}"}
            self._record(method, path, body, status, f"fault status {status}")
            self._send(status, payload, headers=headers)
            return True
        if mode == "timeout":
            delay = float(fault.get("delay_ms", 11_000)) / 1000.0
            self._record(method, path, body, 0, f"fault timeout {delay}s")
            time.sleep(delay)
            try:
                self._send(200, {"late": True})
            except Exception:
                pass
            return True
        if mode == "malformed":
            self._record(method, path, body, 200, "fault malformed JSON")
            self._send(200, raw=b'{"data":[', summary="malformed")
            return True
        # `truncate` is handled inside the reporting builder.
        if mode == "truncate":
            return False
        self._record(method, path, body, 500, f"unknown fault mode {mode}")
        self._send(500, {"error": f"unknown fault mode {mode}"})
        return True


class Server(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, address, state):
        super().__init__(address, Handler)
        self.state = state


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=18081)
    parser.add_argument("--secret-key", default="lp-secret")
    parser.add_argument("--refresh-token", default="lp-refresh")
    parser.add_argument("--app-key", default="lp-app-key")
    parser.add_argument("--token-ttl", type=int, default=86_400)
    parser.add_argument(
        "--id-style",
        choices=["numeric", "string"],
        default="numeric",
        help="documented v4 responses use numeric ids; `string` is the "
        "compatibility shape the current apex autopilot adapter requires",
    )
    parser.add_argument(
        "--report-style",
        choices=["v1", "legacy"],
        default="v1",
        help="v1 is the documented data/page/pageSize/totalResults envelope; "
        "legacy is the code/count/results shape older clients expect",
    )
    parser.add_argument(
        "--strict-writes",
        action="store_true",
        help="refuse the legacy object write shapes (documented v4 uses arrays)",
    )
    parser.add_argument("--page-size", type=int, default=5000)
    parser.add_argument("--seed-days", type=int, default=120)
    parser.add_argument(
        "--b-lift",
        type=float,
        default=1.03,
        help="revenue multiplier for the B arm in the seeded report facts",
    )
    parser.add_argument(
        "--variant-group-id",
        default=None,
        help="use this id for the abTest B group (compatibility probe for "
        "callers that use the group id as the experiment handle)",
    )
    parser.add_argument("--state-file", default=None)
    parser.add_argument("--quiet", action="store_true")
    return parser


def main(argv=None):
    args = build_parser().parse_args(argv)
    state = State(args)
    state.persist()
    server = Server((args.host, args.port), state)
    actual_port = server.server_address[1]
    if not args.quiet:
        print(
            f"levelplay-sim listening on http://{args.host}:{actual_port} "
            f"(id_style={args.id_style}, report_style={args.report_style})",
            flush=True,
        )

    def stop(_signum, _frame):
        threading.Thread(target=server.shutdown, daemon=True).start()

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        server.serve_forever(poll_interval=0.05)
    finally:
        state.persist()
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
