#!/usr/bin/env python3
"""Deterministic and adversarial datasets for the lift twin.

Every case is a complete JSON document. The Rust example and the Python
reference read the same bytes, so a disagreement is a rules disagreement, not
a dataset difference. Seeds are fixed (SplitMix64 for bootstrap draws and the
switchback planner), and the generator never consults wall-clock time.
"""

from __future__ import annotations

import datetime as _dt
import hashlib

import apex_lift_reference as ref


def _day_strings(count: int, start: _dt.date | None = None):
    start = start or _dt.date(2026, 10, 1)
    return [(start + _dt.timedelta(days=index)).isoformat() for index in range(count)]


def _hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def arm_day(day: str, revenue: int, users=None, fill=None, final=True):
    first_at = 1_700_000_000
    return {
        "day": day,
        "revenue": revenue,
        "users": users,
        "secondPullRevenue": revenue if final else None,
        "secondPullUsers": users if final else None,
        "fill": fill,
        "secondPullFill": fill,
        "firstObservedAtUnixS": first_at,
        "secondObservedAtUnixS": first_at + ref.SECOND_PULL_MIN_SECS,
        "firstSourceSha256": _hash(f"{day}-first"),
        "secondSourceSha256": _hash(f"{day}-second"),
    }


def prereg(**overrides):
    value = {
        "experimentId": "twin-exp",
        "instanceId": "twin-exp@1",
        "scope": "unit-1",
        "changeDescription": "raise t1 floor",
        "beforeValue": '{"floor":1000000}',
        "testShare": 0.25,
        "burnInDays": 3,
        "windowDays": 14,
        "interimLook": True,
        "extensionAllowed": True,
        "codeVersionHash": "twin-1",
        "rateBp": 2000,
        "harmPoint": -0.05,
        "fillRewarded": 0.02,
        "fillBanner": 0.05,
        "splitChiSquare": 10.8276,
        "minPointLift": 0.01,
    }
    value.update(overrides)
    return value


def window(control, test, **overrides):
    value = {
        "control": control,
        "test": test,
        "testAllocatedShare": 0.25,
        "atFinalLook": True,
        "extensionUsed": False,
        "observedScope": "unit-1",
        "observedChangeDescription": "raise t1 floor",
        "bootstrapIterations": 2_000,
        "bootstrapBlockLen": 7,
        "bootstrapSeed": 99,
    }
    value.update(overrides)
    return value


def series(
    raw_days: int,
    ratios,
    *,
    base_revenue: int = 1_000_000,
    users=1_000,
    control_fill=0.70,
    test_fill_deltas=None,
):
    """One control-first series with a per-day test ratio.

    ``ratios`` may be one float or one value per raw day. ``users=None`` uses
    the MAX share path; otherwise both arms report the same user count.
    """
    days = _day_strings(raw_days)
    if not isinstance(ratios, (list, tuple)):
        ratios = [ratios] * raw_days
    control = []
    test = []
    for index, (day, ratio) in enumerate(zip(days, ratios)):
        test_revenue = int(round(base_revenue * ratio))
        fill_delta = (test_fill_deltas or [0.0] * raw_days)[index]
        control.append(
            arm_day(day, base_revenue, users=users, fill=control_fill if users else None)
        )
        test.append(
            arm_day(
                day,
                test_revenue,
                users=users,
                fill=(control_fill + fill_delta) if users else None,
            )
        )
    return control, test


def max_share_series(raw_days: int, ratio: float, share: float = 0.25):
    days = _day_strings(raw_days)
    control_revenue = 1_000_000
    test_revenue = int(round(control_revenue * ratio * share / (1.0 - share)))
    control = [arm_day(day, control_revenue) for day in days]
    test = [arm_day(day, test_revenue) for day in days]
    return control, test


def _player_rows(
    control_players: int,
    test_players: int,
    control_delta: int,
    test_delta: int,
    control_pre: int = 1_000,
    test_pre: int = 1_000,
):
    """Per-player pre/post rows with within-arm covariate spread.

    CUPED removes the *between-arm* pre-period gap, but a treatment effect
    survives only because pre values vary inside each arm; a two-point
    covariate would make the pooled regression fit both arm means exactly.
    """
    rows = []
    pretest = {}
    day = "2026-10-20"
    for index in range(control_players):
        user = f"c{index:05d}"
        pre = control_pre + (index * 37 % 201) - 100
        post = pre + control_delta + (index * 13 % 7) - 3
        rows.append(
            {
                "day": day,
                "userId": user,
                "idfv": None,
                "idfa": None,
                "test": False,
                "revenueMicros": post,
            }
        )
        pretest[user] = pre
    for index in range(test_players):
        user = f"t{index:05d}"
        pre = test_pre + (index * 37 % 201) - 100
        post = pre + test_delta + (index * 13 % 7) - 3
        rows.append(
            {
                "day": day,
                "userId": user,
                "idfv": None,
                "idfa": None,
                "test": True,
                "revenueMicros": post,
            }
        )
        pretest[user] = pre
    return rows, pretest


def build_cases():
    """Return the deterministic twin corpus."""
    cases = []

    # --- Daily log-ratio estimator and decision paths. --------------------
    control, test = series(17, 1.02)
    cases.append(
        {
            "id": "daily-keep",
            "prereg": prereg(),
            "window": window(control, test),
        }
    )

    control, test = series(17, 1.0)
    cases.append(
        {
            "id": "daily-flat",
            "prereg": prereg(),
            "window": window(control, test),
        }
    )

    control, test = series(10, 1.02)
    cases.append(
        {
            "id": "daily-interim-99",
            "prereg": prereg(),
            "window": window(control, test, atFinalLook=False),
        }
    )

    alternating = [1.12 if index % 2 == 0 else 0.92 for index in range(17)]
    control, test = series(17, alternating)
    cases.append(
        {
            "id": "daily-extension",
            "prereg": prereg(),
            "window": window(control, test),
        }
    )

    control, test = series(17, 0.94)
    cases.append(
        {
            "id": "daily-fast-harm",
            "prereg": prereg(),
            "window": window(control, test),
        }
    )

    control, test = series(10, 0.96)
    cases.append(
        {
            "id": "daily-confirmed-harm",
            "prereg": prereg(windowDays=7),
            "window": window(control, test),
        }
    )

    control, test = series(
        17,
        1.02,
        test_fill_deltas=[0.0, 0.0, 0.0, -0.04, -0.04] + [0.0] * 12,
    )
    cases.append(
        {
            "id": "daily-fill-stop",
            "prereg": prereg(),
            "window": window(control, test),
            "guardrails": {"fillFormat": "rewarded"},
        }
    )

    control, test = series(17, 1.02)
    cases.append(
        {
            "id": "daily-split-mismatch",
            "prereg": prereg(),
            "window": window(control, test),
            "guardrails": {"uniquePlayers": [7_000, 3_000]},
        }
    )

    control, test = series(17, 1.02)
    cases.append(
        {
            "id": "daily-publisher-edit",
            "prereg": prereg(),
            "window": window(control, test),
            "guardrails": {"settingsChanged": ["2026-10-06"]},
        }
    )

    control, test = series(17, 1.02)
    cases.append(
        {
            "id": "daily-reporting-gap-pause",
            "prereg": prereg(),
            "window": window(control, test),
            "guardrails": {"staleDays": 1},
        }
    )

    control, test = series(17, 1.02)
    cases.append(
        {
            "id": "daily-reporting-gap-void",
            "prereg": prereg(),
            "window": window(control, test),
            "guardrails": {"staleDays": 3},
        }
    )

    control, test = series(17, 1.02)
    cases.append(
        {
            "id": "daily-incident",
            "prereg": prereg(),
            "window": window(control, test),
            "guardrails": {"incidentRuns": [["2026-10-06", 2]]},
        }
    )

    control, test = series(17, 1.02)
    test[8]["secondPullRevenue"] = None
    cases.append(
        {
            "id": "daily-incomplete",
            "prereg": prereg(),
            "window": window(control, test),
        }
    )

    control, test = series(17, 1.02)
    control[6]["day"], test[6]["day"] = control[7]["day"], test[7]["day"]
    cases.append(
        {
            "id": "daily-fatal-sequence",
            "prereg": prereg(),
            "window": window(control, test),
        }
    )

    control, test = series(17, 1.02)
    cases.append(
        {
            "id": "daily-ten-percent",
            "prereg": prereg(testShare=0.10),
            "window": window(control, test, testAllocatedShare=0.10),
        }
    )

    control, test = max_share_series(17, 1.02)
    cases.append(
        {
            "id": "daily-max-share",
            "prereg": prereg(testShare=0.25),
            "window": window(control, test, testAllocatedShare=0.25),
        }
    )

    # --- Per-player Horvitz-Thompson and CUPED. ---------------------------
    control, test = series(17, 1.0)
    rows, pretest = _player_rows(3_000, 1_000, 20, 60)
    cases.append(
        {
            "id": "player-ht-keep",
            "prereg": prereg(),
            "window": window(control, test),
            "players": {"rows": rows, "pretest": pretest, "cuped": False},
        }
    )

    control, test = series(17, 1.0)
    rows, pretest = _player_rows(3_000, 1_000, 20, 80)
    cases.append(
        {
            "id": "player-cuped",
            "prereg": prereg(),
            "window": window(control, test),
            "players": {"rows": rows, "pretest": pretest, "cuped": True},
        }
    )

    control, test = series(17, 1.0)
    rows, pretest = _player_rows(3_000, 1_000, 20, 20, test_pre=2_000)
    cases.append(
        {
            "id": "player-rollback",
            "prereg": prereg(),
            "window": window(control, test),
            "players": {"rows": rows, "pretest": pretest, "cuped": True},
        }
    )

    # --- Switchback plan and block analysis. ------------------------------
    for case_id, seed, noisy in [
        ("switchback-billable", 7, False),
        ("switchback-not-billable", 11, True),
    ]:
        planned = ref.switchback_plan(14, seed)
        test_indexes = [index for index, is_test in enumerate(planned["blocks"]) if is_test]
        values = [1.04 if is_test else 1.0 for is_test in planned["blocks"]]
        if noisy:
            for position, index in enumerate(test_indexes):
                values[index] = 1.04 if position % 2 == 0 else 0.97
        control, test = series(17, 1.0)
        cases.append(
            {
                "id": case_id,
                "prereg": prereg(),
                "window": window(control, test),
                "switchback": {
                    "blockCount": 14,
                    "seed": seed,
                    "values": values,
                    "usedBlocksPerArm": 7,
                },
            }
        )

    control, test = series(17, 1.0)
    cases.append(
        {
            "id": "switchback-plan-only",
            "prereg": prereg(),
            "window": window(control, test),
            "switchback": {"blockCount": 14, "seed": 11},
        }
    )

    # --- Re-run kit with burn-in. -----------------------------------------
    csv_lines = ["day,arm,revenue,users"]
    for index, day in enumerate(_day_strings(10)):
        test_revenue = "200.00" if index < 3 else "106.00"
        csv_lines.append(f"{day},control,100.00,1000")
        csv_lines.append(f"{day},test,{test_revenue},1000")
    control, test = series(17, 1.0)
    cases.append(
        {
            "id": "rerun-burn-in",
            "prereg": prereg(windowDays=7),
            "window": window(control, test),
            "rerun": {
                "csv": "\n".join(csv_lines) + "\n",
                "windowDays": 7,
                "allocatedShare": 0.5,
                "burnInDays": 3,
            },
        }
    )

    # --- Fee basis on a kept result. --------------------------------------
    control, test = series(17, 1.02)
    cases.append(
        {
            "id": "fee-on-keep",
            "prereg": prereg(),
            "window": window(control, test),
            "billing": {"scopeRevenueMicros": 32_000_000, "keptDay": "2026-10-01"},
        }
    )

    control, test = series(17, 1.0)
    cases.append(
        {
            "id": "fee-on-rollback",
            "prereg": prereg(),
            "window": window(control, test),
            "billing": {"scopeRevenueMicros": 32_000_000, "keptDay": "2026-10-01"},
        }
    )

    # --- One combined case exercises every output block at once. ----------
    control, test = series(17, 1.02)
    planned = ref.switchback_plan(14, 3)
    values = [1.04 if is_test else 1.0 for is_test in planned["blocks"]]
    cases.append(
        {
            "id": "combined",
            "prereg": prereg(),
            "window": window(control, test),
            "guardrails": {"armCounts": [[250, 750], [250, 750]]},
            "switchback": {
                "blockCount": 14,
                "seed": 3,
                "values": values,
                "usedBlocksPerArm": 7,
            },
            "billing": {"scopeRevenueMicros": 10_000_000, "keptDay": "2026-10-01"},
            "rerun": {
                "csv": "\n".join(csv_lines) + "\n",
                "windowDays": 7,
                "allocatedShare": 0.5,
                "burnInDays": 3,
            },
        }
    )

    return cases
