#!/usr/bin/env node
// serve-dist.mjs — the audit-time static server for the built Zola site.
// It exists because generic one-line servers fail Lighthouse performance
// audits by construction: python http.server sends no Content-Encoding and no
// cache policy, docking `uses-text-compression` and `uses-long-cache-ttl`
// before any page content is judged. This server is deterministic instead:
// - gzip for text assets when the client accepts it,
// - one documented cache policy (day-long max-age for assets, revalidating HTML),
// - path traversal refused, directories resolve to index.html.
//
// Usage: node tools/site/serve-dist.mjs <dist-dir> <port>
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';

const ASSET_MAX_AGE = 60 * 60 * 24; // 1 day; assets are unhashed so keep it modest
const HTML_MAX_AGE = 3_600; // 1 hour: static marketing pages; deploys are rare

const TEXT_TYPES = new Map([
  ['.html', 'text/html; charset=utf-8'],
  ['.css', 'text/css; charset=utf-8'],
  ['.js', 'text/javascript; charset=utf-8'],
  ['.json', 'application/json; charset=utf-8'],
  ['.svg', 'image/svg+xml'],
  ['.txt', 'text/plain; charset=utf-8'],
  ['.xml', 'application/xml; charset=utf-8'],
]);

/** Pure: headers for a served file (content type, cache policy, gzip eligibility). */
export function headersFor(filePath, { gzipAccepted }) {
  const ext = path.extname(filePath).toLowerCase();
  const type = TEXT_TYPES.get(ext) ?? 'application/octet-stream';
  const cacheable = ext !== '.html';
  const headers = {
    'Content-Type': type,
    'Cache-Control': cacheable
      ? `public, max-age=${ASSET_MAX_AGE}`
      : `public, max-age=${HTML_MAX_AGE}`,
    'X-Content-Type-Options': 'nosniff',
  };
  const compressible = TEXT_TYPES.has(ext) && ext !== '.svg';
  if (compressible && gzipAccepted) headers['Content-Encoding'] = 'gzip';
  return { headers, compressible, cacheable };
}

/** Pure: map a request path to a file under dist (null on traversal/outside). */
export function resolveUnderDist(distRoot, requestPath) {
  const decoded = decodeURIComponent(requestPath.split('?')[0]);
  if (decoded.includes('\0')) return null;
  const relative = decoded.replace(/^\/+/, '');
  const candidate = path.normalize(path.join(distRoot, relative));
  const root = path.normalize(distRoot + path.sep);
  if (!(candidate + path.sep).startsWith(root) && candidate !== path.normalize(distRoot)) {
    return null;
  }
  if (fs.existsSync(candidate) && fs.statSync(candidate).isDirectory()) {
    return path.join(candidate, 'index.html');
  }
  return candidate;
}

function main() {
  const [distArg, portArg] = process.argv.slice(2);
  if (!distArg || !portArg) {
    console.error('usage: node tools/site/serve-dist.mjs <dist-dir> <port>');
    process.exit(2);
  }
  const distRoot = path.resolve(distArg);
  const port = Number(portArg);

  const server = http.createServer((req, res) => {
    const filePath = resolveUnderDist(distRoot, req.url ?? '/');
    if (filePath === null || !fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
      const notFound = path.join(distRoot, '404.html');
      if (fs.existsSync(notFound)) {
        const body = fs.readFileSync(notFound);
        res.writeHead(404, { 'Content-Type': 'text/html; charset=utf-8' });
        res.end(req.method === 'HEAD' ? undefined : body);
        return;
      }
      res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
      res.end('not found');
      return;
    }
    const body = fs.readFileSync(filePath);
    const gzipAccepted = /(^|,)\s*gzip\s*(;|,|$)/.test(req.headers['accept-encoding'] ?? '');
    const { headers, compressible } = headersFor(filePath, { gzipAccepted });
    const payload = compressible && gzipAccepted ? zlib.gzipSync(body) : body;
    res.writeHead(200, headers);
    res.end(req.method === 'HEAD' ? undefined : payload);
  });

  server.listen(port, '127.0.0.1', () => {
    console.log(`serving ${distRoot} on http://127.0.0.1:${port}`);
  });
}

if (import.meta.url === `file://${process.argv[1]}`) main();
