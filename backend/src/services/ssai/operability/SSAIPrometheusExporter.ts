/**
 * SSAI Prometheus Metrics Exporter
 * Provides detailed SLI metrics for monitoring and alerting
 */

import { Registry, Counter, Histogram, Gauge, Summary } from 'prom-client';

export class SSAIPrometheusExporter {
  private registry: Registry;
  
  // Session metrics
  private sessionsCreated!: Counter;
  private sessionsActive!: Gauge;
  private sessionDuration!: Histogram;
  
  // Manifest metrics
  private manifestRequestsTotal!: Counter;
  private manifestLatencyMs!: Histogram;
  private manifestErrorsTotal!: Counter;
  
  // Pod decisioning metrics
  private podDecisionsTotal!: Counter;
  private podDecisionLatencyMs!: Histogram;
  private fillRate!: Gauge;
  private noFillTotal!: Counter;
  
  // Stitching metrics
  private stitchingOperationsTotal!: Counter;
  private stitchingLatencyMs!: Histogram;
  private stitchingErrorsTotal!: Counter;
  private discontinuityCount!: Histogram;
  
  // Tracking metrics
  private beaconsFiredTotal!: Counter;
  private beaconLatencyMs!: Histogram;
  private beaconErrorsTotal!: Counter;
  
  // Partner metrics
  private partnerRequestsTotal!: Counter;
  private partnerLatencyMs!: Histogram;
  private partnerErrorsTotal!: Counter;
  private circuitBreakerState!: Gauge;
  
  // SLI metrics
  private sliAvailability!: Gauge;
  private sliLatencyP99!: Gauge;
  private sliErrorRate!: Gauge;
  private sliFillRate!: Gauge;

  constructor() {
    this.registry = new Registry();
    this.registry.setDefaultLabels({
      service: 'ssai',
    });

    // Initialize all metrics
    this.initSessionMetrics();
    this.initManifestMetrics();
    this.initPodMetrics();
    this.initStitchingMetrics();
    this.initTrackingMetrics();
    this.initPartnerMetrics();
    this.initSLIMetrics();
  }

  private initSessionMetrics() {
    this.sessionsCreated = new Counter({
      name: 'ssai_sessions_created_total',
      help: 'Total number of SSAI sessions created',
      labelNames: ['protocol', 'tenant_id'],
      registers: [this.registry],
    });

    this.sessionsActive = new Gauge({
      name: 'ssai_sessions_active',
      help: 'Number of currently active SSAI sessions',
      labelNames: ['protocol'],
      registers: [this.registry],
    });

    this.sessionDuration = new Histogram({
      name: 'ssai_session_duration_seconds',
      help: 'Duration of SSAI sessions in seconds',
      labelNames: ['protocol', 'status'],
      buckets: [60, 300, 600, 1800, 3600, 7200],
      registers: [this.registry],
    });
  }

  private initManifestMetrics() {
    this.manifestRequestsTotal = new Counter({
      name: 'ssai_manifest_requests_total',
      help: 'Total number of manifest requests',
      labelNames: ['protocol', 'type'],
      registers: [this.registry],
    });

    this.manifestLatencyMs = new Histogram({
      name: 'ssai_manifest_latency_ms',
      help: 'Manifest generation latency in milliseconds',
      labelNames: ['protocol', 'type'],
      buckets: [10, 25, 50, 100, 250, 500, 1000, 2500],
      registers: [this.registry],
    });

    this.manifestErrorsTotal = new Counter({
      name: 'ssai_manifest_errors_total',
      help: 'Total number of manifest errors',
      labelNames: ['protocol', 'error_type'],
      registers: [this.registry],
    });
  }

  private initPodMetrics() {
    this.podDecisionsTotal = new Counter({
      name: 'ssai_pod_decisions_total',
      help: 'Total number of pod decisions made',
      labelNames: ['partner_id', 'result'],
      registers: [this.registry],
    });

    this.podDecisionLatencyMs = new Histogram({
      name: 'ssai_pod_decision_latency_ms',
      help: 'Pod decision latency in milliseconds',
      labelNames: ['partner_id'],
      buckets: [10, 25, 50, 100, 200, 500, 1000],
      registers: [this.registry],
    });

    this.fillRate = new Gauge({
      name: 'ssai_fill_rate',
      help: 'Current fill rate (0-1)',
      labelNames: ['partner_id', 'tenant_id'],
      registers: [this.registry],
    });

    this.noFillTotal = new Counter({
      name: 'ssai_no_fill_total',
      help: 'Total number of no-fill decisions',
      labelNames: ['partner_id', 'reason'],
      registers: [this.registry],
    });
  }

  private initStitchingMetrics() {
    this.stitchingOperationsTotal = new Counter({
      name: 'ssai_stitching_operations_total',
      help: 'Total number of stitching operations',
      labelNames: ['protocol', 'mode'],
      registers: [this.registry],
    });

    this.stitchingLatencyMs = new Histogram({
      name: 'ssai_stitching_latency_ms',
      help: 'Stitching latency in milliseconds',
      labelNames: ['protocol', 'mode'],
      buckets: [5, 10, 25, 50, 100, 250, 500],
      registers: [this.registry],
    });

    this.stitchingErrorsTotal = new Counter({
      name: 'ssai_stitching_errors_total',
      help: 'Total number of stitching errors',
      labelNames: ['protocol', 'error_type'],
      registers: [this.registry],
    });

    this.discontinuityCount = new Histogram({
      name: 'ssai_discontinuity_count',
      help: 'Number of discontinuities per manifest',
      labelNames: ['protocol'],
      buckets: [0, 1, 2, 4, 8, 16, 32],
      registers: [this.registry],
    });
  }

  private initTrackingMetrics() {
    this.beaconsFiredTotal = new Counter({
      name: 'ssai_beacons_fired_total',
      help: 'Total number of tracking beacons fired',
      labelNames: ['type'],
      registers: [this.registry],
    });

    this.beaconLatencyMs = new Histogram({
      name: 'ssai_beacon_latency_ms',
      help: 'Beacon firing latency in milliseconds',
      labelNames: ['type'],
      buckets: [10, 25, 50, 100, 250, 500, 1000],
      registers: [this.registry],
    });

    this.beaconErrorsTotal = new Counter({
      name: 'ssai_beacon_errors_total',
      help: 'Total number of beacon errors',
      labelNames: ['type', 'error_type'],
      registers: [this.registry],
    });
  }

  private initPartnerMetrics() {
    this.partnerRequestsTotal = new Counter({
      name: 'ssai_partner_requests_total',
      help: 'Total number of partner requests',
      labelNames: ['partner_id', 'endpoint'],
      registers: [this.registry],
    });

    this.partnerLatencyMs = new Histogram({
      name: 'ssai_partner_latency_ms',
      help: 'Partner response latency in milliseconds',
      labelNames: ['partner_id', 'endpoint'],
      buckets: [50, 100, 200, 500, 1000, 2000, 5000],
      registers: [this.registry],
    });

    this.partnerErrorsTotal = new Counter({
      name: 'ssai_partner_errors_total',
      help: 'Total number of partner errors',
      labelNames: ['partner_id', 'error_type'],
      registers: [this.registry],
    });

    this.circuitBreakerState = new Gauge({
      name: 'ssai_circuit_breaker_state',
      help: 'Circuit breaker state (0=closed, 1=half-open, 2=open)',
      labelNames: ['partner_id'],
      registers: [this.registry],
    });
  }

  private initSLIMetrics() {
    this.sliAvailability = new Gauge({
      name: 'ssai_sli_availability',
      help: 'SSAI availability SLI (0-1)',
      registers: [this.registry],
    });

    this.sliLatencyP99 = new Gauge({
      name: 'ssai_sli_latency_p99_ms',
      help: 'P99 latency SLI in milliseconds',
      labelNames: ['operation'],
      registers: [this.registry],
    });

    this.sliErrorRate = new Gauge({
      name: 'ssai_sli_error_rate',
      help: 'Error rate SLI (0-1)',
      labelNames: ['category'],
      registers: [this.registry],
    });

    this.sliFillRate = new Gauge({
      name: 'ssai_sli_fill_rate',
      help: 'Fill rate SLI (0-1)',
      registers: [this.registry],
    });
  }

  // Public methods for recording metrics
  recordSessionCreated(protocol: string, tenantId: string) {
    this.sessionsCreated.inc({ protocol, tenant_id: tenantId });
  }

  setActiveSessions(protocol: string, count: number) {
    this.sessionsActive.set({ protocol }, count);
  }

  recordSessionDuration(protocol: string, status: string, durationSeconds: number) {
    this.sessionDuration.observe({ protocol, status }, durationSeconds);
  }

  recordManifestRequest(protocol: string, type: string, latencyMs: number, success: boolean) {
    this.manifestRequestsTotal.inc({ protocol, type });
    this.manifestLatencyMs.observe({ protocol, type }, latencyMs);
    if (!success) {
      this.manifestErrorsTotal.inc({ protocol, error_type: 'request_failed' });
    }
  }

  recordPodDecision(partnerId: string, result: 'fill' | 'no_fill' | 'partial', latencyMs: number) {
    this.podDecisionsTotal.inc({ partner_id: partnerId, result });
    this.podDecisionLatencyMs.observe({ partner_id: partnerId }, latencyMs);
  }

  setFillRate(partnerId: string, tenantId: string, rate: number) {
    this.fillRate.set({ partner_id: partnerId, tenant_id: tenantId }, rate);
  }

  recordNoFill(partnerId: string, reason: string) {
    this.noFillTotal.inc({ partner_id: partnerId, reason });
  }

  recordStitching(protocol: string, mode: string, latencyMs: number, success: boolean) {
    this.stitchingOperationsTotal.inc({ protocol, mode });
    this.stitchingLatencyMs.observe({ protocol, mode }, latencyMs);
    if (!success) {
      this.stitchingErrorsTotal.inc({ protocol, error_type: 'stitching_failed' });
    }
  }

  recordDiscontinuities(protocol: string, count: number) {
    this.discontinuityCount.observe({ protocol }, count);
  }

  recordBeacon(type: string, latencyMs: number, success: boolean) {
    this.beaconsFiredTotal.inc({ type });
    this.beaconLatencyMs.observe({ type }, latencyMs);
    if (!success) {
      this.beaconErrorsTotal.inc({ type, error_type: 'beacon_failed' });
    }
  }

  recordPartnerRequest(partnerId: string, endpoint: string, latencyMs: number, success: boolean) {
    this.partnerRequestsTotal.inc({ partner_id: partnerId, endpoint });
    this.partnerLatencyMs.observe({ partner_id: partnerId, endpoint }, latencyMs);
    if (!success) {
      this.partnerErrorsTotal.inc({ partner_id: partnerId, error_type: 'request_failed' });
    }
  }

  setCircuitBreakerState(partnerId: string, state: 'closed' | 'half-open' | 'open') {
    const stateValue = state === 'closed' ? 0 : state === 'half-open' ? 1 : 2;
    this.circuitBreakerState.set({ partner_id: partnerId }, stateValue);
  }

  setSLIAvailability(availability: number) {
    this.sliAvailability.set(availability);
  }

  setSLILatencyP99(operation: string, latencyMs: number) {
    this.sliLatencyP99.set({ operation }, latencyMs);
  }

  setSLIErrorRate(category: string, rate: number) {
    this.sliErrorRate.set({ category }, rate);
  }

  setSLIFillRate(rate: number) {
    this.sliFillRate.set(rate);
  }

  // Get metrics in Prometheus format
  async getMetrics(): Promise<string> {
    return this.registry.metrics();
  }

  // Get metrics as JSON
  async getMetricsJSON(): Promise<object> {
    return this.registry.getMetricsAsJSON();
  }
}

// Singleton instance
export const ssaiMetricsExporter = new SSAIPrometheusExporter();
