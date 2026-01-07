import { AdResponseSchema } from './schemas';
import type { AdRequest, AdResponse, ConsentState, InitOptions } from './types';
import { Errors, SdkError } from './errors';

export type AuctionClientConfig = Required<
  Pick<InitOptions, 'endpoint' | 'timeoutMs' | 'maxRetries' | 'retryBackoffBaseMs' | 'retryJitterMs'>
> & {
  publisherId?: string;
  appId?: string;
  sdkVersion: string;
  correlationIdGenerator?: () => string;
};

/**
 * Generate a correlation ID for distributed tracing
 * Uses crypto.randomUUID if available, falls back to timestamp-based ID
 */
function generateCorrelationId(): string {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  // Fallback for older browsers
  return `${Date.now().toString(36)}-${Math.random().toString(36).substring(2, 11)}`;
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const t = setTimeout(resolve, ms);
    if (signal) {
      signal.addEventListener(
        'abort',
        () => {
          clearTimeout(t);
          // Match fetch Abort naming where possible
          const err: any = new Error('Aborted');
          err.name = 'AbortError';
          reject(err);
        },
        { once: true }
      );
    }
  });
}

function isRetryableStatus(status: number): boolean {
  return status >= 500 && status < 600; // retry 5xx
}

export async function requestAuction(
  cfg: AuctionClientConfig,
  consent: ConsentState | undefined,
  req: AdRequest
): Promise<AdResponse> {
  const endpoint = `${cfg.endpoint.replace(/\/$/, '')}/auction`;

  // Generate correlation ID for distributed tracing
  const correlationId = cfg.correlationIdGenerator
    ? cfg.correlationIdGenerator()
    : generateCorrelationId();

  let attempt = 0;
  // Use a local controller per attempt so backoff sleep can also be aborted
  const globalController = new AbortController();
  const globalTimer = setTimeout(() => globalController.abort(), cfg.timeoutMs);

  try {
    // eslint-disable-next-line no-constant-condition
    while (true) {
      attempt++;
      const controller = new AbortController();
      const onAbort = () => controller.abort();
      globalController.signal.addEventListener('abort', onAbort, { once: true });
      try {
        const res = await fetch(endpoint, {
          method: 'POST',
          headers: {
            'Content-Type': 'application/json',
            'X-Correlation-ID': correlationId,
            'X-SDK-Version': cfg.sdkVersion,
          },
          body: JSON.stringify({
            request: req,
            consent: consent ?? {},
            meta: {
              sdk: { name: '@rivalapex/web-sdk', version: cfg.sdkVersion },
              publisherId: cfg.publisherId,
              appId: cfg.appId,
              correlationId,
            },
          }),
          signal: controller.signal,
        });

        if (!res.ok) {
          // Non-2xx
          let body: unknown = undefined;
          try {
            body = await res.json();
          } catch {
            // ignore parse error for error bodies
          }
          if (isRetryableStatus(res.status) && attempt <= (cfg.maxRetries ?? 0)) {
            const backoff = cfg.retryBackoffBaseMs * Math.pow(2, attempt - 1);
            const jitter = (Math.random() * (cfg.retryJitterMs * 2)) - cfg.retryJitterMs;
            await sleep(Math.max(0, Math.round(backoff + jitter)), globalController.signal);
            continue; // retry
          }
          throw Errors.badResponse(res.status, body);
        }

        let json: unknown;
        try {
          json = await res.json();
        } catch (e) {
          throw Errors.badResponse(200, { parseError: String(e) });
        }

        const parsed = AdResponseSchema.safeParse(json);
        if (!parsed.success) {
          throw Errors.validation('Invalid ad response schema', { issues: parsed.error.issues });
        }

        const data = parsed.data;
        if (!data.fill) {
          throw Errors.noFill();
        }

        return data;
      } catch (e: any) {
        if (e instanceof SdkError) throw e;
        const nameStr = String((e && (e.name || (e.constructor && e.constructor.name))) || '').toLowerCase();
        if (nameStr.includes('abort')) {
          // Global timeout always maps to TIMEOUT; otherwise, if aborted mid-attempt and retries left, retry
          if (globalController.signal.aborted) throw Errors.timeout(cfg.timeoutMs);
          if (attempt <= (cfg.maxRetries ?? 0)) {
            const backoff = cfg.retryBackoffBaseMs * Math.pow(2, attempt - 1);
            const jitter = (Math.random() * (cfg.retryJitterMs * 2)) - cfg.retryJitterMs;
            await sleep(Math.max(0, Math.round(backoff + jitter)), globalController.signal);
            continue;
          }
          throw Errors.timeout(cfg.timeoutMs);
        }
        // Network error (TypeError in fetch) → retry if attempts left
        if (attempt <= (cfg.maxRetries ?? 0)) {
          const backoff = cfg.retryBackoffBaseMs * Math.pow(2, attempt - 1);
          const jitter = (Math.random() * (cfg.retryJitterMs * 2)) - cfg.retryJitterMs;
          await sleep(Math.max(0, Math.round(backoff + jitter)), globalController.signal);
          continue;
        }
        throw Errors.network('Network error while requesting auction', undefined, e);
      } finally {
        globalController.signal.removeEventListener('abort', onAbort as any);
      }
    }
  } catch (e: any) {
    const nameStr = String((e && (e.name || (e.constructor && e.constructor.name))) || '').toLowerCase();
    if (nameStr.includes('abort')) {
      throw Errors.timeout(cfg.timeoutMs);
    }
    throw e;
  } finally {
    clearTimeout(globalTimer);
  }
}
