/**
 * SSAI Slate Manager
 *
 * Manages fallback slate content for unfilled ad slots.
 * Provides appropriate slate based on format and duration requirements.
 */

import { SlateConfig, SlateMediaFile } from './types';

export interface SlateManagerConfig {
  /** Default slate duration (seconds) */
  defaultDuration: number;

  /** Enable looping for short slates */
  enableLooping: boolean;

  /** Cache TTL for slate URLs (seconds) */
  cacheTtlSeconds: number;
}

const DEFAULT_CONFIG: SlateManagerConfig = {
  defaultDuration: 30,
  enableLooping: true,
  cacheTtlSeconds: 3600,
};

interface SlateSelection {
  slate: SlateConfig;
  mediaFile: SlateMediaFile;
  loopCount: number;
}

export class SlateManager {
  private config: SlateManagerConfig;
  private slates: Map<string, SlateConfig> = new Map();
  private tenantDefaults: Map<string, string> = new Map();

  constructor(config: Partial<SlateManagerConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.initializeDefaultSlates();
  }

  /**
   * Initialize default slate configurations
   */
  private initializeDefaultSlates(): void {
    // Default 30s slate (TS format)
    this.registerSlate({
      id: 'default-ts',
      name: 'Default Slate (TS)',
      isDefault: true,
      duration: 30,
      loopable: true,
      mediaFiles: [
        {
          container: 'ts',
          codec: 'h264',
          resolution: { width: 1920, height: 1080 },
          bitrate: 5000000,
          url: '/slates/default/1080p.ts',
        },
        {
          container: 'ts',
          codec: 'h264',
          resolution: { width: 1280, height: 720 },
          bitrate: 3000000,
          url: '/slates/default/720p.ts',
        },
        {
          container: 'ts',
          codec: 'h264',
          resolution: { width: 854, height: 480 },
          bitrate: 1500000,
          url: '/slates/default/480p.ts',
        },
      ],
    });

    // Default 30s slate (CMAF format)
    this.registerSlate({
      id: 'default-cmaf',
      name: 'Default Slate (CMAF)',
      isDefault: false,
      duration: 30,
      loopable: true,
      mediaFiles: [
        {
          container: 'cmaf',
          codec: 'h264',
          resolution: { width: 1920, height: 1080 },
          bitrate: 5000000,
          url: '/slates/default/1080p.mp4',
        },
        {
          container: 'cmaf',
          codec: 'h264',
          resolution: { width: 1280, height: 720 },
          bitrate: 3000000,
          url: '/slates/default/720p.mp4',
        },
        {
          container: 'cmaf',
          codec: 'h264',
          resolution: { width: 854, height: 480 },
          bitrate: 1500000,
          url: '/slates/default/480p.mp4',
        },
      ],
    });

    // Short 15s slate
    this.registerSlate({
      id: 'short-ts',
      name: 'Short Slate (TS)',
      isDefault: false,
      duration: 15,
      loopable: true,
      mediaFiles: [
        {
          container: 'ts',
          codec: 'h264',
          resolution: { width: 1920, height: 1080 },
          bitrate: 5000000,
          url: '/slates/short/1080p.ts',
        },
      ],
    });
  }

  /**
   * Register a slate configuration
   */
  registerSlate(config: SlateConfig): void {
    this.slates.set(config.id, config);
  }

  /**
   * Set default slate for a tenant
   */
  setTenantDefault(tenantId: string, slateId: string): boolean {
    if (!this.slates.has(slateId)) return false;
    this.tenantDefaults.set(tenantId, slateId);
    return true;
  }

  /**
   * Get best slate for given requirements
   */
  selectSlate(
    requirements: {
      tenantId?: string;
      container: 'ts' | 'cmaf';
      resolution?: { width: number; height: number };
      maxBitrate?: number;
      targetDuration: number;
    }
  ): SlateSelection | null {
    // Find appropriate slate
    let slateId: string | undefined;

    // Check tenant default
    if (requirements.tenantId) {
      slateId = this.tenantDefaults.get(requirements.tenantId);
    }

    // Find slate by container format
    if (!slateId) {
      for (const [id, slate] of this.slates) {
        const hasFormat = slate.mediaFiles.some(
          (mf) => mf.container === requirements.container
        );
        if (hasFormat && slate.isDefault) {
          slateId = id;
          break;
        }
      }
    }

    // Fallback to first slate with matching format
    if (!slateId) {
      for (const [id, slate] of this.slates) {
        const hasFormat = slate.mediaFiles.some(
          (mf) => mf.container === requirements.container
        );
        if (hasFormat) {
          slateId = id;
          break;
        }
      }
    }

    if (!slateId) return null;

    const slate = this.slates.get(slateId)!;

    // Find best media file
    const matchingFiles = slate.mediaFiles.filter(
      (mf) => mf.container === requirements.container
    );

    if (matchingFiles.length === 0) return null;

    // Sort by resolution match
    const scored = matchingFiles.map((mf) => {
      let score = 0;

      // Resolution match
      if (requirements.resolution) {
        const resDiff = Math.abs(
          mf.resolution.width - requirements.resolution.width
        );
        score -= resDiff;
      }

      // Bitrate match
      if (requirements.maxBitrate) {
        if (mf.bitrate <= requirements.maxBitrate) {
          score += 100;
        } else {
          score -= (mf.bitrate - requirements.maxBitrate) / 1000000;
        }
      }

      return { file: mf, score };
    });

    scored.sort((a, b) => b.score - a.score);
    const selectedFile = scored[0].file;

    // Calculate loop count
    let loopCount = 1;
    if (
      this.config.enableLooping &&
      slate.loopable &&
      requirements.targetDuration > slate.duration
    ) {
      loopCount = Math.ceil(requirements.targetDuration / slate.duration);
    }

    return {
      slate,
      mediaFile: selectedFile,
      loopCount,
    };
  }

  /**
   * Get slate segments for a duration
   */
  getSlateSegments(
    slateId: string,
    targetDuration: number,
    segmentDuration: number = 6
  ): Array<{
    index: number;
    startTime: number;
    duration: number;
    url: string;
  }> {
    const slate = this.slates.get(slateId);
    if (!slate || slate.mediaFiles.length === 0) return [];

    const segments: Array<{
      index: number;
      startTime: number;
      duration: number;
      url: string;
    }> = [];

    const baseUrl = slate.mediaFiles[0].url.replace(/\.[^.]+$/, '');
    let currentTime = 0;
    let index = 0;

    while (currentTime < targetDuration) {
      const remaining = targetDuration - currentTime;
      const duration = Math.min(segmentDuration, remaining);

      // Wrap within slate duration if looping
      const slateTime = currentTime % slate.duration;
      const segmentIndex = Math.floor(slateTime / segmentDuration);

      segments.push({
        index,
        startTime: currentTime,
        duration,
        url: `${baseUrl}/segment-${segmentIndex}.ts`,
      });

      currentTime += duration;
      index++;
    }

    return segments;
  }

  /**
   * Get all slates
   */
  getAll(): SlateConfig[] {
    return Array.from(this.slates.values());
  }

  /**
   * Get a specific slate
   */
  get(slateId: string): SlateConfig | null {
    return this.slates.get(slateId) || null;
  }

  /**
   * Delete a slate
   */
  delete(slateId: string): boolean {
    // Remove from tenant defaults
    for (const [tenantId, defaultId] of this.tenantDefaults) {
      if (defaultId === slateId) {
        this.tenantDefaults.delete(tenantId);
      }
    }
    return this.slates.delete(slateId);
  }

  /**
   * Clear all slates
   */
  clear(): void {
    this.slates.clear();
    this.tenantDefaults.clear();
    this.initializeDefaultSlates();
  }
}
