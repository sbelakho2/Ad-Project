/**
 * SSAI Player Monitor for Web SDK
 *
 * Monitors player state and automatically triggers tracking events
 * based on playback position and ad break detection.
 */

import {
  SSAIAdBreak,
  SSAIAd,
  SSAIPlayerEvents,
  SSAITrackingEventType,
} from './types';
import { SSAISessionClient } from './SSAISessionClient';
import { SSAIManifestParser } from './SSAIManifestParser';

export interface SSAIPlayerMonitorConfig {
  /** Update interval (ms) */
  updateIntervalMs: number;

  /** Quartile threshold tolerance (%) */
  quartileTolerancePercent: number;

  /** Auto-track events */
  autoTrack: boolean;
}

const DEFAULT_CONFIG: SSAIPlayerMonitorConfig = {
  updateIntervalMs: 250,
  quartileTolerancePercent: 1,
  autoTrack: true,
};

interface PlayerState {
  currentTime: number;
  duration: number;
  paused: boolean;
  muted: boolean;
  fullscreen: boolean;
}

interface AdState {
  inAdBreak: boolean;
  currentBreak: SSAIAdBreak | null;
  currentAd: SSAIAd | null;
  adStartTime: number;
  firedQuartiles: Set<SSAITrackingEventType>;
}

export class SSAIPlayerMonitor {
  private config: SSAIPlayerMonitorConfig;
  private sessionClient: SSAISessionClient;
  private manifestParser: SSAIManifestParser;
  private events: SSAIPlayerEvents;
  private updateTimer: ReturnType<typeof setInterval> | null = null;
  private playerElement: HTMLMediaElement | null = null;
  private adState: AdState = {
    inAdBreak: false,
    currentBreak: null,
    currentAd: null,
    adStartTime: 0,
    firedQuartiles: new Set(),
  };
  private lastPlayerState: PlayerState | null = null;

  constructor(
    sessionClient: SSAISessionClient,
    manifestParser: SSAIManifestParser,
    events: SSAIPlayerEvents = {},
    config: Partial<SSAIPlayerMonitorConfig> = {}
  ) {
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.sessionClient = sessionClient;
    this.manifestParser = manifestParser;
    this.events = events;
  }

  /**
   * Attach to a media element
   */
  attach(element: HTMLMediaElement): void {
    this.playerElement = element;

    // Add event listeners
    element.addEventListener('play', this.handlePlay.bind(this));
    element.addEventListener('pause', this.handlePause.bind(this));
    element.addEventListener('seeked', this.handleSeeked.bind(this));
    element.addEventListener('volumechange', this.handleVolumeChange.bind(this));
    element.addEventListener('ended', this.handleEnded.bind(this));
    element.addEventListener('error', this.handleError.bind(this));

    // Fullscreen events
    document.addEventListener('fullscreenchange', this.handleFullscreenChange.bind(this));

    // Start monitoring
    this.startMonitoring();
  }

  /**
   * Detach from media element
   */
  detach(): void {
    this.stopMonitoring();

    if (this.playerElement) {
      this.playerElement.removeEventListener('play', this.handlePlay.bind(this));
      this.playerElement.removeEventListener('pause', this.handlePause.bind(this));
      this.playerElement.removeEventListener('seeked', this.handleSeeked.bind(this));
      this.playerElement.removeEventListener('volumechange', this.handleVolumeChange.bind(this));
      this.playerElement.removeEventListener('ended', this.handleEnded.bind(this));
      this.playerElement.removeEventListener('error', this.handleError.bind(this));
      this.playerElement = null;
    }

    document.removeEventListener('fullscreenchange', this.handleFullscreenChange.bind(this));
  }

  /**
   * Start position monitoring
   */
  private startMonitoring(): void {
    this.updateTimer = setInterval(() => {
      this.update();
    }, this.config.updateIntervalMs);
  }

  /**
   * Stop position monitoring
   */
  private stopMonitoring(): void {
    if (this.updateTimer) {
      clearInterval(this.updateTimer);
      this.updateTimer = null;
    }
  }

  /**
   * Update loop - check for ad breaks and track progress
   */
  private update(): void {
    if (!this.playerElement) return;

    const state = this.getPlayerState();
    if (!state) return;

    // Check for ad break transitions
    const adBreak = this.manifestParser.getAdBreakAtTime(state.currentTime);

    if (adBreak && !this.adState.inAdBreak) {
      this.enterAdBreak(adBreak);
    } else if (!adBreak && this.adState.inAdBreak) {
      this.exitAdBreak();
    } else if (adBreak && this.adState.inAdBreak) {
      // Check for ad transitions within break
      this.updateAdProgress(adBreak, state.currentTime);
    }

    this.lastPlayerState = state;
  }

  /**
   * Get current player state
   */
  private getPlayerState(): PlayerState | null {
    if (!this.playerElement) return null;

    return {
      currentTime: this.playerElement.currentTime,
      duration: this.playerElement.duration,
      paused: this.playerElement.paused,
      muted: this.playerElement.muted,
      fullscreen: !!document.fullscreenElement,
    };
  }

  /**
   * Enter an ad break
   */
  private enterAdBreak(adBreak: SSAIAdBreak): void {
    this.adState.inAdBreak = true;
    this.adState.currentBreak = adBreak;
    this.adState.currentAd = adBreak.ads[0] || null;
    this.adState.adStartTime = adBreak.position;
    this.adState.firedQuartiles.clear();

    // Fire ad break start event
    this.events.onAdBreakStart?.(adBreak);

    // Fire first ad start
    if (this.adState.currentAd) {
      this.events.onAdStart?.(this.adState.currentAd, adBreak);

      // Track impression
      if (this.config.autoTrack) {
        this.sessionClient.trackImpression(
          adBreak.podId,
          this.adState.currentAd.slotId,
          this.adState.currentAd.creativeId,
          this.adState.adStartTime
        );
      }
    }
  }

  /**
   * Exit an ad break
   */
  private exitAdBreak(): void {
    const adBreak = this.adState.currentBreak;
    const lastAd = this.adState.currentAd;

    // Fire last ad end
    if (lastAd && adBreak) {
      this.events.onAdEnd?.(lastAd, adBreak);
    }

    // Fire ad break end
    if (adBreak) {
      this.events.onAdBreakEnd?.(adBreak);
    }

    // Reset state
    this.adState.inAdBreak = false;
    this.adState.currentBreak = null;
    this.adState.currentAd = null;
    this.adState.firedQuartiles.clear();
  }

  /**
   * Update ad progress and track quartiles
   */
  private updateAdProgress(adBreak: SSAIAdBreak, currentTime: number): void {
    const currentAd = this.manifestParser.getAdAtTime(adBreak, currentTime);

    // Check for ad transition
    if (currentAd && currentAd !== this.adState.currentAd) {
      // End previous ad
      if (this.adState.currentAd) {
        this.events.onAdEnd?.(this.adState.currentAd, adBreak);
      }

      // Start new ad
      this.adState.currentAd = currentAd;
      this.adState.adStartTime = this.getAdStartTime(adBreak, currentAd);
      this.adState.firedQuartiles.clear();

      this.events.onAdStart?.(currentAd, adBreak);

      // Track impression
      if (this.config.autoTrack) {
        this.sessionClient.trackImpression(
          adBreak.podId,
          currentAd.slotId,
          currentAd.creativeId,
          currentTime
        );
      }
    }

    // Track progress
    if (currentAd && this.config.autoTrack) {
      const adProgress = this.calculateAdProgress(currentAd, currentTime);
      this.trackQuartileProgress(adBreak, currentAd, adProgress, currentTime);

      // Fire progress callback
      this.events.onAdProgress?.(currentAd, adProgress);
    }
  }

  /**
   * Get the start time of an ad within a break
   */
  private getAdStartTime(adBreak: SSAIAdBreak, targetAd: SSAIAd): number {
    let startTime = adBreak.position;

    for (const ad of adBreak.ads) {
      if (ad === targetAd) {
        return startTime;
      }
      startTime += ad.duration;
    }

    return startTime;
  }

  /**
   * Calculate progress percentage for an ad
   */
  private calculateAdProgress(ad: SSAIAd, currentTime: number): number {
    const elapsed = currentTime - this.adState.adStartTime;
    if (ad.duration <= 0) return 0;
    return Math.min(100, (elapsed / ad.duration) * 100);
  }

  /**
   * Track quartile progress events
   */
  private trackQuartileProgress(
    adBreak: SSAIAdBreak,
    ad: SSAIAd,
    progress: number,
    mediaTime: number
  ): void {
    const quartiles: Array<{ threshold: number; type: SSAITrackingEventType }> = [
      { threshold: 0, type: 'start' },
      { threshold: 25, type: 'firstQuartile' },
      { threshold: 50, type: 'midpoint' },
      { threshold: 75, type: 'thirdQuartile' },
      { threshold: 100, type: 'complete' },
    ];

    for (const { threshold, type } of quartiles) {
      const adjustedThreshold = threshold - this.config.quartileTolerancePercent;

      if (
        progress >= adjustedThreshold &&
        !this.adState.firedQuartiles.has(type)
      ) {
        this.adState.firedQuartiles.add(type);

        this.sessionClient.trackProgress(
          adBreak.podId,
          ad.slotId,
          ad.creativeId,
          mediaTime,
          threshold
        );
      }
    }
  }

  /**
   * Handle play event
   */
  private handlePlay(): void {
    if (this.adState.inAdBreak && this.adState.currentAd && this.adState.currentBreak) {
      const state = this.getPlayerState();
      if (state && this.lastPlayerState?.paused) {
        // Resume from pause
        this.sessionClient.trackEvent({
          type: 'resume',
          podId: this.adState.currentBreak.podId,
          slotId: this.adState.currentAd.slotId,
          creativeId: this.adState.currentAd.creativeId,
          mediaTime: state.currentTime,
        });
      }
    }
  }

  /**
   * Handle pause event
   */
  private handlePause(): void {
    if (this.adState.inAdBreak && this.adState.currentAd && this.adState.currentBreak) {
      const state = this.getPlayerState();
      if (state) {
        this.sessionClient.trackEvent({
          type: 'pause',
          podId: this.adState.currentBreak.podId,
          slotId: this.adState.currentAd.slotId,
          creativeId: this.adState.currentAd.creativeId,
          mediaTime: state.currentTime,
        });
      }
    }
  }

  /**
   * Handle seek event
   */
  private handleSeeked(): void {
    // Seeking during ads is generally blocked by players
    // but we track it if it happens
  }

  /**
   * Handle volume change
   */
  private handleVolumeChange(): void {
    if (!this.adState.inAdBreak || !this.adState.currentAd || !this.adState.currentBreak) {
      return;
    }

    const state = this.getPlayerState();
    if (!state) return;

    const wasMuted = this.lastPlayerState?.muted || false;

    if (state.muted && !wasMuted) {
      this.sessionClient.trackEvent({
        type: 'mute',
        podId: this.adState.currentBreak.podId,
        slotId: this.adState.currentAd.slotId,
        creativeId: this.adState.currentAd.creativeId,
        mediaTime: state.currentTime,
      });
    } else if (!state.muted && wasMuted) {
      this.sessionClient.trackEvent({
        type: 'unmute',
        podId: this.adState.currentBreak.podId,
        slotId: this.adState.currentAd.slotId,
        creativeId: this.adState.currentAd.creativeId,
        mediaTime: state.currentTime,
      });
    }
  }

  /**
   * Handle fullscreen change
   */
  private handleFullscreenChange(): void {
    if (!this.adState.inAdBreak || !this.adState.currentAd || !this.adState.currentBreak) {
      return;
    }

    const state = this.getPlayerState();
    if (!state) return;

    const wasFullscreen = this.lastPlayerState?.fullscreen || false;

    if (state.fullscreen && !wasFullscreen) {
      this.sessionClient.trackEvent({
        type: 'fullscreen',
        podId: this.adState.currentBreak.podId,
        slotId: this.adState.currentAd.slotId,
        creativeId: this.adState.currentAd.creativeId,
        mediaTime: state.currentTime,
      });
    } else if (!state.fullscreen && wasFullscreen) {
      this.sessionClient.trackEvent({
        type: 'exitFullscreen',
        podId: this.adState.currentBreak.podId,
        slotId: this.adState.currentAd.slotId,
        creativeId: this.adState.currentAd.creativeId,
        mediaTime: state.currentTime,
      });
    }
  }

  /**
   * Handle playback ended
   */
  private handleEnded(): void {
    // Ensure all ads are properly ended
    if (this.adState.inAdBreak) {
      this.exitAdBreak();
    }
  }

  /**
   * Handle playback error
   */
  private handleError(): void {
    if (this.adState.inAdBreak && this.adState.currentAd && this.adState.currentBreak) {
      const state = this.getPlayerState();

      this.sessionClient.trackError(
        this.adState.currentBreak.podId,
        this.adState.currentAd.slotId,
        this.adState.currentAd.creativeId,
        'PLAYBACK_ERROR',
        'Media playback error during ad'
      );

      this.events.onAdError?.({
        code: 'PLAYBACK_ERROR',
        message: 'Media playback error during ad',
        fatal: false,
        podId: this.adState.currentBreak.podId,
        slotId: this.adState.currentAd.slotId,
      });
    }
  }

  /**
   * Handle ad skip (call from UI)
   */
  skipAd(): boolean {
    if (!this.adState.inAdBreak || !this.adState.currentAd || !this.adState.currentBreak) {
      return false;
    }

    const ad = this.adState.currentAd;

    // Check if skippable
    if (ad.skipOffset === undefined) {
      return false;
    }

    const state = this.getPlayerState();
    if (!state) return false;

    const elapsed = state.currentTime - this.adState.adStartTime;
    if (elapsed < ad.skipOffset) {
      return false;
    }

    // Track skip
    this.sessionClient.trackSkip(
      this.adState.currentBreak.podId,
      ad.slotId,
      ad.creativeId,
      state.currentTime
    );

    this.events.onAdSkipped?.(ad);

    return true;
  }

  /**
   * Handle ad click (call from UI)
   */
  clickAd(): string | null {
    if (!this.adState.inAdBreak || !this.adState.currentAd || !this.adState.currentBreak) {
      return null;
    }

    const ad = this.adState.currentAd;
    const state = this.getPlayerState();

    // Track click
    this.sessionClient.trackClick(
      this.adState.currentBreak.podId,
      ad.slotId,
      ad.creativeId,
      state?.currentTime || 0
    );

    this.events.onAdClicked?.(ad);

    return ad.clickThroughUrl || null;
  }

  /**
   * Get current ad state
   */
  getAdState(): AdState {
    return { ...this.adState };
  }

  /**
   * Check if currently in an ad break
   */
  isInAdBreak(): boolean {
    return this.adState.inAdBreak;
  }

  /**
   * Get current ad
   */
  getCurrentAd(): SSAIAd | null {
    return this.adState.currentAd;
  }

  /**
   * Get current ad break
   */
  getCurrentAdBreak(): SSAIAdBreak | null {
    return this.adState.currentBreak;
  }
}
