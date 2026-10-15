#!/bin/sh
# Caddy edge contract: adapt the production Caddyfile with fixed test domains
# and assert the routing table. Run from the repository root with `caddy` and
# `python3` on PATH. Exits nonzero on any violation.
#
# The checks pin the one-origin layout: the marketing host serves the static
# site and proxies only POST /free-audit, /audit-requested/* and /console* to
# apex-api; the API host fronts apex-api; the operator host fronts apex-control
# alone; the marketing host carries the JSON access log `apex-ingest` reads;
# and no route reaches anything but the two loopback services (ClickHouse is
# never exposed).
set -eu

CONFIG=${1:-infrastructure/caddy/Caddyfile}
ADAPTED=$(mktemp)
trap 'rm -f "$ADAPTED"' EXIT

APEX_ACCESS_LOG=/tmp/apex-access-contract.json \
APEX_DOMAIN=api.edge.test \
SITE_DOMAIN=www.edge.test \
CONTROL_DOMAIN=control.edge.test \
	caddy adapt --config "$CONFIG" --adapter caddyfile > "$ADAPTED"

python3 - "$ADAPTED" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    config = json.load(handle)

failures = []


def check(condition, message):
    if not condition:
        failures.append(message)


def servers():
    return config.get("apps", {}).get("http", {}).get("servers", {})


def host_routes(host):
    """The first matching host block's subroute list, in evaluation order."""
    for server in servers().values():
        for route in server.get("routes", []):
            hosts = [h for m in route.get("match", []) for h in m.get("host", [])]
            if host in hosts:
                for handler in route.get("handle", []):
                    if handler.get("handler") == "subroute":
                        return handler.get("routes", [])
                return []
    return None


def matcher_matches(matcher, method, path):
    if "host" in matcher or "method" in matcher or "path" in matcher:
        if "host" in matcher and not matcher["host"]:
            return False
        if "method" in matcher and method not in matcher["method"]:
            return False
        if "path" in matcher:
            if not any(path_matches(pattern, path) for pattern in matcher["path"]):
                return False
    return True


def path_matches(pattern, path):
    if pattern == "*":
        return True
    if pattern.endswith("*"):
        return path.startswith(pattern[:-1])
    return path == pattern


def route_matches(route, method, path):
    match = route.get("match")
    if not match:
        return True
    return any(matcher_matches(matcher, method, path) for matcher in match)


# Handlers that end request handling; everything before them (headers, encode,
# vars) composes into the response, so the first matching route that carries a
# terminal handler decides where the request goes.
TERMINAL_HANDLERS = {"reverse_proxy", "file_server", "respond", "static_response", "error"}


def handlers_for(routes, method, path):
    for route in routes:
        if not route_matches(route, method, path):
            continue
        handlers = route.get("handle", [])
        if any(handler.get("handler") in TERMINAL_HANDLERS for handler in handlers):
            return handlers
    return []


def proxies_in(handlers):
    dials = []
    for handler in handlers:
        if handler.get("handler") == "reverse_proxy":
            dials.extend(upstream.get("dial") for upstream in handler.get("upstreams", []))
    return dials


def has_file_server(handlers):
    return any(handler.get("handler") == "file_server" for handler in handlers)


def dials_in_routes(routes):
    dials = set()
    for route in routes:
        for handler in route.get("handle", []):
            if handler.get("handler") == "subroute":
                dials |= dials_in_routes(handler.get("routes", []))
            if handler.get("handler") == "reverse_proxy":
                dials.update(
                    upstream.get("dial") for upstream in handler.get("upstreams", [])
                )
    return dials


site = host_routes("www.edge.test")
api = host_routes("api.edge.test")
control = host_routes("control.edge.test")
check(site is not None, "marketing host block is missing")
check(api is not None, "API host block is missing")
check(control is not None, "operator host block is missing")

if site is not None:
    # POST /free-audit reaches apex-api; the GET marketing pages stay static.
    check(
        proxies_in(handlers_for(site, "POST", "/free-audit")) == ["127.0.0.1:8787"],
        "POST /free-audit must proxy to apex-api",
    )
    check(
        not proxies_in(handlers_for(site, "GET", "/free-audit")),
        "GET /free-audit must stay static",
    )
    check(
        not proxies_in(handlers_for(site, "GET", "/free-audit/")),
        "GET /free-audit/ must stay static",
    )
    check(
        has_file_server(handlers_for(site, "GET", "/free-audit/")),
        "GET /free-audit/ must be served by the static file server",
    )
    check(
        has_file_server(handlers_for(site, "GET", "/pricing")),
        "marketing pages must be served by the static file server",
    )
    # The confirmation pages and the console reach apex-api.
    check(
        proxies_in(handlers_for(site, "GET", "/audit-requested/?ref=req_0123456789"))
        == ["127.0.0.1:8787"],
        "GET /audit-requested/* must proxy to apex-api",
    )
    for path in ("/console", "/console/", "/console/audit", "/console/ledger"):
        check(
            proxies_in(handlers_for(site, "GET", path)) == ["127.0.0.1:8787"],
            f"GET {path} must proxy to apex-api",
        )
    check(
        proxies_in(handlers_for(site, "POST", "/console/ledger")) == ["127.0.0.1:8787"],
        "POST /console/ledger must proxy to apex-api",
    )
    # Static assets are untouched.
    check(
        has_file_server(handlers_for(site, "GET", "/fonts/manrope-latin.woff2")),
        "static asset paths must be served by the static file server",
    )

if api is not None:
    for method, path in (("GET", "/"), ("POST", "/v1/leads"), ("GET", "/console/audit")):
        check(
            proxies_in(handlers_for(api, method, path)) == ["127.0.0.1:8787"],
            f"{method} {path} on the API host must proxy to apex-api",
        )

if control is not None:
    for method, path in (("GET", "/"), ("POST", "/control/login"), ("GET", "/control/system")):
        handlers = handlers_for(control, method, path)
        check(
            proxies_in(handlers) == ["127.0.0.1:8790"],
            f"{method} {path} on the operator host must proxy to apex-control alone",
        )
        check(
            not has_file_server(handlers),
            "the operator host must not serve static files",
        )

# Host isolation: each host reaches exactly its own service.
if site is not None:
    check(
        dials_in_routes(site) == {"127.0.0.1:8787"},
        "the marketing host must only reach apex-api",
    )
if api is not None:
    check(
        dials_in_routes(api) == {"127.0.0.1:8787"},
        "the API host must only reach apex-api",
    )
if control is not None:
    check(
        dials_in_routes(control) == {"127.0.0.1:8790"},
        "the operator host must only reach apex-control",
    )

# No route anywhere reaches ClickHouse or any other service.
all_dials = set()

def walk(value):
    if isinstance(value, dict):
        if value.get("handler") == "reverse_proxy":
            for upstream in value.get("upstreams", []):
                dial = upstream.get("dial")
                if dial:
                    all_dials.add(dial)
        for child in value.values():
            walk(child)
    elif isinstance(value, list):
        for child in value:
            walk(child)

walk(config)
check(
    all_dials <= {"127.0.0.1:8787", "127.0.0.1:8790"},
    f"unexpected upstream(s); ClickHouse must stay unreachable: {sorted(all_dials)}",
)
text = json.dumps(config)
for port in ("8123", "9000"):
    check(port not in text, f"ClickHouse port {port} appears in the edge config")

# The marketing origin carries the JSON access log apex-ingest reads.
logs = config.get("logging", {}).get("logs", {})
json_file_logs = [
    log
    for log in logs.values()
    if log.get("encoder", {}).get("format") == "json"
    and log.get("writer", {}).get("output") == "file"
]
check(json_file_logs, "the marketing access log must be a JSON file log")
if json_file_logs:
    filename = json_file_logs[0]["writer"].get("filename", "")
    check(
        filename.endswith(".json"),
        f"the marketing access log file must be a JSON file: {filename}",
    )
attached = set()
for server in servers().values():
    for host in server.get("logs", {}).get("logger_names", {}):
        attached.add(host)
skipped = set()
for server in servers().values():
    skipped.update(server.get("logs", {}).get("skip_hosts", []))
check("www.edge.test" in attached, "the marketing host must write the access log")
check(
    "api.edge.test" not in attached and "control.edge.test" not in attached,
    "only the marketing host carries the access log",
)
check(
    {"api.edge.test", "control.edge.test"} <= skipped,
    "the API and operator hosts must be excluded from the marketing access log",
)

if failures:
    print("CADDY-EDGE-CONTRACT FAILED:")
    for failure in failures:
        print(f"  - {failure}")
    sys.exit(1)
print(
    "caddy-edge-contract PASS: 3 hosts, POST /free-audit + /audit-requested/* + "
    "/console* to apex-api, GET marketing pages static, operator host isolated to "
    "apex-control, JSON access log on the marketing origin, no ClickHouse route"
)
PY
