#!/usr/bin/env node
// claims-contract.mjs — two-way contract between the marketing site and the code:
//   1. every claim in site/claims.json must be implementable evidence: its anchor
//      path must EXIST and its pinned test name must appear IN that anchor file;
//   2. the claim text must literally appear (whitespace-normalized) somewhere in
//      the site content/templates, so claims.json can never drift from the pages.
// A claim that stops being true (test renamed, file moved, sentence reworded)
// fails the gate until both sides move together.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export function normalize(text) {
  return text.replace(/\s+/g, ' ').trim().toLowerCase();
}

/** Pure: validate a parsed claims file against a virtual fs surface. */
export function checkClaims(claims, { readFile, exists }) {
  const violations = [];
  if (!Array.isArray(claims?.claims) || claims.claims.length === 0) {
    return ['claims.json has no claims array'];
  }
  const seen = new Set();
  for (const claim of claims.claims) {
    const id = claim?.id ?? '<missing id>';
    if (seen.has(id)) violations.push(`${id}: duplicate claim id`);
    seen.add(id);
    if (!claim?.claim || !claim?.anchor || !claim?.test) {
      violations.push(`${id}: claim/anchor/test are all required`);
      continue;
    }
    if (!exists(claim.anchor)) {
      violations.push(`${id}: anchor path does not exist: ${claim.anchor}`);
      continue;
    }
    // `test` is either a bare name (pinned inside the anchor file) or
    // `name (path)` when the test lives in a sibling test file; both forms must
    // resolve to a file that CONTAINS the exact test name.
    const pinned = claim.test.match(/^(.*?)\s*\(([^)]+)\)\s*$/);
    const testName = (pinned ? pinned[1] : claim.test).trim();
    const testPath = pinned ? pinned[2].trim() : claim.anchor;
    if (!exists(testPath)) {
      violations.push(`${id}: pinned test file does not exist: ${testPath}`);
      continue;
    }
    let testText;
    try {
      testText = readFile(testPath);
    } catch (error) {
      violations.push(`${id}: pinned test file unreadable: ${error.message}`);
      continue;
    }
    if (!normalize(testText).includes(normalize(testName))) {
      violations.push(`${id}: pinned test '${testName}' not found in ${testPath}`);
    }
  }
  return violations;
}

export function checkPages(claims, pages) {
  const violations = [];
  const haystack = normalize(pages.join('\n'));
  for (const claim of claims.claims ?? []) {
    if (!claim?.claim) continue;
    if (!haystack.includes(normalize(claim.claim))) {
      violations.push(`${claim.id}: claim text not found on any page: ${JSON.stringify(claim.claim)}`);
    }
  }
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
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
  const siteRoot = path.join(repoRoot, 'site');
  const claimsPath = path.join(siteRoot, 'claims.json');
  if (!fs.existsSync(claimsPath)) {
    console.error('claims-contract: site/claims.json is missing');
    process.exit(1);
  }
  const claims = JSON.parse(fs.readFileSync(claimsPath, 'utf8'));
  const violations = checkClaims(claims, {
    exists: (p) => fs.existsSync(path.join(repoRoot, p)),
    readFile: (p) => fs.readFileSync(path.join(repoRoot, p), 'utf8'),
  });

  const pageFiles = [
    ...walk(path.join(siteRoot, 'content')).filter((f) => f.endsWith('.md')),
    ...walk(path.join(siteRoot, 'templates')).filter((f) => f.endsWith('.html')),
  ];
  if (pageFiles.length === 0) violations.push('no site content/templates found to verify claim text');
  const pages = pageFiles.map((f) => fs.readFileSync(f, 'utf8'));
  violations.push(...checkPages(claims, pages));

  if (violations.length) {
    console.error(`CLAIMS CONTRACT FAILED (${violations.length}):`);
    for (const v of violations) console.error(`  - ${v}`);
    process.exit(1);
  }
  console.log(`claims contract PASS: ${claims.claims.length} claims anchored and present on the site`);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
