#!/usr/bin/env node
/**
 * Find the exact location of a syntax error in admin-dashboard.js
 */

const fs = require('fs');
const path = require('path');

const distPath = path.join(__dirname, '../dist/index.js');
const content = fs.readFileSync(distPath, 'utf8');

// Extract the JS content between the template literal backticks
const dashboardStart = content.indexOf("app.get('/static/admin-dashboard.js'");
const sendStart = content.indexOf('res.status(200).send(`', dashboardStart);
const jsStart = sendStart + 'res.status(200).send(`'.length;

// Find end - look for `); at the end
let jsEnd = -1;
let depth = 1;
for (let i = jsStart; i < content.length; i++) {
  if (content[i] === '`') {
    jsEnd = i;
    break;
  }
}

if (jsEnd === -1) {
  console.log('Could not find end of template literal');
  process.exit(1);
}

const jsContent = content.slice(jsStart, jsEnd);
console.log('Extracted', jsContent.length, 'characters');
console.log('First 50 chars:', JSON.stringify(jsContent.slice(0, 50)));
console.log('Last 50 chars:', JSON.stringify(jsContent.slice(-50)));

// Write to file for inspection
fs.writeFileSync('/tmp/admin-dashboard-extracted.js', jsContent);
console.log('Wrote to /tmp/admin-dashboard-extracted.js');

// Try to parse
try {
  new Function(jsContent);
  console.log('SUCCESS: JavaScript parses correctly');
} catch (e) {
  console.log('ERROR:', e.message);
  
  // Try line by line
  const lines = jsContent.split('\n');
  console.log('Total lines:', lines.length);
  
  // Binary search
  let low = 0;
  let high = lines.length;
  
  while (high - low > 1) {
    const mid = Math.floor((low + high) / 2);
    try {
      new Function(lines.slice(0, mid).join('\n'));
      low = mid;
    } catch (e) {
      high = mid;
    }
  }
  
  console.log('Error at line', high);
  console.log('Line content:', JSON.stringify(lines[high - 1]));
  
  // Show context
  for (let i = Math.max(0, high - 3); i < Math.min(lines.length, high + 2); i++) {
    const marker = i === high - 1 ? '>>>' : '   ';
    console.log(marker, i + 1, ':', JSON.stringify(lines[i]).slice(0, 100));
  }
}
