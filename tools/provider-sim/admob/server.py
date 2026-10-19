#!/usr/bin/env python3
"""Stateful AdMob v1beta provider simulator (Python standard library only).

The simulator models the surface `apex-connectors/src/admob.rs` and the
`apex-runner` AdMob adapter actually call, and keeps the canonical v1beta
resource shape alongside the legacy shape the connector parses:

  OAuth2
    POST /oauth2/token, /v1/oauth2/token, /token
      grant_type=refresh_token  (refresh token + optional client credentials)
      grant_type=client_credentials
    Access tokens expire (default 3600s) and expired/unknown tokens answer 401.

  Reads
    GET  /v1beta/accounts
    GET  /v1beta/accounts/{account}
    GET  /v1beta/accounts/{account}/apps[/{app}]
    GET  /v1beta/accounts/{account}/adUnits[/{unit}]
    GET  /v1beta/accounts/{account}/mediationGroups[/{group}]
      Paginated with pageSize/pageToken. A mediation group response carries the
      canonical `mediationGroupLines`/`targeting` shape *and* the legacy
      `adUnitMappings`/`format` shape the connector parses.

  Writes
    PATCH /v1beta/accounts/{account}/mediationGroups/{group}
      Accepts the legacy full `adUnitMappings` list (what the connector sends)
      or canonical `mediationGroupLines`. Mutable state, Idempotency-Key
      replay/conflict handling, `updateMask` tolerated.
    POST /v1beta/accounts/{account}/mediationGroups/{group}/mediationAbExperiments
    POST /v1beta/accounts/{account}/mediationAbExperiments            (legacy)
    POST .../mediationAbExperiments/{experiment}:stop                  (both)
      Lifecycle state is preserved and stop returns canonical v1beta enums.

  Reports
    POST /v1beta/accounts/{account}/mediationReport:generate
      Dimensions DATE, COUNTRY, AD_UNIT_ID, APP_ID, MEDIATION_GROUP_ID,
      NETWORK, AD_FORMAT, PLATFORM (any subset). Metrics ESTIMATED_EARNINGS
      (microsValue) and IMPRESSIONS (integerValue). Paginated.

  Faults (all injected per target, consumed once unless count is given)
    401, 429+Retry-After, 500, timeout, malformed_json, pagination_truncation,
    empty_body.
    Targets: any, groups.list, groups.get, groups.patch, report,
    experiment.create, experiment.stop, apps.list, adunits.list, token.

  Admin (no auth; loopback simulator only)
    GET  /__admin/state
    GET  /__admin/requests
    POST /__admin/faults          {"target": "...", "kind": "...", "count": n,
                                   "retry_after": seconds, "seconds": n}
    POST /__admin/expire-tokens   {"all": true} or {"token": "..."}
    POST /__admin/token           {"account": "...", "ttl_seconds": n}
    POST /__admin/seed-report     {"rows": [{day, country, adUnitId, ...}]}
    POST /__admin/reset

Run `python3 tools/provider-sim/admob/server.py --help` for the flags. On
startup one JSON line is printed to stdout with the bound port and the
bootstrap credentials.
"""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import decimal
import json
import os
import sys
import threading
import time
from collections import deque
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlsplit

MAX_PAGE_SIZE = 1000
DEFAULT_PAGE_SIZE = 100
TOKEN_TYPE = "Bearer"


# ---------------------------------------------------------------------------
# money


def parse_money_to_micros(raw) -> int:
    """Parse an AdMob `ecpm` currency value into integer micros.

    The legacy mapping shape spells eCPM as a currency amount ("1.75" or
    1.75), while canonical `cpmMicros` is integer micros. Both are accepted
    here, but a bare number is a currency amount exactly like the real
    resource: 1.75 is $1.75, never 1.75 micros.
    """
    if isinstance(raw, bool) or raw is None:
        raise ValueError("eCPM must be a number or decimal string")
    if isinstance(raw, int):
        text = str(raw)
    elif isinstance(raw, float):
        text = repr(raw)
    elif isinstance(raw, str):
        text = raw.strip()
    else:
        raise ValueError("eCPM must be a number or decimal string")
    if not text:
        raise ValueError("eCPM is empty")
    try:
        amount = decimal.Decimal(text)
    except decimal.InvalidOperation as error:
        raise ValueError(f"eCPM {text!r} is not a decimal") from error
    if not amount.is_finite() or amount < 0:
        raise ValueError(f"eCPM {text!r} is not a non-negative decimal")
    micros = amount * 1_000_000
    if micros != micros.to_integral_value():
        raise ValueError(f"eCPM {text!r} has more than six fractional digits")
    return int(micros)


def format_micros_as_currency(micros: int) -> str:
    """Integer micros -> the decimal currency string used by legacy `ecpm`."""
    whole, frac = divmod(int(micros), 1_000_000)
    if frac == 0:
        return str(whole)
    return f"{whole}.{frac:06d}".rstrip("0")


def parse_cpm_micros(raw) -> int:
    if isinstance(raw, bool) or raw is None:
        raise ValueError("cpmMicros must be an integer string")
    if isinstance(raw, int):
        value = raw
    elif isinstance(raw, str) and raw.strip().lstrip("-").isdigit():
        value = int(raw.strip())
    else:
        raise ValueError("cpmMicros must be an integer string")
    if value < 0:
        raise ValueError("cpmMicros must be non-negative")
    return value


# ---------------------------------------------------------------------------
# dates


def parse_date(value, field: str) -> dt.date:
    if not isinstance(value, dict):
        raise ValueError(f"{field} must be a Date object")
    try:
        year = int(value["year"])
        month = int(value["month"])
        day = int(value["day"])
        return dt.date(year, month, day)
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError(f"{field} is not a valid calendar date: {error}") from error


def date_key(day: dt.date) -> str:
    return day.isoformat()


def day_range(start: dt.date, end: dt.date):
    current = start
    while current <= end:
        yield current
        current += dt.timedelta(days=1)


# ---------------------------------------------------------------------------
# state


class SimState:
    def __init__(
        self,
        account_id: str,
        access_token: str,
        refresh_token: str,
        client_id: str,
        client_secret: str,
        token_ttl: int,
        seed_days: int,
        state_file: str | None,
    ) -> None:
        self.lock = threading.RLock()
        self.account_id = account_id
        self.state_file = state_file
        self.clients = {client_id: client_secret}
        self.refresh_tokens = {
            refresh_token: {
                "account": account_id,
                "client_id": client_id,
                "client_secret": client_secret,
            }
        }
        self.tokens: dict[str, dict] = {}
        self.faults: dict[str, deque] = {}
        self.idempotency: dict[tuple[str, str], dict] = {}
        self.requests: deque = deque(maxlen=4000)
        self.accounts: dict[str, dict] = {}
        self._seed(account_id, seed_days)
        if state_file and os.path.isfile(state_file):
            self._load(state_file)
        self._mint_token(account_id, token_ttl, access_token)

    def _load(self, state_file: str) -> None:
        try:
            with open(state_file, "r", encoding="utf-8") as handle:
                payload = json.load(handle)
        except (OSError, json.JSONDecodeError):
            return
        if isinstance(payload.get("accounts"), dict) and payload["accounts"]:
            self.accounts = payload["accounts"]
        if isinstance(payload.get("tokens"), dict):
            self.tokens = payload["tokens"]
        if isinstance(payload.get("refresh_tokens"), dict):
            self.refresh_tokens = payload["refresh_tokens"]
        if isinstance(payload.get("clients"), dict):
            self.clients = payload["clients"]
        for entry in payload.get("idempotency", []):
            if isinstance(entry, dict) and "account" in entry and "key" in entry:
                key = (entry["account"], entry["key"])
                self.idempotency[key] = {
                    field: value
                    for field, value in entry.items()
                    if field not in ("account", "key")
                }

    # -- bootstrap ---------------------------------------------------------

    def _seed(self, account: str, seed_days: int) -> None:
        app_id = "ca-app-pub-1234567890123456~1111111111"
        unit_us = "ca-app-pub-1234567890123456/2222222222"
        unit_de = "ca-app-pub-1234567890123456/3333333333"
        unit_fr = "ca-app-pub-1234567890123456/4444444444"
        groups = {
            "g1": self._canonical_group(
                account,
                "g1",
                "Interstitial group",
                "INTERSTITIAL",
                [
                    ("line-admob", "admob", 2_000_000, [unit_us, unit_de]),
                    ("line-meta", "facebook", 1_500_000, [unit_fr]),
                ],
            ),
            "g2": self._canonical_group(
                account,
                "g2",
                "Rewarded group",
                "REWARDED",
                [("line-unity", "unityads", 3_250_000, [unit_us])],
            ),
        }
        today = dt.datetime.now(dt.timezone.utc).date()
        rows = []
        for offset in range(seed_days, 0, -1):
            day = today - dt.timedelta(days=offset)
            rows.extend(
                [
                    {
                        "day": date_key(day),
                        "country": "US",
                        "adUnitId": unit_us,
                        "appId": app_id,
                        "mediationGroupId": "g1",
                        "network": "admob",
                        "platform": "ANDROID",
                        "adFormat": "INTERSTITIAL",
                        "impressions": 100_000,
                        "revenueMicros": 12_000_000,
                    },
                    {
                        "day": date_key(day),
                        "country": "DE",
                        "adUnitId": unit_us,
                        "appId": app_id,
                        "mediationGroupId": "g1",
                        "network": "admob",
                        "platform": "ANDROID",
                        "adFormat": "INTERSTITIAL",
                        "impressions": 40_000,
                        "revenueMicros": 2_000_000,
                    },
                    {
                        "day": date_key(day),
                        "country": "FR",
                        "adUnitId": unit_fr,
                        "appId": app_id,
                        "mediationGroupId": "g1",
                        "network": "facebook",
                        "platform": "ANDROID",
                        "adFormat": "INTERSTITIAL",
                        "impressions": 20_000,
                        "revenueMicros": 1_000_000,
                    },
                ]
            )
        self.accounts[account] = {
            "account": {
                "name": f"accounts/{account}",
                "publisherId": account,
                "currencyCode": "USD",
                "reportingTimeZone": "America/Los_Angeles",
            },
            "apps": {
                app_id: {
                    "name": f"accounts/{account}/apps/{app_id}",
                    "appId": app_id,
                    "displayName": "Northstar",
                    "platform": "ANDROID",
                    "state": "ENABLED",
                }
            },
            "adUnits": {
                unit_us: {
                    "name": f"accounts/{account}/adUnits/{unit_us}",
                    "adUnitId": unit_us,
                    "displayName": "Rewarded / US",
                    "appId": app_id,
                    "adFormat": "REWARDED",
                    "state": "ENABLED",
                },
                unit_de: {
                    "name": f"accounts/{account}/adUnits/{unit_de}",
                    "adUnitId": unit_de,
                    "displayName": "Interstitial / DE",
                    "appId": app_id,
                    "adFormat": "INTERSTITIAL",
                    "state": "ENABLED",
                },
                unit_fr: {
                    "name": f"accounts/{account}/adUnits/{unit_fr}",
                    "adUnitId": unit_fr,
                    "displayName": "Interstitial / FR",
                    "appId": app_id,
                    "adFormat": "INTERSTITIAL",
                    "state": "ENABLED",
                },
            },
            "groups": groups,
            "experiments": {},
            "reportRows": rows,
            "reportSeq": 0,
            "experimentSeq": 0,
            "lineSeq": 0,
        }

    def _canonical_group(self, account, group_id, display_name, ad_format, lines):
        targeting = {
            "platform": "ANDROID",
            "format": ad_format,
            "adUnitIds": [unit for _, _, _, units in lines for unit in units],
        }
        canonical_lines = {}
        for index, (line_id, network, micros, units) in enumerate(lines, start=1):
            canonical_lines[line_id] = {
                "id": line_id,
                "displayName": f"{display_name} / {network}",
                "adSourceId": network,
                "network": network,
                "cpmMode": "MANUAL",
                "cpmMicros": str(micros),
                "adUnitMappings": {unit: f"accounts/{account}/adUnitMappings/{unit}" for unit in units},
                "state": "ENABLED",
                "experimentVariant": "ORIGINAL",
            }
        return {
            "name": f"accounts/{account}/mediationGroups/{group_id}",
            "mediationGroupId": group_id,
            "displayName": display_name,
            "targeting": targeting,
            "state": "ENABLED",
            "mediationGroupLines": canonical_lines,
            "mediationAbExperimentState": "NOT_RUNNING",
            "createTime": "2026-01-01T00:00:00Z",
        }

    def _mint_token(self, account: str, ttl_seconds: int, fixed: str | None = None) -> str:
        token = fixed or base64.urlsafe_b64encode(os.urandom(24)).decode().rstrip("=")
        self.tokens[token] = {
            "account": account,
            "issued_at": time.time(),
            "expires_at": time.time() + max(0, int(ttl_seconds)),
            "scope": "https://www.googleapis.com/auth/admob.readonly",
        }
        return token

    # -- persistence -------------------------------------------------------

    def save(self) -> None:
        if not self.state_file:
            return
        with self.lock:
            payload = {
                "accounts": self.accounts,
                "tokens": self.tokens,
                "refresh_tokens": self.refresh_tokens,
                "clients": self.clients,
                "idempotency": [
                    {"account": key[0], "key": key[1], **value}
                    for key, value in self.idempotency.items()
                ],
            }
            os.makedirs(os.path.dirname(os.path.abspath(self.state_file)) or ".", exist_ok=True)
            temp = f"{self.state_file}.{os.getpid()}.tmp"
            with open(temp, "w", encoding="utf-8") as handle:
                json.dump(payload, handle, separators=(",", ":"))
            os.replace(temp, self.state_file)

    # -- accessors ---------------------------------------------------------

    def account(self, account_id: str) -> dict | None:
        with self.lock:
            return self.accounts.get(account_id)

    def require_account(self, account_id: str) -> dict:
        account = self.account(account_id)
        if account is None:
            raise KeyError(f"account '{account_id}' not found")
        return account

    def log(self, entry: dict) -> None:
        with self.lock:
            self.requests.append(entry)

    def take_fault(self, target: str):
        with self.lock:
            for key in (target, "any"):
                queue = self.faults.get(key)
                if queue:
                    fault = queue.popleft()
                    if not queue:
                        self.faults.pop(key, None)
                    return fault
        return None


# ---------------------------------------------------------------------------
# group views


def canonical_to_legacy(group: dict) -> dict:
    """The connector's mapping view over the canonical group lines."""
    mappings = []
    for line in group.get("mediationGroupLines", {}).values():
        if line.get("state") == "REMOVED":
            continue
        micros = parse_cpm_micros(line.get("cpmMicros", "0"))
        network = line.get("network") or line.get("adSourceId") or ""
        for unit in sorted(line.get("adUnitMappings", {})):
            mappings.append(
                {
                    "adUnitId": unit,
                    "network": network,
                    "ecpm": format_micros_as_currency(micros),
                    "eCPM": format_micros_as_currency(micros),
                }
            )
    mappings.sort(key=lambda item: item["adUnitId"])
    return {
        "name": group["name"],
        "mediationGroupId": group["mediationGroupId"],
        "displayName": group.get("displayName", ""),
        "format": str(group.get("targeting", {}).get("format", "")).lower(),
        "adUnitMappings": mappings,
        "mediationGroupLines": group.get("mediationGroupLines", {}),
        "targeting": group.get("targeting", {}),
        "state": group.get("state", "ENABLED"),
        "mediationAbExperimentState": group.get("mediationAbExperimentState", "NOT_RUNNING"),
    }


def apply_legacy_mappings(group: dict, mappings: list, account_id: str) -> None:
    if not isinstance(mappings, list):
        raise ValueError("adUnitMappings must be an array")
    lines = group.setdefault("mediationGroupLines", {})
    next_line = len(lines) + 1
    used_units = set()
    new_lines = {}
    for mapping in mappings:
        if not isinstance(mapping, dict):
            raise ValueError("each adUnitMapping must be an object")
        unit = mapping.get("adUnitId")
        network = mapping.get("network")
        if not isinstance(unit, str) or not unit.strip():
            raise ValueError("adUnitMapping missing adUnitId")
        if not isinstance(network, str) or not network.strip():
            raise ValueError("adUnitMapping missing network")
        if unit in used_units:
            raise ValueError(f"duplicate adUnitId '{unit}' in adUnitMappings")
        used_units.add(unit)
        raw = mapping.get("cpmMicros")
        if raw is not None:
            micros = parse_cpm_micros(raw)
        else:
            raw = mapping.get("ecpm", mapping.get("eCPM"))
            if raw is None:
                raise ValueError(f"adUnitMapping {unit} missing ecpm")
            micros = parse_money_to_micros(raw)
        line_id = None
        for existing_id, existing in lines.items():
            if set(existing.get("adUnitMappings", {})) == {unit}:
                line_id = existing_id
                break
        if line_id is None:
            line_id = f"line-{next_line}"
            next_line += 1
        new_lines[line_id] = {
            "id": line_id,
            "displayName": f"{group.get('displayName', 'group')} / {network}",
            "adSourceId": network,
            "network": network,
            "cpmMode": "MANUAL",
            "cpmMicros": str(micros),
            "adUnitMappings": {
                unit: f"accounts/{account_id}/adUnitMappings/{unit}",
            },
            "state": "ENABLED",
            "experimentVariant": "ORIGINAL",
        }
    group["mediationGroupLines"] = new_lines
    group.setdefault("targeting", {})["adUnitIds"] = sorted(used_units)


def apply_canonical_lines(group: dict, lines: dict) -> None:
    if not isinstance(lines, dict):
        raise ValueError("mediationGroupLines must be an object")
    for line_id, line in lines.items():
        if not isinstance(line, dict):
            raise ValueError(f"mediationGroupLines[{line_id}] must be an object")
        for unit in line.get("adUnitMappings", {}):
            if not isinstance(unit, str) or not unit:
                raise ValueError("adUnitMappings keys must be ad unit ids")
        if "cpmMicros" in line:
            parse_cpm_micros(line["cpmMicros"])
    group["mediationGroupLines"] = lines
    units = sorted(
        {
            unit
            for line in lines.values()
            for unit in line.get("adUnitMappings", {})
            if line.get("state") != "REMOVED"
        }
    )
    group.setdefault("targeting", {})["adUnitIds"] = units


def _idempotency_hash(payload) -> str:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"))


# ---------------------------------------------------------------------------
# HTTP handler factory


def make_handler(state: SimState):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        server_version = "AdMobSim/1.0"

        # -- low level ----------------------------------------------------

        def log_message(self, fmt, *args):  # noqa: A003
            if os.environ.get("ADMOB_SIM_VERBOSE"):
                sys.stderr.write("admob-sim: " + fmt % args + "\n")

        def _read_body(self) -> bytes:
            length = int(self.headers.get("Content-Length", "0") or "0")
            return self.rfile.read(length) if length else b""

        def _json_body(self):
            raw = self._read_body()
            if not raw:
                return None
            try:
                return json.loads(raw)
            except json.JSONDecodeError as error:
                raise ValueError(f"request body is not JSON: {error}") from error

        def _send(self, status, body: bytes, headers=None, content_type="application/json"):
            if isinstance(body, str):
                body = body.encode()
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            for name, value in (headers or {}).items():
                self.send_header(name, value)
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def _json(self, status, payload, headers=None):
            self._send(
                status,
                json.dumps(payload, separators=(",", ":"), ensure_ascii=False),
                headers=headers,
            )

        def _error(self, status, message, headers=None):
            body = {
                "error": {
                    "code": status,
                    "message": message,
                    "status": {
                        400: "INVALID_ARGUMENT",
                        401: "UNAUTHENTICATED",
                        403: "PERMISSION_DENIED",
                        404: "NOT_FOUND",
                        409: "ALREADY_EXISTS",
                        429: "RESOURCE_EXHAUSTED",
                        500: "INTERNAL",
                        503: "UNAVAILABLE",
                        504: "DEADLINE_EXCEEDED",
                    }.get(status, "UNKNOWN"),
                }
            }
            self._json(status, body, headers=headers)

        # -- auth ---------------------------------------------------------

        def _bearer(self):
            header = self.headers.get("Authorization", "")
            if not header.lower().startswith("bearer "):
                return None
            return header.split(None, 1)[1].strip()

        def _authorize(self, account_id: str | None) -> bool:
            token = self._bearer()
            if not token:
                self._error(401, "Request had no bearer credentials", {"WWW-Authenticate": "Bearer"})
                return False
            with state.lock:
                record = state.tokens.get(token)
                expired = record is not None and record["expires_at"] <= time.time()
                if record is None or expired:
                    if record is not None:
                        state.tokens.pop(token, None)
                    self._error(
                        401,
                        "Request had invalid authentication credentials",
                        {"WWW-Authenticate": 'Bearer error="invalid_token"'},
                    )
                    return False
                if account_id is not None and record["account"] != account_id:
                    self._error(403, "The token does not belong to this account")
                    return False
            return True

        # -- fault injection ----------------------------------------------

        def _fault(self, target: str) -> bool:
            fault = state.take_fault(target)
            if not fault:
                return False
            kind = fault.get("kind")
            state.log({"event": "fault", "target": target, "kind": kind})
            if kind == "401":
                self._error(401, "Injected token expiry", {"WWW-Authenticate": "Bearer"})
            elif kind == "429":
                retry_after = int(fault.get("retry_after", 1))
                self._error(
                    429,
                    "Injected rate limit",
                    {"Retry-After": str(retry_after)},
                )
            elif kind == "500":
                self._error(500, "Injected internal error")
            elif kind == "timeout":
                time.sleep(float(fault.get("seconds", 11)))
                self._error(504, "Injected upstream timeout")
            elif kind == "malformed_json":
                self._send(200, '{"rows": [{"metricValues": ', content_type="application/json")
            elif kind == "empty_body":
                self._send(200, b"")
            elif kind == "pagination_truncation":
                # Not a response of its own: the list/report code below
                # observes the marker and truncates the first page.
                self._truncate = True
                return False
            else:
                self._error(500, f"unknown injected fault '{kind}'")
            return True

        # -- routing ------------------------------------------------------

        def do_GET(self):  # noqa: N802
            self._dispatch("GET")

        def do_POST(self):  # noqa: N802
            self._dispatch("POST")

        def do_PATCH(self):  # noqa: N802
            self._dispatch("PATCH")

        def do_DELETE(self):  # noqa: N802
            self._dispatch("DELETE")

        def _dispatch(self, method: str):
            self._truncate = False
            split = urlsplit(self.path)
            path = unquote(split.path)
            query = parse_qs(split.query, keep_blank_values=True)
            entry = {
                "at": time.time(),
                "method": method,
                "path": path,
                "query": {k: v for k, v in query.items()},
                "headers": {
                    name: ("[redacted]" if name.lower() in ("authorization", "idempotency-key") else value)
                    for name, value in self.headers.items()
                },
            }
            try:
                self._route(method, path, query)
            except ValueError as error:
                self._error(400, str(error))
            except KeyError as error:
                self._error(404, str(error))
            finally:
                state.log(entry)

        def _route(self, method: str, path: str, query: dict):
            if path.startswith("/__admin"):
                return self._admin(method, path)
            if path in (
                "/oauth2/token",
                "/v1/oauth2/token",
                "/oauth2/v4/token",
                "/token",
            ) and method == "POST":
                return self._token()
            segments = [segment for segment in path.split("/") if segment]
            if len(segments) < 2 or segments[0] != "v1beta":
                raise KeyError(f"no route for {method} {path}")
            if segments[1] != "accounts":
                raise KeyError(f"no route for {method} {path}")
            if len(segments) == 2:
                if method == "GET":
                    if not self._authorize(None):
                        return
                    return self._json(
                        200,
                        {
                            "accounts": [
                                {
                                    "name": f"accounts/{state.account_id}",
                                    "publisherId": state.account_id,
                                    "currencyCode": "USD",
                                }
                            ]
                        },
                    )
                raise KeyError(f"no route for {method} {path}")
            account_id = segments[2]
            if method == "GET" and len(segments) == 3:
                if not self._authorize(None):
                    return
                if self._fault("accounts.get"):
                    return
                account = state.require_account(account_id)
                return self._json(200, account["account"])
            if len(segments) == 3:
                raise KeyError(f"no route for {method} {path}")

            resource = segments[3]
            rest = segments[4:]
            if not self._authorize(account_id):
                return
            account = state.require_account(account_id)

            if resource == "apps":
                return self._apps(method, account, rest, query)
            if resource == "adUnits":
                return self._ad_units(method, account, rest, query)
            if resource == "mediationGroups":
                return self._mediation_groups(method, account, rest, query)
            if resource == "mediationAbExperiments":
                return self._experiments(method, account, None, rest, query)
            if resource == "mediationReport:generate":
                if method != "POST":
                    raise KeyError(f"no route for {method} {path}")
                if self._fault("report"):
                    return
                return self._report(account)
            raise KeyError(f"no route for {method} {path}")

        # -- OAuth --------------------------------------------------------

        def _token(self):
            if self._fault("token"):
                return
            raw = self._read_body().decode("utf-8", "replace")
            form = parse_qs(raw, keep_blank_values=True)
            grant = (form.get("grant_type") or [""])[0]
            client_id = (form.get("client_id") or [""])[0]
            client_secret = (form.get("client_secret") or [""])[0]
            if grant == "refresh_token":
                refresh = (form.get("refresh_token") or [""])[0]
                with state.lock:
                    record = state.refresh_tokens.get(refresh)
                if record is None:
                    return self._error(400, "invalid_grant: unknown refresh token")
                if record.get("client_id") and client_id and record["client_id"] != client_id:
                    return self._error(400, "invalid_grant: client_id mismatch")
                if record.get("client_secret") and client_secret and record["client_secret"] != client_secret:
                    return self._error(401, "invalid_client")
                account = record["account"]
                ttl = int((form.get("expires_in") or ["3600"])[0] or 3600)
                token = state._mint_token(account, ttl)
            elif grant == "client_credentials":
                with state.lock:
                    secret = state.clients.get(client_id)
                if secret is None or (client_secret and secret != client_secret):
                    return self._error(401, "invalid_client")
                account = (form.get("account") or [state.account_id])[0]
                ttl = int((form.get("expires_in") or ["3600"])[0] or 3600)
                token = state._mint_token(account, ttl)
            else:
                return self._error(400, "unsupported_grant_type")
            state.save()
            return self._json(
                200,
                {
                    "access_token": token,
                    "expires_in": int((form.get("expires_in") or ["3600"])[0] or 3600),
                    "token_type": TOKEN_TYPE,
                    "scope": "https://www.googleapis.com/auth/admob.readonly",
                },
            )

        # -- resource reads ----------------------------------------------

        def _paginate(self, items, query):
            try:
                page_size = int((query.get("pageSize") or [str(DEFAULT_PAGE_SIZE)])[0])
            except ValueError:
                raise ValueError("pageSize must be an integer")
            page_size = max(1, min(MAX_PAGE_SIZE, page_size))
            offset = 0
            token = (query.get("pageToken") or [""])[0]
            if token:
                try:
                    offset = int(base64.urlsafe_b64decode(token + "===").decode())
                except Exception as error:  # noqa: BLE001
                    raise ValueError("pageToken is invalid") from error
                if offset < 0 or offset > len(items):
                    raise ValueError("pageToken is out of range")
            if getattr(self, "_truncate", False):
                page_size = 1
            page = items[offset : offset + page_size]
            next_token = ""
            if offset + page_size < len(items):
                next_token = base64.urlsafe_b64encode(
                    str(offset + page_size).encode()
                ).decode().rstrip("=")
            return page, next_token

        def _apps(self, method, account, rest, query):
            if method != "GET":
                raise KeyError("apps is read-only")
            if self._fault("apps.list"):
                return
            apps = [account["apps"][key] for key in sorted(account["apps"])]
            if rest:
                app = account["apps"].get(rest[0])
                if app is None:
                    raise KeyError(f"app '{rest[0]}' not found")
                return self._json(200, app)
            page, token = self._paginate(apps, query)
            body = {"apps": page}
            if token:
                body["nextPageToken"] = token
            return self._json(200, body)

        def _ad_units(self, method, account, rest, query):
            if method != "GET":
                raise KeyError("adUnits is read-only")
            if self._fault("adunits.list"):
                return
            units = [account["adUnits"][key] for key in sorted(account["adUnits"])]
            app_filter = (query.get("appId") or [""])[0]
            if app_filter:
                units = [unit for unit in units if unit.get("appId") == app_filter]
            if rest:
                unit = account["adUnits"].get(rest[0])
                if unit is None:
                    raise KeyError(f"ad unit '{rest[0]}' not found")
                return self._json(200, unit)
            page, token = self._paginate(units, query)
            body = {"adUnits": page}
            if token:
                body["nextPageToken"] = token
            return self._json(200, body)

        def _mediation_groups(self, method, account, rest, query):
            groups = account["groups"]
            if not rest:
                if method != "GET":
                    raise KeyError("mediationGroups collection is read-only")
                if self._fault("groups.list"):
                    return
                ordered = [canonical_to_legacy(groups[key]) for key in sorted(groups)]
                page, token = self._paginate(ordered, query)
                body = {"mediationGroups": page}
                if token:
                    body["nextPageToken"] = token
                return self._json(200, body)

            group_id = rest[0]
            tail = rest[1:]
            if not tail:
                if method == "GET":
                    if self._fault("groups.get"):
                        return
                    group = groups.get(group_id)
                    if group is None:
                        raise KeyError(f"mediation group '{group_id}' not found")
                    return self._json(200, canonical_to_legacy(group))
                if method == "PATCH":
                    return self._patch_group(account, group_id)
                raise KeyError(f"no route for {method} mediationGroups")

            if tail[0] == "mediationAbExperiments":
                return self._experiments(method, account, group_id, tail[1:], query)
            raise KeyError(f"no route for mediationGroups/{group_id}/{'/'.join(tail)}")

        def _patch_group(self, account, group_id):
            if self._fault("groups.patch"):
                return
            body = self._json_body() or {}
            if not isinstance(body, dict):
                raise ValueError("PATCH body must be a JSON object")
            key = self.headers.get("Idempotency-Key")
            payload_hash = _idempotency_hash(body)
            if key:
                with state.lock:
                    seen = state.idempotency.get((account["account"]["publisherId"], key))
                if seen:
                    if seen["body_hash"] != payload_hash:
                        return self._error(409, "Idempotency-Key was reused with a different body")
                    return self._json(seen["status"], seen["body"])
            group = account["groups"].get(group_id)
            if group is None:
                raise KeyError(f"mediation group '{group_id}' not found")
            if "adUnitMappings" in body:
                apply_legacy_mappings(group, body["adUnitMappings"], account["account"]["publisherId"])
            elif "mediationGroupLines" in body:
                apply_canonical_lines(group, body["mediationGroupLines"])
            else:
                raise ValueError("PATCH body must carry adUnitMappings or mediationGroupLines")
            if isinstance(body.get("displayName"), str) and body["displayName"].strip():
                group["displayName"] = body["displayName"].strip()[:120]
            if isinstance(body.get("targeting"), dict):
                group["targeting"].update(body["targeting"])
                if isinstance(body["targeting"].get("format"), str):
                    group["targeting"]["format"] = body["targeting"]["format"].upper()
            if isinstance(body.get("state"), str) and body["state"] in ("ENABLED", "DISABLED"):
                group["state"] = body["state"]
            response = canonical_to_legacy(group)
            if key:
                with state.lock:
                    state.idempotency[(account["account"]["publisherId"], key)] = {
                        "body_hash": payload_hash,
                        "status": 200,
                        "body": response,
                    }
            state.save()
            return self._json(200, response)

        # -- experiments --------------------------------------------------

        def _experiments(self, method, account, group_id, rest, query):
            experiments = account["experiments"]
            if not rest:
                if method != "POST":
                    raise KeyError("mediationAbExperiments collection is write-only")
                if self._fault("experiment.create"):
                    return
                return self._create_experiment(account, group_id)
            experiment_id = rest[0]
            stop = experiment_id.endswith(":stop")
            if stop:
                experiment_id = experiment_id[: -len(":stop")]
            if not stop or method != "POST":
                raise KeyError(f"no route for {method} mediationAbExperiments")
            if self._fault("experiment.stop"):
                return
            experiment = experiments.get(experiment_id)
            if experiment is None:
                raise KeyError(f"experiment '{experiment_id}' not found")
            if group_id is not None and experiment.get("mediationGroupId") != group_id:
                raise KeyError("experiment does not belong to that mediation group")
            experiment["state"] = "ENDED"
            experiment["endTime"] = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
            experiment["variantLeader"] = "TREATMENT"
            experiment["controlMediationLines"] = experiment.pop("controlMediationLines", [])
            group = account["groups"].get(experiment.get("mediationGroupId"))
            if group is not None:
                group["mediationAbExperimentState"] = "NOT_RUNNING"
            state.save()
            return self._json(200, experiment)

        def _create_experiment(self, account, group_id):
            body = self._json_body() or {}
            if not isinstance(body, dict):
                raise ValueError("experiment body must be a JSON object")
            group_id = group_id or body.get("mediationGroupId")
            if not isinstance(group_id, str) or not group_id.strip():
                raise ValueError("experiment requires mediationGroupId")
            group = account["groups"].get(group_id)
            if group is None:
                raise KeyError(f"mediation group '{group_id}' not found")
            display_name = body.get("displayName")
            if not isinstance(display_name, str) or not display_name.strip():
                raise ValueError("experiment requires displayName")
            traffic = body.get("treatmentTrafficPercentage", body.get("testShare", "50"))
            try:
                traffic = int(str(traffic))
            except ValueError as error:
                raise ValueError("treatmentTrafficPercentage must be an integer") from error
            if not 1 <= traffic <= 99:
                raise ValueError("treatmentTrafficPercentage must be in [1,99]")
            account["experimentSeq"] += 1
            experiment_id = f"exp-{account['experimentSeq']}"
            treatment = body.get("treatmentMediationLines")
            if not isinstance(treatment, list) or not treatment:
                # The connector does not send the proposed change. Preserve
                # that fact instead of inventing treatment lines.
                treatment = []
            experiment = {
                "name": f"accounts/{account['account']['publisherId']}/mediationGroups/{group_id}/mediationAbExperiment/{experiment_id}",
                "displayName": display_name.strip(),
                "experimentId": experiment_id,
                "treatmentTrafficPercentage": str(traffic),
                "treatmentMediationLines": treatment,
                "controlMediationLines": [
                    {"mediationGroupLine": line}
                    for line in group.get("mediationGroupLines", {}).values()
                    if line.get("state") != "REMOVED"
                ],
                "state": "RUNNING",
                "startTime": dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z"),
                "mediationGroupId": group_id,
                # Canonical v1beta enum. Before the connector patch it only
                # knew "UNKNOWN"/"CONTROL"/"VARIANT"/"VARIANT_LEADER_UNSPECIFIED".
                "variantLeader": "TOO_EARLY_TO_CALL",
            }
            account["experiments"][experiment_id] = experiment
            group["mediationAbExperimentState"] = "RUNNING"
            state.save()
            return self._json(200, experiment)

        # -- reports ------------------------------------------------------

        def _report(self, account):
            body = self._json_body() or {}
            if not isinstance(body, dict):
                raise ValueError("report body must be a JSON object")
            start = parse_date(body.get("startDate"), "startDate")
            end = parse_date(body.get("endDate"), "endDate")
            if end < start:
                raise ValueError("endDate is before startDate")
            dimensions = body.get("dimensions") or []
            metrics = body.get("metrics") or ["ESTIMATED_EARNINGS"]
            if not isinstance(dimensions, list) or not all(isinstance(d, str) for d in dimensions):
                raise ValueError("dimensions must be an array of strings")
            if not isinstance(metrics, list) or not all(isinstance(m, str) for m in metrics):
                raise ValueError("metrics must be an array of strings")
            known_dimensions = {
                "DATE",
                "COUNTRY",
                "AD_UNIT_ID",
                "APP_ID",
                "MEDIATION_GROUP_ID",
                "NETWORK",
                "AD_FORMAT",
                "PLATFORM",
            }
            unknown = [d for d in dimensions if d not in known_dimensions]
            if unknown:
                raise ValueError(f"unsupported dimensions: {unknown}")
            known_metrics = {"ESTIMATED_EARNINGS", "IMPRESSIONS"}
            unknown_metrics = [m for m in metrics if m not in known_metrics]
            if unknown_metrics:
                raise ValueError(f"unsupported metrics: {unknown_metrics}")

            row_key = {
                "DATE": "day",
                "COUNTRY": "country",
                "AD_UNIT_ID": "adUnitId",
                "APP_ID": "appId",
                "MEDIATION_GROUP_ID": "mediationGroupId",
                "NETWORK": "network",
                "AD_FORMAT": "adFormat",
                "PLATFORM": "platform",
            }
            aggregated: dict[tuple, dict] = {}
            for row in account["reportRows"]:
                try:
                    day = dt.date.fromisoformat(row["day"])
                except ValueError:
                    continue
                if not (start <= day <= end):
                    continue
                key = tuple(str(row.get(row_key[d], "")) for d in dimensions)
                bucket = aggregated.setdefault(
                    key,
                    {"revenueMicros": 0, "impressions": 0},
                )
                bucket["revenueMicros"] += int(row.get("revenueMicros", 0))
                bucket["impressions"] += int(row.get("impressions", 0))

            rows = []
            for key in sorted(aggregated):
                bucket = aggregated[key]
                dimension_values = {
                    dimension: {"value": value}
                    for dimension, value in zip(dimensions, key)
                }
                metric_values = {}
                if "ESTIMATED_EARNINGS" in metrics:
                    metric_values["ESTIMATED_EARNINGS"] = {
                        "microsValue": str(bucket["revenueMicros"])
                    }
                if "IMPRESSIONS" in metrics:
                    metric_values["IMPRESSIONS"] = {
                        "integerValue": str(bucket["impressions"])
                    }
                rows.append(
                    {
                        "dimensionValues": dimension_values,
                        "metricValues": metric_values,
                    }
                )
            if not dimensions and not rows:
                rows = [
                    {
                        "dimensionValues": {},
                        "metricValues": {
                            "ESTIMATED_EARNINGS": {"microsValue": "0"},
                            "IMPRESSIONS": {"integerValue": "0"},
                        },
                    }
                ]

            page_size = body.get("pageSize", DEFAULT_PAGE_SIZE)
            try:
                page_size = max(1, min(MAX_PAGE_SIZE, int(page_size)))
            except (TypeError, ValueError) as error:
                raise ValueError("pageSize must be an integer") from error
            if getattr(self, "_truncate", False):
                page_size = max(1, min(page_size, 1))
            page = rows[:page_size]
            response = {
                "header": {
                    "currencyCode": body.get("currencyCode", "USD"),
                    "dateRange": {
                        "startDate": body.get("startDate"),
                        "endDate": body.get("endDate"),
                    },
                },
                "rows": page,
            }
            if len(rows) > page_size:
                response["nextPageToken"] = base64.urlsafe_b64encode(
                    str(page_size).encode()
                ).decode().rstrip("=")
            return self._json(200, response)

        # -- admin --------------------------------------------------------

        def _admin(self, method, path):
            name = path[len("/__admin") :].strip("/")
            if name == "state" and method == "GET":
                with state.lock:
                    accounts = {}
                    for account_id, account in state.accounts.items():
                        accounts[account_id] = {
                            "apps": list(account["apps"]),
                            "adUnits": list(account["adUnits"]),
                            "groups": {
                                group_id: {
                                    "displayName": group.get("displayName"),
                                    "targeting": group.get("targeting"),
                                    "state": group.get("state"),
                                    "mediationAbExperimentState": group.get(
                                        "mediationAbExperimentState"
                                    ),
                                    "lines": group.get("mediationGroupLines"),
                                    "legacy": canonical_to_legacy(group),
                                }
                                for group_id, group in account["groups"].items()
                            },
                            "experiments": account["experiments"],
                            "reportRows": len(account["reportRows"]),
                        }
                    tokens = {
                        token: {
                            "account": record["account"],
                            "expires_in": max(0, record["expires_at"] - time.time()),
                        }
                        for token, record in state.tokens.items()
                    }
                    return self._json(
                        200,
                        {
                            "account": state.account_id,
                            "accounts": accounts,
                            "tokens": tokens,
                            "faults": {key: list(queue) for key, queue in state.faults.items()},
                            "requests": len(state.requests),
                        },
                    )
            if name == "requests" and method == "GET":
                with state.lock:
                    return self._json(200, {"requests": list(state.requests)})
            if name == "faults" and method == "POST":
                body = self._json_body() or {}
                target = body.get("target", "any")
                kind = body.get("kind")
                if not kind:
                    raise ValueError("fault requires kind")
                count = int(body.get("count", 1))
                queue = state.faults.setdefault(target, deque())
                for _ in range(max(1, count)):
                    queue.append(body)
                return self._json(200, {"queued": count, "target": target, "kind": kind})
            if name == "expire-tokens" and method == "POST":
                body = self._json_body() or {}
                with state.lock:
                    if body.get("token"):
                        record = state.tokens.get(body["token"])
                        if record is not None:
                            record["expires_at"] = 0
                    else:
                        for record in state.tokens.values():
                            record["expires_at"] = 0
                return self._json(200, {"expired": True})
            if name == "token" and method == "POST":
                body = self._json_body() or {}
                account = body.get("account", state.account_id)
                ttl = int(body.get("ttl_seconds", 3600))
                token = state._mint_token(account, ttl)
                state.save()
                return self._json(
                    200,
                    {"access_token": token, "token_type": TOKEN_TYPE, "expires_in": ttl},
                )
            if name == "seed-report" and method == "POST":
                body = self._json_body() or {}
                rows = body.get("rows")
                if not isinstance(rows, list):
                    raise ValueError("seed-report requires a rows array")
                account_id = body.get("account", state.account_id)
                account = state.require_account(account_id)
                for row in rows:
                    if not isinstance(row, dict):
                        raise ValueError("each report row must be an object")
                    account["reportRows"].append(row)
                state.save()
                return self._json(200, {"added": len(rows)})
            if name == "reset" and method == "POST":
                body = self._json_body() or {}
                account_id = body.get("account", state.account_id)
                days = int(body.get("seed_days", 45))
                with state.lock:
                    state._seed(account_id, days)
                    state.accounts[account_id]["experiments"] = {}
                state.save()
                return self._json(200, {"reset": True})
            raise KeyError(f"no admin route for {method} {path}")

    return Handler


# ---------------------------------------------------------------------------
# entrypoint


def main(argv=None):
    parser = argparse.ArgumentParser(description="Stateful AdMob provider simulator")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0, help="0 picks a free port")
    parser.add_argument("--account", default="pub-1234567890123456")
    parser.add_argument("--access-token", default="sim-access-token")
    parser.add_argument("--refresh-token", default="sim-refresh-token")
    parser.add_argument("--client-id", default="sim-client-id")
    parser.add_argument("--client-secret", default="sim-client-secret")
    parser.add_argument("--token-ttl", type=int, default=3600)
    parser.add_argument("--seed-days", type=int, default=45)
    parser.add_argument("--state-file", default=None)
    args = parser.parse_args(argv)

    state = SimState(
        account_id=args.account,
        access_token=args.access_token,
        refresh_token=args.refresh_token,
        client_id=args.client_id,
        client_secret=args.client_secret,
        token_ttl=args.token_ttl,
        seed_days=args.seed_days,
        state_file=args.state_file,
    )
    server = ThreadingHTTPServer((args.host, args.port), make_handler(state))
    server.daemon_threads = True
    bound_port = server.server_address[1]
    banner = {
        "event": "ready",
        "host": args.host,
        "port": bound_port,
        "base_url": f"http://{args.host}:{bound_port}",
        "account": f"accounts/{args.account}",
        "access_token": args.access_token,
        "refresh_token": args.refresh_token,
        "client_id": args.client_id,
        "client_secret": args.client_secret,
        "token_ttl_seconds": args.token_ttl,
    }
    print(json.dumps(banner, separators=(",", ":")), flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
