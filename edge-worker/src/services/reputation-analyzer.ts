/**
 * REPUTATION ANALYZER
 * 
 * Analyzes email reputation signals from DMARC, SPF, bounce patterns,
 * and engagement metrics to determine trust scores.
 * 
 * Features:
 *   - DMARC/SPF policy lookup (DNS-based)
 *   - Bounce pattern detection
 *   - Engagement-based scoring
 *   - Domain reputation caching
 *   - Throttling recommendations
 * 
 * Trust Score (0-1):
 *   - 0.9+ : Excellent - full send capacity
 *   - 0.7-0.9 : Good - normal operations
 *   - 0.5-0.7 : Fair - reduced capacity
 *   - 0.3-0.5 : Poor - throttled sending
 *   - 0.0-0.3 : Critical - pause sending
 */

import dns from 'dns';
import { promisify } from 'util';

const resolveTxt = promisify(dns.resolveTxt);

// ============================================================================
// TYPES
// ============================================================================

export interface DmarcRecord {
  version: string;
  policy: 'none' | 'quarantine' | 'reject';
  subdomainPolicy?: 'none' | 'quarantine' | 'reject';
  percentage?: number;
  rua?: string; // Aggregate report address
  ruf?: string; // Forensic report address
}

export interface SpfRecord {
  raw: string;
  mechanisms: string[];
  all: 'pass' | 'fail' | 'softfail' | 'neutral' | 'none';
}

export interface BounceMetrics {
  total: number;
  hard: number;
  soft: number;
  rate: number;
  recentRate: number; // Last 24h
}

export interface EngagementMetrics {
  openRate: number;
  clickRate: number;
  replyRate: number;
  unsubscribeRate: number;
  spamComplaintRate: number;
}

export interface DomainReputation {
  domain: string;
  dmarc: DmarcRecord | null;
  spf: SpfRecord | null;
  dmarcScore: number;
  spfScore: number;
  bounceScore: number;
  engagementScore: number;
  trustScore: number;
  throttleRecommendation: ThrottleLevel;
  analysis: string[];
  lastUpdated: Date;
}

export type ThrottleLevel = 
  | 'NONE'      // Full capacity
  | 'LIGHT'     // 75% capacity
  | 'MODERATE'  // 50% capacity
  | 'HEAVY'     // 25% capacity
  | 'PAUSE';    // Stop sending

export interface AnalysisInput {
  domain: string;
  bounces?: {
    total: number;
    hard: number;
    soft: number;
    sent: number;
    recent: {
      hard: number;
      soft: number;
      sent: number;
    };
  };
  engagement?: {
    sent: number;
    opened: number;
    clicked: number;
    replied: number;
    unsubscribed: number;
    spamComplaints: number;
  };
}

// ============================================================================
// REPUTATION CACHE
// ============================================================================

interface CacheEntry {
  data: DomainReputation;
  expiresAt: number;
}

const DEFAULT_CACHE_TTL_MS = 6 * 60 * 60 * 1000; // 6 hours

// ============================================================================
// REPUTATION ANALYZER CLASS
// ============================================================================

export class ReputationAnalyzer {
  private cache: Map<string, CacheEntry> = new Map();
  private cacheTTL: number;
  
  constructor(options?: { cacheTTLMs?: number }) {
    this.cacheTTL = options?.cacheTTLMs ?? DEFAULT_CACHE_TTL_MS;
  }
  
  /**
   * Analyze domain reputation
   */
  async analyze(input: AnalysisInput): Promise<DomainReputation> {
    const cacheKey = input.domain.toLowerCase();
    
    // Check cache
    const cached = this.cache.get(cacheKey);
    if (cached && cached.expiresAt > Date.now()) {
      // Refresh scores with new bounce/engagement data if provided
      if (input.bounces || input.engagement) {
        return this.refreshScores(cached.data, input);
      }
      return cached.data;
    }
    
    // Lookup DNS records
    const [dmarc, spf] = await Promise.all([
      this.lookupDmarc(input.domain),
      this.lookupSpf(input.domain),
    ]);
    
    // Calculate scores
    const dmarcScore = this.scoreDmarc(dmarc);
    const spfScore = this.scoreSpf(spf);
    const bounceScore = this.scoreBounces(input.bounces);
    const engagementScore = this.scoreEngagement(input.engagement);
    
    // Combined trust score (weighted average)
    const trustScore = this.calculateTrustScore({
      dmarcScore,
      spfScore,
      bounceScore,
      engagementScore,
    });
    
    const throttleRecommendation = this.recommendThrottle(trustScore, bounceScore);
    
    const analysis = this.generateAnalysis({
      dmarc,
      spf,
      dmarcScore,
      spfScore,
      bounceScore,
      engagementScore,
      trustScore,
      bounces: input.bounces,
      engagement: input.engagement,
    });
    
    const reputation: DomainReputation = {
      domain: input.domain,
      dmarc,
      spf,
      dmarcScore,
      spfScore,
      bounceScore,
      engagementScore,
      trustScore,
      throttleRecommendation,
      analysis,
      lastUpdated: new Date(),
    };
    
    // Cache result
    this.cache.set(cacheKey, {
      data: reputation,
      expiresAt: Date.now() + this.cacheTTL,
    });
    
    return reputation;
  }
  
  /**
   * Refresh scores without re-fetching DNS
   */
  private refreshScores(
    cached: DomainReputation,
    input: AnalysisInput
  ): DomainReputation {
    const bounceScore = this.scoreBounces(input.bounces);
    const engagementScore = this.scoreEngagement(input.engagement);
    
    const trustScore = this.calculateTrustScore({
      dmarcScore: cached.dmarcScore,
      spfScore: cached.spfScore,
      bounceScore,
      engagementScore,
    });
    
    const throttleRecommendation = this.recommendThrottle(trustScore, bounceScore);
    
    return {
      ...cached,
      bounceScore,
      engagementScore,
      trustScore,
      throttleRecommendation,
      lastUpdated: new Date(),
    };
  }
  
  // ==========================================================================
  // DNS LOOKUPS
  // ==========================================================================
  
  private async lookupDmarc(domain: string): Promise<DmarcRecord | null> {
    try {
      const records = await resolveTxt(`_dmarc.${domain}`);
      const dmarcRecord = records
        .flat()
        .find(r => r.startsWith('v=DMARC1'));
      
      if (!dmarcRecord) return null;
      
      const parsed: DmarcRecord = {
        version: 'DMARC1',
        policy: 'none',
      };
      
      const policyMatch = dmarcRecord.match(/\bp=(\w+)/);
      if (policyMatch) {
        const policy = policyMatch[1].toLowerCase();
        if (['none', 'quarantine', 'reject'].includes(policy)) {
          parsed.policy = policy as 'none' | 'quarantine' | 'reject';
        }
      }
      
      const spMatch = dmarcRecord.match(/\bsp=(\w+)/);
      if (spMatch) {
        const sp = spMatch[1].toLowerCase();
        if (['none', 'quarantine', 'reject'].includes(sp)) {
          parsed.subdomainPolicy = sp as 'none' | 'quarantine' | 'reject';
        }
      }
      
      const pctMatch = dmarcRecord.match(/\bpct=(\d+)/);
      if (pctMatch) {
        parsed.percentage = parseInt(pctMatch[1], 10);
      }
      
      const ruaMatch = dmarcRecord.match(/\brua=([^\s;]+)/);
      if (ruaMatch) {
        parsed.rua = ruaMatch[1];
      }
      
      const rufMatch = dmarcRecord.match(/\bruf=([^\s;]+)/);
      if (rufMatch) {
        parsed.ruf = rufMatch[1];
      }
      
      return parsed;
    } catch {
      return null;
    }
  }
  
  private async lookupSpf(domain: string): Promise<SpfRecord | null> {
    try {
      const records = await resolveTxt(domain);
      const spfRecord = records
        .flat()
        .find(r => r.startsWith('v=spf1'));
      
      if (!spfRecord) return null;
      
      const mechanisms = spfRecord
        .split(/\s+/)
        .filter(m => m && m !== 'v=spf1');
      
      let all: SpfRecord['all'] = 'none';
      
      for (const mech of mechanisms) {
        if (mech === '+all' || mech === 'all') all = 'pass';
        else if (mech === '-all') all = 'fail';
        else if (mech === '~all') all = 'softfail';
        else if (mech === '?all') all = 'neutral';
      }
      
      return {
        raw: spfRecord,
        mechanisms,
        all,
      };
    } catch {
      return null;
    }
  }
  
  // ==========================================================================
  // SCORING
  // ==========================================================================
  
  private scoreDmarc(dmarc: DmarcRecord | null): number {
    if (!dmarc) return 0.3; // No DMARC is risky
    
    switch (dmarc.policy) {
      case 'reject':
        return 1.0;
      case 'quarantine':
        return 0.8;
      case 'none':
        return 0.5;
      default:
        return 0.3;
    }
  }
  
  private scoreSpf(spf: SpfRecord | null): number {
    if (!spf) return 0.3; // No SPF is risky
    
    switch (spf.all) {
      case 'fail':
        return 1.0; // Strict -all
      case 'softfail':
        return 0.7; // Permissive ~all
      case 'neutral':
        return 0.5;
      case 'pass':
        return 0.2; // +all is very risky
      default:
        return 0.4;
    }
  }
  
  private scoreBounces(bounces?: AnalysisInput['bounces']): number {
    if (!bounces || bounces.sent === 0) return 0.8; // No data, assume OK
    
    const hardRate = bounces.hard / bounces.sent;
    const recentHardRate = bounces.recent.sent > 0
      ? bounces.recent.hard / bounces.recent.sent
      : 0;
    
    // Hard bounce rate thresholds
    if (recentHardRate > 0.05) return 0.1; // >5% recent hard bounces = critical
    if (hardRate > 0.03) return 0.3;       // >3% overall = poor
    if (hardRate > 0.01) return 0.6;       // >1% = fair
    if (hardRate > 0.005) return 0.8;      // >0.5% = good
    
    return 1.0; // <0.5% = excellent
  }
  
  private scoreEngagement(engagement?: AnalysisInput['engagement']): number {
    if (!engagement || engagement.sent === 0) return 0.7; // No data
    
    const openRate = engagement.opened / engagement.sent;
    const replyRate = engagement.replied / engagement.sent;
    const spamRate = engagement.spamComplaints / engagement.sent;
    
    // Spam complaints are critical
    if (spamRate > 0.001) return 0.2; // >0.1% spam = poor
    
    // Positive engagement signals
    let score = 0.5;
    
    if (openRate > 0.3) score += 0.2;
    else if (openRate > 0.15) score += 0.1;
    
    if (replyRate > 0.05) score += 0.2;
    else if (replyRate > 0.02) score += 0.1;
    
    return Math.min(score, 1.0);
  }
  
  private calculateTrustScore(scores: {
    dmarcScore: number;
    spfScore: number;
    bounceScore: number;
    engagementScore: number;
  }): number {
    // Weighted average with bounce being most important
    const weights = {
      dmarc: 0.15,
      spf: 0.15,
      bounce: 0.45,
      engagement: 0.25,
    };
    
    return (
      scores.dmarcScore * weights.dmarc +
      scores.spfScore * weights.spf +
      scores.bounceScore * weights.bounce +
      scores.engagementScore * weights.engagement
    );
  }
  
  private recommendThrottle(
    trustScore: number,
    bounceScore: number
  ): ThrottleLevel {
    // Immediate pause on critical bounce issues
    if (bounceScore < 0.2) return 'PAUSE';
    
    if (trustScore >= 0.9) return 'NONE';
    if (trustScore >= 0.7) return 'LIGHT';
    if (trustScore >= 0.5) return 'MODERATE';
    if (trustScore >= 0.3) return 'HEAVY';
    
    return 'PAUSE';
  }
  
  // ==========================================================================
  // ANALYSIS
  // ==========================================================================
  
  private generateAnalysis(data: {
    dmarc: DmarcRecord | null;
    spf: SpfRecord | null;
    dmarcScore: number;
    spfScore: number;
    bounceScore: number;
    engagementScore: number;
    trustScore: number;
    bounces?: AnalysisInput['bounces'];
    engagement?: AnalysisInput['engagement'];
  }): string[] {
    const analysis: string[] = [];
    
    // DMARC analysis
    if (!data.dmarc) {
      analysis.push('⚠️ No DMARC record found - consider adding one');
    } else if (data.dmarc.policy === 'none') {
      analysis.push('⚠️ DMARC policy is "none" - consider upgrading to quarantine/reject');
    } else if (data.dmarc.policy === 'reject') {
      analysis.push('✅ Strong DMARC policy (reject)');
    }
    
    // SPF analysis
    if (!data.spf) {
      analysis.push('⚠️ No SPF record found - email deliverability at risk');
    } else if (data.spf.all === 'pass') {
      analysis.push('🚨 SPF +all policy is dangerous - anyone can send as you');
    } else if (data.spf.all === 'fail') {
      analysis.push('✅ Strict SPF policy (-all)');
    }
    
    // Bounce analysis
    if (data.bounces) {
      const hardRate = data.bounces.hard / Math.max(data.bounces.sent, 1) * 100;
      
      if (hardRate > 3) {
        analysis.push(`🚨 Critical hard bounce rate: ${hardRate.toFixed(2)}%`);
      } else if (hardRate > 1) {
        analysis.push(`⚠️ High hard bounce rate: ${hardRate.toFixed(2)}%`);
      } else if (hardRate < 0.5) {
        analysis.push(`✅ Good bounce rate: ${hardRate.toFixed(2)}%`);
      }
    }
    
    // Engagement analysis
    if (data.engagement) {
      const openRate = data.engagement.opened / Math.max(data.engagement.sent, 1) * 100;
      const replyRate = data.engagement.replied / Math.max(data.engagement.sent, 1) * 100;
      
      if (openRate > 30) {
        analysis.push(`✅ Excellent open rate: ${openRate.toFixed(1)}%`);
      } else if (openRate < 10) {
        analysis.push(`⚠️ Low open rate: ${openRate.toFixed(1)}%`);
      }
      
      if (replyRate > 5) {
        analysis.push(`✅ Strong reply rate: ${replyRate.toFixed(1)}%`);
      }
    }
    
    // Trust summary
    if (data.trustScore >= 0.9) {
      analysis.push('🏆 Overall reputation: Excellent');
    } else if (data.trustScore >= 0.7) {
      analysis.push('✅ Overall reputation: Good');
    } else if (data.trustScore >= 0.5) {
      analysis.push('⚠️ Overall reputation: Fair - monitor closely');
    } else if (data.trustScore >= 0.3) {
      analysis.push('🚨 Overall reputation: Poor - throttle sending');
    } else {
      analysis.push('🛑 Overall reputation: Critical - pause sending');
    }
    
    return analysis;
  }
  
  /**
   * Batch analyze multiple domains
   */
  async analyzeBatch(domains: string[]): Promise<Map<string, DomainReputation>> {
    const results = new Map<string, DomainReputation>();
    
    await Promise.all(
      domains.map(async (domain) => {
        const rep = await this.analyze({ domain });
        results.set(domain, rep);
      })
    );
    
    return results;
  }
  
  /**
   * Clear cache
   */
  clearCache(): void {
    this.cache.clear();
  }
  
  /**
   * Get cache stats
   */
  getCacheStats(): { size: number; entries: string[] } {
    return {
      size: this.cache.size,
      entries: Array.from(this.cache.keys()),
    };
  }
}
