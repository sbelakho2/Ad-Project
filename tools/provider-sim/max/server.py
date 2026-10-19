#!/usr/bin/env python3
"""Stateful AppLovin MAX provider simulator (management + reporting).

The simulator implements the documented MAX contract the Rust client in
``rust/crates/apex-connectors/src/max.rs`` consumes:

* Management API (``<base>/mediation/v1``), authenticated with the ``Api-Key``
  header: ``GET /ad_units``, ``GET /ad_unit/{id}``, ``POST /ad_unit/{id}``,
  ``GET|POST /ad_unit_experiment/{id}``. The write shape replaces the whole
  ``bid_floors`` list, exactly like the provider.
* Revenue Reporting API (``<base>/maxReport``), authenticated with the
  ``api_key`` query parameter: ``columns``, ``start``, ``end``, ``format``,
  ``limit``, ``offset``, ``filter_<column>`` and ``not_zero``. Every numeric
  value is a JSON string.

State lives in a JSON file so a floor written through the real client is
visible to the next read and to an external verifier. The file also records
the experiments, generated/seeded report rows and the request log.

Fault injection is available from the environment (startup defaults) and per
request (``X-Max-Sim-Fault`` / ``sim_fault``). Each fault can fire once or
persistently:

    auth            401 (or the configured status)
    rate_limit      429 with Retry-After
    server_error    500
    slow            sleep ``delay_ms`` then answer normally
    malformed       200 with an invalid JSON body
    truncated       first page then an empty page while ``count`` says more
    empty           empty array / empty results

Control endpoints (not part of the provider contract) live under ``/__sim``:

    GET  /__sim/health
    GET  /__sim/state
    GET  /__sim/requests
    POST /__sim/reset
    POST /__sim/fault
    DELETE /__sim/fault
    POST /__sim/report/seed
    POST /__sim/experiment/effect

Run:

    python3 tools/provider-sim/max/server.py --port 18080

Environment: MAX_SIM_STATE, MAX_SIM_API_KEY, MAX_SIM_REPORT_KEY,
MAX_SIM_COUNT_MODE (page|total), MAX_SIM_FAULT, MAX_SIM_FAULT_TARGET,
MAX_SIM_FAULT_MODE, MAX_SIM_FAULT_DELAY_MS, MAX_SIM_FAULT_RETRY_AFTER,
MAX_SIM_RESET.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import hashlib
import io
import json
import os
import pathlib
import random
import re
import sys
import threading
import time
import urllib.parse
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DEFAULT_API_KEY = "sim-management-key"
DEFAULT_REPORT_KEY = "sim-report-key"
MANAGEMENT_PREFIX = "/mediation/v1"
REPORT_PATH = "/maxReport"
MAX_REPORT_WINDOW_DAYS = 45
DEFAULT_PAGE_LIMIT = 1000

SCRIPT_DIR = pathlib.Path(__file__).resolve().parent


def _default_state_path() -> pathlib.Path:
    return pathlib.Path(os.environ.get("MAX_SIM_STATE", SCRIPT_DIR / ".state" / "state.json"))


def _iso_now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def _micros(text: str) -> int:
    text = text.strip()
    if text.startswith("-"):
        raise ValueError("negative decimals are not accepted")
    if "." in text:
        whole, frac = text.split(".", 1)
    else:
        whole, frac = text, ""
    if not whole.isdigit() or (frac and not frac.isdigit()):
        raise ValueError(f"invalid decimal {text!r}")
    frac = (frac + "000000")[:6]
    return int(whole or "0") * 1_000_000 + int(frac or "0")


def _decimal(micros: int) -> str:
    sign = "-" if micros < 0 else ""
    micros = abs(micros)
    whole, frac = divmod(micros, 1_000_000)
    if frac == 0:
        return f"{sign}{whole}"
    return f"{sign}{whole}.{frac:06d}".rstrip("0")


def _parse_day(text: str) -> dt.date:
    return dt.date.fromisoformat(text)


def _day_ordinal(day: dt.date) -> int:
    return (day - dt.date(1970, 1, 1)).days


def _hash_int(*parts: object) -> int:
    raw = "|".join(str(part) for part in parts).encode("utf-8")
    return int.from_bytes(hashlib.sha256(raw).digest()[:8], "big")


def default_state() -> dict:
    return {
        "version": 1,
        "created_at": _iso_now(),
        "updated_at": _iso_now(),
        "api_key": os.environ.get("MAX_SIM_API_KEY", DEFAULT_API_KEY),
        "report_key": os.environ.get("MAX_SIM_REPORT_KEY", DEFAULT_REPORT_KEY),
        "apps": [
            {
                "id": "app_android_demo",
                "name": "Apex Demo (Android)",
                "platform": "android",
                "package_name": "com.apex.demo",
            },
            {
                "id": "app_ios_demo",
                "name": "Apex Demo (iOS)",
                "platform": "ios",
                "package_name": "com.apex.demo.ios",
            },
        ],
        "ad_units": [
            {
                "id": "rw_demo_01",
                "name": "Apex Rewarded",
                "platform": "android",
                "package_name": "com.apex.demo",
                "ad_format": "REWARD",
                "disabled": False,
                "bid_floors": [
                    {
                        "country_group_name": "t1",
                        "cpm": "2.5",
                        "countries": {"type": "INCLUDE", "values": ["us", "ca"]},
                    },
                    {
                        "country_group_name": "ee",
                        "cpm": "1.5",
                        "countries": {"type": "INCLUDE", "values": ["ee"]},
                    },
                ],
            },
            {
                "id": "bn_demo_01",
                "name": "Apex Banner",
                "platform": "ios",
                "package_name": "com.apex.demo.ios",
                "ad_format": "BANNER",
                "disabled": False,
                "bid_floors": [
                    {
                        "country_group_name": "default",
                        "cpm": "0.5",
                        "countries": {"type": "INCLUDE", "values": ["us", "gb"]},
                    }
                ],
            },
        ],
        "experiments": {},
        "report": {
            "count_mode": os.environ.get("MAX_SIM_COUNT_MODE", "page"),
            "generate": True,
            "seed_days": 45,
            "seeds": [
                {
                    "ad_unit_id": "rw_demo_01",
                    "countries": ["us", "ca", "ee"],
                    "base_revenue": "12.5",
                    "base_impressions": 10000,
                },
                {
                    "ad_unit_id": "bn_demo_01",
                    "countries": ["us", "gb"],
                    "base_revenue": "1.25",
                    "base_impressions": 50000,
                },
            ],
            "rows": [],
        },
        "faults": [],
        "requests": [],
    }


class Simulator:
    def __init__(self, state_path: pathlib.Path):
        self.state_path = state_path
        self.lock = threading.RLock()
        self.state = self._load()
        env_fault = os.environ.get("MAX_SIM_FAULT", "").strip()
        if env_fault:
            self.add_fault(
                {
                    "fault": env_fault,
                    "target": os.environ.get("MAX_SIM_FAULT_TARGET", "all"),
                    "mode": os.environ.get("MAX_SIM_FAULT_MODE", "once"),
                    "delay_ms": int(os.environ.get("MAX_SIM_FAULT_DELAY_MS", "0")),
                    "retry_after": os.environ.get("MAX_SIM_FAULT_RETRY_AFTER", "0"),
                    "status": int(os.environ.get("MAX_SIM_FAULT_STATUS", "0")),
                }
            )

    # --- persistence -------------------------------------------------
    def _load(self) -> dict:
        if os.environ.get("MAX_SIM_RESET") == "1" or not self.state_path.is_file():
            state = default_state()
            self._save(state)
            return state
        try:
            state = json.loads(self.state_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise SystemExit(f"max-sim: cannot read state {self.state_path}: {error}")
        state.setdefault("faults", [])
        state.setdefault("requests", [])
        state.setdefault("experiments", {})
        return state

    def _save(self, state: dict | None = None) -> None:
        state = self.state if state is None else state
        state["updated_at"] = _iso_now()
        self.state_path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.state_path.with_suffix(self.state_path.suffix + ".tmp")
        tmp.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        os.replace(tmp, self.state_path)

    # --- requests ----------------------------------------------------
    def log_request(self, record: dict) -> None:
        with self.lock:
            self.state.setdefault("requests", []).append(record)
            self.state["requests"] = self.state["requests"][-200:]
            # The request log is diagnostics, not provider state: keep it out
            # of the atomic write on every request to avoid disk churn.
            # GET /__sim/requests reads it from memory until the next save.
        with self.lock:
            pass

    # --- faults ------------------------------------------------------
    def add_fault(self, spec: dict) -> dict:
        fault = {
            "id": f"fault_{int(time.time() * 1_000_000)}_{random.randrange(1000):03d}",
            "fault": spec.get("fault", "server_error"),
            "target": spec.get("target", "all"),
            "mode": spec.get("mode", "once"),
            "delay_ms": int(spec.get("delay_ms", 0)),
            "retry_after": str(spec.get("retry_after", "0")),
            "status": int(spec.get("status", 0)),
            "remaining": int(spec.get("remaining", 1)),
            "signature": None,
            "created_at": _iso_now(),
        }
        with self.lock:
            self.state.setdefault("faults", []).append(fault)
            self._save()
        return fault

    def clear_faults(self) -> None:
        with self.lock:
            self.state["faults"] = []
            self._save()

    @staticmethod
    def _target_matches(target: str, route: str) -> bool:
        if target in ("all", route):
            return True
        if target == "management":
            return route.startswith("management:")
        if target == "reporting":
            return route == "maxReport"
        aliases = {
            "list_ad_units": "management:list_ad_units",
            "ad_units": "management:list_ad_units",
            "ad_unit": "management:ad_unit",
            "update_floors": "management:ad_unit",
            "experiment": "management:ad_unit_experiment",
            "experiments": "management:ad_unit_experiment",
            "create_experiment": "management:ad_unit_experiment",
            "get_experiment": "management:ad_unit_experiment",
            "maxReport": "maxReport",
            "report": "maxReport",
            "reporting": "maxReport",
            "apps": "management:apps",
        }
        return aliases.get(target) == route

    def select_fault(self, route: str, query: dict, headers) -> dict | None:
        with self.lock:
            candidates = list(self.state.get("faults", []))
        for fault in candidates:
            if not self._target_matches(fault.get("target", "all"), route):
                continue
            if fault.get("mode") == "persistent":
                return fault
            if fault.get("remaining", 0) > 0:
                return fault
            if fault.get("fault") == "truncated" and fault.get("signature"):
                return fault
        return None

    def consume_fault(self, fault: dict) -> None:
        with self.lock:
            for known in self.state.get("faults", []):
                if known.get("id") == fault.get("id"):
                    if known.get("mode") != "persistent":
                        known["remaining"] = max(0, int(known.get("remaining", 0)) - 1)
                    self._save()
                    return

    # --- management helpers -----------------------------------------
    def find_unit(self, unit_id: str) -> dict | None:
        for unit in self.state["ad_units"]:
            if unit["id"] == unit_id:
                return unit
        return None

    def active_experiment(self, unit_id: str) -> dict | None:
        experiment = self.state.get("experiments", {}).get(unit_id)
        if experiment and experiment.get("state") == "active":
            return experiment
        return None

    def render_unit(self, unit: dict, with_floors: bool) -> dict:
        rendered = {
            "id": unit["id"],
            "name": unit["name"],
            "platform": unit["platform"],
            "package_name": unit["package_name"],
            "ad_format": unit["ad_format"],
            "has_active_experiment": self.active_experiment(unit["id"]) is not None,
            "disabled": bool(unit.get("disabled", False)),
        }
        if with_floors:
            rendered["bid_floors"] = json.loads(json.dumps(unit.get("bid_floors", [])))
        return rendered

    def validate_floors(self, floors: object) -> list[dict]:
        if not isinstance(floors, list):
            raise ValueError("bid_floors must be an array")
        out = []
        for group in floors:
            if not isinstance(group, dict):
                raise ValueError("each bid floor group must be an object")
            name = group.get("country_group_name")
            cpm = group.get("cpm")
            countries = group.get("countries")
            if not isinstance(name, str) or not name:
                raise ValueError("bid floor group requires country_group_name")
            if not isinstance(cpm, str):
                raise ValueError("bid floor group requires a string cpm")
            _micros(cpm)  # raises on garbage
            if not isinstance(countries, dict) or countries.get("type") != "INCLUDE":
                raise ValueError("bid floor countries must be an INCLUDE object")
            values = countries.get("values")
            if not isinstance(values, list) or not all(isinstance(v, str) and v for v in values):
                raise ValueError("bid floor countries.values must be a string array")
            out.append(
                {
                    "country_group_name": name,
                    "cpm": cpm,
                    "countries": {"type": "INCLUDE", "values": list(values)},
                }
            )
        return out

    # --- report helpers ---------------------------------------------
    def experiment_for_day(self, unit_id: str, day: dt.date) -> dict | None:
        experiment = self.state.get("experiments", {}).get(unit_id)
        if not experiment:
            return None
        created = _parse_day(experiment["created_day"])
        if day < created:
            return None
        ended = experiment.get("ended_day")
        if ended and day >= _parse_day(ended):
            return None
        return experiment

    def generated_rows(self, start: dt.date, end: dt.date) -> list[dict]:
        report = self.state["report"]
        if not report.get("generate", True):
            return []
        rows = []
        seeds = report.get("seeds", [])
        day = start
        while day <= end:
            for seed in seeds:
                unit = self.find_unit(seed["ad_unit_id"])
                if unit is None:
                    continue
                base_revenue = _micros(str(seed.get("base_revenue", "0")))
                base_impressions = int(seed.get("base_impressions", 0))
                experiment = self.experiment_for_day(unit["id"], day)
                arms = [{"label": "", "share": 1.0, "lift_bp": 0}]
                if experiment:
                    allocation = float(experiment.get("test_group_allocation", 25)) / 100.0
                    arms = [
                        {"label": "Control", "share": 1.0 - allocation, "lift_bp": 0},
                        {
                            "label": experiment["experiment_name"],
                            "share": allocation,
                            "lift_bp": int(experiment.get("sim_lift_bp", 0)),
                        },
                    ]
                for country in seed.get("countries", []):
                    for arm in arms:
                        h = _hash_int(_day_ordinal(day), unit["id"], country, arm["label"])
                        value = 900 + h % 201  # 0.900 .. 1.100
                        impressions = max(0, round(base_impressions * (value / 1000.0)))
                        impressions = round(impressions * arm["share"])
                        revenue = base_revenue * value // 1000
                        revenue = round(revenue * arm["share"])
                        revenue = revenue * (10_000 + int(arm["lift_bp"])) // 10_000
                        rows.append(
                            {
                                "day": day.isoformat(),
                                "country": country,
                                "max_ad_unit_id": unit["id"],
                                "max_ad_unit_test": arm["label"],
                                "estimated_revenue": _decimal(revenue),
                                "impressions": str(impressions),
                                "application": unit["name"],
                                "platform": unit["platform"],
                                "package_name": unit["package_name"],
                                "ad_format": unit["ad_format"],
                                "ecpm": _decimal(
                                    revenue * 1_000 // impressions if impressions else 0
                                ),
                            }
                        )
            day += dt.timedelta(days=1)
        return rows

    def report_rows(self, start: dt.date, end: dt.date) -> list[dict]:
        rows = self.generated_rows(start, end)
        # Explicit seeded rows overlay generated rows for the same key.
        by_key = {
            (r["day"], r.get("country"), r.get("max_ad_unit_id"), r.get("max_ad_unit_test")): r
            for r in rows
        }
        for row in self.state["report"].get("rows", []):
            day = row.get("day")
            if not day or not (start.isoformat() <= day <= end.isoformat()):
                continue
            key = (day, row.get("country"), row.get("max_ad_unit_id"), row.get("max_ad_unit_test"))
            by_key[key] = row
        return list(by_key.values())


SIM: Simulator


class Handler(BaseHTTPRequestHandler):
    server_version = "ApexMaxSim/1.0"
    protocol_version = "HTTP/1.1"

    # --- HTTP plumbing ----------------------------------------------
    def log_message(self, fmt: str, *args) -> None:  # keep the test console quiet
        if os.environ.get("MAX_SIM_VERBOSE") == "1":
            sys.stderr.write("max-sim: " + (fmt % args) + "\n")

    def _body(self) -> bytes:
        length = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(length) if length else b""

    def _json_body(self) -> dict:
        raw = self._body()
        if not raw:
            return {}
        try:
            value = json.loads(raw)
        except json.JSONDecodeError as error:
            raise ValueError(f"request body is not JSON: {error}")
        if not isinstance(value, dict):
            raise ValueError("request body must be a JSON object")
        return value

    def _send(self, status: int, body: bytes, headers: dict | None = None) -> None:
        self.send_response(status)
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def _json(self, status: int, value: object, headers: dict | None = None) -> None:
        body = json.dumps(value, separators=(",", ":")).encode("utf-8")
        merged = {"Content-Type": "application/json"}
        merged.update(headers or {})
        self._send(status, body, merged)

    def _text(self, status: int, value: str, content_type: str = "text/plain") -> None:
        self._send(status, value.encode("utf-8"), {"Content-Type": content_type})

    # --- dispatch ----------------------------------------------------
    def do_GET(self) -> None:
        self._dispatch("GET")

    def do_POST(self) -> None:
        self._dispatch("POST")

    def do_DELETE(self) -> None:
        self._dispatch("DELETE")

    def _dispatch(self, method: str) -> None:
        parsed = urllib.parse.urlsplit(self.path)
        # Keep the path percent-encoded for routing: an id may itself contain
        # an encoded slash, so split segments first and unquote each id later.
        path = parsed.path
        query = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
        if not path.startswith("/__sim"):
            SIM.log_request(
                {
                    "at": _iso_now(),
                    "method": method,
                    "path": path,
                    "query": {key: values for key, values in query.items()},
                    "has_api_key_header": bool(self.headers.get("Api-Key")),
                    "fault": (query.get("sim_fault") or [None])[0]
                    or self.headers.get("X-Max-Sim-Fault"),
                }
            )
        try:
            if path.startswith("/__sim"):
                self._control(method, path, query)
                return
            if path == REPORT_PATH:
                self._reporting(method, query)
                return
            if path == MANAGEMENT_PREFIX or path.startswith(MANAGEMENT_PREFIX + "/"):
                self._management(method, path, query)
                return
            self._json(HTTPStatus.NOT_FOUND, {"error": f"no route for {method} {path}"})
        except BrokenPipeError:
            return
        except ValueError as error:
            self._json(HTTPStatus.BAD_REQUEST, {"error": str(error)})
        except Exception as error:  # noqa: BLE001 - simulator boundary
            self._json(HTTPStatus.INTERNAL_SERVER_ERROR, {"error": f"simulator: {error}"})

    # --- auth + faults ----------------------------------------------
    def _management_auth(self) -> bool:
        supplied = self.headers.get("Api-Key")
        if not supplied:
            self._json(
                HTTPStatus.UNAUTHORIZED,
                {"error": "missing Api-Key header"},
            )
            return False
        if supplied != SIM.state["api_key"]:
            self._json(HTTPStatus.FORBIDDEN, {"error": "invalid management key"})
            return False
        return True

    def _report_auth(self, query: dict) -> bool:
        supplied = (query.get("api_key") or [None])[0]
        if not supplied:
            self._json(HTTPStatus.UNAUTHORIZED, {"code": 401, "count": 0, "results": []})
            return False
        if supplied != SIM.state["report_key"]:
            self._json(HTTPStatus.FORBIDDEN, {"code": 403, "count": 0, "results": []})
            return False
        return True

    def _request_fault(self, route: str, query: dict) -> dict | None:
        # Request knobs win over persistent faults.
        header_fault = self.headers.get("X-Max-Sim-Fault")
        query_fault = (query.get("sim_fault") or [None])[0]
        kind = (header_fault or query_fault or "").strip()
        if kind:
            return {
                "id": "ephemeral",
                "fault": kind,
                "target": route,
                "mode": (query.get("sim_fault_mode") or ["once"])[0],
                "delay_ms": int((query.get("sim_delay_ms") or ["0"])[0]),
                "retry_after": (query.get("sim_retry_after") or ["0"])[0],
                "status": int((query.get("sim_status") or ["0"])[0]),
                "remaining": 1,
                "signature": None,
            }
        return SIM.select_fault(route, query, self.headers)

    def _apply_status_fault(
        self, route: str, query: dict, offset: int
    ) -> dict | None:
        """Return a fault descriptor the caller must handle, or None.

        Status faults are answered here. Data-shaping faults (slow, malformed,
        empty, truncated) are returned to the route so it can shape its own
        payload.
        """
        fault = self._request_fault(route, query)
        if fault is None:
            return None
        kind = fault.get("fault", "")
        if kind == "slow":
            delay = max(0, int(fault.get("delay_ms", 0)))
            SIM.consume_fault(fault)
            if delay:
                time.sleep(delay / 1000.0)
            return None
        if kind in ("auth", "invalid_auth"):
            status = int(fault.get("status", 0)) or HTTPStatus.UNAUTHORIZED
            SIM.consume_fault(fault)
            self._json(status, {"error": "simulated credential refusal"})
            return fault
        if kind in ("rate_limit", "429"):
            status = int(fault.get("status", 0)) or HTTPStatus.TOO_MANY_REQUESTS
            retry_after = str(fault.get("retry_after", "0"))
            SIM.consume_fault(fault)
            self._json(
                status,
                {"error": "simulated rate limit"},
                {"Retry-After": retry_after},
            )
            return fault
        if kind in ("server_error", "500"):
            status = int(fault.get("status", 0)) or HTTPStatus.INTERNAL_SERVER_ERROR
            SIM.consume_fault(fault)
            self._text(status, "simulated upstream failure\n")
            return fault
        if kind == "truncated":
            signature = self._report_signature(query)
            if fault.get("id") == "ephemeral":
                fault["signature"] = signature
                return fault
            with SIM.lock:
                for known in SIM.state.get("faults", []):
                    if known.get("id") == fault.get("id"):
                        if known.get("signature") and offset == 0 and known.get("remaining", 0) <= 0:
                            # A new report sequence after the fault already
                            # fired: the once-mode fault is spent.
                            known["signature"] = None
                            SIM._save()
                            return None
                        if known.get("signature") is None:
                            known["signature"] = signature
                            if known.get("mode") != "persistent":
                                known["remaining"] = max(
                                    0, int(known.get("remaining", 0)) - 1
                                )
                            SIM._save()
                        return dict(known)
            return fault
        if kind in ("empty", "malformed"):
            SIM.consume_fault(fault)
            return fault
        return fault

    def _report_signature(self, query: dict) -> str:
        parts = []
        for key in sorted(query):
            if key in ("offset", "limit", "sim_fault", "sim_fault_mode", "sim_delay_ms"):
                continue
            parts.append(f"{key}={query[key]}")
        return "&".join(parts)

    # --- management --------------------------------------------------
    def _management(self, method: str, path: str, query: dict) -> None:
        rel = path[len(MANAGEMENT_PREFIX) :] or "/"
        route, offset = self._management_route(rel)
        fault = self._apply_status_fault(route, query, offset)
        if fault is not None and self._fault_answered(fault):
            return
        if not self._management_auth():
            return

        empty = fault is not None and fault.get("fault") == "empty"
        malformed = fault is not None and fault.get("fault") == "malformed"

        if rel == "/apps" and method == "GET":
            if malformed:
                self._send(200, b'{"apps":', {"Content-Type": "application/json"})
                return
            self._json(200, [] if empty else SIM.state.get("apps", []))
            return
        if rel.startswith("/apps/") and method == "GET":
            app_id = urllib.parse.unquote(rel[len("/apps/") :])
            app = next((a for a in SIM.state.get("apps", []) if a["id"] == app_id), None)
            if app is None or empty:
                self._json(404, {"error": "app not found"})
            else:
                self._json(200, app)
            return
        if rel == "/ad_units" and method == "GET":
            self._list_ad_units(query, empty, malformed, fault)
            return
        if rel.startswith("/ad_unit/") and rel.count("/") == 2:
            unit_id = urllib.parse.unquote(rel[len("/ad_unit/") :])
            if method == "GET":
                self._get_ad_unit(unit_id, query, empty, malformed)
                return
            if method == "POST":
                self._update_ad_unit(unit_id, empty, malformed)
                return
        if rel.startswith("/ad_unit_experiment/") and rel.count("/") == 2:
            unit_id = urllib.parse.unquote(rel[len("/ad_unit_experiment/") :])
            if method == "GET":
                self._get_experiment(unit_id, query, empty, malformed)
                return
            if method == "POST":
                self._write_experiment(unit_id, empty, malformed)
                return
        self._json(HTTPStatus.NOT_FOUND, {"error": f"no management route for {method} {rel}"})

    @staticmethod
    def _management_route(rel: str) -> tuple[str, int]:
        if rel == "/ad_units":
            return "management:list_ad_units", 0
        if rel.startswith("/ad_unit_experiment/"):
            return "management:ad_unit_experiment", 0
        if rel.startswith("/ad_unit/"):
            return "management:ad_unit", 0
        if rel.startswith("/apps"):
            return "management:apps", 0
        return "management:unknown", 0

    def _fault_answered(self, fault: dict) -> bool:
        return fault.get("fault") in (
            "auth",
            "invalid_auth",
            "rate_limit",
            "429",
            "server_error",
            "500",
        )

    def _list_ad_units(self, query: dict, empty: bool, malformed: bool, fault: dict | None) -> None:
        with_floors = "bid_floors" in self._fields(query)
        units = [
            unit
            for unit in sorted(SIM.state["ad_units"], key=lambda u: u["id"])
            if not unit.get("disabled", False)
        ]
        limit = max(1, min(10_000, int((query.get("limit") or [DEFAULT_PAGE_LIMIT])[0])))
        offset = max(0, int((query.get("offset") or ["0"])[0]))
        page = units[offset : offset + limit]
        if fault and fault.get("fault") == "truncated":
            signature = fault.get("signature")
            if signature and offset > 0:
                page = []
            else:
                page = page[:1]
        if empty:
            page = []
        body = [SIM.render_unit(unit, with_floors) for unit in page]
        if malformed:
            self._send(200, b'[{"id":', {"Content-Type": "application/json"})
            return
        self._json(200, body)

    @staticmethod
    def _fields(query: dict) -> set[str]:
        raw = (query.get("fields") or [""])[0]
        return {field.strip() for field in raw.split(",") if field.strip()}

    def _get_ad_unit(self, unit_id: str, query: dict, empty: bool, malformed: bool) -> None:
        unit = SIM.find_unit(unit_id)
        if unit is None or empty:
            self._json(404, {"error": f"ad unit {unit_id} not found"})
            return
        if malformed:
            self._send(200, b'{"id":', {"Content-Type": "application/json"})
            return
        self._json(200, SIM.render_unit(unit, "bid_floors" in self._fields(query)))

    def _update_ad_unit(self, unit_id: str, empty: bool, malformed: bool) -> None:
        unit = SIM.find_unit(unit_id)
        if unit is None:
            self._json(404, {"error": f"ad unit {unit_id} not found"})
            return
        body = self._json_body()
        declared = body.get("id")
        if declared is not None and declared != unit_id:
            raise ValueError("body id does not match the ad unit path")
        with SIM.lock:
            for field in ("name", "platform", "package_name", "ad_format"):
                if field in body:
                    if not isinstance(body[field], str) or not body[field]:
                        raise ValueError(f"{field} must be a non-empty string")
                    unit[field] = body[field]
            if "bid_floors" in body:
                unit["bid_floors"] = SIM.validate_floors(body["bid_floors"])
            SIM._save()
        if empty:
            self._json(200, {})
            return
        if malformed:
            self._send(200, b'{"id":', {"Content-Type": "application/json"})
            return
        self._json(200, SIM.render_unit(unit, True))

    def _experiment_payload(self, experiment: dict, with_floors: bool) -> dict:
        payload = {
            "id": experiment["ad_unit_id"],
            "experiment_name": experiment["experiment_name"],
            "platform": experiment.get("platform"),
            "ad_format": experiment.get("ad_format"),
            "package_name": experiment.get("package_name"),
            "disabled": bool(experiment.get("disabled", False)),
            "promote": experiment.get("state") == "promoted",
            "deprecate": experiment.get("state") == "deprecated",
            "test_group_allocation": experiment.get("test_group_allocation", 25),
        }
        if with_floors:
            payload["bid_floors"] = json.loads(json.dumps(experiment.get("bid_floors", [])))
        return payload

    def _get_experiment(self, unit_id: str, query: dict, empty: bool, malformed: bool) -> None:
        experiment = SIM.state.get("experiments", {}).get(unit_id)
        if experiment is None or empty:
            self._json(404, {"error": f"no experiment for ad unit {unit_id}"})
            return
        if malformed:
            self._send(200, b'{"id":', {"Content-Type": "application/json"})
            return
        self._json(
            200,
            self._experiment_payload(experiment, "bid_floors" in self._fields(query)),
        )

    def _write_experiment(self, unit_id: str, empty: bool, malformed: bool) -> None:
        unit = SIM.find_unit(unit_id)
        if unit is None:
            self._json(404, {"error": f"ad unit {unit_id} not found"})
            return
        body = self._json_body()
        if "promote" in body or "deprecate" in body:
            self._promote_or_deprecate(unit, body, empty, malformed)
            return
        self._create_experiment(unit, body, empty, malformed)

    def _create_experiment(self, unit: dict, body: dict, empty: bool, malformed: bool) -> None:
        if body.get("id") not in (None, ""):
            raise ValueError("create experiment must omit id")
        name = body.get("experiment_name")
        if not isinstance(name, str) or not name.strip():
            raise ValueError("experiment_name is required")
        allocation = body.get("test_group_allocation")
        if allocation not in (5, 10, 25, 50):
            raise ValueError("test_group_allocation must be 5, 10, 25 or 50")
        if SIM.active_experiment(unit["id"]) is not None:
            self._json(
                HTTPStatus.BAD_REQUEST,
                {"error": "ad unit already has an active experiment"},
            )
            return
        floors = body.get("bid_floors")
        if floors is None:
            floors = json.loads(json.dumps(unit.get("bid_floors", [])))
        else:
            floors = SIM.validate_floors(floors)
        now = dt.datetime.now(dt.timezone.utc)
        experiment = {
            "ad_unit_id": unit["id"],
            "experiment_name": name,
            "test_group_allocation": allocation,
            "bid_floors": floors,
            "state": "active",
            "created_day": now.date().isoformat(),
            "created_at": now.replace(microsecond=0).isoformat(),
            "sim_lift_bp": int(body.get("sim_lift_bp", 0)),
            "platform": unit["platform"],
            "ad_format": unit["ad_format"],
            "package_name": unit["package_name"],
            "disabled": False,
        }
        with SIM.lock:
            SIM.state["experiments"][unit["id"]] = experiment
            SIM._save()
        if empty:
            self._json(200, {})
            return
        if malformed:
            self._send(200, b'{"id":', {"Content-Type": "application/json"})
            return
        self._json(200, self._experiment_payload(experiment, True))

    def _promote_or_deprecate(
        self, unit: dict, body: dict, empty: bool, malformed: bool
    ) -> None:
        experiment = SIM.active_experiment(unit["id"])
        if experiment is None:
            self._json(HTTPStatus.BAD_REQUEST, {"error": "no active experiment"})
            return
        declared = body.get("id")
        if declared != unit["id"]:
            raise ValueError("promote/deprecate requires the parent ad unit id")
        name = body.get("experiment_name")
        if not isinstance(name, str) or name != experiment["experiment_name"]:
            raise ValueError("experiment_name does not match the active experiment")
        promote = bool(body.get("promote", False))
        deprecate = bool(body.get("deprecate", False))
        if promote == deprecate:
            raise ValueError("exactly one of promote/deprecate must be true")
        now = dt.datetime.now(dt.timezone.utc)
        with SIM.lock:
            if promote:
                unit["bid_floors"] = json.loads(json.dumps(experiment.get("bid_floors", [])))
                experiment["state"] = "promoted"
                message = "Experiment successfully promoted"
            else:
                experiment["state"] = "deprecated"
                message = "Experiment successfully deprecated"
            experiment["ended_day"] = now.date().isoformat()
            experiment["ended_at"] = now.replace(microsecond=0).isoformat()
            SIM._save()
        if empty:
            self._json(200, {})
            return
        if malformed:
            self._send(200, b'{"message":', {"Content-Type": "application/json"})
            return
        self._json(200, {"message": message})

    # --- reporting ---------------------------------------------------
    def _reporting(self, method: str, query: dict) -> None:
        if method != "GET":
            self._json(HTTPStatus.METHOD_NOT_ALLOWED, {"code": 405, "count": 0, "results": []})
            return
        offset = max(0, int((query.get("offset") or ["0"])[0]))
        fault = self._apply_status_fault("maxReport", query, offset)
        if fault is not None and self._fault_answered(fault):
            return
        if not self._report_auth(query):
            return

        if fault and fault.get("fault") == "malformed":
            self._send(
                HTTPStatus.OK,
                b'{"code":200,"count":1,"results":[',
                {"Content-Type": "application/json"},
            )
            return
        empty = bool(fault and fault.get("fault") == "empty")
        truncated = bool(fault and fault.get("fault") == "truncated")

        columns = [
            column.strip()
            for column in (query.get("columns") or [""])[0].split(",")
            if column.strip()
        ]
        if not columns:
            self._json(HTTPStatus.BAD_REQUEST, {"code": 400, "count": 0, "results": [],
                                                "error": "columns is required"})
            return
        try:
            start = _parse_day((query.get("start") or [""])[0])
            end = _parse_day((query.get("end") or [""])[0])
        except ValueError:
            self._json(HTTPStatus.BAD_REQUEST, {"code": 400, "count": 0, "results": [],
                                                "error": "start/end must be YYYY-MM-DD"})
            return
        if end < start:
            self._json(HTTPStatus.BAD_REQUEST, {"code": 400, "count": 0, "results": [],
                                                "error": "end is before start"})
            return
        if (end - start).days + 1 > MAX_REPORT_WINDOW_DAYS:
            self._json(
                HTTPStatus.BAD_REQUEST,
                {
                    "code": 400,
                    "count": 0,
                    "results": [],
                    "error": (
                        f"request window is {(end - start).days + 1} days; "
                        f"the MAX reporting API allows {MAX_REPORT_WINDOW_DAYS}"
                    ),
                },
            )
            return

        rows = [] if empty else SIM.report_rows(start, end)
        for key, values in query.items():
            if not key.startswith("filter_"):
                continue
            column = key[len("filter_") :]
            wanted = values[0] if values else ""
            rows = [row for row in rows if str(row.get(column, "")) == wanted]
        if (query.get("not_zero") or ["0"])[0] == "1":
            rows = [
                row
                for row in rows
                if str(row.get("estimated_revenue", "0")) != "0"
                or str(row.get("impressions", "0")) != "0"
            ]
        rows.sort(
            key=lambda row: (
                row.get("day", ""),
                row.get("country", ""),
                row.get("max_ad_unit_id", ""),
                row.get("max_ad_unit_test", ""),
            )
        )
        total = len(rows)
        limit = max(1, min(10_000, int((query.get("limit") or [DEFAULT_PAGE_LIMIT])[0])))
        offset = max(0, int((query.get("offset") or ["0"])[0]))
        page = rows[offset : offset + limit]
        if truncated:
            # The provider answered the first page, then returned an empty
            # page while still advertising the original total: the client
            # must not mistake that for a complete report.
            if offset > 0:
                page = []
            else:
                page = page[: max(1, len(page) // 2)]
        count_mode = SIM.state["report"].get("count_mode", "page")
        count = total if count_mode == "total" or truncated else len(page)
        requested = [{column: str(row.get(column, "")) for column in columns} for row in page]

        if (query.get("format") or ["json"])[0] == "csv":
            buffer = io.StringIO()
            writer = csv.writer(buffer)
            writer.writerow(columns)
            for row in page:
                writer.writerow([str(row.get(column, "")) for column in columns])
            self._text(HTTPStatus.OK, buffer.getvalue(), "text/csv")
            return

        self._json(
            HTTPStatus.OK,
            {"code": 200, "results": requested, "count": count},
        )

    # --- control plane ----------------------------------------------
    def _control(self, method: str, path: str, query: dict) -> None:
        if path == "/__sim/health" and method == "GET":
            self._json(200, {"ok": True, "state": str(SIM.state_path)})
            return
        if path == "/__sim/state" and method == "GET":
            with SIM.lock:
                self._json(200, json.loads(json.dumps(SIM.state)))
            return
        if path == "/__sim/requests" and method == "GET":
            with SIM.lock:
                self._json(200, list(SIM.state.get("requests", [])))
            return
        if path == "/__sim/reset" and method == "POST":
            seed = self._json_body()
            with SIM.lock:
                SIM.state = default_state()
                for key in ("api_key", "report_key", "apps", "ad_units", "experiments"):
                    if key in seed:
                        SIM.state[key] = seed[key]
                if "report" in seed:
                    SIM.state["report"].update(seed["report"])
                SIM._save()
            self._json(200, {"ok": True})
            return
        if path == "/__sim/fault" and method == "POST":
            spec = self._json_body()
            fault = SIM.add_fault(spec)
            self._json(200, fault)
            return
        if path == "/__sim/fault" and method == "DELETE":
            SIM.clear_faults()
            self._json(200, {"ok": True})
            return
        if path == "/__sim/report/seed" and method == "POST":
            body = self._json_body()
            with SIM.lock:
                if "rows" in body:
                    if not isinstance(body["rows"], list):
                        raise ValueError("rows must be a list")
                    SIM.state["report"].setdefault("rows", []).extend(body["rows"])
                if "seeds" in body:
                    if not isinstance(body["seeds"], list):
                        raise ValueError("seeds must be a list")
                    SIM.state["report"]["seeds"] = body["seeds"]
                if "generate" in body:
                    SIM.state["report"]["generate"] = bool(body["generate"])
                if "count_mode" in body:
                    if body["count_mode"] not in ("page", "total"):
                        raise ValueError("count_mode must be page or total")
                    SIM.state["report"]["count_mode"] = body["count_mode"]
                SIM._save()
            self._json(200, {"ok": True, "rows": len(SIM.state["report"].get("rows", []))})
            return
        if path == "/__sim/experiment/effect" and method == "POST":
            body = self._json_body()
            unit_id = body.get("ad_unit_id")
            with SIM.lock:
                experiment = SIM.state.get("experiments", {}).get(unit_id)
                if experiment is None:
                    self._json(404, {"error": f"no experiment for {unit_id}"})
                    return
                experiment["sim_lift_bp"] = int(body.get("lift_bp", 0))
                SIM._save()
            self._json(200, SIM._experiment_payload(experiment, False))
            return
        self._json(HTTPStatus.NOT_FOUND, {"error": f"no control route for {method} {path}"})


def main() -> int:
    parser = argparse.ArgumentParser(description="Stateful AppLovin MAX simulator")
    parser.add_argument("--bind", default=os.environ.get("MAX_SIM_BIND", "127.0.0.1"))
    parser.add_argument("--port", type=int, default=int(os.environ.get("MAX_SIM_PORT", "18080")))
    parser.add_argument("--state", default=str(_default_state_path()))
    args = parser.parse_args()
    global SIM
    SIM = Simulator(pathlib.Path(args.state))
    server = ThreadingHTTPServer((args.bind, args.port), Handler)
    server.daemon_threads = True
    print(
        f"max-sim: listening on http://{args.bind}:{args.port} "
        f"(management {MANAGEMENT_PREFIX}, reporting {REPORT_PATH}, state {args.state})",
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
