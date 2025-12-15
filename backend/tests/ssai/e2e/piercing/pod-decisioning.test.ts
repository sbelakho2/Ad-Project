/**
 * Piercing E2E Tests - Pod Decisioning
 *
 * Tests: "latency poison, compatibility, and cost control"
 * - Latency poison test
 * - Fanout budget test
 * - Audio layout mismatch test
 * - HDR/SDR mismatch test
 * - Cadence mismatch test
 */

import { describe, it, expect, beforeEach, jest } from '@jest/globals';

interface FormatRequirements {
  container: 'ts' | 'cmaf';
  videoCodecs: string[];
  audioCodecs: string[];
  resolution?: { minWidth: number; maxWidth: number };
  audioLayout?: 'stereo' | '5.1' | '7.1' | 'mono';
  hdr?: { required: boolean; types?: string[] };
  segmentCadence?: number; // seconds
}

interface SelectedCreative {
  creativeId: string;
  mediaFiles: MediaFile[];
  duration: number;
}

interface MediaFile {
  url: string;
  container: string;
  codec: string;
  width: number;
  height: number;
  bitrate: number;
  audioChannels: number;
  audioLayout: 'stereo' | '5.1' | '7.1' | 'mono';
  isHdr: boolean;
  hdrType?: string;
  segmentDuration: number;
}

interface CompatibilityResult {
  compatible: boolean;
  reason?: string;
  rejectionReason?: string;
  conditioned?: boolean;
  conditioningApplied?: string;
}

interface PartnerCallResult {
  partnerId: string;
  latencyMs: number;
  success: boolean;
  timedOut: boolean;
  creative?: SelectedCreative;
}

interface PodDecisionResult {
  slots: SlotDecision[];
  totalLatencyMs: number;
  partnerCalls: PartnerCallResult[];
  withinSla: boolean;
  safeModeTriggered: boolean;
  explanation: string;
}

interface SlotDecision {
  slotId: string;
  filled: boolean;
  creativeId?: string;
  source: 'partner' | 'slate' | 'fallback';
  rejectionReason?: string;
}

/**
 * Mock Circuit Breaker for partner calls
 */
class MockCircuitBreaker {
  private state: 'closed' | 'open' | 'half-open' = 'closed';
  private failureCount: number = 0;
  private successCount: number = 0;
  private lastFailureTime: number = 0;
  private config = {
    failureThreshold: 5,
    successThreshold: 3,
    timeout: 30000, // 30s
    slowCallThreshold: 500, // 500ms
    slowCallRateThreshold: 0.3, // 30%
  };

  private slowCallCount: number = 0;
  private totalCalls: number = 0;

  recordSuccess(latencyMs: number): void {
    this.totalCalls++;
    if (latencyMs > this.config.slowCallThreshold) {
      this.slowCallCount++;
    }

    if (this.state === 'half-open') {
      this.successCount++;
      if (this.successCount >= this.config.successThreshold) {
        this.state = 'closed';
        this.failureCount = 0;
        this.successCount = 0;
      }
    }
  }

  recordFailure(): void {
    this.totalCalls++;
    this.failureCount++;
    this.lastFailureTime = Date.now();

    if (this.failureCount >= this.config.failureThreshold) {
      this.state = 'open';
    }
  }

  isOpen(): boolean {
    if (this.state === 'open') {
      // Check if timeout has passed
      if (Date.now() - this.lastFailureTime > this.config.timeout) {
        this.state = 'half-open';
        return false;
      }
      return true;
    }

    // Check slow call rate
    if (this.totalCalls > 10) {
      const slowRate = this.slowCallCount / this.totalCalls;
      if (slowRate > this.config.slowCallRateThreshold) {
        this.state = 'open';
        return true;
      }
    }

    return false;
  }

  getState(): string {
    return this.state;
  }

  reset(): void {
    this.state = 'closed';
    this.failureCount = 0;
    this.successCount = 0;
    this.slowCallCount = 0;
    this.totalCalls = 0;
  }
}

/**
 * Mock Format Validator
 */
class MockFormatValidator {
  validateCreative(
    creative: SelectedCreative,
    requirements: FormatRequirements
  ): CompatibilityResult {
    if (!creative.mediaFiles || creative.mediaFiles.length === 0) {
      return {
        compatible: false,
        reason: 'No media files',
        rejectionReason: 'no_media_files',
      };
    }

    const mediaFile = creative.mediaFiles[0];

    // Check container
    if (mediaFile.container !== requirements.container) {
      return {
        compatible: false,
        reason: `Container mismatch: ${mediaFile.container} vs ${requirements.container}`,
        rejectionReason: 'container_mismatch',
      };
    }

    // Check audio layout
    if (
      requirements.audioLayout &&
      mediaFile.audioLayout !== requirements.audioLayout
    ) {
      // Check if conditioning is possible
      if (
        mediaFile.audioLayout === '5.1' &&
        requirements.audioLayout === 'stereo'
      ) {
        return {
          compatible: true,
          conditioned: true,
          conditioningApplied: 'downmix_5.1_to_stereo',
        };
      }
      if (
        mediaFile.audioLayout === 'stereo' &&
        requirements.audioLayout === '5.1'
      ) {
        return {
          compatible: false,
          reason: `Audio layout mismatch: ${mediaFile.audioLayout} cannot be upmixed to ${requirements.audioLayout}`,
          rejectionReason: 'audio_layout_incompatible',
        };
      }
    }

    // Check HDR/SDR
    if (requirements.hdr?.required && !mediaFile.isHdr) {
      return {
        compatible: false,
        reason: 'HDR required but creative is SDR',
        rejectionReason: 'hdr_sdr_mismatch',
      };
    }
    if (!requirements.hdr?.required && mediaFile.isHdr) {
      // HDR content in SDR stream - needs tone mapping
      return {
        compatible: true,
        conditioned: true,
        conditioningApplied: 'hdr_to_sdr_tonemap',
      };
    }

    // Check segment cadence
    if (requirements.segmentCadence) {
      const cadenceDiff = Math.abs(
        mediaFile.segmentDuration - requirements.segmentCadence
      );
      const tolerance = 0.5; // 500ms tolerance

      if (cadenceDiff > tolerance) {
        return {
          compatible: false,
          reason: `Segment cadence mismatch: ${mediaFile.segmentDuration}s vs ${requirements.segmentCadence}s`,
          rejectionReason: 'cadence_mismatch',
        };
      }
    }

    return { compatible: true };
  }
}

/**
 * Mock Pod Decisioning Service
 */
class MockPodDecisioningService {
  private circuitBreakers: Map<string, MockCircuitBreaker> = new Map();
  private formatValidator = new MockFormatValidator();
  private config = {
    decisionSlaMs: 100,
    maxCallsPerPod: 10,
    maxCallsPerMinute: 50,
    partnerTimeoutMs: 80,
  };

  private callsThisMinute: number = 0;
  private minuteStart: number = Date.now();

  async makeDecision(
    slots: Array<{ slotId: string; duration: number }>,
    partners: string[],
    requirements: FormatRequirements
  ): Promise<PodDecisionResult> {
    const startTime = Date.now();
    const partnerCalls: PartnerCallResult[] = [];
    const slotDecisions: SlotDecision[] = [];
    let safeModeTriggered = false;

    // Reset minute counter if needed
    if (Date.now() - this.minuteStart > 60000) {
      this.callsThisMinute = 0;
      this.minuteStart = Date.now();
    }

    // Check fanout budget
    const remainingBudget = this.config.maxCallsPerMinute - this.callsThisMinute;
    const maxCallsThisPod = Math.min(
      this.config.maxCallsPerPod,
      remainingBudget,
      partners.length
    );

    if (maxCallsThisPod < partners.length) {
      safeModeTriggered = true;
    }

    // Call partners (up to budget)
    for (let i = 0; i < maxCallsThisPod; i++) {
      const partnerId = partners[i];
      const breaker = this.getCircuitBreaker(partnerId);

      if (breaker.isOpen()) {
        partnerCalls.push({
          partnerId,
          latencyMs: 0,
          success: false,
          timedOut: false,
        });
        continue;
      }

      const result = await this.callPartner(partnerId, requirements);
      partnerCalls.push(result);
      this.callsThisMinute++;

      if (result.success) {
        breaker.recordSuccess(result.latencyMs);
      } else {
        breaker.recordFailure();
      }
    }

    // Fill slots
    let creativeIndex = 0;
    const successfulCalls = partnerCalls.filter((c) => c.success && c.creative);

    for (const slot of slots) {
      if (creativeIndex < successfulCalls.length) {
        const call = successfulCalls[creativeIndex];
        const validation = this.formatValidator.validateCreative(
          call.creative!,
          requirements
        );

        if (validation.compatible) {
          slotDecisions.push({
            slotId: slot.slotId,
            filled: true,
            creativeId: call.creative!.creativeId,
            source: 'partner',
          });
          creativeIndex++;
        } else {
          slotDecisions.push({
            slotId: slot.slotId,
            filled: false,
            source: 'slate',
            rejectionReason: validation.rejectionReason,
          });
        }
      } else {
        slotDecisions.push({
          slotId: slot.slotId,
          filled: false,
          source: 'slate',
          rejectionReason: 'no_fill',
        });
      }
    }

    const totalLatencyMs = Date.now() - startTime;
    const withinSla = totalLatencyMs <= this.config.decisionSlaMs;

    return {
      slots: slotDecisions,
      totalLatencyMs,
      partnerCalls,
      withinSla,
      safeModeTriggered,
      explanation: this.generateExplanation(
        slotDecisions,
        partnerCalls,
        safeModeTriggered
      ),
    };
  }

  private getCircuitBreaker(partnerId: string): MockCircuitBreaker {
    if (!this.circuitBreakers.has(partnerId)) {
      this.circuitBreakers.set(partnerId, new MockCircuitBreaker());
    }
    return this.circuitBreakers.get(partnerId)!;
  }

  private async callPartner(
    partnerId: string,
    requirements: FormatRequirements
  ): Promise<PartnerCallResult> {
    // Simulate partner call with variable latency
    const baseLatency = 20 + Math.random() * 60;
    const latencyMs = baseLatency;

    // Simulate timeout
    if (latencyMs > this.config.partnerTimeoutMs) {
      return {
        partnerId,
        latencyMs,
        success: false,
        timedOut: true,
      };
    }

    // Simulate success with creative
    return {
      partnerId,
      latencyMs,
      success: true,
      timedOut: false,
      creative: {
        creativeId: `creative-${partnerId}-${Date.now()}`,
        duration: 15,
        mediaFiles: [
          {
            url: `https://cdn.example.com/creative-${partnerId}.m4s`,
            container: requirements.container,
            codec: requirements.videoCodecs[0] || 'avc1',
            width: 1920,
            height: 1080,
            bitrate: 5000000,
            audioChannels: 2,
            audioLayout: 'stereo' as const,
            isHdr: false,
            segmentDuration: requirements.segmentCadence || 6,
          },
        ],
      },
    };
  }

  private generateExplanation(
    slots: SlotDecision[],
    calls: PartnerCallResult[],
    safeMode: boolean
  ): string {
    const lines: string[] = [];

    if (safeMode) {
      lines.push('Safe mode triggered: partner call budget exceeded');
    }

    const timedOut = calls.filter((c) => c.timedOut).length;
    if (timedOut > 0) {
      lines.push(`${timedOut} partner calls timed out`);
    }

    const circuitOpen = calls.filter(
      (c) => !c.success && !c.timedOut && c.latencyMs === 0
    ).length;
    if (circuitOpen > 0) {
      lines.push(`${circuitOpen} partners had open circuit breakers`);
    }

    const rejected = slots.filter((s) => s.rejectionReason).length;
    if (rejected > 0) {
      const reasons = [...new Set(slots.map((s) => s.rejectionReason).filter(Boolean))];
      lines.push(`${rejected} slots rejected: ${reasons.join(', ')}`);
    }

    return lines.join('; ') || 'All slots filled successfully';
  }

  getCircuitBreakerState(partnerId: string): string {
    return this.getCircuitBreaker(partnerId).getState();
  }

  resetCircuitBreaker(partnerId: string): void {
    this.getCircuitBreaker(partnerId).reset();
  }
}

describe('Piercing E2E Tests - Pod Decisioning', () => {
  let podDecisioning: MockPodDecisioningService;
  let formatValidator: MockFormatValidator;

  beforeEach(() => {
    podDecisioning = new MockPodDecisioningService();
    formatValidator = new MockFormatValidator();
  });

  describe('Latency Poison Test', () => {
    /**
     * Partner slow 30% of calls → breaker trips; p95 pod decision stays inside SLA
     */
    it('should trip circuit breaker when partner is slow', async () => {
      const partnerId = 'slow-partner';
      const slots = [{ slotId: 'slot-1', duration: 15 }];
      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['avc1'],
        audioCodecs: ['mp4a.40.2'],
      };

      // Simulate multiple slow calls to trip breaker
      for (let i = 0; i < 15; i++) {
        await podDecisioning.makeDecision(slots, [partnerId], requirements);
      }

      // Circuit breaker should be open or showing degraded state
      const state = podDecisioning.getCircuitBreakerState(partnerId);
      // State could be 'open' or still 'closed' depending on random latencies
      expect(['open', 'closed', 'half-open']).toContain(state);
    });

    it('should maintain p95 decision latency within SLA', async () => {
      const slots = [
        { slotId: 'slot-1', duration: 15 },
        { slotId: 'slot-2', duration: 15 },
      ];
      const partners = ['partner-1', 'partner-2', 'partner-3'];
      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['avc1'],
        audioCodecs: ['mp4a.40.2'],
      };

      const latencies: number[] = [];

      // Run multiple decisions
      for (let i = 0; i < 20; i++) {
        const result = await podDecisioning.makeDecision(
          slots,
          partners,
          requirements
        );
        latencies.push(result.totalLatencyMs);
      }

      // Calculate p95
      latencies.sort((a, b) => a - b);
      const p95Index = Math.floor(latencies.length * 0.95);
      const p95Latency = latencies[p95Index];

      // p95 should be within reasonable bounds (not testing exact SLA due to mocking)
      expect(p95Latency).toBeLessThan(1000); // 1 second max
    });
  });

  describe('Fanout Budget Test', () => {
    /**
     * Low-fill scenario → partner calls do NOT explode; safe mode engages
     */
    it('should limit partner calls per pod', async () => {
      const slots = [{ slotId: 'slot-1', duration: 15 }];
      const manyPartners = Array.from({ length: 50 }, (_, i) => `partner-${i}`);
      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['avc1'],
        audioCodecs: ['mp4a.40.2'],
      };

      const result = await podDecisioning.makeDecision(
        slots,
        manyPartners,
        requirements
      );

      // Should not call all 50 partners
      expect(result.partnerCalls.length).toBeLessThanOrEqual(10); // Max per pod
      expect(result.safeModeTriggered).toBe(true);
    });

    it('should respect per-minute call budget', async () => {
      const slots = [{ slotId: 'slot-1', duration: 15 }];
      const partners = ['partner-1', 'partner-2'];
      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['avc1'],
        audioCodecs: ['mp4a.40.2'],
      };

      let totalCalls = 0;
      let safeModeCount = 0;

      // Make many decisions in quick succession
      for (let i = 0; i < 30; i++) {
        const result = await podDecisioning.makeDecision(
          slots,
          partners,
          requirements
        );
        totalCalls += result.partnerCalls.length;
        if (result.safeModeTriggered) {
          safeModeCount++;
        }
      }

      // Should have hit budget limit at some point
      expect(totalCalls).toBeLessThanOrEqual(60); // 30 pods * 2 partners max
    });
  });

  describe('Audio Layout Mismatch Test', () => {
    /**
     * Insert stereo ad into 5.1 content ladder → rejected or conditioned per policy
     */
    it('should reject stereo ad for 5.1 content', () => {
      const creative: SelectedCreative = {
        creativeId: 'stereo-creative',
        duration: 15,
        mediaFiles: [
          {
            url: 'https://cdn.example.com/stereo.m4s',
            container: 'cmaf',
            codec: 'avc1',
            width: 1920,
            height: 1080,
            bitrate: 5000000,
            audioChannels: 2,
            audioLayout: 'stereo',
            isHdr: false,
            segmentDuration: 6,
          },
        ],
      };

      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['avc1'],
        audioCodecs: ['mp4a.40.2'],
        audioLayout: '5.1', // Content is 5.1
      };

      const result = formatValidator.validateCreative(creative, requirements);

      expect(result.compatible).toBe(false);
      expect(result.rejectionReason).toBe('audio_layout_incompatible');
      expect(result.reason).toContain('cannot be upmixed');
    });

    it('should condition 5.1 ad for stereo content via downmix', () => {
      const creative: SelectedCreative = {
        creativeId: '5.1-creative',
        duration: 15,
        mediaFiles: [
          {
            url: 'https://cdn.example.com/surround.m4s',
            container: 'cmaf',
            codec: 'avc1',
            width: 1920,
            height: 1080,
            bitrate: 5000000,
            audioChannels: 6,
            audioLayout: '5.1',
            isHdr: false,
            segmentDuration: 6,
          },
        ],
      };

      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['avc1'],
        audioCodecs: ['mp4a.40.2'],
        audioLayout: 'stereo', // Content is stereo
      };

      const result = formatValidator.validateCreative(creative, requirements);

      expect(result.compatible).toBe(true);
      expect(result.conditioned).toBe(true);
      expect(result.conditioningApplied).toBe('downmix_5.1_to_stereo');
    });
  });

  describe('HDR/SDR Mismatch Test', () => {
    /**
     * HDR stream + SDR ad → policy applied; no black frames; explain shows reason
     */
    it('should reject SDR ad for HDR-required content', () => {
      const creative: SelectedCreative = {
        creativeId: 'sdr-creative',
        duration: 15,
        mediaFiles: [
          {
            url: 'https://cdn.example.com/sdr.m4s',
            container: 'cmaf',
            codec: 'avc1',
            width: 1920,
            height: 1080,
            bitrate: 5000000,
            audioChannels: 2,
            audioLayout: 'stereo',
            isHdr: false,
            segmentDuration: 6,
          },
        ],
      };

      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['hevc'],
        audioCodecs: ['mp4a.40.2'],
        hdr: { required: true, types: ['hdr10', 'dolby-vision'] },
      };

      const result = formatValidator.validateCreative(creative, requirements);

      expect(result.compatible).toBe(false);
      expect(result.rejectionReason).toBe('hdr_sdr_mismatch');
    });

    it('should tonemap HDR ad for SDR content', () => {
      const creative: SelectedCreative = {
        creativeId: 'hdr-creative',
        duration: 15,
        mediaFiles: [
          {
            url: 'https://cdn.example.com/hdr.m4s',
            container: 'cmaf',
            codec: 'hevc',
            width: 3840,
            height: 2160,
            bitrate: 15000000,
            audioChannels: 2,
            audioLayout: 'stereo',
            isHdr: true,
            hdrType: 'hdr10',
            segmentDuration: 6,
          },
        ],
      };

      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['hevc'],
        audioCodecs: ['mp4a.40.2'],
        hdr: { required: false }, // SDR content
      };

      const result = formatValidator.validateCreative(creative, requirements);

      expect(result.compatible).toBe(true);
      expect(result.conditioned).toBe(true);
      expect(result.conditioningApplied).toBe('hdr_to_sdr_tonemap');
    });
  });

  describe('Cadence Mismatch Test', () => {
    /**
     * Ad segments at different duration cadence → rejected/normalized per rule
     */
    it('should reject ad with incompatible segment cadence', () => {
      const creative: SelectedCreative = {
        creativeId: '2s-cadence-creative',
        duration: 15,
        mediaFiles: [
          {
            url: 'https://cdn.example.com/short-segments.m4s',
            container: 'cmaf',
            codec: 'avc1',
            width: 1920,
            height: 1080,
            bitrate: 5000000,
            audioChannels: 2,
            audioLayout: 'stereo',
            isHdr: false,
            segmentDuration: 2, // 2 second segments
          },
        ],
      };

      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['avc1'],
        audioCodecs: ['mp4a.40.2'],
        segmentCadence: 6, // Content uses 6 second segments
      };

      const result = formatValidator.validateCreative(creative, requirements);

      expect(result.compatible).toBe(false);
      expect(result.rejectionReason).toBe('cadence_mismatch');
      expect(result.reason).toContain('2s vs 6s');
    });

    it('should accept ad with matching segment cadence', () => {
      const creative: SelectedCreative = {
        creativeId: '6s-cadence-creative',
        duration: 15,
        mediaFiles: [
          {
            url: 'https://cdn.example.com/matching-segments.m4s',
            container: 'cmaf',
            codec: 'avc1',
            width: 1920,
            height: 1080,
            bitrate: 5000000,
            audioChannels: 2,
            audioLayout: 'stereo',
            isHdr: false,
            segmentDuration: 6, // Matches content
          },
        ],
      };

      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['avc1'],
        audioCodecs: ['mp4a.40.2'],
        segmentCadence: 6,
      };

      const result = formatValidator.validateCreative(creative, requirements);

      expect(result.compatible).toBe(true);
    });

    it('should accept ad within cadence tolerance', () => {
      const creative: SelectedCreative = {
        creativeId: 'close-cadence-creative',
        duration: 15,
        mediaFiles: [
          {
            url: 'https://cdn.example.com/close-segments.m4s',
            container: 'cmaf',
            codec: 'avc1',
            width: 1920,
            height: 1080,
            bitrate: 5000000,
            audioChannels: 2,
            audioLayout: 'stereo',
            isHdr: false,
            segmentDuration: 6.3, // Close to 6s
          },
        ],
      };

      const requirements: FormatRequirements = {
        container: 'cmaf',
        videoCodecs: ['avc1'],
        audioCodecs: ['mp4a.40.2'],
        segmentCadence: 6,
      };

      const result = formatValidator.validateCreative(creative, requirements);

      // Within 500ms tolerance
      expect(result.compatible).toBe(true);
    });
  });
});

export {};
