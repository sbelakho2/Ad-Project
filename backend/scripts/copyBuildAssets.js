#!/usr/bin/env node
/*
 * Copies non-TS build assets into dist/ so runtime helpers (like ensureRollupTables)
 * work when the backend runs from compiled output.
 */

const fsp = require('node:fs/promises');
const path = require('node:path');

async function exists(p) {
  try {
    await fsp.access(p);
    return true;
  } catch {
    return false;
  }
}

async function copyDir(src, dest) {
  await fsp.mkdir(dest, { recursive: true });
  const entries = await fsp.readdir(src, { withFileTypes: true });
  for (const e of entries) {
    const s = path.join(src, e.name);
    const d = path.join(dest, e.name);
    if (e.isDirectory()) {
      await copyDir(s, d);
    } else if (e.isFile()) {
      await fsp.copyFile(s, d);
    }
  }
}

async function main() {
  const root = path.resolve(__dirname, '..');
  const dist = path.join(root, 'dist');

  if (!(await exists(dist))) {
    console.error('[copyBuildAssets] dist/ not found; did tsc run?');
    process.exit(1);
  }

  const scriptsSrc = path.join(root, 'scripts');
  const scriptsDest = path.join(dist, 'scripts');
  if (await exists(scriptsSrc)) {
    await copyDir(scriptsSrc, scriptsDest);
  }

  const migrationsSrc = path.join(root, 'migrations');
  const migrationsDest = path.join(dist, 'migrations');
  if (await exists(migrationsSrc)) {
    await copyDir(migrationsSrc, migrationsDest);
  }

  console.log('[copyBuildAssets] Copied scripts/ and migrations/ into dist/.');
}

main().catch((err) => {
  console.error('[copyBuildAssets] Failed', err);
  process.exit(1);
});
