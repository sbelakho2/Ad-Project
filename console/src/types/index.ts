// Publisher types
export interface Publisher {
  id: string
  name: string
  email: string
  status: 'active' | 'suspended' | 'pending'
  createdAt: string
  balance: number
  revenueThisMonth: number
  impressionsThisMonth: number
}

// Placement types
export interface Placement {
  id: string
  name: string
  type: 'banner' | 'interstitial' | 'rewarded'
  status: 'active' | 'paused' | 'archived'
  format: string
  platformId: string
  publisherId: string
  createdAt: string
  updatedAt: string
  config?: {
    supplyChain?: {
      domain: string
      sellerId?: string
      appStoreId?: string
      siteId?: string
    }
    [key: string]: any
  }
  supplyChainStatus?: SupplyChainStatusResult
}

// Adapter types
export interface Adapter {
  id: string
  name: string
  network: string
  status: 'active' | 'inactive' | 'testing'
  priority: number
  ecpm: number
  fillRate: number
  requestCount: number
  impressionCount: number
  placementId: string
  createdAt: string
  updatedAt: string
}

// Tools: app-ads.txt inspector
export interface AppAdsInspectorVendorResult {
  vendor: string
  pass: boolean
  missing: string[]
  suggested: string[]
}

export interface AppAdsInspectorResult {
  domain: string
  fetched: boolean
  httpStatus?: number
  vendors: AppAdsInspectorVendorResult[]
  rawSample?: string
}

// Tools: supply chain status
export interface SupplyChainStatusEntry {
  sellerId: string
  relationship?: string
  appStoreId?: string
  siteId?: string
}

export interface SupplyChainStatusResult {
  domain: string
  sellerId?: string
  appStoreId?: string
  siteId?: string
  authorized: boolean
  reason?: string
  sellerInfo?: { sellerId: string; domain?: string; name?: string; status?: 'active' | 'inactive' }
  entries?: SupplyChainStatusEntry[]
}

export interface SupplyChainPlacementStatus {
  placementId: string
  placementName: string
  domain?: string
  sellerId?: string
  appStoreId?: string
  siteId?: string
  authorized: boolean
  reason?: string
}

export interface SupplyChainAppSummary {
  appId: string
  ok: number
  issues: number
  missingDomain: number
  placements: SupplyChainPlacementStatus[]
}

export interface SupplyChainSummaryResponse {
  summary: {
    publisherId: string
    generatedAt: string
    apps: SupplyChainAppSummary[]
  }
  snapshotPath?: string
  snapshotId?: string
  persistedAt?: string
}

// Revenue types
export interface RevenueData {
  date: string
  revenue: number
  impressions: number
  clicks: number
  ecpm: number
  fillRate: number
}

// Analytics types
export interface AnalyticsSummary {
  totalRevenue: number
  totalImpressions: number
  totalClicks: number
  averageEcpm: number
  averageFillRate: number
  periodStart: string
  periodEnd: string
  revenueChangePercent?: number
  impressionsChangePercent?: number
  ecpmChangePercent?: number
  fillRateChangePercent?: number
}

// Fraud types
export interface FraudAlert {
  id: string
  timestamp: string
  alertType: 'givt' | 'sivt' | 'ml_fraud' | 'anomaly'
  severity: 'low' | 'medium' | 'high' | 'critical'
  publisherId: string
  deviceId?: string
  ipAddress?: string
  fraudScore: number
  detectionType: 'rule_based' | 'ml_based' | 'anomaly'
  action: 'block' | 'flag' | 'monitor'
  metadata: Record<string, any>
}

export interface FraudStats {
  publisherId: string
  timeWindow: string
  totalRequests: number
  fraudRequests: number
  fraudRate: number
  givtDetections: number
  sivtDetections: number
  mlDetections: number
  anomalyDetections: number
  blockedRequests: number
  flaggedRequests: number
  averageFraudScore: number
  topFraudTypes: string[]
  lastUpdated: string
}

// Payment types
export interface PayoutHistory {
  id: string
  amount: number
  currency: string
  status: 'pending' | 'processing' | 'completed' | 'failed'
  method: 'stripe' | 'paypal' | 'wire'
  scheduledFor: string
  processedAt?: string
}

export interface FraudSettings {
  alertEmails: string[]
  warningThreshold: number
  blockThreshold: number
  autoBlock: boolean
  webhookUrl?: string
}

export interface PayoutSettings {
  threshold: number
  method: 'stripe' | 'paypal' | 'wire'
  currency: string
  schedule: 'monthly'
  accountName: string
  accountReference: string
  autoPayout: boolean
  backupMethod?: 'stripe' | 'paypal' | 'wire'
  updatedAt: string
}

// Team types
export interface TeamMember {
  id: string
  email: string
  name: string
  role: 'owner' | 'admin' | 'developer' | 'finance'
  status: 'active' | 'invited' | 'suspended'
  invitedAt?: string
  joinedAt?: string
  lastActiveAt?: string
  permissions: string[]
}

export interface TeamInvitation {
  email: string
  role: TeamMember['role'] | string
  permissions?: string[]
}

export interface TeamRoleDefinition {
  id: string
  label: string
  description: string
  permissions?: string[]
  invitable?: boolean
}

// Notification types
export interface NotificationSettings {
  emailAlerts: {
    enabled: boolean
    recipients: string[]
    events: string[]
  }
  slackIntegration: {
    enabled: boolean
    webhookUrl?: string
    channel?: string
    events: string[]
  }
  webhooks: {
    enabled: boolean
    endpoints: WebhookEndpoint[]
  }
  digest: {
    enabled: boolean
    frequency: 'daily' | 'weekly' | 'monthly'
    recipients: string[]
  }
}

export interface WebhookEndpoint {
  id: string
  url: string
  events: string[]
  secret: string
  active: boolean
  createdAt: string
}

// Compliance types
export interface ComplianceSettings {
  gdprEnabled: boolean
  ccpaEnabled: boolean
  coppaMode: boolean
  consentManagement: {
    provider?: 'custom' | 'onetrust' | 'cookiebot'
    autoBlock: boolean
    consentString?: string
    encryptedConsent?: {
      algorithm: 'AES-GCM'
      keyId: string
      iv: string
      cipherText: string
    }
  }
  dataRetention: {
    rawEventsDays: number
    aggregatedDataDays: number
    userDataDays: number
  }
  regionalSettings: {
    euTrafficOnly: boolean
    blockedCountries: string[]
    sensitiveCategories: string[]
  }
}

// API Response types
export interface ApiResponse<T> {
  data: T
  message?: string
  error?: string
}

export interface PaginatedResponse<T> {
  data: T[]
  total: number
  page: number
  pageSize: number
  hasMore: boolean
}

// Auth types
export interface User {
  id: string
  email: string
  name: string
  role: 'publisher' | 'admin'
  publisherId?: string
  avatar?: string
}

export interface LoginRequest {
  email: string
  password: string
}

export interface LoginResponse {
  user: User
  token: string
  expiresAt: string
}

// Migration Studio types
export type MigrationExperimentStatus = 'draft' | 'active' | 'paused' | 'completed' | 'archived'

export type MigrationMappingStatus = 'pending' | 'confirmed' | 'skipped' | 'conflict'
export type MigrationMappingConfidence = 'high' | 'medium' | 'low'
export type MigrationImportSource = 'csv' | 'ironSource' | 'applovin'
export type MigrationImportStatus = 'draft' | 'pending_review' | 'completed' | 'failed'

export interface MigrationGuardrails {
  latency_budget_ms?: number
  revenue_floor_percent?: number
  max_error_rate_percent?: number
  min_impressions?: number
}

export interface MigrationExperiment {
  id: string
  publisher_id: string
  name: string
  description?: string
  app_id?: string
  placement_id?: string
  objective: 'revenue_comparison' | 'fill_rate' | 'latency'
  seed: string
  mirror_percent: number
  mode?: 'shadow' | 'mirroring'
  status: MigrationExperimentStatus
  activated_at?: string
  paused_at?: string
  completed_at?: string
  guardrails: MigrationGuardrails | null
  last_guardrail_check?: string
  created_by?: string
  created_at: string
  updated_at: string
}

export interface MigrationMapping {
  id: string
  experiment_id: string
  incumbent_network: string
  incumbent_instance_id: string
  incumbent_instance_name?: string
  incumbent_waterfall_position?: number
  incumbent_ecpm_cents?: number
  mapping_status: MigrationMappingStatus
  mapping_confidence?: MigrationMappingConfidence
  our_adapter_id?: string
  our_adapter_name?: string
  conflict_reason?: string
  created_at: string
  updated_at: string
}

export interface MigrationImportResponse {
  import_id: string
  experiment_id?: string
  placement_id?: string
  source: MigrationImportSource
  status: MigrationImportStatus
  created_at: string
  mappings: MigrationMapping[]
  summary: MigrationImportSummary
  signed_comparison?: MigrationSignedComparison
}

export interface MigrationImportSummary {
  total_mappings: number
  status_breakdown: Record<MigrationMappingStatus, number>
  confidence_breakdown: Record<MigrationMappingConfidence, number>
  unique_networks: number
}

export interface MigrationSignedComparisonMetric {
  label: string
  unit: 'currency_cents' | 'percent' | 'milliseconds'
  control: number
  test: number
  uplift_percent: number
}

export interface MigrationSignedComparisonConfidenceBand {
  lower: number
  upper: number
  confidence_level: number
  method: string
}

export interface MigrationSignedComparison {
  generated_at: string
  sample_size: {
    control_impressions: number
    test_impressions: number
  }
  metrics: {
    ecpm: MigrationSignedComparisonMetric
    fill: MigrationSignedComparisonMetric
    latency_p50: MigrationSignedComparisonMetric
    latency_p95: MigrationSignedComparisonMetric
    ivt_adjusted_revenue: MigrationSignedComparisonMetric
  }
  confidence_band: MigrationSignedComparisonConfidenceBand
  signature: {
    key_id: string
    algo: 'ed25519'
    payload_base64: string
    signature_base64: string
    public_key_base64: string
  }
}

export interface MigrationMappingUpdateResponse {
  mapping: MigrationMapping
  summary: MigrationImportSummary
}

export interface EvaluateGuardrailsResponse {
  shouldPause: boolean
  violations: string[]
}

export type MigrationMetricUnit = 'currency_cents' | 'percent' | 'milliseconds' | 'ratio' | 'count'

export interface MigrationExperimentMetric {
  id: string
  label: string
  description?: string
  unit: MigrationMetricUnit
  control: number | null
  test: number | null
  uplift: number | null
  sample_size?: {
    control?: number
    test?: number
  }
}

export interface MigrationExperimentTimeseriesPoint {
  timestamp: string
  control: number | null
  test: number | null
}

export interface MigrationExperimentMetricTimeseries {
  id: string
  label: string
  unit: MigrationMetricUnit
  points: MigrationExperimentTimeseriesPoint[]
}

export interface MigrationExperimentReportWindow {
  start: string
  end: string
  timezone?: string
}

export interface MigrationExperimentReport {
  experiment_id: string
  generated_at: string
  window: MigrationExperimentReportWindow
  metrics: {
    overall: MigrationExperimentMetric[]
    timeseries?: MigrationExperimentMetricTimeseries[]
  }
}

export interface MigrationExperimentShareLink {
  id: string
  url: string
  expires_at: string
  created_at: string
  created_by?: string
  last_accessed_at?: string
}

// SDK Integration types
export interface MigrationSdkSnippet {
  language: string
  code: string
  instructions: string
}

export interface MigrationSdkIntegration {
  experiment_id: string
  placement_id: string
  adapters: Array<{ id: string; name: string }>
  api_endpoint: string
  snippets: Record<string, MigrationSdkSnippet>
}

export interface MigrationMapping {
  id: string
  experiment_id: string
  incumbent_network: string
  incumbent_instance_id: string
  incumbent_instance_name?: string
  incumbent_waterfall_position?: number
  incumbent_ecpm_cents?: number
  our_adapter_id?: string
  our_adapter_name?: string
  mapping_status: 'pending' | 'confirmed' | 'skipped' | 'conflict'
  mapping_confidence?: 'high' | 'medium' | 'low'
  conflict_reason?: string
  resolved_by?: string
  resolved_at?: string
  created_at: string
  updated_at: string
}

// BYO (Bring Your Own) Network Credentials types
export interface NetworkCredential {
  id: string
  network: 'admob' | 'unity' | 'applovin' | 'ironsource' | 'mintegral' | 'facebook'
  version: number
  createdAt: string
  updatedAt: string
  hasCredentials: boolean
}

export interface NetworkCredentialInput {
  network: string
  credentials: Record<string, unknown>
}

export interface NetworkCredentialToken {
  token: string
  expiresAt: string
  expiresIn: number
}

export interface NetworkCredentialsList {
  credentials: NetworkCredential[]
}

export interface NetworkVerificationResult {
  network: string
  verified: boolean
  checkedAt: string
  issues: string[]
  snapshotCount?: number
  sampleWindow?: { start: string; end: string }
}

export interface NetworkIngestionResult {
  success: boolean
  rowsProcessed: number
  rowsInserted: number
  rowsSkipped: number
  errors: string[]
  startDate: string
  endDate: string
}

export interface FraudDiagnostics {
  mode: 'shadow' | 'block'
  updatedAt: string
  histogramBuckets: string[]
  histogramValues: number[]
  drift: {
    current: number
    trend: Array<{ timestamp: string; value: number }>
  }
  weakLabels: Array<{ label: string; correlation: number }>
  promotionSuggestion?: {
    recommendedMode: 'shadow' | 'block'
    confidence: number
    reason: string
    blockers?: string[]
  }
}

// App API Keys
export interface AppApiKey {
  id: string
  appId: string
  environment: 'sandbox' | 'live'
  prefix: string
  last4: string
  createdAt: string
  lastUsedAt: string | null
  revokedAt: string | null
}

export interface AppApiKeyWithSecret extends AppApiKey {
  secret: string
}

export interface App {
  id: string
  publisherId: string
  bundleId: string
  name: string
  platform: 'android' | 'ios' | 'web' | 'tvos' | 'android-tv'
  status: 'active' | 'paused'
  createdAt: string
  updatedAt: string
}
