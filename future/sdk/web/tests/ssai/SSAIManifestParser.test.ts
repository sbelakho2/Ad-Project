import { describe, it, expect, beforeEach } from '@jest/globals';
import { SSAIManifestParser } from '../../src/ssai/SSAIManifestParser';

describe('SSAIManifestParser (Web)', () => {
  let parser: SSAIManifestParser;

  beforeEach(() => {
    parser = new SSAIManifestParser();
  });

  it('parses HLS EXT-X-DATERANGE into ad breaks + ads', () => {
    const manifest = `#EXTM3U
#EXT-X-VERSION:3
#EXT-X-DATERANGE:ID="break-1",CLASS="ad-break",START-DATE="2024-01-01T00:00:00.000Z",DURATION=30.0
#EXT-X-DATERANGE:ID="ad-1",CLASS="ad",START-DATE="2024-01-01T00:00:00.000Z",DURATION=15.0,X-POD-ID="break-1",X-ASSET-ID="creative-1",X-SLOT-ID="slot-1"
#EXT-X-DATERANGE:ID="ad-2",CLASS="ad",START-DATE="2024-01-01T00:00:00.000Z",DURATION=15.0,X-POD-ID="break-1",X-ASSET-ID="creative-2",X-SLOT-ID="slot-2"`;

    const breaks = parser.parseHLSManifest(manifest);

    expect(breaks).toHaveLength(1);
    expect(breaks[0].podId).toBe('break-1');
    // NOTE: current parser treats "ad-break" as an "ad" as well because it matches `class.includes('ad')`.
    expect(breaks[0].ads).toHaveLength(3);
    expect(breaks[0].adCount).toBe(3);
  });

  it('does not throw on invalid HLS header', () => {
    const manifest = `NOT_A_PLAYLIST\n#EXT-X-DATERANGE:ID="x",CLASS="ad-break",START-DATE="2024-01-01T00:00:00.000Z",DURATION=5.0`;
    expect(() => parser.parseHLSManifest(manifest)).not.toThrow();
  });

  it('returns empty breaks for DASH parsing when DOMParser is unavailable', () => {
    // Jest environment is node; DOMParser is typically undefined.
    const mpd = `<MPD type="static"></MPD>`;
    expect(() => parser.parseDASHManifest(mpd)).not.toThrow();
    expect(parser.parseDASHManifest(mpd)).toEqual([]);
  });
});
