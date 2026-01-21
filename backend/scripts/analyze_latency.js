#!/usr/bin/env node
/**
 * Analyze latency distribution and identify bottlenecks
 */

require('dotenv').config({ path: '/opt/adproject/.env' });
const { Pool } = require('pg');
const pool = new Pool({ connectionString: process.env.DATABASE_URL });

async function analyze() {
  console.log('=== Latency Analysis ===\n');
  
  // 1. Overall latency distribution
  const dist = await pool.query(`
    SELECT 
      CASE 
        WHEN latency_ms < 100 THEN '0-100ms'
        WHEN latency_ms < 200 THEN '100-200ms'
        WHEN latency_ms < 300 THEN '200-300ms'
        WHEN latency_ms < 400 THEN '300-400ms'
        WHEN latency_ms < 500 THEN '400-500ms'
        WHEN latency_ms < 600 THEN '500-600ms'
        ELSE '600ms+'
      END as bucket,
      COUNT(*) as cnt,
      ROUND(COUNT(*) * 100.0 / SUM(COUNT(*)) OVER(), 2) as pct
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '7 days' 
      AND is_test_mode = false
      AND latency_ms IS NOT NULL
    GROUP BY 1
    ORDER BY MIN(latency_ms)
  `);
  console.log('1. Latency distribution (7d):');
  console.table(dist.rows);
  
  // 2. Latency by adapter
  const byAdapter = await pool.query(`
    SELECT 
      COALESCE(adapter_name, 'unknown') as adapter,
      COUNT(*) as cnt,
      ROUND(AVG(latency_ms)) as avg_ms,
      ROUND(percentile_cont(0.50) WITHIN GROUP (ORDER BY latency_ms)) as p50_ms,
      ROUND(percentile_cont(0.95) WITHIN GROUP (ORDER BY latency_ms)) as p95_ms,
      ROUND(percentile_cont(0.99) WITHIN GROUP (ORDER BY latency_ms)) as p99_ms,
      MIN(latency_ms) as min_ms,
      MAX(latency_ms) as max_ms
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '7 days' 
      AND is_test_mode = false
      AND latency_ms IS NOT NULL
    GROUP BY 1
    ORDER BY cnt DESC
    LIMIT 10
  `);
  console.log('\n2. Latency by adapter:');
  console.table(byAdapter.rows);
  
  // 3. Latency by hour of day (to find patterns)
  const byHour = await pool.query(`
    SELECT 
      EXTRACT(HOUR FROM observed_at) as hour,
      COUNT(*) as cnt,
      ROUND(AVG(latency_ms)) as avg_ms,
      ROUND(percentile_cont(0.95) WITHIN GROUP (ORDER BY latency_ms)) as p95_ms
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '7 days' 
      AND is_test_mode = false
      AND latency_ms IS NOT NULL
    GROUP BY 1
    ORDER BY 1
  `);
  console.log('\n3. Latency by hour of day:');
  console.table(byHour.rows);
  
  // 4. Check what meta info we have on high latency requests
  const highLatencyMeta = await pool.query(`
    SELECT 
      meta->>'source' as source,
      meta->>'operation' as operation,
      COUNT(*) as cnt,
      ROUND(AVG(latency_ms)) as avg_ms
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '7 days' 
      AND is_test_mode = false
      AND latency_ms > 500
    GROUP BY 1, 2
    ORDER BY cnt DESC
    LIMIT 10
  `);
  console.log('\n4. High latency (>500ms) requests by source/operation:');
  console.table(highLatencyMeta.rows);
  
  // 5. Sample some high latency rows to understand the meta
  const samples = await pool.query(`
    SELECT id, adapter_name, latency_ms, meta, observed_at
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '1 day' 
      AND is_test_mode = false
      AND latency_ms > 500
    ORDER BY latency_ms DESC
    LIMIT 5
  `);
  console.log('\n5. Sample high-latency rows:');
  samples.rows.forEach(r => {
    console.log(`  ID ${r.id}: ${r.latency_ms}ms, adapter=${r.adapter_name}, meta=${JSON.stringify(r.meta)}`);
  });
  
  // 6. Latency trend over time (last 7 days by day)
  const trend = await pool.query(`
    SELECT 
      DATE(observed_at) as day,
      COUNT(*) as cnt,
      ROUND(AVG(latency_ms)) as avg_ms,
      ROUND(percentile_cont(0.95) WITHIN GROUP (ORDER BY latency_ms)) as p95_ms
    FROM analytics_impressions 
    WHERE observed_at >= NOW() - INTERVAL '7 days' 
      AND is_test_mode = false
      AND latency_ms IS NOT NULL
    GROUP BY 1
    ORDER BY 1
  `);
  console.log('\n6. Latency trend (last 7 days):');
  console.table(trend.rows);
  
  await pool.end();
  console.log('\n=== Done ===');
}

analyze().catch(e => {
  console.error('Error:', e);
  process.exit(1);
});
