import crypto from 'crypto';
import { registerDefaultAdapters, getAdaptersForFormat } from './adapterRegistry';
import { AdapterBid, AdapterBidRequest, AdapterDefinition } from './adapters/types';
import { auctionLatencySeconds, rtbErrorsTotal, rtbNoFillTotal, rtbWinsTotal } from '../../utils/prometheus';
import { safeInc } from '../../utils/metrics';
import type { ExperimentArm, ExperimentMode } from '../../types/migration';
import { signToken } from '../../utils/signing';
import { recordShadowOutcome, OutcomeStatus, CandidateBidSnapshot } from './shadowRecorder';
import config from '../../config/index';
import * as breaker from '../../utils/redisCircuitBreaker';
import { generateLandscapeId } from './auctionIdempotency';
import { scoreShadow, ShadowScoreInput } from '../inference/shadowFraudScoring';

const envInt = (key: string, dflt: number) => {
  const v = parseInt(process.env[key] || '', 10);
  return Number.isFinite(v) ? v : dflt;
};

const AUCTION_TTL_MS = envInt('AUCTION_TTL_MS', 120);
const DELIVERY_TOKEN_TTL_SEC = envInt('DELIVERY_TOKEN_TTL_SEC', 300);
const TRACK_TOKEN_TTL_SEC = envInt('TRACK_TOKEN_TTL_SEC', 600);

export interface MigrationContext {
  experimentId: string;
  arm: ExperimentArm;
  assignmentTs: string;
  mirrorPercent?: number;
  mode?: ExperimentMode;
}

export interface AuctionInput extends AdapterBidRequest {
  migration?: MigrationContext;
}

export interface AuctionOutput {
  success: boolean;
  landscapeId: string;
  response?: {
    requestId: string;
    landscapeId: string;
    bidId: string;
    adapter: string;
    cpm: number;
    currency: 'USD';
    ttlSeconds: number;
    creativeUrl: string; // contains signed delivery token
    tracking: { impression: string; click: string };
    payload: Record<string, unknown>;
    consentEcho?: Record<string, unknown>;
  };
  reason?: string;
  latencyMs?: number;
}

const makeAbortError = () => { const e: any = new Error('Aborted'); e.name = 'AbortError'; return e; };
function withTimeout<T>(p: Promise<T>, ms: number, signal: AbortSignal): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const to = setTimeout(() => {
      (signal as any)._timeoutFired = true;
      reject(makeAbortError());
    }, ms);
    signal.addEventListener('abort', () => {
      clearTimeout(to);
      reject(makeAbortError());
    }, { once: true });
    p.then((v) => { clearTimeout(to); resolve(v); }, (e) => { clearTimeout(to); reject(e); });
  });
}

export async function runAuction(input: AuctionInput, baseUrl: string): Promise<AuctionOutput> {
  registerDefaultAdapters();

  const requestId = input.requestId || crypto.randomUUID();
  const migration = input.migration;
  const mode: ExperimentMode = migration?.mode ?? 'shadow';
  const armLabel = migration ? migration.arm : 'control';
  const metricLabels = { arm: armLabel, exp_id: migration?.experimentId ?? 'none' } as const;
  const decisionStartedAt = Date.now();

  const baseMetadata = migration
    ? {
        assignment_ts: migration.assignmentTs,
        mirror_percent: migration.mirrorPercent ?? null,
        ad_format: input.adFormat,
        floor_cpm: input.floorCpm,
      }
    : undefined;

  const end = auctionLatencySeconds.startTimer(metricLabels);

  const recordOutcome = async (status: OutcomeStatus, options?: Partial<Omit<Parameters<typeof recordShadowOutcome>[0], 'experimentId' | 'requestId' | 'placementId' | 'arm' | 'mode'>>) => {
    if (!migration) return;

    await recordShadowOutcome({
      experimentId: migration.experimentId,
      requestId,
      placementId: input.placementId,
      arm: migration.arm,
      mode,
      status,
      latencyMs: Date.now() - decisionStartedAt,
      mirrorPercent: migration.mirrorPercent,
      ...(options || {}),
      metadata: {
        ...(baseMetadata || {}),
        ...((options && options.metadata) || {}),
      },
    });
  };

  if (migration && migration.arm === 'control') {
    const landscapeId = generateLandscapeId(requestId);
    await recordOutcome('skipped', {
      metadata: {
        ...(baseMetadata || {}),
        skip_reason: 'control_arm',
      },
    });
    return { success: false, landscapeId, reason: 'CONTROL_ARM' };
  }

  try {
  const adapters = getAdaptersForFormat(input.adFormat);
    if (adapters.length === 0) {
      const landscapeId = generateLandscapeId(requestId);
      rtbErrorsTotal.inc({ code: 'NO_ADAPTER', adapter: 'none', ...metricLabels });
      if (migration) {
        await recordOutcome('error', {
          errorReason: 'NO_ADAPTER',
          metadata: {
            ...(baseMetadata || {}),
            adapter_count: 0,
          },
        });
      }
      return { success: false, landscapeId, reason: 'NO_ADAPTER' };
    }

    const abort = new AbortController();
    const deadline = Date.now() + AUCTION_TTL_MS;

    const tasks = adapters.map(async (adapter) => ({
      adapter,
      result: await bidWithAdapter(adapter, input, abort.signal, deadline),
    }));
    const results = await Promise.all(tasks);

    const candidateSnapshots: CandidateBidSnapshot[] = results.map(({ adapter, result }) => {
      if (!result) {
        return { adapter: adapter.name, status: 'nobid', reason: 'NO_RESPONSE' };
      }

      if ((result as any).nobid) {
        return {
          adapter: (result as any).adapter || adapter.name,
          status: 'nobid',
          reason: (result as any).reason,
        };
      }

      const bid = result as AdapterBid;
      return {
        adapter: bid.adapter || adapter.name,
        status: 'bid',
        cpm: bid.cpm,
        latencyMs: bid.latencyMs,
      };
    });

    const bids = results
      .map(({ result }) => result)
      .filter((r): r is AdapterBid => Boolean(r) && !(r as any).nobid);

    if (bids.length === 0) {
      const landscapeId = generateLandscapeId(requestId);
      if (migration) {
        await recordOutcome('no_fill', {
          bids: candidateSnapshots,
          metadata: {
            ...(baseMetadata || {}),
            candidate_count: candidateSnapshots.length,
          },
        });
      }

      if (!migration || mode === 'mirroring') {
        safeInc(rtbNoFillTotal, metricLabels);
      }

      if (migration && mode === 'shadow') {
        return { success: false, landscapeId, reason: 'SHADOW_NO_BID', latencyMs: Date.now() - decisionStartedAt };
      }

      return { success: true, landscapeId, response: undefined, reason: 'NO_BID', latencyMs: Date.now() - decisionStartedAt };
    }

    bids.sort((a: any, b: any) => (b.cpm - a.cpm) || ((a.latencyMs || 9999) - (b.latencyMs || 9999)));
    const winner = bids[0] as any;
    abort.abort();

    const latencyMs = Date.now() - decisionStartedAt;
    const bidId = crypto.randomUUID();
    const landscapeId = generateLandscapeId(requestId, bidId);

    // Shadow fraud scoring: score ALL requests but NEVER block traffic (SDK_CHECKS 7.1)
    // This is async/fire-and-forget - auction proceeds regardless of scoring result
    const deviceRecord = (input.device as Record<string, unknown>) || {};
    void scoreShadow({
      requestId,
      placementId: input.placementId,
      deviceInfo: {
        ip: deviceRecord.ip as string | undefined,
        userAgent: deviceRecord.ua as string | undefined,
        platform: deviceRecord.os as string | undefined,
        osVersion: deviceRecord.osv as string | undefined,
      },
      auctionContext: {
        adFormat: input.adFormat,
        floorCpm: input.floorCpm,
        bidCount: bids.length,
        winningCpm: winner.cpm,
        latencyMs,
      },
      metadata: migration ? { experimentId: migration.experimentId, arm: migration.arm } : undefined,
    }).catch(() => { /* silent fail - shadow scoring must never impact auction */ });

    if (migration && mode === 'shadow') {
      await recordOutcome('win', {
        adapterName: winner.adapter,
        adapterId: winner.adapter,
        bidCpm: winner.cpm,
        currency: winner.currency,
        latencyMs,
        bids: candidateSnapshots,
        metadata: {
          ...(baseMetadata || {}),
          ttl_seconds: winner.ttlSeconds || 0,
          delivered: false,
        },
      });

      return { success: false, landscapeId, reason: 'SHADOW_MODE', latencyMs };
    }

    const impressionToken = signToken({
      bidId,
      placementId: input.placementId,
      adapter: winner.adapter,
      cpm: winner.cpm,
      currency: 'USD',
      purpose: 'imp',
      nonce: crypto.randomUUID(),
    }, TRACK_TOKEN_TTL_SEC);

    const clickToken = signToken({
      bidId,
      placementId: input.placementId,
      adapter: winner.adapter,
      cpm: winner.cpm,
      currency: 'USD',
      purpose: 'click',
      nonce: crypto.randomUUID(),
    }, TRACK_TOKEN_TTL_SEC);

    const deliveryToken = signToken({
      bidId,
      placementId: input.placementId,
      adapter: winner.adapter,
      cpm: winner.cpm,
      currency: 'USD',
      purpose: 'delivery',
      nonce: crypto.randomUUID(),
      url: winner.creativeUrl,
    }, DELIVERY_TOKEN_TTL_SEC);

    const impression = `${baseUrl}/api/v1/rtb/t/imp?token=${encodeURIComponent(impressionToken)}`;
    const click = `${baseUrl}/api/v1/rtb/t/click?token=${encodeURIComponent(clickToken)}`;
    const creativeUrl = `${baseUrl}/api/v1/rtb/creative?token=${encodeURIComponent(deliveryToken)}`;

    safeInc(rtbWinsTotal, { adapter: winner.adapter, ...metricLabels });

    if (migration) {
      await recordOutcome('win', {
        adapterName: winner.adapter,
        adapterId: winner.adapter,
        bidCpm: winner.cpm,
        currency: winner.currency,
        latencyMs,
        bids: candidateSnapshots,
        metadata: {
          ...(baseMetadata || {}),
          ttl_seconds: winner.ttlSeconds || 0,
          delivered: true,
        },
      });
    }

    const payload: Record<string, unknown> = { ...(winner.meta || {}) };
    if (migration) {
      payload.migration = {
        experiment_id: migration.experimentId,
        arm: migration.arm,
        assignment_ts: migration.assignmentTs,
        ...(typeof migration.mirrorPercent === 'number'
          ? { mirror_percent: migration.mirrorPercent }
          : {}),
        ...(mode ? { mode } : {}),
      };
    }

    return {
      success: true,
      landscapeId,
      latencyMs,
      response: {
        requestId,
        landscapeId,
        bidId,
        adapter: winner.adapter,
        cpm: winner.cpm,
        currency: 'USD',
        ttlSeconds: winner.ttlSeconds || 300,
        creativeUrl,
        tracking: { impression, click },
        payload,
        consentEcho: input.consent || {},
      }
    };
  } catch (error) {
    if (migration) {
      await recordOutcome('error', {
        errorReason: (error as Error)?.message || 'UNKNOWN_ERROR',
        metadata: {
          ...(baseMetadata || {}),
          stack: (error as Error)?.stack,
        },
      });
    }
    throw error;
  } finally {
    try { end(); } catch (e) { void e; }
  }
}

async function bidWithAdapter(adapter: AdapterDefinition, req: AdapterBidRequest, signal: AbortSignal, deadlineTs: number) {
  const remaining = Math.max(1, deadlineTs - Date.now());
  const perAdapterTimeout = Math.min(adapter.timeoutMs, remaining);
  const ctl = new AbortController();
  const onAbort = () => ctl.abort();
  signal.addEventListener('abort', onAbort, { once: true });
  try {
    // If breakers enabled and circuit is open, skip calling the adapter
    if (config.redisBreakersEnabled) {
      const allowed = await breaker.allow(adapter.name);
      if (!allowed) {
        return { nobid: true, reason: 'CIRCUIT_OPEN', adapter: adapter.name } as any;
      }
    }
    const result = await withTimeout(
      adapter.requestBid({ signal: ctl.signal, deadlineMs: perAdapterTimeout }, req),
      perAdapterTimeout,
      ctl.signal
    );

    if (result && (result as any).nobid) {
      // nobid is a valid outcome, not a failure
      if (config.redisBreakersEnabled) { await breaker.recordSuccess(adapter.name); }
      return { ...(result as any), adapter: adapter.name } as any;
    }

    // Successful bid
    if (config.redisBreakersEnabled) { await breaker.recordSuccess(adapter.name); }
    return result;
  } catch (e) {
    // Record failure for breaker logic
    if (config.redisBreakersEnabled) { await breaker.recordFailure(adapter.name); }
    return {
      nobid: true,
      reason: (e as any)?.name === 'AbortError' ? 'TIMEOUT' : 'ERROR',
      adapter: adapter.name,
    } as any;
  } finally {
    signal.removeEventListener('abort', onAbort);
  }
}
