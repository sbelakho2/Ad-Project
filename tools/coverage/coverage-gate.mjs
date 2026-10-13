#!/usr/bin/env node
// coverage-gate.mjs — per-domain hard gate. Parses a coverage artifact and fails
// unless every metric is >= the domain floor in tools/coverage/coverage-floor.json.
// Tools: jest (coverage-summary.json), lcov (lcov.info), go-func (cover.func), c8 (coverage-summary.json).
import fs from 'node:fs';
import path from 'node:path';
import { parseJestSummary, parseLcov, lcovTotals, parseGoFunc, REPO_ROOT } from './coverage-lib.mjs';

const argv = process.argv.slice(2);
const get = (k, d = null) => { const i = argv.indexOf(k); return i >= 0 ? argv[i + 1] : d; };
const domain = get('--domain');
const artifact = get('--artifact');
const tool = get('--tool', 'jest');
const requireBranches = argv.includes('--require-branches');
if (requireBranches && tool !== 'lcov') {
  console.error(`--require-branches is only meaningful for --tool lcov (got "${tool}") — refusing the silent no-op.`);
  process.exit(2);
}

function fail(msg, violations = []) {
  console.error(`COVERAGE GATE FAILED [${domain}] ${msg}`);
  for (const v of violations.slice(0, 50)) console.error(`  - ${v}`);
  process.exit(1);
}

const floors = JSON.parse(fs.readFileSync(path.join(REPO_ROOT, 'tools/coverage/coverage-floor.json'), 'utf8'));
if (!(domain in floors.domains)) fail(`unknown domain "${domain}" (not present in coverage-floor.json domains — a typo must not silently floor to 0)`);
const floor = floors.domains[domain];

const abs = path.isAbsolute(artifact) ? artifact : path.join(REPO_ROOT, artifact);
if (!fs.existsSync(abs)) fail(`artifact not found: ${artifact}`);

function assertMetrics(m) {
  const violations = [];
  for (const [k, v] of Object.entries(m)) {
    if (typeof v !== 'number') continue;
    if (v < floor) violations.push(`${k}: ${v}% < floor ${floor}%`);
  }
  return violations;
}

switch (tool) {
  case 'jest':
  case 'c8': {
    const t = parseJestSummary(fs.readFileSync(abs, 'utf8'));
    const v = assertMetrics({ lines: t.lines, branches: t.branches, functions: t.functions, statements: t.statements });
    if (v.length) fail(`below floor ${floor}%`, v);
    console.log(`gate PASS [${domain}]: lines=${t.lines} branches=${t.branches} functions=${t.functions} statements=${t.statements} (floor=${floor})`);
    break;
  }
  case 'lcov': {
    const files = parseLcov(fs.readFileSync(abs, 'utf8'));
    if (files.length === 0) fail('lcov contains no records');
    const t = lcovTotals(files, { requireBranches });
    const v = assertMetrics({ lines: t.lines, branches: t.branches, functions: t.functions });
    v.push(...t.violations);
    if (t.zeroLineFiles.length && floor > 0) v.push(`files with zero executable lines present in record: ${t.zeroLineFiles.slice(0, 20).join(', ')}`);
    if (v.length) fail(`below floor ${floor}%`, v);
    console.log(`gate PASS [${domain}]: lines=${t.lines} branches=${t.branches} functions=${t.functions} over ${files.length} files (floor=${floor})`);
    break;
  }
  case 'go-func': {
    const { rows, total } = parseGoFunc(fs.readFileSync(abs, 'utf8'));
    if (total === null) fail('cover.func has no total line');
    const v = [];
    if (total < floor) v.push(`total: ${total}% < floor ${floor}%`);
    for (const r of rows) if (r.pct < floor) v.push(`${r.func}: ${r.pct}%`);
    if (v.length) fail(`below floor ${floor}%`, v);
    console.log(`gate PASS [${domain}]: go total=${total}% over ${rows.length} funcs (floor=${floor})`);
    break;
  }
  default:
    fail(`unknown tool "${tool}" (use jest|c8|lcov|go-func)`);
}
