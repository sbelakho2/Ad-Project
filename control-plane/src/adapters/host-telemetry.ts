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
export async function getHostTelemetry(endpointOverride?: string): Promise<HostTelemetry | null> {
  const cfg = getConfig();
  const endpoint = endpointOverride || cfg.HOST_TELEMETRY_ENDPOINT_APP || cfg.HOST_TELEMETRY_ENDPOINT;

  if (!endpoint) {
    logger.warn('Host telemetry endpoint not configured');
    return null;
  }

  return getHostTelemetryForEndpoint(endpoint);
}

export async function getHostTelemetryForEndpoint(endpoint?: string): Promise<HostTelemetry | null> {
  if (!endpoint) return null;

  try {
    const response = await fetch(`${endpoint}/metrics`);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }

    const text = await response.text();
    return parsePrometheusMetrics(text);
  } catch (err) {
    logger.error('Failed to fetch host telemetry', {
      endpoint,
      error: (err as Error).message,
    });
    return null;
  }
}

export async function getHostTelemetryBundle(): Promise<{ app: HostTelemetry | null; db: HostTelemetry | null }> {
  const cfg = getConfig();
  const appEndpoint = cfg.HOST_TELEMETRY_ENDPOINT_APP || cfg.HOST_TELEMETRY_ENDPOINT;
  const dbEndpoint = cfg.HOST_TELEMETRY_ENDPOINT_DB;

  const [app, db] = await Promise.all([
    getHostTelemetryForEndpoint(appEndpoint),
    getHostTelemetryForEndpoint(dbEndpoint),
  ]);

  return { app, db };
}

/**
 * Parse Prometheus metrics format (from node-exporter)
 * Properly handles labeled metrics like node_filesystem_size_bytes{device="/dev/sda1",mountpoint="/"}
 */
function parseLabelString(raw: string): Record<string, string> {
  const labels: Record<string, string> = {};
  raw.split(',').forEach((pair) => {
    const match = pair.trim().match(/^([^=]+)="([^"]*)"$/);
    if (match) {
      labels[match[1]] = match[2];
    }
  });
  return labels;
}

function parsePrometheusMetrics(text: string): HostTelemetry {
  const lines = text.split('\n');
  
  // Store metrics with labels
  const labeledMetrics: Array<{ name: string; labels: Record<string, string>; value: number }> = [];
  const simpleMetrics: Record<string, number> = {};
  
  for (const line of lines) {
    if (line.startsWith('#') || !line.trim()) continue;
    
    // Parse metric with labels: metric_name{label1="val1",label2="val2"} value
    const labeledMatch = line.match(/^(\w+)\{([^}]*)\}\s+([\d.e+-]+)$/);
    if (labeledMatch) {
      const [, name, labels, value] = labeledMatch;
      labeledMetrics.push({
        name,
        labels: parseLabelString(labels),
        value: parseFloat(value),
      });
      continue;
    }
    
    // Parse simple metric: metric_name value
    const simpleMatch = line.match(/^(\w+)\s+([\d.e+-]+)$/);
    if (simpleMatch) {
      const [, name, value] = simpleMatch;
      simpleMetrics[name] = parseFloat(value);
    }
  }
  
  // Build disk metrics per mountpoint
  const diskMap = new Map<string, Partial<DiskStats>>();
  labeledMetrics.forEach((metric) => {
    const mountpoint = metric.labels.mountpoint;
    const device = metric.labels.device || 'unknown';
    if (!mountpoint) return;

    if (!diskMap.has(mountpoint)) {
      diskMap.set(mountpoint, {
        device,
        mountpoint,
      });
    }

    const entry = diskMap.get(mountpoint)!;
    if (metric.name === 'node_filesystem_size_bytes') {
      entry.total_bytes = metric.value;
    }
    if (metric.name === 'node_filesystem_avail_bytes') {
      entry.available_bytes = metric.value;
    }
  });

  const disks: DiskStats[] = Array.from(diskMap.values())
    .filter((d) => typeof d.total_bytes === 'number' && typeof d.available_bytes === 'number')
    .map((d) => {
      const total = d.total_bytes || 0;
      const avail = d.available_bytes || 0;
      const used = total - avail;
      return {
        device: d.device || 'unknown',
        mountpoint: d.mountpoint || 'unknown',
        total_bytes: total,
        used_bytes: used,
        available_bytes: avail,
        use_percent: total > 0 ? (used / total) * 100 : 0,
      };
    });
  
  const rootDisk = disks.find((d) => d.mountpoint === '/') || {
    device: '/dev/unknown',
    mountpoint: '/',
    total_bytes: 0,
    used_bytes: 0,
    available_bytes: 0,
    use_percent: 0,
  };
  
  return {
    timestamp: new Date(),
    hostname: 'unknown',
    uptime_seconds: simpleMetrics['node_boot_time_seconds'] 
      ? Date.now() / 1000 - simpleMetrics['node_boot_time_seconds']
      : 0,
    load_average: [
      simpleMetrics['node_load1'] || 0,
      simpleMetrics['node_load5'] || 0,
      simpleMetrics['node_load15'] || 0,
    ],
    disk: disks.length > 0 ? disks : [rootDisk],
    memory: {
      total_bytes: simpleMetrics['node_memory_MemTotal_bytes'] || 0,
      available_bytes: simpleMetrics['node_memory_MemAvailable_bytes'] || 0,
      used_bytes: (simpleMetrics['node_memory_MemTotal_bytes'] || 0) - (simpleMetrics['node_memory_MemAvailable_bytes'] || 0),
      use_percent: simpleMetrics['node_memory_MemTotal_bytes']
        ? ((simpleMetrics['node_memory_MemTotal_bytes'] - simpleMetrics['node_memory_MemAvailable_bytes']) / simpleMetrics['node_memory_MemTotal_bytes']) * 100
        : 0,
      swap_total_bytes: simpleMetrics['node_memory_SwapTotal_bytes'] || 0,
      swap_used_bytes: (simpleMetrics['node_memory_SwapTotal_bytes'] || 0) - (simpleMetrics['node_memory_SwapFree_bytes'] || 0),
      swap_percent: simpleMetrics['node_memory_SwapTotal_bytes']
        ? (((simpleMetrics['node_memory_SwapTotal_bytes'] || 0) - (simpleMetrics['node_memory_SwapFree_bytes'] || 0)) / simpleMetrics['node_memory_SwapTotal_bytes']) * 100
        : 0,
    },
    cpu: {
      user_percent: 0,
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
