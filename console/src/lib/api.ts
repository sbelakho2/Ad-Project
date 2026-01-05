import { apiClient, fraudApiClient, analyticsApiClient } from './api-client'
import type {
  Publisher,
  Placement,
  Adapter,
  RevenueData,
  AnalyticsSummary,
  FraudAlert,
  FraudStats,
  PaginatedResponse,
  FraudSettings,
  TeamMember,
  TeamInvitation,
  TeamRoleDefinition,
  NotificationSettings,
  ComplianceSettings,
  ApiResponse,
  MigrationExperiment,
  MigrationImportResponse,
  MigrationImportSource,
  MigrationMapping,
  MigrationMappingUpdateResponse,
  MigrationGuardrails,
  EvaluateGuardrailsResponse,
  MigrationExperimentReport,
  MigrationExperimentShareLink,
  NetworkCredential,
  NetworkCredentialInput,
  NetworkCredentialToken,
  NetworkCredentialsList,
  NetworkVerificationResult,
  NetworkIngestionResult,
  AppAdsInspectorResult,
  SupplyChainStatusResult,
  SupplyChainSummaryResponse,
} from '@/types'

const USE_MOCK_API = process.env.NEXT_PUBLIC_USE_MOCK_API === 'true' && process.env.NODE_ENV !== 'production'
const DEFAULT_PAGE = 1
const DEFAULT_PAGE_SIZE = 25
const MAX_IMPORT_FILE_SIZE_BYTES = 5 * 1024 * 1024 // 5 MB
const ALLOWED_IMPORT_MIME_TYPES = ['text/csv', 'application/vnd.ms-excel', 'application/octet-stream']

type PaginationParams = { page?: number; pageSize?: number }

const withPaginationDefaults = (params?: PaginationParams): Required<PaginationParams> => ({
  page: params?.page ?? DEFAULT_PAGE,
  pageSize: params?.pageSize ?? DEFAULT_PAGE_SIZE,
})

const unwrapApiResponse = async <T>(request: Promise<{ data: ApiResponse<T> }>): Promise<T> => {
  const response = await request
  return response.data.data
}

const validateCsvFile = (file?: File) => {
  if (!file) {
    throw new Error('A CSV export is required for this import.')
  }

  if (file.size > MAX_IMPORT_FILE_SIZE_BYTES) {
    throw new Error('Import files must be 5 MB or smaller.')
  }

  if (file.type && !ALLOWED_IMPORT_MIME_TYPES.includes(file.type)) {
    throw new Error('Only CSV uploads are supported.')
  }
}

// Helper to create mock API calls
const mockApiCall = async <T>(endpoint: string): Promise<{ data: T }> => {
  if (!USE_MOCK_API) {
    throw new Error('Mock API not enabled')
  }

  const response = await fetch(`/api/mock?endpoint=${endpoint}`, { cache: 'no-store' })
  if (!response.ok) {
    const message = await response.text().catch(() => null)
    throw new Error(message || 'Mock API error')
  }
  const data = await response.json()
  return { data }
}

type SuccessEnvelope<T> = {
  success: boolean
  data: T
}

const unwrapSuccessEnvelope = <T>(payload: T | SuccessEnvelope<T>): T => {
  if (
    payload &&
    typeof payload === 'object' &&
    'success' in payload &&
    typeof (payload as SuccessEnvelope<T>).success === 'boolean' &&
    'data' in payload
  ) {
    return (payload as SuccessEnvelope<T>).data
  }
  return payload as T
}

const unwrapResponseData = <T>(payload: unknown): T => {
  const unwrapped = unwrapSuccessEnvelope(payload as any)
  if (isRecord(unwrapped) && 'data' in unwrapped) {
    return (unwrapped as any).data as T
  }
  return unwrapped as T
}

const coerceNumber = (value: unknown): number => {
  if (typeof value === 'number') return Number.isFinite(value) ? value : 0
  if (typeof value === 'string') {
    const parsed = Number(value)
    return Number.isFinite(parsed) ? parsed : 0
  }
  return 0
}

const isRecord = (value: unknown): value is Record<string, any> => Boolean(value) && typeof value === 'object'

// Publisher API
export const publisherApi = {
  getCurrent: () => apiClient.get<Publisher>('/publishers/me'),
  update: (data: Partial<Publisher>) => apiClient.put<Publisher>('/publishers/me', data),
}

// Placement API
export const placementApi = {
  list: async (params?: { page?: number; pageSize?: number }): Promise<PaginatedResponse<Placement>> => {
    if (USE_MOCK_API) {
      const mockResp = await mockApiCall<PaginatedResponse<Placement>>('placements')
      return mockResp.data
    }
    const pagination = withPaginationDefaults(params)
    const resp = await apiClient.get('/placements', { params: { ...pagination, includeSupplyChainStatus: true } })
    const payload: any = resp.data
    const base = payload?.data ?? payload ?? {}
    const items = base.items ?? base.data ?? []
    return {
      data: items as Placement[],
      total: base.total ?? items.length,
      page: base.page ?? pagination.page,
      pageSize: base.pageSize ?? pagination.pageSize,
      hasMore: Boolean(base.hasMore ?? (items.length === pagination.pageSize)),
    } satisfies PaginatedResponse<Placement>
  },
  get: async (id: string) => {
    const resp = await apiClient.get(`/placements/${id}`, { params: { includeSupplyChainStatus: true } })
    return unwrapSuccessEnvelope(resp.data as any) as Placement
  },
  create: (data: Partial<Placement>) => apiClient.post<Placement>('/placements', data),
  update: (id: string, data: Partial<Placement>) =>
    apiClient.put<Placement>(`/placements/${id}`, data),
  delete: (id: string) => apiClient.delete(`/placements/${id}`),
  getFormatCatalog: async (): Promise<Record<string, string[]>> => {
    if (USE_MOCK_API) {
      const mockResp = await mockApiCall<Record<string, string[]>>('placement-formats')
      return mockResp.data
    }
    const resp = await apiClient.get<Record<string, string[]>>('/placements/formats')
    return resp.data
  },
  getSupplyChainSummary: async (): Promise<SupplyChainSummaryResponse> => {
    const resp = await apiClient.get<SuccessEnvelope<SupplyChainSummaryResponse> | SupplyChainSummaryResponse>(`/tools/supply-chain/summary`)
    return unwrapSuccessEnvelope(resp.data as any)
  },
}

// BYO Observability (Adapter Spans)
export const analyticsByo = {
  getAdapterMetrics: async (params: { appId: string; placement?: string; adapter?: string; from?: number; to?: number }) => {
    const resp = await analyticsApiClient.get('/analytics/byo/adapter-metrics', { params })
    return resp.data as { summary: { p50: number; p95: number; p99: number; fills: number; noFills: number; timeouts: number; errors: number; total: number } }
  },
  getTraces: async (params: { appId: string; placement?: string; adapter?: string; from?: number; to?: number; limit?: number }) => {
    const resp = await analyticsApiClient.get('/analytics/byo/traces', { params })
    return resp.data as { traces: Array<{ trace_id: string; placement: string; startedAt?: number; spans: Array<{ adapter: string; t0?: number; t1?: number; outcome?: string; latency_ms?: number; error_code?: string }> }> }
  },
}

// Tools API
export const toolsApi = {
  inspectAppAds: async (domain: string): Promise<AppAdsInspectorResult> => {
    if (!domain || !domain.trim()) throw new Error('Domain is required')
    const resp = await apiClient.get<AppAdsInspectorResult>(`/tools/app-ads-inspector`, {
      params: { domain: domain.trim() },
    })
    return unwrapSuccessEnvelope(resp.data as any)
  },
  getSupplyChainStatus: async (params: { domain: string; sellerId?: string; appStoreId?: string; siteId?: string }): Promise<SupplyChainStatusResult> => {
    if (!params.domain || !params.domain.trim()) throw new Error('Domain is required')
    const resp = await apiClient.get<SuccessEnvelope<SupplyChainStatusResult> | SupplyChainStatusResult>(`/tools/supply-chain-status`, {
      params: {
        domain: params.domain.trim(),
        sellerId: params.sellerId?.trim(),
        appStoreId: params.appStoreId?.trim(),
        siteId: params.siteId?.trim(),
      },
    })
    return unwrapSuccessEnvelope(resp.data as any)
  },
  getSupplyChainSummary: async (): Promise<SupplyChainSummaryResponse> => {
    const resp = await apiClient.get<SuccessEnvelope<SupplyChainSummaryResponse> | SupplyChainSummaryResponse>(`/tools/supply-chain/summary`)
    return unwrapSuccessEnvelope(resp.data as any)
  },
}

// Adapter API
export const adapterApi = {
  list: async (placementId?: string) => {
    if (USE_MOCK_API) return mockApiCall<Adapter[]>('adapters')
    const resp = await apiClient.get<unknown>('/adapters', {
      params: placementId ? { placement_id: placementId } : undefined,
    })

    ;(resp as any).data = unwrapResponseData<Adapter[]>(resp.data)
    return resp as any
  },
  get: async (id: string) => {
    const resp = await apiClient.get<unknown>(`/adapters/${id}`)
    ;(resp as any).data = unwrapResponseData<Adapter>(resp.data)
    return resp as any
  },
  update: async (id: string, data: Partial<Adapter>) => {
    const resp = await apiClient.put<unknown>(`/adapters/${id}`, data)
    ;(resp as any).data = unwrapResponseData<Adapter>(resp.data)
    return resp as any
  },
  create: async (data: Partial<Adapter>) => {
    const resp = await apiClient.post<unknown>('/adapters', data)
    ;(resp as any).data = unwrapResponseData<Adapter>(resp.data)
    return resp as any
  },
  delete: (id: string) => apiClient.delete(`/adapters/${id}`),
}

// Revenue API
type RevenueTimeSeriesRequest = {
  startDate: string
  endDate: string
  granularity?: 'hour' | 'day' | 'week' | 'month'
  signal?: AbortSignal
}

type RevenueSummaryRequest = {
  startDate: string
  endDate: string
  signal?: AbortSignal
}

export const revenueApi = {
  getTimeSeries: async ({ signal, ...params }: RevenueTimeSeriesRequest) => {
    if (USE_MOCK_API) return mockApiCall<RevenueData[]>('revenue-timeseries')
    const resp = await apiClient.get<unknown>('/revenue/timeseries', {
      params,
      signal,
    })

    ;(resp as any).data = unwrapResponseData<RevenueData[]>(resp.data)
    return resp as any
  },
  getSummary: async ({ signal, ...params }: RevenueSummaryRequest) => {
    if (USE_MOCK_API) return mockApiCall<AnalyticsSummary>('revenue-summary')
    const resp = await apiClient.get<unknown>('/revenue/summary', {
      params,
      signal,
    })

    ;(resp as any).data = unwrapResponseData<AnalyticsSummary>(resp.data)
    return resp as any
  },
}

// Analytics API
export const analyticsApi = {
  getImpressions: async (params: {
    startDate: string
    endDate: string
    groupBy?: 'placement' | 'adapter' | 'date'
  }) => {
    if (USE_MOCK_API) return mockApiCall('analytics')
    return analyticsApiClient.get('/analytics/impressions', { params })
  },
  getPerformance: (placementId: string, params: { startDate: string; endDate: string }) =>
    analyticsApiClient.get(`/analytics/performance/${placementId}`, { params }),
}

// Fraud API
export const fraudApi = {
  getAlerts: async (
    publisherId: string,
    params?: { limit?: number; signal?: AbortSignal }
  ) => {
    if (USE_MOCK_API) return mockApiCall<{ alerts: FraudAlert[] }>('fraud-alerts')
    const { signal, ...query } = params ?? {}

    const resp = await fraudApiClient.get<SuccessEnvelope<any> | any>(`/fraud/alerts`, {
      params: query,
      signal,
    })

    const payload = unwrapSuccessEnvelope(resp.data as any)
    const rawAlerts = Array.isArray(payload)
      ? payload
      : isRecord(payload) && Array.isArray(payload.alerts)
        ? payload.alerts
        : []

    const alerts: FraudAlert[] = rawAlerts.map((raw: any) => {
      const id = String(raw?.id ?? globalThis.crypto?.randomUUID?.() ?? `${Date.now()}`)
      const timestamp = String(raw?.detectedAt ?? raw?.timestamp ?? new Date().toISOString())

      const alertTypeRaw = String(raw?.alertType ?? raw?.type ?? 'anomaly').toLowerCase()
      const alertType: FraudAlert['alertType'] =
        alertTypeRaw === 'givt' || alertTypeRaw === 'sivt' || alertTypeRaw === 'ml_fraud' || alertTypeRaw === 'anomaly'
          ? (alertTypeRaw as FraudAlert['alertType'])
          : 'anomaly'

      const severityRaw = String(raw?.severity ?? 'low').toLowerCase()
      const severity: FraudAlert['severity'] =
        severityRaw === 'low' || severityRaw === 'medium' || severityRaw === 'high' || severityRaw === 'critical'
          ? (severityRaw as FraudAlert['severity'])
          : 'low'

      const detectionTypeRaw = String(raw?.detectionType ?? 'rule_based')
      const detectionType: FraudAlert['detectionType'] =
        detectionTypeRaw === 'rule_based' || detectionTypeRaw === 'ml_based' || detectionTypeRaw === 'anomaly'
          ? (detectionTypeRaw as FraudAlert['detectionType'])
          : 'rule_based'

      const actionRaw = String(raw?.action ?? 'flag')
      const action: FraudAlert['action'] =
        actionRaw === 'block' || actionRaw === 'flag' || actionRaw === 'monitor' ? (actionRaw as FraudAlert['action']) : 'flag'

      const metadata: Record<string, any> =
        isRecord(raw?.metadata) ? raw.metadata : isRecord(raw) && typeof raw.details === 'string' ? { details: raw.details } : {}

      return {
        id,
        timestamp,
        alertType,
        severity,
        publisherId,
        deviceId: typeof raw?.deviceId === 'string' ? raw.deviceId : undefined,
        ipAddress: typeof raw?.ipAddress === 'string' ? raw.ipAddress : undefined,
        fraudScore: coerceNumber(raw?.fraudScore),
        detectionType,
        action,
        metadata,
      }
    })

    return { data: { alerts } }
  },
  getStats: async (
    publisherId: string,
    params?: { window?: '1h' | '24h' | '7d' | '30d'; signal?: AbortSignal }
  ) => {
    if (USE_MOCK_API) return mockApiCall<FraudStats>('fraud-stats')
    const { signal, ...query } = params ?? {}

    const resp = await fraudApiClient.get<SuccessEnvelope<any> | any>(`/fraud/stats`, {
      params: query,
      signal,
    })

    const payload = unwrapSuccessEnvelope(resp.data as any)

    if (isRecord(payload) && typeof payload.fraudRate === 'number') {
      return { data: payload as FraudStats }
    }

    // Backend currently returns a compact fraud stats shape. Normalize to the console's FraudStats contract.
    const totalAlerts = coerceNumber((payload as any)?.totalAlerts)
    const highSeverity = coerceNumber((payload as any)?.highSeverity)

    const totalRequests = totalAlerts
    const fraudRequests = highSeverity
    const fraudRate = totalRequests > 0 ? fraudRequests / totalRequests : 0

    const normalized: FraudStats = {
      publisherId,
      timeWindow: String(params?.window ?? '24h'),
      totalRequests,
      fraudRequests,
      fraudRate,
      givtDetections: 0,
      sivtDetections: 0,
      mlDetections: 0,
      anomalyDetections: 0,
      blockedRequests: 0,
      flaggedRequests: 0,
      averageFraudScore: 0,
      topFraudTypes: [],
      lastUpdated: String((payload as any)?.lastDetectionAt ?? new Date().toISOString()),
    }

    return { data: normalized }
  },
  getDashboard: (publisherId: string) =>
    fraudApiClient.get(`/fraud/dashboard/${publisherId}`),
  getTrend: (
    publisherId: string,
    params: { start: string; end: string; granularity: 'hourly' | 'daily' | 'weekly' }
  ) => fraudApiClient.get(`/fraud/trend/${publisherId}`, { params }),
}

// Migration Studio API
type CreateMigrationImportParams = {
  placementId: string
  source: MigrationImportSource
  experimentId?: string
  file?: File
  credentials?: Record<string, string>
}

type UpdateMigrationExperimentParams = {
  name?: string
  description?: string
  mirror_percent?: number
  guardrails?: Partial<MigrationGuardrails>
}

type ActivateMigrationExperimentParams = {
  mirror_percent: number
}

type UpdateMigrationMappingParams = {
  mappingId: string
  ourAdapterId: string
  notes?: string
}

type GetMigrationExperimentReportParams = {
  window?: string
  start?: string
  end?: string
  granularity?: 'hour' | 'day'
  timezone?: string
}

type CreateMigrationExperimentShareLinkParams = {
  expires_in_hours: number
}

export const migrationApi = {
  listExperiments: async (params?: { placementId?: string; status?: string }) => {
    const query = params
      ? {
          status: params.status,
          placement_id: params.placementId,
        }
      : undefined
    if (USE_MOCK_API) {
      return unwrapApiResponse(
        mockApiCall<ApiResponse<MigrationExperiment[]>>('migration-experiments')
      )
    }
    return unwrapApiResponse(
      apiClient.get<ApiResponse<MigrationExperiment[]>>('/migration/experiments', {
        params: query,
      })
    )
  },
  getExperiment: async (id: string) => {
    if (USE_MOCK_API) {
      return unwrapApiResponse(mockApiCall<ApiResponse<MigrationExperiment>>('migration-experiment'))
    }
    return unwrapApiResponse(
      apiClient.get<ApiResponse<MigrationExperiment>>(`/migration/experiments/${id}`)
    )
  },
  getExperimentReport: async (id: string, params?: GetMigrationExperimentReportParams) => {
    if (USE_MOCK_API) {
      const now = new Date()
      const sevenDaysAgo = new Date(now.getTime() - 7 * 24 * 60 * 60 * 1000)
      const days = Array.from({ length: 7 }).map((_, index) => {
        const offset = 6 - index
        const date = new Date(now.getTime() - offset * 24 * 60 * 60 * 1000)
        date.setHours(0, 0, 0, 0)
        return date
      })

      const buildCurrencySeries = (baseControl: number, baseTest: number) =>
        days.map((date, idx) => {
          const modifier = 1 + Math.sin(idx / 2.5) * 0.05
          return {
            timestamp: date.toISOString(),
            control: Math.round(baseControl * modifier),
            test: Math.round(baseTest * modifier * 1.06),
          }
        })

      const buildPercentSeries = (baseControl: number, baseTest: number) =>
        days.map((date, idx) => {
          const modifier = Math.cos(idx / 3) * 1.5
          return {
            timestamp: date.toISOString(),
            control: Number((baseControl + modifier).toFixed(1)),
            test: Number((baseTest + modifier + 0.8).toFixed(1)),
          }
        })

      const buildMillisecondsSeries = (baseControl: number, baseTest: number) =>
        days.map((date, idx) => {
          const modifier = Math.sin(idx / 2) * 12
          return {
            timestamp: date.toISOString(),
            control: Math.round(baseControl + modifier),
            test: Math.round(baseTest + modifier - 18),
          }
        })

      return {
        experiment_id: id,
        generated_at: now.toISOString(),
        window: {
          start: sevenDaysAgo.toISOString(),
          end: now.toISOString(),
          timezone: 'UTC',
        },
        metrics: {
          overall: [
            {
              id: 'revenue',
              label: 'Net revenue',
              unit: 'currency_cents',
              control: 125_000,
              test: 138_000,
              uplift: 10.4,
              sample_size: { control: 120000, test: 118500 },
            },
            {
              id: 'ecpm',
              label: 'eCPM',
              unit: 'currency_cents',
              control: 235,
              test: 252,
              uplift: 7.2,
            },
            {
              id: 'fill',
              label: 'Fill rate',
              unit: 'percent',
              control: 87.4,
              test: 90.9,
              uplift: 4.0,
            },
            {
              id: 'latency_p95',
              label: 'Latency p95',
              unit: 'milliseconds',
              control: 430,
              test: 405,
              uplift: 5.8,
            },
            {
              id: 'ivt',
              label: 'IVT rate',
              unit: 'percent',
              control: 1.8,
              test: 1.5,
              uplift: 16.7,
            },
          ],
          timeseries: [
            {
              id: 'revenue',
              label: 'Net revenue',
              unit: 'currency_cents',
              points: buildCurrencySeries(17_500, 19_200),
            },
            {
              id: 'ecpm',
              label: 'eCPM',
              unit: 'currency_cents',
              points: buildCurrencySeries(220, 236),
            },
            {
              id: 'fill',
              label: 'Fill rate',
              unit: 'percent',
              points: buildPercentSeries(86.2, 88.5),
            },
            {
              id: 'latency_p95',
              label: 'Latency p95',
              unit: 'milliseconds',
              points: buildMillisecondsSeries(440, 418),
            },
            {
              id: 'ivt',
              label: 'IVT rate',
              unit: 'percent',
              points: buildPercentSeries(1.9, 1.5),
            },
          ],
        },
      } satisfies MigrationExperimentReport
    }
    return unwrapApiResponse(
      apiClient.get<ApiResponse<MigrationExperimentReport>>(
        `/migration/experiments/${id}/report`,
        { params }
      )
    )
  },
  getExperimentShareLinks: async (id: string) => {
    if (USE_MOCK_API) {
      const now = new Date()
      const expires = new Date(now.getTime() + 7 * 24 * 60 * 60 * 1000)
      return [
        {
          id: 'mock-share-link',
          url: `https://console.example.dev/migration-reports/${id}/mock-share-link`,
          created_at: now.toISOString(),
          expires_at: expires.toISOString(),
        },
      ] satisfies MigrationExperimentShareLink[]
    }
    return unwrapApiResponse(
      apiClient.get<ApiResponse<MigrationExperimentShareLink[]>>(
        `/migration/experiments/${id}/share-links`
      )
    )
  },
  createExperimentShareLink: async (
    id: string,
    params: CreateMigrationExperimentShareLinkParams
  ) => {
    if (USE_MOCK_API) {
      const now = new Date()
      const expires = new Date(now.getTime() + params.expires_in_hours * 60 * 60 * 1000)
      return {
        id: `mock-share-link-${params.expires_in_hours}`,
        url: `https://console.example.dev/migration-reports/${id}/mock-share-link`,
        created_at: now.toISOString(),
        expires_at: expires.toISOString(),
      } satisfies MigrationExperimentShareLink
    }
    return unwrapApiResponse(
      apiClient.post<ApiResponse<MigrationExperimentShareLink>>(
        `/migration/experiments/${id}/share-links`,
        params
      )
    )
  },
  revokeExperimentShareLink: async (id: string, shareLinkId: string) => {
    if (USE_MOCK_API) {
      return
    }
    await apiClient.delete<ApiResponse<null>>(
      `/migration/experiments/${id}/share-links/${shareLinkId}`
    )
  },
  downloadExperimentReport: async (id: string) => {
    if (USE_MOCK_API) {
      const mockArtifact = {
        experiment_id: id,
        generated_at: new Date().toISOString(),
        signature: 'mock-signature',
      }
      return new Blob([JSON.stringify(mockArtifact, null, 2)], {
        type: 'application/json',
      })
    }
    const response = await apiClient.get(`/migration/experiments/${id}/report/artifact`, {
      responseType: 'blob',
    })
    return response.data as Blob
  },
  createImport: async (params: CreateMigrationImportParams) => {
    if (USE_MOCK_API) {
      const response = await mockApiCall<MigrationImportResponse>('migration-import')
      return response.data
    }

    if (params.source === 'csv') {
      validateCsvFile(params.file)
      const formData = new FormData()
      formData.append('source', params.source)
      formData.append('placement_id', params.placementId)
      if (params.experimentId) formData.append('experiment_id', params.experimentId)
      if (params.file) formData.append('file', params.file)
      return unwrapApiResponse(
        apiClient.post<ApiResponse<MigrationImportResponse>>('/migration/import', formData, {
          headers: { 'Content-Type': 'multipart/form-data' },
        })
      )
    }

    return unwrapApiResponse(
      apiClient.post<ApiResponse<MigrationImportResponse>>('/migration/import', {
        source: params.source,
        placement_id: params.placementId,
        experiment_id: params.experimentId,
        credentials: params.credentials ?? {},
      })
    )
  },
  updateMapping: async ({ mappingId, ourAdapterId, notes }: UpdateMigrationMappingParams) => {
    if (USE_MOCK_API) {
      const response = await mockApiCall<MigrationMapping>('migration-mapping')
      return { mapping: response.data, summary: { total_mappings: 1, status_breakdown: { pending: 0, confirmed: 1, skipped: 0, conflict: 0 }, confidence_breakdown: { high: 0, medium: 1, low: 0 }, unique_networks: 1 } }
    }
    return unwrapApiResponse(
      apiClient.put<ApiResponse<MigrationMappingUpdateResponse>>(`/migration/mappings/${mappingId}`, {
        our_adapter_id: ourAdapterId,
        notes,
      })
    )
  },
  finalizeImport: async (importId: string) => {
    if (USE_MOCK_API) {
      const response = await mockApiCall<MigrationImportResponse>('migration-import')
      return response.data
    }
    return unwrapApiResponse(
      apiClient.post<ApiResponse<MigrationImportResponse>>(
        `/migration/import/${importId}/finalize`,
        {}
      )
    )
  },
  updateExperiment: async (id: string, params: UpdateMigrationExperimentParams) => {
    if (USE_MOCK_API) {
      return unwrapApiResponse(mockApiCall<ApiResponse<MigrationExperiment>>('migration-experiment'))
    }
    return unwrapApiResponse(
      apiClient.put<ApiResponse<MigrationExperiment>>(`/migration/experiments/${id}`, params)
    )
  },
  activateExperiment: async (id: string, params: ActivateMigrationExperimentParams) => {
    if (USE_MOCK_API) {
      return unwrapApiResponse(mockApiCall<ApiResponse<MigrationExperiment>>('migration-experiment'))
    }
    return unwrapApiResponse(
      apiClient.post<ApiResponse<MigrationExperiment>>(
        `/migration/experiments/${id}/activate`,
        params
      )
    )
  },
  pauseExperiment: async (id: string, reason?: string) => {
    if (USE_MOCK_API) {
      return unwrapApiResponse(mockApiCall<ApiResponse<MigrationExperiment>>('migration-experiment'))
    }
    return unwrapApiResponse(
      apiClient.post<ApiResponse<MigrationExperiment>>(`/migration/experiments/${id}/pause`, {
        reason,
      })
    )
  },
  evaluateGuardrails: async (id: string) => {
    if (USE_MOCK_API) {
      return { shouldPause: false, violations: [] } as EvaluateGuardrailsResponse
    }
    return unwrapApiResponse(
      apiClient.post<ApiResponse<EvaluateGuardrailsResponse>>(
        `/migration/experiments/${id}/guardrails/evaluate`,
        {}
      )
    )
  },
  completeMigration: async (id: string) => {
    if (USE_MOCK_API) {
      return unwrapApiResponse(mockApiCall<ApiResponse<MigrationExperiment>>('migration-experiment'))
    }
    return unwrapApiResponse(
      apiClient.post<ApiResponse<MigrationExperiment>>(
        `/migration/experiments/${id}/complete`,
        {}
      )
    )
  },
  rollbackMigration: async (id: string, reason?: string) => {
    if (USE_MOCK_API) {
      return unwrapApiResponse(mockApiCall<ApiResponse<MigrationExperiment>>('migration-experiment'))
    }
    return unwrapApiResponse(
      apiClient.post<ApiResponse<MigrationExperiment>>(
        `/migration/experiments/${id}/rollback`,
        { reason }
      )
    )
  },
  getSdkIntegration: async (id: string, platform?: string) => {
    if (USE_MOCK_API) {
      return {
        experiment_id: id,
        placement_id: 'mock-placement-id',
        adapters: [
          { id: 'apex_adapter_applovin_max_v1', name: 'Apex · AppLovin MAX' },
          { id: 'apex_adapter_ironsource_v1', name: 'Apex · ironSource' },
        ],
        api_endpoint: 'https://api.apex-ad.com/api/v1/migration/assign',
        snippets: {
          ios: {
            language: 'swift',
            code: '// iOS SDK Code...',
            instructions: '1. Add ApexSDK to your Podfile',
          },
          android: {
            language: 'kotlin',
            code: '// Android SDK Code...',
            instructions: '1. Add to build.gradle',
          },
        },
      }
    }
    return unwrapApiResponse(
      apiClient.get<ApiResponse<any>>(
        `/migration/experiments/${id}/sdk-integration`,
        { params: platform ? { platform } : undefined }
      )
    )
  },
  getExperimentMappings: async (id: string) => {
    if (USE_MOCK_API) {
      return []
    }
    return unwrapApiResponse(
      apiClient.get<ApiResponse<any[]>>(`/migration/experiments/${id}/mappings`)
    )
  },
}

// Settings API
export const settingsApi = {
  getFraudSettings: () =>
    apiClient.get<FraudSettings>('/settings/fraud').then((resp) => {
      ;(resp as any).data = unwrapResponseData<FraudSettings>(resp.data)
      return resp
    }),
  updateFraudSettings: (data: FraudSettings) =>
    apiClient.put('/settings/fraud', data).then((resp) => {
      ;(resp as any).data = unwrapResponseData<FraudSettings>(resp.data)
      return resp
    }),
  getNotificationSettings: () =>
    apiClient.get<NotificationSettings>('/settings/notifications').then((resp) => {
      ;(resp as any).data = unwrapResponseData<NotificationSettings>(resp.data)
      return resp
    }),
  updateNotificationSettings: (data: Partial<NotificationSettings>) =>
    apiClient.put<NotificationSettings>('/settings/notifications', data).then((resp) => {
      ;(resp as any).data = unwrapResponseData<NotificationSettings>(resp.data)
      return resp
    }),
  getComplianceSettings: () =>
    apiClient.get<ComplianceSettings>('/settings/compliance').then((resp) => {
      ;(resp as any).data = unwrapResponseData<ComplianceSettings>(resp.data)
      return resp
    }),
  updateComplianceSettings: (data: Partial<ComplianceSettings>) =>
    apiClient.put<ComplianceSettings>('/settings/compliance', data).then((resp) => {
      ;(resp as any).data = unwrapResponseData<ComplianceSettings>(resp.data)
      return resp
    }),
}

// Team API
export const teamApi = {
  listMembers: () =>
    apiClient.get<TeamMember[]>('/team/members').then((resp) => {
      ;(resp as any).data = unwrapResponseData<TeamMember[]>(resp.data)
      return resp
    }),
  inviteMember: (invitation: TeamInvitation) =>
    apiClient.post<TeamMember>('/team/invite', invitation).then((resp) => {
      ;(resp as any).data = unwrapResponseData<TeamMember>(resp.data)
      return resp
    }),
  updateMember: (id: string, data: Partial<TeamMember>) =>
    apiClient.put<TeamMember>(`/team/members/${id}`, data).then((resp) => {
      ;(resp as any).data = unwrapResponseData<TeamMember>(resp.data)
      return resp
    }),
  removeMember: (id: string) => apiClient.delete(`/team/members/${id}`),
  resendInvite: (id: string) => apiClient.post(`/team/members/${id}/resend`),
  listRoles: () =>
    apiClient.get<TeamRoleDefinition[]>('/team/roles').then((resp) => {
      ;(resp as any).data = unwrapResponseData<TeamRoleDefinition[]>(resp.data)
      return resp
    }),
}

// BYO (Bring Your Own) Network Credentials API
export const byoApi = {
  /**
   * List all stored network credentials for the publisher
   */
  listCredentials: () =>
    apiClient.get<NetworkCredentialsList>('/byo/credentials').then((resp) => {
      ;(resp as any).data = unwrapResponseData<NetworkCredentialsList>(resp.data)
      return resp
    }),

  /**
   * Store or update network credentials
   */
  storeCredentials: (data: NetworkCredentialInput) =>
    apiClient.post<{ credentialId: string }>('/byo/credentials', data),

  /**
   * Get metadata for a specific network's credentials
   * Note: Does not return the actual credentials for security
   */
  getCredentials: (network: string) =>
    apiClient.get<NetworkCredential>(`/byo/credentials/${network}`),

  /**
   * Generate a short-lived token for SDK authentication
   * @param network - Network name (admob, unity, etc.)
   * @param ttlMinutes - Token lifetime in minutes (default 15)
   */
  generateToken: (network: string, ttlMinutes?: number) =>
    apiClient.post<NetworkCredentialToken>(`/byo/credentials/${network}/token`, {
      ttlMinutes: ttlMinutes || 15,
    }),

  /**
   * Rotate credentials for a network (create new version)
   */
  rotateCredentials: (network: string, newCredentials: Record<string, unknown>) =>
    apiClient.post<{ version: number }>(`/byo/credentials/${network}/rotate`, {
      newCredentials,
    }),

  /**
   * Delete credentials for a network
   */
  deleteCredentials: (network: string) =>
    apiClient.delete(`/byo/credentials/${network}`),

  /**
   * Request an on-demand verification of the stored identifiers against the network's public endpoints.
   */
  verifyNetwork: async (network: string) => {
    const response = await apiClient.post<SuccessEnvelope<NetworkVerificationResult>>(
      `/byo/credentials/${network}/verify`,
      {}
    )
    return unwrapSuccessEnvelope(response.data)
  },

  /**
   * Upload and ingest an AdMob CSV report file
   */
  ingestAdmobCsv: async (file: File): Promise<NetworkIngestionResult> => {
    const formData = new FormData()
    formData.append('file', file)
    const response = await apiClient.post<NetworkIngestionResult>(
      '/byo/ingest/admob-csv',
      formData,
      { headers: { 'Content-Type': 'multipart/form-data' } }
    )
    return response.data
  },

  /**
   * Ingest AdMob data via API for a date range
   */
  ingestAdmobApi: async (range: { startDate: string; endDate: string }): Promise<NetworkIngestionResult> => {
    const response = await apiClient.post<NetworkIngestionResult>('/byo/ingest/admob-api', range)
    return response.data
  },

  /**
   * Ingest Unity data via API for a date range
   */
  ingestUnityApi: async (range: { startDate: string; endDate: string }): Promise<NetworkIngestionResult> => {
    const response = await apiClient.post<NetworkIngestionResult>('/byo/ingest/unity-api', range)
    return response.data
  },
}

// =============================================================================
// App API Keys
// =============================================================================

import type { App, AppApiKey, AppApiKeyWithSecret } from '@/types'

export type CreateAppInput = {
  name: string
  bundleId: string
  platform: string
}

export type UpdateAppInput = {
  name?: string
  bundleId?: string
}

export const appsApi = {
  /**
   * List all apps for the current publisher
   */
  list: async (): Promise<App[]> => {
    const response = await apiClient.get<SuccessEnvelope<App[]>>('/apps')
    return unwrapSuccessEnvelope(response.data)
  },

  /**
   * Get a single app by ID
   */
  get: async (appId: string): Promise<App> => {
    const response = await apiClient.get<SuccessEnvelope<App>>(`/apps/${appId}`)
    return unwrapSuccessEnvelope(response.data)
  },

  /**
   * Create a new app
   */
  create: async (input: CreateAppInput): Promise<App> => {
    const response = await apiClient.post<SuccessEnvelope<App>>('/apps', input)
    return unwrapSuccessEnvelope(response.data)
  },

  /**
   * Update an existing app
   */
  update: async (appId: string, input: UpdateAppInput): Promise<App> => {
    const response = await apiClient.put<SuccessEnvelope<App>>(`/apps/${appId}`, input)
    return unwrapSuccessEnvelope(response.data)
  },

  /**
   * Delete an app
   */
  delete: async (appId: string): Promise<void> => {
    await apiClient.delete(`/apps/${appId}`)
  },
}

export const appKeysApi = {
  /**
   * List all API keys for an app
   */
  list: async (appId: string): Promise<AppApiKey[]> => {
    const response = await apiClient.get<SuccessEnvelope<AppApiKey[]>>(`/apps/${appId}/keys`)
    return unwrapSuccessEnvelope(response.data)
  },

  /**
   * Create a new API key for an app
   */
  create: async (appId: string, options: { live: boolean }): Promise<AppApiKeyWithSecret> => {
    const response = await apiClient.post<SuccessEnvelope<AppApiKeyWithSecret>>(`/apps/${appId}/keys`, options)
    return unwrapSuccessEnvelope(response.data)
  },

  /**
   * Rotate an existing API key (revokes old, creates new)
   */
  rotate: async (appId: string, keyId: string): Promise<AppApiKeyWithSecret> => {
    const response = await apiClient.post<SuccessEnvelope<AppApiKeyWithSecret>>(`/apps/${appId}/keys/${keyId}/rotate`, {})
    return unwrapSuccessEnvelope(response.data)
  },

  /**
   * Revoke an API key
   */
  revoke: async (appId: string, keyId: string): Promise<void> => {
    await apiClient.delete(`/apps/${appId}/keys/${keyId}`)
  },
}

// Me API - current user/publisher info
export const meApi = {
  get: async () => {
    const resp = await apiClient.get('/publishers/me')
    const payload: any = resp.data
    return payload?.data ?? payload
  },
}
