/**
 * Kill Switch API Routes
 * 
 * Control Plane endpoints for managing the global kill switch
 * and system controls for sales automation.
 * 
 * Routes:
 *   GET  /api/sales-controls/status     - Get current status of all controls
 *   POST /api/sales-controls/kill-switch - Toggle global kill switch
 *   POST /api/sales-controls/update      - Update any control setting
 *   GET  /api/sales-controls/audit       - Get audit log
 * 
 * SECURITY: All routes require operator authentication via JWT.
 */

import { Router, Request, Response } from 'express';
import { requireOperator } from '../auth';
import {
  getKillSwitchStatus,
  toggleKillSwitch,
  updateControlSetting,
  checkSendsAllowed,
} from '../services/global-kill-switch';
import {
  isEmailSuppressed,
  addToSuppressionList,
  checkSafeMode,
  setSafeMode,
} from '../services/suppression-list';
import { queryReadOnly } from '../database';
import { logger } from '../logger';

interface SuppressionRow {
  suppression_type: string;
  count: string;
}

const router = Router();

// Apply authentication middleware to all routes in this router
// This ensures only authenticated operators can access sales controls
router.use(requireOperator);

/**
 * GET /api/sales-controls/status
 * Get the current status of all sales controls
 */
router.get('/status', async (req: Request, res: Response) => {
  try {
    const status = await getKillSwitchStatus();
    
    // Add system health info
    const v1Check = await checkSendsAllowed('v1_outbound');
    const v2Check = await checkSendsAllowed('v2_autonomous');
    
    res.json({
      success: true,
      ...status,
      systemHealth: {
        v1CanSend: v1Check.allowed,
        v1BlockReason: v1Check.reason,
        v2CanSend: v2Check.allowed,
        v2BlockReason: v2Check.reason,
      },
    });
  } catch (error) {
    logger.error('Error getting sales controls status', { error });
    res.status(500).json({ 
      success: false, 
      error: 'Failed to get controls status',
      details: (error as Error).message,
    });
  }
});

/**
 * POST /api/sales-controls/kill-switch
 * Toggle the global kill switch
 * 
 * Body: { active: boolean, reason: string }
 */
router.post('/kill-switch', async (req: Request, res: Response) => {
  try {
    const { active, reason } = req.body;
    
    if (typeof active !== 'boolean') {
      res.status(400).json({ success: false, error: 'active must be a boolean' });
      return;
    }
    
    if (!reason || typeof reason !== 'string' || reason.length < 5) {
      res.status(400).json({ success: false, error: 'reason is required (min 5 characters)' });
      return;
    }
    
    // Get operator identity from authenticated operator (set by requireOperator middleware)
    const changedBy = req.operator?.email || 'unknown_operator';
    const ipAddress = req.ip || req.headers['x-forwarded-for'] as string || undefined;
    
    const result = await toggleKillSwitch(active, changedBy, reason, ipAddress);
    
    if (result.success) {
      // Log important action
      logger.warn(`🚨 KILL SWITCH ${active ? 'ACTIVATED' : 'DEACTIVATED'} by ${changedBy}`, {
        reason,
        previousState: result.previousState,
        ipAddress,
      });
      
      res.json({
        success: true,
        killSwitchActive: active,
        previousState: result.previousState,
        message: active 
          ? '🚨 All outbound email sending is now BLOCKED' 
          : '✅ Outbound email sending is now ENABLED',
      });
    } else {
      res.status(500).json({ success: false, error: result.error });
    }
  } catch (error) {
    logger.error('Error toggling kill switch', { error });
    res.status(500).json({ success: false, error: (error as Error).message });
  }
});

/**
 * POST /api/sales-controls/update
 * Update a specific control setting
 * 
 * Body: { key: string, value: string, reason: string }
 */
router.post('/update', async (req: Request, res: Response) => {
  try {
    const { key, value, reason } = req.body;
    
    // Validate inputs
    const validKeys = [
      'global_kill_switch',
      'v1_outbound_enabled',
      'v2_autonomous_enabled',
      'max_daily_sends_global',
      'bounce_rate_threshold',
    ];
    
    if (!validKeys.includes(key)) {
      res.status(400).json({ 
        success: false, 
        error: `Invalid key. Must be one of: ${validKeys.join(', ')}`,
      });
      return;
    }
    
    if (!reason || reason.length < 5) {
      res.status(400).json({ success: false, error: 'reason is required (min 5 characters)' });
      return;
    }
    
    const changedBy = req.operator?.email || 'unknown_operator';
    const ipAddress = req.ip || req.headers['x-forwarded-for'] as string || undefined;
    
    const result = await updateControlSetting(key, value, changedBy, reason, ipAddress);
    
    if (result.success) {
      res.json({ success: true, key, value, message: `Control ${key} updated to ${value}` });
    } else {
      res.status(500).json({ success: false, error: result.error });
    }
  } catch (error) {
    logger.error('Error updating control setting', { error });
    res.status(500).json({ success: false, error: (error as Error).message });
  }
});

/**
 * GET /api/sales-controls/audit
 * Get the audit log for control changes
 * 
 * Query: { limit?: number, offset?: number, key?: string }
 */
router.get('/audit', async (req: Request, res: Response) => {
  try {
    const limit = Math.min(parseInt(req.query.limit as string) || 50, 200);
    const offset = parseInt(req.query.offset as string) || 0;
    const keyFilter = req.query.key as string | undefined;
    
    let auditEntries: any[] = [];
    let total = 0;
    
    if (keyFilter) {
      auditEntries = await queryReadOnly(`
        SELECT * FROM sales_control_audit 
        WHERE control_key = $1
        ORDER BY changed_at DESC
        LIMIT $2 OFFSET $3
      `, [keyFilter, limit, offset]);
      
      const countResult = await queryReadOnly<{ count: string }>(`
        SELECT COUNT(*) as count FROM sales_control_audit WHERE control_key = $1
      `, [keyFilter]);
      total = parseInt(countResult[0]?.count || '0');
    } else {
      auditEntries = await queryReadOnly(`
        SELECT * FROM sales_control_audit 
        ORDER BY changed_at DESC
        LIMIT $1 OFFSET $2
      `, [limit, offset]);
      
      const countResult = await queryReadOnly<{ count: string }>(`
        SELECT COUNT(*) as count FROM sales_control_audit
      `, []);
      total = parseInt(countResult[0]?.count || '0');
    }
    
    res.json({
      success: true,
      entries: auditEntries,
      total,
      limit,
      offset,
    });
  } catch (error) {
    // Handle missing table gracefully
    if ((error as any)?.code === '42P01') {
      res.json({ success: true, entries: [], total: 0, limit: 50, offset: 0 });
      return;
    }
    logger.error('Error getting audit log', { error });
    res.status(500).json({ success: false, error: (error as Error).message });
  }
});

/**
 * GET /api/sales-controls/unified-metrics
 * Get unified metrics across V1 and V2 systems
 */
router.get('/unified-metrics', async (req: Request, res: Response) => {
  try {
    // Get daily metrics
    const dailyMetrics = await queryReadOnly(`
      SELECT * FROM unified_daily_metrics
      WHERE metric_date >= CURRENT_DATE - INTERVAL '30 days'
      ORDER BY metric_date DESC, source_system
    `, []);
    
    // Get totals
    const totals = await queryReadOnly(`
      SELECT * FROM unified_metrics_totals
    `, []);
    
    // Get template performance
    const templatePerformance = await queryReadOnly(`
      SELECT * FROM unified_template_performance
      LIMIT 20
    `, []);
    
    // Get domain health
    const domainHealth = await queryReadOnly(`
      SELECT * FROM unified_domain_health
      WHERE bounce_rate > 3 OR complaint_rate > 0.1
      LIMIT 20
    `, []);
    
    // Get today's status
    const todayStatus = await queryReadOnly(`
      SELECT * FROM unified_today_status
    `, []);
    
    res.json({
      success: true,
      dailyMetrics,
      totals: totals[0] || {},
      templatePerformance,
      domainHealth,
      todayStatus,
    });
  } catch (error) {
    // Handle missing views gracefully
    if ((error as any)?.code === '42P01') {
      res.json({
        success: true,
        dailyMetrics: [],
        totals: {},
        templatePerformance: [],
        domainHealth: [],
        todayStatus: [],
        message: 'Unified metrics views not yet created. Run migration 057.',
      });
      return;
    }
    logger.error('Error getting unified metrics', { error });
    res.status(500).json({ success: false, error: (error as Error).message });
  }
});

// ========== SUPPRESSION LIST ENDPOINTS ==========

/**
 * GET /api/sales-controls/suppression-stats
 * Get suppression list statistics by type
 */
router.get('/suppression-stats', async (req: Request, res: Response) => {
  try {
    const stats = await queryReadOnly(`
      SELECT 
        suppression_type,
        COUNT(*) as count
      FROM sales_suppression_list
      WHERE is_active = true
      GROUP BY suppression_type
    `, []);

    const result: { hardBounces: number; unsubscribes: number; complaints: number; manual: number } = {
      hardBounces: 0,
      unsubscribes: 0,
      complaints: 0,
      manual: 0,
    };

    for (const row of stats as SuppressionRow[]) {
      switch (row.suppression_type) {
        case 'hard_bounce':
          result.hardBounces = parseInt(row.count);
          break;
        case 'unsubscribe':
          result.unsubscribes = parseInt(row.count);
          break;
        case 'spam_complaint':
          result.complaints = parseInt(row.count);
          break;
        case 'manual_block':
        case 'competitor':
        case 'gdpr_erasure':
          result.manual += parseInt(row.count);
          break;
      }
    }

    res.json(result);
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      res.json({ hardBounces: 0, unsubscribes: 0, complaints: 0, manual: 0 });
      return;
    }
    logger.error('Error getting suppression stats', { error });
    res.status(500).json({ error: (error as Error).message });
  }
});

/**
 * GET /api/sales-controls/suppression-list
 * Get recent suppression entries
 */
router.get('/suppression-list', async (req: Request, res: Response) => {
  try {
    const limit = Math.min(parseInt(req.query.limit as string) || 50, 200);
    const offset = parseInt(req.query.offset as string) || 0;

    const entries = await queryReadOnly(`
      SELECT 
        email,
        suppression_type,
        reason,
        source_system,
        created_at
      FROM sales_suppression_list
      WHERE is_active = true
      ORDER BY created_at DESC
      LIMIT $1 OFFSET $2
    `, [limit, offset]);

    res.json(entries);
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      res.json([]);
      return;
    }
    logger.error('Error getting suppression list', { error });
    res.status(500).json({ error: (error as Error).message });
  }
});

/**
 * POST /api/sales-controls/suppression-list
 * Add an email to the suppression list
 */
router.post('/suppression-list', async (req: Request, res: Response) => {
  try {
    const { email, type, reason } = req.body;

    if (!email || !reason) {
      res.status(400).json({ error: 'email and reason are required' });
      return;
    }

    const addedBy = req.operator?.email || 'unknown_operator';
    const suppressionId = await addToSuppressionList({
      email,
      type: type || 'manual_block',
      sourceSystem: 'manual_operator',
      reason,
      createdBy: addedBy,
    });

    res.json({ success: true, message: `Added ${email} to suppression list`, id: suppressionId });
  } catch (error) {
    logger.error('Error adding to suppression list', { error });
    res.status(500).json({ error: (error as Error).message });
  }
});

/**
 * POST /api/sales-controls/suppression-check
 * Check if an email is suppressed (for testing)
 */
router.post('/suppression-check', async (req: Request, res: Response) => {
  try {
    const { email } = req.body;

    if (!email) {
      res.status(400).json({ error: 'email is required' });
      return;
    }

    const result = await isEmailSuppressed(email);
    res.json(result);
  } catch (error) {
    logger.error('Error checking suppression', { error });
    res.status(500).json({ error: (error as Error).message });
  }
});

// ========== SAFE MODE ENDPOINTS ==========

/**
 * GET /api/sales-controls/safe-mode
 * Get current safe mode status
 */
router.get('/safe-mode', async (req: Request, res: Response) => {
  try {
    const status = await checkSafeMode();
    res.json(status);
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      res.json({
        state: 'normal',
        bounceRate: 0,
        complaintRate: 0,
        throttlePercent: 100,
        emailsSent7d: 0,
      });
      return;
    }
    logger.error('Error getting safe mode status', { error });
    res.status(500).json({ error: (error as Error).message });
  }
});

/**
 * POST /api/sales-controls/safe-mode
 * Manually set safe mode state (for emergencies)
 */
router.post('/safe-mode', async (req: Request, res: Response) => {
  try {
    const { state, reason } = req.body;

    if (!['normal', 'warning', 'critical', 'paused'].includes(state)) {
      res.status(400).json({ error: 'state must be normal, warning, critical, or paused' });
      return;
    }

    if (!reason) {
      res.status(400).json({ error: 'reason is required' });
      return;
    }

    const changedBy = req.operator?.email || 'unknown_operator';
    await setSafeMode(state, changedBy, reason);

    res.json({ success: true, state, message: `Safe mode set to ${state}` });
  } catch (error) {
    logger.error('Error setting safe mode', { error });
    res.status(500).json({ error: (error as Error).message });
  }
});

// ========== CONTROL AUDIT LOG (simplified) ==========

/**
 * GET /api/sales-controls/control-audit-log
 * Get recent control change audit entries
 */
router.get('/control-audit-log', async (req: Request, res: Response) => {
  try {
    const limit = Math.min(parseInt(req.query.limit as string) || 20, 100);

    const entries = await queryReadOnly(`
      SELECT 
        control_key as action,
        old_value || ' → ' || new_value as details,
        changed_by as actor,
        changed_at as created_at
      FROM sales_control_audit
      ORDER BY changed_at DESC
      LIMIT $1
    `, [limit]);

    res.json(entries);
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      res.json([]);
      return;
    }
    logger.error('Error getting control audit log', { error });
    res.status(500).json({ error: (error as Error).message });
  }
});

export default router;
