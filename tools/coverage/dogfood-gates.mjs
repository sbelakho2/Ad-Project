#!/usr/bin/env node
// dogfood-gates.mjs — meta-adversarial gates M-1/M-2/M-7 plus mutation floor and floor sync.
// Subcommands: orphan-gate | floor-ratchet | floors-sync | assertion-presence | mutation-floor
import fs from 'node:fs';
import path from 'node:path';
import { walkRepo, globToRegExp, classify, REPO_ROOT } from './coverage-lib.mjs';

const [cmd, ...rest] = process.argv.slice(2);
// NOTE: values follow their flag directly; indexes must be computed on `rest`, not on a filtered list.
const get = (k, d = null) => { const i = rest.indexOf(k); return i >= 0 ? rest[i + 1] : d; };

function die(msg, extra = []) {
  console.error(msg);
  for (const e of extra) console.error(`  - ${e}`);
  process.exit(1);
}

function loadJson(p) {
  return JSON.parse(fs.readFileSync(path.isAbsolute(p) ? p : path.join(REPO_ROOT, p), 'utf8'));
}

function readFloorsTxt() {
  const txt = fs.readFileSync(path.join(REPO_ROOT, 'tools/coverage/floors.txt'), 'utf8');
  const map = new Map();
  for (const line of txt.split('\n')) {
    const t = line.trim();
    if (!t || t.startsWith('#')) continue;
    const [domain, floor] = t.split(/\s+/);
    map.set(domain, Number(floor));
  }
  return map;
}

switch (cmd) {
  case 'orphan-gate': {
    const manifest = loadJson(get('--manifest', 'coverage-manifest.json'));
    const suites = loadJson(get('--suites', 'tools/coverage/suites.json'));
    const floorCfg = loadJson('tools/coverage/coverage-floor.json');
    const maxOrphan = Number(get('--max-orphan-percent', String(floorCfg.orphanFloorPercent ?? 100)));
    const codeFiles = manifest.files.filter((f) => f.kind === 'code');
    const claimed = (f) => suites.suites.some((s) => (s.covers ?? []).some((g) => globToRegExp(g).test(f.path)));
    const orphans = codeFiles.filter((f) => f.status === 'planned' && !claimed(f));
    const pct = codeFiles.length ? (orphans.length / codeFiles.length) * 100 : 0;
    if (pct > maxOrphan) {
      die(`ORPHAN GATE FAILED: ${orphans.length}/${codeFiles.length} code files (${pct.toFixed(2)}%) unclaimed (max ${maxOrphan}%). Register owning suites in tools/coverage/suites.json or add justified exclusions. First 50:`,
        orphans.slice(0, 50).map((o) => o.path));
    }
    console.log(`orphan gate PASS: ${orphans.length}/${codeFiles.length} orphans (${pct.toFixed(2)}% <= ${maxOrphan}%)`);
    break;
  }
  case 'floor-ratchet': {
    // Direction per field (this is the whole point of a ratchet):
    //   orphanFloorPercent        = MAXIMUM allowed orphan share -> may only DECREASE (target 0)
    //   mutationKillFloorPercent  = MINIMUM kill rate               -> may only INCREASE
    //   domains[*]                = MINIMUM coverage percent        -> may only INCREASE (target 100)
    // EVERY history step AND the current values are checked (not just consecutive pairs).
    const f = loadJson('tools/coverage/coverage-floor.json');
    const hist = f.history ?? [];
    for (let i = 1; i < hist.length; i++) {
      const a = hist[i - 1], b = hist[i];
      if ((b.orphanFloorPercent ?? 0) > (a.orphanFloorPercent ?? 0)) die(`floor-ratchet FAILED: orphanFloorPercent increased in history step ${i} (${a.orphanFloorPercent} -> ${b.orphanFloorPercent}); it is a CEILING and may only fall toward 0`);
      if ((b.mutationKillFloorPercent ?? 0) < (a.mutationKillFloorPercent ?? 0)) die(`floor-ratchet FAILED: mutationKillFloorPercent decreased in history step ${i}`);
      for (const [d, v] of Object.entries(b.domains ?? {})) {
        if ((a.domains?.[d] ?? 0) > v) die(`floor-ratchet FAILED: domain ${d} floor decreased in history step ${i} (${a.domains[d]} -> ${v})`);
      }
    }
    const last = hist[hist.length - 1];
    if (last) {
      if ((f.orphanFloorPercent ?? 0) > (last.orphanFloorPercent ?? 100)) die(`floor-ratchet FAILED: current orphanFloorPercent ${f.orphanFloorPercent} exceeds last recorded ceiling ${last.orphanFloorPercent}`);
      if ((f.mutationKillFloorPercent ?? 0) < (last.mutationKillFloorPercent ?? 0)) die(`floor-ratchet FAILED: current mutationKillFloorPercent ${f.mutationKillFloorPercent} below last recorded floor ${last.mutationKillFloorPercent}`);
      for (const [d, v] of Object.entries(last.domains ?? {})) {
        if ((f.domains?.[d] ?? 0) < v) die(`floor-ratchet FAILED: current domain ${d} floor ${f.domains?.[d]} below last recorded ${v}`);
      }
      for (const [d, v] of Object.entries(f.domains ?? {})) {
        if (v < 0) die(`floor-ratchet FAILED: domain ${d} has negative floor`);
      }
    }
    console.log('floor-ratchet PASS: orphan ceiling non-increasing, domain/mutation floors non-decreasing, current values within history bounds');
    break;
  }
  case 'floors-sync': {
    const f = loadJson('tools/coverage/coverage-floor.json');
    const txt = readFloorsTxt();
    const problems = [];
    for (const [domain, floor] of Object.entries(f.domains)) {
      if (!txt.has(domain)) problems.push(`floors.txt missing domain '${domain}'`);
      else if (txt.get(domain) !== floor) problems.push(`floors.txt ${domain}=${txt.get(domain)} != coverage-floor.json ${floor}`);
    }
    for (const domain of txt.keys()) {
      if (!(domain in f.domains)) problems.push(`floors.txt has unknown domain '${domain}'`);
    }
    if (problems.length) die('FLOORS-SYNC FAILED: run `node tools/coverage/sync-floors.mjs --write` after editing coverage-floor.json', problems);
    console.log(`floors-sync PASS: ${txt.size} domains in lockstep`);
    break;
  }
  case 'assertion-presence': {
    // M-7: every file that DEFINES test cases must also assert something.
    // Support files (helpers, msw handlers, fixtures, setup) define no test cases
    // and are therefore out of scope; the classifier decides what is a test file.
    const TEST_EXT = /\.(py|ts|tsx|js|jsx|mjs|go|rs|kt|java|swift|cs|bats)$/;
    const HAS_TEST_CASE = /(\bdef test_|(?<!\.)\bit\(|(?<!\.)\btest\(|@Test\b|#\[test\]|#\[tokio::test\]|XCTestCase|\[Fact\]|\[Theory\]|^@test\b|class\s+\w*Test\w*\s*:|fun\s+\w*[Tt]est\w*\s*\()/m;
    const HAS_ASSERT = /(#expect|XCTAssert|XCTFail|expect\(|assert\.|\bassert\b|Assert\.|require\.Equal|t\.Error|t\.Fatal|\[Fact\]|\[ "\$|assertTrue|assertFalse|assertEquals|assertNotNull|assertNull|assertThrows|assertFailsWith|@Test\(expected|ViewAssertions|\bmatches\()/;
    const bad = [];
    for (const rel of walkRepo(REPO_ROOT)) {
      if (!TEST_EXT.test(rel)) continue;
      const c = classify(rel);
      if (c.kind !== 'test') continue;
      let text;
      try { text = fs.readFileSync(path.join(REPO_ROOT, rel), 'utf8'); } catch { continue; }
      if (!HAS_TEST_CASE.test(text)) continue; // support file, not a test definition
      if (!HAS_ASSERT.test(text)) bad.push(`${rel} (defines test cases but contains no assertion — always-green test ban, M-7)`);
    }
    if (bad.length) die('ASSERTION-PRESENCE GATE FAILED:', bad);
    console.log('assertion-presence PASS');
    break;
  }
  case 'mutation-floor': {
    const floor = Number(get('--floor', '0'));
    const f = loadJson('tools/coverage/coverage-floor.json');
    const required = Math.max(floor, f.mutationKillFloorPercent ?? 0);
    const resultsPath = 'tools/coverage/mutation-results.json';
    if (!fs.existsSync(path.join(REPO_ROOT, resultsPath))) {
      console.log(`mutation-floor SKIP: no ${resultsPath} yet (phase 3; floor=${required}%)`);
      break;
    }
    const r = loadJson(resultsPath);
    const score = r.killRatePercent ?? 0;
    if (score < required) die(`MUTATION FLOOR FAILED: kill rate ${score}% < ${required}%`);
    console.log(`mutation-floor PASS: kill rate ${score}% >= ${required}%`);
    break;
  }
  case 'stub-ban': {
    // 0-scaffolding law: the new-model surfaces (Rust workspace, gate tooling,
    // docs-lint port) may never carry scaffold markers. Legacy TS surfaces are
    // phase-1 refactors and are out of scope here (they are tracked by floors).
    const roots = ['rust/', 'tools/coverage/', 'tools/docs-lint/'];
    const patterns = [
      [/todo!\(/, 'rust todo!()'],
      [/unimplemented!\(/, 'rust unimplemented!()'],
      [/unreachable_unchecked/, 'unsafe unreachable'],
      [/\bTODO\b/, 'TODO marker'],
      [/\bFIXME\b/, 'FIXME marker'],
      [/\bTBD\b/, 'TBD marker'],
      [/lorem ipsum/i, 'lorem ipsum'],
      [/coming soon/i, 'coming soon'],
      [/not implemented/i, 'not-implemented prose'],
      [/\bplaceholder\b/i, 'placeholder marker'],
      [/\bWIP\b/, 'work-in-progress marker'],
    ];
    const allowLine = (line) =>
      // Legitimate uses that are NOT scaffolding: HTML input attributes and CSS
      // pseudo-elements (the read-only console uses real labels next to inputs).
      /placeholder\s*=|::placeholder|placeholder-shown/.test(line);
    const bad = [];
    for (const rel of walkRepo(REPO_ROOT)) {
      if (!roots.some((r) => rel.startsWith(r))) continue;
      // The gate's own pattern table is data, not scaffolding (self-reference).
      if (rel === 'tools/coverage/dogfood-gates.mjs') continue;
      let text;
      try { text = fs.readFileSync(path.join(REPO_ROOT, rel), 'utf8'); } catch { continue; }
      for (const [i, line] of text.split('\n').entries()) {
        if (allowLine(line)) continue;
        for (const [re, name] of patterns) {
          if (re.test(line)) bad.push(`${rel}:${i + 1}: [${name}] ${line.trim().slice(0, 120)}`);
        }
      }
    }
    if (bad.length) die(`STUB-BAN FAILED (${bad.length} scaffold markers):`, bad.slice(0, 60));
    console.log('stub-ban PASS: no scaffolding markers in rust/, tools/coverage/, tools/docs-lint/');
    break;
  }
  default:
    die('usage: dogfood-gates.mjs <orphan-gate|floor-ratchet|floors-sync|assertion-presence|mutation-floor|stub-ban> [flags]');
}
