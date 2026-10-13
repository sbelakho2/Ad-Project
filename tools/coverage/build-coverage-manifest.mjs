#!/usr/bin/env node
// build-coverage-manifest.mjs — the single definition of "100% of repo".
// Walks the repo, classifies every file into a domain (or an exclusion with a reason
// code), and emits coverage-manifest.json. Exits 1 if any file is unclassified.
import fs from 'node:fs';
import path from 'node:path';
import { walkRepo, classify, EXCLUSION_CODES, REPO_ROOT } from './coverage-lib.mjs';

const args = process.argv.slice(2);
const outIdx = args.indexOf('--out');
const outFile = outIdx >= 0 ? args[outIdx + 1] : 'coverage-manifest.json';

const files = walkRepo(REPO_ROOT).map((rel) => {
  const c = classify(rel);
  if (c.status === 'excluded' && !EXCLUSION_CODES.includes(c.reason)) {
    throw new Error(`classify() returned unknown exclusion reason '${c.reason}' for ${rel} — reasons are a closed set (${EXCLUSION_CODES.join(', ')})`);
  }
  return {
    path: rel,
    domain: c.domain,
    kind: c.kind ?? null,
    status: c.status,
    reason: c.reason ?? null,
    suite: null,
  };
});

const unclassified = files.filter((f) => f.status === 'unclassified');
if (unclassified.length > 0) {
  console.error(`UNCLASSIFIED FILES (${unclassified.length}) — extend DOMAINS in coverage-lib.mjs or add a justified exclusion:`);
  for (const f of unclassified) console.error(`  ${f.path}`);
  process.exit(1);
}

const manifest = {
  policy: 'docs/Internal/Testing/ADVERSARIAL_DOGFOODING_100PCT.md',
  generatedAt: new Date().toISOString(),
  counts: files.reduce((acc, f) => ((acc[f.status] = (acc[f.status] ?? 0) + 1), acc), {}),
  files,
};
fs.writeFileSync(path.join(REPO_ROOT, outFile), JSON.stringify(manifest, null, 2));
console.log(`manifest written: ${outFile} (${files.length} files; counts=${JSON.stringify(manifest.counts)})`);
