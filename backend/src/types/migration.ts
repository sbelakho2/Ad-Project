/**
 * Type definitions for Migration Studio
 */

/**
 * Experiment lifecycle states:
 * - draft: Initial creation, not yet activated
 * - active: Running and collecting data
 * - paused: Temporarily stopped (recoverable, can be resumed)
 * - completed: Successfully finished migration
 * - failed: Guardrail kill switch triggered (terminal state, requires re-creation)
 * - archived: Soft-deleted, kept for audit purposes
 */
export type ExperimentStatus = 'draft' | 'active' | 'paused' | 'completed' | 'failed' | 'archived';
export type ExperimentObjective = 'revenue_comparison' | 'fill_rate' | 'latency';
export type ExperimentMode = 'shadow' | 'mirroring';
export type MappingStatus = 'pending' | 'confirmed' | 'skipped' | 'conflict';
export type MappingConfidence = 'high' | 'medium' | 'low';
export type ExperimentArm = 'control' | 'test';
export type EventType = 'assignment' | 'guardrail_pause' | 'guardrail_kill' | 'activation' | 'deactivation';

export interface Guardrails {
  latency_budget_ms: number;
  revenue_floor_percent: number; // Negative values allowed (e.g., -10 = 10% revenue drop tolerance)
  max_error_rate_percent: number;
  min_impressions: number; // Minimum sample size before evaluating guardrails
}

export interface MigrationExperiment {
  id: string;
  publisher_id: string;
  name: string;
  description?: string;
  app_id?: string;
  placement_id?: string;
  
  // Configuration
  objective: ExperimentObjective;
  seed: string;
  mirror_percent: number; // 0-20
  mode: ExperimentMode;
  
  // Status
  status: ExperimentStatus;
  activated_at?: Date;
  paused_at?: Date;
  completed_at?: Date;
  last_guardrail_check?: Date;
  
  // Guardrails
  guardrails: Guardrails;
  
  // Metadata
  created_by?: string;
  created_at: Date;
  updated_at: Date;
}

export interface MigrationMapping {
  id: string;
  experiment_id: string;
  
  // Incumbent details
  incumbent_network: string;
  incumbent_instance_id: string;
  incumbent_instance_name?: string;
  incumbent_waterfall_position?: number;
  incumbent_ecpm_cents?: number;
  
  // Our mapping
  our_adapter_id?: string;
  our_adapter_name?: string;
  mapping_status: MappingStatus;
  mapping_confidence?: MappingConfidence;
  
  // Conflict resolution
  conflict_reason?: string;
  resolved_by?: string;
  resolved_at?: Date;
  
  // Metadata
  created_at: Date;
  updated_at: Date;
}

export interface MigrationImportSummary {
  total_mappings: number;
  status_breakdown: Record<MappingStatus, number>;
  confidence_breakdown: Record<MappingConfidence, number>;
  unique_networks: number;
}

export interface MigrationSignedComparisonMetric {
  label: string;
  unit: 'currency_cents' | 'percent' | 'milliseconds';
  control: number;
  test: number;
  uplift_percent: number;
}

export interface MigrationSignedComparisonConfidenceBand {
  lower: number;
  upper: number;
  confidence_level: number;
  method: string;
}

export interface MigrationSignedComparison {
  generated_at: string;
  sample_size: {
    control_impressions: number;
    test_impressions: number;
  };
  metrics: {
    ecpm: MigrationSignedComparisonMetric;
    fill: MigrationSignedComparisonMetric;
    latency_p50: MigrationSignedComparisonMetric;
    latency_p95: MigrationSignedComparisonMetric;
    ivt_adjusted_revenue: MigrationSignedComparisonMetric;
  };
  confidence_band: MigrationSignedComparisonConfidenceBand;
  signature: {
    key_id: string;
    algo: 'ed25519';
    payload_base64: string;
    signature_base64: string;
    public_key_base64: string;
    not_before?: string | null;
    not_after?: string | null;
  };
}

export interface MigrationEvent {
  id: string;
  experiment_id: string;
  event_type: EventType;
  arm?: ExperimentArm;
  user_identifier?: string;
  placement_id?: string;
  event_data: Record<string, unknown>;
  reason?: string;
  triggered_by?: string;
  created_at: Date;
}

export interface MigrationAuditLog {
  id: string;
  experiment_id?: string;
  user_id: string;
  action: string;
  resource_type: string;
  resource_id?: string;
  old_value?: Record<string, unknown>;
  new_value?: Record<string, unknown>;
  ip_address?: string;
  user_agent?: string;
  created_at: Date;
}

export interface MigrationReportToken {
  id: string;
  experiment_id: string;
  token: string;
  expires_at: Date;
  created_by?: string;
  created_at: Date;
  last_accessed_at?: Date;
  access_count: number;
}

export interface MigrationImport {
  id: string;
  publisher_id: string;
  experiment_id?: string;
  placement_id?: string;
  source: string;
  status: 'draft' | 'pending_review' | 'completed' | 'failed';
  summary: MigrationImportSummary;
  error_message?: string;
  created_by?: string;
  created_at: Date;
  updated_at: Date;
  completed_at?: Date;
}

export interface MigrationGuardrailSnapshot {
  id: string;
  experiment_id: string;
  captured_at: Date;
  arm: ExperimentArm;
  impressions: number;
  fills: number;
  revenue_micros: number;
  latency_p95_ms?: number;
  latency_p50_ms?: number;
  error_rate_percent?: number;
  ivt_rate_percent?: number;
  rolling_window_minutes: number;
}

// Request/Response DTOs
export interface CreateExperimentRequest {
  name: string;
  description?: string;
  app_id?: string;
  placement_id?: string;
  objective?: ExperimentObjective;
  seed?: string;
  guardrails?: Partial<Guardrails>;
}

export interface UpdateExperimentRequest {
  name?: string;
  description?: string;
  mirror_percent?: number;
  guardrails?: Partial<Guardrails>;
}

export interface ActivateExperimentRequest {
  mirror_percent: number; // 0-20
}

export interface ImportMappingsRequest {
  experiment_id: string;
  source: 'csv' | 'api'; // 'ironSource' | 'MAX' for API source
  data: unknown; // CSV parsed data or API response
}

export interface ConfirmMappingRequest {
  mapping_id: string;
  our_adapter_id: string;
  notes?: string;
}

export interface ExperimentMetrics {
  arm: ExperimentArm;
  impressions: number;
  fills: number;
  revenue_micros: number;
  latency_p50: number;
  latency_p95: number;
  latency_p99: number;
  error_rate: number;
  ivt_rate: number;
}

export interface ComparisonReport {
  experiment: MigrationExperiment;
  period: {
    start: Date;
    end: Date;
  };
  control: ExperimentMetrics;
  test: ExperimentMetrics;
  uplift: {
    revenue_percent: number;
    fill_percent: number;
    latency_p95_ms: number;
    ivt_reduction_percent: number;
  };
  statistical_significance: {
    revenue_pvalue: number;
    fill_pvalue: number;
    confident: boolean;
  };
  projection: {
    if_100_percent_test: {
      revenue_uplift_micros: number;
      revenue_uplift_percent: number;
    };
  };
}

export interface CreateReportTokenRequest {
  experiment_id: string;
  expires_in_hours?: number; // Default 72 hours
}

export interface CreateReportTokenResponse {
  token: string;
  expires_at: Date;
  url: string;
}

export interface EvaluateGuardrailsResult {
  shouldPause: boolean;
  violations: string[];
}

// Dry-run simulation types
export interface DryRunSimulationRequest {
  mirror_percent: number;
  simulation_hours?: number; // Default 24 hours
}

export interface DryRunSimulationResult {
  simulation_id: string;
  experiment_id: string;
  mirror_percent: number;
  simulation_hours: number;
  timestamp: Date;
  
  // Pre-flight checks
  preflight: {
    passed: boolean;
    checks: Array<{
      name: string;
      status: 'pass' | 'fail' | 'warn';
      message: string;
      severity: 'critical' | 'warning' | 'info';
    }>;
  };
  
  // Estimated traffic impact
  traffic_impact: {
    estimated_total_requests: number;
    estimated_test_requests: number;
    estimated_control_requests: number;
    test_percentage: number;
    placements_affected: number;
    apps_affected: number;
  };
  
  // Mapping readiness
  mapping_readiness: {
    total_mappings: number;
    confirmed_mappings: number;
    pending_mappings: number;
    skipped_mappings: number;
    conflict_mappings: number;
    coverage_percent: number;
    unmapped_networks: string[];
  };
  
  // Guardrail configuration review
  guardrail_review: {
    latency_budget_ms: number;
    latency_budget_status: 'conservative' | 'moderate' | 'aggressive';
    revenue_floor_percent: number;
    revenue_floor_status: 'conservative' | 'moderate' | 'aggressive';
    max_error_rate_percent: number;
    error_rate_status: 'conservative' | 'moderate' | 'aggressive';
    min_impressions: number;
    estimated_time_to_min_impressions_hours: number;
  };
  
  // Risk assessment
  risk_assessment: {
    overall_risk: 'low' | 'medium' | 'high';
    risk_factors: Array<{
      factor: string;
      level: 'low' | 'medium' | 'high';
      description: string;
      mitigation?: string;
    }>;
  };
  
  // Rollback plan
  rollback_plan: {
    automatic_rollback_enabled: boolean;
    rollback_triggers: string[];
    estimated_rollback_time_seconds: number;
  };
  
  // Recommendations
  recommendations: Array<{
    type: 'action' | 'warning' | 'info';
    message: string;
    priority: 'high' | 'medium' | 'low';
  }>;
}

