#!/usr/bin/env node
/**
 * Fix Sales Automation Schema
 * Run on Test Server to add missing columns and clean data
 */
const { Client } = require('pg');

const client = new Client({
  host: '127.0.0.1',
  port: 6432,
  user: 'adproject',
  password: 'xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f',
  database: 'adproject',
  ssl: false
});

async function main() {
  await client.connect();
  console.log('Connected to database');

  // 1. Add missing columns to sa_scrape_jobs
  console.log('\n=== FIXING sa_scrape_jobs ===');
  await client.query('ALTER TABLE sa_scrape_jobs ADD COLUMN IF NOT EXISTS apps_discovered INTEGER DEFAULT 0');
  await client.query('ALTER TABLE sa_scrape_jobs ADD COLUMN IF NOT EXISTS apps_processed INTEGER DEFAULT 0');
  console.log('✅ Added apps_discovered and apps_processed columns');

  // 2. Add missing column to sa_contacts
  console.log('\n=== FIXING sa_contacts ===');
  await client.query('ALTER TABLE sa_contacts ADD COLUMN IF NOT EXISTS last_scraped_at TIMESTAMP');
  console.log('✅ Added last_scraped_at column');

  // 3. Clean up invalid sources
  console.log('\n=== CLEANING INVALID SOURCES ===');
  const beforeCount = await client.query('SELECT count(*) FROM sa_sources WHERE source_type IS NULL OR source_type = \'\'');
  console.log('Invalid sources before:', beforeCount.rows[0].count);
  
  await client.query('DELETE FROM sa_sources WHERE source_type IS NULL OR source_type = \'\'');
  console.log('✅ Removed invalid sources');

  // 4. Clear stuck running jobs
  console.log('\n=== CLEARING STUCK JOBS ===');
  const stuckJobs = await client.query('SELECT count(*) FROM sa_scrape_jobs WHERE status = \'running\'');
  console.log('Stuck running jobs:', stuckJobs.rows[0].count);
  
  await client.query('UPDATE sa_scrape_jobs SET status = \'failed\', error_message = \'Cleared: stuck in running state\' WHERE status = \'running\'');
  console.log('✅ Marked stuck jobs as failed');

  // 5. Verify final state
  console.log('\n=== VERIFICATION ===');
  const cols = await client.query("SELECT column_name FROM information_schema.columns WHERE table_name = 'sa_scrape_jobs'");
  console.log('sa_scrape_jobs columns:', cols.rows.map(r => r.column_name).join(', '));
  
  const jobStats = await client.query('SELECT status, count(*) FROM sa_scrape_jobs GROUP BY status');
  console.log('Job stats:', jobStats.rows);
  
  const sourceStats = await client.query('SELECT source_type, count(*) FROM sa_sources GROUP BY source_type ORDER BY count(*) DESC');
  console.log('Source types:', sourceStats.rows);

  await client.end();
  console.log('\n✅ Schema fixes complete!');
}

main().catch(e => {
  console.error('Error:', e);
  process.exit(1);
});
