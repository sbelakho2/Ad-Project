import { describe, it, expect } from '@jest/globals';
import { SSAIManifestParser, ParseResult, AdCue } from '../../../src/ssai/SSAIManifestParser';

describe('SSAIManifestParser', () => {
  let parser: SSAIManifestParser;

  beforeEach(() => {
    parser = new SSAIManifestParser();
  });

  describe('HLS Parsing', () => {
    it('should parse basic HLS manifest', () => {
      const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXTINF:10.0,
segment1.ts
#EXTINF:10.0,
segment2.ts
#EXTINF:10.0,
segment3.ts
#EXT-X-ENDLIST`;

      const result = parser.parseHLS(manifest);

      expect(result.isLive).toBe(false);
      expect(result.targetDuration).toBe(10);
      expect(result.totalDuration).toBe(30);
      expect(result.adCues).toHaveLength(0);
    });

    it('should parse HLS manifest with EXT-X-DATERANGE ad cues', () => {
      const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXT-X-DATERANGE:ID="ad-1",CLASS="ssai-ad",START-DATE="2024-01-01T00:00:00.000Z",DURATION=30.0,X-POD-ID="pod-1"
#EXTINF:10.0,
ad1-segment1.ts
#EXTINF:10.0,
ad1-segment2.ts
#EXTINF:10.0,
ad1-segment3.ts
#EXTINF:10.0,
content1.ts
#EXTINF:10.0,
content2.ts
#EXT-X-ENDLIST`;

      const result = parser.parseHLS(manifest);

      expect(result.adCues).toHaveLength(1);
      expect(result.adCues[0]).toMatchObject({
        id: 'ad-1',
        podId: 'pod-1',
        duration: 30.0,
        cueType: 'preroll',
      });
    });

    it('should parse HLS manifest with EXT-X-CUE-OUT', () => {
      const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXTINF:10.0,
content1.ts
#EXTINF:10.0,
content2.ts
#EXT-X-CUE-OUT:30.0
#EXTINF:10.0,
ad1.ts
#EXTINF:10.0,
ad2.ts
#EXTINF:10.0,
ad3.ts
#EXT-X-CUE-IN
#EXTINF:10.0,
content3.ts
#EXT-X-ENDLIST`;

      const result = parser.parseHLS(manifest);

      expect(result.adCues).toHaveLength(1);
      expect(result.adCues[0]).toMatchObject({
        duration: 30.0,
        cueType: 'midroll',
      });
    });

    it('should detect live streams without EXT-X-ENDLIST', () => {
      const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXTINF:10.0,
segment1.ts
#EXTINF:10.0,
segment2.ts`;

      const result = parser.parseHLS(manifest);

      expect(result.isLive).toBe(true);
      expect(result.totalDuration).toBeNull();
    });

    it('should parse multiple ad breaks', () => {
      const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXT-X-DATERANGE:ID="preroll",CLASS="ssai-ad",START-DATE="2024-01-01T00:00:00.000Z",DURATION=15.0,X-POD-ID="pod-1"
#EXTINF:5.0,
preroll-1.ts
#EXTINF:5.0,
preroll-2.ts
#EXTINF:5.0,
preroll-3.ts
#EXTINF:10.0,
content1.ts
#EXTINF:10.0,
content2.ts
#EXT-X-DATERANGE:ID="midroll",CLASS="ssai-ad",START-DATE="2024-01-01T00:00:35.000Z",DURATION=30.0,X-POD-ID="pod-2"
#EXTINF:10.0,
midroll-1.ts
#EXTINF:10.0,
midroll-2.ts
#EXTINF:10.0,
midroll-3.ts
#EXTINF:10.0,
content3.ts
#EXT-X-ENDLIST`;

      const result = parser.parseHLS(manifest);

      expect(result.adCues).toHaveLength(2);
      expect(result.adCues[0].podId).toBe('pod-1');
      expect(result.adCues[0].cueType).toBe('preroll');
      expect(result.adCues[1].podId).toBe('pod-2');
      expect(result.adCues[1].cueType).toBe('midroll');
    });

    it('should handle malformed DATERANGE gracefully', () => {
      const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXT-X-DATERANGE:MALFORMED_DATA_HERE
#EXTINF:10.0,
segment1.ts
#EXT-X-ENDLIST`;

      const result = parser.parseHLS(manifest);

      expect(result.errors.length).toBeGreaterThanOrEqual(0);
      expect(result.adCues).toHaveLength(0);
    });
  });

  describe('DASH Parsing', () => {
    it('should parse basic DASH MPD', () => {
      const mpd = `<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT1H30M45S">
  <Period id="1">
    <AdaptationSet>
      <Representation id="1" bandwidth="1000000"/>
    </AdaptationSet>
  </Period>
</MPD>`;

      const result = parser.parseDASH(mpd);

      expect(result.isLive).toBe(false);
      expect(result.totalDuration).toBe(5445); // 1h30m45s in seconds
    });

    it('should parse DASH MPD with EventStream SCTE-35 cues', () => {
      const mpd = `<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic">
  <Period id="1">
    <EventStream schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="90000">
      <Event presentationTime="0" duration="2700000" id="ad-1">
        <Signal xmlns="urn:scte:scte35:2014:xml+bin">
          <Binary>/DAlAAAAAAAAAP/wFAUA...</Binary>
        </Signal>
      </Event>
    </EventStream>
    <AdaptationSet>
      <Representation id="1" bandwidth="1000000"/>
    </AdaptationSet>
  </Period>
</MPD>`;

      const result = parser.parseDASH(mpd);

      expect(result.adCues).toHaveLength(1);
      expect(result.adCues[0].id).toBe('ad-1');
    });

    it('should detect live DASH streams', () => {
      const mpd = `<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic">
  <Period id="1">
    <AdaptationSet>
      <Representation id="1" bandwidth="1000000"/>
    </AdaptationSet>
  </Period>
</MPD>`;

      const result = parser.parseDASH(mpd);

      expect(result.isLive).toBe(true);
    });
  });

  describe('ISO 8601 Duration Parsing', () => {
    it('should parse hours, minutes, and seconds', () => {
      const mpd = `<?xml version="1.0" encoding="UTF-8"?>
<MPD type="static" mediaPresentationDuration="PT2H30M15S">
  <Period id="1"/>
</MPD>`;

      const result = parser.parseDASH(mpd);

      expect(result.totalDuration).toBe(9015); // 2*3600 + 30*60 + 15
    });

    it('should parse minutes and seconds only', () => {
      const mpd = `<?xml version="1.0" encoding="UTF-8"?>
<MPD type="static" mediaPresentationDuration="PT45M30S">
  <Period id="1"/>
</MPD>`;

      const result = parser.parseDASH(mpd);

      expect(result.totalDuration).toBe(2730); // 45*60 + 30
    });

    it('should parse seconds only', () => {
      const mpd = `<?xml version="1.0" encoding="UTF-8"?>
<MPD type="static" mediaPresentationDuration="PT120S">
  <Period id="1"/>
</MPD>`;

      const result = parser.parseDASH(mpd);

      expect(result.totalDuration).toBe(120);
    });
  });

  describe('Ad Cue Types', () => {
    it('should identify preroll at content start', () => {
      const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXT-X-DATERANGE:ID="ad-1",CLASS="ssai-ad",START-DATE="2024-01-01T00:00:00.000Z",DURATION=30.0
#EXTINF:10.0,
ad1.ts
#EXTINF:10.0,
content1.ts
#EXT-X-ENDLIST`;

      const result = parser.parseHLS(manifest);

      expect(result.adCues[0].cueType).toBe('preroll');
    });

    it('should identify midroll in content middle', () => {
      const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXTINF:10.0,
content1.ts
#EXTINF:10.0,
content2.ts
#EXT-X-DATERANGE:ID="ad-1",CLASS="ssai-ad",START-DATE="2024-01-01T00:00:20.000Z",DURATION=30.0
#EXTINF:10.0,
ad1.ts
#EXT-X-ENDLIST`;

      const result = parser.parseHLS(manifest);

      expect(result.adCues[0].cueType).toBe('midroll');
    });

    it('should identify postroll marked in CLASS', () => {
      const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXTINF:10.0,
content1.ts
#EXT-X-DATERANGE:ID="ad-1",CLASS="ssai-ad-postroll",START-DATE="2024-01-01T00:00:10.000Z",DURATION=30.0
#EXTINF:10.0,
ad1.ts
#EXT-X-ENDLIST`;

      const result = parser.parseHLS(manifest);

      expect(result.adCues[0].cueType).toBe('postroll');
    });
  });
});
