#!/usr/bin/env python3
"""Run the lift twin: one dataset, two implementations, one verdict.

For every case in ``cases.build_cases()`` the harness sends the same JSON to
the Rust production engine (``apex-lift`` via its example CLI) and to the
Python reference kit. It compares the whole result: the window shape and
problems, the daily and per-player estimates and bounds, the guardrail events,
the decision verdict and lambda, the split evidence, the switchback analysis,
the re-run report and the fee basis. Any disagreement fails the run.

The harness is deterministic: cases carry fixed datasets and seeds, and the
bootstrap/switchback seeds are part of the case.

Usage:
    python3 tools/lift-validation/twin.py
    python3 tools/lift-validation/twin.py --case daily-keep --case fee-on-keep
    python3 tools/lift-validation/twin.py --rust-bin rust/target/debug/examples/apex-lift-twin
"""

from __future__ import annotations

import argparse
import json
import math
import subprocess
import sys
from pathlib import Path

import apex_lift_reference as reference
from cases import build_cases

ROOT = Path(__file__).resolve().parents[2]
RUST = ROOT / "rust"
DEFAULT_BIN = RUST / "target" / "debug" / "examples" / "apex-lift-twin"
TOLERANCE = 1e-9

SKIP_VALUE_KEYS = {"reason"}


def build_rust(bin_path: Path) -> None:
    # Always rebuild the example: a cached target directory may hold a binary
    # compiled from a different source revision. Cargo is incremental.
    print("building apex-lift twin example", flush=True)
    result = subprocess.run(
        ["cargo", "build", "-q", "-p", "apex-lift", "--example", "apex-lift-twin"],
        cwd=RUST,
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        raise SystemExit(result.stderr or result.stdout)
    if not bin_path.is_file():
        raise SystemExit(f"cargo did not produce {bin_path}")


def run_rust(bin_path: Path, case: dict) -> dict:
    result = subprocess.run(
        [str(bin_path)],
        input=json.dumps(case),
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"rust twin failed for {case.get('id')}:\n{result.stderr.strip()}"
        )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"rust twin emitted invalid JSON for {case.get('id')}: {error}"
        ) from error


def _kind(value):
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, int):
        return "int"
    if isinstance(value, float):
        return "float"
    if isinstance(value, str):
        return "str"
    if value is None:
        return "null"
    if isinstance(value, list):
        return "list"
    if isinstance(value, dict):
        return "dict"
    return type(value).__name__


def compare(expected, actual, path: str, errors: list[str]) -> None:
    expected_kind = _kind(expected)
    actual_kind = _kind(actual)
    if expected_kind != actual_kind:
        if {expected_kind, actual_kind} <= {"int", "float"}:
            expected = float(expected)
            actual = float(actual)
            expected_kind = actual_kind = "float"
        else:
            errors.append(
                f"{path}: type {actual_kind} != expected {expected_kind} "
                f"({actual!r} != {expected!r})"
            )
            return
    if expected_kind == "dict":
        missing = sorted(set(expected) - set(actual))
        extra = sorted(set(actual) - set(expected))
        if missing:
            errors.append(f"{path}: missing keys {missing}")
        if extra:
            errors.append(f"{path}: unexpected keys {extra}")
        for key in sorted(set(expected) & set(actual)):
            child = f"{path}.{key}" if path else key
            if key in SKIP_VALUE_KEYS:
                continue
            compare(expected[key], actual[key], child, errors)
        return
    if expected_kind == "list":
        if len(expected) != len(actual):
            errors.append(f"{path}: length {len(actual)} != expected {len(expected)}")
        for index, (left, right) in enumerate(zip(expected, actual)):
            compare(left, right, f"{path}[{index}]", errors)
        return
    if expected_kind == "float":
        if not math.isclose(expected, actual, rel_tol=TOLERANCE, abs_tol=TOLERANCE):
            errors.append(f"{path}: {actual!r} != expected {expected!r}")
        return
    if expected != actual:
        errors.append(f"{path}: {actual!r} != expected {expected!r}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-bin", type=Path, default=DEFAULT_BIN)
    parser.add_argument(
        "--case",
        action="append",
        dest="case_ids",
        help="only run these case ids (repeatable)",
    )
    parser.add_argument(
        "--keep-going",
        action="store_true",
        help="compare every case even after a disagreement",
    )
    args = parser.parse_args()

    bin_path = args.rust_bin
    if not bin_path.is_absolute():
        bin_path = ROOT / bin_path
    build_rust(bin_path)

    cases = build_cases()
    if args.case_ids:
        wanted = set(args.case_ids)
        cases = [case for case in cases if case["id"] in wanted]
        missing = wanted - {case["id"] for case in cases}
        if missing:
            raise SystemExit(f"unknown case ids: {sorted(missing)}")

    failures = 0
    for case in cases:
        case_id = case["id"]
        try:
            python_result = reference.run_case(case)
            rust_result = run_rust(bin_path, case)
        except Exception as error:  # noqa: BLE001 - the harness reports every failure.
            failures += 1
            print(f"FAIL {case_id}: {error}")
            if not args.keep_going:
                break
            continue
        errors: list[str] = []
        compare(python_result, rust_result, "", errors)
        if errors:
            failures += 1
            print(f"FAIL {case_id} ({len(errors)} disagreement(s))")
            for error in errors[:20]:
                print(f"  {error}")
            if len(errors) > 20:
                print(f"  ... {len(errors) - 20} more")
            if not args.keep_going:
                break
        else:
            print(f"ok   {case_id}")

    total = len(cases)
    if failures:
        print(f"\nlift twin FAILED: {failures} of {total} case(s) disagreed")
        return 1
    print(f"\nlift twin PASS: {total} case(s) agreed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
