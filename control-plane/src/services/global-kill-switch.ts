/**
 * Global Kill Switch Service
 * 
 * Centralized control for ALL outbound email sending across V1 and V2 systems.
 * Uses the sales_system_controls table for persistent, DB-backed configuration.
 * 
 * USAGE:
 *   import { isGlobalKillSwitchActive, checkSendsAllowed, toggleKillSwitch } from './global-kill-switch';
 *   
 *   // At the top of any send function:
 *   const sendsAllowed = await checkSendsAllowed();
 *   if (!sendsAllowed.allowed) {
 *     return { error: sendsAllowed.reason };
 *   }
 * 
 * SAFETY:
 *   - Kill switch is checked at EVERY send entry point (V1, V2, manual sends)
 *   - All changes are audit-logged with timestamp, user, and reason
 *   - Can be toggled from Control Plane UI or API
 */

import { queryReadOnly, queryOperator } from '../database';
import { logger } from '../logger';

// Cache configuration
interface KillSwitchCache {
  globalKillSwitch: boolean;
  v1OutboundEnabled: boolean;
  v2AutonomousEnabled: boolean;
  maxDailySendsGlobal: number;
  bounceRateThreshold: number;
  lastFetched: number;
}

let cache: KillSwitchCache | null = null;
const CACHE_TTL_MS = 10000; // 10 seconds - short for safety-critical settings

/**
 * Check if the global kill switch is active (TRUE = all sends blocked)
 */
export async function isGlobalKillSwitchActive(): Promise<boolean> {
  try {
    const row = await queryReadOnly<{ control_value: string }>(`
      SELECT control_value FROM sales_system_controls WHERE control_key = 'global_kill_switch'
    `, []);
    
    if (row.length === 0) {
      // Table doesn't exist or no row - fail OPEN (allow sends)
      logger.warn('Global kill switch not found in DB, defaulting to INACTIVE');
      return false;
    }
    
    return row[0].control_value === 'true';
  } catch (error) {
    // If table doesn't exist, fail OPEN
    if ((error as any)?.code === '42P01') {
      logger.warn('sales_system_controls table missing, kill switch inactive by default');
      return false;
    }
    throw error;
  }
}

/**
 * Get all control settings (cached)
 */
export async function getControlSettings(): Promise<KillSwitchCache> {
  const now = Date.now();
  
  if (cache && (now - cache.lastFetched) < CACHE_TTL_MS) {
    return cache;
  }
  
  try {
    const rows = await queryReadOnly<{ control_key: string; control_value: string }>(`
      SELECT control_key, control_value FROM sales_system_controls
    `, []);
    
    const settings = new Map(rows.map(r => [r.control_key, r.control_value]));
    
    cache = {
      globalKillSwitch: settings.get('global_kill_switch') === 'true',
      v1OutboundEnabled: settings.get('v1_outbound_enabled') !== 'false', // default true
      v2AutonomousEnabled: settings.get('v2_autonomous_enabled') !== 'false', // default true
      maxDailySendsGlobal: parseInt(settings.get('max_daily_sends_global') || '500'),
      bounceRateThreshold: parseInt(settings.get('bounce_rate_threshold') || '7'),
      lastFetched: now,
    };
    
    return cache;
  } catch (error) {
    // If table doesn't exist, return safe defaults
    if ((error as any)?.code === '42P01') {
      logger.warn('sales_system_controls table missing, using defaults');
      return {
        globalKillSwitch: false,
        v1OutboundEnabled: true,
        v2AutonomousEnabled: true,
        maxDailySendsGlobal: 500,
        bounceRateThreshold: 7,
        lastFetched: now,
      };
    }
    throw error;
  }
}

/**
 * Invalidate cache (call after any control change)
 */
export function invalidateKillSwitchCache(): void {
  cache = null;
}

/**
 * Check if sends are allowed for a specific system
 * 
 * @param system - 'v1_outbound' | 'v2_autonomous' | 'any'
 * @returns { allowed: boolean, reason?: string }
 */
export async function checkSendsAllowed(system: 'v1_outbound' | 'v2_autonomous' | 'any' = 'any'): Promise<{
  allowed: boolean;
  reason?: string;
}> {
  try {
    const settings = await getControlSettings();
    
    // Global kill switch takes precedence
    if (settings.globalKillSwitch) {
      logger.warn('🚨 GLOBAL KILL SWITCH ACTIVE - All sends blocked');
      return {
        allowed: false,
        reason: '🚨 GLOBAL KILL SWITCH ACTIVE - All outbound email sending is blocked. Contact an administrator to re-enable.',
      };
    }
    
    // System-specific checks
    if (system === 'v1_outbound' && !settings.v1OutboundEnabled) {
      return {
        allowed: false,
        reason: 'V1 outbound sending is disabled. Enable it from Control Plane settings.',
      };
    }
    
    if (system === 'v2_autonomous' && !settings.v2AutonomousEnabled) {
      return {
        allowed: false,
        reason: 'V2 autonomous sending is disabled. Enable it from Control Plane settings.',
      };
    }
    
    return { allowed: true };
  } catch (error) {
    // If we can't check, fail OPEN with warning
    logger.error('Error checking kill switch, failing OPEN', { error });
    return { allowed: true };
  }
}

/**
 * Toggle the global kill switch
 * 
 * @param active - true = block all sends, false = allow sends
 * @param changedBy - User/operator identifier
 * @param reason - Why the change was made
 * @param ipAddress - Optional IP for audit
 */
export async function toggleKillSwitch(
  active: boolean,
  changedBy: string,
  reason: string,
  ipAddress?: string
): Promise<{ success: boolean; previousState: boolean; error?: string }> {
  try {
    // Get current state
    const currentState = await isGlobalKillSwitchActive();
    
    // Update via the stored procedure for proper auditing
    await queryOperator(`
      SELECT update_sales_control('global_kill_switch', $1, $2, $3, $4)
    `, [
      active ? 'true' : 'false',
      changedBy,
      reason,
      ipAddress || null,
    ], changedBy, `Toggle global kill switch to ${active ? 'ACTIVE' : 'INACTIVE'}: ${reason}`);
    
    // Invalidate cache
    invalidateKillSwitchCache();
    
    // Log the change
    logger.warn(`🚨 GLOBAL KILL SWITCH ${active ? 'ACTIVATED' : 'DEACTIVATED'}`, {
      changedBy,
      reason,
      previousState: currentState,
      newState: active,
    });
    
    return {
      success: true,
      previousState: currentState,
    };
  } catch (error) {
    logger.error('Failed to toggle kill switch', { error, active, changedBy, reason });
    return {
      success: false,
      previousState: await isGlobalKillSwitchActive(),
      error: (error as Error).message,
    };
  }
}

/**
 * Update a specific control setting
 */
export async function updateControlSetting(
  key: string,
  value: string,
  changedBy: string,
  reason: string,
  ipAddress?: string
): Promise<{ success: boolean; error?: string }> {
  try {
    await queryOperator(`
      SELECT update_sales_control($1, $2, $3, $4, $5)
    `, [key, value, changedBy, reason, ipAddress || null],
    changedBy, `Update sales control ${key} to ${value}: ${reason}`);
    
    invalidateKillSwitchCache();
    
    logger.info(`Sales control updated: ${key} = ${value}`, { changedBy, reason });
    
    return { success: true };
  } catch (error) {
    logger.error('Failed to update control setting', { error, key, value, changedBy });
    return { success: false, error: (error as Error).message };
  }
}

/**
 * Get kill switch status for dashboard display
 */
export async function getKillSwitchStatus(): Promise<{
  globalKillSwitchActive: boolean;
  v1OutboundEnabled: boolean;
  v2AutonomousEnabled: boolean;
  maxDailySendsGlobal: number;
  lastChecked: string;
  recentAuditEntries: Array<{
    control_key: string;
    old_value: string | null;
    new_value: string;
    changed_by: string;
    change_reason: string | null;
    changed_at: string;
  }>;
}> {
  const settings = await getControlSettings();
  
  // Get recent audit entries
  let auditEntries: any[] = [];
  try {
    auditEntries = await queryReadOnly<{
      control_key: string;
      old_value: string | null;
      new_value: string;
      changed_by: string;
      change_reason: string | null;
      changed_at: string;
    }>(`
      SELECT control_key, old_value, new_value, changed_by, change_reason, changed_at
      FROM sales_control_audit
      ORDER BY changed_at DESC
      LIMIT 10
    `, []);
  } catch {
    // Audit table might not exist yet
  }
  
  return {
    globalKillSwitchActive: settings.globalKillSwitch,
    v1OutboundEnabled: settings.v1OutboundEnabled,
    v2AutonomousEnabled: settings.v2AutonomousEnabled,
    maxDailySendsGlobal: settings.maxDailySendsGlobal,
    lastChecked: new Date().toISOString(),
    recentAuditEntries: auditEntries,
  };
}
