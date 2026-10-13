#!/usr/bin/env node
// docs-links.mjs — the docs-domain link contract, the docs suite in the design:
// every relative markdown link must resolve to a real file, and every in-page
// anchor link must match a real heading. External links are out of scope
// (availability is not a repo property). Legacy debt is frozen in a ratchet
// baseline exactly like docs-lint: rows may only shrink, and a stale entry is
// a failure so closed items must delete their rows.
//
// Usage:
//   node tools/coverage/docs-links.mjs --check [--baseline tools/coverage/docs-links-baseline.txt]
//   node tools/coverage/docs-links.mjs --update-baseline tools/coverage/docs-links-baseline.txt
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export function slugify(heading) {
  return heading
    .toLowerCase()
    .replace(/<[^>]+>/g, '')
    .replace(/[`*_~]/g, '')
    .replace(/[^\p{L}\p{N}\s-]/gu, '')
    .trim()
    .replace(/\s+/g, '-');
}

/** Pure: headings in one markdown document, slugged. */
export function headingSlugs(markdown) {
  const slugs = new Set();
  for (const line of markdown.split('\n')) {
    const m = line.match(/^(#{1,6})\s+(.+?)\s*$/);
    if (m) {
      slugs.add(slugify(m[2]));
      slugs.add(m[2].trim().toLowerCase()); // raw fallback for non-GitHub renderers
    }
  }
  return slugs;
}

/** Pure: relative links in one markdown document (external types skipped). */
export function relativeLinks(markdown) {
  const links = [];
  for (const m of markdown.matchAll(/\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g)) {
    const target = m[1];
    if (/^(https?:|mailto:|tel:|data:)/i.test(target)) continue;
    links.push(target);
  }
  return links;
}

/** Pure: violations for one doc given a loader for other docs. */
export function checkDoc(rel, markdown, { fileExists, readDoc }) {
  const violations = [];
  const ownSlugs = headingSlugs(markdown);
  for (const target of relativeLinks(markdown)) {
    const [rawPath, anchor] = target.split('#');
    const cleanPath = decodeURIComponent(rawPath || '');
    if (cleanPath === '') {
      // Pure anchor link: must match a heading in this document.
      if (anchor && !ownSlugs.has(anchor.toLowerCase())) {
        violations.push(`${rel}: anchor '#${anchor}' has no matching heading`);
      }
      continue;
    }
    const resolved = path.posix.normalize(path.posix.join(path.posix.dirname(rel), cleanPath));
    if (!fileExists(resolved)) {
      violations.push(`${rel}: link target missing: ${cleanPath}`);
      continue;
    }
    if (anchor && resolved.endsWith('.md')) {
      const other = readDoc(resolved);
      if (other === null) {
        violations.push(`${rel}: linked doc unreadable: ${resolved}`);
      } else if (!headingSlugs(other).has(anchor.toLowerCase())) {
        violations.push(`${rel}: anchor '#${anchor}' not found in ${resolved}`);
      }
    }
  }
  return violations;
}

function walkMarkdown(root) {
  const out = [];
  const rec = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (['node_modules', '.git', 'target', 'public', 'lhci-report', '.kilo', '.junie'].includes(entry.name)) continue;
        rec(full);
      } else if (entry.name.endsWith('.md')) {
        out.push(path.relative(root, full).split(path.sep).join('/'));
      }
    }
  };
  rec(root);
  return out.sort();
}

function main() {
  const args = process.argv.slice(2);
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
  const docs = walkMarkdown(repoRoot);
  const byPos = new Map();
  for (const rel of docs) byPos.set(rel, fs.readFileSync(path.join(repoRoot, rel), 'utf8'));
  const ctx = {
    fileExists: (resolved) => fs.existsSync(path.join(repoRoot, resolved)) || byPos.has(resolved),
    readDoc: (resolved) => byPos.get(resolved) ?? null,
  };
  const perFile = new Map();
  let total = 0;
  for (const rel of docs) {
    const violations = checkDoc(rel, byPos.get(rel), ctx);
    if (violations.length) {
      perFile.set(rel, { count: violations.length, violations });
      total += violations.length;
    }
  }

  const updateIdx = args.indexOf('--update-baseline');
  if (updateIdx >= 0) {
    const target = args[updateIdx + 1];
    const lines = ['# docs-links ratchet baseline: per-file broken-link counts (rows may only shrink)'];
    for (const [rel, { count }] of [...perFile.entries()].sort()) lines.push(`${count} ${rel}`);
    lines.push(`TOTAL ${total}`);
    fs.writeFileSync(path.resolve(repoRoot, target), lines.join('\n') + '\n');
    console.log(`docs-links baseline written: ${target} (TOTAL ${total} across ${perFile.size} files)`);
    return;
  }

  const baseIdx = args.indexOf('--baseline');
  if (baseIdx >= 0) {
    const baselinePath = path.resolve(repoRoot, args[baseIdx + 1]);
    if (!fs.existsSync(baselinePath)) {
      console.error(`docs-links: baseline not found: ${args[baseIdx + 1]}`);
      process.exit(1);
    }
    const allowed = new Map();
    let allowedTotal = 0;
    for (const line of fs.readFileSync(baselinePath, 'utf8').split('\n')) {
      const totalMatch = line.match(/^TOTAL (\d+)$/);
      if (totalMatch) { allowedTotal = Number(totalMatch[1]); continue; }
      const m = line.match(/^(\d+)\s+(.+)$/);
      if (!m) continue;
      allowed.set(m[2], Number(m[1]));
    }
    const failures = [];
    let actualTotal = 0;
    for (const [rel, { count, violations }] of perFile) {
      actualTotal += count;
      const allowedCount = allowed.get(rel) ?? 0;
      if (count > allowedCount) {
        failures.push(`${rel}: ${count} broken links (baseline ${allowedCount})`);
        for (const v of violations.slice(0, 5)) failures.push(`    ${v}`);
      }
    }
    for (const rel of allowed.keys()) {
      if (!perFile.has(rel)) failures.push(`${rel}: baseline row no longer needed (stale) — remove it`);
    }
    if (total > allowedTotal) failures.push(`total ${total} exceeds baseline total ${allowedTotal}`);
    if (failures.length) {
      console.error(`DOCS-LINKS FAILED (${failures.length}):`);
      for (const f of failures) console.error(`  - ${f}`);
      process.exit(1);
    }
    console.log(`docs-links PASS: ${total} broken links at or below baseline (${allowedTotal}) across ${docs.length} documents`);
    return;
  }

  console.error('usage: docs-links.mjs --check --baseline <file> | --update-baseline <file>');
  process.exit(2);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
