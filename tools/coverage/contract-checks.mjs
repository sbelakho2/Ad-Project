#!/usr/bin/env node
// contract-checks.mjs — S-CI / S-OPENAPI / S-CONTENT / S-SAMPLE / S-DOCS adversarial contract checks.
// Subcommands: ci-reference-integrity | openapi-parity | content-guard-adversarial | samples | docs-references
//
// ci-reference-integrity ratchet (tools/coverage/ci-ref-baseline.json):
//   - missing refs listed in the baseline (legacy workflow debt, phase-1 remediation) are tolerated;
//   - NEW missing refs fail;
//   - baseline entries that are no longer missing FAIL ("stale entry") so the baseline can only shrink.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { walkRepo, REPO_ROOT, CI_GENERATED_PATH_PATTERNS } from './coverage-lib.mjs';

const [cmd] = process.argv.slice(2);
function die(msg, extra = []) { console.error(`contract gate failed [${cmd}] ${msg}`); for (const e of extra) console.error(`  - ${e}`); process.exit(1); }
const exists = (p) => fs.existsSync(path.join(REPO_ROOT, p));

// Generated-at-build paths that are legitimately absent in a fresh checkout.
// Pattern list is single-sourced in coverage-lib.mjs (CI_GENERATED_PATH_PATTERNS).
const GENERATED_ARTIFACT = new RegExp(CI_GENERATED_PATH_PATTERNS.join('|'));

// Extract path refs from workflow yaml: only structured fields, never free-text tokens.
function extractPathRefs(rel, text) {
  const refs = [];
  const add = (raw) => {
    if (!raw) return;
    const p = raw.replace(/^["']|["']$/g, '').split('${')[0].split('{{')[0].trim();
    if (!p || p.startsWith('$') || p === '.') return;
    if (/^[|>][+-]?$/.test(p)) return; // yaml block scalar indicators (run: |)
    if (GENERATED_ARTIFACT.test(p)) return;
    refs.push({ raw: p, rel });
  };
  for (const m of text.matchAll(/^\s*working-directory:\s*(\S+)\s*$/gm)) add(m[1]);
  for (const m of text.matchAll(/^\s*-\s*(dir|file|source|target):\s*(\S+)\s*$/gm)) add(m[2]);
  for (const m of text.matchAll(/^\s*(paths?|files):\s*(\S+)\s*$/gm)) add(m[2]);
  // path-filter lists: "  - 'backend/**'" (quoted glob entries)
  for (const m of text.matchAll(/^\s*-\s*'([A-Za-z0-9_.\-/]+\*\*[A-Za-z0-9_.\-/*]*)'\s*$/gm)) {
    const p = m[1].replace(/\/\*\*$/, '').replace(/\*\*.*$/, '');
    if (p && !GENERATED_ARTIFACT.test(p)) refs.push({ raw: p, rel });
  }
  // bare path: values (checkout/download/upload actions)
  for (const m of text.matchAll(/^\s*path:\s*(\S+)\s*$/gm)) add(m[1]);
  return refs;
}

switch (cmd) {
  case 'ci-reference-integrity': {
    const baselinePath = path.join(REPO_ROOT, 'tools/coverage/ci-ref-baseline.json');
    const baseline = fs.existsSync(baselinePath) ? JSON.parse(fs.readFileSync(baselinePath, 'utf8')) : { knownMissing: {} };
    const known = new Set(Object.keys(baseline.knownMissing));
    const missing = [];
    const yamlFiles = walkRepo(REPO_ROOT).filter((p) => /\.ya?ml$/.test(p) && (p.startsWith('.github/workflows/') || p.startsWith('.woodpecker/')));
    for (const rel of yamlFiles) {
      for (const ref of extractPathRefs(rel, fs.readFileSync(path.join(REPO_ROOT, rel), 'utf8'))) {
        // Path filters describe repo subtrees; require the static prefix to exist.
        if (!exists(ref.raw) && !exists(ref.raw.replace(/\/$/, ''))) missing.push(`${ref.rel} -> ${ref.raw}`);
      }
    }
    const key = (m) => m;
    const unexpected = missing.filter((m) => !known.has(key(m)));
    const stillMissing = new Set(missing);
    const stale = [...known].filter((k) => !stillMissing.has(k));
    if (unexpected.length || stale.length) {
      die(`missing refs: ${unexpected.length} new, ${stale.length} stale baseline entries (remove fixed entries from ci-ref-baseline.json)`, [...unexpected, ...stale.map((s) => `[stale] ${s}`)]);
    }
    console.log(`ci-reference-integrity PASS (${yamlFiles.length} workflow files; ${known.size} known-missing baselined for phase-1 remediation)`);
    break;
  }
  case 'openapi-parity': {
    const specPath = 'backend/src/openapi/billing.yaml';
    if (!exists(specPath)) die(`${specPath} missing`);
    const spec = fs.readFileSync(path.join(REPO_ROOT, specPath), 'utf8');
    const paths = [...spec.matchAll(/^\s{2}\/([^\s:]+):/gm)].map((m) => `/${m[1]}`);
    if (paths.length === 0) die('no paths found in billing.yaml (parsed incorrectly?)');
    const backendSrc = walkRepo(REPO_ROOT).filter((p) => /^backend\/(src\/routes|routes|src\/controllers)/.test(p) && /\.ts$/.test(p));
    const codeBlob = backendSrc.map((p) => fs.readFileSync(path.join(REPO_ROOT, p), 'utf8')).join('\n');
    const routeRe = (docPath) => {
      const normalized = docPath.replace(/^\/(v1|api\/v1)\//, '/').replace(/\{[^}]+\}/g, ':param');
      const candidates = [];
      const segments = normalized.split('/').filter(Boolean);
      // Route registrars commonly mount sub-routers, so a documented path may appear in
      // source without its mount prefix (e.g. doc /billing/invoices/{id}/pdf vs
      // router.get('/invoices/:id/pdf') under app.use('/billing', ...)). Try suffixes.
      for (let i = 0; i <= Math.max(0, segments.length - 2); i++) {
        const candidatePath = `/${segments.slice(i).join('/')}`;
        const esc = candidatePath.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/:param/g, ':[A-Za-z_][A-Za-z0-9_]*');
        candidates.push(new RegExp(`['"\`]${esc}['"\`]`));
      }
      return candidates;
    };
    const missing = paths.filter((p) => {
      if (codeBlob.includes(p.replace(/^\/(v1|api\/v1)\//, ''))) return false;
      return !routeRe(p).some((re) => re.test(codeBlob));
    });
    if (missing.length) die('documented OpenAPI paths not found in route/controller sources:', missing);
    console.log(`openapi-parity PASS (${paths.length} documented paths all resolvable in sources)`);
    break;
  }
  case 'content-guard-adversarial': {
    const guardPath = path.join(REPO_ROOT, 'tools/content-guard.js');
    const mod = await import(pathToFileURL(guardPath).href).catch((e) => ({ __error: e }));
    if (mod.__error) die(`tools/content-guard.js could not be imported: ${mod.__error.message}`);
    if (typeof mod.scanText !== 'function' || typeof mod.shouldScanFile !== 'function') {
      die('tools/content-guard.js must export scanText(text) and shouldScanFile(absPath) predicates (dogfood import contract)');
    }
    const cases = [
      ['we use HEROKU for this', true],
      ['V e r c e l', false], // split tokens are NOT matches (documented non-goal: no fuzzy matching)
      ['deployment is on vercel.com', true],
      ['nothing to see here', false],
      ['upstash-redis://xyz', true],
      ['backblaze b2 bucket', true],
    ];
    const failures = [];
    for (const [text, expected] of cases) {
      const got = mod.scanText(text);
      if (got !== expected) failures.push(`scanText(${JSON.stringify(text)}) -> ${got}, expected ${expected}`);
    }
    if (failures.length) die('content guard adversarial matrix failed:', failures);
    console.log(`content-guard-adversarial PASS (${cases.length} payload cases)`);
    break;
  }
  case 'samples': {
    // Structural contract anchored to what each sample actually ships. The
    // spec lives in samples-contract.json; compile execution is the phase-2 lane.
    const specPath = path.join(REPO_ROOT, 'tools/coverage/samples-contract.json');
    const specText = fs.readFileSync(specPath, 'utf8');
    const spec = JSON.parse(specText);
    const failures = [];
    for (const check of spec.shapeChecks) {
      if (!exists(check.dir)) {
        failures.push(`${check.dir}: sample directory missing`);
        continue;
      }
      for (const rel of check.required) {
        if (!exists(`${check.dir}/${rel}`)) failures.push(`${check.dir}: required path missing: ${rel}`);
      }
    }
    if (!exists(spec.web.packageJson)) {
      failures.push(`${spec.web.packageJson}: missing`);
    } else {
      const pkgPath = path.join(REPO_ROOT, spec.web.packageJson);
      const pkgText = fs.readFileSync(pkgPath, 'utf8');
      const pkg = JSON.parse(pkgText);
      const build = pkg.scripts?.build;
      if (typeof build !== 'string' || build.trim() === '') {
        failures.push(`${spec.web.packageJson}: no build script; the sample must be buildable`);
      }
      if (!exists(spec.web.entry)) failures.push(`${spec.web.entry}: missing entry page`);
    }
    if (failures.length) die('sample projects failed the structural contract:', failures);
    console.log(
      `samples contract PASS: ${spec.shapeChecks.length + 1} projects carry their buildable shape; compile execution is the phase-2 lane`,
    );
    break;
  }

  case 'docs-references': {
    const bad = [];
    const pkgJson = ['package.json', 'backend/package.json', 'console/package.json', 'website/package.json', 'control-plane/package.json', 'sdk/web/package.json', 'tools/package.json'];
    const scriptNames = new Set();
    for (const p of pkgJson) {
      if (!exists(p)) continue;
      for (const k of Object.keys(JSON.parse(fs.readFileSync(path.join(REPO_ROOT, p), 'utf8')).scripts ?? {})) scriptNames.add(k);
    }
    for (const rel of walkRepo(REPO_ROOT)) {
      if (!rel.endsWith('.md')) continue;
      const text = fs.readFileSync(path.join(REPO_ROOT, rel), 'utf8');
      for (const m of text.matchAll(/`npm run ([a-z0-9:_-]+)`/g)) {
        if (!scriptNames.has(m[1])) bad.push(`${rel}: references npm script '${m[1]}' which exists nowhere`);
      }
    }
    if (bad.length) die('docs reference nonexistent scripts:', bad);
    console.log('docs-reference-contract PASS');
    break;
  }
  default:
    die('usage: contract-checks.mjs <ci-reference-integrity|openapi-parity|content-guard-adversarial|samples|docs-references>');
}
