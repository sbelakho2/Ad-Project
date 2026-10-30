#!/usr/bin/env python3
"""Recompute an A3 monthly holdout fee from its signed player evidence.

Usage:
  python3 holdout_rerun.py record.json [--treated-revenue MICROS] \
      [--bound-alpha LOWER_ALPHA]

The input may be the whole signed monthlyBilling record, the enclosing
holdout line (it carries `evidence`), or the bare `holdout.evidence` object.
The fee is production's exact expression:

    fee = round(0.30 * treatedRevenue * (1 - 1/(1 + max(lowerAlpha, 0))))

- `treatedRevenue` is read from the record (`treatedRevenue`, then the
  holdout line's `revenueMicros`) and must be a real positive number of
  micros; the script refuses to invent a fee for a placeholder revenue.
  `--treated-revenue` overrides it explicitly.
- `lowerAlpha` is recomputed from the signed evidence and compared against
  the signed value; a disagreement refuses the fee. The line's signed bound
  is what the fee is then sized from. `--bound-alpha` supplies that signed
  value explicitly (and is verified the same way).

The evidence gates apply before any fee:

- exactly 28 contiguous measurement days in `windowDays` (or the distinct
  days the rows carry), otherwise there is no fee to rebuild;
- at least 30 unique players in each arm;
- the labelled feed must look like the registered 90/10 split. Without a
  roster this is a feed-shape diagnostic, not a proof of the original
  assignment law; with a roster the assignment counts drive the chi-square
  at p = 0.001.

Revenue with no resolvable player id is kept by arm and priced in that
arm's total, as production does. Its share is reported as a diagnostic and
is not a fee gate: a global ceiling cannot see differential arm loss.

The script resolves player ids in the documented order, folds repeated
player-days, applies CUPED with the estimated coefficient when a roster is
fully observed (the planned-share estimator does not use CUPED), and mirrors
production's two variance formulas:

- roster reading: the ratio delta-method standard error over assigned users;
- planned-share reading: the design-aware Cauchy bound over the two arm
  totals, with the registered 90/10 denominators.
"""
import json
import math
import sys

Z90 = 1.2815515655446  # production Z90_ONE_SIDED
INITIAL_ALPHA = 0.0225  # the registered one-sided error of lowerAlpha
RATE = 0.30
MIN_PLAYERS_PER_ARM = 30
CHI_SQUARE_THRESHOLD = 10.8277  # production estimate() SRM gate
ROLLOUT_SHARE = 0.90
ZERO_IDS = {
    "00000000-0000-0000-0000-000000000000",
    "00000000000000000000000000000000",
}


def _z_one_sided(alpha):
    """The production Acklam quantile (apex_lift::stats::z_one_sided)."""
    if not (0.0 < alpha < 0.5):
        return math.nan
    p = 1.0 - alpha
    a = (-3.969683028665376e1, 2.209460984245205e2, -2.759285104469687e2,
         1.38357751867269e2, -3.066479806614716e1, 2.506628277459239)
    b = (-5.447609879822406e1, 1.615858368580409e2, -1.556989798598866e2,
         6.680131188771972e1, -1.328068155288572e1)
    c = (-7.784894002430293e-3, -0.3223964580411365, -2.400758277161838,
         -2.549732539343734, 4.374664141464968, 2.938163982698783)
    d = (7.784695709041462e-3, 0.3224671290700398, 2.445134137142996,
         3.754408661907416)
    p_low, p_high = 0.02425, 1.0 - 0.02425
    if p < p_low:
        q = math.sqrt(-2.0 * math.log(p))
        x = (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) / (
            (((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0
        )
    elif p >= p_high:
        q = math.sqrt(-2.0 * math.log(1.0 - p))
        x = -(((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) / (
            (((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0
        )
    else:
        q = p - 0.5
        r = q * q
        x = (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q / (
            ((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1.0
        )
    return x


def _round_half_away(value):
    """Rust's f64::round: halves go away from zero, not to even."""
    if value >= 0.0:
        return int(math.floor(value + 0.5))
    return -int(math.floor(-value + 0.5))


def player_key(row):
    for field in ("userId", "idfv", "idfa"):
        value = row.get(field)
        if value and value not in ZERO_IDS:
            return value
    return None


def _parse_day(day):
    parts = day.split("-")
    if len(parts) != 3:
        return None
    try:
        return (int(parts[0]), int(parts[1]), int(parts[2]))
    except ValueError:
        return None


def _days_in_month(year, month):
    if month in (1, 3, 5, 7, 8, 10, 12):
        return 31
    if month in (4, 6, 9, 11):
        return 30
    if month == 2:
        leap = (year % 4 == 0 and year % 100 != 0) or year % 400 == 0
        return 29 if leap else 28
    return 30


def _next_day(day):
    year, month, day_of_month = day
    if day_of_month < _days_in_month(year, month):
        return (year, month, day_of_month + 1)
    if month < 12:
        return (year, month + 1, 1)
    return (year + 1, 1, 1)


def _require_complete_window(days):
    """Exactly 28 distinct, contiguous measurement days (production's MEASUREMENT_DAYS)."""
    unique = sorted(set(days))
    if len(unique) != 28:
        raise SystemExit(
            f"evidence window covers {len(unique)} distinct days, not the required "
            "28 contiguous days; no fee can be built from it"
        )
    parsed = [_parse_day(day) for day in unique]
    if any(day is None for day in parsed):
        raise SystemExit("evidence window carries a day that is not a YYYY-MM-DD date")
    for previous, current in zip(parsed, parsed[1:]):
        if _next_day(previous) != current:
            raise SystemExit(
                "evidence window days are not contiguous; no fee can be built from it"
            )
    return unique


def load_input(path):
    """Return (evidence, line, record) from one of the accepted shapes.

    `line` is the object carrying `lowerAlpha` (the holdout invoice line),
    `record` the enclosing monthlyBilling record when one was supplied.
    Both are None for a bare evidence object.
    """
    with open(path, encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise SystemExit("input must be a JSON object")
    holdout = data.get("holdout")
    if isinstance(holdout, dict) and isinstance(holdout.get("evidence"), dict):
        return holdout["evidence"], holdout, data
    if isinstance(data.get("evidence"), dict):
        return data["evidence"], data, None
    return data, None, None


def _ceiling_share_bp(unidentified, total):
    """Unidentified revenue share in basis points, rounded up (production)."""
    if total <= 0 or unidentified <= 0:
        return 0
    return min((unidentified * 10_000 + total - 1) // total, 10_000)


def main(argv):
    if len(argv) < 2:
        raise SystemExit(__doc__)
    evidence, line, record = load_input(argv[1])
    # The fee is priced on the treated revenue the signed record carries; a
    # placeholder (or absent) revenue cannot produce a fee.
    treated = None
    if "--treated-revenue" in argv:
        treated = int(argv[argv.index("--treated-revenue") + 1])
    else:
        for source in (record, line):
            if source is None:
                continue
            for field in ("treatedRevenue", "revenueMicros"):
                value = source.get(field)
                if isinstance(value, int) and not isinstance(value, bool) and value > 0:
                    treated = value
                    break
            if treated is not None:
                break
    if treated is None or treated <= 0:
        raise SystemExit(
            "the record carries no positive treatedRevenueMicros; the fee would be "
            "invented. Pass --treated-revenue MICROS only if you knowingly override."
        )
    # The billing bound is the line's registered lowerAlpha — the bound
    # production bills from — never the script's own recomputation (OO).
    bound_alpha = None
    if line is not None and isinstance(line.get("lowerAlpha"), (int, float)):
        bound_alpha = float(line["lowerAlpha"])
    if "--bound-alpha" in argv:
        bound_alpha = float(argv[argv.index("--bound-alpha") + 1])
    exposed_is_test = bool(evidence["exposedIsTest"])
    rows = evidence.get("rows", [])
    declared_window = evidence.get("windowDays") or []
    day_list = declared_window or sorted({
        row.get("day") for row in rows if row.get("day")
    })
    window_days = _require_complete_window(day_list)
    total = sum(int(row["revenueMicros"]) for row in rows)
    unknown = sum(
        int(row["revenueMicros"]) for row in rows if player_key(row) is None
    )
    share_bp = _ceiling_share_bp(unknown, total)
    # The share is a signed diagnostic. It never decides the fee: arm totals
    # include unidentified revenue, and a global ceiling cannot see loss that
    # is concentrated in one arm.
    unidentified = {"exposed": 0, "control": 0}
    for row in rows:
        if player_key(row) is not None:
            continue
        arm = "exposed" if bool(row["test"]) == exposed_is_test else "control"
        unidentified[arm] += int(row["revenueMicros"])
    pretest = evidence.get("pretest", {})
    arms = {}
    for row in rows:
        key = player_key(row)
        if key is None:
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
    if len(exposed_values) < MIN_PLAYERS_PER_ARM or len(control_values) < MIN_PLAYERS_PER_ARM:
        raise SystemExit("fewer than 30 unique players in an arm")

    assigned = evidence.get("assigned")
    has_roster = (
        isinstance(assigned, dict)
        and isinstance(assigned.get("exposed"), int)
        and isinstance(assigned.get("control"), int)
    )
    q = ROLLOUT_SHARE
    p_arm = 1.0 - q
    if has_roster:
        roster_exposed = assigned["exposed"]
        roster_control = assigned["control"]
        total_assigned = roster_exposed + roster_control
        if roster_exposed < len(exposed_values) or roster_control < len(control_values):
            raise SystemExit(
                "the eligible roster counts fewer assigned players than the feed observed"
            )
        if total_assigned <= 0:
            raise SystemExit("the eligible roster is empty")
        # Assignment-count SRM at p = 0.001 over the assigned counts.
        chi_square = (roster_exposed - total_assigned * q) ** 2 / (
            total_assigned * q * p_arm
        )
        if chi_square > CHI_SQUARE_THRESHOLD:
            raise SystemExit(
                "the assignment-count (SRM) chi-square test failed; no fee can be "
                "built from this roster"
            )
        # Assigned counts are the denominators: zero-revenue assigned users
        # the impression feed never shows still count in the means.
        complete = roster_exposed == len(exposed_values) and roster_control == len(control_values)
        denominator_exposed = float(roster_exposed)
        denominator_control = float(roster_control)
        estimator = "roster_eligible_v2"
    else:
        # Planned-share totals: the registered split supplies the
        # denominators, so an observed-user count is not a population count.
        total_players = len(exposed_values) + len(control_values)
        if total_players <= 0:
            raise SystemExit("the evidence has no identified players")
        # Feed-shape diagnostic over labelled rows: a feed that does not look
        # like the registered split cannot be priced here. It is not admitted
        # as proof of the original assignment law.
        chi_square = (len(exposed_values) - total_players * q) ** 2 / (
            total_players * q * p_arm
        )
        if chi_square > CHI_SQUARE_THRESHOLD:
            raise SystemExit(
                "the labelled feed does not look like the registered 90/10 split "
                "(feed-shape diagnostic); no fee can be built from it"
            )
        complete = False
        denominator_exposed = q * float(total_players)
        denominator_control = p_arm * float(total_players)
        estimator = "planned_share_total_v2"

    pairs = [(float(p[3]), float(p[2])) for p in players if p[3] is not None]
    theta = 0.0
    mean_pre = 0.0
    # CUPED is fitted only on a fully observed roster: fitting it on the
    # monetized survivors alone is the selection bias the roster removes.
    # The planned-share estimator does not use a covariate.
    if has_roster and complete and len(pairs) >= 2:
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

    e = adjusted(exposed_values)
    c = adjusted(control_values)
    # Unidentified revenue stays in its own arm; it enters the arm totals and,
    # as one worst-case aggregate, the arm's sum of squares.
    u_exposed = float(unidentified["exposed"])
    u_control = float(unidentified["control"])
    e_sum = sum(e) + u_exposed
    c_sum = sum(c) + u_control
    if e_sum <= 0 or c_sum <= 0:
        raise SystemExit("an arm's revenue is not positive")
    if denominator_exposed <= 1.0 or denominator_control <= 1.0:
        raise SystemExit("an arm's denominator is not above one")
    e_mean = e_sum / denominator_exposed
    c_mean = c_sum / denominator_control
    if c_mean <= 0:
        raise SystemExit("control revenue is not positive")
    e_squares = sum(value * value for value in e) + u_exposed * u_exposed
    c_squares = sum(value * value for value in c) + u_control * u_control
    if has_roster:
        # Sample-variance ratio over assigned users.
        e_var = max((e_squares - denominator_exposed * e_mean * e_mean)
                    / (denominator_exposed - 1.0), 0.0)
        c_var = max((c_squares - denominator_control * c_mean * c_mean)
                    / (denominator_control - 1.0), 0.0)
        variance = (e_var / denominator_exposed / (c_mean ** 2)
                    + (e_mean ** 2) * c_var / denominator_control / (c_mean ** 4))
        se = math.sqrt(variance)
    else:
        # Design-aware Cauchy bound over the two arm totals.
        arm_t = p_arm * e_squares / (e_sum ** 2)
        arm_c = q * c_squares / (c_sum ** 2)
        se = math.sqrt((math.sqrt(arm_t) + math.sqrt(arm_c)) ** 2)
    lift = (e_mean - c_mean) / c_mean
    lower = lift - Z90 * se
    upper = lift + Z90 * se
    lower_alpha = lift - _z_one_sided(INITIAL_ALPHA) * se
    # The rerun is independent only if it also reproduces the signed bound.
    verified = None
    if bound_alpha is not None:
        tolerance = max(1e-9, abs(bound_alpha) * 1e-9)
        if abs(bound_alpha - lower_alpha) > tolerance:
            raise SystemExit(
                f"the signed lowerAlpha {bound_alpha!r} does not match the "
                f"independent recomputation {lower_alpha!r} for {estimator}; "
                "no fee can be built"
            )
        verified = True
    # The fee is sized from the registered bound, floored at zero. There is
    # no second cap: a dimensionless bound and an absolute certificate gain
    # can never be `min`-ed together.
    bound = max(bound_alpha if bound_alpha is not None else lower_alpha, 0.0)
    incremental = treated * (1.0 - 1.0 / (1.0 + bound)) if bound > 0 else 0.0
    fee = _round_half_away(RATE * incremental)
    print(json.dumps({
        "windowDays": len(window_days),
        "windowComplete": True,
        "players": {"exposed": len(exposed_values), "control": len(control_values)},
        "estimator": estimator,
        "assigned": ({"exposed": assigned["exposed"], "control": assigned["control"]}
                     if has_roster else None),
        "coverage": {
            "unidentifiedMicros": unknown,
            "unidentifiedExposedMicros": unidentified["exposed"],
            "unidentifiedControlMicros": unidentified["control"],
            "unidentifiedShareBp": share_bp,
        },
        "cupedTheta": theta,
        "lift": lift,
        "standardError": se,
        "lower90": lower,
        "upper90": upper,
        "lowerAlpha": lower_alpha,
        "lowerAlphaVerified": verified,
        "boundAlpha": bound_alpha,
        "billingLift": bound,
        "treatedRevenueMicros": treated,
        "feeMicros": fee,
    }, indent=2))


if __name__ == "__main__":
    main(sys.argv)
