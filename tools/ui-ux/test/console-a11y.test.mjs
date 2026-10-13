import test from 'node:test';
import assert from 'node:assert/strict';
import { failingViolations, structureViolations } from '../console-a11y.mjs';

const good = `<!doctype html><html lang="en"><head><title>Console</title></head><body><main><h1>Overview</h1></main></body></html>`;

test('structure: a clean page passes', () => {
  assert.deepEqual(structureViolations('overview.html', good), []);
});

test('structure: missing lang, title, wrong h1 count and script tags are violations', () => {
  const v = structureViolations(
    'x.html',
    '<html><head></head><body><h1>a</h1><h1>b</h1><script>1</script></body></html>',
  );
  assert.ok(v.some((m) => m.includes('lang attribute')));
  assert.ok(v.some((m) => m.includes('title')));
  assert.ok(v.some((m) => m.includes('exactly one <h1>')));
  assert.ok(v.some((m) => m.includes('script-free')));
});

test('axe: only critical/serious violations fail the gate', () => {
  const results = {
    violations: [
      { id: 'color-contrast', impact: 'serious', help: 'contrast', nodes: [1, 2] },
      { id: 'region', impact: 'moderate', help: 'landmarks', nodes: [1] },
      { id: 'html-has-lang', impact: 'critical', help: 'lang', nodes: [1] },
    ],
  };
  const failing = failingViolations(results);
  assert.deepEqual(failing.map((v) => v.id).sort(), ['color-contrast', 'html-has-lang']);
});

test('axe: empty results pass', () => {
  assert.deepEqual(failingViolations({}), []);
});
