/**
 * CONTROL PLANE LOGGER
 * Separate logging for operator actions with full audit trail
 */

import winston from 'winston';
import { getConfig } from './config';

const logFormat = winston.format.combine(
  winston.format.timestamp(),
  winston.format.errors({ stack: true }),
  winston.format.json()
);

export const logger = winston.createLogger({
  level: process.env.LOG_LEVEL || 'info',
  format: logFormat,
  defaultMeta: { service: 'control-plane' },
  transports: [
    new winston.transports.Console({
      format: winston.format.combine(
        winston.format.colorize(),
        winston.format.simple()
      ),
    }),
  ],
});

// Audit logger - separate stream for compliance
export const auditLogger = winston.createLogger({
  level: 'info',
  format: logFormat,
  defaultMeta: { service: 'control-plane-audit' },
  transports: [
    new winston.transports.Console(),
    // In production, this would go to a separate audit log store
  ],
});

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

export function logAudit(event: AuditEvent): void {
  auditLogger.info('AUDIT', {
    ...event,
    timestamp: new Date().toISOString(),
    retentionYears: parseInt(getConfig().AUDIT_RETENTION_YEARS, 10),
  });
}
