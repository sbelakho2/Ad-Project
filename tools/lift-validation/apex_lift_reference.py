#!/usr/bin/env python3
"""A dependency-free reference implementation of the Apex lift engine.

The kit mirrors the production Rust modules rather than only the mean:

* the daily log-ratio estimator and its one-sided Student-t bounds;
* the per-player Horvitz-Thompson estimator and per-player CUPED;
* calendar-position burn-in, two-pull finality, whole-week windows and
  incident runs;
* the daily guardrails (fast/confirmed harm, fill, split, reporting gap,
  publisher edits and platform incidents);
* the interim 99% look, the one pre-registered extension, the bootstrap
  cross-check and the keep/rollback/void/pending decision;
* the randomized two-day switchback plan and its block analysis;
* the re-run kit's CSV path, including the three burn-in days.

The incomplete beta continued fraction matches the Rust formulation exactly
(the ``a + b + m`` term, not a bare ``b + m``), so the t quantiles cannot
drift on a different continued fraction.

This file is the Python half of the twin harness. It deliberately exposes
``run_case`` with the same JSON case/output shape as
``rust/crates/apex-lift/examples/apex-lift-twin.rs``.
"""

from __future__ import annotations

import json
import math
import statistics
import sys

# ---------------------------------------------------------------------------
# Numeric constants and incomplete beta / Student-t machinery.
# ---------------------------------------------------------------------------

Z90_ONE_SIDED = 1.2815515655446004
Z99_ONE_SIDED = 2.3263478740408408
INITIAL_ALPHA = 0.0225
INTERIM_ALPHA = 0.005
ESTIMATOR_ID = "planned_share_total_v2"

_NORMAL = statistics.NormalDist()


def _erf(x: float) -> float:
    sign = -1.0 if x < 0 else 1.0
    x = abs(x)
    t = 1.0 / (1.0 + 0.3275911 * x)
    poly = (
        (((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736)
        * t
        + 0.254829592
    ) * t
    return sign * (1.0 - poly * math.exp(-x * x))


def z_one_sided(alpha: float) -> float:
    """The production Acklam quantile, refined once against the same CDF.

    The exact-binomial oracle is the independent check of the estimator; this
    function only has to be the same number as the Rust twin to the last bit
    the JSON comparison sees.
    """
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
CHI_SQUARE_DF1_P001 = 10.8276
BOOTSTRAP_AGREEMENT = 0.01
ALLOWED_WINDOW_DAYS = (7, 14, 21, 28)
FINALITY_TOLERANCE = 0.005
SECOND_PULL_MIN_SECS = 86_400
RERUN_BURN_IN_DAYS = 3
AA_SPLIT_Z = 3.3

_LANCZOS = (
    0.99999999999980993,
    676.5203681218851,
    -1259.1392167224028,
    771.32342877765313,
    -176.61502916214059,
    12.507343278686905,
    -0.13857109526572012,
    9.9843695780195716e-6,
    1.5056327351493116e-7,
)


def lgamma(x: float) -> float:
    """The Rust engine's Lanczos log-gamma, evaluated in the same order."""
    if x < 0.5:
        return math.log(math.pi / abs(math.sin(math.pi * x))) - lgamma(1.0 - x)
    x = x - 1.0
    a = _LANCZOS[0]
    t = x + 7.5
    for index, coefficient in enumerate(_LANCZOS[1:], start=1):
        a += coefficient / (x + index)
    return 0.5 * math.log(2.0 * math.pi) + (x + 0.5) * math.log(t) - t + math.log(a)


def betacf(a: float, b: float, x: float) -> float:
    """Continued fraction for the incomplete beta, identical to Rust."""
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
        # Production uses qab + m, never b + m. This line is the historical
        # mismatch the twin exists to catch.
        aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2))
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


def betai(a: float, b: float, x: float) -> float:
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


def t_cdf(t: float, df: int) -> float:
    if t == 0.0:
        return 0.5
    x = df / (df + t * t)
    beta = betai(df / 2.0, 0.5, x) / 2.0
    return 1.0 - beta if t > 0.0 else beta


def t_quantile_alpha(df: int, alpha: float) -> float:
    if df == 0:
        return math.inf
    target = 1.0 - alpha
    low, high = 0.0, 128.0
    for _ in range(96):
        mid = (low + high) / 2.0
        if t_cdf(mid, df) < target:
            low = mid
        else:
            high = mid
    return (low + high) / 2.0


def t_quantile(df: int, level_99: bool = False) -> float:
    return t_quantile_alpha(df, 0.01 if level_99 else 0.10)


def _sum(values) -> float:
    """Left-to-right float summation, matching Rust's iterator sum order."""
    total = 0.0
    for value in values:
        total += value
    return total


def _round_half_away(value: float) -> int:
    """Rust's f64::round for the non-negative values the fee path uses."""
    return int(math.floor(value + 0.5)) if value >= 0.0 else -int(math.floor(-value + 0.5))


# ---------------------------------------------------------------------------
# Deterministic SplitMix64, so the bootstrap replays exactly.
# ---------------------------------------------------------------------------


class SplitMix64:
    _MASK = (1 << 64) - 1

    def __init__(self, seed: int) -> None:
        self.state = seed & self._MASK

    def next_u64(self) -> int:
        self.state = (self.state + 0x9E3779B97F4A7C15) & self._MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & self._MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & self._MASK
        return (z ^ (z >> 31)) & self._MASK

    def below(self, bound: int) -> int:
        return self.next_u64() % bound


def bootstrap_lower_bound(days, block_len: int, iterations: int, seed: int):
    """The engine's block bootstrap tenth percentile of the ratio of totals."""
    if not days or block_len == 0 or iterations < 100:
        return None
    if any(
        not math.isfinite(test) or not math.isfinite(control) or test <= 0.0 or control <= 0.0
        for test, control in days
    ):
        return None
    block_len = min(block_len, len(days))
    block_count = (len(days) + block_len - 1) // block_len
    rng = SplitMix64(seed)
    draws = []
    for _ in range(iterations):
        test_total = 0.0
        control_total = 0.0
        for _ in range(block_count):
            start = rng.below(len(days))
            for offset in range(block_len):
                test, control = days[(start + offset) % len(days)]
                test_total += test
                control_total += control
        draws.append(test_total / control_total - 1.0)
    draws.sort()
    index = _round_half_away((iterations - 1) * 0.10)
    return draws[index]


# ---------------------------------------------------------------------------
# Estimators.
# ---------------------------------------------------------------------------


def estimate(ratios):
    if len(ratios) < 2 or any(not math.isfinite(value) for value in ratios):
        return None
    count = float(len(ratios))
    mean = _sum(ratios) / count
    variance = _sum((value - mean) ** 2 for value in ratios) / (count - 1.0)
    sd = math.sqrt(variance)
    se = sd / math.sqrt(count)
    t90 = t_quantile(len(ratios) - 1, False)
    t99 = t_quantile(len(ratios) - 1, True)
    lower_90 = math.exp(mean - t90 * se) - 1.0
    lower_99 = math.exp(mean - t99 * se) - 1.0
    upper_90 = math.exp(mean + t90 * se) - 1.0
    upper_99 = math.exp(mean + t99 * se) - 1.0
    lower_alpha = math.exp(mean - t_quantile_alpha(len(ratios) - 1, INITIAL_ALPHA) * se) - 1.0
    lower_interim = math.exp(mean - t_quantile_alpha(len(ratios) - 1, INTERIM_ALPHA) * se) - 1.0
    lift = math.exp(mean) - 1.0
    if not (
        math.isfinite(mean)
        and math.isfinite(sd)
        and math.isfinite(lift)
        and math.isfinite(lower_90)
        and math.isfinite(lower_99)
        and math.isfinite(upper_90)
        and math.isfinite(upper_99)
        and math.isfinite(lower_alpha)
        and math.isfinite(lower_interim)
    ):
        return None
    return {
        "days": len(ratios),
        "meanLog": mean,
        "lift": lift,
        "sd": sd,
        "lower90": lower_90,
        "lower99": lower_99,
        "upper90": upper_90,
        "upper99": upper_99,
        "lowerAlpha": lower_alpha,
        "lowerInterim": lower_interim,
    }


def horvitz_thompson_estimate(control, test, test_probability: float):
    if not control or not test:
        return None
    if not math.isfinite(test_probability) or not (0.0 < test_probability < 1.0):
        return None
    if any(not math.isfinite(value) for value in list(control) + list(test)):
        return None
    q = test_probability
    p = 1.0 - q
    total_players = float(len(control) + len(test))
    if total_players <= 0.0:
        return None
    test_total = _sum(test)
    control_total = _sum(control)
    test_mean = test_total / q / total_players
    control_mean = control_total / p / total_players
    if test_mean <= 0.0 or control_mean <= 0.0:
        return None
    test_squares = _sum(value * value for value in test)
    control_squares = _sum(value * value for value in control)
    test_arm = p * test_squares / (test_total * test_total)
    control_arm = q * control_squares / (control_total * control_total)
    mean_log = math.log(test_mean / control_mean)
    variance = (math.sqrt(test_arm) + math.sqrt(control_arm)) ** 2
    if not math.isfinite(mean_log) or not math.isfinite(variance) or variance < 0.0:
        return None
    se = math.sqrt(variance)
    lift = math.exp(mean_log) - 1.0
    lower_90 = math.exp(mean_log - Z90_ONE_SIDED * se) - 1.0
    lower_99 = math.exp(mean_log - Z99_ONE_SIDED * se) - 1.0
    upper_90 = math.exp(mean_log + Z90_ONE_SIDED * se) - 1.0
    upper_99 = math.exp(mean_log + Z99_ONE_SIDED * se) - 1.0
    lower_alpha = math.exp(mean_log - z_one_sided(INITIAL_ALPHA) * se) - 1.0
    lower_interim = math.exp(mean_log - z_one_sided(INTERIM_ALPHA) * se) - 1.0
    if not all(
        math.isfinite(value)
        for value in (
            lift,
            lower_90,
            lower_99,
            upper_90,
            upper_99,
            lower_alpha,
            lower_interim,
        )
    ):
        return None
    return {
        "estimate": {
            "days": min(len(control), len(test)),
            "meanLog": mean_log,
            "lift": lift,
            "sd": se,
            "lower90": lower_90,
            "lower99": lower_99,
            "upper90": upper_90,
            "upper99": upper_99,
            "lowerAlpha": lower_alpha,
            "lowerInterim": lower_interim,
        },
        "variance": variance,
        "testProbability": q,
        "controlProbability": p,
        "testPlayers": len(test),
        "controlPlayers": len(control),
        "testMean": test_mean,
        "controlMean": control_mean,
    }


# ---------------------------------------------------------------------------
# Windows: burn-in, finality, incident exclusion, whole weeks.
# ---------------------------------------------------------------------------


def _problem(kind: str, day: str) -> dict:
    return {"kind": kind, "day": day}


def _scaled_within_tolerance(a: int, b: int) -> bool:
    base = max(abs(a), 1)
    return abs(a - b) * 1_000 <= base * 5


def _relative_within_tolerance(a: float, b: float) -> bool:
    if not math.isfinite(a) or not math.isfinite(b):
        return False
    base = max(abs(a), 1e-12)
    return abs(a - b) / base <= FINALITY_TOLERANCE


def day_is_final(day: dict) -> bool:
    first_at = day.get("firstObservedAtUnixS")
    second_at = day.get("secondObservedAtUnixS")
    if first_at is None or second_at is None:
        return False
    first_source = day.get("firstSourceSha256")
    second_source = day.get("secondSourceSha256")
    if first_source is None or second_source is None:
        return False
    if second_at - first_at < SECOND_PULL_MIN_SECS:
        return False
    if first_at == second_at and first_source == second_source:
        return False
    second_revenue = day.get("secondPullRevenue")
    if second_revenue is None:
        return False
    if not _scaled_within_tolerance(day["revenue"], second_revenue):
        return False
    first_users = day.get("users")
    second_users = day.get("secondPullUsers")
    if first_users is None and second_users is None:
        pass
    elif (
        first_users is not None
        and second_users is not None
        and _scaled_within_tolerance(first_users, second_users)
    ):
        pass
    else:
        return False
    if first_users is None and second_users is None:
        pass
    elif first_users is not None and second_users is not None:
        if first_users > 0 and second_users > 0:
            first_value = day["revenue"] / first_users
            second_value = second_revenue / second_users
            if not _relative_within_tolerance(first_value, second_value):
                return False
        elif first_users == 0 and second_users == 0:
            pass
        else:
            return False
    else:
        return False
    first_fill = day.get("fill")
    second_fill = day.get("secondPullFill")
    if first_fill is None and second_fill is None:
        pass
    elif (
        first_fill is not None
        and second_fill is not None
        and abs(first_fill - second_fill) <= FINALITY_TOLERANCE + 1e-12
    ):
        pass
    else:
        return False
    return True


def arm_value(day: dict, allocated_share: float):
    users = day.get("users")
    if users is not None:
        if users == 0:
            return None
        return day["revenue"] / users
    if not (0.0 < allocated_share < 1.0) or allocated_share == 0.0:
        return None
    return day["revenue"] / allocated_share


def incident_runs(control, test):
    out = []
    run = 0
    for index in range(len(control)):
        if (
            index > 0
            and control[index - 1]["revenue"] > 0
            and test[index - 1]["revenue"] > 0
            and control[index]["revenue"] < 0.5 * control[index - 1]["revenue"]
            and test[index]["revenue"] < 0.5 * test[index - 1]["revenue"]
        ):
            run += 1
            out.append((control[index]["day"], run))
        else:
            run = 0
    return out


def _parse_day(day: str):
    parts = day.split("-")
    if len(parts) != 3:
        return None
    try:
        year = int(parts[0])
        month = int(parts[1])
        day_of_month = int(parts[2])
    except ValueError:
        return None
    if not (1 <= month <= 12) or not (1 <= day_of_month <= 31):
        return None
    return (year, month, day_of_month)


def _days_in_month(year: int, month: int) -> int:
    if month in (1, 3, 5, 7, 8, 10, 12):
        return 31
    if month in (4, 6, 9, 11):
        return 30
    if month == 2:
        leap = (year % 4 == 0 and year % 100 != 0) or year % 400 == 0
        return 29 if leap else 28
    return 30


def _next_date(year: int, month: int, day: int):
    last = _days_in_month(year, month)
    if day < last:
        return (year, month, day + 1)
    if month < 12:
        return (year, month + 1, 1)
    return (year + 1, 1, 1)


def _is_next_day(current: str, following: str) -> bool:
    left = _parse_day(current)
    right = _parse_day(following)
    if left is None or right is None:
        return False
    return _next_date(*left) == right


def build_window(control, test, window_days: int, test_allocated_share: float, burn_in_days: int):
    problems = []
    if window_days not in ALLOWED_WINDOW_DAYS:
        return {
            "days": [],
            "problems": [_problem("schema", f"invalid window length {window_days}")],
            "complete": False,
            "incomplete": False,
            "fatal": True,
        }
    if not control or len(control) != len(test):
        return {
            "days": [],
            "problems": [_problem("missingDay", "pull length mismatch")],
            "complete": False,
            "incomplete": True,
            "fatal": False,
        }

    run_by_day = incident_runs(control, test)
    index_by_day = {day["day"]: index for index, day in enumerate(control)}
    runs = [0] * len(control)
    for day, length in run_by_day:
        index = index_by_day.get(day)
        if index is not None:
            runs[index] = length

    seen = set()
    previous_day = None
    analyzed = []
    for index in range(len(control)):
        control_day = control[index]
        test_day = test[index]
        if control_day["day"] != test_day["day"]:
            problems.append(_problem("missingDay", control_day["day"]))
            continue
        if control_day["day"] in seen:
            problems.append(_problem("sequence", control_day["day"]))
            continue
        seen.add(control_day["day"])
        if previous_day is not None and not _is_next_day(previous_day, control_day["day"]):
            problems.append(_problem("sequence", control_day["day"]))
            previous_day = control_day["day"]
            continue
        previous_day = control_day["day"]

        if index < burn_in_days:
            problems.append(_problem("burnIn", control_day["day"]))
            continue
        if runs[index] >= 1:
            problems.append(_problem("incident", control_day["day"]))
            if runs[index] >= 2:
                return {
                    "days": analyzed,
                    "problems": problems,
                    "complete": False,
                    "incomplete": False,
                    "fatal": False,
                }
            continue
        if not day_is_final(control_day) or not day_is_final(test_day):
            problems.append(_problem("notFinal", control_day["day"]))
            continue
        control_value = arm_value(control_day, 1.0 - test_allocated_share)
        if control_value is None:
            problems.append(_problem("missingDay", control_day["day"]))
            continue
        test_value = arm_value(test_day, test_allocated_share)
        if test_value is None:
            problems.append(_problem("missingDay", control_day["day"]))
            continue
        if control_value <= 0.0 or test_value <= 0.0:
            problems.append(_problem("missingDay", control_day["day"]))
            continue
        first_fill = control_day.get("fill")
        second_fill = test_day.get("fill")
        fill_delta = (
            second_fill - first_fill
            if first_fill is not None and second_fill is not None
            else None
        )
        analyzed.append(
            {
                "day": control_day["day"],
                "testValue": test_value,
                "controlValue": control_value,
                "logRatio": math.log(test_value / control_value),
                "fillDelta": fill_delta,
                "calendarIndex": index,
                "testUsers": test_day.get("users"),
                "controlUsers": control_day.get("users"),
            }
        )

    whole_weeks = (len(analyzed) // 7) * 7
    keep = min(whole_weeks, window_days)
    analyzed = analyzed[:keep]
    keep_index = analyzed[-1]["calendarIndex"] if analyzed else None

    def within_window(problem):
        kind = problem["kind"]
        if kind in ("burnIn", "incident"):
            return False
        if kind in ("sequence", "schema"):
            return True
        index = index_by_day.get(problem["day"])
        if index is None:
            return True
        return keep_index is None or index <= keep_index

    fatal = any(problem["kind"] in ("sequence", "schema") for problem in problems)
    incomplete = any(
        problem["kind"] in ("missingDay", "notFinal") and within_window(problem)
        for problem in problems
    )
    complete = keep == window_days and not fatal and not incomplete
    return {
        "days": analyzed,
        "problems": problems,
        "complete": complete,
        "incomplete": incomplete,
        "fatal": fatal,
    }


# ---------------------------------------------------------------------------
# Guardrails.
# ---------------------------------------------------------------------------


def split_chi_square(test_players: int, control_players: int, planned_share: float):
    if not math.isfinite(planned_share) or not (0.0 < planned_share < 1.0):
        return None
    total = float(test_players + control_players)
    if total <= 0.0:
        return None
    expected_test = total * planned_share
    expected_control = total * (1.0 - planned_share)
    if expected_test <= 0.0 or expected_control <= 0.0:
        return None
    chi_square = (test_players - expected_test) ** 2 / expected_test + (
        control_players - expected_control
    ) ** 2 / expected_control
    return chi_square if math.isfinite(chi_square) else None


def _guardrail(kind: str, day: str, value=None) -> dict:
    return {"kind": kind, "day": day, "value": value}


def _voids(event: dict) -> bool:
    if event["kind"] in ("publisherEdit", "splitMismatch", "platformIncident"):
        return True
    if event["kind"] == "reportingGap":
        return event["value"] is not None and event["value"] >= 3
    return False


def _rolls_back(event: dict) -> bool:
    return event["kind"] in (
        "fastHarm",
        "confirmedHarm",
        "fillRewardedInterstitial",
        "fillBanner",
    )


def _pauses(event: dict) -> bool:
    return event["kind"] == "reportingGap" and event["value"] is not None and event["value"] < 3


def evaluate_guardrails(
    prereg: dict,
    days,
    arm_counts,
    unique_players,
    fill_format: str,
    settings_changed,
    stale_days: int,
    incident_runs_input,
):
    events = []
    harm_point = prereg["harmPoint"]

    for cutoff in range(2, len(days) + 1):
        ratios = [day["logRatio"] for day in days[:cutoff]]
        current = estimate(ratios)
        if current is None:
            continue
        day = days[cutoff - 1]["day"]
        if current["lift"] < harm_point:
            events.append(_guardrail("fastHarm", day, current["lift"]))
        confirmed = (cutoff == 7 and current["upper99"] < 0.0) or (
            cutoff == 14 and current["upper90"] < 0.0
        )
        if confirmed:
            events.append(
                _guardrail(
                    "confirmedHarm",
                    day,
                    current["upper99"] if cutoff == 7 else current["upper90"],
                )
            )

    if fill_format in ("rewarded", "interstitial"):
        threshold = prereg["fillRewarded"]
    else:
        threshold = prereg["fillBanner"]
    streak = 0
    previous_index = None
    for day in days:
        consecutive = previous_index is not None and day["calendarIndex"] == previous_index + 1
        previous_index = day["calendarIndex"]
        delta = day["fillDelta"]
        if delta is not None and delta < -threshold:
            streak = streak + 1 if consecutive else 1
            if streak >= 2:
                kind = (
                    "fillBanner"
                    if fill_format == "banner"
                    else "fillRewardedInterstitial"
                )
                events.append(_guardrail(kind, day["day"], delta))
        else:
            streak = 0

    for day in settings_changed:
        events.append(_guardrail("publisherEdit", day))

    if unique_players is not None:
        split = unique_players
    else:
        test_sum = sum(pair[0] for pair in arm_counts)
        control_sum = sum(pair[1] for pair in arm_counts)
        split = (test_sum, control_sum) if test_sum + control_sum > 0 else None
    if split is not None:
        chi_square = split_chi_square(split[0], split[1], prereg["testShare"])
        if chi_square is not None and chi_square > prereg["splitChiSquare"]:
            day = days[-1]["day"] if days else ""
            events.append(_guardrail("splitMismatch", day, chi_square))

    if stale_days > 0:
        day = days[-1]["day"] if days else ""
        events.append(_guardrail("reportingGap", day, float(stale_days)))

    for day, run in incident_runs_input:
        if run >= 2:
            events.append(_guardrail("platformIncident", day))

    return events


def aa_split_void(ratios) -> bool:
    current = estimate(ratios)
    if current is None:
        return True
    se = current["sd"] / math.sqrt(current["days"])
    return not math.isfinite(se) or abs(current["meanLog"]) > AA_SPLIT_Z * se


# ---------------------------------------------------------------------------
# Decision.
# ---------------------------------------------------------------------------


def _prereg_validate(prereg: dict):
    if prereg["testShare"] not in (0.5, 0.25, 0.10):
        return f"test_share {prereg['testShare']} must be one of 0.5, 0.25, 0.10"
    if prereg["windowDays"] not in ALLOWED_WINDOW_DAYS:
        return f"window_days {prereg['windowDays']} must be one of 7, 14, 21, 28"
    if not (1500 <= prereg["rateBp"] <= 2000):
        return f"rate_bp {prereg['rateBp']} must be in the contracted 1500..=2000 band"
    if not str(prereg["codeVersionHash"]).strip():
        return "code_version_hash must not be empty"
    if not str(prereg["beforeValue"]).strip():
        return "before_value must carry the prior platform value"
    min_lift = prereg["minPointLift"]
    if not math.isfinite(min_lift) or not (0.0 <= min_lift <= 1.0):
        return f"min_point_lift {min_lift} must be a finite number in [0, 1]"
    if not (-0.05 <= prereg["harmPoint"] <= 0.0):
        return f"harm_point {prereg['harmPoint']} must be in (-0.05, 0]"
    if not (0.0 <= prereg["fillRewarded"] <= 0.02):
        return f"fill_rewarded {prereg['fillRewarded']} must be in [0, 0.02]"
    if not (0.0 <= prereg["fillBanner"] <= 0.05):
        return f"fill_banner {prereg['fillBanner']} must be in [0, 0.05]"
    if not (0.0 < prereg["splitChiSquare"] <= CHI_SQUARE_DF1_P001):
        return f"split_chi_square {prereg['splitChiSquare']} must be in (0, 10.8276]"
    return None


def _decision(verdict: str, reason: str, **fields) -> dict:
    return {
        "verdict": verdict,
        "lambda": fields.get("lam"),
        "usedInterim99": fields.get("used_interim_99", False),
        "bootstrapLower": fields.get("bootstrap_lower"),
        "reason": reason,
    }


def _pre_decision(prereg: dict, window: dict, events):
    problem = _prereg_validate(prereg)
    if problem is not None:
        return _decision("void", f"invalid pre-registration: {problem}")
    if prereg["testShare"] == 0.10:
        return _decision("rollback", "10% splits are exploratory and never billed")
    fatal = next(
        (problem for problem in window["problems"] if problem["kind"] in ("sequence", "schema")),
        None,
    )
    if fatal is not None:
        return _decision("void", f"fatal window problem: {fatal['kind']}")
    void_event = next((event for event in events if _voids(event)), None)
    if void_event is not None:
        return _decision("void", void_event["kind"])
    rollback_event = next((event for event in events if _rolls_back(event)), None)
    if rollback_event is not None:
        return _decision("rollback", rollback_event["kind"])
    pause_event = next((event for event in events if _pauses(event)), None)
    if pause_event is not None:
        return _decision("pending", pause_event["kind"])
    return None


def _bootstrap_for(window: dict, bootstrap: dict):
    totals = [(day["testValue"], day["controlValue"]) for day in window["days"]]
    return bootstrap_lower_bound(
        totals,
        bootstrap["block_len"],
        bootstrap["iterations"],
        bootstrap["seed"],
    )


def _decide_completed(prereg, window, current, cross_check, bootstrap, observed_scope, observed_change, at_final_look, extension_used):
    interim_days = (prereg["windowDays"] // 2 // 7) * 7
    if (
        prereg["interimLook"]
        and interim_days > 0
        and len(window["days"]) == interim_days
        and not window["incomplete"]
        and current["lowerInterim"] > 0.0
        and current["lift"] >= prereg["minPointLift"]
    ):
        boot = _bootstrap_for(window, bootstrap) if cross_check else None
        if cross_check and boot is None:
            return _decision("pending", "the bootstrap cross-check could not run")
        if boot is None or (
            boot > 0.0 and abs(boot - current["lowerInterim"]) <= BOOTSTRAP_AGREEMENT + 1e-9
        ):
            return _decision(
                "keep",
                "",
                lam=current["lowerInterim"],
                used_interim_99=True,
                bootstrap_lower=boot if boot is not None else current["lowerInterim"],
            )

    if (
        window["complete"]
        and at_final_look
        and current["lift"] > 0.0
        and current["lowerAlpha"] <= 0.0
        and prereg["extensionAllowed"]
        and not extension_used
        and not window["incomplete"]
    ):
        return _decision("extend", "positive estimate, bound not above zero: one extra week")

    if not window["complete"] or not at_final_look:
        return _decision(
            "pending",
            f"{len(window['days'])} of {prereg['windowDays']} analysis days are final",
        )

    failed = []
    if window["incomplete"]:
        failed.append("the window has non-final, missing or non-consecutive days")
    if observed_scope != prereg["scope"]:
        failed.append("the scope changed since pre-registration")
    if observed_change != prereg["changeDescription"]:
        failed.append("the change values changed since pre-registration")
    if current["lift"] < prereg["minPointLift"]:
        failed.append("point lift is below the registered minimum")
    if current["lowerAlpha"] <= 0.0:
        failed.append("the registered 97.75% lower bound is not above zero")

    if cross_check:
        boot = _bootstrap_for(window, bootstrap)
        if boot is None:
            return _decision("pending", "the bootstrap cross-check could not run")
        if abs(boot - current["lower90"]) > BOOTSTRAP_AGREEMENT + 1e-9:
            if (
                prereg["extensionAllowed"]
                and not extension_used
                and current["lift"] > 0.0
                and not window["incomplete"]
            ):
                return _decision(
                    "extend",
                    "the bootstrap and analytic bounds differ by more than one point",
                )
            failed.append("the bootstrap cross-check disagrees by more than one point")
    else:
        boot = current["lower90"]
    if failed:
        return _decision("rollback", "; ".join(failed))
    return _decision(
        "keep",
        "",
        lam=max(current["lowerAlpha"], 0.0),
        used_interim_99=False,
        bootstrap_lower=boot,
    )


def decide(prereg, window, events, observed_scope, observed_change, at_final_look, extension_used, bootstrap):
    pre = _pre_decision(prereg, window, events)
    if pre is not None:
        return pre
    ratios = [day["logRatio"] for day in window["days"]]
    current = estimate(ratios)
    if current is None:
        return _decision("pending", "fewer than two analysis days have accumulated")
    return _decide_completed(
        prereg,
        window,
        current,
        True,
        bootstrap,
        observed_scope,
        observed_change,
        at_final_look,
        extension_used,
    )


def decide_with_estimate(prereg, window, events, current, observed_scope, observed_change, at_final_look, extension_used, bootstrap):
    pre = _pre_decision(prereg, window, events)
    if pre is not None:
        return pre
    return _decide_completed(
        prereg,
        window,
        current,
        False,
        bootstrap,
        observed_scope,
        observed_change,
        at_final_look,
        extension_used,
    )


# ---------------------------------------------------------------------------
# Per-player window, CUPED and the Horvitz-Thompson estimate.
# ---------------------------------------------------------------------------


def _valid_id(value):
    return (
        value is not None
        and value != ""
        and value != "00000000-0000-0000-0000-000000000000"
        and value != "00000000000000000000000000000000"
    )


def player_key(row: dict):
    for field in ("userId", "idfv", "idfa"):
        value = row.get(field)
        if _valid_id(value):
            return value
    return None


def _cuped_mean(values) -> float:
    if not values:
        return 0.0
    return _sum(values) / len(values)


def _cuped_theta(arms, pretest_by_player):
    pairs = [
        (pretest_by_player[player], float(revenue))
        for player, (_, revenue) in arms.items()
        if player in pretest_by_player
    ]
    if len(pairs) < 2:
        return 0.0
    count = float(len(pairs))
    mean_pre = _sum(pre for pre, _ in pairs) / count
    mean_post = _sum(post for _, post in pairs) / count
    covariance = (
        _sum((pre - mean_pre) * (post - mean_post) for pre, post in pairs) / (count - 1.0)
    )
    variance = _sum((pre - mean_pre) ** 2 for pre, _ in pairs) / (count - 1.0)
    if variance <= 0.0 or not math.isfinite(covariance) or not math.isfinite(variance):
        return 0.0
    return covariance / variance


def player_window(rows, pretest):
    arms = {}
    unidentified = 0
    for row in rows:
        key = player_key(row)
        if key is None:
            unidentified += row["revenueMicros"]
            continue
        existing = arms.get(key)
        if existing is not None:
            if existing[0] != row["test"]:
                raise ValueError(f"player '{key}' appears in both the control and test arms")
            arms[key] = (existing[0], existing[1] + row["revenueMicros"])
        else:
            arms[key] = (row["test"], row["revenueMicros"])

    ordered = sorted(arms.items())
    pretest_by_player = {}
    for player, (_, _) in ordered:
        if pretest is not None and player in pretest:
            pretest_by_player[player] = float(pretest[player])
    theta = _cuped_theta(dict(ordered), pretest_by_player) if pretest is not None else 0.0
    common_pre = _cuped_mean(list(pretest_by_player.values()))

    raw_test = []
    raw_control = []
    cuped_test = []
    cuped_control = []
    for player, (is_test, revenue) in ordered:
        post = float(revenue)
        if player in pretest_by_player and theta != 0.0:
            adjusted = post - theta * (pretest_by_player[player] - common_pre)
        else:
            adjusted = post
        if is_test:
            raw_test.append(post)
            cuped_test.append(adjusted)
        else:
            raw_control.append(post)
            cuped_control.append(adjusted)

    return {
        "test": raw_test,
        "control": raw_control,
        "cupedTest": cuped_test,
        "cupedControl": cuped_control,
        "testPlayers": len(raw_test),
        "controlPlayers": len(raw_control),
        "unidentifiedMicros": unidentified,
        "cupedTheta": theta,
    }


def estimate_window(window: dict, test_probability: float, cuped: bool):
    if cuped:
        return horvitz_thompson_estimate(
            window["cupedControl"], window["cupedTest"], test_probability
        )
    return horvitz_thompson_estimate(
        window["control"], window["test"], test_probability
    )


# ---------------------------------------------------------------------------
# Switchback plan and block analysis.
# ---------------------------------------------------------------------------


def _has_triple(blocks) -> bool:
    for index in range(len(blocks) - 2):
        if blocks[index] == blocks[index + 1] == blocks[index + 2]:
            return True
    return False


def switchback_plan(block_count: int, seed: int):
    if block_count != 14:
        return None
    feasible = []
    for mask in range(1 << block_count):
        if bin(mask).count("1") != block_count // 2:
            continue
        blocks = [bool(mask & (1 << k)) for k in range(block_count)]
        if _has_triple(blocks):
            continue
        per_weekday = [0] * 7
        for k, is_test in enumerate(blocks):
            if not is_test:
                continue
            per_weekday[(2 * k) % 7] += 1
            per_weekday[(2 * k + 1) % 7] += 1
        if all(count == 2 for count in per_weekday):
            feasible.append(blocks)
    if not feasible:
        return None
    return {
        "blocks": feasible[seed % len(feasible)],
        "blocksPerArm": block_count // 2,
    }


def _difference_estimate(test_logs, control_logs):
    if len(test_logs) < 2 or len(control_logs) < 2:
        return None

    def mean(values):
        return _sum(values) / len(values)

    def variance(values, average):
        return _sum((value - average) ** 2 for value in values) / (len(values) - 1.0)

    test_mean = mean(test_logs)
    control_mean = mean(control_logs)
    test_variance = variance(test_logs, test_mean)
    control_variance = variance(control_logs, control_mean)
    test_term = test_variance / len(test_logs)
    control_term = control_variance / len(control_logs)
    se = math.sqrt(test_term + control_term)
    if not math.isfinite(se):
        return None
    if test_term + control_term > 0.0:
        df = int(
            math.floor(
                (test_term + control_term) ** 2
                / (
                    test_term**2 / (len(test_logs) - 1.0)
                    + control_term**2 / (len(control_logs) - 1.0)
                )
            )
        )
    else:
        df = min(len(test_logs), len(control_logs)) - 1
    df = max(df, 1)
    mean_log = test_mean - control_mean
    t90 = t_quantile(df, False)
    t99 = t_quantile(df, True)
    return {
        "days": len(test_logs) + len(control_logs),
        "meanLog": mean_log,
        "lift": math.exp(mean_log) - 1.0,
        "sd": se,
        "lower90": math.exp(mean_log - t90 * se) - 1.0,
        "lower99": math.exp(mean_log - t99 * se) - 1.0,
        "upper90": math.exp(mean_log + t90 * se) - 1.0,
        "upper99": math.exp(mean_log + t99 * se) - 1.0,
        "lowerAlpha": math.exp(mean_log - t_quantile_alpha(df, INITIAL_ALPHA) * se) - 1.0,
        "lowerInterim": math.exp(mean_log - t_quantile_alpha(df, INTERIM_ALPHA) * se) - 1.0,
    }


def switchback_out(input_data: dict) -> dict:
    planned = switchback_plan(input_data["blockCount"], input_data["seed"])
    if planned is None:
        return {
            "plan": [],
            "blocksPerArm": 0,
            "billable": False,
            "lambda": None,
            "estimate": None,
            "reason": "no weekday-balanced plan for this block count",
        }
    values = input_data.get("values") or []
    if not values:
        return {
            "plan": planned["blocks"],
            "blocksPerArm": planned["blocksPerArm"],
            "billable": False,
            "lambda": None,
            "estimate": None,
            "reason": "no observations",
        }
    used = input_data.get("usedBlocksPerArm", 0)
    blocks = planned["blocks"]
    if used < 6:
        return _switchback_not_billable(planned, f"{used} blocks per arm: fewer than the required six")
    if planned["blocksPerArm"] < 6:
        return _switchback_not_billable(
            planned, "the plan itself has fewer than six blocks per arm"
        )
    planned_test = sum(1 for is_test in blocks if is_test)
    if (
        planned_test != planned["blocksPerArm"]
        or len(blocks) - planned_test != planned["blocksPerArm"]
    ):
        return _switchback_not_billable(
            planned, "the plan's block labels disagree with blocks_per_arm"
        )
    if len(values) != len(blocks) or used != planned["blocksPerArm"]:
        return _switchback_not_billable(
            planned,
            f"{len(values)} block values for {len(blocks)} planned blocks "
            f"({used} of {planned['blocksPerArm']} blocks per arm)",
        )
    if any(not math.isfinite(value) or value <= 0.0 for value in values):
        return _switchback_not_billable(planned, "a block value is non-positive or non-finite")
    test_logs = [math.log(value) for is_test, value in zip(blocks, values) if is_test]
    control_logs = [math.log(value) for is_test, value in zip(blocks, values) if not is_test]
    current = _difference_estimate(test_logs, control_logs)
    if current is None:
        return _switchback_not_billable(planned, "fewer than two usable blocks per arm")
    if current["lowerAlpha"] <= 0.0:
        return _switchback_not_billable(planned, "the registered 97.75% lower bound is not above zero")
    return {
        "plan": blocks,
        "blocksPerArm": planned["blocksPerArm"],
        "billable": True,
        "lambda": current["lowerAlpha"],
        "estimate": current,
        "reason": "",
    }


def _switchback_not_billable(planned, reason):
    return {
        "plan": planned["blocks"],
        "blocksPerArm": planned["blocksPerArm"],
        "billable": False,
        "lambda": None,
        "estimate": None,
        "reason": reason,
    }


# ---------------------------------------------------------------------------
# Re-run CSV kit.
# ---------------------------------------------------------------------------


def parse_revenue_micros(text: str):
    text = text.strip()
    if not text or text.startswith("-"):
        return None
    if "." in text:
        whole, fraction = text.split(".", 1)
    else:
        whole, fraction = text, ""
    if not whole:
        return None
    if (whole and not whole.isdigit()) or (fraction and not fraction.isdigit()):
        return None
    if len(fraction) > 6:
        return None
    whole_value = int(whole) if whole else 0
    fraction_value = int(fraction.ljust(6, "0")) if fraction else 0
    value = whole_value * 1_000_000 + fraction_value
    if value > 9_223_372_036_854_775_807:
        return None
    return value


def parse_csv(csv_text: str):
    lines = [line for line in csv_text.splitlines() if line.strip()]
    if not lines:
        raise ValueError("malformed input: empty CSV")
    header = [column.strip() for column in lines[0].split(",")]
    if "day" not in header:
        raise ValueError("malformed input: CSV needs a 'day' column")
    if "arm" not in header:
        raise ValueError("malformed input: CSV needs an 'arm' column")
    if "revenue" not in header:
        raise ValueError("malformed input: CSV needs a 'revenue' column")
    day_at = header.index("day")
    arm_at = header.index("arm")
    revenue_at = header.index("revenue")
    users_at = header.index("users") if "users" in header else None
    rows = []
    for offset, line in enumerate(lines[1:]):
        cells = [cell.strip() for cell in line.split(",")]
        row_number = offset + 2
        if day_at >= len(cells) or not cells[day_at]:
            raise ValueError(f"malformed input: row {row_number}: missing day")
        if arm_at >= len(cells):
            raise ValueError(f"malformed input: row {row_number}: missing arm")
        arm = cells[arm_at]
        if arm not in ("control", "test"):
            raise ValueError(
                f"malformed input: row {row_number}: arm '{arm}' is not control or test"
            )
        if revenue_at >= len(cells):
            raise ValueError(f"malformed input: row {row_number}: missing revenue")
        revenue = parse_revenue_micros(cells[revenue_at])
        if revenue is None:
            raise ValueError(
                f"malformed input: row {row_number}: revenue "
                f"'{cells[revenue_at]}' is not a number"
            )
        users = None
        if users_at is not None and users_at < len(cells) and cells[users_at] != "":
            try:
                users = int(cells[users_at])
            except ValueError as error:
                raise ValueError(
                    f"malformed input: row {row_number}: users "
                    f"'{cells[users_at]}' is not a number"
                ) from error
        rows.append(
            {
                "day": cells[day_at],
                "test": arm == "test",
                "revenue": revenue,
                "users": users,
            }
        )
    if not rows:
        raise ValueError("malformed input: CSV has no data rows")
    return rows


def rerun(rows, window_days: int, allocated_share: float, burn_in_days: int = RERUN_BURN_IN_DAYS):
    if window_days not in ALLOWED_WINDOW_DAYS:
        raise ValueError(
            f"malformed input: window must be one of {list(ALLOWED_WINDOW_DAYS)}"
        )
    by_day = {}
    for row in rows:
        control, test = by_day.get(row["day"], (None, None))
        if row["test"]:
            test = row
        else:
            control = row
        by_day[row["day"]] = (control, test)

    ratios = []
    for index, day in enumerate(sorted(by_day)):
        if index < burn_in_days:
            continue
        control, test = by_day[day]
        if control is None or test is None:
            continue
        if control["users"] is not None and control["users"] > 0:
            control_value = control["revenue"] / control["users"]
        elif control["users"] is None:
            control_value = control["revenue"] / (1.0 - allocated_share)
        else:
            continue
        if test["users"] is not None and test["users"] > 0:
            test_value = test["revenue"] / test["users"]
        elif test["users"] is None:
            test_value = test["revenue"] / allocated_share
        else:
            continue
        if control_value > 0.0 and test_value > 0.0:
            ratios.append(math.log(test_value / control_value))

    if len(ratios) > window_days:
        ratios = ratios[: (window_days // 7) * 7]
    current = estimate(ratios)
    if current is None:
        raise ValueError("malformed input: the export has fewer than two usable days")
    return {
        "days": current["days"],
        "meanLog": current["meanLog"],
        "lift": current["lift"],
        "lower90": current["lower90"],
        "upper90": current["upper90"],
        "dailyLogRatios": ratios,
    }


# ---------------------------------------------------------------------------
# Fee basis.
# ---------------------------------------------------------------------------


def _fee_out(billing: dict, prereg: dict, decision: dict):
    lam = decision["lambda"] if decision["verdict"] == "keep" else 0.0
    if lam is None:
        lam = 0.0
    if lam <= 0.0:
        return {
            "lambda": lam,
            "factor": 1.0,
            "incremental": 0,
            "logShare": 1.0,
            "rateBp": prereg["rateBp"],
            "feeMicros": 0,
        }
    scope_revenue = billing["scopeRevenueMicros"]
    if scope_revenue <= 0:
        return None
    factor = 1.0 + lam
    if factor <= 1.0 or not math.isfinite(factor):
        return None
    incremental = _round_half_away(scope_revenue * (1.0 - 1.0 / factor))
    log_factor = math.log(factor)
    share = math.log(1.0 + lam) / log_factor
    rate = prereg["rateBp"] / 10_000.0
    fee = _round_half_away(rate * incremental * share)
    return {
        "lambda": lam,
        "factor": factor,
        "incremental": incremental,
        "logShare": share,
        "rateBp": prereg["rateBp"],
        "feeMicros": fee,
    }


# ---------------------------------------------------------------------------
# The twin case runner.
# ---------------------------------------------------------------------------


def _split_out(guardrails: dict, prereg: dict):
    unique = guardrails.get("uniquePlayers")
    if unique is not None:
        split = tuple(unique)
    else:
        pairs = guardrails.get("armCounts") or []
        test_sum = sum(pair[0] for pair in pairs)
        control_sum = sum(pair[1] for pair in pairs)
        split = (test_sum, control_sum) if test_sum + control_sum > 0 else None
    if split is None:
        return {"chiSquare": None, "testPlayers": None, "controlPlayers": None}
    return {
        "chiSquare": split_chi_square(split[0], split[1], prereg["testShare"]),
        "testPlayers": split[0],
        "controlPlayers": split[1],
    }


def run_case(case: dict) -> dict:
    prereg = case["prereg"]
    window_input = case["window"]
    guardrails_input = case.get("guardrails") or {}
    window = build_window(
        window_input["control"],
        window_input["test"],
        prereg["windowDays"],
        window_input["testAllocatedShare"],
        prereg["burnInDays"],
    )
    events = evaluate_guardrails(
        prereg,
        window["days"],
        guardrails_input.get("armCounts") or [],
        guardrails_input.get("uniquePlayers"),
        guardrails_input.get("fillFormat") or "rewarded",
        guardrails_input.get("settingsChanged") or [],
        guardrails_input.get("staleDays") or 0,
        guardrails_input.get("incidentRuns") or [],
    )
    bootstrap = {
        "iterations": window_input["bootstrapIterations"],
        "block_len": window_input["bootstrapBlockLen"],
        "seed": window_input["bootstrapSeed"],
    }
    daily_estimate = estimate([day["logRatio"] for day in window["days"]])

    player_estimate = None
    players = case.get("players")
    if players is not None:
        folded = player_window(players.get("rows") or [], players.get("pretest") or {})
        ht = estimate_window(folded, prereg["testShare"], bool(players.get("cuped")))
        player_estimate = ht
        if ht is not None:
            decision = decide_with_estimate(
                prereg,
                window,
                events,
                ht["estimate"],
                window_input["observedScope"],
                window_input["observedChangeDescription"],
                window_input["atFinalLook"],
                window_input["extensionUsed"],
                bootstrap,
            )
        else:
            decision = decide(
                prereg,
                window,
                events,
                window_input["observedScope"],
                window_input["observedChangeDescription"],
                window_input["atFinalLook"],
                window_input["extensionUsed"],
                bootstrap,
            )
    else:
        decision = decide(
            prereg,
            window,
            events,
            window_input["observedScope"],
            window_input["observedChangeDescription"],
            window_input["atFinalLook"],
            window_input["extensionUsed"],
            bootstrap,
        )

    billing = case.get("billing")
    fee = _fee_out(billing, prereg, decision) if billing is not None else None

    rerun_input = case.get("rerun")
    rerun_out = None
    if rerun_input is not None:
        rows = parse_csv(rerun_input["csv"])
        rerun_out = rerun(
            rows,
            rerun_input["windowDays"],
            rerun_input["allocatedShare"],
            rerun_input.get("burnInDays", RERUN_BURN_IN_DAYS),
        )

    return {
        "id": case["id"],
        "window": {
            "complete": window["complete"],
            "incomplete": window["incomplete"],
            "fatal": window["fatal"],
            "days": window["days"],
            "problems": window["problems"],
        },
        "dailyEstimate": daily_estimate,
        "playerEstimate": player_estimate,
        "guardrails": events,
        "decision": decision,
        "split": _split_out(guardrails_input, prereg),
        "switchback": switchback_out(case["switchback"]) if case.get("switchback") else None,
        "rerun": rerun_out,
        "fee": fee,
    }


def main(argv=None) -> int:
    """Run one JSON twin case and print the canonical result."""
    args = sys.argv[1:] if argv is None else list(argv)
    if len(args) != 1:
        print("usage: apex_lift_reference.py case.json", file=sys.stderr)
        return 2
    with open(args[0], encoding="utf-8") as handle:
        case = json.load(handle)
    print(json.dumps(run_case(case), indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
