/**
 * Caption Handler
 *
 * Manages caption continuity across ad breaks, ensuring content captions
 * resume properly and ad captions are handled safely.
 */

export type CaptionFormat = 'cea-608' | 'cea-708' | 'webvtt' | 'ttml' | 'srt';

export interface CaptionTrack {
  id: string;
  language: string;
  format: CaptionFormat;
  uri?: string;
  isDefault: boolean;
  isForced: boolean;
  characteristics?: string[];
}

export interface CaptionCue {
  id: string;
  startTime: number;
  endTime: number;
  text: string;
  position?: {
    line?: number;
    column?: number;
  };
  style?: CaptionStyle;
}

export interface CaptionStyle {
  color?: string;
  backgroundColor?: string;
  fontSize?: string;
  fontFamily?: string;
  fontStyle?: 'normal' | 'italic';
  fontWeight?: 'normal' | 'bold';
}

export interface CaptionConfig {
  /** Preserve content captions during ad breaks */
  preserveContentCaptions: boolean;

  /** Include ad captions if available */
  includeAdCaptions: boolean;

  /** Clear captions during ad break */
  clearDuringAds: boolean;

  /** Minimum caption display duration (seconds) */
  minDisplayDuration: number;

  /** Maximum caption display duration (seconds) */
  maxDisplayDuration: number;

  /** Buffer time for caption continuity (seconds) */
  continuityBuffer: number;
}

export interface CaptionState {
  activeCue?: CaptionCue;
  pendingCues: CaptionCue[];
  lastContentCue?: CaptionCue;
  inAdBreak: boolean;
  resumePosition?: number;
}

export interface CaptionHandlerResult {
  outputCues: CaptionCue[];
  warnings: string[];
  errors: string[];
  state: CaptionState;
}

/**
 * Caption Handler for SSAI caption continuity.
 */
export class CaptionHandler {
  private config: CaptionConfig;
  private state: CaptionState;

  constructor(config?: Partial<CaptionConfig>) {
    this.config = {
      preserveContentCaptions: true,
      includeAdCaptions: true,
      clearDuringAds: false,
      minDisplayDuration: 1,
      maxDisplayDuration: 7,
      continuityBuffer: 0.5,
      ...config,
    };

    this.state = {
      pendingCues: [],
      inAdBreak: false,
    };
  }

  /**
   * Process captions for content segment.
   */
  processContentCaptions(cues: CaptionCue[]): CaptionHandlerResult {
    const result: CaptionHandlerResult = {
      outputCues: [],
      warnings: [],
      errors: [],
      state: this.state,
    };

    for (const cue of cues) {
      // Validate cue
      const validation = this.validateCue(cue);
      if (validation.errors.length > 0) {
        result.errors.push(...validation.errors);
        continue;
      }
      result.warnings.push(...validation.warnings);

      // Adjust duration if needed
      const adjustedCue = this.adjustCueDuration(cue);
      result.outputCues.push(adjustedCue);

      // Track last content cue
      this.state.lastContentCue = adjustedCue;
    }

    this.state.activeCue = result.outputCues[result.outputCues.length - 1];
    result.state = this.state;

    return result;
  }

  /**
   * Handle captions during ad break entry.
   */
  enterAdBreak(adStartTime: number): CaptionHandlerResult {
    const result: CaptionHandlerResult = {
      outputCues: [],
      warnings: [],
      errors: [],
      state: this.state,
    };

    this.state.inAdBreak = true;
    this.state.resumePosition = adStartTime;

    // Handle active cue
    if (this.state.activeCue) {
      if (this.config.preserveContentCaptions) {
        // Truncate active cue at ad start
        if (this.state.activeCue.endTime > adStartTime) {
          const truncatedCue = {
            ...this.state.activeCue,
            endTime: adStartTime,
          };
          result.outputCues.push(truncatedCue);
          result.warnings.push(
            `Truncated caption at ad break: "${this.state.activeCue.text.substring(0, 30)}..."`
          );
        }
      }

      this.state.lastContentCue = this.state.activeCue;
    }

    result.state = this.state;
    return result;
  }

  /**
   * Process captions during ad.
   */
  processAdCaptions(cues: CaptionCue[]): CaptionHandlerResult {
    const result: CaptionHandlerResult = {
      outputCues: [],
      warnings: [],
      errors: [],
      state: this.state,
    };

    if (!this.config.includeAdCaptions) {
      result.state = this.state;
      return result;
    }

    for (const cue of cues) {
      // Validate cue
      const validation = this.validateCue(cue);
      if (validation.errors.length > 0) {
        result.errors.push(...validation.errors);
        continue;
      }

      const adjustedCue = this.adjustCueDuration(cue);
      result.outputCues.push(adjustedCue);
    }

    result.state = this.state;
    return result;
  }

  /**
   * Handle captions during ad break exit.
   */
  exitAdBreak(adEndTime: number): CaptionHandlerResult {
    const result: CaptionHandlerResult = {
      outputCues: [],
      warnings: [],
      errors: [],
      state: this.state,
    };

    this.state.inAdBreak = false;

    // Resume any pending content captions
    if (this.state.pendingCues.length > 0) {
      for (const cue of this.state.pendingCues) {
        // Adjust timing to resume after ad
        const offset = adEndTime - (this.state.resumePosition || 0);
        const resumedCue: CaptionCue = {
          ...cue,
          startTime: cue.startTime + offset,
          endTime: cue.endTime + offset,
        };
        result.outputCues.push(resumedCue);
      }
      this.state.pendingCues = [];
    }

    result.state = this.state;
    return result;
  }

  /**
   * Validate caption cue.
   */
  private validateCue(cue: CaptionCue): {
    valid: boolean;
    warnings: string[];
    errors: string[];
  } {
    const warnings: string[] = [];
    const errors: string[] = [];

    // Check timing
    if (cue.endTime <= cue.startTime) {
      errors.push(`Invalid cue timing: end (${cue.endTime}) <= start (${cue.startTime})`);
    }

    // Check duration
    const duration = cue.endTime - cue.startTime;
    if (duration < this.config.minDisplayDuration) {
      warnings.push(`Cue duration ${duration.toFixed(2)}s is very short`);
    }

    // Check for empty text
    if (!cue.text || cue.text.trim().length === 0) {
      warnings.push('Empty caption cue');
    }

    // Check for potentially crash-inducing content
    if (this.mightCrashPlayer(cue)) {
      errors.push('Caption contains potentially problematic content');
    }

    return {
      valid: errors.length === 0,
      warnings,
      errors,
    };
  }

  /**
   * Check if cue might crash player.
   */
  private mightCrashPlayer(cue: CaptionCue): boolean {
    // Check for excessively long text
    if (cue.text.length > 10000) {
      return true;
    }

    // Check for null bytes
    if (cue.text.includes('\0')) {
      return true;
    }

    // Check for extreme timing
    if (cue.startTime < 0 || cue.endTime > 86400) {
      return true;
    }

    return false;
  }

  /**
   * Adjust cue duration to be within limits.
   */
  private adjustCueDuration(cue: CaptionCue): CaptionCue {
    const duration = cue.endTime - cue.startTime;

    if (duration > this.config.maxDisplayDuration) {
      return {
        ...cue,
        endTime: cue.startTime + this.config.maxDisplayDuration,
      };
    }

    return cue;
  }

  /**
   * Merge overlapping caption tracks.
   */
  mergeTracks(
    contentTrack: CaptionTrack,
    adTrack: CaptionTrack
  ): CaptionTrack | null {
    // Can only merge same format
    if (contentTrack.format !== adTrack.format) {
      return null;
    }

    // Can only merge same language
    if (contentTrack.language !== adTrack.language) {
      return null;
    }

    return {
      ...contentTrack,
      id: `merged-${contentTrack.id}-${adTrack.id}`,
    };
  }

  /**
   * Get current state.
   */
  getState(): CaptionState {
    return { ...this.state };
  }

  /**
   * Reset state.
   */
  reset(): void {
    this.state = {
      pendingCues: [],
      inAdBreak: false,
    };
  }

  /**
   * Get configuration.
   */
  getConfig(): CaptionConfig {
    return { ...this.config };
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<CaptionConfig>): void {
    this.config = { ...this.config, ...config };
  }
}

export default CaptionHandler;
