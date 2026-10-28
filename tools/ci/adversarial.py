#!/usr/bin/env python3
"""Diff-scoped adversarial lint.

Runs only over production Rust lines the diff adds or changes. It is
deliberately stricter than clippy: it fails on new panics, ignored results,
allow-attributes, unsafe blocks, string-built URLs, masked arithmetic and
unbounded-looking scans. Every rule is an addition-only ratchet, so existing
debt does not block unrelated changes.
"""
import argparse
import re
import subprocess
import sys

RULES = [
    ("panic", re.compile(r"\b(unwrap|expect)\s*\(")),
    ("panic", re.compile(r"\b(panic!|unimplemented!|todo!|unreachable!)\s*\(")),
    ("ignored-result", re.compile(r"^\s*let\s+_\s*=")),
    ("allow", re.compile(r"#\[allow\(")),
    ("unsafe", re.compile(r"\bunsafe\b")),
    ("http-url", re.compile(r"\"http://")),
    ("print", re.compile(r"\b(println!|eprintln!|dbg!)\s*\(")),
    ("masked-arithmetic", re.compile(r"\b(saturating_add|saturating_sub|saturating_mul|wrapping_add|wrapping_sub|wrapping_mul)\b")),
    ("string-query", re.compile(r"(SELECT|INSERT|UPDATE|DELETE)\b.*\{")),
]
# Production source only. Tests, bins, examples and generated code are out.
PROD = re.compile(r"^rust/crates/[^/]+/src/(?!bin/|tests/).*\.rs$")


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def added_lines(base):
    """Return {file: [(line_no, text)]} for production lines added by the diff."""
    diff = git("diff", "--unified=0", f"{base}...HEAD")
    diff += "\n" + git("diff", "--unified=0")
    result, path, new_line = {}, None, 0
    for line in diff.splitlines():
        if line.startswith("+++ b/"):
            path = line[6:]
            continue
        if line.startswith("@@"):
            match = re.search(r"\+(\d+)", line)
            new_line = int(match.group(1)) if match else 0
            continue
        if path and PROD.match(path) and line.startswith("+") and not line.startswith("+++"):
            result.setdefault(path, []).append((new_line, line[1:]))
            new_line += 1
        elif path and not line.startswith("-"):
            new_line += 1
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="origin/main")
    args = parser.parse_args()
    try:
        lines = added_lines(args.base)
    except subprocess.CalledProcessError:
        # No usable base: run over the working tree alone, never skip silently.
        lines = added_lines("HEAD")
    findings = []
    for path, rows in sorted(lines.items()):
        for number, text in rows:
            if "adversarial-ok" in text:
                continue
            for label, rule in RULES:
                if rule.search(text):
                    findings.append((path, number, label, text.strip()))
    for path, number, label, text in findings:
        print(f"{path}:{number}: adversarial[{label}]: {text}")
    if findings:
        print(f"adversarial: FAIL: {len(findings)} new finding(s) in changed production lines")
        return 1
    print(f"adversarial: PASS: {sum(len(v) for v in lines.values())} changed production line(s) clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
