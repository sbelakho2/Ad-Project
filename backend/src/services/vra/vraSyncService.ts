/**
 * VRA Sync Service
 * 
 * Orchestrates syncing revenue data from connected ad networks
 * and running reconciliation jobs.
 * 
 * CONCURRENCY FIX: Uses distributed locking via Redis to prevent race conditions
 * in multi-instance deployments. Without locking, concurrent syncs could cause:
 * - Duplicate revenue inserts (bypassing unique constraints via timing)
 * - Conflicting reconciliation calculations
 * - Inconsistent state across instances
 */

import { Pool } from 'pg';
import logger from '../../utils/logger';
import { NetworkConnectorService, NetworkConnection, NetworkType, ConnectionCredentials } from './networkConnectorService';
import { reconcileWindow, ReconcileParams, ReconcileResult } from './reconcile';
import { withLock, lockKeys, acquireLock, releaseLock } from './vraLock';

export interface SyncResult {
  connectionId: string;
  networkType: NetworkType;
  status: 'success' | 'partial' | 'error' | 'skipped';
  rowsFetched: number;
  rowsInserted: number;
  rowsUpdated: number;
  revenueUsd: number;
  durationMs: number;
  error?: string;
  /** Indicates sync was skipped due to another instance holding the lock */
  lockedBy?: string;
}

export interface ReconciliationResult {
  runId?: string;  // Optional when skipped
  publisherId: string;
  status: 'completed' | 'error' | 'skipped';  // FIX #6: Added 'skipped' for lock contention
  windowStart: string | Date;  // Allow Date for skipped results
  windowEnd: string | Date;    // Allow Date for skipped results
  expectedUsd?: number;        // Optional when skipped
  paidUsd?: number;            // Alias for actualRevenueUsd
  actualRevenueUsd?: number;   // Alias for paidUsd  
  varianceUsd?: number;        // Optional when skipped
  variancePercent?: number;    // Optional when skipped
  discrepancyUsd?: number;     // Alias for varianceUsd
  discrepancyPct?: number;     // Alias for variancePercent
  matchedRows?: number;        // For tracking
  unmatchedRows?: number;      // For tracking
  coveragePercent?: number;    // Optional when skipped
  deltasCount?: number;        // Optional when skipped
  deltasByKind?: Record<string, number>;  // Optional when skipped
  durationMs?: number;         // Optional when skipped
  alerts?: Array<{ type: string; severity: string; message: string }>;  // Optional when skipped
  error?: string;              // Error message when skipped or error status
  expectedRevenueUsd?: number; // Alias for expectedUsd
}

export class VraSyncService {
  private connectorService: NetworkConnectorService;

  constructor(private pool: Pool) {
    this.connectorService = new NetworkConnectorService(pool);
  }

  /**
   * Sync all connections due for update
   * Uses distributed locking to prevent concurrent syncs of the same connection
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
   * Sync a specific connection with distributed locking
   * 
   * FIX #6: Race condition prevention - uses Redis-based distributed lock
   * to prevent concurrent syncs of the same connection across multiple instances.
   */
  async syncConnection(connectionId: string): Promise<SyncResult> {
    const lockKey = lockKeys.syncConnection(connectionId);
    
    // Try to acquire distributed lock (5 min TTL, suitable for sync operations)
    const lock = await acquireLock(lockKey, { ttlSeconds: 300 });
    
    if (!lock) {
      logger.warn('Sync already in progress for connection', { connectionId, lockKey });
      return {
        connectionId,
        networkType: 'unknown' as NetworkType,
        status: 'skipped',
        rowsFetched: 0,
        rowsInserted: 0,
        rowsUpdated: 0,
        revenueUsd: 0,
        durationMs: 0,
        error: 'Sync already in progress (locked by another instance)',
        lockedBy: lockKey,
      };
    }
    
    const startTime = Date.now();
    
    try {
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
    } finally {
      // Always release the lock, even on error
      await releaseLock(lock);
    }
  }

  /**
   * Run reconciliation for a publisher with distributed locking
   * Delegates core financial calculations to reconcileWindow for precision
   * 
   * FIX #6: Race condition prevention - uses Redis-based distributed lock
   * to prevent concurrent reconciliation of the same publisher/window.
   */
  async runReconciliation(
    publisherId: string,
    windowStart: Date,
    windowEnd: Date
  ): Promise<ReconciliationResult> {
    const lockKey = lockKeys.reconciliation(publisherId, windowStart.toISOString().split('T')[0]);
    
    // Try to acquire distributed lock (10 min TTL for reconciliation)
    const lock = await acquireLock(lockKey, { ttlSeconds: 600 });
    
    if (!lock) {
      logger.warn('Reconciliation already in progress', { 
        publisherId, 
        windowStart: windowStart.toISOString(),
        lockKey 
      });
      return {
        publisherId,
        windowStart,
        windowEnd,
        status: 'skipped',
        expectedRevenueUsd: 0,
        actualRevenueUsd: 0,
        discrepancyUsd: 0,
        discrepancyPct: 0,
        matchedRows: 0,
        unmatchedRows: 0,
        error: 'Reconciliation already in progress (locked by another instance)',
      };
    }
    
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
      // Delegate to reconcileWindow for precise financial calculations using Decimal.js
      const reconResult = await reconcileWindow({
        from: windowStart.toISOString(),
        to: windowEnd.toISOString(),
        publisherId,
        dryRun: false, // We want deltas persisted
      });

      // Use reconcileWindow's calculated amounts
      const expectedUsd = reconResult.amounts.expectedUsd;
      const paidUsd = reconResult.amounts.paidUsd;
      const varianceUsd = expectedUsd - paidUsd;
      const variancePercent = expectedUsd > 0 ? (varianceUsd / expectedUsd) * 100 : 0;

      // Get statement count for coverage calculation
      const statementResult = await this.pool.query(
        `SELECT COUNT(*)::int AS statement_count
         FROM recon_statements_norm
         WHERE publisher_id = $1
           AND event_date >= $2::date AND event_date < $3::date`,
        [publisherId, windowStart.toISOString(), windowEnd.toISOString()]
      );
      const statementCount = Number(statementResult.rows[0]?.statement_count || 0);

      // Calculate coverage (how much of expected has a matching statement)
      const coverageResult = await this.pool.query(
        `SELECT COUNT(DISTINCT re.adapter_name)::int AS networks_with_events,
                (SELECT COUNT(DISTINCT network_type) FROM recon_statements_norm 
                 WHERE publisher_id = $1 AND event_date >= $2::date AND event_date < $3::date)::int AS networks_with_statements
         FROM analytics_revenue_events re
         WHERE re.publisher_id = $1 AND re.observed_at >= $2 AND re.observed_at < $3`,
        [publisherId, windowStart.toISOString(), windowEnd.toISOString()]
      );
      const networksWithEvents = Number(coverageResult.rows[0]?.networks_with_events || 0);
      const networksWithStatements = Number(coverageResult.rows[0]?.networks_with_statements || 0);
      const coveragePercent = networksWithEvents > 0 
        ? (networksWithStatements / networksWithEvents) * 100 
        : (statementCount > 0 ? 100 : 0);

      // Build delta summary from reconcileWindow results
      const deltasByKind: Record<string, number> = {};
      const alerts: Array<{ type: string; severity: string; message: string }> = [];

      // Count deltas from reconcileWindow (already persisted)
      if (reconResult.amounts.timingLagUsd > 0) {
        deltasByKind['timing_lag'] = 1;
      }
      if (reconResult.amounts.underpayUsd > 0) {
        deltasByKind['underpay'] = 1;
        // Create alert for significant underpayment
        if (variancePercent > 5) {
          alerts.push({
            type: 'underpay_detected',
            severity: variancePercent > 15 ? 'critical' : 'warning',
            message: `Underpayment detected: $${varianceUsd.toFixed(2)} (${variancePercent.toFixed(1)}% of expected)`,
          });
        }
      }
      const deltasCount = reconResult.deltas;

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
    } finally {
      // Always release the reconciliation lock, even on error
      await releaseLock(lock);
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

  // ========================================================================
  // DATA INTEGRITY FIX: Upsert helper using functional unique index
  // ========================================================================
  // 
  // CRITICAL: Networks often report data with NULL dimensions:
  //   - AdMob: may be missing app_id
  //   - Unity: may be missing ad_unit_id
  //   - AppLovin: may have NULL country
  //
  // PostgreSQL's UNIQUE constraint treats NULL != NULL, so without special
  // handling, identical rows with NULL dimensions would NOT conflict,
  // causing duplicate revenue entries.
  //
  // Solution: Use ON CONFLICT on the functional index that COALESCEs NULLs
  // to empty strings: idx_recon_statements_norm_unique_coalesced
  // ========================================================================

  /**
   * Upsert a statement row using the COALESCED functional unique index
   * Returns true if inserted (new row), false if updated (existing row)
   */
  private async upsertStatement(params: {
    publisherId: string;
    connectionId: string;
    networkType: string;
    eventDate: string;
    appId?: string | null;
    adUnitId?: string | null;
    country?: string | null;
    format?: string | null;
    impressions?: number;
    clicks?: number;
    paid: number;
    originalCurrency: string;
    originalAmount: number;
    gross?: number;
    ivtAdjustments?: number;
    exchangeRate?: number;
    rawData?: any;
  }): Promise<boolean> {
    // Use the functional unique index with COALESCE expressions
    // PostgreSQL ON CONFLICT requires matching the exact expressions in the index
    // Index: idx_recon_statements_norm_unique_coalesced on
    //   (publisher_id, network_type, event_date, COALESCE(app_id, ''), COALESCE(ad_unit_id, ''), 
    //    COALESCE(country, ''), COALESCE(format, ''))
    const result = await this.pool.query(
      `INSERT INTO recon_statements_norm 
       (publisher_id, connection_id, network_type, event_date, app_id, ad_unit_id,
        country, format, impressions, clicks, paid, original_currency, original_amount,
        gross, ivt_adjustments, exchange_rate, raw_data)
       VALUES ($1, $2, $3, $4::date, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
       ON CONFLICT (publisher_id, network_type, event_date, 
                    COALESCE(app_id, ''), COALESCE(ad_unit_id, ''), 
                    COALESCE(country, ''), COALESCE(format, ''))
       DO UPDATE SET 
         impressions = EXCLUDED.impressions,
         clicks = EXCLUDED.clicks,
         paid = EXCLUDED.paid,
         original_amount = EXCLUDED.original_amount,
         gross = COALESCE(EXCLUDED.gross, recon_statements_norm.gross),
         ivt_adjustments = COALESCE(EXCLUDED.ivt_adjustments, recon_statements_norm.ivt_adjustments),
         exchange_rate = COALESCE(EXCLUDED.exchange_rate, recon_statements_norm.exchange_rate),
         raw_data = COALESCE(EXCLUDED.raw_data, recon_statements_norm.raw_data)
       RETURNING (xmax = 0) AS inserted`,
      [
        params.publisherId,
        params.connectionId,
        params.networkType,
        params.eventDate,
        params.appId ?? null,
        params.adUnitId ?? null,
        params.country ?? null,
        params.format ?? null,
        params.impressions ?? 0,
        params.clicks ?? 0,
        params.paid,
        params.originalCurrency,
        params.originalAmount,
        params.gross ?? 0,
        params.ivtAdjustments ?? 0,
        params.exchangeRate ?? 1,
        params.rawData ? JSON.stringify(params.rawData) : null,
      ]
    );

    return result.rows[0]?.inserted === true;
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

      // Use the new upsert helper with COALESCED unique index
      const inserted = await this.upsertStatement({
        publisherId: conn.publisherId,
        connectionId: conn.id,
        networkType: 'admob',
        eventDate,
        appId: null, // AdMob may not provide app_id in this report
        adUnitId,
        country,
        format,
        impressions,
        clicks,
        paid: earnings,
        originalCurrency: 'USD',
        originalAmount: earnings,
      });

      if (inserted) {
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

      // Use the new upsert helper - Unity often missing ad_unit_id
      const inserted = await this.upsertStatement({
        publisherId: conn.publisherId,
        connectionId: conn.id,
        networkType: 'unity',
        eventDate,
        appId,
        adUnitId: null, // Unity doesn't provide ad_unit_id in this report
        country,
        format,
        impressions,
        paid: revenue,
        originalCurrency: 'USD',
        originalAmount: revenue,
      });

      if (inserted) {
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

      // Use the new upsert helper - AppLovin may have NULL dimensions
      const inserted = await this.upsertStatement({
        publisherId: conn.publisherId,
        connectionId: conn.id,
        networkType: 'applovin',
        eventDate,
        appId,
        adUnitId: null, // AppLovin doesn't provide ad_unit_id in this report
        country,
        format,
        impressions,
        clicks,
        paid: revenue,
        originalCurrency: 'USD',
        originalAmount: revenue,
      });

      if (inserted) {
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
