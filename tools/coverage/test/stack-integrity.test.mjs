import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { DOMAINS, globToRegExp } from '../coverage-lib.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const coverageDir = path.resolve(here, '..'); // tools/coverage
const toolsDir = path.resolve(coverageDir, '..'); // tools
const repoRoot = path.resolve(toolsDir, '..'); // repo root

test('floor ratchet: orphan ceiling may only decrease; other floors may only increase', () => {
  const f = JSON.parse(fs.readFileSync(path.join(coverageDir, 'coverage-floor.json'), 'utf8'));
  const hist = f.history ?? [];
  for (let i = 1; i < hist.length; i++) {
    assert.ok(hist[i].orphanFloorPercent <= hist[i - 1].orphanFloorPercent, 'orphan ceiling increased in history');
    assert.ok(hist[i].mutationKillFloorPercent >= hist[i - 1].mutationKillFloorPercent, 'mutation floor decreased in history');
    for (const [d, v] of Object.entries(hist[i].domains ?? {})) {
      assert.ok(v >= (hist[i - 1].domains?.[d] ?? 0), `domain ${d} floor decreased in history`);
    }
  }
  const last = hist[hist.length - 1];
  if (last) {
    assert.ok((f.orphanFloorPercent ?? 0) <= (last.orphanFloorPercent ?? 100), 'current orphan ceiling exceeds last recorded');
    assert.ok((f.mutationKillFloorPercent ?? 0) >= (last.mutationKillFloorPercent ?? 0), 'current mutation floor below last recorded');
    for (const [d, v] of Object.entries(last.domains ?? {})) {
      assert.ok((f.domains?.[d] ?? 0) >= v, `current domain ${d} floor below last recorded`);
    }
  }
});

test('floor file has an entry for every code domain (and no unknown domains)', () => {
  const f = JSON.parse(fs.readFileSync(path.join(coverageDir, 'coverage-floor.json'), 'utf8'));
  const codeDomains = DOMAINS.filter((d) => d.kind === 'code').map((d) => d.id);
  const missing = codeDomains.filter((id) => !(id in f.domains));
  assert.deepEqual(missing, [], `coverage-floor.json missing code domains: ${missing.join(', ')}`);
  const floorDomains = Object.keys(f.domains);
  const unknown = floorDomains.filter((id) => !DOMAINS.some((d) => d.id === id));
  assert.deepEqual(unknown, [], `coverage-floor.json has domains that no longer exist: ${unknown.join(', ')}`);
});

test('floors.txt is in sync with coverage-floor.json', () => {
  const f = JSON.parse(fs.readFileSync(path.join(coverageDir, 'coverage-floor.json'), 'utf8'));
  const txt = fs.readFileSync(path.join(coverageDir, 'floors.txt'), 'utf8');
  const map = new Map();
  for (const line of txt.split('\n')) {
    const t = line.trim();
    if (!t || t.startsWith('#')) continue;
    const [d, v] = t.split(/\s+/);
    map.set(d, Number(v));
  }
  for (const [d, v] of Object.entries(f.domains)) {
    assert.equal(map.get(d), v, `floors.txt ${d} mismatch (fix: node tools/coverage/sync-floors.mjs --write)`);
  }
  assert.equal(map.size, Object.keys(f.domains).length);
});

// ---- coverage-scope parity: jest/c8 artifacts must not silently miss manifest files ----

function expandBraces(glob) {
  const m = glob.match(/\{([^{}]+)\}/);
  if (!m) return [glob];
  const out = [];
  for (const alt of m[1].split(',')) {
    out.push(...expandBraces(glob.slice(0, m.index) + alt + glob.slice(m.index + m[0].length)));
  }
  return out;
}

test('collect-scope.json positive globs equal the manifest domain includes (resolved per workspace)', () => {
  const scope = JSON.parse(fs.readFileSync(path.join(coverageDir, 'collect-scope.json'), 'utf8'));
  for (const [domain, entry] of Object.entries(scope)) {
    if (domain.startsWith('__')) continue;
    if (entry.__scope) continue; // declared subsets (js-tools) checked separately below
    const dom = DOMAINS.find((d) => d.id === domain);
    assert.ok(dom, `collect-scope.json entry '${domain}' has no matching DOMAINS entry`);
    const positives = entry.collectCoverageFrom.filter((g) => !g.startsWith('!'));
    const actual = positives.flatMap(expandBraces).map((g) => `${entry.dir}/${g}`).sort();
    const expected = [...dom.include].flatMap(expandBraces).sort();
    assert.deepEqual(actual, expected, `collect scope for '${domain}' diverges from DOMAINS.include (left survives in artifact, right in manifest)`);
  }
});

test('tools/package.json c8 --include flags match the declared js-tools scope', () => {
  const scope = JSON.parse(fs.readFileSync(path.join(coverageDir, 'collect-scope.json'), 'utf8'));
  const pkg = JSON.parse(fs.readFileSync(path.join(toolsDir, 'package.json'), 'utf8'));
  const cov = pkg.scripts['test:cov'];
  const listed = [...cov.matchAll(/--include\s+'([^']+)'/g)].map((m) => m[1]).sort();
  assert.deepEqual(listed, [...scope['js-tools'].collectCoverageFrom].sort());
});

test('every pipeline referenced by coverage-gate depends_on exists', () => {
  const gate = fs.readFileSync(path.resolve(repoRoot, '.woodpecker/coverage-gate.yml'), 'utf8');
  const block = gate.match(/depends_on:\n((?:\s+-\s+[a-z0-9-]+\n?)+)/);
  assert.ok(block, 'depends_on block not found in coverage-gate.yml');
  const deps = [...block[1].matchAll(/^\s+-\s+([a-z0-9-]+)$/gm)].map((m) => m[1]);
  assert.ok(deps.length >= 13, `expected at least 13 depends_on entries, got ${deps.length}`);
  for (const d of deps) {
    assert.ok(fs.existsSync(path.resolve(repoRoot, `.woodpecker/${d}.yml`)), `depends_on target missing: ${d}`);
  }
});

test('jest config files read their scope from collect-scope.json (no hardcoded globs)', () => {
  const configs = [
    'backend/jest.coverage.config.cjs', 'console/jest.coverage.config.js',
    'website/jest.coverage.config.cjs', 'control-plane/jest.coverage.config.js',
    'edge-worker/jest.coverage.config.cjs', 'services/autopilot/jest.coverage.config.js',
    'sdk/web/jest.coverage.config.cjs', 'sdk/react-native/jest.coverage.config.cjs',
  ];
  for (const rel of configs) {
    const text = fs.readFileSync(path.join(repoRoot, rel), 'utf8');
    assert.ok(text.includes("collect-scope.json"), `${rel} must read tools/coverage/collect-scope.json`);
    assert.ok(!/coverageThreshold/.test(text), `${rel} must not hardcode a coverage threshold (floors are single-sourced)`);
  }
});

test('no jest config hardcodes collectCoverageFrom glob literals', () => {
  const configs = [
    'backend/jest.coverage.config.cjs', 'console/jest.coverage.config.js',
    'website/jest.coverage.config.cjs', 'control-plane/jest.coverage.config.js',
    'edge-worker/jest.coverage.config.cjs', 'services/autopilot/jest.coverage.config.js',
    'sdk/web/jest.coverage.config.cjs', 'sdk/react-native/jest.coverage.config.cjs',
  ];
  for (const rel of configs) {
    const text = fs.readFileSync(path.join(repoRoot, rel), 'utf8');
    assert.ok(!/collectCoverageFrom:\s*\[/.test(text), `${rel} must use scope.collectCoverageFrom instead of an inline array`);
  }
});

test('contract-checks builds its artifact filter from the single-sourced pattern list', () => {
  const text = fs.readFileSync(path.join(coverageDir, 'contract-checks.mjs'), 'utf8');
  assert.ok(text.includes('CI_GENERATED_PATH_PATTERNS'), 'contract-checks must import the shared pattern list');
  assert.ok(!/const GENERATED_ARTIFACT = \//.test(text), 'no inline artifact regex literal allowed in contract-checks');
});

// keep globToRegExp referenced so accidental removal of the import is caught
test('glob helper is importable from the stack test', () => {
  assert.ok(globToRegExp('a/**/*.ts').test('a/b/c.ts'));
});
