import type { Pool } from 'pg';
import * as jwt from 'jsonwebtoken';
import { aesGcmEncrypt, aesGcmDecrypt, AesGcmCiphertext } from '../utils/crypto';
import logger from '../utils/logger';

export interface NetworkCredential {
  id: string;
  publisherId: string;
  network: string;
  credentials: Record<string, any>;
  createdAt: Date;
  updatedAt: Date;
  version: number;
}

export interface NetworkCredentialInput {
  publisherId: string;
  network: string;
  credentials: Record<string, any>;
}

export interface ShortLivedToken {
  token: string;
  expiresAt: Date;
  network: string;
  publisherId: string;
}

/**
 * Network Credential Vault Service
 * 
 * Implements BYO model credential separation:
 * - Stores long-lived network credentials server-side only
 * - Encrypts credentials with AES-256-GCM
 * - Generates short-lived JWT tokens (5-15 min TTL) when needed
 * - Never exposes credentials to SDKs/adapters
 * - Provides audit trail for all credential access
 */
export class NetworkCredentialVaultService {
  constructor(private pool: Pool) {}

  /**
   * Store encrypted network credentials
   */
  async storeCredentials(input: NetworkCredentialInput): Promise<string> {
    const client = await this.pool.connect();
    try {
      await client.query('BEGIN');

      // Encrypt credentials
      const encrypted = aesGcmEncrypt(JSON.stringify(input.credentials));
      const credentialsCiphertext = JSON.stringify(encrypted);

      // Check for existing credential
      const existing = await client.query(
        `SELECT id, version FROM encrypted_network_credentials 
         WHERE publisher_id = $1 AND network = $2 AND deleted_at IS NULL`,
        [input.publisherId, input.network]
      );

      let credentialId: string;
      let version: number;

      if (existing.rows.length > 0) {
        // Update existing
        version = existing.rows[0].version + 1;
        credentialId = existing.rows[0].id;

        await client.query(
          `UPDATE encrypted_network_credentials 
           SET credentials_ciphertext = $1, version = $2, updated_at = NOW()
           WHERE id = $3`,
          [credentialsCiphertext, version, credentialId]
        );

        logger.info('Network credentials updated', {
          publisherId: input.publisherId,
          network: input.network,
          version,
        });
      } else {
        // Insert new
        const result = await client.query(
          `INSERT INTO encrypted_network_credentials 
           (id, publisher_id, network, credentials_ciphertext, version, created_at, updated_at)
           VALUES (gen_random_uuid(), $1, $2, $3, 1, NOW(), NOW())
           RETURNING id`,
          [input.publisherId, input.network, credentialsCiphertext]
        );
        credentialId = result.rows[0].id;
        version = 1;

        logger.info('Network credentials stored', {
          publisherId: input.publisherId,
          network: input.network,
          credentialId,
        });
      }

      // Audit trail
      await client.query(
        `INSERT INTO credential_audit_log 
         (id, credential_id, action, actor_type, actor_id, created_at)
         VALUES (gen_random_uuid(), $1, $2, 'system', $3, NOW())`,
        [credentialId, existing.rows.length > 0 ? 'UPDATE' : 'CREATE', input.publisherId]
      );

      await client.query('COMMIT');
      return credentialId;
    } catch (error) {
      await client.query('ROLLBACK');
      logger.error('Failed to store network credentials', { error, input });
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Retrieve decrypted network credentials (server-side only)
   */
  async getCredentials(
    publisherId: string,
    network: string
  ): Promise<NetworkCredential | null> {
    const client = await this.pool.connect();
    try {
      const result = await client.query(
        `SELECT id, publisher_id, network, credentials_ciphertext, version, created_at, updated_at
         FROM encrypted_network_credentials
         WHERE publisher_id = $1 AND network = $2 AND deleted_at IS NULL`,
        [publisherId, network]
      );

      if (result.rows.length === 0) {
        return null;
      }

      const row = result.rows[0];
      const encrypted: AesGcmCiphertext = JSON.parse(row.credentials_ciphertext);
      const decrypted = aesGcmDecrypt(encrypted);
      const credentials = JSON.parse(decrypted);

      // Audit access
      await client.query(
        `INSERT INTO credential_audit_log 
         (id, credential_id, action, actor_type, actor_id, created_at)
         VALUES (gen_random_uuid(), $1, 'ACCESS', 'system', $2, NOW())`,
        [row.id, publisherId]
      );

      return {
        id: row.id,
        publisherId: row.publisher_id,
        network: row.network,
        credentials,
        createdAt: row.created_at,
        updatedAt: row.updated_at,
        version: row.version,
      };
    } finally {
      client.release();
    }
  }

  /**
   * Generate short-lived token for adapter use
   * Token contains only placement IDs, never credentials
   */
  async generateShortLivedToken(
    publisherId: string,
    network: string,
    ttlMinutes: number = 15
  ): Promise<ShortLivedToken | null> {
    const credential = await this.getCredentials(publisherId, network);
    if (!credential) {
      return null;
    }

    // SECURITY: Require JWT_SECRET in production (P1 Fix #1)
    const secret = process.env.JWT_SECRET;
    if (!secret) {
      if (process.env.NODE_ENV === 'production') {
        throw new Error('JWT_SECRET must be set in production');
      }
      logger.warn('JWT_SECRET not set - using insecure default (non-production only)');
    }
    const signingSecret = secret || 'dev-secret-not-for-production';
    
    const expiresAt = new Date(Date.now() + ttlMinutes * 60 * 1000);
    
    const payload = {
      publisherId,
      network,
      credentialId: credential.id,
      type: 'network_adapter',
      exp: Math.floor(expiresAt.getTime() / 1000),
    };

    const token = jwt.sign(payload, signingSecret);

    // Log token generation
    logger.info('Short-lived token generated', {
      publisherId,
      network,
      ttlMinutes,
      expiresAt,
    });

    return {
      token,
      expiresAt,
      network,
      publisherId,
    };
  }

  /**
   * Rotate credentials (creates new version)
   */
  async rotateCredentials(
    publisherId: string,
    network: string,
    newCredentials: Record<string, any>
  ): Promise<void> {
    await this.storeCredentials({
      publisherId,
      network,
      credentials: newCredentials,
    });
  }

  /**
   * Soft delete credentials
   */
  async deleteCredentials(publisherId: string, network: string): Promise<void> {
    const client = await this.pool.connect();
    try {
      const result = await client.query(
        `UPDATE encrypted_network_credentials 
         SET deleted_at = NOW()
         WHERE publisher_id = $1 AND network = $2 AND deleted_at IS NULL
         RETURNING id`,
        [publisherId, network]
      );

      if (result.rows.length > 0) {
        // Audit deletion
        await client.query(
          `INSERT INTO credential_audit_log 
           (id, credential_id, action, actor_type, actor_id, created_at)
           VALUES (gen_random_uuid(), $1, 'DELETE', 'system', $2, NOW())`,
          [result.rows[0].id, publisherId]
        );

        logger.info('Network credentials deleted', { publisherId, network });
      }
    } finally {
      client.release();
    }
  }

  /**
   * List all networks with stored credentials for a publisher
   */
  async listNetworks(publisherId: string): Promise<string[]> {
    const client = await this.pool.connect();
    try {
      const result = await client.query(
        `SELECT DISTINCT network FROM encrypted_network_credentials
         WHERE publisher_id = $1 AND deleted_at IS NULL
         ORDER BY network`,
        [publisherId]
      );
      return result.rows.map((r: any) => r.network);
    } finally {
      client.release();
    }
  }

  async listCredentialSummaries(
    publisherId: string
  ): Promise<Array<{ id: string; network: string; version: number; createdAt: Date; updatedAt: Date }>> {
    const client = await this.pool.connect();
    try {
      const result = await client.query(
        `SELECT id, network, version, created_at, updated_at
         FROM encrypted_network_credentials
         WHERE publisher_id = $1 AND deleted_at IS NULL
         ORDER BY network`,
        [publisherId]
      );
      return result.rows.map((row) => ({
        id: row.id,
        network: row.network,
        version: row.version,
        createdAt: row.created_at,
        updatedAt: row.updated_at,
      }));
    } finally {
      client.release();
    }
  }

  /**
   * Get credential metadata (without decrypting)
   */
  async getCredentialMetadata(publisherId: string, network: string) {
    const client = await this.pool.connect();
    try {
      const result = await client.query(
        `SELECT id, publisher_id, network, version, created_at, updated_at
         FROM encrypted_network_credentials
         WHERE publisher_id = $1 AND network = $2 AND deleted_at IS NULL`,
        [publisherId, network]
      );

      if (result.rows.length === 0) {
        return null;
      }

      return result.rows[0];
    } finally {
      client.release();
    }
  }
}
