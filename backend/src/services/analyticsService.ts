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
  
  private readonly BATCH_SIZE: number;
  private readonly FLUSH_INTERVAL_MS: number;
  private flushInterval: NodeJS.Timeout | null = null;

  constructor(options: AnalyticsServiceOptions = {}) {
    this.BATCH_SIZE = options.batchSize ?? 100;
    this.FLUSH_INTERVAL_MS = options.flushIntervalMs ?? 5000;

    if (this.FLUSH_INTERVAL_MS > 0) {
      const interval = setInterval(() => {
        void this.flushAll();
      }, this.FLUSH_INTERVAL_MS);

      interval.unref?.();
      this.flushInterval = interval;
    }
  }

  /**
   * Record an impression event
   */
  async recordImpression(event: ImpressionEventDTO): Promise<void> {
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
    this.impressionBuffer.push(...events);
    this.updateBufferGauge('impressions');
    
    if (this.impressionBuffer.length >= this.BATCH_SIZE) {
      await this.flushImpressions();
    }
  }

  /**
   * Record a click event
   */
  async recordClick(event: ClickEventDTO): Promise<void> {
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
    this.clickBuffer.push(...events);
    this.updateBufferGauge('clicks');
    
    if (this.clickBuffer.length >= this.BATCH_SIZE) {
      await this.flushClicks();
    }
  }

  /**
   * Record a revenue event
   */
  async recordRevenue(event: RevenueEventDTO): Promise<void> {
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
    this.revenueBuffer.push(...events);
    this.updateBufferGauge('revenue');
    
    if (this.revenueBuffer.length >= this.BATCH_SIZE) {
      await this.flushRevenue();
    }
  }

  /**
   * Flush impression buffer to Postgres
   */
  private async flushImpressions(): Promise<void> {
    if (this.impressionBuffer.length === 0) {
      return;
    }

    const eventsToFlush = [...this.impressionBuffer];
    this.impressionBuffer = [];
    this.updateBufferGauge('impressions');

    try {
      const rows = eventsToFlush.map(toImpressionRow);
      await insertMany(
        STAGE_TABLE.impressions,
        [...IMPRESSION_COLUMNS],
        rows
      );
      await this.mergeStageIntoTarget(
        STAGE_TABLE.impressions,
        'analytics_impressions',
        [...IMPRESSION_COLUMNS],
        ['observed_at', 'event_id']
      );
      logger.debug('Flushed impressions', { count: eventsToFlush.length });
    } catch (error) {
      logger.debug('Failed to flush impressions (retrying)', { 
        count: eventsToFlush.length
      });
      // Re-add to buffer for retry
      this.impressionBuffer.unshift(...eventsToFlush);
      this.updateBufferGauge('impressions');
    }
  }

  /**
   * Flush click buffer to Postgres
   */
  private async flushClicks(): Promise<void> {
    if (this.clickBuffer.length === 0) {
      return;
    }

    const eventsToFlush = [...this.clickBuffer];
    this.clickBuffer = [];
    this.updateBufferGauge('clicks');

    try {
      const rows = eventsToFlush.map(toClickRow);
      await insertMany(
        STAGE_TABLE.clicks,
        [...CLICK_COLUMNS],
        rows
      );
      await this.mergeStageIntoTarget(
        STAGE_TABLE.clicks,
        'analytics_clicks',
        [...CLICK_COLUMNS],
        ['observed_at', 'event_id']
      );
      logger.debug('Flushed clicks', { count: eventsToFlush.length });
    } catch (error) {
      logger.error('Failed to flush clicks', { 
        count: eventsToFlush.length,
        error 
      });
      this.clickBuffer.unshift(...eventsToFlush);
      this.updateBufferGauge('clicks');
    }
  }

  /**
   * Flush revenue buffer to Postgres
   */
  private async flushRevenue(): Promise<void> {
    if (this.revenueBuffer.length === 0) {
      return;
    }

    const eventsToFlush = [...this.revenueBuffer];
    this.revenueBuffer = [];
    this.updateBufferGauge('revenue');

    try {
      const rows = eventsToFlush.map(toRevenueRow);
      await insertMany(
        STAGE_TABLE.revenue,
        [...REVENUE_COLUMNS],
        rows
      );
      await this.mergeStageIntoTarget(
        STAGE_TABLE.revenue,
        'analytics_revenue_events',
        [...REVENUE_COLUMNS],
        ['observed_at', 'event_id']
      );
      logger.debug('Flushed revenue events', { count: eventsToFlush.length });
    } catch (error) {
      logger.debug('Failed to flush revenue events (retrying)', { 
        count: eventsToFlush.length
      });
      this.revenueBuffer.unshift(...eventsToFlush);
      this.updateBufferGauge('revenue');
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
    };
  }

  private async mergeStageIntoTarget(
    stageTable: string,
    targetTable: string,
    columns: ReadonlyArray<string>,
    conflictColumns: ReadonlyArray<string>
  ): Promise<void> {
    const columnSql = columns.map(quoteIdentifier).join(', ');
    const conflictSql = conflictColumns.map(quoteIdentifier).join(', ');

    await query(
      `INSERT INTO ${quoteIdentifier(targetTable)} (${columnSql})
       SELECT ${columnSql} FROM ${quoteIdentifier(stageTable)}
       ON CONFLICT (${conflictSql}) DO NOTHING`
    );
    await query(`TRUNCATE ${quoteIdentifier(stageTable)}`);
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
