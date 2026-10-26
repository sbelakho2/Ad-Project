#!/usr/bin/env python3
"""Plan and run the checks a diff actually needs.

Maps changed repository paths to Rust crates (with reverse-dependency
closure) and to the non-Rust gates, so a pull request runs only the tests,
clippy targets and gates its files can affect. The full workspace lanes still
run on main; this is the fast, diff-scoped gate.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUST = ROOT / "rust"


def git(*args: str) -> str:
    out = subprocess.run(
        ["git", *args], cwd=ROOT, capture_output=True, text=True, check=False
    )
    return out.stdout if out.returncode == 0 else ""


def changed_paths(base: str | None) -> list[str]:
    paths: set[str] = set()
    merge_base = ""
    if base:
        merge_base = git("merge-base", base, "HEAD").strip()
    if not merge_base:
        merge_base = git("rev-parse", "HEAD").strip()
    if merge_base:
        paths.update(git("diff", "--name-only", f"{merge_base}...HEAD").splitlines())
    paths.update(git("diff", "--name-only").splitlines())
    paths.update(git("diff", "--cached", "--name-only").splitlines())
    paths.update(git("ls-files", "--others", "--exclude-standard").splitlines())
    return sorted(
        p
        for p in paths
        if p and not p.startswith((".dogfood-live/", "rust/target/"))
    )


def changed_line_ranges(base: str | None) -> dict[str, list[tuple[int, int]]]:
    """New-side changed line ranges per path, for hunk-scoped mutation."""
    ranges: dict[str, list[tuple[int, int]]] = {}
    merge_base = ""
    if base:
        merge_base = git("merge-base", base, "HEAD").strip()
    diffs = []
    if merge_base:
        diffs.append(git("diff", "--unified=0", "--no-color", f"{merge_base}..HEAD"))
    diffs.append(git("diff", "--unified=0", "--no-color", "HEAD"))
    for diff in diffs:
        path = ""
        for line in diff.splitlines():
            if line.startswith("+++ b/"):
                path = line[6:]
            elif line.startswith("@@") and path:
                try:
                    plus = line.split("+", 1)[1].split(" ", 1)[0]
                    start_s, _, count_s = plus.partition(",")
                    start = int(start_s)
                    count = int(count_s) if count_s else 1
                except (IndexError, ValueError):
                    continue
                if count > 0:
                    ranges.setdefault(path, []).append((start, start + count - 1))
                else:
                    ranges.setdefault(path, [])
    return ranges


def metadata() -> dict:
    out = subprocess.run(
        [
            "cargo",
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            str(RUST / "Cargo.toml"),
        ],
        cwd=RUST,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(out.stdout)


def crate_map(meta: dict) -> tuple[dict[str, str], dict[str, set[str]], set[str]]:
    dir_to_pkg: dict[str, str] = {}
    rev: dict[str, set[str]] = {}
    all_pkgs: set[str] = set()
    local = {pkg["name"] for pkg in meta["packages"]}
    for pkg in meta["packages"]:
        all_pkgs.add(pkg["name"])
        rel = os.path.relpath(pkg["manifest_path"], RUST)
        top = Path(rel).parts[0]
        if top == "crates" and len(Path(rel).parts) > 1:
            dir_to_pkg[Path(rel).parts[1]] = pkg["name"]
        elif top == "apps" and len(Path(rel).parts) > 1:
            dir_to_pkg[Path(rel).parts[1]] = pkg["name"]
        for dep in pkg.get("dependencies", []):
            name = dep.get("name", "")
            if name in local:
                rev.setdefault(name, set()).add(pkg["name"])
    return dir_to_pkg, rev, all_pkgs


def rust_crates(paths: list[str], meta: dict) -> tuple[set[str], bool]:
    dir_to_pkg, rev, all_pkgs = crate_map(meta)
    hit: set[str] = set()
    full = False
    for p in paths:
        if not p.startswith("rust/"):
            continue
        parts = Path(p).parts
        if len(parts) == 2 and parts[1] in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
            full = True
        elif p in ("rust/Cargo.toml", "rust/Cargo.lock"):
            full = True
        elif len(parts) >= 4 and parts[1] == "crates":
            pkg = dir_to_pkg.get(parts[2])
            if pkg:
                hit.add(pkg)
        elif len(parts) >= 4 and parts[1] == "apps":
            pkg = dir_to_pkg.get(parts[2])
            if pkg:
                hit.add(pkg)
    # Reverse-dependency closure already pulls in every crate that depends
    # on apex-core; escalating to the whole workspace would defeat the diff
    # scope. Only workspace-wide manifests force a full run.
    if full:
        return all_pkgs, True
    closure = set(hit)
    queue = list(hit)
    while queue:
        current = queue.pop()
        for parent in rev.get(current, set()):
            if parent not in closure:
                closure.add(parent)
                queue.append(parent)
    return closure, False


def gates(paths: list[str], meta: dict) -> list[str]:
    selected: list[str] = []
    def has(prefix: str) -> bool:
        return any(p == prefix or p.startswith(prefix + "/") for p in paths)
    if has("docs") or has("README.md"):
        selected += ["docs"]
    if has("site"):
        selected += ["site"]
    if has("infrastructure/caddy"):
        selected += ["caddy"]
    if has("infrastructure/systemd") or has("infrastructure/sync"):
        selected += ["systemd-sync"]
    if has("tools/dogfood"):
        selected += ["dogfood-plan"]
    if has(".github") or has(".kiwi"):
        selected += ["github"]
    if has("tools/coverage") or has("Makefile") or has(".kiwi"):
        selected += ["dogfood-audit"]
    if rust_crates(paths, meta)[0]:
        selected += ["a11y"] if any(
            p.startswith((
                "rust/crates/apex-api",
                "rust/crates/apex-control",
                "rust/crates/apex-sales-web",
                "rust/crates/apex-ui",
                "rust/crates/apex-metrics",
                "rust/crates/apex-tools",
            ))
            for p in paths
        ) else []
    return sorted(set(selected))


def run(cmd: list[str], cwd: Path = ROOT) -> int:
    print("+", " ".join(cmd), flush=True)
    return subprocess.run(cmd, cwd=cwd).returncode


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--test", action="store_true")
    ap.add_argument("--clippy", action="store_true")
    ap.add_argument("--gates", action="store_true")
    ap.add_argument("--mutation", action="store_true")
    ap.add_argument("--security", action="store_true")
    ap.add_argument("--receipt", default="", help="write a verification receipt JSON")
    ap.add_argument("--paths", action="append", default=[], help="extra changed paths (testing/CI)")
    args = ap.parse_args()
    # --paths is the deterministic test seam: when supplied it is the exact
    # changed set, not a union with the caller's dirty working tree.
    paths = (
        sorted(set(args.paths))
        if args.paths
        else sorted(set(changed_paths(args.base)))
    )
    meta = metadata()
    if not paths:
        # No diff could be computed (shallow clone, detached head): failing
        # open would silently skip every check, so run the full workspace.
        print("changed.py: no diff paths; falling back to the full workspace", file=sys.stderr)
        crates, full = {pkg["name"] for pkg in meta["packages"]}, True
        selected = [
            "a11y",
            "caddy",
            "docs",
            "dogfood-audit",
            "dogfood-plan",
            "github",
            "site",
            "systemd-sync",
        ]
    else:
        crates, full = rust_crates(paths, meta)
        selected = gates(paths, meta)
    if args.list or not (
        args.test or args.clippy or args.gates or args.mutation or args.security
    ):
        print("changed files:", len(paths))
        for p in paths[:40]:
            print("  ", p)
        print("rust crates:", ", ".join(sorted(crates)) or "(none)")
        print("gates:", ", ".join(selected) or "(none)")
        return 0
    failures = 0
    if args.security:
        import re

        secret_patterns = [
            re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
            re.compile(r"\bAKIA[0-9A-Z]{16}\b"),
            re.compile(r"\bghp_[A-Za-z0-9]{36}\b"),
            re.compile(r"\bsk_live_[A-Za-z0-9]{16,}\b"),
            re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b"),
        ]
        for path in paths:
            if (
                path.startswith("rust/crates/")
                and ("/tests/" in path or path.endswith("/tests.rs"))
            ) or path.endswith((".png", ".jpg", ".jpeg", ".woff2", ".pdf", ".lock")):
                continue
            file_path = ROOT / path
            if not file_path.is_file() or file_path.stat().st_size > 2_000_000:
                continue
            try:
                text = file_path.read_text(errors="ignore")
            except OSError:
                continue
            for pattern in secret_patterns:
                if pattern.search(text):
                    print(f"security-scan: possible credential in {path}", flush=True)
                    failures += 1
                    break
    if args.test and crates:
        failures += run(
            ["cargo", "test", *sum((["-p", c] for c in sorted(crates)), [])], RUST
        )
    if args.clippy and crates:
        failures += run(
            ["cargo", "clippy", *sum((["-p", c] for c in sorted(crates)), []), "--all-targets", "--", "-D", "warnings"],
            RUST,
        )
    line_ranges = changed_line_ranges(args.base)
    mutation_files = [
        p
        for p in paths
        if p.startswith("rust/crates/")
        and "/src/" in p
        and p.endswith(".rs")
        and "/tests/" not in p
        and "/src/bin/" not in p
        and not p.endswith(("/tests.rs", "/test_support.rs", "/testkit.rs", "/testing.rs"))
        and (ROOT / p).is_file()
    ]
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target_dir.is_absolute():
        target_dir = RUST / target_dir
    tools = target_dir / "debug" / "apex-tools"
    if args.mutation or args.gates:
        build = run(["cargo", "build", "-q", "-p", "apex-tools"], RUST)
        if build:
            return build
    if args.mutation and not paths:
        # No diff means no reliable hunk selection; run the curated sample
        # rather than silently claiming a mutation pass.
        failures += run([str(tools), "mutation-smoke"])
    elif args.mutation and mutation_files:
        command = [str(tools), "mutation-diff"]
        for p in mutation_files:
            command += ["--file", p]
            for start, end in line_ranges.get(p, []):
                command += ["--line", f"{p}:{start}-{end}"]
        failures += run(command)
    if args.gates and "dogfood-audit" in selected:
        failures += run(["make", "dogfood"], ROOT)
    if args.gates:
        target_dir = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
        if not target_dir.is_absolute():
            target_dir = RUST / target_dir
        tools = target_dir / "debug" / "apex-tools"
        if crates:
            failures += run([str(tools), "stub-ban"])
            failures += run([str(tools), "assertion-presence"])
            failures += run([str(tools), "yaml-policy", "--full"])
        # The retired-provider contract is repository-wide: every changed
        # file, including docs and fixtures, can reintroduce it.
        failures += run([str(tools), "contract", "no-retired-provider"])
        if "github" in selected:
            failures += run([str(tools), "contract", "github-metadata"])
        if "docs" in selected:
            failures += run([str(tools), "docs-lint", "--source", "--baseline", "tools/coverage/docs-lint-baseline.txt"])
            failures += run([str(tools), "docs-links", "--baseline", "tools/coverage/docs-links-baseline.txt"])
        if "dogfood-plan" in selected:
            failures += run([str(tools), "dogfood-plan"])
        if "site" in selected:
            failures += run(["zola", "build"], ROOT / "site")
            failures += run([str(tools), "claims"])
            failures += run([str(tools), "perf-budget", "--min-pages", "39"])
        if "caddy" in selected:
            caddy = "/tmp/caddy" if Path("/tmp/caddy").exists() else "caddy"
            failures += run(["sh", "infrastructure/caddy/tests/contract.sh", "infrastructure/caddy/Caddyfile"], ROOT)
        if "systemd-sync" in selected:
            failures += run(["sh", "infrastructure/sync/tests/run-tests.sh"], ROOT)
        if "a11y" in selected:
            failures += run(
                [
                    "cargo",
                    "run",
                    "-q",
                    "-p",
                    "apex-api",
                    "--bin",
                    "render-fixtures",
                    "--",
                    "crates/apex-api/fixtures",
                    "target/ui-fixtures",
                ],
                RUST,
            )
            failures += run(
                [
                    str(tools),
                    "console-a11y",
                    "rust/target/ui-fixtures",
                    "--min-pages",
                    "62",
                ]
            )
            failures += run(
                [
                    "cargo",
                    "run",
                    "-q",
                    "-p",
                    "apex-control",
                    "--bin",
                    "render-fixtures",
                    "--",
                    "target/ui-fixtures-control",
                ],
                RUST,
            )
            failures += run(
                [
                    str(tools),
                    "console-a11y",
                    "rust/target/ui-fixtures-control",
                    "--min-pages",
                    "56",
                ]
            )
            failures += run(
                [
                    "cargo",
                    "run",
                    "-q",
                    "-p",
                    "apex-sales-web",
                    "--bin",
                    "render-fixtures",
                    "--",
                    "target/ui-fixtures-sales",
                ],
                RUST,
            )
            failures += run(
                [
                    str(tools),
                    "console-a11y",
                    "rust/target/ui-fixtures-sales",
                    "--min-pages",
                    "38",
                ]
            )
    if args.receipt:
        from datetime import datetime, timezone
        import json

        receipt = {
            "commit": git("rev-parse", "HEAD").strip(),
            "generatedAt": datetime.now(timezone.utc).isoformat(),
            "paths": paths,
            "crates": sorted(crates),
            "fullWorkspace": full,
            "result": "pass" if failures == 0 else "fail",
            "failures": failures,
            "checks": [
                name
                for name, enabled in [
                    ("test", args.test),
                    ("clippy", args.clippy),
                    ("mutation", args.mutation),
                    ("security", args.security),
                    ("gates", args.gates),
                ]
                if enabled
            ],
        }
        target = Path(args.receipt)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(json.dumps(receipt, indent=2) + "\n")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
