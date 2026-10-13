import test from 'node:test';
import assert from 'node:assert/strict';
import { globToRegExp, classify, parseLcov, lcovTotals, parseGoFunc, parseJestSummary } from '../coverage-lib.mjs';

test('globToRegExp: **/ spans zero or more directories (root files match)', () => {
  assert.ok(globToRegExp('backend/src/**/*.ts').test('backend/src/index.ts'));
  assert.ok(globToRegExp('backend/src/**/*.ts').test('backend/src/services/a/b.ts'));
  assert.ok(globToRegExp('**/package.json').test('package.json'));
  assert.ok(globToRegExp('**/package.json').test('backend/package.json'));
  assert.ok(!globToRegExp('backend/src/*.ts').test('backend/src/services/a.ts'));
  assert.ok(globToRegExp('backend/src/*.ts').test('backend/src/index.ts'));
});

test('globToRegExp: braces expand to alternatives', () => {
  const re = globToRegExp('monitoring/**/*.{yml,yaml}');
  assert.ok(re.test('monitoring/alerts.yml'));
  assert.ok(re.test('monitoring/loki-tenant-limits.yaml'));
  assert.ok(re.test('monitoring/grafana/dashboards/x.yml'));
  assert.ok(!re.test('monitoring/grafana/dashboards/x.json'));
});

test('classify: code domain deep paths', () => {
  const c = classify('backend/src/services/billing/x.ts');
  assert.equal(c.domain, 'backend');
  assert.equal(c.kind, 'code');
  assert.equal(c.status, 'planned');
});

test('classify: in-source test trees are meta-audited, not line-covered', () => {
  assert.equal(classify('backend/src/services/__tests__/x.test.ts').domain, 'meta-test');
  assert.equal(classify('backend/tests/helpers/testApp.ts').domain, 'meta-test');
  assert.equal(classify('sdk/core/android/src/test/kotlin/dx/KillSwitchTest.kt').domain, 'meta-test');
  assert.equal(classify('sdk/core/android/src/androidTest/kotlin/x.kt').domain, 'meta-test');
});

test('classify: root-level files are reachable', () => {
  assert.equal(classify('package.json').domain, 'contract-build');
  assert.equal(classify('README.md').domain, 'docs');
  assert.equal(classify('sitecustomize.py').domain, 'py-scripts');
  assert.equal(classify('start-dev.sh').domain, 'shell');
});

test('classify: declaration files and generated surface carry reason codes', () => {
  assert.equal(classify('backend/src/types/foo.d.ts').reason, 'GENERATED');
  assert.equal(classify('console/src/types/next-auth.d.ts').reason, 'GENERATED');
  assert.equal(classify('backend/package-lock.json').reason, 'BUILD_ARTIFACT');
  assert.equal(classify('website/public/logo.png').reason, 'NON_EXECUTABLE');
  assert.equal(classify('models/fraud/latest/fraud_model.onnx').reason, 'DATA_ARTIFACT');
  assert.equal(classify('quality/go.sum').reason, 'BUILD_ARTIFACT');
});

test('classify: contract domains stay precise', () => {
  assert.equal(classify('backend/migrations/postgres/20251111_104800_init_baseline.up.sql').domain, 'contract-sql');
  assert.equal(classify('monitoring/alerts.yml').domain, 'contract-iac');
  assert.equal(classify('monitoring/grafana/dashboards/x.json').domain, 'contract-iac');
  assert.equal(classify('Dockerfile.ml').domain, 'contract-iac');
  assert.equal(classify('backend/Dockerfile').domain, 'contract-iac');
  assert.equal(classify('backend/src/openapi/billing.yaml').domain, 'contract-openapi');
  assert.equal(classify('console/src/i18n/messages/en.json').domain, 'contract-i18n');
  assert.equal(classify('services/mail-server/crates/mail-proto/proto/mail.proto').domain, 'contract-iac');
  assert.equal(classify('sdk/samples/ios/ApexSandboxiOS/App.swift').domain, 'sample-app');
  assert.equal(classify('infrastructure/helm/backend/templates/deployment.yaml').domain, 'contract-iac');
  assert.equal(classify('infrastructure/helm/backend/README.md').domain, 'docs');
});

test('classify: workspace-root config files are contract-build', () => {
  assert.equal(classify('console/jest.coverage.config.js').domain, 'contract-build');
  assert.equal(classify('website/next.config.js').domain, 'contract-build');
  assert.equal(classify('sdk/web/tsconfig.json').domain, 'contract-build');
  assert.equal(classify('sdk/core/android/gradle.properties').domain, 'contract-build');
  assert.equal(classify('backend/scripts/testEndpoints.js').domain, 'js-tools');
});

test('parseLcov: totals computed and zero-line files reported', () => {
  const lcov = [
    'SF:a.ts', 'LF:10', 'LH:10', 'BRF:4', 'BRH:2', 'FNF:3', 'FNH:3', 'end_of_record',
    'SF:b.ts', 'LF:10', 'LH:5', 'BRF:2', 'BRH:2', 'FNF:1', 'FNH:0', 'end_of_record',
  ].join('\n');
  const files = parseLcov(lcov);
  assert.equal(files.length, 2);
  const t = lcovTotals(files, { requireBranches: true });
  assert.equal(t.lines, 75);
  assert.ok(Math.abs(t.branches - 66.6666) < 0.01); // 4/6 branch hits
  assert.equal(t.functions, 75); // 3/4 function hits
  assert.equal(t.violations.length, 0);
});

test('lcovTotals: requireBranches fails closed when no branch records exist', () => {
  const files = parseLcov('SF:a.ts\nLF:10\nLH:10\nFNF:1\nFNH:1\nend_of_record');
  const t = lcovTotals(files, { requireBranches: true });
  assert.equal(t.branchRecords, 0);
  assert.ok(t.violations.some((v) => v.includes('no branch data')));
});

test('parseGoFunc: real tab-separated go tool cover -func output', () => {
  const { rows, total } = parseGoFunc(
    'github.com/rivalapexmediation/auction/internal/bidding/engine.go:42:\tEngine.Hedge\t85.7%\ntotal:\t(statements)\t92.3%'
  );
  assert.equal(rows.length, 1);
  assert.equal(rows[0].pct, 85.7);
  assert.equal(total, 92.3);
});

test('parseJestSummary: reads totals', () => {
  const t = parseJestSummary(JSON.stringify({
    total: { lines: { pct: 100 }, branches: { pct: 99.1 }, functions: { pct: 100 }, statements: { pct: 100 } },
  }));
  assert.equal(t.lines, 100);
  assert.equal(t.branches, 99.1);
});
