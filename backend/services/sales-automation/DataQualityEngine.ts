// services/sales-automation/DataQualityEngine.ts
// OVERCOMES: "data is garbage", "verified emails still bounce", "outdated contacts"
// OVERCOMES: "credits wasted on dead emails", "phone numbers are wrong"
// Based on Apollo.io complaints: "verified emails not accurate", "data is months/years old"
// Zero-cost: Algorithmic data quality scoring and validation, no paid verification APIs

import { Pool } from 'pg';

interface DataQualityScore {
  overall: number;  // 0-100
  components: {
    emailValidity: number;
    dataFreshness: number;
    completeness: number;
    engagement: number;
    bounceHistory: number;
  };
  flags: DataQualityFlag[];
  recommendations: string[];
}

interface DataQualityFlag {
  type: 'warning' | 'error' | 'info';
  field: string;
  message: string;
  autoFixable: boolean;
}

interface ContactData {
  id: string;
  email: string;
  firstName?: string;
  lastName?: string;
  title?: string;
  phone?: string;
  linkedinUrl?: string;
  companyName?: string;
  companyDomain?: string;
  createdAt: Date;
  updatedAt: Date;
  lastVerified?: Date;
  bounceCount: number;
  lastBounce?: Date;
}

// Common disposable email domains
const DISPOSABLE_DOMAINS = new Set([
  'mailinator.com', 'guerrillamail.com', 'tempmail.com', 'throwaway.email',
  'temp-mail.org', 'fakeinbox.com', 'getnada.com', 'mailnesia.com',
  'yopmail.com', '10minutemail.com', 'trashmail.com', 'sharklasers.com',
]);

// Role-based emails that often bounce or are shared
const ROLE_BASED_PREFIXES = [
  'info', 'admin', 'support', 'sales', 'contact', 'hello', 'help',
  'team', 'office', 'mail', 'webmaster', 'postmaster', 'noreply',
];

// Common typo patterns in email domains
const DOMAIN_TYPOS: Record<string, string> = {
  'gmial.com': 'gmail.com',
  'gamil.com': 'gmail.com',
  'gmai.com': 'gmail.com',
  'gnail.com': 'gmail.com',
  'hotmal.com': 'hotmail.com',
  'hotmai.com': 'hotmail.com',
  'outlok.com': 'outlook.com',
  'outllok.com': 'outlook.com',
  'yahho.com': 'yahoo.com',
  'yahooo.com': 'yahoo.com',
};

/**
 * DataQualityEngine - Ensure data accuracy before outreach
 * OVERCOMES: "verified emails bounce", "data is garbage"
 */
export class DataQualityEngine {
  private pool: Pool;

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * Comprehensive quality check for a contact
   */
  async assessContactQuality(contactId: string): Promise<DataQualityScore> {
    const contact = await this.getContactData(contactId);
    if (!contact) {
      return this.createEmptyScore('Contact not found');
    }

    const flags: DataQualityFlag[] = [];
    const recommendations: string[] = [];

    // 1. Email validity check
    const emailValidity = this.checkEmailValidity(contact.email, flags);

    // 2. Data freshness check
    const dataFreshness = this.checkDataFreshness(contact, flags);

    // 3. Completeness check
    const completeness = this.checkCompleteness(contact, flags);

    // 4. Engagement score (has this contact engaged positively?)
    const engagement = await this.checkEngagementHistory(contactId, flags);

    // 5. Bounce history check
    const bounceHistory = this.checkBounceHistory(contact, flags);

    // Calculate overall score
    const overall = Math.round(
      emailValidity * 0.35 +
      dataFreshness * 0.20 +
      completeness * 0.15 +
      engagement * 0.15 +
      bounceHistory * 0.15
    );

    // Generate recommendations
    this.generateRecommendations(flags, recommendations, contact);

    return {
      overall,
      components: {
        emailValidity,
        dataFreshness,
        completeness,
        engagement,
        bounceHistory,
      },
      flags,
      recommendations,
    };
  }

  /**
   * Batch quality assessment
   */
  async assessBatchQuality(contactIds: string[]): Promise<Map<string, DataQualityScore>> {
    const results = new Map<string, DataQualityScore>();
    
    // Process in parallel batches
    const batchSize = 50;
    for (let i = 0; i < contactIds.length; i += batchSize) {
      const batch = contactIds.slice(i, i + batchSize);
      const assessments = await Promise.all(
        batch.map(id => this.assessContactQuality(id))
      );
      batch.forEach((id, idx) => results.set(id, assessments[idx]));
    }

    return results;
  }

  /**
   * Filter contacts by minimum quality score
   */
  async filterByQuality(contactIds: string[], minScore: number = 60): Promise<string[]> {
    const assessments = await this.assessBatchQuality(contactIds);
    return contactIds.filter(id => {
      const score = assessments.get(id);
      return score && score.overall >= minScore;
    });
  }

  /**
   * Auto-fix common data issues
   * OVERCOMES: "data is garbage but can't fix it"
   */
  async autoFixContact(contactId: string): Promise<{
    fixed: boolean;
    changes: string[];
    newQualityScore: number;
  }> {
    const contact = await this.getContactData(contactId);
    if (!contact) {
      return { fixed: false, changes: [], newQualityScore: 0 };
    }

    const changes: string[] = [];
    const updates: Record<string, unknown> = {};

    // Fix email typos
    const emailDomain = contact.email.split('@')[1]?.toLowerCase();
    if (emailDomain && DOMAIN_TYPOS[emailDomain]) {
      const correctedEmail = contact.email.replace(emailDomain, DOMAIN_TYPOS[emailDomain]);
      updates.email = correctedEmail;
      changes.push(`Fixed email typo: ${contact.email} → ${correctedEmail}`);
    }

    // Normalize name capitalization
    if (contact.firstName) {
      const normalizedFirst = this.capitalizeName(contact.firstName);
      if (normalizedFirst !== contact.firstName) {
        updates.first_name = normalizedFirst;
        changes.push(`Normalized first name: ${contact.firstName} → ${normalizedFirst}`);
      }
    }

    if (contact.lastName) {
      const normalizedLast = this.capitalizeName(contact.lastName);
      if (normalizedLast !== contact.lastName) {
        updates.last_name = normalizedLast;
        changes.push(`Normalized last name: ${contact.lastName} → ${normalizedLast}`);
      }
    }

    // Fix LinkedIn URL format
    if (contact.linkedinUrl) {
      const fixedLinkedin = this.normalizeLinkedInUrl(contact.linkedinUrl);
      if (fixedLinkedin !== contact.linkedinUrl) {
        updates.linkedin_url = fixedLinkedin;
        changes.push(`Normalized LinkedIn URL`);
      }
    }

    // Apply updates if any
    if (Object.keys(updates).length > 0) {
      updates.updated_at = new Date();
      
      const setClauses = Object.keys(updates).map((k, i) => `${k} = $${i + 2}`);
      await this.pool.query(
        `UPDATE sales_contacts SET ${setClauses.join(', ')} WHERE id = $1`,
        [contactId, ...Object.values(updates)]
      );
    }

    // Reassess quality
    const newScore = await this.assessContactQuality(contactId);

    return {
      fixed: changes.length > 0,
      changes,
      newQualityScore: newScore.overall,
    };
  }

  /**
   * Find and merge duplicate contacts
   * OVERCOMES: "duplicates everywhere"
   */
  async findDuplicates(): Promise<Array<{
    primaryId: string;
    duplicateIds: string[];
    matchReason: string;
    confidence: number;
  }>> {
    const duplicates: Array<{
      primaryId: string;
      duplicateIds: string[];
      matchReason: string;
      confidence: number;
    }> = [];

    // Find exact email duplicates
    const emailDupes = await this.pool.query(`
      SELECT email, array_agg(id ORDER BY created_at) as ids
      FROM sales_contacts
      WHERE email IS NOT NULL
      GROUP BY LOWER(email)
      HAVING COUNT(*) > 1
    `);

    for (const row of emailDupes.rows) {
      const ids = row.ids;
      duplicates.push({
        primaryId: ids[0],
        duplicateIds: ids.slice(1),
        matchReason: 'Exact email match',
        confidence: 100,
      });
    }

    // Find phone duplicates
    const phoneDupes = await this.pool.query(`
      SELECT 
        regexp_replace(phone, '[^0-9]', '', 'g') as normalized_phone,
        array_agg(id ORDER BY created_at) as ids
      FROM sales_contacts
      WHERE phone IS NOT NULL AND phone != ''
      GROUP BY regexp_replace(phone, '[^0-9]', '', 'g')
      HAVING COUNT(*) > 1
    `);

    for (const row of phoneDupes.rows) {
      const ids = row.ids;
      // Don't add if already marked as duplicate by email
      if (!duplicates.some(d => d.primaryId === ids[0])) {
        duplicates.push({
          primaryId: ids[0],
          duplicateIds: ids.slice(1),
          matchReason: 'Same phone number',
          confidence: 95,
        });
      }
    }

    return duplicates;
  }

  /**
   * Flag stale contacts that need re-verification
   */
  async flagStaleContacts(staleDays: number = 90): Promise<string[]> {
    const cutoff = new Date(Date.now() - staleDays * 24 * 60 * 60 * 1000);
    
    const result = await this.pool.query(`
      SELECT id FROM sales_contacts
      WHERE (last_verified IS NULL OR last_verified < $1)
      AND status = 'active'
      ORDER BY last_verified ASC NULLS FIRST
      LIMIT 1000
    `, [cutoff]);

    // Mark them as needing re-verification
    const ids = result.rows.map(r => r.id);
    if (ids.length > 0) {
      await this.pool.query(`
        UPDATE sales_contacts 
        SET needs_reverification = true
        WHERE id = ANY($1)
      `, [ids]);
    }

    return ids;
  }

  /**
   * Generate quality report for all contacts
   */
  async generateQualityReport(): Promise<{
    totalContacts: number;
    qualityDistribution: Record<string, number>;
    commonIssues: Array<{ issue: string; count: number }>;
    actionRequired: number;
  }> {
    // Get all active contacts
    const contactsResult = await this.pool.query(`
      SELECT id FROM sales_contacts WHERE status = 'active' LIMIT 5000
    `);

    const contactIds = contactsResult.rows.map(r => r.id);
    const assessments = await this.assessBatchQuality(contactIds);

    // Count by quality tier
    const distribution: Record<string, number> = {
      excellent: 0,  // 80-100
      good: 0,       // 60-79
      fair: 0,       // 40-59
      poor: 0,       // 0-39
    };

    const issueCounts = new Map<string, number>();
    let actionRequired = 0;

    for (const score of assessments.values()) {
      if (score.overall >= 80) distribution.excellent++;
      else if (score.overall >= 60) distribution.good++;
      else if (score.overall >= 40) distribution.fair++;
      else distribution.poor++;

      if (score.overall < 60) actionRequired++;

      for (const flag of score.flags) {
        const key = flag.message;
        issueCounts.set(key, (issueCounts.get(key) || 0) + 1);
      }
    }

    // Sort issues by count
    const commonIssues = Array.from(issueCounts.entries())
      .map(([issue, count]) => ({ issue, count }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 10);

    return {
      totalContacts: contactIds.length,
      qualityDistribution: distribution,
      commonIssues,
      actionRequired,
    };
  }

  // ===== Private Methods =====

  private async getContactData(contactId: string): Promise<ContactData | null> {
    const result = await this.pool.query(`
      SELECT 
        c.id, c.email, c.first_name, c.last_name, c.title, c.phone, c.linkedin_url,
        c.created_at, c.updated_at, c.last_verified,
        comp.name as company_name, comp.website as company_domain,
        COALESCE((SELECT COUNT(*) FROM outreach_emails WHERE contact_id = c.id AND bounced_at IS NOT NULL), 0) as bounce_count,
        (SELECT MAX(bounced_at) FROM outreach_emails WHERE contact_id = c.id) as last_bounce
      FROM sales_contacts c
      LEFT JOIN sales_companies comp ON c.company_id = comp.id
      WHERE c.id = $1
    `, [contactId]);

    if (result.rows.length === 0) return null;

    const row = result.rows[0];
    return {
      id: row.id,
      email: row.email,
      firstName: row.first_name,
      lastName: row.last_name,
      title: row.title,
      phone: row.phone,
      linkedinUrl: row.linkedin_url,
      companyName: row.company_name,
      companyDomain: row.company_domain,
      createdAt: row.created_at,
      updatedAt: row.updated_at,
      lastVerified: row.last_verified,
      bounceCount: parseInt(row.bounce_count) || 0,
      lastBounce: row.last_bounce,
    };
  }

  private checkEmailValidity(email: string, flags: DataQualityFlag[]): number {
    let score = 100;

    if (!email) {
      flags.push({ type: 'error', field: 'email', message: 'Email is missing', autoFixable: false });
      return 0;
    }

    // Basic format validation
    const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    if (!emailRegex.test(email)) {
      flags.push({ type: 'error', field: 'email', message: 'Invalid email format', autoFixable: false });
      return 10;
    }

    const [localPart, domain] = email.toLowerCase().split('@');

    // Check for disposable domain
    if (DISPOSABLE_DOMAINS.has(domain)) {
      flags.push({ type: 'warning', field: 'email', message: 'Disposable email domain detected', autoFixable: false });
      score -= 40;
    }

    // Check for role-based email
    if (ROLE_BASED_PREFIXES.some(prefix => localPart === prefix || localPart.startsWith(`${prefix}.`) || localPart.startsWith(`${prefix}_`))) {
      flags.push({ type: 'warning', field: 'email', message: 'Role-based email address (may be shared)', autoFixable: false });
      score -= 20;
    }

    // Check for common typos
    if (DOMAIN_TYPOS[domain]) {
      flags.push({ type: 'warning', field: 'email', message: `Possible typo in domain: ${domain}`, autoFixable: true });
      score -= 15;
    }

    // Check for suspicious patterns
    if (/test|fake|example|asdf/i.test(localPart)) {
      flags.push({ type: 'warning', field: 'email', message: 'Suspicious email pattern detected', autoFixable: false });
      score -= 30;
    }

    return Math.max(0, score);
  }

  private checkDataFreshness(contact: ContactData, flags: DataQualityFlag[]): number {
    const now = Date.now();
    let score = 100;

    // Check when contact was last verified
    if (!contact.lastVerified) {
      flags.push({ type: 'info', field: 'last_verified', message: 'Contact never verified', autoFixable: false });
      score -= 20;
    } else {
      const daysSinceVerification = (now - contact.lastVerified.getTime()) / (1000 * 60 * 60 * 24);
      if (daysSinceVerification > 180) {
        flags.push({ type: 'warning', field: 'last_verified', message: 'Verification over 6 months old', autoFixable: false });
        score -= 30;
      } else if (daysSinceVerification > 90) {
        flags.push({ type: 'info', field: 'last_verified', message: 'Verification over 3 months old', autoFixable: false });
        score -= 15;
      }
    }

    // Check when contact was last updated
    const daysSinceUpdate = (now - contact.updatedAt.getTime()) / (1000 * 60 * 60 * 24);
    if (daysSinceUpdate > 365) {
      flags.push({ type: 'warning', field: 'updated_at', message: 'Contact data over 1 year old', autoFixable: false });
      score -= 20;
    }

    return Math.max(0, score);
  }

  private checkCompleteness(contact: ContactData, flags: DataQualityFlag[]): number {
    let fieldsPresent = 0;
    const totalFields = 6;

    if (contact.firstName && contact.firstName.length > 1) fieldsPresent++;
    else flags.push({ type: 'info', field: 'first_name', message: 'First name missing or incomplete', autoFixable: false });

    if (contact.lastName && contact.lastName.length > 1) fieldsPresent++;
    else flags.push({ type: 'info', field: 'last_name', message: 'Last name missing or incomplete', autoFixable: false });

    if (contact.title) fieldsPresent++;
    if (contact.phone) fieldsPresent++;
    if (contact.linkedinUrl) fieldsPresent++;
    if (contact.companyName) fieldsPresent++;

    return Math.round((fieldsPresent / totalFields) * 100);
  }

  private async checkEngagementHistory(contactId: string, flags: DataQualityFlag[]): Promise<number> {
    const result = await this.pool.query(`
      SELECT 
        COUNT(*) FILTER (WHERE opened_at IS NOT NULL) as opens,
        COUNT(*) FILTER (WHERE clicked_at IS NOT NULL) as clicks,
        COUNT(*) FILTER (WHERE replied_at IS NOT NULL) as replies,
        COUNT(*) as total_sent
      FROM outreach_emails
      WHERE contact_id = $1 AND status = 'sent'
    `, [contactId]);

    const stats = result.rows[0];
    const totalSent = parseInt(stats.total_sent) || 0;

    if (totalSent === 0) {
      // New contact, neutral score
      return 70;
    }

    const opens = parseInt(stats.opens) || 0;
    const clicks = parseInt(stats.clicks) || 0;
    const replies = parseInt(stats.replies) || 0;

    // Calculate engagement score
    let score = 50; // Base

    const openRate = opens / totalSent;
    const clickRate = clicks / totalSent;

    if (openRate > 0.5) score += 20;
    else if (openRate > 0.2) score += 10;
    else if (openRate === 0 && totalSent >= 3) {
      flags.push({ type: 'warning', field: 'engagement', message: 'No opens after multiple emails', autoFixable: false });
      score -= 20;
    }

    if (clickRate > 0.1) score += 15;
    if (replies > 0) score += 15;

    return Math.min(100, Math.max(0, score));
  }

  private checkBounceHistory(contact: ContactData, flags: DataQualityFlag[]): number {
    let score = 100;

    if (contact.bounceCount > 0) {
      if (contact.bounceCount >= 3) {
        flags.push({ type: 'error', field: 'bounce', message: 'Multiple bounces - likely invalid email', autoFixable: false });
        score = 10;
      } else if (contact.bounceCount === 2) {
        flags.push({ type: 'warning', field: 'bounce', message: 'Email has bounced twice', autoFixable: false });
        score = 40;
      } else {
        flags.push({ type: 'info', field: 'bounce', message: 'Email has bounced once', autoFixable: false });
        score = 70;
      }

      // Recent bounce is worse
      if (contact.lastBounce) {
        const daysSinceBounce = (Date.now() - contact.lastBounce.getTime()) / (1000 * 60 * 60 * 24);
        if (daysSinceBounce < 30) score -= 20;
      }
    }

    return Math.max(0, score);
  }

  private generateRecommendations(flags: DataQualityFlag[], recommendations: string[], contact: ContactData): void {
    const hasError = flags.some(f => f.type === 'error');
    const autoFixable = flags.filter(f => f.autoFixable);

    if (hasError) {
      recommendations.push('Critical issues detected - review before sending emails');
    }

    if (autoFixable.length > 0) {
      recommendations.push(`${autoFixable.length} issue(s) can be auto-fixed`);
    }

    if (!contact.linkedinUrl) {
      recommendations.push('Add LinkedIn profile for better personalization');
    }

    if (contact.bounceCount > 0) {
      recommendations.push('Verify email address before next outreach attempt');
    }

    if (recommendations.length === 0) {
      recommendations.push('Data quality is good - safe to proceed with outreach');
    }
  }

  private createEmptyScore(reason: string): DataQualityScore {
    return {
      overall: 0,
      components: {
        emailValidity: 0,
        dataFreshness: 0,
        completeness: 0,
        engagement: 0,
        bounceHistory: 0,
      },
      flags: [{ type: 'error', field: 'general', message: reason, autoFixable: false }],
      recommendations: [reason],
    };
  }

  private capitalizeName(name: string): string {
    if (!name) return name;
    
    // Handle hyphenated names
    return name.split('-').map(part =>
      part.charAt(0).toUpperCase() + part.slice(1).toLowerCase()
    ).join('-');
  }

  private normalizeLinkedInUrl(url: string): string {
    if (!url) return url;
    
    // Extract just the profile path
    const match = url.match(/linkedin\.com\/in\/([a-zA-Z0-9\-]+)/i);
    if (match) {
      return `https://www.linkedin.com/in/${match[1].toLowerCase()}`;
    }
    
    return url;
  }
}
