#!/usr/bin/env node
// console-a11y.mjs — adversarial accessibility audit of the rendered console
// pages (produced by `cargo run -p apex-api --bin render-fixtures`). Runs axe-core
// under a single persistent jsdom context over the WCAG 2.0/2.1 A+AA tag set.
//
// Coverage split, deliberately:
// - everything axe can evaluate without layout runs here (landmarks, roles,
//   names, labels, heading order, table markup, form structure, lang, title);
// - `color-contrast` needs a real layout engine (jsdom has none) and is covered
//   twice elsewhere: the console palette ratios are asserted in Rust
//   (apex-api html.rs palette_contrast tests) and the marketing site's contrast
//   runs in the Lighthouse lane on a real browser.
//
// Usage: node tools/ui-ux/console-a11y.mjs <rendered-dir>
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const FAILING_IMPACTS = ['critical', 'serious'];
export const LAYOUT_DEPENDENT_RULES = ['color-contrast', 'target-size'];

/** Pure: violations of failing impact from an axe results object. */
export function failingViolations(results) {
  return (results.violations ?? []).filter((v) => FAILING_IMPACTS.includes(v.impact ?? 'minor'));
}

/** Pure: structural contract every console page must satisfy. */
export function structureViolations(rel, html) {
  const violations = [];
  if (!/<html[^>]+lang="[^"]+"/i.test(html) && !/<html[^>]+lang=[a-z-]+/i.test(html)) {
    violations.push(`${rel}: <html> is missing a lang attribute`);
  }
  if (!/<title>[^<]+<\/title>/i.test(html)) violations.push(`${rel}: missing a non-empty <title>`);
  const h1Count = [...html.matchAll(/<h1[\s>]/gi)].length;
  if (h1Count !== 1) violations.push(`${rel}: expected exactly one <h1>, found ${h1Count}`);
  if (/<script[\s>]/i.test(html)) violations.push(`${rel}: console must stay script-free`);
  return violations;
}

async function main() {
  const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
  const dir = path.resolve(repoRoot, process.argv[2] ?? 'rust/target/ui-fixtures');
  if (!fs.existsSync(dir)) {
    console.error(`console-a11y: rendered dir not found: ${dir} (run the render-fixtures bin first)`);
    process.exit(2);
  }
  const pages = fs
    .readdirSync(dir)
    .filter((f) => f.endsWith('.html'))
    .sort();
  if (pages.length < 4) {
    console.error(`console-a11y: expected at least 4 rendered pages, found ${pages.length}`);
    process.exit(2);
  }

  const { JSDOM } = await import('jsdom');
  const dom = new JSDOM('<!doctype html><html lang="en"><head><title>boot</title></head><body></body></html>', {
    url: 'http://console.local/',
  });
  globalThis.window = dom.window;
  globalThis.document = dom.window.document;
  globalThis.Node = dom.window.Node;
  globalThis.Element = dom.window.Element;
  globalThis.HTMLElement = dom.window.HTMLElement;
  // Import axe after the globals exist (it binds them at module load), once.
  const axeModule = await import('axe-core');
  const axe = axeModule.default ?? axeModule;

  const findings = [];
  const structural = [];
  for (const page of pages) {
    const html = fs.readFileSync(path.join(dir, page), 'utf8');
    structural.push(...structureViolations(page, html));
    dom.window.document.open();
    dom.window.document.write(html);
    dom.window.document.close();
    const results = await axe.run(dom.window.document, {
      runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'] },
      rules: Object.fromEntries(LAYOUT_DEPENDENT_RULES.map((id) => [id, { enabled: false }])),
    });
    findings.push(
      ...failingViolations(results).map((v) => ({
        rel: page,
        id: v.id,
        impact: v.impact,
        help: v.help,
        nodes: v.nodes.length,
      })),
    );
  }

  const total = findings.length + structural.length;
  if (total > 0) {
    console.error(`CONSOLE A11Y FAILED (${total}):`);
    for (const s of structural) console.error(`  - ${s}`);
    for (const f of findings) {
      console.error(`  - ${f.rel}: [${f.impact}] ${f.id}: ${f.help} (${f.nodes} node(s))`);
    }
    process.exit(1);
  }
  console.log(
    `console a11y PASS: ${pages.length} rendered pages, WCAG 2.0/2.1 A+AA via axe (layout-dependent rules delegated to the palette test and the Lighthouse lane), structure contract clean`,
  );
}

if (import.meta.url === `file://${process.argv[1]}`) main();
