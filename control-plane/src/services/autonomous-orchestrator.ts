/**
 * AUTONOMOUS SALES ORCHESTRATOR
 * 
 * Coordinates all autonomous sales modules:
 *   - Hunter: Discovers leads from app stores
 *   - Analyst: Scores leads with heuristics
 *   - Writer (Spintax): Generates personalized messages
 *   - Optimizer (Thompson): Optimizes subject lines
 *   - Clerk: Classifies inbox responses
 * 
 * This is the main entry point for autonomous sales operations.
 * Designed to run as background jobs on the test server.
 */

import { queryOperator, queryReadOnly } from '../database';
import { logger } from '../logger';

// Import modules
import { Hunter, getSourcesDueForScrape } from './discovery/hunter';
import { Analyst, calculateLeadScore, getContactsNeedingScoring } from './discovery/analyst';
import { SpintaxEngine, getTemplates, recordGeneration, type PersonalizationContext } from './messaging/spintax-engine';
import { sampleAndSelect, recordOutcome, getBanditStats } from './math/thompson-sampler';
import { Clerk, initializeClerkFromDatabase, saveMessageClassification, saveClassifierModel, getClassificationStats, submitReview, loadTrainingData, type MessageCategory } from './inbox/clerk';
import { getEdgeWorkerClient, isEdgeWorkerAvailable } from './edge-worker-client';
import { IcebreakerEngine, batchGenerateIcebreakers } from './messaging/icebreaker-engine';

// ============================================================================
// TYPES
// ============================================================================

export interface AutonomousJobConfig {
  jobType: 'discover' | 'score' | 'compose' | 'classify' | 'enrich' | 'train' | 'icebreaker';
  batchSize?: number;
  dryRun?: boolean;
}

export interface JobResult {
  jobId: string;
  jobType: string;
  success: boolean;
  processed: number;
  errors: number;
  duration: number;
  details?: Record<string, unknown>;
}

// ============================================================================
// MAIN ORCHESTRATOR CLASS
// ============================================================================

export class AutonomousSalesOrchestrator {
  private hunter: Hunter;
  private analyst: Analyst;
  private spintaxEngine: SpintaxEngine;
  private clerk: Clerk | null = null;
  private schemaValidated = false;
  
  // Email validation regex - RFC 5322 simplified
  private static readonly EMAIL_REGEX = /^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+$/;
  
  constructor() {
    this.hunter = new Hunter({ headless: true, requestDelayMs: 2000 });
    this.analyst = new Analyst();
    this.spintaxEngine = new SpintaxEngine({ minDistance: 50 });
  }
  
  /**
   * Validate and clean an email address
   * 
   * SECURITY: This is a critical validation function for the sales pipeline.
   * Bad emails can damage sender reputation, trigger spam filters, and waste resources.
   * 
   * Known malformed patterns this catches:
   *   - "hello@amanotes.comterms" → text after TLD (REJECT)
   *   - "Pageshopsupport@zigzagame.netzigzagame.comPhotos" → multi-TLD garbage (REJECT)
   *   - "605a7baede844d278b89dc95ae0a9123@sentry..." → UUID tracking IDs (REJECT)
   *   - "hello@apexmediation.ee" → our own domain (REJECT)
   *   - "carlos@test.local" → test domains (REJECT)
   *   - "noreply@company.com" → no-reply addresses (REJECT)
   * 
   * Returns cleaned email or null if invalid
   */
  private validateAndCleanEmail(rawEmail: string): string | null {
    if (!rawEmail || typeof rawEmail !== 'string') return null;
    
    // Trim whitespace
    let email = rawEmail.trim().toLowerCase();
    
    // Skip obviously invalid
    if (email.length < 5 || email.length > 254) return null;
    if (!email.includes('@')) return null;
    
    // Split into local and domain parts
    const atIndex = email.indexOf('@');
    const localPart = email.substring(0, atIndex);
    let domain = email.substring(atIndex + 1);
    
    // =========================================================================
    // LOCAL PART VALIDATION (before @)
    // =========================================================================
    
    // Reject UUID-like usernames (Sentry tracking IDs, analytics garbage)
    // Pattern: 32 hex chars or UUID format (with or without dashes)
    const uuidPattern = /^[0-9a-f]{32}$|^[0-9a-f]{8}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{12}$/i;
    if (uuidPattern.test(localPart)) {
      logger.debug('Email rejected: UUID tracking ID', { email: rawEmail });
      return null;
    }
    
    // Reject extremely long local parts (likely scraped garbage)
    if (localPart.length > 64) {
      logger.debug('Email rejected: local part too long', { email: rawEmail });
      return null;
    }
    
    // Reject no-reply patterns
    const noReplyPatterns = /^(noreply|no-reply|donotreply|do-not-reply|mailer-daemon|postmaster|nobody|bounce)$/i;
    if (noReplyPatterns.test(localPart)) {
      logger.debug('Email rejected: no-reply pattern', { email: rawEmail });
      return null;
    }
    
    // =========================================================================
    // DOMAIN VALIDATION (after @)
    // =========================================================================
    
    // Valid TLDs we accept (common ones only)
    const validTlds = ['com', 'net', 'org', 'io', 'co', 'ee', 'app', 'dev', 'games', 'ai', 
                       'tech', 'xyz', 'info', 'biz', 'us', 'uk', 'de', 'jp', 'cn', 'kr',
                       'ru', 'br', 'au', 'ca', 'fr', 'it', 'es', 'nl', 'pl', 'sg'];
    
    // Check for text AFTER the TLD (e.g., "amanotes.comterms" → reject)
    // Find where the TLD ends and check if there's more text
    const tldPattern = new RegExp(`\\.(${validTlds.join('|')})([a-z0-9].*)$`, 'i');
    const tldMatch = domain.match(tldPattern);
    if (tldMatch && tldMatch[2]) {
      // There's text after the TLD - this is malformed
      // Try to fix by truncating at the TLD
      const cleanedDomain = domain.replace(tldMatch[2], '');
      logger.debug('Email domain had trailing garbage, cleaned', { 
        original: domain, 
        cleaned: cleanedDomain,
        garbage: tldMatch[2]
      });
      domain = cleanedDomain;
    }
    
    // Check for multiple TLDs (e.g., "zigzagame.netzigzagame.com")
    const tldCountPattern = new RegExp(`\\.(${validTlds.join('|')})`, 'gi');
    const tldMatches = domain.match(tldCountPattern);
    if (tldMatches && tldMatches.length > 1) {
      // Multiple TLDs in domain - malformed, try to extract first valid part
      const firstTldIndex = domain.search(tldCountPattern);
      if (firstTldIndex > 0) {
        const firstTld = tldMatches[0];
        domain = domain.substring(0, firstTldIndex + firstTld.length);
        logger.debug('Email had multiple TLDs, truncated', { original: rawEmail, cleanedDomain: domain });
      }
    }
    
    // Rebuild email
    email = `${localPart}@${domain}`;
    
    // Must end with a valid TLD
    const endsWithValidTld = new RegExp(`\\.(${validTlds.join('|')})$`, 'i');
    if (!endsWithValidTld.test(email)) {
      logger.debug('Email rejected: invalid TLD', { email });
      return null;
    }
    
    // Validate domain structure (must have at least one subdomain + TLD)
    if (!domain.includes('.')) {
      logger.debug('Email rejected: domain has no TLD', { email });
      return null;
    }
    
    // Validate with strict regex
    if (!AutonomousSalesOrchestrator.EMAIL_REGEX.test(email)) {
      logger.debug('Email rejected: failed regex', { email });
      return null;
    }
    
    // =========================================================================
    // BLOCKLIST: Known invalid domains and patterns
    // =========================================================================
    const invalidPatterns = [
      // Our own domain - never contact ourselves
      '@apexmediation.ee',
      // Test/development domains
      '.test.local', '.example.com', '.localhost', '.local',
      '.test', '.example', '.invalid',
      // Email service providers (we want business emails, not consumer)
      '@gmail.com', '@yahoo.com', '@hotmail.com', '@outlook.com',
      '@aol.com', '@icloud.com', '@mail.com', '@protonmail.com',
      // Tracking/analytics domains
      '@sentry.io', '@sentry-next.', '@segment.com',
      '@mixpanel.com', '@amplitude.com', '@analytics.',
      // Generic addresses that won't respond
      'support@', 'info@', 'contact@', 'sales@', 'admin@',
      'hello@', 'help@', 'feedback@', 'team@', 'office@',
    ];
    
    for (const pattern of invalidPatterns) {
      if (email.includes(pattern)) {
        logger.debug('Email rejected: blocklist pattern', { email, pattern });
        return null;
      }
    }
    
    // =========================================================================
    // FINAL VALIDATION PASSED
    // =========================================================================
    logger.debug('Email validated successfully', { original: rawEmail, cleaned: email });
    return email;
  }
  
  /**
   * Validate database schema has required columns
   * Fail-fast if schema drift is detected
   */
  private async validateSchema(): Promise<void> {
    if (this.schemaValidated) return;
    
    const requiredColumns: Record<string, string[]> = {
      'sa_scrape_jobs': ['apps_discovered', 'apps_processed', 'status', 'error_message'],
      'sa_contacts': ['score', 'icebreaker', 'company_name'],
      'sa_sources': ['source_type', 'status', 'last_scraped_at'],
    };
    
    for (const [table, columns] of Object.entries(requiredColumns)) {
      const result = await queryReadOnly<{ column_name: string }>(`
        SELECT column_name 
        FROM information_schema.columns 
        WHERE table_name = $1
      `, [table]);
      
      const existingColumns = new Set(result.map(r => r.column_name));
      const missingColumns = columns.filter(c => !existingColumns.has(c));
      
      if (missingColumns.length > 0) {
        const errorMsg = `Schema drift detected: table '${table}' missing columns: ${missingColumns.join(', ')}`;
        logger.error(errorMsg);
        throw new Error(errorMsg);
      }
    }
    
    this.schemaValidated = true;
    logger.info('Database schema validation passed');
  }
  
  /**
   * Initialize the orchestrator (validate schema + load clerk from DB)
   */
  async initialize(): Promise<void> {
    // Validate schema FIRST - fail fast if drift detected
    await this.validateSchema();
    
    this.clerk = await initializeClerkFromDatabase();
    logger.info('Autonomous Sales Orchestrator initialized');
  }
  
  /**
   * Run a discovery job (Hunter)
   */
  async runDiscoveryJob(batchSize = 3): Promise<JobResult> {
    const startTime = Date.now();
    const jobId = await this.createJob('discover');
    
    try {
      await this.updateJobStatus(jobId, 'running');
      
      const sources = await getSourcesDueForScrape();
      const sourcesToProcess = sources.slice(0, batchSize);
      
      let totalApps = 0;
      let totalInserted = 0;
      let errors = 0;
      
      for (const source of sourcesToProcess) {
        try {
          const result = await this.hunter.runJob(source.id);
          totalApps += result.appsFound;
          totalInserted += result.inserted;
          
          // Mark source as scraped
          await queryOperator(`
            UPDATE sa_sources SET last_scraped_at = CURRENT_TIMESTAMP WHERE id = $1
          `, [source.id]);
          
        } catch (err) {
          logger.error('Discovery job source failed', {
            sourceId: source.id,
            error: (err as Error).message,
          });
          errors++;
        }
      }
      
      await this.updateJobStatus(jobId, 'completed', {
        appsFound: totalApps,
        appsInserted: totalInserted,
        sourcesProcessed: sourcesToProcess.length,
      });
      
      return {
        jobId,
        jobType: 'discover',
        success: true,
        processed: totalInserted,
        errors,
        duration: Date.now() - startTime,
        details: { totalApps, sourcesProcessed: sourcesToProcess.length },
      };
      
    } catch (err) {
      await this.updateJobStatus(jobId, 'failed', {
        error: (err as Error).message,
      });
      throw err;
    }
  }
  
  /**
   * Run a scoring job (Analyst)
   */
  async runScoringJob(batchSize = 50): Promise<JobResult> {
    const startTime = Date.now();
    const jobId = await this.createJob('score');
    
    try {
      await this.updateJobStatus(jobId, 'running');
      
      const result = await this.analyst.scoreBatch(batchSize);
      
      await this.updateJobStatus(jobId, 'completed', {
        scored: result.scored,
        byTier: result.byTier,
      });
      
      return {
        jobId,
        jobType: 'score',
        success: true,
        processed: result.scored,
        errors: 0,
        duration: Date.now() - startTime,
        details: { byTier: result.byTier },
      };
      
    } catch (err) {
      await this.updateJobStatus(jobId, 'failed', {
        error: (err as Error).message,
      });
      throw err;
    }
  }
  
  /**
   * Run an enrichment job (Email Discovery)
   * 
   * Finds contacts with websites but no emails and enriches them
   * using the Edge Worker's company enrichment endpoint.
 * 
 * FIX: Added robust email validation to prevent malformed emails.
 */
async runEnrichmentJob(batchSize = 20): Promise<JobResult> {
    const startTime = Date.now();
    const jobId = await this.createJob('enrich');
    
    try {
      await this.updateJobStatus(jobId, 'running');
      
      // Check if Edge Worker is available
      if (!isEdgeWorkerAvailable()) {
        throw new Error('Edge Worker not available for enrichment');
      }
      
      const edgeWorker = getEdgeWorkerClient();
      
      // Get contacts with websites but no emails
      const contactsToEnrich = await queryReadOnly<{
        id: string;
        company_name: string | null;
        website: string | null;
      }>(`
        SELECT id, company_name, website
        FROM sa_contacts
        WHERE email IS NULL 
          AND website IS NOT NULL
          AND website != ''
        ORDER BY score DESC, created_at DESC
        LIMIT $1
      `, [batchSize]);
      
      logger.info('Starting enrichment job', { 
        contactsToEnrich: contactsToEnrich.length,
        batchSize,
      });
      
      let enriched = 0;
      let errors = 0;
      let skippedInvalid = 0;
      const enrichedEmails: string[] = [];
      
      for (const contact of contactsToEnrich) {
        try {
          const result = await edgeWorker.enrichCompany({
            name: contact.company_name || 'Unknown',
            website: contact.website || undefined,
          });
          
          if (result.success && result.data?.emails && result.data.emails.length > 0) {
            // FIX: Validate and clean emails before saving
            let validEmail: string | null = null;
            
            for (const rawEmail of result.data.emails) {
              const cleanedEmail = this.validateAndCleanEmail(rawEmail);
              if (cleanedEmail) {
                validEmail = cleanedEmail;
                break;
              }
            }
            
            if (validEmail) {
              await queryOperator(`
                UPDATE sa_contacts 
                SET email = $1, updated_at = CURRENT_TIMESTAMP
                WHERE id = $2
              `, [validEmail, contact.id]);
              
              enrichedEmails.push(validEmail);
              enriched++;
              
              logger.debug('Enriched contact with email', {
                contactId: contact.id,
                email: validEmail,
                totalEmailsFound: result.data.emails.length,
              });
            } else {
              skippedInvalid++;
              logger.warn('Skipped malformed emails from enrichment', {
                contactId: contact.id,
                rawEmails: result.data.emails.slice(0, 3), // Log first 3 for debugging
              });
            }
          } else {
            logger.debug('No emails found for contact', {
              contactId: contact.id,
              website: contact.website,
              error: result.error,
            });
          }
        } catch (err) {
          logger.error('Failed to enrich contact', {
            contactId: contact.id,
            error: (err as Error).message,
          });
          errors++;
        }
      }
      
      await this.updateJobStatus(jobId, 'completed', {
        enriched,
        totalProcessed: contactsToEnrich.length,
        errors,
        skippedInvalid,
        emailsFound: enrichedEmails.length,
      });
      
      logger.info('Enrichment job completed', {
        enriched,
        errors,
        skippedInvalid,
      });
      
      return {
        jobId,
        jobType: 'enrich',
        success: true,
        processed: enriched,
        errors,
        duration: Date.now() - startTime,
        details: { 
          enriched,
          totalProcessed: contactsToEnrich.length,
          emailsFound: enrichedEmails,
          skippedInvalid,
        },
      };
      
    } catch (err) {
      await this.updateJobStatus(jobId, 'failed', {
        error: (err as Error).message,
      });
      throw err;
    }
  }
  
  /**
   * Run an icebreaker generation job
   * 
   * CRITICAL: This is the "Brain Wiring" fix - connects the IcebreakerEngine
   * to the autonomous pipeline so contacts get personalized icebreakers
   * before the compose step tries to use them.
   * 
   * Pipeline order should be: Discover → Score → Icebreaker → Compose
   * 
   * Finds contacts with app_name but no icebreaker and generates one
   * using the Edge Worker LLM or template fallback.
   */
  async runIcebreakerJob(batchSize = 30): Promise<JobResult> {
    const startTime = Date.now();
    const jobId = await this.createJob('icebreaker');
    
    try {
      await this.updateJobStatus(jobId, 'running');
      
      logger.info('Starting icebreaker generation job', { batchSize });
      
      // Use the IcebreakerEngine's batch generator
      const result = await batchGenerateIcebreakers(batchSize);
      
      await this.updateJobStatus(jobId, 'completed', {
        processed: result.processed,
        llmGenerated: result.llmGenerated,
        templateGenerated: result.templateGenerated,
        errors: result.errors,
      });
      
      logger.info('Icebreaker job completed', {
        processed: result.processed,
        llmGenerated: result.llmGenerated,
        templateGenerated: result.templateGenerated,
        errors: result.errors,
      });
      
      return {
        jobId,
        jobType: 'icebreaker',
        success: true,
        processed: result.processed,
        errors: result.errors,
        duration: Date.now() - startTime,
        details: {
          llmGenerated: result.llmGenerated,
          templateGenerated: result.templateGenerated,
        },
      };
      
    } catch (err) {
      await this.updateJobStatus(jobId, 'failed', {
        error: (err as Error).message,
      });
      throw err;
    }
  }
  
  /**
   * Run a classifier training job
   * 
   * Loads all training data from the database and retrains the classifier.
   * This creates a closed-loop learning system where human reviews improve
   * the classifier over time.
   */
  async runTrainingJob(): Promise<JobResult> {
    const startTime = Date.now();
    const jobId = await this.createJob('train');
    
    try {
      await this.updateJobStatus(jobId, 'running');
      
      // Load all training data from database
      const trainingData = await loadTrainingData();
      
      if (trainingData.length === 0) {
        logger.info('No training data available');
        await this.updateJobStatus(jobId, 'completed', {
          trained: 0,
          message: 'No training data available',
        });
        
        return {
          jobId,
          jobType: 'train',
          success: true,
          processed: 0,
          errors: 0,
          duration: Date.now() - startTime,
          details: { message: 'No training data available' },
        };
      }
      
      // Get the previous model stats
      const prevStats = this.clerk?.getStats() || { totalDocuments: 0 };
      
      // Reinitialize clerk to reset the classifier
      this.clerk = await initializeClerkFromDatabase();
      
      // Train on all human-reviewed data
      let trained = 0;
      for (const item of trainingData) {
        try {
          this.clerk.learn(item.text, item.category);
          trained++;
        } catch (err) {
          logger.error('Failed to train on item', { 
            category: item.category,
            error: (err as Error).message,
          });
        }
      }
      
      // Persist the updated model
      await saveClassifierModel(this.clerk);
      
      const newStats = this.clerk.getStats();
      
      await this.updateJobStatus(jobId, 'completed', {
        trained,
        totalDocuments: newStats.totalDocuments,
        vocabularySize: newStats.vocabularySize,
        prevDocuments: prevStats.totalDocuments,
      });
      
      logger.info('Classifier training completed', {
        trained,
        totalDocuments: newStats.totalDocuments,
        vocabularySize: newStats.vocabularySize,
      });
      
      return {
        jobId,
        jobType: 'train',
        success: true,
        processed: trained,
        errors: 0,
        duration: Date.now() - startTime,
        details: {
          trained,
          totalDocuments: newStats.totalDocuments,
          vocabularySize: newStats.vocabularySize,
          categoryCounts: newStats.categoryCounts,
        },
      };
      
    } catch (err) {
      await this.updateJobStatus(jobId, 'failed', {
        error: (err as Error).message,
      });
      throw err;
    }
  }
  
  /**
   * Compose a personalized message for a contact
   * 
   * QUALITY GATES:
   * - Contact must have email
   * - Contact must have valid company_name
   * - Contact must have app_name OR icebreaker
   * - Score must be >= threshold (default 50)
   */
  async composeMessage(contactId: string, options: { minScore?: number } = {}): Promise<{
    subject: string;
    body: string;
    templateId: string;
    subjectArmId: string;
    variationHash: string;
  } | null> {
    const minScore = options.minScore ?? 50;
    
    // Get contact details with QUALITY GATES
    // FIXED (Jan 2026): Added lift_percent and lift_range for {{LIFT_PERCENT}} variable support
    const contacts = await queryReadOnly<{
      id: string;
      first_name: string | null;
      last_name: string | null;
      company_name: string;
      app_name: string | null;
      app_genre: string | null;
      icebreaker: string | null;
      email: string | null;
      score: number | null;
      lift_percent: number | null;
      lift_range: string | null;
    }>(`
      SELECT id, first_name, last_name, company_name, app_name, app_genre, 
             icebreaker, email, score, lift_percent, lift_range
      FROM sa_contacts WHERE id = $1
    `, [contactId]);
    
    if (contacts.length === 0) {
      logger.warn('Contact not found', { contactId });
      return null;
    }
    const contact = contacts[0];
    
    // ====== QUALITY GATE 1: Must have email ======
    if (!contact.email || contact.email.trim() === '') {
      logger.warn('Quality gate failed: no email', { contactId });
      return null;
    }
    
    // ====== QUALITY GATE 2: Must have valid company name ======
    if (!contact.company_name || contact.company_name.trim() === '' || 
        contact.company_name.toLowerCase() === 'unknown') {
      logger.warn('Quality gate failed: invalid company name', { 
        contactId, 
        companyName: contact.company_name 
      });
      return null;
    }
    
    // ====== QUALITY GATE 3: Must have personalization data ======
    const hasAppName = contact.app_name && contact.app_name.trim() !== '';
    const hasIcebreaker = contact.icebreaker && contact.icebreaker.trim() !== '';
    
    if (!hasAppName && !hasIcebreaker) {
      logger.warn('Quality gate failed: no personalization data', { 
        contactId, 
        hasAppName, 
        hasIcebreaker 
      });
      return null;
    }
    
    // ====== QUALITY GATE 4: Score threshold ======
    const score = contact.score ?? 0;
    if (score < minScore) {
      logger.warn('Quality gate failed: score below threshold', { 
        contactId, 
        score, 
        minScore 
      });
      return null;
    }
    
    logger.debug('Quality gates passed', { 
      contactId, 
      email: contact.email,
      score,
      hasAppName,
      hasIcebreaker 
    });
    
    // Get appropriate template (initial outreach)
    const templates = await getTemplates('initial_outreach');
    if (templates.length === 0) {
      logger.warn('No templates available for initial outreach');
      return null;
    }
    const template = templates[0];
    
    // Select subject line using Thompson Sampling
    const subjectArm = await sampleAndSelect('subject_line');
    if (!subjectArm) {
      logger.warn('No subject line arms available');
      return null;
    }
    
    // Build personalization context
    // FIXED (Jan 2026): Include lift_percent and lift_range for Truthful Puffery in emails
    const context: PersonalizationContext = {
      firstName: contact.first_name || undefined,
      lastName: contact.last_name || undefined,
      company: contact.company_name,
      appName: contact.app_name || undefined,
      appGenre: contact.app_genre || undefined,
      icebreaker: contact.icebreaker || undefined,
      liftPercent: contact.lift_percent || undefined,
      liftRange: contact.lift_range || undefined,
    };
    
    // Generate unique message with HTML-safe personalization
    // FIXED: Use generateUniqueHtml() instead of generateUnique() to prevent XSS
    // and ensure special characters like <, >, & display correctly in emails
    const result = this.spintaxEngine.generateUniqueHtml(
      subjectArm.armValue, // Use Thompson-selected subject
      template.bodySpintax,
      context
    );
    
    // ====== QUALITY GATE 5: Validate no unresolved placeholders ======
    // Check for common unresolved placeholder patterns
    // CRITICAL: This catches bugs where personalization fails silently
    //
    // FIXED (Jan 2026): Removed '/your app|your company/gi' pattern that was conflicting
    // with legitimate fallbacks. The spintax-engine.ts now uses empty strings instead
    // of 'your company' fallbacks, so this pattern is no longer needed.
    const unresolvedPatterns = [
      /\{[A-Z_]+\}/g,                    // {PLACEHOLDER} uppercase
      /\{[a-z_]+\}/g,                    // {placeholder} lowercase
      /\{\{[A-Z_]+\}\}/g,                // {{PLACEHOLDER}} double-brace uppercase
      /\{\{[a-z_]+\}\}/g,                // {{placeholder}} double-brace lowercase
      /\b(APP_NAME|APP_GENRE|COMPANY|FIRST_NAME|ICEBREAKER)\b/g,  // Unresolved var names as standalone words
      /\[\[.+?\]\]/g,                    // [[placeholder]]
      /%%[a-z_]+%%/gi,                   // %%placeholder%%
      // REMOVED: /your app|your company/gi - these were causing false positives
      // The quality gate now relies on empty string detection instead
    ];
    
    // Check for unresolved placeholders using non-global regex to avoid state issues
    const subjectHasUnresolved = unresolvedPatterns.some(pattern => {
      // Create a fresh regex without the global flag for testing
      const testPattern = new RegExp(pattern.source, pattern.flags.replace('g', ''));
      return testPattern.test(result.subject);
    });
    const bodyHasUnresolved = unresolvedPatterns.some(pattern => {
      const testPattern = new RegExp(pattern.source, pattern.flags.replace('g', ''));
      return testPattern.test(result.body);
    });
    
    // ====== QUALITY GATE 6: Check for empty critical fields ======
    // FIXED: Check if subject or body is too short (indicates missing personalization)
    const subjectTooShort = result.subject.trim().length < 15;
    const bodyTooShort = result.body.trim().length < 50;
    
    // ====== QUALITY GATE 7: Check subject line for empty app name ======
    // FIXED: Detect subject lines that end abruptly due to missing app name
    // e.g., "Free ad revenue audit for" (missing app name at end)
    const subjectEndsAbruptly = /\s+(for|about|re:|regarding)\s*$/i.test(result.subject);
    
    // ====== QUALITY GATE 8: Mobile subject line length check ======
    // Mobile email clients truncate at ~35 characters - ensure key message is visible
    const subjectTooLongForMobile = result.subject.length > 50;
    if (subjectTooLongForMobile) {
      const mobilePreview = result.subject.substring(0, 32) + '...';
      logger.debug('Subject line will be truncated on mobile', {
        contactId,
        fullSubject: result.subject,
        mobilePreview,
      });
      // Note: This is a warning, not a rejection - long subjects are suboptimal but acceptable
    }
    
    // Additional check: Empty first name with "Hi," followed by nothing or just whitespace
    const hasEmptyGreeting = /^(Hi|Hey|Hello),?\s*\n/i.test(result.body);
    
    if (subjectHasUnresolved || bodyHasUnresolved) {
      logger.warn('Quality gate failed: unresolved placeholders detected', { 
        contactId,
        subjectPreview: result.subject.substring(0, 50),
        subjectHasUnresolved,
        bodyHasUnresolved 
      });
      return null;
    }
    
    if (subjectTooShort || bodyTooShort) {
      logger.warn('Quality gate failed: content too short (missing personalization)', {
        contactId,
        subjectLength: result.subject.trim().length,
        bodyLength: result.body.trim().length,
        subjectTooShort,
        bodyTooShort,
      });
      return null;
    }
    
    if (subjectEndsAbruptly) {
      logger.warn('Quality gate failed: subject line ends abruptly (missing app name)', {
        contactId,
        subject: result.subject,
      });
      return null;
    }
    
    // If body starts with empty greeting and we have no first name AND no icebreaker, reject
    // FIXED: Also check if we're using 'team' fallback which is acceptable
    const hasTeamFallback = /^(Hi|Hey|Hello)\s+team/i.test(result.body);
    if (hasEmptyGreeting && !context.firstName && !context.icebreaker && !hasTeamFallback) {
      logger.warn('Quality gate failed: empty greeting with no personalization', {
        contactId,
        hasFirstName: !!context.firstName,
        hasIcebreaker: !!context.icebreaker,
        hasTeamFallback,
      });
      return null;
    }
    
    // Record the generation
    await recordGeneration({
      templateId: template.id,
      resolvedSubject: result.subject,
      resolvedBody: result.body,
    });
    
    // CLOSED LOOP: Save outbound message with arm ID for tracking
    // This enables the system to learn from responses
    await this.saveOutboundMessage({
      contactId,
      subject: result.subject,
      body: result.body,
      subjectArmId: subjectArm.armId,
      templateId: template.id,
      variationHash: result.variationHash,
    });
    
    return {
      subject: result.subject,
      body: result.body,
      templateId: template.id,
      subjectArmId: subjectArm.armId,
      variationHash: result.variationHash,
    };
  }
  
  /**
   * Save outbound message for closed-loop tracking
   * This is critical for the Thompson Sampler to learn from responses
   */
  private async saveOutboundMessage(params: {
    contactId: string;
    subject: string;
    body: string;
    subjectArmId: string;
    templateId: string;
    variationHash: string;
  }): Promise<string> {
    try {
      const result = await queryOperator<{ id: string }>(`
        INSERT INTO sa_outbound_messages 
        (contact_id, subject, body_text, subject_arm_id, template_id, variation_hash, status)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending')
        RETURNING id
      `, [
        params.contactId,
        params.subject,
        params.body,
        params.subjectArmId,
        params.templateId,
        params.variationHash,
      ]);
      
      logger.debug('Saved outbound message for closed-loop tracking', {
        contactId: params.contactId,
        armId: params.subjectArmId,
      });
      
      return result[0].id;
    } catch (err) {
      // Don't fail the compose if tracking fails
      logger.warn('Failed to save outbound message', { error: (err as Error).message });
      return '';
    }
  }
  
  /**
   * Process email open/reply event (for Thompson Sampler feedback)
   */
  async recordEmailEvent(params: {
    contactId: string;
    eventType: 'open' | 'reply' | 'bounce';
    subjectArmId?: string;
  }): Promise<void> {
    if (!params.subjectArmId) return;
    
    const success = params.eventType === 'open' || params.eventType === 'reply';
    await recordOutcome(params.subjectArmId, success);
    
    logger.info('Recorded email event for Thompson Sampler', {
      contactId: params.contactId,
      eventType: params.eventType,
      success,
    });
  }
  
  /**
   * Classify an incoming email
   */
  async classifyEmail(params: {
    fromEmail: string;
    subject: string;
    body: string;
    contactId?: string;
  }): Promise<{
    category: string;
    confidence: number;
    requiresReview: boolean;
    messageId: string;
  }> {
    if (!this.clerk) {
      await this.initialize();
    }
    
    const result = this.clerk!.classify(params.subject, params.body);
    
    const messageId = await saveMessageClassification({
      contactId: params.contactId,
      fromEmail: params.fromEmail,
      subject: params.subject,
      body: params.body,
      classification: result,
    });
    
    logger.info('Classified incoming email', {
      fromEmail: params.fromEmail,
      category: result.category,
      confidence: result.confidence.toFixed(2),
    });
    
    return {
      category: result.category,
      confidence: result.confidence,
      requiresReview: result.requiresReview,
      messageId,
    };
  }
  
  /**
   * Submit a review for a classified message
   * This trains the classifier and persists the updated model
   */
  async submitMessageReview(
    messageId: string,
    correctCategory: MessageCategory
  ): Promise<void> {
    if (!this.clerk) {
      await this.initialize();
    }
    
    // Submit review with clerk instance for training + persistence
    await submitReview(messageId, correctCategory, this.clerk!);
    
    logger.info('Submitted message review with model update', {
      messageId,
      correctCategory,
    });
  }
  
  /**
   * Persist the current classifier model state
   */
  async persistClassifierModel(): Promise<void> {
    if (!this.clerk) {
      await this.initialize();
    }
    
    await saveClassifierModel(this.clerk!);
  }
  
  /**
   * Get system health stats (includes job health metrics)
   */
  async getSystemStats(): Promise<{
    discovery: { sources: number; lastScrape: Date | null };
    scoring: { contacts: number; scored: number; avgScore: number };
    optimizer: { arms: number; trials: number; convergence: number };
    inbox: { total: number; pendingReview: number };
    jobs: { completed: number; failed: number; failureRate: number; recentErrors: string[] };
  }> {
    // Discovery stats
    const discoveryStats = await queryReadOnly<{
      source_count: string;
      last_scrape: string | null;
    }>(`
      SELECT COUNT(*) as source_count, MAX(last_scraped_at) as last_scrape
      FROM sa_sources WHERE status = 'active'
    `);
    
    // Job health stats (CRITICAL for visibility)
    const jobStats = await queryReadOnly<{
      status: string;
      count: string;
    }>(`
      SELECT status, COUNT(*) as count 
      FROM sa_scrape_jobs 
      GROUP BY status
    `);
    
    // Recent error messages for debugging (last 24h only, unique errors)
    const recentErrors = await queryReadOnly<{
      error_message: string;
    }>(`
      SELECT error_message FROM (
        SELECT DISTINCT ON (error_message) error_message, completed_at
        FROM sa_scrape_jobs 
        WHERE status = 'failed' 
          AND error_message IS NOT NULL
          AND completed_at > NOW() - INTERVAL '24 hours'
        ORDER BY error_message, completed_at DESC NULLS LAST
      ) sub
      ORDER BY completed_at DESC NULLS LAST
      LIMIT 5
    `);
    
    const jobStatusMap: Record<string, number> = {};
    for (const row of jobStats) {
      jobStatusMap[row.status] = parseInt(row.count);
    }
    const completedJobs = jobStatusMap['completed'] || 0;
    const failedJobs = jobStatusMap['failed'] || 0;
    const totalJobs = completedJobs + failedJobs;
    
    // Scoring stats
    const analystStats = await this.analyst.getStats();
    
    // Optimizer stats
    const banditStats = await getBanditStats('subject_line');
    
    // Inbox stats
    const inboxStats = await getClassificationStats();
    
    return {
      discovery: {
        sources: parseInt(discoveryStats[0].source_count),
        lastScrape: discoveryStats[0].last_scrape 
          ? new Date(discoveryStats[0].last_scrape) 
          : null,
      },
      jobs: {
        completed: completedJobs,
        failed: failedJobs,
        failureRate: totalJobs > 0 ? Math.round((failedJobs / totalJobs) * 100) : 0,
        recentErrors: recentErrors.map(r => r.error_message).filter(Boolean),
      },
      scoring: {
        contacts: analystStats.totalContacts,
        scored: analystStats.scoredContacts,
        avgScore: analystStats.averageScore,
      },
      optimizer: {
        arms: banditStats.totalArms,
        trials: banditStats.totalTrials,
        convergence: banditStats.convergenceScore,
      },
      inbox: {
        total: inboxStats.totalMessages,
        pendingReview: inboxStats.pendingReview,
      },
    };
  }
  
  /**
   * Create a job record
   * Note: scheduled_at is NOT NULL in schema, so we set it to current timestamp
   */
  private async createJob(jobType: string): Promise<string> {
    const result = await queryOperator<{ id: string }>(`
      INSERT INTO sa_autonomous_jobs (job_type, status, scheduled_at)
      VALUES ($1, 'pending', CURRENT_TIMESTAMP)
      RETURNING id
    `, [jobType]);
    
    return result[0].id;
  }
  
  /**
   * Update job status
   * Note: Schema uses output_data (not result) for job output
   */
  private async updateJobStatus(
    jobId: string, 
    status: string, 
    result?: Record<string, unknown>
  ): Promise<void> {
    if (status === 'running') {
      await queryOperator(`
        UPDATE sa_autonomous_jobs 
        SET status = 'running', started_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [jobId]);
    } else {
      await queryOperator(`
        UPDATE sa_autonomous_jobs 
        SET status = $1, completed_at = CURRENT_TIMESTAMP, output_data = $2
        WHERE id = $3
      `, [status, JSON.stringify(result || {}), jobId]);
    }
  }
  
  /**
   * Cleanup resources
   */
  async shutdown(): Promise<void> {
    await this.hunter.close();
    logger.info('Autonomous Sales Orchestrator shut down');
  }
}

// ============================================================================
// SINGLETON INSTANCE
// ============================================================================

let orchestratorInstance: AutonomousSalesOrchestrator | null = null;
let initializationPromise: Promise<AutonomousSalesOrchestrator> | null = null;

export async function getOrchestrator(): Promise<AutonomousSalesOrchestrator> {
  // If already initialized, return immediately
  if (orchestratorInstance) {
    return orchestratorInstance;
  }
  
  // If initialization is in progress, wait for it
  if (initializationPromise) {
    return initializationPromise;
  }
  
  // Start initialization (only once)
  initializationPromise = (async () => {
    const instance = new AutonomousSalesOrchestrator();
    await instance.initialize();
    orchestratorInstance = instance;
    return instance;
  })();
  
  return initializationPromise;
}

// ============================================================================
// CLI COMMANDS (for cron jobs)
// ============================================================================

export async function runAutonomousJob(config: AutonomousJobConfig): Promise<JobResult> {
  const orchestrator = await getOrchestrator();
  
  switch (config.jobType) {
    case 'discover':
      return orchestrator.runDiscoveryJob(config.batchSize || 3);
      
    case 'score':
      return orchestrator.runScoringJob(config.batchSize || 50);
      
    case 'icebreaker':
      return orchestrator.runIcebreakerJob(config.batchSize || 30);
      
    case 'enrich':
      return orchestrator.runEnrichmentJob(config.batchSize || 20);
      
    case 'train':
      return orchestrator.runTrainingJob();
      
    default:
      throw new Error(`Unknown job type: ${config.jobType}`);
  }
}
