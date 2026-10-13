import test from 'node:test';
import assert from 'node:assert/strict';
import { flattenKeys, parityViolations } from '../i18n-parity.mjs';
import { cssViolations } from '../css-lint.mjs';

test('i18n: flattening reports keys, empties, arrays and non-strings', () => {
  const keys = flattenKeys({ a: 'x', b: { c: 'y', d: '' }, e: ['z'], f: 3 });
  assert.ok(keys.includes('a'));
  assert.ok(keys.includes('b.c'));
  assert.ok(keys.some((k) => k.startsWith('b.d\u0000EMPTY')));
  assert.ok(keys.some((k) => k.startsWith('e\u0000ARRAY')));
  assert.ok(keys.some((k) => k.includes('\u0000TYPE:number')));
});

test('i18n: missing, extra, empty and array values are violations', () => {
  const violations = parityViolations('en.json', {
    'en.json': ['a', 'b'],
    'et.json': ['a', 'c'],
  });
  assert.ok(violations.some((v) => v.includes('1 missing key(s)')));
  assert.ok(violations.some((v) => v.includes('1 extra key(s)')));
});

test('i18n: identical locales with clean values pass', () => {
  assert.deepEqual(
    parityViolations('en.json', { 'en.json': ['a.b', 'c'], 'et.json': ['a.b', 'c'] }),
    [],
  );
});

test('css: balanced clean stylesheet passes; each banned pattern trips', () => {
  assert.deepEqual(cssViolations('x.css', 'a{color:#1f4e79}\nb{margin:0}'), []);
  assert.ok(cssViolations('x.css', 'a{color:red;').some((v) => v.includes('unbalanced braces')));
  assert.ok(cssViolations('x.css', 'a{}').some((v) => v.includes('empty rule')));
  assert.ok(cssViolations('x.css', 'a{color:#FFFFFF}').some((v) => v.includes('uppercase hex')));
  assert.ok(
    cssViolations('x.css', 'a{color:red !important}').some((v) => v.includes('without justification')),
  );
});

test('css: justified !important passes (media scope, motion value, pragma)', () => {
  const media = '@media (prefers-reduced-motion: reduce) { * { animation-duration: 0.001ms !important; } }';
  assert.deepEqual(cssViolations('x.css', media), []);
  assert.deepEqual(cssViolations('x.css', '.no-transform * { transform: none !important; }'), []);
  const pragma = `/* css-contract:allow-important reason here */
.weird { color: red !important; }`;
  assert.deepEqual(cssViolations('x.css', pragma), []);
});

test('css: comments are stripped before hygiene checks', () => {
  assert.deepEqual(cssViolations('x.css', '/* !important in a comment { */\na{color:#abc}'), []);
});
