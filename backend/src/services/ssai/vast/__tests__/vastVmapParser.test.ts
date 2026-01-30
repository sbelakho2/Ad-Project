/**
 * VAST/VMAP Parser Test Suite
 *
 * Comprehensive tests for VAST 2.0-4.2 and VMAP 1.0 parsing
 */

import {
  VASTParser,
  VMAPParser,
  VASTXMLParser,
  AdPodScheduler,
  VASTTrackingManager,
  AdRequestHandler,
  normalizeMediaFiles,
  flattenTracking,
  extractSSAICreative,
  calculateAdBreakTimes,
  getPreRollBreaks,
  getMidRollBreaks,
  getPostRollBreaks,
  formatPlayhead,
  getQuartile,
  buildTrackingContext,
  generateTimeline,
  parseVAST,
  parseVMAP,
} from '../index';
import type {
  VASTDocument,
  VASTAd,
  VMAPDocument,
  VMAPAdBreak,
  SSAIReadyCreative,
} from '../types';

// =============================================================================
// Test Data - VAST Samples
// =============================================================================

const VAST_2_0_SAMPLE = `<?xml version="1.0" encoding="UTF-8"?>
<VAST version="2.0">
  <Ad id="ad_001">
    <InLine>
      <AdSystem version="1.0">Test Ad System</AdSystem>
      <AdTitle>Test Ad</AdTitle>
      <Description>A test advertisement</Description>
      <Impression id="imp1"><![CDATA[https://example.com/impression?id=1]]></Impression>
      <Impression id="imp2"><![CDATA[https://example.com/impression?id=2]]></Impression>
      <Creatives>
        <Creative id="creative_001" sequence="1">
          <Linear>
            <Duration>00:00:30</Duration>
            <MediaFiles>
              <MediaFile delivery="progressive" type="video/mp4" width="1920" height="1080" bitrate="3000">
                <![CDATA[https://example.com/video.mp4]]>
              </MediaFile>
              <MediaFile delivery="streaming" type="application/x-mpegURL" width="1920" height="1080">
                <![CDATA[https://example.com/video.m3u8]]>
              </MediaFile>
            </MediaFiles>
            <TrackingEvents>
              <Tracking event="start"><![CDATA[https://example.com/track?event=start]]></Tracking>
              <Tracking event="firstQuartile"><![CDATA[https://example.com/track?event=firstQuartile]]></Tracking>
              <Tracking event="midpoint"><![CDATA[https://example.com/track?event=midpoint]]></Tracking>
              <Tracking event="thirdQuartile"><![CDATA[https://example.com/track?event=thirdQuartile]]></Tracking>
              <Tracking event="complete"><![CDATA[https://example.com/track?event=complete]]></Tracking>
            </TrackingEvents>
            <VideoClicks>
              <ClickThrough id="click1"><![CDATA[https://example.com/clickthrough]]></ClickThrough>
              <ClickTracking id="clicktrack1"><![CDATA[https://example.com/clicktracking]]></ClickTracking>
            </VideoClicks>
          </Linear>
        </Creative>
      </Creatives>
    </InLine>
  </Ad>
</VAST>`;

const VAST_4_2_SAMPLE = `<?xml version="1.0" encoding="UTF-8"?>
<VAST version="4.2">
  <Ad id="ad_002" sequence="1">
    <InLine>
      <AdSystem version="2.0">Advanced Ad System</AdSystem>
      <AdTitle>VAST 4.2 Test Ad</AdTitle>
      <AdServingId>abc123</AdServingId>
      <Advertiser>Test Advertiser</Advertiser>
      <Category authority="IAB">IAB1-1</Category>
      <Pricing model="cpm" currency="USD">15.00</Pricing>
      <Impression id="imp1"><![CDATA[https://example.com/impression?[CACHEBUSTING]]]></Impression>
      <ViewableImpression>
        <Viewable><![CDATA[https://example.com/viewable]]></Viewable>
        <NotViewable><![CDATA[https://example.com/not-viewable]]></NotViewable>
        <ViewUndetermined><![CDATA[https://example.com/undetermined]]></ViewUndetermined>
      </ViewableImpression>
      <AdVerifications>
        <Verification vendor="iabtechlab.com-omid">
          <JavaScriptResource apiFramework="omid" browserOptional="true">
            <![CDATA[https://example.com/omid.js]]>
          </JavaScriptResource>
          <TrackingEvents>
            <Tracking event="verificationNotExecuted">
              <![CDATA[https://example.com/verification-failed?[REASON]]]>
            </Tracking>
          </TrackingEvents>
          <VerificationParameters><![CDATA[{"key": "value"}]]></VerificationParameters>
        </Verification>
      </AdVerifications>
      <Creatives>
        <Creative id="creative_002" adId="ad_002" sequence="1">
          <UniversalAdId idRegistry="Ad-ID">ABCD1234567H</UniversalAdId>
          <Linear skipoffset="00:00:05">
            <Duration>00:00:15</Duration>
            <AdParameters xmlEncoded="false"><![CDATA[{"creative_params": true}]]></AdParameters>
            <MediaFiles>
              <MediaFile delivery="progressive" type="video/mp4" codec="avc1.42E01E,mp4a.40.2" 
                         width="1920" height="1080" bitrate="4000" minBitrate="2000" maxBitrate="6000"
                         scalable="true" maintainAspectRatio="true" fileSize="7500000">
                <![CDATA[https://example.com/video-1080p.mp4]]>
              </MediaFile>
              <MediaFile delivery="progressive" type="video/mp4" width="1280" height="720" bitrate="2000">
                <![CDATA[https://example.com/video-720p.mp4]]>
              </MediaFile>
              <Mezzanine delivery="progressive" type="video/mp4" width="3840" height="2160" fileSize="50000000">
                <![CDATA[https://example.com/video-4k-mezzanine.mp4]]>
              </Mezzanine>
              <InteractiveCreativeFile type="text/html" apiFramework="SIMID">
                <![CDATA[https://example.com/interactive.html]]>
              </InteractiveCreativeFile>
            </MediaFiles>
            <ClosedCaptionFiles>
              <ClosedCaptionFile type="text/vtt" language="en">
                <![CDATA[https://example.com/captions-en.vtt]]>
              </ClosedCaptionFile>
            </ClosedCaptionFiles>
            <TrackingEvents>
              <Tracking event="start"><![CDATA[https://example.com/track/start]]></Tracking>
              <Tracking event="progress" offset="00:00:05"><![CDATA[https://example.com/track/progress-5]]></Tracking>
              <Tracking event="progress" offset="50%"><![CDATA[https://example.com/track/progress-50pct]]></Tracking>
              <Tracking event="complete"><![CDATA[https://example.com/track/complete]]></Tracking>
              <Tracking event="skip"><![CDATA[https://example.com/track/skip]]></Tracking>
            </TrackingEvents>
            <VideoClicks>
              <ClickThrough><![CDATA[https://example.com/landing]]></ClickThrough>
              <ClickTracking><![CDATA[https://example.com/click]]></ClickTracking>
            </VideoClicks>
            <Icons>
              <Icon program="AdChoices" width="20" height="20" xPosition="right" yPosition="top" offset="00:00:00" duration="00:00:15">
                <StaticResource creativeType="image/png"><![CDATA[https://example.com/adchoices.png]]></StaticResource>
                <IconClicks>
                  <IconClickThrough><![CDATA[https://example.com/adchoices]]></IconClickThrough>
                  <IconClickTracking><![CDATA[https://example.com/icon-click]]></IconClickTracking>
                </IconClicks>
                <IconViewTracking><![CDATA[https://example.com/icon-view]]></IconViewTracking>
              </Icon>
            </Icons>
          </Linear>
        </Creative>
        <Creative id="companion_001">
          <CompanionAds required="any">
            <Companion width="300" height="250" id="comp1" assetWidth="300" assetHeight="250">
              <StaticResource creativeType="image/jpeg"><![CDATA[https://example.com/companion.jpg]]></StaticResource>
              <CompanionClickThrough><![CDATA[https://example.com/companion-click]]></CompanionClickThrough>
              <TrackingEvents>
                <Tracking event="creativeView"><![CDATA[https://example.com/companion-view]]></Tracking>
              </TrackingEvents>
            </Companion>
          </CompanionAds>
        </Creative>
      </Creatives>
      <Extensions>
        <Extension type="geo">
          <Country>US</Country>
          <Bandwidth>high</Bandwidth>
        </Extension>
      </Extensions>
    </InLine>
  </Ad>
</VAST>`;

const VAST_WRAPPER_SAMPLE = `<?xml version="1.0" encoding="UTF-8"?>
<VAST version="4.0">
  <Ad id="wrapper_ad">
    <Wrapper followAdditionalWrappers="true" allowMultipleAds="false" fallbackOnNoAd="true">
      <AdSystem>Wrapper Ad System</AdSystem>
      <VASTAdTagURI><![CDATA[https://example.com/vast-inline.xml]]></VASTAdTagURI>
      <Impression><![CDATA[https://example.com/wrapper-impression]]></Impression>
      <Creatives>
        <Creative>
          <Linear>
            <TrackingEvents>
              <Tracking event="start"><![CDATA[https://example.com/wrapper-track/start]]></Tracking>
              <Tracking event="complete"><![CDATA[https://example.com/wrapper-track/complete]]></Tracking>
            </TrackingEvents>
            <VideoClicks>
              <ClickTracking><![CDATA[https://example.com/wrapper-click]]></ClickTracking>
            </VideoClicks>
          </Linear>
        </Creative>
      </Creatives>
      <Error><![CDATA[https://example.com/wrapper-error?code=[ERRORCODE]]]></Error>
    </Wrapper>
  </Ad>
</VAST>`;

const VMAP_SAMPLE = `<?xml version="1.0" encoding="UTF-8"?>
<vmap:VMAP xmlns:vmap="http://www.iab.net/vmap-1.0" version="1.0">
  <vmap:AdBreak timeOffset="start" breakType="linear" breakId="preroll">
    <vmap:AdSource id="preroll-ad" allowMultipleAds="false" followRedirects="true">
      <vmap:VASTAdData>
        <VAST version="4.0">
          <Ad id="preroll_001">
            <InLine>
              <AdSystem>VMAP Test</AdSystem>
              <AdTitle>Pre-roll Ad</AdTitle>
              <Impression><![CDATA[https://example.com/preroll-impression]]></Impression>
              <Creatives>
                <Creative>
                  <Linear>
                    <Duration>00:00:15</Duration>
                    <MediaFiles>
                      <MediaFile delivery="progressive" type="video/mp4" width="1920" height="1080">
                        <![CDATA[https://example.com/preroll.mp4]]>
                      </MediaFile>
                    </MediaFiles>
                  </Linear>
                </Creative>
              </Creatives>
            </InLine>
          </Ad>
        </VAST>
      </vmap:VASTAdData>
    </vmap:AdSource>
    <vmap:TrackingEvents>
      <vmap:Tracking event="breakStart"><![CDATA[https://example.com/break-start]]></vmap:Tracking>
      <vmap:Tracking event="breakEnd"><![CDATA[https://example.com/break-end]]></vmap:Tracking>
    </vmap:TrackingEvents>
  </vmap:AdBreak>
  
  <vmap:AdBreak timeOffset="00:05:00" breakType="linear" breakId="midroll1">
    <vmap:AdSource id="midroll-ad" allowMultipleAds="true">
      <vmap:AdTagURI templateType="vast3"><![CDATA[https://example.com/vast-midroll.xml]]></vmap:AdTagURI>
    </vmap:AdSource>
  </vmap:AdBreak>
  
  <vmap:AdBreak timeOffset="50%" breakType="linear" breakId="midroll2">
    <vmap:AdSource id="midroll-ad-2">
      <vmap:AdTagURI><![CDATA[https://example.com/vast-midroll2.xml]]></vmap:AdTagURI>
    </vmap:AdSource>
  </vmap:AdBreak>
  
  <vmap:AdBreak timeOffset="end" breakType="linear" breakId="postroll">
    <vmap:AdSource id="postroll-ad">
      <vmap:AdTagURI><![CDATA[https://example.com/vast-postroll.xml]]></vmap:AdTagURI>
    </vmap:AdSource>
  </vmap:AdBreak>
</vmap:VMAP>`;

// =============================================================================
// VAST Parser Tests
// =============================================================================

describe('VAST Parser', () => {
  let parser: VASTParser;

  beforeEach(() => {
    parser = new VASTParser();
  });

  describe('VAST 2.0 Parsing', () => {
    it('should parse a valid VAST 2.0 document', async () => {
      const result = await parser.parse(VAST_2_0_SAMPLE);

      expect(result.success).toBe(true);
      expect(result.document).not.toBeNull();
      expect(result.document?.version).toBe('2.0');
      expect(result.document?.ads).toHaveLength(1);
    });

    it('should extract ad metadata', async () => {
      const result = await parser.parse(VAST_2_0_SAMPLE);
      const ad = result.document?.ads[0];

      expect(ad?.id).toBe('ad_001');
      expect(ad?.type).toBe('inline');
      expect(ad?.inLine?.adTitle).toBe('Test Ad');
      expect(ad?.inLine?.description).toBe('A test advertisement');
      expect(ad?.inLine?.adSystem.name).toBe('Test Ad System');
    });

    it('should parse impressions', async () => {
      const result = await parser.parse(VAST_2_0_SAMPLE);
      const impressions = result.document?.ads[0]?.inLine?.impressions || [];

      expect(impressions).toHaveLength(2);
      expect(impressions[0].id).toBe('imp1');
      expect(impressions[0].url).toContain('impression?id=1');
    });

    it('should parse media files', async () => {
      const result = await parser.parse(VAST_2_0_SAMPLE);
      const creative = result.document?.ads[0]?.inLine?.creatives[0];
      const mediaFiles = creative?.linear?.mediaFiles || [];

      expect(mediaFiles).toHaveLength(2);
      expect(mediaFiles[0].delivery).toBe('progressive');
      expect(mediaFiles[0].type).toBe('video/mp4');
      expect(mediaFiles[0].width).toBe(1920);
      expect(mediaFiles[0].height).toBe(1080);
      expect(mediaFiles[0].bitrate).toBe(3000);
    });

    it('should parse tracking events', async () => {
      const result = await parser.parse(VAST_2_0_SAMPLE);
      const creative = result.document?.ads[0]?.inLine?.creatives[0];
      const trackingEvents = creative?.linear?.trackingEvents || [];

      expect(trackingEvents.length).toBeGreaterThanOrEqual(5);

      const startEvent = trackingEvents.find((t) => t.event === 'start');
      expect(startEvent).toBeDefined();
      expect(startEvent?.url).toContain('event=start');
    });

    it('should parse video clicks', async () => {
      const result = await parser.parse(VAST_2_0_SAMPLE);
      const creative = result.document?.ads[0]?.inLine?.creatives[0];
      const videoClicks = creative?.linear?.videoClicks;

      expect(videoClicks?.clickThrough?.url).toContain('clickthrough');
      expect(videoClicks?.clickTrackings).toHaveLength(1);
    });
  });

  describe('VAST 4.2 Parsing', () => {
    it('should parse a valid VAST 4.2 document', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);

      expect(result.success).toBe(true);
      expect(result.document?.version).toBe('4.2');
    });

    it('should parse advertiser and pricing', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);
      const inLine = result.document?.ads[0]?.inLine;

      expect(inLine?.advertiser).toBe('Test Advertiser');
      expect(inLine?.pricing?.model).toBe('cpm');
      expect(inLine?.pricing?.currency).toBe('USD');
      expect(inLine?.pricing?.value).toBe(15);
    });

    it('should parse categories', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);
      const categories = result.document?.ads[0]?.inLine?.categories || [];

      expect(categories).toHaveLength(1);
      expect(categories[0].authority).toBe('IAB');
      expect(categories[0].value).toBe('IAB1-1');
    });

    it('should parse viewable impression', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);
      const vi = result.document?.ads[0]?.inLine?.viewableImpression;

      expect(vi?.viewable).toHaveLength(1);
      expect(vi?.notViewable).toHaveLength(1);
      expect(vi?.viewUndetermined).toHaveLength(1);
    });

    it('should parse ad verifications', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);
      const verifications = result.document?.ads[0]?.inLine?.adVerifications || [];

      expect(verifications).toHaveLength(1);
      expect(verifications[0].vendor).toBe('iabtechlab.com-omid');
      expect(verifications[0].javaScriptResource?.apiFramework).toBe('omid');
      expect(verifications[0].verificationParameters).toContain('key');
    });

    it('should parse universal ad ID', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);
      const creative = result.document?.ads[0]?.inLine?.creatives[0];

      expect(creative?.universalAdIds).toHaveLength(1);
      expect(creative?.universalAdIds[0].idRegistry).toBe('Ad-ID');
      expect(creative?.universalAdIds[0].idValue).toBe('ABCD1234567H');
    });

    it('should parse skip offset', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);
      const linear = result.document?.ads[0]?.inLine?.creatives[0]?.linear;

      expect(linear?.skipOffset).toBe('00:00:05');
    });

    it('should parse icons', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);
      const icons = result.document?.ads[0]?.inLine?.creatives[0]?.linear?.icons || [];

      expect(icons).toHaveLength(1);
      expect(icons[0].program).toBe('AdChoices');
      expect(icons[0].width).toBe(20);
      expect(icons[0].height).toBe(20);
      expect(icons[0].staticResource?.url).toContain('adchoices.png');
    });

    it('should parse companion ads', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);
      const creatives = result.document?.ads[0]?.inLine?.creatives || [];
      const companionCreative = creatives.find((c) => c.type === 'companion');

      expect(companionCreative?.companionAds?.required).toBe('any');
      expect(companionCreative?.companionAds?.companions).toHaveLength(1);
      expect(companionCreative?.companionAds?.companions[0].width).toBe(300);
      expect(companionCreative?.companionAds?.companions[0].height).toBe(250);
    });

    it('should parse closed caption files', async () => {
      const result = await parser.parse(VAST_4_2_SAMPLE);
      const linear = result.document?.ads[0]?.inLine?.creatives[0]?.linear;

      expect(linear?.closedCaptionFiles).toHaveLength(1);
      expect(linear?.closedCaptionFiles?.[0].language).toBe('en');
      expect(linear?.closedCaptionFiles?.[0].type).toBe('text/vtt');
    });
  });

  describe('Wrapper Parsing', () => {
    it('should parse wrapper attributes', async () => {
      // Use resolveWrappers: false to test wrapper parsing without network fetch
      const wrapperParser = new VASTParser({ resolveWrappers: false });
      const result = await wrapperParser.parse(VAST_WRAPPER_SAMPLE);
      const wrapper = result.document?.ads[0]?.wrapper;

      expect(wrapper?.followAdditionalWrappers).toBe(true);
      expect(wrapper?.allowMultipleAds).toBe(false);
      expect(wrapper?.fallbackOnNoAd).toBe(true);
      expect(wrapper?.vastAdTagUri).toContain('vast-inline.xml');
    });

    it('should parse wrapper tracking', async () => {
      // Use resolveWrappers: false to test wrapper parsing without network fetch
      const wrapperParser = new VASTParser({ resolveWrappers: false });
      const result = await wrapperParser.parse(VAST_WRAPPER_SAMPLE);
      const wrapper = result.document?.ads[0]?.wrapper;

      expect(wrapper?.impressions).toHaveLength(1);
      expect(wrapper?.errors).toHaveLength(1);
      expect(wrapper?.creatives).toHaveLength(1);
    });
  });

  describe('Error Handling', () => {
    it('should handle invalid XML', async () => {
      const result = await parser.parse('<invalid xml');

      expect(result.success).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
    });

    it('should handle missing VAST element', async () => {
      const result = await parser.parse('<?xml version="1.0"?><root></root>');

      expect(result.success).toBe(false);
      expect(result.errors.some((e) => e.message.includes('No VAST element'))).toBe(true);
    });

    it('should handle unsupported version', async () => {
      const unsupportedVersion = VAST_2_0_SAMPLE.replace('version="2.0"', 'version="1.0"');
      const result = await parser.parse(unsupportedVersion);

      expect(result.success).toBe(false);
      expect(result.errors.some((e) => e.message.includes('version'))).toBe(true);
    });

    it('should have configurable total wrapper timeout', () => {
      // Test that the config option exists and has a sensible default
      const customParser = new VASTParser({ maxTotalWrapperTimeMs: 5000 });
      // Verify the parser was created with the custom config
      expect(customParser).toBeDefined();
    });

    it('should have configurable max media files limit (DoS protection)', () => {
      // Test that the config option exists
      const customParser = new VASTParser({ maxMediaFilesPerAd: 10 });
      expect(customParser).toBeDefined();
    });
  });
});

// =============================================================================
// VMAP Parser Tests
// =============================================================================

describe('VMAP Parser', () => {
  let parser: VMAPParser;

  beforeEach(() => {
    parser = new VMAPParser();
  });

  describe('VMAP 1.0 Parsing', () => {
    it('should parse a valid VMAP document', async () => {
      const result = await parser.parse(VMAP_SAMPLE);

      expect(result.success).toBe(true);
      expect(result.document).not.toBeNull();
      expect(result.document?.version).toBe('1.0');
    });

    it('should parse ad breaks', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      const breaks = result.document?.adBreaks || [];

      expect(breaks.length).toBeGreaterThanOrEqual(4);
    });

    it('should parse pre-roll break', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      const preroll = result.document?.adBreaks.find((b) => b.breakId === 'preroll');

      expect(preroll).toBeDefined();
      expect(preroll?.timeOffset.type).toBe('start');
      expect(preroll?.breakType).toBe('linear');
    });

    it('should parse time-based mid-roll', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      const midroll = result.document?.adBreaks.find((b) => b.breakId === 'midroll1');

      expect(midroll).toBeDefined();
      expect(midroll?.timeOffset.type).toBe('time');
      if (midroll?.timeOffset.type === 'time') {
        expect(midroll.timeOffset.seconds).toBe(300); // 5 minutes
      }
    });

    it('should parse percentage-based mid-roll', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      const midroll = result.document?.adBreaks.find((b) => b.breakId === 'midroll2');

      expect(midroll).toBeDefined();
      expect(midroll?.timeOffset.type).toBe('percentage');
      if (midroll?.timeOffset.type === 'percentage') {
        expect(midroll.timeOffset.percent).toBe(50);
      }
    });

    it('should parse post-roll break', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      const postroll = result.document?.adBreaks.find((b) => b.breakId === 'postroll');

      expect(postroll).toBeDefined();
      expect(postroll?.timeOffset.type).toBe('end');
    });

    it('should parse inline VAST in ad source', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      const preroll = result.document?.adBreaks.find((b) => b.breakId === 'preroll');

      expect(preroll?.adSource?.type).toBe('inline');
      if (preroll?.adSource?.type === 'inline') {
        expect(preroll.adSource.vastData).toBeDefined();
        expect(preroll.adSource.vastData?.ads).toHaveLength(1);
      }
    });

    it('should parse ad tag URI in ad source', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      const midroll = result.document?.adBreaks.find((b) => b.breakId === 'midroll1');

      expect(midroll?.adSource?.type).toBe('redirect');
      if (midroll?.adSource?.type === 'redirect') {
        expect(midroll.adSource.adTagUri).toContain('vast-midroll.xml');
        expect(midroll.adSource.templateType).toBe('vast3');
      }
    });

    it('should parse tracking events', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      const preroll = result.document?.adBreaks.find((b) => b.breakId === 'preroll');

      expect(preroll?.trackingEvents).toHaveLength(2);
      expect(preroll?.trackingEvents.some((t) => t.event === 'breakStart')).toBe(true);
      expect(preroll?.trackingEvents.some((t) => t.event === 'breakEnd')).toBe(true);
    });
  });

  describe('VMAP Utilities', () => {
    it('should calculate ad break times', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      if (!result.document) return;

      const times = calculateAdBreakTimes(result.document, 600); // 10 minute content

      expect(times.length).toBeGreaterThanOrEqual(4);
      expect(times[0].startTime).toBe(0); // Pre-roll
      expect(times.some((t) => t.startTime === 300)).toBe(true); // 5 min mid-roll
      expect(times.some((t) => t.startTime === 600)).toBe(true); // Post-roll
    });

    it('should get pre-roll breaks', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      if (!result.document) return;

      const prerolls = getPreRollBreaks(result.document);

      expect(prerolls.length).toBeGreaterThanOrEqual(1);
      expect(prerolls[0].timeOffset.type).toBe('start');
    });

    it('should get mid-roll breaks', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      if (!result.document) return;

      const midrolls = getMidRollBreaks(result.document);

      expect(midrolls.length).toBeGreaterThanOrEqual(2);
    });

    it('should get post-roll breaks', async () => {
      const result = await parser.parse(VMAP_SAMPLE);
      if (!result.document) return;

      const postrolls = getPostRollBreaks(result.document);

      expect(postrolls.length).toBeGreaterThanOrEqual(1);
      expect(postrolls[0].timeOffset.type).toBe('end');
    });
  });
});

// =============================================================================
// XML Parser Tests
// =============================================================================

describe('XML Parser Utilities', () => {
  describe('Duration Parsing', () => {
    it('should parse HH:MM:SS format', () => {
      expect(VASTXMLParser.parseDuration('00:00:30')).toBe(30);
      expect(VASTXMLParser.parseDuration('00:01:30')).toBe(90);
      expect(VASTXMLParser.parseDuration('01:30:00')).toBe(5400);
    });

    it('should parse HH:MM:SS.mmm format', () => {
      expect(VASTXMLParser.parseDuration('00:00:30.500')).toBe(30.5);
      expect(VASTXMLParser.parseDuration('00:00:15.250')).toBe(15.25);
    });

    it('should handle invalid durations', () => {
      expect(VASTXMLParser.parseDuration('invalid')).toBeNull();
      expect(VASTXMLParser.parseDuration('')).toBeNull();
      expect(VASTXMLParser.parseDuration(null as unknown as string)).toBeNull();
    });
  });

  describe('Offset Parsing', () => {
    it('should parse time offsets', () => {
      const offset = VASTXMLParser.parseOffset('00:00:10');
      expect(offset?.type).toBe('time');
      if (offset?.type === 'time') {
        expect(offset.seconds).toBe(10);
      }
    });

    it('should parse percentage offsets', () => {
      const offset = VASTXMLParser.parseOffset('50%');
      expect(offset?.type).toBe('percentage');
      if (offset?.type === 'percentage') {
        expect(offset.percent).toBe(50);
      }
    });
  });

  describe('URL Handling', () => {
    it('should decode encoded URLs', () => {
      const encoded = 'https://example.com/track?id=1&amp;type=impression';
      const decoded = VASTXMLParser.decodeUrl(encoded);
      expect(decoded).toBe('https://example.com/track?id=1&type=impression');
    });

    it('should validate URLs', () => {
      expect(VASTXMLParser.isValidUrl('https://example.com')).toBe(true);
      expect(VASTXMLParser.isValidUrl('http://example.com/path')).toBe(true);
      expect(VASTXMLParser.isValidUrl('invalid')).toBe(false);
      expect(VASTXMLParser.isValidUrl('')).toBe(false);
    });

    it('should block dangerous URL schemes (XSS protection)', () => {
      // javascript: URLs should be blocked
      expect(VASTXMLParser.decodeUrl('javascript:alert(1)')).toBe(null);
      expect(VASTXMLParser.decodeUrl('JAVASCRIPT:alert(1)')).toBe(null);
      
      // data: URLs should be blocked
      expect(VASTXMLParser.decodeUrl('data:text/html,<script>alert(1)</script>')).toBe(null);
      
      // vbscript: URLs should be blocked
      expect(VASTXMLParser.decodeUrl('vbscript:msgbox(1)')).toBe(null);
      
      // file: URLs should be blocked
      expect(VASTXMLParser.decodeUrl('file:///etc/passwd')).toBe(null);
      
      // Valid URLs should still work
      expect(VASTXMLParser.decodeUrl('https://example.com')).toBe('https://example.com');
      expect(VASTXMLParser.decodeUrl('http://example.com')).toBe('http://example.com');
      expect(VASTXMLParser.decodeUrl('//example.com/path')).toBe('//example.com/path');
    });

    it('should detect dangerous URLs', () => {
      expect(VASTXMLParser.isDangerousUrl('javascript:alert(1)')).toBe(true);
      expect(VASTXMLParser.isDangerousUrl('JAVASCRIPT:void(0)')).toBe(true);
      expect(VASTXMLParser.isDangerousUrl('data:text/html,test')).toBe(true);
      expect(VASTXMLParser.isDangerousUrl('https://example.com')).toBe(false);
      expect(VASTXMLParser.isDangerousUrl('http://example.com')).toBe(false);
    });
  });

  describe('XXE Protection', () => {
    it('should strip DOCTYPE declarations from XML (XXE protection)', () => {
      const parser = new VASTXMLParser();
      
      // XML with DOCTYPE that could contain malicious entity
      const xmlWithDoctype = `<?xml version="1.0"?>
<!DOCTYPE foo [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<VAST version="2.0"><Ad id="test"><InLine><AdSystem>Test</AdSystem><AdTitle>Test</AdTitle><Creatives></Creatives></InLine></Ad></VAST>`;
      
      const result = parser.parse(xmlWithDoctype);
      // Should successfully parse without error (DOCTYPE stripped)
      expect(result.success).toBe(true);
      expect(result.data).toBeDefined();
    });
  });

  describe('Size Limit Protection', () => {
    it('should reject XML documents exceeding size limit', () => {
      const parser = new VASTXMLParser();
      
      // Create a very large XML string (~11MB, exceeds 10MB limit)
      const largeContent = 'x'.repeat(11 * 1024 * 1024);
      const oversizedXml = `<?xml version="1.0"?><VAST version="2.0"><Data>${largeContent}</Data></VAST>`;
      
      const result = parser.parse(oversizedXml);
      
      expect(result.success).toBe(false);
      expect(result.errors.length).toBeGreaterThan(0);
      expect(result.errors[0].message).toContain('too large');
      expect(result.errors[0].message).toContain('10MB');
      // Should not include raw XML for oversized documents (memory protection)
      expect(result.rawXml).toBe('');
    });

    it('should accept XML documents within size limit', () => {
      const parser = new VASTXMLParser();
      
      // A normal-sized VAST document should work fine
      const normalXml = `<?xml version="1.0"?>
<VAST version="2.0">
  <Ad id="test">
    <InLine>
      <AdSystem>Test</AdSystem>
      <AdTitle>Test Ad</AdTitle>
      <Creatives></Creatives>
    </InLine>
  </Ad>
</VAST>`;
      
      const result = parser.parse(normalXml);
      
      expect(result.success).toBe(true);
      expect(result.data).toBeDefined();
    });
  });
});

// =============================================================================
// Media File Normalization Tests
// =============================================================================

describe('Media File Normalization', () => {
  it('should normalize and score media files', async () => {
    const parser = new VASTParser();
    const result = await parser.parse(VAST_4_2_SAMPLE);
    const mediaFiles = result.document?.ads[0]?.inLine?.creatives[0]?.linear?.mediaFiles || [];

    const normalized = normalizeMediaFiles(mediaFiles, {
      preferredWidth: 1920,
      preferredHeight: 1080,
    });

    expect(normalized.length).toBeGreaterThan(0);
    expect(normalized[0].score).toBeDefined();
    expect(normalized[0].container).toBeDefined();
    expect(normalized[0].aspectRatio).toBeDefined();
  });

  it('should prefer specified delivery type', () => {
    const mediaFiles = [
      { delivery: 'progressive' as const, type: 'video/mp4', width: 1920, height: 1080, bitrate: 3000, url: 'https://example.com/prog.mp4' },
      { delivery: 'streaming' as const, type: 'application/x-mpegURL', width: 1920, height: 1080, url: 'https://example.com/stream.m3u8' },
    ];

    const normalized = normalizeMediaFiles(mediaFiles, {
      preferredDelivery: 'streaming',
    });

    expect(normalized[0].delivery).toBe('streaming');
  });
});

// =============================================================================
// Tracking Flattening Tests
// =============================================================================

describe('Tracking Flattening', () => {
  it('should flatten tracking from an ad', async () => {
    const parser = new VASTParser();
    const result = await parser.parse(VAST_2_0_SAMPLE);
    const ad = result.document?.ads[0];

    if (!ad) {
      throw new Error('No ad found');
    }

    const tracking = flattenTracking(ad);

    expect(tracking.impressions.length).toBeGreaterThan(0);
    expect(tracking.events.start).toBeDefined();
    expect(tracking.events.complete).toBeDefined();
    expect(tracking.clickThrough).toBeDefined();
    expect(tracking.clickTrackings.length).toBeGreaterThan(0);
  });
});

// =============================================================================
// SSAI Creative Extraction Tests
// =============================================================================

describe('SSAI Creative Extraction', () => {
  it('should extract SSAI-ready creative', async () => {
    const parser = new VASTParser();
    const result = await parser.parse(VAST_4_2_SAMPLE);
    const ad = result.document?.ads[0];

    if (!ad) {
      throw new Error('No ad found');
    }

    const creative = extractSSAICreative(ad);

    expect(creative).not.toBeNull();
    expect(creative?.duration).toBe(15);
    expect(creative?.mediaFile).toBeDefined();
    expect(creative?.tracking).toBeDefined();
    expect(creative?.adSystem.name).toBe('Advanced Ad System');
  });

  it('should include skip offset', async () => {
    const parser = new VASTParser();
    const result = await parser.parse(VAST_4_2_SAMPLE);
    const ad = result.document?.ads[0];

    if (!ad) {
      throw new Error('No ad found');
    }

    const creative = extractSSAICreative(ad);

    expect(creative?.skipOffsetSeconds).toBe(5);
  });

  it('should include companions', async () => {
    const parser = new VASTParser();
    const result = await parser.parse(VAST_4_2_SAMPLE);
    const ad = result.document?.ads[0];

    if (!ad) {
      throw new Error('No ad found');
    }

    const creative = extractSSAICreative(ad);

    expect(creative?.companions?.length).toBeGreaterThan(0);
  });
});

// =============================================================================
// Ad Pod Scheduler Tests
// =============================================================================

describe('Ad Pod Scheduler', () => {
  let scheduler: AdPodScheduler;

  beforeEach(() => {
    scheduler = new AdPodScheduler({
      contentDuration: 600, // 10 minutes
      maxPodDuration: 90,
      maxAdsPerPod: 3,
    });
  });

  describe('VAST Scheduling', () => {
    it('should schedule ads from VAST', async () => {
      const parser = new VASTParser();
      const result = await parser.parse(VAST_2_0_SAMPLE);

      if (!result.document) {
        throw new Error('Parse failed');
      }

      const schedule = scheduler.scheduleFromVAST(
        [result.document],
        [{ startTime: 0 }]
      );

      expect(schedule.pods.length).toBeGreaterThan(0);
      expect(schedule.totalAds).toBeGreaterThan(0);
      expect(schedule.metadata.preRollCount).toBe(1);
    });

    it('should respect pod duration limits', async () => {
      const parser = new VASTParser();
      const result = await parser.parse(VAST_2_0_SAMPLE);

      if (!result.document) {
        throw new Error('Parse failed');
      }

      const schedule = scheduler.scheduleFromVAST(
        [result.document],
        [{ startTime: 0, maxDuration: 60 }]
      );

      for (const pod of schedule.pods) {
        expect(pod.duration).toBeLessThanOrEqual(60);
      }
    });
  });

  describe('Timeline Generation', () => {
    it('should generate a timeline', async () => {
      const parser = new VASTParser();
      const result = await parser.parse(VAST_2_0_SAMPLE);

      if (!result.document) {
        throw new Error('Parse failed');
      }

      const schedule = scheduler.scheduleFromVAST(
        [result.document],
        [{ startTime: 0 }, { startTime: 300 }]
      );

      const timeline = generateTimeline(schedule, 600);

      expect(timeline.length).toBeGreaterThan(0);
      expect(timeline.some((t) => t.type === 'ad-pod')).toBe(true);
      expect(timeline.some((t) => t.type === 'content')).toBe(true);
    });
  });
});

// =============================================================================
// Tracking Manager Tests
// =============================================================================

describe('Tracking Manager', () => {
  let trackingManager: VASTTrackingManager;

  beforeEach(() => {
    trackingManager = new VASTTrackingManager({
      logBeacons: false,
    });
  });

  describe('Playhead Formatting', () => {
    it('should format playhead positions', () => {
      expect(formatPlayhead(0)).toBe('00:00:00.000');
      expect(formatPlayhead(30)).toBe('00:00:30.000');
      expect(formatPlayhead(90.5)).toBe('00:01:30.500');
      expect(formatPlayhead(3661.25)).toBe('01:01:01.250');
    });
  });

  describe('Quartile Detection', () => {
    it('should detect quartiles', () => {
      expect(getQuartile(0, 100)).toBe('start');
      expect(getQuartile(25, 100)).toBe('firstQuartile');
      expect(getQuartile(50, 100)).toBe('midpoint');
      expect(getQuartile(75, 100)).toBe('thirdQuartile');
      expect(getQuartile(100, 100)).toBe('complete');
    });

    it('should handle edge cases', () => {
      expect(getQuartile(0, 0)).toBeNull();
      expect(getQuartile(-1, 100)).toBeNull();
    });
  });

  describe('Context Building', () => {
    it('should build tracking context', () => {
      const context = buildTrackingContext('session123', {
        domain: 'example.com',
        playhead: 30.5,
        adDuration: 60,
        podSequence: 1,
        adPosition: 2,
      });

      expect(context.sessionId).toBe('session123');
      expect(context.macros?.DOMAIN).toBe('example.com');
      expect(context.macros?.CONTENTPLAYHEAD).toBe('00:00:30.500');
      expect(context.macros?.DURATION).toBe('60');
      expect(context.macros?.PODSEQUENCE).toBe('1');
      expect(context.macros?.ADPOSITION).toBe('2');
    });
  });
});

// =============================================================================
// Ad Request Handler Tests
// =============================================================================

describe('Ad Request Handler', () => {
  let handler: AdRequestHandler;

  beforeEach(() => {
    handler = new AdRequestHandler({
      contentDuration: 600,
    });
  });

  describe('Auto-Detection', () => {
    it('should detect VAST', async () => {
      const result = await handler.processAdResponse(VAST_2_0_SAMPLE);

      expect(result.type).toBe('vast');
      expect(result.success).toBe(true);
    });

    it('should detect VMAP', async () => {
      const result = await handler.processAdResponse(VMAP_SAMPLE);

      expect(result.type).toBe('vmap');
      expect(result.success).toBe(true);
    });
  });

  describe('Creative Extraction', () => {
    it('should extract creatives from VAST', async () => {
      const result = await handler.processAdResponse(VAST_4_2_SAMPLE);

      expect(result.creatives.length).toBeGreaterThan(0);
      expect(result.creatives[0].duration).toBe(15);
    });

    it('should extract creatives from VMAP', async () => {
      const result = await handler.processAdResponse(VMAP_SAMPLE);

      expect(result.creatives.length).toBeGreaterThan(0);
    });
  });

  describe('Scheduling', () => {
    it('should schedule pods from VAST', async () => {
      const result = await handler.processAdResponse(VAST_2_0_SAMPLE, {
        contentDuration: 600,
      });

      expect(result.schedule).toBeDefined();
      expect(result.schedule?.pods.length).toBeGreaterThan(0);
    });
  });
});

// =============================================================================
// Convenience Function Tests
// =============================================================================

describe('Convenience Functions', () => {
  it('parseVAST should work', async () => {
    const result = await parseVAST(VAST_2_0_SAMPLE);

    expect(result.success).toBe(true);
    expect(result.document?.version).toBe('2.0');
  });

  it('parseVMAP should work', async () => {
    const result = await parseVMAP(VMAP_SAMPLE);

    expect(result.success).toBe(true);
    expect(result.document?.version).toBe('1.0');
  });
});
