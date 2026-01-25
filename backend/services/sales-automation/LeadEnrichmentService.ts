// services/sales-automation/LeadEnrichmentService.ts
// Enriches discovered prospects with company and contact data
// Uses only free public sources: WHOIS, DNS, GitHub, LinkedIn public pages

import { Pool } from 'pg';
import dns from 'dns';
import { promisify } from 'util';
import https from 'https';

const resolveMx = promisify(dns.resolveMx);
const resolveNs = promisify(dns.resolveNs);
const resolveTxt = promisify(dns.resolveTxt);

interface EnrichmentResult {
  enriched: number;
  errors: number;
  contacts_found: number;
}

interface CompanyEnrichment {
  domain: string;
  name?: string;
  industry?: string;
  employeeCountRange?: string;
  country?: string;
  techStack: string[];
  adNetworks: string[];
  emailProvider?: string;
  linkedinUrl?: string;
  twitterHandle?: string;
  githubOrg?: string;
}

interface ContactInfo {
  email: string;
  firstName?: string;
  lastName?: string;
  fullName?: string;
  jobTitle?: string;
  department?: string;
  seniority?: string;
  source: string;
}

// Common email patterns by format
const EMAIL_PATTERNS = [
  '{first}@{domain}',
  '{first}.{last}@{domain}',
  '{f}{last}@{domain}',
  '{first}{l}@{domain}',
  '{first}_{last}@{domain}',
  '{last}@{domain}',
  '{first}{last}@{domain}',
];

export class LeadEnrichmentService {
  private pool: Pool;

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * Run enrichment cycle - processes pending prospects
   */
  async runEnrichmentCycle(batchSize: number = 50): Promise<EnrichmentResult> {
    console.log('[Enrichment] Starting lead enrichment cycle...');
    
    let enriched = 0;
    let errors = 0;
    let contactsFound = 0;

    try {
      // Get prospects to enrich using the helper function
      const prospects = await this.pool.query(`
        SELECT * FROM get_prospects_to_enrich($1)
      `, [batchSize]);

      console.log(`[Enrichment] Processing ${prospects.rows.length} prospects`);

      for (const prospect of prospects.rows) {
        try {
          // Mark as enriching
          await this.pool.query(`
            UPDATE sales_prospects SET status = 'enriching', updated_at = CURRENT_TIMESTAMP
            WHERE id = $1
          `, [prospect.id]);

          // 1. Enrich company data
          const companyData = await this.enrichCompany(prospect);
          
          // 2. Find contacts
          const contacts = await this.findContacts(prospect, companyData);
          contactsFound += contacts.length;

          // 3. Calculate scores
          const fitScore = await this.calculateFitScore(prospect.id);
          const intentScore = await this.calculateIntentScore(prospect.id);

          // 4. Update prospect status
          const isQualified = fitScore >= 40 && contacts.length > 0;
          await this.pool.query(`
            UPDATE sales_prospects 
            SET status = $2,
                fit_score = $3,
                intent_score = $4,
                enrichment_score = $5,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = $1
          `, [
            prospect.id,
            isQualified ? 'qualified' : 'enriched',
            fitScore,
            intentScore,
            Math.round((fitScore + intentScore) / 2),
          ]);

          enriched++;

          // Rate limiting
          await this.delay(1000);
        } catch (error) {
          console.error(`[Enrichment] Error enriching prospect ${prospect.id}:`, error);
          errors++;
          
          // Mark as error but don't block
          await this.pool.query(`
            UPDATE sales_prospects 
            SET status = 'discovered', updated_at = CURRENT_TIMESTAMP
            WHERE id = $1
          `, [prospect.id]);
        }
      }

      // Update daily metrics
      await this.pool.query(`
        INSERT INTO sales_metrics_daily (date, prospects_enriched, contacts_discovered)
        VALUES (CURRENT_DATE, $1, $2)
        ON CONFLICT (date) DO UPDATE SET
          prospects_enriched = sales_metrics_daily.prospects_enriched + $1,
          contacts_discovered = sales_metrics_daily.contacts_discovered + $2,
          updated_at = CURRENT_TIMESTAMP
      `, [enriched, contactsFound]);

      console.log(`[Enrichment] Cycle complete: ${enriched} enriched, ${contactsFound} contacts, ${errors} errors`);
      return { enriched, errors, contacts_found: contactsFound };
    } catch (error) {
      console.error('[Enrichment] Error in enrichment cycle:', error);
      throw error;
    }
  }

  /**
   * Enrich company data from multiple free sources
   */
  private async enrichCompany(prospect: any): Promise<CompanyEnrichment> {
    const domain = prospect.company_domain || this.guessDomain(prospect.developer_name);
    
    if (!domain) {
      return { domain: '', techStack: [], adNetworks: [] };
    }

    const enrichment: CompanyEnrichment = {
      domain,
      techStack: [],
      adNetworks: [],
    };

    try {
      // 1. DNS/MX lookup - determine email provider and validate domain
      const emailProvider = await this.detectEmailProvider(domain);
      enrichment.emailProvider = emailProvider;

      // 2. Check for common tech indicators via DNS
      const techSignals = await this.detectTechFromDns(domain);
      enrichment.techStack = techSignals;

      // 3. Try to find company info from GitHub
      const githubInfo = await this.enrichFromGitHub(domain, prospect.developer_name);
      if (githubInfo) {
        enrichment.githubOrg = githubInfo.org;
        enrichment.name = githubInfo.name || prospect.developer_name;
        enrichment.techStack = [...enrichment.techStack, ...githubInfo.languages];
      }

      // 4. Detect ad networks from app store page or GitHub
      const adNetworks = await this.detectAdNetworks(prospect);
      enrichment.adNetworks = adNetworks;

      // Save to companies table
      await this.pool.query(`
        INSERT INTO sales_companies (
          domain, name, tech_stack, ad_networks, github_org, country_code,
          enrichment_sources, last_enriched_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, CURRENT_TIMESTAMP)
        ON CONFLICT (domain) DO UPDATE SET
          name = COALESCE(EXCLUDED.name, sales_companies.name),
          tech_stack = EXCLUDED.tech_stack,
          ad_networks = EXCLUDED.ad_networks,
          github_org = COALESCE(EXCLUDED.github_org, sales_companies.github_org),
          enrichment_sources = EXCLUDED.enrichment_sources,
          last_enriched_at = CURRENT_TIMESTAMP,
          updated_at = CURRENT_TIMESTAMP
      `, [
        domain,
        enrichment.name || prospect.developer_name,
        JSON.stringify(enrichment.techStack),
        JSON.stringify(enrichment.adNetworks),
        enrichment.githubOrg,
        null, // Country determined later
        JSON.stringify(['dns', 'github']),
      ]);

      // Link prospect to company
      await this.pool.query(`
        UPDATE sales_prospects 
        SET company_domain = $1, company_name = $2, updated_at = CURRENT_TIMESTAMP
        WHERE id = $3
      `, [domain, enrichment.name, prospect.id]);

      return enrichment;
    } catch (error) {
      console.error(`[Enrichment] Error enriching company ${domain}:`, error);
      return enrichment;
    }
  }

  /**
   * Detect email provider from MX records
   */
  private async detectEmailProvider(domain: string): Promise<string | undefined> {
    try {
      const mxRecords = await resolveMx(domain);
      if (!mxRecords || mxRecords.length === 0) return undefined;

      const mxHost = mxRecords[0].exchange.toLowerCase();

      if (mxHost.includes('google') || mxHost.includes('googlemail')) return 'google_workspace';
      if (mxHost.includes('outlook') || mxHost.includes('microsoft')) return 'microsoft_365';
      if (mxHost.includes('zoho')) return 'zoho';
      if (mxHost.includes('protonmail')) return 'protonmail';
      if (mxHost.includes('fastmail')) return 'fastmail';
      if (mxHost.includes('icloud')) return 'icloud';
      if (mxHost.includes('mxroute')) return 'mxroute';

      return 'custom';
    } catch {
      return undefined;
    }
  }

  /**
   * Detect tech stack from DNS records
   */
  private async detectTechFromDns(domain: string): Promise<string[]> {
    const tech: string[] = [];

    try {
      // Check TXT records for verification entries
      const txtRecords = await resolveTxt(domain).catch(() => []);
      const txtFlat = txtRecords.flat().join(' ').toLowerCase();

      if (txtFlat.includes('google-site-verification')) tech.push('google_analytics');
      if (txtFlat.includes('facebook-domain-verification')) tech.push('facebook_pixel');
      if (txtFlat.includes('ms=')) tech.push('microsoft_365');
      if (txtFlat.includes('v=spf1')) tech.push('spf_configured');
      if (txtFlat.includes('_dmarc')) tech.push('dmarc_configured');
      if (txtFlat.includes('stripe-verification')) tech.push('stripe');
      if (txtFlat.includes('hubspot')) tech.push('hubspot');
      if (txtFlat.includes('mailchimp')) tech.push('mailchimp');
      if (txtFlat.includes('salesforce')) tech.push('salesforce');
    } catch {
      // DNS lookup failed
    }

    return tech;
  }

  /**
   * Enrich from GitHub organization/user
   */
  private async enrichFromGitHub(domain: string, developerName: string): Promise<{
    org?: string;
    name?: string;
    languages: string[];
  } | null> {
    try {
      // Try to find GitHub org by domain or name
      const orgName = domain.split('.')[0].toLowerCase();
      const searchTerm = encodeURIComponent(developerName || orgName);
      
      const searchUrl = `https://api.github.com/search/users?q=${searchTerm}+type:org&per_page=1`;
      
      const response = await this.fetchUrl(searchUrl, {
        'User-Agent': 'ApexMediation-Enrichment/1.0',
        'Accept': 'application/vnd.github.v3+json',
      });
      
      const data = JSON.parse(response);
      
      if (data.items && data.items.length > 0) {
        const org = data.items[0];
        
        // Get org details
        const orgUrl = `https://api.github.com/orgs/${org.login}`;
        const orgResponse = await this.fetchUrl(orgUrl, {
          'User-Agent': 'ApexMediation-Enrichment/1.0',
          'Accept': 'application/vnd.github.v3+json',
        });
        const orgData = JSON.parse(orgResponse);
        
        // Get top languages from repos
        const reposUrl = `https://api.github.com/orgs/${org.login}/repos?per_page=10&sort=pushed`;
        const reposResponse = await this.fetchUrl(reposUrl, {
          'User-Agent': 'ApexMediation-Enrichment/1.0',
          'Accept': 'application/vnd.github.v3+json',
        });
        const repos = JSON.parse(reposResponse) as Array<{ language?: string }>;
        
        const languageSet = new Set<string>();
        for (const r of repos) {
          if (r.language) {
            languageSet.add(r.language);
          }
        }
        const languages = Array.from(languageSet);

        return {
          org: org.login,
          name: orgData.name || org.login,
          languages,
        };
      }
      
      return null;
    } catch (error) {
      return null;
    }
  }

  /**
   * Detect which ad networks the app is using
   */
  private async detectAdNetworks(prospect: any): Promise<string[]> {
    const networks: string[] = [];
    
    // Check if we have info from GitHub
    if (prospect.app_id) {
      // Common ad SDKs to look for
      const sdkIndicators: Record<string, string[]> = {
        'admob': ['com.google.android.gms.ads', 'Google-Mobile-Ads-SDK'],
        'facebook': ['com.facebook.ads', 'FBAudienceNetwork'],
        'unity': ['com.unity3d.ads', 'UnityAds'],
        'applovin': ['com.applovin', 'AppLovinSDK'],
        'ironsource': ['com.ironsource', 'IronSource'],
        'vungle': ['com.vungle', 'VungleSDK'],
        'chartboost': ['com.chartboost', 'Chartboost'],
        'mopub': ['com.mopub', 'mopub-ios-sdk'],
        'inmobi': ['com.inmobi', 'InMobiSDK'],
        'tapjoy': ['com.tapjoy', 'Tapjoy'],
      };
      
      // This would check the app's manifest/dependencies
      // For now, mark as unknown if we can't detect
      if (prospect.has_ads) {
        networks.push('unknown_ad_network');
      }
    }
    
    return networks;
  }

  /**
   * Find contacts for a prospect/company
   */
  private async findContacts(prospect: any, company: CompanyEnrichment): Promise<ContactInfo[]> {
    const contacts: ContactInfo[] = [];
    
    if (!company.domain) return contacts;

    try {
      // 1. Try to find contacts from GitHub
      if (company.githubOrg) {
        const githubContacts = await this.findContactsFromGitHub(company.githubOrg);
        contacts.push(...githubContacts);
      }

      // 2. Generate likely email addresses based on common patterns
      const guessedContacts = await this.guessContactEmails(company.domain);
      contacts.push(...guessedContacts);

      // Save contacts to database
      for (const contact of contacts) {
        await this.saveContact(contact, prospect.id, company.domain);
      }

      return contacts;
    } catch (error) {
      console.error(`[Enrichment] Error finding contacts for ${company.domain}:`, error);
      return contacts;
    }
  }

  /**
   * Find contacts from GitHub organization
   */
  private async findContactsFromGitHub(orgName: string): Promise<ContactInfo[]> {
    const contacts: ContactInfo[] = [];

    try {
      // Get org members (public only)
      const membersUrl = `https://api.github.com/orgs/${orgName}/members?per_page=10`;
      const response = await this.fetchUrl(membersUrl, {
        'User-Agent': 'ApexMediation-Enrichment/1.0',
        'Accept': 'application/vnd.github.v3+json',
      });
      
      const members = JSON.parse(response);
      
      for (const member of members) {
        // Get user details
        const userUrl = `https://api.github.com/users/${member.login}`;
        const userResponse = await this.fetchUrl(userUrl, {
          'User-Agent': 'ApexMediation-Enrichment/1.0',
          'Accept': 'application/vnd.github.v3+json',
        });
        const user = JSON.parse(userResponse);
        
        // Check if they have a public email
        if (user.email) {
          contacts.push({
            email: user.email,
            fullName: user.name || user.login,
            jobTitle: 'Developer',
            department: 'engineering',
            seniority: 'individual',
            source: 'github',
          });
        }
        
        await this.delay(500); // Rate limit
      }
    } catch (error) {
      // GitHub API limits or org not found
    }

    return contacts;
  }

  /**
   * Guess likely email addresses for a domain
   */
  private async guessContactEmails(domain: string): Promise<ContactInfo[]> {
    const contacts: ContactInfo[] = [];
    
    // Common roles we want to reach
    const targetRoles = [
      { first: 'founder', title: 'Founder', seniority: 'c_level' },
      { first: 'ceo', title: 'CEO', seniority: 'c_level' },
      { first: 'cto', title: 'CTO', seniority: 'c_level' },
      { first: 'developer', title: 'Developer', seniority: 'individual' },
      { first: 'tech', title: 'Tech Lead', seniority: 'manager' },
      { first: 'ads', title: 'Ad Ops', seniority: 'individual' },
      { first: 'monetization', title: 'Monetization Manager', seniority: 'manager' },
      { first: 'contact', title: 'Contact', seniority: 'unknown' },
      { first: 'hello', title: 'General', seniority: 'unknown' },
      { first: 'info', title: 'General', seniority: 'unknown' },
    ];
    
    for (const role of targetRoles) {
      const email = `${role.first}@${domain}`;
      
      // Verify email with MX check (basic validation)
      const isValid = await this.verifyEmailDomain(domain);
      
      if (isValid) {
        contacts.push({
          email,
          firstName: role.first,
          jobTitle: role.title,
          department: role.first === 'ads' || role.first === 'monetization' ? 'ad_ops' : 'engineering',
          seniority: role.seniority,
          source: 'email_pattern',
        });
      }
    }
    
    return contacts;
  }

  /**
   * Verify email domain has valid MX records
   */
  private async verifyEmailDomain(domain: string): Promise<boolean> {
    try {
      const mxRecords = await resolveMx(domain);
      return mxRecords && mxRecords.length > 0;
    } catch {
      return false;
    }
  }

  /**
   * Save contact to database
   */
  private async saveContact(contact: ContactInfo, prospectId: string, domain: string): Promise<void> {
    try {
      // Get company ID
      const companyResult = await this.pool.query(`
        SELECT id FROM sales_companies WHERE domain = $1 LIMIT 1
      `, [domain]);
      
      const companyId = companyResult.rows[0]?.id;
      
      await this.pool.query(`
        INSERT INTO sales_contacts (
          company_id, prospect_id, email, first_name, last_name, full_name,
          job_title, department, seniority, discovery_source, status
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'discovered')
        ON CONFLICT (email) DO UPDATE SET
          job_title = COALESCE(EXCLUDED.job_title, sales_contacts.job_title),
          updated_at = CURRENT_TIMESTAMP
      `, [
        companyId,
        prospectId,
        contact.email.toLowerCase(),
        contact.firstName,
        contact.lastName,
        contact.fullName,
        contact.jobTitle,
        contact.department,
        contact.seniority,
        contact.source,
      ]);
    } catch (error) {
      // Ignore duplicate errors
    }
  }

  /**
   * Calculate fit score for a prospect
   */
  private async calculateFitScore(prospectId: string): Promise<number> {
    // Use the database function
    const result = await this.pool.query(`
      SELECT calculate_prospect_fit_score($1) as score
    `, [prospectId]);
    
    return result.rows[0]?.score || 0;
  }

  /**
   * Calculate intent score based on signals
   */
  private async calculateIntentScore(prospectId: string): Promise<number> {
    let score = 0;
    
    const prospect = await this.pool.query(`
      SELECT * FROM sales_prospects WHERE id = $1
    `, [prospectId]);
    
    if (!prospect.rows[0]) return 0;
    
    const p = prospect.rows[0];
    
    // Recent app update (active development) - 30 points
    if (p.last_update_date) {
      const daysSinceUpdate = Math.floor((Date.now() - new Date(p.last_update_date).getTime()) / (1000 * 60 * 60 * 24));
      if (daysSinceUpdate <= 7) score += 30;
      else if (daysSinceUpdate <= 30) score += 20;
      else if (daysSinceUpdate <= 90) score += 10;
    }
    
    // Has ads (already monetizing) - 25 points
    if (p.has_ads) score += 25;
    
    // High rating (quality product) - 15 points
    if (p.rating >= 4.5) score += 15;
    else if (p.rating >= 4.0) score += 10;
    
    // Growing downloads - 20 points (based on download tier)
    if (p.downloads_estimate) {
      const tier = p.downloads_estimate;
      if (tier.includes('M')) score += 20;
      else if (tier.includes('500K') || tier.includes('100K')) score += 15;
      else if (tier.includes('50K') || tier.includes('10K')) score += 10;
    }
    
    // Gaming category (highest ad spend) - 10 points
    if (p.app_category?.toLowerCase().includes('game')) score += 10;
    
    return Math.min(score, 100);
  }

  /**
   * Guess domain from developer name
   */
  private guessDomain(developerName: string): string | null {
    if (!developerName) return null;
    
    // Clean up common suffixes
    const cleaned = developerName
      .toLowerCase()
      .replace(/\s+(inc|llc|ltd|corp|gmbh|limited|co\.?|company|studios?|games?)\.?$/gi, '')
      .trim()
      .replace(/[^a-z0-9]/g, '');
    
    if (cleaned.length >= 3) {
      return `${cleaned}.com`;
    }
    
    return null;
  }

  /**
   * Fetch URL helper
   */
  private fetchUrl(url: string, headers: Record<string, string> = {}): Promise<string> {
    return new Promise((resolve, reject) => {
      const parsedUrl = new URL(url);
      
      const options = {
        hostname: parsedUrl.hostname,
        port: parsedUrl.port,
        path: parsedUrl.pathname + parsedUrl.search,
        method: 'GET',
        headers: {
          'User-Agent': 'Mozilla/5.0',
          ...headers,
        },
      };
      
      const req = https.request(options, (res) => {
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
      req.setTimeout(15000, () => {
        req.destroy();
        reject(new Error('Timeout'));
      });
      req.end();
    });
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
