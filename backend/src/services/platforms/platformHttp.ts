/**
 * Shared HTTP transport for the platform management-API clients.
 *
 * Behaviors pinned by adversarial tests (backend/tests/platforms/platformHttp.test.ts):
 * - 401/403  -> fail fast, never retried (key is wrong; retrying burns rate limits)
 * - 429/5xx  -> retried up to maxRetries with bounded exponential backoff, honoring
 *               Retry-After when present (capped, to never hang a scheduler tick)
 * - malformed JSON / schema failures -> fail CLOSED (never treat garbage as success)
 * - writes are retried only when the caller supplies an idempotency key
 * - every error is mapped to PlatformApiError with an explicit retryable classification
 */
import type { AxiosInstance } from 'axios';
import { PlatformApiError, isRetryableStatus, schemaError, type PlatformId } from './types';

export const MAX_RETRY_AFTER_MS = 60_000;

export interface PlatformHttpOptions {
  platform: PlatformId;
  baseUrl: string;
  /** Builds the auth headers for this platform/key pair (called per request). */
  auth: () => Record<string, string>;
  /** Injectable for tests; defaults to a fresh axios instance via require('axios'). */
  client?: AxiosInstance;
  maxRetries?: number;
  timeoutMs?: number;
  /** Injectable for deterministic tests. */
  sleep?: (ms: number) => Promise<void>;
  now?: () => number;
}

export interface RequestArgs {
  operation: string;
  method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  path: string;
  params?: Record<string, string | number>;
  body?: unknown;
  /** When set, a non-idempotent request may be retried on 429/5xx. */
  idempotencyKey?: string;
}

function parseRetryAfter(header: unknown, nowMs: number): number | undefined {
  if (typeof header !== 'string' || header.trim() === '') return undefined;
  const seconds = Number(header);
  if (Number.isFinite(seconds) && seconds >= 0) return Math.min(seconds * 1000, MAX_RETRY_AFTER_MS);
  const at = Date.parse(header);
  if (!Number.isNaN(at)) return Math.min(Math.max(0, at - nowMs), MAX_RETRY_AFTER_MS);
  return undefined;
}

export class PlatformHttp {
  private readonly maxRetries: number;
  private readonly timeoutMs: number;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly now: () => number;
  private readonly client: AxiosInstance;

  constructor(private readonly opts: PlatformHttpOptions) {
    this.maxRetries = opts.maxRetries ?? 2;
    this.timeoutMs = opts.timeoutMs ?? 10_000;
    this.sleep = opts.sleep ?? ((ms) => new Promise((r) => setTimeout(r, ms)));
    this.now = opts.now ?? (() => Date.now());
    if (!opts.client) {
      // Lazy require keeps this module importable in fixture-only tests without deps.
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const axios = require('axios');
      this.client = axios.create();
    } else {
      this.client = opts.client;
    }
  }

  async request<T>(args: RequestArgs): Promise<T> {
    const isWrite = args.method !== 'GET';
    const canRetryCall = !isWrite || Boolean(args.idempotencyKey);
    const attempts = canRetryCall ? this.maxRetries + 1 : 1;

    let lastError: PlatformApiError | undefined;
    for (let attempt = 0; attempt < attempts; attempt++) {
      if (attempt > 0) {
        const backoff = Math.min(250 * 2 ** (attempt - 1), 2000);
        const wait = lastError?.code === 'RATE_LIMITED' && lastError.retryAfterMs !== undefined ? lastError.retryAfterMs : backoff;
        await this.sleep(wait);
      }

      let response;
      try {
        response = await this.client.request({
          url: `${this.opts.baseUrl}${args.path}`,
          method: args.method,
          params: args.params,
          data: args.body,
          timeout: this.timeoutMs,
          headers: {
            ...this.opts.auth(),
            ...(args.idempotencyKey ? { 'Idempotency-Key': args.idempotencyKey } : {}),
          },
          validateStatus: () => true,
        });
      } catch (cause) {
        // Network-level failure: retryable only for read-ish calls (same policy as HTTP 5xx).
        lastError = new PlatformApiError(
          `${this.opts.platform} ${args.operation} failed: network error`,
          this.opts.platform,
          args.operation,
          { code: 'NETWORK', retryable: canRetryCall, cause },
        );
        if (!lastError.retryable) throw lastError;
        continue;
      }

      const status: number = response.status;
      if (status >= 200 && status < 300) {
        return response.data as T;
      }

      const retryAfterMs = parseRetryAfter((response.headers as Record<string, unknown>)?.['retry-after'], this.now());
      const retryable = isRetryableStatus(status) && canRetryCall;
      lastError = new PlatformApiError(
        `${this.opts.platform} ${args.operation} failed with HTTP ${status}`,
        this.opts.platform,
        args.operation,
        {
          status,
          code: status === 429 ? 'RATE_LIMITED' : status === 401 || status === 403 ? 'AUTH' : `HTTP_${status}`,
          retryable,
          retryAfterMs,
        },
      );
      if (!retryable) throw lastError;
    }

    throw lastError ?? new PlatformApiError(`${this.opts.platform} ${args.operation} failed`, this.opts.platform, args.operation, { retryable: false });
  }

  /** GET with fail-closed JSON-shape validation. */
  async getValidated<T>(operation: string, path: string, params: Record<string, string | number> | undefined, guard: (data: unknown) => data is T): Promise<T> {
    const data = await this.request<unknown>({ operation, method: 'GET', path, params });
    if (!guard(data)) throw schemaError(this.opts.platform, operation, 'response failed shape validation');
    return data;
  }
}
