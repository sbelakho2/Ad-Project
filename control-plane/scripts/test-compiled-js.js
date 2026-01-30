#!/usr/bin/env node
/**
 * Test script to verify the compiled inline JavaScript syntax is correct
 */

const fs = require('fs');
const path = require('path');

const distPath = path.join(__dirname, '../dist/index.js');

if (!fs.existsSync(distPath)) {
  console.error('dist/index.js not found - run npm run build first');
  process.exit(1);
}

const content = fs.readFileSync(distPath, 'utf8');

// Extract and test admin-dashboard.js
const dashboardStart = content.indexOf("app.get('/static/admin-dashboard.js'");
if (dashboardStart === -1) {
  console.error('Could not find admin-dashboard.js route');
  process.exit(1);
}

// Find the template literal content
const sendStart = content.indexOf('res.status(200).send(`', dashboardStart);
if (sendStart === -1) {
  console.error('Could not find send() call');
  process.exit(1);
}

const jsStart = sendStart + 'res.status(200).send(`'.length;

// Find the end - look for `); that ends the send call
// We need to be careful about nested template literals
let jsEnd = -1;
for (let i = jsStart; i < content.length - 2; i++) {
  if (content[i] === '`' && content[i + 1] === ')' && content[i + 2] === ';') {
    // Make sure this isn't inside a string
    // Count quotes before this position within the template
    jsEnd = i;
    break;
  }
}

if (jsEnd === -1) {
  console.error('Could not find end of template literal');
  process.exit(1);
}

const jsContent = content.slice(jsStart, jsEnd);
console.log(`Extracted ${jsContent.length} characters of JavaScript`);

// Test if it parses
try {
  new Function(jsContent);
  console.log('✅ admin-dashboard.js: Syntax OK');
} catch (e) {
  console.error('❌ admin-dashboard.js: Syntax ERROR');
  console.error('   ', e.message);
  
  // Try to show context
  const lines = jsContent.split('\n');
  const lineMatch = e.message.match(/line (\d+)/i);
  if (lineMatch) {
    const lineNum = parseInt(lineMatch[1]);
    console.log(`\n   Context around line ${lineNum}:`);
    for (let i = Math.max(0, lineNum - 3); i < Math.min(lines.length, lineNum + 2); i++) {
      const marker = i === lineNum - 1 ? '>>>' : '   ';
      console.log(`   ${marker} ${i + 1}: ${lines[i].substring(0, 100)}`);
    }
  }
  
  // Show lines with suspicious patterns - check for single backslash n that's not double-escaped
  const problematicLines = lines.map((line, idx) => ({ line, idx }))
    .filter(({ line }) => {
      // Look for \n that's not \\n 
      return /(?<!\\)\\n/.test(line);
    });
  
  if (problematicLines.length > 0) {
    console.log('\n   Lines with potentially unescaped \\n:');
    problematicLines.slice(0, 10).forEach(({ line, idx }) => {
      console.log(`   ${idx + 1}: ${line.substring(0, 100)}`);
    });
  }
  
  // Also check for backslash issues
  const backslashLines = lines.map((line, idx) => ({ line, idx }))
    .filter(({ line }) => line.includes('\\'));
  
  console.log(`\n   Total lines with backslashes: ${backslashLines.length}`);
  console.log('   First few:');
  backslashLines.slice(0, 5).forEach(({ line, idx }) => {
    console.log(`   ${idx + 1}: ${line.substring(0, 100)}`);
  });
  
  process.exit(1);
}

console.log('\n✅ All JS syntax checks passed');
