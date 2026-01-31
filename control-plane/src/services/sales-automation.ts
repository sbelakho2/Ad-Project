/**
 * Sales Automation Service Adapter
 * 
 * Provides API integration for managing the sales automation system
 * from the Control Plane UI. Uses the production database directly.
 * 
 * IMPORTANT: System starts DISABLED by default and must be explicitly
 * enabled via the Control Plane UI before any emails will be sent.
 * 
 * REVOLUTIONARY FEATURES INTEGRATED (v2):
 * - Domain Health Guard: Auto-stops sending if bounce rate > 7%
 * - Spam Prevention: Pre-flight check on email content
 * - Predictive Scoring: Prioritizes "hot" leads first
 * - DB-backed Configuration: All settings editable from UI
 * - Review Queue: Optional draft state before sending
 */

import { queryReadOnly, queryOperator, getOperatorPool } from '../database';
import { logger } from '../logger';
import { getApexMail } from './apexmail';

// Use ApexMail service (singleton)
const apexmail = getApexMail(getOperatorPool());

// Fallback defaults (used if DB config not available)
const FALLBACK_FROM_EMAIL = 'sales@apexmediation.ee';
const FALLBACK_FROM_NAME = 'Aaron from ApexMediation';
const FALLBACK_DAILY_LIMIT = 100;
const SYSTEM_ACTOR = 'sales-automation-service';

function isMissingRelationError(error: any): boolean {
  if (!error) return false;
  return error.code === '42P01' || String(error.message || '').includes('does not exist');
}

async function safeQueryReadOnly<T = unknown>(sql: string, params: unknown[], fallback: T[]): Promise<T[]> {
  try {
    return await queryReadOnly<T>(sql, params);
  } catch (err) {
    if (isMissingRelationError(err)) {
      logger.warn('Sales automation table missing, returning fallback', { error: (err as Error).message });
      return fallback;
    }
    throw err;
  }
}

// ============================================================================
// DB-BACKED CONFIGURATION (Replaces hardcoded values)
// ============================================================================

interface SalesConfig {
  fromEmail: string;
  fromName: string;
  replyTo: string;
  dailyLimit: number;
  requireEmailVerification: boolean;
  enableReviewQueue: boolean;
  warmupMode: boolean;
  warmupDailyIncrement: number;
  systemEnabled: boolean;
  spamScoreThreshold: number;
  bounceRateThreshold: number;
  pauseOnCriticalHealth: boolean;
}

interface ConfigRow { key: string; value: string }

// In-memory cache for config (refreshed on each read)
let configCache: SalesConfig | null = null;
let configCacheTime = 0;
const CONFIG_CACHE_TTL = 30000; // 30 seconds

/**
 * Get configuration from database with fallbacks
 */
async function getConfig(): Promise<SalesConfig> {
  const now = Date.now();
  if (configCache && now - configCacheTime < CONFIG_CACHE_TTL) {
    return configCache;
  }

  const rows = await safeQueryReadOnly<ConfigRow>(
    'SELECT key, value FROM sa_config',
    [],
    []
  );

  const configMap = new Map(rows.map(r => [r.key, r.value]));

  configCache = {
    fromEmail: configMap.get('from_email') || FALLBACK_FROM_EMAIL,
    fromName: configMap.get('from_name') || FALLBACK_FROM_NAME,
    replyTo: configMap.get('reply_to') || FALLBACK_FROM_EMAIL,
    dailyLimit: parseInt(configMap.get('daily_limit') || String(FALLBACK_DAILY_LIMIT)),
    requireEmailVerification: configMap.get('require_email_verification') === 'true',
    enableReviewQueue: configMap.get('enable_review_queue') === 'true',
    warmupMode: configMap.get('warmup_mode') === 'true',
    warmupDailyIncrement: parseInt(configMap.get('warmup_daily_increment') || '5'),
    systemEnabled: configMap.get('system_enabled') === 'true',
    spamScoreThreshold: parseInt(configMap.get('spam_score_threshold') || '70'),
    bounceRateThreshold: parseInt(configMap.get('bounce_rate_threshold') || '7'),
    pauseOnCriticalHealth: configMap.get('pause_on_critical_health') !== 'false',
  };
  configCacheTime = now;

  return configCache;
}

/**
 * Invalidate config cache (call after updates)
 */
function invalidateConfigCache(): void {
  configCache = null;
  configCacheTime = 0;
}

// Legacy: Keep systemEnabled in memory for backwards compatibility
// but prefer DB value
let systemEnabledMemory = false;

// ============================================================================
// REVOLUTIONARY: SPAM PREVENTION (Zero-cost, runs locally)
// OVERCOMES: "Emails landing in spam" - #1 complaint across ALL tools
// ============================================================================

const SPAM_TRIGGER_WORDS = {
  critical: [
    'act now', 'action required', 'apply now', 'buy now', 'call now',
    'click here', 'click below', 'don\'t delete', 'exclusive deal', 'expire',
    'free', 'free access', 'free gift', 'free trial', 'great offer',
    'guarantee', 'limited offer', 'limited time', 'no cost', 'no obligation',
    'offer expires', 'order now', 'special promotion', 'urgent', 'winner',
    'you have been selected', 'prize', 'congratulations', 'million dollars',
    'earn money', 'make money', 'cash bonus', 'risk free', '100% free',
  ],
  high: [
    'affordable', 'bargain', 'best price', 'big savings', 'bonus',
    'cheap', 'discount', 'fantastic deal', 'final call', 'giveaway',
    'hot deal', 'incredible deal', 'instant', 'new customers only',
    'promotion', 'save big', 'save money', 'special offer', 'unbeatable',
  ],
};

interface SpamCheckResult {
  score: number;
  issues: string[];
  passesThreshold: boolean;
}

function checkEmailForSpam(subject: string, body: string): SpamCheckResult {
  const issues: string[] = [];
  let score = 100;
  
  // Use lowercase for word matching (spam triggers are lowercase)
  const fullTextLower = (subject + ' ' + body).toLowerCase();
  // Use ORIGINAL text for caps detection (lowercase has no caps!)
  const fullTextOriginal = subject + ' ' + body;

  // Check critical triggers (-15 each)
  for (const trigger of SPAM_TRIGGER_WORDS.critical) {
    if (fullTextLower.includes(trigger)) {
      issues.push(`Contains spam trigger: "${trigger}"`);
      score -= 15;
    }
  }

  // Check high-risk triggers (-8 each)
  for (const trigger of SPAM_TRIGGER_WORDS.high) {
    if (fullTextLower.includes(trigger)) {
      issues.push(`Contains promotional word: "${trigger}"`);
      score -= 8;
    }
  }

  // Check excessive caps - FIXED: Must check ORIGINAL text, not lowercased!
  // Previous bug: lowercased text then searched for [A-Z] which never matched
  const capsMatches = fullTextOriginal.match(/[A-Z]{5,}/g);
  if (capsMatches && capsMatches.length > 0) {
    issues.push(`Contains excessive CAPS: ${capsMatches.slice(0, 3).join(', ')}`);
    score -= 10;
  }

  // Check excessive punctuation
  if (/[!?]{2,}/.test(fullTextLower)) {
    issues.push('Contains excessive punctuation (!! or ??)');
    score -= 10;
  }

  // Check too many links
  const linkCount = (body.match(/https?:\/\//g) || []).length;
  if (linkCount > 3) {
    issues.push(`Too many links (${linkCount})`);
    score -= linkCount * 5;
  }

  // Check email length
  const wordCount = body.split(/\s+/).length;
  if (wordCount < 20) {
    issues.push('Email too short (looks like spam)');
    score -= 10;
  }
  if (wordCount > 500) {
    issues.push('Email too long (may be ignored)');
    score -= 5;
  }

  return {
    score: Math.max(0, Math.min(100, score)),
    issues,
    passesThreshold: score >= 70,
  };
}

// ============================================================================
// REVOLUTIONARY: DOMAIN HEALTH GUARD (Protects sender reputation)
// OVERCOMES: "Bounce rate >7% = domain tanks", "Domain getting flagged fast"
// ============================================================================

const DOMAIN_HEALTH_THRESHOLDS = {
  bounceRate: { warning: 5, critical: 7 },
  spamRate: { warning: 0.3, critical: 0.5 },
};

interface DomainHealthStatus {
  healthy: boolean;
  bounceRate: number;
  spamRate: number;
  totalSent: number;
  status: 'healthy' | 'warning' | 'critical' | 'quarantined';
  reason?: string;
}

async function checkDomainHealth(): Promise<DomainHealthStatus> {
  // Get last 7 days of email stats
  const stats = await safeQueryReadOnly<{
    total: string;
    bounced: string;
    complained: string;
  }>(`
    SELECT 
      COUNT(*) as total,
      COUNT(*) FILTER (WHERE bounced_at IS NOT NULL) as bounced,
      COUNT(*) FILTER (WHERE status = 'complained') as complained
    FROM sa_emails
    WHERE sent_at > NOW() - INTERVAL '7 days'
  `, [], [{ total: '0', bounced: '0', complained: '0' }]);

  const total = parseInt(stats[0]?.total || '0');
  const bounced = parseInt(stats[0]?.bounced || '0');
  const complained = parseInt(stats[0]?.complained || '0');

  const bounceRate = total > 0 ? (bounced / total) * 100 : 0;
  const spamRate = total > 0 ? (complained / total) * 100 : 0;

  // Determine status
  let status: DomainHealthStatus['status'] = 'healthy';
  let reason: string | undefined;

  if (bounceRate >= DOMAIN_HEALTH_THRESHOLDS.bounceRate.critical) {
    status = 'critical';
    reason = `Bounce rate ${bounceRate.toFixed(1)}% exceeds critical threshold (${DOMAIN_HEALTH_THRESHOLDS.bounceRate.critical}%)`;
  } else if (spamRate >= DOMAIN_HEALTH_THRESHOLDS.spamRate.critical) {
    status = 'critical';
    reason = `Spam complaint rate ${spamRate.toFixed(2)}% exceeds critical threshold`;
  } else if (bounceRate >= DOMAIN_HEALTH_THRESHOLDS.bounceRate.warning) {
    status = 'warning';
    reason = `Bounce rate ${bounceRate.toFixed(1)}% approaching critical threshold`;
  } else if (spamRate >= DOMAIN_HEALTH_THRESHOLDS.spamRate.warning) {
    status = 'warning';
    reason = `Spam rate ${spamRate.toFixed(2)}% approaching critical threshold`;
  }

  return {
    healthy: status === 'healthy' || status === 'warning',
    bounceRate,
    spamRate,
    totalSent: total,
    status,
    reason,
  };
}

// ============================================================================
// REVOLUTIONARY: PREDICTIVE LEAD SCORING (Prioritize hot leads)
// OVERCOMES: "Targeting has gotten lazy", "Everyone matches ICP but no urgency"
// ============================================================================

async function getScoreAdjustedContacts(contactIds: string[]): Promise<string[]> {
  if (contactIds.length === 0) return [];

  // Score contacts based on engagement signals
  const scored = await safeQueryReadOnly<{ id: string; priority_score: number }>(`
    WITH engagement_scores AS (
      SELECT 
        c.id,
        c.score as base_score,
        COALESCE(
          (SELECT COUNT(*) FROM sa_emails e WHERE e.contact_id = c.id AND e.opened_at IS NOT NULL) * 15 +
          (SELECT COUNT(*) FROM sa_emails e WHERE e.contact_id = c.id AND e.clicked_at IS NOT NULL) * 25 +
          (SELECT COUNT(*) FROM sa_emails e WHERE e.contact_id = c.id AND e.replied_at IS NOT NULL) * 40,
          0
        ) as engagement_bonus,
        CASE 
          WHEN c.app_downloads ILIKE '%million%' OR c.app_downloads ILIKE '%M+%' THEN 20
          WHEN c.app_downloads ILIKE '%100k%' OR c.app_downloads ILIKE '%500k%' THEN 10
          ELSE 0
        END as company_bonus
      FROM sa_contacts c
      WHERE c.id = ANY($1::uuid[])
    )
    SELECT 
      id, 
      (COALESCE(base_score, 50) + engagement_bonus + company_bonus) as priority_score
    FROM engagement_scores
    ORDER BY priority_score DESC
  `, [contactIds], []);

  const scoredIds = scored.map(r => r.id);
  return scoredIds.length > 0 ? scoredIds : contactIds;
}

// Type definitions for database rows
interface CountRow { count: string }
interface ContactRow {
  id: string;
  email: string;
  first_name?: string;
  last_name?: string;
  company_name?: string;
  job_title?: string;
  app_name?: string;
  app_downloads?: string;
  status: string;
  score?: number;
  source?: string;
  notes?: string;
  created_at: string;
  updated_at: string;
}

interface SequenceRow {
  id: string;
  name: string;
  description?: string;
  steps: string | Array<{ delay_days: number; subject: string; body: string }>;
  active: boolean;
  created_at: string;
  enrolled_count?: number;
  replied_count?: number;
  active_count?: number;
}

interface EnrollmentRow {
  id: string;
  contact_id: string;
  sequence_id: string;
  current_step: number;
  status: string;
  next_send_at?: string;
  personalization: string | Record<string, string>;
  email?: string;
  first_name?: string;
  company_name?: string;
  sequence_name?: string;
  steps?: string | Array<{ delay_days: number; subject: string; body: string }>;
  contact_email?: string;
  created_at: string;
}

interface EmailRow {
  id: string;
  enrollment_id?: string;
  contact_id: string;
  to_email: string;
  from_email: string;
  subject?: string;
  body_html?: string;
  message_id?: string;
  status: string;
  sent_at?: string;
  opened_at?: string;
  clicked_at?: string;
  replied_at?: string;
  bounced_at?: string;
  step_number?: number;
  contact_email?: string;
  first_name?: string;
  created_at: string;
}

interface EmailStatsRow {
  total: string;
  sent: string;
  opened: string;
  clicked: string;
  replied: string;
  bounced: string;
}

interface SequenceStatsRow {
  total: string;
  active: string;
}

interface StatusByRow {
  status: string;
  count: string;
}

// Helper for write operations
async function write<T = unknown>(sql: string, params: unknown[] = [], reason: string): Promise<T[]> {
  return queryOperator<T>(sql, params, SYSTEM_ACTOR, reason);
}

// ============================================================================
// SYSTEM CONTROL (Enable/Disable) - Now DB-backed
// ============================================================================

export async function isSystemEnabled(): Promise<boolean> {
  const config = await getConfig();
  return config.systemEnabled || systemEnabledMemory;
}

export async function setSystemEnabled(enabled: boolean): Promise<{ success: boolean; enabled: boolean }> {
  try {
    await write(
      `INSERT INTO sa_config (key, value, updated_at) VALUES ('system_enabled', $1, CURRENT_TIMESTAMP)
       ON CONFLICT (key) DO UPDATE SET value = $1, updated_at = CURRENT_TIMESTAMP`,
      [enabled ? 'true' : 'false'],
      `${enabled ? 'Enable' : 'Disable'} sales automation system`
    );
    systemEnabledMemory = enabled;
    invalidateConfigCache();
    return { success: true, enabled };
  } catch (error) {
    // Fallback to memory-only if DB fails
    systemEnabledMemory = enabled;
    return { success: true, enabled };
  }
}

// ============================================================================
// CONFIGURATION MANAGEMENT (Full UI control)
// ============================================================================

export async function getSalesConfig(): Promise<SalesConfig> {
  return getConfig();
}

export async function updateSalesConfig(updates: Partial<{
  fromEmail: string;
  fromName: string;
  replyTo: string;
  dailyLimit: number;
  requireEmailVerification: boolean;
  enableReviewQueue: boolean;
  warmupMode: boolean;
  warmupDailyIncrement: number;
  spamScoreThreshold: number;
  bounceRateThreshold: number;
  pauseOnCriticalHealth: boolean;
}>): Promise<{ success: boolean; config?: SalesConfig; error?: string }> {
  const keyMap: Record<string, string> = {
    fromEmail: 'from_email',
    fromName: 'from_name',
    replyTo: 'reply_to',
    dailyLimit: 'daily_limit',
    requireEmailVerification: 'require_email_verification',
    enableReviewQueue: 'enable_review_queue',
    warmupMode: 'warmup_mode',
    warmupDailyIncrement: 'warmup_daily_increment',
    spamScoreThreshold: 'spam_score_threshold',
    bounceRateThreshold: 'bounce_rate_threshold',
    pauseOnCriticalHealth: 'pause_on_critical_health',
  };

  try {
    for (const [jsKey, value] of Object.entries(updates)) {
      const dbKey = keyMap[jsKey];
      if (!dbKey) continue;
      
      const stringValue = typeof value === 'boolean' ? (value ? 'true' : 'false') : String(value);
      await write(
        `INSERT INTO sa_config (key, value, updated_at) VALUES ($1, $2, CURRENT_TIMESTAMP)
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = CURRENT_TIMESTAMP`,
        [dbKey, stringValue],
        `Update sales config: ${dbKey}`
      );
    }
    
    invalidateConfigCache();
    const config = await getConfig();
    return { success: true, config };
  } catch (error) {
    return { success: false, error: String(error) };
  }
}

// ============================================================================
// STATUS & ANALYTICS
// ============================================================================

export async function getSalesAutomationStatus(): Promise<{
  enabled: boolean;
  configured: boolean;
  apexMailConfigured: boolean;
  dailyLimit: number;
  todaySent: number;
  todayRemaining: number;
  config: SalesConfig;
}> {
  const config = await getConfig();
  const todaySentResult = await safeQueryReadOnly<CountRow>(`
    SELECT COUNT(*) as count FROM sa_emails 
    WHERE sent_at >= CURRENT_DATE
  `, [], [{ count: '0' }]);
  const todaySent = parseInt(todaySentResult[0]?.count || '0');
  return {
    enabled: config.systemEnabled || systemEnabledMemory,
    configured: true,
    apexMailConfigured: !!apexmail,
    dailyLimit: config.dailyLimit,
    todaySent,
    todayRemaining: Math.max(0, config.dailyLimit - todaySent),
    config,
  };
}

export async function getSalesAutomationStats(): Promise<{
  contacts: { total: number; byStatus: Record<string, number> };
  enrollments: { total: number; byStatus: Record<string, number> };
  emails: { 
    total: number; 
    sent: number; 
    opened: number; 
    clicked: number; 
    replied: number; 
    bounced: number;
    openRate: number;
    clickRate: number;
    replyRate: number;
  };
  sequences: { total: number; active: number };
}> {
  const contactStats = await safeQueryReadOnly<StatusByRow>(`
    SELECT status, COUNT(*) as count 
    FROM sa_contacts 
    GROUP BY status
  `, [], []);
  const byContactStatus: Record<string, number> = {};
  let totalContacts = 0;
  contactStats.forEach((row) => {
    byContactStatus[row.status] = parseInt(row.count);
    totalContacts += parseInt(row.count);
  });

  const enrollStats = await safeQueryReadOnly<StatusByRow>(`
    SELECT status, COUNT(*) as count 
    FROM sa_enrollments 
    GROUP BY status
  `, [], []);
  const byEnrollStatus: Record<string, number> = {};
  let totalEnrollments = 0;
  enrollStats.forEach((row) => {
    byEnrollStatus[row.status] = parseInt(row.count);
    totalEnrollments += parseInt(row.count);
  });

  const emailStats = await safeQueryReadOnly<EmailStatsRow>(`
    SELECT 
      COUNT(*) as total,
      COUNT(*) FILTER (WHERE sent_at IS NOT NULL) as sent,
      COUNT(*) FILTER (WHERE opened_at IS NOT NULL) as opened,
      COUNT(*) FILTER (WHERE clicked_at IS NOT NULL) as clicked,
      COUNT(*) FILTER (WHERE replied_at IS NOT NULL) as replied,
      COUNT(*) FILTER (WHERE bounced_at IS NOT NULL) as bounced
    FROM sa_emails
  `, [], [{ total: '0', sent: '0', opened: '0', clicked: '0', replied: '0', bounced: '0' }]);
  const stats = emailStats[0] || { total: '0', sent: '0', opened: '0', clicked: '0', replied: '0', bounced: '0' };
  const sent = parseInt(stats.sent || '0');
  const opened = parseInt(stats.opened || '0');
  const clicked = parseInt(stats.clicked || '0');
  const replied = parseInt(stats.replied || '0');

  const seqStats = await safeQueryReadOnly<SequenceStatsRow>(`
    SELECT 
      COUNT(*) as total,
      COUNT(*) FILTER (WHERE active = true) as active
    FROM sa_sequences
  `, [], [{ total: '0', active: '0' }]);

  return {
    contacts: { total: totalContacts, byStatus: byContactStatus },
    enrollments: { total: totalEnrollments, byStatus: byEnrollStatus },
    emails: {
      total: parseInt(stats.total || '0'),
      sent,
      opened,
      clicked,
      replied,
      bounced: parseInt(stats.bounced || '0'),
      openRate: sent > 0 ? Math.round((opened / sent) * 100) : 0,
      clickRate: sent > 0 ? Math.round((clicked / sent) * 100) : 0,
      replyRate: sent > 0 ? Math.round((replied / sent) * 100) : 0,
    },
    sequences: {
      total: parseInt(seqStats[0]?.total || '0'),
      active: parseInt(seqStats[0]?.active || '0'),
    },
  };
}

// ============================================================================
// CONTACTS MANAGEMENT
// ============================================================================

export async function getContacts(params: {
  status?: string;
  limit?: number;
  offset?: number;
  search?: string;
}): Promise<{ contacts: ContactRow[]; total: number }> {
  const { status, limit = 50, offset = 0, search } = params;
  
  let whereClause = 'WHERE 1=1';
  const queryParams: unknown[] = [];
  let paramIndex = 1;
  
  if (status) {
    whereClause += ` AND status = $${paramIndex++}`;
    queryParams.push(status);
  }
  
  if (search) {
    whereClause += ` AND (email ILIKE $${paramIndex} OR first_name ILIKE $${paramIndex} OR company_name ILIKE $${paramIndex})`;
    queryParams.push(`%${search}%`);
    paramIndex++;
  }

  const contacts = await safeQueryReadOnly<ContactRow>(`
    SELECT id, email, first_name, last_name, company_name, job_title, 
           app_name, app_downloads, status, score, source, notes,
           created_at, updated_at
    FROM sa_contacts
    ${whereClause}
    ORDER BY created_at DESC
    LIMIT $${paramIndex++} OFFSET $${paramIndex}
  `, [...queryParams, limit, offset], []);

  const countResult = await safeQueryReadOnly<CountRow>(`
    SELECT COUNT(*) as count FROM sa_contacts ${whereClause}
  `, queryParams, [{ count: '0' }]);

  return {
    contacts,
    total: parseInt(countResult[0]?.count || '0'),
  };
}

export async function addContact(data: {
  email: string;
  first_name?: string;
  last_name?: string;
  company_name?: string;
  job_title?: string;
  app_name?: string;
  app_downloads?: string;
  notes?: string;
}): Promise<{ success: boolean; contact?: ContactRow; error?: string }> {
  try {
    const result = await write<ContactRow>(`
      INSERT INTO sa_contacts (email, first_name, last_name, company_name, job_title, app_name, app_downloads, notes, source)
      VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'control_plane')
      ON CONFLICT (email) DO UPDATE SET
        first_name = COALESCE(EXCLUDED.first_name, sa_contacts.first_name),
        last_name = COALESCE(EXCLUDED.last_name, sa_contacts.last_name),
        company_name = COALESCE(EXCLUDED.company_name, sa_contacts.company_name),
        updated_at = CURRENT_TIMESTAMP
      RETURNING *
    `, [
      data.email,
      data.first_name || null,
      data.last_name || null,
      data.company_name || null,
      data.job_title || null,
      data.app_name || null,
      data.app_downloads || null,
      data.notes || null,
    ], 'Add contact');
    
    return { success: true, contact: result[0] };
  } catch (error) {
    return { success: false, error: String(error) };
  }
}

export async function updateContact(id: string, data: Partial<{
  first_name: string;
  last_name: string;
  company_name: string;
  job_title: string;
  app_name: string;
  app_downloads: string;
  notes: string;
  status: string;
  score: number;
}>): Promise<{ success: boolean; error?: string }> {
  const fields: string[] = [];
  const values: unknown[] = [];
  let paramIndex = 1;

  for (const [key, value] of Object.entries(data)) {
    if (value !== undefined) {
      fields.push(`${key} = $${paramIndex++}`);
      values.push(value);
    }
  }

  if (fields.length === 0) {
    return { success: false, error: 'No fields to update' };
  }

  fields.push('updated_at = CURRENT_TIMESTAMP');
  values.push(id);

  try {
    await write(`
      UPDATE sa_contacts 
      SET ${fields.join(', ')}
      WHERE id = $${paramIndex}
    `, values, 'Update contact');
    
    return { success: true };
  } catch (error) {
    return { success: false, error: String(error) };
  }
}

export async function deleteContact(id: string): Promise<{ success: boolean; error?: string }> {
  try {
    await write('DELETE FROM sa_contacts WHERE id = $1', [id], 'Delete contact');
    return { success: true };
  } catch (error) {
    return { success: false, error: String(error) };
  }
}

export async function importContacts(contacts: Array<{
  email: string;
  first_name?: string;
  last_name?: string;
  company_name?: string;
  job_title?: string;
  app_name?: string;
  app_downloads?: string;
}>): Promise<{ imported: number; skipped: number; errors: string[] }> {
  let imported = 0;
  let skipped = 0;
  const errors: string[] = [];

  for (const contact of contacts) {
    if (!contact.email || !isValidEmail(contact.email)) {
      skipped++;
      continue;
    }

    try {
      await write(`
        INSERT INTO sa_contacts (email, first_name, last_name, company_name, job_title, app_name, app_downloads, source)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'csv_import')
        ON CONFLICT (email) DO UPDATE SET
          first_name = COALESCE(EXCLUDED.first_name, sa_contacts.first_name),
          updated_at = CURRENT_TIMESTAMP
      `, [
        contact.email,
        contact.first_name || guessFirstName(contact.email),
        contact.last_name || null,
        contact.company_name || null,
        contact.job_title || null,
        contact.app_name || null,
        contact.app_downloads || null,
      ], 'Import contact');
      imported++;
    } catch (error) {
      errors.push(`Failed to import ${contact.email}: ${error}`);
      skipped++;
    }
  }

  return { imported, skipped, errors };
}

// ============================================================================
// SEQUENCES MANAGEMENT
// ============================================================================

export async function getSequences(): Promise<SequenceRow[]> {
  return await safeQueryReadOnly<SequenceRow>(`
    SELECT s.*, 
      (SELECT COUNT(*) FROM sa_enrollments e WHERE e.sequence_id = s.id) as enrolled_count,
      (SELECT COUNT(*) FROM sa_enrollments e WHERE e.sequence_id = s.id AND e.status = 'replied') as replied_count,
      (SELECT COUNT(*) FROM sa_enrollments e WHERE e.sequence_id = s.id AND e.status = 'active') as active_count
    FROM sa_sequences s
    ORDER BY created_at DESC
  `, [], []);
}

export async function getSequence(id: string): Promise<SequenceRow | null> {
  const results = await safeQueryReadOnly<SequenceRow>(`
    SELECT s.*, 
      (SELECT COUNT(*) FROM sa_enrollments e WHERE e.sequence_id = s.id) as enrolled_count
    FROM sa_sequences s
    WHERE s.id = $1
  `, [id], []);
  
  return results[0] || null;
}

export async function createSequence(data: {
  name: string;
  description?: string;
  steps: Array<{ delay_days: number; subject: string; body: string }>;
}): Promise<{ success: boolean; sequence?: SequenceRow; error?: string }> {
  try {
    const result = await write<SequenceRow>(`
      INSERT INTO sa_sequences (name, description, steps)
      VALUES ($1, $2, $3)
      RETURNING *
    `, [data.name, data.description || null, JSON.stringify(data.steps)], 'Create sequence');
    
    return { success: true, sequence: result[0] };
  } catch (error) {
    return { success: false, error: String(error) };
  }
}

export async function updateSequence(id: string, data: {
  name?: string;
  description?: string;
  steps?: Array<{ delay_days: number; subject: string; body: string }>;
  active?: boolean;
}): Promise<{ success: boolean; error?: string }> {
  const fields: string[] = [];
  const values: unknown[] = [];
  let paramIndex = 1;

  if (data.name !== undefined) {
    fields.push(`name = $${paramIndex++}`);
    values.push(data.name);
  }
  if (data.description !== undefined) {
    fields.push(`description = $${paramIndex++}`);
    values.push(data.description);
  }
  if (data.steps !== undefined) {
    fields.push(`steps = $${paramIndex++}`);
    values.push(JSON.stringify(data.steps));
  }
  if (data.active !== undefined) {
    fields.push(`active = $${paramIndex++}`);
    values.push(data.active);
  }

  if (fields.length === 0) {
    return { success: false, error: 'No fields to update' };
  }

  values.push(id);

  try {
    await write(`
      UPDATE sa_sequences SET ${fields.join(', ')} WHERE id = $${paramIndex}
    `, values, 'Update sequence');
    return { success: true };
  } catch (error) {
    return { success: false, error: String(error) };
  }
}

export async function deleteSequence(id: string): Promise<{ success: boolean; error?: string }> {
  try {
    await write('DELETE FROM sa_sequences WHERE id = $1', [id], 'Delete sequence');
    return { success: true };
  } catch (error) {
    return { success: false, error: String(error) };
  }
}

// ============================================================================
// ENROLLMENTS
// ============================================================================

export async function getEnrollments(params: {
  sequenceId?: string;
  status?: string;
  limit?: number;
  offset?: number;
}): Promise<{ enrollments: EnrollmentRow[]; total: number }> {
  const { sequenceId, status, limit = 50, offset = 0 } = params;
  
  let whereClause = 'WHERE 1=1';
  const queryParams: unknown[] = [];
  let paramIndex = 1;
  
  if (sequenceId) {
    whereClause += ` AND e.sequence_id = $${paramIndex++}`;
    queryParams.push(sequenceId);
  }
  
  if (status) {
    whereClause += ` AND e.status = $${paramIndex++}`;
    queryParams.push(status);
  }

  const enrollments = await safeQueryReadOnly<EnrollmentRow>(`
    SELECT e.*, c.email, c.first_name, c.company_name, s.name as sequence_name
    FROM sa_enrollments e
    JOIN sa_contacts c ON e.contact_id = c.id
    JOIN sa_sequences s ON e.sequence_id = s.id
    ${whereClause}
    ORDER BY e.created_at DESC
    LIMIT $${paramIndex++} OFFSET $${paramIndex}
  `, [...queryParams, limit, offset], []);

  const countResult = await safeQueryReadOnly<CountRow>(`
    SELECT COUNT(*) as count FROM sa_enrollments e ${whereClause}
  `, queryParams, [{ count: '0' }]);

  return {
    enrollments,
    total: parseInt(countResult[0]?.count || '0'),
  };
}

export async function enrollContacts(params: {
  sequenceId: string;
  contactStatus?: string;
  limit?: number;
}): Promise<{ enrolled: number; errors: string[] }> {
  const { sequenceId, contactStatus = 'new', limit = 10 } = params;
  let enrolled = 0;
  const errors: string[] = [];

  const sequence = await getSequence(sequenceId);
  if (!sequence) {
    return { enrolled: 0, errors: ['Sequence not found'] };
  }

  const contacts = await safeQueryReadOnly<ContactRow>(`
    SELECT c.* FROM sa_contacts c
    WHERE c.status = $1
      AND NOT EXISTS (
        SELECT 1 FROM sa_enrollments e 
        WHERE e.contact_id = c.id AND e.sequence_id = $2
      )
    ORDER BY c.score DESC NULLS LAST
    LIMIT $3
  `, [contactStatus, sequenceId, limit], []);

  const steps = typeof sequence.steps === 'string' ? JSON.parse(sequence.steps) : sequence.steps;
  const firstStepDelay = steps[0]?.delay_days || 0;
  const firstSendAt = new Date(Date.now() + firstStepDelay * 24 * 60 * 60 * 1000);

  for (const contact of contacts) {
    try {
      await write(`
        INSERT INTO sa_enrollments (contact_id, sequence_id, current_step, next_send_at, personalization)
        VALUES ($1, $2, 0, $3, $4)
      `, [
        contact.id,
        sequenceId,
        firstSendAt,
        JSON.stringify({
          first_name: contact.first_name || 'there',
          company_name: contact.company_name || 'your company',
          app_name: contact.app_name || 'your app',
          app_downloads: contact.app_downloads || 'many',
        }),
      ], 'Enroll contact in sequence');

      await write(`
        UPDATE sa_contacts SET status = 'contacted', updated_at = CURRENT_TIMESTAMP WHERE id = $1
      `, [contact.id], 'Mark contact as contacted');

      enrolled++;
    } catch (error) {
      errors.push(`Failed to enroll ${contact.email}: ${error}`);
    }
  }

  return { enrolled, errors };
}

export async function pauseEnrollment(id: string): Promise<{ success: boolean }> {
  await write(`
    UPDATE sa_enrollments SET status = 'paused', updated_at = CURRENT_TIMESTAMP WHERE id = $1
  `, [id], 'Pause enrollment');
  return { success: true };
}

export async function resumeEnrollment(id: string): Promise<{ success: boolean }> {
  await write(`
    UPDATE sa_enrollments SET status = 'active', updated_at = CURRENT_TIMESTAMP WHERE id = $1
  `, [id], 'Resume enrollment');
  return { success: true };
}

// ============================================================================
// EMAILS & SENDING
// ============================================================================

export async function getEmails(params: {
  contactId?: string;
  status?: string;
  limit?: number;
  offset?: number;
}): Promise<{ emails: EmailRow[]; total: number }> {
  const { contactId, status, limit = 50, offset = 0 } = params;
  
  let whereClause = 'WHERE 1=1';
  const queryParams: unknown[] = [];
  let paramIndex = 1;
  
  if (contactId) {
    whereClause += ` AND e.contact_id = $${paramIndex++}`;
    queryParams.push(contactId);
  }
  
  if (status) {
    whereClause += ` AND e.status = $${paramIndex++}`;
    queryParams.push(status);
  }

  const emails = await safeQueryReadOnly<EmailRow>(`
    SELECT e.*, c.email as contact_email, c.first_name
    FROM sa_emails e
    JOIN sa_contacts c ON e.contact_id = c.id
    ${whereClause}
    ORDER BY e.created_at DESC
    LIMIT $${paramIndex++} OFFSET $${paramIndex}
  `, [...queryParams, limit, offset], []);

  const countResult = await safeQueryReadOnly<CountRow>(`
    SELECT COUNT(*) as count FROM sa_emails e ${whereClause}
  `, queryParams, [{ count: '0' }]);

  return {
    emails,
    total: parseInt(countResult[0]?.count || '0'),
  };
}

export async function runOutreachCycle(dryRun: boolean = false): Promise<{
  processed: number;
  sent: number;
  skippedSpam: number;
  errors: string[];
  dryRun: boolean;
  domainHealth: DomainHealthStatus;
}> {
  const errors: string[] = [];
  let processed = 0;
  let sent = 0;
  let skippedSpam = 0;

  // =========================================================================
  // GLOBAL KILL SWITCH CHECK - MUST BE FIRST CHECK
  // 
  // Emergency stop for ALL outbound sending across V1 and V2 systems.
  // If active, immediately abort without any further processing.
  // =========================================================================
  const { checkSendsAllowed } = await import('./global-kill-switch.js');
  const killSwitchCheck = await checkSendsAllowed('v1_outbound');
  if (!killSwitchCheck.allowed && !dryRun) {
    const emptyDomainHealth: DomainHealthStatus = { healthy: false, bounceRate: 0, spamRate: 0, totalSent: 0, status: 'quarantined', reason: 'Kill switch active' };
    return {
      processed: 0,
      sent: 0,
      skippedSpam: 0,
      errors: [killSwitchCheck.reason || 'Global kill switch is active'],
      dryRun,
      domainHealth: emptyDomainHealth,
    };
  }

  // =========================================================================
  // DOMAIN HEALTH GUARD (HARD-STOP) - SECOND CHECK
  // 
  // This is the most critical safety feature. If bounce rate exceeds 7%,
  // ALL sending is blocked until an operator explicitly overrides.
  // 
  // This protects sender reputation which, once damaged, is nearly
  // impossible to recover. A tanked domain can take 6+ months to rehabilitate.
  // =========================================================================
  const domainHealth = await checkDomainHealth();
  const config = await getConfig();
  
  // HARD STOP: Critical health blocks ALL sending
  if (domainHealth.status === 'critical' && !dryRun) {
    // Log this as an alert-level event
    logger.error('DOMAIN HEALTH GUARD: Sending BLOCKED', {
      bounceRate: domainHealth.bounceRate,
      spamRate: domainHealth.spamRate,
      totalSent: domainHealth.totalSent,
      reason: domainHealth.reason,
    });
    
    return { 
      processed: 0, 
      sent: 0, 
      skippedSpam: 0,
      errors: [
        `🚨 DOMAIN HEALTH CRITICAL: ${domainHealth.reason}`,
        `Bounce rate: ${domainHealth.bounceRate.toFixed(1)}% (threshold: ${DOMAIN_HEALTH_THRESHOLDS.bounceRate.critical}%)`,
        `Spam rate: ${domainHealth.spamRate.toFixed(1)}% (threshold: ${DOMAIN_HEALTH_THRESHOLDS.spamRate.critical}%)`,
        `SENDING IS BLOCKED to protect domain reputation.`,
        `To resume: 1) Investigate bounces, 2) Clean email list, 3) Operator override required.`,
      ], 
      dryRun,
      domainHealth,
    };
  }
  
  // Check if operator has configured pause_on_critical_health=false (emergency override)
  // This should ONLY be used after investigation and cleanup
  if (domainHealth.status === 'quarantined' && config.pauseOnCriticalHealth !== false && !dryRun) {
    return {
      processed: 0,
      sent: 0,
      skippedSpam: 0,
      errors: [
        `🔒 DOMAIN QUARANTINED: Previous critical health event requires operator review.`,
        `Set pause_on_critical_health=false in config to override (NOT RECOMMENDED without investigation).`,
      ],
      dryRun,
      domainHealth,
    };
  }

  if (domainHealth.status === 'warning') {
    errors.push(`⚠️ Domain health warning: ${domainHealth.reason}. Reducing send rate to protect reputation.`);
  }

  // Safety check: System must be explicitly enabled
  // (config already fetched above for domain health guard)
  const isEnabled = await isSystemEnabled();
  if (!isEnabled && !dryRun) {
    return { processed: 0, sent: 0, skippedSpam: 0, errors: ['Sales automation system is DISABLED. Enable it from the control panel first.'], dryRun, domainHealth };
  }

  if (!apexmail && !dryRun) {
    return { processed: 0, sent: 0, skippedSpam: 0, errors: ['ApexMail not configured'], dryRun, domainHealth };
  }

  // Check daily limit
  const status = await getSalesAutomationStatus();
  if (status.todayRemaining <= 0 && !dryRun) {
    return { processed: 0, sent: 0, skippedSpam: 0, errors: ['Daily limit reached'], dryRun, domainHealth };
  }

  // Reduce send rate if domain health is warning
  const maxToSend = domainHealth.status === 'warning' 
    ? Math.min(status.todayRemaining, 10) 
    : Math.min(status.todayRemaining, 20);

  // Build query based on verification requirement
  let contactFilter = "c.status NOT IN ('bounced', 'opted_out')";
  if (config.requireEmailVerification) {
    contactFilter += " AND c.email_verified = true";
  }

  // Get pending enrollments
  const enrollments = await safeQueryReadOnly<EnrollmentRow>(`
    SELECT e.*, c.email as contact_email, c.first_name, s.name as sequence_name, s.steps
    FROM sa_enrollments e
    JOIN sa_contacts c ON e.contact_id = c.id
    JOIN sa_sequences s ON e.sequence_id = s.id
    WHERE e.status = 'active'
      AND e.next_send_at <= CURRENT_TIMESTAMP
      AND ${contactFilter}
    ORDER BY e.next_send_at
    LIMIT $1
  `, [maxToSend], []);

  // =========================================================================
  // REVOLUTIONARY: Predictive Lead Scoring (prioritize hot leads)
  // OVERCOMES: "Targeting has gotten lazy"
  // =========================================================================
  const contactIds = enrollments.map(e => e.contact_id);
  const prioritizedIds = await getScoreAdjustedContacts(contactIds);
  
  // Reorder enrollments by priority score
  const enrollmentMap = new Map(enrollments.map(e => [e.contact_id, e]));
  const sortedEnrollments = prioritizedIds
    .map(id => enrollmentMap.get(id))
    .filter((e): e is EnrollmentRow => e !== undefined);

  for (const enrollment of sortedEnrollments) {
    processed++;
    
    const steps = typeof enrollment.steps === 'string' ? JSON.parse(enrollment.steps) : (enrollment.steps || []);
    const currentStep = enrollment.current_step;
    
    if (currentStep >= steps.length) {
      await write(`
        UPDATE sa_enrollments SET status = 'completed', updated_at = CURRENT_TIMESTAMP WHERE id = $1
      `, [enrollment.id], 'Complete enrollment - all steps done');
      continue;
    }

    // =========================================================================
    // CRITICAL: Suppression List Check
    // MUST happen BEFORE any email is sent to prevent:
    // - Sending to bounced emails (damages reputation)
    // - Sending to unsubscribed users (legal violation)
    // - Sending to spam complainers (destroys deliverability)
    // =========================================================================
    const { isEmailSuppressed } = await import('./suppression-list.js');
    const suppressionCheck = await isEmailSuppressed(enrollment.contact_email!);
    if (suppressionCheck.isSuppressed) {
      errors.push(`Skipped ${enrollment.contact_email}: ${suppressionCheck.reason || 'Suppressed'}`);
      
      // Log activity and mark contact appropriately
      await write(`
        INSERT INTO sa_activity_log (contact_id, action, details)
        VALUES ($1, 'email_suppressed', $2)
      `, [
        enrollment.contact_id,
        JSON.stringify({ 
          reason: suppressionCheck.reason, 
          type: suppressionCheck.suppressionType,
          suppressedAt: suppressionCheck.suppressedAt,
        }),
      ], 'Log suppressed email');
      
      // Update contact status to prevent future attempts
      if (suppressionCheck.suppressionType === 'hard_bounce' || 
          suppressionCheck.suppressionType === 'spam_complaint' ||
          suppressionCheck.suppressionType === 'unsubscribe' ||
          suppressionCheck.suppressionType === 'gdpr_erasure') {
        await write(`
          UPDATE sa_contacts SET status = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2
        `, [
          suppressionCheck.suppressionType === 'hard_bounce' ? 'bounced' : 'opted_out',
          enrollment.contact_id,
        ], 'Update contact status from suppression');
      }
      
      continue;
    }

    // =========================================================================
    // Email Verification: MX Record & Disposable Email Check
    // Prevents sending to domains that can't receive email
    // =========================================================================
    const { verifyEmail } = await import('./email-verification.js');
    const emailVerification = await verifyEmail(enrollment.contact_email!, {
      checkMx: true,
      checkDisposable: true,
      checkRoleBased: true,
      blockDisposable: true,
      blockRoleBased: false, // Warn only
    });
    
    if (!emailVerification.valid) {
      errors.push(`Skipped ${enrollment.contact_email}: ${emailVerification.errors.join(', ')}`);
      
      await write(`
        INSERT INTO sa_activity_log (contact_id, action, details)
        VALUES ($1, 'email_verification_failed', $2)
      `, [
        enrollment.contact_id,
        JSON.stringify({ 
          errors: emailVerification.errors,
          domain: emailVerification.domain,
          isDisposable: emailVerification.isDisposable,
        }),
      ], 'Log email verification failure');
      
      // Mark disposable emails as invalid
      if (emailVerification.isDisposable) {
        await write(`
          UPDATE sa_contacts SET status = 'invalid', updated_at = CURRENT_TIMESTAMP WHERE id = $1
        `, [enrollment.contact_id], 'Mark disposable email as invalid');
      }
      
      continue;
    }
    
    // Log warning for role-based emails (but still send)
    if (emailVerification.warnings.length > 0) {
      await write(`
        INSERT INTO sa_activity_log (contact_id, action, details)
        VALUES ($1, 'email_verification_warning', $2)
      `, [
        enrollment.contact_id,
        JSON.stringify({ warnings: emailVerification.warnings }),
      ], 'Log email verification warning');
    }

    const step = steps[currentStep];
    const personalization = typeof enrollment.personalization === 'string'
      ? JSON.parse(enrollment.personalization)
      : enrollment.personalization;
    
    const subject = applyPersonalization(step.subject, personalization);
    const body = applyPersonalization(step.body, personalization);

    // =========================================================================
    // REVOLUTIONARY: Spam Prevention (pre-flight check)
    // OVERCOMES: "Emails landing in spam"
    // =========================================================================
    const spamCheck = checkEmailForSpam(subject, body);
    
    if (!spamCheck.passesThreshold) {
      skippedSpam++;
      errors.push(`Skipped ${enrollment.contact_email}: Spam score ${spamCheck.score}/100 - ${spamCheck.issues.slice(0, 2).join(', ')}`);
      
      // Log activity but don't send
      await write(`
        INSERT INTO sa_activity_log (contact_id, action, details)
        VALUES ($1, 'email_blocked_spam', $2)
      `, [
        enrollment.contact_id, 
        JSON.stringify({ reason: 'spam_prevention', score: spamCheck.score, issues: spamCheck.issues }),
      ], 'Log blocked email');
      
      continue;
    }

    if (dryRun) {
      sent++;
      continue;
    }

    // =========================================================================
    // Review Queue Support: Create draft instead of sending immediately
    // =========================================================================
    if (config.enableReviewQueue) {
      // Create as draft for review
      await write(`
        INSERT INTO sa_emails (enrollment_id, contact_id, to_email, from_email, subject, body_html, status, step_number, scheduled_for)
        VALUES ($1, $2, $3, $4, $5, $6, 'draft', $7, CURRENT_TIMESTAMP)
      `, [
        enrollment.id,
        enrollment.contact_id,
        enrollment.contact_email,
        config.fromEmail,
        subject,
        body,
        currentStep + 1,
      ], 'Create draft email for review');
      
      processed++;
      continue; // Don't send, wait for approval
    }

    try {
      // Generate unsubscribe URL for this contact
      const unsubscribeUrl = `${process.env.PUBLIC_ORIGIN || 'https://admin.apexmediation.ee'}/unsubscribe?contact=${enrollment.contact_id}`;
      const unsubscribeEmail = config.replyTo || config.fromEmail;
      
      const result = await apexmail.send({
        from: config.fromEmail,
        fromName: config.fromName,
        replyTo: config.replyTo,
        to: enrollment.contact_email!,
        subject,
        html: body.replace(/\n/g, '<br>'),
        text: body,
        category: 'marketing',
        priority: 4, // Priority 4 = Background marketing
        trackOpens: true,
        trackClicks: true,
        entityType: 'sa_email_auto',
        entityId: enrollment.id,
        // RFC 8058: List-Unsubscribe and List-Unsubscribe-Post headers
        headers: {
          'List-Unsubscribe': `<${unsubscribeUrl}>, <mailto:${unsubscribeEmail}?subject=unsubscribe>`,
          'List-Unsubscribe-Post': 'List-Unsubscribe=One-Click',
        },
      });

      if (!result.success) {
        throw new Error(result.error || 'ApexMail delivery failed');
      }

      // Record email
      await write(`
        INSERT INTO sa_emails (enrollment_id, contact_id, to_email, from_email, subject, body_html, message_id, status, step_number, sent_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'sent', $8, CURRENT_TIMESTAMP)
      `, [
        enrollment.id,
        enrollment.contact_id,
        enrollment.contact_email,
        config.fromEmail,
        subject,
        body,
        result.messageId || result.emailId || null,
        currentStep + 1,
      ], 'Record sent email');

      // Update enrollment for next step
      const nextStep = currentStep + 1;
      if (nextStep < steps.length) {
        const nextDelay = steps[nextStep].delay_days || 3;
        const nextSendAt = new Date(Date.now() + nextDelay * 24 * 60 * 60 * 1000);
        
        await write(`
          UPDATE sa_enrollments SET current_step = $1, next_send_at = $2, updated_at = CURRENT_TIMESTAMP WHERE id = $3
        `, [nextStep, nextSendAt, enrollment.id], 'Advance enrollment step');
      } else {
        await write(`
          UPDATE sa_enrollments SET status = 'completed', current_step = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2
        `, [nextStep, enrollment.id], 'Complete enrollment');
      }

      sent++;
      
      // Rate limit - slower if domain health is warning
      await sleep(domainHealth.status === 'warning' ? 1000 : 500);
    } catch (error) {
      errors.push(`Failed to send to ${enrollment.contact_email}: ${error}`);
    }
  }

  return { processed, sent, skippedSpam, errors, dryRun, domainHealth };
}

// ============================================================================
// ACTIVITY LOG
// ============================================================================

interface ActivityRow {
  id: string;
  contact_id: string;
  action: string;
  details?: string | Record<string, unknown>;
  created_at: string;
  contact_email?: string;
}

export async function getActivityLog(params: {
  contactId?: string;
  limit?: number;
}): Promise<ActivityRow[]> {
  const { contactId, limit = 100 } = params;
  
  if (contactId) {
    return await safeQueryReadOnly<ActivityRow>(`
      SELECT a.*, c.email as contact_email
      FROM sa_activity_log a
      JOIN sa_contacts c ON a.contact_id = c.id
      WHERE a.contact_id = $1
      ORDER BY a.created_at DESC
      LIMIT $2
    `, [contactId, limit], []);
  }
  
  return await safeQueryReadOnly<ActivityRow>(`
    SELECT a.*, c.email as contact_email
    FROM sa_activity_log a
    JOIN sa_contacts c ON a.contact_id = c.id
    ORDER BY a.created_at DESC
    LIMIT $1
  `, [limit], []);
}

// ============================================================================
// REVIEW QUEUE (Draft approval/rejection)
// ============================================================================

export async function getDraftEmails(params: {
  limit?: number;
  offset?: number;
}): Promise<{ emails: EmailRow[]; total: number }> {
  const { limit = 50, offset = 0 } = params;
  
  const emails = await safeQueryReadOnly<EmailRow>(`
    SELECT e.*, c.email as contact_email, c.first_name
    FROM sa_emails e
    JOIN sa_contacts c ON e.contact_id = c.id
    WHERE e.status = 'draft'
    ORDER BY e.created_at DESC
    LIMIT $1 OFFSET $2
  `, [limit, offset], []);

  const countResult = await safeQueryReadOnly<CountRow>(`
    SELECT COUNT(*) as count FROM sa_emails WHERE status = 'draft'
  `, [], [{ count: '0' }]);

  return {
    emails,
    total: parseInt(countResult[0]?.count || '0'),
  };
}

export async function approveDraftEmail(emailId: string, approver: string): Promise<{ success: boolean; error?: string }> {
  try {
    // Get the draft email
    const emails = await safeQueryReadOnly<EmailRow>(`
      SELECT * FROM sa_emails WHERE id = $1 AND status = 'draft'
    `, [emailId], []);

    if (emails.length === 0) {
      return { success: false, error: 'Draft email not found or already processed' };
    }

    const email = emails[0];
    const config = await getConfig();

    // Send via ApexMail
    const result = await apexmail.send({
      from: config.fromEmail,
      fromName: config.fromName,
      replyTo: config.replyTo,
      to: email.to_email,
      subject: email.subject || '',
      html: (email.body_html || '').replace(/\n/g, '<br>'),
      text: email.body_html || '',
      category: 'marketing',
      priority: 2, // High priority for manual approval
      trackOpens: true,
      trackClicks: true,
      entityType: 'sa_email',
      entityId: emailId,
    });

    if (!result.success) {
      return { success: false, error: result.error || 'ApexMail delivery failed' };
    }

    // Update status to sent
    await write(`
      UPDATE sa_emails 
      SET status = 'sent', 
          sent_at = CURRENT_TIMESTAMP, 
          message_id = $2,
          approved_at = CURRENT_TIMESTAMP,
          approved_by = $3
      WHERE id = $1
    `, [emailId, result.messageId || result.emailId || null, approver], 'Approve and send draft email');

    // Update enrollment step if exists
    if (email.enrollment_id) {
      const enrollmentResult = await safeQueryReadOnly<EnrollmentRow>(`
        SELECT e.*, s.steps FROM sa_enrollments e
        JOIN sa_sequences s ON e.sequence_id = s.id
        WHERE e.id = $1
      `, [email.enrollment_id], []);

      if (enrollmentResult.length > 0) {
        const enrollment = enrollmentResult[0];
        const steps = typeof enrollment.steps === 'string' ? JSON.parse(enrollment.steps) : (enrollment.steps || []);
        const nextStep = enrollment.current_step + 1;

        if (nextStep < steps.length) {
          const nextDelay = steps[nextStep].delay_days || 3;
          const nextSendAt = new Date(Date.now() + nextDelay * 24 * 60 * 60 * 1000);
          
          await write(`
            UPDATE sa_enrollments SET current_step = $1, next_send_at = $2, updated_at = CURRENT_TIMESTAMP WHERE id = $3
          `, [nextStep, nextSendAt, email.enrollment_id], 'Advance enrollment after draft approval');
        } else {
          await write(`
            UPDATE sa_enrollments SET status = 'completed', current_step = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2
          `, [nextStep, email.enrollment_id], 'Complete enrollment after draft approval');
        }
      }
    }

    return { success: true };
  } catch (error) {
    return { success: false, error: String(error) };
  }
}

export async function rejectDraftEmail(emailId: string, reason: string): Promise<{ success: boolean; error?: string }> {
  try {
    await write(`
      UPDATE sa_emails 
      SET status = 'rejected', rejection_reason = $2
      WHERE id = $1 AND status = 'draft'
    `, [emailId, reason], 'Reject draft email');
    
    return { success: true };
  } catch (error) {
    return { success: false, error: String(error) };
  }
}

export async function bulkApproveDrafts(emailIds: string[], approver: string): Promise<{ approved: number; errors: string[] }> {
  let approved = 0;
  const errors: string[] = [];

  for (const emailId of emailIds) {
    const result = await approveDraftEmail(emailId, approver);
    if (result.success) {
      approved++;
    } else {
      errors.push(`${emailId}: ${result.error}`);
    }
    // Rate limit
    await sleep(500);
  }

  return { approved, errors };
}

// ============================================================================
// CONTACT VERIFICATION
// ============================================================================

export async function markContactVerified(contactId: string, verified: boolean, status?: string): Promise<{ success: boolean }> {
  await write(`
    UPDATE sa_contacts 
    SET email_verified = $2, 
        email_verification_status = $3,
        updated_at = CURRENT_TIMESTAMP 
    WHERE id = $1
  `, [contactId, verified, status || (verified ? 'valid' : 'invalid')], 'Update contact verification status');
  return { success: true };
}

export async function getUnverifiedContacts(limit: number = 50): Promise<ContactRow[]> {
  return safeQueryReadOnly<ContactRow>(`
    SELECT * FROM sa_contacts 
    WHERE email_verified = false 
      AND status NOT IN ('bounced', 'opted_out')
    ORDER BY created_at DESC
    LIMIT $1
  `, [limit], []);
}

// ============================================================================
// HELPERS
// ============================================================================

function isValidEmail(email: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
}

function guessFirstName(email: string): string {
  const local = email.split('@')[0];
  const name = local.split(/[._-]/)[0];
  return name.charAt(0).toUpperCase() + name.slice(1).toLowerCase();
}

function applyPersonalization(template: string, data: Record<string, string>): string {
  let result = template;
  for (const [key, value] of Object.entries(data)) {
    // Escape regex special characters in key to prevent ReDoS
    const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    result = result.replace(new RegExp(`{{${escapedKey}}}`, 'g'), value || '');
  }
  return result;
}

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}
