const { Pool } = require('pg');
const pool = new Pool({ connectionString: process.env.DATABASE_URL });

async function run() {
  const days = 30;
  const endTime = new Date(Date.now() + 5 * 60 * 1000);
  const startTime = new Date(endTime.getTime() - days * 24 * 60 * 60 * 1000);
  
  console.log('Start:', startTime);
  console.log('End:', endTime);

  const sql = `
        WITH impression_metrics AS (
          SELECT
            COUNT(*)::bigint AS total_requests,
            COUNT(*) FILTER (WHERE status = 'success' OR filled IS TRUE)::bigint AS successful_requests,
            COUNT(*) FILTER (WHERE status = 'error')::bigint AS error_requests,
            percentile_cont(0.95) WITHIN GROUP (ORDER BY latency_ms)
              FILTER (WHERE latency_ms IS NOT NULL) AS latency_p95_ms
          FROM analytics_impressions
          WHERE observed_at >= $1
            AND observed_at < $2
            AND is_test_mode = false
        )
        SELECT * FROM impression_metrics
  `;

  try {
    const res = await pool.query(sql, [startTime, endTime]);
    console.log(JSON.stringify(res.rows, null, 2));
  } catch (e) {
    console.error(e);
  } finally {
    await pool.end();
  }
}

run();
