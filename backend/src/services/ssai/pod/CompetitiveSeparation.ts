/**
 * Competitive Separation Manager
 *
 * Enforces advertiser separation, category separation, and
 * creative repetition rules within ad pods.
 */

import { SelectedCreative, SlotDecision } from './types';

export interface SeparationRule {
  /** Rule ID */
  id: string;

  /** Rule type */
  type: 'advertiser' | 'category' | 'brand' | 'creative';

  /** Values to separate */
  values: string[];

  /** Minimum separation in slots (0 = cannot appear in same pod) */
  minSeparationSlots: number;

  /** Minimum separation in seconds */
  minSeparationSeconds: number;

  /** Priority (higher = more important) */
  priority: number;
}

export interface SeparationConfig {
  /** Global advertiser separation */
  advertiserSeparationSlots: number;

  /** Global category separation */
  categorySeparationSlots: number;

  /** Maximum creative repetition in session */
  maxCreativeRepetitionSession: number;

  /** Maximum creative repetition in pod */
  maxCreativeRepetitionPod: number;

  /** Custom separation rules */
  customRules: SeparationRule[];
}

export interface SeparationCheckResult {
  allowed: boolean;
  reason?: string;
  violatedRule?: SeparationRule;
}

export interface PodSeparationState {
  /** Creatives already placed in this pod */
  placedCreatives: Array<{
    slotPosition: number;
    creative: SelectedCreative;
    startTimeSeconds: number;
  }>;

  /** Session creative counts */
  sessionCreativeCounts: Record<string, number>;

  /** Session advertiser counts */
  sessionAdvertiserCounts: Record<string, number>;
}

/**
 * Competitive Separation Manager ensures no conflicting ads
 * appear together in the same pod or too close together.
 */
export class CompetitiveSeparation {
  private config: SeparationConfig;
  private categoryConflicts: Map<string, Set<string>>;

  constructor(config?: Partial<SeparationConfig>) {
    this.config = {
      advertiserSeparationSlots: 2,
      categorySeparationSlots: 1,
      maxCreativeRepetitionSession: 3,
      maxCreativeRepetitionPod: 1,
      customRules: [],
      ...config,
    };

    // Initialize category conflicts (categories that shouldn't appear together)
    this.categoryConflicts = new Map([
      ['automotive', new Set(['automotive'])],
      ['financial', new Set(['financial'])],
      ['telecom', new Set(['telecom'])],
      ['insurance', new Set(['insurance'])],
      ['fast_food', new Set(['fast_food', 'restaurant'])],
      ['restaurant', new Set(['fast_food', 'restaurant'])],
      ['political', new Set(['political'])],
    ]);
  }

  /**
   * Check if a creative can be placed in a slot.
   */
  checkPlacement(
    creative: SelectedCreative,
    slotPosition: number,
    slotStartTime: number,
    state: PodSeparationState
  ): SeparationCheckResult {
    // Check creative repetition in pod
    const podRepetitionCheck = this.checkPodRepetition(creative, state);
    if (!podRepetitionCheck.allowed) {
      return podRepetitionCheck;
    }

    // Check creative repetition in session
    const sessionRepetitionCheck = this.checkSessionRepetition(creative, state);
    if (!sessionRepetitionCheck.allowed) {
      return sessionRepetitionCheck;
    }

    // Check advertiser separation
    const advertiserCheck = this.checkAdvertiserSeparation(
      creative,
      slotPosition,
      slotStartTime,
      state
    );
    if (!advertiserCheck.allowed) {
      return advertiserCheck;
    }

    // Check category separation
    const categoryCheck = this.checkCategorySeparation(
      creative,
      slotPosition,
      slotStartTime,
      state
    );
    if (!categoryCheck.allowed) {
      return categoryCheck;
    }

    // Check custom rules
    const customCheck = this.checkCustomRules(
      creative,
      slotPosition,
      slotStartTime,
      state
    );
    if (!customCheck.allowed) {
      return customCheck;
    }

    return { allowed: true };
  }

  /**
   * Check creative repetition within pod.
   */
  private checkPodRepetition(
    creative: SelectedCreative,
    state: PodSeparationState
  ): SeparationCheckResult {
    const count = state.placedCreatives.filter(
      (p) => p.creative.creativeId === creative.creativeId
    ).length;

    if (count >= this.config.maxCreativeRepetitionPod) {
      return {
        allowed: false,
        reason: `Creative ${creative.creativeId} already shown ${count} times in this pod (max: ${this.config.maxCreativeRepetitionPod})`,
      };
    }

    return { allowed: true };
  }

  /**
   * Check creative repetition within session.
   */
  private checkSessionRepetition(
    creative: SelectedCreative,
    state: PodSeparationState
  ): SeparationCheckResult {
    const count = state.sessionCreativeCounts[creative.creativeId] || 0;

    if (count >= this.config.maxCreativeRepetitionSession) {
      return {
        allowed: false,
        reason: `Creative ${creative.creativeId} already shown ${count} times in this session (max: ${this.config.maxCreativeRepetitionSession})`,
      };
    }

    return { allowed: true };
  }

  /**
   * Check advertiser separation.
   */
  private checkAdvertiserSeparation(
    creative: SelectedCreative,
    slotPosition: number,
    slotStartTime: number,
    state: PodSeparationState
  ): SeparationCheckResult {
    const requiredSeparation = this.config.advertiserSeparationSlots;

    for (const placed of state.placedCreatives) {
      if (placed.creative.advertiserId === creative.advertiserId) {
        const slotDistance = Math.abs(slotPosition - placed.slotPosition);

        if (slotDistance < requiredSeparation) {
          return {
            allowed: false,
            reason: `Advertiser ${creative.advertiserId} requires ${requiredSeparation} slot separation (current: ${slotDistance})`,
          };
        }
      }
    }

    return { allowed: true };
  }

  /**
   * Check category separation.
   */
  private checkCategorySeparation(
    creative: SelectedCreative,
    slotPosition: number,
    _slotStartTime: number,
    state: PodSeparationState
  ): SeparationCheckResult {
    if (!creative.category) {
      return { allowed: true };
    }

    const requiredSeparation = this.config.categorySeparationSlots;
    const conflictingCategories = this.categoryConflicts.get(creative.category);

    if (!conflictingCategories) {
      return { allowed: true };
    }

    for (const placed of state.placedCreatives) {
      if (
        placed.creative.category &&
        conflictingCategories.has(placed.creative.category)
      ) {
        const slotDistance = Math.abs(slotPosition - placed.slotPosition);

        if (slotDistance < requiredSeparation) {
          return {
            allowed: false,
            reason: `Category ${creative.category} conflicts with ${placed.creative.category} (requires ${requiredSeparation} slot separation)`,
          };
        }
      }
    }

    return { allowed: true };
  }

  /**
   * Check custom separation rules.
   */
  private checkCustomRules(
    creative: SelectedCreative,
    slotPosition: number,
    slotStartTime: number,
    state: PodSeparationState
  ): SeparationCheckResult {
    // Sort rules by priority (highest first)
    const sortedRules = [...this.config.customRules].sort(
      (a, b) => b.priority - a.priority
    );

    for (const rule of sortedRules) {
      const creativeValue = this.getCreativeValueForRule(creative, rule.type);
      if (!creativeValue || !rule.values.includes(creativeValue)) {
        continue;
      }

      // Check against placed creatives
      for (const placed of state.placedCreatives) {
        const placedValue = this.getCreativeValueForRule(
          placed.creative,
          rule.type
        );
        if (!placedValue || !rule.values.includes(placedValue)) {
          continue;
        }

        // Check slot separation
        const slotDistance = Math.abs(slotPosition - placed.slotPosition);
        if (slotDistance < rule.minSeparationSlots) {
          return {
            allowed: false,
            reason: `Rule ${rule.id} requires ${rule.minSeparationSlots} slot separation`,
            violatedRule: rule,
          };
        }

        // Check time separation
        const timeDistance = Math.abs(slotStartTime - placed.startTimeSeconds);
        if (timeDistance < rule.minSeparationSeconds) {
          return {
            allowed: false,
            reason: `Rule ${rule.id} requires ${rule.minSeparationSeconds}s time separation`,
            violatedRule: rule,
          };
        }
      }
    }

    return { allowed: true };
  }

  /**
   * Get creative value for a rule type.
   */
  private getCreativeValueForRule(
    creative: SelectedCreative,
    ruleType: SeparationRule['type']
  ): string | undefined {
    switch (ruleType) {
      case 'advertiser':
        return creative.advertiserId;
      case 'category':
        return creative.category;
      case 'creative':
        return creative.creativeId;
      case 'brand':
        // Brand could be derived from advertiser or stored separately
        return creative.advertiserId;
      default:
        return undefined;
    }
  }

  /**
   * Find best slot for a creative given separation rules.
   */
  findBestSlot(
    creative: SelectedCreative,
    availableSlots: Array<{ position: number; startTime: number }>,
    state: PodSeparationState
  ): { position: number; startTime: number } | null {
    for (const slot of availableSlots) {
      const check = this.checkPlacement(
        creative,
        slot.position,
        slot.startTime,
        state
      );
      if (check.allowed) {
        return slot;
      }
    }

    return null;
  }

  /**
   * Reorder creatives to satisfy separation rules.
   */
  reorderCreatives(
    creatives: SelectedCreative[],
    slotDurations: number[]
  ): SelectedCreative[] | null {
    // Try different orderings to find one that satisfies all rules
    const permutations = this.generatePermutations(creatives);

    for (const perm of permutations) {
      if (this.validateOrdering(perm, slotDurations)) {
        return perm;
      }
    }

    return null;
  }

  /**
   * Validate that an ordering satisfies separation rules.
   */
  private validateOrdering(
    creatives: SelectedCreative[],
    slotDurations: number[]
  ): boolean {
    const state: PodSeparationState = {
      placedCreatives: [],
      sessionCreativeCounts: {},
      sessionAdvertiserCounts: {},
    };

    let currentTime = 0;

    for (let i = 0; i < creatives.length; i++) {
      const check = this.checkPlacement(creatives[i], i, currentTime, state);
      if (!check.allowed) {
        return false;
      }

      // Update state
      state.placedCreatives.push({
        slotPosition: i,
        creative: creatives[i],
        startTimeSeconds: currentTime,
      });

      currentTime += slotDurations[i] || creatives[i].duration;
    }

    return true;
  }

  /**
   * Generate limited permutations (for efficiency).
   */
  private generatePermutations<T>(arr: T[], limit = 100): T[][] {
    if (arr.length <= 1) {
      return [arr];
    }

    const results: T[][] = [];

    const permute = (current: T[], remaining: T[]) => {
      if (results.length >= limit) {
        return;
      }

      if (remaining.length === 0) {
        results.push([...current]);
        return;
      }

      for (let i = 0; i < remaining.length; i++) {
        const next = [...remaining];
        const item = next.splice(i, 1)[0];
        permute([...current, item], next);
      }
    };

    permute([], arr);
    return results;
  }

  /**
   * Add category conflict.
   */
  addCategoryConflict(category1: string, category2: string): void {
    // Add both directions
    if (!this.categoryConflicts.has(category1)) {
      this.categoryConflicts.set(category1, new Set());
    }
    this.categoryConflicts.get(category1)!.add(category2);

    if (!this.categoryConflicts.has(category2)) {
      this.categoryConflicts.set(category2, new Set());
    }
    this.categoryConflicts.get(category2)!.add(category1);
  }

  /**
   * Add custom separation rule.
   */
  addCustomRule(rule: SeparationRule): void {
    this.config.customRules.push(rule);
  }

  /**
   * Update configuration.
   */
  updateConfig(config: Partial<SeparationConfig>): void {
    this.config = { ...this.config, ...config };
  }

  /**
   * Get current configuration.
   */
  getConfig(): SeparationConfig {
    return { ...this.config };
  }
}

export default CompetitiveSeparation;
