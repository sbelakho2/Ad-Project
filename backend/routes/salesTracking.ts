// routes/salesTracking.ts
// API routes for tracking email interactions (opens, clicks, unsubscribes)
// These endpoints are called from emails sent by the sales automation system

import { Router, Request, Response } from 'express';
import { Pool } from 'pg';

// Allowed redirect domains for click tracking (prevents open redirect attacks)
const ALLOWED_REDIRECT_DOMAINS = new Set([
  'apexmediation.com',
  'apexmediation.ee',
  'console.apexmediation.ee',
  'api.apexmediation.ee',
]);

/**
 * Validates that a URL is safe to redirect to.
 * Only allows HTTPS URLs to known safe domains.
 */
function isAllowedRedirectUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    // Only allow HTTPS
    if (parsed.protocol !== 'https:') {
      return false;
    }
    // Check if the domain is in our allowlist
    const hostname = parsed.hostname.toLowerCase();
    if (ALLOWED_REDIRECT_DOMAINS.has(hostname)) {
      return true;
    }
    // Check for subdomains of allowed domains
    for (const domain of ALLOWED_REDIRECT_DOMAINS) {
      if (hostname.endsWith('.' + domain)) {
        return true;
      }
    }
    return false;
  } catch {
    return false;
  }
}

export function createSalesTrackingRouter(pool: Pool): Router {
  const router = Router();

  // 1x1 transparent pixel for tracking email opens
  const TRACKING_PIXEL = Buffer.from(
    'R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7',
    'base64'
  );

  /**
   * GET /track/open/:emailId
   * Records an email open event and returns a 1x1 transparent GIF
   */
  router.get('/track/open/:emailId', async (req: Request, res: Response) => {
    const { emailId } = req.params;

    try {
      // Record the open event (only first open counts)
      await pool.query(
        `UPDATE outreach_emails 
         SET opened_at = COALESCE(opened_at, NOW()),
             metadata = COALESCE(metadata, '{}'::jsonb) || jsonb_build_object(
               'opens', COALESCE((metadata->>'opens')::int, 0) + 1,
               'last_open_at', NOW()::text,
               'last_open_ip', $2,
               'last_open_ua', $3
             )
         WHERE id = $1`,
        [
          emailId,
          req.ip || req.socket.remoteAddress || 'unknown',
          req.headers['user-agent'] || 'unknown',
        ]
      );

      // Log activity
      await pool.query(
        `INSERT INTO sales_activities (contact_id, activity_type, subject, metadata)
         SELECT contact_id, 'email_opened', 'Opened email', jsonb_build_object('email_id', $1)
         FROM outreach_emails WHERE id = $1`,
        [emailId]
      );
    } catch (error) {
      // Don't fail the request on tracking errors
      console.error('Error tracking email open:', error);
    }

    // Always return the tracking pixel regardless of DB errors
    res.set({
      'Content-Type': 'image/gif',
      'Content-Length': TRACKING_PIXEL.length,
      'Cache-Control': 'no-store, no-cache, must-revalidate',
      'Pragma': 'no-cache',
      'Expires': '0',
    });
    res.send(TRACKING_PIXEL);
  });

  /**
   * GET /track/click/:emailId/:linkId
   * Records a link click and redirects to the target URL
   * 
   * SECURITY: Only allows redirects to pre-approved domains to prevent open redirect attacks
   */
  router.get('/track/click/:emailId/:linkId', async (req: Request, res: Response) => {
    const { emailId, linkId } = req.params;
    const targetUrl = req.query.url as string;

    if (!targetUrl) {
      return res.status(400).send('Missing URL parameter');
    }

    // SECURITY: Validate redirect URL to prevent open redirect attacks
    if (!isAllowedRedirectUrl(targetUrl)) {
      console.warn('Blocked redirect to unsafe URL:', { targetUrl, emailId, linkId });
      return res.status(400).send('Invalid redirect URL');
    }

    try {
      // Record the click
      await pool.query(
        `UPDATE outreach_emails 
         SET clicked_at = COALESCE(clicked_at, NOW()),
             metadata = COALESCE(metadata, '{}'::jsonb) || jsonb_build_object(
               'clicks', COALESCE((metadata->>'clicks')::int, 0) + 1,
               'last_click_at', NOW()::text,
               'last_click_link', $2
             )
         WHERE id = $1`,
        [emailId, linkId]
      );

      // Log activity
      await pool.query(
        `INSERT INTO sales_activities (contact_id, activity_type, subject, metadata)
         SELECT contact_id, 'email_clicked', 'Clicked link in email', 
                jsonb_build_object('email_id', $1, 'link_id', $2, 'url', $3)
         FROM outreach_emails WHERE id = $1`,
        [emailId, linkId, targetUrl]
      );
    } catch (error) {
      console.error('Error tracking click:', error);
    }

    // Redirect to the validated target URL
    res.redirect(302, targetUrl);
  });

  /**
   * GET /unsubscribe/:contactId
   * Shows unsubscribe confirmation page
   */
  router.get('/unsubscribe/:contactId', async (req: Request, res: Response) => {
    const { contactId } = req.params;

    try {
      const result = await pool.query(
        `SELECT c.email, c.first_name, co.name as company_name
         FROM sales_contacts c
         LEFT JOIN sales_companies co ON c.company_id = co.id
         WHERE c.id = $1`,
        [contactId]
      );

      if (result.rows.length === 0) {
        return res.status(404).send('Contact not found');
      }

      const contact = result.rows[0];
      const email = contact.email;
      const maskedEmail = email.replace(/(.{2})(.*)(@.*)/, '$1***$3');

      res.send(`
        <!DOCTYPE html>
        <html>
        <head>
          <title>Unsubscribe - ApexMediation</title>
          <meta name="viewport" content="width=device-width, initial-scale=1">
          <style>
            body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 500px; margin: 50px auto; padding: 20px; }
            .card { background: #f9fafb; border-radius: 8px; padding: 30px; text-align: center; }
            h1 { color: #111827; font-size: 24px; margin-bottom: 10px; }
            p { color: #6b7280; line-height: 1.6; }
            .email { font-weight: 600; color: #111827; }
            .btn { display: inline-block; background: #dc2626; color: white; padding: 12px 24px; border-radius: 6px; text-decoration: none; margin-top: 20px; }
            .btn:hover { background: #b91c1c; }
            .cancel { display: block; margin-top: 15px; color: #6b7280; text-decoration: none; }
            .cancel:hover { color: #111827; }
          </style>
        </head>
        <body>
          <div class="card">
            <h1>Unsubscribe</h1>
            <p>You're about to unsubscribe <span class="email">${maskedEmail}</span> from ApexMediation emails.</p>
            <form method="POST" action="/unsubscribe/${contactId}">
              <button type="submit" class="btn">Unsubscribe</button>
            </form>
            <a href="javascript:history.back()" class="cancel">Cancel</a>
          </div>
        </body>
        </html>
      `);
    } catch (error) {
      console.error('Error showing unsubscribe page:', error);
      res.status(500).send('An error occurred');
    }
  });

  /**
   * POST /unsubscribe/:contactId
   * Processes the unsubscribe request
   */
  router.post('/unsubscribe/:contactId', async (req: Request, res: Response) => {
    const { contactId } = req.params;

    try {
      // Mark contact as opted out
      await pool.query(
        `UPDATE sales_contacts 
         SET opted_out = true,
             opted_out_at = NOW(),
             updated_at = NOW()
         WHERE id = $1`,
        [contactId]
      );

      // Cancel any active enrollments
      await pool.query(
        `UPDATE outreach_enrollments 
         SET status = 'unsubscribed',
             updated_at = NOW()
         WHERE contact_id = $1 AND status IN ('active', 'paused')`,
        [contactId]
      );

      // Log activity
      await pool.query(
        `INSERT INTO sales_activities (contact_id, activity_type, subject, metadata)
         VALUES ($1, 'unsubscribed', 'Contact unsubscribed', jsonb_build_object('ip', $2))`,
        [contactId, req.ip || 'unknown']
      );

      res.send(`
        <!DOCTYPE html>
        <html>
        <head>
          <title>Unsubscribed - ApexMediation</title>
          <meta name="viewport" content="width=device-width, initial-scale=1">
          <style>
            body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 500px; margin: 50px auto; padding: 20px; }
            .card { background: #f9fafb; border-radius: 8px; padding: 30px; text-align: center; }
            h1 { color: #111827; font-size: 24px; margin-bottom: 10px; }
            p { color: #6b7280; line-height: 1.6; }
            .success { color: #059669; font-size: 48px; margin-bottom: 15px; }
          </style>
        </head>
        <body>
          <div class="card">
            <div class="success">✓</div>
            <h1>Unsubscribed</h1>
            <p>You've been successfully unsubscribed from ApexMediation emails. We're sorry to see you go!</p>
            <p style="margin-top: 20px; font-size: 14px;">If you unsubscribed by mistake, you can contact us at <a href="mailto:support@apexmediation.com">support@apexmediation.com</a>.</p>
          </div>
        </body>
        </html>
      `);
    } catch (error) {
      console.error('Error processing unsubscribe:', error);
      res.status(500).send('An error occurred processing your request');
    }
  });

  /**
   * GET /track/reply/:emailId
   * Webhook for processing email replies (called by email service)
   */
  router.post('/track/reply', async (req: Request, res: Response) => {
    // This would be called by an email service webhook (e.g., Resend webhook)
    const { email_id, from_email, subject, body_preview } = req.body;

    if (!email_id) {
      return res.status(400).json({ error: 'Missing email_id' });
    }

    try {
      // Record the reply
      await pool.query(
        `UPDATE outreach_emails 
         SET replied_at = NOW(),
             metadata = COALESCE(metadata, '{}'::jsonb) || jsonb_build_object(
               'reply_preview', $2,
               'reply_subject', $3
             )
         WHERE id = $1`,
        [email_id, body_preview?.substring(0, 200) || '', subject || '']
      );

      // Update enrollment status
      await pool.query(
        `UPDATE outreach_enrollments 
         SET status = 'replied',
             updated_at = NOW()
         FROM outreach_emails e
         WHERE outreach_enrollments.id = e.enrollment_id
           AND e.id = $1`,
        [email_id]
      );

      // Log activity
      await pool.query(
        `INSERT INTO sales_activities (contact_id, activity_type, subject, notes, metadata)
         SELECT contact_id, 'email_replied', $2, $3, 
                jsonb_build_object('email_id', $1)
         FROM outreach_emails WHERE id = $1`,
        [email_id, subject || 'Reply received', body_preview?.substring(0, 500) || '']
      );

      res.json({ success: true });
    } catch (error) {
      console.error('Error processing reply webhook:', error);
      res.status(500).json({ error: 'Internal error' });
    }
  });

  /**
   * GET /track/bounce
   * Webhook for processing email bounces
   */
  router.post('/track/bounce', async (req: Request, res: Response) => {
    const { email_id, bounce_type, bounce_reason } = req.body;

    if (!email_id) {
      return res.status(400).json({ error: 'Missing email_id' });
    }

    try {
      // Record the bounce
      await pool.query(
        `UPDATE outreach_emails 
         SET bounced_at = NOW(),
             metadata = COALESCE(metadata, '{}'::jsonb) || jsonb_build_object(
               'bounce_type', $2,
               'bounce_reason', $3
             )
         WHERE id = $1`,
        [email_id, bounce_type || 'unknown', bounce_reason || '']
      );

      // Mark contact email as invalid if hard bounce
      if (bounce_type === 'hard') {
        await pool.query(
          `UPDATE sales_contacts 
           SET email_verified = false,
               updated_at = NOW()
           FROM outreach_emails e
           WHERE sales_contacts.id = e.contact_id AND e.id = $1`,
          [email_id]
        );
      }

      // Update enrollment status
      await pool.query(
        `UPDATE outreach_enrollments 
         SET status = 'bounced',
             updated_at = NOW()
         FROM outreach_emails e
         WHERE outreach_enrollments.id = e.enrollment_id
           AND e.id = $1`,
        [email_id]
      );

      res.json({ success: true });
    } catch (error) {
      console.error('Error processing bounce webhook:', error);
      res.status(500).json({ error: 'Internal error' });
    }
  });

  return router;
}

export default createSalesTrackingRouter;
