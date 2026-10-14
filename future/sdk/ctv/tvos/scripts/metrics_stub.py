#!/usr/bin/env python3
"""Simple dev backend stub for tvOS metrics testing."""

import json
import os
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

STATE = {
    "metrics_enabled": os.environ.get("METRICS_ENABLED", "false").lower() in {"1", "true", "yes"},
    "metrics": [],
}


class Handler(BaseHTTPRequestHandler):
    server_version = "MetricsStub/1.0"

    def _set_headers(self, status=200, content_type="application/json"):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.end_headers()

    def do_GET(self):  # noqa: N802 (stdlib signature)
        parsed = urlparse(self.path)
        if parsed.path == "/api/v1/sdk/config":
            body = {
                "version": 1,
                "rolloutPercent": 100,
                "features": {
                    "killSwitch": False,
                    "disableShow": False,
                    "metricsEnabled": STATE["metrics_enabled"],
                },
                "placements": {},
            }
            payload = json.dumps(body).encode("utf-8")
            self._set_headers(200)
            self.wfile.write(payload)
            return
        if parsed.path == "/admin/metrics" or parsed.path == "/admin/metrics_log":
            payload = json.dumps(STATE["metrics"]).encode("utf-8")
            self._set_headers(200)
            self.wfile.write(payload)
            return
        self._set_headers(404)
        self.wfile.write(b"{}")

    def do_POST(self):  # noqa: N802 (stdlib signature)
        parsed = urlparse(self.path)
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length else b""
        if parsed.path == "/api/v1/rtb/bid":
            # Always respond no-fill to keep flows simple.
            self._set_headers(204)
            return
        if parsed.path == "/api/v1/sdk/metrics":
            text = body.decode("utf-8") if body else ""
            STATE["metrics"].append(text)
            print(f"[metrics] {text}")
            self._set_headers(200)
            self.wfile.write(b"{}")
            return
        if parsed.path == "/admin/toggle_metrics":
            params = parse_qs(parsed.query)
            value = params.get("enabled", ["false"])[0].lower() in {"1", "true", "yes"}
            STATE["metrics_enabled"] = value
            print(f"[config] metricsEnabled set to {value}")
            self._set_headers(200)
            self.wfile.write(json.dumps({"metricsEnabled": value}).encode("utf-8"))
            return
        self._set_headers(404)
        self.wfile.write(b"{}")


def main():
    port = int(os.environ.get("METRICS_STUB_PORT", "8123"))
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"Metrics stub running on http://127.0.0.1:{port}")
    print("Use POST /admin/toggle_metrics?enabled=true|false to flip the feature flag")
    server.serve_forever()


if __name__ == "__main__":
    main()
