// services/sales-automation/AutomationEngine.ts
// The MASTER orchestrator - fully autonomous sales automation
// REVOLUTIONARY: Completely hands-off operation with self-optimization
// Combines all revolutionary services into one self-running engine

import { Pool } from 'pg';

// Import all revolutionary services
import { PredictiveLeadScoring } from './PredictiveLeadScoring';
import { SelfLearningSequences } from './SelfLearningSequences';
import { SmartReplyAnalyzer } from './SmartReplyAnalyzer';
import { DomainHealthGuard } from './DomainHealthGuard';
import { IntentSignalDetector } from './IntentSignalDetector';
import { FollowUpOptimizer } from './FollowUpOptimizer';

// Import existing services
import { SpamPrevention } from './SpamPrevention';
import { ReliabilityGuard } from './ReliabilityGuard';
import { IntelligentDeduplication } from './IntelligentDeduplication';
import { AdaptivePersonalization } from './AdaptivePersonalization';
import { TransparentLogging } from './TransparentLogging';
import { DataQualityEngine } from './DataQualityEngine';

// Import core services
import { OutreachService } from './OutreachService';
import { EmailWarmupService } from './EmailWarmupService';
import { EmailVerificationService } from './EmailVerificationService';
import { SendTimingOptimizer } from './SendTimingOptimizer';
import { EngagementBranchingService } from './EngagementBranchingService';

interface EngineConfig {
  databaseUrl: string;
  defaultMailbox: string;
  maxDailyEmails: number;
  enableSelfLearning: boolean;
  enableAutonomousMode: boolean;
}

interface EngineStats {
  todaySent: number;
  todayReplies: number;
  todayHotLeads: number;
  domainHealth: number;
  sequenceOptimizations: number;
  errorsToday: number;
}

interface TaskResult {
  task: string;
  success: boolean;
  duration: number;
  details?: string;
}

/**
 * AutomationEngine - The fully autonomous sales machine
 * REVOLUTIONARY: Self-running, self-optimizing, self-healing
 */
export class AutomationEngine {
  private pool: Pool;
  private config: EngineConfig;
  
  // Revolutionary services
  private leadScoring!: PredictiveLeadScoring;
  private selfLearning!: SelfLearningSequences;
  private replyAnalyzer!: SmartReplyAnalyzer;
  private domainHealth!: DomainHealthGuard;
  private intentDetector!: IntentSignalDetector;
  private followUpOptimizer!: FollowUpOptimizer;
  
  // Improvement services
  private spamPrevention!: SpamPrevention;
  private reliabilityGuard!: ReliabilityGuard;
  private deduplication!: IntelligentDeduplication;
  private personalization!: AdaptivePersonalization;
  private logging!: TransparentLogging;
  private dataQuality!: DataQualityEngine;
  
  // Core services
  private outreach!: OutreachService;
  private warmup!: EmailWarmupService;
  private verification!: EmailVerificationService;
  private timing!: SendTimingOptimizer;
  private branching!: EngagementBranchingService;

  private isRunning = false;
  private lastRunTime: Date | null = null;

  constructor(config: EngineConfig) {
    this.config = config;
    this.pool = new Pool({ connectionString: config.databaseUrl });
  }

  /**
   * Initialize all services
   */
  async initialize(): Promise<void> {
    console.log('[AutomationEngine] 🚀 Initializing all services...');

    // Initialize revolutionary services
    this.leadScoring = new PredictiveLeadScoring(this.config.databaseUrl);
    this.selfLearning = new SelfLearningSequences(this.config.databaseUrl);
    this.replyAnalyzer = new SmartReplyAnalyzer(this.config.databaseUrl);
    this.domainHealth = new DomainHealthGuard(this.config.databaseUrl);
    this.intentDetector = new IntentSignalDetector(this.config.databaseUrl);
    this.followUpOptimizer = new FollowUpOptimizer(this.config.databaseUrl);

    // Initialize improvement services
    this.spamPrevention = new SpamPrevention(this.config.databaseUrl);
    this.reliabilityGuard = new ReliabilityGuard(this.config.databaseUrl);
    this.deduplication = new IntelligentDeduplication(this.config.databaseUrl);
    this.personalization = new AdaptivePersonalization(this.config.databaseUrl);
    this.logging = new TransparentLogging(this.config.databaseUrl);
    this.dataQuality = new DataQualityEngine(this.config.databaseUrl);

    // Initialize core services
    this.outreach = new OutreachService(this.config.databaseUrl);
    this.warmup = new EmailWarmupService(this.config.databaseUrl);
    this.verification = new EmailVerificationService(this.config.databaseUrl);
    this.timing = new SendTimingOptimizer(this.config.databaseUrl);
    this.branching = new EngagementBranchingService(this.config.databaseUrl);

    // Initialize all tables
    await Promise.all([
      this.leadScoring.initialize(),
      this.selfLearning.initialize(),
      this.replyAnalyzer.initialize(),
      this.domainHealth.initialize(),
      this.intentDetector.initialize(),
      this.followUpOptimizer.initialize(),
      this.spamPrevention.initialize(),
      this.reliabilityGuard.initialize(),
      this.deduplication.initialize(),
      this.personalization.initialize(),
      this.logging.initialize(),
      this.dataQuality.initialize(),
    ]);

    console.log('[AutomationEngine] ✅ All services initialized');
  }

  /**
   * MAIN: Run the full automation cycle
   * This is what runs autonomously
   */
  async runCycle(): Promise<TaskResult[]> {
    if (this.isRunning) {
      console.log('[AutomationEngine] Cycle already running, skipping...');
      return [];
    }

    this.isRunning = true;
    this.lastRunTime = new Date();
    const results: TaskResult[] = [];

    try {
      console.log('[AutomationEngine] ═══════════════════════════════════════');
      console.log('[AutomationEngine] 🔄 Starting automation cycle...');
      console.log('[AutomationEngine] ═══════════════════════════════════════');

      // 1. Health checks first
      results.push(await this.runTask('Domain Health Check', async () => {
        const released = await this.domainHealth.checkQuarantines();
        if (released.length > 0) {
          console.log(`[AutomationEngine] Released domains from quarantine: ${released.join(', ')}`);
        }
        const health = await this.domainHealth.getDomainHealth(
          this.config.defaultMailbox.split('@')[1]
        );
        return `Health: ${health.healthScore}/100 (${health.status})`;
      }));

      // 2. Process new replies
      results.push(await this.runTask('Process Replies', async () => {
        const processed = await this.replyAnalyzer.analyzeUnprocessedReplies();
        const hotLeads = await this.replyAnalyzer.getHotLeads();
        return `Processed ${processed} replies, ${hotLeads.length} hot leads`;
      }));

      // 3. Score leads and prioritize
      results.push(await this.runTask('Score Leads', async () => {
        const hotLeads = await this.leadScoring.getHotLeads(20);
        const risingIntents = await this.intentDetector.getRisingIntentContacts();
        return `${hotLeads.length} hot leads, ${risingIntents.length} rising intent`;
      }));

      // 4. Run data quality checks
      results.push(await this.runTask('Data Quality', async () => {
        const issues = await this.dataQuality.runQualityChecks();
        if (issues.length > 0) {
          await this.dataQuality.autoFixIssues(issues.slice(0, 10));
        }
        return `${issues.length} issues found`;
      }));

      // 5. Deduplicate contacts
      results.push(await this.runTask('Deduplication', async () => {
        const duplicates = await this.deduplication.findDuplicates();
        if (duplicates.length > 0) {
          await this.deduplication.autoMergeDuplicates(duplicates.slice(0, 5));
        }
        return `${duplicates.length} duplicate sets`;
      }));

      // 6. Can we send? Check domain health
      const sendDecision = await this.domainHealth.canSend(this.config.defaultMailbox);
      
      if (sendDecision.allowed) {
        // 7. Get contacts ready for outreach
        results.push(await this.runTask('Prepare Outreach', async () => {
          const prioritized = await this.leadScoring.getHotLeads(
            Math.min(50, this.config.maxDailyEmails)
          );
          return `${prioritized.length} contacts ready`;
        }));

        // 8. Send emails with all protections
        results.push(await this.runTask('Send Emails', async () => {
          let sent = 0;
          const hotLeads = await this.leadScoring.getHotLeads(20);

          for (const lead of hotLeads) {
            // Check if we can still send
            const canSend = await this.domainHealth.canSend(this.config.defaultMailbox);
            if (!canSend.allowed) {
              console.log(`[AutomationEngine] Stopping sends: ${canSend.reason}`);
              break;
            }

            try {
              // Get optimal timing
              const timing = await this.timing.getOptimalSendTime(lead.contactId);

              // Get personalized content
              const personalized = await this.personalization.personalizeEmail(
                lead.contactId,
                'default-template',
                {}
              );

              // Check spam score
              const spamCheck = await this.spamPrevention.checkEmail(
                personalized.subject,
                personalized.body
              );

              if (spamCheck.score > 5) {
                console.log(`[AutomationEngine] Skipping ${lead.contactId}: spam score too high`);
                continue;
              }

              // Send via outreach service (if timing is now)
              if (timing <= new Date()) {
                await this.outreach.sendEmail({
                  contactId: lead.contactId,
                  from: this.config.defaultMailbox,
                  subject: personalized.subject,
                  body: personalized.body,
                });

                await this.domainHealth.recordSend(this.config.defaultMailbox);
                sent++;
              }
            } catch (error) {
              console.error(`[AutomationEngine] Error sending to ${lead.contactId}:`, error);
              await this.reliabilityGuard.recordError('send', error);
            }
          }

          return `${sent} emails sent`;
        }));
      } else {
        results.push({
          task: 'Send Emails',
          success: false,
          duration: 0,
          details: `Skipped: ${sendDecision.reason}`,
        });
      }

      // 9. Self-learning optimization
      if (this.config.enableSelfLearning) {
        results.push(await this.runTask('Self-Learning', async () => {
          // Evaluate A/B tests
          const winners = await this.selfLearning.evaluateABTests();

          // Learn from top performers
          const patterns = await this.selfLearning.learnFromTopPerformers();

          // Find sequence gaps
          const gaps = await this.followUpOptimizer.findSequenceGaps();

          return `${winners.length} test winners, ${patterns.length} patterns, ${gaps.length} gaps`;
        }));
      }

      // 10. Run warmup (if needed)
      results.push(await this.runTask('Warmup', async () => {
        const warmupStatus = await this.warmup.runDailyWarmup(this.config.defaultMailbox);
        return `Warmup: ${warmupStatus}`;
      }));

      // 11. Log summary
      await this.logging.logCycleComplete(results);

      console.log('[AutomationEngine] ═══════════════════════════════════════');
      console.log('[AutomationEngine] ✅ Cycle complete');
      console.log('[AutomationEngine] ═══════════════════════════════════════');

    } catch (error) {
      console.error('[AutomationEngine] ❌ Cycle failed:', error);
      await this.reliabilityGuard.recordError('cycle', error);
    } finally {
      this.isRunning = false;
    }

    return results;
  }

  /**
   * Get current engine statistics
   */
  async getStats(): Promise<EngineStats> {
    const today = new Date().toISOString().split('T')[0];

    // Sent today
    const sentResult = await this.pool.query(`
      SELECT COUNT(*) as count FROM outreach_emails 
      WHERE sent_at::date = $1::date
    `, [today]);

    // Replies today
    const repliesResult = await this.pool.query(`
      SELECT COUNT(*) as count FROM outreach_emails 
      WHERE replied_at::date = $1::date
    `, [today]);

    // Hot leads
    const hotLeadsResult = await this.pool.query(`
      SELECT COUNT(*) as count FROM intent_scores 
      WHERE intent_level = 'hot'
    `);

    // Domain health
    const domain = this.config.defaultMailbox.split('@')[1];
    const health = await this.domainHealth.getDomainHealth(domain);

    // Optimizations
    const optimizationsResult = await this.pool.query(`
      SELECT COUNT(*) as count FROM sequence_ab_tests 
      WHERE created_at::date = $1::date
    `, [today]);

    // Errors
    const errorsResult = await this.pool.query(`
      SELECT COUNT(*) as count FROM reliability_errors 
      WHERE occurred_at::date = $1::date
    `, [today]);

    return {
      todaySent: parseInt(sentResult.rows[0]?.count) || 0,
      todayReplies: parseInt(repliesResult.rows[0]?.count) || 0,
      todayHotLeads: parseInt(hotLeadsResult.rows[0]?.count) || 0,
      domainHealth: health.healthScore,
      sequenceOptimizations: parseInt(optimizationsResult.rows[0]?.count) || 0,
      errorsToday: parseInt(errorsResult.rows[0]?.count) || 0,
    };
  }

  /**
   * Run a diagnostic check on all systems
   */
  async runDiagnostics(): Promise<{
    service: string;
    status: 'healthy' | 'warning' | 'error';
    message: string;
  }[]> {
    const results: {
      service: string;
      status: 'healthy' | 'warning' | 'error';
      message: string;
    }[] = [];

    // Check database connection
    try {
      await this.pool.query('SELECT 1');
      results.push({ service: 'Database', status: 'healthy', message: 'Connected' });
    } catch (error) {
      results.push({ service: 'Database', status: 'error', message: String(error) });
    }

    // Check domain health
    try {
      const domain = this.config.defaultMailbox.split('@')[1];
      const health = await this.domainHealth.getDomainHealth(domain);
      results.push({
        service: 'Domain Health',
        status: health.status === 'healthy' ? 'healthy' : 
                health.status === 'warning' ? 'warning' : 'error',
        message: `Score: ${health.healthScore}/100`,
      });
    } catch (error) {
      results.push({ service: 'Domain Health', status: 'error', message: String(error) });
    }

    // Check lead scoring
    try {
      const hotLeads = await this.leadScoring.getHotLeads(1);
      results.push({
        service: 'Lead Scoring',
        status: 'healthy',
        message: `System active, ${hotLeads.length > 0 ? 'leads scored' : 'no leads yet'}`,
      });
    } catch (error) {
      results.push({ service: 'Lead Scoring', status: 'error', message: String(error) });
    }

    // Check self-learning
    try {
      const patterns = await this.selfLearning.learnFromTopPerformers();
      results.push({
        service: 'Self-Learning',
        status: 'healthy',
        message: `${patterns.length} patterns detected`,
      });
    } catch (error) {
      results.push({ service: 'Self-Learning', status: 'error', message: String(error) });
    }

    // Check spam prevention
    try {
      const testCheck = await this.spamPrevention.checkEmail('Test', 'Test body');
      results.push({
        service: 'Spam Prevention',
        status: 'healthy',
        message: `Active, test score: ${testCheck.score}`,
      });
    } catch (error) {
      results.push({ service: 'Spam Prevention', status: 'error', message: String(error) });
    }

    // Check reliability guard
    try {
      const circuitStatus = await this.reliabilityGuard.getCircuitStatus();
      results.push({
        service: 'Reliability Guard',
        status: circuitStatus === 'closed' ? 'healthy' : 'warning',
        message: `Circuit: ${circuitStatus}`,
      });
    } catch (error) {
      results.push({ service: 'Reliability Guard', status: 'error', message: String(error) });
    }

    return results;
  }

  /**
   * Start autonomous mode - runs on schedule
   */
  startAutonomousMode(intervalMinutes: number = 30): void {
    if (!this.config.enableAutonomousMode) {
      console.log('[AutomationEngine] Autonomous mode disabled in config');
      return;
    }

    console.log(`[AutomationEngine] 🤖 Starting autonomous mode (every ${intervalMinutes} min)`);

    // Run immediately
    this.runCycle();

    // Schedule recurring runs
    setInterval(() => {
      this.runCycle();
    }, intervalMinutes * 60 * 1000);
  }

  // ===== Private Methods =====

  private async runTask(
    name: string,
    fn: () => Promise<string>
  ): Promise<TaskResult> {
    const start = Date.now();
    try {
      const details = await fn();
      const duration = Date.now() - start;
      console.log(`[AutomationEngine] ✓ ${name} (${duration}ms): ${details}`);
      return { task: name, success: true, duration, details };
    } catch (error) {
      const duration = Date.now() - start;
      console.error(`[AutomationEngine] ✗ ${name} (${duration}ms):`, error);
      return { task: name, success: false, duration, details: String(error) };
    }
  }
}

// ===== Factory Function =====
export function createAutomationEngine(config: Partial<EngineConfig> = {}): AutomationEngine {
  const fullConfig: EngineConfig = {
    databaseUrl: process.env.DATABASE_URL || '',
    defaultMailbox: config.defaultMailbox || 'sabel@apexmediation.ee',
    maxDailyEmails: config.maxDailyEmails || 100,
    enableSelfLearning: config.enableSelfLearning ?? true,
    enableAutonomousMode: config.enableAutonomousMode ?? true,
    ...config,
  };

  return new AutomationEngine(fullConfig);
}
