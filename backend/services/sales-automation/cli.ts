#!/usr/bin/env npx ts-node
// services/sales-automation/cli.ts
// THE ACTUAL USABLE INTERFACE - Run sales automation from command line
// Usage: npx ts-node cli.ts <command> [options]

import { Pool } from 'pg';
import { ApexMailClient } from '../../src/services/apexmail-client';
import * as readline from 'readline';
import * as fs from 'fs';
import * as path from 'path';

// ============================================================================
// CONFIG
// ============================================================================

const DATABASE_URL = process.env.DATABASE_URL || '';
const APEXMAIL_URL = process.env.APEXMAIL_URL || '';
const DEFAULT_FROM_EMAIL = 'sales@apexmediation.ee';
const DEFAULT_FROM_NAME = 'Aaron from ApexMediation';
const DAILY_LIMIT = 500; // ApexMail self-hosted - higher limit

// ============================================================================
// DATABASE SETUP
// ============================================================================

const SCHEMA = `
-- Core tables for sales automation (minimal viable)
CREATE TABLE IF NOT EXISTS sa_contacts (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  email VARCHAR(255) UNIQUE NOT NULL,
  first_name VARCHAR(100),
  last_name VARCHAR(100),
  company_name VARCHAR(255),
  job_title VARCHAR(255),
  app_name VARCHAR(255),
  app_downloads VARCHAR(50),
  status VARCHAR(30) DEFAULT 'new', -- new, contacted, replied, converted, bounced, opted_out
  source VARCHAR(100),
  notes TEXT,
  score INTEGER DEFAULT 50,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sa_sequences (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name VARCHAR(255) NOT NULL,
  description TEXT,
  active BOOLEAN DEFAULT true,
  steps JSONB NOT NULL DEFAULT '[]', -- Array of {delay_days, subject, body}
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sa_enrollments (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  sequence_id UUID REFERENCES sa_sequences(id) ON DELETE CASCADE,
  status VARCHAR(30) DEFAULT 'active', -- active, paused, completed, replied, bounced
  current_step INTEGER DEFAULT 0,
  next_send_at TIMESTAMP,
  personalization JSONB DEFAULT '{}',
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(contact_id, sequence_id)
);

CREATE TABLE IF NOT EXISTS sa_emails (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  enrollment_id UUID REFERENCES sa_enrollments(id) ON DELETE CASCADE,
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  to_email VARCHAR(255) NOT NULL,
  from_email VARCHAR(255) NOT NULL,
  subject TEXT NOT NULL,
  body_html TEXT NOT NULL,
  message_id VARCHAR(255),
  status VARCHAR(30) DEFAULT 'pending', -- pending, sent, opened, clicked, replied, bounced
  step_number INTEGER,
  scheduled_for TIMESTAMP,
  sent_at TIMESTAMP,
  opened_at TIMESTAMP,
  clicked_at TIMESTAMP,
  replied_at TIMESTAMP,
  bounced_at TIMESTAMP,
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sa_activity_log (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  contact_id UUID REFERENCES sa_contacts(id) ON DELETE CASCADE,
  action VARCHAR(100) NOT NULL,
  details JSONB DEFAULT '{}',
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_sa_contacts_email ON sa_contacts(email);
CREATE INDEX IF NOT EXISTS idx_sa_contacts_status ON sa_contacts(status);
CREATE INDEX IF NOT EXISTS idx_sa_enrollments_next_send ON sa_enrollments(next_send_at) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_sa_emails_status ON sa_emails(status);
`;

// ============================================================================
// DEFAULT SEQUENCES (Actual usable templates)
// ============================================================================

const DEFAULT_SEQUENCES = [
  {
    name: 'Mobile App Publisher - Ad Mediation',
    description: 'For mobile app developers who show ads',
    steps: [
      {
        delay_days: 0,
        subject: '{{app_name}} ad revenue question',
        body: `Hi {{first_name}},

I noticed {{app_name}} is doing well with {{app_downloads}} downloads. Quick question - are you using a single ad network or mediation?

Most publishers we work with leave 20-40% of their ad revenue on the table by not optimizing their waterfall.

Would you be open to a 15-minute call to see if we could help improve your numbers?

Best,
Aaron`
      },
      {
        delay_days: 3,
        subject: 'Re: {{app_name}} ad revenue question',
        body: `Hi {{first_name}},

Following up on my last email. I understand you're busy building great apps.

Here's a quick case study: Similar gaming apps saw 35% revenue increase within 30 days of switching to our mediation.

Worth a quick chat?

Aaron`
      },
      {
        delay_days: 5,
        subject: 'One more thing about {{app_name}}',
        body: `Hi {{first_name}},

Last email - I promise!

If ad monetization isn't a priority right now, I totally understand. But if you're curious about how other {{app_downloads}}+ download apps optimize their stack, happy to share insights.

Either way, wishing you continued success with {{app_name}}.

Cheers,
Aaron`
      }
    ]
  },
  {
    name: 'Simple Introduction',
    description: 'Basic intro sequence for general outreach',
    steps: [
      {
        delay_days: 0,
        subject: 'Quick question for {{company_name}}',
        body: `Hi {{first_name}},

I came across {{company_name}} and wanted to reach out directly.

We help companies like yours improve their ad revenue without changing their current setup.

Would you be open to a brief call this week?

Best,
Aaron`
      },
      {
        delay_days: 4,
        subject: 'Re: Quick question for {{company_name}}',
        body: `Hi {{first_name}},

Just following up on my previous email.

I know you're busy, so I'll keep it short: we've helped similar companies increase revenue by 30%+ with minimal effort on their end.

5 minutes to chat?

Aaron`
      }
    ]
  }
];

// ============================================================================
// CORE FUNCTIONALITY
// ============================================================================

class SalesAutomationCLI {
  private pool: Pool;
  private apexmail: ApexMailClient | null = null;

  constructor() {
    if (!DATABASE_URL) {
      console.error('❌ DATABASE_URL not set');
      process.exit(1);
    }
    this.pool = new Pool({ connectionString: DATABASE_URL });
    
    if (APEXMAIL_URL) {
      this.apexmail = new ApexMailClient();
    } else {
      console.warn('⚠️  APEXMAIL_URL not set - email sending disabled');
    }
  }

  // ==========================================================================
  // SETUP COMMANDS
  // ==========================================================================

  async setup(): Promise<void> {
    console.log('🔧 Setting up sales automation database...\n');
    
    try {
      await this.pool.query(SCHEMA);
      console.log('✅ Database tables created\n');
      
      // Insert default sequences
      for (const seq of DEFAULT_SEQUENCES) {
        const existing = await this.pool.query(
          'SELECT id FROM sa_sequences WHERE name = $1',
          [seq.name]
        );
        
        if (existing.rows.length === 0) {
          await this.pool.query(
            `INSERT INTO sa_sequences (name, description, steps) VALUES ($1, $2, $3)`,
            [seq.name, seq.description, JSON.stringify(seq.steps)]
          );
          console.log(`  ✓ Created sequence: ${seq.name}`);
        } else {
          console.log(`  ⏭️  Sequence exists: ${seq.name}`);
        }
      }
      
      console.log('\n✅ Setup complete!');
      console.log('\nNext steps:');
      console.log('  1. Import contacts: npx ts-node cli.ts import <file.csv>');
      console.log('  2. Check status:    npx ts-node cli.ts status');
      console.log('  3. Run outreach:    npx ts-node cli.ts run');
    } catch (error) {
      console.error('❌ Setup failed:', error);
    }
  }

  // ==========================================================================
  // CONTACT MANAGEMENT
  // ==========================================================================

  async importContacts(filePath: string): Promise<void> {
    console.log(`📥 Importing contacts from ${filePath}...\n`);
    
    if (!fs.existsSync(filePath)) {
      console.error(`❌ File not found: ${filePath}`);
      return;
    }

    const content = fs.readFileSync(filePath, 'utf-8');
    const lines = content.split('\n').filter(line => line.trim());
    
    if (lines.length < 2) {
      console.error('❌ File must have header row and at least one contact');
      return;
    }

    const header = lines[0].toLowerCase().split(',').map(h => h.trim());
    const emailIdx = header.findIndex(h => h === 'email');
    
    if (emailIdx === -1) {
      console.error('❌ CSV must have "email" column');
      return;
    }

    let imported = 0;
    let skipped = 0;

    for (let i = 1; i < lines.length; i++) {
      const values = this.parseCSVLine(lines[i]);
      const contact: Record<string, string> = {};
      
      header.forEach((key, idx) => {
        if (values[idx]) {
          contact[key] = values[idx];
        }
      });

      if (!contact.email || !this.isValidEmail(contact.email)) {
        skipped++;
        continue;
      }

      try {
        await this.pool.query(`
          INSERT INTO sa_contacts (email, first_name, last_name, company_name, job_title, app_name, app_downloads, source)
          VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
          ON CONFLICT (email) DO UPDATE SET
            first_name = COALESCE(EXCLUDED.first_name, sa_contacts.first_name),
            last_name = COALESCE(EXCLUDED.last_name, sa_contacts.last_name),
            company_name = COALESCE(EXCLUDED.company_name, sa_contacts.company_name),
            updated_at = CURRENT_TIMESTAMP
        `, [
          contact.email,
          contact.first_name || contact.firstname || this.guessFirstName(contact.email),
          contact.last_name || contact.lastname || null,
          contact.company_name || contact.company || null,
          contact.job_title || contact.title || null,
          contact.app_name || contact.app || null,
          contact.app_downloads || contact.downloads || null,
          'csv_import'
        ]);
        imported++;
      } catch (error) {
        console.log(`  ⚠️  Failed to import: ${contact.email}`);
        skipped++;
      }
    }

    console.log(`\n✅ Import complete: ${imported} imported, ${skipped} skipped`);
  }

  async addContact(email: string, data: Partial<Contact> = {}): Promise<void> {
    if (!this.isValidEmail(email)) {
      console.error('❌ Invalid email address');
      return;
    }

    try {
      await this.pool.query(`
        INSERT INTO sa_contacts (email, first_name, last_name, company_name, job_title, app_name, app_downloads, source)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 'manual')
        ON CONFLICT (email) DO NOTHING
      `, [
        email,
        data.first_name || this.guessFirstName(email),
        data.last_name || null,
        data.company_name || null,
        data.job_title || null,
        data.app_name || null,
        data.app_downloads || null,
      ]);
      console.log(`✅ Added contact: ${email}`);
    } catch (error) {
      console.error('❌ Failed to add contact:', error);
    }
  }

  async listContacts(status?: string): Promise<void> {
    let query = 'SELECT * FROM sa_contacts';
    const params: string[] = [];
    
    if (status) {
      query += ' WHERE status = $1';
      params.push(status);
    }
    query += ' ORDER BY created_at DESC LIMIT 50';

    const result = await this.pool.query(query, params);
    
    console.log('\n📋 Contacts:\n');
    console.log('Email                           | Name            | Company         | Status     | Score');
    console.log('-'.repeat(95));
    
    for (const row of result.rows) {
      const email = (row.email || '').substring(0, 30).padEnd(30);
      const name = `${row.first_name || ''} ${row.last_name || ''}`.substring(0, 15).padEnd(15);
      const company = (row.company_name || '-').substring(0, 15).padEnd(15);
      const status = (row.status || 'new').padEnd(10);
      console.log(`${email} | ${name} | ${company} | ${status} | ${row.score}`);
    }
    
    console.log(`\nTotal: ${result.rows.length} contacts`);
  }

  // ==========================================================================
  // SEQUENCE MANAGEMENT
  // ==========================================================================

  async listSequences(): Promise<void> {
    const result = await this.pool.query(`
      SELECT s.*, 
        (SELECT COUNT(*) FROM sa_enrollments e WHERE e.sequence_id = s.id) as enrolled,
        (SELECT COUNT(*) FROM sa_enrollments e WHERE e.sequence_id = s.id AND e.status = 'replied') as replied
      FROM sa_sequences s
      ORDER BY created_at DESC
    `);
    
    console.log('\n📧 Sequences:\n');
    console.log('Name                                    | Steps | Enrolled | Replied | Active');
    console.log('-'.repeat(80));
    
    for (const row of result.rows) {
      const steps = Array.isArray(row.steps) ? row.steps.length : JSON.parse(row.steps || '[]').length;
      const name = row.name.substring(0, 38).padEnd(38);
      console.log(`${name} | ${String(steps).padEnd(5)} | ${String(row.enrolled).padEnd(8)} | ${String(row.replied).padEnd(7)} | ${row.active ? '✓' : '✗'}`);
    }
  }

  // ==========================================================================
  // ENROLLMENT
  // ==========================================================================

  async enrollContacts(sequenceName: string, status: string = 'new', limit: number = 10): Promise<void> {
    // Find sequence
    const seqResult = await this.pool.query(
      'SELECT * FROM sa_sequences WHERE name ILIKE $1 AND active = true',
      [`%${sequenceName}%`]
    );
    
    if (seqResult.rows.length === 0) {
      console.error('❌ Sequence not found or inactive');
      return;
    }
    
    const sequence = seqResult.rows[0];
    console.log(`\n📝 Enrolling contacts in: ${sequence.name}\n`);
    
    // Find eligible contacts
    const contactsResult = await this.pool.query(`
      SELECT c.* FROM sa_contacts c
      WHERE c.status = $1
        AND NOT EXISTS (
          SELECT 1 FROM sa_enrollments e 
          WHERE e.contact_id = c.id AND e.sequence_id = $2
        )
      ORDER BY c.score DESC
      LIMIT $3
    `, [status, sequence.id, limit]);
    
    if (contactsResult.rows.length === 0) {
      console.log('⚠️  No eligible contacts found');
      return;
    }

    let enrolled = 0;
    const steps = typeof sequence.steps === 'string' ? JSON.parse(sequence.steps) : sequence.steps;
    const firstStepDelay = steps[0]?.delay_days || 0;
    const firstSendAt = new Date(Date.now() + firstStepDelay * 24 * 60 * 60 * 1000);
    
    for (const contact of contactsResult.rows) {
      try {
        await this.pool.query(`
          INSERT INTO sa_enrollments (contact_id, sequence_id, current_step, next_send_at, personalization)
          VALUES ($1, $2, 0, $3, $4)
        `, [
          contact.id,
          sequence.id,
          firstSendAt,
          JSON.stringify({
            first_name: contact.first_name || 'there',
            company_name: contact.company_name || 'your company',
            app_name: contact.app_name || 'your app',
            app_downloads: contact.app_downloads || 'many'
          })
        ]);
        
        // Update contact status
        await this.pool.query(
          'UPDATE sa_contacts SET status = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2',
          ['contacted', contact.id]
        );
        
        console.log(`  ✓ Enrolled: ${contact.email}`);
        enrolled++;
      } catch (error) {
        console.log(`  ⚠️  Failed to enroll: ${contact.email}`);
      }
    }
    
    console.log(`\n✅ Enrolled ${enrolled} contacts`);
    console.log(`   First emails will send at: ${firstSendAt.toLocaleString()}`);
  }

  // ==========================================================================
  // OUTREACH EXECUTION
  // ==========================================================================

  async run(dryRun: boolean = false): Promise<void> {
    console.log(`\n🚀 Running outreach cycle${dryRun ? ' (DRY RUN)' : ''}...\n`);
    
    if (!this.apexmail && !dryRun) {
      console.error('❌ APEXMAIL_URL not configured. Use --dry-run to test.');
      return;
    }

    // Check daily limit
    const todaySentResult = await this.pool.query(`
      SELECT COUNT(*) as count FROM sa_emails 
      WHERE sent_at >= CURRENT_DATE
    `);
    const todaySent = parseInt(todaySentResult.rows[0].count);
    const remaining = DAILY_LIMIT - todaySent;
    
    console.log(`📊 Today's sending: ${todaySent}/${DAILY_LIMIT} (${remaining} remaining)\n`);
    
    if (remaining <= 0 && !dryRun) {
      console.log('⚠️  Daily limit reached. Try again tomorrow.');
      return;
    }

    // Get enrollments ready to send
    const enrollmentsResult = await this.pool.query(`
      SELECT e.*, c.email, c.first_name, s.name as sequence_name, s.steps
      FROM sa_enrollments e
      JOIN sa_contacts c ON e.contact_id = c.id
      JOIN sa_sequences s ON e.sequence_id = s.id
      WHERE e.status = 'active'
        AND e.next_send_at <= CURRENT_TIMESTAMP
        AND c.status NOT IN ('bounced', 'opted_out')
      ORDER BY e.next_send_at
      LIMIT $1
    `, [Math.min(remaining, 20)]);
    
    if (enrollmentsResult.rows.length === 0) {
      console.log('✅ No emails to send right now.');
      return;
    }

    let sent = 0;
    let errors = 0;

    for (const enrollment of enrollmentsResult.rows) {
      const steps = typeof enrollment.steps === 'string' 
        ? JSON.parse(enrollment.steps) 
        : enrollment.steps;
      const currentStep = enrollment.current_step;
      
      if (currentStep >= steps.length) {
        // Sequence complete
        await this.pool.query(
          'UPDATE sa_enrollments SET status = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2',
          ['completed', enrollment.id]
        );
        continue;
      }

      const step = steps[currentStep];
      const personalization = typeof enrollment.personalization === 'string'
        ? JSON.parse(enrollment.personalization)
        : enrollment.personalization;
      
      // Apply personalization
      const subject = this.applyPersonalization(step.subject, personalization);
      const body = this.applyPersonalization(step.body, personalization);
      
      console.log(`\n📤 Sending to: ${enrollment.email}`);
      console.log(`   Subject: ${subject}`);
      console.log(`   Step: ${currentStep + 1}/${steps.length}`);

      if (dryRun) {
        console.log('   [DRY RUN - not actually sent]');
        sent++;
        continue;
      }

      try {
        const result = await this.apexmail!.send({
          from: DEFAULT_FROM_EMAIL,
          fromName: DEFAULT_FROM_NAME,
          to: enrollment.email,
          subject: subject,
          html: body.replace(/\n/g, '<br>'),
          text: body,
          category: 'marketing',
        });

        // Record email
        await this.pool.query(`
          INSERT INTO sa_emails (enrollment_id, contact_id, to_email, from_email, subject, body_html, message_id, status, step_number, sent_at)
          VALUES ($1, $2, $3, $4, $5, $6, $7, 'sent', $8, CURRENT_TIMESTAMP)
        `, [
          enrollment.id,
          enrollment.contact_id,
          enrollment.email,
          DEFAULT_FROM_EMAIL,
          subject,
          body,
          result.emailId || null,
          currentStep + 1
        ]);

        // Update enrollment for next step
        const nextStep = currentStep + 1;
        if (nextStep < steps.length) {
          const nextDelay = steps[nextStep].delay_days || 3;
          const nextSendAt = new Date(Date.now() + nextDelay * 24 * 60 * 60 * 1000);
          
          await this.pool.query(`
            UPDATE sa_enrollments 
            SET current_step = $1, next_send_at = $2, updated_at = CURRENT_TIMESTAMP
            WHERE id = $3
          `, [nextStep, nextSendAt, enrollment.id]);
        } else {
          await this.pool.query(`
            UPDATE sa_enrollments 
            SET status = 'completed', current_step = $1, updated_at = CURRENT_TIMESTAMP
            WHERE id = $2
          `, [nextStep, enrollment.id]);
        }

        // Log activity
        await this.pool.query(`
          INSERT INTO sa_activity_log (contact_id, action, details)
          VALUES ($1, 'email_sent', $2)
        `, [enrollment.contact_id, JSON.stringify({ subject, step: currentStep + 1 })]);

        sent++;
        console.log('   ✓ Sent successfully');
        
        // Rate limit: wait between emails
        await this.sleep(1000);
      } catch (error) {
        console.log(`   ❌ Failed to send: ${error}`);
        errors++;
      }
    }

    console.log(`\n✅ Outreach complete: ${sent} sent, ${errors} errors`);
  }

  // ==========================================================================
  // STATUS & REPORTING
  // ==========================================================================

  async status(): Promise<void> {
    console.log('\n📊 Sales Automation Status\n');
    console.log('='.repeat(50));
    
    // Contacts
    const contactStats = await this.pool.query(`
      SELECT status, COUNT(*) as count 
      FROM sa_contacts 
      GROUP BY status
    `);
    
    console.log('\n📋 Contacts:');
    let totalContacts = 0;
    for (const row of contactStats.rows) {
      console.log(`   ${row.status}: ${row.count}`);
      totalContacts += parseInt(row.count);
    }
    console.log(`   Total: ${totalContacts}`);
    
    // Enrollments
    const enrollStats = await this.pool.query(`
      SELECT status, COUNT(*) as count 
      FROM sa_enrollments 
      GROUP BY status
    `);
    
    console.log('\n📧 Enrollments:');
    for (const row of enrollStats.rows) {
      console.log(`   ${row.status}: ${row.count}`);
    }
    
    // Emails
    const emailStats = await this.pool.query(`
      SELECT 
        COUNT(*) as total,
        COUNT(*) FILTER (WHERE sent_at IS NOT NULL) as sent,
        COUNT(*) FILTER (WHERE opened_at IS NOT NULL) as opened,
        COUNT(*) FILTER (WHERE clicked_at IS NOT NULL) as clicked,
        COUNT(*) FILTER (WHERE replied_at IS NOT NULL) as replied,
        COUNT(*) FILTER (WHERE bounced_at IS NOT NULL) as bounced
      FROM sa_emails
    `);
    
    const stats = emailStats.rows[0];
    console.log('\n📨 Emails:');
    console.log(`   Total: ${stats.total}`);
    console.log(`   Sent: ${stats.sent}`);
    console.log(`   Opened: ${stats.opened} (${stats.sent > 0 ? ((stats.opened / stats.sent) * 100).toFixed(1) : 0}%)`);
    console.log(`   Clicked: ${stats.clicked}`);
    console.log(`   Replied: ${stats.replied}`);
    console.log(`   Bounced: ${stats.bounced}`);
    
    // Today
    const todayStats = await this.pool.query(`
      SELECT COUNT(*) as count FROM sa_emails WHERE sent_at >= CURRENT_DATE
    `);
    console.log(`\n📅 Today: ${todayStats.rows[0].count}/${DAILY_LIMIT} emails sent`);
    
    // Pending
    const pendingResult = await this.pool.query(`
      SELECT COUNT(*) as count FROM sa_enrollments 
      WHERE status = 'active' AND next_send_at <= CURRENT_TIMESTAMP
    `);
    console.log(`⏳ Pending to send: ${pendingResult.rows[0].count}`);
    
    console.log('\n' + '='.repeat(50));
  }

  // ==========================================================================
  // WEBHOOK HANDLER (for ApexMail events)
  // ==========================================================================

  async handleWebhook(eventType: string, data: any): Promise<void> {
    const messageId = data.email_id || data.message_id;
    
    if (!messageId) {
      console.log('⚠️  Webhook missing message_id');
      return;
    }

    const emailResult = await this.pool.query(
      'SELECT * FROM sa_emails WHERE message_id = $1',
      [messageId]
    );
    
    if (emailResult.rows.length === 0) {
      console.log(`⚠️  Email not found for message_id: ${messageId}`);
      return;
    }

    const email = emailResult.rows[0];
    
    switch (eventType) {
      case 'email.opened':
        await this.pool.query(
          'UPDATE sa_emails SET opened_at = COALESCE(opened_at, CURRENT_TIMESTAMP), status = $1 WHERE id = $2',
          ['opened', email.id]
        );
        console.log(`📬 Email opened: ${email.to_email}`);
        break;
        
      case 'email.clicked':
        await this.pool.query(
          'UPDATE sa_emails SET clicked_at = COALESCE(clicked_at, CURRENT_TIMESTAMP), status = $1 WHERE id = $2',
          ['clicked', email.id]
        );
        console.log(`🖱️  Email clicked: ${email.to_email}`);
        break;
        
      case 'email.bounced':
        await this.pool.query(
          'UPDATE sa_emails SET bounced_at = CURRENT_TIMESTAMP, status = $1 WHERE id = $2',
          ['bounced', email.id]
        );
        await this.pool.query(
          'UPDATE sa_contacts SET status = $1 WHERE id = $2',
          ['bounced', email.contact_id]
        );
        await this.pool.query(
          'UPDATE sa_enrollments SET status = $1 WHERE contact_id = $2',
          ['bounced', email.contact_id]
        );
        console.log(`❌ Email bounced: ${email.to_email}`);
        break;
        
      case 'email.complained':
        await this.pool.query(
          'UPDATE sa_contacts SET status = $1 WHERE id = $2',
          ['opted_out', email.contact_id]
        );
        await this.pool.query(
          'UPDATE sa_enrollments SET status = $1 WHERE contact_id = $2',
          ['opted_out', email.contact_id]
        );
        console.log(`🚫 Spam complaint: ${email.to_email}`);
        break;
    }
  }

  // ==========================================================================
  // HELPERS
  // ==========================================================================

  private parseCSVLine(line: string): string[] {
    const result: string[] = [];
    let current = '';
    let inQuotes = false;
    
    for (const char of line) {
      if (char === '"') {
        inQuotes = !inQuotes;
      } else if (char === ',' && !inQuotes) {
        result.push(current.trim());
        current = '';
      } else {
        current += char;
      }
    }
    result.push(current.trim());
    return result;
  }

  private isValidEmail(email: string): boolean {
    return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
  }

  private guessFirstName(email: string): string {
    const local = email.split('@')[0];
    const name = local.split(/[._-]/)[0];
    return name.charAt(0).toUpperCase() + name.slice(1).toLowerCase();
  }

  private applyPersonalization(template: string, data: Record<string, string>): string {
    let result = template;
    for (const [key, value] of Object.entries(data)) {
      // Escape regex special characters in key to prevent ReDoS
      const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      result = result.replace(new RegExp(`{{${escapedKey}}}`, 'g'), value || '');
    }
    return result;
  }

  private sleep(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }

  async close(): Promise<void> {
    await this.pool.end();
  }
}

// ============================================================================
// CLI INTERFACE
// ============================================================================

interface Contact {
  first_name?: string;
  last_name?: string;
  company_name?: string;
  job_title?: string;
  app_name?: string;
  app_downloads?: string;
}

async function main() {
  const args = process.argv.slice(2);
  const command = args[0];
  
  if (!command) {
    showHelp();
    return;
  }

  const cli = new SalesAutomationCLI();

  try {
    switch (command) {
      case 'setup':
        await cli.setup();
        break;
        
      case 'import':
        if (!args[1]) {
          console.error('Usage: cli.ts import <file.csv>');
          return;
        }
        await cli.importContacts(args[1]);
        break;
        
      case 'add':
        if (!args[1]) {
          console.error('Usage: cli.ts add <email> [--name "First Last"] [--company "Company"]');
          return;
        }
        const data: Contact = {};
        for (let i = 2; i < args.length; i += 2) {
          if (args[i] === '--name') data.first_name = args[i + 1]?.split(' ')[0];
          if (args[i] === '--company') data.company_name = args[i + 1];
          if (args[i] === '--app') data.app_name = args[i + 1];
        }
        await cli.addContact(args[1], data);
        break;
        
      case 'contacts':
        await cli.listContacts(args[1]);
        break;
        
      case 'sequences':
        await cli.listSequences();
        break;
        
      case 'enroll':
        if (!args[1]) {
          console.error('Usage: cli.ts enroll <sequence-name> [status=new] [limit=10]');
          return;
        }
        await cli.enrollContacts(args[1], args[2] || 'new', parseInt(args[3]) || 10);
        break;
        
      case 'run':
        const dryRun = args.includes('--dry-run');
        await cli.run(dryRun);
        break;
        
      case 'status':
        await cli.status();
        break;
        
      case 'help':
      default:
        showHelp();
    }
  } finally {
    await cli.close();
  }
}

function showHelp() {
  console.log(`
📧 Sales Automation CLI
=======================

Usage: npx ts-node cli.ts <command> [options]

Commands:
  setup                     Initialize database and default sequences
  import <file.csv>         Import contacts from CSV file
  add <email> [options]     Add a single contact
  contacts [status]         List contacts (optionally filter by status)
  sequences                 List available sequences
  enroll <name> [status] [limit]  Enroll contacts in a sequence
  run [--dry-run]           Send pending emails
  status                    Show overall status

Examples:
  npx ts-node cli.ts setup
  npx ts-node cli.ts import leads.csv
  npx ts-node cli.ts add john@company.com --name "John Doe" --company "Acme Inc"
  npx ts-node cli.ts enroll "Mobile App" new 5
  npx ts-node cli.ts run --dry-run
  npx ts-node cli.ts status

CSV Format:
  Required: email
  Optional: first_name, last_name, company_name, job_title, app_name, app_downloads

Environment Variables:
  DATABASE_URL      PostgreSQL connection string (required)
  APEXMAIL_URL      ApexMail API URL for sending emails
`);
}

main().catch(console.error);
