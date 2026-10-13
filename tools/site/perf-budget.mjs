#!/usr/bin/env node
// perf-budget.mjs — hard performance budgets for the built Zola marketing site.
// The budget is the product: a static site that ships zero JavaScript, one small
// stylesheet and small pages, verified against the BUILT output (never intent).
//
// Usage: node tools/site/perf-budget.mjs [--dist site/public] [--min-pages 6]
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const HTML_BUDGET_BYTES = 40_000;
export const CSS_BUDGET_BYTES = 12_000;

/** Parse attributes from a tag's inner text (quoted or unquoted, any order). */
export function parseAttrs(tagInner) {
  const attrs = new Map();
  const re = /([a-zA-Z_:][a-zA-Z0-9_:.-]*)(?:\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+)))?/g;
  let m;
  while ((m = re.exec(tagInner)) !== null) {
    const value = m[3] ?? m[4] ?? m[5] ?? '';
    attrs.set(m[1].toLowerCase(), value);
  }
  return attrs;
}

/** Pure: evaluate one built page. Returns violations for that page.
 * `origin` is the site's own origin (e.g. "https://apexmediation.example"); resource
 * links pointing at the SAME origin are self-hosted and fine, anything else is an
 * external request. */
export function checkPage(rel, html, htmlBudget = HTML_BUDGET_BYTES, origin = null) {
  const violations = [];
  const bytes = Buffer.byteLength(html, 'utf8');
  if (bytes > htmlBudget) violations.push(`${rel}: ${bytes} bytes > html budget ${htmlBudget}`);
  if (/<script[\s>]/i.test(html)) violations.push(`${rel}: contains a <script> tag (zero-JS budget)`);

  const metas = [...html.matchAll(/<meta\b([^>]*)>/gi)].map((m) => parseAttrs(m[1]));
  const hasDescription = metas.some(
    (attrs) => attrs.get('name') === 'description' && (attrs.get('content') ?? '').trim() !== '',
  );
  if (!hasDescription) violations.push(`${rel}: missing meta description`);

  const links = [...html.matchAll(/<link\b([^>]*)>/gi)].map((m) => parseAttrs(m[1]));
  // 404 is legitimately canonical-less; every other page must name its canonical URL.
  if (rel !== '404.html') {
    const canonical = links.some((attrs) => (attrs.get('rel') ?? '').toLowerCase() === 'canonical' && (attrs.get('href') ?? '').trim() !== '');
    if (!canonical) violations.push(`${rel}: missing canonical link`);
  }
  for (const attrs of links) {
    const kind = (attrs.get('rel') ?? '').toLowerCase();
    const href = attrs.get('href') ?? '';
    // Only resource-fetching links are external-request violations; a canonical
    // (or alternate/sitemap) URL is metadata, not a network dependency.
    if (!/(stylesheet|preconnect|dns-prefetch|preload|prefetch|icon|manifest)/.test(kind)) continue;
    const absolute = /^(https?:)?\/\//i.test(href);
    if (!absolute) continue; // relative path: self-hosted by construction
    const sameOrigin = origin !== null && origin !== '' && href.startsWith(origin + '/');
    if (!sameOrigin) {
      violations.push(`${rel}: external ${kind} request (self-host only): ${href}`);
    }
  }
  return violations;
}

/** Pure: evaluate the whole distribution. */
export function checkDist(pages, cssBytes, opts = {}) {
  const htmlBudget = opts.htmlBudget ?? HTML_BUDGET_BYTES;
  const cssBudget = opts.cssBudget ?? CSS_BUDGET_BYTES;
  const minPages = opts.minPages ?? 6;
  const violations = [];
  for (const [rel, html] of pages) violations.push(...checkPage(rel, html, htmlBudget, opts.origin ?? null));
  if (pages.size < minPages) violations.push(`built ${pages.size} pages < required ${minPages}`);
  if (cssBytes > cssBudget) violations.push(`stylesheets total ${cssBytes} bytes > budget ${cssBudget}`);
  return violations;
}

function walk(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, out);
    else out.push(full);
  }
  return out;
}

function main() {
  const args = process.argv.slice(2);
  const get = (k, d) => { const i = args.indexOf(k); return i >= 0 ? args[i + 1] : d; };
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
  const dist = path.resolve(repoRoot, get('--dist', 'site/public'));
  const minPages = Number(get('--min-pages', '6'));

  if (!fs.existsSync(dist)) {
    console.error(`site perf budget: dist not found at ${dist} — build the site first (zola build)`);
    process.exit(1);
  }

  // The site's own origin comes from config.toml's base_url; same-origin absolute
  // URLs (canonical, stylesheet) are self-hosted, cross-origin ones are violations.
  let origin = null;
  const configPath = path.join(repoRoot, 'site', 'config.toml');
  if (fs.existsSync(configPath)) {
    const match = fs.readFileSync(configPath, 'utf8').match(/^base_url\s*=\s*"([^"]+)"/m);
    if (match) origin = match[1].replace(/\/+$/, '');
  }

  const files = walk(dist);
  const pages = new Map();
  let cssBytes = 0;
  for (const file of files) {
    const rel = path.relative(dist, file);
    if (file.endsWith('.html')) pages.set(rel, fs.readFileSync(file, 'utf8'));
    else if (file.endsWith('.css')) cssBytes += fs.statSync(file).size;
  }
  const violations = checkDist(pages, cssBytes, { minPages, origin });
  if (!fs.existsSync(path.join(dist, 'robots.txt'))) violations.push('dist missing robots.txt');
  if (!fs.existsSync(path.join(dist, 'sitemap.xml'))) violations.push('dist missing sitemap.xml');

  if (violations.length) {
    console.error(`SITE PERF BUDGET FAILED (${violations.length}):`);
    for (const v of violations) console.error(`  - ${v}`);
    process.exit(1);
  }
  console.log(
    `site perf budget PASS: ${pages.size} pages (max html ${HTML_BUDGET_BYTES}B), css ${cssBytes}B (budget ${CSS_BUDGET_BYTES}B), zero JS, robots+sitemap present`,
  );
}

if (import.meta.url === `file://${process.argv[1]}`) main();
