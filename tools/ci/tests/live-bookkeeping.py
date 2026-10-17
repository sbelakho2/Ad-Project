#!/usr/bin/env python3
"""Bookkeeping equivalence and scaling check for tools/dogfood/live.py.

The live harness holds 259 required cells; run_all_probes and the per-class
probes used to rescan the whole cell set (and rebuild the per-service lookup
tables) once per cell. This script proves the per-run indexes
(cells_of/_cell_index, response_contract_expectations, no_inline_targets)
return exactly what the pre-index code returned, then prints an n vs 2n
timing comparison of the indexed bookkeeping against the pre-fix linear
scans. Only the equivalence checks assert; the timings are printed evidence,
never asserted, so the check never flakes on a slow or busy machine.

Usage: python3 tools/ci/tests/live-bookkeeping.py
"""

import importlib.util
import json
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]

spec = importlib.util.spec_from_file_location(
    "dogfood_live", ROOT / "tools" / "dogfood" / "live.py"
)
live = importlib.util.module_from_spec(spec)
spec.loader.exec_module(live)

SERVICES = ["apex-api", "apex-control", "apex-sales-web"]
MATRIX_ROUNDS = 200


class FakeHarness:
    """The smallest object the indexed helpers need: the required set and the
    caches Harness.__init__ installs."""

    cells_of = live.Harness.cells_of
    _cell_index = live.Harness._cell_index
    response_contract_expectations = live.Harness.response_contract_expectations
    _response_contract_table = live.Harness._response_contract_table
    no_inline_targets = live.Harness.no_inline_targets
    _build_no_inline_targets = live.Harness._build_no_inline_targets

    def __init__(self, required):
        self.required = set(required)
        self._cell_index_cache = None
        self._contract_expectations = {}
        self._no_inline_targets = None


def reference_cells_of(required, cls, service=None):
    """The pre-fix cells_of body: a full scan of sorted(required) per call."""
    out = []
    for cell_id in sorted(required):
        svc, surface, cell_cls = cell_id.split("|", 2)
        if cell_cls != cls:
            continue
        if service and svc != service:
            continue
        out.append((svc, surface, cell_id))
    return out


def real_required():
    with open(ROOT / "tools" / "dogfood" / "surfaces.json", encoding="utf-8") as fh:
        surface = json.load(fh)
    required = set()
    for service, declared in surface["services"].items():
        for cell in declared["required"]:
            for cls in cell["classes"]:
                required.add("%s|%s|%s" % (service, cell["surface"], cls))
    return required


def check_cells_of_equivalence(required):
    fake = FakeHarness(required)
    checked = 0
    for cls in live.FROZEN_PROBE_CLASSES:
        variants = [None] + SERVICES
        for service in variants:
            got = fake.cells_of(cls, service)
            want = reference_cells_of(required, cls, service)
            if got != want:
                raise SystemExit(
                    "cells_of(%r, %r) diverged from the pre-fix scan: %r != %r"
                    % (cls, service, got, want)
                )
            checked += 1
    return checked


def check_table_caches(required):
    fake = FakeHarness(required)
    fake._contract_expectations = {}
    fake._no_inline_targets = None
    checks = 0
    for service in SERVICES:
        first = live.Harness.response_contract_expectations(fake, service)
        second = live.Harness.response_contract_expectations(fake, service)
        if first is not second:
            raise SystemExit("response-contract table for %s is rebuilt per call" % service)
        if first != live.Harness._response_contract_table(fake, service):
            raise SystemExit("cached response-contract table for %s altered content" % service)
        checks += 1
    targets_first = live.Harness.no_inline_targets(fake)
    targets_second = live.Harness.no_inline_targets(fake)
    if targets_first is not targets_second:
        raise SystemExit("no-inline targets are rebuilt per call")
    if targets_first != live.Harness._build_no_inline_targets(fake):
        raise SystemExit("cached no-inline targets altered content")
    checks += 1
    return checks


def synth_matrix(n):
    cells = []
    for index in range(n):
        service = SERVICES[index % len(SERVICES)]
        cls = live.FROZEN_PROBE_CLASSES[(index // len(SERVICES)) % len(live.FROZEN_PROBE_CLASSES)]
        cells.append("%s|GET /synth/%d|%s" % (service, index, cls))
    return cells


def time_cells_of(required, rounds):
    fake = FakeHarness(required)
    start = time.perf_counter()
    for _round in range(rounds):
        for cls in live.FROZEN_PROBE_CLASSES:
            for _svc, _surface, _cell_id in fake.cells_of(cls):
                pass
    return time.perf_counter() - start


def time_reference_cells_of(required, rounds):
    start = time.perf_counter()
    for _round in range(rounds):
        for cls in live.FROZEN_PROBE_CLASSES:
            for _entry in reference_cells_of(required, cls):
                pass
    return time.perf_counter() - start


def time_contract_tables(required, rebuild):
    fake = FakeHarness(required)
    fake._contract_expectations = {}
    cells_per_service = {}
    for service, _surface, _cell_id in live.Harness.cells_of(fake, "response-contract"):
        cells_per_service[service] = cells_per_service.get(service, 0) + 1
    start = time.perf_counter()
    for _round in range(200):
        fake._contract_expectations = {}
        for service, count in cells_per_service.items():
            for _index in range(count):
                if rebuild:
                    live.Harness._response_contract_table(fake, service)
                else:
                    live.Harness.response_contract_expectations(fake, service)
    return time.perf_counter() - start


def time_no_inline_targets(required, rebuild):
    fake = FakeHarness(required)
    fake._no_inline_targets = None
    cells = live.Harness.cells_of(fake, "no-inline")
    start = time.perf_counter()
    for _round in range(200):
        if rebuild:
            for service, _surface, cell_id in cells:
                surface = cell_id.split("|")[1]
                target = None
                for template, path in live.Harness._build_no_inline_targets(fake)[service]:
                    if template == surface:
                        target = path
                        break
        else:
            fake._no_inline_targets = None
            index = {
                (service, template): path
                for service, pairs in live.Harness.no_inline_targets(fake).items()
                for template, path in pairs
            }
            for service, _surface, cell_id in cells:
                surface = cell_id.split("|")[1]
                index.get((service, surface))
    return time.perf_counter() - start


def main():
    required = real_required()
    if len(required) != 259:
        raise SystemExit("expected the frozen 259-cell matrix, found %d" % len(required))
    pairs_checked = check_cells_of_equivalence(required)
    tables_checked = check_table_caches(required)
    print(
        "equivalence: cells_of matches the pre-fix scan for %d class/service pairs; "
        "%d per-run tables are built once and byte-identical" % (pairs_checked, tables_checked)
    )

    for n in (259, 518):
        matrix = synth_matrix(n)
        indexed = time_cells_of(matrix, MATRIX_ROUNDS)
        reference = time_reference_cells_of(matrix, MATRIX_ROUNDS)
        print(
            "cells_of cells=%d rounds=%d: indexed=%.1f ms pre-fix-scan=%.1f ms"
            % (n, MATRIX_ROUNDS, indexed * 1000, reference * 1000)
        )

    tables_rebuild = time_contract_tables(required, rebuild=True)
    tables_cached = time_contract_tables(required, rebuild=False)
    print(
        "response-contract tables (68 cells x 200 rounds): cached=%.1f ms rebuild-per-cell=%.1f ms"
        % (tables_cached * 1000, tables_rebuild * 1000)
    )

    targets_rebuild = time_no_inline_targets(required, rebuild=True)
    targets_cached = time_no_inline_targets(required, rebuild=False)
    print(
        "no-inline targets (52 cells x 200 rounds): cached=%.1f ms rebuild-scan-per-cell=%.1f ms"
        % (targets_cached * 1000, targets_rebuild * 1000)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
