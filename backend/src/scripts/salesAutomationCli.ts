#!/usr/bin/env node
// scripts/salesAutomationCli.ts
// CLI for running the automated sales system on the Hetzner app server
// Usage: node dist/src/scripts/salesAutomationCli.js [command] [options]

import path from 'path';
import { config } from 'dotenv';

// Load environment variables from multiple locations
const envPaths = [
  path.resolve(process.cwd(), '.env'),
  path.resolve(process.cwd(), '../.env'),
  '/opt/adproject/.env',
  '/opt/adproject/backend/.env',
];

for (const envPath of envPaths) {
  config({ path: envPath });
}

import { SalesAutomationEngine } from '../../services/sales-automation';

interface CommandResult {
  success: boolean;
  message: string;
  data?: any;
}

async function runCommand(command: string, args: string[]): Promise<CommandResult> {
  const databaseUrl = process.env.DATABASE_URL;
  
  if (!databaseUrl) {
    return {
      success: false,
      message: 'DATABASE_URL not set. Please set DATABASE_URL environment variable.',
    };
  }

  const engine = new SalesAutomationEngine(databaseUrl);

  try {
    switch (command) {
      case 'discover':
        console.log('🔍 Running prospect discovery...');
        const discoveryResult = await engine['discovery'].runDiscoveryCycle();
        return {
          success: true,
          message: `Discovery complete: ${discoveryResult.discovered} new prospects, ${discoveryResult.errors} errors`,
          data: discoveryResult,
        };

      case 'enrich':
        const batchSize = parseInt(args[0] || '50', 10);
        console.log(`📊 Running lead enrichment (batch size: ${batchSize})...`);
        const enrichResult = await engine['enrichment'].runEnrichmentCycle(batchSize);
        return {
          success: true,
          message: `Enrichment complete: ${enrichResult.enriched} enriched, ${enrichResult.contacts_found} contacts found`,
          data: enrichResult,
        };

      case 'outreach':
        console.log('📧 Running outreach cycle...');
        const outreachResult = await engine['outreach'].runOutreachCycle();
        return {
          success: true,
          message: `Outreach complete: ${outreachResult.sent} emails sent, ${outreachResult.errors} errors`,
          data: outreachResult,
        };

      case 'enroll':
        console.log('📝 Enrolling qualified contacts...');
        const enrolled = await engine['outreach'].enrollQualifiedContacts();
        return {
          success: true,
          message: `Enrolled ${enrolled} new contacts`,
          data: { enrolled },
        };

      case 'analytics':
        console.log('📈 Running analytics roll-up...');
        await engine['analytics'].runDailyRollup();
        return {
          success: true,
          message: 'Analytics roll-up complete',
        };

      case 'full':
        console.log('🚀 Running full sales cycle...');
        const fullResult = await engine.runFullCycle();
        return {
          success: true,
          message: 'Full cycle complete',
          data: fullResult,
        };

      case 'pipeline':
        console.log('📊 Fetching pipeline metrics...');
        const metrics = await engine.getPipelineMetrics();
        console.log('\n=== PIPELINE METRICS ===');
        console.log(`Total Prospects:   ${metrics.total_prospects}`);
        console.log(`Qualified:         ${metrics.total_qualified}`);
        console.log(`Contacted:         ${metrics.total_contacted}`);
        console.log(`Responded:         ${metrics.total_responded}`);
        console.log(`Converted:         ${metrics.total_converted}`);
        console.log(`Conversion Rate:   ${metrics.conversion_rate}%`);
        console.log(`Pipeline Value:    $${(metrics.pipeline_value_cents / 100).toFixed(2)}`);
        return {
          success: true,
          message: 'Pipeline metrics retrieved',
          data: metrics,
        };

      case 'funnel':
        console.log('📊 Fetching funnel metrics...');
        const funnel = await engine.getFunnelMetrics();
        console.log('\n=== FUNNEL ===');
        for (const stage of funnel) {
          console.log(`${stage.stage.padEnd(15)} ${stage.count.toString().padStart(6)} (${stage.conversion_rate.toFixed(1)}%)`);
        }
        return {
          success: true,
          message: 'Funnel metrics retrieved',
          data: funnel,
        };

      case 'sequences':
        console.log('📊 Fetching sequence performance...');
        const sequences = await engine.getSequencePerformance();
        console.log('\n=== SEQUENCE PERFORMANCE ===');
        for (const seq of sequences) {
          console.log(`\n${seq.sequence_name}:`);
          console.log(`  Enrolled:       ${seq.total_enrolled}`);
          console.log(`  Open Rate:      ${seq.open_rate.toFixed(1)}%`);
          console.log(`  Reply Rate:     ${seq.reply_rate.toFixed(1)}%`);
          console.log(`  Conversion:     ${seq.conversion_rate.toFixed(1)}%`);
        }
        return {
          success: true,
          message: 'Sequence performance retrieved',
          data: sequences,
        };

      case 'trend':
        const days = parseInt(args[0] || '7', 10);
        console.log(`📊 Fetching ${days}-day trend...`);
        const trend = await engine.getMetricsTrend(days);
        console.log('\n=== DAILY TREND ===');
        console.log('Date       | Discovered | Qualified | Sent | Opened | Replied | Won');
        console.log('-----------|------------|-----------|------|--------|---------|----');
        for (const day of trend) {
          console.log(
            `${day.date} | ${day.prospects_discovered.toString().padStart(10)} | ` +
            `${day.prospects_qualified.toString().padStart(9)} | ${day.emails_sent.toString().padStart(4)} | ` +
            `${day.emails_opened.toString().padStart(6)} | ${day.emails_replied.toString().padStart(7)} | ` +
            `${day.deals_won.toString().padStart(3)}`
          );
        }
        return {
          success: true,
          message: 'Trend retrieved',
          data: trend,
        };

      case 'daemon':
        console.log('🤖 Starting sales automation daemon...');
        console.log('Press Ctrl+C to stop\n');
        engine.start();
        
        // Keep process running
        await new Promise<void>((resolve) => {
          process.on('SIGINT', () => {
            console.log('\nShutting down...');
            engine.stop();
            resolve();
          });
          process.on('SIGTERM', () => {
            console.log('\nReceived SIGTERM, shutting down...');
            engine.stop();
            resolve();
          });
        });
        
        return {
          success: true,
          message: 'Daemon stopped',
        };

      case 'help':
      default:
        console.log(`
🤖 ApexMediation Sales Automation CLI
=====================================

Usage: node dist/src/scripts/salesAutomationCli.js <command> [options]

Commands:
  discover           Run prospect discovery from app stores
  enrich [batch]     Enrich discovered prospects (default batch: 50)
  outreach           Send scheduled outreach emails
  enroll             Enroll qualified contacts into sequences
  analytics          Run daily analytics roll-up
  full               Run complete sales cycle (all steps)
  pipeline           Show pipeline metrics
  funnel             Show conversion funnel
  sequences          Show sequence performance
  trend [days]       Show daily trend (default: 7 days)
  daemon             Run as background daemon (all jobs scheduled)
  help               Show this help message

Environment Variables:
  DATABASE_URL       PostgreSQL connection string (required)
  APEXMAIL_API_KEY   ApexMail API key for email sending (optional)
  SMTP_HOST          SMTP server hostname (optional)

Examples:
  # Run discovery only
  node dist/src/scripts/salesAutomationCli.js discover

  # Enrich 100 prospects
  node dist/src/scripts/salesAutomationCli.js enrich 100

  # Check pipeline health
  node dist/src/scripts/salesAutomationCli.js pipeline

  # Run as daemon (production)
  node dist/src/scripts/salesAutomationCli.js daemon
        `);
        return {
          success: true,
          message: 'Help displayed',
        };
    }
  } catch (error) {
    console.error('Error:', error);
    return {
      success: false,
      message: `Error: ${error instanceof Error ? error.message : String(error)}`,
    };
  } finally {
    if (command !== 'daemon') {
      await engine.close();
    }
  }
}

// Main entry point
async function main() {
  const args = process.argv.slice(2);
  const command = args[0] || 'help';
  const commandArgs = args.slice(1);

  console.log(`\n🚀 ApexMediation Sales Automation`);
  console.log(`   Command: ${command}`);
  console.log(`   Time: ${new Date().toISOString()}\n`);

  const result = await runCommand(command, commandArgs);

  if (!result.success) {
    console.error(`\n❌ ${result.message}`);
    process.exit(1);
  }

  console.log(`\n✅ ${result.message}`);
  process.exit(0);
}

main().catch((error) => {
  console.error('Fatal error:', error);
  process.exit(1);
});
