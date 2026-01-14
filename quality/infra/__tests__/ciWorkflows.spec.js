const fs = require('fs');
const path = require('path');

function listYamlFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir)
    .filter((f) => f.endsWith('.yml') || f.endsWith('.yaml'))
    .map((f) => path.join(dir, f));
}

function read(p) {
  return fs.readFileSync(p, 'utf8');
}

describe('CI Workflows — forbid legacy provider deploy steps', () => {
  const repoRoot = path.resolve(__dirname, '../../../');
  const workflowsDir = path.join(repoRoot, '.github/workflows');
  const files = listYamlFiles(workflowsDir);

  const forbidden = [
    /\bheroku\b/i,
    /vercel/i,
    /render\.com/i,
    /\brailway\b/i,
    /upstash/i,
  ];

  test('Workflows directory exists and contains YAML files', () => {
    expect(fs.existsSync(workflowsDir)).toBe(true);
    expect(files.length).toBeGreaterThan(0);
  });

  test('No residual deploy steps for Heroku/Vercel/Render/Railway/Upstash in CI workflows', () => {
    for (const file of files) {
      const txt = read(file);
      // allow explicit deprecation notes in comments
      const contentNoComments = txt
        .split('\n')
        .filter((line) => !/^\s*#/.test(line))
        .join('\n');
      for (const rx of forbidden) {
        const ok = !rx.test(contentNoComments);
        if (!ok) {
          throw new Error(`Forbidden provider reference matched in workflow: ${file} (${rx})`);
        }
      }
    }
  });

  test('ci-all.yml includes provider content guard step', () => {
    const ciAll = path.join(workflowsDir, 'ci-all.yml');
    if (!fs.existsSync(ciAll)) return; // tolerate missing file
    const txt = read(ciAll);
    expect(/guard:providers/.test(txt)).toBe(true);
  });
});
