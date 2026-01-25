import { queryOperator } from './database';
import { logger, auditLogger } from './logger';
import { getConfig } from './config';

/**
 * Log an audit event (immutable, compliance-grade)
 */
export interface AuditEvent {
  actor: string;
  action: string;
  target?: string;
  preState?: unknown;
  postState?: unknown;
  result: 'success' | 'failure' | 'denied';
  reason?: string;
  ip?: string;
  userAgent?: string;
}

export async function logAudit(event: AuditEvent): Promise<void> {
  // 1. Log to console/winston for immediate visibility
  auditLogger.info('AUDIT', {
    ...event,
    timestamp: new Date().toISOString(),
    retentionYears: parseInt(getConfig().AUDIT_RETENTION_YEARS, 10),
  });

  // 2. Persist to database for 7-year retention
  try {
    const sql = `
      INSERT INTO audit_logs (
        actor,
        action,
        target,
        pre_state,
        post_state,
        result,
        reason,
        ip_address,
        user_agent,
        timestamp
      ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW())
    `;

    await queryOperator(
      sql,
      [
        event.actor,
        event.action,
        event.target || null,
        event.preState ? JSON.stringify(event.preState) : null,
        event.postState ? JSON.stringify(event.postState) : null,
        event.result,
        event.reason || null,
        event.ip || null,
        event.userAgent || null
      ],
      'system',
      'Audit log persistence'
    );
  } catch (err) {
    // Critical failure: cannot write to audit log
    logger.error('CRITICAL: Failed to persist audit log to database', { 
      error: (err as Error).message,
      event 
    });
  }
}
