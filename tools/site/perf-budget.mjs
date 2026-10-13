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

/** Pure: evaluate one built page. Returns violations for that page. */
export function checkPage(rel, html, htmlBudget = HTML_BUDGET_BYTES) {
  const violations = [];
  const bytes = Buffer.byteLength(html, 'utf8');
  if (bytes > htmlBudget) violations.push(`${rel}: ${bytes} bytes > html budget ${htmlBudget}`);
  if (/<script[\s>]/i.test(html)) violations.push(`${rel}: contains a <script> tag (zero-JS budget)`);
  if (!/<meta\s+name="description"\s+content="[^"]+"/i.test(html)) violations.push(`${rel}: missing meta description`);
  const links = html.match(/<link\b[^>]*>/gi) ?? [];
  const canonical = links.some((tag) => /rel="canonical"/i.test(tag));
  if (!canonical) violations.push(`${rel}: missing canonical link`);
  for (const tag of links) {
    const relMatch = tag.match(/rel="([^"]+)"/i);
    const hrefMatch = tag.match(/href="(https?:\/\/[^"]+)"/i);
    if (!relMatch || !hrefMatch) continue;
    const kind = relMatch[1].toLowerCase();
    // Only resource-fetching links are external-request violations; a canonical
    // (or alternate/sitemap) URL is metadata, not a network dependency.
    if (/(stylesheet|preconnect|dns-prefetch|preload|prefetch|icon|manifest)/.test(kind)) {
      violations.push(`${rel}: external ${kind} request (self-host only): ${hrefMatch[1]}`);
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
  for (const [rel, html] of pages) violations.push(...checkPage(rel, html, htmlBudget));
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

  const files = walk(dist);
  const pages = new Map();
  let cssBytes = 0;
  for (const file of files) {
    const rel = path.relative(dist, file);
    if (file.endsWith('.html')) pages.set(rel, fs.readFileSync(file, 'utf8'));
    else if (file.endsWith('.css')) cssBytes += fs.statSync(file).size;
  }
  const violations = checkDist(pages, cssBytes, { minPages });
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
