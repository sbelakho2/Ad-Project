/**
 * Format Compatibility Validator
 *
 * Validates creative format compatibility with the player/session
 * requirements. Rejects incompatible creatives with explanations.
 */

import { SelectedCreative, MediaFile, FormatRequirements, RejectionReason } from './types';

export interface CompatibilityResult {
  compatible: boolean;
  reason?: string;
  rejectionReason?: RejectionReason;
  recommendations?: string[];
  selectedMediaFile?: MediaFile;
}

export interface ValidationConfig {
  /** Strict mode - reject any mismatch */
  strictMode: boolean;

  /** Maximum allowed bitrate difference (%) */
  maxBitrateDifference: number;

  /** Allowed segment duration variance (seconds) */
  segmentDurationTolerance: number;

  /** Require exact codec match */
  requireExactCodecMatch: boolean;

  /** Require init segment for CMAF */
  requireInitSegment: boolean;
}

/**
 * Format Validator ensures creative media files are compatible
 * with the session's protocol, container, and codec requirements.
 */
export class FormatValidator {
  private config: ValidationConfig;

  constructor(config?: Partial<ValidationConfig>) {
    this.config = {
      strictMode: false,
      maxBitrateDifference: 50,
      segmentDurationTolerance: 0.5,
      requireExactCodecMatch: false,
      requireInitSegment: true,
      ...config,
    };
  }

  /**
   * Validate creative compatibility with format requirements.
   */
  validateCreative(
    creative: SelectedCreative,
    requirements: FormatRequirements
  ): CompatibilityResult {
    // Must have at least one media file
    if (!creative.mediaFiles || creative.mediaFiles.length === 0) {
      return {
        compatible: false,
        reason: 'Creative has no media files',
        rejectionReason: 'format_incompatible',
      };
    }

    // Find best matching media file
    const bestMatch = this.findBestMediaFile(creative.mediaFiles, requirements);

    if (!bestMatch.mediaFile) {
      return {
        compatible: false,
        reason: bestMatch.reason || 'No compatible media file found',
        rejectionReason: bestMatch.rejectionReason || 'format_incompatible',
        recommendations: bestMatch.recommendations,
      };
    }

    return {
      compatible: true,
      selectedMediaFile: bestMatch.mediaFile,
    };
  }

  /**
   * Find the best matching media file for requirements.
   */
  findBestMediaFile(
    mediaFiles: MediaFile[],
    requirements: FormatRequirements
  ): {
    mediaFile?: MediaFile;
    reason?: string;
    rejectionReason?: RejectionReason;
    recommendations?: string[];
  } {
    const candidates: Array<{
      file: MediaFile;
      score: number;
      issues: string[];
    }> = [];

    for (const file of mediaFiles) {
      const validation = this.validateMediaFile(file, requirements);
      if (validation.compatible) {
        candidates.push({
          file,
          score: this.scoreMediaFile(file, requirements),
          issues: [],
        });
      } else {
        candidates.push({
          file,
          score: -1,
          issues: [validation.reason || 'Incompatible'],
        });
      }
    }

    // Sort by score (highest first)
    candidates.sort((a, b) => b.score - a.score);

    const bestCandidate = candidates[0];

    if (bestCandidate && bestCandidate.score >= 0) {
      return { mediaFile: bestCandidate.file };
    }

    // No compatible file found - provide helpful error
    const allIssues = candidates.flatMap((c) => c.issues);
    const uniqueIssues = [...new Set(allIssues)];

    return {
      reason: uniqueIssues.join('; '),
      rejectionReason: this.determineRejectionReason(uniqueIssues),
      recommendations: this.generateRecommendations(uniqueIssues, requirements),
    };
  }

  /**
   * Validate a single media file against requirements.
   */
  validateMediaFile(
    file: MediaFile,
    requirements: FormatRequirements
  ): CompatibilityResult {
    // Check container compatibility
    const containerCheck = this.checkContainerCompatibility(
      file.container,
      requirements.container
    );
    if (!containerCheck.compatible) {
      return containerCheck;
    }

    // Check codec compatibility
    const codecCheck = this.checkCodecCompatibility(
      file.codec,
      requirements.videoCodecs,
      requirements.audioCodecs
    );
    if (!codecCheck.compatible) {
      return codecCheck;
    }

    // Check resolution requirements
    if (requirements.resolution) {
      const resolutionCheck = this.checkResolutionCompatibility(
        file.width,
        file.height,
        requirements.resolution
      );
      if (!resolutionCheck.compatible) {
        return resolutionCheck;
      }
    }

    // Check init segment for CMAF
    if (
      requirements.container === 'cmaf' &&
      this.config.requireInitSegment &&
      !file.initSegment
    ) {
      return {
        compatible: false,
        reason: 'CMAF requires init segment',
        rejectionReason: 'format_incompatible',
      };
    }

    return { compatible: true };
  }

  /**
   * Check container compatibility.
   */
  private checkContainerCompatibility(
    fileContainer: string,
    requiredContainer: string
  ): CompatibilityResult {
    const normalizedFile = this.normalizeContainer(fileContainer);
    const normalizedRequired = this.normalizeContainer(requiredContainer);

    if (normalizedFile === normalizedRequired) {
      return { compatible: true };
    }

    // Check for compatible containers
    const tsContainers = ['ts', 'm2ts', 'mts', 'mp2t'];
    const cmafContainers = ['cmaf', 'fmp4', 'm4s', 'mp4'];

    if (
      tsContainers.includes(normalizedFile) &&
      tsContainers.includes(normalizedRequired)
    ) {
      return { compatible: true };
    }

    if (
      cmafContainers.includes(normalizedFile) &&
      cmafContainers.includes(normalizedRequired)
    ) {
      return { compatible: true };
    }

    return {
      compatible: false,
      reason: `Container mismatch: creative uses ${fileContainer}, session requires ${requiredContainer}`,
      rejectionReason: 'format_incompatible',
      recommendations: [`Transcode creative to ${requiredContainer}`],
    };
  }

  /**
   * Check codec compatibility.
   */
  private checkCodecCompatibility(
    fileCodec: string | undefined,
    requiredVideoCodecs: string[],
    requiredAudioCodecs: string[]
  ): CompatibilityResult {
    if (!fileCodec) {
      // No codec info - allow in non-strict mode
      if (this.config.strictMode) {
        return {
          compatible: false,
          reason: 'Creative codec information missing',
          rejectionReason: 'codec_mismatch',
        };
      }
      return { compatible: true };
    }

    // Parse codec string (e.g., "avc1.64001f,mp4a.40.2")
    const codecs = fileCodec.split(',').map((c) => c.trim());

    // Check video codec
    if (requiredVideoCodecs.length > 0) {
      const videoCodec = codecs.find((c) => this.isVideoCodec(c));
      if (videoCodec) {
        const videoCompatible = this.isCodecCompatible(
          videoCodec,
          requiredVideoCodecs
        );
        if (!videoCompatible) {
          return {
            compatible: false,
            reason: `Video codec ${videoCodec} not compatible with ${requiredVideoCodecs.join(', ')}`,
            rejectionReason: 'codec_mismatch',
          };
        }
      }
    }

    // Check audio codec
    if (requiredAudioCodecs.length > 0) {
      const audioCodec = codecs.find((c) => this.isAudioCodec(c));
      if (audioCodec) {
        const audioCompatible = this.isCodecCompatible(
          audioCodec,
          requiredAudioCodecs
        );
        if (!audioCompatible) {
          return {
            compatible: false,
            reason: `Audio codec ${audioCodec} not compatible with ${requiredAudioCodecs.join(', ')}`,
            rejectionReason: 'codec_mismatch',
          };
        }
      }
    }

    return { compatible: true };
  }

  /**
   * Check resolution compatibility.
   */
  private checkResolutionCompatibility(
    width: number | undefined,
    height: number | undefined,
    requirements: FormatRequirements['resolution']
  ): CompatibilityResult {
    if (!requirements) {
      return { compatible: true };
    }

    if (!width || !height) {
      // No resolution info - allow in non-strict mode
      if (this.config.strictMode) {
        return {
          compatible: false,
          reason: 'Creative resolution information missing',
          rejectionReason: 'format_incompatible',
        };
      }
      return { compatible: true };
    }

    if (requirements.minWidth && width < requirements.minWidth) {
      return {
        compatible: false,
        reason: `Width ${width} below minimum ${requirements.minWidth}`,
        rejectionReason: 'format_incompatible',
      };
    }

    if (requirements.maxWidth && width > requirements.maxWidth) {
      return {
        compatible: false,
        reason: `Width ${width} exceeds maximum ${requirements.maxWidth}`,
        rejectionReason: 'format_incompatible',
      };
    }

    if (requirements.minHeight && height < requirements.minHeight) {
      return {
        compatible: false,
        reason: `Height ${height} below minimum ${requirements.minHeight}`,
        rejectionReason: 'format_incompatible',
      };
    }

    if (requirements.maxHeight && height > requirements.maxHeight) {
      return {
        compatible: false,
        reason: `Height ${height} exceeds maximum ${requirements.maxHeight}`,
        rejectionReason: 'format_incompatible',
      };
    }

    return { compatible: true };
  }

  /**
   * Score a media file for ranking (higher is better).
   */
  private scoreMediaFile(
    file: MediaFile,
    requirements: FormatRequirements
  ): number {
    let score = 100;

    // Prefer streaming delivery
    if (file.delivery === 'streaming') {
      score += 20;
    }

    // Prefer higher bitrate (up to a point)
    if (file.bitrate) {
      // Optimal around 4000kbps for 1080p
      const optimal = 4000;
      const diff = Math.abs(file.bitrate - optimal);
      score -= diff / 100;
    }

    // Prefer higher resolution
    if (file.width && file.height) {
      if (file.width >= 1920 && file.height >= 1080) {
        score += 10;
      } else if (file.width >= 1280 && file.height >= 720) {
        score += 5;
      }
    }

    // Prefer files with init segment for CMAF
    if (requirements.container === 'cmaf' && file.initSegment) {
      score += 10;
    }

    return score;
  }

  /**
   * Normalize container name.
   */
  private normalizeContainer(container: string): string {
    const normalized = container.toLowerCase().trim();

    // Map common variations
    const mappings: Record<string, string> = {
      'video/mp2t': 'ts',
      mp2t: 'ts',
      m2ts: 'ts',
      mts: 'ts',
      'video/mp4': 'cmaf',
      fmp4: 'cmaf',
      m4s: 'cmaf',
      mp4: 'cmaf',
    };

    return mappings[normalized] || normalized;
  }

  /**
   * Check if codec string is a video codec.
   */
  private isVideoCodec(codec: string): boolean {
    return (
      codec.startsWith('avc') ||
      codec.startsWith('hvc') ||
      codec.startsWith('hev') ||
      codec.startsWith('vp') ||
      codec.startsWith('av0')
    );
  }

  /**
   * Check if codec string is an audio codec.
   */
  private isAudioCodec(codec: string): boolean {
    return (
      codec.startsWith('mp4a') ||
      codec.startsWith('ac-') ||
      codec.startsWith('ec-') ||
      codec.startsWith('opus')
    );
  }

  /**
   * Check if a codec is compatible with required codecs.
   */
  private isCodecCompatible(codec: string, required: string[]): boolean {
    if (this.config.requireExactCodecMatch) {
      return required.some((r) => codec.toLowerCase() === r.toLowerCase());
    }

    // Check codec family match
    const codecFamily = this.getCodecFamily(codec);
    return required.some((r) => {
      const requiredFamily = this.getCodecFamily(r);
      return codecFamily === requiredFamily;
    });
  }

  /**
   * Get codec family from codec string.
   */
  private getCodecFamily(codec: string): string {
    const lower = codec.toLowerCase();

    if (lower.startsWith('avc')) return 'h264';
    if (lower.startsWith('hvc') || lower.startsWith('hev')) return 'h265';
    if (lower.startsWith('vp8')) return 'vp8';
    if (lower.startsWith('vp9')) return 'vp9';
    if (lower.startsWith('av0') || lower.startsWith('av1')) return 'av1';
    if (lower.startsWith('mp4a.40')) return 'aac';
    if (lower === 'ac-3') return 'ac3';
    if (lower === 'ec-3') return 'eac3';
    if (lower.startsWith('opus')) return 'opus';

    return 'unknown';
  }

  /**
   * Determine rejection reason from issues.
   */
  private determineRejectionReason(issues: string[]): RejectionReason {
    for (const issue of issues) {
      if (issue.toLowerCase().includes('codec')) {
        return 'codec_mismatch';
      }
      if (issue.toLowerCase().includes('container')) {
        return 'format_incompatible';
      }
      if (issue.toLowerCase().includes('duration')) {
        return 'duration_mismatch';
      }
    }
    return 'format_incompatible';
  }

  /**
   * Generate recommendations based on issues.
   */
  private generateRecommendations(
    issues: string[],
    requirements: FormatRequirements
  ): string[] {
    const recommendations: string[] = [];

    for (const issue of issues) {
      if (issue.includes('container')) {
        recommendations.push(
          `Transcode creative to ${requirements.container} container`
        );
      }
      if (issue.includes('codec')) {
        recommendations.push(
          `Transcode creative to ${requirements.videoCodecs[0] || 'h264'}`
        );
      }
      if (issue.includes('init segment')) {
        recommendations.push('Ensure CMAF creative includes init segment');
      }
    }

    return [...new Set(recommendations)];
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<ValidationConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Get current configuration.
   */
  getConfig(): ValidationConfig {
    return { ...this.config };
  }
}

export default FormatValidator;
