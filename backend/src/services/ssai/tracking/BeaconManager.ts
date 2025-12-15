/**
 * Beacon Manager
 * Manages tracking beacon creation, scheduling, and firing for SSAI
 */

export interface BeaconConfig {
  type: BeaconType;
  sessionId: string;
  podId: string;
  creativeId: string;
  url: string;
  triggerTime?: number;
}

export type BeaconType = 
  | 'impression'
  | 'start'
  | 'firstQuartile'
  | 'midpoint'
  | 'thirdQuartile'
  | 'complete'
  | 'click'
  | 'skip'
  | 'mute'
  | 'unmute'
  | 'pause'
  | 'resume'
  | 'rewind'
  | 'fullscreen'
  | 'exitFullscreen';

export interface Beacon {
  id: string;
  type: BeaconType;
  sessionId: string;
  podId: string;
  creativeId: string;
  url: string;
  triggerTime: number;
  createdAt: number;
  firedAt?: number;
}

export interface QuartileBeaconConfig {
  sessionId: string;
  podId: string;
  creativeId: string;
  duration: number;
  urls: {
    start?: string;
    firstQuartile?: string;
    midpoint?: string;
    thirdQuartile?: string;
    complete?: string;
  };
}

export class BeaconManager {
  private beacons = new Map<string, Beacon>();
  private firedBeacons = new Set<string>();
  private idCounter = 0;

  /**
   * Create a new beacon
   */
  createBeacon(config: BeaconConfig): Beacon {
    const beacon: Beacon = {
      id: this.generateId(),
      type: config.type,
      sessionId: config.sessionId,
      podId: config.podId,
      creativeId: config.creativeId,
      url: config.url,
      triggerTime: config.triggerTime ?? 0,
      createdAt: Date.now(),
    };

    this.beacons.set(beacon.id, beacon);
    return beacon;
  }

  /**
   * Create quartile beacons for a creative
   */
  createQuartileBeacons(config: QuartileBeaconConfig): Beacon[] {
    const beacons: Beacon[] = [];
    const quartileTimes = {
      start: 0,
      firstQuartile: config.duration * 0.25,
      midpoint: config.duration * 0.5,
      thirdQuartile: config.duration * 0.75,
      complete: config.duration,
    };

    if (config.urls.start) {
      beacons.push(this.createBeacon({
        type: 'start',
        sessionId: config.sessionId,
        podId: config.podId,
        creativeId: config.creativeId,
        url: config.urls.start,
        triggerTime: quartileTimes.start,
      }));
    }

    if (config.urls.firstQuartile) {
      beacons.push(this.createBeacon({
        type: 'firstQuartile',
        sessionId: config.sessionId,
        podId: config.podId,
        creativeId: config.creativeId,
        url: config.urls.firstQuartile,
        triggerTime: quartileTimes.firstQuartile,
      }));
    }

    if (config.urls.midpoint) {
      beacons.push(this.createBeacon({
        type: 'midpoint',
        sessionId: config.sessionId,
        podId: config.podId,
        creativeId: config.creativeId,
        url: config.urls.midpoint,
        triggerTime: quartileTimes.midpoint,
      }));
    }

    if (config.urls.thirdQuartile) {
      beacons.push(this.createBeacon({
        type: 'thirdQuartile',
        sessionId: config.sessionId,
        podId: config.podId,
        creativeId: config.creativeId,
        url: config.urls.thirdQuartile,
        triggerTime: quartileTimes.thirdQuartile,
      }));
    }

    if (config.urls.complete) {
      beacons.push(this.createBeacon({
        type: 'complete',
        sessionId: config.sessionId,
        podId: config.podId,
        creativeId: config.creativeId,
        url: config.urls.complete,
        triggerTime: quartileTimes.complete,
      }));
    }

    return beacons;
  }

  /**
   * Check if beacon should fire at given playhead position
   */
  shouldFire(beacon: Beacon, playheadPosition: number): boolean {
    // Already fired
    if (this.firedBeacons.has(beacon.id)) {
      return false;
    }

    // Check if playhead has passed trigger time
    return playheadPosition >= beacon.triggerTime;
  }

  /**
   * Mark beacon as fired
   */
  markFired(beaconId: string): void {
    this.firedBeacons.add(beaconId);
    const beacon = this.beacons.get(beaconId);
    if (beacon) {
      beacon.firedAt = Date.now();
    }
  }

  /**
   * Get pending beacons for a session
   */
  getPendingBeacons(sessionId: string, playheadPosition: number): Beacon[] {
    return Array.from(this.beacons.values())
      .filter(b => 
        b.sessionId === sessionId && 
        !this.firedBeacons.has(b.id) &&
        b.triggerTime <= playheadPosition
      );
  }

  /**
   * Get beacon by ID
   */
  getBeacon(beaconId: string): Beacon | undefined {
    return this.beacons.get(beaconId);
  }

  /**
   * Clean up beacons for ended session
   */
  cleanupSession(sessionId: string): void {
    for (const [id, beacon] of this.beacons) {
      if (beacon.sessionId === sessionId) {
        this.beacons.delete(id);
        this.firedBeacons.delete(id);
      }
    }
  }

  private generateId(): string {
    return `beacon-${Date.now()}-${++this.idCounter}`;
  }
}

export default BeaconManager;
