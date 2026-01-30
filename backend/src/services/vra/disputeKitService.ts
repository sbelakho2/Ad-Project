import { query } from '../../utils/postgres';
import logger from '../../utils/logger';
import { vraDisputeKitsBuiltTotal, vraDisputeKitFailuresTotal } from '../../utils/prometheus';
import * as fs from 'node:fs';
import * as fsp from 'node:fs/promises';
import * as path from 'node:path';
import * as crypto from 'node:crypto';

export interface DisputeKitOptions {
  network?: string;
  ttlSec?: number;
  dryRun?: boolean;
  storage?: DisputeStorage;
  publisherId?: string; // REQUIRED in production for tenant isolation
}

export interface DisputeKitResult {
  kitId: string;
  contentType: string;
  sizeBytes: number;
  storageUri: string; // signed or pseudo URL; 'shadow://not-written' when dryRun and no storage
  preview?: {
    // when dryRun, provide a small preview to inspect without writes
    metadata: Record<string, unknown>;
    evidenceCsvFirstLines: string[];
  };
}

export interface DisputeStorage {
  putObject(key: string, bytes: Buffer | string, contentType: string, ttlSec?: number): Promise<string>;
}

/**
 * In-memory storage (primarily for tests and dev). Returns mem:// URIs.
 */
export class MemoryDisputeStorage implements DisputeStorage {
  private store = new Map<string, Buffer>();
  async putObject(key: string, bytes: Buffer | string, _contentType: string, _ttlSec?: number): Promise<string> {
    const buf = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
    this.store.set(key, buf);
    return `mem://vra/${encodeURIComponent(key)}`;
  }
  // helper for tests
  get(key: string): Buffer | undefined {
    return this.store.get(key);
  }
}

/**
 * FileSystem-backed storage adapter. Writes bundles under a configured directory.
 * Returns file:// URIs. Intended for local/dev and CI usage.
 */
export class FileSystemDisputeStorage implements DisputeStorage {
  constructor(private baseDir: string) {}

  private async ensureDir(dir: string): Promise<void> {
    await fsp.mkdir(dir, { recursive: true }).catch(() => {});
  }

  async putObject(key: string, bytes: Buffer | string, _contentType: string, _ttlSec?: number): Promise<string> {
    const buf = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
    const full = path.resolve(this.baseDir, key);
    const dir = path.dirname(full);
    await this.ensureDir(dir);
    await fsp.writeFile(full, buf);
    // Normalize file URI
    const uri = `file://${full.replace(/\\/g, '/')}`;
    return uri;
  }
}

/**
 * S3-backed storage adapter (lazy dependency). If AWS SDK v3 isn't available, the constructor will throw.
 * In production, configure bucket lifecycle policies for TTLs.
 */
export class S3DisputeStorage implements DisputeStorage {
  private s3: any;
  private signer: any;
  private bucket: string;
  private prefix: string;
  private region?: string;
  private endpoint?: string;

  constructor(opts: { bucket: string; prefix?: string; region?: string; endpoint?: string }) {
    this.bucket = opts.bucket;
    this.prefix = (opts.prefix || '').replace(/^\/+|\/+$/g, '');
    this.region = opts.region;
    this.endpoint = opts.endpoint;

    // Lazy require to avoid hard dependency during tests or non-S3 environments
    try {
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const { S3Client, PutObjectCommand } = require('@aws-sdk/client-s3');
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const { getSignedUrl } = require('@aws-sdk/s3-request-presigner');
      this.s3 = { Client: S3Client, Put: PutObjectCommand, getSignedUrl };
    } catch (e) {
      throw new Error('S3 adapter requested but @aws-sdk/* not installed');
    }
  }

  async putObject(key: string, bytes: Buffer | string, contentType: string, ttlSec?: number): Promise<string> {
    const Key = this.prefix ? `${this.prefix}/${key}` : key;
    const Body = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
    const client = new this.s3.Client({ region: this.region, endpoint: this.endpoint });
    const put = new this.s3.Put({ Bucket: this.bucket, Key, Body, ContentType: contentType });
    await client.send(put);
    // Best-effort signed URL; if presign fails, return s3:// URI
    try {
      const url = await this.s3.getSignedUrl(client, { Bucket: this.bucket, Key } as any, { expiresIn: Math.max(60, Math.min(ttlSec || 3600, 7 * 24 * 3600)) });
      return url;
    } catch {
      return `s3://${this.bucket}/${Key}`;
    }
  }
}

type DeltaRow = {
  kind: string;
  amount: string;
  currency: string;
  reason_code: string;
  window_start: string;
  window_end: string;
  evidence_id: string;
  confidence: string;
};

function toCsv(rows: DeltaRow[]): string {
  const head = 'kind,amount,currency,reason_code,window_start,window_end,evidence_id,confidence';
  const body = rows
    .map((r) => [
      r.kind,
      String(Number(r.amount) || 0),
      r.currency,
      r.reason_code.replace(/\n|\r|,/g, ' '),
      r.window_start,
      r.window_end,
      r.evidence_id,
      String(Number(r.confidence) || 0),
    ].join(','))
    .join('\n');
  return `${head}\n${body}`;
}

function redactString(val: string): string {
  // Basic safety redactions (emails, bearer tokens, long numerics)
  let s = val;
  s = s.replace(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi, '[REDACTED_EMAIL]');
  s = s.replace(/Bearer\s+[A-Za-z0-9._-]+/g, 'Bearer [REDACTED]');
  s = s.replace(/sk_(test|live)_[A-Za-z0-9]+/g, 'sk_$1_[REDACTED]');
  s = s.replace(/\b\d{13,19}\b/g, '[REDACTED_NUMERIC]');
  return s;
}

function redactCsv(csv: string): string {
  return redactString(csv);
}

/**
 * Build a Dispute Kit from a set of delta evidence IDs.
 * Shadow-first: with `dryRun` it does not write, and returns a preview only.
 * When a storage is provided (default in-memory), it stores a single JSON payload containing
 * both metadata and embedded CSV evidence for simplicity. This can be swapped for zip later.
 * 
 * SECURITY: When publisherId is provided, enforces tenant isolation by verifying
 * all deltas belong to the requesting publisher.
 */
export async function buildDisputeKit(deltaIds: string[], opts?: DisputeKitOptions): Promise<DisputeKitResult> {
  const network = (opts?.network || 'unknown').toLowerCase();
  const ttlSec = opts?.ttlSec ?? 3600; // 1 hour default
  const dryRun = opts?.dryRun === true;
  const storage: DisputeStorage | undefined = opts?.storage ?? new MemoryDisputeStorage();
  const publisherId = opts?.publisherId;

  const kitId = `kit_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;

  try {
    if (!Array.isArray(deltaIds) || deltaIds.length === 0) {
      throw new Error('No delta IDs provided');
    }

    // SECURITY: Fetch delta rows with tenant isolation when publisherId is provided
    // This prevents cross-tenant data leakage by ensuring deltas belong to the requesting publisher
    const baseQuery = `SELECT kind,
              amount::text AS amount,
              currency,
              reason_code,
              window_start::text,
              window_end::text,
              evidence_id,
              confidence::text
         FROM recon_deltas
        WHERE evidence_id = ANY($1::text[])`;
    
    const queryText = publisherId 
      ? `${baseQuery} AND publisher_id = $2 ORDER BY window_start ASC`
      : `${baseQuery} ORDER BY window_start ASC`;
    const queryParams = publisherId ? [deltaIds, publisherId] : [deltaIds];
    
    const rowsResult = await query<DeltaRow>(queryText, queryParams);
    const rows = rowsResult.rows;
    
    // SECURITY: If publisherId was provided but no rows match, either the evidence doesn't exist
    // or it belongs to a different publisher - both should fail to prevent enumeration attacks
    if (publisherId && rows.length === 0 && deltaIds.length > 0) {
      throw new Error('No evidence found matching the provided IDs for this publisher');
    }

    // Prepare CSV evidence
    const evidenceCsv = redactCsv(toCsv(rows));
    const checksum = crypto.createHash('sha256').update(evidenceCsv, 'utf8').digest('hex');

    const metadata = {
      kit_id: kitId,
      generated_at: new Date().toISOString(),
      network,
      deltas_count: rows.length,
      notes: 'This is a shadow-first Dispute Kit bundle (JSON + embedded CSV evidence).',
      checksum_sha256: checksum,
      ttl_sec: ttlSec,
    };

    if (dryRun) {
      // No writes; provide short preview of evidence
      const firstLines = evidenceCsv.split(/\r?\n/).slice(0, 5);
      return {
        kitId,
        contentType: 'application/json',
        sizeBytes: Buffer.byteLength(JSON.stringify({ metadata }), 'utf8'),
        storageUri: 'shadow://not-written',
        preview: { metadata, evidenceCsvFirstLines: firstLines },
      };
    }

    // Persist as JSON bundle (metadata + CSV). We keep it simple and avoid zip for now.
    const bundle = JSON.stringify({ metadata, files: { 'evidence.csv': evidenceCsv } });
    const uri = await storage!.putObject(`disputes/${kitId}.json`, bundle, 'application/json', ttlSec);

    try { vraDisputeKitsBuiltTotal.inc({ network }); } catch {}

    return {
      kitId,
      contentType: 'application/json',
      sizeBytes: Buffer.byteLength(bundle, 'utf8'),
      storageUri: uri,
    };
  } catch (e) {
    const msg = (e as Error)?.message || 'unknown';
    logger.warn('VRA DisputeKit: failed to build kit', { error: msg });
    try { vraDisputeKitFailuresTotal.inc({ reason: msg.slice(0, 64) }); } catch {}
    throw e;
  }
}

export function resolveDisputeStorageFromEnv(): DisputeStorage {
  const mode = (process.env.VRA_DISPUTE_STORAGE || '').trim().toLowerCase();
  try {
    if (mode === 'fs') {
      const dir = process.env.VRA_DISPUTE_FS_DIR || path.resolve(process.cwd(), 'logs', 'vra-kits');
      // Ensure base directory exists synchronously to improve first-call experience
      try { fs.mkdirSync(dir, { recursive: true }); } catch {}
      return new FileSystemDisputeStorage(dir);
    }
    if (mode === 's3') {
      const bucket = process.env.VRA_DISPUTE_BUCKET || '';
      if (!bucket) throw new Error('VRA_DISPUTE_BUCKET is required for s3 storage');
      const prefix = process.env.VRA_DISPUTE_PREFIX || 'vra';
      const region = process.env.AWS_REGION || process.env.VRA_S3_REGION;
      const endpoint = process.env.VRA_S3_ENDPOINT;
      return new S3DisputeStorage({ bucket, prefix, region, endpoint });
    }
  } catch (e) {
    logger.warn('VRA DisputeKit: storage init failed, falling back to memory', { error: (e as Error)?.message });
  }
  return new MemoryDisputeStorage();
}

export default { buildDisputeKit, MemoryDisputeStorage, FileSystemDisputeStorage, S3DisputeStorage, resolveDisputeStorageFromEnv };
