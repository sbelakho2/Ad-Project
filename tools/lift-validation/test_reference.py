#!/usr/bin/env python3
"""Unit tests for the Python lift reference kit.

Run with:
    python3 -m unittest discover -s tools/lift-validation -p 'test_*.py'
"""

from __future__ import annotations

import math
import unittest

import apex_lift_reference as ref
from cases import build_cases


class TQuantileTests(unittest.TestCase):
    def test_published_one_sided_quantiles(self):
        cases = [
            (1, False, 3.0777),
            (1, True, 31.8205),
            (6, False, 1.4398),
            (6, True, 3.1427),
            (13, False, 1.3502),
            (27, False, 1.3137),
            (30, False, 1.3104),
            (60, False, 1.2958),
            (120, False, 1.2886),
            (1000, False, 1.2824),
        ]
        for df, level_99, expected in cases:
            with self.subTest(df=df, level_99=level_99):
                self.assertAlmostEqual(ref.t_quantile(df, level_99), expected, delta=5e-4)
        self.assertTrue(math.isinf(ref.t_quantile(0, False)))

    def test_incomplete_beta_uses_the_rust_qab_term(self):
        # The historical Python kit used (b + m) here. That shifts the
        # continued fraction and the t bound; the Rust term is a + b + m.
        got = ref.betacf(2.0, 3.0, 0.4)
        self.assertTrue(math.isfinite(got))
        # A stable independent identity: I_x(a,b) is symmetric under
        # (a,b,x) -> (b,a,1-x).
        left = ref.betai(2.5, 4.0, 0.3)
        right = 1.0 - ref.betai(4.0, 2.5, 0.7)
        self.assertAlmostEqual(left, right, delta=1e-12)


class EstimatorTests(unittest.TestCase):
    def test_estimate_matches_the_hand_computed_golden(self):
        ratios = [0.030, 0.012, -0.004, 0.026, 0.041, 0.019, 0.008]
        current = ref.estimate(ratios)
        self.assertIsNotNone(current)
        self.assertEqual(current["days"], 7)
        self.assertAlmostEqual(current["meanLog"], 0.018857, delta=1e-6)
        self.assertAlmostEqual(current["sd"], 0.015016, delta=1e-6)
        self.assertAlmostEqual(current["lower90"], 0.010743, delta=1e-6)
        self.assertLess(current["lower99"], current["lower90"])
        self.assertGreater(current["upper90"], current["lift"])

    def test_estimate_refuses_short_and_non_finite_series(self):
        self.assertIsNone(ref.estimate([]))
        self.assertIsNone(ref.estimate([0.01]))
        self.assertIsNone(ref.estimate([0.01, float("nan")]))
        self.assertIsNone(ref.estimate([float("inf"), 1.0]))

    def test_bootstrap_is_deterministic_and_reaches_the_low_tail(self):
        days = [(1.2 if index % 2 == 0 else 0.85, 1.0) for index in range(14)]
        first = ref.bootstrap_lower_bound(days, 7, 4_000, 17)
        second = ref.bootstrap_lower_bound(days, 7, 4_000, 17)
        self.assertEqual(first, second)
        self.assertLess(first, 0.0)
        steady = [(1.03, 1.0) for _ in range(14)]
        self.assertGreater(ref.bootstrap_lower_bound(steady, 7, 2_000, 11), 0.0)
        self.assertIsNone(ref.bootstrap_lower_bound([], 7, 100, 1))
        self.assertIsNone(ref.bootstrap_lower_bound([(1.0, 0.0)], 7, 100, 1))
        self.assertIsNone(ref.bootstrap_lower_bound([(1.0, 1.0)], 0, 100, 1))
        self.assertIsNone(ref.bootstrap_lower_bound([(1.0, 1.0)], 7, 99, 1))

    def test_horvitz_thompson_uses_split_probabilities(self):
        half = ref.horvitz_thompson_estimate([100.0] * 3, [110.0] * 3, 0.5)
        self.assertIsNotNone(half)
        self.assertAlmostEqual(half["estimate"]["lift"], 0.10, delta=1e-12)
        self.assertEqual(half["testProbability"], 0.5)
        self.assertGreater(half["variance"], 0.0)
        quarter = ref.horvitz_thompson_estimate([100.0] * 9, [110.0] * 3, 0.25)
        self.assertAlmostEqual(quarter["estimate"]["lift"], 0.10, delta=1e-12)
        self.assertIsNone(ref.horvitz_thompson_estimate([], [1.0], 0.5))
        self.assertIsNone(ref.horvitz_thompson_estimate([1.0], [1.0], 0.0))
        self.assertIsNone(ref.horvitz_thompson_estimate([-1.0], [1.0], 0.5))


class WindowTests(unittest.TestCase):
    def test_burn_in_is_by_calendar_position(self):
        window = ref.build_window(
            [
                {
                    "day": "2026-10-01",
                    "revenue": 1_000_000,
                    "users": None,
                    "secondPullRevenue": 1_000_000,
                    "secondPullUsers": None,
                    "fill": None,
                    "secondPullFill": None,
                    "firstObservedAtUnixS": 1_700_000_000,
                    "secondObservedAtUnixS": 1_700_086_400,
                    "firstSourceSha256": "a",
                    "secondSourceSha256": "b",
                }
            ],
            [
                {
                    "day": "2026-10-01",
                    "revenue": 1_000_000,
                    "users": None,
                    "secondPullRevenue": 1_000_000,
                    "secondPullUsers": None,
                    "fill": None,
                    "secondPullFill": None,
                    "firstObservedAtUnixS": 1_700_000_000,
                    "secondObservedAtUnixS": 1_700_086_400,
                    "firstSourceSha256": "a",
                    "secondSourceSha256": "b",
                }
            ],
            7,
            0.25,
            3,
        )
        self.assertEqual(window["problems"][0]["kind"], "burnIn")

    def test_incident_runs_only_count_consecutive_days(self):
        days = [f"2026-10-{index:02d}" for index in range(1, 6)]

        def arm(day, revenue):
            return {
                "day": day,
                "revenue": revenue,
                "users": None,
                "secondPullRevenue": revenue,
                "secondPullUsers": None,
                "fill": None,
                "secondPullFill": None,
                "firstObservedAtUnixS": 1_700_000_000,
                "secondObservedAtUnixS": 1_700_086_400,
                "firstSourceSha256": "a",
                "secondSourceSha256": "b",
            }

        control = [arm(day, 1_000_000) for day in days]
        test = [arm(day, 1_000_000) for day in days]
        control[3]["revenue"] = test[3]["revenue"] = 400_000
        control[4]["revenue"] = test[4]["revenue"] = 100_000
        self.assertEqual(
            ref.incident_runs(control, test),
            [("2026-10-04", 1), ("2026-10-05", 2)],
        )


class GuardrailTests(unittest.TestCase):
    def test_fast_harm_fill_and_split(self):
        prereg = {
            "harmPoint": -0.05,
            "fillRewarded": 0.02,
            "fillBanner": 0.05,
            "splitChiSquare": 10.8276,
            "testShare": 0.25,
        }
        days = [
            {
                "day": "2026-10-01",
                "logRatio": -0.07,
                "lift": math.exp(-0.07) - 1.0,
                "upper99": -0.07,
                "upper90": -0.07,
                "fillDelta": -0.03,
                "calendarIndex": 0,
            },
            {
                "day": "2026-10-02",
                "logRatio": -0.07,
                "lift": math.exp(-0.07) - 1.0,
                "upper99": -0.07,
                "upper90": -0.07,
                "fillDelta": -0.03,
                "calendarIndex": 1,
            },
        ]
        events = ref.evaluate_guardrails(
            prereg, days, [], (7_000, 3_000), "rewarded", [], 0, []
        )
        kinds = [event["kind"] for event in events]
        self.assertIn("fastHarm", kinds)
        self.assertIn("fillRewardedInterstitial", kinds)
        self.assertIn("splitMismatch", kinds)
        self.assertTrue(ref.aa_split_void([]))
        self.assertFalse(ref.aa_split_void([0.001, -0.001, 0.002, -0.002, 0.0, 0.001]))


class DecisionTests(unittest.TestCase):
    def setUp(self):
        self.case = build_cases()[0]

    def test_daily_keep_rollback_and_extension(self):
        outcomes = {
            case["id"]: ref.run_case(case)["decision"]["verdict"]
            for case in build_cases()
        }
        self.assertEqual(outcomes["daily-keep"], "keep")
        self.assertEqual(outcomes["daily-flat"], "rollback")
        self.assertEqual(outcomes["daily-interim-99"], "keep")
        self.assertEqual(outcomes["daily-extension"], "extend")
        self.assertEqual(outcomes["daily-fast-harm"], "rollback")
        self.assertEqual(outcomes["daily-confirmed-harm"], "rollback")
        self.assertEqual(outcomes["daily-split-mismatch"], "void")
        self.assertEqual(outcomes["daily-ten-percent"], "rollback")

    def test_the_lambda_is_the_lower_bound_and_fee_follows_it(self):
        result = next(
            ref.run_case(case) for case in build_cases() if case["id"] == "fee-on-keep"
        )
        self.assertEqual(result["decision"]["verdict"], "keep")
        self.assertAlmostEqual(
            result["decision"]["lambda"], result["decision"]["bootstrapLower"], delta=1e-12
        )
        fee = result["fee"]
        self.assertGreater(fee["feeMicros"], 0)
        self.assertAlmostEqual(fee["logShare"], 1.0, delta=1e-12)
        expected_fee = round(fee["rateBp"] / 10_000.0 * fee["incremental"] * fee["logShare"])
        self.assertEqual(fee["feeMicros"], expected_fee)


class PlayerTests(unittest.TestCase):
    def test_cuped_reduces_the_covariate_signal(self):
        planned = next(case for case in build_cases() if case["id"] == "player-cuped")
        result = ref.run_case(planned)
        self.assertEqual(result["decision"]["verdict"], "keep")
        self.assertIsNotNone(result["playerEstimate"])
        self.assertGreater(result["playerEstimate"]["estimate"]["lower90"], 0.0)

    def test_player_window_folds_players_and_rejects_cross_arm(self):
        rows = [
            {"day": "d1", "userId": "u1", "test": True, "revenueMicros": 10},
            {"day": "d2", "userId": "u1", "test": True, "revenueMicros": 5},
            {"day": "d1", "userId": "u2", "test": False, "revenueMicros": 7},
        ]
        window = ref.player_window(rows, None)
        self.assertEqual(window["test"], [15.0])
        self.assertEqual(window["control"], [7.0])
        with self.assertRaises(ValueError):
            ref.player_window(
                rows + [{"day": "d3", "userId": "u1", "test": False, "revenueMicros": 1}],
                None,
            )


class SwitchbackTests(unittest.TestCase):
    def test_plan_is_weekday_balanced_and_never_triples(self):
        planned = ref.switchback_plan(14, 7)
        self.assertIsNotNone(planned)
        self.assertEqual(planned["blocksPerArm"], 7)
        self.assertEqual(sum(planned["blocks"]), 7)
        self.assertFalse(ref._has_triple(planned["blocks"]))
        self.assertIsNone(ref.switchback_plan(12, 7))
        self.assertEqual(ref.switchback_plan(14, 3), ref.switchback_plan(14, 3))

    def test_block_analysis_uses_the_plan_labels(self):
        planned = ref.switchback_plan(14, 7)
        values = [1.04 if is_test else 1.0 for is_test in planned["blocks"]]
        billable = ref.switchback_out(
            {
                "blockCount": 14,
                "seed": 7,
                "values": values,
                "usedBlocksPerArm": 7,
            }
        )
        self.assertTrue(billable["billable"])
        self.assertGreater(billable["lambda"], 0.0)
        flipped = {
            "blockCount": 14,
            "seed": 7,
            "values": [1.0 if is_test else 1.04 for is_test in planned["blocks"]],
            "usedBlocksPerArm": 7,
        }
        self.assertFalse(ref.switchback_out(flipped)["billable"])


class RerunTests(unittest.TestCase):
    def test_revenue_parsing_is_exact_and_strict(self):
        self.assertEqual(ref.parse_revenue_micros("12.50"), 12_500_000)
        self.assertEqual(ref.parse_revenue_micros("0.006"), 6_000)
        self.assertEqual(ref.parse_revenue_micros("7"), 7_000_000)
        for bad in ("", "-1.00", "abc", "1.2345678", "1..2", ".", ".5"):
            self.assertIsNone(ref.parse_revenue_micros(bad), bad)

    def test_rerun_drops_three_burn_in_days(self):
        planned = next(case for case in build_cases() if case["id"] == "rerun-burn-in")
        rows = ref.parse_csv(planned["rerun"]["csv"])
        report = ref.rerun(rows, 7, 0.5, 3)
        self.assertEqual(report["days"], 7)
        self.assertAlmostEqual(report["lift"], 0.06, delta=1e-9)
        kept = ref.rerun(rows, 7, 0.5, 0)
        self.assertGreater(abs(kept["lift"] - 0.06), 0.01)

class CaseCorpusTests(unittest.TestCase):
    def test_the_corpus_is_deterministic_and_covers_every_verdict(self):
        first = build_cases()
        second = build_cases()
        self.assertEqual(first, second)
        verdicts = {ref.run_case(case)["decision"]["verdict"] for case in first}
        self.assertEqual(verdicts, {"keep", "rollback", "void", "pending", "extend"})


if __name__ == "__main__":
    unittest.main()
