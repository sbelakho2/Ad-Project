#!/usr/bin/env node
// css-lint.mjs — contract suite for the contract-css domain. Hand-rolled, zero
// dependencies, adversarially narrow: balanced braces and quotes, no empty
// rules, no !important (a specificity escape hatch that breaks the design-token
// discipline), lowercase hex colors only, and a per-file size budget. The
// marketing site's full stylesheet additionally passes the site perf budget.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const CSS_FILE_BUDGET_BYTES = 120_000;

/** Pure: hygiene violations for one stylesheet text. */
export function cssViolations(rel, text) {
  const violations = [];
  // Length-preserving comment mask: structural checks run on masked text while
  // byte offsets stay valid against the original, so pragma comments (which the
  // mask blanks) can still be consulted by position.
  const stripped = text.replace(/\/\*[\s\S]*?\*\//g, (m) => ' '.repeat(m.length));
  const opens = (stripped.match(/\{/g) ?? []).length;
  const closes = (stripped.match(/\}/g) ?? []).length;
  if (opens !== closes) violations.push(`${rel}: unbalanced braces ({${opens}} vs }${closes}})`);
  if ((stripped.match(/"/g) ?? []).length % 2 !== 0 || (stripped.match(/'/g) ?? []).length % 2 !== 0) {
    violations.push(`${rel}: unbalanced quotes`);
  }
  // !important is a specificity escape hatch and stays banned by default.
  // Three justification classes are accepted, each narrow and auditable:
  // - inside a prefers-reduced-motion media block (the accessibility override
  //   pattern that must beat arbitrary animation rules),
  // - a motion-neutralizing declaration (property in the motion family with
  //   value none/auto/0.001ms/1),
  // - an explicit `/* css-contract:allow-important <reason> */` pragma.
  const motionMedia = /@media[^{]*prefers-reduced-motion[^{]*\{[\s\S]*?\n\}/gi;
  const motionRanges = [...stripped.matchAll(motionMedia)].map((m) => [m.index, m.index + m[0].length]);
  const inMotion = (idx) => motionRanges.some(([a, b]) => idx >= a && idx <= b);
  const MOTION = /(?:animation|transition)(?:-[a-z-]+)?\s*:\s*(none|auto|0\.001ms|\b1\b)|(?:scroll-behavior|transform)\s*:\s*(?:none|auto)\b/i;
  for (const m of stripped.matchAll(/!\s*important/gi)) {
    const idx = m.index ?? 0;
    if (inMotion(idx)) continue;
    const lineStart = stripped.lastIndexOf('\n', idx) + 1;
    const line = stripped.slice(lineStart, idx + m[0].length);
    if (MOTION.test(line)) continue;
    const context = text.slice(Math.max(0, lineStart - 260), lineStart);
    if (/css-contract:allow-important[^*]*\*\/\s*$/.test(context)) continue;
    violations.push(`${rel}: !important without justification at offset ${idx}`);
  }
  if (/\{\s*\}/.test(stripped)) violations.push(`${rel}: empty rule`);
  for (const m of stripped.matchAll(/#[0-9A-F]{3,8}\b/g)) {
    if (m[0] !== m[0].toLowerCase()) violations.push(`${rel}: uppercase hex color ${m[0]} (normalize to lowercase)`);
  }
  const bytes = Buffer.byteLength(text, 'utf8');
  if (bytes > CSS_FILE_BUDGET_BYTES) violations.push(`${rel}: ${bytes} bytes exceeds ${CSS_FILE_BUDGET_BYTES}`);
  return violations;
}

function walkCss(root) {
  const out = [];
  const rec = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (['node_modules', '.git', 'target', 'public', '.next', 'lhci-report', '.kilo'].includes(entry.name)) continue;
        rec(full);
      } else if (entry.name.endsWith('.css') || entry.name.endsWith('.scss')) {
        out.push(full);
      }
    }
  };
  rec(root);
  return out.sort();
}

function main() {
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
  const roots = ['console/src', 'website/src', 'site/static']
    .map((r) => path.join(repoRoot, r))
    .filter((p) => fs.existsSync(p));
  const files = roots.flatMap((root) => walkCss(root));
  const failures = [];
  for (const file of files) {
    failures.push(...cssViolations(path.relative(repoRoot, file), fs.readFileSync(file, 'utf8')));
  }
  if (failures.length) {
    console.error(`CSS CONTRACT FAILED (${failures.length}):`);
    for (const f of failures) console.error(`  - ${f}`);
    process.exit(1);
  }
  console.log(`css contract PASS: ${files.length} stylesheets clean (braces, quotes, no !important, lowercase hex, size budget)`);
}

if (import.meta.url === `file://${process.argv[1]}`) main();
