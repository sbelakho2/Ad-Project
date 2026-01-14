#!/usr/bin/env node
/*
 Provider Content Guard
 - Scans code/config areas of the repo to block accidental reintroduction of legacy managed PaaS providers.
 - Excludes docs/, quality/, node_modules/, and any file carrying a top-of-file "DEPRECATION NOTICE".
*/

const fs = require('fs');
const path = require('path');

const repoRoot = path.resolve(__dirname, '..');

const SCAN_DIRS = [
  'backend',
  'console',
  'infrastructure',
  'scripts',
  'services',
  'website',
  '.github',
  'packages',
  'sdk',
  'sdks',
];

const EXCLUDE_DIRS = new Set([
  'node_modules',
  '.git',
  'logs',
  'quality',
  'docs', // docs are allowed to mention legacy providers when marked
  'dist',
  'build',
  '.next',
  'website', // marketing site may have separate hosting notes; excluded from guard
]);

const FORBIDDEN = [
  /\bheroku\b/i,
  /vercel/i,
  /render\.com/i,
  /\brailway\b/i,
  /upstash/i,
  /backblaze/i,
  /backblazeb2/i,
  /\bB2\b/i,
];

const TEXT_EXT = new Set([
  '.js', '.cjs', '.mjs', '.ts', '.tsx', '.jsx',
  '.json', '.yml', '.yaml', '.sh', '.bash',
  '.conf', '.toml', '.env', '.gradle', '.properties', '.xml', '.txt'
]);

function shouldScanFile(filePath) {
  const rel = path.relative(repoRoot, filePath);
  const parts = rel.split(path.sep);
  for (const part of parts) {
    if (EXCLUDE_DIRS.has(part)) return false;
  }
  const ext = path.extname(filePath).toLowerCase();
  if (!TEXT_EXT.has(ext)) return false;
  try {
    const stat = fs.statSync(filePath);
    if (stat.size > 2 * 1024 * 1024) return false; // skip large files >2MB
  } catch {
    return false;
  }

  // Allow files that explicitly declare deprecation at top
  try {
    const head = fs.readFileSync(filePath, 'utf8').slice(0, 4096);
    if (/DEPRECATION NOTICE|DEPRECATED —/i.test(head)) return false;
  } catch {
    return false;
  }
  return true;
}

function* walk(dir) {
  let entries = [];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return; // ignore missing dirs
  }
  for (const e of entries) {
    const full = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (EXCLUDE_DIRS.has(e.name)) continue;
      yield* walk(full);
    } else if (e.isFile()) {
      yield full;
    }
  }
}

function scan() {
  const offenders = [];
  for (const root of SCAN_DIRS) {
    const start = path.join(repoRoot, root);
    for (const f of walk(start)) {
      if (!shouldScanFile(f)) continue;
      let txt;
      try {
        txt = fs.readFileSync(f, 'utf8');
      } catch {
        continue;
      }
      for (const rx of FORBIDDEN) {
        if (rx.test(txt)) {
          offenders.push(`${path.relative(repoRoot, f)} → ${rx}`);
          break;
        }
      }
    }
  }

  if (offenders.length > 0) {
    console.error('[CONTENT GUARD] Forbidden provider references detected in non-deprecated files:');
    for (const o of offenders) console.error(' -', o);
    console.error('\nRemediation: remove legacy provider usage or add an explicit deprecation banner at the top of historical files.');
    process.exit(2);
  } else {
    console.log('[CONTENT GUARD] OK — no forbidden provider references found in code/config.');
  }
}

scan();
