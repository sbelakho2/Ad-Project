import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { headersFor, resolveUnderDist } from '../serve-dist.mjs';

test('headers: css is cacheable, gzip-eligible; html revalidates', () => {
  const css = headersFor('/x/style.css', { gzipAccepted: true });
  assert.equal(css.headers['Content-Type'], 'text/css; charset=utf-8');
  assert.equal(css.headers['Cache-Control'], 'public, max-age=86400');
  assert.equal(css.headers['Content-Encoding'], 'gzip');
  const html = headersFor('/x/index.html', { gzipAccepted: false });
  assert.equal(html.headers['Cache-Control'], 'public, max-age=3600');
  assert.equal(html.headers['Content-Encoding'], undefined);
});

test('headers: binary assets are not compressed, still cacheable', () => {
  const ico = headersFor('/x/favicon.ico', { gzipAccepted: true });
  assert.equal(ico.headers['Content-Encoding'], undefined);
  assert.equal(ico.headers['Cache-Control'], 'public, max-age=86400');
  assert.equal(ico.headers['X-Content-Type-Options'], 'nosniff');
});

test('resolve: plain files, query strip, and traversal containment', () => {
  const root = path.resolve('/tmp/dist-test');
  assert.equal(resolveUnderDist(root, '/index.html'), path.join(root, 'index.html'));
  assert.equal(resolveUnderDist(root, '/style.css?cache=1'), path.join(root, 'style.css'));
  // Percent-decoded slash traversal escapes and must be refused outright.
  assert.equal(resolveUnderDist(root, '/..%2Fetc%2Fpasswd'), null);
  assert.equal(resolveUnderDist(root, '/%00'), null);
  // A backslash on POSIX is a literal name, not a separator: whatever comes
  // back must stay inside the dist root.
  const backslash = resolveUnderDist(root, '/..%5Cwindows');
  if (backslash !== null) {
    assert.ok(backslash.startsWith(root + path.sep), `escaped root: ${backslash}`);
  }
});
