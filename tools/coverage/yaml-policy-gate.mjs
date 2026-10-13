#!/usr/bin/env node
// yaml-policy-gate.mjs — M-3/M-4 policy gates with a ratchet baseline.
//   --shell : every first-party .sh must declare `set -eu` (or `set -euo pipefail`) early,
//             and destructive scripts must honor DOGFOOD_DRY_RUN.
//   --full  : bans coverage-ignoring pragmas, unregistered describe.skip/it.skip, and
//             `|| true` on test/coverage commands in CI yaml.
//
// Ratchet semantics (tools/coverage/policy-baseline.json):
//   - a violation listed in the baseline for its category is tolerated;
//   - a violation NOT in the baseline fails the gate;
//   - a baseline entry that is no longer a violation FAILS ("stale entry") so the
//     baseline can only shrink — fixes must remove their entry.
import fs from 'node:fs';
import path from 'node:path';
import { walkRepo, REPO_ROOT } from './coverage-lib.mjs';

const full = process.argv.includes('--full');
const shell = process.argv.includes('--shell');

const baselinePath = path.join(REPO_ROOT, 'tools/coverage/policy-baseline.json');
const baseline = fs.existsSync(baselinePath) ? JSON.parse(fs.readFileSync(baselinePath, 'utf8')) : { pragma: [], skip: [], orTrue: [] };

const shellAllowlistNoSetEu = [
  'infrastructure/hetzner/', // sourced config fragments (documented policy)
  'tools/docs-lint/', // upstream-faithful port (kiwicaptcha-standalone); uses `set -u` by contract
];

const actual = { shell: [], pragma: [], skip: [], orTrue: [] };

const PRAGMA = /(pragma:\s*no cover|istanbul ignore|c8 ignore-next|coverage-ignore)/;
const SKIP = /(describe|it|test)\.(skip|todo)\(/;

if (shell || full) {
  for (const rel of walkRepo(REPO_ROOT)) {
    const isShell = rel.endsWith('.sh') || rel === '.husky/pre-commit';
    if (!isShell) continue;
    const text = fs.readFileSync(path.join(REPO_ROOT, rel), 'utf8');
    if (rel.endsWith('.sh')) {
      const firstHundred = text.slice(0, 2000);
      const hasSetEu = /set\s+-eu(o\s+pipefail)?/.test(firstHundred);
      if (!hasSetEu && !shellAllowlistNoSetEu.some((p) => rel.startsWith(p))) actual.shell.push(`${rel}: set -eu missing`);
    }
    // Word-boundary destructive keywords: 'dropped' must not match DROP (the
    // docs-lint port surfaced that substring false positive), and deploy-all.sh
    // as a FILE name is not content; only actual words count.
    if (/\b(deploy|ssh|kubectl|docker|pm2|systemctl|helm|terraform|migrate|drop|truncate)\b/i.test(text) && !/DOGFOOD_DRY_RUN/.test(text)) {
      actual.shell.push(`${rel}: destructive without DOGFOOD_DRY_RUN guard`);
    }
  }
}

if (full) {
  for (const rel of walkRepo(REPO_ROOT)) {
    let text;
    try { text = fs.readFileSync(path.join(REPO_ROOT, rel), 'utf8'); } catch { continue; }
    if (/\.(ts|tsx|js|jsx|mjs|cjs|py|kt|swift|rs|cs|go)$/.test(rel) && PRAGMA.test(text) && rel !== 'tools/coverage/yaml-policy-gate.mjs') actual.pragma.push(rel);
    if (isInTestTree(rel) && /\.(ts|tsx|js|jsx)$/.test(rel) && SKIP.test(text) && !/dogfood:skip:/.test(text)) actual.skip.push(rel);
    if (/\.(yml|yaml)$/.test(rel) && (rel.startsWith('.github/workflows/') || rel.startsWith('.woodpecker/'))) {
      for (const [i, line] of text.split('\n').entries()) {
        if (line.trim().startsWith('#')) continue; // comments cannot violate the soft-fail ban
        if (/\|\|\s*true/.test(line)) actual.orTrue.push(`${rel}:${i + 1}`);
      }
    }
  }
}

function isInTestTree(rel) {
  return rel.includes('/tests/') || rel.includes('__tests__') || /\.(test|spec)\./.test(rel);
}

const violations = [];
const staleBaseline = [];
// Ratchet applies only to the categories actually evaluated in this invocation.
const scopes = full ? ['shell', 'pragma', 'skip', 'orTrue'] : ['shell'];
for (const kind of scopes) {
  const base = new Set(baseline[kind] ?? []);
  const seenBase = new Set();
  for (const v of actual[kind]) {
    const fileKey = kind === 'orTrue' ? v.replace(/:\d+$/, '') : v.replace(/: .*$/, '');
    if (base.has(fileKey) || base.has(v)) { seenBase.add(base.has(fileKey) ? fileKey : v); continue; }
    violations.push(`[${kind}] ${v}`);
  }
  for (const b of base) {
    const stillPresent = actual[kind].some((v) => {
      const fileKey = kind === 'orTrue' ? v.replace(/:\d+$/, '') : v.replace(/: .*$/, '');
      return v === b || fileKey === b;
    });
    if (!stillPresent) staleBaseline.push(`[${kind}] '${b}' is no longer a violation — remove it from policy-baseline.json`);
  }
}

if (violations.length || staleBaseline.length) {
  console.error(`POLICY GATE FAILED (${violations.length} new, ${staleBaseline.length} stale):`);
  for (const v of [...violations, ...staleBaseline]) console.error(`  - ${v}`);
  process.exit(1);
}
console.log(`policy gate PASS (${shell ? 'shell ' : ''}${full ? 'full' : ''}; baseline: ${['shell', 'pragma', 'skip', 'orTrue'].map((k) => `${k}=${(baseline[k] ?? []).length}`).join(' ')})`);
