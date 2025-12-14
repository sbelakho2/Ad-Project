/**
 * HOST TELEMETRY ADAPTER
 * 
 * Collects host-level metrics:
 * - Disk usage and I/O wait
 * - Memory and swap usage
 * - CPU usage and steal time
 * - Time drift
 * - Reboot history
 */

import { getConfig } from '../config';
import { logger } from '../logger';

export interface DiskStats {
  device: string;
  mountpoint: string;
  total_bytes: number;
  used_bytes: number;
  available_bytes: number;
  use_percent: number;
}

export interface MemoryStats {
  total_bytes: number;
  available_bytes: number;
  used_bytes: number;
  use_percent: number;
  swap_total_bytes: number;
  swap_used_bytes: number;
  swap_percent: number;
}

export interface CPUStats {
  user_percent: number;
  system_percent: number;
  idle_percent: number;
  iowait_percent: number;
  steal_percent: number;
}

export interface HostTelemetry {
  timestamp: Date;
  hostname: string;
  uptime_seconds: number;
  load_average: [number, number, number];
  disk: DiskStats[];
  memory: MemoryStats;
  cpu: CPUStats;
  time_drift_ms?: number;
}

/**
 * Fetch host telemetry from node-exporter or similar endpoint
 */
export async function getHostTelemetry(): Promise<HostTelemetry | null> {
  const cfg = getConfig();
  const endpoint = cfg.HOST_TELEMETRY_ENDPOINT;
  
  if (!endpoint) {
    logger.warn('Host telemetry endpoint not configured');
    return null;
  }
  
  try {
    // Fetch metrics from node-exporter
    const response = await fetch(`${endpoint}/metrics`);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    
    const text = await response.text();
    return parsePrometheusMetrics(text);
  } catch (err) {
    logger.error('Failed to fetch host telemetry', { 
      endpoint, 
      error: (err as Error).message 
    });
    return null;
  }
}

/**
 * Parse Prometheus metrics format (from node-exporter)
 */
function parsePrometheusMetrics(text: string): HostTelemetry {
  const lines = text.split('\n');
  const metrics: Record<string, number> = {};
  
  for (const line of lines) {
    if (line.startsWith('#') || !line.trim()) continue;
    
    // Simple metric parsing (metric_name{labels} value)
    const match = line.match(/^(\w+)(\{[^}]*\})?\s+([\d.e+-]+)$/);
    if (match) {
      const [, name, , value] = match;
      metrics[name] = parseFloat(value);
    }
  }
  
  return {
    timestamp: new Date(),
    hostname: 'unknown', // Would come from node_uname_info
    uptime_seconds: metrics['node_boot_time_seconds'] 
      ? Date.now() / 1000 - metrics['node_boot_time_seconds']
      : 0,
    load_average: [
      metrics['node_load1'] || 0,
      metrics['node_load5'] || 0,
      metrics['node_load15'] || 0,
    ],
    disk: [{
      device: 'root',
      mountpoint: '/',
      total_bytes: metrics['node_filesystem_size_bytes'] || 0,
      used_bytes: (metrics['node_filesystem_size_bytes'] || 0) - (metrics['node_filesystem_avail_bytes'] || 0),
      available_bytes: metrics['node_filesystem_avail_bytes'] || 0,
      use_percent: metrics['node_filesystem_size_bytes'] 
        ? ((metrics['node_filesystem_size_bytes'] - metrics['node_filesystem_avail_bytes']) / metrics['node_filesystem_size_bytes']) * 100
        : 0,
    }],
    memory: {
      total_bytes: metrics['node_memory_MemTotal_bytes'] || 0,
      available_bytes: metrics['node_memory_MemAvailable_bytes'] || 0,
      used_bytes: (metrics['node_memory_MemTotal_bytes'] || 0) - (metrics['node_memory_MemAvailable_bytes'] || 0),
      use_percent: metrics['node_memory_MemTotal_bytes']
        ? ((metrics['node_memory_MemTotal_bytes'] - metrics['node_memory_MemAvailable_bytes']) / metrics['node_memory_MemTotal_bytes']) * 100
        : 0,
      swap_total_bytes: metrics['node_memory_SwapTotal_bytes'] || 0,
      swap_used_bytes: (metrics['node_memory_SwapTotal_bytes'] || 0) - (metrics['node_memory_SwapFree_bytes'] || 0),
      swap_percent: metrics['node_memory_SwapTotal_bytes']
        ? (((metrics['node_memory_SwapTotal_bytes'] || 0) - (metrics['node_memory_SwapFree_bytes'] || 0)) / metrics['node_memory_SwapTotal_bytes']) * 100
        : 0,
    },
    cpu: {
      user_percent: 0, // Would need rate calculation
      system_percent: 0,
      idle_percent: 0,
      iowait_percent: 0,
      steal_percent: 0,
    },
  };
}

/**
 * Calculate days until disk reaches threshold
 */
export function calculateDaysToThreshold(
  currentUsedBytes: number,
  totalBytes: number,
  dailyGrowthBytes: number,
  thresholdPercent = 85
): number {
  if (dailyGrowthBytes <= 0) return Infinity;
  
  const thresholdBytes = totalBytes * (thresholdPercent / 100);
  const remainingBytes = thresholdBytes - currentUsedBytes;
  
  if (remainingBytes <= 0) return 0;
  
  return Math.floor(remainingBytes / dailyGrowthBytes);
}
