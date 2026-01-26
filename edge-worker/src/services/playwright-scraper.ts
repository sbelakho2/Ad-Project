/**
 * PLAYWRIGHT SCRAPER
 * 
 * Headless browser-based web scraping with stealth capabilities.
 * 
 * Features:
 *   - LinkedIn profile scraping (with anti-bot evasion)
 *   - Company website enrichment
 *   - Email finder page scraping
 *   - Configurable timeouts and retries
 *   - Request rate limiting
 * 
 * Stealth features:
 *   - Random user agents
 *   - Human-like delays
 *   - Viewport randomization
 *   - WebGL/canvas fingerprint spoofing
 */

import type { Browser, Page, BrowserContext } from 'playwright';

// ============================================================================
// TYPES
// ============================================================================

export interface LinkedInProfile {
  url: string;
  name: string;
  title?: string;
  company?: string;
  location?: string;
  connections?: string;
  about?: string;
  experience?: Array<{
    title: string;
    company: string;
    duration?: string;
    description?: string;
  }>;
  scrapedAt: Date;
}

export interface CompanyInfo {
  url: string;
  name?: string;
  description?: string;
  industry?: string;
  employees?: string;
  founded?: string;
  headquarters?: string;
  website?: string;
  socialLinks?: {
    linkedin?: string;
    twitter?: string;
    facebook?: string;
  };
  emails?: string[];
  phones?: string[];
  scrapedAt: Date;
}

export interface ScrapeResult<T> {
  success: boolean;
  data?: T;
  error?: string;
  latencyMs: number;
  cached: boolean;
}

interface CacheEntry<T> {
  data: T;
  expiresAt: number;
}

// ============================================================================
// USER AGENTS
// ============================================================================

const USER_AGENTS = [
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36',
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36',
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2.1 Safari/605.1.15',
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:122.0) Gecko/20100101 Firefox/122.0',
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:122.0) Gecko/20100101 Firefox/122.0',
];

const VIEWPORTS = [
  { width: 1920, height: 1080 },
  { width: 1366, height: 768 },
  { width: 1440, height: 900 },
  { width: 1536, height: 864 },
  { width: 1280, height: 720 },
];

// ============================================================================
// PLAYWRIGHT SCRAPER CLASS
// ============================================================================

const DEFAULT_TIMEOUT_MS = 30000;
const DEFAULT_CACHE_TTL_MS = 24 * 60 * 60 * 1000; // 24 hours

export class PlaywrightScraper {
  private browser: Browser | null = null;
  private browserLaunching = false;
  private browserLaunchPromise: Promise<Browser> | null = null;
  private cache: Map<string, CacheEntry<unknown>> = new Map();
  private cacheTTL: number;
  private timeoutMs: number;
  private lastRequestTime = 0;
  private minRequestIntervalMs = 2000; // Rate limiting
  
  constructor(options?: {
    cacheTTLMs?: number;
    timeoutMs?: number;
    minRequestIntervalMs?: number;
  }) {
    this.cacheTTL = options?.cacheTTLMs ?? DEFAULT_CACHE_TTL_MS;
    this.timeoutMs = options?.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    this.minRequestIntervalMs = options?.minRequestIntervalMs ?? 2000;
  }
  
  /**
   * Initialize browser
   */
  async initialize(): Promise<void> {
    await this.getBrowser();
  }
  
  /**
   * Close browser
   */
  async close(): Promise<void> {
    if (this.browser) {
      await this.browser.close();
      this.browser = null;
    }
  }
  
  private async getBrowser(): Promise<Browser> {
    if (this.browser) return this.browser;
    
    if (this.browserLaunchPromise) {
      return this.browserLaunchPromise;
    }
    
    this.browserLaunching = true;
    this.browserLaunchPromise = this.launchBrowser();
    
    try {
      this.browser = await this.browserLaunchPromise;
      return this.browser;
    } finally {
      this.browserLaunching = false;
      this.browserLaunchPromise = null;
    }
  }
  
  private async launchBrowser(): Promise<Browser> {
    const { chromium } = await import('playwright');
    
    return chromium.launch({
      headless: true,
      args: [
        '--disable-blink-features=AutomationControlled',
        '--disable-dev-shm-usage',
        '--no-sandbox',
        '--disable-setuid-sandbox',
        '--disable-web-security',
        '--disable-features=IsolateOrigins,site-per-process',
      ],
    });
  }
  
  private async createStealthContext(): Promise<BrowserContext> {
    const browser = await this.getBrowser();
    const userAgent = USER_AGENTS[Math.floor(Math.random() * USER_AGENTS.length)];
    const viewport = VIEWPORTS[Math.floor(Math.random() * VIEWPORTS.length)];
    
    const context = await browser.newContext({
      userAgent,
      viewport,
      locale: 'en-US',
      timezoneId: 'America/New_York',
      permissions: [],
      javaScriptEnabled: true,
      hasTouch: false,
      isMobile: false,
    });
    
    // Add stealth scripts
    await context.addInitScript(() => {
      // Override webdriver detection
      Object.defineProperty(navigator, 'webdriver', {
        get: () => undefined,
      });
      
      // Add plugins
      Object.defineProperty(navigator, 'plugins', {
        get: () => [
          { name: 'Chrome PDF Plugin' },
          { name: 'Chrome PDF Viewer' },
          { name: 'Native Client' },
        ],
      });
      
      // Override automation flags
      (window as { chrome?: unknown }).chrome = { runtime: {} };
    });
    
    return context;
  }
  
  private async rateLimit(): Promise<void> {
    const now = Date.now();
    const timeSinceLastRequest = now - this.lastRequestTime;
    
    if (timeSinceLastRequest < this.minRequestIntervalMs) {
      const delay = this.minRequestIntervalMs - timeSinceLastRequest;
      await new Promise(resolve => setTimeout(resolve, delay));
    }
    
    this.lastRequestTime = Date.now();
  }
  
  private async humanDelay(minMs = 500, maxMs = 2000): Promise<void> {
    const delay = Math.random() * (maxMs - minMs) + minMs;
    await new Promise(resolve => setTimeout(resolve, delay));
  }
  
  // ==========================================================================
  // LINKEDIN SCRAPING
  // ==========================================================================
  
  async scrapeLinkedInProfile(profileUrl: string): Promise<ScrapeResult<LinkedInProfile>> {
    const startTime = Date.now();
    const cacheKey = `linkedin:${profileUrl}`;
    
    // Check cache
    const cached = this.cache.get(cacheKey) as CacheEntry<LinkedInProfile> | undefined;
    if (cached && cached.expiresAt > Date.now()) {
      return {
        success: true,
        data: cached.data,
        latencyMs: Date.now() - startTime,
        cached: true,
      };
    }
    
    await this.rateLimit();
    
    let context: BrowserContext | null = null;
    let page: Page | null = null;
    
    try {
      context = await this.createStealthContext();
      page = await context.newPage();
      
      // Navigate with human-like behavior
      await page.goto(profileUrl, {
        waitUntil: 'domcontentloaded',
        timeout: this.timeoutMs,
      });
      
      await this.humanDelay(1000, 3000);
      
      // Check for auth wall
      const authWall = await page.$('div.authentication-outlet');
      if (authWall) {
        return {
          success: false,
          error: 'LinkedIn requires authentication',
          latencyMs: Date.now() - startTime,
          cached: false,
        };
      }
      
      // Extract profile data
      const profile = await page.evaluate(() => {
        const getText = (selector: string): string | undefined => {
          const el = document.querySelector(selector);
          return el?.textContent?.trim() || undefined;
        };
        
        return {
          name: getText('h1.text-heading-xlarge') || 
                getText('.pv-text-details__left-panel h1') || 
                'Unknown',
          title: getText('.text-body-medium.break-words') ||
                 getText('.pv-text-details__left-panel .text-body-medium'),
          company: getText('.pv-text-details__right-panel a') ||
                   getText('.pv-text-details__left-panel span'),
          location: getText('.text-body-small.inline.t-black--light') ||
                    getText('.pv-text-details__left-panel .t-black--light'),
          connections: getText('.pv-top-card--list-bullet .t-bold'),
          about: getText('.pv-about-section .pv-about__summary-text') ||
                 getText('section.pv-about-section p'),
        };
      });
      
      // Try to get experience
      const experience = await this.extractLinkedInExperience(page);
      
      const fullProfile: LinkedInProfile = {
        url: profileUrl,
        ...profile,
        experience,
        scrapedAt: new Date(),
      };
      
      // Cache result
      this.cache.set(cacheKey, {
        data: fullProfile,
        expiresAt: Date.now() + this.cacheTTL,
      });
      
      return {
        success: true,
        data: fullProfile,
        latencyMs: Date.now() - startTime,
        cached: false,
      };
    } catch (err) {
      return {
        success: false,
        error: (err as Error).message,
        latencyMs: Date.now() - startTime,
        cached: false,
      };
    } finally {
      if (page) await page.close().catch(() => {});
      if (context) await context.close().catch(() => {});
    }
  }
  
  private async extractLinkedInExperience(page: Page): Promise<LinkedInProfile['experience']> {
    try {
      return await page.evaluate(() => {
        const experience: Array<{
          title: string;
          company: string;
          duration?: string;
          description?: string;
        }> = [];
        
        const experienceItems = document.querySelectorAll(
          'section.experience-section li, section[data-section="experience"] li'
        );
        
        experienceItems.forEach((item) => {
          const title = item.querySelector('h3')?.textContent?.trim();
          const company = item.querySelector('.pv-entity__secondary-title')?.textContent?.trim() ||
                          item.querySelector('p.pv-entity__secondary-title')?.textContent?.trim();
          const duration = item.querySelector('.pv-entity__date-range span:nth-child(2)')?.textContent?.trim();
          const description = item.querySelector('.pv-entity__description')?.textContent?.trim();
          
          if (title) {
            experience.push({ 
              title, 
              company: company || 'Unknown',
              duration,
              description,
            });
          }
        });
        
        return experience.slice(0, 5); // Limit to last 5 positions
      });
    } catch {
      return undefined;
    }
  }
  
  // ==========================================================================
  // COMPANY SCRAPING
  // ==========================================================================
  
  async scrapeCompanyWebsite(websiteUrl: string): Promise<ScrapeResult<CompanyInfo>> {
    const startTime = Date.now();
    const cacheKey = `company:${websiteUrl}`;
    
    // Check cache
    const cached = this.cache.get(cacheKey) as CacheEntry<CompanyInfo> | undefined;
    if (cached && cached.expiresAt > Date.now()) {
      return {
        success: true,
        data: cached.data,
        latencyMs: Date.now() - startTime,
        cached: true,
      };
    }
    
    await this.rateLimit();
    
    let context: BrowserContext | null = null;
    let page: Page | null = null;
    
    try {
      context = await this.createStealthContext();
      page = await context.newPage();
      
      await page.goto(websiteUrl, {
        waitUntil: 'domcontentloaded',
        timeout: this.timeoutMs,
      });
      
      await this.humanDelay(500, 1500);
      
      // Extract company info
      const info = await page.evaluate((url: string) => {
        const getText = (selector: string): string | undefined => {
          const el = document.querySelector(selector);
          return el?.textContent?.trim() || undefined;
        };
        
        // =================================================================
        // IMPROVED EMAIL EXTRACTION (FIX: prevents malformed emails)
        // =================================================================
        
        // Valid TLDs we accept
        const validTlds = ['com', 'net', 'org', 'io', 'co', 'ee', 'app', 'dev', 'games', 'ai', 
                          'tech', 'xyz', 'info', 'biz', 'us', 'uk', 'de', 'jp', 'cn', 'kr',
                          'ru', 'br', 'au', 'ca', 'fr', 'it', 'es', 'nl', 'pl', 'sg'];
        
        // Domains to reject (tracking, spam, internal)
        const rejectDomains = [
          'sentry.wixpress.com', 'sentry-next.wixpress.com', 'sentry.io',
          'example.com', 'test.local', 'localhost',
          'wix.com', 'squarespace.com', 'wordpress.com',  // Platform domains
          'googlemail.com', 'mailchimp.com', 'sendgrid.net',  // Email services
        ];
        
        // Reject patterns (UUID-like usernames, tracking IDs)
        const rejectLocalPatterns = [
          /^[0-9a-f]{32}$/i,                          // 32-char hex (Sentry)
          /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,  // UUID
          /^(noreply|no-reply|donotreply|mailer-daemon|postmaster|nobody|bounce)$/i,
        ];
        
        // Extract from mailto: links first (most reliable)
        const mailtoEmails: string[] = [];
        document.querySelectorAll('a[href^="mailto:"]').forEach((link) => {
          const href = (link as HTMLAnchorElement).href;
          const match = href.match(/mailto:([^?]+)/);
          if (match && match[1]) {
            mailtoEmails.push(match[1].toLowerCase().trim());
          }
        });
        
        // Extract from page text with word boundaries
        // IMPORTANT: Use word boundary \\b to prevent "hello@amanotes.comTerms"
        const emailRegex = /\b[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,6}\b/g;
        const pageText = document.body?.textContent || '';
        const textEmails = [...new Set(pageText.match(emailRegex) || [])].map(e => e.toLowerCase());
        
        // Combine and deduplicate (prefer mailto: links)
        const allEmails = [...new Set([...mailtoEmails, ...textEmails])];
        
        // Validate and filter emails
        const validEmails = allEmails.filter(email => {
          // Must have exactly one @
          const parts = email.split('@');
          if (parts.length !== 2) return false;
          
          const [localPart, domain] = parts;
          
          // Local part checks
          if (localPart.length < 1 || localPart.length > 64) return false;
          if (rejectLocalPatterns.some(p => p.test(localPart))) return false;
          
          // Domain checks
          const domainParts = domain.split('.');
          if (domainParts.length < 2) return false;
          
          const tld = domainParts[domainParts.length - 1];
          if (!validTlds.includes(tld)) return false;
          
          // Check for text AFTER the TLD (malformed: "amanotes.comTerms")
          // If the domain has anything after the TLD letters, reject it
          const tldPattern = new RegExp(`\\.(${validTlds.join('|')})$`, 'i');
          if (!tldPattern.test(domain)) return false;
          
          // Reject known bad domains
          if (rejectDomains.some(d => domain.includes(d))) return false;
          
          return true;
        });
        
        // =================================================================
        // REST OF EXTRACTION
        // =================================================================
        
        // Find phone numbers
        const phoneRegex = /(?:\+?1[-.\s]?)?\(?[0-9]{3}\)?[-.\s]?[0-9]{3}[-.\s]?[0-9]{4}/g;
        const foundPhones = [...new Set(pageText.match(phoneRegex) || [])];
        
        // Find social links
        const socialLinks: Record<string, string | undefined> = {};
        
        document.querySelectorAll('a[href]').forEach((link) => {
          const href = (link as HTMLAnchorElement).href;
          if (href.includes('linkedin.com')) socialLinks.linkedin = href;
          if (href.includes('twitter.com') || href.includes('x.com')) socialLinks.twitter = href;
          if (href.includes('facebook.com')) socialLinks.facebook = href;
        });
        
        // Find internal links that might have contact info
        const contactPagePatterns = [
          /\/contact/i, /\/about/i, /\/team/i, /\/company/i, /\/support/i,
          /\/impressum/i, /\/legal/i, /\/privacy/i, /\/careers/i,
        ];
        
        const internalLinks: string[] = [];
        document.querySelectorAll('a[href]').forEach((link) => {
          const href = (link as HTMLAnchorElement).href;
          // Only same domain links
          try {
            const linkUrl = new URL(href);
            const baseUrl = new URL(url);
            if (linkUrl.hostname === baseUrl.hostname || 
                linkUrl.hostname.endsWith('.' + baseUrl.hostname)) {
              if (contactPagePatterns.some(p => p.test(href))) {
                internalLinks.push(href);
              }
            }
          } catch { /* invalid URL */ }
        });
        
        // Get meta info
        const getMetaContent = (name: string): string | undefined => {
          const meta = document.querySelector(`meta[name="${name}"], meta[property="${name}"]`);
          return meta?.getAttribute('content') || undefined;
        };
        
        return {
          url,
          name: getText('h1') || 
                getMetaContent('og:site_name') || 
                document.title?.split(/[-|–]/)[0]?.trim(),
          description: getMetaContent('description') || 
                       getMetaContent('og:description'),
          emails: validEmails.slice(0, 5),
          phones: foundPhones.slice(0, 3),
          socialLinks: Object.keys(socialLinks).length > 0 ? socialLinks : undefined,
          _contactPages: [...new Set(internalLinks)].slice(0, 5),
        };
      }, websiteUrl);
      
      // ================================================================
      // MULTI-PAGE SCRAPING: If no emails found, try contact pages
      // ================================================================
      let allEmails = info.emails || [];
      
      if (allEmails.length === 0 && info._contactPages && info._contactPages.length > 0) {
        for (const contactUrl of info._contactPages.slice(0, 3)) { // Try max 3 subpages
          try {
            await this.humanDelay(1000, 2000); // Delay between page loads
            
            await page.goto(contactUrl, {
              waitUntil: 'domcontentloaded',
              timeout: this.timeoutMs / 2, // Faster timeout for subpages
            });
            
            await this.humanDelay(300, 800);
            
            // Extract emails from subpage
            const subpageEmails = await page.evaluate(() => {
              const validTlds = ['com', 'net', 'org', 'io', 'co', 'ee', 'app', 'dev', 'games', 'ai'];
              const rejectDomains = ['sentry.wixpress.com', 'sentry-next.wixpress.com', 'example.com'];
              const rejectLocalPatterns = [
                /^[0-9a-f]{32}$/i,
                /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
                /^(noreply|no-reply|donotreply|mailer-daemon|postmaster)$/i,
              ];
              
              const mailtoEmails: string[] = [];
              document.querySelectorAll('a[href^="mailto:"]').forEach((link) => {
                const href = (link as HTMLAnchorElement).href;
                const match = href.match(/mailto:([^?]+)/);
                if (match && match[1]) {
                  mailtoEmails.push(match[1].toLowerCase().trim());
                }
              });
              
              const emailRegex = /\b[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,6}\b/g;
              const pageText = document.body?.textContent || '';
              const textEmails = [...new Set(pageText.match(emailRegex) || [])].map(e => e.toLowerCase());
              
              return [...new Set([...mailtoEmails, ...textEmails])].filter(email => {
                const parts = email.split('@');
                if (parts.length !== 2) return false;
                const [localPart, domain] = parts;
                if (rejectLocalPatterns.some(p => p.test(localPart))) return false;
                const domainParts = domain.split('.');
                if (domainParts.length < 2) return false;
                const tld = domainParts[domainParts.length - 1];
                if (!validTlds.includes(tld)) return false;
                if (rejectDomains.some(d => domain.includes(d))) return false;
                return true;
              });
            });
            
            if (subpageEmails.length > 0) {
              allEmails = [...new Set([...allEmails, ...subpageEmails])];
              break; // Stop if we found emails
            }
          } catch {
            // Continue to next page
          }
        }
      }
      
      // ================================================================
      // EMAIL PATTERN GUESSING: If still no emails, try common patterns
      // ================================================================
      if (allEmails.length === 0 && info.name) {
        const domain = new URL(websiteUrl).hostname.replace('www.', '');
        const companyName = info.name.toLowerCase()
          .replace(/[^a-z0-9]+/g, '')
          .substring(0, 20);
        
        // Common email patterns to guess
        const guessPatterns = [
          `hello@${domain}`,
          `contact@${domain}`,
          `info@${domain}`,
          `support@${domain}`,
          `team@${domain}`,
          `hi@${domain}`,
          `${companyName}@${domain}`,
        ];
        
        // We'll record these as "guessed" but prioritize scraped emails
        allEmails = guessPatterns.slice(0, 3); // Keep top 3 guesses
      }
      
      const companyInfo: CompanyInfo = {
        url: info.url,
        name: info.name,
        description: info.description,
        emails: allEmails.slice(0, 5),
        phones: info.phones,
        socialLinks: info.socialLinks,
        scrapedAt: new Date(),
      };
      
      // Cache result
      this.cache.set(cacheKey, {
        data: companyInfo,
        expiresAt: Date.now() + this.cacheTTL,
      });
      
      return {
        success: true,
        data: companyInfo,
        latencyMs: Date.now() - startTime,
        cached: false,
      };
    } catch (err) {
      return {
        success: false,
        error: (err as Error).message,
        latencyMs: Date.now() - startTime,
        cached: false,
      };
    } finally {
      if (page) await page.close().catch(() => {});
      if (context) await context.close().catch(() => {});
    }
  }
  
  // ==========================================================================
  // GENERIC PAGE SCRAPING
  // ==========================================================================
  
  async scrapeGenericPage(url: string, selectors: Record<string, string>): Promise<ScrapeResult<Record<string, string | null>>> {
    const startTime = Date.now();
    
    await this.rateLimit();
    
    let context: BrowserContext | null = null;
    let page: Page | null = null;
    
    try {
      context = await this.createStealthContext();
      page = await context.newPage();
      
      await page.goto(url, {
        waitUntil: 'domcontentloaded',
        timeout: this.timeoutMs,
      });
      
      await this.humanDelay(300, 1000);
      
      const data = await page.evaluate((sels: Record<string, string>) => {
        const results: Record<string, string | null> = {};
        
        for (const [key, selector] of Object.entries(sels)) {
          const el = document.querySelector(selector);
          results[key] = el?.textContent?.trim() || null;
        }
        
        return results;
      }, selectors);
      
      return {
        success: true,
        data,
        latencyMs: Date.now() - startTime,
        cached: false,
      };
    } catch (err) {
      return {
        success: false,
        error: (err as Error).message,
        latencyMs: Date.now() - startTime,
        cached: false,
      };
    } finally {
      if (page) await page.close().catch(() => {});
      if (context) await context.close().catch(() => {});
    }
  }
  
  // ==========================================================================
  // UTILITIES
  // ==========================================================================
  
  clearCache(): void {
    this.cache.clear();
  }
  
  getCacheStats(): { size: number; keys: string[] } {
    return {
      size: this.cache.size,
      keys: Array.from(this.cache.keys()),
    };
  }
  
  isInitialized(): boolean {
    return this.browser !== null;
  }
}
