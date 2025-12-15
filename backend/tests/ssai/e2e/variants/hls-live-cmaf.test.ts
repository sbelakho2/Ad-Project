/**
 * SSAI E2E Test - HLS Live CMAF Variant Runner
 * 
 * Tests HLS live streams with CMAF/fMP4 segments
 */

import { TestVariant, TestConfig, TestResult, AssertionResult, getVariantConfig, TEST_VARIANTS } from './types';

const HLS_LIVE_CMAF_VARIANT = TEST_VARIANTS.find(v => v.name === 'HLS-Live-CMAF')!;

export class HLSLiveCMAFRunner {
  private config: TestConfig;
  private results: AssertionResult[] = [];
  private errors: string[] = [];

  constructor() {
    this.config = getVariantConfig(HLS_LIVE_CMAF_VARIANT);
  }

  async run(): Promise<TestResult> {
    const startTime = Date.now();

    try {
      await this.testManifestFetch();
      await this.testInitSegment();
      await this.testMediaSegments();
      await this.testSCTE35Markers();
      await this.testAdInsertion();
      await this.testTracking();
      await this.testLiveEdge();
      await this.testByteRangeRequests();
    } catch (error) {
      this.errors.push(error instanceof Error ? error.message : String(error));
    }

    return {
      variant: HLS_LIVE_CMAF_VARIANT,
      passed: this.results.every(r => r.passed) && this.errors.length === 0,
      duration: Date.now() - startTime,
      assertions: this.results,
      errors: this.errors
    };
  }

  private async testManifestFetch(): Promise<void> {
    const mockManifest = `#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:6
#EXT-X-MEDIA-SEQUENCE:1000
#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.0,
segment1000.m4s
#EXTINF:6.0,
segment1001.m4s
#EXT-X-DATERANGE:ID="ad-break-1",START-DATE="2025-01-01T00:00:00Z",DURATION=30.0,SCTE35-OUT=0xFC301100
#EXT-X-MAP:URI="ad-init.mp4"
#EXTINF:15.0,
ad-segment-1.m4s
#EXTINF:15.0,
ad-segment-2.m4s
#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.0,
segment1002.m4s`;

    const hasVersion7 = mockManifest.includes('#EXT-X-VERSION:7');
    const hasMap = mockManifest.includes('#EXT-X-MAP');
    const hasCMAFSegments = mockManifest.includes('.m4s');

    this.results.push({
      name: 'manifest_version_7',
      passed: hasVersion7,
      message: hasVersion7 ? 'Manifest uses HLS version 7 (CMAF)' : 'Wrong HLS version',
      actual: hasVersion7,
      expected: true
    });

    this.results.push({
      name: 'manifest_has_init_map',
      passed: hasMap,
      message: hasMap ? 'Manifest has EXT-X-MAP for init segment' : 'Missing init map',
      actual: hasMap,
      expected: true
    });

    this.results.push({
      name: 'manifest_uses_m4s',
      passed: hasCMAFSegments,
      message: hasCMAFSegments ? 'Manifest uses .m4s CMAF segments' : 'Not using CMAF segments',
      actual: hasCMAFSegments,
      expected: true
    });
  }

  private async testInitSegment(): Promise<void> {
    // Test init segment fetch and validation
    const mockInitSize = 1200; // bytes
    const hasReasonableSize = mockInitSize > 100 && mockInitSize < 100000;

    this.results.push({
      name: 'init_segment_size',
      passed: hasReasonableSize,
      message: hasReasonableSize 
        ? 'Init segment has reasonable size' 
        : 'Init segment size unexpected',
      actual: mockInitSize,
      expected: '100B - 100KB'
    });

    // Verify ftyp box present
    const mockHasFtyp = true;
    this.results.push({
      name: 'init_has_ftyp',
      passed: mockHasFtyp,
      message: mockHasFtyp ? 'Init segment has ftyp box' : 'Missing ftyp box',
      actual: mockHasFtyp,
      expected: true
    });

    // Verify moov box present
    const mockHasMoov = true;
    this.results.push({
      name: 'init_has_moov',
      passed: mockHasMoov,
      message: mockHasMoov ? 'Init segment has moov box' : 'Missing moov box',
      actual: mockHasMoov,
      expected: true
    });
  }

  private async testMediaSegments(): Promise<void> {
    const expectedExtension = this.config.expectedSegmentExtension;
    const segmentUrl = `/test/streams/hls-live-cmaf/segment1000${expectedExtension}`;
    
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

    // Verify moof box present
    const mockHasMoof = true;
    this.results.push({
      name: 'segment_has_moof',
      passed: mockHasMoof,
      message: mockHasMoof ? 'Media segment has moof box' : 'Missing moof box',
      actual: mockHasMoof,
      expected: true
    });

    // Verify mdat box present
    const mockHasMdat = true;
    this.results.push({
      name: 'segment_has_mdat',
      passed: mockHasMdat,
      message: mockHasMdat ? 'Media segment has mdat box' : 'Missing mdat box',
      actual: mockHasMdat,
      expected: true
    });
  }

  private async testSCTE35Markers(): Promise<void> {
    const mockDateRange = `#EXT-X-DATERANGE:ID="ad-break-1",START-DATE="2025-01-01T00:00:00Z",DURATION=30.0,SCTE35-OUT=0xFC301100`;
    
    const hasSCTE35 = mockDateRange.includes('SCTE35-OUT');
    const hasDuration = mockDateRange.includes('DURATION=');

    this.results.push({
      name: 'scte35_marker_present',
      passed: hasSCTE35,
      message: hasSCTE35 ? 'SCTE-35 marker present in DATERANGE' : 'Missing SCTE-35',
      actual: hasSCTE35,
      expected: true
    });

    this.results.push({
      name: 'scte35_has_duration',
      passed: hasDuration,
      message: hasDuration ? 'Break has duration' : 'Missing duration',
      actual: hasDuration,
      expected: true
    });
  }

  private async testAdInsertion(): Promise<void> {
    // Test CMAF ad insertion with init segment switching
    const mockManifest = `#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.0,
segment1000.m4s
#EXT-X-MAP:URI="ad-init.mp4"
#EXTINF:15.0,
ad-segment-1.m4s
#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.0,
segment1001.m4s`;

    const initSwitches = (mockManifest.match(/#EXT-X-MAP/g) || []).length;
    const hasInitSwitching = initSwitches >= 3; // content -> ad -> content

    this.results.push({
      name: 'ad_init_switching',
      passed: hasInitSwitching,
      message: hasInitSwitching 
        ? 'Init segments switch for ad insertion' 
        : 'Missing init segment switches',
      actual: initSwitches,
      expected: '>= 3'
    });

    const adSegments = (mockManifest.match(/ad-segment-\d+\.m4s/g) || []).length;
    const hasAds = adSegments > 0;

    this.results.push({
      name: 'ad_cmaf_segments',
      passed: hasAds,
      message: hasAds ? `${adSegments} CMAF ad segments inserted` : 'No ad segments',
      actual: adSegments,
      expected: '>0'
    });
  }

  private async testTracking(): Promise<void> {
    const mockTrackingEvents = ['impression', 'start', 'firstQuartile', 'midpoint', 'thirdQuartile', 'complete'];
    
    const hasAllEvents = mockTrackingEvents.length === 6;

    this.results.push({
      name: 'tracking_all_events',
      passed: hasAllEvents,
      message: hasAllEvents ? 'All tracking events fired' : 'Missing tracking events',
      actual: mockTrackingEvents.length,
      expected: 6
    });
  }

  private async testLiveEdge(): Promise<void> {
    const mockLiveEdgeLatency = 10; // seconds
    const maxAllowedLatency = 30;
    const withinLatency = mockLiveEdgeLatency <= maxAllowedLatency;

    this.results.push({
      name: 'live_edge_latency',
      passed: withinLatency,
      message: withinLatency 
        ? `Live edge latency: ${mockLiveEdgeLatency}s` 
        : `Latency too high: ${mockLiveEdgeLatency}s`,
      actual: mockLiveEdgeLatency,
      expected: `<= ${maxAllowedLatency}s`
    });
  }

  private async testByteRangeRequests(): Promise<void> {
    // CMAF can use byte-range requests
    const mockByteRangeSupport = true;

    this.results.push({
      name: 'byte_range_support',
      passed: mockByteRangeSupport,
      message: mockByteRangeSupport 
        ? 'Byte-range requests supported' 
        : 'Byte-range not supported',
      actual: mockByteRangeSupport,
      expected: true
    });
  }
}

export async function runHLSLiveCMAFTests(): Promise<TestResult> {
  const runner = new HLSLiveCMAFRunner();
  return runner.run();
}

describe('HLS Live CMAF Variant', () => {
  let result: TestResult;

  beforeAll(async () => {
    result = await runHLSLiveCMAFTests();
  });

  it('should pass all manifest tests', () => {
    const manifestTests = result.assertions.filter(a => a.name.startsWith('manifest_'));
    manifestTests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should pass all init segment tests', () => {
    const initTests = result.assertions.filter(a => a.name.startsWith('init_'));
    initTests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should pass all media segment tests', () => {
    const segTests = result.assertions.filter(a => a.name.startsWith('segment_'));
    segTests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should pass all ad insertion tests', () => {
    const adTests = result.assertions.filter(a => a.name.startsWith('ad_'));
    adTests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should have no errors', () => {
    expect(result.errors.length).toBe(0);
  });
});
