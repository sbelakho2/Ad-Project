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

/**
 * GET /api/v1/recon/review-queue
 * Returns mid-confidence matches (fuzzy matches between 0.4 and 0.8 confidence) 
 * that need manual review before being finalized.
 * 
 * FIXED: Join uses request_id (what matchEngine stores) instead of expected_id (which doesn't exist)
 */
export async function getReviewQueue(req: Request, res: Response) {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const q = req.query as Record<string, string | undefined>;
    
    // Validate pagination
    const page = q.page ? parseInt(q.page, 10) : 1;
    const pageSize = q.page_size ? parseInt(q.page_size, 10) : 50;
    
    if (!Number.isInteger(page) || page < 1) {
      return res.status(400).json({ success: false, error: 'Invalid page' });
    }
    if (!Number.isInteger(pageSize) || pageSize < 1 || pageSize > 200) {
      return res.status(400).json({ success: false, error: 'page_size must be between 1 and 200' });
    }

    // Min/max confidence for review queue (fuzzy match range)
    // Note: link_confidence is stored as 0-100 in DB, not 0-1
    const minConfInput = q.min_conf ? parseFloat(q.min_conf) : 0.4;
    const maxConfInput = q.max_conf ? parseFloat(q.max_conf) : 0.8;
    
    if (minConfInput < 0 || minConfInput > 1 || maxConfInput < 0 || maxConfInput > 1 || minConfInput >= maxConfInput) {
      return res.status(400).json({ success: false, error: 'Invalid confidence range' });
    }
    
    // Convert to percentage scale (0-100) for DB query
    const minConf = minConfInput * 100;
    const maxConf = maxConfInput * 100;

    // Query for review items from recon_match_review (review-band matches)
    // FIXED: Join on request_id which is what the matching engine actually stores
    const offset = (page - 1) * pageSize;
    
    const itemsResult = await query<{
      id: string;
      statement_id: string;
      request_id: string;
      link_confidence: string;
      keys_used: string;
      reasons: string;
      matched_at: string;
      expected_value: string;
      paid: string;
      network: string;
      event_date: string;
    }>(
      `SELECT 
         mr.id::text,
         mr.statement_id,
         mr.request_id,
         mr.link_confidence::text,
         mr.keys_used,
         COALESCE(mr.reasons::text, '{}') AS reasons,
         mr.matched_at::text,
         COALESCE(e.expected_value, 0)::text AS expected_value,
         COALESCE(s.paid, 0)::text AS paid,
         COALESCE(s.network, s.network_type, '') AS network,
         COALESCE(s.event_date::text, '') AS event_date
       FROM recon_match_review mr
       LEFT JOIN recon_expected e ON e.request_id = mr.request_id
       LEFT JOIN recon_statements_norm s ON s.id::text = mr.statement_id OR 
         (s.app_id || s.ad_unit_id || s.event_date::text = SUBSTRING(mr.statement_id FROM 1 FOR 64))
       WHERE mr.publisher_id = $1
         AND mr.link_confidence >= $2
         AND mr.link_confidence <= $3
       ORDER BY mr.link_confidence ASC, mr.matched_at DESC
       LIMIT $4 OFFSET $5`,
      [publisherId, minConf, maxConf, pageSize, offset]
    );

    // Get total count
    const countResult = await query<{ count: string }>(
      `SELECT COUNT(*)::text AS count
       FROM recon_match_review
       WHERE publisher_id = $1
         AND link_confidence >= $2
         AND link_confidence <= $3`,
      [publisherId, minConf, maxConf]
    );
    
    const total = parseInt(countResult.rows[0]?.count || '0', 10);
    const totalPages = Math.ceil(total / pageSize);

    const items = itemsResult.rows.map(row => ({
      id: row.id,
      statementId: row.statement_id,
      requestId: row.request_id,
      confidence: parseFloat(row.link_confidence) / 100, // Convert back to 0-1 for API response
      keysUsed: row.keys_used,
      reasons: JSON.parse(row.reasons || '{}'),
      createdAt: row.matched_at,
      expectedAmount: parseFloat(row.expected_value),
      statementAmount: parseFloat(row.paid),
      networkType: row.network,
      eventDate: row.event_date,
    }));

    return res.json({
      success: true,
      data: {
        items,
        pagination: {
          page,
          pageSize,
          total,
          totalPages,
        },
        filters: {
          minConf: minConfInput,
          maxConf: maxConfInput,
        },
      },
    });
  } catch (err) {
    logger.error('VRA getReviewQueue failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

/**
 * POST /api/v1/recon/review-queue/:id/approve
 * Approves a mid-confidence match, promoting it to confirmed
 */
export async function approveReviewItem(req: Request, res: Response) {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const { id } = req.params;
    if (!id) {
      return res.status(400).json({ success: false, error: 'Missing review item ID' });
    }

    // Update the match to mark it as approved
    const result = await query<{ id: string }>(
      `UPDATE recon_match
       SET review_status = 'approved',
           reviewed_at = NOW(),
           reviewed_by = $3
       WHERE id = $1 AND publisher_id = $2
       RETURNING id`,
      [id, publisherId, req.user?.email || 'api']
    );

    if (result.rows.length === 0) {
      return res.status(404).json({ success: false, error: 'Review item not found' });
    }

    logger.info('VRA review item approved', { id, publisherId, reviewedBy: req.user?.email });

    return res.json({
      success: true,
      data: {
        id,
        status: 'approved',
        reviewedAt: new Date().toISOString(),
      },
    });
  } catch (err) {
    logger.error('VRA approveReviewItem failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

/**
 * POST /api/v1/recon/review-queue/:id/reject
 * Rejects a mid-confidence match, flagging it for investigation
 */
export async function rejectReviewItem(req: Request, res: Response) {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const { id } = req.params;
    if (!id) {
      return res.status(400).json({ success: false, error: 'Missing review item ID' });
    }

    const { reason } = req.body || {};
    const rejectReason = typeof reason === 'string' ? reason.slice(0, 500) : '';

    // Update the match to mark it as rejected
    const result = await query<{ id: string }>(
      `UPDATE recon_match
       SET review_status = 'rejected',
           reviewed_at = NOW(),
           reviewed_by = $3,
           review_notes = $4
       WHERE id = $1 AND publisher_id = $2
       RETURNING id`,
      [id, publisherId, req.user?.email || 'api', rejectReason]
    );

    if (result.rows.length === 0) {
      return res.status(404).json({ success: false, error: 'Review item not found' });
    }

    logger.info('VRA review item rejected', { id, publisherId, reviewedBy: req.user?.email, reason: rejectReason });

    return res.json({
      success: true,
      data: {
        id,
        status: 'rejected',
        reason: rejectReason,
        reviewedAt: new Date().toISOString(),
      },
    });
  } catch (err) {
    logger.error('VRA rejectReviewItem failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

// Import dispute lifecycle service
import disputeLifecycle from '../services/vra/disputeLifecycleService';

/**
 * GET /api/v1/recon/disputes
 * List disputes for the authenticated publisher
 */
export async function listDisputes(req: Request, res: Response) {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const q = req.query as Record<string, string | undefined>;
    const page = q.page ? parseInt(q.page, 10) : 1;
    const pageSize = q.page_size ? parseInt(q.page_size, 10) : 50;

    if (!Number.isInteger(page) || page < 1) {
      return res.status(400).json({ success: false, error: 'Invalid page' });
    }
    if (!Number.isInteger(pageSize) || pageSize < 1 || pageSize > 100) {
      return res.status(400).json({ success: false, error: 'page_size must be between 1 and 100' });
    }

    const result = await disputeLifecycle.listDisputes({
      publisherId,
      network: q.network,
      status: q.status as any,
      page,
      pageSize,
    });

    return res.json({
      success: true,
      data: {
        items: result.items,
        pagination: {
          page,
          pageSize,
          total: result.total,
          totalPages: Math.ceil(result.total / pageSize),
        },
      },
    });
  } catch (err) {
    logger.error('VRA listDisputes failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

/**
 * GET /api/v1/recon/disputes/:id
 * Get a specific dispute by ID
 */
export async function getDisputeById(req: Request, res: Response) {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const { id } = req.params;
    // SECURITY: Pass publisherId to enforce tenant isolation at query level
    const dispute = await disputeLifecycle.getDispute(id, publisherId);

    if (!dispute) {
      return res.status(404).json({ success: false, error: 'Dispute not found' });
    }

    return res.json({ success: true, data: dispute });
  } catch (err) {
    logger.error('VRA getDisputeById failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

/**
 * POST /api/v1/recon/disputes/:id/submit
 * Submit a draft dispute to the network
 */
export async function submitDisputeById(req: Request, res: Response) {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const { id } = req.params;
    const { contact, notes } = req.body || {};

    // SECURITY: Pass publisherId to enforce tenant isolation at query level
    const dispute = await disputeLifecycle.getDispute(id, publisherId);
    if (!dispute) {
      return res.status(404).json({ success: false, error: 'Dispute not found' });
    }

    const result = await disputeLifecycle.submitDispute(id, { contact, notes });

    if (!result.success) {
      return res.status(400).json({ success: false, error: result.error });
    }

    return res.json({ success: true, data: result.dispute });
  } catch (err) {
    logger.error('VRA submitDisputeById failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

/**
 * POST /api/v1/recon/disputes/:id/cancel
 * Cancel a dispute
 */
export async function cancelDisputeById(req: Request, res: Response) {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const { id } = req.params;
    const { reason } = req.body || {};

    // SECURITY: Pass publisherId to enforce tenant isolation at query level
    const dispute = await disputeLifecycle.getDispute(id, publisherId);
    if (!dispute) {
      return res.status(404).json({ success: false, error: 'Dispute not found' });
    }

    const result = await disputeLifecycle.cancelDispute(id, reason);

    if (!result.success) {
      return res.status(400).json({ success: false, error: result.error });
    }

    return res.json({ success: true, data: result.dispute });
  } catch (err) {
    logger.error('VRA cancelDisputeById failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}

/**
 * GET /api/v1/recon/disputes/stats
 * Get dispute statistics for the publisher
 */
export async function getDisputeStats(req: Request, res: Response) {
  try {
    const publisherId = req.user?.publisherId;
    if (!publisherId) {
      return res.status(401).json({ success: false, error: 'Unauthorized' });
    }

    const stats = await disputeLifecycle.getDisputeStats(publisherId);
    return res.json({ success: true, data: stats });
  } catch (err) {
    logger.error('VRA getDisputeStats failed', { error: (err as Error)?.message });
    return res.status(500).json({ success: false, error: 'Internal error' });
  }
}
