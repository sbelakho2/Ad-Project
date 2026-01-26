/**
 * HUNTER STANDALONE MODULE
 * 
 * This version of Hunter has NO database dependencies.
 * It scrapes data and returns it as pure JSON for the worker.
 * The worker then POSTs this data to the Control Plane API.
 * 
 * Used by: Worker (Test Server)
 * NOT used by: Control Plane (App Server) - uses hunter.ts instead
 */

// Dynamic import for Playwright (optional dependency)
let playwrightModule: typeof import('playwright') | null = null;
async function getPlaywright(): Promise<typeof import('playwright') | null> {
  if (playwrightModule === undefined) {
    try {
      playwrightModule = await import('playwright');
    } catch {
      console.warn('[Hunter] Playwright not available - Google Play scraping disabled');
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
  lastUpdated?: string;
  scrapedAt: string;
}

export interface SourceConfig {
  id: string;
  sourceType: 'google_play' | 'itunes' | 'product_hunt';
  sourceConfig: {
    searchTerm?: string;
    category?: string;
    region?: string;
    limit?: number;
  };
}

export interface HunterConfig {
  headless?: boolean;
  requestDelayMs?: number;
  maxPagesPerSource?: number;
  userAgent?: string;
}

export interface ScrapeResult {
  success: boolean;
  sourceId: string;
  apps: DiscoveredApp[];
  appsFound: number;
  error?: string;
}

// ============================================================================
// BROWSER UTILITIES
// ============================================================================

const DEFAULT_USER_AGENT = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';

let browserInstance: Browser | null = null;

async function getBrowser(config: HunterConfig): Promise<Browser | null> {
  const playwright = await getPlaywright();
  if (!playwright) return null;
  
  if (!browserInstance || !browserInstance.isConnected()) {
    browserInstance = await playwright.chromium.launch({
      headless: config.headless ?? true,
    });
  }
  return browserInstance;
}

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

function delay(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

// ============================================================================
// GOOGLE PLAY SCRAPER
// ============================================================================

async function scrapeGooglePlayCategory(
  category: string,
  config: HunterConfig = {}
): Promise<DiscoveredApp[]> {
  const apps: DiscoveredApp[] = [];
  const requestDelay = config.requestDelayMs || 2000;
  const maxPages = config.maxPagesPerSource || 5;
  
  const browser = await getBrowser(config);
  if (!browser) {
    console.warn('[Hunter] Playwright not available, skipping Google Play scrape');
    return apps;
  }
  
  const context = await createStealthContext(browser, config);
  if (!context) return apps;
  
  const page = await context.newPage();
  
  try {
    console.log(`[Hunter] Starting Google Play scrape for category: ${category}`);
    
    const categoryUrl = `https://play.google.com/store/apps/category/${category}/collection/topselling_free`;
    await page.goto(categoryUrl, { waitUntil: 'networkidle' });
    await delay(requestDelay);
    
    for (let i = 0; i < maxPages; i++) {
      await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
      await delay(1000);
    }
    
    const appLinks = await page.$$eval('a[href*="/store/apps/details"]', links =>
      [...new Set(links.map(a => a.getAttribute('href')).filter(Boolean))]
    );
    
    console.log(`[Hunter] Found ${appLinks.length} app links`);
    
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
        console.warn(`[Hunter] Failed to scrape app ${link}: ${(err as Error).message}`);
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
    const appName = await page.$eval('h1[itemprop="name"]', el => el.textContent?.trim() || '').catch(() => '');
    const developerName = await page.$eval('a[href*="/store/apps/dev"]', el => el.textContent?.trim() || '').catch(() => '');
    const description = await page.$eval('[data-g-id="description"]', el => el.textContent?.trim().slice(0, 500) || '').catch(() => '');
    const genre = await page.$eval('[itemprop="genre"]', el => el.textContent?.trim() || '').catch(() => '');
    const installs = await page.$eval('.ClM7O', el => el.textContent?.trim() || '').catch(() => '');
    const ratingText = await page.$eval('[itemprop="starRating"] [aria-label]', el => el.getAttribute('aria-label') || '').catch(() => '');
    const rating = parseFloat(ratingText.match(/[\d.]+/)?.[0] || '0');
    
    const developerEmail = await extractDeveloperEmail(page);
    
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
      scrapedAt: new Date().toISOString(),
    };
  } catch (err) {
    console.warn(`[Hunter] Error parsing app page ${appId}: ${(err as Error).message}`);
    return null;
  }
}

async function extractDeveloperEmail(page: Page): Promise<string | null> {
  try {
    const expandButton = await page.$('button[aria-label*="Developer"]');
    if (expandButton) {
      await expandButton.click();
      await delay(500);
    }
    
    const pageContent = await page.content();
    const emailMatch = pageContent.match(/[\w.-]+@[\w.-]+\.[a-z]{2,}/i);
    
    if (emailMatch) {
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
// ITUNES API (No scraping needed)
// ============================================================================

async function searchITunesApps(
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
      lastUpdated: app.currentVersionReleaseDate,
      scrapedAt: new Date().toISOString(),
    }));
  } catch (err) {
    console.error(`[Hunter] iTunes API error: ${(err as Error).message}`);
    return [];
  }
}

// ============================================================================
// APP-ADS.TXT CHECKER
// ============================================================================

async function checkAppAdsTxt(
  websiteUrl: string,
  config: HunterConfig = {}
): Promise<{ hasAdsTxt: boolean; content?: string; adNetworks?: string[] }> {
  const requestDelay = config.requestDelayMs || 2000;
  
  try {
    let baseUrl = websiteUrl;
    if (!baseUrl.startsWith('http')) {
      baseUrl = `https://${baseUrl}`;
    }
    const url = new URL(baseUrl);
    const adsTxtUrl = `${url.protocol}//${url.host}/app-ads.txt`;
    
    await delay(requestDelay);
    
    const response = await fetch(adsTxtUrl, {
      headers: { 'User-Agent': DEFAULT_USER_AGENT },
    });
    
    if (!response.ok) {
      return { hasAdsTxt: false };
    }
    
    const content = await response.text();
    const adNetworks = extractAdNetworks(content);
    
    return {
      hasAdsTxt: true,
      content: content.slice(0, 5000),
      adNetworks,
    };
  } catch {
    return { hasAdsTxt: false };
  }
}

function extractAdNetworks(adsTxtContent: string): string[] {
  const networks = new Set<string>();
  const lines = adsTxtContent.split('\n');
  
  for (const line of lines) {
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
// STANDALONE HUNTER CLASS
// ============================================================================

export class HunterStandalone {
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
   * Scrape a source and return discovered apps (NO database operations)
   */
  async scrapeSource(source: SourceConfig): Promise<ScrapeResult> {
    console.log(`[Hunter] Starting scrape for source ${source.id} (${source.sourceType})`);
    
    try {
      let apps: DiscoveredApp[] = [];
      
      switch (source.sourceType) {
        case 'google_play':
          apps = await scrapeGooglePlayCategory(
            source.sourceConfig.category || 'GAME',
            this.config
          );
          break;
          
        case 'itunes':
          apps = await searchITunesApps(
            source.sourceConfig.searchTerm || 'game',
            source.sourceConfig.limit || 50
          );
          break;
          
        default:
          throw new Error(`Unknown source type: ${source.sourceType}`);
      }
      
      // Optionally check app-ads.txt for apps with websites
      for (const app of apps) {
        if (app.developerWebsite) {
          try {
            const adsTxt = await checkAppAdsTxt(app.developerWebsite, this.config);
            app.hasAdsTxt = adsTxt.hasAdsTxt;
            app.adsTxtContent = adsTxt.content;
            app.adNetworks = adsTxt.adNetworks;
          } catch {
            // Ignore ads.txt errors
          }
        }
      }
      
      console.log(`[Hunter] Scraped ${apps.length} apps from source ${source.id}`);
      
      return {
        success: true,
        sourceId: source.id,
        apps,
        appsFound: apps.length,
      };
      
    } catch (err) {
      console.error(`[Hunter] Scrape failed for source ${source.id}: ${(err as Error).message}`);
      return {
        success: false,
        sourceId: source.id,
        apps: [],
        appsFound: 0,
        error: (err as Error).message,
      };
    }
  }
  
  /**
   * Close browser resources
   */
  async close(): Promise<void> {
    if (browserInstance) {
      await browserInstance.close();
      browserInstance = null;
    }
  }
}
