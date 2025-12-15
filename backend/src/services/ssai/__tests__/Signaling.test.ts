/**
 * SSAI Signaling Module Unit Tests
 */

import { SCTE35Parser } from '../signaling/SCTE35Parser';
import { HLSCueHandler } from '../signaling/HLSCueHandler';
import { DASHEventHandler } from '../signaling/DASHEventHandler';
import { PodPlanner } from '../signaling/PodPlanner';
import { VODMarkerProcessor } from '../signaling/VODMarkerProcessor';
import { DriftHandler } from '../signaling/DriftHandler';

describe('SCTE35Parser', () => {
  let parser: SCTE35Parser;

  beforeEach(() => {
    parser = new SCTE35Parser();
  });

  describe('parse', () => {
    it('should parse valid SCTE-35 base64', () => {
      // Valid SCTE-35 splice_insert message
      const base64 = '/DAlAAAAAAAAAP/wFAUAAAABf+//PafFJP4B////ACxGH0AAAMvUjAo=';

      const result = parser.parse(base64);

      expect(result).toBeDefined();
      expect(result.tableId).toBe(0xFC);
    });

    it('should throw on invalid base64', () => {
      expect(() => parser.parse('!!invalid!!')).toThrow();
    });
  });
});

describe('HLSCueHandler', () => {
  let handler: HLSCueHandler;

  beforeEach(() => {
    handler = new HLSCueHandler();
  });

  describe('constructor', () => {
    it('should create handler', () => {
      expect(handler).toBeDefined();
    });
  });
});

describe('DASHEventHandler', () => {
  let handler: DASHEventHandler;

  beforeEach(() => {
    handler = new DASHEventHandler();
  });

  describe('constructor', () => {
    it('should create handler', () => {
      expect(handler).toBeDefined();
    });
  });
});

describe('PodPlanner', () => {
  let planner: PodPlanner;

  beforeEach(() => {
    planner = new PodPlanner({
      defaultSlotDuration: 30,
    });
  });

  describe('constructor', () => {
    it('should create with default config', () => {
      const defaultPlanner = new PodPlanner();
      expect(defaultPlanner).toBeDefined();
    });

    it('should create with custom config', () => {
      expect(planner).toBeDefined();
    });
  });
});

describe('VODMarkerProcessor', () => {
  let processor: VODMarkerProcessor;

  beforeEach(() => {
    processor = new VODMarkerProcessor();
  });

  describe('constructor', () => {
    it('should create processor', () => {
      expect(processor).toBeDefined();
    });
  });
});

describe('DriftHandler', () => {
  let handler: DriftHandler;

  beforeEach(() => {
    handler = new DriftHandler();
  });

  describe('analyzeDrift', () => {
    it('should detect no drift', () => {
      const cue = { id: 'cue-1', startTimeMs: 0, durationMs: 30000, cueType: 'ad-start' as const };

      const analysis = handler.analyzeDrift(cue, 1000, 1000);

      expect(analysis.driftType).toBe('none');
      expect(analysis.action).toBe('proceed');
    });

    it('should detect late arrival', () => {
      const cue = { id: 'cue-2', startTimeMs: 0, durationMs: 30000, cueType: 'ad-start' as const };

      const analysis = handler.analyzeDrift(cue, 1000, 4000);

      expect(analysis.driftType).toBe('late');
    });

    it('should detect early arrival', () => {
      const cue = { id: 'cue-3', startTimeMs: 0, durationMs: 30000, cueType: 'ad-start' as const };

      const analysis = handler.analyzeDrift(cue, 10000, 4000);

      expect(analysis.driftType).toBe('early');
    });

    it('should recommend slate for severe drift', () => {
      const cue = { id: 'cue-4', startTimeMs: 0, durationMs: 30000, cueType: 'ad-start' as const };

      const analysis = handler.analyzeDrift(cue, 1000, 15000);

      expect(analysis.driftType).toBe('severe');
      expect(analysis.insertSlate).toBe(true);
    });
  });

  describe('generateSlateSegments', () => {
    it('should generate slate segments', () => {
      handler.setSlateUrl('https://cdn.example.com/slate.ts');

      const segments = handler.generateSlateSegments(45000);

      expect(segments.length).toBeGreaterThan(0);
    });
  });
});
