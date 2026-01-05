import { Request, Response } from 'express';
import { vraService } from '../services/vra/vraService';
import { getFeatureFlags } from '../utils/featureFlags';
import { query, insertMany } from '../utils/postgres';
import logger from '../utils/logger';
import { vraDisputeShadowAcksTotal, vraDisputesCreatedTotal } from '../utils/prometheus';
import { redactString as redactCsvField } from '../services/vra/redaction';
import { buildDisputeKit, resolveDisputeStorageFromEnv } from '../services/vra/disputeKitService';

function parseNumber(v: string | undefined): number | undefined {
  if (v == null) return undefined;
  const n = Number(v);
  return Number.isFinite(n) ? n : undefined;
}

function isIsoLike(v: string | undefined): boolean {
  if (!v) return true; // optional
  const t = Date.parse(v);
  return Number.isFinite(t);
}

function isWindowOrdered(from?: string, to?: string): boolean {
  if (!from || !to) return true; // if either missing, accept
  const f = Date.parse(from);
  const t = Date.parse(to);
  if (!Number.isFinite(f) || !Number.isFinite(t)) return false;
  return f <= t;
}

function validateDeltasQuery(q: Record<string, string | undefined>): { ok: true } | { ok: false; error: string } {
  // Validate ISO-ish dates for from/to
  if (!isIsoLike(q.from) || !isIsoLike(q.to)) {
    return { ok: false, error: 'Invalid from/to timestamp' };
  }
  // Enforce window ordering when both present
  if (!isWindowOrdered(q.from, q.to)) {
    return { ok: false, error: 'from must be <= to' };
  }
  // Validate pagination
  const page = parseNumber(q.page);
  const pageSize = parseNumber(q.page_size);
  if (page != null && (!Number.isInteger(page) || page < 1)) {
    return { ok: false, error: 'Invalid page' };
  }
  if (pageSize != null && (!Number.isInteger(pageSize) || pageSize < 1 || pageSize > 500)) {
    return { ok: false, error: 'Invalid page_size' };
  }
  // Validate min_conf 0..1
  const mc = parseNumber(q.min_conf);
  if (mc != null && (mc < 0 || mc > 1)) {
    return { ok: false, error: 'min_conf must be between 0 and 1' };
  }
  // kind is optional; if present must be one of known kinds
  if (q.kind) {
    const allowed = new Set(['underpay', 'missing', 'viewability_gap', 'ivt_outlier', 'fx_mismatch', 'timing_lag']);
    if (!allowed.has(q.kind)) {
      return { ok: false, error: 'Invalid kind' };
    }
  }
  return { ok: true };
}

export async function getReconOverview(req: Request, res: Response) {
  try {
    const { app_id: appId, from, to } = req.query as Record<string, string | undefined>;
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }
    // from/to are optional; if either is provided require both and validate
    if ((from && !to) || (!from && to)) {
      return res.status(400).json({ success: false, error: 'Both from and to are required' });
    }
    if ((from && !isIsoLike(from)) || (to && !isIsoLike(to))) {
      return res.status(400).json({ success: false, error: 'Invalid from/to timestamp' });
    }
    // Enforce window ordering when both are present
    if (!isWindowOrdered(from, to)) {
      return res.status(400).json({ success: false, error: 'from must be <= to' });
    }
    const data = await vraService.getOverview({ publisherId, appId, from, to });
    return res.json({ success: true, data });
  } catch (err) {
    logger.error('VRA getReconOverview failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

export async function getReconDeltas(req: Request, res: Response) {
  try {
    const q = req.query as Record<string, string | undefined>;
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }
    const v = validateDeltasQuery(q);
    if (!v.ok) {
      return res.status(400).json({ success: false, error: v.error });
    }
    const data = await vraService.getDeltas({
      publisherId,
      appId: q.app_id,
      from: q.from,
      to: q.to,
      kind: q.kind as any,
      minConf: q.min_conf ? Number(q.min_conf) : undefined,
      page: q.page ? Number(q.page) : undefined,
      pageSize: q.page_size ? Number(q.page_size) : undefined,
    });
    return res.json({ success: true, ...data });
  } catch (err) {
    logger.error('VRA getReconDeltas failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

export async function getReconDeltasCsv(req: Request, res: Response) {
  try {
    const q = req.query as Record<string, string | undefined>;
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }
    const v = validateDeltasQuery(q);
    if (!v.ok) {
      return res.status(400).json({ success: false, error: v.error });
    }
    const data = await vraService.getDeltas({
      publisherId,
      appId: q.app_id,
      from: q.from,
      to: q.to,
      kind: q.kind as any,
      minConf: q.min_conf ? Number(q.min_conf) : undefined,
      page: q.page ? Number(q.page) : undefined,
      pageSize: q.page_size ? Number(q.page_size) : undefined,
    });

    // Set headers for CSV download (streamed)
    res.setHeader('Content-Type', 'text/csv; charset=utf-8');
    // Build a helpful filename with optional window
    const ts = new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-');
    const fromSafe = (q.from || '').slice(0, 10);
    const toSafe = (q.to || '').slice(0, 10);
    const suffix = fromSafe && toSafe ? `_${fromSafe}_to_${toSafe}` : '';
    res.setHeader('Content-Disposition', `attachment; filename="recon_deltas${suffix}_${ts}.csv"`);

    // CSV header
    const header = 'kind,amount,currency,reason_code,window_start,window_end,evidence_id,confidence\n';
    res.write(header);
    for (const r of data.items) {
      // Redact any sensitive content from reason, then escape for CSV
      const redacted = redactCsvField(String(r.reasonCode || ''));
      const reason = redacted
        .replace(/\r|\n/g, ' ')
        .replace(/\t/g, ' ')
        .replace(/"/g, '""') // escape quotes if any
        .replace(/,/g, ' ');
      // Stable numeric formatting: amount to 6 decimals, confidence to 2 decimals
      const amountStr = (Number(r.amount) || 0).toFixed(6);
      const confStr = Number.isFinite(Number(r.confidence)) ? (Number(r.confidence) as number).toFixed(2) : '0.00';
      const line = [
        r.kind,
        amountStr,
        r.currency,
        reason,
        r.windowStart,
        r.windowEnd,
        r.evidenceId,
        confStr,
      ].join(',') + '\n';
      res.write(line);
    }
    res.end();
  } catch (err) {
    logger.error('VRA getReconDeltasCsv failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

export async function createDispute(req: Request, res: Response) {
  try {
    const flags = getFeatureFlags();
    const { delta_ids = [], network = '', contact = '' } = req.body || {};
    const normalizedNetwork = typeof network === 'string' ? network.trim() : '';
    if (!Array.isArray(delta_ids) || delta_ids.length === 0) {
      return res.status(400).json({ success: false, error: 'Invalid request body' });
    }
    if (!normalizedNetwork) {
      return res.status(400).json({ success: false, error: 'Invalid request body' });
    }
    const ids = delta_ids.map((x: any) => String(x).trim()).filter(Boolean);
    if (ids.length === 0) {
      return res.status(400).json({ success: false, error: 'Invalid request body' });
    }

    // Shadow-only: acknowledge without side-effects
    if (flags.vraShadowOnly) {
      const disputeId = `dry-${Date.now()}`;
      try {
        if (normalizedNetwork) {
          vraDisputeShadowAcksTotal.inc({ network: normalizedNetwork });
        } else {
          vraDisputeShadowAcksTotal.inc({ network: 'unknown' });
        }
      } catch (_) {
        // metrics are best-effort; never fail the request on metrics errors
      }
      return res.status(202).json({
        success: true,
        data: {
          dispute_id: disputeId,
          status: 'draft',
          evidence_uri: 'shadow://not-written',
          amount: 0,
          network: normalizedNetwork,
          contact,
          shadow: true,
        },
      });
    }

    // Sum amount from recon_deltas by evidence_id (delta_ids interpreted as evidence ids per VRA.md)
    const sumRows = await query<{ amount: string }>(
      `SELECT ROUND(COALESCE(SUM(amount), 0)::numeric, 6)::text AS amount
         FROM recon_deltas
        WHERE evidence_id = ANY($1::text[])`,
      [ids]
    );
    const amount = Number(sumRows.rows[0]?.amount || 0);
    const disputeId = `disp_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;

    // Attempt to build a Dispute Kit bundle and store it (shadow-first design; best-effort)
    let evidenceUri = `vra://disputes/${disputeId}`;
    try {
      const storage = resolveDisputeStorageFromEnv();
      const kit = await buildDisputeKit(ids, { network: normalizedNetwork, dryRun: false, storage });
      if (kit?.storageUri) {
        evidenceUri = kit.storageUri;
      }
    } catch (e) {
      // If kit building fails, fall back to placeholder URI. Do not fail the request.
      logger.warn('VRA createDispute: kit build failed, using placeholder URI', { error: (e as Error)?.message });
    }

    await insertMany(
      'recon_disputes',
      ['dispute_id', 'network', 'amount', 'status', 'evidence_uri'],
      [[disputeId, normalizedNetwork, amount, 'draft', evidenceUri]]
    );

    try {
      vraDisputesCreatedTotal.inc({ network: normalizedNetwork || 'unknown' });
    } catch (_) {
      // ignore metrics errors
    }

    return res.status(201).json({
      success: true,
      data: { dispute_id: disputeId, amount, status: 'draft', evidence_uri: evidenceUri },
    });
  } catch (err) {
    logger.error('VRA createDispute failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

export async function getMonthlyRevenueDigest(req: Request, res: Response) {
  try {
    const month = String((req.query?.month as string) || '').trim();
    if (!/^\d{4}-\d{2}$/.test(month)) {
      return res.status(400).json({ success: false, error: 'Invalid month format. Use YYYY-MM.' });
    }
    const digest = await vraService.getMonthlyDigest(month);
    if (!digest) {
      return res.status(404).json({ success: false, error: 'Digest not found' });
    }
    return res.json({ success: true, data: digest });
  } catch (err) {
    logger.error('VRA getMonthlyRevenueDigest failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}
