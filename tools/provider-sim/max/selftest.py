#!/usr/bin/env python3
"""Contract self-test for the MAX simulator.

Starts the simulator on a free loopback port with a temporary state file and
drives it over real HTTP. It covers the documented management/report shapes,
state transitions, pagination and every fault kind in once and persistent
mode. Run with:

    python3 tools/provider-sim/max/selftest.py
"""

from __future__ import annotations

import datetime as dt
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
API_KEY = "selftest-management-key"
REPORT_KEY = "selftest-report-key"


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class Sim:
    def __init__(self) -> None:
        self.port = free_port()
        self.root = tempfile.TemporaryDirectory(prefix="max-sim-selftest-")
        self.state = Path(self.root.name) / "state.json"
        env = os.environ.copy()
        env.update(
            {
                "MAX_SIM_API_KEY": API_KEY,
                "MAX_SIM_REPORT_KEY": REPORT_KEY,
                "MAX_SIM_STATE": str(self.state),
            }
        )
        self.proc = subprocess.Popen(
            [
                sys.executable,
                str(HERE / "server.py"),
                "--port",
                str(self.port),
                "--state",
                str(self.state),
            ],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        deadline = time.time() + 10
        while time.time() < deadline:
            try:
                self.get("/__sim/health")
                return
            except (urllib.error.URLError, ConnectionError):
                time.sleep(0.05)
        raise RuntimeError("simulator did not start")

    def url(self, path: str, query: dict | None = None) -> str:
        if query:
            path = f"{path}?{urllib.parse.urlencode(query)}"
        return f"http://127.0.0.1:{self.port}{path}"

    def request(
        self,
        method: str,
        path: str,
        query: dict | None = None,
        body: dict | None = None,
        api_key: str | None = API_KEY,
        report_key: str | None = None,
        headers: dict | None = None,
    ) -> tuple[int, bytes, dict]:
        if report_key is not None:
            query = dict(query or {})
            query["api_key"] = report_key
        data = None
        merged = dict(headers or {})
        if api_key is not None:
            merged["Api-Key"] = api_key
        if body is not None:
            data = json.dumps(body).encode()
            merged["Content-Type"] = "application/json"
        request = urllib.request.Request(self.url(path, query), data=data, method=method)
        for name, value in merged.items():
            request.add_header(name, value)
        try:
            with urllib.request.urlopen(request, timeout=10) as response:
                return response.status, response.read(), dict(response.headers)
        except urllib.error.HTTPError as error:
            return error.code, error.read(), dict(error.headers)

    def get(self, path: str, **kwargs):
        return self.request("GET", path, **kwargs)

    def post(self, path: str, **kwargs):
        return self.request("POST", path, **kwargs)

    def json(self, *args, **kwargs):
        status, raw, headers = self.request(*args, **kwargs)
        return status, json.loads(raw), headers

    def close(self) -> None:
        self.proc.terminate()
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.root.cleanup()


class SimulatorContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.sim = Sim()

    @classmethod
    def tearDownClass(cls) -> None:
        cls.sim.close()

    def setUp(self) -> None:
        self.sim.post("/__sim/reset")

    def test_management_list_is_a_top_level_array_with_optional_floors(self) -> None:
        status, body, _ = self.sim.json("GET", "/mediation/v1/ad_units")
        self.assertEqual(status, 200)
        self.assertIsInstance(body, list)
        self.assertTrue(all("bid_floors" not in unit for unit in body))
        status, body, _ = self.sim.json(
            "GET", "/mediation/v1/ad_units", query={"fields": "bid_floors"}
        )
        self.assertEqual(status, 200)
        self.assertTrue(all("bid_floors" in unit for unit in body))

    def test_management_list_paginates(self) -> None:
        first = self.sim.json(
            "GET", "/mediation/v1/ad_units", query={"limit": "1", "offset": "0"}
        )[1]
        second = self.sim.json(
            "GET", "/mediation/v1/ad_units", query={"limit": "1", "offset": "1"}
        )[1]
        beyond = self.sim.json(
            "GET", "/mediation/v1/ad_units", query={"limit": "1", "offset": "9"}
        )[1]
        self.assertEqual(len(first), 1)
        self.assertEqual(len(second), 1)
        self.assertNotEqual(first[0]["id"], second[0]["id"])
        self.assertEqual(beyond, [])

    def test_management_auth_header(self) -> None:
        status, _, _ = self.sim.request("GET", "/mediation/v1/ad_units", api_key=None)
        self.assertEqual(status, 401)
        status, _, _ = self.sim.request(
            "GET", "/mediation/v1/ad_units", api_key="wrong"
        )
        self.assertEqual(status, 403)

    def test_encoded_ids_survive_routing(self) -> None:
        self.sim.post(
            "/__sim/reset",
            body={
                "ad_units": [
                    {
                        "id": "u/1 ?",
                        "name": "Encoded",
                        "platform": "ios",
                        "package_name": "com.x",
                        "ad_format": "BANNER",
                        "disabled": False,
                        "bid_floors": [],
                    }
                ]
            },
        )
        status, body, _ = self.sim.json(
            "GET", "/mediation/v1/ad_unit/u%2F1%20%3F?fields=bid_floors"
        )
        self.assertEqual(status, 200)
        self.assertEqual(body["id"], "u/1 ?")

    def test_floor_update_is_visible_next_read_and_in_the_state_file(self) -> None:
        status, unit, _ = self.sim.json(
            "GET", "/mediation/v1/ad_unit/rw_demo_01", query={"fields": "bid_floors"}
        )
        self.assertEqual(status, 200)
        floors = unit["bid_floors"]
        for group in floors:
            if group["country_group_name"] == "ee":
                group["cpm"] = "4.25"
        status, updated, _ = self.sim.json(
            "POST", "/mediation/v1/ad_unit/rw_demo_01", body=unit
        )
        self.assertEqual(status, 200)
        self.assertIn(
            {"country_group_name": "ee", "cpm": "4.25",
             "countries": {"type": "INCLUDE", "values": ["ee"]}},
            updated["bid_floors"],
        )
        again = self.sim.json(
            "GET", "/mediation/v1/ad_unit/rw_demo_01", query={"fields": "bid_floors"}
        )[1]
        self.assertIn("4.25", [group["cpm"] for group in again["bid_floors"]])
        persisted = json.loads(Path(self.sim.state).read_text())
        saved = next(unit for unit in persisted["ad_units"] if unit["id"] == "rw_demo_01")
        self.assertIn("4.25", [group["cpm"] for group in saved["bid_floors"]])

    def test_experiment_create_read_promote_applies_the_test_floors(self) -> None:
        status, created, _ = self.sim.json(
            "POST",
            "/mediation/v1/ad_unit_experiment/rw_demo_01",
            body={
                "experiment_name": "raise-ee",
                "test_group_allocation": 25,
                "bid_floors": [
                    {
                        "country_group_name": "ee",
                        "cpm": "4.25",
                        "countries": {"type": "INCLUDE", "values": ["ee"]},
                    }
                ],
            },
        )
        self.assertEqual(status, 200)
        self.assertEqual(created["id"], "rw_demo_01")
        status, live, _ = self.sim.json(
            "GET", "/mediation/v1/ad_units", query={"fields": "bid_floors"}
        )
        self.assertTrue(
            next(unit for unit in live if unit["id"] == "rw_demo_01")[
                "has_active_experiment"
            ]
        )
        status, running, _ = self.sim.json(
            "GET",
            "/mediation/v1/ad_unit_experiment/rw_demo_01",
            query={"fields": "bid_floors"},
        )
        self.assertEqual(status, 200)
        self.assertFalse(running["promote"])
        self.assertFalse(running["deprecate"])
        status, promoted, _ = self.sim.json(
            "POST",
            "/mediation/v1/ad_unit_experiment/rw_demo_01",
            body={
                "id": "rw_demo_01",
                "experiment_name": "raise-ee",
                "promote": True,
                "deprecate": False,
            },
        )
        self.assertEqual(status, 200)
        self.assertIn("promoted", promoted["message"].lower())
        live = self.sim.json(
            "GET", "/mediation/v1/ad_unit/rw_demo_01", query={"fields": "bid_floors"}
        )[1]
        self.assertIn("4.25", [group["cpm"] for group in live["bid_floors"]])

    def test_report_shape_columns_filters_and_pagination(self) -> None:
        end = dt.date.today() - dt.timedelta(days=1)
        start = end - dt.timedelta(days=6)
        columns = ["day", "country", "max_ad_unit_id", "estimated_revenue", "impressions"]
        status, first, _ = self.sim.json(
            "GET",
            "/maxReport",
            query={
                "columns": ",".join(columns),
                "start": start.isoformat(),
                "end": end.isoformat(),
                "format": "json",
                "limit": "2",
                "offset": "0",
            },
            report_key=REPORT_KEY,
        )
        self.assertEqual(status, 200)
        self.assertEqual(first["code"], 200)
        self.assertEqual(len(first["results"]), 2)
        self.assertEqual(sorted(first["results"][0]), sorted(columns))
        for row in first["results"]:
            self.assertIsInstance(row["estimated_revenue"], str)
            self.assertIsInstance(row["impressions"], str)
        self.assertEqual(first["count"], 2, "documented count is the page size")
        second = self.sim.json(
            "GET",
            "/maxReport",
            query={
                "columns": ",".join(columns),
                "start": start.isoformat(),
                "end": end.isoformat(),
                "limit": "2",
                "offset": "2",
            },
            report_key=REPORT_KEY,
        )[1]
        self.assertNotEqual(first["results"], second["results"])
        filtered = self.sim.json(
            "GET",
            "/maxReport",
            query={
                "columns": "day,country,max_ad_unit_id,estimated_revenue,impressions",
                "start": start.isoformat(),
                "end": end.isoformat(),
                "limit": "100",
                "offset": "0",
                "filter_max_ad_unit_id": "rw_demo_01",
                "filter_country": "ee",
            },
            report_key=REPORT_KEY,
        )[1]
        self.assertTrue(filtered["results"])
        self.assertTrue(
            all(row["max_ad_unit_id"] == "rw_demo_01" for row in filtered["results"])
        )
        self.assertTrue(all(row["country"] == "ee" for row in filtered["results"]))

    def test_report_rejects_a_window_over_45_days(self) -> None:
        status, body, _ = self.sim.json(
            "GET",
            "/maxReport",
            query={
                "columns": "day,estimated_revenue,impressions",
                "start": "2026-01-01",
                "end": "2026-03-01",
            },
            report_key=REPORT_KEY,
        )
        self.assertEqual(status, 400)
        self.assertEqual(body["code"], 400)
        self.assertIn("45", body["error"])

    def test_report_auth_query(self) -> None:
        status, _, _ = self.sim.request(
            "GET",
            "/maxReport",
            query={
                "columns": "day",
                "start": "2026-10-01",
                "end": "2026-10-02",
            },
            api_key=None,
        )
        self.assertEqual(status, 401)
        status, _, _ = self.sim.request(
            "GET",
            "/maxReport",
            query={
                "columns": "day",
                "start": "2026-10-01",
                "end": "2026-10-02",
                "api_key": "wrong",
            },
            api_key=None,
        )
        self.assertEqual(status, 403)

    def _add(self, **spec):
        spec.setdefault("target", "management")
        status, body, _ = self.sim.json("POST", "/__sim/fault", body=spec)
        self.assertEqual(status, 200)
        return body

    def test_status_faults_fire_once_or_persistently(self) -> None:
        for kind, expected in (("auth", 401), ("rate_limit", 429), ("server_error", 500)):
            self.sim.post("/__sim/reset")
            self._add(fault=kind, target="management", mode="once")
            status, _, headers = self.sim.request("GET", "/mediation/v1/ad_units")
            self.assertEqual(status, expected, kind)
            if kind == "rate_limit":
                self.assertIn("Retry-After", headers)
            status, _, _ = self.sim.request("GET", "/mediation/v1/ad_units")
            self.assertEqual(status, 200, f"{kind} should fire only once")

        self.sim.post("/__sim/reset")
        self._add(fault="server_error", target="reporting", mode="persistent")
        for _ in range(2):
            status, _, _ = self.sim.request(
                "GET",
                "/maxReport",
                query={
                    "columns": "day,estimated_revenue,impressions",
                    "start": (dt.date.today() - dt.timedelta(days=2)).isoformat(),
                    "end": (dt.date.today() - dt.timedelta(days=1)).isoformat(),
                },
                report_key=REPORT_KEY,
            )
            self.assertEqual(status, 500)

    def test_data_faults(self) -> None:
        start = (dt.date.today() - dt.timedelta(days=2)).isoformat()
        end = (dt.date.today() - dt.timedelta(days=1)).isoformat()
        query = {
            "columns": "day,country,max_ad_unit_id,estimated_revenue,impressions",
            "start": start,
            "end": end,
            "limit": "100",
            "offset": "0",
        }

        self.sim.post("/__sim/reset")
        self._add(fault="empty", target="reporting", mode="once")
        body = self.sim.json(
            "GET", "/maxReport", query=query, report_key=REPORT_KEY
        )[1]
        self.assertEqual(body["results"], [])
        self.assertTrue(
            self.sim.json("GET", "/maxReport", query=query, report_key=REPORT_KEY)[1][
                "results"
            ]
        )

        self.sim.post("/__sim/reset")
        self._add(fault="malformed", target="reporting", mode="once")
        status, raw, _ = self.sim.request(
            "GET", "/maxReport", query=query, report_key=REPORT_KEY
        )
        self.assertEqual(status, 200)
        with self.assertRaises(json.JSONDecodeError):
            json.loads(raw)

        self.sim.post("/__sim/reset")
        self._add(fault="truncated", target="reporting", mode="once")
        first = self.sim.json(
            "GET", "/maxReport", query=query, report_key=REPORT_KEY
        )[1]
        self.assertLess(len(first["results"]), first["count"])
        second = self.sim.json(
            "GET",
            "/maxReport",
            query={**query, "offset": str(len(first["results"]))},
            report_key=REPORT_KEY,
        )[1]
        self.assertEqual(second["results"], [])
        self.assertEqual(second["count"], first["count"])

        self.sim.post("/__sim/reset")
        self._add(fault="slow", target="reporting", mode="once", delay_ms=120)
        began = time.time()
        status, _, _ = self.sim.request(
            "GET", "/maxReport", query=query, report_key=REPORT_KEY
        )
        self.assertEqual(status, 200)
        self.assertGreaterEqual(time.time() - began, 0.1)


if __name__ == "__main__":
    unittest.main(verbosity=2)
