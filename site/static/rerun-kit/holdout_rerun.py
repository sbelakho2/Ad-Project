#!/usr/bin/env python3
"""Recompute an A3 monthly holdout reading from its signed player evidence.

Usage:
  python3 holdout_rerun.py evidence.json [--treated-revenue MICROS]

`evidence.json` is the `holdout.evidence` object in a signed monthlyBilling
record. The script resolves player ids in the documented order, folds repeated
player-days, applies CUPED with the estimated coefficient, computes the ratio
delta-method standard error and the conservative 90% lower bound, then applies
the 30% fee to the supplied treated revenue (or prints the fee for 1 micro).
"""
import json
import math
import sys

T90 = 1.697  # conservative Student-t at n >= 30 per arm
RATE = 0.30
ZERO_IDS = {
    "00000000-0000-0000-0000-000000000000",
    "00000000000000000000000000000000",
}


def player_key(row):
    for field in ("userId", "idfv", "idfa"):
        value = row.get(field)
        if value and value not in ZERO_IDS:
            return value
    return None


def main(argv):
    if len(argv) < 2:
        raise SystemExit(__doc__)
    with open(argv[1], encoding="utf-8") as handle:
        evidence = json.load(handle)
    treated = 1
    if "--treated-revenue" in argv:
        treated = int(argv[argv.index("--treated-revenue") + 1])
    exposed_is_test = bool(evidence["exposedIsTest"])
    pretest = evidence.get("pretest", {})
    rows = evidence.get("rows", [])
    total = sum(int(row["revenueMicros"]) for row in rows)
    unknown = 0
    arms = {}
    for row in rows:
        key = player_key(row)
        if key is None:
            unknown += int(row["revenueMicros"])
            continue
        exposed = bool(row["test"]) == exposed_is_test
        if key in arms and arms[key][0] != exposed:
            raise SystemExit(f"player {key} appears in both arms")
        arms.setdefault(key, [exposed, 0])[1] += int(row["revenueMicros"])
    players = [
        (key, exposed, revenue, pretest.get(key))
        for key, (exposed, revenue) in arms.items()
    ]
    exposed_values = [p for p in players if p[1]]
    control_values = [p for p in players if not p[1]]
    if len(exposed_values) < 30 or len(control_values) < 30:
        raise SystemExit("fewer than 30 unique players in an arm")
    pairs = [(float(p[3]), float(p[2])) for p in players if p[3] is not None]
    theta = 0.0
    mean_pre = 0.0
    if len(pairs) >= 2:
        n = len(pairs)
        mean_pre = sum(pre for pre, _ in pairs) / n
        mean_post = sum(post for _, post in pairs) / n
        cov = sum((pre - mean_pre) * (post - mean_post) for pre, post in pairs) / (n - 1)
        var = sum((pre - mean_pre) ** 2 for pre, _ in pairs) / (n - 1)
        if var > 0:
            theta = cov / var

    def adjusted(group):
        return [
            float(revenue) - (theta * (float(pre) - mean_pre) if pre is not None and theta != 0 else 0.0)
            for _, _, revenue, pre in group
        ]

    def mean(values):
        return sum(values) / len(values)

    def variance(values, mu):
        return sum((value - mu) ** 2 for value in values) / (len(values) - 1)

    e = adjusted(exposed_values)
    c = adjusted(control_values)
    e_mean = mean(e)
    c_mean = mean(c)
    if c_mean <= 0:
        raise SystemExit("control revenue is not positive")
    lift = (e_mean - c_mean) / c_mean
    se = math.sqrt(
        variance(e, e_mean) / len(e) / (c_mean ** 2)
        + (e_mean ** 2) * variance(c, c_mean) / len(c) / (c_mean ** 4)
    )
    lower = lift - T90 * se
    upper = lift + T90 * se
    bound = max(lower, 0.0)
    incremental = treated * (1.0 - 1.0 / (1.0 + bound)) if bound > 0 else 0.0
    fee = round(RATE * incremental)
    share_bp = (unknown * 10_000 // total) if total > 0 else 0
    print(json.dumps({
        "players": {"exposed": len(exposed_values), "control": len(control_values)},
        "coverage": {"unidentifiedMicros": unknown, "unidentifiedShareBp": share_bp},
        "cupedTheta": theta,
        "lift": lift,
        "standardError": se,
        "lower90": lower,
        "upper90": upper,
        "billingLift": bound,
        "treatedRevenueMicros": treated,
        "feeMicros": fee,
    }, indent=2))


if __name__ == "__main__":
    main(sys.argv)
