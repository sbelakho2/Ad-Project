/**
 * SALES AUTOMATION SCHEDULER
 * 
 * Automated cron scheduler for the sales automation pipeline.
 * ONLY runs when SALES_AUTOMATION_ROLE=execution (Test Server).
 * 
 * Schedule:
 * - Health Check: Every 5 minutes
 * - Discovery:    Every 6 hours (batch 5)
 * - Scoring:      Every 2 hours (batch 100)
 * - Enrichment:   Every hour (batch 20)
 * - Composition:  Daily at 9 AM (minScore 70)
 * - Sending:      Daily at 10 AM (max 50/day)
 * - Training:     Weekly Monday at 3 AM
 * - Bandit Close: Every 30 minutes (record outcomes)
 */

import cron from 'node-cron';
import { logger } from '../logger';
import { queryOperator, queryReadOnly } from '../database';

// Track scheduled tasks for graceful shutdown
const scheduledTasks: cron.ScheduledTask[] = [];
let isSchedulerRunning = false;

// Config for scheduled jobs
interface ScheduleConfig {
  discoverBatch: number;
  enrichBatch: number;
  scoreBatch: number;
  composeBatch: number;
  sendBatch: number;
  minComposeScore: number;
  maxDailySends: number;
}

const DEFAULT_CONFIG: ScheduleConfig = {
  discoverBatch: 5,
  enrichBatch: 20,
  scoreBatch: 100,
  composeBatch: 10,
  sendBatch: 50,
  minComposeScore: 70,
  maxDailySends: 50,
};

/**
 * Call internal API endpoint for run-cycle
 */
async function callRunCycle(params: Record<string, unknown>): Promise<unknown> {
  const port = process.env.PORT || 4001;
  const apiKey = process.env.AUTONOMOUS_API_KEY || '';
  
  try {
    const response = await fetch(`http://localhost:${port}/api/autonomous/run-cycle`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'X-API-Key': apiKey,
      },
      body: JSON.stringify(params),
    });
    
    if (!response.ok) {
      const text = await response.text();
      throw new Error(`API call failed: ${response.status} - ${text}`);
    }
    
    return await response.json();
  } catch (error) {
    throw new Error(`Run cycle request failed: ${(error as Error).message}`);
  }
}

/**
 * Run a scheduled job with error handling and logging
 * 
 * FIX: Uses correct column names (job_type, error_message, duration_ms)
 * FIX: Properly captures error message and stack trace
 */
async function runScheduledJob(
  jobName: string,
  jobFn: () => Promise<unknown>
): Promise<void> {
  const startTime = Date.now();
  logger.info(`[Scheduler] Starting ${jobName}`, { jobName });
  
  try {
    const result = await jobFn();
    const duration = Date.now() - startTime;
    
    // Log to database with CORRECT column names
    await queryOperator(`
      INSERT INTO sa_autonomous_jobs 
        (job_type, job_name, status, started_at, completed_at, output_data, duration_ms)
      VALUES 
        ($1, $2, 'completed', $3, CURRENT_TIMESTAMP, $4, $5)
    `, [
      jobName,           // job_type
      `${jobName}_${Date.now()}`,  // job_name (unique)
      new Date(startTime), 
      JSON.stringify(result || {}),
      duration
    ]).catch((err) => {
      logger.warn(`[Scheduler] Failed to log job to database`, { jobName, error: (err as Error).message });
    });
    
    logger.info(`[Scheduler] ${jobName} completed`, { jobName, durationMs: duration, result });
  } catch (error) {
    const duration = Date.now() - startTime;
    const errorMsg = (error as Error).message;
    const errorStack = (error as Error).stack || '';
    
    // Log failure to database with FULL ERROR CAPTURE
    await queryOperator(`
      INSERT INTO sa_autonomous_jobs 
        (job_type, job_name, status, started_at, completed_at, output_data, error_message, error_stack, duration_ms)
      VALUES 
        ($1, $2, 'failed', $3, CURRENT_TIMESTAMP, $4, $5, $6, $7)
    `, [
      jobName,           // job_type
      `${jobName}_${Date.now()}`,  // job_name (unique)
      new Date(startTime),
      JSON.stringify({ error: errorMsg }),
      errorMsg,          // error_message - CAPTURED!
      errorStack,        // error_stack - CAPTURED!
      duration
    ]).catch((err) => {
      logger.warn(`[Scheduler] Failed to log job failure to database`, { jobName, error: (err as Error).message });
    });
    
    logger.error(`[Scheduler] ${jobName} failed`, { jobName, durationMs: duration, error: errorMsg });
  }
}

/**
 * Health check - verify system is operational
 */
async function runHealthCheck(): Promise<{ status: string; checks: Record<string, boolean> }> {
  const checks: Record<string, boolean> = {};
  
  // DB connectivity
  try {
    await queryReadOnly('SELECT 1 as ok');
    checks.database = true;
  } catch {
    checks.database = false;
  }
  
  // Edge worker connectivity
  const edgeUrl = process.env.EDGE_WORKER_URL;
  if (edgeUrl) {
    try {
      const resp = await fetch(`${edgeUrl}/health`, {
        headers: { 'X-Edge-Secret': process.env.EDGE_WORKER_SECRET || '' },
        signal: AbortSignal.timeout(5000),
      });
      checks.edgeWorker = resp.ok;
    } catch {
      checks.edgeWorker = false;
    }
  }
  
  // Email service - ApexMail replaces Resend (check SMTP config)
  checks.emailConfigured = !!(process.env.SMTP_HOST || process.env.EMAIL_FROM);
  
  const allPassing = Object.values(checks).every(v => v);
  return { status: allPassing ? 'healthy' : 'degraded', checks };
}

/**
 * Discovery - find new leads from sources
 */
async function runDiscovery(batch: number): Promise<unknown> {
  const port = process.env.PORT || 4001;
  const apiKey = process.env.AUTONOMOUS_API_KEY || '';
  
  // FIXED (Jan 2026): Use correct endpoint /discover instead of non-existent /jobs
  const response = await fetch(`http://localhost:${port}/api/autonomous/discover`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
    body: JSON.stringify({ batchSize: batch }),
  });
  
  if (!response.ok) throw new Error(`Discovery failed: ${response.status}`);
  return response.json();
}

/**
 * Scoring - score unscored contacts
 */
async function runScoring(batch: number): Promise<unknown> {
  const port = process.env.PORT || 4001;
  const apiKey = process.env.AUTONOMOUS_API_KEY || '';
  
  // FIXED (Jan 2026): Use correct endpoint /score instead of non-existent /jobs
  const response = await fetch(`http://localhost:${port}/api/autonomous/score`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
    body: JSON.stringify({ batchSize: batch }),
  });
  
  if (!response.ok) throw new Error(`Scoring failed: ${response.status}`);
  return response.json();
}

/**
 * Enrichment - find emails for contacts
 */
async function runEnrichment(batch: number): Promise<unknown> {
  const port = process.env.PORT || 4001;
  const apiKey = process.env.AUTONOMOUS_API_KEY || '';
  
  // FIXED (Jan 2026): Use correct endpoint /enrich instead of non-existent /jobs
  const response = await fetch(`http://localhost:${port}/api/autonomous/enrich`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
    body: JSON.stringify({ batchSize: batch }),
  });
  
  if (!response.ok) throw new Error(`Enrichment failed: ${response.status}`);
  return response.json();
}

/**
 * Compose messages for hot leads
 */
async function runCompose(batch: number, minScore: number): Promise<{ composed: number }> {
  const port = process.env.PORT || 4001;
  const apiKey = process.env.AUTONOMOUS_API_KEY || '';
  
  // Find hot leads without messages
  const leads = await queryReadOnly<{ id: string }>(`
    SELECT c.id 
    FROM sa_contacts c
    LEFT JOIN sa_outbound_messages om ON om.contact_id = c.id
    WHERE c.score >= $1
      AND c.email IS NOT NULL
      AND c.email != ''
      AND om.id IS NULL
    ORDER BY c.score DESC
    LIMIT $2
  `, [minScore, batch]);
  
  let composed = 0;
  for (const lead of leads) {
    try {
      const response = await fetch(`http://localhost:${port}/api/autonomous/compose`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
        body: JSON.stringify({ contactId: lead.id }),
      });
      
      if (response.ok) composed++;
      
      // Small delay between compositions
      await new Promise(resolve => setTimeout(resolve, 500));
    } catch {
      // Continue with next lead
    }
  }
  
  return { composed };
}

/**
 * Send pending messages (with daily limit)
 */
async function runSend(maxDaily: number): Promise<{ sent: number; failed: number; remaining: number }> {
  const port = process.env.PORT || 4001;
  const apiKey = process.env.AUTONOMOUS_API_KEY || '';
  
  // Check how many sent today
  const [todayCount] = await queryReadOnly<{ count: number }>(`
    SELECT COUNT(*) as count 
    FROM sa_outbound_messages 
    WHERE sent_at >= CURRENT_DATE
  `);
  
  const sentToday = todayCount?.count || 0;
  const remaining = Math.max(0, maxDaily - sentToday);
  
  if (remaining === 0) {
    return { sent: 0, failed: 0, remaining: 0 };
  }
  
  // Get pending messages
  const pending = await queryReadOnly<{ id: string }>(`
    SELECT om.id 
    FROM sa_outbound_messages om
    JOIN sa_contacts c ON c.id = om.contact_id
    WHERE om.status = 'pending'
      AND c.email IS NOT NULL
      AND c.email != ''
    ORDER BY c.score DESC, om.created_at ASC
    LIMIT $1
  `, [remaining]);
  
  let sent = 0;
  let failed = 0;
  
  for (const msg of pending) {
    try {
      const response = await fetch(`http://localhost:${port}/api/autonomous/send`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
        body: JSON.stringify({ messageId: msg.id }),
      });
      
      if (response.ok) {
        sent++;
      } else {
        failed++;
      }
      
      // Rate limit: 3 second delay between sends
      await new Promise(resolve => setTimeout(resolve, 3000));
    } catch {
      failed++;
    }
  }
  
  return { sent, failed, remaining: remaining - sent - failed };
}

/**
 * Record bandit outcomes from reply classifications
 * 
 * FIX: Uses subject_arm_id instead of bandit_arm_id (correct column name)
 * FIX: Uses total_trials/total_successes instead of trials/successes  
 * FIX: Also records opens as positive outcomes for learning
 */
async function closeBanditLoop(): Promise<{ recorded: number; opens: number; positiveReplies: number; negativeReplies: number }> {
  let recorded = 0;
  let opens = 0;
  let positiveReplies = 0;
  let negativeReplies = 0;
  
  // ====== PART 1: Record email opens as weak positive signal ======
  // Opens indicate subject line was compelling enough to click
  const unrecordedOpens = await queryReadOnly<{
    id: string;
    contact_id: string;
    subject_arm_id: string;
  }>(`
    SELECT om.id, om.contact_id, om.subject_arm_id
    FROM sa_outbound_messages om
    WHERE om.opened_at IS NOT NULL
      AND om.subject_arm_id IS NOT NULL
      AND NOT EXISTS (
        SELECT 1 FROM sa_bandit_trials bt
        WHERE bt.arm_id = om.subject_arm_id
          AND bt.contact_id = om.contact_id
      )
  `);
  
  for (const open of unrecordedOpens) {
    try {
      // Record the trial with weak positive (open = 0.3 reward)
      await queryOperator(`
        INSERT INTO sa_bandit_trials (arm_id, contact_id, success, sampled_score)
        VALUES ($1, $2, true, 0.3)
        ON CONFLICT DO NOTHING
      `, [open.subject_arm_id, open.contact_id]);
      
      // Update arm statistics with CORRECT column names
      await queryOperator(`
        UPDATE sa_bandit_arms
        SET 
          total_trials = total_trials + 1,
          total_successes = total_successes + 1,
          updated_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [open.subject_arm_id]);
      
      opens++;
      recorded++;
    } catch (err) {
      logger.warn('[BanditClose] Failed to record open', { 
        armId: open.subject_arm_id, 
        error: (err as Error).message 
      });
    }
  }
  
  // ====== PART 2: Record positive replies (INTERESTED, MEETING_REQUEST) ======
  const positiveReplyData = await queryReadOnly<{
    id: string;
    contact_id: string;
    classification: string;
    subject_arm_id: string;
  }>(`
    SELECT r.id, r.contact_id, r.classification, om.subject_arm_id
    FROM sa_inbox_messages r
    JOIN sa_outbound_messages om ON om.contact_id = r.contact_id
    WHERE r.classification IN ('interested', 'meeting', 'INTERESTED', 'MEETING_REQUEST', 'positive')
      AND om.subject_arm_id IS NOT NULL
      AND NOT EXISTS (
        SELECT 1 FROM sa_bandit_trials bt
        WHERE bt.arm_id = om.subject_arm_id
          AND bt.contact_id = r.contact_id
      )
  `);
  
  for (const reply of positiveReplyData) {
    try {
      // Determine reward: meeting request = 1.0, interested = 0.7
      const reward = reply.classification.toUpperCase().includes('MEETING') ? 1.0 : 0.7;
      
      // Record the trial
      await queryOperator(`
        INSERT INTO sa_bandit_trials (arm_id, contact_id, success, sampled_score)
        VALUES ($1, $2, true, $3)
        ON CONFLICT DO NOTHING
      `, [reply.subject_arm_id, reply.contact_id, reward]);
      
      // Update arm statistics with CORRECT column names
      await queryOperator(`
        UPDATE sa_bandit_arms
        SET 
          total_trials = total_trials + 1,
          total_successes = total_successes + 1,
          updated_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [reply.subject_arm_id]);
      
      positiveReplies++;
      recorded++;
    } catch (err) {
      logger.warn('[BanditClose] Failed to record positive reply', { 
        armId: reply.subject_arm_id, 
        error: (err as Error).message 
      });
    }
  }
  
  // ====== PART 3: Record negative outcomes (bounces, unsubscribes) ======
  const negativeReplyData = await queryReadOnly<{
    id: string;
    contact_id: string;
    classification: string;
    subject_arm_id: string;
  }>(`
    SELECT r.id, r.contact_id, r.classification, om.subject_arm_id
    FROM sa_inbox_messages r
    JOIN sa_outbound_messages om ON om.contact_id = r.contact_id
    WHERE r.classification IN ('not_interested', 'unsubscribe', 'spam', 'NOT_INTERESTED', 'UNSUBSCRIBE', 'SPAM_COMPLAINT')
      AND om.subject_arm_id IS NOT NULL
      AND NOT EXISTS (
        SELECT 1 FROM sa_bandit_trials bt
        WHERE bt.arm_id = om.subject_arm_id
          AND bt.contact_id = r.contact_id
      )
  `);
  
  for (const reply of negativeReplyData) {
    try {
      // Record negative trial
      await queryOperator(`
        INSERT INTO sa_bandit_trials (arm_id, contact_id, success, sampled_score)
        VALUES ($1, $2, false, 0)
        ON CONFLICT DO NOTHING
      `, [reply.subject_arm_id, reply.contact_id]);
      
      // Update arm statistics (trials only, not successes)
      await queryOperator(`
        UPDATE sa_bandit_arms
        SET 
          total_trials = total_trials + 1,
          updated_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [reply.subject_arm_id]);
      
      negativeReplies++;
      recorded++;
    } catch (err) {
      logger.warn('[BanditClose] Failed to record negative reply', { 
        armId: reply.subject_arm_id, 
        error: (err as Error).message 
      });
    }
  }
  
  if (recorded > 0) {
    logger.info('[BanditClose] Recorded outcomes', { 
      total: recorded, 
      opens, 
      positiveReplies, 
      negativeReplies 
    });
  }
  
  return { recorded, opens, positiveReplies, negativeReplies };
}

/**
 * Re-classify UNKNOWN inbox messages
 */
async function reclassifyUnknown(): Promise<{ reclassified: number }> {
  const port = process.env.PORT || 4001;
  const apiKey = process.env.AUTONOMOUS_API_KEY || '';
  
  // Find UNKNOWN messages
  const unknowns = await queryReadOnly<{ id: string }>(`
    SELECT id FROM sa_inbox_messages
    WHERE classification = 'UNKNOWN'
    LIMIT 20
  `);
  
  let reclassified = 0;
  
  for (const msg of unknowns) {
    try {
      // Try to reclassify via the classify endpoint
      const response = await fetch(`http://localhost:${port}/api/autonomous/classify`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-API-Key': apiKey },
        body: JSON.stringify({ messageId: msg.id }),
      });
      
      if (response.ok) {
        const result = (await response.json()) as { classification?: string };
        if (result.classification && result.classification !== 'UNKNOWN') {
          reclassified++;
        }
      }
      
      await new Promise(resolve => setTimeout(resolve, 200));
    } catch {
      // Continue with next message
    }
  }
  
  return { reclassified };
}

/**
 * Initialize the sales automation scheduler
 */
export function initializeSalesScheduler(): void {
  // Only run on execution server
  if (process.env.SALES_AUTOMATION_ROLE !== 'execution') {
    logger.info('[Scheduler] Skipping scheduler - not in execution mode');
    return;
  }
  
  if (isSchedulerRunning) {
    logger.warn('[Scheduler] Already running, skipping duplicate initialization');
    return;
  }
  
  logger.info('[Scheduler] Initializing sales automation scheduler');
  
  const config = DEFAULT_CONFIG;
  
  // ======== Health Check - Every 5 minutes ========
  const healthJob = cron.schedule('*/5 * * * *', async () => {
    await runScheduledJob('health_check', runHealthCheck);
  });
  scheduledTasks.push(healthJob);
  logger.info('[Scheduler] Health check scheduled: every 5 minutes');
  
  // ======== Safe Mode Update - Every 15 minutes ========
  // Updates safe mode state based on bounce/complaint rates
  const safeModeJob = cron.schedule('*/15 * * * *', async () => {
    await runScheduledJob('safe_mode_update', async () => {
      const { updateSafeModeState, checkSafeMode } = await import('./suppression-list.js');
      await updateSafeModeState();
      const status = await checkSafeMode();
      return {
        mode: status.mode,
        canSend: status.canSend,
        bounceRate: status.bounceRate,
        complaintRate: status.complaintRate,
      };
    });
  });
  scheduledTasks.push(safeModeJob);
  logger.info('[Scheduler] Safe mode update scheduled: every 15 minutes');
  
  // ======== Discovery - Every 6 hours ========
  const discoveryJob = cron.schedule('0 */6 * * *', async () => {
    await runScheduledJob('discover', () => runDiscovery(config.discoverBatch));
  });
  scheduledTasks.push(discoveryJob);
  logger.info('[Scheduler] Discovery scheduled: every 6 hours');
  
  // ======== Scoring - Every 2 hours ========
  const scoringJob = cron.schedule('0 */2 * * *', async () => {
    await runScheduledJob('score', () => runScoring(config.scoreBatch));
  });
  scheduledTasks.push(scoringJob);
  logger.info('[Scheduler] Scoring scheduled: every 2 hours');
  
  // ======== Enrichment - Every hour ========
  const enrichJob = cron.schedule('30 * * * *', async () => {
    await runScheduledJob('enrich', () => runEnrichment(config.enrichBatch));
  });
  scheduledTasks.push(enrichJob);
  logger.info('[Scheduler] Enrichment scheduled: every hour at :30');
  
  // ======== Compose - Daily at 9 AM ========
  const composeJob = cron.schedule('0 9 * * *', async () => {
    await runScheduledJob('compose', () => runCompose(config.composeBatch, config.minComposeScore));
  });
  scheduledTasks.push(composeJob);
  logger.info('[Scheduler] Composition scheduled: daily at 9 AM');
  
  // ======== Send - Daily at 10 AM ========
  const sendJob = cron.schedule('0 10 * * *', async () => {
    await runScheduledJob('send', () => runSend(config.maxDailySends));
  });
  scheduledTasks.push(sendJob);
  logger.info('[Scheduler] Sending scheduled: daily at 10 AM');
  
  // ======== Bandit Loop Closure - Every 30 minutes ========
  const banditJob = cron.schedule('*/30 * * * *', async () => {
    await runScheduledJob('bandit_close', closeBanditLoop);
  });
  scheduledTasks.push(banditJob);
  logger.info('[Scheduler] Bandit loop closure scheduled: every 30 minutes');
  
  // ======== UNKNOWN Reclassification - Every hour ========
  const reclassifyJob = cron.schedule('45 * * * *', async () => {
    await runScheduledJob('reclassify_unknown', reclassifyUnknown);
  });
  scheduledTasks.push(reclassifyJob);
  logger.info('[Scheduler] UNKNOWN reclassification scheduled: every hour at :45');
  
  // ======== Full Cycle - Twice daily (6 AM, 6 PM) ========
  const fullCycleJob = cron.schedule('0 6,18 * * *', async () => {
    await runScheduledJob('full_cycle', () => callRunCycle({
      discoverBatch: config.discoverBatch,
      enrichBatch: config.enrichBatch,
      scoreBatch: config.scoreBatch,
      sendBatch: Math.floor(config.maxDailySends / 2), // Half daily limit per cycle
      dryRun: false,
    }));
  });
  scheduledTasks.push(fullCycleJob);
  logger.info('[Scheduler] Full cycle scheduled: 6 AM and 6 PM');
  
  isSchedulerRunning = true;
  
  logger.info('[Scheduler] Sales automation scheduler initialized', {
    totalJobs: scheduledTasks.length,
    config,
  });
  
  // Run initial health check after 30 seconds
  setTimeout(async () => {
    await runScheduledJob('health_check', runHealthCheck);
  }, 30000);
}

/**
 * Stop all scheduled tasks
 */
export function stopSalesScheduler(): void {
  logger.info('[Scheduler] Stopping sales automation scheduler');
  
  for (const task of scheduledTasks) {
    task.stop();
  }
  
  scheduledTasks.length = 0;
  isSchedulerRunning = false;
  
  logger.info('[Scheduler] Scheduler stopped');
}

/**
 * Get scheduler status
 */
export function getSchedulerStatus(): {
  running: boolean;
  taskCount: number;
  nextRuns: Record<string, string>;
} {
  return {
    running: isSchedulerRunning,
    taskCount: scheduledTasks.length,
    nextRuns: {
      healthCheck: 'Every 5 minutes',
      discovery: 'Every 6 hours',
      scoring: 'Every 2 hours',
      enrichment: 'Every hour at :30',
      composition: 'Daily at 9 AM',
      sending: 'Daily at 10 AM',
      banditClosure: 'Every 30 minutes',
      reclassification: 'Every hour at :45',
      fullCycle: '6 AM and 6 PM',
    },
  };
}

/**
 * Manually trigger a job
 */
export async function triggerScheduledJob(jobName: string): Promise<{ success: boolean; result?: unknown; error?: string }> {
  const config = DEFAULT_CONFIG;
  
  try {
    let result: unknown;
    
    switch (jobName) {
      case 'health_check':
        result = await runHealthCheck();
        break;
      case 'discover':
        result = await runDiscovery(config.discoverBatch);
        break;
      case 'score':
        result = await runScoring(config.scoreBatch);
        break;
      case 'enrich':
        result = await runEnrichment(config.enrichBatch);
        break;
      case 'compose':
        result = await runCompose(config.composeBatch, config.minComposeScore);
        break;
      case 'send':
        result = await runSend(config.maxDailySends);
        break;
      case 'bandit_close':
        result = await closeBanditLoop();
        break;
      case 'reclassify_unknown':
        result = await reclassifyUnknown();
        break;
      case 'full_cycle':
        result = await callRunCycle({
          discoverBatch: config.discoverBatch,
          enrichBatch: config.enrichBatch,
          scoreBatch: config.scoreBatch,
          sendBatch: config.maxDailySends,
          dryRun: false,
        });
        break;
      default:
        return { success: false, error: `Unknown job: ${jobName}` };
    }
    
    return { success: true, result };
  } catch (error) {
    return { success: false, error: (error as Error).message };
  }
}

/**
 * Get list of available jobs
 */
export function getAvailableJobs(): string[] {
  return [
    'health_check',
    'discover',
    'score',
    'enrich',
    'compose',
    'send',
    'bandit_close',
    'reclassify_unknown',
    'full_cycle',
  ];
}
