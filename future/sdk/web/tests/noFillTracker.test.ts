/**
 * NoFillTracker Tests
 */
import {
  NoFillTracker,
  NoFillReason,
  PatternType,
  PatternSeverity,
  NoFillAnalyzer,
  NoFillPattern,
} from '../src/noFillTracker';

describe('NoFillTracker', () => {
  let tracker: NoFillTracker;

  beforeEach(() => {
    NoFillTracker.resetInstance();
    tracker = new NoFillTracker();
  });

  describe('Event Recording', () => {
    it('should record a no-fill event', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);

      const stats = tracker.getStats();
      expect(stats.totalNoFills).toBe(1);
    });

    it('should record multiple events', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      tracker.recordNoFill('source2', 'placement2', NoFillReason.NO_INVENTORY);
      tracker.recordNoFill('source1', 'placement3', NoFillReason.NETWORK_ERROR);

      const stats = tracker.getStats();
      expect(stats.totalNoFills).toBe(3);
    });

    it('should track latency', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT, 100);
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT, 200);
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT, 300);

      const stats = tracker.getStats();
      expect(stats.averageLatencyMs).toBe(200);
    });

    it('should store metadata', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT, 0, {
        requestId: 'req123',
        adFormat: 'banner',
      });

      const events = tracker.getRecentEvents(1);
      expect(events[0].metadata.requestId).toBe('req123');
      expect(events[0].metadata.adFormat).toBe('banner');
    });
  });

  describe('Statistics', () => {
    it('should track no-fills by source', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      tracker.recordNoFill('source1', 'placement2', NoFillReason.TIMEOUT);
      tracker.recordNoFill('source2', 'placement1', NoFillReason.TIMEOUT);

      const bySource = tracker.getNoFillsBySource();
      expect(bySource.get('source1')).toBe(2);
      expect(bySource.get('source2')).toBe(1);
    });

    it('should track no-fills by placement', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      tracker.recordNoFill('source2', 'placement1', NoFillReason.TIMEOUT);
      tracker.recordNoFill('source1', 'placement2', NoFillReason.TIMEOUT);

      const byPlacement = tracker.getNoFillsByPlacement();
      expect(byPlacement.get('placement1')).toBe(2);
      expect(byPlacement.get('placement2')).toBe(1);
    });

    it('should track no-fills by reason', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      tracker.recordNoFill('source1', 'placement1', NoFillReason.NO_INVENTORY);

      const byReason = tracker.getNoFillsByReason();
      expect(byReason.get(NoFillReason.TIMEOUT)).toBe(2);
      expect(byReason.get(NoFillReason.NO_INVENTORY)).toBe(1);
    });

    it('should track hourly breakdown', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);

      const hourly = tracker.getHourlyBreakdown();
      const currentHour = new Date().getHours();
      expect(hourly.get(currentHour)).toBe(1);
    });

    it('should track daily breakdown', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);

      const daily = tracker.getDailyBreakdown();
      const currentDay = new Date().getDay();
      expect(daily.get(currentDay)).toBe(1);
    });

    it('should return top reasons sorted by count', () => {
      for (let i = 0; i < 5; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      }
      for (let i = 0; i < 3; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.NO_INVENTORY);
      }
      tracker.recordNoFill('source1', 'placement1', NoFillReason.NETWORK_ERROR);

      const stats = tracker.getStats();
      const reasonsArray = [...stats.topReasons.entries()];
      expect(reasonsArray[0][0]).toBe(NoFillReason.TIMEOUT);
      expect(reasonsArray[0][1]).toBe(5);
    });
  });

  describe('Fill Recording', () => {
    it('should reset consecutive failures on fill', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);

      // Record a fill
      tracker.recordFill('source1');

      // Record another no-fill - should start fresh count
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);

      // No pattern should be detected for just 1 consecutive failure
      const patterns = tracker.getDetectedPatterns();
      const consecutivePatterns = patterns.filter(
        (p) => p.type === PatternType.CONSECUTIVE_FAILURES
      );
      expect(consecutivePatterns.length).toBe(0);
    });
  });

  describe('Event Retrieval', () => {
    it('should get recent events', () => {
      for (let i = 0; i < 10; i++) {
        tracker.recordNoFill(`source${i}`, 'placement1', NoFillReason.TIMEOUT);
      }

      const recent = tracker.getRecentEvents(5);
      expect(recent.length).toBe(5);
      expect(recent[4].sourceId).toBe('source9');
    });

    it('should get events in time range', () => {
      const now = Date.now();
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);

      const events = tracker.getEvents(now - 1000, now + 1000);
      expect(events.length).toBe(1);
    });

    it('should filter events outside time range', () => {
      const now = Date.now();
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);

      const events = tracker.getEvents(now + 1000, now + 2000);
      expect(events.length).toBe(0);
    });
  });

  describe('Pattern Detection', () => {
    it('should detect consecutive failures', () => {
      // Default threshold is 5
      for (let i = 0; i < 5; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      }

      const patterns = tracker.getDetectedPatterns();
      const consecutivePattern = patterns.find(
        (p) => p.type === PatternType.CONSECUTIVE_FAILURES
      );

      expect(consecutivePattern).toBeDefined();
      expect(consecutivePattern?.affectedSourceId).toBe('source1');
    });

    it('should use configurable threshold for consecutive failures', () => {
      const customTracker = new NoFillTracker({
        consecutiveFailureThreshold: 3,
      });

      for (let i = 0; i < 3; i++) {
        customTracker.recordNoFill(
          'source1',
          'placement1',
          NoFillReason.TIMEOUT
        );
      }

      const patterns = customTracker.getDetectedPatterns();
      expect(patterns.length).toBeGreaterThan(0);
    });

    it('should detect elevated source rate', () => {
      // Record 15 events, all from one source (100% rate, > threshold)
      for (let i = 0; i < 15; i++) {
        tracker.recordNoFill('badSource', 'placement1', NoFillReason.TIMEOUT);
      }

      const patterns = tracker.getDetectedPatterns();
      const sourcePattern = patterns.find(
        (p) => p.type === PatternType.SOURCE_SPECIFIC
      );

      expect(sourcePattern).toBeDefined();
      expect(sourcePattern?.affectedSourceId).toBe('badSource');
    });

    it('should detect placement-specific issues', () => {
      // Record 25 events for one placement (should trigger > 30% threshold)
      for (let i = 0; i < 25; i++) {
        tracker.recordNoFill('source1', 'badPlacement', NoFillReason.TIMEOUT);
      }

      const patterns = tracker.getDetectedPatterns();
      const placementPattern = patterns.find(
        (p) => p.type === PatternType.PLACEMENT_SPECIFIC
      );

      expect(placementPattern).toBeDefined();
      expect(placementPattern?.affectedPlacementId).toBe('badPlacement');
    });

    it('should not duplicate patterns within 5 minutes', () => {
      for (let i = 0; i < 10; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      }

      const patterns = tracker.getDetectedPatterns();
      const consecutivePatterns = patterns.filter(
        (p) =>
          p.type === PatternType.CONSECUTIVE_FAILURES &&
          p.affectedSourceId === 'source1'
      );

      expect(consecutivePatterns.length).toBe(1);
    });
  });

  describe('Pattern Listeners', () => {
    it('should notify pattern listeners', () => {
      const detectedPatterns: NoFillPattern[] = [];
      tracker.addPatternListener((pattern) => {
        detectedPatterns.push(pattern);
      });

      for (let i = 0; i < 5; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      }

      expect(detectedPatterns.length).toBeGreaterThan(0);
    });

    it('should allow removing pattern listeners', () => {
      const detectedPatterns: NoFillPattern[] = [];
      const listener = (pattern: NoFillPattern) => {
        detectedPatterns.push(pattern);
      };

      tracker.addPatternListener(listener);
      tracker.removePatternListener(listener);

      for (let i = 0; i < 5; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      }

      expect(detectedPatterns.length).toBe(0);
    });

    it('should handle listener errors gracefully', () => {
      tracker.addPatternListener(() => {
        throw new Error('Listener error');
      });

      // Should not throw
      for (let i = 0; i < 5; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      }

      expect(true).toBe(true);
    });
  });

  describe('Configuration', () => {
    it('should use default configuration', () => {
      const config = tracker.getConfiguration();
      expect(config.maxEventsRetained).toBe(10000);
      expect(config.maxRetentionHours).toBe(24);
      expect(config.elevatedRateThreshold).toBe(0.5);
    });

    it('should allow custom configuration', () => {
      const customTracker = new NoFillTracker({
        maxEventsRetained: 5000,
        elevatedRateThreshold: 0.3,
      });

      const config = customTracker.getConfiguration();
      expect(config.maxEventsRetained).toBe(5000);
      expect(config.elevatedRateThreshold).toBe(0.3);
    });

    it('should update configuration', () => {
      tracker.updateConfiguration({ maxEventsRetained: 1000 });

      const config = tracker.getConfiguration();
      expect(config.maxEventsRetained).toBe(1000);
    });

    it('should disable pattern detection when configured', () => {
      const noPatternTracker = new NoFillTracker({
        patternDetectionEnabled: false,
      });

      for (let i = 0; i < 10; i++) {
        noPatternTracker.recordNoFill(
          'source1',
          'placement1',
          NoFillReason.TIMEOUT
        );
      }

      const patterns = noPatternTracker.getDetectedPatterns();
      expect(patterns.length).toBe(0);
    });
  });

  describe('Cleanup', () => {
    it('should enforce max events retained', () => {
      const smallTracker = new NoFillTracker({ maxEventsRetained: 5 });

      for (let i = 0; i < 10; i++) {
        smallTracker.recordNoFill(`source${i}`, 'placement1', NoFillReason.TIMEOUT);
      }

      const events = smallTracker.getRecentEvents(100);
      expect(events.length).toBeLessThanOrEqual(5);
    });

    it('should clear all data', () => {
      for (let i = 0; i < 5; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      }

      tracker.clear();

      const stats = tracker.getStats();
      expect(stats.totalNoFills).toBe(0);
      expect(stats.averageLatencyMs).toBe(0);
      expect(tracker.getRecentEvents().length).toBe(0);
      expect(tracker.getDetectedPatterns().length).toBe(0);
    });
  });

  describe('Singleton', () => {
    it('should return same instance', () => {
      const instance1 = NoFillTracker.getInstance();
      const instance2 = NoFillTracker.getInstance();

      expect(instance1).toBe(instance2);
    });

    it('should allow resetting instance', () => {
      const instance1 = NoFillTracker.getInstance();
      NoFillTracker.resetInstance();
      const instance2 = NoFillTracker.getInstance();

      expect(instance1).not.toBe(instance2);
    });
  });
});

describe('NoFillAnalyzer', () => {
  let tracker: NoFillTracker;
  let analyzer: NoFillAnalyzer;

  beforeEach(() => {
    NoFillTracker.resetInstance();
    tracker = new NoFillTracker();
    analyzer = new NoFillAnalyzer(tracker);
  });

  describe('Analysis', () => {
    it('should return healthy summary when no issues', () => {
      tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);

      const analysis = analyzer.analyze();
      expect(analysis.healthScore).toBeGreaterThanOrEqual(80);
      expect(analysis.summary).toContain('healthy');
    });

    it('should identify problematic sources', () => {
      // Create a source with > 30% of no-fills
      for (let i = 0; i < 5; i++) {
        tracker.recordNoFill('badSource', 'placement1', NoFillReason.TIMEOUT);
      }

      const analysis = analyzer.analyze();
      expect(analysis.problematicSources).toContain('badSource');
    });

    it('should identify problematic placements', () => {
      // Create a placement with > 30% of no-fills
      for (let i = 0; i < 5; i++) {
        tracker.recordNoFill('source1', 'badPlacement', NoFillReason.TIMEOUT);
      }

      const analysis = analyzer.analyze();
      expect(analysis.problematicPlacements).toContain('badPlacement');
    });

    it('should provide recommendations for timeout issues', () => {
      for (let i = 0; i < 10; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.TIMEOUT);
      }

      const analysis = analyzer.analyze();
      const timeoutRecommendation = analysis.recommendations.find((r) =>
        r.toLowerCase().includes('timeout')
      );
      expect(timeoutRecommendation).toBeDefined();
    });

    it('should provide recommendations for no inventory', () => {
      for (let i = 0; i < 10; i++) {
        tracker.recordNoFill('source1', 'placement1', NoFillReason.NO_INVENTORY);
      }

      const analysis = analyzer.analyze();
      const inventoryRecommendation = analysis.recommendations.find((r) =>
        r.toLowerCase().includes('demand')
      );
      expect(inventoryRecommendation).toBeDefined();
    });

    it('should reduce health score for critical patterns', () => {
      // Create many consecutive failures to trigger critical pattern
      for (let i = 0; i < 20; i++) {
        tracker.recordNoFill('badSource', 'placement1', NoFillReason.TIMEOUT);
      }

      const analysis = analyzer.analyze();
      expect(analysis.healthScore).toBeLessThan(100);
    });

    it('should use singleton tracker if none provided', () => {
      const defaultAnalyzer = new NoFillAnalyzer();
      const analysis = defaultAnalyzer.analyze();

      expect(analysis.healthScore).toBeDefined();
    });
  });
});

describe('NoFillReason', () => {
  it('should have all expected values', () => {
    expect(NoFillReason.TIMEOUT).toBe('timeout');
    expect(NoFillReason.NO_INVENTORY).toBe('no_inventory');
    expect(NoFillReason.NETWORK_ERROR).toBe('network_error');
    expect(NoFillReason.POLICY_VIOLATION).toBe('policy_violation');
    expect(NoFillReason.FREQUENCY_CAP).toBe('frequency_cap');
    expect(NoFillReason.GEOGRAPHIC_RESTRICTION).toBe('geographic_restriction');
    expect(NoFillReason.BUDGET_EXHAUSTED).toBe('budget_exhausted');
    expect(NoFillReason.MALFORMED_RESPONSE).toBe('malformed_response');
    expect(NoFillReason.SERVER_ERROR).toBe('server_error');
    expect(NoFillReason.UNKNOWN).toBe('unknown');
  });
});

describe('PatternType', () => {
  it('should have all expected values', () => {
    expect(PatternType.ELEVATED_RATE).toBe('elevated_rate');
    expect(PatternType.SOURCE_SPECIFIC).toBe('source_specific');
    expect(PatternType.PLACEMENT_SPECIFIC).toBe('placement_specific');
    expect(PatternType.CONSECUTIVE_FAILURES).toBe('consecutive_failures');
  });
});

describe('PatternSeverity', () => {
  it('should have all expected values', () => {
    expect(PatternSeverity.LOW).toBe('low');
    expect(PatternSeverity.MEDIUM).toBe('medium');
    expect(PatternSeverity.HIGH).toBe('high');
    expect(PatternSeverity.CRITICAL).toBe('critical');
  });
});
