import { apiClient, fraudApiClient, analyticsApiClient } from './api-client'
import type {
  Publisher,
  Placement,
  Adapter,
  RevenueData,
  AnalyticsSummary,
  FraudAlert,
  FraudStats,
  PayoutHistory,
  PaginatedResponse,
  FraudSettings,
  PayoutSettings,
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
    return apiClient.get<Adapter[]>('/adapters', {
      params: placementId ? { placement_id: placementId } : undefined,
    })
  },
  get: (id: string) => apiClient.get<Adapter>(`/adapters/${id}`),
  update: (id: string, data: Partial<Adapter>) =>
    apiClient.put<Adapter>(`/adapters/${id}`, data),
  create: (data: Partial<Adapter>) => apiClient.post<Adapter>('/adapters', data),
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
    return apiClient.get<RevenueData[]>('/revenue/timeseries', {
      params,
      signal,
    })
  },
  getSummary: async ({ signal, ...params }: RevenueSummaryRequest) => {
    if (USE_MOCK_API) return mockApiCall<AnalyticsSummary>('revenue-summary')
    return apiClient.get<AnalyticsSummary>('/revenue/summary', {
      params,
      signal,
    })
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
    return fraudApiClient.get<{ alerts: FraudAlert[] }>(`/fraud/alerts/${publisherId}`, {
      params: query,
      signal,
    })
  },
  getStats: async (
    publisherId: string,
    params?: { window?: '1h' | '24h' | '7d' | '30d'; signal?: AbortSignal }
  ) => {
    if (USE_MOCK_API) return mockApiCall<FraudStats>('fraud-stats')
    const { signal, ...query } = params ?? {}
    return fraudApiClient.get<FraudStats>(`/fraud/stats/${publisherId}`, {
      params: query,
      signal,
    })
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
}

// Payout API
type PayoutHistoryRequest = {
  page?: number
  pageSize?: number
  publisherId?: string
  signal?: AbortSignal
}

type UpcomingPayoutRequest = {
  publisherId?: string
  signal?: AbortSignal
}

type UpdatePayoutSettingsPayload = {
  threshold: number
  method: 'stripe' | 'paypal' | 'wire'
  currency: string
  schedule?: 'monthly'
  accountName: string
  accountReference: string
  autoPayout: boolean
  backupMethod?: 'stripe' | 'paypal' | 'wire'
}

export const payoutApi = {
  getHistory: async (params: PayoutHistoryRequest = {}) => {
    if (USE_MOCK_API) {
      const response = await mockApiCall<PaginatedResponse<PayoutHistory>>('payout-history')
      return response.data
    }
    const { signal, ...query } = params
    const pagination = withPaginationDefaults({ page: query.page, pageSize: query.pageSize })
    const response = await apiClient.get<
      PaginatedResponse<PayoutHistory> | SuccessEnvelope<PaginatedResponse<PayoutHistory>>
    >('/payouts/history', {
      params: { ...query, ...pagination },
      signal,
    })
    return unwrapSuccessEnvelope(response.data)
  },
  getUpcoming: async (params: UpcomingPayoutRequest = {}) => {
    if (USE_MOCK_API) {
      const response = await mockApiCall<PayoutHistory | null>('payout-upcoming')
      return response.data
    }
    const { signal, ...query } = params
    const response = await apiClient.get<
      PayoutHistory | null | SuccessEnvelope<PayoutHistory | null>
    >('/payouts/upcoming', {
      params: query,
      signal,
    })
    return unwrapSuccessEnvelope(response.data)
  },
  updateMethod: (method: 'stripe' | 'paypal' | 'wire', details: Record<string, any>) =>
    apiClient.put('/payouts/method', { method, details }),
}

// Settings API
export const settingsApi = {
  getFraudSettings: () => apiClient.get<FraudSettings>('/settings/fraud'),
  updateFraudSettings: (data: FraudSettings) => apiClient.put('/settings/fraud', data),
  getPayoutSettings: async () => {
    const response = await apiClient.get<
      PayoutSettings | null | SuccessEnvelope<PayoutSettings | null>
    >('/settings/payout')
    return unwrapSuccessEnvelope(response.data)
  },
  updatePayoutSettings: async (data: Partial<UpdatePayoutSettingsPayload>) => {
    const response = await apiClient.put<PayoutSettings | SuccessEnvelope<PayoutSettings>>(
      '/settings/payout',
      data
    )
    return unwrapSuccessEnvelope(response.data)
  },
  getNotificationSettings: () => apiClient.get<NotificationSettings>('/settings/notifications'),
  updateNotificationSettings: (data: Partial<NotificationSettings>) =>
    apiClient.put<NotificationSettings>('/settings/notifications', data),
  getComplianceSettings: () => apiClient.get<ComplianceSettings>('/settings/compliance'),
  updateComplianceSettings: (data: Partial<ComplianceSettings>) =>
    apiClient.put<ComplianceSettings>('/settings/compliance', data),
}

// Team API
export const teamApi = {
  listMembers: () => apiClient.get<TeamMember[]>('/team/members'),
  inviteMember: (invitation: TeamInvitation) =>
    apiClient.post<TeamMember>('/team/invite', invitation),
  updateMember: (id: string, data: Partial<TeamMember>) =>
    apiClient.put<TeamMember>(`/team/members/${id}`, data),
  removeMember: (id: string) => apiClient.delete(`/team/members/${id}`),
  resendInvite: (id: string) => apiClient.post(`/team/members/${id}/resend`),
  listRoles: () => apiClient.get<TeamRoleDefinition[]>('/team/roles'),
}

// BYO (Bring Your Own) Network Credentials API
export const byoApi = {
  /**
   * List all stored network credentials for the publisher
   */
  listCredentials: () =>
    apiClient.get<NetworkCredentialsList>('/byo/credentials'),

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
