/**
 * ApexMail Service Tests
 *
 * Tests for the self-hosted email infrastructure
 */

import { Pool } from 'pg';
import { ApexMailService, createApexMail } from '../src/services/apexmail';

// Mock nodemailer
jest.mock('nodemailer', () => ({
  createTransport: jest.fn(() => ({
    sendMail: jest.fn().mockResolvedValue({
      messageId: 'mock-message-id-123',
      response: '250 OK',
    }),
    verify: jest.fn().mockResolvedValue(true),
    close: jest.fn(),
  })),
}));

// Mock pool
const mockQuery = jest.fn();
const mockPool = {
  query: mockQuery,
} as unknown as Pool;

describe('ApexMailService', () => {
  let apexmail: ApexMailService;

  beforeEach(() => {
    jest.clearAllMocks();
    apexmail = createApexMail(mockPool, {
      smtpHost: 'localhost',
      smtpPort: 587,
      defaultFrom: 'test@apexmediation.ee',
      defaultFromName: 'Test',
      publicOrigin: 'https://test.apexmediation.ee',
    });
  });

  afterEach(() => {
    apexmail.stopQueueProcessor();
  });

  describe('send', () => {
    beforeEach(() => {
      // Mock suppression check - no suppressions
      mockQuery.mockResolvedValueOnce({ rows: [] });
      // Mock domain health - healthy
      mockQuery.mockResolvedValueOnce({ rows: [] });
    });

    it('should queue an email successfully', async () => {
      // Mock queue insert
      mockQuery.mockResolvedValueOnce({ rows: [{ id: 'email-uuid-123' }] });

      const result = await apexmail.send({
        to: 'recipient@example.com',
        subject: 'Test Subject',
        html: '<p>Test body</p>',
      });

      expect(result.success).toBe(true);
      expect(result.emailId).toBe('email-uuid-123');
      expect(result.trackingId).toMatch(/^am_/);
      expect(result.queued).toBe(true);
    });

    it('should send immediately for high priority emails', async () => {
      // Mock queue insert
      mockQuery.mockResolvedValueOnce({ rows: [{ id: 'email-uuid-456' }] });
      // Mock email fetch for sendNow
      mockQuery.mockResolvedValueOnce({
        rows: [
          {
            id: 'email-uuid-456',
            from_email: 'test@apexmediation.ee',
            from_name: 'Test',
            to_email: 'recipient@example.com',
            subject: 'Test',
            body_html: '<p>Test</p>',
            body_text: 'Test',
            headers: {},
          },
        ],
      });

      const result = await apexmail.send({
        to: 'recipient@example.com',
        subject: 'Urgent Test',
        html: '<p>Urgent body</p>',
        priority: 1,
      });

      expect(result.success).toBe(true);
      expect(result.queued).toBeFalsy();
      expect(result.messageId).toBe('mock-message-id-123');
    });

    it('should reject suppressed recipients', async () => {
      // Reset mocks and add suppressed email
      mockQuery.mockReset();
      mockQuery.mockResolvedValueOnce({ rows: [{ email: 'suppressed@example.com' }] });

      const result = await apexmail.send({
        to: 'suppressed@example.com',
        subject: 'Test',
        html: '<p>Test</p>',
      });

      expect(result.success).toBe(false);
      expect(result.error).toContain('suppressed');
    });

    it('should reject when domain is suspended', async () => {
      // Reset mocks
      mockQuery.mockReset();
      // No suppressions
      mockQuery.mockResolvedValueOnce({ rows: [] });
      // Domain is suspended
      mockQuery.mockResolvedValueOnce({
        rows: [
          {
            status: 'suspended',
            bounce_rate_7d: '0.15',
            complaint_rate_7d: '0.002',
            suspension_reason: 'Bounce rate too high',
          },
        ],
      });

      const result = await apexmail.send({
        to: 'recipient@example.com',
        subject: 'Test',
        html: '<p>Test</p>',
      });

      expect(result.success).toBe(false);
      expect(result.error).toContain('suspended');
    });
  });

  describe('renderTemplate', () => {
    it('should render a template with data', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [
          {
            subject_template: 'Hello {{name}}',
            body_html_template: '<h1>Welcome {{name}}!</h1><p>Your code is {{code}}</p>',
            body_text_template: 'Welcome {{name}}! Your code is {{code}}',
          },
        ],
      });

      const result = await apexmail.renderTemplate('welcome', {
        name: 'John',
        code: '12345',
      });

      expect(result.subject).toBe('Hello John');
      expect(result.html).toBe('<h1>Welcome John!</h1><p>Your code is 12345</p>');
      expect(result.text).toBe('Welcome John! Your code is 12345');
    });

    it('should throw if template not found', async () => {
      mockQuery.mockResolvedValueOnce({ rows: [] });

      await expect(
        apexmail.renderTemplate('nonexistent', {})
      ).rejects.toThrow('Template not found');
    });

    it('should handle nested properties', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [
          {
            subject_template: 'Order {{order.id}}',
            body_html_template: '<p>{{user.name}} ordered {{order.product}}</p>',
            body_text_template: null,
          },
        ],
      });

      const result = await apexmail.renderTemplate('order', {
        user: { name: 'Alice' },
        order: { id: '123', product: 'Widget' },
      });

      expect(result.subject).toBe('Order 123');
      expect(result.html).toBe('<p>Alice ordered Widget</p>');
    });
  });

  describe('recordEvent', () => {
    it('should record an event in the database', async () => {
      mockQuery.mockResolvedValueOnce({ rowCount: 1 });

      await apexmail.recordEvent({
        type: 'opened',
        emailId: 'email-123',
        trackingId: 'am_tracking_123',
        occurredAt: new Date('2025-01-10T12:00:00Z'),
        metadata: { ip: '192.168.1.1', userAgent: 'Mozilla/5.0' },
      });

      expect(mockQuery).toHaveBeenCalledWith(
        expect.stringContaining('INSERT INTO apexmail_events'),
        expect.arrayContaining(['email-123', 'am_tracking_123', 'opened'])
      );
    });

    it('should add to suppression list on hard bounce', async () => {
      // Insert event
      mockQuery.mockResolvedValueOnce({ rowCount: 1 });
      // Get email for domain update
      mockQuery.mockResolvedValueOnce({
        rows: [{ from_email: 'sender@apexmediation.ee' }],
      });
      // Upsert domain health
      mockQuery.mockResolvedValueOnce({ rowCount: 1 });
      // Update domain metrics
      mockQuery.mockResolvedValueOnce({ rowCount: 1 });
      // Check if should suspend
      mockQuery.mockResolvedValueOnce({
        rows: [{ status: 'healthy', bounce_rate_7d: '0.01', complaint_rate_7d: '0' }],
      });
      // Add suppression
      mockQuery.mockResolvedValueOnce({ rowCount: 1 });

      await apexmail.recordEvent({
        type: 'bounced',
        emailId: 'email-123',
        trackingId: 'am_tracking_123',
        occurredAt: new Date(),
        metadata: { bounceType: 'hard', email: 'bounced@example.com' },
      });

      // Verify suppression was added
      expect(mockQuery).toHaveBeenCalledWith(
        expect.stringContaining('INSERT INTO apexmail_suppressions'),
        expect.arrayContaining(['bounced@example.com', 'hard_bounce'])
      );
    });
  });

  describe('getStats', () => {
    it('should return aggregated statistics', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [
          { event_type: 'sent', count: '100' },
          { event_type: 'opened', count: '45' },
          { event_type: 'clicked', count: '15' },
          { event_type: 'bounced', count: '5' },
        ],
      });

      const stats = await apexmail.getStats();

      expect(stats.sent).toBe(100);
      expect(stats.opened).toBe(45);
      expect(stats.clicked).toBe(15);
      expect(stats.bounced).toBe(5);
      expect(stats.openRate).toBe(0.45);
      expect(stats.clickRate).toBe(0.15);
      expect(stats.bounceRate).toBe(0.05);
    });
  });

  describe('getQueueStatus', () => {
    it('should return queue counts', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [
          {
            queued: '10',
            sending: '2',
            deferred: '3',
            failed: '1',
            sent_24h: '500',
          },
        ],
      });

      const status = await apexmail.getQueueStatus();

      expect(status.queued).toBe(10);
      expect(status.sending).toBe(2);
      expect(status.deferred).toBe(3);
      expect(status.failed).toBe(1);
      expect(status.sent24h).toBe(500);
    });
  });

  describe('suppressions', () => {
    it('should suppress an email', async () => {
      mockQuery.mockResolvedValueOnce({ rowCount: 1 });

      await apexmail.suppress('spam@example.com', 'manual');

      expect(mockQuery).toHaveBeenCalledWith(
        expect.stringContaining('INSERT INTO apexmail_suppressions'),
        ['spam@example.com', 'manual']
      );
    });

    it('should unsuppress an email', async () => {
      mockQuery.mockResolvedValueOnce({ rowCount: 1 });

      const removed = await apexmail.unsuppress('unspam@example.com');

      expect(removed).toBe(true);
      expect(mockQuery).toHaveBeenCalledWith(
        expect.stringContaining('DELETE FROM apexmail_suppressions'),
        ['unspam@example.com']
      );
    });

    it('should check if email is suppressed', async () => {
      mockQuery.mockResolvedValueOnce({ rows: [{ email: 'suppressed@example.com' }] });

      const isSuppressed = await apexmail.isSuppressed('suppressed@example.com');

      expect(isSuppressed).toBe(true);
    });

    it('should return suppressions list', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [
          {
            email: 'spam1@example.com',
            reason: 'hard_bounce',
            source: 'automatic',
            created_at: new Date('2025-01-10'),
          },
          {
            email: 'spam2@example.com',
            reason: 'unsubscribe',
            source: 'user',
            created_at: new Date('2025-01-09'),
          },
        ],
      });

      const suppressions = await apexmail.getSuppressions();

      expect(suppressions).toHaveLength(2);
      expect(suppressions[0].email).toBe('spam1@example.com');
      expect(suppressions[0].reason).toBe('hard_bounce');
    });
  });

  describe('templates', () => {
    it('should get all templates', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [
          { id: '1', name: 'welcome', version: 2, category: 'transactional', active: true },
          { id: '2', name: 'password-reset', version: 1, category: 'transactional', active: true },
        ],
      });

      const templates = await apexmail.getTemplates();

      expect(templates).toHaveLength(2);
      expect(templates[0].name).toBe('welcome');
    });

    it('should create a new template', async () => {
      // Get current version
      mockQuery.mockResolvedValueOnce({ rows: [] });
      // Deactivate old
      mockQuery.mockResolvedValueOnce({ rowCount: 0 });
      // Insert new
      mockQuery.mockResolvedValueOnce({ rows: [{ id: 'new-template-id' }] });

      const id = await apexmail.upsertTemplate({
        name: 'new-template',
        subjectTemplate: 'Hello',
        bodyHtmlTemplate: '<p>Hello</p>',
        category: 'transactional',
      });

      expect(id).toBe('new-template-id');
    });

    it('should version existing template', async () => {
      // Get current version (v2)
      mockQuery.mockResolvedValueOnce({ rows: [{ version: 2 }] });
      // Deactivate old
      mockQuery.mockResolvedValueOnce({ rowCount: 2 });
      // Insert new (v3)
      mockQuery.mockResolvedValueOnce({ rows: [{ id: 'template-v3' }] });

      const id = await apexmail.upsertTemplate({
        name: 'existing-template',
        subjectTemplate: 'Updated Subject',
        bodyHtmlTemplate: '<p>Updated</p>',
      });

      expect(id).toBe('template-v3');
      // Verify version 3 was inserted
      expect(mockQuery).toHaveBeenCalledWith(
        expect.stringContaining('INSERT INTO apexmail_templates'),
        expect.arrayContaining(['existing-template', 3])
      );
    });
  });

  describe('domain health', () => {
    it('should get all domain health', async () => {
      mockQuery.mockResolvedValueOnce({
        rows: [
          {
            domain: 'apexmediation.ee',
            status: 'healthy',
            sent_7d: 1000,
            bounce_rate_7d: '0.02',
            complaint_rate_7d: '0.001',
          },
        ],
      });

      const health = await apexmail.getAllDomainHealth();

      expect(health).toHaveLength(1);
      expect(health[0].domain).toBe('apexmediation.ee');
      expect(health[0].status).toBe('healthy');
      expect(health[0].bounceRate).toBe(0.02);
    });

    it('should unsuspend a domain', async () => {
      mockQuery.mockResolvedValueOnce({ rowCount: 1 });

      await apexmail.unsuspendDomain('suspended.example.com');

      expect(mockQuery).toHaveBeenCalledWith(
        expect.stringContaining("status = 'healthy'"),
        ['suspended.example.com']
      );
    });
  });

  describe('verifyConnection', () => {
    it('should verify SMTP connection', async () => {
      const result = await apexmail.verifyConnection();
      expect(result).toBe(true);
    });
  });
});

describe('ApexMail HTML utilities', () => {
  let apexmail: ApexMailService;

  beforeEach(() => {
    apexmail = createApexMail(mockPool, {
      publicOrigin: 'https://test.apexmediation.ee',
    });
  });

  afterEach(() => {
    apexmail.stopQueueProcessor();
  });

  // We can test the htmlToText indirectly through send
  it('should convert HTML to plain text when text not provided', async () => {
    // Mock all the queries needed for send
    mockQuery.mockReset();
    mockQuery.mockResolvedValueOnce({ rows: [] }); // suppressions
    mockQuery.mockResolvedValueOnce({ rows: [] }); // domain health
    mockQuery.mockResolvedValueOnce({ rows: [{ id: 'test-id' }] }); // queue insert

    const result = await apexmail.send({
      to: 'test@example.com',
      subject: 'Test',
      html: '<h1>Hello</h1><p>This is a test.</p><br><p>Bye!</p>',
    });

    expect(result.success).toBe(true);

    // The queueEmail call should have converted HTML to text
    const insertCall = mockQuery.mock.calls.find((call) =>
      call[0].includes('INSERT INTO apexmail_queue')
    );
    expect(insertCall).toBeDefined();
    // The text param (index 6) should be plain text
    const textParam = insertCall![1][6];
    expect(textParam).toContain('Hello');
    expect(textParam).toContain('This is a test.');
    expect(textParam).not.toContain('<h1>');
  });
});
