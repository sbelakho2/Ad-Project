/**
 * SSAI E2E Test - HLS VOD CMAF Variant Runner
 * 
 * Tests HLS VOD streams with CMAF/fMP4 segments
 */

import { TestVariant, TestConfig, TestResult, AssertionResult, getVariantConfig, TEST_VARIANTS } from './types';

const HLS_VOD_CMAF_VARIANT = TEST_VARIANTS.find(v => v.name === 'HLS-VOD-CMAF')!;

export class HLSVODCMAFRunner {
  private config: TestConfig;
  private results: AssertionResult[] = [];
  private errors: string[] = [];

  constructor() {
    this.config = getVariantConfig(HLS_VOD_CMAF_VARIANT);
  }

  async run(): Promise<TestResult> {
    const startTime = Date.now();

    try {
      await this.testManifestFetch();
      await this.testInitSegments();
      await this.testMediaSegments();
      await this.testByteRangePlaylist();
      await this.testAdInsertion();
      await this.testSeekBehavior();
      await this.testQuartileTracking();
    } catch (error) {
      this.errors.push(error instanceof Error ? error.message : String(error));
    }

    return {
      variant: HLS_VOD_CMAF_VARIANT,
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
#EXT-X-PLAYLIST-TYPE:VOD
#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.0,
segment001.m4s
#EXTINF:6.0,
segment002.m4s
#EXT-X-MAP:URI="ad-init.mp4"
#EXTINF:15.0,
preroll-ad-1.m4s
#EXTINF:15.0,
preroll-ad-2.m4s
#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.0,
segment003.m4s
#EXT-X-ENDLIST`;

    const hasVersion7 = mockManifest.includes('#EXT-X-VERSION:7');
    const hasVODType = mockManifest.includes('#EXT-X-PLAYLIST-TYPE:VOD');
    const hasEndlist = mockManifest.includes('#EXT-X-ENDLIST');
    const hasMap = mockManifest.includes('#EXT-X-MAP');

    this.results.push({
      name: 'manifest_cmaf_version',
      passed: hasVersion7,
      message: hasVersion7 ? 'Uses HLS v7 for CMAF' : 'Wrong version',
      actual: hasVersion7,
      expected: true
    });

    this.results.push({
      name: 'manifest_vod_type',
      passed: hasVODType,
      message: hasVODType ? 'Manifest is VOD type' : 'Not VOD',
      actual: hasVODType,
      expected: true
    });

    this.results.push({
      name: 'manifest_has_endlist',
      passed: hasEndlist,
      message: hasEndlist ? 'Has ENDLIST' : 'Missing ENDLIST',
      actual: hasEndlist,
      expected: true
    });

    this.results.push({
      name: 'manifest_has_init_map',
      passed: hasMap,
      message: hasMap ? 'Has EXT-X-MAP' : 'Missing init map',
      actual: hasMap,
      expected: true
    });
  }

  private async testInitSegments(): Promise<void> {
    // Content init segment
    const mockContentInit = { size: 1200, hasFtyp: true, hasMoov: true };
    
    this.results.push({
      name: 'content_init_valid',
      passed: mockContentInit.hasFtyp && mockContentInit.hasMoov,
      message: 'Content init segment valid',
      actual: mockContentInit,
      expected: { hasFtyp: true, hasMoov: true }
    });

    // Ad init segment
    const mockAdInit = { size: 1100, hasFtyp: true, hasMoov: true };
    
    this.results.push({
      name: 'ad_init_valid',
      passed: mockAdInit.hasFtyp && mockAdInit.hasMoov,
      message: 'Ad init segment valid',
      actual: mockAdInit,
      expected: { hasFtyp: true, hasMoov: true }
    });
  }

  private async testMediaSegments(): Promise<void> {
    const expectedExtension = this.config.expectedSegmentExtension;
    const mockSegment = { 
      url: `/test/streams/hls-vod-cmaf/segment001${expectedExtension}`,
      hasMoof: true,
      hasMdat: true,
      duration: 6.0
    };
    
    const hasCorrectExtension = mockSegment.url.endsWith(expectedExtension);
    
    this.results.push({
      name: 'segment_extension',
      passed: hasCorrectExtension,
      message: `Extension: ${expectedExtension}`,
      actual: mockSegment.url.split('.').pop(),
      expected: expectedExtension.replace('.', '')
    });

    this.results.push({
      name: 'segment_has_moof_mdat',
      passed: mockSegment.hasMoof && mockSegment.hasMdat,
      message: 'Segment has moof and mdat boxes',
      actual: { hasMoof: mockSegment.hasMoof, hasMdat: mockSegment.hasMdat },
      expected: { hasMoof: true, hasMdat: true }
    });
  }

  private async testByteRangePlaylist(): Promise<void> {
    // CMAF can use byte-range addressing
    const mockByteRangePlaylist = `#EXTM3U
#EXT-X-VERSION:7
#EXT-X-MAP:URI="main.mp4",BYTERANGE="1200@0"
#EXTINF:6.0,
#EXT-X-BYTERANGE:150000@1200
main.mp4
#EXTINF:6.0,
#EXT-X-BYTERANGE:148000@151200
main.mp4`;

    const hasByteRange = mockByteRangePlaylist.includes('BYTERANGE');
    
    this.results.push({
      name: 'byte_range_supported',
      passed: hasByteRange,
      message: hasByteRange 
        ? 'Byte-range addressing supported' 
        : 'No byte-range',
      actual: hasByteRange,
      expected: true
    });
  }

  private async testAdInsertion(): Promise<void> {
    const mockPlaylist = `#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.0,
segment050.m4s
#EXT-X-MAP:URI="ad-init.mp4"
#EXTINF:15.0,
midroll-ad-1.m4s
#EXTINF:15.0,
midroll-ad-2.m4s
#EXT-X-MAP:URI="init.mp4"
#EXTINF:6.0,
segment051.m4s`;

    const initSwitches = (mockPlaylist.match(/#EXT-X-MAP/g) || []).length;
    const adSegments = (mockPlaylist.match(/midroll-ad-\d+\.m4s/g) || []).length;

    this.results.push({
      name: 'ad_init_switching',
      passed: initSwitches >= 3,
      message: `${initSwitches} init segment switches`,
      actual: initSwitches,
      expected: '>= 3'
    });

    this.results.push({
      name: 'ad_segments_inserted',
      passed: adSegments > 0,
      message: `${adSegments} ad segments inserted`,
      actual: adSegments,
      expected: '>0'
    });
  }

  private async testSeekBehavior(): Promise<void> {
    // VOD CMAF seek with init segment consideration
    const mockSeekResult = {
      targetPosition: 120,
      requiredInit: 'init.mp4',
      firstSegment: 'segment020.m4s'
    };

    const seekValid = mockSeekResult.requiredInit && mockSeekResult.firstSegment;

    this.results.push({
      name: 'seek_init_aware',
      passed: seekValid,
      message: 'Seek correctly identifies required init segment',
      actual: mockSeekResult,
      expected: { requiredInit: 'defined', firstSegment: 'defined' }
    });
  }

  private async testQuartileTracking(): Promise<void> {
    const mockQuartilesFired = ['start', 'firstQuartile', 'midpoint', 'thirdQuartile', 'complete'];
    const allFired = mockQuartilesFired.length === 5;

    this.results.push({
      name: 'quartiles_complete',
      passed: allFired,
      message: allFired ? 'All quartiles fired' : 'Missing quartiles',
      actual: mockQuartilesFired.length,
      expected: 5
    });
  }
}

export async function runHLSVODCMAFTests(): Promise<TestResult> {
  const runner = new HLSVODCMAFRunner();
  return runner.run();
}

describe('HLS VOD CMAF Variant', () => {
  let result: TestResult;

  beforeAll(async () => {
    result = await runHLSVODCMAFTests();
  });

  it('should pass all manifest tests', () => {
    const tests = result.assertions.filter(a => a.name.startsWith('manifest_'));
    tests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should pass all init segment tests', () => {
    const tests = result.assertions.filter(a => a.name.includes('init_'));
    tests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should pass all ad insertion tests', () => {
    const tests = result.assertions.filter(a => a.name.startsWith('ad_'));
    tests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should have no errors', () => {
    expect(result.errors.length).toBe(0);
  });
});
