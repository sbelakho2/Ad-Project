/**
 * ANALYST MODULE (The Scorer)
 * 
 * Zero-cost lead qualification using heuristic scoring.
 * Scores leads based on publicly available signals.
 * 
 * Scoring Signals:
 *   - app-ads.txt presence (monetizing with ads = likely buyer)
 *   - SDK detection (MoPub, AdMob, IronSource = ad focused)
 *   - App store ranking (more downloads = more ad inventory)
 *   - Update velocity (active development = engaged team)
 *   - Company size signals (domain age, LinkedIn presence)
 *   - Website technology (modern stack = funded company)
 * 
 * Each signal contributes 0-100 points with configurable weights.
 * Final score is weighted average normalized to 0-100.
 */

import { queryOperator, queryReadOnly } from '../../database';
import { logger } from '../../logger';
import { checkAppAdsTxt } from './hunter';

// ============================================================================
// TYPES
// ============================================================================

export interface HeuristicScore {
  signal: string;
  score: number;
  weight: number;
  explanation: string;
}

export interface LeadScore {
  contactId: string;
  totalScore: number;
  tier: 'hot' | 'warm' | 'cold' | 'ice';
  signals: HeuristicScore[];
  scoredAt: Date;
}

export interface Contact {
  id: string;
  companyName: string;
  website?: string;
  email?: string;
  appStore?: string;
  appId?: string;
  appGenre?: string;
  adsTxtContent?: string;
  lastUpdated?: Date;
}

// ============================================================================
// SIGNAL WEIGHTS (Configurable)
// ============================================================================

const DEFAULT_WEIGHTS: Record<string, number> = {
  ads_txt_present: 25,        // Strong signal - they monetize with ads
  ad_network_count: 15,       // More networks = more sophisticated
  popular_network: 15,        // Google, Facebook = serious about ads
  competitor_detected: 20,    // CRITICAL: Using a competitor = HOT lead
  app_ranking: 10,            // Higher ranking = more inventory
  update_velocity: 10,        // Recent updates = active team
  has_email: 10,              // Discoverable contact = easier sale
  has_website: 5,             // Professional presence
  app_genre_fit: 10,          // Games, utilities = high ad volume
};

// ============================================================================
// COMPETITOR TARGETING (The "Sniper" Upgrade)
// ============================================================================

/**
 * Competitors whose users we want to convert.
 * These are ad networks we can replace with superior mediation.
 * When we detect these in app-ads.txt, it's a HOT lead.
 */
const TARGET_COMPETITORS = [
  // Tier 1: Direct competitors (highest priority)
  { domain: 'ironsrc.com', name: 'IronSource', tier: 1, boost: 50 },
  { domain: 'is.com', name: 'IronSource (alt)', tier: 1, boost: 50 },
  { domain: 'unity.com', name: 'Unity Ads', tier: 1, boost: 50 },
  { domain: 'unityads.unity3d.com', name: 'Unity Ads (alt)', tier: 1, boost: 50 },
  { domain: 'applovin.com', name: 'AppLovin', tier: 1, boost: 45 },
  { domain: 'max.com', name: 'AppLovin MAX', tier: 1, boost: 45 },
  
  // Tier 2: Major players (medium priority)
  { domain: 'mintegral.com', name: 'Mintegral', tier: 2, boost: 35 },
  { domain: 'vungle.com', name: 'Vungle', tier: 2, boost: 35 },
  { domain: 'chartboost.com', name: 'Chartboost', tier: 2, boost: 30 },
  { domain: 'fyber.com', name: 'Fyber', tier: 2, boost: 30 },
  { domain: 'inmobi.com', name: 'InMobi', tier: 2, boost: 30 },
  
  // Tier 3: Legacy/weaker networks (opportunity)
  { domain: 'adcolony.com', name: 'AdColony', tier: 3, boost: 25 },
  { domain: 'tapjoy.com', name: 'Tapjoy', tier: 3, boost: 25 },
  { domain: 'startapp.com', name: 'StartApp', tier: 3, boost: 20 },
  { domain: 'ogury.co', name: 'Ogury', tier: 3, boost: 20 },
];

// ============================================================================
// SCORING FUNCTIONS
// ============================================================================

/**
 * Score presence of app-ads.txt
 */
function scoreAdsTxtPresent(contact: Contact): HeuristicScore {
  const hasAdsTxt = Boolean(contact.adsTxtContent);
  
  return {
    signal: 'ads_txt_present',
    score: hasAdsTxt ? 100 : 0,
    weight: DEFAULT_WEIGHTS.ads_txt_present,
    explanation: hasAdsTxt 
      ? 'Has app-ads.txt file (monetizing with ads)'
      : 'No app-ads.txt found',
  };
}

/**
 * Score number of ad networks in app-ads.txt
 */
function scoreAdNetworkCount(adsTxtContent?: string): HeuristicScore {
  if (!adsTxtContent) {
    return {
      signal: 'ad_network_count',
      score: 0,
      weight: DEFAULT_WEIGHTS.ad_network_count,
      explanation: 'No ads.txt content to analyze',
    };
  }
  
  const networks = extractNetworksFromAdsTxt(adsTxtContent);
  const count = networks.length;
  
  // Score: 0 networks = 0, 1-2 = 30, 3-5 = 60, 6+ = 100
  let score = 0;
  if (count >= 6) score = 100;
  else if (count >= 3) score = 60;
  else if (count >= 1) score = 30;
  
  return {
    signal: 'ad_network_count',
    score,
    weight: DEFAULT_WEIGHTS.ad_network_count,
    explanation: `${count} ad network${count === 1 ? '' : 's'} detected`,
  };
}

/**
 * Score presence of popular/premium ad networks
 */
function scorePopularNetworks(adsTxtContent?: string): HeuristicScore {
  if (!adsTxtContent) {
    return {
      signal: 'popular_network',
      score: 0,
      weight: DEFAULT_WEIGHTS.popular_network,
      explanation: 'No ads.txt content to analyze',
    };
  }
  
  const content = adsTxtContent.toLowerCase();
  const premiumNetworks = [
    { name: 'google', domain: 'google.com' },
    { name: 'facebook', domain: 'facebook.com' },
    { name: 'amazon', domain: 'amazon-adsystem.com' },
    { name: 'unity', domain: 'unity.com' },
    { name: 'applovin', domain: 'applovin.com' },
    { name: 'ironsource', domain: 'ironsrc.com' },
    { name: 'mintegral', domain: 'mintegral.com' },
    { name: 'vungle', domain: 'vungle.com' },
  ];
  
  const found = premiumNetworks.filter(n => 
    content.includes(n.domain) || content.includes(n.name)
  );
  
  // Score: 0 = 0, 1 = 40, 2 = 70, 3+ = 100
  let score = 0;
  if (found.length >= 3) score = 100;
  else if (found.length === 2) score = 70;
  else if (found.length === 1) score = 40;
  
  return {
    signal: 'popular_network',
    score,
    weight: DEFAULT_WEIGHTS.popular_network,
    explanation: found.length > 0
      ? `Premium networks: ${found.map(n => n.name).join(', ')}`
      : 'No premium ad networks detected',
  };
}

/**
 * Score competitor network detection (The "Sniper" Signal)
 * 
 * This is the HIGHEST value signal. If an app is using a competitor
 * network (IronSource, Unity, AppLovin), they are actively monetizing
 * and are a prime target for conversion.
 */
function scoreCompetitorDetected(adsTxtContent?: string): HeuristicScore {
  if (!adsTxtContent) {
    return {
      signal: 'competitor_detected',
      score: 0,
      weight: DEFAULT_WEIGHTS.competitor_detected,
      explanation: 'No ads.txt content to analyze for competitors',
    };
  }
  
  const content = adsTxtContent.toLowerCase();
  const detectedCompetitors: Array<{ name: string; tier: number; boost: number }> = [];
  
  for (const competitor of TARGET_COMPETITORS) {
    if (content.includes(competitor.domain)) {
      detectedCompetitors.push({
        name: competitor.name,
        tier: competitor.tier,
        boost: competitor.boost,
      });
    }
  }
  
  if (detectedCompetitors.length === 0) {
    return {
      signal: 'competitor_detected',
      score: 0,
      weight: DEFAULT_WEIGHTS.competitor_detected,
      explanation: 'No competitor networks detected',
    };
  }
  
  // Calculate score based on best competitor found
  // Multiple competitors = even better (shows sophisticated monetization)
  const bestBoost = Math.max(...detectedCompetitors.map(c => c.boost));
  const multiCompetitorBonus = Math.min(20, (detectedCompetitors.length - 1) * 10);
  const score = Math.min(100, bestBoost + multiCompetitorBonus);
  
  const tier1Count = detectedCompetitors.filter(c => c.tier === 1).length;
  const competitorNames = detectedCompetitors.map(c => c.name).slice(0, 3).join(', ');
  
  return {
    signal: 'competitor_detected',
    score,
    weight: DEFAULT_WEIGHTS.competitor_detected,
    explanation: tier1Count > 0
      ? `🎯 HIGH VALUE: Using ${competitorNames} - prime conversion target!`
      : `Using competitors: ${competitorNames}`,
  };
}

/**
 * Score app genre fit (games and utilities have highest ad volume)
 */
function scoreAppGenreFit(genre?: string): HeuristicScore {
  if (!genre) {
    return {
      signal: 'app_genre_fit',
      score: 50, // Unknown = neutral
      weight: DEFAULT_WEIGHTS.app_genre_fit,
      explanation: 'App genre unknown',
    };
  }
  
  const genreLower = genre.toLowerCase();
  
  // High value genres (lots of ad inventory)
  const highValueGenres = ['game', 'arcade', 'casino', 'puzzle', 'action'];
  const mediumValueGenres = ['entertainment', 'music', 'video', 'social', 'news', 'weather', 'utilities'];
  const lowValueGenres = ['business', 'finance', 'medical', 'education', 'productivity'];
  
  let score = 50;
  let explanation = `Genre: ${genre}`;
  
  if (highValueGenres.some(g => genreLower.includes(g))) {
    score = 100;
    explanation = `High-value genre: ${genre} (games have high ad volume)`;
  } else if (mediumValueGenres.some(g => genreLower.includes(g))) {
    score = 70;
    explanation = `Medium-value genre: ${genre}`;
  } else if (lowValueGenres.some(g => genreLower.includes(g))) {
    score = 30;
    explanation = `Lower ad-value genre: ${genre}`;
  }
  
  return {
    signal: 'app_genre_fit',
    score,
    weight: DEFAULT_WEIGHTS.app_genre_fit,
    explanation,
  };
}

/**
 * Score email availability
 */
function scoreHasEmail(contact: Contact): HeuristicScore {
  const hasEmail = Boolean(contact.email);
  
  return {
    signal: 'has_email',
    score: hasEmail ? 100 : 0,
    weight: DEFAULT_WEIGHTS.has_email,
    explanation: hasEmail
      ? `Contact email available: ${contact.email}`
      : 'No contact email discovered',
  };
}

/**
 * Score website availability
 */
function scoreHasWebsite(contact: Contact): HeuristicScore {
  const hasWebsite = Boolean(contact.website);
  
  return {
    signal: 'has_website',
    score: hasWebsite ? 100 : 0,
    weight: DEFAULT_WEIGHTS.has_website,
    explanation: hasWebsite
      ? `Website: ${contact.website}`
      : 'No website found',
  };
}

/**
 * Extract networks from ads.txt content
 */
function extractNetworksFromAdsTxt(content: string): string[] {
  const networks = new Set<string>();
  const lines = content.split('\n');
  
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith('#') || !trimmed.includes(',')) continue;
    
    const domain = trimmed.split(',')[0].trim().toLowerCase();
    if (domain && domain.includes('.')) {
      networks.add(domain);
    }
  }
  
  return Array.from(networks);
}

/**
 * Score update velocity (Recency)
 */
function scoreUpdateVelocity(contact: Contact): HeuristicScore {
  let score = 0;
  let explanation = 'No update date available';

  if (contact.lastUpdated) {
     const now = new Date();
     const monthsAgo = (now.getTime() - contact.lastUpdated.getTime()) / (1000 * 60 * 60 * 24 * 30);
     
     if (monthsAgo < 1) { score = 100; explanation = 'Updated in last month'; }
     else if (monthsAgo < 3) { score = 80; explanation = 'Updated in last quarter'; }
     else if (monthsAgo < 6) { score = 60; explanation = 'Updated in last 6 months'; }
     else if (monthsAgo < 12) { score = 40; explanation = 'Updated in last year'; }
     else { score = 0; explanation = 'No updates in >1 year'; }
  }

  return {
    signal: 'update_velocity',
    score,
    weight: DEFAULT_WEIGHTS.update_velocity,
    explanation
  };
}

// ============================================================================
// LIFT PERCENTAGE CALCULATOR ("Truthful Puffery")
// ============================================================================

/**
 * Calculate estimated revenue lift percentage based on current ad setup
 * 
 * This turns the sales claim "15-30% lift" into a defensible, personalized insight.
 * The calculation is based on industry benchmarks:
 * 
 * - Single network (AdMob only): 25-35% potential lift from adding mediation
 * - 2-3 networks (basic setup): 15-20% potential lift from optimization
 * - 4+ networks (sophisticated): 8-15% potential lift from fine-tuning
 * - No ads.txt (not monetizing): Higher potential but harder to validate
 * 
 * Returns: { liftPercent, confidence, reasoning }
 */
export interface LiftEstimate {
  liftPercent: number;
  liftRange: string;
  confidence: 'high' | 'medium' | 'low';
  reasoning: string;
  currentSetup: string;
}

export function calculateLiftEstimate(adsTxtContent?: string): LiftEstimate {
  if (!adsTxtContent) {
    return {
      liftPercent: 25,
      liftRange: '20-30%',
      confidence: 'low',
      reasoning: 'No app-ads.txt found - unable to assess current setup',
      currentSetup: 'unknown',
    };
  }
  
  const networks = extractNetworksFromAdsTxt(adsTxtContent);
  const networkCount = networks.length;
  const content = adsTxtContent.toLowerCase();
  
  // Check for specific network patterns
  const hasGoogle = content.includes('google.com');
  const hasFacebook = content.includes('facebook.com');
  const hasUnity = content.includes('unity') || content.includes('unityads');
  const hasIronSource = content.includes('ironsrc') || content.includes('is.com');
  const hasAppLovin = content.includes('applovin');
  
  const premiumCount = [hasGoogle, hasFacebook, hasUnity, hasIronSource, hasAppLovin]
    .filter(Boolean).length;
  
  // Calculate lift based on current setup
  if (networkCount === 0) {
    return {
      liftPercent: 30,
      liftRange: '25-35%',
      confidence: 'low',
      reasoning: 'Empty or invalid app-ads.txt',
      currentSetup: 'none',
    };
  }
  
  if (networkCount === 1) {
    // Single network = biggest opportunity
    const networkName = hasGoogle ? 'Google AdMob' : 
                        hasFacebook ? 'Facebook Audience Network' :
                        hasUnity ? 'Unity Ads' :
                        networks[0] || 'single network';
    return {
      liftPercent: 30,
      liftRange: '25-35%',
      confidence: 'high',
      reasoning: `Currently using only ${networkName}. Adding 2-3 additional demand sources typically increases eCPMs by 25-35% through competitive waterfall optimization.`,
      currentSetup: `single-network (${networkName})`,
    };
  }
  
  if (networkCount <= 3) {
    // Basic multi-network setup
    return {
      liftPercent: 18,
      liftRange: '15-22%',
      confidence: 'high',
      reasoning: `Basic ${networkCount}-network setup detected. Expanding to 5+ networks with optimized waterfall bidding typically yields 15-22% lift.`,
      currentSetup: `basic (${networkCount} networks)`,
    };
  }
  
  if (networkCount <= 5) {
    // Medium sophistication
    return {
      liftPercent: 12,
      liftRange: '10-15%',
      confidence: 'medium',
      reasoning: `Moderate ${networkCount}-network setup. Fine-tuning waterfall order and adding header bidding typically improves revenue by 10-15%.`,
      currentSetup: `moderate (${networkCount} networks)`,
    };
  }
  
  // Sophisticated setup (6+ networks)
  return {
    liftPercent: 8,
    liftRange: '5-12%',
    confidence: 'medium',
    reasoning: `Sophisticated ${networkCount}-network setup with ${premiumCount} premium networks. Advanced optimization (A/B testing, segment-based waterfalls) can still yield 5-12% improvement.`,
    currentSetup: `sophisticated (${networkCount} networks, ${premiumCount} premium)`,
  };
}

/**
 * Save lift estimate to contact record
 */
export async function saveLiftEstimate(contactId: string, estimate: LiftEstimate): Promise<void> {
  await queryOperator(`
    UPDATE sa_contacts 
    SET 
      lift_percent = $1,
      lift_range = $2,
      lift_confidence = $3,
      lift_reasoning = $4,
      current_ad_setup = $5,
      updated_at = CURRENT_TIMESTAMP
    WHERE id = $6
  `, [
    estimate.liftPercent,
    estimate.liftRange,
    estimate.confidence,
    estimate.reasoning,
    estimate.currentSetup,
    contactId,
  ]);
}

// ============================================================================
// MAIN SCORING FUNCTION
// ============================================================================

/**
 * Calculate full lead score for a contact
 */
export function calculateLeadScore(contact: Contact): LeadScore {
  const signals: HeuristicScore[] = [
    scoreAdsTxtPresent(contact),
    scoreAdNetworkCount(contact.adsTxtContent),
    scorePopularNetworks(contact.adsTxtContent),
    scoreCompetitorDetected(contact.adsTxtContent), // THE SNIPER SIGNAL
    scoreAppGenreFit(contact.appGenre),
    scoreHasEmail(contact),
    scoreHasWebsite(contact),
    scoreUpdateVelocity(contact),
  ];
  
  // Calculate weighted average
  let weightedSum = 0;
  let totalWeight = 0;
  
  for (const signal of signals) {
    weightedSum += signal.score * signal.weight;
    totalWeight += signal.weight;
  }
  
  const totalScore = totalWeight > 0 ? Math.round(weightedSum / totalWeight) : 0;
  
  // Determine tier
  let tier: LeadScore['tier'];
  if (totalScore >= 70) tier = 'hot';
  else if (totalScore >= 50) tier = 'warm';
  else if (totalScore >= 30) tier = 'cold';
  else tier = 'ice';
  
  return {
    contactId: contact.id,
    totalScore,
    tier,
    signals,
    scoredAt: new Date(),
  };
}

// ============================================================================
// DATABASE OPERATIONS
// ============================================================================

/**
 * Get signal score by name
 */
function getSignalScore(signals: HeuristicScore[], signalName: string): number {
  const signal = signals.find(s => s.signal === signalName);
  return signal ? signal.score : 0;
}

/**
 * Save heuristic score to database
 * Schema: sa_heuristic_scores stores individual signal scores plus computed totals.
 * Migration 050 aligned the schema with this code's signal names.
 */
export async function saveLeadScore(score: LeadScore): Promise<void> {
  // Extract individual signal scores
  const adsTxtPresentScore = getSignalScore(score.signals, 'ads_txt_present');
  const adNetworkCountScore = getSignalScore(score.signals, 'ad_network_count');
  const popularNetworkScore = getSignalScore(score.signals, 'popular_network');
  const appGenreFitScore = getSignalScore(score.signals, 'app_genre_fit');
  const hasEmailScore = getSignalScore(score.signals, 'has_email');
  const hasWebsiteScore = getSignalScore(score.signals, 'has_website');
  
  // Store all signals as JSONB for reference
  const signalsJson = JSON.stringify(
    score.signals.reduce((acc, s) => {
      acc[s.signal] = { score: s.score, weight: s.weight, explanation: s.explanation };
      return acc;
    }, {} as Record<string, unknown>)
  );
  
  // Insert or update sa_heuristic_scores with actual signal columns
  await queryOperator(`
    INSERT INTO sa_heuristic_scores (
      contact_id, 
      ads_txt_present_score, ad_network_count_score, popular_network_score,
      app_genre_fit_score, has_email_score, has_website_score,
      overall_score, tier, signals
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
    ON CONFLICT (contact_id) 
    DO UPDATE SET 
      ads_txt_present_score = $2,
      ad_network_count_score = $3,
      popular_network_score = $4,
      app_genre_fit_score = $5,
      has_email_score = $6,
      has_website_score = $7,
      overall_score = $8,
      tier = $9,
      signals = $10,
      last_analyzed_at = CURRENT_TIMESTAMP,
      updated_at = CURRENT_TIMESTAMP
  `, [
    score.contactId,
    adsTxtPresentScore,
    adNetworkCountScore,
    popularNetworkScore,
    appGenreFitScore,
    hasEmailScore,
    hasWebsiteScore,
    score.totalScore,
    score.tier,
    signalsJson
  ]);
  
  // Update contact with total score (sa_contacts has 'score' column)
  await queryOperator(`
    UPDATE sa_contacts 
    SET score = $1, updated_at = CURRENT_TIMESTAMP
    WHERE id = $2
  `, [score.totalScore, score.contactId]);
  
  // SNIPER UPGRADE: Save competitor detections for targeted outreach
  // Re-detect competitors directly from ads.txt content (more robust than parsing explanation)
  const competitorSignal = score.signals.find(s => s.signal === 'competitor_detected');
  if (competitorSignal && competitorSignal.score > 0) {
    // Get the original contact to access ads.txt content directly
    const contact = await getContact(score.contactId);
    if (contact?.adsTxtContent) {
      const detectedCompetitors = detectCompetitorsFromAdsTxt(contact.adsTxtContent);
      if (detectedCompetitors.length > 0) {
        await saveCompetitorDetectionsFromList(score.contactId, detectedCompetitors);
      }
    }
  }
  
  logger.debug('Saved lead score', {
    contactId: score.contactId,
    score: score.totalScore,
    tier: score.tier,
  });
}

/**
 * Detect competitors directly from app-ads.txt content
 * Returns a list of detected competitor configs
 */
function detectCompetitorsFromAdsTxt(adsTxtContent: string): Array<{
  domain: string;
  name: string;
  tier: number;
  boost: number;
}> {
  const content = adsTxtContent.toLowerCase();
  const detected: typeof TARGET_COMPETITORS = [];
  
  for (const competitor of TARGET_COMPETITORS) {
    if (content.includes(competitor.domain)) {
      detected.push(competitor);
    }
  }
  
  return detected;
}

/**
 * Save competitor detections from structured list (robust version)
 * This is the preferred method - uses direct competitor data instead of parsing strings
 */
async function saveCompetitorDetectionsFromList(
  contactId: string, 
  competitors: Array<{ domain: string; name: string; tier: number; boost: number }>
): Promise<void> {
  try {
    for (const competitor of competitors) {
      await queryOperator(`
        INSERT INTO sa_competitor_detections (contact_id, competitor_domain, competitor_name, competitor_tier)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (contact_id, competitor_domain) DO UPDATE SET
          competitor_tier = $4,
          created_at = CURRENT_TIMESTAMP
      `, [contactId, competitor.domain, competitor.name, competitor.tier]);
    }
    
    // Update contact's competitor flags for quick filtering
    // Sort by tier (lowest = highest priority) and pick the best one
    const sortedCompetitors = [...competitors].sort((a, b) => a.tier - b.tier);
    const topCompetitor = sortedCompetitors[0];
    
    await queryOperator(`
      UPDATE sa_contacts 
      SET has_competitor = true, top_competitor = $1, competitor_tier = $2
      WHERE id = $3
    `, [topCompetitor.name, topCompetitor.tier, contactId]);
    
    logger.debug('Saved competitor detections', {
      contactId,
      competitors: competitors.map(c => c.name),
      topCompetitor: topCompetitor.name,
    });
    
  } catch (err) {
    // Don't fail scoring if competitor detection fails
    logger.warn('Failed to save competitor detections', { 
      contactId, 
      error: (err as Error).message 
    });
  }
}

/**
 * Save competitor detections to database for targeted outreach
 * @deprecated Use saveCompetitorDetectionsFromList instead - this parses from explanation string
 * This enables filtering contacts by which competitor they use
 */
async function saveCompetitorDetections(contactId: string, explanation: string): Promise<void> {
  try {
    // Parse competitor names from explanation
    // Format: "🎯 HIGH VALUE: Using IronSource, Unity Ads - prime conversion target!"
    // or: "Using competitors: Mintegral, Vungle"
    const names = explanation
      .replace(/.*:/, '') // Remove prefix
      .replace(/ - .*/, '') // Remove suffix
      .split(',')
      .map(n => n.trim())
      .filter(n => n.length > 0);
    
    for (const name of names) {
      // Find the competitor config
      const competitor = TARGET_COMPETITORS.find(c => 
        c.name.toLowerCase() === name.toLowerCase() ||
        c.name.toLowerCase().includes(name.toLowerCase())
      );
      
      if (!competitor) continue;
      
      await queryOperator(`
        INSERT INTO sa_competitor_detections (contact_id, competitor_domain, competitor_name, competitor_tier)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (contact_id, competitor_domain) DO UPDATE SET
          competitor_tier = $4,
          created_at = CURRENT_TIMESTAMP
      `, [contactId, competitor.domain, competitor.name, competitor.tier]);
    }
    
    // Update contact's competitor flags for quick filtering
    const topCompetitor = names[0];
    const topTier = TARGET_COMPETITORS.find(c => 
      c.name.toLowerCase().includes(topCompetitor?.toLowerCase() || '')
    )?.tier || 3;
    
    await queryOperator(`
      UPDATE sa_contacts 
      SET has_competitor = true, top_competitor = $1, competitor_tier = $2
      WHERE id = $3
    `, [topCompetitor, topTier, contactId]);
    
  } catch (err) {
    // Don't fail scoring if competitor detection fails
    logger.warn('Failed to save competitor detections', { 
      contactId, 
      error: (err as Error).message 
    });
  }
}

/**
 * Get contacts needing scoring
 */
export async function getContactsNeedingScoring(limit = 50): Promise<Contact[]> {
  const rows = await queryReadOnly<{
    id: string;
    company_name: string;
    website: string | null;
    email: string | null;
    app_store: string | null;
    app_id: string | null;
    app_genre: string | null;
    app_ads_txt_content: string | null;
  }>(`
    SELECT c.id, c.company_name, c.website, c.email, c.app_store, c.app_id, 
           c.app_genre, c.app_ads_txt_content
    FROM sa_contacts c
    LEFT JOIN sa_heuristic_scores h ON h.contact_id = c.id
    WHERE h.id IS NULL 
       OR h.last_analyzed_at < c.updated_at
       OR h.last_analyzed_at < CURRENT_TIMESTAMP - INTERVAL '7 days'
    ORDER BY c.created_at DESC
    LIMIT $1
  `, [limit]);
  
  return rows.map(row => ({
    id: row.id,
    companyName: row.company_name,
    website: row.website || undefined,
    email: row.email || undefined,
    appStore: row.app_store || undefined,
    appId: row.app_id || undefined,
    appGenre: row.app_genre || undefined,
    adsTxtContent: row.app_ads_txt_content || undefined,
  }));
}

/**
 * Get contact by ID
 */
export async function getContact(contactId: string): Promise<Contact | null> {
  const rows = await queryReadOnly<{
    id: string;
    company_name: string;
    website: string | null;
    email: string | null;
    app_store: string | null;
    app_id: string | null;
    app_genre: string | null;
    app_ads_txt_content: string | null;
  }>(`
    SELECT id, company_name, website, email, app_store, app_id, 
           app_genre, app_ads_txt_content
    FROM sa_contacts
    WHERE id = $1
  `, [contactId]);
  
  if (rows.length === 0) return null;
  
  const row = rows[0];
  return {
    id: row.id,
    companyName: row.company_name,
    website: row.website || undefined,
    email: row.email || undefined,
    appStore: row.app_store || undefined,
    appId: row.app_id || undefined,
    appGenre: row.app_genre || undefined,
    adsTxtContent: row.app_ads_txt_content || undefined,
  };
}

// ============================================================================
// ANALYST CLASS (Main Orchestrator)
// ============================================================================

export class Analyst {
  /**
   * Score a batch of contacts
   */
  async scoreBatch(limit = 50): Promise<{
    scored: number;
    byTier: Record<string, number>;
  }> {
    const contacts = await getContactsNeedingScoring(limit);
    const byTier: Record<string, number> = { hot: 0, warm: 0, cold: 0, ice: 0 };
    
    for (const contact of contacts) {
      try {
        // Refresh app-ads.txt if we have a website and haven't scraped recently
        if (contact.website && !contact.adsTxtContent) {
          const adsTxt = await checkAppAdsTxt(contact.website);
          if (adsTxt.hasAdsTxt) {
            contact.adsTxtContent = adsTxt.content;
          }
        }
        
        const score = calculateLeadScore(contact);
        await saveLeadScore(score);
        byTier[score.tier]++;
        
      } catch (err) {
        logger.warn('Failed to score contact', {
          contactId: contact.id,
          error: (err as Error).message,
        });
      }
    }
    
    logger.info('Analyst batch completed', {
      scored: contacts.length,
      byTier,
    });
    
    return {
      scored: contacts.length,
      byTier,
    };
  }
  
  /**
   * Score a single contact
   */
  async scoreContact(contactId: string): Promise<LeadScore | null> {
    const contact = await getContact(contactId);
    if (!contact) return null;
    
    // Refresh app-ads.txt if we have a website
    if (contact.website) {
      const adsTxt = await checkAppAdsTxt(contact.website);
      if (adsTxt.hasAdsTxt) {
        contact.adsTxtContent = adsTxt.content;
      }
    }
    
    const score = calculateLeadScore(contact);
    await saveLeadScore(score);
    
    return score;
  }
  
  /**
   * Get scoring statistics
   */
  async getStats(): Promise<{
    totalContacts: number;
    scoredContacts: number;
    tierDistribution: Record<string, number>;
    averageScore: number;
  }> {
    // Get counts from sa_contacts (score column exists)
    const stats = await queryReadOnly<{
      total_contacts: string;
      scored_contacts: string;
      avg_score: string;
    }>(`
      SELECT 
        COUNT(*) as total_contacts,
        COUNT(CASE WHEN score > 0 THEN 1 END) as scored_contacts,
        COALESCE(AVG(NULLIF(score, 0)), 0) as avg_score
      FROM sa_contacts
    `);
    
    // Get tier distribution from sa_heuristic_scores (tier is auto-generated)
    const tierDist = await queryReadOnly<{
      tier: string;
      count: string;
    }>(`
      SELECT tier, COUNT(*) as count
      FROM sa_heuristic_scores
      WHERE tier IS NOT NULL
      GROUP BY tier
    `);
    
    const tierDistribution: Record<string, number> = {};
    for (const row of tierDist) {
      tierDistribution[row.tier] = parseInt(row.count);
    }
    
    return {
      totalContacts: parseInt(stats[0].total_contacts),
      scoredContacts: parseInt(stats[0].scored_contacts),
      tierDistribution,
      averageScore: parseFloat(stats[0].avg_score),
    };
  }
}
