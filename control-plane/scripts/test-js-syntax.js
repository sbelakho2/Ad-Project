#!/usr/bin/env node
/**
 * Test script to verify inline JavaScript syntax is correct
 * Run after making changes to index.ts to catch template string escaping bugs
 */

const fs = require('fs');
const path = require('path');

const indexPath = path.join(__dirname, '../src/index.ts');
const indexTs = fs.readFileSync(indexPath, 'utf8');

// Extract all inline JS blocks
const patterns = [
  { name: 'admin-login.js', regex: /app\.get\('\/static\/admin-login\.js'[^]*?res\.status\(200\)\.send\(`([\s\S]*?)`\);/ },
  { name: 'admin-dashboard.js', regex: /app\.get\('\/static\/admin-dashboard\.js'[^]*?res\.status\(200\)\.send\(`([\s\S]*?)`\);/ },
  { name: 'admin-logout.js', regex: /app\.get\('\/static\/admin-logout\.js'[^]*?res\.status\(200\)\.send\(`([\s\S]*?)`\);/ },
];

let allPassed = true;

console.log('Testing inline JavaScript syntax...\n');

for (const { name, regex } of patterns) {
  const match = indexTs.match(regex);
  if (!match) {
    console.log(`⚠️  ${name}: Could not extract (pattern not found)`);
    continue;
  }
  
  const jsContent = match[1];
  
  try {
    // Try to parse the JavaScript
    new Function(jsContent);
    console.log(`✅ ${name}: Syntax OK (${jsContent.length} chars)`);
  } catch (e) {
    console.log(`❌ ${name}: SYNTAX ERROR - ${e.message}`);
    allPassed = false;
    
    // Show context around the error
    const lines = jsContent.split('\n');
    const errorMatch = e.message.match(/line (\d+)/i);
    if (errorMatch) {
      const lineNum = parseInt(errorMatch[1]);
      console.log(`   Near line ${lineNum}:`);
      for (let i = Math.max(0, lineNum - 3); i < Math.min(lines.length, lineNum + 2); i++) {
        const marker = i === lineNum - 1 ? '>>>' : '   ';
        console.log(`   ${marker} ${i + 1}: ${lines[i].substring(0, 80)}`);
      }
    }
  }
}

// Check for common escaping issues
console.log('\nChecking for common escaping issues...');

const dangerousPatterns = [
  { pattern: /\\n(?!\\)/g, name: 'Unescaped \\n in string' },
  { pattern: /\\t(?!\\)/g, name: 'Unescaped \\t in string' },
];

// Only check within the template string blocks, not the whole file
for (const { name, regex } of patterns) {
  const match = indexTs.match(regex);
  if (!match) continue;
  
  const jsContent = match[1];
  
  for (const { pattern, name: patternName } of dangerousPatterns) {
    const matches = jsContent.match(pattern);
    if (matches && matches.length > 0) {
      // Check if they're properly escaped (inside string literals)
      // This is a heuristic check - actual parsing is done above
      console.log(`⚠️  ${name}: Found ${matches.length} potential ${patternName} instances (verify they're intentional)`);
    }
  }
}

console.log('\n' + (allPassed ? '✅ All syntax checks passed!' : '❌ Some checks failed!'));
process.exit(allPassed ? 0 : 1);
