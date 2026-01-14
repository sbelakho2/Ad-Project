#!/usr/bin/env node
/**
 * Generates deterministic sandbox revenue for FakeNetworkA/B/C across all placements
 * so the Starter (~$3k), Growth (~$50k), and Scale (~$150k) demos stay fresh.
 *
 * Usage examples:
 *   DATABASE_URL=... npm run sandbox:revenue -- --tier=starter --days=30
 *   DATABASE_URL=... node backend/scripts/generateSandboxRevenue.js --tier=growth --dry-run
 */
require('dotenv/config');
const { Pool } = require('pg');
const { buildSslConfig } = require('./dbSsl');
const {
  SANDBOX_PUBLISHER_ID,
  FAKE_ADAPTERS,
} = require('./sandboxConstants');

const DATABASE_URL = process.env.DATABASE_URL;
if (!DATABASE_URL) {
  console.error('DATABASE_URL is required to seed sandbox revenue.');
  process.exit(1);
}

const pool = new Pool({
  connectionString: DATABASE_URL,
  ssl: buildSslConfig(),
});

const TIERS = {
  starter: {
    monthlyRevenue: 3000,
    baseCtr: 0.008,
  },
  growth: {
    monthlyRevenue: 50000,
    baseCtr: 0.011,
  },
  scale: {
    monthlyRevenue: 150000,
    baseCtr: 0.014,
  },
};

const PLACEMENT_WEIGHTS = {
  interstitial: 1,
  rewarded: 1.15,
  banner: 0.4,
};

const CTR_MULTIPLIERS = {
  interstitial: 1,
  rewarded: 0.85,
  banner: 0.45,
};

function parseArgs(argv) {
  const opts = {
    tier: 'all',
    days: 30,
    dryRun: false,
    keepHistory: false,
    start: null,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--tier' && argv[i + 1]) {
      opts.tier = argv[i + 1].toLowerCase();
      i += 1;
    } else if (arg === '--days' && argv[i + 1]) {
      opts.days = Math.max(1, parseInt(argv[i + 1], 10));
      i += 1;
    } else if (arg === '--start' && argv[i + 1]) {
      opts.start = argv[i + 1];
      i += 1;
    } else if (arg === '--dry-run') {
      opts.dryRun = true;
    } else if (arg === '--keep-history') {
      opts.keepHistory = true;
    }
  }
  return opts;
}

function resolveTiers(requestedTier) {
  if (requestedTier === 'all') {
    return Object.keys(TIERS);
  }
  if (!TIERS[requestedTier]) {
    throw new Error(`Unknown tier "${requestedTier}". Use starter|growth|scale|all.`);
  }
  return [requestedTier];
}

function formatDate(date) {
  return date.toISOString().slice(0, 10);
}

function addDays(date, days) {
  const cloned = new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()));
  cloned.setUTCDate(cloned.getUTCDate() + days);
  return cloned;
}

function weightForType(type) {
  return PLACEMENT_WEIGHTS[type] ?? 0.75;
}

function ctrFor(type, baseCtr) {
  return baseCtr * (CTR_MULTIPLIERS[type] ?? 0.7);
}

function seededNoise(seed) {
  const value = Math.sin(seed) * 10000;
  return value - Math.floor(value);
}

function hashSeed(...parts) {
  const joined = parts.join('|');
  let hash = 0;
  for (let i = 0; i < joined.length; i += 1) {
    hash = (hash * 31 + joined.charCodeAt(i)) >>> 0;
  }
  return hash || 1;
}

async function fetchPlacements(client) {
  const result = await client.query(
    `SELECT pl.id, pl.type
       FROM placements pl
       JOIN apps a ON a.id = pl.app_id
      WHERE a.publisher_id = $1
      ORDER BY pl.name`,
    [SANDBOX_PUBLISHER_ID]
  );
  return result.rows;
}

function resolveStartDate(days, explicitStart) {
  if (explicitStart) {
    const parsed = new Date(`${explicitStart}T00:00:00Z`);
    if (Number.isNaN(parsed.getTime())) {
      throw new Error(`Invalid --start value: ${explicitStart}`);
    }
    return parsed;
  }
  const now = new Date();
  const base = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  base.setUTCDate(base.getUTCDate() - (days - 1));
  return base;
}

async function seedTier(client, tierId, tierConfig, placements, options) {
  if (placements.length === 0) {
    throw new Error('No sandbox placements found; run setupSandboxNetworks first.');
  }

  const startDate = resolveStartDate(options.days, options.start);
  const endDate = addDays(startDate, options.days - 1);
  const dailyTarget = tierConfig.monthlyRevenue / Math.max(options.days, 1);
  const placementWeights = placements.map((placement) => weightForType(placement.type));
  const weightSum = placementWeights.reduce((sum, weight) => sum + weight, 0);

  const rows = [];
  for (let day = 0; day < options.days; day += 1) {
    const eventDate = addDays(startDate, day);
    const daySeed = hashSeed(tierId, day, eventDate.toISOString());
    const dayVariance = 0.9 + seededNoise(daySeed) * 0.2; // 0.9x - 1.1x
    const dayRevenue = dailyTarget * dayVariance;

    placements.forEach((placement, index) => {
      const baseShare = (placementWeights[index] / weightSum) * dayRevenue;
      const shareSeed = hashSeed(tierId, day, placement.id);
      const shareVariance = 0.85 + seededNoise(shareSeed) * 0.3;
      const revenueUsd = Number((baseShare * shareVariance).toFixed(2));
      if (revenueUsd <= 0) {
        return;
      }

      const adapter = FAKE_ADAPTERS[(index + day) % FAKE_ADAPTERS.length];
      const impressions = Math.max(80, Math.round((revenueUsd * 1000) / Math.max(adapter.ecpm, 0.5)));
      const ctr = ctrFor(placement.type, tierConfig.baseCtr);
      const clicks = Math.max(0, Math.round(impressions * ctr));

      rows.push({
        placementId: placement.id,
        adapterId: adapter.id,
        impressions,
        clicks,
        revenue: revenueUsd,
        eventDate: formatDate(eventDate),
      });
    });
  }

  if (options.dryRun) {
    const total = rows.reduce((sum, row) => sum + row.revenue, 0);
    console.log(
      `[dry-run] ${tierId} would insert ${rows.length} revenue_events rows totaling $${total.toFixed(2)} ` +
        `(${formatDate(startDate)} → ${formatDate(endDate)})`
    );
    return;
  }

  await client.query('BEGIN');
  try {
    if (!options.keepHistory) {
      await client.query(
        `DELETE FROM revenue_events WHERE publisher_id = $1 AND event_date BETWEEN $2 AND $3`,
        [SANDBOX_PUBLISHER_ID, formatDate(startDate), formatDate(endDate)]
      );
    }

    for (const row of rows) {
      await client.query(
        `INSERT INTO revenue_events (publisher_id, placement_id, adapter_id, impressions, clicks, revenue, event_date)
         VALUES ($1, $2, $3, $4, $5, $6, $7)`,
        [
          SANDBOX_PUBLISHER_ID,
          row.placementId,
          row.adapterId,
          row.impressions,
          row.clicks,
          row.revenue,
          row.eventDate,
        ]
      );
    }

    await client.query('COMMIT');
    const total = rows.reduce((sum, row) => sum + row.revenue, 0);
    console.log(
      `• Seeded ${rows.length} ${tierId} revenue rows totaling $${total.toFixed(2)} ` +
        `(${formatDate(startDate)} → ${formatDate(endDate)})`
    );
  } catch (error) {
    await client.query('ROLLBACK');
    throw error;
  }
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  let tiers;
  try {
    tiers = resolveTiers(args.tier);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
    return;
  }

  const client = await pool.connect();
  try {
    const placements = await fetchPlacements(client);
    for (const tierId of tiers) {
      await seedTier(client, tierId, TIERS[tierId], placements, {
        days: args.days,
        start: args.start,
        dryRun: args.dryRun,
        keepHistory: args.keepHistory,
      });
    }
  } catch (error) {
    console.error('Failed to seed sandbox revenue:', error.message);
    process.exitCode = 1;
  } finally {
    client.release();
    await pool.end();
  }
}

main();
