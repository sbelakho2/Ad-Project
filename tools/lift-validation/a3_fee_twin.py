#!/usr/bin/env python3
"""The A3 composition twin: one fixture, two implementations, one fee.

`a3_fee_fixture.json` is the shared golden vector. The Rust half is
`rust/crates/apex-autopilot/tests/a3_composition.rs`, which runs the real
chain (`PlayerEvidence` -> `measure_from_feed` -> `run_holdout_month` ->
`BillingBasisRecord` -> `compute_invoice_from_basis`) and asserts the
invoice's fee micros against the fixture. This module is the Python half:
it rebuilds the eligible-roster estimate from the same per-player rows with
the production formulas (assignment counts as denominators, CUPED skipped
on an empty pretest, ratio delta-method standard error, the registered
one-sided bound) and derives the bound fee

    round(0.30 * treatedRevenue * (1 - 1/(1 + max(lowerAlpha, 0))))

and fails if any value disagrees with the fixture. The fixture's measured
lift is strictly above its lowerAlpha, so a point-estimate fee cannot
accidentally match.

Usage:
    python3 tools/lift-validation/a3_fee_twin.py
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

from apex_lift_reference import z_one_sided, _round_half_away

ROOT = Path(__file__).resolve().parent
FIXTURE = ROOT / "a3_fee_fixture.json"
Z90_ONE_SIDED = 1.2815515655446
INITIAL_ALPHA = 0.0225
RATE_BP = 3000
TOLERANCE = 1e-12


def estimate_from_fixture(fixture: dict) -> dict:
    """The production eligible-roster estimate over the fixture rows."""
    # Every player earns the same value on every day of the window, so the
    # per-player fold is the daily value times the window length.
    exposed = [
        float(
            fixture["exposedDailyMicrosBase"]
            + index % fixture["exposedDailyMicrosMod"]
            * fixture["exposedDailyMicrosStride"]
        )
        * fixture["windowDays"]
        for index in range(fixture["exposedPlayers"])
    ]
    control = [
        float(
            fixture["controlDailyMicrosBase"]
            + index % fixture["controlDailyMicrosMod"]
            * fixture["controlDailyMicrosStride"]
        )
        * fixture["windowDays"]
        for index in range(fixture["controlPlayers"])
    ]

    roster_exposed = float(fixture["exposedPlayers"])
    roster_control = float(fixture["controlPlayers"])
    total_assigned = roster_exposed + roster_control
    q = 0.90
    chi_square = (roster_exposed - total_assigned * q) ** 2 / (total_assigned * q * (1 - q))
    if chi_square > 10.8277:
        raise SystemExit(f"assignment-count chi-square {chi_square} failed")

    exposed_mean = sum(exposed) / roster_exposed
    control_mean = sum(control) / roster_control
    exposed_squares = sum(value * value for value in exposed)
    control_squares = sum(value * value for value in control)
    exposed_variance = max(
        (exposed_squares - roster_exposed * exposed_mean * exposed_mean)
        / (roster_exposed - 1.0),
        0.0,
    )
    control_variance = max(
        (control_squares - roster_control * control_mean * control_mean)
        / (roster_control - 1.0),
        0.0,
    )
    variance = (
        exposed_variance / roster_exposed / (control_mean * control_mean)
        + exposed_mean * exposed_mean * control_variance / roster_control
        / control_mean**4
    )
    standard_error = math.sqrt(variance)
    lift = (exposed_mean - control_mean) / control_mean
    return {
        "lift": lift,
        "standardError": standard_error,
        "lower90": lift - Z90_ONE_SIDED * standard_error,
        "upper90": lift + Z90_ONE_SIDED * standard_error,
        "lowerAlpha": lift - z_one_sided(INITIAL_ALPHA) * standard_error,
        "treatedRevenueMicros": sum(exposed),
    }


def fee_from_bound(treated: int, lower_alpha: float) -> int:
    bound = max(lower_alpha, 0.0)
    if bound <= 0.0:
        return 0
    incremental = treated * (1.0 - 1.0 / (1.0 + bound))
    return _round_half_away(incremental * RATE_BP / 10_000.0)


def main() -> int:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    estimate = estimate_from_fixture(fixture)
    failures = []

    def check(name: str, actual, expected) -> None:
        if isinstance(expected, int):
            if actual != expected:
                failures.append(f"{name}: {actual} != expected {expected}")
        elif not math.isclose(actual, expected, rel_tol=0.0, abs_tol=TOLERANCE):
            failures.append(f"{name}: {actual!r} != expected {expected!r}")

    check("lift", estimate["lift"], fixture["measuredLift"])
    check("standardError", estimate["standardError"], fixture["standardError"])
    check("lower90", estimate["lower90"], fixture["lower90"])
    check("upper90", estimate["upper90"], fixture["upper90"])
    check("lowerAlpha", estimate["lowerAlpha"], fixture["lowerAlpha"])
    check("treatedRevenueMicros", estimate["treatedRevenueMicros"],
          fixture["treatedRevenueMicros"])

    if estimate["lift"] == estimate["lowerAlpha"]:
        failures.append("the fixture no longer separates lift from lowerAlpha")

    fee = fee_from_bound(estimate["treatedRevenueMicros"], estimate["lowerAlpha"])
    check("feeMicros", fee, fixture["feeMicros"])

    if failures:
        for failure in failures:
            print(f"FAIL {failure}")
        print(f"\na3 fee twin FAILED: {len(failures)} disagreement(s)")
        return 1
    print(
        f"a3 fee twin PASS: lift={estimate['lift']!r} lowerAlpha="
        f"{estimate['lowerAlpha']!r} feeMicros={fee}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
