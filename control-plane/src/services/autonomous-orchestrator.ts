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
import { Clerk, initializeClerkFromDatabase, saveMessageClassification, getClassificationStats } from './inbox/clerk';

// ============================================================================
// TYPES
// ============================================================================

export interface AutonomousJobConfig {
  jobType: 'discover' | 'score' | 'compose' | 'classify';
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
  
  constructor() {
    this.hunter = new Hunter({ headless: true, requestDelayMs: 2000 });
    this.analyst = new Analyst();
    this.spintaxEngine = new SpintaxEngine({ minDistance: 50 });
  }
  
  /**
   * Initialize the orchestrator (load clerk from DB)
   */
  async initialize(): Promise<void> {
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
   * Compose a personalized message for a contact
   */
  async composeMessage(contactId: string): Promise<{
    subject: string;
    body: string;
    templateId: string;
    subjectArmId: string;
    variationHash: string;
  } | null> {
    // Get contact details
    const contacts = await queryReadOnly<{
      id: string;
      first_name: string | null;
      last_name: string | null;
      company_name: string;
      app_name: string | null;
      app_genre: string | null;
      icebreaker: string | null;
    }>(`
      SELECT id, first_name, last_name, company_name, app_name, app_genre, icebreaker
      FROM sa_contacts WHERE id = $1
    `, [contactId]);
    
    if (contacts.length === 0) return null;
    const contact = contacts[0];
    
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
    const context: PersonalizationContext = {
      firstName: contact.first_name || undefined,
      lastName: contact.last_name || undefined,
      company: contact.company_name,
      appName: contact.app_name || undefined,
      appGenre: contact.app_genre || undefined,
      icebreaker: contact.icebreaker || undefined,
    };
    
    // Generate unique message
    const result = this.spintaxEngine.generateUnique(
      subjectArm.armValue, // Use Thompson-selected subject
      template.bodySpintax,
      context
    );
    
    // Record the generation
    await recordGeneration({
      templateId: template.id,
      resolvedSubject: result.subject,
      resolvedBody: result.body,
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
   * Get system health stats
   */
  async getSystemStats(): Promise<{
    discovery: { sources: number; lastScrape: Date | null };
    scoring: { contacts: number; scored: number; avgScore: number };
    optimizer: { arms: number; trials: number; convergence: number };
    inbox: { total: number; pendingReview: number };
  }> {
    // Discovery stats
    const discoveryStats = await queryReadOnly<{
      source_count: string;
      last_scrape: string | null;
    }>(`
      SELECT COUNT(*) as source_count, MAX(last_scraped_at) as last_scrape
      FROM sa_sources WHERE status = 'active'
    `);
    
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
   */
  private async createJob(jobType: string): Promise<string> {
    const result = await queryOperator<{ id: string }>(`
      INSERT INTO sa_autonomous_jobs (job_type, status)
      VALUES ($1, 'pending')
      RETURNING id
    `, [jobType]);
    
    return result[0].id;
  }
  
  /**
   * Update job status
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
        SET status = $1, completed_at = CURRENT_TIMESTAMP, result = $2
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

export async function getOrchestrator(): Promise<AutonomousSalesOrchestrator> {
  if (!orchestratorInstance) {
    orchestratorInstance = new AutonomousSalesOrchestrator();
    await orchestratorInstance.initialize();
  }
  return orchestratorInstance;
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
      
    default:
      throw new Error(`Unknown job type: ${config.jobType}`);
  }
}
