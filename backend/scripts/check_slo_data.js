#!/usr/bin/env node
/**
 * Check SLO data in analytics_impressions to diagnose availability calculation
 */

require('dotenv').config({ path: '/opt/adproject/.env' });
const { Pool } = require('pg');
const pool = new Pool({ connectionString: process.env.DATABASE_URL });

async function main() {
  console.log('=== SLO Data Diagnostics ===\n');
  
  // 1. Check status/filled distribution
  const statusDist = await pool.query(`
    SELECT status, filled, COUNT(*) as cnt 
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '30 days' AND is_test_mode = false
    GROUP BY status, filled 
    ORDER BY cnt DESC 
    LIMIT 15
  `);
  console.log('1. Status/filled distribution (last 30d):');
  console.table(statusDist.rows);
  
  // 2. Check null counts
  const nullCount = await pool.query(`
    SELECT 
      COUNT(*) FILTER (WHERE status IS NULL) as null_status,
      COUNT(*) FILTER (WHERE filled IS NULL) as null_filled,
      COUNT(*) FILTER (WHERE status IS NOT NULL) as has_status,
      COUNT(*) FILTER (WHERE filled IS NOT NULL) as has_filled,
      COUNT(*) as total
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '30 days' AND is_test_mode = false
  `);
  console.log('\n2. Null counts:');
  console.table(nullCount.rows);
  
  // 3. Check distinct status values
  const distinctStatus = await pool.query(`
    SELECT DISTINCT status FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '30 days'
    LIMIT 20
  `);
  console.log('\n3. Distinct status values:');
  console.log(distinctStatus.rows.map(r => r.status));
  
  // 4. Check latency distribution
  const latencyDist = await pool.query(`
    SELECT 
      COUNT(*) FILTER (WHERE latency_ms IS NOT NULL) as has_latency,
      COUNT(*) FILTER (WHERE latency_ms IS NULL) as null_latency,
      MIN(latency_ms) as min_latency,
      MAX(latency_ms) as max_latency,
      AVG(latency_ms)::int as avg_latency,
      percentile_cont(0.95) WITHIN GROUP (ORDER BY latency_ms)::int as p95_latency
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '30 days' AND is_test_mode = false
  `);
  console.log('\n4. Latency distribution:');
  console.table(latencyDist.rows);
  
  // 5. Sample a few rows to see actual data
  const sample = await pool.query(`
    SELECT id, status, filled, latency_ms, is_test_mode, observed_at
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '1 day' AND is_test_mode = false
    ORDER BY observed_at DESC
    LIMIT 5
  `);
  console.log('\n5. Sample recent rows:');
  console.table(sample.rows);
  
  // 6. Check the schema
  const schema = await pool.query(`
    SELECT column_name, data_type, is_nullable 
    FROM information_schema.columns 
    WHERE table_name = 'analytics_impressions'
    ORDER BY ordinal_position
  `);
  console.log('\n6. Table schema:');
  console.table(schema.rows);
  
  await pool.end();
  console.log('\n=== Done ===');
}

main().catch(e => {
  console.error('Error:', e);
  process.exit(1);
});
