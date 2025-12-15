/**
 * SSAI E2E Test Variant Types
 * 
 * Type definitions for all 8 protocol/container test variants
 */

export type Protocol = 'hls' | 'dash';
export type StreamType = 'live' | 'vod';
export type Container = 'ts' | 'cmaf';

export interface TestVariant {
  protocol: Protocol;
  streamType: StreamType;
  container: Container;
  name: string;
  description: string;
}

export interface TestConfig {
  variant: TestVariant;
  manifestUrl: string;
  expectedSegmentExtension: string;
  expectedMimeType: string;
  supportsDiscontinuity: boolean;
  supportsSCTE35: boolean;
}

export interface TestResult {
  variant: TestVariant;
  passed: boolean;
  duration: number;
  assertions: AssertionResult[];
  errors: string[];
}

export interface AssertionResult {
  name: string;
  passed: boolean;
  message: string;
  actual?: unknown;
  expected?: unknown;
}

export interface ManifestTestCase {
  name: string;
  manifest: string;
  expectedBreaks: number;
  expectedAds: number;
  expectedDuration: number;
}

export interface SegmentTestCase {
  name: string;
  segmentUrl: string;
  expectedDuration: number;
  expectedKeyframe: boolean;
  expectedCodec: string;
}

export interface TrackingTestCase {
  name: string;
  events: string[];
  expectedPixelFires: number;
  expectedQuartiles: string[];
}

// All 8 test variants
export const TEST_VARIANTS: TestVariant[] = [
  {
    protocol: 'hls',
    streamType: 'live',
    container: 'ts',
    name: 'HLS-Live-TS',
    description: 'HLS live stream with TS segments'
  },
  {
    protocol: 'hls',
    streamType: 'live',
    container: 'cmaf',
    name: 'HLS-Live-CMAF',
    description: 'HLS live stream with CMAF/fMP4 segments'
  },
  {
    protocol: 'hls',
    streamType: 'vod',
    container: 'ts',
    name: 'HLS-VOD-TS',
    description: 'HLS VOD stream with TS segments'
  },
  {
    protocol: 'hls',
    streamType: 'vod',
    container: 'cmaf',
    name: 'HLS-VOD-CMAF',
    description: 'HLS VOD stream with CMAF/fMP4 segments'
  },
  {
    protocol: 'dash',
    streamType: 'live',
    container: 'ts',
    name: 'DASH-Live-TS',
    description: 'DASH live stream with TS segments'
  },
  {
    protocol: 'dash',
    streamType: 'live',
    container: 'cmaf',
    name: 'DASH-Live-CMAF',
    description: 'DASH live stream with CMAF/fMP4 segments'
  },
  {
    protocol: 'dash',
    streamType: 'vod',
    container: 'ts',
    name: 'DASH-VOD-TS',
    description: 'DASH VOD stream with TS segments'
  },
  {
    protocol: 'dash',
    streamType: 'vod',
    container: 'cmaf',
    name: 'DASH-VOD-CMAF',
    description: 'DASH VOD stream with CMAF/fMP4 segments'
  }
];

export function getVariantConfig(variant: TestVariant): TestConfig {
  const isHLS = variant.protocol === 'hls';
  const isTS = variant.container === 'ts';
  
  return {
    variant,
    manifestUrl: isHLS 
      ? `/test/streams/${variant.name.toLowerCase()}/master.m3u8`
      : `/test/streams/${variant.name.toLowerCase()}/manifest.mpd`,
    expectedSegmentExtension: isTS ? '.ts' : '.m4s',
    expectedMimeType: isTS 
      ? 'video/mp2t'
      : 'video/mp4',
    supportsDiscontinuity: isHLS,
    supportsSCTE35: true
  };
}
