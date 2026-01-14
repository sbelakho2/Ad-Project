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
        action,
        resource_type,
        resource_id,
        changes,
        ip_address,
        user_agent,
        created_at
      ) VALUES ($1, $2, $3, $4, $5, $6, NOW())
    `;

    // Map AuditEvent to audit_logs table
    // Since audit_logs has publisher_id/user_id as UUIDs, and we have actor as string,
    // we store the actor in the 'changes' or metadata for now, or use a dedicated table.
    // Given the schema from data/schemas/postgresql.sql:
    // resource_type, resource_id, changes (JSONB)
    
    const changes = {
      actor: event.actor,
      result: event.result,
      reason: event.reason,
      preState: event.preState,
      postState: event.postState,
    };

    // We don't have a reliable mapping for UUIDs here without extra lookups,
    // so we store the technical details in 'changes'.
    
    await queryOperator(
      sql,
      [
        event.action,
        event.target || 'system',
        null, // resource_id
        JSON.stringify(changes),
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
