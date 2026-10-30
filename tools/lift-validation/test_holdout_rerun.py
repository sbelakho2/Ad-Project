"""The public holdout rerun kit reproduces the conservative fee."""
import importlib.util
import json
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "site" / "static" / "rerun-kit" / "holdout_rerun.py"
FIXTURE = ROOT / "tools" / "lift-validation" / "a3_fee_fixture.json"


def load_module():
    spec = importlib.util.spec_from_file_location("holdout_rerun", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run_script(module, args):
    import contextlib
    import io

    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        module.main(["holdout_rerun.py", *args])
    return json.loads(output.getvalue())


class HoldoutRerunTest(unittest.TestCase):
    def test_a_gain_uses_the_lower_bound_and_reports_coverage(self):
        # The planned-share bound carries the 30-player control arm's
        # counting noise, so the fixture shows a 50% lift: the same reading
        # production needs before a no-roster month can clear zero.
        rows = []
        for day in range(4, 32):  # the complete 28-day window ending 2026-10-31
            day_id = f"2026-10-{day:02d}"
            for index in range(270):
                rows.append({
                    "day": day_id, "userId": f"e{index}", "idfv": None,
                    "idfa": None, "test": False, "revenueMicros": 150_000,
                })
            for index in range(30):
                rows.append({
                    "day": day_id, "userId": f"c{index}", "idfv": None,
                    "idfa": None, "test": True, "revenueMicros": 100_000,
                })
        evidence = {
            "monthEnd": "2026-10-31", "exposedIsTest": False, "rows": rows,
            "pretest": {}, "unidentifiedMicros": 0, "unidentifiedShareBp": 0,
        }
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(evidence, handle)
            path = handle.name
        module = load_module()
        report = run_script(module, [path, "--treated-revenue", "900000000"])
        assert report["feeMicros"] > 0
        self.assertEqual(report["players"], {"exposed": 270, "control": 30})
        self.assertEqual(report["windowDays"], 28)
        self.assertTrue(report["windowComplete"])
        self.assertEqual(report["estimator"], "planned_share_total_v2")
        self.assertAlmostEqual(report["lift"], 0.5)
        self.assertAlmostEqual(report["billingLift"], report["lowerAlpha"])
        self.assertIsNone(report["lowerAlphaVerified"])
        self.assertGreater(report["feeMicros"], 0)
        self.assertEqual(report["coverage"]["unidentifiedShareBp"], 0)


class GoldenVectorTest(unittest.TestCase):
    """The bound-derived fee for the shared A3 fixture, to the micro.

    The fixture measures a lift strictly above its registered lowerAlpha, so
    a point-estimate fee (or the old dimensionless/absolute `min`) would
    produce a different number. `a3_fee_fixture.json` is the same golden
    vector the Rust composition test
    (rust/crates/apex-autopilot/tests/a3_composition.rs) asserts.
    """

    def build_record(self, fixture):
        rows = []
        for offset in range(fixture["windowDays"]):
            day = f"2026-10-{fixture['windowStartDay'] + offset:02d}"
            for index in range(fixture["exposedPlayers"]):
                rows.append({
                    "day": day, "userId": f"e{index}", "idfv": None, "idfa": None,
                    "test": fixture["exposedIsTest"],
                    "revenueMicros": fixture["exposedDailyMicrosBase"]
                    + index % fixture["exposedDailyMicrosMod"]
                    * fixture["exposedDailyMicrosStride"],
                })
            for index in range(fixture["controlPlayers"]):
                rows.append({
                    "day": day, "userId": f"c{index}", "idfv": None, "idfa": None,
                    "test": not fixture["exposedIsTest"],
                    "revenueMicros": fixture["controlDailyMicrosBase"]
                    + index % fixture["controlDailyMicrosMod"]
                    * fixture["controlDailyMicrosStride"],
                })
        return {
            "type": "monthlyBilling",
            "changeId": "chg-a3-composition",
            "month": fixture["month"],
            "scopeRevenue": fixture["scopeRevenueMicros"],
            "treatedRevenue": fixture["treatedRevenueMicros"],
            "feeMicros": fixture["feeMicros"],
            "holdout": {
                "measuredLift": fixture["measuredLift"],
                "lowerAlpha": fixture["lowerAlpha"],
                "revenueMicros": fixture["treatedRevenueMicros"],
                "feeMicros": fixture["feeMicros"],
                "evidence": {
                    "monthEnd": fixture["monthEnd"],
                    "exposedIsTest": fixture["exposedIsTest"],
                    "measurementScope": "u1",
                    "windowDays": [
                        f"2026-10-{fixture['windowStartDay'] + offset:02d}"
                        for offset in range(fixture["windowDays"])
                    ],
                    "rows": rows,
                    "pretest": {},
                    "unidentifiedMicros": 0,
                    "unidentifiedShareBp": 0,
                    "assigned": {
                        "exposed": fixture["exposedPlayers"],
                        "control": fixture["controlPlayers"],
                    },
                },
            },
        }

    def test_signed_record_fee_equals_the_golden_vector(self):
        fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        # The premise of the golden vector: the bound, not the point estimate.
        self.assertNotEqual(fixture["measuredLift"], fixture["lowerAlpha"])
        record = self.build_record(fixture)
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(record, handle)
            path = handle.name
        module = load_module()
        report = run_script(module, [path])
        self.assertEqual(report["feeMicros"], fixture["feeMicros"])
        self.assertEqual(report["billingLift"], fixture["lowerAlpha"])
        self.assertEqual(report["treatedRevenueMicros"], fixture["treatedRevenueMicros"])
        # The evidence's own recomputed bound differs from the billed one.
        self.assertAlmostEqual(report["lowerAlpha"], fixture["lowerAlpha"], places=12)
        self.assertAlmostEqual(report["lift"], fixture["measuredLift"], places=12)
        self.assertGreater(report["lift"], report["lowerAlpha"])
        self.assertEqual(report["coverage"]["unidentifiedShareBp"], 0)

    def test_incomplete_window_refuses_a_fee(self):
        fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        record = self.build_record(fixture)
        record["holdout"]["evidence"]["windowDays"] = \
            record["holdout"]["evidence"]["windowDays"][:-1]
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(record, handle)
            path = handle.name
        module = load_module()
        with self.assertRaises(SystemExit):
            run_script(module, [path])

    def test_non_contiguous_window_refuses_a_fee(self):
        fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        record = self.build_record(fixture)
        window = record["holdout"]["evidence"]["windowDays"]
        window[1] = "2026-10-15"  # skip a day, keep 28 entries
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(record, handle)
            path = handle.name
        module = load_module()
        with self.assertRaises(SystemExit):
            run_script(module, [path])

    def test_identity_share_is_a_diagnostic_not_a_gate(self):
        fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        record = self.build_record(fixture)
        # A large unidentified block in the control arm. It stays in the
        # control total, so it can only lower the measured lift; the share is
        # reported, and the signed bound still has to match the recomputation.
        day = record["holdout"]["evidence"]["rows"][0]["day"]
        total = sum(
            row["revenueMicros"] for row in record["holdout"]["evidence"]["rows"]
        )
        record["holdout"]["evidence"]["rows"].append({
            "day": day, "userId": None, "idfv": None, "idfa": None,
            "test": True, "revenueMicros": total // 8,
        })
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(record, handle)
            path = handle.name
        module = load_module()
        # The anonymous revenue changes the estimate, so the old signed bound
        # no longer matches: that disagreement is the refusal.
        with self.assertRaises(SystemExit):
            run_script(module, [path])

    def test_tampered_signed_bound_refuses_the_fee(self):
        fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        record = self.build_record(fixture)
        record["holdout"]["lowerAlpha"] = float(record["holdout"]["lowerAlpha"]) + 0.5
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(record, handle)
            path = handle.name
        module = load_module()
        with self.assertRaises(SystemExit):
            run_script(module, [path])

    def test_record_without_treated_revenue_refuses_a_fee(self):
        fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        record = self.build_record(fixture)
        del record["treatedRevenue"]
        record["holdout"]["revenueMicros"] = 0
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(record, handle)
            path = handle.name
        module = load_module()
        with self.assertRaises(SystemExit):
            run_script(module, [path])


if __name__ == "__main__":
    unittest.main()

