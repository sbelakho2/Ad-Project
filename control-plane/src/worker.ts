/**
 * SALES AUTOMATION WORKER
 * 
 * Runs on the TEST SERVER (157.90.126.205) and communicates with the
 * Control Plane API on the APP SERVER (admin.apexmediation.ee).
 * 
 * This separation ensures:
 *   - Scraping/automation runs on isolated test infrastructure
 *   - Production app server isn't burdened with scraping tasks
 *   - Clean API boundary between worker and control plane
 * 
 * Jobs:
 *   - Discovery: Scrape app stores for leads, submit to Control Plane
 *   - Scoring: Calculate lead scores, submit to Control Plane
 * 
 * IMPORTANT: This file has NO database dependencies!
 * All data flows through the Control Plane API.
 */

import { HunterStandalone, type SourceConfig, type DiscoveredApp } from './services/discovery/hunter-standalone';

// Simple logger (no winston dependency needed)
const log = {
  info: (msg: string, meta?: Record<string, unknown>) => console.log(`[INFO] ${msg}`, meta ? JSON.stringify(meta) : ''),
  warn: (msg: string, meta?: Record<string, unknown>) => console.warn(`[WARN] ${msg}`, meta ? JSON.stringify(meta) : ''),
  error: (msg: string, meta?: Record<string, unknown>) => console.error(`[ERROR] ${msg}`, meta ? JSON.stringify(meta) : ''),
};

// ============================================================================
// CONFIGURATION
// ============================================================================

const CONTROL_PLANE_URL = process.env.CONTROL_PLANE_URL || 'https://admin.apexmediation.ee';
const CONTROL_PLANE_API_KEY = process.env.CONTROL_PLANE_API_KEY || '';
const WORKER_ID = process.env.WORKER_ID || `worker-${Date.now()}`;
const WORKER_INTERVAL_MS = parseInt(process.env.WORKER_INTERVAL_MS || '60000', 10);

// ============================================================================
// CONTROL PLANE API CLIENT
// ============================================================================

class ControlPlaneClient {
  private baseUrl: string;
  private apiKey: string;
  
  constructor(baseUrl: string, apiKey: string) {
    this.baseUrl = baseUrl.replace(/\/$/, '');
    this.apiKey = apiKey;
  }
  
  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const url = `${this.baseUrl}/api/autonomous${path}`;
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      'X-Worker-ID': WORKER_ID,
    };
    
    if (this.apiKey) {
      headers['X-Api-Key'] = this.apiKey;
    }
    
    const response = await fetch(url, {
      method,
      headers,
      body: body ? JSON.stringify(body) : undefined,
    });
    
    if (!response.ok) {
      const text = await response.text();
      throw new Error(`API error ${response.status}: ${text}`);
    }
    
    return response.json() as Promise<T>;
  }
  
  // Health check
  async health(): Promise<{ status: string }> {
    return this.request('GET', '/health');
  }
  
  // Get sources due for scraping
  async getSourcesDueForScrape(): Promise<{ 
    success: boolean; 
    sources: Array<{
      id: string;
      sourceName: string;
      sourceType: string;
      sourceConfig: Record<string, unknown>;
      scrapeIntervalHours?: number;
    }>;
  }> {
    return this.request('GET', '/sources?status=active&due=true');
  }
  
  // Report scrape results and submit discovered apps
  async reportScrapeResults(
    sourceId: string, 
    result: { 
      success: boolean;
      appsFound: number; 
      apps: DiscoveredApp[];
      error?: string;
    }
  ): Promise<{ success: boolean; inserted: number; updated: number }> {
    return this.request('POST', `/sources/${sourceId}/scraped`, result);
  }
  
  // Get contacts needing scoring
  async getContactsNeedingScoring(limit: number): Promise<{ 
    success: boolean;
    contacts: Array<{
      id: string;
      companyName: string;
      website?: string;
      email?: string;
      appStore?: string;
      appId?: string;
      appGenre?: string;
      adsTxtContent?: string;
    }>;
  }> {
    return this.request('GET', `/contacts/unscored?limit=${limit}`);
  }
  
  // Submit lead score
  async submitLeadScore(
    contactId: string, 
    score: {
      totalScore: number;
      tier: string;
      signals: Array<{ signal: string; score: number; weight: number; explanation: string }>;
    }
  ): Promise<{ success: boolean }> {
    return this.request('POST', `/contacts/${contactId}/score`, score);
  }
  
  // Get stats
  async getStats(): Promise<unknown> {
    return this.request('GET', '/stats');
  }
}

// ============================================================================
// LOCAL SCORING (No DB required)
// ============================================================================

interface Contact {
  id: string;
  companyName: string;
  website?: string;
  email?: string;
  appStore?: string;
  appId?: string;
  appGenre?: string;
  adsTxtContent?: string;
}

interface HeuristicScore {
  signal: string;
  score: number;
  weight: number;
  explanation: string;
}

const WEIGHTS: Record<string, number> = {
  ads_txt_present: 25,
  ad_network_count: 15,
  popular_network: 15,
  app_genre_fit: 10,
  has_email: 10,
  has_website: 5,
};

const POPULAR_NETWORKS = ['google.com', 'facebook.com', 'applovin.com', 'unity3d.com', 'ironsrc.com', 'mopub.com'];
const HIGH_VALUE_GENRES = ['games', 'game', 'arcade', 'puzzle', 'casual', 'action', 'adventure', 'simulation'];

function calculateLocalScore(contact: Contact): { totalScore: number; tier: string; signals: HeuristicScore[] } {
  const signals: HeuristicScore[] = [];
  
  // Score ads.txt presence
  const hasAdsTxt = Boolean(contact.adsTxtContent);
  signals.push({
    signal: 'ads_txt_present',
    score: hasAdsTxt ? 100 : 0,
    weight: WEIGHTS.ads_txt_present,
    explanation: hasAdsTxt ? 'Has app-ads.txt (monetizing)' : 'No app-ads.txt',
  });
  
  // Score ad network count
  let networkCount = 0;
  let hasPopularNetwork = false;
  if (contact.adsTxtContent) {
    const lines = contact.adsTxtContent.split('\n').filter(l => !l.startsWith('#') && l.includes(','));
    networkCount = lines.length;
    hasPopularNetwork = POPULAR_NETWORKS.some(n => contact.adsTxtContent!.toLowerCase().includes(n));
  }
  signals.push({
    signal: 'ad_network_count',
    score: Math.min(networkCount * 10, 100),
    weight: WEIGHTS.ad_network_count,
    explanation: `${networkCount} ad networks found`,
  });
  
  signals.push({
    signal: 'popular_network',
    score: hasPopularNetwork ? 100 : 0,
    weight: WEIGHTS.popular_network,
    explanation: hasPopularNetwork ? 'Uses popular ad network' : 'No popular networks',
  });
  
  // Score genre fit
  const genreFit = contact.appGenre && HIGH_VALUE_GENRES.some(g => contact.appGenre!.toLowerCase().includes(g));
  signals.push({
    signal: 'app_genre_fit',
    score: genreFit ? 100 : 30,
    weight: WEIGHTS.app_genre_fit,
    explanation: genreFit ? `High-value genre: ${contact.appGenre}` : 'Standard genre',
  });
  
  // Score email presence
  signals.push({
    signal: 'has_email',
    score: contact.email ? 100 : 0,
    weight: WEIGHTS.has_email,
    explanation: contact.email ? 'Has contact email' : 'No email found',
  });
  
  // Score website presence
  signals.push({
    signal: 'has_website',
    score: contact.website ? 100 : 0,
    weight: WEIGHTS.has_website,
    explanation: contact.website ? 'Has website' : 'No website',
  });
  
  // Calculate weighted total
  let weightedSum = 0;
  let totalWeight = 0;
  for (const s of signals) {
    weightedSum += s.score * s.weight;
    totalWeight += s.weight;
  }
  const totalScore = Math.round(weightedSum / totalWeight);
  
  // Determine tier
  let tier: string;
  if (totalScore >= 70) tier = 'hot';
  else if (totalScore >= 50) tier = 'warm';
  else if (totalScore >= 30) tier = 'cold';
  else tier = 'ice';
  
  return { totalScore, tier, signals };
}

// ============================================================================
// WORKER CLASS
// ============================================================================

export class SalesAutomationWorker {
  private client: ControlPlaneClient;
  private hunter: HunterStandalone;
  private isRunning = false;
  
  constructor() {
    this.client = new ControlPlaneClient(CONTROL_PLANE_URL, CONTROL_PLANE_API_KEY);
    this.hunter = new HunterStandalone({ headless: true, requestDelayMs: 2000 });
  }
  
  /**
   * Run a discovery job (scrape app stores, submit results to Control Plane)
   */
  async runDiscoveryJob(batchSize = 3): Promise<{ processed: number; errors: number }> {
    log.info('Starting discovery job', { workerId: WORKER_ID, batchSize });
    
    let processed = 0;
    let errors = 0;
    
    try {
      // Get sources due for scraping from Control Plane
      const { sources } = await this.client.getSourcesDueForScrape();
      const sourcesToProcess = sources.slice(0, batchSize);
      
      log.info('Sources to process', { count: sourcesToProcess.length });
      
      for (const source of sourcesToProcess) {
        try {
          log.info('Processing source', { sourceId: source.id, type: source.sourceType });
          
          // Scrape locally (no DB)
          const scrapeResult = await this.hunter.scrapeSource({
            id: source.id,
            sourceType: source.sourceType as 'google_play' | 'itunes' | 'product_hunt',
            sourceConfig: source.sourceConfig as SourceConfig['sourceConfig'],
          });
          
          // Submit results to Control Plane
          const apiResult = await this.client.reportScrapeResults(source.id, {
            success: scrapeResult.success,
            appsFound: scrapeResult.appsFound,
            apps: scrapeResult.apps,
            error: scrapeResult.error,
          });
          
          processed++;
          log.info('Source processed', { 
            sourceId: source.id, 
            appsFound: scrapeResult.appsFound,
            inserted: apiResult.inserted,
            updated: apiResult.updated,
          });
          
        } catch (err) {
          log.error('Source processing failed', { 
            sourceId: source.id, 
            error: (err as Error).message 
          });
          errors++;
        }
      }
      
    } catch (err) {
      log.error('Discovery job failed', { error: (err as Error).message });
      errors++;
    }
    
    log.info('Discovery job completed', { processed, errors });
    return { processed, errors };
  }
  
  /**
   * Run a scoring job (get contacts, score locally, submit to Control Plane)
   */
  async runScoringJob(batchSize = 50): Promise<{ processed: number; errors: number }> {
    log.info('Starting scoring job', { workerId: WORKER_ID, batchSize });
    
    let processed = 0;
    let errors = 0;
    
    try {
      // Get contacts needing scoring from Control Plane
      const { contacts } = await this.client.getContactsNeedingScoring(batchSize);
      
      log.info('Contacts to score', { count: contacts.length });
      
      for (const contact of contacts) {
        try {
          // Calculate score locally
          const score = calculateLocalScore(contact);
          
          // Submit to Control Plane
          await this.client.submitLeadScore(contact.id, score);
          
          processed++;
          
        } catch (err) {
          log.error('Contact scoring failed', { 
            contactId: contact.id, 
            error: (err as Error).message 
          });
          errors++;
        }
      }
      
    } catch (err) {
      log.error('Scoring job failed', { error: (err as Error).message });
      errors++;
    }
    
    log.info('Scoring job completed', { processed, errors });
    return { processed, errors };
  }
  
  /**
   * Health check - verify connection to control plane
   */
  async healthCheck(): Promise<boolean> {
    try {
      const result = await this.client.health();
      return result.status === 'healthy';
    } catch (err) {
      log.error('Health check failed', { error: (err as Error).message });
      return false;
    }
  }
  
  /**
   * Get stats from control plane
   */
  async getStats(): Promise<unknown> {
    return this.client.getStats();
  }
  
  /**
   * Start worker loop
   */
  async start(intervalMs = WORKER_INTERVAL_MS): Promise<void> {
    this.isRunning = true;
    log.info('Sales Automation Worker starting', { 
      workerId: WORKER_ID,
      controlPlane: CONTROL_PLANE_URL,
      intervalMs,
    });
    
    // Initial health check
    const healthy = await this.healthCheck();
    if (!healthy) {
      log.error('Control plane not reachable - worker will retry');
    } else {
      log.info('Control plane connection verified');
    }
    
    // Main loop
    while (this.isRunning) {
      try {
        // Run discovery job
        await this.runDiscoveryJob(3);
        
        // Run scoring job
        await this.runScoringJob(50);
        
      } catch (err) {
        log.error('Worker loop error', { error: (err as Error).message });
      }
      
      // Wait for next iteration
      await new Promise(resolve => setTimeout(resolve, intervalMs));
    }
  }
  
  /**
   * Stop worker
   */
  stop(): void {
    this.isRunning = false;
    log.info('Sales Automation Worker stopping');
  }
  
  /**
   * Cleanup resources
   */
  async shutdown(): Promise<void> {
    this.stop();
    await this.hunter.close();
    log.info('Sales Automation Worker shut down');
  }
}

// ============================================================================
// CLI ENTRY POINT
// ============================================================================

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  const command = args[0] || 'start';
  
  const worker = new SalesAutomationWorker();
  
  switch (command) {
    case 'start':
      // Handle shutdown signals
      process.on('SIGINT', () => worker.shutdown());
      process.on('SIGTERM', () => worker.shutdown());
      
      await worker.start();
      break;
      
    case 'health':
      const healthy = await worker.healthCheck();
      console.log(healthy ? 'Control plane: HEALTHY' : 'Control plane: UNREACHABLE');
      process.exit(healthy ? 0 : 1);
      break;
      
    case 'discover':
      const batchSize = parseInt(args[1] || '3', 10);
      const discoverResult = await worker.runDiscoveryJob(batchSize);
      console.log('Discovery result:', discoverResult);
      await worker.shutdown();
      break;
      
    case 'score':
      const scoreBatchSize = parseInt(args[1] || '50', 10);
      const scoreResult = await worker.runScoringJob(scoreBatchSize);
      console.log('Scoring result:', scoreResult);
      await worker.shutdown();
      break;
      
    case 'stats':
      const stats = await worker.getStats();
      console.log(JSON.stringify(stats, null, 2));
      break;
      
    default:
      console.log(`
Sales Automation Worker

Usage:
  node worker.js start           Start the worker daemon
  node worker.js health          Check control plane connection
  node worker.js discover [n]    Run one discovery job (n sources)
  node worker.js score [n]       Run one scoring job (n contacts)
  node worker.js stats           Get stats from control plane

Environment:
  CONTROL_PLANE_URL      Control plane API URL (default: https://admin.apexmediation.ee)
  CONTROL_PLANE_API_KEY  API key for authentication (optional)
  WORKER_ID              Unique worker identifier
  WORKER_INTERVAL_MS     Loop interval in ms (default: 60000)
`);
  }
}

// Run if called directly
main().catch(err => {
  console.error('Fatal error:', err);
  process.exit(1);
});
