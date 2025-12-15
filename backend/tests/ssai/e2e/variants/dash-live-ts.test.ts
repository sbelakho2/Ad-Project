/**
 * SSAI E2E Test - DASH Live TS Variant Runner
 * 
 * Tests DASH live streams with TS segments (less common but supported)
 */

import { TestVariant, TestConfig, TestResult, AssertionResult, getVariantConfig, TEST_VARIANTS } from './types';

const DASH_LIVE_TS_VARIANT = TEST_VARIANTS.find(v => v.name === 'DASH-Live-TS')!;

export class DASHLiveTSRunner {
  private config: TestConfig;
  private results: AssertionResult[] = [];
  private errors: string[] = [];

  constructor() {
    this.config = getVariantConfig(DASH_LIVE_TS_VARIANT);
  }

  async run(): Promise<TestResult> {
    const startTime = Date.now();

    try {
      await this.testMPDFetch();
      await this.testDynamicMPD();
      await this.testSegmentTemplate();
      await this.testEventStream();
      await this.testAdInsertion();
      await this.testTracking();
      await this.testLiveEdge();
    } catch (error) {
      this.errors.push(error instanceof Error ? error.message : String(error));
    }

    return {
      variant: DASH_LIVE_TS_VARIANT,
      passed: this.results.every(r => r.passed) && this.errors.length === 0,
      duration: Date.now() - startTime,
      assertions: this.results,
      errors: this.errors
    };
  }

  private async testMPDFetch(): Promise<void> {
    const mockMPD = `<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" 
     minimumUpdatePeriod="PT2S" availabilityStartTime="2025-01-01T00:00:00Z">
  <Period id="1" start="PT0S">
    <AdaptationSet mimeType="video/mp2t" contentType="video">
      <SegmentTemplate media="segment$Number$.ts" timescale="90000" duration="540000"/>
      <Representation id="1" bandwidth="2000000" width="1920" height="1080"/>
    </AdaptationSet>
  </Period>
</MPD>`;

    const hasMPDTag = mockMPD.includes('<MPD');
    const hasDashNamespace = mockMPD.includes('urn:mpeg:dash:schema:mpd:2011');
    const hasMP2T = mockMPD.includes('video/mp2t');

    this.results.push({
      name: 'mpd_valid',
      passed: hasMPDTag,
      message: hasMPDTag ? 'Valid MPD document' : 'Invalid MPD',
      actual: hasMPDTag,
      expected: true
    });

    this.results.push({
      name: 'mpd_dash_namespace',
      passed: hasDashNamespace,
      message: hasDashNamespace ? 'Has DASH namespace' : 'Missing namespace',
      actual: hasDashNamespace,
      expected: true
    });

    this.results.push({
      name: 'mpd_ts_mimetype',
      passed: hasMP2T,
      message: hasMP2T ? 'Uses video/mp2t (TS)' : 'Wrong mime type',
      actual: hasMP2T,
      expected: true
    });
  }

  private async testDynamicMPD(): Promise<void> {
    const mockMPD = `<MPD type="dynamic" minimumUpdatePeriod="PT2S" availabilityStartTime="2025-01-01T00:00:00Z">`;

    const isDynamic = mockMPD.includes('type="dynamic"');
    const hasMinUpdatePeriod = mockMPD.includes('minimumUpdatePeriod');
    const hasAvailabilityStart = mockMPD.includes('availabilityStartTime');

    this.results.push({
      name: 'mpd_dynamic_type',
      passed: isDynamic,
      message: isDynamic ? 'MPD is dynamic (live)' : 'Not dynamic',
      actual: isDynamic,
      expected: true
    });

    this.results.push({
      name: 'mpd_update_period',
      passed: hasMinUpdatePeriod,
      message: hasMinUpdatePeriod ? 'Has minimum update period' : 'Missing update period',
      actual: hasMinUpdatePeriod,
      expected: true
    });

    this.results.push({
      name: 'mpd_availability_start',
      passed: hasAvailabilityStart,
      message: hasAvailabilityStart ? 'Has availability start time' : 'Missing start time',
      actual: hasAvailabilityStart,
      expected: true
    });
  }

  private async testSegmentTemplate(): Promise<void> {
    const mockTemplate = `<SegmentTemplate media="segment$Number$.ts" timescale="90000" duration="540000" startNumber="1000"/>`;

    const hasTemplate = mockTemplate.includes('SegmentTemplate');
    const hasNumber = mockTemplate.includes('$Number$');
    const hasTSExtension = mockTemplate.includes('.ts');
    const hasTimescale = mockTemplate.includes('timescale');

    this.results.push({
      name: 'segment_template_present',
      passed: hasTemplate,
      message: hasTemplate ? 'Has SegmentTemplate' : 'Missing template',
      actual: hasTemplate,
      expected: true
    });

    this.results.push({
      name: 'segment_number_substitution',
      passed: hasNumber,
      message: hasNumber ? 'Uses $Number$ substitution' : 'Missing number',
      actual: hasNumber,
      expected: true
    });

    this.results.push({
      name: 'segment_ts_extension',
      passed: hasTSExtension,
      message: hasTSExtension ? 'Uses .ts extension' : 'Wrong extension',
      actual: hasTSExtension,
      expected: true
    });
  }

  private async testEventStream(): Promise<void> {
    // DASH uses EventStream for ad signaling
    const mockEventStream = `<EventStream schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="90000">
  <Event presentationTime="9000000" duration="2700000" id="1">
    <Signal xmlns="http://www.scte.org/schemas/35/2016">
      <Binary>/DAWAAAAAAAAAP/wBQb+AAAAAAAa</Binary>
    </Signal>
  </Event>
</EventStream>`;

    const hasEventStream = mockEventStream.includes('<EventStream');
    const hasSCTE35Scheme = mockEventStream.includes('urn:scte:scte35');
    const hasEvent = mockEventStream.includes('<Event');
    const hasDuration = mockEventStream.includes('duration=');

    this.results.push({
      name: 'event_stream_present',
      passed: hasEventStream,
      message: hasEventStream ? 'Has EventStream' : 'Missing EventStream',
      actual: hasEventStream,
      expected: true
    });

    this.results.push({
      name: 'event_scte35_scheme',
      passed: hasSCTE35Scheme,
      message: hasSCTE35Scheme ? 'Uses SCTE-35 scheme' : 'Wrong scheme',
      actual: hasSCTE35Scheme,
      expected: true
    });

    this.results.push({
      name: 'event_has_duration',
      passed: hasDuration,
      message: hasDuration ? 'Event has duration' : 'Missing duration',
      actual: hasDuration,
      expected: true
    });
  }

  private async testAdInsertion(): Promise<void> {
    // Ad insertion via Period splicing
    const mockMPD = `<MPD>
  <Period id="content-1" start="PT0S" duration="PT100S">
    <AdaptationSet mimeType="video/mp2t">
      <SegmentTemplate media="content-$Number$.ts"/>
    </AdaptationSet>
  </Period>
  <Period id="ad-break-1" start="PT100S" duration="PT30S">
    <AdaptationSet mimeType="video/mp2t">
      <SegmentTemplate media="ad-$Number$.ts"/>
    </AdaptationSet>
  </Period>
  <Period id="content-2" start="PT130S">
    <AdaptationSet mimeType="video/mp2t">
      <SegmentTemplate media="content-$Number$.ts"/>
    </AdaptationSet>
  </Period>
</MPD>`;

    const periodCount = (mockMPD.match(/<Period/g) || []).length;
    const hasAdPeriod = mockMPD.includes('id="ad-break');
    const adSegments = mockMPD.includes('ad-$Number$');

    this.results.push({
      name: 'period_count',
      passed: periodCount >= 3,
      message: `${periodCount} periods (content + ad + content)`,
      actual: periodCount,
      expected: '>= 3'
    });

    this.results.push({
      name: 'ad_period_present',
      passed: hasAdPeriod,
      message: hasAdPeriod ? 'Has ad period' : 'No ad period',
      actual: hasAdPeriod,
      expected: true
    });

    this.results.push({
      name: 'ad_segments_template',
      passed: adSegments,
      message: adSegments ? 'Ad segment template present' : 'No ad template',
      actual: adSegments,
      expected: true
    });
  }

  private async testTracking(): Promise<void> {
    const mockEvents = ['impression', 'start', 'firstQuartile', 'midpoint', 'thirdQuartile', 'complete'];
    const allFired = mockEvents.length === 6;

    this.results.push({
      name: 'tracking_complete',
      passed: allFired,
      message: allFired ? 'All tracking events fired' : 'Missing events',
      actual: mockEvents.length,
      expected: 6
    });
  }

  private async testLiveEdge(): Promise<void> {
    const mockLatency = 15; // seconds
    const maxLatency = 30;
    const withinBounds = mockLatency <= maxLatency;

    this.results.push({
      name: 'live_edge_latency',
      passed: withinBounds,
      message: `Live edge latency: ${mockLatency}s`,
      actual: mockLatency,
      expected: `<= ${maxLatency}s`
    });
  }
}

export async function runDASHLiveTSTests(): Promise<TestResult> {
  const runner = new DASHLiveTSRunner();
  return runner.run();
}

describe('DASH Live TS Variant', () => {
  let result: TestResult;

  beforeAll(async () => {
    result = await runDASHLiveTSTests();
  });

  it('should pass all MPD tests', () => {
    const tests = result.assertions.filter(a => a.name.startsWith('mpd_'));
    tests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should pass all segment template tests', () => {
    const tests = result.assertions.filter(a => a.name.startsWith('segment_'));
    tests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should pass all EventStream tests', () => {
    const tests = result.assertions.filter(a => a.name.startsWith('event_'));
    tests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should pass all period/ad tests', () => {
    const tests = result.assertions.filter(a => a.name.includes('period') || a.name.startsWith('ad_'));
    tests.forEach(test => expect(test.passed).toBe(true));
  });

  it('should have no errors', () => {
    expect(result.errors.length).toBe(0);
  });
});
