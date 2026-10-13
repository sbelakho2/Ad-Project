#!/usr/bin/env node
// lcov-gate.mjs — standalone lcov enforcer used by flutter/apple/shell/rust pipelines.
// Usage: node lcov-gate.mjs <lcov.info> <floorPercent> [--require-branches]
import fs from 'node:fs';
import { parseLcov, lcovTotals } from './coverage-lib.mjs';

const [file, floorArg, ...flags] = process.argv.slice(2);
if (!file || !floorArg) {
  console.error('usage: node lcov-gate.mjs <lcov.info> <floorPercent> [--require-branches]');
  process.exit(2);
}
const floor = Number(floorArg);
const requireBranches = flags.includes('--require-branches');
if (!fs.existsSync(file)) {
  console.error(`lcov not found: ${file}`);
  process.exit(1);
}
const files = parseLcov(fs.readFileSync(file, 'utf8'));
if (files.length === 0) {
  console.error('lcov contains no records');
  process.exit(1);
}
const t = lcovTotals(files, { requireBranches });
const violations = [...t.violations];
if (t.lines < floor) violations.push(`lines: ${t.lines.toFixed(2)}% < ${floor}%`);
if (requireBranches && t.branchRecords > 0 && t.branches < floor) violations.push(`branches: ${t.branches.toFixed(2)}% < ${floor}%`);
if (t.functions < floor) violations.push(`functions: ${t.functions.toFixed(2)}% < ${floor}%`);
if (violations.length) {
  console.error(`LCOV GATE FAILED (${file})`);
  for (const v of violations) console.error(`  - ${v}`);
  const worst = files
    .filter((f) => f.LF > 0 && (f.LH / f.LF) * 100 < floor)
    .sort((a, b) => (a.LH / a.LF) * 100 - (b.LH / b.LF) * 100)
    .slice(0, 20);
  for (const f of worst) console.error(`  low: ${f.file} ${(f.LH / f.LF * 100).toFixed(1)}%`);
  process.exit(1);
}
console.log(`lcov gate PASS (${file}): lines=${t.lines.toFixed(2)}% branches=${t.branches.toFixed(2)}% functions=${t.functions.toFixed(2)}% over ${files.length} files`);
