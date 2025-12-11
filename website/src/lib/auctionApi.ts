/**
 * Auction service API client (Go service)
 * Separate from Node backend API to avoid proxying for read-only admin endpoints.
 * Adds limited retries/backoff, optional auth header, and de‑duping.
 */

// In production, require NEXT_PUBLIC_AUCTION_URL to be set
const AUCTION_URL_RAW = process.env.NEXT_PUBLIC_AUCTION_URL;
if (process.env.NODE_ENV === 'production' && !AUCTION_URL_RAW) {
  console.warn('[auctionApi] NEXT_PUBLIC_AUCTION_URL not set, auction features will be disabled');
}
const AUCTION_URL = (AUCTION_URL_RAW || '/api/auction').replace(/\/$/, '');

export interface ApiEnvelope<T = any> {
  success: boolean;
  data?: T;
  error?: string;
  status?: number;
}

type Method = 'GET' | 'POST' | 'PUT' | 'DELETE' | 'PATCH'

const inflight = new Map<string, Promise<ApiEnvelope<any>>>();

function keyOf(url: string, method: Method, body?: unknown) {
  const b = method === 'GET' ? '' : JSON.stringify(body ?? '');
  return `${method} ${url} ${b}`;
}

async function fetchWithRetry(url: string, init: RequestInit, attempts = 3, backoffMs = 200): Promise<Response> {
  let lastErr: any;
  for (let i = 0; i < attempts; i++) {
    try {
      const res = await fetch(url, init);
      if (res.ok || (res.status >= 400 && res.status < 500)) return res; // don't retry 4xx
      lastErr = new Error(`HTTP ${res.status}`);
    } catch (e) {
      lastErr = e;
    }
    const delay = backoffMs * Math.pow(2, i) + Math.floor(Math.random() * 50);
    await new Promise((r) => setTimeout(r, delay));
  }
  throw lastErr;
}

async function request<T = any>(
  endpoint: string,
  options: RequestInit & { method?: Method; attempts?: number; token?: string; dedupeKey?: string } = {}
): Promise<ApiEnvelope<T>> {
  const url = `${AUCTION_URL}${endpoint}`;
  const method = (options.method || 'GET') as Method;
  const headers: HeadersInit = {
    'Content-Type': 'application/json',
    ...(options.headers || {}),
  };
  // Optional bearer token for services that require it
  if (options.token && !('authorization' in (headers as any))) {
    (headers as any).Authorization = `Bearer ${options.token}`;
  }

  const dedupeKey = options.dedupeKey || keyOf(url, method, (options as any).body);
  if (inflight.has(dedupeKey)) return inflight.get(dedupeKey)! as Promise<ApiEnvelope<T>>;

  const exec = (async () => {
    try {
      const controller = typeof AbortController !== 'undefined' ? new AbortController() : undefined;
      const res = await fetchWithRetry(
        url,
        {
          ...options,
          headers,
          cache: 'no-store',
          signal: controller?.signal,
        },
        options.attempts ?? 3
      );
      const contentType = res.headers.get('content-type') || '';
      const isJson = contentType.includes('application/json');
      const body = isJson ? await res.json().catch(() => ({})) : await res.text();
      if (!res.ok) {
        const msg = (isJson && (body as any)?.error) || `Request failed (${res.status})`;
        return { success: false, error: msg, status: res.status };
      }
      const data = (isJson ? (body as any) : body) as any;
      return { success: true, data: (data?.data ?? data) as T, status: res.status };
    } catch (e: any) {
      return { success: false, error: e?.message || 'Network error' };
    } finally {
      inflight.delete(dedupeKey);
    }
  })();

  inflight.set(dedupeKey, exec as Promise<ApiEnvelope<any>>);
  return exec as Promise<ApiEnvelope<T>>;
}

export const auctionApi = {
  getAdapterMetrics: (options?: RequestInit & { attempts?: number; token?: string; dedupeKey?: string }) =>
    request('/v1/metrics/adapters', options as any),
  getAdapterMetricsTimeSeries: (days = 7, options?: RequestInit & { attempts?: number; token?: string; dedupeKey?: string }) => {
    const d = Math.max(1, Math.min(14, days));
    return request(`/v1/metrics/adapters/timeseries?days=${d}`, options as any);
  },
  getAdapterSLO: (options?: RequestInit & { attempts?: number; token?: string; dedupeKey?: string }) =>
    request('/v1/metrics/slo', options as any),
  getMediationDebugEvents: (placementId = '', n = 50, options?: RequestInit & { attempts?: number; token?: string; dedupeKey?: string }) => {
    const params = new URLSearchParams();
    if (placementId) params.set('placement_id', placementId);
    if (n) params.set('n', String(n));
    const qs = params.toString();
    return request(`/v1/debug/mediation${qs ? `?${qs}` : ''}`, options as any);
  },
};
