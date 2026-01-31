import { getConfig } from '../config';
import { logger } from '../logger';
import { S3Client, ListObjectsV2Command } from '@aws-sdk/client-s3';

export interface S3StorageTelemetry {
  bucket: string;
  total_bytes: number;
  object_count?: number;
  updated_at?: string;
}

export async function getS3StorageTelemetry(): Promise<S3StorageTelemetry | null> {
  const cfg = getConfig();
  const endpoint = cfg.S3_STORAGE_ENDPOINT;

  if (!endpoint) {
    const bucket = cfg.S3_STORAGE_BUCKET;
    if (!bucket || !cfg.AWS_ACCESS_KEY_ID || !cfg.AWS_SECRET_ACCESS_KEY) {
      logger.warn('S3 storage endpoint not configured');
      return null;
    }

    return getS3StorageFromSdk(bucket, cfg.S3_ENDPOINT, cfg.AWS_DEFAULT_REGION);
  }

  try {
    const response = await fetch(endpoint);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }

    const data = (await response.json()) as { bucket?: string; total_bytes?: number; object_count?: number; updated_at?: string };
    return {
      bucket: data.bucket || 'unknown',
      total_bytes: Number(data.total_bytes || 0),
      object_count: data.object_count !== undefined ? Number(data.object_count) : undefined,
      updated_at: data.updated_at || new Date().toISOString(),
    };
  } catch (err) {
    logger.error('Failed to fetch S3 storage telemetry', {
      endpoint,
      error: (err as Error).message,
    });
    return null;
  }
}

async function getS3StorageFromSdk(bucket: string, endpoint?: string, region?: string): Promise<S3StorageTelemetry | null> {
  try {
    const client = new S3Client({
      region: region || 'us-east-1',
      endpoint: endpoint || undefined,
      forcePathStyle: !!endpoint,
      credentials: {
        accessKeyId: process.env.AWS_ACCESS_KEY_ID || '',
        secretAccessKey: process.env.AWS_SECRET_ACCESS_KEY || '',
      },
    });

    let continuationToken: string | undefined;
    let totalBytes = 0;
    let objectCount = 0;
    let pages = 0;
    const maxPages = 200; // safety cap

    do {
      const result = await client.send(new ListObjectsV2Command({
        Bucket: bucket.replace('s3://', ''),
        ContinuationToken: continuationToken,
      }));
      (result.Contents || []).forEach((obj) => {
        if (obj.Size) totalBytes += obj.Size;
        objectCount += 1;
      });
      continuationToken = result.IsTruncated ? result.NextContinuationToken : undefined;
      pages += 1;
    } while (continuationToken && pages < maxPages);

    return {
      bucket,
      total_bytes: totalBytes,
      object_count: objectCount,
      updated_at: new Date().toISOString(),
    };
  } catch (err) {
    logger.error('Failed to fetch S3 storage via SDK', { error: (err as Error).message });
    return null;
  }
}
