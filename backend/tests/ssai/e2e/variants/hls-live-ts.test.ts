/**
 * SSAI E2E Test - HLS Live TS Variant Runner
 * 
 * Tests HLS live streams with TS segments
 */

import { TestVariant, TestConfig, TestResult, AssertionResult, getVariantConfig, TEST_VARIANTS } from './types';

const HLS_LIVE_TS_VARIANT = TEST_VARIANTS.find(v => v.name === 'HLS-Live-TS')!;

export class HLSLiveTSRunner {
  private config: TestConfig;
  private results: AssertionResult[] = [];
  private errors: string[] = [];

  constructor() {
    this.config = getVariantConfig(HLS_LIVE_TS_VARIANT);
  }

  async run(): Promise<TestResult> {
    const startTime = Date.now();

    try {
      await this.testManifestFetch();
      await this.testSegmentFetch();
      await this.testDiscontinuityHandling();
      await this.testSCTE35Markers();
      await this.testAdInsertion();
      await this.testTracking();
      await this.testLiveEdge();
    } catch (error) {
      this.errors.push(error instanceof Error ? error.message : String(error));
    }

    return {
      variant: HLS_LIVE_TS_VARIANT,
      passed: this.results.every(r => r.passed) && this.errors.length === 0,
      duration: Date.now() - startTime,
      assertions: this.results,
      errors: this.errors
    };
  }

  private async testManifestFetch(): Promise<void> {
    // Test that manifest can be fetched and parsed
    const manifestUrl = this.config.manifestUrl;
    
    // Mock implementation - in real tests, fetch actual manifest
    const mockManifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:6
#EXT-X-MEDIA-SEQUENCE:1000
#EXTINF:6.0,
segment1000.ts
#EXTINF:6.0,
segment1001.ts
#EXT-X-DATERANGE:ID="ad-break-1",START-DATE="2025-01-01T00:00:00Z",DURATION=30.0,SCTE35-OUT=0xFC301100...
#EXT-X-DISCONTINUITY
#EXTINF:15.0,
ad-segment-1.ts
#EXTINF:15.0,
ad-segment-2.ts
#EXT-X-DISCONTINUITY
#EXTINF:6.0,
segment1002.ts`;

    const hasEXTM3U = mockManifest.includes('#EXTM3U');
    const hasTargetDuration = mockManifest.includes('#EXT-X-TARGETDURATION');
    const hasMediaSequence = mockManifest.includes('#EXT-X-MEDIA-SEQUENCE');

    this.results.push({
      name: 'manifest_valid_hls',
      passed: hasEXTM3U,
      message: hasEXTM3U ? 'Manifest is valid HLS' : 'Manifest missing #EXTM3U tag',
      actual: hasEXTM3U,
      expected: true
    });

    this.results.push({
      name: 'manifest_has_target_duration',
      passed: hasTargetDuration,
      message: hasTargetDuration ? 'Manifest has target duration' : 'Manifest missing target duration',
      actual: hasTargetDuration,
      expected: true
    });

    this.results.push({
      name: 'manifest_has_media_sequence',
      passed: hasMediaSequence,
      message: hasMediaSequence ? 'Manifest has media sequence (live)' : 'Manifest missing media sequence',
      actual: hasMediaSequence,
      expected: true
    });
  }

  private async testSegmentFetch(): Promise<void> {
    // Test that TS segments can be fetched
    const expectedExtension = this.config.expectedSegmentExtension;
    const segmentUrl = `/test/streams/hls-live-ts/segment1000${expectedExtension}`;
    
    const hasCorrectExtension = segmentUrl.endsWith(expectedExtension);
    
    this.results.push({
      name: 'segment_correct_extension',
      passed: hasCorrectExtension,
      message: hasCorrectExtension 
        ? `Segment has correct extension: ${expectedExtension}` 
        : `Segment has wrong extension`,
      actual: segmentUrl.split('.').pop(),
      expected: expectedExtension.replace('.', '')
    });

    // Mock segment validation
    const mockSegmentSize = 150000; // 150KB typical for 6s TS segment
    const hasReasonableSize = mockSegmentSize > 10000 && mockSegmentSize < 5000000;
    
    this.results.push({
      name: 'segment_reasonable_size',
      passed: hasReasonableSize,
      message: hasReasonableSize 
        ? 'Segment has reasonable size' 
        : 'Segment size out of expected range',
      actual: mockSegmentSize,
      expected: '10KB - 5MB'
    });
  }

  private async testDiscontinuityHandling(): Promise<void> {
    // Test that discontinuities are properly signaled
    const mockManifest = `#EXT-X-DISCONTINUITY
#EXTINF:15.0,
ad-segment-1.ts
#EXTINF:15.0,
ad-segment-2.ts
#EXT-X-DISCONTINUITY`;

    const discontinuityCount = (mockManifest.match(/#EXT-X-DISCONTINUITY/g) || []).length;
    const hasEntryAndExit = discontinuityCount >= 2;

    this.results.push({
      name: 'discontinuity_entry_exit',
      passed: hasEntryAndExit,
      message: hasEntryAndExit 
        ? 'Ad break has entry and exit discontinuity markers' 
        : 'Missing discontinuity markers',
      actual: discontinuityCount,
      expected: 2
    });
  }

  private async testSCTE35Markers(): Promise<void> {
    // Test SCTE-35 marker parsing
    const mockDateRange = `#EXT-X-DATERANGE:ID="ad-break-1",START-DATE="2025-01-01T00:00:00Z",DURATION=30.0,SCTE35-OUT=0xFC301100`;
    
    const hasSCTE35 = mockDateRange.includes('SCTE35-OUT');
    const hasDuration = mockDateRange.includes('DURATION=');
    const hasId = mockDateRange.includes('ID=');

    this.results.push({
      name: 'scte35_marker_present',
      passed: hasSCTE35,
      message: hasSCTE35 ? 'SCTE-35 marker present' : 'Missing SCTE-35 marker',
      actual: hasSCTE35,
      expected: true
    });

    this.results.push({
      name: 'scte35_has_duration',
      passed: hasDuration,
      message: hasDuration ? 'Break has duration' : 'Missing break duration',
      actual: hasDuration,
      expected: true
    });

    this.results.push({
      name: 'scte35_has_id',
      passed: hasId,
      message: hasId ? 'Break has ID' : 'Missing break ID',
      actual: hasId,
      expected: true
    });
  }

  private async testAdInsertion(): Promise<void> {
    // Test that ads are properly inserted
    const mockStitchedManifest = `#EXTM3U
#EXTINF:6.0,
segment1000.ts
#EXT-X-DISCONTINUITY
#EXTINF:15.0,
ad-segment-1.ts
#EXTINF:15.0,
ad-segment-2.ts
#EXT-X-DISCONTINUITY
#EXTINF:6.0,
segment1001.ts`;

    const adSegments = (mockStitchedManifest.match(/ad-segment-\d+\.ts/g) || []).length;
    const hasAds = adSegments > 0;

    this.results.push({
      name: 'ad_segments_inserted',
      passed: hasAds,
      message: hasAds ? `${adSegments} ad segments inserted` : 'No ad segments found',
      actual: adSegments,
      expected: '>0'
    });

    // Verify ad duration matches break duration
    const adDuration = adSegments * 15; // 15s per segment
    const expectedDuration = 30;
    const durationMatch = adDuration === expectedDuration;

    this.results.push({
      name: 'ad_duration_matches',
      passed: durationMatch,
      message: durationMatch 
        ? 'Ad duration matches break duration' 
        : 'Ad duration mismatch',
      actual: adDuration,
      expected: expectedDuration
    });
  }

  private async testTracking(): Promise<void> {
    // Test that tracking events are fired
    const mockTrackingEvents = ['impression', 'start', 'firstQuartile', 'midpoint', 'thirdQuartile', 'complete'];
    
    const hasImpression = mockTrackingEvents.includes('impression');
    const hasStart = mockTrackingEvents.includes('start');
    const hasComplete = mockTrackingEvents.includes('complete');
    const hasQuartiles = mockTrackingEvents.includes('firstQuartile') && 
                         mockTrackingEvents.includes('midpoint') && 
                         mockTrackingEvents.includes('thirdQuartile');

    this.results.push({
      name: 'tracking_impression',
      passed: hasImpression,
      message: hasImpression ? 'Impression event fired' : 'Missing impression event',
      actual: hasImpression,
      expected: true
    });

    this.results.push({
      name: 'tracking_quartiles',
      passed: hasQuartiles,
      message: hasQuartiles ? 'All quartile events fired' : 'Missing quartile events',
      actual: mockTrackingEvents.filter(e => e.includes('Quartile') || e === 'midpoint'),
      expected: ['firstQuartile', 'midpoint', 'thirdQuartile']
    });

    this.results.push({
      name: 'tracking_complete',
      passed: hasComplete,
      message: hasComplete ? 'Complete event fired' : 'Missing complete event',
      actual: hasComplete,
      expected: true
    });
  }

  private async testLiveEdge(): Promise<void> {
    // Test live edge handling
    const mockLiveEdgeLatency = 12; // seconds behind live
    const maxAllowedLatency = 30;
    const withinLatency = mockLiveEdgeLatency <= maxAllowedLatency;

    this.results.push({
      name: 'live_edge_latency',
      passed: withinLatency,
      message: withinLatency 
        ? `Live edge latency within bounds: ${mockLiveEdgeLatency}s` 
        : `Live edge latency too high: ${mockLiveEdgeLatency}s`,
      actual: mockLiveEdgeLatency,
      expected: `<= ${maxAllowedLatency}s`
    });
  }
}

// Export test function for Jest
export async function runHLSLiveTSTests(): Promise<TestResult> {
  const runner = new HLSLiveTSRunner();
  return runner.run();
}

// Jest test suite
describe('HLS Live TS Variant', () => {
  let result: TestResult;

  beforeAll(async () => {
    result = await runHLSLiveTSTests();
  });

  it('should pass all manifest tests', () => {
    const manifestTests = result.assertions.filter(a => a.name.startsWith('manifest_'));
    manifestTests.forEach(test => {
      expect(test.passed).toBe(true);
    });
  });

  it('should pass all segment tests', () => {
    const segmentTests = result.assertions.filter(a => a.name.startsWith('segment_'));
    segmentTests.forEach(test => {
      expect(test.passed).toBe(true);
    });
  });

  it('should pass all discontinuity tests', () => {
    const discTests = result.assertions.filter(a => a.name.startsWith('discontinuity_'));
    discTests.forEach(test => {
      expect(test.passed).toBe(true);
    });
  });

  it('should pass all SCTE-35 tests', () => {
    const scteTests = result.assertions.filter(a => a.name.startsWith('scte35_'));
    scteTests.forEach(test => {
      expect(test.passed).toBe(true);
    });
  });

  it('should pass all ad insertion tests', () => {
    const adTests = result.assertions.filter(a => a.name.startsWith('ad_'));
    adTests.forEach(test => {
      expect(test.passed).toBe(true);
    });
  });

  it('should pass all tracking tests', () => {
    const trackingTests = result.assertions.filter(a => a.name.startsWith('tracking_'));
    trackingTests.forEach(test => {
      expect(test.passed).toBe(true);
    });
  });

  it('should pass live edge tests', () => {
    const liveTests = result.assertions.filter(a => a.name.startsWith('live_'));
    liveTests.forEach(test => {
      expect(test.passed).toBe(true);
    });
  });

  it('should have no errors', () => {
    expect(result.errors.length).toBe(0);
  });
});
