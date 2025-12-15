/**
 * Candidate Orderer
 *
 * Implements deterministic ordering of ad candidates to ensure
 * consistent selection given the same inputs, configuration, and health state.
 */

import crypto from 'crypto';

export interface AdCandidate {
  id: string;
  creativeId: string;
  advertiserId: string;
  campaignId: string;
  priority: number;
  price: number;
  duration: number;
  healthScore: number;
  partnerId: string;
  metadata?: Record<string, unknown>;
}

export interface OrderingConfig {
  /** Weight for priority in scoring */
  priorityWeight: number;

  /** Weight for price in scoring */
  priceWeight: number;

  /** Weight for health score in scoring */
  healthWeight: number;

  /** Weight for partner diversity in scoring */
  diversityWeight: number;

  /** Minimum health score to consider */
  minHealthScore: number;

  /** Enable deterministic tie-breaking */
  deterministicTieBreak: boolean;

  /** Salt for deterministic hashing */
  hashSalt: string;
}

export interface OrderingContext {
  /** Session ID for deterministic seeding */
  sessionId: string;

  /** Pod ID for deterministic seeding */
  podId: string;

  /** Slot index in the pod */
  slotIndex: number;

  /** Partner health states */
  partnerHealth: Map<string, number>;

  /** Already selected candidates in this pod */
  selectedCandidates: AdCandidate[];

  /** Current timestamp */
  timestamp: number;
}

export interface OrderedCandidate extends AdCandidate {
  score: number;
  rank: number;
  scoreBreakdown: ScoreBreakdown;
}

export interface ScoreBreakdown {
  priorityScore: number;
  priceScore: number;
  healthScore: number;
  diversityScore: number;
  totalScore: number;
}

/**
 * Candidate Orderer ensures deterministic, fair ordering of ad candidates.
 */
export class CandidateOrderer {
  private config: OrderingConfig;

  constructor(config?: Partial<OrderingConfig>) {
    this.config = {
      priorityWeight: 0.3,
      priceWeight: 0.3,
      healthWeight: 0.25,
      diversityWeight: 0.15,
      minHealthScore: 0.5,
      deterministicTieBreak: true,
      hashSalt: 'ssai-candidate-orderer-v1',
      ...config,
    };
  }

  /**
   * Order candidates deterministically based on scoring and context.
   */
  orderCandidates(
    candidates: AdCandidate[],
    context: OrderingContext
  ): OrderedCandidate[] {
    // Filter out unhealthy candidates
    const healthyCandidates = candidates.filter(
      (c) => c.healthScore >= this.config.minHealthScore
    );

    // Calculate scores for each candidate
    const scoredCandidates = healthyCandidates.map((candidate) => {
      const scoreBreakdown = this.calculateScore(candidate, context);
      return {
        ...candidate,
        score: scoreBreakdown.totalScore,
        rank: 0,
        scoreBreakdown,
      };
    });

    // Sort by score (descending), with deterministic tie-breaking
    scoredCandidates.sort((a, b) => {
      const scoreDiff = b.score - a.score;

      if (Math.abs(scoreDiff) > 0.0001) {
        return scoreDiff;
      }

      // Tie-break deterministically
      if (this.config.deterministicTieBreak) {
        const hashA = this.deterministicHash(a, context);
        const hashB = this.deterministicHash(b, context);
        return hashA.localeCompare(hashB);
      }

      return 0;
    });

    // Assign ranks
    scoredCandidates.forEach((candidate, index) => {
      candidate.rank = index + 1;
    });

    return scoredCandidates;
  }

  /**
   * Calculate score for a single candidate.
   */
  private calculateScore(
    candidate: AdCandidate,
    context: OrderingContext
  ): ScoreBreakdown {
    // Normalize priority (assuming 1-10 scale)
    const priorityScore = Math.min(candidate.priority / 10, 1);

    // Normalize price (assuming 0-100 CPM range)
    const priceScore = Math.min(candidate.price / 100, 1);

    // Health score is already normalized
    const healthScore = candidate.healthScore;

    // Calculate diversity score based on partner diversity
    const diversityScore = this.calculateDiversityScore(candidate, context);

    // Calculate weighted total
    const totalScore =
      this.config.priorityWeight * priorityScore +
      this.config.priceWeight * priceScore +
      this.config.healthWeight * healthScore +
      this.config.diversityWeight * diversityScore;

    return {
      priorityScore,
      priceScore,
      healthScore,
      diversityScore,
      totalScore,
    };
  }

  /**
   * Calculate diversity score to favor partner diversity.
   */
  private calculateDiversityScore(
    candidate: AdCandidate,
    context: OrderingContext
  ): number {
    // Count how many times this partner has been selected
    const partnerSelectionCount = context.selectedCandidates.filter(
      (c) => c.partnerId === candidate.partnerId
    ).length;

    // Penalize repeated partner selections
    if (partnerSelectionCount === 0) {
      return 1.0;
    }

    // Exponential decay for repeated selections
    return Math.exp(-0.5 * partnerSelectionCount);
  }

  /**
   * Generate deterministic hash for tie-breaking.
   */
  private deterministicHash(
    candidate: AdCandidate,
    context: OrderingContext
  ): string {
    const data = [
      this.config.hashSalt,
      context.sessionId,
      context.podId,
      context.slotIndex.toString(),
      candidate.id,
      candidate.creativeId,
    ].join('|');

    return crypto.createHash('sha256').update(data).digest('hex');
  }

  /**
   * Verify ordering is deterministic.
   * Returns true if ordering the same inputs produces the same output.
   */
  verifyDeterminism(
    candidates: AdCandidate[],
    context: OrderingContext
  ): boolean {
    const result1 = this.orderCandidates(candidates, context);
    const result2 = this.orderCandidates(candidates, context);

    if (result1.length !== result2.length) {
      return false;
    }

    return result1.every((c, i) => c.id === result2[i].id && c.rank === result2[i].rank);
  }

  /**
   * Get the top N candidates.
   */
  getTopCandidates(
    candidates: AdCandidate[],
    context: OrderingContext,
    n: number
  ): OrderedCandidate[] {
    const ordered = this.orderCandidates(candidates, context);
    return ordered.slice(0, n);
  }

  /**
   * Select the best candidate for a slot.
   */
  selectBestCandidate(
    candidates: AdCandidate[],
    context: OrderingContext
  ): OrderedCandidate | null {
    const ordered = this.orderCandidates(candidates, context);
    return ordered.length > 0 ? ordered[0] : null;
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<OrderingConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Get current configuration.
   */
  getConfig(): OrderingConfig {
    return { ...this.config };
  }
}

export default CandidateOrderer;
