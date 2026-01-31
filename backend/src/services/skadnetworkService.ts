/**
 * SKAdNetwork Service
 * 
 * Implements Apple's SKAdNetwork 4.0+ for privacy-compliant iOS attribution.
 * Handles postback processing, conversion value updates, and campaign tracking.
 */

import crypto from 'crypto';
import logger from '../utils/logger';
import { AppDataSource } from '../database';
import { SkanPostback } from '../database/entities/skanPostback.entity';
import { sha256Hex } from '../utils/crypto';

// ========================================
// SKAdNetwork Types
// ========================================

export interface SKAdNetworkVersion {
  version: string;
  supported: boolean;
  features: string[];
}

export interface SKAdNetworkSignature {
  version: string;
  network: string;
  campaign: string;
  itunesitem: string;
  nonce: string;
  sourceapp: string;
  timestamp: string;
  signature: string;
}

export interface SKAdNetworkPostback {
  version: string;
  'ad-network-id': string;
  'campaign-id': string;
  'transaction-id'?: string;
  'app-id': string;
  'attribution-signature': string;
  'redownload'?: boolean;
  'source-app-id'?: string;
  'fidelity-type'?: 0 | 1; // 0 = StoreKit-rendered, 1 = view-through
  'conversion-value'?: number; // 0-63 for SKAdNetwork 3.0+
  'coarse-conversion-value'?: 'low' | 'medium' | 'high'; // SKAdNetwork 4.0+
  'did-win'?: boolean;
  'source-identifier'?: number; // 0-99 for multiple placements
}

export interface ConversionValue {
  value: number; // 0-63
  coarseValue?: 'low' | 'medium' | 'high';
  lockWindow?: number; // Days until value locks (0-35)
}

export interface SKAdNetworkCampaign {
  campaignId: string;
  networkId: string;
  conversionValueSchema: ConversionValueMapping[];
  createdAt: Date;
  updatedAt: Date;
}

export interface ConversionValueMapping {
  min: number;
  max: number;
  events: string[];
  value: number;
  coarseValue: 'low' | 'medium' | 'high';
}

// ========================================
// SKAdNetwork Configuration
// ========================================

const DEFAULT_CONVERSION_SCHEMA: ConversionValueMapping[] = [
  { min: 0, max: 0, events: [], value: 0, coarseValue: 'low' },
  { min: 1, max: 1, events: ['app_open'], value: 1, coarseValue: 'low' },
  { min: 2, max: 5, events: ['session_start'], value: 10, coarseValue: 'low' },
  { min: 6, max: 10, events: ['level_complete'], value: 20, coarseValue: 'medium' },
  { min: 11, max: 20, events: ['purchase'], value: 40, coarseValue: 'high' },
  { min: 21, max: 50, events: ['purchase'], value: 50, coarseValue: 'high' },
  { min: 51, max: 100, events: ['purchase'], value: 63, coarseValue: 'high' },
];

// ========================================
// SKAdNetwork Service
// ========================================

export class SKAdNetworkService {
  private campaigns: Map<string, SKAdNetworkCampaign> = new Map();
  private postbacks: SKAdNetworkPostback[] = [];

  /**
   * Get supported SKAdNetwork versions
   */
  getSupportedVersions(): SKAdNetworkVersion[] {
    return [
      {
        version: '4.0',
        supported: true,
        features: [
          'coarse_conversion_values',
          'multiple_postbacks',
          'source_identifier',
          'hierarchical_source_ids',
          'web_to_app_attribution',
        ],
      },
      {
        version: '3.0',
        supported: true,
        features: ['conversion_values', 'view_through_attribution', 'fidelity_type'],
      },
      {
        version: '2.2',
        supported: true,
        features: ['install_attribution', 'campaign_tracking'],
      },
    ];
  }

  /**
   * Generate SKAdNetwork signature parameters for bid response
   */
  generateSignatureParams(params: {
    version: string;
    networkId: string;
    campaignId: string;
    appId: string;
    nonce: string;
    sourceAppId?: string;
    timestamp?: number;
  }): SKAdNetworkSignature {
    const timestamp = params.timestamp || Date.now();

    const signatureData = {
      version: params.version,
      network: params.networkId,
      campaign: params.campaignId,
      itunesitem: params.appId,
      nonce: params.nonce,
      sourceapp: params.sourceAppId || '',
      timestamp: timestamp.toString(),
    };

    // In production, sign with your private key
    // This is a placeholder - use actual cryptographic signing
    const signature = this.signData(signatureData);

    return {
      ...signatureData,
      signature,
    };
  }

  /**
   * Create or update campaign conversion value schema
   */
  createCampaign(params: {
    campaignId: string;
    networkId: string;
    conversionSchema?: ConversionValueMapping[];
  }): SKAdNetworkCampaign {
    const campaign: SKAdNetworkCampaign = {
      campaignId: params.campaignId,
      networkId: params.networkId,
      conversionValueSchema: params.conversionSchema || DEFAULT_CONVERSION_SCHEMA,
      createdAt: new Date(),
      updatedAt: new Date(),
    };

    this.campaigns.set(params.campaignId, campaign);

    logger.info('SKAdNetwork campaign created', {
      campaignId: params.campaignId,
      networkId: params.networkId,
    });

    return campaign;
  }

  /**
   * Process SKAdNetwork postback from Apple
   */
  async processPostback(postback: SKAdNetworkPostback): Promise<{
    success: boolean;
    attribution?: {
      campaignId: string;
      networkId: string;
      appId: string;
      conversionValue?: number;
      coarseValue?: string;
      didWin: boolean;
    };
    error?: string;
  }> {
    try {
      // Verify signature
      const isValid = await this.verifyPostbackSignature(postback);
      if (!isValid) {
        logger.warn('Invalid SKAdNetwork postback signature', {
          campaignId: postback['campaign-id'],
          networkId: postback['ad-network-id'],
        });

        return {
          success: false,
          error: 'Invalid signature',
        };
      }

      // Persist postback with replay protection
      try {
        const repo = AppDataSource.getRepository(SkanPostback);
        const fingerprint = this.computeReplayFingerprint(postback);
        const entity = repo.create({
          networkId: postback['ad-network-id'],
          campaignId: postback['campaign-id'],
          version: postback.version,
          redownload: Boolean(postback['redownload']),
          fingerprint,
          signatureFields: this.pickSignatureFields(postback),
          raw: postback as any,
        });
        await repo.save(entity);
      } catch (e: any) {
        // Unique constraint => replay; treat as success (idempotent)
        const msg = String(e?.message || '');
        const isUnique = /duplicate key|unique constraint/i.test(msg);
        if (!isUnique) {
          throw e;
        }
        logger.warn('SKAN replay postback ignored (duplicate)', {
          networkId: postback['ad-network-id'],
          campaignId: postback['campaign-id'],
        });
      }

      // Extract attribution data
      const attribution = {
        campaignId: postback['campaign-id'],
        networkId: postback['ad-network-id'],
        appId: postback['app-id'],
        conversionValue: postback['conversion-value'],
        coarseValue: postback['coarse-conversion-value'],
        didWin: postback['did-win'] ?? false,
      };

      logger.info('SKAdNetwork postback processed', {
        campaignId: attribution.campaignId,
        conversionValue: attribution.conversionValue,
        coarseValue: attribution.coarseValue,
        didWin: attribution.didWin,
      });

      // Track recent postbacks in-memory for analytics/tests (bounded to prevent leaks)
      this.postbacks.push({ ...postback });
      if (this.postbacks.length > 2000) {
        this.postbacks.shift();
      }

      return {
        success: true,
        attribution,
      };
    } catch (error) {
      logger.error('Failed to process SKAdNetwork postback', { error });
      return {
        success: false,
        error: error instanceof Error ? error.message : 'Unknown error',
      };
    }
  }

  private computeReplayFingerprint(postback: SKAdNetworkPostback): string {
    // Use stable set of core fields; omit fields that can vary but not identity (e.g., timestamp precision).
    const core = {
      v: postback.version,
      n: postback['ad-network-id'],
      c: postback['campaign-id'],
      a: postback['app-id'],
      t: postback['transaction-id'] || '',
      s: postback['source-app-id'] || '',
      f: postback['fidelity-type'] ?? '',
      cv: postback['conversion-value'] ?? '',
      ccv: postback['coarse-conversion-value'] || '',
      dw: postback['redownload'] ? '1' : '0',
    };
    return sha256Hex(JSON.stringify(core));
  }

  private pickSignatureFields(postback: SKAdNetworkPostback): Record<string, unknown> {
    const fields: Record<string, unknown> = {
      version: postback.version,
      ad_network_id: postback['ad-network-id'],
      campaign_id: postback['campaign-id'],
      app_id: postback['app-id'],
      source_app_id: postback['source-app-id'],
      fidelity_type: postback['fidelity-type'],
      conversion_value: postback['conversion-value'],
      coarse_conversion_value: postback['coarse-conversion-value'],
      did_win: postback['did-win'],
    };
    return fields;
  }

  /**
   * Calculate conversion value based on user events
   */
  calculateConversionValue(params: {
    campaignId: string;
    eventCount: number;
    eventTypes: string[];
    revenue?: number;
  }): ConversionValue {
    const campaign = this.campaigns.get(params.campaignId);
    const schema = campaign?.conversionValueSchema || DEFAULT_CONVERSION_SCHEMA;

    // Find matching conversion value based on event count
    let matchedMapping = schema[0]; // Default to lowest

    for (const mapping of schema) {
      if (params.eventCount >= mapping.min && params.eventCount <= mapping.max) {
        // Check if event types match
        const hasMatchingEvent = mapping.events.some((event) =>
          params.eventTypes.includes(event)
        );

        if (hasMatchingEvent || mapping.events.length === 0) {
          matchedMapping = mapping;
          break;
        }
      }
    }

    // SKAdNetwork 4.0+ coarse conversion values
    let coarseValue: 'low' | 'medium' | 'high' = matchedMapping.coarseValue;

    // Override with revenue-based logic if provided
    if (params.revenue !== undefined) {
      if (params.revenue >= 50) {
        coarseValue = 'high';
      } else if (params.revenue >= 10) {
        coarseValue = 'medium';
      } else {
        coarseValue = 'low';
      }
    }

    return {
      value: matchedMapping.value,
      coarseValue,
      lockWindow: this.calculateLockWindow(matchedMapping.value),
    };
  }

  /**
   * Update conversion value (can be called multiple times)
   */
  updateConversionValue(params: {
    transactionId: string;
    campaignId: string;
    newValue: number;
    coarseValue?: 'low' | 'medium' | 'high';
  }): { success: boolean; error?: string } {
    // Validate conversion value range (0-63)
    if (params.newValue < 0 || params.newValue > 63) {
      return {
        success: false,
        error: 'Conversion value must be between 0 and 63',
      };
    }

    logger.info('Conversion value updated', {
      transactionId: params.transactionId,
      campaignId: params.campaignId,
      value: params.newValue,
      coarseValue: params.coarseValue,
    });

    return { success: true };
  }

  /**
   * Get campaign attribution statistics
   */
  getCampaignStats(campaignId: string): {
    totalPostbacks: number;
    installs: number;
    redownloads: number;
    averageConversionValue: number;
    conversionDistribution: Record<string, number>;
  } {
    const campaignPostbacks = this.postbacks.filter(
      (p) => p['campaign-id'] === campaignId
    );

    const installs = campaignPostbacks.filter((p) => !p.redownload).length;
    const redownloads = campaignPostbacks.filter((p) => p.redownload).length;

    const conversionValues = campaignPostbacks
      .filter((p) => p['conversion-value'] !== undefined)
      .map((p) => p['conversion-value']!);

    const averageConversionValue =
      conversionValues.length > 0
        ? conversionValues.reduce((a, b) => a + b, 0) / conversionValues.length
        : 0;

    // Distribution by coarse value
    const distribution: Record<string, number> = {
      low: 0,
      medium: 0,
      high: 0,
    };

    campaignPostbacks.forEach((p) => {
      if (p['coarse-conversion-value']) {
        distribution[p['coarse-conversion-value']]++;
      }
    });

    return {
      totalPostbacks: campaignPostbacks.length,
      installs,
      redownloads,
      averageConversionValue,
      conversionDistribution: distribution,
    };
  }

  /**
   * Generate nonce for signature
   */
  generateNonce(): string {
    return crypto.randomUUID();
  }

  // ========================================
  // Private Methods
  // ========================================

  private signData(data: Record<string, string>): string {
    // In production, use Apple's provided signing method with your private key
    // This is a simplified example using HMAC-SHA256
    const dataString = Object.entries(data)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, value]) => `${key}=${value}`)
      .join('&');

    // SECURITY: Require SKADNETWORK_SECRET in production (P1 Fix #1)
    const secret = process.env.SKADNETWORK_SECRET;
    if (!secret && process.env.NODE_ENV === 'production') {
      throw new Error('SKADNETWORK_SECRET must be set in production');
    }
    const signingSecret = secret || 'dev-skadnetwork-secret-not-for-production';
    return crypto.createHmac('sha256', signingSecret).update(dataString).digest('base64');
  }

  private async verifyPostbackSignature(
    postback: SKAdNetworkPostback
  ): Promise<boolean> {
    // In production, verify signature using Apple's public key
    // This is a placeholder for demonstration
    try {
      const signature = postback['attribution-signature'];
      return Boolean(signature && signature.length > 0);
    } catch (error) {
      logger.error('Signature verification failed', { error });
      return false;
    }
  }

  private calculateLockWindow(conversionValue: number): number {
    // Higher conversion values lock sooner to preserve privacy
    if (conversionValue >= 50) return 7; // Lock after 7 days
    if (conversionValue >= 30) return 14; // Lock after 14 days
    if (conversionValue >= 10) return 21; // Lock after 21 days
    return 35; // Maximum 35 days
  }
}

// Singleton instance
export const skadnetworkService = new SKAdNetworkService();

export default {
  skadnetworkService,
  SKAdNetworkService,
};
