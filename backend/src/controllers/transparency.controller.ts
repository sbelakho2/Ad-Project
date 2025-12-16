import { Request, Response, NextFunction } from 'express';
import type { QueryResultRow } from 'pg';
import { query } from '../utils/postgres';
import { AppError } from '../middleware/errorHandler';
import * as crypto from 'crypto';
import { canonicalizeForSignature } from '../services/transparency/canonicalizer';
import { transparencyWriter } from '../services/transparencyWriter';
import { validateISODate, validateInteger, validateEnum, validateBoolean } from '../utils/validation';
import { Histogram } from 'prom-client';
import { promRegister } from '../utils/prometheus';

// Constants
const CANONICAL_SIZE_CAP = 32 * 1024; // 32KB

// Prometheus histograms (register once)
let verifyLatencyMs: Histogram<string>;
let canonicalSizeBytes: Histogram<string>;
try {
  verifyLatencyMs = new Histogram({
    name: 'transparency_verify_latency_ms',
    help: 'Latency of transparency signature verification in milliseconds',
    buckets: [5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000],
    registers: [promRegister],
  });
} catch (_) {
  // metric may already be registered in tests/hot reload
  verifyLatencyMs = (promRegister.getSingleMetric('transparency_verify_latency_ms') as Histogram<string>) || (new Histogram({ name: 'noop_latency', help: 'noop', registers: [] }));
}
try {
  canonicalSizeBytes = new Histogram({
    name: 'transparency_canonical_size_bytes',
    help: 'Size of canonical strings produced during verification',
    buckets: [128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768],
    registers: [promRegister],
  });
} catch (_) {
  canonicalSizeBytes = (promRegister.getSingleMetric('transparency_canonical_size_bytes') as Histogram<string>) || (new Histogram({ name: 'noop_size', help: 'noop', registers: [] }));
}

// Allowed values for sort and order parameters
const ALLOWED_SORT_FIELDS = ['timestamp', 'winner_bid_ecpm', 'aletheia_fee_bp'];
const SORT_FIELD_MAP: Record<string, string> = {
  timestamp: 'observed_at',
  winner_bid_ecpm: 'winner_bid_ecpm',
  aletheia_fee_bp: 'aletheia_fee_bp',
};
const ALLOWED_ORDER_VALUES = ['asc', 'desc'];

// Helpers
const getPagination = (req: Request) => {
  const limit = validateInteger(req.query.limit as string, 'limit', 50, 1, 500);
  const page = validateInteger(req.query.page as string, 'page', 1, 1, Number.MAX_SAFE_INTEGER);
  const offset = (page - 1) * limit;
  return { limit, offset, page };
};

/**
 * Truncate canonical string if it exceeds size cap
 */
const truncateCanonical = (canonical: string): { canonical: string; truncated: boolean } => {
  if (canonical.length <= CANONICAL_SIZE_CAP) {
    return { canonical, truncated: false };
  }
  
  const truncated = canonical.substring(0, CANONICAL_SIZE_CAP);
  return { canonical: truncated, truncated: true };
};

const toIsoString = (value: unknown): string => {
  if (value instanceof Date) {
    return value.toISOString();
  }
  if (typeof value === 'string') {
    return new Date(value).toISOString();
  }
  if (value === null || value === undefined) {
    return new Date(0).toISOString();
  }
  return new Date(value as string).toISOString();
};

const ensureTransparencyEnabled = () => {
  if (process.env.TRANSPARENCY_API_ENABLED !== 'true') {
    throw new AppError('Transparency API is disabled', 503);
  }
};

const replicaQuery = <T extends QueryResultRow = QueryResultRow>(
  text: string,
  params?: ReadonlyArray<unknown>,
  label?: string
) => query<T>(text, params, { replica: true, label });

/** GET /auctions */
export const getAuctions = async (req: Request, res: Response, next: NextFunction) => {
  try {
    ensureTransparencyEnabled();

    const userPublisherId = req.user?.publisherId;
    if (!userPublisherId) {
      throw new AppError('Unauthorized', 401);
    }

    const requestedPublisher = (req.query.publisher_id as string) || userPublisherId;
    if (requestedPublisher !== userPublisherId) {
      throw new AppError('Forbidden: cannot access other publishers', 403);
    }

    const { limit, offset, page } = getPagination(req);

    const from = validateISODate(req.query.from as string, 'from');
    const to = validateISODate(req.query.to as string, 'to');

    // Cross-field validation: from must be <= to (when both provided)
    if (from && to && new Date(from).getTime() > new Date(to).getTime()) {
      throw new AppError('Invalid range: from must be less than or equal to to', 400);
    }

    // Enforce maximum window of 31 days for heavy queries (FIX-11: 656)
    if (from && to) {
      const startMs = new Date(from).getTime();
      const endMs = new Date(to).getTime();
      const maxWindowMs = 31 * 24 * 60 * 60 * 1000;
      if (endMs - startMs > maxWindowMs) {
        const cappedStartISO = new Date(endMs - maxWindowMs).toISOString();
        // Override from with capped start
        req.query.from = cappedStartISO;
      }
    }

    const placementId = (req.query.placement_id as string) || undefined;
    const surface = (req.query.surface as string) || undefined;
    const geo = (req.query.geo as string) || undefined;
    
    // Validate sort and order if provided
    const sort = validateEnum(req.query.sort as string, 'sort', ALLOWED_SORT_FIELDS);
    const order = validateEnum(req.query.order as string, 'order', ALLOWED_ORDER_VALUES) || 'desc';

    // Build WHERE clause and params
    const params: unknown[] = [requestedPublisher];
    const where: string[] = ['publisher_id = $1'];
    let paramIdx = 2;

    if (from) {
      where.push(`observed_at >= $${paramIdx}`);
      params.push(new Date(from));
      paramIdx += 1;
    }
    if (to) {
      where.push(`observed_at <= $${paramIdx}`);
      params.push(new Date(to));
      paramIdx += 1;
    }
    if (placementId) {
      where.push(`placement_id = $${paramIdx}`);
      params.push(placementId);
      paramIdx += 1;
    }
    if (surface) {
      where.push(`surface_type = $${paramIdx}`);
      params.push(surface);
      paramIdx += 1;
    }
    if (geo) {
      where.push(`device_geo = $${paramIdx}`);
      params.push(geo);
      paramIdx += 1;
    }

    const sortField = SORT_FIELD_MAP[sort || 'timestamp'] ?? 'observed_at';
    const sortOrder = order === 'asc' ? 'ASC' : 'DESC';

    const limitParam = paramIdx;
    const offsetParam = paramIdx + 1;
    params.push(limit, offset);

    const baseQuery = `
      SELECT 
        auction_id,
        observed_at,
        publisher_id,
        app_or_site_id,
        placement_id,
        surface_type,
        device_os,
        device_geo,
        att_status,
        tc_string_sha256,
        winner_source,
        winner_bid_ecpm,
        winner_gross_price,
        winner_currency,
        winner_reason,
        aletheia_fee_bp,
        sample_bps,
        effective_publisher_share,
        integrity_algo,
        integrity_key_id,
        integrity_signature
      FROM transparency_auctions
      WHERE ${where.join(' AND ')}
      ORDER BY ${sortField} ${sortOrder}
      LIMIT $${limitParam} OFFSET $${offsetParam}
    `;

    const { rows } = await replicaQuery(baseQuery, params, 'TRANSPARENCY_AUCTIONS_LIST');

    const auctionIds = rows.map((r) => r.auction_id);
    const candidatesByAuction: Record<string, any[]> = {};

    if (auctionIds.length > 0) {
      const candRows = await replicaQuery(
        `SELECT auction_id, source, bid_ecpm, currency, response_time_ms, status, metadata_hash
         FROM transparency_auction_candidates
         WHERE auction_id = ANY($1::uuid[])`,
        [auctionIds],
        'TRANSPARENCY_CANDIDATES_BY_AUCTIONS'
      );

      for (const c of candRows.rows) {
        if (!candidatesByAuction[c.auction_id]) {
          candidatesByAuction[c.auction_id] = [];
        }
        candidatesByAuction[c.auction_id].push({
          source: c.source,
          bid_ecpm: Number(c.bid_ecpm),
          currency: c.currency,
          response_time_ms: c.response_time_ms,
          status: c.status,
          metadata_hash: c.metadata_hash,
        });
      }
    }

    const data = rows.map((r) => ({
      auction_id: r.auction_id,
      timestamp: toIsoString(r.observed_at),
      publisher_id: r.publisher_id,
      app_or_site_id: r.app_or_site_id,
      placement_id: r.placement_id,
      surface_type: r.surface_type,
      device_context: {
        os: r.device_os,
        geo: r.device_geo,
        att: r.att_status,
        tc_string_sha256: r.tc_string_sha256,
      },
      candidates: candidatesByAuction[r.auction_id] || [],
      winner: {
        source: r.winner_source,
        bid_ecpm: Number(r.winner_bid_ecpm),
        gross_price: Number(r.winner_gross_price),
        currency: r.winner_currency,
        reason: r.winner_reason,
      },
      fees: {
        aletheia_fee_bp: r.aletheia_fee_bp,
        effective_publisher_share: Number(r.effective_publisher_share),
      },
      integrity: {
        signature: r.integrity_signature,
        algo: r.integrity_algo,
        key_id: r.integrity_key_id,
      },
    }));

    res.json({
      page,
      limit,
      count: data.length,
      data,
    });
  } catch (err) {
    next(err);
  }
};

/** GET /auctions/:auction_id */
export const getAuctionById = async (req: Request, res: Response, next: NextFunction) => {
  try {
    ensureTransparencyEnabled();

    const userPublisherId = req.user?.publisherId;
    if (!userPublisherId) {
      throw new AppError('Unauthorized', 401);
    }

    const auctionId = req.params.auction_id;
    
    // Validate includeCanonical flag
    const includeCanonical = validateBoolean(
      req.query.includeCanonical as string,
      'includeCanonical',
      false
    );

    const { rows } = await replicaQuery(
      `SELECT 
        auction_id,
        observed_at,
        publisher_id,
        app_or_site_id,
        placement_id,
        surface_type,
        device_os,
        device_geo,
        att_status,
        tc_string_sha256,
        winner_source,
        winner_bid_ecpm,
        winner_gross_price,
        winner_currency,
        winner_reason,
        aletheia_fee_bp,
        sample_bps,
        effective_publisher_share,
        integrity_algo,
        integrity_key_id,
        integrity_signature
      FROM transparency_auctions
      WHERE auction_id = $1
      LIMIT 1`,
      [auctionId],
      'TRANSPARENCY_AUCTION_BY_ID'
    );

    if (rows.length === 0) {
      throw new AppError('Not found', 404);
    }

    if (rows[0].publisher_id !== userPublisherId) {
      throw new AppError('Forbidden: cannot access other publishers', 403);
    }

    const candRows = await replicaQuery(
      `SELECT source, bid_ecpm, currency, response_time_ms, status, metadata_hash
       FROM transparency_auction_candidates
       WHERE auction_id = $1`,
      [auctionId],
      'TRANSPARENCY_CANDIDATES_BY_ID'
    );

    const r = rows[0];
    const payload: any = {
      auction_id: r.auction_id,
      timestamp: toIsoString(r.observed_at),
      publisher_id: r.publisher_id,
      app_or_site_id: r.app_or_site_id,
      placement_id: r.placement_id,
      surface_type: r.surface_type,
      device_context: {
        os: r.device_os,
        geo: r.device_geo,
        att: r.att_status,
        tc_string_sha256: r.tc_string_sha256,
      },
      candidates: candRows.rows.map((c) => ({
        source: c.source,
        bid_ecpm: Number(c.bid_ecpm),
        currency: c.currency,
        response_time_ms: c.response_time_ms,
        status: c.status,
        metadata_hash: c.metadata_hash,
      })),
      winner: {
        source: r.winner_source,
        bid_ecpm: Number(r.winner_bid_ecpm),
        gross_price: Number(r.winner_gross_price),
        currency: r.winner_currency,
        reason: r.winner_reason,
      },
      fees: {
        aletheia_fee_bp: r.aletheia_fee_bp,
        effective_publisher_share: Number(r.effective_publisher_share),
      },
      integrity: {
        signature: r.integrity_signature,
        algo: r.integrity_algo,
        key_id: r.integrity_key_id,
      },
      sample_bps: Number(r.sample_bps || 0),
    };

    // Include canonical string if requested
    if (includeCanonical) {
      // Build canonical payload for signing (same structure as writer)
      const canonicalPayload = {
        auction: {
          auction_id: r.auction_id,
          publisher_id: r.publisher_id,
          timestamp: toIsoString(r.observed_at),
          winner_source: r.winner_source,
          winner_bid_ecpm: Number(r.winner_bid_ecpm || 0),
          winner_currency: r.winner_currency,
          winner_reason: r.winner_reason,
          sample_bps: Number(r.sample_bps || 0),
        },
        candidates: candRows.rows.map((c: any) => ({
          source: c.source,
          bid_ecpm: Number(c.bid_ecpm || 0),
          status: c.status,
        })),
      };
      
      const canonical = canonicalizeForSignature(canonicalPayload);
      const { canonical: truncatedCanonical, truncated } = truncateCanonical(canonical);
      
      payload.canonical = {
        string: truncatedCanonical,
        truncated,
        size_bytes: canonical.length,
      };
    }

    res.json(payload);
  } catch (err) {
    next(err);
  }
};

/** GET /summary/auctions */
export const getAuctionSummary = async (req: Request, res: Response, next: NextFunction) => {
  try {
    ensureTransparencyEnabled();

    const userPublisherId = req.user?.publisherId;
    if (!userPublisherId) throw new AppError('Unauthorized', 401);

    const requestedPublisher = (req.query.publisher_id as string) || userPublisherId;
    if (requestedPublisher !== userPublisherId) {
      throw new AppError('Forbidden: cannot access other publishers', 403);
    }

    const from = validateISODate(req.query.from as string, 'from');
    const to = validateISODate(req.query.to as string, 'to');

    // Cross-field validation
    if (from && to && new Date(from).getTime() > new Date(to).getTime()) {
      throw new AppError('Invalid range: from must be less than or equal to to', 400);
    }

    const where: string[] = ['publisher_id = $1'];
    const params: unknown[] = [requestedPublisher];
    let paramIdx = 2;

    if (from) {
      where.push(`observed_at >= $${paramIdx}`);
      params.push(new Date(from));
      paramIdx += 1;
    }
    if (to) {
      where.push(`observed_at <= $${paramIdx}`);
      params.push(new Date(to));
      paramIdx += 1;
    }

    const whereSql = where.join(' AND ');

    const totalRows = await replicaQuery(
      `SELECT COUNT(*)::bigint as total_sampled
       FROM transparency_auctions
       WHERE ${whereSql} AND length(integrity_signature) > 0`,
      [...params],
      'TRANSPARENCY_SUMMARY_TOTAL'
    );

    const winnersRows = await replicaQuery(
      `SELECT winner_source as source, COUNT(*)::bigint as count
       FROM transparency_auctions
       WHERE ${whereSql}
       GROUP BY winner_source
       ORDER BY count DESC
       LIMIT 20`,
      [...params],
      'TRANSPARENCY_SUMMARY_WINNERS'
    );

    const avgRows = await replicaQuery(
      `SELECT avg(aletheia_fee_bp) as avg_fee_bp, avg(effective_publisher_share) as publisher_share_avg
       FROM transparency_auctions
       WHERE ${whereSql}`,
      [...params],
      'TRANSPARENCY_SUMMARY_AVG'
    );

    const summary = {
      total_sampled: Number(totalRows.rows?.[0]?.total_sampled ?? 0),
      winners_by_source: winnersRows.rows.map((r: any) => ({ source: r.source, count: Number(r.count) })),
      avg_fee_bp: Number(avgRows.rows?.[0]?.avg_fee_bp ?? 0),
      publisher_share_avg: Number(avgRows.rows?.[0]?.publisher_share_avg ?? 0),
    };

    res.json(summary);
  } catch (err) {
    next(err);
  }
};


// --- Transparency verification helpers & handlers ---

/** GET /metrics — transparency writer counters (attempted/succeeded/failed, sampled/unsampled) */
export const getTransparencyMetrics = async (_req: Request, res: Response, next: NextFunction) => {
  try {
    ensureTransparencyEnabled();
    const m = transparencyWriter.getMetrics();
    res.json({ success: true, data: m });
  } catch (err) {
    next(err);
  }
};

const ensurePublisherScope = (req: Request, publisherId: string) => {
  const userPublisherId = req.user?.publisherId;
  if (!userPublisherId) {
    throw new AppError('Unauthorized', 401);
  }
  if (publisherId !== userPublisherId) {
    throw new AppError('Forbidden: cannot access other publishers', 403);
  }
};

const parsePublicKey = (source: string) => {
  try {
    const trimmed = source.trim();
    if (trimmed.includes('BEGIN')) {
      return crypto.createPublicKey(trimmed);
    }
    // Try SPKI DER base64
    const der = Buffer.from(trimmed, 'base64');
    try {
      return crypto.createPublicKey({ key: der, format: 'der', type: 'spki' });
    } catch (_) {
      // Last resort: treat as PEM after wrapping (best-effort, may fail)
      const pem = `-----BEGIN PUBLIC KEY-----\n${trimmed}\n-----END PUBLIC KEY-----\n`;
      return crypto.createPublicKey(pem);
    }
  } catch (err) {
    return null;
  }
};

const toPublicKeyPem = (source: unknown): string | null => {
  if (typeof source !== 'string' || source.trim().length === 0) {
    return null;
  }

  const key = parsePublicKey(source);
  if (!key) {
    return null;
  }

  try {
    return key.export({ format: 'pem', type: 'spki' }).toString();
  } catch (_err) {
    return null;
  }
};

/** GET /keys — return active transparency signer public keys */
export const getTransparencyKeys = async (req: Request, res: Response, next: NextFunction) => {
  try {
    ensureTransparencyEnabled();
    const userPublisherId = req.user?.publisherId;
    if (!userPublisherId) {
      throw new AppError('Unauthorized', 401);
    }

    const { rows } = await replicaQuery(
      `SELECT key_id, algo, public_key_base64, active
       FROM transparency_signer_keys
       WHERE active = true
       ORDER BY created_at DESC
       LIMIT 10`,
      undefined,
      'TRANSPARENCY_KEYS'
    );

    const fallbackKey = process.env.TRANSPARENCY_PUBLIC_KEY_BASE64;
    const keyId = process.env.TRANSPARENCY_KEY_ID || 'env-default';

    const data = rows.length > 0
      ? rows
      : (fallbackKey
          ? [{ key_id: keyId, algo: 'ed25519', public_key_base64: fallbackKey, active: 1 }]
          : []);

    const enriched = data.map((k: any) => ({
      ...k,
      public_key_pem: toPublicKeyPem(k.public_key_base64) || undefined,
    }));

    // Generate a stable ETag based on the concatenation of key_id+public_key_base64
    const etagSource = data.map((k: any) => `${k.key_id}:${k.public_key_base64}`).join('|');
    const etag = 'W/"' + crypto.createHash('sha256').update(etagSource).digest('hex').slice(0, 32) + '"';

    // Check If-None-Match to support 304s
    const inm = req.headers['if-none-match'];
    if (inm && inm === etag) {
      res.status(304).end();
      return;
    }

    // Set caching headers: cache for 5 minutes
    res.setHeader('ETag', etag);
    res.setHeader('Cache-Control', 'public, max-age=300, must-revalidate');

    res.json({ count: enriched.length, data: enriched });
  } catch (err) {
    next(err);
  }
};

/** GET /keys (public) — return active transparency signer public keys (no auth) */
export const getTransparencyKeysPublic = async (_req: Request, res: Response, next: NextFunction) => {
  try {
    const { rows } = await replicaQuery(
      `SELECT key_id, algo, public_key_base64, active
       FROM transparency_signer_keys
       WHERE active = true
       ORDER BY created_at DESC
       LIMIT 10`,
      undefined,
      'TRANSPARENCY_KEYS_PUBLIC'
    );

    const fallbackKey = process.env.TRANSPARENCY_PUBLIC_KEY_BASE64;
    const keyId = process.env.TRANSPARENCY_KEY_ID || 'env-default';

    const data = rows.length > 0
      ? rows
      : (fallbackKey
          ? [{ key_id: keyId, algo: 'ed25519', public_key_base64: fallbackKey, active: 1 }]
          : []);

    const enriched = data.map((k: any) => ({
      ...k,
      public_key_pem: toPublicKeyPem(k.public_key_base64) || undefined,
    }));

    const etagSource = data.map((k: any) => `${k.key_id}:${k.public_key_base64}`).join('|');
    const etag = 'W/"' + crypto.createHash('sha256').update(etagSource).digest('hex').slice(0, 32) + '"';

    const inm = _req.headers['if-none-match'];
    if (inm && inm === etag) {
      res.status(304).end();
      return;
    }

    res.setHeader('ETag', etag);
    res.setHeader('Cache-Control', 'public, max-age=300, must-revalidate');

    res.json({ count: enriched.length, data: enriched });
  } catch (err) {
    next(err);
  }
};

/** GET /auctions/:auction_id/verify — verify integrity signature */
export const verifyAuction = async (req: Request, res: Response, next: NextFunction) => {
  try {
    ensureTransparencyEnabled();

    const auctionId = req.params.auction_id;
    const tStart = process.hrtime.bigint();

    // Load auction row
    const auctionRows = await replicaQuery(
      `SELECT 
         auction_id,
         observed_at,
         publisher_id,
         winner_source,
         winner_bid_ecpm,
         winner_currency,
         winner_reason,
         aletheia_fee_bp,
         sample_bps,
         integrity_algo,
         integrity_key_id,
         integrity_signature
       FROM transparency_auctions
       WHERE auction_id = $1
       LIMIT 1`,
      [auctionId],
      'TRANSPARENCY_VERIFY_AUCTION'
    );

    if (auctionRows.rows.length === 0) {
      throw new AppError('Not found', 404);
    }

    const auction = auctionRows.rows[0];

    // Publisher scoping
    ensurePublisherScope(req, auction.publisher_id);

    // Load candidates
    const cRows = await replicaQuery(
      `SELECT source, bid_ecpm, status
       FROM transparency_auction_candidates
       WHERE auction_id = $1`,
      [auctionId],
      'TRANSPARENCY_VERIFY_CANDIDATES'
    );

    // Build canonical payload (must mirror writer)
    const payload = {
      auction: {
        auction_id: auction.auction_id,
        publisher_id: auction.publisher_id,
        timestamp: toIsoString(auction.observed_at),
        winner_source: auction.winner_source,
        winner_bid_ecpm: Number(auction.winner_bid_ecpm || 0),
        winner_currency: auction.winner_currency,
        winner_reason: auction.winner_reason,
        sample_bps: Number(auction.sample_bps || 0),
      },
      candidates: cRows.rows.map((c: any) => ({
        source: c.source,
        bid_ecpm: Number(c.bid_ecpm || 0),
        status: c.status,
      })),
    };

    const canonical = canonicalizeForSignature(payload);
    const signatureB64 = auction.integrity_signature as string | null;
    const algo = (auction.integrity_algo as string | null) || 'ed25519';
    const keyId = auction.integrity_key_id as string | null;

    if (!signatureB64 || !keyId || algo.toLowerCase() !== 'ed25519') {
      return res.json({
        status: 'not_applicable',
        reason: 'missing_signature_or_unsupported_algo',
        key_id: keyId,
        algo,
      });
    }

    // Fetch matching public key (active)
    const kRows = await replicaQuery(
      `SELECT key_id, algo, public_key_base64, active
       FROM transparency_signer_keys
       WHERE key_id = $1 AND active = true
       LIMIT 1`,
      [keyId],
      'TRANSPARENCY_VERIFY_KEYS'
    );

    const fallbackKey = process.env.TRANSPARENCY_PUBLIC_KEY_BASE64;
    const keyBase64 = (kRows.rows[0]?.public_key_base64 as string | undefined) || fallbackKey;

    if (!keyBase64) {
      return res.json({
        status: 'unknown_key',
        reason: 'public_key_not_found',
        key_id: keyId,
        algo,
      });
    }

    const pubKey = parsePublicKey(keyBase64);
    if (!pubKey) {
      return res.json({
        status: 'fail',
        reason: 'public_key_parse_failed',
        key_id: keyId,
        algo,
      });
    }

    let verified = false;
    try {
      verified = crypto.verify(
        null,
        Buffer.from(canonical, 'utf8'),
        pubKey,
        Buffer.from(signatureB64, 'base64')
      );
    } catch (e) {
      return res.json({ status: 'fail', reason: 'verification_threw', key_id: keyId, algo });
    }

    // Check if canonical should be included and potentially truncated
    const { canonical: returnCanonical, truncated } = truncateCanonical(canonical);

    // Observe canonical size
    try { canonicalSizeBytes.observe(canonical.length); } catch (_) {}

    const tEnd = process.hrtime.bigint();
    const latencyMs = Number(tEnd - tStart) / 1_000_000;
    try { verifyLatencyMs.observe(latencyMs); } catch (_) {}
    
    return res.json({
      status: verified ? 'pass' : 'fail',
      key_id: keyId,
      algo,
      canonical: returnCanonical,
      canonical_truncated: truncated,
      canonical_size_bytes: canonical.length,
      sample_bps: Number(auction.sample_bps || 0),
    });
  } catch (err) {
    next(err);
  }
};
