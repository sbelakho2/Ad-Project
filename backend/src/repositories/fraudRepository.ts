import { query } from '../utils/postgres';
import { toNumber } from '../utils/number';

export interface FraudStats {
  totalAlerts: number;
  highSeverity: number;
  mediumSeverity: number;
  lowSeverity: number;
  blockedRevenue: number;
  lastDetectionAt?: string;
}

export interface FraudAlertRow {
  id: string;
  type: string;
  severity: string;
  details: string;
  detectedAt: string;
}

export interface FraudTypeBreakdown {
  type: string;
  alerts: number;
  blockedRevenue: number;
}

export type FraudWindow = '1h' | '24h' | '7d' | '30d';

export interface FraudStatsV2 {
  publisherId: string;
  timeWindow: FraudWindow;
  totalRequests: number;
  fraudRequests: number;
  fraudRate: number;
  givtDetections: number;
  sivtDetections: number;
  mlDetections: number;
  anomalyDetections: number;
  blockedRequests: number;
  flaggedRequests: number;
  averageFraudScore: number;
  topFraudTypes: string[];
  lastUpdated: string;
}

export interface FraudAlertV2Row {
  id: string;
  timestamp: string;
  alertType: 'givt' | 'sivt' | 'ml_fraud' | 'anomaly';
  severity: 'low' | 'medium' | 'high' | 'critical';
  publisherId: string;
  deviceId?: string;
  ipAddress?: string;
  fraudScore: number;
  detectionType: 'rule_based' | 'ml_based' | 'anomaly';
  action: 'block' | 'flag' | 'monitor';
  metadata: Record<string, any>;
}

const clampWindow = (value: string | undefined): FraudWindow => {
  if (value === '1h' || value === '24h' || value === '7d' || value === '30d') return value;
  return '24h';
};

export const windowToSince = (window: string | undefined, now = new Date()): { timeWindow: FraudWindow; since: Date } => {
  const timeWindow = clampWindow(window);
  const since = new Date(now.getTime());
  if (timeWindow === '1h') since.setHours(since.getHours() - 1);
  else if (timeWindow === '24h') since.setHours(since.getHours() - 24);
  else if (timeWindow === '7d') since.setDate(since.getDate() - 7);
  else since.setDate(since.getDate() - 30);
  return { timeWindow, since };
};

const safeInt = (n: any): number => {
  const v = typeof n === 'number' ? n : Number(n);
  return Number.isFinite(v) ? Math.max(0, Math.floor(v)) : 0;
};

const safeFloat = (n: any): number => {
  const v = typeof n === 'number' ? n : Number(n);
  return Number.isFinite(v) ? v : 0;
};

const parseJsonish = (value: string): any | null => {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (!(trimmed.startsWith('{') || trimmed.startsWith('['))) return null;
  try {
    return JSON.parse(trimmed);
  } catch {
    return null;
  }
};

const severityToScore = (severity: string): number => {
  if (severity === 'critical') return 0.95;
  if (severity === 'high') return 0.8;
  if (severity === 'medium') return 0.5;
  return 0.2;
};

const normalizeAlertType = (raw: string | null | undefined): FraudAlertV2Row['alertType'] => {
  const v = String(raw || '').toLowerCase();
  if (v === 'givt') return 'givt';
  if (v === 'sivt') return 'sivt';
  if (v === 'ml_fraud' || v === 'ml' || v === 'ml-based' || v === 'ml_based') return 'ml_fraud';
  return 'anomaly';
};

const normalizeSeverity = (raw: string | null | undefined): FraudAlertV2Row['severity'] => {
  const v = String(raw || '').toLowerCase();
  if (v === 'critical') return 'critical';
  if (v === 'high') return 'high';
  if (v === 'medium') return 'medium';
  return 'low';
};

const normalizeDetectionType = (alertType: FraudAlertV2Row['alertType']): FraudAlertV2Row['detectionType'] => {
  if (alertType === 'ml_fraud') return 'ml_based';
  if (alertType === 'anomaly') return 'anomaly';
  return 'rule_based';
};

const normalizeAction = (blocked: boolean | null | undefined, severity: FraudAlertV2Row['severity']): FraudAlertV2Row['action'] => {
  if (blocked) return 'block';
  if (severity === 'high' || severity === 'critical') return 'flag';
  return 'monitor';
};

export const fetchFraudStatsV2 = async (
  publisherId: string,
  opts: { window?: string; now?: Date } = {}
): Promise<FraudStatsV2> => {
  const now = opts.now ?? new Date();
  const { timeWindow, since } = windowToSince(opts.window, now);

  // Prefer analytics tables (publisher_id stored as text) when present.
  try {
    const [{ rows: totalRows }, { rows: fraudRows }, { rows: topTypeRows }] = await Promise.all([
      query<{ total: string | number | null; last: Date | null }>(
        `SELECT COUNT(*) AS total, MAX(observed_at) AS last
         FROM analytics_impressions
         WHERE publisher_id = $1 AND observed_at >= $2`,
        [publisherId, since]
      ),
      query<{
        fraud: string | number | null;
        blocked: string | number | null;
        last: Date | null;
        givt: string | number | null;
        sivt: string | number | null;
        ml: string | number | null;
        anomaly: string | number | null;
      }>(
        `SELECT
           COUNT(*) AS fraud,
           COUNT(*) FILTER (WHERE blocked = true) AS blocked,
           MAX(observed_at) AS last,
           COUNT(*) FILTER (WHERE fraud_type = 'givt') AS givt,
           COUNT(*) FILTER (WHERE fraud_type = 'sivt') AS sivt,
           COUNT(*) FILTER (WHERE fraud_type IN ('ml_fraud','ml_based','ml')) AS ml,
           COUNT(*) FILTER (WHERE fraud_type = 'anomaly') AS anomaly
         FROM analytics_fraud_events
         WHERE publisher_id = $1 AND observed_at >= $2`,
        [publisherId, since]
      ),
      query<{ fraud_type: string | null }>(
        `SELECT fraud_type
         FROM analytics_fraud_events
         WHERE publisher_id = $1 AND observed_at >= $2 AND fraud_type IS NOT NULL
         GROUP BY fraud_type
         ORDER BY COUNT(*) DESC
         LIMIT 10`,
        [publisherId, since]
      ),
    ]);

    const totalRequests = safeInt(totalRows[0]?.total);
    const fraudRequests = safeInt(fraudRows[0]?.fraud);
    const blockedRequests = safeInt(fraudRows[0]?.blocked);
    const flaggedRequests = Math.max(0, fraudRequests - blockedRequests);
    const fraudRate = totalRequests > 0 ? fraudRequests / totalRequests : 0;

    const lastFraud = fraudRows[0]?.last ? fraudRows[0].last.toISOString() : undefined;
    const lastImpressions = totalRows[0]?.last ? totalRows[0].last.toISOString() : undefined;
    const lastUpdated = (lastFraud ?? lastImpressions ?? now.toISOString());

    return {
      publisherId,
      timeWindow,
      totalRequests,
      fraudRequests,
      fraudRate,
      givtDetections: safeInt(fraudRows[0]?.givt),
      sivtDetections: safeInt(fraudRows[0]?.sivt),
      mlDetections: safeInt(fraudRows[0]?.ml),
      anomalyDetections: safeInt(fraudRows[0]?.anomaly),
      blockedRequests,
      flaggedRequests,
      averageFraudScore: 0,
      topFraudTypes: topTypeRows.map((r) => String(r.fraud_type)),
      lastUpdated,
    };
  } catch {
    // fall back to legacy fraud_alerts + revenue_events only
  }

  // Legacy fallback: fraud_requests = fraud_alerts within window; total_requests unknown.
  const { rows: legacyFraudRows } = await query<{ fraud: string | number | null; last: Date | null }>(
    `SELECT COUNT(*) AS fraud, MAX(detected_at) AS last
     FROM fraud_alerts
     WHERE publisher_id = $1 AND detected_at >= $2`,
    [publisherId, since]
  );

  const fraudRequests = safeInt(legacyFraudRows[0]?.fraud);
  const lastUpdated = legacyFraudRows[0]?.last ? legacyFraudRows[0].last.toISOString() : now.toISOString();

  return {
    publisherId,
    timeWindow,
    totalRequests: 0,
    fraudRequests,
    fraudRate: 0,
    givtDetections: 0,
    sivtDetections: 0,
    mlDetections: 0,
    anomalyDetections: 0,
    blockedRequests: 0,
    flaggedRequests: 0,
    averageFraudScore: 0,
    topFraudTypes: [],
    lastUpdated,
  };
};

export const fetchRecentFraudAlertsV2 = async (
  publisherId: string,
  limit: number,
  opts: { window?: string; now?: Date } = {}
): Promise<FraudAlertV2Row[]> => {
  const now = opts.now ?? new Date();
  const { since } = windowToSince(opts.window, now);

  // Prefer analytics table with JSON details.
  try {
    const { rows } = await query<{
      event_id: string;
      observed_at: Date;
      fraud_type: string | null;
      blocked: boolean;
      details: any;
    }>(
      `SELECT event_id, observed_at, fraud_type, blocked, details
       FROM analytics_fraud_events
       WHERE publisher_id = $1 AND observed_at >= $2
       ORDER BY observed_at DESC
       LIMIT $3`,
      [publisherId, since, limit]
    );

    return rows.map((row) => {
      const alertType = normalizeAlertType(row.fraud_type);
      const severity = normalizeSeverity(row.details?.severity ?? row.details?.level ?? (row.blocked ? 'high' : 'medium'));
      const fraudScore = safeFloat(row.details?.fraudScore ?? row.details?.score ?? severityToScore(severity));
      return {
        id: String(row.event_id),
        timestamp: row.observed_at.toISOString(),
        alertType,
        severity,
        publisherId,
        deviceId: row.details?.deviceId ?? row.details?.device_id,
        ipAddress: row.details?.ipAddress ?? row.details?.ip,
        fraudScore,
        detectionType: normalizeDetectionType(alertType),
        action: normalizeAction(Boolean(row.blocked), severity),
        metadata: (row.details && typeof row.details === 'object') ? row.details : {},
      };
    });
  } catch {
    // fall back to legacy fraud_alerts
  }

  const { rows } = await query<{
    id: string;
    type: string;
    severity: string;
    details: string;
    detected_at: Date;
  }>(
    `SELECT id, type, severity, details, detected_at
     FROM fraud_alerts
     WHERE publisher_id = $1 AND detected_at >= $2
     ORDER BY detected_at DESC
     LIMIT $3`,
    [publisherId, since, limit]
  );

  return rows.map((row) => {
    const parsed = parseJsonish(row.details);
    const alertType = normalizeAlertType(row.type);
    const severity = normalizeSeverity(row.severity);
    const fraudScore = safeFloat(parsed?.fraudScore ?? parsed?.score ?? severityToScore(severity));
    const metadata: Record<string, any> = parsed && typeof parsed === 'object' ? parsed : { details: row.details };
    return {
      id: String(row.id),
      timestamp: row.detected_at.toISOString(),
      alertType,
      severity,
      publisherId,
      deviceId: metadata.deviceId ?? metadata.device_id,
      ipAddress: metadata.ipAddress ?? metadata.ip,
      fraudScore,
      detectionType: normalizeDetectionType(alertType),
      action: normalizeAction(undefined, severity),
      metadata,
    };
  });
};

export const fetchFraudStats = async (publisherId: string): Promise<FraudStats> => {
  const { rows } = await query<{
    total_alerts: string | number | null;
    high_severity: string | number | null;
    medium_severity: string | number | null;
    low_severity: string | number | null;
    last_detection_at: Date | null;
  }>(
    `SELECT
        COUNT(*) AS total_alerts,
        COUNT(*) FILTER (WHERE severity = 'high') AS high_severity,
        COUNT(*) FILTER (WHERE severity = 'medium') AS medium_severity,
        COUNT(*) FILTER (WHERE severity = 'low') AS low_severity,
        MAX(detected_at) AS last_detection_at
      FROM fraud_alerts
      WHERE publisher_id = $1`,
    [publisherId]
  );

  const stats = rows[0];

  const blockedRevenueResult = await query<{ blocked: string | number | null }>(
    `SELECT COALESCE(SUM(revenue), 0) AS blocked
      FROM revenue_events
      WHERE publisher_id = $1
        AND impressions = 0
        AND revenue > 0`,
    [publisherId]
  );

  const lastDetection = stats?.last_detection_at
    ? stats.last_detection_at.toISOString()
    : undefined;

  return {
    totalAlerts: toNumber(stats?.total_alerts),
    highSeverity: toNumber(stats?.high_severity),
    mediumSeverity: toNumber(stats?.medium_severity),
    lowSeverity: toNumber(stats?.low_severity),
    blockedRevenue: toNumber(blockedRevenueResult.rows[0]?.blocked),
    lastDetectionAt: lastDetection,
  };
};

export const fetchRecentFraudAlerts = async (
  publisherId: string,
  limit: number
): Promise<FraudAlertRow[]> => {
  const { rows } = await query<{
    id: string;
    type: string;
    severity: string;
    details: string;
    detected_at: Date;
  }>(
    `SELECT id, type, severity, details, detected_at
      FROM fraud_alerts
      WHERE publisher_id = $1
      ORDER BY detected_at DESC
      LIMIT $2`,
    [publisherId, limit]
  );

  return rows.map((row) => ({
    id: String(row.id),
    type: row.type,
    severity: row.severity,
    details: row.details,
    detectedAt: row.detected_at.toISOString(),
  }));
};

export const fetchFraudTypeBreakdown = async (
  publisherId: string
): Promise<FraudTypeBreakdown[]> => {
  const { rows } = await query<{
    type: string;
    alerts: string | number | null;
  }>(
    `SELECT type, COUNT(*) AS alerts
      FROM fraud_alerts
      WHERE publisher_id = $1
      GROUP BY type
      ORDER BY alerts DESC`,
    [publisherId]
  );

  return rows.map((row) => ({
    type: row.type,
    alerts: toNumber(row.alerts),
    blockedRevenue: 0,
  }));
};
