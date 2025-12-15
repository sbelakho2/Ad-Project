/**
 * DRM Policy Handler
 *
 * Manages DRM policy enforcement for SSAI, including clear vs encrypted
 * ads, key continuity, and mixed-scheme handling.
 */

export type DRMScheme = 'widevine' | 'fairplay' | 'playready' | 'clearkey' | 'none';

export interface DRMInfo {
  scheme: DRMScheme;
  keyId?: string;
  licenseUrl?: string;
  initData?: string;
  isEncrypted: boolean;
}

export interface DRMPolicyConfig {
  /** Allow clear ads in encrypted content */
  allowClearAdsInEncryptedContent: boolean;

  /** Allow encrypted ads in clear content */
  allowEncryptedAdsInClearContent: boolean;

  /** Require same DRM scheme for ads and content */
  requireSameScheme: boolean;

  /** Require key continuity at boundaries */
  requireKeyContinuity: boolean;

  /** Supported DRM schemes */
  supportedSchemes: DRMScheme[];

  /** Default scheme for unspecified content */
  defaultScheme: DRMScheme;
}

export interface DRMValidationResult {
  valid: boolean;
  compatible: boolean;
  warnings: string[];
  errors: string[];
  recommendation?: string;
}

export interface KeyRotationInfo {
  currentKeyId: string;
  nextKeyId?: string;
  rotationTime?: number;
  requiresRekey: boolean;
}

/**
 * DRM Policy Handler for SSAI content protection.
 */
export class DRMPolicyHandler {
  private config: DRMPolicyConfig;

  constructor(config?: Partial<DRMPolicyConfig>) {
    this.config = {
      allowClearAdsInEncryptedContent: true,
      allowEncryptedAdsInClearContent: false,
      requireSameScheme: false,
      requireKeyContinuity: true,
      supportedSchemes: ['widevine', 'fairplay', 'playready', 'clearkey', 'none'],
      defaultScheme: 'none',
      ...config,
    };
  }

  /**
   * Validate DRM compatibility between content and ad.
   */
  validateCompatibility(
    contentDrm: DRMInfo,
    adDrm: DRMInfo
  ): DRMValidationResult {
    const result: DRMValidationResult = {
      valid: true,
      compatible: true,
      warnings: [],
      errors: [],
    };

    // Check scheme support
    if (!this.config.supportedSchemes.includes(contentDrm.scheme)) {
      result.errors.push(`Unsupported content DRM scheme: ${contentDrm.scheme}`);
      result.valid = false;
    }

    if (!this.config.supportedSchemes.includes(adDrm.scheme)) {
      result.errors.push(`Unsupported ad DRM scheme: ${adDrm.scheme}`);
      result.valid = false;
    }

    // Check clear ads in encrypted content
    if (contentDrm.isEncrypted && !adDrm.isEncrypted) {
      if (!this.config.allowClearAdsInEncryptedContent) {
        result.errors.push('Clear ads not allowed in encrypted content');
        result.compatible = false;
      } else {
        result.warnings.push('Inserting clear ads into encrypted content');
      }
    }

    // Check encrypted ads in clear content
    if (!contentDrm.isEncrypted && adDrm.isEncrypted) {
      if (!this.config.allowEncryptedAdsInClearContent) {
        result.errors.push('Encrypted ads not allowed in clear content');
        result.compatible = false;
      } else {
        result.warnings.push('Inserting encrypted ads into clear content');
      }
    }

    // Check scheme matching
    if (this.config.requireSameScheme) {
      if (contentDrm.isEncrypted && adDrm.isEncrypted) {
        if (contentDrm.scheme !== adDrm.scheme) {
          result.errors.push(
            `DRM scheme mismatch: content=${contentDrm.scheme}, ad=${adDrm.scheme}`
          );
          result.compatible = false;
        }
      }
    }

    // Add recommendation
    if (!result.compatible) {
      result.recommendation = this.generateRecommendation(contentDrm, adDrm);
    }

    return result;
  }

  /**
   * Check key continuity at splice boundary.
   */
  checkKeyContinuity(
    contentDrm: DRMInfo,
    adDrm: DRMInfo
  ): { continuous: boolean; issue?: string } {
    // Both clear - continuous
    if (!contentDrm.isEncrypted && !adDrm.isEncrypted) {
      return { continuous: true };
    }

    // Mixed clear/encrypted
    if (contentDrm.isEncrypted !== adDrm.isEncrypted) {
      return {
        continuous: false,
        issue: 'Key continuity broken: clear/encrypted boundary',
      };
    }

    // Both encrypted - check key ID
    if (this.config.requireKeyContinuity) {
      if (contentDrm.keyId !== adDrm.keyId) {
        return {
          continuous: false,
          issue: `Key ID change: ${contentDrm.keyId} → ${adDrm.keyId}`,
        };
      }
    }

    return { continuous: true };
  }

  /**
   * Handle key rotation during ad insertion.
   */
  handleKeyRotation(
    currentKey: string,
    nextKey: string | undefined,
    splicePosition: number
  ): KeyRotationInfo {
    const requiresRekey = !!nextKey && nextKey !== currentKey;

    return {
      currentKeyId: currentKey,
      nextKeyId: nextKey,
      rotationTime: requiresRekey ? splicePosition : undefined,
      requiresRekey,
    };
  }

  /**
   * Generate DRM info for stitched output.
   */
  generateStitchedDrmInfo(
    contentDrm: DRMInfo,
    adDrm: DRMInfo
  ): DRMInfo {
    // If content is encrypted, use content DRM for output
    if (contentDrm.isEncrypted) {
      return { ...contentDrm };
    }

    // If only ad is encrypted, use ad DRM
    if (adDrm.isEncrypted) {
      return { ...adDrm };
    }

    // Both clear
    return {
      scheme: 'none',
      isEncrypted: false,
    };
  }

  /**
   * Generate recommendation for incompatible DRM.
   */
  private generateRecommendation(
    contentDrm: DRMInfo,
    adDrm: DRMInfo
  ): string {
    if (contentDrm.isEncrypted && !adDrm.isEncrypted) {
      return 'Use clear ads with encrypted content, or transcode ads with matching DRM';
    }

    if (!contentDrm.isEncrypted && adDrm.isEncrypted) {
      return 'Use clear ads for clear content';
    }

    if (contentDrm.scheme !== adDrm.scheme) {
      return `Transcode ads to use ${contentDrm.scheme} DRM scheme`;
    }

    return 'Check DRM configuration and ad transcoding settings';
  }

  /**
   * Check if scheme is supported.
   */
  isSchemeSupported(scheme: DRMScheme): boolean {
    return this.config.supportedSchemes.includes(scheme);
  }

  /**
   * Get configuration.
   */
  getConfig(): DRMPolicyConfig {
    return { ...this.config };
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<DRMPolicyConfig>): void {
    this.config = { ...this.config, ...config };
  }
}

export default DRMPolicyHandler;
