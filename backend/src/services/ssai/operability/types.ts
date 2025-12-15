/**
 * SSAI Operability Types
 *
 * Types and interfaces for operability features including
 * kill switches, rollout management, safe mode, and SLI metrics.
 */

// Kill Switch Types
export interface KillSwitchConfig {
  /** Kill switch ID */
  id: string;

  /** Kill switch name */
  name: string;

  /** Current state */
  enabled: boolean;

  /** Scope of the kill switch */
  scope: KillSwitchScope;

  /** Last updated */
  lastUpdated: Date;

  /** Updated by */
  updatedBy: string;

  /** Reason for current state */
  reason?: string;

  /** Auto-restore time (if temporary) */
  autoRestoreAt?: Date;
}

export type KillSwitchScope =
  | { type: 'global' }
  | { type: 'tenant'; tenantId: string }
  | { type: 'partner'; partnerId: string }
  | { type: 'creative'; creativeId: string }
  | { type: 'region'; regionCode: string };

export interface KillSwitchResult {
  /** Whether the operation should proceed */
  proceed: boolean;

  /** Active kill switches */
  activeKillSwitches: string[];

  /** Fallback action */
  fallbackAction?: 'slate' | 'passthrough' | 'reject';
}

// Rollout Types
export interface RolloutConfig {
  /** Feature name */
  featureName: string;

  /** Current rollout percentage (0-100) */
  percentage: number;

  /** Rollout strategy */
  strategy: RolloutStrategy;

  /** Sticky key (for consistent assignment) */
  stickyKey?: 'session' | 'device' | 'tenant';

  /** Enabled regions */
  enabledRegions?: string[];

  /** Disabled regions */
  disabledRegions?: string[];

  /** Minimum version (for SDK features) */
  minVersion?: string;

  /** Last updated */
  lastUpdated: Date;
}

export type RolloutStrategy =
  | 'percentage'
  | 'allowlist'
  | 'denylist'
  | 'canary'
  | 'geographic';

export interface RolloutDecision {
  /** Whether feature is enabled */
  enabled: boolean;

  /** Variant (for A/B tests) */
  variant?: string;

  /** Rollout config used */
  config: RolloutConfig;

  /** Decision reason */
  reason: string;
}

// Safe Mode Types
export interface SafeModeConfig {
  /** Whether safe mode is active */
  active: boolean;

  /** Trigger threshold (error rate %) */
  errorRateThreshold: number;

  /** Trigger threshold (latency p99 ms) */
  latencyThreshold: number;

  /** Minimum samples before triggering */
  minimumSamples: number;

  /** Auto-recovery enabled */
  autoRecovery: boolean;

  /** Recovery check interval (seconds) */
  recoveryCheckSeconds: number;

  /** Safe mode behavior */
  behavior: SafeModeBehavior;
}

export interface SafeModeBehavior {
  /** Skip decision calls */
  skipDecisions: boolean;

  /** Use slate for all slots */
  useSlate: boolean;

  /** Reduce partner timeout */
  reducedTimeout?: number;

  /** Disable non-critical features */
  disableNonCritical: boolean;
}

export interface SafeModeStatus {
  /** Whether safe mode is currently active */
  isActive: boolean;

  /** Activation reason */
  reason?: string;

  /** Activated at */
  activatedAt?: Date;

  /** Current metrics */
  currentMetrics: {
    errorRate: number;
    latencyP99: number;
    sampleCount: number;
  };

  /** Recovery status */
  recoveryStatus?: {
    checking: boolean;
    lastCheck: Date;
    checksPassed: number;
    checksRequired: number;
  };
}

// SLI Types
export interface SLIDefinition {
  /** SLI name */
  name: string;

  /** Description */
  description: string;

  /** Metric type */
  type: 'latency' | 'availability' | 'error_rate' | 'throughput';

  /** Target value */
  target: number;

  /** Unit */
  unit: string;

  /** Window size (seconds) */
  windowSeconds: number;
}

export interface SLIMetric {
  /** SLI name */
  name: string;

  /** Current value */
  value: number;

  /** Target value */
  target: number;

  /** Whether meeting target */
  meetingTarget: boolean;

  /** Error budget remaining (%) */
  errorBudgetRemaining: number;

  /** Trend */
  trend: 'improving' | 'stable' | 'degrading';

  /** Last updated */
  lastUpdated: Date;
}

export interface SLIReport {
  /** Report time */
  timestamp: Date;

  /** Window period */
  window: {
    start: Date;
    end: Date;
    seconds: number;
  };

  /** Metrics */
  metrics: SLIMetric[];

  /** Overall health */
  overallHealth: 'healthy' | 'degraded' | 'unhealthy';

  /** Recommendations */
  recommendations: string[];
}

// Explain Types
export interface ExplainPodRequest {
  /** Session ID */
  sessionId: string;

  /** Pod ID */
  podId: string;

  /** Include full decision trace */
  includeDecisionTrace: boolean;

  /** Include manifest samples */
  includeManifestSamples: boolean;
}

export interface ExplainPodResponse {
  /** Session info */
  session: {
    sessionId: string;
    tenantId: string;
    deviceType: string;
    capabilities: string[];
  };

  /** Break info */
  break: {
    breakId: string;
    cueType: string;
    cueTime: Date;
    duration: number;
  };

  /** Decision trace */
  decisionTrace: ExplainDecisionStep[];

  /** Final pod */
  finalPod: {
    slotCount: number;
    slots: ExplainSlot[];
  };

  /** Manifest info */
  manifest?: {
    type: 'HLS' | 'DASH';
    sampleUrl?: string;
  };

  /** Warnings */
  warnings: string[];
}

export interface ExplainDecisionStep {
  /** Step name */
  step: string;

  /** Step order */
  order: number;

  /** Input */
  input: unknown;

  /** Output */
  output: unknown;

  /** Duration (ms) */
  durationMs: number;

  /** Notes */
  notes?: string[];
}

export interface ExplainSlot {
  /** Slot index */
  index: number;

  /** Slot ID */
  slotId: string;

  /** Creative ID (if filled) */
  creativeId?: string;

  /** Partner ID */
  partnerId?: string;

  /** Fill reason */
  fillReason: string;

  /** Rejected options */
  rejectedOptions?: Array<{
    creativeId: string;
    reason: string;
  }>;
}

// Slate Types
export interface SlateConfig {
  /** Slate ID */
  id: string;

  /** Slate name */
  name: string;

  /** Default slate for tenant */
  isDefault: boolean;

  /** Media files by format */
  mediaFiles: SlateMediaFile[];

  /** Duration (seconds) */
  duration: number;

  /** Loop if needed */
  loopable: boolean;
}

export interface SlateMediaFile {
  /** Container format */
  container: 'ts' | 'cmaf';

  /** Codec */
  codec: string;

  /** Resolution */
  resolution: { width: number; height: number };

  /** Bitrate */
  bitrate: number;

  /** URL */
  url: string;
}

// Alert Types
export interface AlertConfig {
  /** Alert name */
  name: string;

  /** Severity */
  severity: 'info' | 'warning' | 'critical';

  /** Condition */
  condition: AlertCondition;

  /** Notification channels */
  channels: string[];

  /** Cooldown period (seconds) */
  cooldownSeconds: number;

  /** Auto-resolve after (seconds) */
  autoResolveSeconds?: number;
}

export interface AlertCondition {
  /** Metric to check */
  metric: string;

  /** Operator */
  operator: 'gt' | 'gte' | 'lt' | 'lte' | 'eq' | 'neq';

  /** Threshold value */
  threshold: number;

  /** Duration for condition to be true */
  forSeconds: number;
}

export interface Alert {
  /** Alert ID */
  id: string;

  /** Alert name */
  name: string;

  /** Severity */
  severity: 'info' | 'warning' | 'critical';

  /** Status */
  status: 'firing' | 'resolved';

  /** Message */
  message: string;

  /** First fired at */
  firedAt: Date;

  /** Resolved at */
  resolvedAt?: Date;

  /** Metric value at trigger */
  metricValue: number;
}
