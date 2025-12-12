/**
 * VRA Network Connector Service
 * 
 * Manages network connections, credentials, and sync operations.
 * Supports: AdMob, Unity, AppLovin, IronSource, Meta Audience Network, Vungle
 */

import { Pool } from 'pg';
import crypto from 'crypto';
import logger from '../../utils/logger';

// Supported ad networks
export type NetworkType = 'admob' | 'unity' | 'applovin' | 'ironsource' | 'meta' | 'vungle';

export const SUPPORTED_NETWORKS: Record<NetworkType, NetworkConfig> = {
  admob: {
    name: 'Google AdMob',
    authType: 'oauth',
    oauthConfig: {
      authUrl: 'https://accounts.google.com/o/oauth2/v2/auth',
      tokenUrl: 'https://oauth2.googleapis.com/token',
      scopes: ['https://www.googleapis.com/auth/admob.report'],
    },
    requiredFields: ['accountId'],
    docsUrl: 'https://developers.google.com/admob/api/v1/getting-started',
  },
  unity: {
    name: 'Unity Ads',
    authType: 'api_key',
    requiredFields: ['apiKey', 'organizationId'],
    docsUrl: 'https://docs.unity.com/ads/en-us/manual/MonetizationResourcesAPI',
  },
  applovin: {
    name: 'AppLovin MAX',
    authType: 'api_key',
    requiredFields: ['reportKey', 'sdkKey'],
    docsUrl: 'https://dash.applovin.com/documentation/mediation/reporting-api',
  },
  ironsource: {
    name: 'ironSource',
    authType: 'api_key',
    requiredFields: ['secretKey', 'refreshToken'],
    docsUrl: 'https://developers.is.com/ironsource-mobile/general/reporting-api/',
  },
  meta: {
    name: 'Meta Audience Network',
    authType: 'oauth',
    oauthConfig: {
      authUrl: 'https://www.facebook.com/v18.0/dialog/oauth',
      tokenUrl: 'https://graph.facebook.com/v18.0/oauth/access_token',
      scopes: ['ads_read'],
    },
    requiredFields: ['appId'],
    docsUrl: 'https://developers.facebook.com/docs/audience-network/guides/reporting',
  },
  vungle: {
    name: 'Vungle',
    authType: 'api_key',
    requiredFields: ['apiKey', 'accountId'],
    docsUrl: 'https://support.vungle.com/hc/en-us/articles/360002925151-Reporting-API',
  },
};

interface NetworkConfig {
  name: string;
  authType: 'oauth' | 'api_key';
  oauthConfig?: {
    authUrl: string;
    tokenUrl: string;
    scopes: string[];
  };
  requiredFields: string[];
  docsUrl: string;
}

export interface NetworkConnection {
  id: string;
  publisherId: string;
  networkType: NetworkType;
  displayName: string | null;
  status: 'pending' | 'active' | 'error' | 'disabled';
  statusMessage: string | null;
  lastSyncAt: Date | null;
  networkAccountId: string | null;
  networkAppIds: string[];
  syncEnabled: boolean;
  syncFrequencyHours: number;
  syncLookbackDays: number;
  createdAt: Date;
  updatedAt: Date;
}

export interface ConnectionCredentials {
  // Common
  accountId?: string;
  apiKey?: string;
  
  // OAuth
  accessToken?: string;
  refreshToken?: string;
  expiresAt?: string;
  
  // Network-specific
  organizationId?: string; // Unity
  reportKey?: string; // AppLovin
  sdkKey?: string; // AppLovin
  secretKey?: string; // IronSource
  appId?: string; // Meta
}

interface CreateConnectionInput {
  publisherId: string;
  networkType: NetworkType;
  displayName?: string;
  credentials: ConnectionCredentials;
  networkAccountId?: string;
  networkAppIds?: string[];
}

// Encryption key from environment (should be 32 bytes for AES-256)
const ENCRYPTION_KEY = process.env.VRA_ENCRYPTION_KEY || 'default-dev-key-change-in-prod!!';

function encrypt(text: string): { encrypted: Buffer; iv: Buffer } {
  const iv = crypto.randomBytes(16);
  const key = crypto.scryptSync(ENCRYPTION_KEY, 'salt', 32);
  const cipher = crypto.createCipheriv('aes-256-cbc', key, iv);
  let encrypted = cipher.update(text, 'utf8');
  encrypted = Buffer.concat([encrypted, cipher.final()]);
  return { encrypted, iv };
}

function decrypt(encrypted: Buffer, iv: Buffer): string {
  const key = crypto.scryptSync(ENCRYPTION_KEY, 'salt', 32);
  const decipher = crypto.createDecipheriv('aes-256-cbc', key, iv);
  let decrypted = decipher.update(encrypted);
  decrypted = Buffer.concat([decrypted, decipher.final()]);
  return decrypted.toString('utf8');
}

export class NetworkConnectorService {
  constructor(private pool: Pool) {}

  /**
   * Get all supported networks with configuration info
   */
  getSupportedNetworks(): Array<{ type: NetworkType; config: NetworkConfig }> {
    return Object.entries(SUPPORTED_NETWORKS).map(([type, config]) => ({
      type: type as NetworkType,
      config,
    }));
  }

  /**
   * Get OAuth URL for networks that use OAuth
   */
  getOAuthUrl(networkType: NetworkType, publisherId: string, redirectUri: string): string | null {
    const network = SUPPORTED_NETWORKS[networkType];
    if (network.authType !== 'oauth' || !network.oauthConfig) {
      return null;
    }

    const state = Buffer.from(JSON.stringify({ publisherId, networkType })).toString('base64');
    const params = new URLSearchParams({
      client_id: process.env[`${networkType.toUpperCase()}_CLIENT_ID`] || '',
      redirect_uri: redirectUri,
      response_type: 'code',
      scope: network.oauthConfig.scopes.join(' '),
      state,
      access_type: 'offline',
      prompt: 'consent',
    });

    return `${network.oauthConfig.authUrl}?${params.toString()}`;
  }

  /**
   * Exchange OAuth code for tokens
   */
  async handleOAuthCallback(
    networkType: NetworkType,
    code: string,
    redirectUri: string
  ): Promise<{ accessToken: string; refreshToken?: string; expiresAt?: Date }> {
    const network = SUPPORTED_NETWORKS[networkType];
    if (network.authType !== 'oauth' || !network.oauthConfig) {
      throw new Error(`${networkType} does not use OAuth`);
    }

    const response = await fetch(network.oauthConfig.tokenUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        client_id: process.env[`${networkType.toUpperCase()}_CLIENT_ID`] || '',
        client_secret: process.env[`${networkType.toUpperCase()}_CLIENT_SECRET`] || '',
        code,
        redirect_uri: redirectUri,
        grant_type: 'authorization_code',
      }),
    });

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`OAuth token exchange failed: ${error}`);
    }

    interface OAuthTokenResponse {
      access_token?: string;
      refresh_token?: string;
      expires_in?: number;
    }
    const data = await response.json() as OAuthTokenResponse;
    
    if (!data.access_token) {
      throw new Error('OAuth token exchange failed: No access token returned');
    }
    
    return {
      accessToken: data.access_token,
      refreshToken: data.refresh_token,
      expiresAt: data.expires_in ? new Date(Date.now() + data.expires_in * 1000) : undefined,
    };
  }

  /**
   * Create a new network connection
   */
  async createConnection(input: CreateConnectionInput): Promise<NetworkConnection> {
    const { publisherId, networkType, displayName, credentials, networkAccountId, networkAppIds } = input;

    // Validate required fields
    const required = SUPPORTED_NETWORKS[networkType].requiredFields;
    for (const field of required) {
      if (!credentials[field as keyof ConnectionCredentials]) {
        throw new Error(`Missing required field: ${field}`);
      }
    }

    // Encrypt credentials
    const credJson = JSON.stringify(credentials);
    const { encrypted, iv } = encrypt(credJson);

    const result = await this.pool.query(
      `INSERT INTO vra_network_connections 
       (publisher_id, network_type, display_name, credentials_encrypted, credentials_iv,
        oauth_access_token, oauth_refresh_token, oauth_expires_at,
        network_account_id, network_app_ids, status)
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'pending')
       ON CONFLICT (publisher_id, network_type) 
       DO UPDATE SET 
         display_name = EXCLUDED.display_name,
         credentials_encrypted = EXCLUDED.credentials_encrypted,
         credentials_iv = EXCLUDED.credentials_iv,
         oauth_access_token = EXCLUDED.oauth_access_token,
         oauth_refresh_token = EXCLUDED.oauth_refresh_token,
         oauth_expires_at = EXCLUDED.oauth_expires_at,
         network_account_id = EXCLUDED.network_account_id,
         network_app_ids = EXCLUDED.network_app_ids,
         status = 'pending',
         updated_at = NOW()
       RETURNING *`,
      [
        publisherId,
        networkType,
        displayName || SUPPORTED_NETWORKS[networkType].name,
        encrypted,
        iv,
        credentials.accessToken || null,
        credentials.refreshToken || null,
        credentials.expiresAt || null,
        networkAccountId || credentials.accountId || null,
        JSON.stringify(networkAppIds || []),
      ]
    );

    const conn = this.mapConnection(result.rows[0]);

    // Trigger initial validation
    this.validateConnection(conn.id).catch((err) => {
      logger.warn('Initial connection validation failed', { connectionId: conn.id, error: err.message });
    });

    return conn;
  }

  /**
   * Get all connections for a publisher
   */
  async getConnections(publisherId: string): Promise<NetworkConnection[]> {
    const result = await this.pool.query(
      `SELECT * FROM vra_network_connections WHERE publisher_id = $1 ORDER BY created_at DESC`,
      [publisherId]
    );
    return result.rows.map(this.mapConnection);
  }

  /**
   * Get a single connection
   */
  async getConnection(connectionId: string): Promise<NetworkConnection | null> {
    const result = await this.pool.query(
      `SELECT * FROM vra_network_connections WHERE id = $1`,
      [connectionId]
    );
    return result.rows[0] ? this.mapConnection(result.rows[0]) : null;
  }

  /**
   * Get decrypted credentials for a connection
   */
  async getCredentials(connectionId: string): Promise<ConnectionCredentials | null> {
    const result = await this.pool.query(
      `SELECT credentials_encrypted, credentials_iv, oauth_access_token, oauth_refresh_token, oauth_expires_at
       FROM vra_network_connections WHERE id = $1`,
      [connectionId]
    );

    if (!result.rows[0]) return null;

    const row = result.rows[0];
    let creds: ConnectionCredentials = {};

    if (row.credentials_encrypted && row.credentials_iv) {
      try {
        const decrypted = decrypt(row.credentials_encrypted, row.credentials_iv);
        creds = JSON.parse(decrypted);
      } catch (err) {
        logger.error('Failed to decrypt credentials', { connectionId, error: (err as Error).message });
      }
    }

    // Add OAuth tokens
    if (row.oauth_access_token) {
      creds.accessToken = row.oauth_access_token;
    }
    if (row.oauth_refresh_token) {
      creds.refreshToken = row.oauth_refresh_token;
    }
    if (row.oauth_expires_at) {
      creds.expiresAt = row.oauth_expires_at;
    }

    return creds;
  }

  /**
   * Update connection status
   */
  async updateConnectionStatus(
    connectionId: string,
    status: NetworkConnection['status'],
    message?: string
  ): Promise<void> {
    await this.pool.query(
      `UPDATE vra_network_connections 
       SET status = $2, status_message = $3, 
           last_error_at = CASE WHEN $2 = 'error' THEN NOW() ELSE last_error_at END,
           updated_at = NOW()
       WHERE id = $1`,
      [connectionId, status, message || null]
    );
  }

  /**
   * Record successful sync
   */
  async recordSyncSuccess(connectionId: string): Promise<void> {
    await this.pool.query(
      `UPDATE vra_network_connections 
       SET status = 'active', last_sync_at = NOW(), status_message = NULL, updated_at = NOW()
       WHERE id = $1`,
      [connectionId]
    );
  }

  /**
   * Validate connection by testing API access
   */
  async validateConnection(connectionId: string): Promise<{ valid: boolean; message: string }> {
    const conn = await this.getConnection(connectionId);
    if (!conn) {
      return { valid: false, message: 'Connection not found' };
    }

    const creds = await this.getCredentials(connectionId);
    if (!creds) {
      await this.updateConnectionStatus(connectionId, 'error', 'No credentials found');
      return { valid: false, message: 'No credentials found' };
    }

    try {
      // Network-specific validation
      const result = await this.testNetworkApi(conn.networkType, creds);
      
      if (result.valid) {
        await this.updateConnectionStatus(connectionId, 'active', 'Connection verified');
      } else {
        await this.updateConnectionStatus(connectionId, 'error', result.message);
      }
      
      return result;
    } catch (err) {
      const message = (err as Error).message;
      await this.updateConnectionStatus(connectionId, 'error', message);
      return { valid: false, message };
    }
  }

  /**
   * Delete a connection
   */
  async deleteConnection(connectionId: string): Promise<void> {
    await this.pool.query(
      `DELETE FROM vra_network_connections WHERE id = $1`,
      [connectionId]
    );
  }

  /**
   * Get connections due for sync
   */
  async getConnectionsDueForSync(): Promise<NetworkConnection[]> {
    const result = await this.pool.query(
      `SELECT * FROM vra_network_connections 
       WHERE sync_enabled = true 
         AND status = 'active'
         AND (last_sync_at IS NULL OR last_sync_at < NOW() - (sync_frequency_hours || ' hours')::interval)
       ORDER BY last_sync_at ASC NULLS FIRST
       LIMIT 50`
    );
    return result.rows.map(this.mapConnection);
  }

  /**
   * Test network API access
   */
  private async testNetworkApi(
    networkType: NetworkType,
    creds: ConnectionCredentials
  ): Promise<{ valid: boolean; message: string }> {
    switch (networkType) {
      case 'admob':
        return this.testAdMobApi(creds);
      case 'unity':
        return this.testUnityApi(creds);
      case 'applovin':
        return this.testAppLovinApi(creds);
      case 'ironsource':
        return this.testIronSourceApi(creds);
      case 'meta':
        return this.testMetaApi(creds);
      case 'vungle':
        return this.testVungleApi(creds);
      default:
        return { valid: false, message: 'Unknown network type' };
    }
  }

  private async testAdMobApi(creds: ConnectionCredentials): Promise<{ valid: boolean; message: string }> {
    if (!creds.accessToken || !creds.accountId) {
      return { valid: false, message: 'Missing access token or account ID' };
    }

    try {
      const response = await fetch(
        `https://admob.googleapis.com/v1/accounts/${creds.accountId}`,
        { headers: { Authorization: `Bearer ${creds.accessToken}` } }
      );

      if (response.status === 401) {
        return { valid: false, message: 'Invalid or expired access token' };
      }
      if (!response.ok) {
        return { valid: false, message: `AdMob API error: ${response.status}` };
      }

      return { valid: true, message: 'AdMob connection verified' };
    } catch (err) {
      return { valid: false, message: `AdMob API unreachable: ${(err as Error).message}` };
    }
  }

  private async testUnityApi(creds: ConnectionCredentials): Promise<{ valid: boolean; message: string }> {
    if (!creds.apiKey || !creds.organizationId) {
      return { valid: false, message: 'Missing API key or organization ID' };
    }

    try {
      const response = await fetch(
        `https://services.api.unity.com/monetize/v1/orgs/${creds.organizationId}/projects`,
        { headers: { Authorization: `Basic ${Buffer.from(creds.apiKey + ':').toString('base64')}` } }
      );

      if (response.status === 401) {
        return { valid: false, message: 'Invalid API key' };
      }
      if (!response.ok) {
        return { valid: false, message: `Unity API error: ${response.status}` };
      }

      return { valid: true, message: 'Unity connection verified' };
    } catch (err) {
      return { valid: false, message: `Unity API unreachable: ${(err as Error).message}` };
    }
  }

  private async testAppLovinApi(creds: ConnectionCredentials): Promise<{ valid: boolean; message: string }> {
    if (!creds.reportKey) {
      return { valid: false, message: 'Missing report key' };
    }

    try {
      // AppLovin uses a simple key-based auth for reporting
      const today = new Date().toISOString().split('T')[0];
      const response = await fetch(
        `https://r.applovin.com/report?api_key=${creds.reportKey}&start=${today}&end=${today}&format=json&columns=day`,
        { method: 'GET' }
      );

      if (response.status === 401 || response.status === 403) {
        return { valid: false, message: 'Invalid report key' };
      }
      if (!response.ok) {
        return { valid: false, message: `AppLovin API error: ${response.status}` };
      }

      return { valid: true, message: 'AppLovin connection verified' };
    } catch (err) {
      return { valid: false, message: `AppLovin API unreachable: ${(err as Error).message}` };
    }
  }

  private async testIronSourceApi(creds: ConnectionCredentials): Promise<{ valid: boolean; message: string }> {
    if (!creds.secretKey || !creds.refreshToken) {
      return { valid: false, message: 'Missing secret key or refresh token' };
    }

    // IronSource uses a bearer token that needs to be refreshed
    return { valid: true, message: 'IronSource credentials saved (will validate on first sync)' };
  }

  private async testMetaApi(creds: ConnectionCredentials): Promise<{ valid: boolean; message: string }> {
    if (!creds.accessToken) {
      return { valid: false, message: 'Missing access token' };
    }

    try {
      const response = await fetch(
        `https://graph.facebook.com/v18.0/me?access_token=${creds.accessToken}`
      );

      if (response.status === 401) {
        return { valid: false, message: 'Invalid or expired access token' };
      }
      if (!response.ok) {
        return { valid: false, message: `Meta API error: ${response.status}` };
      }

      return { valid: true, message: 'Meta connection verified' };
    } catch (err) {
      return { valid: false, message: `Meta API unreachable: ${(err as Error).message}` };
    }
  }

  private async testVungleApi(creds: ConnectionCredentials): Promise<{ valid: boolean; message: string }> {
    if (!creds.apiKey) {
      return { valid: false, message: 'Missing API key' };
    }

    // Vungle requires account ID and API key
    return { valid: true, message: 'Vungle credentials saved (will validate on first sync)' };
  }

  private mapConnection(row: any): NetworkConnection {
    return {
      id: row.id,
      publisherId: row.publisher_id,
      networkType: row.network_type,
      displayName: row.display_name,
      status: row.status,
      statusMessage: row.status_message,
      lastSyncAt: row.last_sync_at,
      networkAccountId: row.network_account_id,
      networkAppIds: row.network_app_ids || [],
      syncEnabled: row.sync_enabled,
      syncFrequencyHours: row.sync_frequency_hours,
      syncLookbackDays: row.sync_lookback_days || 30,
      createdAt: row.created_at,
      updatedAt: row.updated_at,
    };
  }
}

export const createNetworkConnectorService = (pool: Pool) => new NetworkConnectorService(pool);
