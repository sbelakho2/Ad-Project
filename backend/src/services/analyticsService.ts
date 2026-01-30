/**
 * Analytics Service
 *
 * Handles ingestion of ad events into Postgres analytics fact tables
 */

import { insertMany, query } from '../utils/postgres';
import { analyticsIngestBufferGauge } from '../utils/prometheus';
import logger from '../utils/logger';
import {
  ImpressionEventDTO,
  ClickEventDTO,
  RevenueEventDTO,
} from '../types/analytics.types';

type AnalyticsServiceOptions = {
  batchSize?: number;
  flushIntervalMs?: number;
};

const IMPRESSION_COLUMNS = [
  'event_id',
  'observed_at',
  'publisher_id',
  'app_id',
  'placement_id',
  'adapter_id',
  'adapter_name',
  'ad_unit_id',
  'ad_format',
  'country_code',
  'device_type',
  'os',
  'os_version',
  'session_id',
  'user_id',
  'request_id',
  'status',
  'filled',
  'latency_ms',
  'meta',
  'is_test_mode',
] as const;

const CLICK_COLUMNS = [
  'event_id',
  'observed_at',
  'impression_id',
  'publisher_id',
  'app_id',
  'placement_id',
  'adapter_id',
  'adapter_name',
  'click_url',
  'country_code',
  'device_type',
  'os',
  'session_id',
  'user_id',
  'request_id',
  'time_to_click_ms',
  'is_verified',
  'meta',
  'is_test_mode',
] as const;

const REVENUE_COLUMNS = [
  'event_id',
  'observed_at',
  'publisher_id',
  'app_id',
  'placement_id',
  'adapter_id',
  'adapter_name',
  'impression_id',
  'revenue_type',
  'revenue_usd',
  'revenue_currency',
  'revenue_original',
  'exchange_rate',
  'ecpm_usd',
  'country_code',
  'ad_format',
  'os',
  'is_test_mode',
  // 'reconciliation_status',
  'metadata',
] as const;

const asDate = (value: string): Date => new Date(value);
const quoteIdentifier = (identifier: string): string => `"${identifier.replace(/"/g, '""')}"`;
const STAGE_TABLE = {
  impressions: 'analytics_impressions_stage',
  clicks: 'analytics_clicks_stage',
  revenue: 'analytics_revenue_events_stage',
} as const;

const toImpressionRow = (event: ImpressionEventDTO): unknown[] => [
  event.event_id,
  asDate(event.timestamp),
  event.publisher_id,
  event.app_id,
  event.placement_id,
  event.adapter_id,
  event.adapter_name,
  event.ad_unit_id,
  event.ad_format,
  event.country_code,
  event.device_type,
  event.os,
  event.os_version,
  event.session_id,
  event.user_id,
  event.request_id,
  'success',
  true,
  event.latency_ms,
  {
    app_version: event.app_version,
    sdk_version: event.sdk_version,
    bid_price_usd: event.bid_price_usd,
    ecpm_usd: event.ecpm_usd,
    ad_unit_id: event.ad_unit_id,
    ad_format: event.ad_format,
    country_code: event.country_code,
    device_type: event.device_type,
    os: event.os,
    os_version: event.os_version,
    session_id: event.session_id,
    user_id: event.user_id,
  },
  event.is_test_mode,
];

const toClickRow = (event: ClickEventDTO): unknown[] => [
  event.event_id,
  asDate(event.timestamp),
  event.impression_id,
  event.publisher_id,
  event.app_id,
  event.placement_id,
  event.adapter_id,
  event.adapter_name,
  event.click_url,
  event.country_code,
  event.device_type,
  event.os,
  event.session_id,
  event.user_id,
  event.request_id,
  event.time_to_click_ms,
  event.is_verified,
  {
    request_id: event.request_id,
  },
  event.is_test_mode,
];

const toRevenueRow = (event: RevenueEventDTO): unknown[] => [
  event.event_id,
  asDate(event.timestamp),
  event.publisher_id,
  event.app_id,
  event.placement_id,
  event.adapter_id,
  event.adapter_name,
  event.impression_id,
  event.revenue_type,
  event.revenue_usd,
  event.revenue_currency,
  event.revenue_original,
  event.exchange_rate,
  event.ecpm_usd,
  event.country_code,
  event.ad_format,
  event.os,
  event.is_test_mode,
  // event.reconciliation_status,
  {
    reconciliation_status: event.reconciliation_status,
  },
];

export class AnalyticsService {
  private impressionBuffer: ImpressionEventDTO[] = [];
  private clickBuffer: ClickEventDTO[] = [];
  private revenueBuffer: RevenueEventDTO[] = [];

  private flushingImpressions = false;
  private flushingClicks = false;
  private flushingRevenue = false;

  private columnCache = new Map<string, ReadonlySet<string>>();
  
  private readonly BATCH_SIZE: number;
  private readonly FLUSH_INTERVAL_MS: number;
  // Maximum buffer size to prevent unbounded memory growth
  // Events are dropped (oldest first) if buffer exceeds this limit
  private readonly MAX_BUFFER_SIZE: number;
  private flushInterval: NodeJS.Timeout | null = null;

  // Track dropped events for monitoring
  private droppedImpressions = 0;
  private droppedClicks = 0;
  private droppedRevenue = 0;

  private uniqueIndexColumnsCache = new Map<string, ReadonlyArray<ReadonlyArray<string>>>();

  constructor(options: AnalyticsServiceOptions = {}) {
    this.BATCH_SIZE = options.batchSize ?? 100;
    this.FLUSH_INTERVAL_MS = options.flushIntervalMs ?? 5000;
    // Default max buffer: 10,000 events per type
    // At ~500 bytes/event, this is ~15MB total (3 buffers * 5MB each)
    this.MAX_BUFFER_SIZE = Number(process.env.ANALYTICS_MAX_BUFFER_SIZE) || 10000;

    if (this.FLUSH_INTERVAL_MS > 0) {
      const interval = setInterval(() => {
        void this.flushAll();
      }, this.FLUSH_INTERVAL_MS);

      interval.unref?.();
      this.flushInterval = interval;
    }
  }

  /**
   * Enforce max buffer size by dropping oldest events for a specific buffer
   * Uses the MAX_BUFFER_SIZE class property
   */
  private enforceBufferLimit(bufferType: 'impressions' | 'clicks' | 'revenue'): void {
    let buffer: unknown[];
    let droppedCounter: 'droppedImpressions' | 'droppedClicks' | 'droppedRevenue';
    
    switch (bufferType) {
      case 'impressions':
        buffer = this.impressionBuffer;
        droppedCounter = 'droppedImpressions';
        break;
      case 'clicks':
        buffer = this.clickBuffer;
        droppedCounter = 'droppedClicks';
        break;
      case 'revenue':
        buffer = this.revenueBuffer;
        droppedCounter = 'droppedRevenue';
        break;
    }
    
    if (buffer.length <= this.MAX_BUFFER_SIZE) return;
    
    const overflow = buffer.length - this.MAX_BUFFER_SIZE;
    // Reassign buffer to the sliced version (keeps newest events)
    switch (bufferType) {
      case 'impressions':
        this.impressionBuffer = this.impressionBuffer.slice(overflow);
        break;
      case 'clicks':
        this.clickBuffer = this.clickBuffer.slice(overflow);
        break;
      case 'revenue':
        this.revenueBuffer = this.revenueBuffer.slice(overflow);
        break;
    }
    
    this[droppedCounter] += overflow;
    
    logger.warn('[AnalyticsService] Buffer overflow, dropped oldest events', {
      eventType: bufferType,
      dropped: overflow,
      totalDropped: this[droppedCounter],
      maxSize: this.MAX_BUFFER_SIZE,
    });
  }

  private async getTableColumns(tableName: string): Promise<ReadonlySet<string>> {
    const cached = this.columnCache.get(tableName);
    if (cached) return cached;

    const result = await query(
      `SELECT column_name
       FROM information_schema.columns
       WHERE table_schema = 'public'
         AND table_name = $1`,
      [tableName]
    );
    const cols = new Set<string>();
    for (const row of (result.rows ?? []) as any[]) {
      const name = row?.column_name;
      if (typeof name === 'string' && name.length > 0) cols.add(name);
    }
    this.columnCache.set(tableName, cols);
    return cols;
  }

  private pickExistingColumns<T extends readonly string[]>(
    desired: T,
    existing: ReadonlySet<string>
  ): string[] {
    return desired.filter((c) => existing.has(c));
  }

  private buildRow(columns: ReadonlyArray<string>, values: Record<string, unknown>): unknown[] {
    return columns.map((c) => values[c]);
  }

  /**
   * Record an impression event
   */
  async recordImpression(event: ImpressionEventDTO): Promise<void> {
    // Enforce buffer limit to prevent memory leaks during DB outages
    this.enforceBufferLimit('impressions');
    
    this.impressionBuffer.push(event);
    this.updateBufferGauge('impressions');
    
    if (this.impressionBuffer.length >= this.BATCH_SIZE) {
      await this.flushImpressions();
    }
  }

  /**
   * Record impression events in batch
   */
  async recordImpressions(events: ImpressionEventDTO[]): Promise<void> {
    // Enforce buffer limit to prevent memory leaks during DB outages
    this.enforceBufferLimit('impressions');
    
    for (const ev of events) this.impressionBuffer.push(ev);
    this.updateBufferGauge('impressions');
    
    if (this.impressionBuffer.length >= this.BATCH_SIZE) {
      await this.flushImpressions();
    }
  }

  /**
   * Record a click event
   */
  async recordClick(event: ClickEventDTO): Promise<void> {
    // Enforce buffer limit to prevent memory leaks during DB outages
    this.enforceBufferLimit('clicks');
    
    this.clickBuffer.push(event);
    this.updateBufferGauge('clicks');
    
    if (this.clickBuffer.length >= this.BATCH_SIZE) {
      await this.flushClicks();
    }
  }

  /**
   * Record click events in batch
   */
  async recordClicks(events: ClickEventDTO[]): Promise<void> {
    // Enforce buffer limit to prevent memory leaks during DB outages
    this.enforceBufferLimit('clicks');
    
    for (const ev of events) this.clickBuffer.push(ev);
    this.updateBufferGauge('clicks');
    
    if (this.clickBuffer.length >= this.BATCH_SIZE) {
      await this.flushClicks();
    }
  }

  /**
   * Record a revenue event
   */
  async recordRevenue(event: RevenueEventDTO): Promise<void> {
    // Enforce buffer limit to prevent memory leaks during DB outages
    this.enforceBufferLimit('revenue');
    
    this.revenueBuffer.push(event);
    this.updateBufferGauge('revenue');
    
    if (this.revenueBuffer.length >= this.BATCH_SIZE) {
      await this.flushRevenue();
    }
  }

  /**
   * Record revenue events in batch
   */
  async recordRevenueEvents(events: RevenueEventDTO[]): Promise<void> {
    // Enforce buffer limit to prevent memory leaks during DB outages
    this.enforceBufferLimit('revenue');
    
    for (const ev of events) this.revenueBuffer.push(ev);
    this.updateBufferGauge('revenue');
    
    if (this.revenueBuffer.length >= this.BATCH_SIZE) {
      await this.flushRevenue();
    }
  }

  /**
   * Flush impression buffer to Postgres
   */
  private async flushImpressions(): Promise<void> {
    if (this.flushingImpressions) return;
    this.flushingImpressions = true;

    const stageTable = STAGE_TABLE.impressions;
    const targetTable = 'analytics_impressions';

    let eventsToFlush: ImpressionEventDTO[] = [];

    let staged = false;
    try {
      const stageCols = await this.getTableColumns(stageTable);
      const targetCols = await this.getTableColumns(targetTable);

      const stageInsertColumns = stageCols.size > 0
        ? this.pickExistingColumns(IMPRESSION_COLUMNS, new Set([...stageCols].filter((c) => targetCols.has(c))))
        : [];

      // If the in-memory buffer is empty, still attempt to drain any staged rows.
      if (this.impressionBuffer.length === 0) {
        if (stageInsertColumns.length > 0) {
          const hasStageRows = await query(`SELECT 1 FROM ${quoteIdentifier(stageTable)} LIMIT 1`);
          if ((hasStageRows.rows ?? []).length > 0) {
            const mergeColumns = stageInsertColumns.filter((c) => targetCols.has(c));
            if (mergeColumns.length > 0) {
              await this.mergeStageIntoTargetByNotExists(stageTable, targetTable, mergeColumns, ['observed_at', 'event_id']);
            }
          }
        }
        return;
      }

      eventsToFlush = [...this.impressionBuffer];
      this.impressionBuffer = [];
      this.updateBufferGauge('impressions');

      const valuesByColumn = (event: ImpressionEventDTO): Record<string, unknown> => ({
        event_id: event.event_id,
        observed_at: asDate(event.timestamp),
        publisher_id: event.publisher_id,
        app_id: event.app_id,
        placement_id: event.placement_id,
        adapter_id: event.adapter_id,
        adapter_name: event.adapter_name,
        ad_unit_id: event.ad_unit_id,
        ad_format: event.ad_format,
        country_code: event.country_code,
        device_type: event.device_type,
        os: event.os,
        os_version: event.os_version,
        session_id: event.session_id,
        user_id: event.user_id,
        request_id: event.request_id,
        status: 'success',
        filled: true,
        latency_ms: event.latency_ms,
        meta: {
          app_version: event.app_version,
          sdk_version: event.sdk_version,
          bid_price_usd: event.bid_price_usd,
          ecpm_usd: event.ecpm_usd,
          ad_unit_id: event.ad_unit_id,
          ad_format: event.ad_format,
          country_code: event.country_code,
          device_type: event.device_type,
          os: event.os,
          os_version: event.os_version,
          session_id: event.session_id,
          user_id: event.user_id,
          request_id: event.request_id,
        },
        is_test_mode: event.is_test_mode,
      });

      if (stageInsertColumns.length > 0) {
        const rows = eventsToFlush.map((ev) => this.buildRow(stageInsertColumns, valuesByColumn(ev)));
        await insertMany(stageTable, stageInsertColumns, rows);

        // At this point events are persisted in stage. If the merge fails, do not re-buffer
        // (re-buffering would cause duplicate stage rows and potentially huge buffers).
        staged = true;

        const mergeColumns = stageInsertColumns.filter((c) => targetCols.has(c));
        if (mergeColumns.length === 0) {
          throw new Error(`No overlapping columns between ${stageTable} and ${targetTable}`);
        }
        await this.mergeStageIntoTargetByNotExists(stageTable, targetTable, mergeColumns, ['observed_at', 'event_id']);
      } else {
        const targetInsertColumns = this.pickExistingColumns(IMPRESSION_COLUMNS, targetCols);
        if (targetInsertColumns.length === 0) {
          throw new Error(`Target table ${targetTable} has none of the expected columns for impressions`);
        }
        const rows = eventsToFlush.map((ev) => this.buildRow(targetInsertColumns, valuesByColumn(ev)));
        await insertMany(targetTable, targetInsertColumns, rows);
      }
      logger.debug('Flushed impressions', { count: eventsToFlush.length });
    } catch (error) {
      logger.warn('Failed to flush impressions (retrying)', {
        count: eventsToFlush.length,
        error: error instanceof Error ? error.message : String(error),
      });

      if (!staged) {
        // Re-add to buffer for retry, but respect MAX_BUFFER_SIZE to prevent OOM
        const combined = eventsToFlush.concat(this.impressionBuffer);
        if (combined.length > this.MAX_BUFFER_SIZE) {
          const dropped = combined.length - this.MAX_BUFFER_SIZE;
          this.droppedImpressions += dropped;
          logger.warn('Dropping oldest impressions due to buffer limit on re-buffer', {
            dropped,
            totalDropped: this.droppedImpressions,
            maxBufferSize: this.MAX_BUFFER_SIZE,
          });
          this.impressionBuffer = combined.slice(-this.MAX_BUFFER_SIZE);
        } else {
          this.impressionBuffer = combined;
        }
        this.updateBufferGauge('impressions');
      }
    } finally {
      this.flushingImpressions = false;
    }
  }

  /**
   * Flush click buffer to Postgres
   */
  private async flushClicks(): Promise<void> {
    if (this.flushingClicks) return;
    this.flushingClicks = true;

    const stageTable = STAGE_TABLE.clicks;
    const targetTable = 'analytics_clicks';

    let eventsToFlush: ClickEventDTO[] = [];

    let staged = false;
    try {
      const stageCols = await this.getTableColumns(stageTable);
      const targetCols = await this.getTableColumns(targetTable);

      const stageInsertColumns = stageCols.size > 0
        ? this.pickExistingColumns(CLICK_COLUMNS, new Set([...stageCols].filter((c) => targetCols.has(c))))
        : [];

      // If the in-memory buffer is empty, still attempt to drain any staged rows.
      if (this.clickBuffer.length === 0) {
        if (stageInsertColumns.length > 0) {
          const hasStageRows = await query(`SELECT 1 FROM ${quoteIdentifier(stageTable)} LIMIT 1`);
          if ((hasStageRows.rows ?? []).length > 0) {
            const mergeColumns = stageInsertColumns.filter((c) => targetCols.has(c));
            if (mergeColumns.length > 0) {
              await this.mergeStageIntoTargetByNotExists(stageTable, targetTable, mergeColumns, ['observed_at', 'event_id']);
            }
          }
        }
        return;
      }

      eventsToFlush = [...this.clickBuffer];
      this.clickBuffer = [];
      this.updateBufferGauge('clicks');

      const valuesByColumn = (event: ClickEventDTO): Record<string, unknown> => ({
        event_id: event.event_id,
        observed_at: asDate(event.timestamp),
        impression_id: event.impression_id,
        publisher_id: event.publisher_id,
        app_id: event.app_id,
        placement_id: event.placement_id,
        adapter_id: event.adapter_id,
        adapter_name: event.adapter_name,
        click_url: event.click_url,
        country_code: event.country_code,
        device_type: event.device_type,
        os: event.os,
        session_id: event.session_id,
        user_id: event.user_id,
        request_id: event.request_id,
        time_to_click_ms: event.time_to_click_ms,
        is_verified: event.is_verified,
        meta: {
          request_id: event.request_id,
        },
        is_test_mode: event.is_test_mode,
      });

      if (stageInsertColumns.length > 0) {
        const rows = eventsToFlush.map((ev) => this.buildRow(stageInsertColumns, valuesByColumn(ev)));
        await insertMany(stageTable, stageInsertColumns, rows);

        // Events persisted in stage; do not re-buffer if merge fails.
        staged = true;
        const mergeColumns = stageInsertColumns.filter((c) => targetCols.has(c));
        if (mergeColumns.length === 0) {
          throw new Error(`No overlapping columns between ${stageTable} and ${targetTable}`);
        }
        await this.mergeStageIntoTargetByNotExists(stageTable, targetTable, mergeColumns, ['observed_at', 'event_id']);
      } else {
        const targetInsertColumns = this.pickExistingColumns(CLICK_COLUMNS, targetCols);
        if (targetInsertColumns.length === 0) {
          throw new Error(`Target table ${targetTable} has none of the expected columns for clicks`);
        }
        const rows = eventsToFlush.map((ev) => this.buildRow(targetInsertColumns, valuesByColumn(ev)));
        await insertMany(targetTable, targetInsertColumns, rows);
      }

      logger.debug('Flushed clicks', { count: eventsToFlush.length });
    } catch (error) {
      logger.warn('Failed to flush clicks (retrying)', {
        count: eventsToFlush.length,
        error: error instanceof Error ? error.message : String(error),
      });

      if (!staged) {
        // Re-add to buffer for retry, but respect MAX_BUFFER_SIZE to prevent OOM
        const combined = eventsToFlush.concat(this.clickBuffer);
        if (combined.length > this.MAX_BUFFER_SIZE) {
          const dropped = combined.length - this.MAX_BUFFER_SIZE;
          this.droppedClicks += dropped;
          logger.warn('Dropping oldest clicks due to buffer limit on re-buffer', {
            dropped,
            totalDropped: this.droppedClicks,
            maxBufferSize: this.MAX_BUFFER_SIZE,
          });
          this.clickBuffer = combined.slice(-this.MAX_BUFFER_SIZE);
        } else {
          this.clickBuffer = combined;
        }
        this.updateBufferGauge('clicks');
      }
    } finally {
      this.flushingClicks = false;
    }
  }

  /**
   * Flush revenue buffer to Postgres
   */
  private async flushRevenue(): Promise<void> {
    if (this.flushingRevenue) return;
    this.flushingRevenue = true;

    const stageTable = STAGE_TABLE.revenue;
    const targetTable = 'analytics_revenue_events';

    let eventsToFlush: RevenueEventDTO[] = [];

    let staged = false;
    try {
      const stageCols = await this.getTableColumns(stageTable);
      const targetCols = await this.getTableColumns(targetTable);

      const stageInsertColumns = stageCols.size > 0
        ? this.pickExistingColumns(REVENUE_COLUMNS, new Set([...stageCols].filter((c) => targetCols.has(c))))
        : [];

      // If the in-memory buffer is empty, still attempt to drain any staged rows.
      if (this.revenueBuffer.length === 0) {
        if (stageInsertColumns.length > 0) {
          const hasStageRows = await query(`SELECT 1 FROM ${quoteIdentifier(stageTable)} LIMIT 1`);
          if ((hasStageRows.rows ?? []).length > 0) {
            const mergeColumns = stageInsertColumns.filter((c) => targetCols.has(c));
            if (mergeColumns.length > 0) {
              await this.mergeStageIntoTargetByNotExists(stageTable, targetTable, mergeColumns, ['observed_at', 'event_id']);
            }
          }
        }
        return;
      }

      eventsToFlush = [...this.revenueBuffer];
      this.revenueBuffer = [];
      this.updateBufferGauge('revenue');

      const valuesByColumn = (event: RevenueEventDTO): Record<string, unknown> => ({
        event_id: event.event_id,
        observed_at: asDate(event.timestamp),
        publisher_id: event.publisher_id,
        app_id: event.app_id,
        placement_id: event.placement_id,
        adapter_id: event.adapter_id,
        adapter_name: event.adapter_name,
        impression_id: event.impression_id,
        revenue_type: event.revenue_type,
        revenue_usd: event.revenue_usd,
        revenue_currency: event.revenue_currency,
        revenue_original: event.revenue_original,
        exchange_rate: event.exchange_rate,
        ecpm_usd: event.ecpm_usd,
        country_code: event.country_code,
        ad_format: event.ad_format,
        os: event.os,
        is_test_mode: event.is_test_mode,
        metadata: {
          reconciliation_status: event.reconciliation_status,
        },
      });

      if (stageInsertColumns.length > 0) {
        const rows = eventsToFlush.map((ev) => this.buildRow(stageInsertColumns, valuesByColumn(ev)));
        await insertMany(stageTable, stageInsertColumns, rows);

        // Events persisted in stage; do not re-buffer if merge fails.
        staged = true;
        const mergeColumns = stageInsertColumns.filter((c) => targetCols.has(c));
        if (mergeColumns.length === 0) {
          throw new Error(`No overlapping columns between ${stageTable} and ${targetTable}`);
        }
        await this.mergeStageIntoTargetByNotExists(stageTable, targetTable, mergeColumns, ['observed_at', 'event_id']);
      } else {
        const targetInsertColumns = this.pickExistingColumns(REVENUE_COLUMNS, targetCols);
        if (targetInsertColumns.length === 0) {
          throw new Error(`Target table ${targetTable} has none of the expected columns for revenue`);
        }
        const rows = eventsToFlush.map((ev) => this.buildRow(targetInsertColumns, valuesByColumn(ev)));
        await insertMany(targetTable, targetInsertColumns, rows);
      }
      logger.debug('Flushed revenue events', { count: eventsToFlush.length });
    } catch (error) {
      logger.warn('Failed to flush revenue events (retrying)', {
        count: eventsToFlush.length,
        error: error instanceof Error ? error.message : String(error),
      });

      if (!staged) {
        // Re-add to buffer for retry, but respect MAX_BUFFER_SIZE to prevent OOM
        const combined = eventsToFlush.concat(this.revenueBuffer);
        if (combined.length > this.MAX_BUFFER_SIZE) {
          const dropped = combined.length - this.MAX_BUFFER_SIZE;
          this.droppedRevenue += dropped;
          logger.warn('Dropping oldest revenue events due to buffer limit on re-buffer', {
            dropped,
            totalDropped: this.droppedRevenue,
            maxBufferSize: this.MAX_BUFFER_SIZE,
          });
          this.revenueBuffer = combined.slice(-this.MAX_BUFFER_SIZE);
        } else {
          this.revenueBuffer = combined;
        }
        this.updateBufferGauge('revenue');
      }
    } finally {
      this.flushingRevenue = false;
    }
  }

  /**
   * Flush all buffers
   */
  async flushAll(): Promise<void> {
    await Promise.all([
      this.flushImpressions(),
      this.flushClicks(),
      this.flushRevenue(),
    ]);
  }

  /**
   * Cleanup interval for graceful shutdown (primarily for tests)
   */
  shutdown(): void {
    if (this.flushInterval) {
      clearInterval(this.flushInterval);
      this.flushInterval = null;
    }
  }

  /**
   * Get buffer stats (for monitoring)
   */
  getBufferStats() {
    return {
      impressions: this.impressionBuffer.length,
      clicks: this.clickBuffer.length,
      revenue: this.revenueBuffer.length,
      total: this.impressionBuffer.length + this.clickBuffer.length + this.revenueBuffer.length,
      maxBufferSize: this.MAX_BUFFER_SIZE,
      droppedImpressions: this.droppedImpressions,
      droppedClicks: this.droppedClicks,
      droppedRevenue: this.droppedRevenue,
      totalDropped: this.droppedImpressions + this.droppedClicks + this.droppedRevenue,
    };
  }

  private async mergeStageIntoTargetByNotExists(
    stageTable: string,
    targetTable: string,
    columns: ReadonlyArray<string>,
    conflictColumns: ReadonlyArray<string>
  ): Promise<void> {
    const columnSql = columns.map(quoteIdentifier).join(', ');

    const targetCols = await this.getTableColumns(targetTable);
    const stageCols = await this.getTableColumns(stageTable);
    const usableConflictCols = conflictColumns.filter((c) => targetCols.has(c) && stageCols.has(c));

    const whereSql = usableConflictCols.length > 0
      ? `WHERE NOT EXISTS (
          SELECT 1
          FROM ${quoteIdentifier(targetTable)} t
          WHERE ${usableConflictCols
            .map((c) => `t.${quoteIdentifier(c)} = s.${quoteIdentifier(c)}`)
            .join(' AND ')}
        )`
      : '';

    // Use ON CONFLICT DO NOTHING to handle race conditions where another
    // flush inserted the same event_id between the NOT EXISTS check and INSERT.
    // We infer the conflict target from the actual unique indexes on the target
    // table to avoid mismatches (especially with partitioned tables).
    const onConflictSql = await this.buildOnConflictSql(targetTable, usableConflictCols);

    // Wrap INSERT + TRUNCATE in a transaction to prevent race conditions
    // where concurrent flushes could lose data if TRUNCATE runs between
    // another flush's INSERT and its own TRUNCATE.
    await query('BEGIN');
    try {
      await query(
        `INSERT INTO ${quoteIdentifier(targetTable)} (${columnSql})
         SELECT ${columnSql}
         FROM ${quoteIdentifier(stageTable)} s
         ${whereSql}
         ${onConflictSql}`
      );
      await query(`TRUNCATE ${quoteIdentifier(stageTable)}`);
      await query('COMMIT');
    } catch (err) {
      await query('ROLLBACK');
      throw err;
    }
  }

  private parseSchemaAndTable(table: string): { schema: string; name: string } {
    const parts = table.split('.').map((p) => p.trim()).filter(Boolean);
    if (parts.length === 2) {
      return { schema: parts[0].replace(/"/g, ''), name: parts[1].replace(/"/g, '') };
    }
    return { schema: 'public', name: table.replace(/"/g, '') };
  }

  private async getUniqueIndexColumns(table: string): Promise<ReadonlyArray<ReadonlyArray<string>>> {
    const cacheKey = table;
    const cached = this.uniqueIndexColumnsCache.get(cacheKey);
    if (cached) return cached;

    const { schema, name } = this.parseSchemaAndTable(table);

    const res = await query<{ cols: string[] }>(
      `
        SELECT array_agg(a.attname ORDER BY x.ordinality) AS cols
        FROM pg_index i
        JOIN pg_class t ON t.oid = i.indrelid
        JOIN pg_namespace n ON n.oid = t.relnamespace
        JOIN unnest(i.indkey) WITH ORDINALITY AS x(attnum, ordinality) ON true
        JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = x.attnum
        WHERE i.indisunique
          AND n.nspname = $1
          AND t.relname = $2
        GROUP BY i.indexrelid
      `,
      [schema, name]
    );

    const indexes = res.rows
      .map((r) => r.cols)
      .filter((cols): cols is string[] => Array.isArray(cols) && cols.length > 0);

    this.uniqueIndexColumnsCache.set(cacheKey, indexes);
    return indexes;
  }

  private async buildOnConflictSql(targetTable: string, usableConflictCols: ReadonlyArray<string>): Promise<string> {
    if (!usableConflictCols.includes('event_id')) return '';

    const uniqueIndexes = await this.getUniqueIndexColumns(targetTable);
    const hasUniqueOn = (desired: ReadonlyArray<string>): boolean => {
      const desiredSet = new Set(desired);
      return uniqueIndexes.some((idx) => idx.length === desired.length && idx.every((c) => desiredSet.has(c)));
    };

    if (usableConflictCols.includes('observed_at') && hasUniqueOn(['event_id', 'observed_at'])) {
      return 'ON CONFLICT ("event_id", "observed_at") DO NOTHING';
    }
    if (hasUniqueOn(['event_id'])) {
      return 'ON CONFLICT ("event_id") DO NOTHING';
    }

    // Safety fallback: if the target has unique constraints we didn't detect (e.g. partition-local
    // unique indexes), inserting a batch with duplicate rows can wedge the stage->fact merge.
    // `ON CONFLICT DO NOTHING` (without a conflict target) is valid and will ignore any unique
    // violations the insert encounters.
    return 'ON CONFLICT DO NOTHING';
  }

  private updateBufferGauge(kind: 'impressions' | 'clicks' | 'revenue'): void {
    const size = kind === 'impressions'
      ? this.impressionBuffer.length
      : kind === 'clicks'
        ? this.clickBuffer.length
        : this.revenueBuffer.length;
    analyticsIngestBufferGauge.set({ kind }, size);
  }
}

// Singleton instance
export const analyticsService = new AnalyticsService();

export default analyticsService;
