/**
 * HUNTER MODULE (The Prospector)
 * 
 * Zero-cost lead discovery using Playwright browser automation.
 * Scrapes app stores and public data for potential customers.
 * 
 * Sources:
 *   - Google Play Store (apps in target categories)
 *   - App Store (via iTunes API - no scraping needed)
 *   - Product Hunt (recently launched apps)
 *   - app-ads.txt files (for monetization signals)
 * 
 * Ethical Guidelines:
 *   - Respects robots.txt
 *   - Rate-limited to 1 request per 2 seconds
 *   - Only scrapes publicly available data
 *   - Caches results to minimize requests
 * 
 * NOTE: Playwright scraping is optional. If playwright is not installed,
 * only the iTunes API and app-ads.txt checking will work.
 */

import { queryOperator, queryReadOnly } from '../../database';
import { logger } from '../../logger';

// Dynamic import for Playwright (optional dependency)
let playwrightModule: typeof import('playwright') | null = null;
async function getPlaywright(): Promise<typeof import('playwright') | null> {
  if (playwrightModule === undefined) {
    try {
      playwrightModule = await import('playwright');
    } catch {
      logger.warn('Playwright not available - Google Play scraping disabled');
      playwrightModule = null;
    }
  }
  return playwrightModule;
}

type Browser = import('playwright').Browser;
type Page = import('playwright').Page;
type BrowserContext = import('playwright').BrowserContext;

// ============================================================================
// TYPES
// ============================================================================

export interface DiscoveredApp {
  appStore: 'google_play' | 'app_store' | 'product_hunt';
  appId: string;
  appName: string;
  developerName: string;
  developerWebsite?: string;
  developerEmail?: string;
  genre?: string;
  description?: string;
  installs?: string;
  rating?: number;
  hasAdsTxt?: boolean;
  adsTxtContent?: string;
  adNetworks?: string[];
  lastUpdated?: Date;
  scrapedAt: Date;
}

export interface ScrapeJob {
  id: string;
  sourceId: string;
  status: 'pending' | 'running' | 'completed' | 'failed';
  appsDiscovered: number;
  appsProcessed: number;
  errorMessage?: string;
  startedAt?: Date;
  completedAt?: Date;
}

export interface HunterConfig {
  headless?: boolean;
  requestDelayMs?: number;
  maxPagesPerSource?: number;
  userAgent?: string;
  /** 
   * Proxy URL for browser requests (e.g., http://user:pass@proxy.example.com:8080)
   * CRITICAL for production: Google Play will block Hetzner IPs immediately.
   * Use a residential proxy provider like Bright Data, Smartproxy, or Oxylabs.
   */
  proxyUrl?: string;
}

// ============================================================================
// BROWSER UTILITIES
// ============================================================================

const DEFAULT_USER_AGENT = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';

let browserInstance: Browser | null = null;
let currentProxyUrl: string | undefined = undefined;

/**
 * Get or create a shared browser instance
 * 
 * NOTE: If proxyUrl changes, browser is recreated with new proxy.
 */
async function getBrowser(config: HunterConfig): Promise<Browser | null> {
  const playwright = await getPlaywright();
  if (!playwright) return null;
  
  // If proxy changed, recreate browser
  if (browserInstance && currentProxyUrl !== config.proxyUrl) {
    await browserInstance.close();
    browserInstance = null;
  }
  
  if (!browserInstance || !browserInstance.isConnected()) {
    const launchOptions: { headless: boolean; proxy?: { server: string } } = {
      headless: config.headless ?? true,
    };
    
    // Add proxy if configured
    if (config.proxyUrl) {
      launchOptions.proxy = { server: config.proxyUrl };
      logger.info('[Hunter] Launching browser with proxy', { 
        proxyHost: new URL(config.proxyUrl).host 
      });
    } else {
      logger.warn('[Hunter] Launching browser WITHOUT proxy - may get blocked by Google Play');
    }
    
    browserInstance = await playwright.chromium.launch(launchOptions);
    currentProxyUrl = config.proxyUrl;
  }
  return browserInstance;
}

/**
 * Create a new context with stealth settings
 */
async function createStealthContext(browser: Browser | null, config: HunterConfig): Promise<BrowserContext | null> {
  if (!browser) return null;
  
  return browser.newContext({
    userAgent: config.userAgent || DEFAULT_USER_AGENT,
    viewport: { width: 1920, height: 1080 },
    locale: 'en-US',
    timezoneId: 'America/New_York',
    deviceScaleFactor: 1,
    isMobile: false,
    hasTouch: false,
    javaScriptEnabled: true,
  });
}

/**
 * Rate-limited delay
 */
function delay(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

// ============================================================================
// GOOGLE PLAY SCRAPER
// ============================================================================

export async function scrapeGooglePlayCategory(
  category: string,
  config: HunterConfig = {}
): Promise<DiscoveredApp[]> {
  const apps: DiscoveredApp[] = [];
  const requestDelay = config.requestDelayMs || 2000;
  const maxPages = config.maxPagesPerSource || 5;
  
  const browser = await getBrowser(config);
  if (!browser) {
    logger.warn('Playwright not available, skipping Google Play scrape');
    return apps;
  }
  
  const context = await createStealthContext(browser, config);
  if (!context) return apps;
  
  const page = await context.newPage();
  
  try {
    logger.info('Starting Google Play scrape', { category });
    
    // Navigate to category page
    const categoryUrl = `https://play.google.com/store/apps/category/${category}/collection/topselling_free`;
    await page.goto(categoryUrl, { waitUntil: 'networkidle' });
    await delay(requestDelay);
    
    // Scroll to load more apps
    for (let i = 0; i < maxPages; i++) {
      await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
      await delay(1000);
    }
    
    // Extract app links
    const appLinks = await page.$$eval('a[href*="/store/apps/details"]', links =>
      [...new Set(links.map(a => a.getAttribute('href')).filter(Boolean))]
    );
    
    logger.info('Found app links', { count: appLinks.length, category });
    
    // Process each app (limited to avoid rate limiting)
    const appsToProcess = appLinks.slice(0, 20);
    
    for (const link of appsToProcess) {
      try {
        const appId = new URLSearchParams(link!.split('?')[1]).get('id');
        if (!appId) continue;
        
        await delay(requestDelay);
        const appData = await scrapeGooglePlayApp(page, appId, requestDelay);
        if (appData) {
          apps.push(appData);
        }
      } catch (err) {
        logger.warn('Failed to scrape app', { link, error: (err as Error).message });
      }
    }
    
  } finally {
    await context.close();
  }
  
  return apps;
}

async function scrapeGooglePlayApp(
  page: Page,
  appId: string,
  delayMs: number
): Promise<DiscoveredApp | null> {
  const url = `https://play.google.com/store/apps/details?id=${appId}`;
  await page.goto(url, { waitUntil: 'networkidle' });
  await delay(delayMs / 2);
  
  try {
    // Extract app details
    const appName = await page.$eval('h1[itemprop="name"]', el => el.textContent?.trim() || '').catch(() => '');
    const developerName = await page.$eval('a[href*="/store/apps/dev"]', el => el.textContent?.trim() || '').catch(() => '');
    const description = await page.$eval('[data-g-id="description"]', el => el.textContent?.trim().slice(0, 500) || '').catch(() => '');
    const genre = await page.$eval('[itemprop="genre"]', el => el.textContent?.trim() || '').catch(() => '');
    const installs = await page.$eval('.ClM7O', el => el.textContent?.trim() || '').catch(() => '');
    const ratingText = await page.$eval('[itemprop="starRating"] [aria-label]', el => el.getAttribute('aria-label') || '').catch(() => '');
    const rating = parseFloat(ratingText.match(/[\d.]+/)?.[0] || '0');

    // Extract Last Updated date
    const lastUpdatedText = await page.evaluate(() => {
        const divs = Array.from(document.querySelectorAll('div'));
        const label = divs.find(d => d.textContent === 'Updated on');
        return label ? label.nextElementSibling?.textContent?.trim() : '';
    }).catch(() => '');
    
    let lastUpdated: Date | undefined;
    if (lastUpdatedText) {
        const parsed = new Date(lastUpdatedText);
        if (!isNaN(parsed.getTime())) lastUpdated = parsed;
    }
    
    // Try to find developer email in description or privacy policy
    const developerEmail = await extractDeveloperEmail(page, delayMs);
    
    // Try to find developer website
    const developerWebsite = await page.$eval('a[href*="developer-"]', el => {
      const href = el.getAttribute('href');
      if (href?.includes('url=')) {
        return decodeURIComponent(href.split('url=')[1].split('&')[0]);
      }
      return '';
    }).catch(() => '');
    
    return {
      appStore: 'google_play',
      appId,
      appName,
      developerName,
      developerWebsite: developerWebsite || undefined,
      developerEmail: developerEmail || undefined,
      genre: genre || undefined,
      description: description || undefined,
      installs: installs || undefined,
      rating: rating || undefined,
      lastUpdated: lastUpdated || undefined,
      scrapedAt: new Date(),
    };
  } catch (err) {
    logger.warn('Error parsing app page', { appId, error: (err as Error).message });
    return null;
  }
}

async function extractDeveloperEmail(page: Page, delayMs: number): Promise<string | null> {
  try {
    // Click to expand developer info if available
    const expandButton = await page.$('button[aria-label*="Developer"]');
    if (expandButton) {
      await expandButton.click();
      await delay(500);
    }
    
    // Look for email in expanded section
    const pageContent = await page.content();
    const emailMatch = pageContent.match(/[\w.-]+@[\w.-]+\.[a-z]{2,}/i);
    
    if (emailMatch) {
      // Filter out common non-developer emails
      const email = emailMatch[0].toLowerCase();
      if (!email.includes('google.com') && !email.includes('example.com')) {
        return email;
      }
    }
    
    return null;
  } catch {
    return null;
  }
}

// ============================================================================
// ITUNES API (No scraping needed - Free public API)
// ============================================================================

export async function searchITunesApps(
  term: string,
  limit = 50
): Promise<DiscoveredApp[]> {
  const url = `https://itunes.apple.com/search?term=${encodeURIComponent(term)}&entity=software&limit=${limit}`;
  
  try {
    const response = await fetch(url);
    const data = await response.json() as {
      results: Array<{
        trackId: number;
        trackName: string;
        artistName: string;
        sellerUrl?: string;
        primaryGenreName: string;
        description: string;
        averageUserRating?: number;
        currentVersionReleaseDate: string;
      }>;
    };
    
    return data.results.map(app => ({
      appStore: 'app_store' as const,
      appId: String(app.trackId),
      appName: app.trackName,
      developerName: app.artistName,
      developerWebsite: app.sellerUrl,
      genre: app.primaryGenreName,
      description: app.description?.slice(0, 500),
      rating: app.averageUserRating,
      lastUpdated: new Date(app.currentVersionReleaseDate),
      scrapedAt: new Date(),
    }));
  } catch (err) {
    logger.error('iTunes API error', { error: (err as Error).message });
    return [];
  }
}

// ============================================================================
// APP-ADS.TXT CHECKER (Upgraded with robust URL handling)
// ============================================================================

/**
 * Check for app-ads.txt file at a website URL
 * 
 * UPGRADES from original implementation:
 * 1. Tries multiple URL permutations (www vs non-www, http vs https)
 * 2. Follows redirects up to 3 levels
 * 3. Distinguishes 404 (no ads.txt) from connection errors (scrape failed)
 * 4. Logs detailed error context for debugging data quality issues
 */
export async function checkAppAdsTxt(
  websiteUrl: string,
  config: HunterConfig = {}
): Promise<{ 
  hasAdsTxt: boolean; 
  content?: string; 
  adNetworks?: string[]; 
  error?: string;
  attemptedUrls?: string[];
}> {
  const requestDelay = config.requestDelayMs || 2000;
  const attemptedUrls: string[] = [];
  
  // Generate URL permutations to try
  const urlPermutations = generateUrlPermutations(websiteUrl);
  
  for (const baseUrl of urlPermutations) {
    const adsTxtUrl = `${baseUrl}/app-ads.txt`;
    attemptedUrls.push(adsTxtUrl);
    
    try {
      await delay(requestDelay);
      
      const response = await fetch(adsTxtUrl, {
        headers: { 'User-Agent': DEFAULT_USER_AGENT },
        redirect: 'follow', // Follow up to 20 redirects (Node.js default)
      });
      
      // 404/410 = No ads.txt (valid signal, not an error)
      if (response.status === 404 || response.status === 410) {
        continue; // Try next permutation
      }
      
      // 403 = Blocked by WAF/Cloudflare (interesting signal - active site)
      if (response.status === 403) {
        logger.debug('app-ads.txt blocked by WAF', { url: adsTxtUrl });
        continue;
      }
      
      // Other non-success status
      if (!response.ok) {
        logger.debug('app-ads.txt non-success status', { 
          url: adsTxtUrl, 
          status: response.status 
        });
        continue;
      }
      
      const content = await response.text();
      
      // Validate content looks like app-ads.txt (not an HTML error page)
      if (content.includes('<html') || content.includes('<!DOCTYPE')) {
        logger.debug('app-ads.txt returned HTML instead of text', { url: adsTxtUrl });
        continue;
      }
      
      // Parse ad networks from content
      const adNetworks = extractAdNetworks(content);
      
      // Require at least one valid entry to consider it a real ads.txt
      if (adNetworks.length === 0) {
        logger.debug('app-ads.txt found but empty/invalid', { url: adsTxtUrl });
        continue;
      }
      
      return {
        hasAdsTxt: true,
        content: content.slice(0, 5000), // Limit size
        adNetworks,
        attemptedUrls,
      };
      
    } catch (err) {
      // Network errors (DNS failure, timeout, connection refused)
      // Don't treat as "no ads.txt" - treat as scrape failure
      const errorMessage = (err as Error).message || String(err);
      logger.debug('app-ads.txt fetch failed', { 
        url: adsTxtUrl, 
        error: errorMessage 
      });
      
      // Continue to next permutation
    }
  }
  
  // None of the permutations worked
  return { 
    hasAdsTxt: false,
    attemptedUrls,
    error: `No valid app-ads.txt found at any URL permutation`,
  };
}

/**
 * Generate URL permutations to try for app-ads.txt
 * This handles common redirect patterns
 */
function generateUrlPermutations(websiteUrl: string): string[] {
  const permutations: string[] = [];
  
  // Clean up input
  let cleanUrl = websiteUrl.trim().toLowerCase();
  
  // Remove trailing slashes and paths
  cleanUrl = cleanUrl.replace(/\/+$/, '').split('/')[0];
  
  // Remove protocol if present
  cleanUrl = cleanUrl.replace(/^https?:\/\//, '');
  
  // Remove www if present (we'll add it back as a permutation)
  const hasWww = cleanUrl.startsWith('www.');
  const domainWithoutWww = hasWww ? cleanUrl.substring(4) : cleanUrl;
  const domainWithWww = hasWww ? cleanUrl : `www.${cleanUrl}`;
  
  // Order: https preferred, non-www first (usually canonical)
  permutations.push(`https://${domainWithoutWww}`);
  permutations.push(`https://${domainWithWww}`);
  permutations.push(`http://${domainWithoutWww}`);
  permutations.push(`http://${domainWithWww}`);
  
  return permutations;
}

function extractAdNetworks(adsTxtContent: string): string[] {
  const networks = new Set<string>();
  const lines = adsTxtContent.split('\n');
  
  for (const line of lines) {
    // Format: domain, publisher_id, relationship, [certification_authority_id]
    const trimmed = line.trim();
    if (trimmed.startsWith('#') || !trimmed.includes(',')) continue;
    
    const domain = trimmed.split(',')[0].trim().toLowerCase();
    if (domain) {
      networks.add(domain);
    }
  }
  
  return Array.from(networks);
}

// ============================================================================
// DATABASE OPERATIONS
// ============================================================================

/**
 * Save discovered apps to database
 */
export async function saveDiscoveredApps(
  apps: DiscoveredApp[],
  sourceId?: string
): Promise<{ inserted: number; updated: number }> {
  let inserted = 0;
  let updated = 0;
  
  for (const app of apps) {
    try {
      // Check if app already exists
      const existing = await queryReadOnly<{ id: string }>(`
        SELECT id FROM sa_contacts 
        WHERE app_store = $1 AND app_id = $2
      `, [app.appStore, app.appId]);
      
      if (existing.length > 0) {
        // Update existing
        await queryOperator(`
          UPDATE sa_contacts SET
            company_name = COALESCE($1, company_name),
            website = COALESCE($2, website),
            email = COALESCE($3, email),
            app_genre = COALESCE($4, app_genre),
            app_ads_txt_content = COALESCE($5, app_ads_txt_content),
            last_scraped_at = $6,
            updated_at = CURRENT_TIMESTAMP
          WHERE app_store = $7 AND app_id = $8
        `, [
          app.developerName,
          app.developerWebsite,
          app.developerEmail,
          app.genre,
          app.adsTxtContent,
          app.scrapedAt,
          app.appStore,
          app.appId,
        ]);
        updated++;
      } else {
        // Insert new contact
        await queryOperator(`
          INSERT INTO sa_contacts (
            source_id, company_name, app_store, app_id, app_genre,
            website, email, app_ads_txt_content, last_scraped_at
          ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        `, [
          sourceId || null,
          app.developerName,
          app.appStore,
          app.appId,
          app.genre,
          app.developerWebsite,
          app.developerEmail,
          app.adsTxtContent,
          app.scrapedAt,
        ]);
        inserted++;
      }
    } catch (err) {
      logger.warn('Failed to save app', { 
        appId: app.appId, 
        error: (err as Error).message 
      });
    }
  }
  
  return { inserted, updated };
}

/**
 * Create a scrape job
 */
export async function createScrapeJob(sourceId: string): Promise<string> {
  const result = await queryOperator<{ id: string }>(`
    INSERT INTO sa_scrape_jobs (source_id, status)
    VALUES ($1, 'pending')
    RETURNING id
  `, [sourceId]);
  
  return result[0].id;
}

/**
 * Update scrape job status
 */
export async function updateScrapeJob(
  jobId: string,
  update: Partial<ScrapeJob>
): Promise<void> {
  const setClauses: string[] = [];
  const values: unknown[] = [];
  let paramIndex = 1;
  
  if (update.status !== undefined) {
    setClauses.push(`status = $${paramIndex++}`);
    values.push(update.status);
    
    if (update.status === 'running') {
      setClauses.push(`started_at = CURRENT_TIMESTAMP`);
    } else if (update.status === 'completed' || update.status === 'failed') {
      setClauses.push(`completed_at = CURRENT_TIMESTAMP`);
    }
  }
  
  if (update.appsDiscovered !== undefined) {
    setClauses.push(`apps_discovered = $${paramIndex++}`);
    values.push(update.appsDiscovered);
  }
  
  if (update.appsProcessed !== undefined) {
    setClauses.push(`apps_processed = $${paramIndex++}`);
    values.push(update.appsProcessed);
  }
  
  if (update.errorMessage !== undefined) {
    setClauses.push(`error_message = $${paramIndex++}`);
    values.push(update.errorMessage);
  }
  
  values.push(jobId);
  
  await queryOperator(`
    UPDATE sa_scrape_jobs 
    SET ${setClauses.join(', ')}
    WHERE id = $${paramIndex}
  `, values);
}

/**
 * Get source configuration
 */
export async function getSources(): Promise<Array<{
  id: string;
  sourceName: string;
  sourceType: string;
  sourceConfig: Record<string, unknown>;
  active: boolean;
  last_scraped_at: string | null;
}>> {
  const rows = await queryReadOnly<{
    id: string;
    source_name: string;
    source_type: string;
    search_query: string | null;
    category: string | null;
    region: string | null;
    status: string;
    last_scraped_at: string | null;
  }>(`
    SELECT id, source_name, source_type, search_query, category, region, status, last_scraped_at
    FROM sa_sources
    WHERE status = 'active'
    ORDER BY created_at DESC
  `);
  
  return rows.map(row => ({
    id: row.id,
    sourceName: row.source_name || 'Unnamed Source',
    sourceType: row.source_type,
    sourceConfig: {
      searchTerm: row.search_query,
      category: row.category,
      region: row.region,
    },
    active: row.status === 'active',
    last_scraped_at: row.last_scraped_at,
  }));
}

// ============================================================================
// HUNTER CLASS (Main Orchestrator)
// ============================================================================

export class Hunter {
  private config: HunterConfig;
  
  constructor(config: HunterConfig = {}) {
    this.config = {
      headless: true,
      requestDelayMs: 2000,
      maxPagesPerSource: 5,
      ...config,
    };
  }
  
  /**
   * Run a discovery job for a specific source
   */
  async runJob(sourceId: string): Promise<{ 
    success: boolean; 
    appsFound: number; 
    inserted: number;
    updated: number;
  }> {
    const job = await this.getSource(sourceId);
    if (!job) {
      return { success: false, appsFound: 0, inserted: 0, updated: 0 };
    }
    
    const jobId = await createScrapeJob(sourceId);
    await updateScrapeJob(jobId, { status: 'running' });
    
    try {
      let apps: DiscoveredApp[] = [];
      
      switch (job.sourceType) {
        case 'google_play':
          apps = await scrapeGooglePlayCategory(
            job.sourceConfig.category as string,
            this.config
          );
          break;
          
        case 'itunes':
          apps = await searchITunesApps(
            job.sourceConfig.searchTerm as string,
            job.sourceConfig.limit as number || 50
          );
          break;
          
        default:
          throw new Error(`Unknown source type: ${job.sourceType}`);
      }
      
      // Optionally check app-ads.txt for apps with websites
      for (const app of apps) {
        if (app.developerWebsite) {
          const adsTxt = await checkAppAdsTxt(app.developerWebsite, this.config);
          app.hasAdsTxt = adsTxt.hasAdsTxt;
          app.adsTxtContent = adsTxt.content;
          app.adNetworks = adsTxt.adNetworks;
        }
      }
      
      const { inserted, updated } = await saveDiscoveredApps(apps, sourceId);
      
      await updateScrapeJob(jobId, {
        status: 'completed',
        appsDiscovered: apps.length,
        appsProcessed: inserted + updated,
      });
      
      logger.info('Hunter job completed', {
        sourceId,
        appsFound: apps.length,
        inserted,
        updated,
      });
      
      return {
        success: true,
        appsFound: apps.length,
        inserted,
        updated,
      };
      
    } catch (err) {
      await updateScrapeJob(jobId, {
        status: 'failed',
        errorMessage: (err as Error).message,
      });
      
      logger.error('Hunter job failed', {
        sourceId,
        error: (err as Error).message,
      });
      
      return { success: false, appsFound: 0, inserted: 0, updated: 0 };
    }
  }
  
  /**
   * Get source by ID
   */
  private async getSource(sourceId: string): Promise<{
    sourceType: string;
    sourceConfig: Record<string, unknown>;
  } | null> {
    const sources = await getSources();
    return sources.find(s => s.id === sourceId) || null;
  }
  
  /**
   * Cleanup: Close browser
   */
  async close(): Promise<void> {
    if (browserInstance) {
      await browserInstance.close();
      browserInstance = null;
    }
  }
}

// ============================================================================
// SCHEDULER
// ============================================================================

/**
 * Get sources that need to be scraped
 * Returns full source configuration needed by worker
 */
export async function getSourcesDueForScrape(): Promise<Array<{
  id: string;
  sourceName: string;
  sourceType: string;
  sourceConfig: Record<string, unknown>;
  scrapeIntervalHours: number;
}>> {
  const rows = await queryReadOnly<{
    id: string;
    source_name: string;
    source_type: string;
    search_query: string | null;
    category: string | null;
    region: string | null;
    scrape_interval_hours: number;
  }>(`
    SELECT id, source_name, source_type, search_query, category, region, scrape_interval_hours
    FROM sa_sources
    WHERE status = 'active'
      AND (last_scraped_at IS NULL 
           OR last_scraped_at < CURRENT_TIMESTAMP - (scrape_interval_hours || ' hours')::INTERVAL)
    ORDER BY last_scraped_at NULLS FIRST
    LIMIT 5
  `);
  
  return rows.map(row => ({
    id: row.id,
    sourceName: row.source_name || 'Unnamed Source',
    sourceType: row.source_type,
    sourceConfig: {
      searchTerm: row.search_query,
      category: row.category,
      region: row.region,
    },
    scrapeIntervalHours: row.scrape_interval_hours || 24,
  }));
}
