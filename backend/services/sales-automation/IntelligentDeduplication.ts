// services/sales-automation/IntelligentDeduplication.ts
// OVERCOMES: "duplicate contacts", "same person emailed multiple times", "messy data"
// OVERCOMES: "credits wasted on duplicates", "angry recipients getting multiple emails"
// Based on Apollo complaints: "data is garbage", "outdated contacts", "duplicates everywhere"
// Zero-cost: Pure algorithmic deduplication, no external APIs

import { Pool } from 'pg';

interface DuplicateGroup {
  primary: ContactRecord;
  duplicates: ContactRecord[];
  mergeStrategy: MergeStrategy;
  confidence: number;
}

interface ContactRecord {
  id: string;
  email: string;
  first_name: string | null;
  last_name: string | null;
  company_id: string | null;
  company_name: string | null;
  title: string | null;
  phone: string | null;
  linkedin_url: string | null;
  created_at: Date;
  enrichment_score: number;
  emails_sent: number;
  replied: boolean;
}

interface MergeStrategy {
  keepFields: string[];
  mergeFields: string[];
  sourcePreference: 'newest' | 'oldest' | 'most_enriched' | 'most_engaged';
}

interface DeduplicationResult {
  duplicatesFound: number;
  merged: number;
  skipped: number;
  errors: number;
  groups: DuplicateGroup[];
}

/**
 * IntelligentDeduplication - Smart duplicate detection and merge
 * OVERCOMES: Data quality issues that waste credits and annoy prospects
 */
export class IntelligentDeduplication {
  private pool: Pool;

  // Similarity thresholds
  private readonly THRESHOLDS = {
    emailExactMatch: 1.0,      // Same email = definite duplicate
    nameCompanyMatch: 0.85,   // Same name + company = likely duplicate
    phoneMatch: 0.95,          // Same phone = likely duplicate
    linkedinMatch: 1.0,        // Same LinkedIn = definite duplicate
    fuzzyNameThreshold: 0.8,   // Levenshtein similarity for names
  };

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * MAIN: Run full deduplication cycle
   * OVERCOMES: "database full of duplicates"
   */
  async runDeduplicationCycle(): Promise<DeduplicationResult> {
    console.log('[Dedup] Starting deduplication cycle...');

    const result: DeduplicationResult = {
      duplicatesFound: 0,
      merged: 0,
      skipped: 0,
      errors: 0,
      groups: [],
    };

    try {
      // 1. Find email duplicates (exact match - highest confidence)
      const emailDupes = await this.findEmailDuplicates();
      result.groups.push(...emailDupes);

      // 2. Find LinkedIn duplicates
      const linkedinDupes = await this.findLinkedInDuplicates();
      result.groups.push(...linkedinDupes);

      // 3. Find name+company duplicates (fuzzy matching)
      const nameCompanyDupes = await this.findNameCompanyDuplicates();
      result.groups.push(...nameCompanyDupes);

      // 4. Find phone duplicates
      const phoneDupes = await this.findPhoneDuplicates();
      result.groups.push(...phoneDupes);

      // Deduplicate the groups themselves (a contact might appear in multiple detection methods)
      const uniqueGroups = this.deduplicateGroups(result.groups);
      result.duplicatesFound = uniqueGroups.reduce((sum, g) => sum + g.duplicates.length, 0);

      // 5. Merge duplicates
      for (const group of uniqueGroups) {
        try {
          await this.mergeGroup(group);
          result.merged += group.duplicates.length;
        } catch (error) {
          console.error(`[Dedup] Error merging group:`, error);
          result.errors++;
        }
      }

      console.log(`[Dedup] Complete: ${result.duplicatesFound} duplicates found, ${result.merged} merged`);
      return result;
    } catch (error) {
      console.error('[Dedup] Error in deduplication cycle:', error);
      throw error;
    }
  }

  /**
   * Find exact email duplicates
   * Highest confidence - same email is definitely same person
   */
  private async findEmailDuplicates(): Promise<DuplicateGroup[]> {
    const groups: DuplicateGroup[] = [];

    const result = await this.pool.query(`
      SELECT email, array_agg(id ORDER BY created_at ASC) as ids
      FROM sales_contacts
      WHERE email IS NOT NULL
        AND status NOT IN ('deleted', 'merged')
      GROUP BY LOWER(TRIM(email))
      HAVING COUNT(*) > 1
      LIMIT 100
    `);

    for (const row of result.rows) {
      const contacts = await this.getContactsByIds(row.ids);
      if (contacts.length > 1) {
        groups.push({
          primary: this.selectPrimary(contacts),
          duplicates: contacts.slice(1),
          mergeStrategy: {
            keepFields: ['email', 'id'],
            mergeFields: ['title', 'phone', 'linkedin_url', 'notes'],
            sourcePreference: 'most_enriched',
          },
          confidence: 1.0,
        });
      }
    }

    return groups;
  }

  /**
   * Find LinkedIn duplicates
   * Also very high confidence
   */
  private async findLinkedInDuplicates(): Promise<DuplicateGroup[]> {
    const groups: DuplicateGroup[] = [];

    const result = await this.pool.query(`
      SELECT 
        REGEXP_REPLACE(LOWER(linkedin_url), '/+$', '') as normalized_linkedin,
        array_agg(id ORDER BY created_at ASC) as ids
      FROM sales_contacts
      WHERE linkedin_url IS NOT NULL
        AND linkedin_url != ''
        AND status NOT IN ('deleted', 'merged')
      GROUP BY REGEXP_REPLACE(LOWER(linkedin_url), '/+$', '')
      HAVING COUNT(*) > 1
      LIMIT 100
    `);

    for (const row of result.rows) {
      const contacts = await this.getContactsByIds(row.ids);
      if (contacts.length > 1) {
        groups.push({
          primary: this.selectPrimary(contacts),
          duplicates: contacts.slice(1),
          mergeStrategy: {
            keepFields: ['linkedin_url', 'id'],
            mergeFields: ['email', 'title', 'phone'],
            sourcePreference: 'most_enriched',
          },
          confidence: 1.0,
        });
      }
    }

    return groups;
  }

  /**
   * Find name + company duplicates
   * Uses fuzzy matching for typos, different formats
   */
  private async findNameCompanyDuplicates(): Promise<DuplicateGroup[]> {
    const groups: DuplicateGroup[] = [];

    // Get contacts grouped by company
    const result = await this.pool.query(`
      SELECT 
        c.company_id,
        comp.name as company_name,
        array_agg(c.id) as contact_ids
      FROM sales_contacts c
      JOIN sales_companies comp ON c.company_id = comp.id
      WHERE c.first_name IS NOT NULL 
        AND c.last_name IS NOT NULL
        AND c.status NOT IN ('deleted', 'merged')
      GROUP BY c.company_id, comp.name
      HAVING COUNT(*) > 1
      LIMIT 50
    `);

    for (const row of result.rows) {
      const contacts = await this.getContactsByIds(row.contact_ids);
      
      // Within each company, find name matches
      const nameDupes = this.findFuzzyNameMatches(contacts);
      
      for (const group of nameDupes) {
        if (group.duplicates.length > 0) {
          groups.push({
            ...group,
            mergeStrategy: {
              keepFields: ['first_name', 'last_name', 'company_id'],
              mergeFields: ['email', 'title', 'phone', 'linkedin_url'],
              sourcePreference: 'most_enriched',
            },
          });
        }
      }
    }

    return groups;
  }

  /**
   * Find phone number duplicates
   */
  private async findPhoneDuplicates(): Promise<DuplicateGroup[]> {
    const groups: DuplicateGroup[] = [];

    // Normalize phone numbers for comparison
    const result = await this.pool.query(`
      SELECT 
        REGEXP_REPLACE(phone, '[^0-9]', '', 'g') as normalized_phone,
        array_agg(id ORDER BY created_at ASC) as ids
      FROM sales_contacts
      WHERE phone IS NOT NULL
        AND LENGTH(REGEXP_REPLACE(phone, '[^0-9]', '', 'g')) >= 10
        AND status NOT IN ('deleted', 'merged')
      GROUP BY REGEXP_REPLACE(phone, '[^0-9]', '', 'g')
      HAVING COUNT(*) > 1
      LIMIT 100
    `);

    for (const row of result.rows) {
      const contacts = await this.getContactsByIds(row.ids);
      if (contacts.length > 1) {
        groups.push({
          primary: this.selectPrimary(contacts),
          duplicates: contacts.slice(1),
          mergeStrategy: {
            keepFields: ['phone', 'id'],
            mergeFields: ['email', 'title', 'linkedin_url'],
            sourcePreference: 'most_enriched',
          },
          confidence: 0.95,
        });
      }
    }

    return groups;
  }

  /**
   * Find fuzzy name matches within a group
   */
  private findFuzzyNameMatches(contacts: ContactRecord[]): DuplicateGroup[] {
    const groups: DuplicateGroup[] = [];
    const processed = new Set<string>();

    for (let i = 0; i < contacts.length; i++) {
      if (processed.has(contacts[i].id)) continue;

      const duplicates: ContactRecord[] = [];
      const name1 = this.normalizeName(contacts[i].first_name, contacts[i].last_name);

      for (let j = i + 1; j < contacts.length; j++) {
        if (processed.has(contacts[j].id)) continue;

        const name2 = this.normalizeName(contacts[j].first_name, contacts[j].last_name);
        const similarity = this.calculateNameSimilarity(name1, name2);

        if (similarity >= this.THRESHOLDS.fuzzyNameThreshold) {
          duplicates.push(contacts[j]);
          processed.add(contacts[j].id);
        }
      }

      if (duplicates.length > 0) {
        processed.add(contacts[i].id);
        groups.push({
          primary: contacts[i],
          duplicates,
          mergeStrategy: {
            keepFields: ['first_name', 'last_name'],
            mergeFields: ['email', 'title', 'phone'],
            sourcePreference: 'most_enriched',
          },
          confidence: this.THRESHOLDS.fuzzyNameThreshold,
        });
      }
    }

    return groups;
  }

  /**
   * Normalize name for comparison
   */
  private normalizeName(firstName: string | null, lastName: string | null): string {
    const first = (firstName || '').toLowerCase().trim();
    const last = (lastName || '').toLowerCase().trim();
    return `${first} ${last}`.trim();
  }

  /**
   * Calculate name similarity using Levenshtein distance
   */
  private calculateNameSimilarity(name1: string, name2: string): number {
    if (name1 === name2) return 1.0;
    if (!name1 || !name2) return 0;

    const len1 = name1.length;
    const len2 = name2.length;
    const maxLen = Math.max(len1, len2);

    if (maxLen === 0) return 1.0;

    const distance = this.levenshteinDistance(name1, name2);
    return 1 - (distance / maxLen);
  }

  /**
   * Levenshtein distance implementation
   */
  private levenshteinDistance(str1: string, str2: string): number {
    const m = str1.length;
    const n = str2.length;

    const dp: number[][] = Array(m + 1).fill(null).map(() => Array(n + 1).fill(0));

    for (let i = 0; i <= m; i++) dp[i][0] = i;
    for (let j = 0; j <= n; j++) dp[0][j] = j;

    for (let i = 1; i <= m; i++) {
      for (let j = 1; j <= n; j++) {
        if (str1[i - 1] === str2[j - 1]) {
          dp[i][j] = dp[i - 1][j - 1];
        } else {
          dp[i][j] = 1 + Math.min(
            dp[i - 1][j],     // deletion
            dp[i][j - 1],     // insertion
            dp[i - 1][j - 1]  // substitution
          );
        }
      }
    }

    return dp[m][n];
  }

  /**
   * Get contacts by IDs
   */
  private async getContactsByIds(ids: string[]): Promise<ContactRecord[]> {
    const result = await this.pool.query(`
      SELECT 
        c.id, c.email, c.first_name, c.last_name, 
        c.company_id, comp.name as company_name,
        c.title, c.phone, c.linkedin_url, c.created_at,
        COALESCE(c.enrichment_score, 0) as enrichment_score,
        (SELECT COUNT(*) FROM outreach_emails WHERE contact_id = c.id AND status = 'sent') as emails_sent,
        EXISTS(SELECT 1 FROM sales_activities WHERE contact_id = c.id AND activity_type = 'email_replied') as replied
      FROM sales_contacts c
      LEFT JOIN sales_companies comp ON c.company_id = comp.id
      WHERE c.id = ANY($1)
      ORDER BY c.created_at ASC
    `, [ids]);

    return result.rows;
  }

  /**
   * Select which contact should be the primary (survivor)
   * OVERCOMES: "merging loses important data"
   */
  private selectPrimary(contacts: ContactRecord[]): ContactRecord {
    // Priority: 1) Has replies, 2) Most enriched, 3) Most emails sent, 4) Oldest
    const sorted = [...contacts].sort((a, b) => {
      // Replied contacts are most valuable
      if (a.replied && !b.replied) return -1;
      if (!a.replied && b.replied) return 1;

      // Higher enrichment score
      if (a.enrichment_score !== b.enrichment_score) {
        return b.enrichment_score - a.enrichment_score;
      }

      // More emails sent (more engagement history)
      if (a.emails_sent !== b.emails_sent) {
        return b.emails_sent - a.emails_sent;
      }

      // Oldest record (original)
      return new Date(a.created_at).getTime() - new Date(b.created_at).getTime();
    });

    return sorted[0];
  }

  /**
   * Deduplicate the duplicate groups themselves
   */
  private deduplicateGroups(groups: DuplicateGroup[]): DuplicateGroup[] {
    const seenContacts = new Set<string>();
    const uniqueGroups: DuplicateGroup[] = [];

    for (const group of groups) {
      if (seenContacts.has(group.primary.id)) continue;

      // Filter out already-seen duplicates
      const newDuplicates = group.duplicates.filter(d => !seenContacts.has(d.id));
      
      if (newDuplicates.length > 0) {
        seenContacts.add(group.primary.id);
        newDuplicates.forEach(d => seenContacts.add(d.id));
        
        uniqueGroups.push({
          ...group,
          duplicates: newDuplicates,
        });
      }
    }

    return uniqueGroups;
  }

  /**
   * Merge a duplicate group
   * OVERCOMES: "merge loses data", "wrong record kept"
   */
  private async mergeGroup(group: DuplicateGroup): Promise<void> {
    const client = await this.pool.connect();
    
    try {
      await client.query('BEGIN');

      const primary = group.primary;
      const duplicateIds = group.duplicates.map(d => d.id);

      // 1. Merge data into primary (fill in blanks)
      const mergeFields = group.mergeStrategy.mergeFields;
      
      for (const dup of group.duplicates) {
        for (const field of mergeFields) {
          const primaryValue = (primary as any)[field];
          const dupValue = (dup as any)[field];
          
          // If primary is missing this field but duplicate has it, update
          if (!primaryValue && dupValue) {
            await client.query(`
              UPDATE sales_contacts 
              SET ${field} = $1, updated_at = CURRENT_TIMESTAMP
              WHERE id = $2
            `, [dupValue, primary.id]);
          }
        }
      }

      // 2. Move related records to primary
      // Move emails
      await client.query(`
        UPDATE outreach_emails 
        SET contact_id = $1, updated_at = CURRENT_TIMESTAMP
        WHERE contact_id = ANY($2)
      `, [primary.id, duplicateIds]);

      // Move activities
      await client.query(`
        UPDATE sales_activities 
        SET contact_id = $1
        WHERE contact_id = ANY($2)
      `, [primary.id, duplicateIds]);

      // Move enrollments (be careful with active ones)
      await client.query(`
        UPDATE outreach_enrollments 
        SET contact_id = $1, updated_at = CURRENT_TIMESTAMP
        WHERE contact_id = ANY($2)
          AND contact_id != $1
      `, [primary.id, duplicateIds]);

      // 3. Mark duplicates as merged
      await client.query(`
        UPDATE sales_contacts 
        SET status = 'merged',
            merged_into_id = $1,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ANY($2)
      `, [primary.id, duplicateIds]);

      // 4. Log the merge
      await client.query(`
        INSERT INTO sales_activities (contact_id, activity_type, metadata)
        VALUES ($1, 'contacts_merged', $2)
      `, [primary.id, JSON.stringify({
        merged_contacts: duplicateIds,
        confidence: group.confidence,
        merge_strategy: group.mergeStrategy,
      })]);

      await client.query('COMMIT');
      
      console.log(`[Dedup] Merged ${duplicateIds.length} contacts into ${primary.id} (${primary.email})`);
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Check if a new contact is a duplicate before inserting
   * OVERCOMES: "duplicates keep getting added"
   */
  async checkForDuplicateBeforeInsert(email: string, firstName?: string, lastName?: string, phone?: string, linkedinUrl?: string): Promise<{
    isDuplicate: boolean;
    existingContactId?: string;
    confidence: number;
    matchType?: string;
  }> {
    // 1. Check email (exact)
    if (email) {
      const emailMatch = await this.pool.query(`
        SELECT id FROM sales_contacts 
        WHERE LOWER(TRIM(email)) = LOWER(TRIM($1))
          AND status NOT IN ('deleted', 'merged')
        LIMIT 1
      `, [email]);

      if (emailMatch.rows[0]) {
        return {
          isDuplicate: true,
          existingContactId: emailMatch.rows[0].id,
          confidence: 1.0,
          matchType: 'email',
        };
      }
    }

    // 2. Check LinkedIn (exact)
    if (linkedinUrl) {
      const linkedinMatch = await this.pool.query(`
        SELECT id FROM sales_contacts 
        WHERE LOWER(REGEXP_REPLACE(linkedin_url, '/+$', '')) = LOWER(REGEXP_REPLACE($1, '/+$', ''))
          AND status NOT IN ('deleted', 'merged')
        LIMIT 1
      `, [linkedinUrl]);

      if (linkedinMatch.rows[0]) {
        return {
          isDuplicate: true,
          existingContactId: linkedinMatch.rows[0].id,
          confidence: 1.0,
          matchType: 'linkedin',
        };
      }
    }

    // 3. Check phone (normalized)
    if (phone && phone.replace(/\D/g, '').length >= 10) {
      const normalizedPhone = phone.replace(/\D/g, '');
      const phoneMatch = await this.pool.query(`
        SELECT id FROM sales_contacts 
        WHERE REGEXP_REPLACE(phone, '[^0-9]', '', 'g') = $1
          AND status NOT IN ('deleted', 'merged')
        LIMIT 1
      `, [normalizedPhone]);

      if (phoneMatch.rows[0]) {
        return {
          isDuplicate: true,
          existingContactId: phoneMatch.rows[0].id,
          confidence: 0.95,
          matchType: 'phone',
        };
      }
    }

    return { isDuplicate: false, confidence: 0 };
  }

  /**
   * Get deduplication statistics
   */
  async getDeduplicationStats(): Promise<{
    totalContacts: number;
    potentialDuplicates: number;
    mergedContacts: number;
    duplicatesByType: Record<string, number>;
  }> {
    const result = await this.pool.query(`
      SELECT 
        (SELECT COUNT(*) FROM sales_contacts WHERE status NOT IN ('deleted', 'merged')) as total,
        (SELECT COUNT(*) FROM sales_contacts WHERE status = 'merged') as merged,
        (SELECT COUNT(*) FROM (
          SELECT email FROM sales_contacts 
          WHERE email IS NOT NULL AND status NOT IN ('deleted', 'merged')
          GROUP BY LOWER(TRIM(email)) HAVING COUNT(*) > 1
        ) as email_dupes) as email_duplicates,
        (SELECT COUNT(*) FROM (
          SELECT REGEXP_REPLACE(phone, '[^0-9]', '', 'g') FROM sales_contacts 
          WHERE phone IS NOT NULL AND LENGTH(REGEXP_REPLACE(phone, '[^0-9]', '', 'g')) >= 10
            AND status NOT IN ('deleted', 'merged')
          GROUP BY REGEXP_REPLACE(phone, '[^0-9]', '', 'g') HAVING COUNT(*) > 1
        ) as phone_dupes) as phone_duplicates
    `);

    const stats = result.rows[0];
    return {
      totalContacts: parseInt(stats.total) || 0,
      mergedContacts: parseInt(stats.merged) || 0,
      potentialDuplicates: (parseInt(stats.email_duplicates) || 0) + (parseInt(stats.phone_duplicates) || 0),
      duplicatesByType: {
        email: parseInt(stats.email_duplicates) || 0,
        phone: parseInt(stats.phone_duplicates) || 0,
      },
    };
  }
}
