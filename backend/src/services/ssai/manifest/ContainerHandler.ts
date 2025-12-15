/**
 * SSAI Container Handler
 *
 * Handles TS (.ts) and CMAF/fMP4 (.m4s) segment containers.
 * Enforces correct MIME types and file conventions.
 */

export interface ContainerInfo {
  type: 'ts' | 'cmaf';
  extension: string;
  mimeType: string;
  videoCodec?: string;
  audioCodec?: string;
  duration?: number;
  byteSize?: number;
  hasKeyframe?: boolean;
  initSegmentRequired?: boolean;
}

export interface SegmentMetadata {
  uri: string;
  container: ContainerInfo;
  startTime: number;
  duration: number;
  discontinuity: boolean;
  isAd: boolean;
  creativeId?: string;
}

export interface ContainerCompatibility {
  compatible: boolean;
  reason?: string;
  recommendations?: string[];
}

export interface InitSegment {
  uri: string;
  byteRange?: {
    offset: number;
    length: number;
  };
}

/**
 * Container Handler for SSAI segment management.
 *
 * Responsibilities:
 * - Detect container type from URI or content
 * - Validate container compatibility between content and ads
 * - Enforce correct MIME types for CDN/player compatibility
 * - Handle init segment requirements for fMP4
 */
export class ContainerHandler {
  private static readonly TS_EXTENSIONS = ['.ts', '.m2ts', '.mts'];
  private static readonly CMAF_EXTENSIONS = ['.m4s', '.mp4', '.m4a', '.m4v'];
  private static readonly INIT_EXTENSIONS = ['.mp4', '.m4s'];

  // Standard MIME types per HLS/DASH specs
  private static readonly MIME_TYPES = {
    ts: {
      video: 'video/MP2T',
      audio: 'audio/MP2T',
    },
    cmaf: {
      video: 'video/mp4',
      audio: 'audio/mp4',
      init: 'video/mp4',
    },
    manifest: {
      hls: 'application/vnd.apple.mpegurl',
      dash: 'application/dash+xml',
    },
  };

  // Common codec strings
  private static readonly CODEC_PATTERNS = {
    h264: /avc[1-4]\.[0-9A-Fa-f]+/,
    h265: /hvc1\.[0-9A-Za-z.]+|hev1\.[0-9A-Za-z.]+/,
    aac: /mp4a\.40\.[0-9]+/,
    ac3: /ac-3/,
    eac3: /ec-3/,
  };

  /**
   * Detect container type from segment URI.
   */
  detectContainerType(uri: string): 'ts' | 'cmaf' | 'unknown' {
    const lowerUri = uri.toLowerCase();
    const extension = this.extractExtension(lowerUri);

    if (ContainerHandler.TS_EXTENSIONS.includes(extension)) {
      return 'ts';
    }

    if (ContainerHandler.CMAF_EXTENSIONS.includes(extension)) {
      return 'cmaf';
    }

    return 'unknown';
  }

  /**
   * Get container info from URI.
   */
  getContainerInfo(uri: string): ContainerInfo {
    const type = this.detectContainerType(uri);
    const extension = this.extractExtension(uri);

    if (type === 'ts') {
      return {
        type: 'ts',
        extension,
        mimeType: ContainerHandler.MIME_TYPES.ts.video,
        initSegmentRequired: false,
      };
    }

    if (type === 'cmaf') {
      return {
        type: 'cmaf',
        extension,
        mimeType: ContainerHandler.MIME_TYPES.cmaf.video,
        initSegmentRequired: true,
      };
    }

    // Unknown container - return best guess
    return {
      type: 'ts',
      extension: extension || '.ts',
      mimeType: ContainerHandler.MIME_TYPES.ts.video,
      initSegmentRequired: false,
    };
  }

  /**
   * Check compatibility between content and ad containers.
   */
  checkCompatibility(
    contentContainer: ContainerInfo,
    adContainer: ContainerInfo
  ): ContainerCompatibility {
    // Container type must match
    if (contentContainer.type !== adContainer.type) {
      return {
        compatible: false,
        reason: `Container type mismatch: content uses ${contentContainer.type}, ad uses ${adContainer.type}`,
        recommendations: [
          `Transcode ad to ${contentContainer.type} format`,
          `Use separate ad pod for ${adContainer.type} format`,
        ],
      };
    }

    // For CMAF, both must have or not have init segments
    if (contentContainer.type === 'cmaf') {
      if (
        contentContainer.initSegmentRequired !== adContainer.initSegmentRequired
      ) {
        return {
          compatible: false,
          reason: 'Init segment requirement mismatch',
          recommendations: [
            'Ensure both content and ad have consistent init segment handling',
          ],
        };
      }
    }

    // Check codec compatibility if available
    if (contentContainer.videoCodec && adContainer.videoCodec) {
      const contentCodecFamily = this.getCodecFamily(contentContainer.videoCodec);
      const adCodecFamily = this.getCodecFamily(adContainer.videoCodec);

      if (contentCodecFamily !== adCodecFamily) {
        return {
          compatible: false,
          reason: `Video codec family mismatch: content uses ${contentCodecFamily}, ad uses ${adCodecFamily}`,
          recommendations: [
            `Transcode ad to ${contentCodecFamily}`,
            'Use codec-specific ad variant',
          ],
        };
      }
    }

    // Check audio codec compatibility if available
    if (contentContainer.audioCodec && adContainer.audioCodec) {
      const contentAudioFamily = this.getAudioCodecFamily(
        contentContainer.audioCodec
      );
      const adAudioFamily = this.getAudioCodecFamily(adContainer.audioCodec);

      if (contentAudioFamily !== adAudioFamily) {
        return {
          compatible: false,
          reason: `Audio codec family mismatch: content uses ${contentAudioFamily}, ad uses ${adAudioFamily}`,
          recommendations: [
            `Transcode ad audio to ${contentAudioFamily}`,
            'Use audio codec-specific ad variant',
          ],
        };
      }
    }

    return { compatible: true };
  }

  /**
   * Get correct MIME type for container.
   */
  getMimeType(
    container: ContainerInfo,
    mediaType: 'video' | 'audio' | 'init' = 'video'
  ): string {
    if (container.type === 'ts') {
      return mediaType === 'video'
        ? ContainerHandler.MIME_TYPES.ts.video
        : ContainerHandler.MIME_TYPES.ts.audio;
    }

    if (container.type === 'cmaf') {
      if (mediaType === 'init') {
        return ContainerHandler.MIME_TYPES.cmaf.init;
      }
      return mediaType === 'video'
        ? ContainerHandler.MIME_TYPES.cmaf.video
        : ContainerHandler.MIME_TYPES.cmaf.audio;
    }

    // Default to TS
    return ContainerHandler.MIME_TYPES.ts.video;
  }

  /**
   * Validate segment metadata for SSAI requirements.
   */
  validateSegment(segment: SegmentMetadata): {
    valid: boolean;
    errors: string[];
  } {
    const errors: string[] = [];

    // Duration must be positive
    if (segment.duration <= 0) {
      errors.push(`Invalid segment duration: ${segment.duration}`);
    }

    // Start time must be non-negative
    if (segment.startTime < 0) {
      errors.push(`Invalid segment start time: ${segment.startTime}`);
    }

    // URI must be valid
    if (!segment.uri || segment.uri.trim() === '') {
      errors.push('Segment URI is required');
    }

    // Ad segments must have creative ID
    if (segment.isAd && !segment.creativeId) {
      errors.push('Ad segment must have creativeId');
    }

    // Container must be valid
    if (segment.container.type !== 'ts' && segment.container.type !== 'cmaf') {
      errors.push(`Unknown container type: ${segment.container.type}`);
    }

    return {
      valid: errors.length === 0,
      errors,
    };
  }

  /**
   * Generate cache-safe segment URI with proper extension.
   */
  normalizeSegmentUri(uri: string, containerType: 'ts' | 'cmaf'): string {
    const currentExtension = this.extractExtension(uri);
    const expectedExtension = containerType === 'ts' ? '.ts' : '.m4s';

    // If extension matches, return as-is
    if (currentExtension === expectedExtension) {
      return uri;
    }

    // If no extension, append expected
    if (!currentExtension) {
      return uri + expectedExtension;
    }

    // Extension mismatch - this is a configuration issue
    console.warn(
      `Container extension mismatch: ${uri} has ${currentExtension}, expected ${expectedExtension}`
    );
    return uri;
  }

  /**
   * Build init segment reference for CMAF.
   */
  buildInitSegmentReference(
    baseUri: string,
    initFileName?: string
  ): InitSegment | null {
    const container = this.getContainerInfo(baseUri);

    // TS doesn't need init segments
    if (container.type === 'ts') {
      return null;
    }

    // Determine init segment URI
    let initUri: string;
    if (initFileName) {
      // Use explicit init filename
      const baseDir = baseUri.substring(0, baseUri.lastIndexOf('/') + 1);
      initUri = baseDir + initFileName;
    } else {
      // Try common init patterns
      const baseName = this.extractBaseName(baseUri);
      const baseDir = baseUri.substring(0, baseUri.lastIndexOf('/') + 1);
      initUri = baseDir + baseName + '_init.mp4';
    }

    return { uri: initUri };
  }

  /**
   * Parse codec string from HLS/DASH manifest.
   */
  parseCodecs(codecsString: string): {
    videoCodec?: string;
    audioCodec?: string;
  } {
    if (!codecsString) {
      return {};
    }

    const codecs = codecsString.split(',').map((c) => c.trim());
    let videoCodec: string | undefined;
    let audioCodec: string | undefined;

    for (const codec of codecs) {
      // Check video codecs
      if (
        ContainerHandler.CODEC_PATTERNS.h264.test(codec) ||
        ContainerHandler.CODEC_PATTERNS.h265.test(codec)
      ) {
        videoCodec = codec;
        continue;
      }

      // Check audio codecs
      if (
        ContainerHandler.CODEC_PATTERNS.aac.test(codec) ||
        ContainerHandler.CODEC_PATTERNS.ac3.test(codec) ||
        ContainerHandler.CODEC_PATTERNS.eac3.test(codec)
      ) {
        audioCodec = codec;
      }
    }

    return { videoCodec, audioCodec };
  }

  /**
   * Get codec family from codec string.
   */
  private getCodecFamily(codec: string): string {
    if (ContainerHandler.CODEC_PATTERNS.h264.test(codec)) {
      return 'h264';
    }
    if (ContainerHandler.CODEC_PATTERNS.h265.test(codec)) {
      return 'h265';
    }
    return 'unknown';
  }

  /**
   * Get audio codec family from codec string.
   */
  private getAudioCodecFamily(codec: string): string {
    if (ContainerHandler.CODEC_PATTERNS.aac.test(codec)) {
      return 'aac';
    }
    if (ContainerHandler.CODEC_PATTERNS.ac3.test(codec)) {
      return 'ac3';
    }
    if (ContainerHandler.CODEC_PATTERNS.eac3.test(codec)) {
      return 'eac3';
    }
    return 'unknown';
  }

  /**
   * Extract file extension from URI.
   */
  private extractExtension(uri: string): string {
    // Remove query string and fragment
    const cleanUri = uri.split('?')[0].split('#')[0];
    const lastDotIndex = cleanUri.lastIndexOf('.');
    const lastSlashIndex = cleanUri.lastIndexOf('/');

    if (lastDotIndex > lastSlashIndex && lastDotIndex < cleanUri.length - 1) {
      return cleanUri.substring(lastDotIndex).toLowerCase();
    }

    return '';
  }

  /**
   * Extract base filename without extension.
   */
  private extractBaseName(uri: string): string {
    const cleanUri = uri.split('?')[0].split('#')[0];
    const lastSlashIndex = cleanUri.lastIndexOf('/');
    const filename = cleanUri.substring(lastSlashIndex + 1);
    const lastDotIndex = filename.lastIndexOf('.');

    if (lastDotIndex > 0) {
      return filename.substring(0, lastDotIndex);
    }

    return filename;
  }
}

export default ContainerHandler;
