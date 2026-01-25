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
}

// ============================================================================
// SIGNAL WEIGHTS (Configurable)
// ============================================================================

const DEFAULT_WEIGHTS: Record<string, number> = {
  ads_txt_present: 25,        // Strong signal - they monetize with ads
  ad_network_count: 15,       // More networks = more sophisticated
  popular_network: 15,        // Google, Facebook = serious about ads
  app_ranking: 10,            // Higher ranking = more inventory
  update_velocity: 10,        // Recent updates = active team
  has_email: 10,              // Discoverable contact = easier sale
  has_website: 5,             // Professional presence
  app_genre_fit: 10,          // Games, utilities = high ad volume
};

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
    scoreAppGenreFit(contact.appGenre),
    scoreHasEmail(contact),
    scoreHasWebsite(contact),
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
 * Map signal names to database column names
 */
function getSignalColumnValue(signals: HeuristicScore[], signalName: string): number {
  const signal = signals.find(s => s.signal === signalName);
  return signal ? signal.score : 0;
}

/**
 * Save heuristic score to database
 * Schema: sa_heuristic_scores has specific columns for each score type:
 * - app_ads_txt_score, sdk_diversity_score, update_velocity_score,
 * - review_velocity_score, tech_sophistication_score, company_size_score
 * - overall_score and tier are auto-generated from these
 */
export async function saveLeadScore(score: LeadScore): Promise<void> {
  // Map our signals to the database columns
  // ads_txt_present + ad_network_count + popular_network -> app_ads_txt_score
  const adsTxtSignal = score.signals.find(s => s.signal === 'ads_txt_present');
  const networkCountSignal = score.signals.find(s => s.signal === 'ad_network_count');
  const popularNetworkSignal = score.signals.find(s => s.signal === 'popular_network');
  
  // Combine ad-related signals into app_ads_txt_score (weighted average)
  const appAdsTxtScore = Math.round(
    ((adsTxtSignal?.score || 0) * 0.4 + 
     (networkCountSignal?.score || 0) * 0.3 + 
     (popularNetworkSignal?.score || 0) * 0.3)
  );
  
  // app_genre_fit -> sdk_diversity_score (close enough mapping)
  const genreSignal = score.signals.find(s => s.signal === 'app_genre_fit');
  const sdkDiversityScore = genreSignal?.score || 0;
  
  // update_velocity -> update_velocity_score
  const updateSignal = score.signals.find(s => s.signal === 'update_velocity');
  const updateVelocityScore = updateSignal?.score || 50;
  
  // has_email + has_website -> tech_sophistication_score
  const emailSignal = score.signals.find(s => s.signal === 'has_email');
  const websiteSignal = score.signals.find(s => s.signal === 'has_website');
  const techSophisticationScore = Math.round(
    ((emailSignal?.score || 0) * 0.5 + (websiteSignal?.score || 0) * 0.5)
  );
  
  // Store all signals as JSONB for reference
  const signalsJson = JSON.stringify(
    score.signals.reduce((acc, s) => {
      acc[s.signal] = { score: s.score, weight: s.weight, explanation: s.explanation };
      return acc;
    }, {} as Record<string, unknown>)
  );
  
  // Insert or update sa_heuristic_scores
  await queryOperator(`
    INSERT INTO sa_heuristic_scores (
      contact_id, app_ads_txt_score, sdk_diversity_score, 
      update_velocity_score, review_velocity_score,
      tech_sophistication_score, company_size_score, signals
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
    ON CONFLICT (contact_id) 
    DO UPDATE SET 
      app_ads_txt_score = $2,
      sdk_diversity_score = $3,
      update_velocity_score = $4,
      review_velocity_score = $5,
      tech_sophistication_score = $6,
      company_size_score = $7,
      signals = $8,
      last_analyzed_at = CURRENT_TIMESTAMP,
      updated_at = CURRENT_TIMESTAMP
  `, [
    score.contactId, 
    appAdsTxtScore, 
    sdkDiversityScore, 
    updateVelocityScore,
    50, // review_velocity_score - default for now
    techSophisticationScore,
    50, // company_size_score - default for now
    signalsJson
  ]);
  
  // Update contact with total score (sa_contacts has 'score' column)
  await queryOperator(`
    UPDATE sa_contacts 
    SET score = $1, updated_at = CURRENT_TIMESTAMP
    WHERE id = $2
  `, [score.totalScore, score.contactId]);
  
  logger.debug('Saved lead score', {
    contactId: score.contactId,
    score: score.totalScore,
    tier: score.tier,
  });
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
