"""The public holdout rerun kit reproduces the conservative fee."""
import importlib.util
import json
import pathlib
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "site" / "static" / "rerun-kit" / "holdout_rerun.py"


def load_module():
    spec = importlib.util.spec_from_file_location("holdout_rerun", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class HoldoutRerunTest(unittest.TestCase):
    def test_a_gain_uses_the_lower_bound_and_reports_coverage(self):
        rows = []
        for day in range(4, 32):  # the complete 28-day window ending 2026-10-31
            day_id = f"2026-10-{day:02d}"
            for index in range(270):
                rows.append({
                    "day": day_id, "userId": f"e{index}", "idfv": None,
                    "idfa": None, "test": False, "revenueMicros": 104_000,
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
        import contextlib, io
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            module.main(["holdout_rerun.py", path, "--treated-revenue", "900000000"])
        report = json.loads(output.getvalue())
        assert report["feeMicros"] > 0
        self.assertEqual(report["players"], {"exposed": 270, "control": 30})
        self.assertEqual(report["windowDays"], 28)
        self.assertTrue(report["windowComplete"])
        self.assertAlmostEqual(report["lift"], 0.04)
        self.assertAlmostEqual(report["billingLift"], report["lower90"])
        self.assertGreater(report["feeMicros"], 0)
        self.assertEqual(report["coverage"]["unidentifiedShareBp"], 0)


if __name__ == "__main__":
    unittest.main()
