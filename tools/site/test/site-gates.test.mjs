import test from 'node:test';
import assert from 'node:assert/strict';
import { checkPage, checkDist, HTML_BUDGET_BYTES } from '../perf-budget.mjs';
import { normalize, checkClaims, checkPages } from '../claims-contract.mjs';

const goodPage = `<!doctype html><html lang="en"><head><title>x</title>
<meta name="description" content="d"><link rel="canonical" href="https://a/">
<link rel="stylesheet" href="/style.css"></head><body>ok</body></html>`;

test('perf: a clean small page passes', () => {
  assert.deepEqual(checkPage('index.html', goodPage), []);
});

test('perf: script tag is a violation even when tiny', () => {
  const v = checkPage('x.html', goodPage.replace('</body>', '<script>1</script></body>'));
  assert.ok(v.some((m) => m.includes('<script>')));
});

test('perf: oversized page violates the byte budget at the boundary +1', () => {
  const big = goodPage.replace('ok', 'o'.repeat(HTML_BUDGET_BYTES));
  assert.ok(checkPage('big.html', big).some((m) => m.includes('bytes > html budget')));
});

test('perf: missing meta description and canonical are flagged', () => {
  const bare = '<!doctype html><html><head></head><body></body></html>';
  const v = checkPage('b.html', bare);
  assert.ok(v.some((m) => m.includes('meta description')));
  assert.ok(v.some((m) => m.includes('canonical')));
});

test('perf: external stylesheet request is rejected', () => {
  const v = checkPage('x.html', goodPage.replace('/style.css', 'https://cdn.example/x.css'));
  assert.ok(v.some((m) => m.includes('external')));
});

test('perf: dist-level budgets count pages and css', () => {
  const pages = new Map(Array.from({ length: 6 }, (_, i) => [`p${i}.html`, goodPage]));
  assert.deepEqual(checkDist(pages, 1_000), []);
  assert.ok(checkDist(pages, 999_999).some((m) => m.includes('stylesheets total')));
  assert.ok(checkDist(new Map([['only.html', goodPage]]), 0).some((m) => m.includes('pages')));
});

test('claims: normalize collapses whitespace and case', () => {
  assert.equal(normalize(' A  b\n C '), 'a b c');
});

test('claims: missing anchor path and missing test token are violations', () => {
  const claims = {
    claims: [
      { id: 'a1', claim: 'we do x', anchor: 'no/such/file.rs', test: 't' },
      { id: 'a2', claim: 'we do y', anchor: 'exists.rs', test: 'settle_keeps_positive_lift' },
    ],
  };
  const v = checkClaims(claims, {
    exists: (p) => p === 'exists.rs',
    readFile: () => 'fn other_test() {}',
  });
  assert.ok(v.some((m) => m.startsWith('a1: anchor path does not exist')));
  assert.ok(v.some((m) => m.includes("pinned test 'settle_keeps_positive_lift' not found")));
});

test('claims: duplicate ids are violations', () => {
  const claims = { claims: [
    { id: 'd', claim: 'x', anchor: 'e.rs', test: 't' },
    { id: 'd', claim: 'y', anchor: 'e.rs', test: 't' },
  ] };
  const v = checkClaims(claims, { exists: () => true, readFile: () => 't' });
  assert.ok(v.some((m) => m.includes('duplicate claim id')));
});

test('claims: claim text missing from pages is a violation; present passes', () => {
  const claims = { claims: [{ id: 'p', claim: 'No SDK in your app.', anchor: 'a', test: 't' }] };
  assert.ok(checkPages(claims, ['unrelated']).some((m) => m.includes('not found on any page')));
  assert.deepEqual(checkPages(claims, ['intro: No SDK in   your app. indeed']), []);
});

test('claims: empty claims array is a violation (silence is not evidence)', () => {
  assert.deepEqual(checkClaims({ claims: [] }, { exists: () => true, readFile: () => '' }), [
    'claims.json has no claims array',
  ]);
});
