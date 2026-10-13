// coverage-lib.mjs — shared walk/classify/parse logic for the coverage gate stack.
// Zero dependencies; Node >= 18. This module is itself covered 100% by tools/coverage/test/ (M-8).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

// Closed set of exclusion reason codes (design doc 0.3).
export const EXCLUSION_CODES = ['VENDOR', 'GENERATED', 'BUILD_ARTIFACT', 'SAMPLE_APP', 'DATA_ARTIFACT', 'NON_EXECUTABLE'];

export const SKIP_DIRS = new Set([
  'node_modules', '.git', 'dist', '.next', 'build', '.gradle', '.dart_tool',
  '.build', '.pytest_cache', '.mypy_cache', '.ruff_cache', '.nyc_output', 'Pods',
  'DerivedData', 'test-results', 'playwright-report', 'artifacts', '.idea', '.kilo',
  '.artifacts', '_out', '_tmp', '.gradle-dist', '.gradle-home', 'shell-cov',
  'obj', 'bin',
]);

// File-pattern exclusions applied AFTER domain classification; each must carry a reason code.
// NOTE: classify() checks these BEFORE domains, so keep them specific.
const EXCLUSIONS = [
  { glob: '**/package-lock.json', code: 'BUILD_ARTIFACT' },
  { glob: '**/go.sum', code: 'BUILD_ARTIFACT' },
  { glob: '**/*.lock', code: 'BUILD_ARTIFACT' },
  // TypeScript declaration files are generated surface, never executable logic.
  { glob: '**/*.d.ts', code: 'GENERATED' },
  { glob: 'backend/src/types/**', code: 'GENERATED' },
  { glob: '**/*.pb.go', code: 'GENERATED' },
  { glob: '**/*_pb2.py', code: 'GENERATED' },
  { glob: '**/*.min.js', code: 'GENERATED' },
  { glob: '**/bundle-size-report.json', code: 'BUILD_ARTIFACT' },
  { glob: 'coverage-manifest.json', code: 'BUILD_ARTIFACT' },
  { glob: 'coverage-report.md', code: 'BUILD_ARTIFACT' },
  { glob: '**/lcov.info', code: 'BUILD_ARTIFACT' },
  { glob: '**/coverage/**', code: 'BUILD_ARTIFACT' },
  { glob: 'backend/test-results.json', code: 'BUILD_ARTIFACT' },
  { glob: 'backend/test_output.txt', code: 'BUILD_ARTIFACT' },
  { glob: '**/*.svg', code: 'NON_EXECUTABLE' },
  { glob: '**/*.png', code: 'NON_EXECUTABLE' },
  { glob: '**/*.jpg', code: 'NON_EXECUTABLE' },
  { glob: '**/*.jpeg', code: 'NON_EXECUTABLE' },
  { glob: '**/*.gif', code: 'NON_EXECUTABLE' },
  { glob: '**/*.webp', code: 'NON_EXECUTABLE' },
  { glob: '**/*.ico', code: 'NON_EXECUTABLE' },
  { glob: '**/*.woff2', code: 'NON_EXECUTABLE' },
  { glob: '**/*.pdf', code: 'NON_EXECUTABLE' },
  { glob: '**/*.parquet', code: 'DATA_ARTIFACT' },
  { glob: '**/*.onnx', code: 'DATA_ARTIFACT' },
  { glob: '**/*.pkl', code: 'DATA_ARTIFACT' },
  { glob: '**/*.csv', code: 'DATA_ARTIFACT' },
  { glob: '**/*.jsonl', code: 'DATA_ARTIFACT' },
  { glob: '**/*.txt', code: 'NON_EXECUTABLE' },
  { glob: 'data/**/*.json', code: 'DATA_ARTIFACT' },
  { glob: 'models/**/*.json', code: 'DATA_ARTIFACT' },
  { glob: '**/*.gitkeep', code: 'NON_EXECUTABLE' },
  { glob: '**/.gitignore', code: 'NON_EXECUTABLE' },
  { glob: '**/.dockerignore', code: 'NON_EXECUTABLE' },
  { glob: '**/CODEOWNERS', code: 'NON_EXECUTABLE' },
  { glob: '**/.env.example', code: 'NON_EXECUTABLE' },
  { glob: '**/.env.local.example', code: 'NON_EXECUTABLE' },
  { glob: '**/*.sln', code: 'NON_EXECUTABLE' },
  { glob: 'sdk/**/src/main/res/**', code: 'NON_EXECUTABLE' },
  { glob: '**/*.docx', code: 'NON_EXECUTABLE' },
  { glob: 'docs/**/*.log', code: 'NON_EXECUTABLE' },
  { glob: 'docs/**/*.html', code: 'SAMPLE_APP' },
  { glob: 'docs/**/*.json', code: 'DATA_ARTIFACT' },
  { glob: '**/.last_dataset_fingerprint', code: 'DATA_ARTIFACT' },
  { glob: '.husky/_/**', code: 'NON_EXECUTABLE' },
];

// Domain classification. Patterns are **-globs evaluated from repo root.
// kind: 'code' (needs a covering suite) | 'contract' (needs a contract suite) | 'docs' | 'test'.
// ORDER MATTERS: first match wins. Test code is classified before code domains so that
// in-source test trees (src/**/__tests__/, src/test/, androidTest/) never receive
// line-coverage obligations; docs.md patterns precede contract patterns so a README
// inside infrastructure/helm/ is a doc, not an IaC artifact.
export const DOMAINS = [
  // --- test code: meta-audited, not line-covered (M-7 / meta suite) ---
  { id: 'meta-test', kind: 'test', include: ['**/*.test.ts', '**/*.test.tsx', '**/*.test.js', '**/*.test.jsx', '**/*.test.mjs', '**/*.spec.ts', '**/*.spec.js', '**/*.spec.mjs', '**/*_test.go', '**/test_*.py', '**/*_test.py', '**/__tests__/**', '**/tests/**', '**/Tests/**', '**/src/test/**', '**/src/androidTest/**', 'sdk/flutter/test/**', '**/*.bats', '**/__mocks__/**', '**/*.stories.tsx', '**/*.stories.ts', '**/*.stories.jsx', '**/playwright*.config.ts', '**/jest.setup.ts', '**/setupTests.ts', '**/.storybook/**', '**/docs/**/*.tsx'] },
  // --- sample apps: validated by build+smoke contract (S-SAMPLE), not line coverage ---
  { id: 'sample-app', kind: 'contract', include: ['sdk/samples/**', 'sdk/adapter-samples/**', 'sdk/core/ios/Demo/**', 'sdk/**/Demo/**', 'sdk/react-native/example/**', 'Test Apps/**'] },
  // --- code domains ---
  { id: 'backend', kind: 'code', include: ['backend/src/**/*.ts', 'backend/routes/**/*.ts', 'backend/services/**/*.ts', 'backend/scripts/**/*.ts'], exclude: ['backend/src/openapi/**'] },
  { id: 'backend-go', kind: 'code', include: ['backend/auction/**/*.go', 'backend/config/**/*.go', 'backend/payments/**/*.go', 'backend/router/**/*.go'], exclude: ['**/*_test.go'] },
  { id: 'console', kind: 'code', include: ['console/src/**/*.ts', 'console/src/**/*.tsx', 'console/scripts/**/*.ts'], exclude: ['console/src/tests/**'] },
  { id: 'website', kind: 'code', include: ['website/src/**/*.ts', 'website/src/**/*.tsx', 'website/src/**/*.js', 'website/src/**/*.jsx'] },
  { id: 'control-plane', kind: 'code', include: ['control-plane/src/**/*.ts', 'control-plane/scripts/**/*.ts', 'control-plane/test-*.ts'] },
  { id: 'edge-worker', kind: 'code', include: ['edge-worker/src/**/*.ts'] },
  { id: 'autopilot', kind: 'code', include: ['services/autopilot/src/**/*.ts'] },
  { id: 'web-sdk', kind: 'code', include: ['sdk/web/src/**/*.ts'] },
  { id: 'react-native-sdk', kind: 'code', include: ['sdk/react-native/src/**/*.tsx', 'sdk/react-native/src/**/*.ts'] },
  { id: 'unity-tier1', kind: 'code', include: ['sdk/core/unity/Runtime/**/*.cs', 'sdk/core/unity/Editor/**/*.cs'] },
  { id: 'ml', kind: 'code', include: ['ML/__init__.py', 'ML/src/**/*.py', 'ML/scripts/**/*.py'] },
  { id: 'py-scripts', kind: 'code', include: ['scripts/**/*.py', 'tools/**/*.py', 'sitecustomize.py', 'sdk/**/*.py'] },
  { id: 'fraud-inference', kind: 'code', include: ['services/fraud-inference/**/*.py'] },
  { id: 'inference-ml', kind: 'code', include: ['services/inference-ml/**/*.py'] },
  { id: 'mail-server', kind: 'code', include: ['services/mail-server/**/*.rs'] },
  { id: 'apex-core', kind: 'code', include: ['rust/crates/apex-core/**/*.rs'] },
  { id: 'apex-connectors', kind: 'code', include: ['rust/crates/apex-connectors/**/*.rs'] },
  { id: 'apex-audit', kind: 'code', include: ['rust/crates/apex-audit/**/*.rs'] },
  { id: 'apex-proof', kind: 'code', include: ['rust/crates/apex-proof/**/*.rs'] },
  { id: 'apex-autopilot', kind: 'code', include: ['rust/crates/apex-autopilot/**/*.rs'] },
  { id: 'apex-billing', kind: 'code', include: ['rust/crates/apex-billing/**/*.rs'] },
  { id: 'apex-api', kind: 'code', include: ['rust/crates/apex-api/**/*.rs'] },
  { id: 'android-core', kind: 'code', include: ['sdk/core/android/src/main/**/*.kt', 'sdk/core/android/strictmode-sample/src/main/**/*.kt'] },
  { id: 'android-ctv', kind: 'code', include: ['sdk/ctv/android-tv/src/main/**/*.kt'] },
  { id: 'ios-swift', kind: 'code', include: ['sdk/core/ios/Sources/**/*.swift', 'sdk/adapter-dev-kit/ios/Sources/**/*.swift'] },
  { id: 'tvos-swift', kind: 'code', include: ['sdk/ctv/tvos/Sources/**/*.swift'] },
  { id: 'flutter', kind: 'code', include: ['sdk/flutter/lib/**/*.dart'] },
  { id: 'go-quality', kind: 'code', include: ['quality/**/*.go'], exclude: ['**/*_test.go'] },
  { id: 'js-tools', kind: 'code', include: ['tools/**/*.js', 'backend/*.js', 'backend/scripts/**/*.js', 'control-plane/scripts/**/*.js', 'quality/**/*.js', 'scripts/**/*.js', 'console/scripts/*.js', 'tools/coverage/**/*.mjs'], exclude: ['**/*.min.js'] },
  { id: 'shell', kind: 'code', include: ['**/*.sh', '.husky/pre-commit'] },
  // py-scripts second block: none (scripts/**/*.py above already covers scripts/ml)
  // --- docs: link/reference contract ---
  { id: 'docs', kind: 'docs', include: ['**/*.md'] },
  // --- contract domains ---
  { id: 'contract-sql', kind: 'contract', include: ['backend/migrations/**/*.sql', 'backend/database/migrations/**/*.sql', 'backend/scripts/**/*.sql', 'backend/services/**/*.sql', 'control-plane/migrations/**/*.sql', 'tools/sql/**/*.sql', 'scripts/**/*.sql', 'data/**/*.sql', 'services/mail-server/crates/**/migrations/**/*.sql'] },
  { id: 'contract-iac', kind: 'contract', include: ['monitoring/**/*.{yml,yaml,json}', 'infrastructure/**/*.{yml,yaml,toml,conf,service,tf}', 'infrastructure/**/grafana/**/*.json', '**/Dockerfile*', '**/docker-compose*.yml', '**/docker-compose*.yaml', '.github/**/*.yml', '.github/**/*.json', '**/.github/workflows/*.yml', '.woodpecker/*.yml', '**/helm/**', '**/*.service', '**/*.service.example', '**/*.timer.example', 'infrastructure/database/cron/*', 'ML/**/*.yaml', '**/*.proto', 'scripts/**/*.conf', '**/AndroidManifest.xml', '**/link.xml', '**/Caddyfile', 'docs/**/evidence/**/*.{yml,yaml}'] },
  { id: 'contract-openapi', kind: 'contract', include: ['backend/src/openapi/**/*.yaml', 'backend/openapi.yaml', '**/openapi*.yaml'] },
  { id: 'contract-i18n', kind: 'contract', include: ['**/i18n/**/*.json', '**/messages/**/*.json'] },
  { id: 'contract-css', kind: 'contract', include: ['**/*.css', '**/*.scss'] },
  { id: 'contract-build', kind: 'contract', include: ['**/package.json', '**/build.gradle', '**/settings.gradle', '**/gradle.properties', '**/*.properties', '**/*.pro', '**/Package.swift', '**/Cargo.toml', '**/*.toml', '**/pubspec.yaml', '**/*.csproj', '**/*.asmdef', '**/Makefile', 'Makefile', '**/tsconfig*.json', 'pytest.ini', '**/pytest.ini', 'pyproject.toml', '**/pyproject.toml', '**/go.mod', 'tools/coverage/*', '**/*.config.{js,cjs,mjs,ts}', '**/.eslintrc.{js,cjs,json,yml,yaml}', '.husky/*'] },
];

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
      } else if (entry.isFile()) {
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
