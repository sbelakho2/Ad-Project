/**
 * SSAI Operability Module
 *
 * Explainability, control plane, and reliability features for SSAI.
 * Provides:
 * - ExplainService - Pod/break/no-fill explanation and debugging
 * - KillSwitchManager - Per-tenant/channel/region kill switches
 * - RolloutManager - Canary rollouts, percentage rollouts, A/B testing
 * - SafeModeManager - Automatic degradation and reduced fanout
 * - SLIManager - SSAI-specific SLIs, error budgets, and reporting
 * - SlateManager - Graceful degradation with slate content
 */

export const OPERABILITY_MODULE_VERSION = '1.0.0';

// Types
export * from './types';

// Kill Switch Management
export { KillSwitchManager, KillSwitchManagerConfig } from './KillSwitchManager';

// Rollout Management
export { RolloutManager, RolloutManagerConfig } from './RolloutManager';

// Safe Mode Management
export { SafeModeManager, SafeModeManagerConfig } from './SafeModeManager';

// SLI Metrics
export { SLIManager, SLIManagerConfig } from './SLIManager';

// Explain Service
export { ExplainService, ExplainServiceConfig } from './ExplainService';

// Slate Management
export { SlateManager, SlateManagerConfig } from './SlateManager';

// Health Check
export { SSAIHealthCheck } from './SSAIHealthCheck';

// Metrics
export { SSAIMetrics } from './SSAIMetrics';

// Debugger
export { SSAIDebugger } from './SSAIDebugger';

// Logger
export { SSAILogger } from './SSAILogger';

// Prometheus Exporter
export { SSAIPrometheusExporter, ssaiMetricsExporter } from './SSAIPrometheusExporter';

