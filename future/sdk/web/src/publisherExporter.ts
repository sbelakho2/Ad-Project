/**
 * @file Publisher Data Exports
 * @description Export SDK analytics and metrics data for publishers
 * @module @rivalapex/web-sdk
 */

import { UsageMetrics, UsageBreakdown } from './usageMeter';

/**
 * Export format options
 */
export enum ExportFormat {
  JSON = 'json',
  CSV = 'csv',
  PARQUET = 'parquet',
}

/**
 * Date range for exports
 */
export interface DateRange {
  startDate: Date;
  endDate: Date;
}

/**
 * Export configuration
 */
export interface ExportConfig {
  format: ExportFormat;
  dateRange?: DateRange;
  dimensions?: ExportDimension[];
  metrics?: ExportMetric[];
  filters?: ExportFilter[];
  includeBreakdowns?: boolean;
  timezone?: string;
}

/**
 * Available dimensions for grouping
 */
export enum ExportDimension {
  DATE = 'date',
  HOUR = 'hour',
  PLACEMENT = 'placement',
  ADAPTER = 'adapter',
  AD_FORMAT = 'ad_format',
  COUNTRY = 'country',
  DEVICE_TYPE = 'device_type',
  OS_VERSION = 'os_version',
  SDK_VERSION = 'sdk_version',
}

/**
 * Available metrics for export
 */
export enum ExportMetric {
  REQUESTS = 'requests',
  IMPRESSIONS = 'impressions',
  CLICKS = 'clicks',
  CTR = 'ctr',
  FILL_RATE = 'fill_rate',
  REVENUE = 'revenue',
  ECPM = 'ecpm',
  VIDEO_STARTS = 'video_starts',
  VIDEO_COMPLETES = 'video_completes',
  VIDEO_COMPLETION_RATE = 'video_completion_rate',
  ERRORS = 'errors',
  LATENCY_P50 = 'latency_p50',
  LATENCY_P95 = 'latency_p95',
  LATENCY_P99 = 'latency_p99',
}

/**
 * Filter for exports
 */
export interface ExportFilter {
  dimension: ExportDimension;
  operator: 'equals' | 'not_equals' | 'contains' | 'in' | 'not_in';
  value: string | string[];
}

/**
 * Export result
 */
export interface ExportResult {
  success: boolean;
  format: ExportFormat;
  data?: string | Blob;
  downloadUrl?: string;
  rowCount?: number;
  generatedAt: Date;
  expiresAt?: Date;
  error?: string;
}

/**
 * Scheduled export configuration
 */
export interface ScheduledExport {
  id: string;
  name: string;
  config: ExportConfig;
  schedule: ExportSchedule;
  destination: ExportDestination;
  enabled: boolean;
  lastRunAt?: Date;
  nextRunAt?: Date;
}

/**
 * Export schedule options
 */
export interface ExportSchedule {
  frequency: 'daily' | 'weekly' | 'monthly';
  hour: number; // 0-23
  dayOfWeek?: number; // 0-6 for weekly
  dayOfMonth?: number; // 1-31 for monthly
  timezone: string;
}

/**
 * Export destination options
 */
export interface ExportDestination {
  type: 'email' | 's3' | 'gcs' | 'webhook';
  email?: string;
  s3Bucket?: string;
  s3Prefix?: string;
  gcsBucket?: string;
  gcsPrefix?: string;
  webhookUrl?: string;
  webhookHeaders?: Record<string, string>;
}

/**
 * Data row for exports
 */
export interface ExportRow {
  date?: string;
  hour?: number;
  placement?: string;
  adapter?: string;
  adFormat?: string;
  country?: string;
  deviceType?: string;
  osVersion?: string;
  sdkVersion?: string;
  requests?: number;
  impressions?: number;
  clicks?: number;
  ctr?: number;
  fillRate?: number;
  revenue?: number;
  ecpm?: number;
  videoStarts?: number;
  videoCompletes?: number;
  videoCompletionRate?: number;
  errors?: number;
  latencyP50?: number;
  latencyP95?: number;
  latencyP99?: number;
}

/**
 * Publisher data exporter
 */
export class PublisherExporter {
  private apiEndpoint: string;
  private apiKey?: string;
  private scheduledExports: Map<string, ScheduledExport> = new Map();

  constructor(config: { apiEndpoint?: string; apiKey?: string } = {}) {
    this.apiEndpoint = config.apiEndpoint ?? '/api/v1/exports';
    this.apiKey = config.apiKey;
  }

  /**
   * Generate an export based on configuration
   */
  async generateExport(config: ExportConfig): Promise<ExportResult> {
    try {
      const response = await fetch(`${this.apiEndpoint}/generate`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
        },
        body: JSON.stringify({
          format: config.format,
          dateRange: config.dateRange
            ? {
                startDate: config.dateRange.startDate.toISOString(),
                endDate: config.dateRange.endDate.toISOString(),
              }
            : undefined,
          dimensions: config.dimensions,
          metrics: config.metrics,
          filters: config.filters,
          includeBreakdowns: config.includeBreakdowns,
          timezone: config.timezone,
        }),
      });

      if (!response.ok) {
        return {
          success: false,
          format: config.format,
          generatedAt: new Date(),
          error: `Export failed: ${response.statusText}`,
        };
      }

      const result = await response.json();
      return {
        success: true,
        format: config.format,
        downloadUrl: result.downloadUrl,
        rowCount: result.rowCount,
        generatedAt: new Date(result.generatedAt),
        expiresAt: result.expiresAt ? new Date(result.expiresAt) : undefined,
      };
    } catch (error) {
      return {
        success: false,
        format: config.format,
        generatedAt: new Date(),
        error: error instanceof Error ? error.message : 'Unknown error',
      };
    }
  }

  /**
   * Generate export data locally from metrics
   */
  generateLocalExport(
    metrics: UsageMetrics,
    breakdown?: UsageBreakdown,
    config?: Partial<ExportConfig>
  ): ExportResult {
    const format = config?.format ?? ExportFormat.JSON;
    const rows = this.metricsToRows(metrics, breakdown);

    let data: string;
    if (format === ExportFormat.CSV) {
      data = this.rowsToCSV(rows);
    } else {
      data = JSON.stringify(rows, null, 2);
    }

    return {
      success: true,
      format,
      data,
      rowCount: rows.length,
      generatedAt: new Date(),
    };
  }

  /**
   * Convert metrics to export rows
   */
  private metricsToRows(metrics: UsageMetrics, breakdown?: UsageBreakdown): ExportRow[] {
    const rows: ExportRow[] = [];

    // Global row
    rows.push({
      date: new Date(metrics.periodStart).toISOString().split('T')[0],
      requests: metrics.adRequests,
      impressions: metrics.adImpressions,
      clicks: metrics.adClicks,
      ctr: metrics.adImpressions > 0 ? metrics.adClicks / metrics.adImpressions : 0,
      fillRate: metrics.adRequests > 0 ? metrics.adImpressions / metrics.adRequests : 0,
      revenue: metrics.totalRevenue,
      ecpm: metrics.adImpressions > 0 ? (metrics.totalRevenue / metrics.adImpressions) * 1000 : 0,
      videoStarts: metrics.videoStarts,
      videoCompletes: metrics.videoCompletes,
      videoCompletionRate: metrics.videoStarts > 0 ? metrics.videoCompletes / metrics.videoStarts : 0,
      errors: metrics.errors,
    });

    // Breakdown rows
    if (breakdown) {
      breakdown.byPlacement.forEach((m, placement) => {
        rows.push({
          ...this.metricsToRow(m),
          placement,
        });
      });

      breakdown.byAdapter.forEach((m, adapter) => {
        rows.push({
          ...this.metricsToRow(m),
          adapter,
        });
      });

      breakdown.byAdFormat.forEach((m, adFormat) => {
        rows.push({
          ...this.metricsToRow(m),
          adFormat,
        });
      });
    }

    return rows;
  }

  /**
   * Convert a single metrics object to a row
   */
  private metricsToRow(m: UsageMetrics): ExportRow {
    return {
      date: new Date(m.periodStart).toISOString().split('T')[0],
      requests: m.adRequests,
      impressions: m.adImpressions,
      clicks: m.adClicks,
      ctr: m.adImpressions > 0 ? m.adClicks / m.adImpressions : 0,
      fillRate: m.adRequests > 0 ? m.adImpressions / m.adRequests : 0,
      revenue: m.totalRevenue,
      ecpm: m.adImpressions > 0 ? (m.totalRevenue / m.adImpressions) * 1000 : 0,
      videoStarts: m.videoStarts,
      videoCompletes: m.videoCompletes,
      videoCompletionRate: m.videoStarts > 0 ? m.videoCompletes / m.videoStarts : 0,
      errors: m.errors,
    };
  }

  /**
   * Convert rows to CSV format
   */
  private rowsToCSV(rows: ExportRow[]): string {
    if (rows.length === 0) return '';

    const headers = Object.keys(rows[0]);
    const lines: string[] = [];

    // Header row
    lines.push(headers.join(','));

    // Data rows
    for (const row of rows) {
      const values = headers.map(h => {
        const val = (row as Record<string, unknown>)[h];
        if (val === undefined || val === null) return '';
        if (typeof val === 'string' && val.includes(',')) {
          return `"${val.replace(/"/g, '""')}"`;
        }
        return String(val);
      });
      lines.push(values.join(','));
    }

    return lines.join('\n');
  }

  /**
   * Download export as file
   */
  downloadExport(result: ExportResult, filename?: string): void {
    if (!result.success || !result.data) {
      throw new Error('No data to download');
    }

    const blob =
      result.data instanceof Blob
        ? result.data
        : new Blob([result.data], {
            type: result.format === ExportFormat.CSV ? 'text/csv' : 'application/json',
          });

    const url = URL.createObjectURL(blob);
    const extension = result.format === ExportFormat.CSV ? 'csv' : 'json';
    const defaultFilename = `export-${new Date().toISOString().split('T')[0]}.${extension}`;

    const a = document.createElement('a');
    a.href = url;
    a.download = filename ?? defaultFilename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }

  // Scheduled exports management

  /**
   * Create a scheduled export
   */
  async createScheduledExport(
    name: string,
    config: ExportConfig,
    schedule: ExportSchedule,
    destination: ExportDestination
  ): Promise<ScheduledExport> {
    const response = await fetch(`${this.apiEndpoint}/scheduled`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
      body: JSON.stringify({ name, config, schedule, destination }),
    });

    if (!response.ok) {
      throw new Error(`Failed to create scheduled export: ${response.statusText}`);
    }

    const result = await response.json();
    const scheduledExport: ScheduledExport = {
      id: result.id,
      name,
      config,
      schedule,
      destination,
      enabled: true,
      nextRunAt: result.nextRunAt ? new Date(result.nextRunAt) : undefined,
    };

    this.scheduledExports.set(result.id, scheduledExport);
    return scheduledExport;
  }

  /**
   * List all scheduled exports
   */
  async listScheduledExports(): Promise<ScheduledExport[]> {
    const response = await fetch(`${this.apiEndpoint}/scheduled`, {
      headers: {
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
    });

    if (!response.ok) {
      throw new Error(`Failed to list scheduled exports: ${response.statusText}`);
    }

    const results = await response.json();
    return results.exports ?? [];
  }

  /**
   * Update a scheduled export
   */
  async updateScheduledExport(id: string, updates: Partial<ScheduledExport>): Promise<ScheduledExport> {
    const response = await fetch(`${this.apiEndpoint}/scheduled/${id}`, {
      method: 'PATCH',
      headers: {
        'Content-Type': 'application/json',
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
      body: JSON.stringify(updates),
    });

    if (!response.ok) {
      throw new Error(`Failed to update scheduled export: ${response.statusText}`);
    }

    const result = await response.json();
    return result;
  }

  /**
   * Delete a scheduled export
   */
  async deleteScheduledExport(id: string): Promise<void> {
    const response = await fetch(`${this.apiEndpoint}/scheduled/${id}`, {
      method: 'DELETE',
      headers: {
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
    });

    if (!response.ok) {
      throw new Error(`Failed to delete scheduled export: ${response.statusText}`);
    }

    this.scheduledExports.delete(id);
  }

  /**
   * Enable or disable a scheduled export
   */
  async setScheduledExportEnabled(id: string, enabled: boolean): Promise<ScheduledExport> {
    return this.updateScheduledExport(id, { enabled });
  }

  /**
   * Trigger a scheduled export immediately
   */
  async triggerScheduledExport(id: string): Promise<ExportResult> {
    const response = await fetch(`${this.apiEndpoint}/scheduled/${id}/trigger`, {
      method: 'POST',
      headers: {
        ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}),
      },
    });

    if (!response.ok) {
      return {
        success: false,
        format: ExportFormat.JSON,
        generatedAt: new Date(),
        error: `Failed to trigger export: ${response.statusText}`,
      };
    }

    const result = await response.json();
    return {
      success: true,
      format: result.format ?? ExportFormat.JSON,
      downloadUrl: result.downloadUrl,
      rowCount: result.rowCount,
      generatedAt: new Date(),
    };
  }
}

/**
 * Builder for export configuration
 */
export class ExportConfigBuilder {
  private config: ExportConfig = {
    format: ExportFormat.JSON,
  };

  format(format: ExportFormat): this {
    this.config.format = format;
    return this;
  }

  dateRange(startDate: Date, endDate: Date): this {
    this.config.dateRange = { startDate, endDate };
    return this;
  }

  lastNDays(days: number): this {
    const endDate = new Date();
    const startDate = new Date();
    startDate.setDate(startDate.getDate() - days);
    return this.dateRange(startDate, endDate);
  }

  dimensions(...dimensions: ExportDimension[]): this {
    this.config.dimensions = dimensions;
    return this;
  }

  metrics(...metrics: ExportMetric[]): this {
    this.config.metrics = metrics;
    return this;
  }

  filter(dimension: ExportDimension, operator: ExportFilter['operator'], value: string | string[]): this {
    if (!this.config.filters) {
      this.config.filters = [];
    }
    this.config.filters.push({ dimension, operator, value });
    return this;
  }

  includeBreakdowns(include: boolean = true): this {
    this.config.includeBreakdowns = include;
    return this;
  }

  timezone(tz: string): this {
    this.config.timezone = tz;
    return this;
  }

  build(): ExportConfig {
    return { ...this.config };
  }
}
