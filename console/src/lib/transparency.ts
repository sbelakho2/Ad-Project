import axios from 'axios'
import { apiClient } from './api-client'

export type TransparencyAuction = {
  auction_id: string
  timestamp: string
  publisher_id: string
  app_or_site_id: string
  placement_id: string
  surface_type: 'mobile_app' | 'web' | 'ctv'
  device_context: {
    os: string
    geo: string
    att: string
    tc_string_sha256: string
  }
  candidates: Array<{
    source: string
    bid_ecpm: number
    currency: string
    response_time_ms: number
    status: string
    metadata_hash: string
  }>
  winner: {
    source: string
    bid_ecpm: number
    gross_price: number
    currency: string
    reason: string
  }
  fees: {
    aletheia_fee_bp: number
    effective_publisher_share: number
  }
  integrity: {
    signature: string
    algo: string
    key_id: string
  }
}

export type VerifyResult = {
  status: 'pass' | 'fail' | 'not_applicable' | 'unknown_key'
  key_id?: string | null
  algo?: string | null
  reason?: string
  canonical?: string
  sample_bps?: number
}

export type TransparencySummary = {
  total_sampled: number
  winners_by_source: Array<{ source: string; count: number }>
  avg_fee_bp: number
  publisher_share_avg: number
  last_updated?: string
}

export class TransparencyApiError extends Error {
  status?: number
  details?: unknown

  constructor(message: string, options: { cause?: unknown; status?: number; details?: unknown } = {}) {
    super(message)
    this.name = 'TransparencyApiError'
    this.status = options.status
    this.details = options.details
    if (options.cause && typeof (this as Record<string, unknown>).cause === 'undefined') {
      ;(this as Record<string, unknown>).cause = options.cause
    }
  }
}

type RequestOptions = { signal?: AbortSignal }

type ListParams = {
    page?: number
    limit?: number
    from?: string
    to?: string
    placement_id?: string
    surface?: string
    geo?: string
    publisher_id?: string
}

function logTransparencyError(context: string, error: TransparencyApiError) {
  if (process.env.NODE_ENV !== 'production') {
    console.error(`[transparencyApi] ${context}`, error)
  }
}

function normalizeTransparencyError(context: string, error: unknown): TransparencyApiError {
  if (axios.isAxiosError(error)) {
    const status = error.response?.status
    const data: any = error.response?.data
    const message =
      (typeof data?.message === 'string' && data.message) ||
      (typeof data?.error === 'string' && data.error) ||
      (typeof data === 'string' && data) ||
      error.message ||
      'Transparency API request failed'
    return new TransparencyApiError(message, { cause: error, status, details: error.response?.data })
  }

  if (error instanceof TransparencyApiError) {
    return error
  }

  if (error instanceof Error) {
    return new TransparencyApiError(error.message, { cause: error })
  }

  return new TransparencyApiError(`Transparency API request failed: ${context}`, { details: error })
}

async function request<T>(context: string, executor: () => Promise<T>): Promise<T> {
  try {
    return await executor()
  } catch (error) {
    const normalized = normalizeTransparencyError(context, error)
    logTransparencyError(context, normalized)
    throw normalized
  }
}

export const transparencyApi = {
  async list(params: ListParams, options: RequestOptions = {}) {
    return request('list auctions', async () => {
      const res = await apiClient.get<{ page: number; limit: number; count: number; data: TransparencyAuction[] }>(
        '/transparency/auctions',
        { params, signal: options.signal }
      )
      return res.data
    })
  },

  async get(auctionId: string, options: RequestOptions = {}) {
    return request('get auction', async () => {
      const res = await apiClient.get<TransparencyAuction>(`/transparency/auctions/${auctionId}`, {
        signal: options.signal,
      })
      return res.data
    })
  },

  async verify(auctionId: string, options: RequestOptions = {}) {
    return request('verify auction', async () => {
      const res = await apiClient.get<VerifyResult>(`/transparency/auctions/${auctionId}/verify`, {
        signal: options.signal,
      })
      return res.data
    })
  },

  async keys(options: RequestOptions = {}) {
    return request('list transparency keys', async () => {
      const res = await apiClient.get<{ count: number; data: Array<{ key_id: string; algo: string; public_key_base64: string; active: number }> }>(
        '/transparency/keys',
        { signal: options.signal }
      )
      return res.data
    })
  },

  async summary(options: RequestOptions = {}) {
    return request('summary', async () => {
      const res = await apiClient.get<TransparencySummary>('/transparency/summary/auctions', {
        signal: options.signal,
      })
      return res.data
    })
  },

  createCancellableRequest<T>(executor: (signal: AbortSignal) => Promise<T>) {
    const controller = new AbortController()
    const promise = executor(controller.signal)
    return {
      cancel: () => controller.abort(),
      signal: controller.signal,
      promise,
    }
  },
}
