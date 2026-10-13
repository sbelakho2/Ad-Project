// coverage-lib.mjs — shared walk/classify/parse logic for the coverage gate stack.
// Zero dependencies; Node >= 18. This module is itself covered 100% by tools/coverage/test/ (M-8).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

// Closed set of exclusion reason codes (design doc 0.3), loaded from policies.json.

export const SKIP_DIRS = new Set([
  'node_modules', '.git', 'dist', '.next', 'build', '.gradle', '.dart_tool',
  '.build', '.pytest_cache', '.mypy_cache', '.ruff_cache', '.nyc_output', 'Pods',
  'DerivedData', 'test-results', 'playwright-report', 'artifacts', '.idea', '.kilo',
  '.gradle-dist', '.gradle-home', 'shell-cov',
  'lhci-report', '.lighthouseci', 'chrome', 'chromium',
  'obj', 'bin',
]);

// Policy tables live in policies.json (data): the classifier reads them at load
// time and stack-integrity tests prove the runtime behaviour matches expectations.
const policies = JSON.parse(
  fs.readFileSync(new URL('./policies.json', import.meta.url), 'utf8'),
);
export const DOMAINS = policies.domains;
export const EXCLUSION_CODES = policies.exclusionCodes;
const EXCLUSIONS = policies.exclusions;

// --- glob compiler ---
// Semantics (documented + tested): '**/' may span zero or more directories,
// '*' never crosses '/', '{a,b}' expands alternatives, patterns anchor at repo root.
export function globToRegExp(glob) {
  const alts = [];
  const withPlaceholders = glob.replace(/\{([^{}]+)\}/g, (_, inner) => {
    alts.push(`(?:${inner.split(',').join('|')})`);
    return `\u0001${alts.length - 1}\u0002`;
  });
  const esc = withPlaceholders
    .replace(/[.+^$?()|[\]{}\\]/g, '\\$&')
    .replace(/\*\*/g, '\u0003')
    .replace(/\*/g, '[^/]*')
    .replace(/\u0003\//g, '(?:.*/)?')
    .replace(/\u0003/g, '.*')
    .replace(/\u0001(\d+)\u0002/g, (_, i) => alts[Number(i)]);
  return new RegExp(`^${esc}$`);
}

const compiledExclusions = EXCLUSIONS.map((e) => ({ ...e, re: globToRegExp(e.glob) }));
const compiledDomains = DOMAINS.map((d) => ({
  ...d,
  includeRes: d.include.map(globToRegExp),
  excludeRes: (d.exclude ?? []).map(globToRegExp),
}));

export function classify(relPath) {
  // tools/coverage is the gate stack itself: first-party code, never build output.
  const isGateStack = relPath.startsWith('tools/coverage/');
  if (!isGateStack) {
    for (const ex of compiledExclusions) {
      if (ex.re.test(relPath)) return { domain: null, kind: null, status: 'excluded', reason: ex.code };
    }
  }
  for (const d of compiledDomains) {
    if (!d.includeRes.some((re) => re.test(relPath))) continue;
    if (d.excludeRes.some((re) => re.test(relPath))) continue;
    return { domain: d.id, kind: d.kind, status: d.kind === 'code' ? 'planned' : d.kind };
  }
  return { domain: null, kind: null, status: 'unclassified' };
}

export function walkRepo(root = REPO_ROOT) {
  const out = [];
  const rec = (dir, rel) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const relChild = rel ? `${rel}/${entry.name}` : entry.name;
      if (entry.isDirectory()) {
        if (SKIP_DIRS.has(entry.name) || entry.name === 'target' || entry.name === 'vendor') {
          // tools/coverage is first-party code: only skip build-output "coverage" dirs.
          if (!(entry.name === 'coverage' && relChild === 'tools/coverage')) continue;
        }
        rec(path.join(dir, entry.name), relChild);
      } else if (entry.isFile() || entry.isSymbolicLink()) {
        // Symlinks are included: a tracked symlink the walker ignored would be
        // invisible to the manifest (walker-parity proves none are).
        out.push(relChild);
      }
    }
  };
  rec(root, '');
  return out.sort();
}

// ---- coverage artifact parsers ----

export function parseJestSummary(jsonText) {
  const j = JSON.parse(jsonText);
  const t = j.total;
  if (!t) throw new Error('coverage-summary.json missing "total"');
  return {
    lines: t.lines?.pct, branches: t.branches?.pct, functions: t.functions?.pct, statements: t.statements?.pct,
  };
}

export function parseLcov(text) {
  const files = [];
  let cur = null;
  for (const line of text.split('\n')) {
    if (line.startsWith('SF:')) cur = { file: line.slice(3).trim(), LH: 0, LF: 0, BRH: 0, BRF: 0, FNF: 0, FNH: 0 };
    else if (cur && line.startsWith('LF:')) cur.LF = Number(line.slice(3));
    else if (cur && line.startsWith('LH:')) cur.LH = Number(line.slice(3));
    else if (cur && line.startsWith('BRF:')) cur.BRF = Number(line.slice(4));
    else if (cur && line.startsWith('BRH:')) cur.BRH = Number(line.slice(4));
    else if (cur && line.startsWith('FNF:')) cur.FNF = Number(line.slice(4));
    else if (cur && line.startsWith('FNH:')) cur.FNH = Number(line.slice(4));
    else if (line.startsWith('end_of_record')) { files.push(cur); cur = null; }
  }
  return files;
}

// requireBranches is FAIL-CLOSED: when requested but the artifact carries no branch
// records at all (BRF totals 0), the gate must fail rather than vacuously pass.
export function lcovTotals(files, { requireBranches = false } = {}) {
  const sum = (key) => files.reduce((a, f) => a + f[key], 0);
  const LF = sum('LF'), LH = sum('LH'), BRF = sum('BRF'), BRH = sum('BRH'), FNF = sum('FNF'), FNH = sum('FNH');
  const pct = (h, t) => (t === 0 ? 100 : (h / t) * 100);
  const violations = [];
  if (requireBranches && BRF === 0) violations.push('no branch data (BRDA/BRF) recorded although --require-branches was requested');
  return {
    lines: pct(LH, LF),
    branches: requireBranches || BRF > 0 ? pct(BRH, BRF) : 100,
    functions: pct(FNH, FNF),
    branchRecords: BRF,
    zeroLineFiles: files.filter((f) => f.LF === 0 && !f.file.endsWith('__init__.py')).map((f) => f.file),
    violations,
  };
}

// `go tool cover -func` output format: `path/file.go:12:\tFuncName\t100.0%` and
// the final summary line `total:\t(statements)\t88.9%`.
export function parseGoFunc(text) {
  const rows = [];
  let total = null;
  for (const line of text.split('\n')) {
    const totalMatch = line.match(/^total:(.*)\t([\d.]+)%\s*$/);
    if (totalMatch) { total = Number(totalMatch[2]); continue; }
    const m = line.match(/^(.+?:[\d,:]*)\t(.+)\t([\d.]+)%\s*$/);
    if (m) rows.push({ func: `${m[2]} @ ${m[1]}`, pct: Number(m[3]) });
  }
  return { rows, total };
}

// Single source for "generated artifact" path patterns used by CI reference checks
// (contract-checks.mjs). Coverage obligations use the DOMAINS/EXCLUSIONS tables above;
// this list answers the different question "can a fresh checkout be expected to contain
// this path that a workflow references?". Keep additions here ONLY, never inline.
export const CI_GENERATED_PATH_PATTERNS = [
  '(^|/)(coverage|dist|build|builds|artifacts|playwright-report|test-results|\\.next|\\.build|node_modules|Library|logs|public-repo|private-repo|docs|typedoc)(/|$)',
  '\\.(log|profdata|xcresult|sarif|gz|zip|tgz|tar|lcov)$',
  '\\.lighthouse',
  '(^|/)(compatibility-matrix\\.txt|compatibility-report\\.md|unity-package-manifest\\.json|k6-results\\.json|eslint-report\\.json|api-diff\\.txt|validation_results\\.json|unity-upm-size-report\\.json)$',
  'Documentation~',
  'quality/perf/reports',
  'website/analyze',
];
