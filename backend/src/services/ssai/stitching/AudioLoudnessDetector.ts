/**
 * Audio Loudness Detector
 *
 * Detects and flags extreme audio loudness differences between
 * content and ads to prevent jarring volume changes.
 */

export interface LoudnessInfo {
  /** Integrated loudness (LUFS) */
  integratedLufs: number;

  /** Loudness range (LU) */
  loudnessRange: number;

  /** True peak (dBTP) */
  truePeak: number;

  /** Short-term loudness (LUFS) */
  shortTermLufs?: number;

  /** Momentary loudness (LUFS) */
  momentaryLufs?: number;
}

export interface LoudnessConfig {
  /** Target loudness (LUFS) */
  targetLufs: number;

  /** Maximum allowed loudness deviation (LU) */
  maxDeviation: number;

  /** Maximum true peak (dBTP) */
  maxTruePeak: number;

  /** Warn threshold (LU above target) */
  warnThreshold: number;

  /** Flag threshold (LU above target) */
  flagThreshold: number;

  /** Enable automatic normalization recommendation */
  enableNormalization: boolean;
}

export interface LoudnessAnalysis {
  source: 'content' | 'ad';
  loudness: LoudnessInfo;
  compliant: boolean;
  deviation: number;
  warnings: string[];
  errors: string[];
  normalizationGain?: number;
}

export interface LoudnessCompareResult {
  contentLoudness: LoudnessInfo;
  adLoudness: LoudnessInfo;
  difference: number;
  jarring: boolean;
  recommendation?: string;
}

/**
 * Audio Loudness Detector for SSAI quality control.
 */
export class AudioLoudnessDetector {
  private config: LoudnessConfig;

  constructor(config?: Partial<LoudnessConfig>) {
    this.config = {
      targetLufs: -24, // EBU R128 broadcast standard
      maxDeviation: 2,
      maxTruePeak: -1, // dBTP
      warnThreshold: 3,
      flagThreshold: 6,
      enableNormalization: true,
      ...config,
    };
  }

  /**
   * Analyze loudness of audio.
   */
  analyzeLoudness(
    loudness: LoudnessInfo,
    source: 'content' | 'ad'
  ): LoudnessAnalysis {
    const result: LoudnessAnalysis = {
      source,
      loudness,
      compliant: true,
      deviation: loudness.integratedLufs - this.config.targetLufs,
      warnings: [],
      errors: [],
    };

    const absDeviation = Math.abs(result.deviation);

    // Check against target
    if (absDeviation > this.config.maxDeviation) {
      result.compliant = false;

      if (absDeviation > this.config.flagThreshold) {
        result.errors.push(
          `Extreme loudness deviation: ${result.deviation.toFixed(1)} LU from target`
        );
      } else if (absDeviation > this.config.warnThreshold) {
        result.warnings.push(
          `Loudness deviation: ${result.deviation.toFixed(1)} LU from target`
        );
      }
    }

    // Check true peak
    if (loudness.truePeak > this.config.maxTruePeak) {
      result.warnings.push(
        `True peak ${loudness.truePeak.toFixed(1)} dBTP exceeds ${this.config.maxTruePeak} dBTP`
      );
    }

    // Calculate normalization gain if needed
    if (this.config.enableNormalization && !result.compliant) {
      result.normalizationGain = this.config.targetLufs - loudness.integratedLufs;
    }

    return result;
  }

  /**
   * Compare loudness between content and ad.
   */
  compareLoudness(
    contentLoudness: LoudnessInfo,
    adLoudness: LoudnessInfo
  ): LoudnessCompareResult {
    const difference = adLoudness.integratedLufs - contentLoudness.integratedLufs;
    const absDifference = Math.abs(difference);

    const result: LoudnessCompareResult = {
      contentLoudness,
      adLoudness,
      difference,
      jarring: absDifference > this.config.flagThreshold,
    };

    if (result.jarring) {
      if (difference > 0) {
        result.recommendation = `Ad is ${absDifference.toFixed(1)} LU louder than content. Reduce ad gain by ${absDifference.toFixed(1)} dB.`;
      } else {
        result.recommendation = `Ad is ${absDifference.toFixed(1)} LU quieter than content. Increase ad gain by ${absDifference.toFixed(1)} dB.`;
      }
    }

    return result;
  }

  /**
   * Detect extreme loudness that may cause issues.
   */
  detectExtremeLoudness(loudness: LoudnessInfo): {
    isExtreme: boolean;
    type?: 'too-loud' | 'too-quiet';
    severity: 'none' | 'warning' | 'error';
  } {
    const deviation = loudness.integratedLufs - this.config.targetLufs;
    const absDeviation = Math.abs(deviation);

    if (absDeviation <= this.config.warnThreshold) {
      return { isExtreme: false, severity: 'none' };
    }

    const type = deviation > 0 ? 'too-loud' : 'too-quiet';
    const severity = absDeviation > this.config.flagThreshold ? 'error' : 'warning';

    return { isExtreme: true, type, severity };
  }

  /**
   * Calculate normalization gain to reach target.
   */
  calculateNormalizationGain(currentLufs: number): number {
    return this.config.targetLufs - currentLufs;
  }

  /**
   * Validate loudness for broadcast compliance.
   */
  validateBroadcastCompliance(loudness: LoudnessInfo): {
    compliant: boolean;
    issues: string[];
  } {
    const issues: string[] = [];

    // EBU R128 compliance checks
    // Integrated loudness should be -24 LUFS ±1
    const integratedDeviation = Math.abs(loudness.integratedLufs - (-24));
    if (integratedDeviation > 1) {
      issues.push(
        `Integrated loudness ${loudness.integratedLufs.toFixed(1)} LUFS not within -24±1 LUFS`
      );
    }

    // True peak should not exceed -1 dBTP
    if (loudness.truePeak > -1) {
      issues.push(
        `True peak ${loudness.truePeak.toFixed(1)} dBTP exceeds -1 dBTP limit`
      );
    }

    // Loudness range recommendation (not a hard limit)
    if (loudness.loudnessRange > 20) {
      issues.push(
        `Loudness range ${loudness.loudnessRange.toFixed(1)} LU is very high`
      );
    }

    return {
      compliant: issues.length === 0,
      issues,
    };
  }

  /**
   * Get configuration.
   */
  getConfig(): LoudnessConfig {
    return { ...this.config };
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<LoudnessConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Set target loudness standard.
   */
  setStandard(standard: 'ebu-r128' | 'atsc-a85' | 'arib-tr-b32'): void {
    switch (standard) {
      case 'ebu-r128':
        this.config.targetLufs = -24;
        this.config.maxTruePeak = -1;
        break;
      case 'atsc-a85':
        this.config.targetLufs = -24;
        this.config.maxTruePeak = -2;
        break;
      case 'arib-tr-b32':
        this.config.targetLufs = -24;
        this.config.maxTruePeak = -1;
        break;
    }
  }
}

export default AudioLoudnessDetector;
