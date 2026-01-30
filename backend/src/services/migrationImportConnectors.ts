/**
 * Lightweight connectors for incumbent mediation providers used during Migration Studio imports.
 *
 * FIX-11-617: Harden connectors by adding credential validation, optional real HTTP clients
 * with timeouts/retries, and simple in-process rate limiting. When live endpoints are not
 * configured, fall back to deterministic sample data so the pipeline and tests can run.
 */

import type { AxiosInstance } from 'axios'
import { createHttpClient } from '../utils/httpClient'

export type ConnectorCredentials = {
  api_key: string
  account_id: string
}

export type NormalizedMappingRow = {
  network: string
  instanceId: string
  instanceName?: string
  waterfallPosition?: number
  ecpmCents?: number
  confidence: 'high' | 'medium' | 'low'
}

function ensureCredentials(source: string, credentials: ConnectorCredentials) {
  if (!credentials?.api_key?.trim() || !credentials?.account_id?.trim()) {
    throw new Error(`${source} credentials are required`)
  }
}

// Simple per-source rate limiter (token bucket-ish)
const RATE_LIMIT_PER_MINUTE = Number(process.env.MIGRATION_CONNECTOR_RPM ?? 60)
const rateState: Record<string, { count: number; resetAt: number }> = {}

function checkRateLimit(source: string) {
  const now = Date.now()
  const bucket = rateState[source] || { count: 0, resetAt: now + 60_000 }
  if (now > bucket.resetAt) {
    bucket.count = 0
    bucket.resetAt = now + 60_000
  }
  bucket.count += 1
  rateState[source] = bucket
  if (bucket.count > RATE_LIMIT_PER_MINUTE) {
    const retryIn = Math.max(0, bucket.resetAt - now)
    const err: any = new Error(`${source} connector rate limit exceeded`)
    err.retryAfterMs = retryIn
    throw err
  }
}

// Optional live mode
const LIVE_MODE = process.env.MIGRATION_CONNECTORS_LIVE === '1'
const ironSourceBaseUrl = process.env.MIGRATION_IRONSOURCE_BASE_URL
const appLovinBaseUrl = process.env.MIGRATION_APPLOVIN_BASE_URL
let http: AxiosInstance | undefined
function getHttp(): AxiosInstance {
  if (!http) http = createHttpClient(undefined, { timeout: 8000 })
  return http
}

async function withRetry<T>(fn: () => Promise<T>, attempts = 3): Promise<T> {
  let lastErr: any
  for (let i = 0; i < attempts; i++) {
    try {
      return await fn()
    } catch (e) {
      lastErr = e
      // backoff 100ms, 250ms, 500ms
      const delay = [100, 250, 500][Math.min(i, 2)]
      await new Promise((r) => setTimeout(r, delay))
    }
  }
  throw lastErr
}

export async function fetchIronSourceSetup(credentials: ConnectorCredentials): Promise<NormalizedMappingRow[]> {
  ensureCredentials('ironSource', credentials)
  checkRateLimit('ironSource')

  if (LIVE_MODE && ironSourceBaseUrl) {
    try {
      const res = await withRetry(() => getHttp().get(
        `${ironSourceBaseUrl}/v1/placements`,
        {
          headers: { Authorization: `Bearer ${credentials.api_key}` },
          params: { account_id: credentials.account_id },
        }
      ))
      // Map minimal fields; unknown fields are ignored
      // UNIT HANDLING: ironSource API returns eCPM in dollars (e.g., 2.75 = $2.75)
      // We convert to cents for internal consistency
      const rows = Array.isArray(res.data) ? res.data : []
      return rows.slice(0, 100).map((r: any, idx: number) => {
        // Prefer explicit ecpm_cents if provided, otherwise convert ecpm from dollars
        let ecpmCents: number | undefined
        if (r.ecpm_cents !== undefined && r.ecpm_cents !== null) {
          ecpmCents = Number(r.ecpm_cents) || undefined
        } else if (r.ecpm !== undefined && r.ecpm !== null) {
          // API returns dollars, convert to cents
          ecpmCents = Math.round(Number(r.ecpm) * 100) || undefined
        }
        
        return {
          network: 'ironSource',
          instanceId: String(r.instance_id ?? r.id ?? `is-${idx}`),
          instanceName: String(r.name ?? 'ironSource Instance'),
          waterfallPosition: Number(r.position ?? idx + 1) || undefined,
          ecpmCents,
          confidence: 'high' as const,
        }
      })
    } catch (error: any) {
      // In live mode, propagate errors instead of falling back to sample data
      const status = error.response?.status
      const message = error.response?.data?.message || error.message || 'Unknown error'
      const connectorError: any = new Error(`ironSource API error: ${message}`)
      connectorError.status = status || 500
      connectorError.source = 'ironSource'
      throw connectorError
    }
  }

  // Return deterministic sample data seeded off account id so tests can assert values
  const base = credentials.account_id.slice(-4)
  return [
    {
      network: 'ironSource',
      instanceId: `is-rv-${base}`,
      instanceName: 'ironSource Rewarded',
      waterfallPosition: 1,
      ecpmCents: 275, // $2.75
      confidence: 'high',
    },
    {
      network: 'ironSource',
      instanceId: `is-int-${base}`,
      instanceName: 'ironSource Interstitial',
      waterfallPosition: 2,
      ecpmCents: 180, // $1.80
      confidence: 'medium',
    },
  ]
}

export async function fetchAppLovinSetup(credentials: ConnectorCredentials): Promise<NormalizedMappingRow[]> {
  ensureCredentials('AppLovin', credentials)
  checkRateLimit('AppLovin')

  if (LIVE_MODE && appLovinBaseUrl) {
    try {
      const res = await withRetry(() => getHttp().get(
        `${appLovinBaseUrl}/v1/max/placements`,
        {
          headers: { 'X-Api-Key': credentials.api_key },
          params: { account_id: credentials.account_id },
        }
      ))
      // UNIT HANDLING: AppLovin API returns eCPM in dollars (e.g., 3.20 = $3.20)
      // We convert to cents for internal consistency
      const rows = Array.isArray(res.data) ? res.data : []
      return rows.slice(0, 100).map((r: any, idx: number) => {
        // Prefer explicit ecpm_cents if provided, otherwise convert ecpm from dollars
        let ecpmCents: number | undefined
        if (r.ecpm_cents !== undefined && r.ecpm_cents !== null) {
          ecpmCents = Number(r.ecpm_cents) || undefined
        } else if (r.ecpm !== undefined && r.ecpm !== null) {
          // API returns dollars, convert to cents
          ecpmCents = Math.round(Number(r.ecpm) * 100) || undefined
        }
        
        return {
          network: 'AppLovin',
          instanceId: String(r.unit_id ?? r.id ?? `max-${idx}`),
          instanceName: String(r.name ?? 'MAX Unit'),
          waterfallPosition: Number(r.position ?? idx + 1) || undefined,
          ecpmCents,
          confidence: 'high' as const,
        }
      })
    } catch (error: any) {
      // In live mode, propagate errors instead of falling back to sample data
      const status = error.response?.status
      const message = error.response?.data?.message || error.message || 'Unknown error'
      const connectorError: any = new Error(`AppLovin API error: ${message}`)
      connectorError.status = status || 500
      connectorError.source = 'AppLovin'
      throw connectorError
    }
  }

  const base = credentials.account_id.slice(-3)
  return [
    {
      network: 'AppLovin',
      instanceId: `max-v${base}`,
      instanceName: 'MAX Video',
      waterfallPosition: 1,
      ecpmCents: 320, // $3.20
      confidence: 'high',
    },
    {
      network: 'AppLovin',
      instanceId: `max-b${base}`,
      instanceName: 'MAX Banner',
      waterfallPosition: 3,
      ecpmCents: 95, // $0.95
      confidence: 'medium',
    },
  ]
}
