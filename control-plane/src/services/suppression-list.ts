/**
 * Unified Suppression List Service
 * 
 * Centralized service for managing email suppression across ALL sales systems (V1 + V2).
 * This ensures NO system can send to:
 * - Hard bounced emails
 * - Spam complainers
 * - Unsubscribed users
 * - GDPR erasure requests
 * - Competitor domains
 * - Role-based addresses
 * 
 * USAGE:
 *   import { isEmailSuppressed, addToSuppressionList } from './suppression-list';
 *   
 *   // Check before ANY send
 *   const suppressed = await isEmailSuppressed('user@example.com');
 *   if (suppressed.isSuppressed) {
 *     return { error: `Cannot send: ${suppressed.reason}` };
 *   }
 */

import { queryReadOnly, queryOperator } from '../database.js';
import { logger } from '../logger.js';

// ============================================================================
// TYPES
// ============================================================================

export type SuppressionType = 
  | 'hard_bounce'
  | 'soft_bounce'
  | 'spam_complaint'
  | 'unsubscribe'
  | 'gdpr_erasure'
  | 'manual_block'
  | 'competitor'
  | 'role_based'
  | 'invalid_format'
  | 'honeypot';

export type SourceSystem = 
  | 'v1_outbound'
  | 'v2_autonomous'
  | 'webhook_bounce'
  | 'webhook_complaint'
  | 'manual_operator'
  | 'sync_import';

export interface SuppressionCheckResult {
  isSuppressed: boolean;
  suppressionType?: SuppressionType;
  reason?: string;
  suppressedAt?: Date;
}

export interface SuppressionEntry {
  id: string;
  email: string;
  domain: string | null;
  suppressionType: SuppressionType;
  sourceSystem: SourceSystem;
  reason: string | null;
  evidence: Record<string, any>;
  expiresAt: Date | null;
  createdAt: Date;
  createdBy: string | null;
}

// ============================================================================
// SUPPRESSION CHECK
// ============================================================================

/**
 * Check if an email is suppressed (MUST be called before every send)
 * 
 * @param email - Email address to check
 * @returns SuppressionCheckResult with suppression details if blocked
 */
export async function isEmailSuppressed(email: string): Promise<SuppressionCheckResult> {
  try {
    // Use the database function for fast lookup
    const result = await queryReadOnly<{
      is_suppressed: boolean;
      suppression_type: SuppressionType | null;
      reason: string | null;
      suppressed_at: Date | null;
    }>(`
      SELECT * FROM is_email_suppressed($1)
    `, [email.toLowerCase()]);
    
    if (result.length > 0 && result[0].is_suppressed) {
      logger.debug('Email suppressed', { 
        email: email.substring(0, 3) + '***', 
        type: result[0].suppression_type 
      });
      
      return {
        isSuppressed: true,
        suppressionType: result[0].suppression_type!,
        reason: result[0].reason || `Email is on suppression list (${result[0].suppression_type})`,
        suppressedAt: result[0].suppressed_at || undefined,
      };
    }
    
    return { isSuppressed: false };
  } catch (error) {
    // If table doesn't exist, fail OPEN with warning
    if ((error as any)?.code === '42P01' || (error as any)?.code === '42883') {
      logger.warn('Suppression list not available, allowing send', { error: (error as Error).message });
      return { isSuppressed: false };
    }
    throw error;
  }
}

/**
 * Check if a domain is blocked (for competitor domains)
 */
export async function isDomainBlocked(domain: string): Promise<boolean> {
  try {
    const result = await queryReadOnly<{ is_domain_blocked: boolean }>(`
      SELECT is_domain_blocked($1) as is_domain_blocked
    `, [domain.toLowerCase()]);
    
    return result[0]?.is_domain_blocked ?? false;
  } catch (error) {
    if ((error as any)?.code === '42P01' || (error as any)?.code === '42883') {
      return false;
    }
    throw error;
  }
}

/**
 * Batch check multiple emails for suppression
 * More efficient than checking one at a time
 */
export async function checkBatchSuppression(emails: string[]): Promise<Map<string, SuppressionCheckResult>> {
  const results = new Map<string, SuppressionCheckResult>();
  
  if (emails.length === 0) {
    return results;
  }
  
  try {
    const suppressed = await queryReadOnly<{
      email: string;
      suppression_type: SuppressionType;
      reason: string | null;
      created_at: Date;
    }>(`
      SELECT LOWER(email) as email, suppression_type, reason, created_at
      FROM sales_suppression_list
      WHERE LOWER(email) = ANY($1)
        AND (expires_at IS NULL OR expires_at > CURRENT_TIMESTAMP)
    `, [emails.map(e => e.toLowerCase())]);
    
    // Initialize all as not suppressed
    for (const email of emails) {
      results.set(email.toLowerCase(), { isSuppressed: false });
    }
    
    // Mark suppressed ones
    for (const row of suppressed) {
      results.set(row.email, {
        isSuppressed: true,
        suppressionType: row.suppression_type,
        reason: row.reason || `Suppressed: ${row.suppression_type}`,
        suppressedAt: row.created_at,
      });
    }
    
    return results;
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      // Table doesn't exist - return all as not suppressed
      for (const email of emails) {
        results.set(email.toLowerCase(), { isSuppressed: false });
      }
      return results;
    }
    throw error;
  }
}

// ============================================================================
// ADD TO SUPPRESSION LIST
// ============================================================================

export interface AddSuppressionParams {
  email: string;
  type: SuppressionType;
  sourceSystem: SourceSystem;
  reason?: string;
  evidence?: Record<string, any>;
  createdBy?: string;
  sourceMessageId?: string;
  sourceContactId?: string;
}

/**
 * Add an email to the suppression list
 * 
 * @param params - Suppression entry details
 * @returns The suppression entry ID
 */
export async function addToSuppressionList(params: AddSuppressionParams): Promise<string> {
  const {
    email,
    type,
    sourceSystem,
    reason,
    evidence = {},
    createdBy = 'system',
    sourceMessageId,
    sourceContactId,
  } = params;
  
  try {
    const result = await queryOperator<{ add_to_suppression_list: string }>(`
      SELECT add_to_suppression_list(
        $1, $2, $3, $4, $5, $6, $7, $8
      ) as add_to_suppression_list
    `, [
      email,
      type,
      sourceSystem,
      reason || null,
      JSON.stringify(evidence),
      createdBy,
      sourceMessageId || null,
      sourceContactId || null,
    ], createdBy, `Added ${email} to suppression list (${type})`);
    
    logger.info('Added email to suppression list', {
      email: email.substring(0, 3) + '***@' + email.split('@')[1],
      type,
      sourceSystem,
    });
    
    return result[0].add_to_suppression_list;
  } catch (error) {
    logger.error('Failed to add to suppression list', { error, email: email.substring(0, 5) + '***' });
    throw error;
  }
}

/**
 * Remove an email from the suppression list (for GDPR compliance - right to be forgotten reversal)
 * Only removes specific suppression types - hard bounces stay permanent
 */
export async function removeFromSuppressionList(
  email: string, 
  type: SuppressionType,
  removedBy: string,
  reason: string
): Promise<boolean> {
  // Certain types should never be removed
  if (['hard_bounce', 'honeypot'].includes(type)) {
    logger.warn('Cannot remove permanent suppression type', { email: email.substring(0, 5) + '***', type });
    return false;
  }
  
  try {
    const result = await queryOperator(`
      DELETE FROM sales_suppression_list
      WHERE LOWER(email) = LOWER($1) AND suppression_type = $2
      RETURNING id
    `, [email, type], removedBy, `Removed ${email} from suppression list (${type}): ${reason}`);
    
    if (result.length > 0) {
      logger.info('Removed email from suppression list', {
        email: email.substring(0, 3) + '***',
        type,
        removedBy,
        reason,
      });
      return true;
    }
    
    return false;
  } catch (error) {
    logger.error('Failed to remove from suppression list', { error });
    throw error;
  }
}

// ============================================================================
// SUPPRESSION LIST MANAGEMENT
// ============================================================================

/**
 * Get suppression list summary by type
 */
export async function getSuppressionSummary(): Promise<{
  type: SuppressionType;
  count: number;
  activeCount: number;
  latestAdded: Date | null;
}[]> {
  try {
    const result = await queryReadOnly<{
      suppression_type: SuppressionType;
      count: string;
      active_count: string;
      latest_added: Date | null;
    }>(`
      SELECT * FROM suppression_list_summary
    `, []);
    
    return result.map(r => ({
      type: r.suppression_type,
      count: parseInt(r.count),
      activeCount: parseInt(r.active_count),
      latestAdded: r.latest_added,
    }));
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      return [];
    }
    throw error;
  }
}

/**
 * Get recent suppression entries
 */
export async function getRecentSuppressions(limit: number = 100): Promise<SuppressionEntry[]> {
  try {
    const result = await queryReadOnly<{
      id: string;
      email: string;
      domain: string | null;
      suppression_type: SuppressionType;
      source_system: SourceSystem;
      reason: string | null;
      evidence: Record<string, any>;
      expires_at: Date | null;
      created_at: Date;
      created_by: string | null;
    }>(`
      SELECT id, email, domain, suppression_type, source_system,
             reason, evidence, expires_at, created_at, created_by
      FROM sales_suppression_list
      ORDER BY created_at DESC
      LIMIT $1
    `, [limit]);
    
    return result.map(r => ({
      id: r.id,
      email: r.email,
      domain: r.domain,
      suppressionType: r.suppression_type,
      sourceSystem: r.source_system,
      reason: r.reason,
      evidence: r.evidence || {},
      expiresAt: r.expires_at,
      createdAt: r.created_at,
      createdBy: r.created_by,
    }));
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      return [];
    }
    throw error;
  }
}

/**
 * Search suppression list
 */
export async function searchSuppressionList(
  query: string,
  options: { type?: SuppressionType; limit?: number } = {}
): Promise<SuppressionEntry[]> {
  const { type, limit = 50 } = options;
  
  try {
    let sql = `
      SELECT id, email, domain, suppression_type, source_system,
             reason, evidence, expires_at, created_at, created_by
      FROM sales_suppression_list
      WHERE (LOWER(email) LIKE $1 OR LOWER(domain) LIKE $1)
    `;
    const params: any[] = [`%${query.toLowerCase()}%`];
    
    if (type) {
      sql += ` AND suppression_type = $2`;
      params.push(type);
    }
    
    sql += ` ORDER BY created_at DESC LIMIT $${params.length + 1}`;
    params.push(limit);
    
    const result = await queryReadOnly<{
      id: string;
      email: string;
      domain: string | null;
      suppression_type: SuppressionType;
      source_system: SourceSystem;
      reason: string | null;
      evidence: Record<string, any>;
      expires_at: Date | null;
      created_at: Date;
      created_by: string | null;
    }>(sql, params);
    
    return result.map(r => ({
      id: r.id,
      email: r.email,
      domain: r.domain,
      suppressionType: r.suppression_type,
      sourceSystem: r.source_system,
      reason: r.reason,
      evidence: r.evidence || {},
      expiresAt: r.expires_at,
      createdAt: r.created_at,
      createdBy: r.created_by,
    }));
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      return [];
    }
    throw error;
  }
}

// ============================================================================
// SAFE MODE
// ============================================================================

export interface SafeModeStatus {
  canSend: boolean;
  mode: 'normal' | 'warning' | 'critical' | 'paused';
  throttlePercent: number;
  reason: string | null;
  bounceRate: number;
  complaintRate: number;
  emailsSent7d: number;
}

/**
 * Check current safe mode status
 * MUST be called before any send operation
 */
export async function checkSafeMode(): Promise<SafeModeStatus> {
  try {
    const result = await queryReadOnly<{
      can_send: boolean;
      current_mode: string;
      throttle_percent: number;
      reason: string | null;
    }>(`SELECT * FROM check_safe_mode()`, []);
    
    const state = await queryReadOnly<{
      current_bounce_rate: string;
      current_complaint_rate: string;
      emails_sent_7d: number;
    }>(`SELECT current_bounce_rate, current_complaint_rate, emails_sent_7d FROM sales_safe_mode_state LIMIT 1`, []);
    
    return {
      canSend: result[0]?.can_send ?? true,
      mode: (result[0]?.current_mode as SafeModeStatus['mode']) ?? 'normal',
      throttlePercent: result[0]?.throttle_percent ?? 100,
      reason: result[0]?.reason ?? null,
      bounceRate: parseFloat(state[0]?.current_bounce_rate ?? '0'),
      complaintRate: parseFloat(state[0]?.current_complaint_rate ?? '0'),
      emailsSent7d: state[0]?.emails_sent_7d ?? 0,
    };
  } catch (error) {
    if ((error as any)?.code === '42P01' || (error as any)?.code === '42883') {
      // Tables don't exist - return normal mode
      return {
        canSend: true,
        mode: 'normal',
        throttlePercent: 100,
        reason: null,
        bounceRate: 0,
        complaintRate: 0,
        emailsSent7d: 0,
      };
    }
    throw error;
  }
}

/**
 * Update safe mode state based on current metrics
 * Should be called periodically (every 15 minutes)
 */
export async function updateSafeModeState(): Promise<void> {
  try {
    await queryOperator(`SELECT update_safe_mode_state()`, [], 'system', 'Update safe mode state');
    logger.debug('Safe mode state updated');
  } catch (error) {
    if ((error as any)?.code === '42P01' || (error as any)?.code === '42883') {
      return; // Tables don't exist
    }
    logger.error('Failed to update safe mode state', { error });
  }
}

/**
 * Manually set safe mode (for emergency use)
 */
export async function setSafeMode(
  mode: 'normal' | 'warning' | 'critical' | 'paused',
  changedBy: string,
  reason: string
): Promise<void> {
  const throttle = mode === 'normal' ? 100 : mode === 'warning' ? 50 : 0;
  
  await queryOperator(`
    UPDATE sales_safe_mode_state SET
      mode = $1,
      throttle_percent = $2,
      mode_changed_at = CURRENT_TIMESTAMP,
      mode_changed_by = $3,
      mode_change_reason = $4,
      updated_at = CURRENT_TIMESTAMP
    WHERE id = 1
  `, [mode, throttle, changedBy, reason], changedBy, `Set safe mode to ${mode}: ${reason}`);
  
  logger.warn(`Safe mode manually set to ${mode}`, { changedBy, reason });
}
