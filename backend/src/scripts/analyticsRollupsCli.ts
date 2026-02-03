// Load environment first, before any other imports that depend on process.env
import { ensureEnvLoaded, getEnvDiagnostics } from '../utils/envLoader';
ensureEnvLoaded();

import logger from '../utils/logger';
import { closePools } from '../utils/postgres';
import { runAnalyticsRollups } from '../services/analyticsRollupService';

// Log safe diagnostics for production ops; never prints credentials.
const diagnostics = getEnvDiagnostics();
if (diagnostics.databaseUrl) {
  logger.info('Analytics rollups CLI: env loaded', {
    cwd: diagnostics.cwd,
    envPath: diagnostics.envPath ?? '<default>',
    dbHost: diagnostics.databaseUrl.host,
    dbPort: diagnostics.databaseUrl.port,
    dbName: diagnostics.databaseUrl.database,
    sslmode: diagnostics.databaseUrl.sslmode ?? undefined,
  });
} else {
  logger.warn('Analytics rollups CLI: DATABASE_URL missing after env load', {
    cwd: diagnostics.cwd,
    envPath: diagnostics.envPath ?? '<default>',
  });
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

const print = (msg: string, data?: Record<string, unknown>) => {
  // Always emit a human-readable progress line to stdout so long runs don't
  // look "stalled" when structured logger output isn't visible in nohup/SSH.
  // Keep it secret-safe (no credentials).
  // eslint-disable-next-line no-console
  console.log(`[${new Date().toISOString()}] ${msg}${data ? ` ${JSON.stringify(data)}` : ''}`);
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
  print('Analytics rollups CLI: starting', {
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
    print('Analytics rollups CLI: running day', {
      startInclusive: d.toISOString(),
      endExclusive: next.toISOString(),
    });

    const t0 = Date.now();
    const heartbeat = setInterval(() => {
      print('Analytics rollups CLI: still running day', {
        startInclusive: d.toISOString(),
        endExclusive: next.toISOString(),
        elapsedMs: Date.now() - t0,
      });
    }, 60_000);
    // Allow the process to exit promptly once work completes.
    heartbeat.unref?.();

    await runAnalyticsRollups({ startInclusive: d, endExclusive: next });

    clearInterval(heartbeat);

    print('Analytics rollups CLI: finished day', {
      startInclusive: d.toISOString(),
      endExclusive: next.toISOString(),
      elapsedMs: Date.now() - t0,
    });
  }

  logger.info('Analytics rollups CLI: completed');
  print('Analytics rollups CLI: completed');

  try {
    await closePools();
  } catch {
    // ignore shutdown errors; the CLI has already completed successfully.
  }
}

main().catch(err => {
  logger.error('Analytics rollups CLI: failed', { error: (err as Error).message });
  print('Analytics rollups CLI: failed', { error: (err as Error).message });
  closePools()
    .catch(() => undefined)
    .finally(() => {
      process.exit(1);
    });
});
