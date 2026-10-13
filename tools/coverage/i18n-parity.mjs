#!/usr/bin/env node
// i18n-parity.mjs — contract suite for the contract-i18n domain: every locale
// file under an i18n/messages tree must carry the same key set exactly, and all
// values must be non-empty strings (a missing translation or an empty label is
// a user-visible defect, not a stylistic one). Nested objects flatten to
// dot-paths; arrays and non-string leaves fail closed.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export function flattenKeys(value, prefix = '') {
  const out = [];
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return out;
  for (const [key, child] of Object.entries(value)) {
    const full = prefix ? `${prefix}.${key}` : key;
    if (Array.isArray(child)) {
      out.push(`${full}\u0000ARRAY`);
    } else if (child !== null && typeof child === 'object') {
      out.push(...flattenKeys(child, full));
    } else if (typeof child === 'string') {
      if (child.trim() === '') out.push(`${full}\u0000EMPTY`);
      else out.push(full);
    } else {
      out.push(`${full}\u0000TYPE:${typeof child}`);
    }
  }
  return out;
}

/** Pure: compare locale key sets against the reference locale. */
export function parityViolations(referenceName, locales) {
  const violations = [];
  const reference = new Set(locales[referenceName] ?? []);
  for (const [name, keys] of Object.entries(locales)) {
    const list = keys;
    const set = new Set(list);
    const missing = [...reference].filter((k) => !set.has(k));
    const extra = [...set].filter((k) => !reference.has(k));
    if (missing.length) violations.push(`${name}: ${missing.length} missing key(s) vs ${referenceName}: ${missing.slice(0, 8).join(', ')}`);
    if (extra.length) violations.push(`${name}: ${extra.length} extra key(s) vs ${referenceName}: ${extra.slice(0, 8).join(', ')}`);
    for (const k of list) {
      if (k.includes('\u0000EMPTY')) violations.push(`${name}: empty value: ${k.replace('\u0000EMPTY', '')}`);
      if (k.includes('\u0000ARRAY')) violations.push(`${name}: array value not allowed: ${k.replace('\u0000ARRAY', '')}`);
      if (k.includes('\u0000TYPE:')) violations.push(`${name}: non-string value: ${k}`);
    }
  }
  return violations;
}

function walkJson(root) {
  const out = [];
  const rec = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) rec(full);
      else if (entry.name.endsWith('.json')) out.push(full);
    }
  };
  if (!fs.existsSync(root)) return out;
  rec(root);
  return out.sort();
}

function main() {
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
  const i18nRoots = [path.join(repoRoot, 'console', 'src', 'i18n'), path.join(repoRoot, 'website', 'src', 'i18n')].filter(
    (p) => fs.existsSync(p),
  );
  const files = i18nRoots.flatMap((root) => walkJson(root));
  if (files.length === 0) {
    console.log('i18n-parity: no i18n trees found; nothing to compare');
    return;
  }
  const byDir = new Map();
  for (const file of files) {
    const dir = path.dirname(file);
    if (!byDir.has(dir)) byDir.set(dir, []);
    byDir.get(dir).push(file);
  }
  const failures = [];
  let compared = 0;
  for (const [dir, group] of byDir) {
    const locales = {};
    for (const file of group) {
      try {
        locales[path.basename(file)] = flattenKeys(JSON.parse(fs.readFileSync(file, 'utf8')));
      } catch (error) {
        failures.push(`${path.relative(repoRoot, file)}: invalid JSON: ${error.message}`);
      }
    }
    const names = Object.keys(locales).sort();
    if (names.length === 0) continue;
    const reference = names.includes('en.json') ? 'en.json' : names[0];
    const violations = parityViolations(reference, locales);
    for (const v of violations) failures.push(`${path.relative(repoRoot, dir)}/${v}`);
    compared += names.length;
  }
  if (failures.length) {
    console.error(`I18N PARITY FAILED (${failures.length}):`);
    for (const f of failures) console.error(`  - ${f}`);
    process.exit(1);
  }
  console.log(`i18n-parity PASS: ${compared} locale files across ${byDir.size} trees share identical key sets with no empty or non-string values`);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
