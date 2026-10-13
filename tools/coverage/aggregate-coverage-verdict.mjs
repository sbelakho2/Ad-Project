#!/usr/bin/env node
// aggregate-coverage-verdict.mjs — final protected-branch gate (pipeline coverage-gate).
// Verifies: (1) manifest builds with zero unclassified files, (2) orphan gate passes,
// (3) floor ratchet intact, (4) emits coverage-report.md with the per-domain matrix.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { REPO_ROOT } from './coverage-lib.mjs';

const reportArgIdx = process.argv.indexOf('--report');
const reportFile = reportArgIdx >= 0 ? process.argv[reportArgIdx + 1] : 'coverage-report.md';
const failures = [];

function run([script, ...args]) {
  try {
    const out = execFileSync('node', [path.join(REPO_ROOT, 'tools/coverage', script), ...args], { encoding: 'utf8', cwd: REPO_ROOT });
    return { ok: true, out };
  } catch (e) {
    return { ok: false, out: `${e.stdout ?? ''}\n${e.stderr ?? ''}` };
  }
}

const manifest = run(['build-coverage-manifest.mjs', '--out', 'coverage-manifest.json']);
if (!manifest.ok) failures.push(['manifest', manifest.out]);

const floors = JSON.parse(fs.readFileSync(path.join(REPO_ROOT, 'tools/coverage/coverage-floor.json'), 'utf8'));
const orphanGate = run(['dogfood-gates.mjs', 'orphan-gate', '--manifest', 'coverage-manifest.json', '--suites', 'tools/coverage/suites.json', '--max-orphan-percent', String(floors.orphanFloorPercent ?? 100)]);
if (!orphanGate.ok) failures.push(['orphan-gate', orphanGate.out]);

const ratchet = run(['dogfood-gates.mjs', 'floor-ratchet']);
if (!ratchet.ok) failures.push(['floor-ratchet', ratchet.out]);

const domainRows = Object.entries(floors.domains)
  .map(([d, v]) => `| ${d} | ${v}% | ${v >= 100 ? 'DONE' : 'ratcheting'} |`)
  .join('\n');
const report = `# Coverage gate report

- generated: ${new Date().toISOString()}
- orphan floor: ${floors.orphanFloorPercent}% (target 0)
- mutation kill floor: ${floors.mutationKillFloorPercent}% (target 80)

| domain | floor | status |
|---|---|---|
${domainRows}

## Verdict

${failures.length === 0 ? 'PASS — manifest complete, floors respected, ratchet intact.' : `FAIL (${failures.length})`}

${failures.map(([n, o]) => `### ${n}\n\n\`\`\`\n${o.slice(0, 4000)}\n\`\`\``).join('\n\n')}
`;
fs.writeFileSync(path.join(REPO_ROOT, reportFile), report);

if (failures.length) {
  console.error(`AGGREGATE COVERAGE VERDICT: FAIL (${failures.map((f) => f[0]).join(', ')})`);
  process.exit(1);
}
console.log('AGGREGATE COVERAGE VERDICT: PASS');
