#!/usr/bin/env node
/**
 * Sales Automation Comprehensive Smoke Test
 * 
 * Tests each segment of the sales automation system:
 * 1. Template Management (sa_spintax_templates)
 * 2. Lead/Prospect Pipeline (sa_contacts, sa_sources)
 * 3. Email Generation Engine (spintax expansion)
 * 4. Sending/Delivery System (sa_emails, sa_outbound_messages)
 * 5. Tracking/Analytics (sa_email_events, sa_activity_log)
 * 6. Scheduling/Sequencing (sa_sequences, sa_enrollments)
 * 7. ML/Optimization (sa_bandit_arms, sa_bayes_training)
 * 8. Configuration (sa_config)
 */

const { Client } = require('pg');

const dbConfig = {
  host: process.env.DB_HOST || '127.0.0.1',
  port: parseInt(process.env.DB_PORT || '6432'),
  database: process.env.DB_NAME || 'adproject',
  user: process.env.DB_USER || 'adproject',
  password: process.env.DB_PASSWORD || 'xP5Qa6z+zov+besvN3Fkx2bKgqkb86qtWDYiClttXBtPB44f',
  ssl: false
};

// Spintax expansion helper
function expandSpintax(text) {
  return text.replace(/\{([^{}]+)\}/g, (match, options) => {
    const choices = options.split('|');
    return choices[Math.floor(Math.random() * choices.length)];
  });
}

async function main() {
  const client = new Client(dbConfig);
  const results = {
    passed: 0,
    failed: 0,
    segments: {}
  };

  try {
    await client.connect();
    console.log('=' .repeat(80));
    console.log('SALES AUTOMATION SMOKE TEST');
    console.log('=' .repeat(80));
    console.log(`Database: ${dbConfig.host}:${dbConfig.port}/${dbConfig.database}`);
    console.log(`Timestamp: ${new Date().toISOString()}`);
    console.log('');

    // =========================================================================
    // SEGMENT 1: TEMPLATE MANAGEMENT
    // =========================================================================
    console.log('\n' + '=' .repeat(80));
    console.log('SEGMENT 1: TEMPLATE MANAGEMENT');
    console.log('=' .repeat(80));

    const templates = await client.query(`
      SELECT id, name, template_type, active,
             length(subject_spintax) as subject_len,
             length(body_spintax) as body_len,
             created_at
      FROM sa_spintax_templates
      ORDER BY active DESC, created_at DESC
    `);

    const activeTemplates = templates.rows.filter(t => t.active);
    const templateTypes = [...new Set(activeTemplates.map(t => t.template_type))];
    
    console.log(`\n  Total templates: ${templates.rows.length}`);
    console.log(`  Active templates: ${activeTemplates.length}`);
    console.log(`  Template types: ${templateTypes.join(', ')}`);
    console.log('\n  Active Templates:');
    activeTemplates.forEach(t => {
      console.log(`    [${t.template_type}] ${t.name} (subject: ${t.subject_len} chars, body: ${t.body_len} chars)`);
    });

    const seg1Pass = activeTemplates.length >= 3 && templateTypes.length >= 2;
    results.segments['1_templates'] = {
      status: seg1Pass ? 'PASS' : 'FAIL',
      active: activeTemplates.length,
      types: templateTypes.length
    };
    if (seg1Pass) results.passed++; else results.failed++;
    console.log(`\n  ✓ Status: ${seg1Pass ? 'PASS' : 'FAIL'}`);

    // =========================================================================
    // SEGMENT 2: LEAD/PROSPECT PIPELINE
    // =========================================================================
    console.log('\n' + '=' .repeat(80));
    console.log('SEGMENT 2: LEAD/PROSPECT PIPELINE');
    console.log('=' .repeat(80));

    const contacts = await client.query(`
      SELECT status, COUNT(*) as count
      FROM sa_contacts
      GROUP BY status
      ORDER BY count DESC
    `);

    const totalContacts = contacts.rows.reduce((sum, r) => sum + parseInt(r.count), 0);
    console.log(`\n  Total contacts: ${totalContacts}`);
    console.log('  By status:');
    contacts.rows.forEach(r => {
      console.log(`    ${r.status}: ${r.count}`);
    });

    const sources = await client.query(`
      SELECT source_type, COUNT(*) as count
      FROM sa_sources
      GROUP BY source_type
    `);
    console.log('\n  Lead sources:');
    sources.rows.forEach(r => {
      console.log(`    ${r.source_type}: ${r.count}`);
    });

    // Sample contact
    const sampleContact = await client.query(`
      SELECT id, email, first_name, company_name, app_name, status, created_at
      FROM sa_contacts
      ORDER BY created_at DESC
      LIMIT 1
    `);
    if (sampleContact.rows.length > 0) {
      const c = sampleContact.rows[0];
      console.log(`\n  Sample contact:`);
      console.log(`    Email: ${c.email}`);
      console.log(`    Name: ${c.first_name || 'N/A'}`);
      console.log(`    Company: ${c.company_name || 'N/A'}`);
      console.log(`    App: ${c.app_name || 'N/A'}`);
      console.log(`    Status: ${c.status}`);
    }

    const seg2Pass = totalContacts > 0;
    results.segments['2_leads'] = {
      status: seg2Pass ? 'PASS' : 'FAIL',
      total: totalContacts,
      sources: sources.rows.length
    };
    if (seg2Pass) results.passed++; else results.failed++;
    console.log(`\n  ✓ Status: ${seg2Pass ? 'PASS' : 'FAIL'}`);

    // =========================================================================
    // SEGMENT 3: EMAIL GENERATION ENGINE
    // =========================================================================
    console.log('\n' + '=' .repeat(80));
    console.log('SEGMENT 3: EMAIL GENERATION ENGINE (Spintax)');
    console.log('=' .repeat(80));

    // Test spintax expansion
    const templateSample = await client.query(`
      SELECT name, subject_spintax, body_spintax
      FROM sa_spintax_templates
      WHERE active = true
      LIMIT 1
    `);

    if (templateSample.rows.length > 0) {
      const t = templateSample.rows[0];
      console.log(`\n  Testing template: ${t.name}`);
      
      // Count spintax options
      const subjectOptions = (t.subject_spintax.match(/\{[^}]+\}/g) || []).length;
      const bodyOptions = (t.body_spintax.match(/\{[^}]+\}/g) || []).length;
      console.log(`  Subject spintax blocks: ${subjectOptions}`);
      console.log(`  Body spintax blocks: ${bodyOptions}`);

      // Generate 3 variations
      console.log('\n  Generated variations:');
      for (let i = 1; i <= 3; i++) {
        let subject = expandSpintax(t.subject_spintax);
        let body = expandSpintax(t.body_spintax);
        // Replace placeholders
        subject = subject.replace(/\{\{FIRST_NAME\}\}/g, 'John');
        subject = subject.replace(/\{\{APP_NAME\}\}/g, 'TestApp');
        body = body.replace(/\{\{FIRST_NAME\}\}/g, 'John');
        body = body.replace(/\{\{APP_NAME\}\}/g, 'TestApp');
        body = body.replace(/\{\{APP_GENRE\}\}/g, 'Gaming');
        body = body.replace(/\{\{ICEBREAKER\}\}/g, 'Saw TestApp in the App Store.');
        
        console.log(`\n  --- Variation ${i} ---`);
        console.log(`  Subject: ${subject}`);
        console.log(`  Body preview: ${body.substring(0, 150)}...`);
      }
    }

    // Check spintax history
    const spintaxHistory = await client.query(`
      SELECT COUNT(*) as count FROM sa_spintax_history
    `);
    console.log(`\n  Spintax history records: ${spintaxHistory.rows[0].count}`);

    const seg3Pass = templateSample.rows.length > 0;
    results.segments['3_generation'] = {
      status: seg3Pass ? 'PASS' : 'FAIL',
      templates: templateSample.rows.length,
      historyRecords: parseInt(spintaxHistory.rows[0].count)
    };
    if (seg3Pass) results.passed++; else results.failed++;
    console.log(`\n  ✓ Status: ${seg3Pass ? 'PASS' : 'FAIL'}`);

    // =========================================================================
    // SEGMENT 4: SENDING/DELIVERY SYSTEM
    // =========================================================================
    console.log('\n' + '=' .repeat(80));
    console.log('SEGMENT 4: SENDING/DELIVERY SYSTEM');
    console.log('=' .repeat(80));

    const emailStats = await client.query(`
      SELECT status, COUNT(*) as count
      FROM sa_emails
      GROUP BY status
      ORDER BY count DESC
    `);

    const totalEmails = emailStats.rows.reduce((sum, r) => sum + parseInt(r.count), 0);
    console.log(`\n  Total emails: ${totalEmails}`);
    console.log('  By status:');
    emailStats.rows.forEach(r => {
      console.log(`    ${r.status}: ${r.count}`);
    });

    // Outbound messages
    const outbound = await client.query(`
      SELECT status, COUNT(*) as count
      FROM sa_outbound_messages
      GROUP BY status
    `);
    console.log('\n  Outbound messages:');
    outbound.rows.forEach(r => {
      console.log(`    ${r.status}: ${r.count}`);
    });

    // Bounce records
    const bounces = await client.query(`
      SELECT bounce_type, COUNT(*) as count
      FROM sa_bounce_records
      GROUP BY bounce_type
    `);
    const totalBounces = bounces.rows.reduce((sum, r) => sum + parseInt(r.count), 0);
    console.log(`\n  Bounce records: ${totalBounces}`);
    bounces.rows.forEach(r => {
      console.log(`    ${r.bounce_type}: ${r.count}`);
    });

    // Sample sent email
    const sampleEmail = await client.query(`
      SELECT id, contact_id, subject, status, sent_at, opened_at, clicked_at
      FROM sa_emails
      WHERE status = 'sent'
      ORDER BY sent_at DESC
      LIMIT 1
    `);
    if (sampleEmail.rows.length > 0) {
      const e = sampleEmail.rows[0];
      console.log(`\n  Sample sent email:`);
      console.log(`    Subject: ${e.subject}`);
      console.log(`    Sent: ${e.sent_at}`);
      console.log(`    Opened: ${e.opened_at || 'No'}`);
      console.log(`    Clicked: ${e.clicked_at || 'No'}`);
    }

    const seg4Pass = totalEmails > 0 || outbound.rows.length > 0;
    results.segments['4_delivery'] = {
      status: seg4Pass ? 'PASS' : 'WARN',
      totalEmails,
      bounces: totalBounces
    };
    if (seg4Pass) results.passed++; else results.failed++;
    console.log(`\n  ✓ Status: ${seg4Pass ? 'PASS' : 'WARN (no emails sent yet)'}`);

    // =========================================================================
    // SEGMENT 5: TRACKING/ANALYTICS
    // =========================================================================
    console.log('\n' + '=' .repeat(80));
    console.log('SEGMENT 5: TRACKING/ANALYTICS');
    console.log('=' .repeat(80));

    const emailEvents = await client.query(`
      SELECT event_type, COUNT(*) as count
      FROM sa_email_events
      GROUP BY event_type
      ORDER BY count DESC
    `);
    console.log('\n  Email events:');
    emailEvents.rows.forEach(r => {
      console.log(`    ${r.event_type}: ${r.count}`);
    });

    const activityLog = await client.query(`
      SELECT action, COUNT(*) as count
      FROM sa_activity_log
      GROUP BY action
      ORDER BY count DESC
      LIMIT 10
    `);
    console.log('\n  Activity log (top 10 actions):');
    activityLog.rows.forEach(r => {
      console.log(`    ${r.action}: ${r.count}`);
    });

    // Recent activity
    const recentActivity = await client.query(`
      SELECT action, details, created_at
      FROM sa_activity_log
      ORDER BY created_at DESC
      LIMIT 3
    `);
    console.log('\n  Recent activity:');
    recentActivity.rows.forEach(r => {
      console.log(`    [${r.created_at}] ${r.action}`);
    });

    const seg5Pass = activityLog.rows.length > 0 || emailEvents.rows.length > 0;
    results.segments['5_tracking'] = {
      status: seg5Pass ? 'PASS' : 'WARN',
      eventTypes: emailEvents.rows.length,
      activityTypes: activityLog.rows.length
    };
    if (seg5Pass) results.passed++; else results.failed++;
    console.log(`\n  ✓ Status: ${seg5Pass ? 'PASS' : 'WARN'}`);

    // =========================================================================
    // SEGMENT 6: SCHEDULING/SEQUENCING
    // =========================================================================
    console.log('\n' + '=' .repeat(80));
    console.log('SEGMENT 6: SCHEDULING/SEQUENCING');
    console.log('=' .repeat(80));

    const sequences = await client.query(`
      SELECT id, name, description, active, 
             (SELECT COUNT(*) FROM sa_enrollments WHERE sequence_id = sa_sequences.id) as enrollments
      FROM sa_sequences
      ORDER BY active DESC, id
    `);
    console.log(`\n  Total sequences: ${sequences.rows.length}`);
    console.log('  Sequences:');
    sequences.rows.forEach(s => {
      console.log(`    [${s.active ? 'ACTIVE' : 'inactive'}] ${s.name} (${s.enrollments} enrollments)`);
    });

    const enrollments = await client.query(`
      SELECT status, COUNT(*) as count
      FROM sa_enrollments
      GROUP BY status
    `);
    console.log('\n  Enrollments by status:');
    enrollments.rows.forEach(r => {
      console.log(`    ${r.status}: ${r.count}`);
    });

    // Autonomous jobs
    const jobs = await client.query(`
      SELECT status, job_type, COUNT(*) as count
      FROM sa_autonomous_jobs
      GROUP BY status, job_type
      ORDER BY count DESC
    `);
    console.log('\n  Autonomous jobs:');
    jobs.rows.forEach(r => {
      console.log(`    [${r.status}] ${r.job_type}: ${r.count}`);
    });

    const seg6Pass = sequences.rows.length > 0;
    results.segments['6_sequencing'] = {
      status: seg6Pass ? 'PASS' : 'FAIL',
      sequences: sequences.rows.length,
      enrollments: enrollments.rows.reduce((sum, r) => sum + parseInt(r.count), 0)
    };
    if (seg6Pass) results.passed++; else results.failed++;
    console.log(`\n  ✓ Status: ${seg6Pass ? 'PASS' : 'FAIL'}`);

    // =========================================================================
    // SEGMENT 7: ML/OPTIMIZATION
    // =========================================================================
    console.log('\n' + '=' .repeat(80));
    console.log('SEGMENT 7: ML/OPTIMIZATION');
    console.log('=' .repeat(80));

    // Thompson Sampling (Bandit)
    const banditArms = await client.query(`
      SELECT arm_type, arm_name, total_successes as successes, total_trials - total_successes as failures,
             COALESCE(empirical_rate, 0) as success_rate,
             active
      FROM sa_bandit_arms
      ORDER BY empirical_rate DESC NULLS LAST
      LIMIT 10
    `);
    console.log(`\n  Thompson Sampling arms (top 10):`);
    banditArms.rows.forEach(a => {
      const total = parseInt(a.successes) + parseInt(a.failures);
      console.log(`    [${a.arm_type}] ${a.arm_name}: ${a.successes}/${total} (${(parseFloat(a.success_rate) * 100).toFixed(1)}%) ${a.active ? '' : '(retired)'}`);
    });

    const banditTrials = await client.query(`
      SELECT COUNT(*) as count FROM sa_bandit_trials
    `);
    console.log(`\n  Bandit trials: ${banditTrials.rows[0].count}`);

    // Bayesian training
    const bayesTraining = await client.query(`
      SELECT label, COUNT(*) as count
      FROM sa_bayes_training
      GROUP BY label
    `);
    console.log('\n  Bayesian training data:');
    bayesTraining.rows.forEach(r => {
      console.log(`    ${r.label}: ${r.count} records`);
    });

    // Heuristic scores
    const heuristics = await client.query(`
      SELECT tier, COUNT(*) as count, AVG(overall_score) as avg_score
      FROM sa_heuristic_scores
      GROUP BY tier
      ORDER BY avg_score DESC
    `);
    console.log('\n  Heuristic scores by tier:');
    heuristics.rows.forEach(r => {
      console.log(`    ${r.tier || 'untiered'}: ${r.count} contacts (avg score: ${parseFloat(r.avg_score || 0).toFixed(2)})`);
    });

    const seg7Pass = banditArms.rows.length > 0 || bayesTraining.rows.length > 0;
    results.segments['7_ml'] = {
      status: seg7Pass ? 'PASS' : 'WARN',
      banditArms: banditArms.rows.length,
      trials: parseInt(banditTrials.rows[0].count)
    };
    if (seg7Pass) results.passed++; else results.failed++;
    console.log(`\n  ✓ Status: ${seg7Pass ? 'PASS' : 'WARN'}`);

    // =========================================================================
    // SEGMENT 8: CONFIGURATION
    // =========================================================================
    console.log('\n' + '=' .repeat(80));
    console.log('SEGMENT 8: CONFIGURATION');
    console.log('=' .repeat(80));

    const config = await client.query(`
      SELECT key, value
      FROM sa_config
      ORDER BY key
    `);
    console.log(`\n  Configuration settings: ${config.rows.length}`);
    config.rows.forEach(c => {
      // Mask sensitive values
      const value = c.key.toLowerCase().includes('key') || c.key.toLowerCase().includes('secret') 
        ? '***masked***' 
        : c.value;
      console.log(`    ${c.key}: ${value}`);
    });

    // Inbox rules
    const inboxRules = await client.query(`
      SELECT COUNT(*) as count FROM sa_inbox_rules
    `);
    console.log(`\n  Inbox rules: ${inboxRules.rows[0].count}`);

    // Reputation alerts
    const alerts = await client.query(`
      SELECT type, severity, COUNT(*) as count
      FROM sa_reputation_alerts
      GROUP BY type, severity
      ORDER BY count DESC
    `);
    console.log('\n  Reputation alerts:');
    if (alerts.rows.length === 0) {
      console.log('    (none)');
    } else {
      alerts.rows.forEach(r => {
        console.log(`    [${r.severity}] ${r.type}: ${r.count}`);
      });
    }

    const seg8Pass = config.rows.length > 0;
    results.segments['8_config'] = {
      status: seg8Pass ? 'PASS' : 'FAIL',
      settings: config.rows.length
    };
    if (seg8Pass) results.passed++; else results.failed++;
    console.log(`\n  ✓ Status: ${seg8Pass ? 'PASS' : 'FAIL'}`);

    // =========================================================================
    // FINAL SUMMARY
    // =========================================================================
    console.log('\n' + '=' .repeat(80));
    console.log('SMOKE TEST SUMMARY');
    console.log('=' .repeat(80));
    
    console.log('\n  Segment Results:');
    Object.entries(results.segments).forEach(([seg, data]) => {
      const icon = data.status === 'PASS' ? '✅' : data.status === 'WARN' ? '⚠️' : '❌';
      console.log(`    ${icon} ${seg}: ${data.status}`);
    });

    console.log(`\n  Overall: ${results.passed}/${results.passed + results.failed} segments passed`);
    
    const overallStatus = results.failed === 0 ? 'ALL PASS' : 
                          results.failed <= 2 ? 'MOSTLY PASS' : 'NEEDS ATTENTION';
    console.log(`  Status: ${overallStatus}`);
    console.log('\n' + '=' .repeat(80));

  } catch (error) {
    console.error('ERROR:', error.message);
    process.exit(1);
  } finally {
    await client.end();
  }
}

main();
