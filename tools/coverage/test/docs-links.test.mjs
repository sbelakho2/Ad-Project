import test from 'node:test';
import assert from 'node:assert/strict';
import { slugify, headingSlugs, relativeLinks, checkDoc } from '../docs-links.mjs';

const ctx = (files) => ({
  fileExists: (p) => Object.prototype.hasOwnProperty.call(files, p),
  readDoc: (p) => files[p] ?? null,
});

test('slugify: headings follow the github rules for the cases we use', () => {
  assert.equal(slugify('The audit, in practice'), 'the-audit-in-practice');
  assert.equal(slugify('`code` and *emphasis*'), 'code-and-emphasis');
  assert.equal(slugify('A/B testing (v2)'), 'ab-testing-v2');
});

test('relativeLinks: external and protocol links are out of scope', () => {
  const md = '[a](https://x) [b](mailto:x@y) [c](./doc.md#h) [d](#local)';
  assert.deepEqual(relativeLinks(md), ['./doc.md#h', '#local']);
});

test('checkDoc: missing file, missing anchor in target, missing local anchor', () => {
  const files = { 'docs/ok.md': '# Present\n' };
  const md = '[x](./missing.md) [y](./ok.md#wrong) [z](#nope)';
  const v = checkDoc('docs/a.md', md, ctx(files));
  assert.equal(v.length, 3);
  assert.ok(v.some((m) => m.includes('link target missing: ./missing.md')));
  assert.ok(v.some((m) => m.includes("anchor '#wrong' not found in docs/ok.md")));
  assert.ok(v.some((m) => m.includes("anchor '#nope' has no matching heading")));
});

test('checkDoc: resolving links and anchors pass', () => {
  const files = {
    'docs/ok.md': '# Present\n## Deep Section\n',
    'docs/b.md': '[x](./ok.md#present) [y](./ok.md#deep-section) [z](#own)\n## Own\n',
  };
  const v = checkDoc('docs/b.md', files['docs/b.md'], ctx(files));
  assert.deepEqual(v, []);
});

test('headingSlugs: raw lowercase fallback accepts non-slugged anchors', () => {
  const slugs = headingSlugs('## Mixed CASE Heading!\n');
  assert.ok(slugs.has('mixed-case-heading'));
  assert.ok(slugs.has('mixed case heading!'));
});
