/**
 * Manifest Service
 *
 * Base manifest service class for HLS and DASH manifest generation.
 * Provides common utilities and validation.
 */

import { SSAISession } from '../session/types';

export interface ManifestRequest {
  session: SSAISession;
  protocol: 'hls' | 'dash';
  variant?: string;
  timestamp?: number;
}

export interface ManifestResponse {
  content: string;
  contentType: string;
  headers: Record<string, string>;
  cachePolicy: CacheDirective;
}

export interface CacheDirective {
  maxAge?: number;
  private?: boolean;
  noStore?: boolean;
  noCache?: boolean;
  mustRevalidate?: boolean;
}

export interface ValidationResult {
  valid: boolean;
  errors: string[];
  warnings: string[];
}

export interface ManifestValidationRules {
  /** Check segment durations */
  validateDurations: boolean;

  /** Check discontinuity sequence */
  validateDiscontinuity: boolean;

  /** Check timeline consistency */
  validateTimeline: boolean;

  /** Check required tags */
  validateRequiredTags: boolean;
}

/**
 * Base Manifest Service providing common functionality.
 */
export class ManifestService {
  protected validationRules: ManifestValidationRules;

  constructor(rules?: Partial<ManifestValidationRules>) {
    this.validationRules = {
      validateDurations: true,
      validateDiscontinuity: true,
      validateTimeline: true,
      validateRequiredTags: true,
      ...rules,
    };
  }

  /**
   * Validate HLS manifest.
   */
  validateHLSManifest(content: string): ValidationResult {
    const errors: string[] = [];
    const warnings: string[] = [];

    const lines = content.split('\n');

    // Check for required header
    if (!lines[0]?.startsWith('#EXTM3U')) {
      errors.push('Missing #EXTM3U header');
    }

    // Check for version
    const versionLine = lines.find((l) => l.startsWith('#EXT-X-VERSION'));
    if (!versionLine) {
      warnings.push('Missing #EXT-X-VERSION tag');
    }

    // Check for target duration (media playlist)
    const hasSegments = lines.some((l) => l.startsWith('#EXTINF'));
    if (hasSegments) {
      const targetDurationLine = lines.find((l) =>
        l.startsWith('#EXT-X-TARGETDURATION')
      );
      if (!targetDurationLine) {
        errors.push('Missing #EXT-X-TARGETDURATION in media playlist');
      }

      // Validate segment durations
      if (this.validationRules.validateDurations) {
        const targetDuration = targetDurationLine
          ? parseInt(targetDurationLine.split(':')[1], 10)
          : 0;
        const durationErrors = this.validateHLSSegmentDurations(
          lines,
          targetDuration
        );
        errors.push(...durationErrors);
      }
    }

    // Check discontinuity sequence
    if (this.validationRules.validateDiscontinuity) {
      const discWarnings = this.validateHLSDiscontinuity(lines);
      warnings.push(...discWarnings);
    }

    return {
      valid: errors.length === 0,
      errors,
      warnings,
    };
  }

  /**
   * Validate HLS segment durations against target duration.
   */
  private validateHLSSegmentDurations(
    lines: string[],
    targetDuration: number
  ): string[] {
    const errors: string[] = [];

    for (const line of lines) {
      if (line.startsWith('#EXTINF:')) {
        const durationStr = line.substring(8).split(',')[0];
        const duration = parseFloat(durationStr);

        if (isNaN(duration) || duration <= 0) {
          errors.push(`Invalid EXTINF duration: ${durationStr}`);
        } else if (duration > targetDuration + 0.5) {
          errors.push(
            `Segment duration ${duration}s exceeds target duration ${targetDuration}s`
          );
        }
      }
    }

    return errors;
  }

  /**
   * Validate HLS discontinuity handling.
   */
  private validateHLSDiscontinuity(lines: string[]): string[] {
    const warnings: string[] = [];

    let discontinuityCount = 0;
    let hasDiscSequence = false;

    for (const line of lines) {
      if (line.startsWith('#EXT-X-DISCONTINUITY-SEQUENCE')) {
        hasDiscSequence = true;
      }
      if (line === '#EXT-X-DISCONTINUITY') {
        discontinuityCount++;
      }
    }

    if (discontinuityCount > 0 && !hasDiscSequence) {
      warnings.push(
        'Manifest has discontinuities but no EXT-X-DISCONTINUITY-SEQUENCE'
      );
    }

    return warnings;
  }

  /**
   * Validate DASH manifest.
   */
  validateDASHManifest(content: string): ValidationResult {
    const errors: string[] = [];
    const warnings: string[] = [];

    // Check for XML declaration
    if (!content.includes('<?xml')) {
      warnings.push('Missing XML declaration');
    }

    // Check for MPD root element
    if (!content.includes('<MPD')) {
      errors.push('Missing MPD root element');
    }

    // Check for required attributes
    const mpdMatch = content.match(/<MPD[^>]*>/);
    if (mpdMatch) {
      const mpdTag = mpdMatch[0];

      if (!mpdTag.includes('type=')) {
        warnings.push('MPD missing type attribute');
      }

      if (!mpdTag.includes('minBufferTime=')) {
        errors.push('MPD missing required minBufferTime attribute');
      }
    }

    // Check for Period
    if (!content.includes('<Period')) {
      errors.push('MPD has no Period elements');
    }

    // Check for AdaptationSet
    if (!content.includes('<AdaptationSet')) {
      errors.push('MPD has no AdaptationSet elements');
    }

    return {
      valid: errors.length === 0,
      errors,
      warnings,
    };
  }

  /**
   * Generate cache headers based on policy.
   */
  generateCacheHeaders(policy: CacheDirective): Record<string, string> {
    const parts: string[] = [];

    if (policy.private) {
      parts.push('private');
    }

    if (policy.noStore) {
      parts.push('no-store');
    }

    if (policy.noCache) {
      parts.push('no-cache');
    }

    if (policy.maxAge !== undefined) {
      parts.push(`max-age=${policy.maxAge}`);
    }

    if (policy.mustRevalidate) {
      parts.push('must-revalidate');
    }

    return {
      'Cache-Control': parts.join(', ') || 'no-cache',
    };
  }

  /**
   * Get content type for manifest.
   */
  getContentType(protocol: 'hls' | 'dash'): string {
    return protocol === 'hls'
      ? 'application/vnd.apple.mpegurl'
      : 'application/dash+xml';
  }

  /**
   * Sanitize manifest content for security.
   */
  sanitizeManifest(content: string): string {
    // Remove any script tags (shouldn't be present but safety)
    let sanitized = content.replace(/<script[^>]*>[\s\S]*?<\/script>/gi, '');

    // Remove any event handlers
    sanitized = sanitized.replace(/on\w+="[^"]*"/gi, '');

    return sanitized;
  }

  /**
   * Update validation rules.
   */
  setValidationRules(rules: Partial<ManifestValidationRules>): void {
    this.validationRules = { ...this.validationRules, ...rules };
  }

  /**
   * Get validation rules.
   */
  getValidationRules(): ManifestValidationRules {
    return { ...this.validationRules };
  }
}

export default ManifestService;
