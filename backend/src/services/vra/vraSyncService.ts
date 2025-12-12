/**
 * VRA Sync Service
 * 
 * Orchestrates syncing revenue data from connected ad networks
 * and running reconciliation jobs.
 */

import { Pool } from 'pg';
import logger from '../../utils/logger';
import { NetworkConnectorService, NetworkConnection, NetworkType, ConnectionCredentials } from './networkConnectorService';

export interface SyncResult {
  connectionId: string;
  networkType: NetworkType;
  status: 'success' | 'partial' | 'error';
  rowsFetched: number;
  rowsInserted: number;
  rowsUpdated: number;
  revenueUsd: number;
  durationMs: number;
  error?: string;
}

export interface ReconciliationResult {
  runId: string;
  publisherId: string;
  status: 'completed' | 'error';
  windowStart: string;
  windowEnd: string;
  expectedUsd: number;
  paidUsd: number;
  varianceUsd: number;
  variancePercent: number;
  coveragePercent: number;
  deltasCount: number;
  deltasByKind: Record<string, number>;
  durationMs: number;
  alerts: Array<{ type: string; severity: string; message: string }>;
}

export class VraSyncService {
  private connectorService: NetworkConnectorService;

  constructor(private pool: Pool) {
    this.connectorService = new NetworkConnectorService(pool);
  }

  /**
   * Sync all connections due for update
   */
  async syncAllDue(): Promise<SyncResult[]> {
    const connections = await this.connectorService.getConnectionsDueForSync();
    logger.info(`VRA Sync: Found ${connections.length} connections due for sync`);

    const results: SyncResult[] = [];
    for (const conn of connections) {
      try {
        const result = await this.syncConnection(conn.id);
        results.push(result);
      } catch (err) {
        results.push({
          connectionId: conn.id,
          networkType: conn.networkType,
          status: 'error',
          rowsFetched: 0,
          rowsInserted: 0,
          rowsUpdated: 0,
          revenueUsd: 0,
          durationMs: 0,
          error: (err as Error).message,
        });
      }
    }

    return results;
  }

  /**
   * Sync a specific connection
   */
  async syncConnection(connectionId: string): Promise<SyncResult> {
    const startTime = Date.now();
    const conn = await this.connectorService.getConnection(connectionId);
    
    if (!conn) {
      throw new Error('Connection not found');
    }

    const creds = await this.connectorService.getCredentials(connectionId);
    if (!creds) {
      throw new Error('No credentials found');
    }

    // Record sync start
    const syncHistoryId = await this.recordSyncStart(connectionId, conn.syncLookbackDays || 3);

    try {
      let result: SyncResult;

      switch (conn.networkType) {
        case 'admob':
          result = await this.syncAdMob(conn, creds);
          break;
        case 'unity':
          result = await this.syncUnity(conn, creds);
          break;
        case 'applovin':
          result = await this.syncAppLovin(conn, creds);
          break;
        case 'ironsource':
          result = await this.syncIronSource(conn, creds);
          break;
        case 'meta':
          result = await this.syncMeta(conn, creds);
          break;
        case 'vungle':
          result = await this.syncVungle(conn, creds);
          break;
        default:
          throw new Error(`Unknown network type: ${conn.networkType}`);
      }

      result.durationMs = Date.now() - startTime;

      // Record success
      await this.recordSyncComplete(syncHistoryId, result);
      await this.connectorService.recordSyncSuccess(connectionId);

      logger.info('VRA Sync completed', {
        connectionId,
        network: conn.networkType,
        rowsInserted: result.rowsInserted,
        revenueUsd: result.revenueUsd,
      });

      return result;
    } catch (err) {
      const error = (err as Error).message;
      await this.recordSyncError(syncHistoryId, error);
      await this.connectorService.updateConnectionStatus(connectionId, 'error', error);

      return {
        connectionId,
        networkType: conn.networkType,
        status: 'error',
        rowsFetched: 0,
        rowsInserted: 0,
        rowsUpdated: 0,
        revenueUsd: 0,
        durationMs: Date.now() - startTime,
        error,
      };
    }
  }

  /**
   * Run reconciliation for a publisher
   */
  async runReconciliation(
    publisherId: string,
    windowStart: Date,
    windowEnd: Date
  ): Promise<ReconciliationResult> {
    const startTime = Date.now();

    // Create reconciliation run record
    const runResult = await this.pool.query(
      `INSERT INTO vra_reconciliation_runs (publisher_id, window_start, window_end, status)
       VALUES ($1, $2, $3, 'running')
       RETURNING id`,
      [publisherId, windowStart.toISOString(), windowEnd.toISOString()]
    );
    const runId = runResult.rows[0].id;

    try {
      // 1. Get expected revenue from SDK events
      const expectedResult = await this.pool.query(
        `SELECT COALESCE(SUM(revenue_usd), 0)::float AS expected_usd,
                COUNT(DISTINCT request_id)::int AS event_count
         FROM revenue_events
         WHERE publisher_id = $1
           AND timestamp >= $2 AND timestamp < $3`,
        [publisherId, windowStart.toISOString(), windowEnd.toISOString()]
      );
      const expectedUsd = Number(expectedResult.rows[0]?.expected_usd || 0);
      const eventCount = Number(expectedResult.rows[0]?.event_count || 0);

      // 2. Get paid revenue from network statements
      const paidResult = await this.pool.query(
        `SELECT COALESCE(SUM(paid), 0)::float AS paid_usd,
                COUNT(*)::int AS statement_count
         FROM recon_statements_norm
         WHERE publisher_id = $1
           AND event_date >= $2::date AND event_date < $3::date`,
        [publisherId, windowStart.toISOString(), windowEnd.toISOString()]
      );
      const paidUsd = Number(paidResult.rows[0]?.paid_usd || 0);
      const statementCount = Number(paidResult.rows[0]?.statement_count || 0);

      // 3. Calculate variance
      const varianceUsd = expectedUsd - paidUsd;
      const variancePercent = expectedUsd > 0 ? (varianceUsd / expectedUsd) * 100 : 0;

      // 4. Calculate coverage (how much of expected has a matching statement)
      const coverageResult = await this.pool.query(
        `SELECT COUNT(DISTINCT re.adapter_name)::int AS networks_with_events,
                (SELECT COUNT(DISTINCT network_type) FROM recon_statements_norm 
                 WHERE publisher_id = $1 AND event_date >= $2::date AND event_date < $3::date)::int AS networks_with_statements
         FROM revenue_events re
         WHERE re.publisher_id = $1 AND re.timestamp >= $2 AND re.timestamp < $3`,
        [publisherId, windowStart.toISOString(), windowEnd.toISOString()]
      );
      const networksWithEvents = Number(coverageResult.rows[0]?.networks_with_events || 0);
      const networksWithStatements = Number(coverageResult.rows[0]?.networks_with_statements || 0);
      const coveragePercent = networksWithEvents > 0 
        ? (networksWithStatements / networksWithEvents) * 100 
        : (statementCount > 0 ? 100 : 0);

      // 5. Detect and insert deltas
      const deltasByKind: Record<string, number> = {};
      let deltasCount = 0;
      const alerts: Array<{ type: string; severity: string; message: string }> = [];

      // Check for significant underpayment (> 2% of expected)
      if (varianceUsd > expectedUsd * 0.02 && varianceUsd > 10) {
        const deltaResult = await this.pool.query(
          `INSERT INTO recon_deltas 
           (publisher_id, reconciliation_run_id, kind, amount, currency, reason_code, 
            window_start, window_end, evidence_id, confidence)
           VALUES ($1, $2, 'underpay', $3, 'USD', 'RECONCILIATION_UNDERPAY', $4, $5, $6, 0.8)
           ON CONFLICT (evidence_id) DO UPDATE SET amount = EXCLUDED.amount
           RETURNING id`,
          [publisherId, runId, varianceUsd, windowStart, windowEnd, `recon-${runId}-underpay`]
        );
        deltasByKind['underpay'] = (deltasByKind['underpay'] || 0) + 1;
        deltasCount++;

        // Create alert for significant underpayment
        if (variancePercent > 5) {
          alerts.push({
            type: 'underpay_detected',
            severity: variancePercent > 15 ? 'critical' : 'warning',
            message: `Underpayment detected: $${varianceUsd.toFixed(2)} (${variancePercent.toFixed(1)}% of expected)`,
          });
        }
      }

      // Check for overpayment (negative variance)
      if (varianceUsd < -expectedUsd * 0.02 && Math.abs(varianceUsd) > 10) {
        await this.pool.query(
          `INSERT INTO recon_deltas 
           (publisher_id, reconciliation_run_id, kind, amount, currency, reason_code, 
            window_start, window_end, evidence_id, confidence)
           VALUES ($1, $2, 'overpay', $3, 'USD', 'RECONCILIATION_OVERPAY', $4, $5, $6, 0.8)
           ON CONFLICT (evidence_id) DO UPDATE SET amount = EXCLUDED.amount`,
          [publisherId, runId, Math.abs(varianceUsd), windowStart, windowEnd, `recon-${runId}-overpay`]
        );
        deltasByKind['overpay'] = (deltasByKind['overpay'] || 0) + 1;
        deltasCount++;
      }

      // Check for missing network data
      if (coveragePercent < 80 && networksWithEvents > 0) {
        alerts.push({
          type: 'missing_data',
          severity: 'warning',
          message: `Low reconciliation coverage: ${coveragePercent.toFixed(0)}%. Some networks may not be connected.`,
        });
      }

      // Insert alerts
      for (const alert of alerts) {
        await this.pool.query(
          `INSERT INTO vra_alerts 
           (publisher_id, reconciliation_run_id, alert_type, severity, message)
           VALUES ($1, $2, $3, $4, $5)`,
          [publisherId, runId, alert.type, alert.severity, alert.message]
        );
      }

      // Update run record
      await this.pool.query(
        `UPDATE vra_reconciliation_runs
         SET status = 'completed',
             completed_at = NOW(),
             expected_usd = $2,
             paid_usd = $3,
             variance_usd = $4,
             variance_percent = $5,
             coverage_percent = $6,
             deltas_count = $7,
             deltas_by_kind = $8,
             duration_ms = $9
         WHERE id = $1`,
        [
          runId,
          expectedUsd,
          paidUsd,
          varianceUsd,
          variancePercent,
          coveragePercent,
          deltasCount,
          JSON.stringify(deltasByKind),
          Date.now() - startTime,
        ]
      );

      return {
        runId,
        publisherId,
        status: 'completed',
        windowStart: windowStart.toISOString(),
        windowEnd: windowEnd.toISOString(),
        expectedUsd,
        paidUsd,
        varianceUsd,
        variancePercent,
        coveragePercent,
        deltasCount,
        deltasByKind,
        durationMs: Date.now() - startTime,
        alerts,
      };
    } catch (err) {
      await this.pool.query(
        `UPDATE vra_reconciliation_runs SET status = 'error', completed_at = NOW() WHERE id = $1`,
        [runId]
      );
      throw err;
    }
  }

  /**
   * Run reconciliation for all publishers with active connections
   */
  async runReconciliationForAll(lookbackDays: number = 3): Promise<ReconciliationResult[]> {
    const windowEnd = new Date();
    windowEnd.setHours(0, 0, 0, 0);
    const windowStart = new Date(windowEnd);
    windowStart.setDate(windowStart.getDate() - lookbackDays);

    // Get all publishers with active connections
    const publishersResult = await this.pool.query(
      `SELECT DISTINCT publisher_id FROM vra_network_connections WHERE status = 'active'`
    );

    const results: ReconciliationResult[] = [];
    for (const row of publishersResult.rows) {
      try {
        const result = await this.runReconciliation(row.publisher_id, windowStart, windowEnd);
        results.push(result);
      } catch (err) {
        logger.error('Reconciliation failed for publisher', {
          publisherId: row.publisher_id,
          error: (err as Error).message,
        });
      }
    }

    return results;
  }

  // Network-specific sync implementations

  private async syncAdMob(conn: NetworkConnection, creds: ConnectionCredentials): Promise<SyncResult> {
    const endDate = new Date();
    const startDate = new Date();
    startDate.setDate(startDate.getDate() - (conn.syncLookbackDays || 3));

    if (!creds.accessToken || !creds.accountId) {
      throw new Error('Missing AdMob credentials');
    }

    // Check if token needs refresh
    if (creds.expiresAt && new Date(creds.expiresAt) < new Date()) {
      throw new Error('OAuth token expired - please reconnect');
    }

    const response = await fetch(
      `https://admob.googleapis.com/v1/accounts/${creds.accountId}/networkReport:generate`,
      {
        method: 'POST',
        headers: {
          Authorization: `Bearer ${creds.accessToken}`,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          reportSpec: {
            dateRange: {
              startDate: {
                year: startDate.getFullYear(),
                month: startDate.getMonth() + 1,
                day: startDate.getDate(),
              },
              endDate: {
                year: endDate.getFullYear(),
                month: endDate.getMonth() + 1,
                day: endDate.getDate(),
              },
            },
            dimensions: ['DATE', 'AD_UNIT', 'COUNTRY', 'FORMAT'],
            metrics: ['IMPRESSIONS', 'CLICKS', 'ESTIMATED_EARNINGS'],
            dimensionFilters: [],
            sortConditions: [{ dimension: 'DATE', order: 'ASCENDING' }],
          },
        }),
      }
    );

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`AdMob API error: ${error}`);
    }

    const data = await response.json() as { row?: { dimensionValues?: Record<string, { value?: string }>, metricValues?: Record<string, { integerValue?: string, microsValue?: string }> } }[];
    let rowsFetched = 0;
    let rowsInserted = 0;
    let totalRevenue = 0;

    // Process each row in the report
    for (const row of data || []) {
      if (!row.row) continue;
      rowsFetched++;

      const dims = row.row.dimensionValues || {};
      const metrics = row.row.metricValues || {};

      const eventDate = dims.DATE?.value;
      const adUnitId = dims.AD_UNIT?.value;
      const country = dims.COUNTRY?.value;
      const format = dims.FORMAT?.value;
      const impressions = parseInt(metrics.IMPRESSIONS?.integerValue || '0', 10);
      const clicks = parseInt(metrics.CLICKS?.integerValue || '0', 10);
      const earnings = parseFloat(metrics.ESTIMATED_EARNINGS?.microsValue || '0') / 1000000;

      if (!eventDate) continue;

      totalRevenue += earnings;

      const insertResult = await this.pool.query(
        `INSERT INTO recon_statements_norm 
         (publisher_id, connection_id, network_type, event_date, ad_unit_id, country, format,
          impressions, clicks, paid, original_currency, original_amount)
         VALUES ($1, $2, 'admob', $3::date, $4, $5, $6, $7, $8, $9, 'USD', $9)
         ON CONFLICT (publisher_id, network_type, event_date, app_id, ad_unit_id, country, format)
         DO UPDATE SET impressions = EXCLUDED.impressions, clicks = EXCLUDED.clicks, 
                       paid = EXCLUDED.paid, original_amount = EXCLUDED.original_amount
         RETURNING (xmax = 0) AS inserted`,
        [conn.publisherId, conn.id, eventDate, adUnitId, country, format, impressions, clicks, earnings]
      );

      if (insertResult.rows[0]?.inserted) {
        rowsInserted++;
      }
    }

    return {
      connectionId: conn.id,
      networkType: 'admob',
      status: 'success',
      rowsFetched,
      rowsInserted,
      rowsUpdated: rowsFetched - rowsInserted,
      revenueUsd: totalRevenue,
      durationMs: 0,
    };
  }

  private async syncUnity(conn: NetworkConnection, creds: ConnectionCredentials): Promise<SyncResult> {
    const endDate = new Date();
    const startDate = new Date();
    startDate.setDate(startDate.getDate() - (conn.syncLookbackDays || 3));

    if (!creds.apiKey || !creds.organizationId) {
      throw new Error('Missing Unity credentials');
    }

    const auth = Buffer.from(creds.apiKey + ':').toString('base64');
    const response = await fetch(
      `https://services.api.unity.com/monetize/stats/v1/orgs/${creds.organizationId}/reports/revenue` +
      `?start=${startDate.toISOString().split('T')[0]}&end=${endDate.toISOString().split('T')[0]}` +
      `&groupBy=date,sourceGame,country,adFormat&scale=day`,
      {
        headers: { Authorization: `Basic ${auth}` },
      }
    );

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`Unity API error: ${error}`);
    }

    const data = await response.json() as { results?: { date?: string, sourceGame?: string, country?: string, adFormat?: string, impressions?: number, revenue?: number }[] };
    let rowsFetched = 0;
    let rowsInserted = 0;
    let totalRevenue = 0;

    for (const row of data.results || []) {
      rowsFetched++;

      const eventDate = row.date;
      const appId = row.sourceGame;
      const country = row.country;
      const format = row.adFormat;
      const impressions = row.impressions || 0;
      const revenue = row.revenue || 0;

      if (!eventDate) continue;

      totalRevenue += revenue;

      const insertResult = await this.pool.query(
        `INSERT INTO recon_statements_norm 
         (publisher_id, connection_id, network_type, event_date, app_id, country, format,
          impressions, paid, original_currency, original_amount)
         VALUES ($1, $2, 'unity', $3::date, $4, $5, $6, $7, $8, 'USD', $8)
         ON CONFLICT (publisher_id, network_type, event_date, app_id, ad_unit_id, country, format)
         DO UPDATE SET impressions = EXCLUDED.impressions, paid = EXCLUDED.paid
         RETURNING (xmax = 0) AS inserted`,
        [conn.publisherId, conn.id, eventDate, appId, country, format, impressions, revenue]
      );

      if (insertResult.rows[0]?.inserted) {
        rowsInserted++;
      }
    }

    return {
      connectionId: conn.id,
      networkType: 'unity',
      status: 'success',
      rowsFetched,
      rowsInserted,
      rowsUpdated: rowsFetched - rowsInserted,
      revenueUsd: totalRevenue,
      durationMs: 0,
    };
  }

  private async syncAppLovin(conn: NetworkConnection, creds: ConnectionCredentials): Promise<SyncResult> {
    const endDate = new Date();
    const startDate = new Date();
    startDate.setDate(startDate.getDate() - (conn.syncLookbackDays || 3));

    if (!creds.reportKey) {
      throw new Error('Missing AppLovin report key');
    }

    const response = await fetch(
      `https://r.applovin.com/report?api_key=${creds.reportKey}` +
      `&start=${startDate.toISOString().split('T')[0]}&end=${endDate.toISOString().split('T')[0]}` +
      `&columns=day,application,country,ad_type,impressions,clicks,revenue` +
      `&format=json`,
      { method: 'GET' }
    );

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`AppLovin API error: ${error}`);
    }

    interface AppLovinRow { day?: string, application?: string, country?: string, ad_type?: string, impressions?: string, clicks?: string, revenue?: string }
    const data = await response.json() as { results?: AppLovinRow[] } | AppLovinRow[];
    let rowsFetched = 0;
    let rowsInserted = 0;
    let totalRevenue = 0;

    const rows = Array.isArray(data) ? data : (data.results || []);
    for (const row of rows) {
      rowsFetched++;

      const eventDate = row.day;
      const appId = row.application;
      const country = row.country;
      const format = row.ad_type;
      const impressions = parseInt(row.impressions || '0', 10);
      const clicks = parseInt(row.clicks || '0', 10);
      const revenue = parseFloat(row.revenue || '0');

      if (!eventDate) continue;

      totalRevenue += revenue;

      const insertResult = await this.pool.query(
        `INSERT INTO recon_statements_norm 
         (publisher_id, connection_id, network_type, event_date, app_id, country, format,
          impressions, clicks, paid, original_currency, original_amount)
         VALUES ($1, $2, 'applovin', $3::date, $4, $5, $6, $7, $8, $9, 'USD', $9)
         ON CONFLICT (publisher_id, network_type, event_date, app_id, ad_unit_id, country, format)
         DO UPDATE SET impressions = EXCLUDED.impressions, clicks = EXCLUDED.clicks, paid = EXCLUDED.paid
         RETURNING (xmax = 0) AS inserted`,
        [conn.publisherId, conn.id, eventDate, appId, country, format, impressions, clicks, revenue]
      );

      if (insertResult.rows[0]?.inserted) {
        rowsInserted++;
      }
    }

    return {
      connectionId: conn.id,
      networkType: 'applovin',
      status: 'success',
      rowsFetched,
      rowsInserted,
      rowsUpdated: rowsFetched - rowsInserted,
      revenueUsd: totalRevenue,
      durationMs: 0,
    };
  }

  private async syncIronSource(conn: NetworkConnection, creds: ConnectionCredentials): Promise<SyncResult> {
    // IronSource implementation would go here
    // Requires OAuth token refresh flow
    return {
      connectionId: conn.id,
      networkType: 'ironsource',
      status: 'success',
      rowsFetched: 0,
      rowsInserted: 0,
      rowsUpdated: 0,
      revenueUsd: 0,
      durationMs: 0,
    };
  }

  private async syncMeta(conn: NetworkConnection, creds: ConnectionCredentials): Promise<SyncResult> {
    // Meta Audience Network implementation would go here
    return {
      connectionId: conn.id,
      networkType: 'meta',
      status: 'success',
      rowsFetched: 0,
      rowsInserted: 0,
      rowsUpdated: 0,
      revenueUsd: 0,
      durationMs: 0,
    };
  }

  private async syncVungle(conn: NetworkConnection, creds: ConnectionCredentials): Promise<SyncResult> {
    // Vungle implementation would go here
    return {
      connectionId: conn.id,
      networkType: 'vungle',
      status: 'success',
      rowsFetched: 0,
      rowsInserted: 0,
      rowsUpdated: 0,
      revenueUsd: 0,
      durationMs: 0,
    };
  }

  // Helper methods for sync history

  private async recordSyncStart(connectionId: string, lookbackDays: number): Promise<string> {
    const syncFrom = new Date();
    syncFrom.setDate(syncFrom.getDate() - lookbackDays);
    const syncTo = new Date();

    const result = await this.pool.query(
      `INSERT INTO vra_sync_history (connection_id, sync_from, sync_to, status)
       VALUES ($1, $2, $3, 'running')
       RETURNING id`,
      [connectionId, syncFrom.toISOString().split('T')[0], syncTo.toISOString().split('T')[0]]
    );

    return result.rows[0].id;
  }

  private async recordSyncComplete(syncHistoryId: string, result: SyncResult): Promise<void> {
    await this.pool.query(
      `UPDATE vra_sync_history
       SET status = $2, completed_at = NOW(), rows_fetched = $3, rows_inserted = $4, 
           rows_updated = $5, revenue_usd_total = $6, duration_ms = $7
       WHERE id = $1`,
      [
        syncHistoryId,
        result.status,
        result.rowsFetched,
        result.rowsInserted,
        result.rowsUpdated,
        result.revenueUsd,
        result.durationMs,
      ]
    );
  }

  private async recordSyncError(syncHistoryId: string, error: string): Promise<void> {
    await this.pool.query(
      `UPDATE vra_sync_history
       SET status = 'error', completed_at = NOW(), error_message = $2
       WHERE id = $1`,
      [syncHistoryId, error]
    );
  }

  /**
   * Get alerts for a publisher
   */
  async getAlerts(publisherId: string, status: string = 'open', limit: number = 50): Promise<any[]> {
    const result = await this.pool.query(
      `SELECT id, alert_type, severity, network_type, amount_usd, message, 
              status, created_at, acknowledged_at, resolved_at
       FROM vra_alerts
       WHERE publisher_id = $1 AND status = $2
       ORDER BY 
         CASE severity WHEN 'critical' THEN 0 WHEN 'warning' THEN 1 ELSE 2 END,
         created_at DESC
       LIMIT $3`,
      [publisherId, status, limit]
    );

    return result.rows.map(row => ({
      id: row.id,
      alertType: row.alert_type,
      severity: row.severity,
      networkType: row.network_type,
      amountUsd: parseFloat(row.amount_usd || 0),
      message: row.message,
      status: row.status,
      createdAt: row.created_at,
      acknowledgedAt: row.acknowledged_at,
      resolvedAt: row.resolved_at,
    }));
  }

  /**
   * Acknowledge an alert
   */
  async acknowledgeAlert(alertId: string, publisherId: string): Promise<boolean> {
    const result = await this.pool.query(
      `UPDATE vra_alerts 
       SET status = 'acknowledged', acknowledged_at = NOW()
       WHERE id = $1 AND publisher_id = $2 AND status = 'open'`,
      [alertId, publisherId]
    );
    return (result.rowCount || 0) > 0;
  }

  /**
   * Resolve an alert
   */
  async resolveAlert(alertId: string, publisherId: string): Promise<boolean> {
    const result = await this.pool.query(
      `UPDATE vra_alerts 
       SET status = 'resolved', resolved_at = NOW()
       WHERE id = $1 AND publisher_id = $2 AND status IN ('open', 'acknowledged')`,
      [alertId, publisherId]
    );
    return (result.rowCount || 0) > 0;
  }
}

export const createVraSyncService = (pool: Pool) => new VraSyncService(pool);
