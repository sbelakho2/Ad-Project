import dotenv from 'dotenv';
import fs from 'node:fs';
import path from 'node:path';
import logger from '../utils/logger';
import { runAnalyticsRollups } from '../services/analyticsRollupService';

// Load .env for scripts executed from /opt/adproject/backend.
// Prefer repo-root .env (../.env) when cwd is backend/.
// Never override existing env (e.g. if DATABASE_URL already provided).
if (!process.env.DATABASE_URL) {
  const cwd = process.cwd();
  const cwdBase = path.basename(cwd);
  const candidates =
    cwdBase === 'backend'
      ? [path.resolve(cwd, '../.env'), path.resolve(cwd, '.env')]
      : [path.resolve(cwd, '.env'), path.resolve(cwd, '../.env')];

  const envPath = [process.env.ENV_FILE, ...candidates].find((p) => !!p && fs.existsSync(p));
  if (envPath) {
    dotenv.config({ path: envPath });
  } else {
    dotenv.config();
  }

  // Safe diagnostics for production ops; never prints credentials.
  try {
    if (process.env.DATABASE_URL) {
      const u = new URL(process.env.DATABASE_URL);
      logger.info('Analytics rollups CLI: env loaded', {
        cwd,
        envPath: envPath ?? '<default>',
        dbHost: u.hostname,
        dbPort: u.port,
        dbName: (u.pathname || '').replace(/^\//, ''),
        sslmode: u.searchParams.get('sslmode') ?? undefined,
      });
    } else {
      logger.warn('Analytics rollups CLI: DATABASE_URL still missing after dotenv load', { cwd, envPath: envPath ?? '<default>' });
    }
  } catch {
    logger.warn('Analytics rollups CLI: could not parse DATABASE_URL after dotenv load', { cwd, envPath: envPath ?? '<default>' });
  }
}

const usage = () => {
  // eslint-disable-next-line no-console
  console.log(
    [
      'Usage:',
      '  node dist/src/scripts/analyticsRollupsCli.js --start YYYY-MM-DD --end YYYY-MM-DD',
      '  node dist/src/scripts/analyticsRollupsCli.js --days N',
      '',
      'Notes:',
      '  - Dates are interpreted as UTC midnights.',
      '  - --end is exclusive.',
      '  - Runs one day at a time (safe for production).',
    ].join('\n')
  );
};

const parseArg = (name: string): string | undefined => {
  const argv = process.argv.slice(2);
  const idx = argv.indexOf(name);
  if (idx === -1) return undefined;
  return argv[idx + 1];
};

const utcMidnight = (d: Date): Date => new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()));

const parseUtcDate = (s: string): Date => {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(s)) {
    throw new Error(`Invalid date format: ${s} (expected YYYY-MM-DD)`);
  }
  // Interpret as UTC midnight.
  const [y, m, d] = s.split('-').map(Number);
  return new Date(Date.UTC(y, m - 1, d));
};

async function main(): Promise<void> {
  const daysArg = parseArg('--days');
  const startArg = parseArg('--start');
  const endArg = parseArg('--end');

  if (!daysArg && (!startArg || !endArg)) {
    usage();
    process.exit(2);
  }

  let startInclusive: Date;
  let endExclusive: Date;

  if (daysArg) {
    const days = Number(daysArg);
    if (!Number.isFinite(days) || days <= 0) {
      throw new Error(`Invalid --days: ${daysArg}`);
    }
    const now = new Date();
    endExclusive = utcMidnight(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() + 1)));
    startInclusive = utcMidnight(new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - Math.floor(days))));
  } else {
    startInclusive = parseUtcDate(startArg!);
    endExclusive = parseUtcDate(endArg!);
  }

  if (startInclusive >= endExclusive) {
    throw new Error('start must be < end');
  }

  logger.info('Analytics rollups CLI: starting', {
    startInclusive: startInclusive.toISOString(),
    endExclusive: endExclusive.toISOString(),
  });

  // Run day-by-day to keep transactions small and reduce lock pressure.
  for (let d = new Date(startInclusive); d < endExclusive; d = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() + 1))) {
    const next = new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate() + 1));

    logger.info('Analytics rollups CLI: running day', {
      startInclusive: d.toISOString(),
      endExclusive: next.toISOString(),
    });

    await runAnalyticsRollups({ startInclusive: d, endExclusive: next });
  }

  logger.info('Analytics rollups CLI: completed');
}

main().catch(err => {
  logger.error('Analytics rollups CLI: failed', { error: (err as Error).message });
  process.exit(1);
});
