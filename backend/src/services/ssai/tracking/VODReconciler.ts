/**
 * SSAI VOD Reconciler (test-facing)
 */

export interface VODReconcilerConfig {
  toleranceMs: number;
}

export class VODReconciler {
  private readonly config: VODReconcilerConfig;

  constructor(config: VODReconcilerConfig) {
    this.config = config;
  }

  calculateExpectedQuartiles(adDuration: number): {
    start: number;
    firstQuartile: number;
    midpoint: number;
    thirdQuartile: number;
    complete: number;
  } {
    return {
      start: 0,
      firstQuartile: adDuration * 0.25,
      midpoint: adDuration * 0.5,
      thirdQuartile: adDuration * 0.75,
      complete: adDuration,
    };
  }

  reconcilePlayback(playback: {
    adDuration: number;
    currentTime: number;
    firedEvents: string[];
    seekEvents?: Array<{ from: number; to: number }>;
  }): {
    missingEvents: string[];
    seekSkipped: string[];
  } {
    const expectedTimes = this.calculateExpectedQuartiles(playback.adDuration);

    const expectedByNow: Array<{ type: string; time: number }> = [
      { type: 'start', time: expectedTimes.start },
      { type: 'firstQuartile', time: expectedTimes.firstQuartile },
      { type: 'midpoint', time: expectedTimes.midpoint },
      { type: 'thirdQuartile', time: expectedTimes.thirdQuartile },
      { type: 'complete', time: expectedTimes.complete },
    ].filter((q) => q.time <= playback.currentTime + this.config.toleranceMs / 1000);

    const fired = new Set(playback.firedEvents);
    const missing = expectedByNow.map((q) => q.type).filter((t) => !fired.has(t));

    const seekSkipped: string[] = [];
    if (playback.seekEvents?.length) {
      for (const seek of playback.seekEvents) {
        const from = Math.min(seek.from, seek.to);
        const to = Math.max(seek.from, seek.to);

        for (const q of expectedByNow) {
          if (q.time > from && q.time <= to) {
            if (!fired.has(q.type) && !seekSkipped.includes(q.type)) {
              seekSkipped.push(q.type);
            }
          }
        }
      }
    }

    const missingEvents = missing.filter((t) => !seekSkipped.includes(t));

    return { missingEvents, seekSkipped };
  }
}
