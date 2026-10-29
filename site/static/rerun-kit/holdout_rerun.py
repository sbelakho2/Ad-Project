#!/usr/bin/env python3
"""Recompute an A3 monthly holdout reading from its signed player evidence.

Usage:
  python3 holdout_rerun.py evidence.json [--treated-revenue MICROS] \
      [--bound-alpha LOWER_ALPHA] [--actual-gain-lower GAIN_LOWER]

The input may be the `holdout.evidence` object of a signed monthlyBilling
record, the enclosing holdout line itself (it carries `evidence`), or the
whole record (it carries `holdout.evidence`). The line carries the billing
bound, so with no flags the defaults are taken from it:

- `--bound-alpha` defaults to the line's `lowerAlpha` — the registered
  one-sided bound production bills from — instead of the script recomputing
  its own 90% lower bound;
- `--actual-gain-lower` defaults to the line's `actualGainLower` — the
  certificate cap — instead of capping by nothing.

With a bare evidence object neither default exists and the flags (or the
script's own recomputation) apply. The script resolves player ids in the
documented order, folds repeated player-days, applies CUPED with the
estimated coefficient, computes the ratio delta-method standard error, then
applies the 30% fee to the supplied treated revenue (or prints the fee for
1 micro).
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


def load_input(path):
    """Return (evidence, invoice_line) from one of the accepted shapes.

    `invoice_line` is the enclosing object that carries `lowerAlpha` and
    `actualGainLower`, or None when the file is a bare evidence object.
    """
    with open(path, encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise SystemExit("input must be a JSON object")
    if isinstance(data.get("evidence"), dict):
        return data["evidence"], data
    holdout = data.get("holdout")
    if isinstance(holdout, dict) and isinstance(holdout.get("evidence"), dict):
        return holdout["evidence"], holdout
    return data, None


def main(argv):
    if len(argv) < 2:
        raise SystemExit(__doc__)
    evidence, line = load_input(argv[1])
    treated = 1
    if "--treated-revenue" in argv:
        treated = int(argv[argv.index("--treated-revenue") + 1])
    # Defaults come from the enclosing invoice line, so without flags the
    # script bills exactly what production billed (OO): its registered bound
    # and its certificate cap, not the script's own recomputation.
    bound_alpha = None
    if line is not None and isinstance(line.get("lowerAlpha"), (int, float)):
        bound_alpha = float(line["lowerAlpha"])
    if "--bound-alpha" in argv:
        bound_alpha = float(argv[argv.index("--bound-alpha") + 1])
    actual_gain_lower = None
    if line is not None and isinstance(line.get("actualGainLower"), (int, float)):
        actual_gain_lower = float(line["actualGainLower"])
    if "--actual-gain-lower" in argv:
        actual_gain_lower = float(argv[argv.index("--actual-gain-lower") + 1])
    exposed_is_test = bool(evidence["exposedIsTest"])
    window_days = evidence.get("windowDays") or sorted({row.get("day") for row in evidence.get("rows", []) if row.get("day")})
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
    if len(window_days) < 28:
        raise SystemExit(
            f"evidence window covers {len(window_days)} of 28 days; no fee can be built from it"
        )
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
    bound_gain = bound_alpha if bound_alpha is not None else lower
    bound = max(bound_gain, 0.0)
    if actual_gain_lower is not None:
        bound = min(bound, actual_gain_lower)
    incremental = treated * (1.0 - 1.0 / (1.0 + bound)) if bound > 0 else 0.0
    fee = round(RATE * incremental)
    share_bp = (unknown * 10_000 // total) if total > 0 else 0
    print(json.dumps({
        "windowDays": len(window_days),
        "windowComplete": len(window_days) >= 28,
        "players": {"exposed": len(exposed_values), "control": len(control_values)},
        "coverage": {"unidentifiedMicros": unknown, "unidentifiedShareBp": share_bp},
        "cupedTheta": theta,
        "lift": lift,
        "standardError": se,
        "lower90": lower,
        "upper90": upper,
        "boundAlpha": bound_alpha,
        "actualGainLower": actual_gain_lower,
        "billingLift": bound,
        "treatedRevenueMicros": treated,
        "feeMicros": fee,
    }, indent=2))


if __name__ == "__main__":
    main(sys.argv)
