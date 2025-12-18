/**
 * Canonical SLO constants – single source of truth.
 */
export const SLO = {
  availability: {
    target: '99.9%',
    scope: 'Core API and SDK endpoints',
  },
  latency: {
    p50: '<50ms',
    p95: '<200ms',
    p99: '<200ms', // Align p99 to 200ms per SSAI docs deadline; adjust here to change everywhere
    scope: 'Decision latency (cue to decision complete)',
  },
  failover: {
    target: '<30s',
    description: 'Automatic failover between regions',
  },
  docsUrl: '/slo',
} as const;

export type LiveStatus = {
  status: 'operational' | 'degraded' | 'down' | 'unknown';
  service?: string;
  timestamp?: string; // ISO8601 in UTC
};
