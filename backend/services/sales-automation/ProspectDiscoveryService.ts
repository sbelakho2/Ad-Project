// services/sales-automation/ProspectDiscoveryService.ts
// Discovers potential customers from public data sources (app stores, GitHub, etc.)
// No paid API calls - uses only free public APIs and scraping

import { Pool } from 'pg';
import https from 'https';
import http from 'http';

interface AppStoreResult {
  appId: string;
  appName: string;
  developerName: string;
  developerId?: string;
  appUrl: string;
  category: string;
  downloadsEstimate?: string;
  rating?: number;
  reviewCount?: number;
  hasAds: boolean;
  hasIap: boolean;
  lastUpdated?: Date;
}

interface DiscoveryConfig {
  maxResultsPerQuery: number;
  searchQueries: string[];
  categories: string[];
  minRating?: number;
  minDownloads?: string;
}

const DEFAULT_CONFIG: DiscoveryConfig = {
  maxResultsPerQuery: 50,
  searchQueries: [
    'ad supported',
    'free with ads',
    'rewarded video',
    'interstitial ads',
    'banner ads',
    'admob',
    'facebook ads',
    'unity ads',
    'applovin',
    'ironsource',
  ],
  categories: [
    'GAME_ARCADE',
    'GAME_PUZZLE',
    'GAME_CASUAL',
    'GAME_ACTION',
    'ENTERTAINMENT',
    'LIFESTYLE',
    'TOOLS',
    'EDUCATION',
    'NEWS_AND_MAGAZINES',
    'MUSIC_AND_AUDIO',
  ],
  minRating: 3.5,
  minDownloads: '10K-50K',
};

export class ProspectDiscoveryService {
  private pool: Pool;
  private config: DiscoveryConfig;

  constructor(databaseUrl: string, config?: Partial<DiscoveryConfig>) {
    this.pool = new Pool({ connectionString: databaseUrl });
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /**
   * Run full discovery cycle
   */
  async runDiscoveryCycle(): Promise<{ discovered: number; errors: number }> {
    console.log('[Discovery] Starting prospect discovery cycle...');
    
    let totalDiscovered = 0;
    let totalErrors = 0;

    try {
      // 1. Google Play Store discovery
      const playStoreResults = await this.discoverFromGooglePlay();
      totalDiscovered += playStoreResults.discovered;
      totalErrors += playStoreResults.errors;

      // 2. Apple App Store discovery (RSS feeds)
      const appStoreResults = await this.discoverFromAppStore();
      totalDiscovered += appStoreResults.discovered;
      totalErrors += appStoreResults.errors;

      // 3. GitHub SDK usage discovery
      const githubResults = await this.discoverFromGitHub();
      totalDiscovered += githubResults.discovered;
      totalErrors += githubResults.errors;

      // 4. Find similar apps to high-value prospects
      const similarResults = await this.discoverSimilarApps();
      totalDiscovered += similarResults.discovered;
      totalErrors += similarResults.errors;

      console.log(`[Discovery] Cycle complete: ${totalDiscovered} new prospects, ${totalErrors} errors`);

      // Update daily metrics
      await this.updateDailyMetrics(totalDiscovered);

      return { discovered: totalDiscovered, errors: totalErrors };
    } catch (error) {
      console.error('[Discovery] Error in discovery cycle:', error);
      throw error;
    }
  }

  /**
   * Discover apps from Google Play Store using public search
   * Uses the unofficial but stable Google Play web search endpoint
   */
  private async discoverFromGooglePlay(): Promise<{ discovered: number; errors: number }> {
    console.log('[Discovery] Searching Google Play Store...');
    
    let discovered = 0;
    let errors = 0;

    // Search by ad-related queries
    for (const query of this.config.searchQueries) {
      try {
        const apps = await this.searchGooglePlay(query);
        
        for (const app of apps) {
          const saved = await this.saveProspect(app, 'google_play', 'google_play_search');
          if (saved) discovered++;
        }
        
        // Rate limiting - 2 second delay between queries
        await this.delay(2000);
      } catch (error) {
        console.error(`[Discovery] Error searching Play Store for "${query}":`, error);
        errors++;
      }
    }

    // Search by category
    for (const category of this.config.categories) {
      try {
        const apps = await this.searchGooglePlayByCategory(category);
        
        for (const app of apps) {
          const saved = await this.saveProspect(app, 'google_play', 'google_play_category');
          if (saved) discovered++;
        }
        
        await this.delay(2000);
      } catch (error) {
        console.error(`[Discovery] Error searching Play Store category "${category}":`, error);
        errors++;
      }
    }

    console.log(`[Discovery] Google Play: ${discovered} new prospects`);
    return { discovered, errors };
  }

  /**
   * Search Google Play using public web endpoints
   * This mimics the browser search to avoid needing API keys
   */
  private async searchGooglePlay(query: string): Promise<AppStoreResult[]> {
    const results: AppStoreResult[] = [];
    
    // Use the suggestion/search API that doesn't require auth
    const searchUrl = `https://play.google.com/store/search?q=${encodeURIComponent(query)}&c=apps&hl=en&gl=us`;
    
    try {
      const html = await this.fetchUrl(searchUrl);
      
      // Parse app IDs from the HTML response
      // Pattern: /store/apps/details?id=com.example.app
      const appIdRegex = /\/store\/apps\/details\?id=([a-zA-Z0-9._]+)/g;
      const matches = new Set<string>();
      let match;
      
      while ((match = appIdRegex.exec(html)) !== null) {
        if (matches.size >= this.config.maxResultsPerQuery) break;
        matches.add(match[1]);
      }

      // Fetch details for each app
      for (const appId of matches) {
        try {
          const appDetails = await this.getGooglePlayAppDetails(appId);
          if (appDetails && this.passesFilters(appDetails)) {
            results.push(appDetails);
          }
          await this.delay(500); // Rate limit
        } catch (e) {
          // Skip apps that fail to fetch
        }
      }
    } catch (error) {
      console.error(`[Discovery] Failed to search Google Play:`, error);
    }

    return results;
  }

  /**
   * Get Google Play app details using the public lookup endpoint
   */
  private async getGooglePlayAppDetails(appId: string): Promise<AppStoreResult | null> {
    const detailsUrl = `https://play.google.com/store/apps/details?id=${appId}&hl=en&gl=us`;
    
    try {
      const html = await this.fetchUrl(detailsUrl);
      
      // Parse app name - look for itemprop="name"
      const nameMatch = html.match(/itemprop="name"[^>]*>([^<]+)</);
      const appName = nameMatch ? this.decodeHtml(nameMatch[1]) : appId;
      
      // Parse developer name
      const developerMatch = html.match(/href="\/store\/apps\/developer[^"]*"[^>]*>([^<]+)</);
      const developerName = developerMatch ? this.decodeHtml(developerMatch[1]) : 'Unknown';
      
      // Parse category
      const categoryMatch = html.match(/href="\/store\/apps\/category\/([A-Z_]+)"/);
      const category = categoryMatch ? categoryMatch[1] : 'UNKNOWN';
      
      // Parse rating
      const ratingMatch = html.match(/(\d+\.\d+)★|aria-label="[^"]*(\d+\.\d+)[^"]*stars/);
      const rating = ratingMatch ? parseFloat(ratingMatch[1] || ratingMatch[2]) : undefined;
      
      // Parse downloads - look for common patterns
      const downloadsMatch = html.match(/([\d,]+[KMB]?\+?)\s*(?:downloads|installs)/i);
      const downloadsText = downloadsMatch ? downloadsMatch[1] : undefined;
      const downloadsEstimate = this.normalizeDownloads(downloadsText);
      
      // Check for ads indicator
      const hasAds = /contains\s*ads|ad[\s-]supported/i.test(html);
      
      // Check for in-app purchases
      const hasIap = /in-app\s*purchases|offers\s*in-app/i.test(html);
      
      // Parse last updated
      const updateMatch = html.match(/Updated on\s*(\w+\s+\d+,\s+\d{4})/);
      const lastUpdated = updateMatch ? new Date(updateMatch[1]) : undefined;

      return {
        appId,
        appName,
        developerName,
        appUrl: detailsUrl,
        category,
        downloadsEstimate,
        rating,
        reviewCount: undefined, // Would need additional parsing
        hasAds,
        hasIap,
        lastUpdated,
      };
    } catch (error) {
      return null;
    }
  }

  /**
   * Search Google Play by category
   */
  private async searchGooglePlayByCategory(category: string): Promise<AppStoreResult[]> {
    const results: AppStoreResult[] = [];
    
    // Top free apps in category (most likely to have ads)
    const categoryUrl = `https://play.google.com/store/apps/category/${category}/collection/topselling_free?hl=en&gl=us`;
    
    try {
      const html = await this.fetchUrl(categoryUrl);
      
      const appIdRegex = /\/store\/apps\/details\?id=([a-zA-Z0-9._]+)/g;
      const matches = new Set<string>();
      let match;
      
      while ((match = appIdRegex.exec(html)) !== null) {
        if (matches.size >= this.config.maxResultsPerQuery) break;
        matches.add(match[1]);
      }

      for (const appId of matches) {
        try {
          const appDetails = await this.getGooglePlayAppDetails(appId);
          if (appDetails && this.passesFilters(appDetails)) {
            results.push(appDetails);
          }
          await this.delay(500);
        } catch (e) {
          // Skip
        }
      }
    } catch (error) {
      console.error(`[Discovery] Failed to search category ${category}:`, error);
    }

    return results;
  }

  /**
   * Discover apps from Apple App Store using RSS feeds (free, no auth)
   */
  private async discoverFromAppStore(): Promise<{ discovered: number; errors: number }> {
    console.log('[Discovery] Searching Apple App Store...');
    
    let discovered = 0;
    let errors = 0;

    // Apple provides free RSS feeds for top apps
    const feedUrls = [
      'https://rss.applemarketingtools.com/api/v2/us/apps/top-free/50/apps.json',
      'https://rss.applemarketingtools.com/api/v2/us/apps/top-grossing/50/apps.json',
    ];

    for (const feedUrl of feedUrls) {
      try {
        const response = await this.fetchUrl(feedUrl);
        const data = JSON.parse(response);
        
        if (data.feed && data.feed.results) {
          for (const app of data.feed.results) {
            const appResult: AppStoreResult = {
              appId: app.id,
              appName: app.name,
              developerName: app.artistName,
              appUrl: app.url,
              category: app.genres?.[0]?.name || 'Unknown',
              hasAds: true, // Assume top free apps have ads
              hasIap: true,
            };
            
            if (this.passesFilters(appResult)) {
              const saved = await this.saveProspect(appResult, 'app_store', 'app_store_rss');
              if (saved) discovered++;
            }
          }
        }
        
        await this.delay(1000);
      } catch (error) {
        console.error(`[Discovery] Error fetching App Store RSS:`, error);
        errors++;
      }
    }

    console.log(`[Discovery] App Store: ${discovered} new prospects`);
    return { discovered, errors };
  }

  /**
   * Discover apps from GitHub by searching for SDK usage
   */
  private async discoverFromGitHub(): Promise<{ discovered: number; errors: number }> {
    console.log('[Discovery] Searching GitHub for SDK usage...');
    
    let discovered = 0;
    let errors = 0;

    // Search for apps using ad SDKs in their code
    const sdkSearchQueries = [
      'com.google.android.gms.ads filename:build.gradle',
      'com.facebook.ads filename:build.gradle',
      'com.unity3d.ads filename:build.gradle',
      'com.applovin filename:build.gradle',
      'MobileAds filename:Podfile',
      'GADMobileAds filename:swift',
    ];

    for (const query of sdkSearchQueries) {
      try {
        // GitHub search API is rate limited but allows unauthenticated requests
        const searchUrl = `https://api.github.com/search/code?q=${encodeURIComponent(query)}&per_page=30`;
        
        const response = await this.fetchUrl(searchUrl, {
          'User-Agent': 'ApexMediation-ProspectDiscovery/1.0',
          'Accept': 'application/vnd.github.v3+json',
        });
        
        const data = JSON.parse(response);
        
        if (data.items) {
          for (const item of data.items) {
            // Extract app info from repository
            const repoOwner = item.repository?.owner?.login;
            const repoName = item.repository?.name;
            
            if (repoOwner && repoName) {
              // Try to extract app package name from the file
              const packageName = await this.extractPackageFromGitHub(repoOwner, repoName);
              
              if (packageName) {
                const appResult: AppStoreResult = {
                  appId: packageName,
                  appName: repoName,
                  developerName: repoOwner,
                  appUrl: `https://github.com/${repoOwner}/${repoName}`,
                  category: 'Unknown',
                  hasAds: true,
                  hasIap: false,
                };
                
                const saved = await this.saveProspect(appResult, 'google_play', 'github_sdk_search');
                if (saved) discovered++;
              }
            }
          }
        }
        
        // GitHub has strict rate limits - wait 10 seconds between searches
        await this.delay(10000);
      } catch (error) {
        console.error(`[Discovery] Error searching GitHub:`, error);
        errors++;
      }
    }

    console.log(`[Discovery] GitHub: ${discovered} new prospects`);
    return { discovered, errors };
  }

  /**
   * Extract package name from GitHub repository
   */
  private async extractPackageFromGitHub(owner: string, repo: string): Promise<string | null> {
    try {
      // Check for Android app
      const gradleUrl = `https://raw.githubusercontent.com/${owner}/${repo}/main/app/build.gradle`;
      const gradle = await this.fetchUrl(gradleUrl).catch(() => null);
      
      if (gradle) {
        const packageMatch = gradle.match(/applicationId\s+["']([^"']+)["']/);
        if (packageMatch) return packageMatch[1];
      }
      
      // Check for iOS app
      const podfileUrl = `https://raw.githubusercontent.com/${owner}/${repo}/main/Podfile`;
      const podfile = await this.fetchUrl(podfileUrl).catch(() => null);
      
      if (podfile) {
        // iOS doesn't have bundle ID in Podfile, but we found an iOS app
        return `ios.${owner}.${repo}`.toLowerCase();
      }
      
      return null;
    } catch {
      return null;
    }
  }

  /**
   * Discover similar apps to high-value prospects already in database
   */
  private async discoverSimilarApps(): Promise<{ discovered: number; errors: number }> {
    console.log('[Discovery] Finding similar apps to high-value prospects...');
    
    let discovered = 0;
    let errors = 0;

    // Get top prospects by fit score
    const topProspects = await this.pool.query(`
      SELECT app_id, app_store, developer_id
      FROM sales_prospects
      WHERE fit_score >= 60
        AND status IN ('qualified', 'contacted')
        AND app_store = 'google_play'
      ORDER BY fit_score DESC
      LIMIT 10
    `);

    for (const prospect of topProspects.rows) {
      try {
        // Google Play "similar apps" section
        const similarUrl = `https://play.google.com/store/apps/details?id=${prospect.app_id}&hl=en&gl=us`;
        const html = await this.fetchUrl(similarUrl);
        
        // Look for "Similar apps" or "More by developer" sections
        const appIdRegex = /\/store\/apps\/details\?id=([a-zA-Z0-9._]+)/g;
        const matches = new Set<string>();
        let match;
        
        while ((match = appIdRegex.exec(html)) !== null) {
          if (match[1] !== prospect.app_id) {
            matches.add(match[1]);
          }
          if (matches.size >= 10) break;
        }

        for (const appId of matches) {
          const appDetails = await this.getGooglePlayAppDetails(appId);
          if (appDetails && this.passesFilters(appDetails)) {
            const saved = await this.saveProspect(appDetails, 'google_play', 'similar_to_' + prospect.app_id);
            if (saved) discovered++;
          }
          await this.delay(500);
        }
        
        await this.delay(2000);
      } catch (error) {
        errors++;
      }
    }

    console.log(`[Discovery] Similar apps: ${discovered} new prospects`);
    return { discovered, errors };
  }

  /**
   * Save a discovered prospect to the database
   */
  private async saveProspect(
    app: AppStoreResult,
    appStore: string,
    source: string
  ): Promise<boolean> {
    try {
      // Extract domain from developer name if possible
      const companyDomain = this.extractDomain(app.developerName);
      
      const result = await this.pool.query(`
        INSERT INTO sales_prospects (
          app_store, app_id, app_name, app_url,
          developer_name, developer_id, company_domain,
          app_category, downloads_estimate, rating, review_count,
          has_iap, has_ads, last_update_date,
          discovery_source, status
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, 'discovered')
        ON CONFLICT (app_store, app_id) DO UPDATE SET
          app_name = EXCLUDED.app_name,
          rating = COALESCE(EXCLUDED.rating, sales_prospects.rating),
          review_count = COALESCE(EXCLUDED.review_count, sales_prospects.review_count),
          downloads_estimate = COALESCE(EXCLUDED.downloads_estimate, sales_prospects.downloads_estimate),
          last_update_date = COALESCE(EXCLUDED.last_update_date, sales_prospects.last_update_date),
          updated_at = CURRENT_TIMESTAMP
        RETURNING (xmax = 0) as is_new
      `, [
        appStore,
        app.appId,
        app.appName,
        app.appUrl,
        app.developerName,
        app.developerId,
        companyDomain,
        app.category,
        app.downloadsEstimate,
        app.rating,
        app.reviewCount,
        app.hasIap,
        app.hasAds,
        app.lastUpdated,
        source,
      ]);

      return result.rows[0]?.is_new === true;
    } catch (error) {
      console.error(`[Discovery] Failed to save prospect ${app.appId}:`, error);
      return false;
    }
  }

  /**
   * Check if app passes our quality filters
   */
  private passesFilters(app: AppStoreResult): boolean {
    // Must have ads
    if (!app.hasAds) return false;
    
    // Check rating if configured
    if (this.config.minRating && app.rating !== undefined && app.rating < this.config.minRating) {
      return false;
    }
    
    // Check downloads if configured
    if (this.config.minDownloads && app.downloadsEstimate) {
      const minValue = this.downloadEstimateToValue(this.config.minDownloads);
      const appValue = this.downloadEstimateToValue(app.downloadsEstimate);
      if (appValue < minValue) return false;
    }
    
    return true;
  }

  /**
   * Extract company domain from developer name
   */
  private extractDomain(developerName: string): string | null {
    // Common patterns: "Company Name" -> company.com, "Company Inc" -> company.com
    const cleaned = developerName
      .toLowerCase()
      .replace(/\s+(inc|llc|ltd|corp|gmbh|limited|co\.?|company)\.?$/i, '')
      .replace(/[^a-z0-9]/g, '');
    
    if (cleaned.length >= 3) {
      return `${cleaned}.com`; // Guess - will be validated during enrichment
    }
    
    return null;
  }

  /**
   * Normalize download count strings to standard format
   */
  private normalizeDownloads(text: string | undefined): string | undefined {
    if (!text) return undefined;
    
    const cleaned = text.replace(/[,+]/g, '').toUpperCase();
    
    if (cleaned.includes('B')) return '1B+';
    if (cleaned.includes('M')) {
      const num = parseFloat(cleaned);
      if (num >= 100) return '100M+';
      if (num >= 50) return '50M-100M';
      if (num >= 10) return '10M-50M';
      if (num >= 5) return '5M-10M';
      if (num >= 1) return '1M-5M';
      return '500K-1M';
    }
    if (cleaned.includes('K')) {
      const num = parseFloat(cleaned);
      if (num >= 500) return '500K-1M';
      if (num >= 100) return '100K-500K';
      if (num >= 50) return '50K-100K';
      if (num >= 10) return '10K-50K';
      return '1K-10K';
    }
    
    const num = parseInt(cleaned, 10);
    if (!isNaN(num)) {
      if (num >= 1000000) return '1M-5M';
      if (num >= 100000) return '100K-500K';
      if (num >= 10000) return '10K-50K';
      if (num >= 1000) return '1K-10K';
    }
    
    return undefined;
  }

  /**
   * Convert download estimate string to numeric value for comparison
   */
  private downloadEstimateToValue(estimate: string): number {
    const map: Record<string, number> = {
      '1K-10K': 1000,
      '10K-50K': 10000,
      '50K-100K': 50000,
      '100K-500K': 100000,
      '500K-1M': 500000,
      '1M-5M': 1000000,
      '5M-10M': 5000000,
      '10M-50M': 10000000,
      '50M-100M': 50000000,
      '100M+': 100000000,
      '1B+': 1000000000,
    };
    return map[estimate] || 0;
  }

  /**
   * Update daily metrics
   */
  private async updateDailyMetrics(discovered: number): Promise<void> {
    await this.pool.query(`
      INSERT INTO sales_metrics_daily (date, prospects_discovered)
      VALUES (CURRENT_DATE, $1)
      ON CONFLICT (date) DO UPDATE SET
        prospects_discovered = sales_metrics_daily.prospects_discovered + $1,
        updated_at = CURRENT_TIMESTAMP
    `, [discovered]);
  }

  /**
   * Fetch URL with proper headers
   */
  private fetchUrl(url: string, headers: Record<string, string> = {}): Promise<string> {
    return new Promise((resolve, reject) => {
      const parsedUrl = new URL(url);
      const protocol = parsedUrl.protocol === 'https:' ? https : http;
      
      const options = {
        hostname: parsedUrl.hostname,
        port: parsedUrl.port,
        path: parsedUrl.pathname + parsedUrl.search,
        method: 'GET',
        headers: {
          'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
          'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8',
          'Accept-Language': 'en-US,en;q=0.9',
          ...headers,
        },
      };
      
      const req = protocol.request(options, (res) => {
        let data = '';
        res.on('data', (chunk) => { data += chunk; });
        res.on('end', () => {
          if (res.statusCode && res.statusCode >= 200 && res.statusCode < 300) {
            resolve(data);
          } else {
            reject(new Error(`HTTP ${res.statusCode}`));
          }
        });
      });
      
      req.on('error', reject);
      req.setTimeout(30000, () => {
        req.destroy();
        reject(new Error('Request timeout'));
      });
      req.end();
    });
  }

  /**
   * Decode HTML entities
   */
  private decodeHtml(text: string): string {
    return text
      .replace(/&amp;/g, '&')
      .replace(/&lt;/g, '<')
      .replace(/&gt;/g, '>')
      .replace(/&quot;/g, '"')
      .replace(/&#39;/g, "'")
      .replace(/&#x([0-9A-Fa-f]+);/g, (_, hex) => String.fromCharCode(parseInt(hex, 16)))
      .replace(/&#(\d+);/g, (_, dec) => String.fromCharCode(parseInt(dec, 10)));
  }

  /**
   * Delay helper
   */
  private delay(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  /**
   * Close database connection
   */
  async close(): Promise<void> {
    await this.pool.end();
  }
}
