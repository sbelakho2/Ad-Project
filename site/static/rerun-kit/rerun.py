#!/usr/bin/env python3
"""Re-run the Apex lift estimate from a dashboard CSV, with no Rust toolchain.

This mirrors the engine's estimator exactly: the per-day log ratio of the two
arms, the sample standard deviation, and the one-sided 90% Student-t lower
bound. Feed it the same export you would give `apex-lift rerun` and it prints
the same numbers to the rounding, with every daily log ratio in the output.

Usage:
    python3 rerun.py export.csv                 # users column present
    python3 rerun.py export.csv --split 0.25    # MAX: no users, pass the test share

CSV columns: day,arm,revenue,users (users may be blank on MAX).
"""

import argparse
import csv
import json
import math
import sys


def lgamma(x):
    return math.lgamma(x)


def betacf(a, b, x):
    max_iterations = 300
    epsilon = 3e-16
    tiny = 1e-300
    qab = a + b
    qap = a + 1.0
    qam = a - 1.0
    c = 1.0
    d = 1.0 - qab * x / qap
    if abs(d) < tiny:
        d = tiny
    d = 1.0 / d
    h = d
    for m in range(1, max_iterations + 1):
        m2 = 2.0 * m
        aa = m * (b - m) * x / ((qam + m2) * (a + m2))
        d = 1.0 + aa * d
        if abs(d) < tiny:
            d = tiny
        c = 1.0 + aa / c
        if abs(c) < tiny:
            c = tiny
        d = 1.0 / d
        h *= d * c
        aa = -(a + m) * (b + m) * x / ((a + m2) * (qap + m2))
        d = 1.0 + aa * d
        if abs(d) < tiny:
            d = tiny
        c = 1.0 + aa / c
        if abs(c) < tiny:
            c = tiny
        d = 1.0 / d
        delta = d * c
        h *= delta
        if abs(delta - 1.0) < epsilon:
            break
    return h


def betai(a, b, x):
    if x <= 0.0:
        return 0.0
    if x >= 1.0:
        return 1.0
    front = math.exp(
        lgamma(a + b) - lgamma(a) - lgamma(b) + a * math.log(x) + b * math.log(1.0 - x)
    )
    if x < (a + 1.0) / (a + b + 2.0):
        return front * betacf(a, b, x) / a
    return 1.0 - front * betacf(b, a, 1.0 - x) / b


def t_cdf(t, df):
    if t == 0.0:
        return 0.5
    x = df / (df + t * t)
    beta = betai(df / 2.0, 0.5, x) / 2.0
    return 1.0 - beta if t > 0.0 else beta


def t_quantile(df, level_99=False):
    if df == 0:
        return math.inf
    target = 0.99 if level_99 else 0.90
    low, high = 0.0, 64.0
    for _ in range(80):
        mid = (low + high) / 2.0
        if t_cdf(mid, df) < target:
            low = mid
        else:
            high = mid
    return (low + high) / 2.0


def load_ratios(path, split):
    control = {}
    test = {}
    users = {}
    with open(path, newline="") as handle:
        for row in csv.DictReader(handle):
            day = (row.get("day") or "").strip()
            arm = (row.get("arm") or "").strip().lower()
            revenue = float(row["revenue"])
            user = (row.get("users") or "").strip()
            if day == "" or arm not in ("control", "test"):
                continue
            target = test if arm == "test" else control
            target[day] = target.get(day, 0.0) + revenue
            if user:
                users.setdefault(day, {})[arm] = float(user)
    ratios = []
    for day in sorted(set(control) | set(test)):
        if day in users and "control" in users[day] and "test" in users[day]:
            control_value = control.get(day, 0.0) / users[day]["control"]
            test_value = test.get(day, 0.0) / users[day]["test"]
        else:
            control_value = control.get(day, 0.0) / (1.0 - split)
            test_value = test.get(day, 0.0) / split
        if control_value > 0.0 and test_value > 0.0:
            ratios.append(math.log(test_value / control_value))
    return ratios


def estimate(ratios):
    if len(ratios) < 2 or any(not math.isfinite(r) for r in ratios):
        return None
    n = float(len(ratios))
    mean = sum(ratios) / n
    variance = sum((r - mean) ** 2 for r in ratios) / (n - 1.0)
    sd = math.sqrt(variance)
    se = sd / math.sqrt(n)
    t90 = t_quantile(len(ratios) - 1)
    return {
        "days": len(ratios),
        "meanLog": mean,
        "lift": math.exp(mean) - 1.0,
        "lower90": math.exp(mean - t90 * se) - 1.0,
        "upper90": math.exp(mean + t90 * se) - 1.0,
        "dailyLogRatios": ratios,
    }


def main():
    parser = argparse.ArgumentParser(description="Re-run the Apex lift estimate.")
    parser.add_argument("csv", help="the dashboard export")
    parser.add_argument(
        "--split",
        type=float,
        default=0.25,
        help="the test arm's allocated share (used when the export has no users)",
    )
    args = parser.parse_args()
    if not 0.0 < args.split < 1.0:
        print("--split must be between 0 and 1", file=sys.stderr)
        return 2
    report = estimate(load_ratios(args.csv, args.split))
    if report is None:
        print("no usable estimate from this export", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
