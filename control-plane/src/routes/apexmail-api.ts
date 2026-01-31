/**
 * ApexMail API Routes
 *
 * Provides:
 * - Tracking endpoints (open pixel, click redirect)
 * - Unsubscribe handling
 * - Admin dashboard data
 * - Queue management
 */

import { Router, Request, Response } from 'express';
import { Pool } from 'pg';
import { ApexMailService, getApexMail } from '../services/apexmail';

export function createApexMailRouter(pool: Pool): Router {
  const router = Router();
  const apexmail = getApexMail(pool);

  // Start queue processor
  apexmail.startQueueProcessor();

  // ==========================================================================
  // TRACKING ENDPOINTS (No auth required - embedded in emails)
  // ==========================================================================

  /**
   * GET /api/apexmail/track/:trackingId/open.gif
   * Tracking pixel for email opens
   */
  router.get('/track/:trackingId/open.gif', async (req: Request, res: Response) => {
    const { trackingId } = req.params;

    try {
      // Record open event asynchronously
      const email = await pool.query(
        `SELECT id FROM apexmail_queue WHERE tracking_id = $1`,
        [trackingId]
      );

      if (email.rows.length > 0) {
        // Fire and forget - don't block response
        apexmail
          .recordEvent({
            type: 'opened',
            emailId: email.rows[0].id,
            trackingId,
            occurredAt: new Date(),
            metadata: {
              ip: req.ip || req.socket.remoteAddress,
              userAgent: req.headers['user-agent'],
            },
          })
          .catch((err) => console.error('[ApexMail] Open tracking error:', err));
      }
    } catch (err) {
      console.error('[ApexMail] Open tracking error:', err);
    }

    // Return 1x1 transparent GIF
    const pixel = Buffer.from(
      'R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7',
      'base64'
    );

    res.set({
      'Content-Type': 'image/gif',
      'Content-Length': String(pixel.length),
      'Cache-Control': 'no-store, no-cache, must-revalidate, proxy-revalidate',
      Pragma: 'no-cache',
      Expires: '0',
    });
    res.send(pixel);
  });

  /**
   * GET /api/apexmail/track/:trackingId/click/:shortCode
   * Click tracking redirect
   */
  router.get(
    '/track/:trackingId/click/:shortCode',
    async (req: Request, res: Response): Promise<void> => {
      const { trackingId, shortCode } = req.params;

      try {
        // Look up original URL
        const link = await pool.query(
          `SELECT original_url, email_id FROM apexmail_links WHERE short_code = $1`,
          [shortCode]
        );

        if (link.rows.length === 0) {
          res.status(404).send('Link not found');
          return;
        }

        const { original_url, email_id } = link.rows[0];

        // Record click event and update counter (fire and forget)
        pool
          .query(
            `UPDATE apexmail_links SET click_count = click_count + 1 WHERE short_code = $1`,
            [shortCode]
          )
          .catch((err) => console.error('[ApexMail] Click count error:', err));

        if (email_id) {
          apexmail
            .recordEvent({
              type: 'clicked',
              emailId: email_id,
              trackingId,
              occurredAt: new Date(),
              metadata: {
                ip: req.ip || req.socket.remoteAddress,
                userAgent: req.headers['user-agent'],
                linkUrl: original_url,
              },
            })
            .catch((err) => console.error('[ApexMail] Click tracking error:', err));
        }

        // Redirect to original URL
        res.redirect(302, original_url);
      } catch (err) {
        console.error('[ApexMail] Click redirect error:', err);
        res.status(500).send('Internal server error');
      }
    }
  );

  /**
   * GET/POST /api/apexmail/unsubscribe/:trackingId
   * One-click unsubscribe (RFC 8058 compliant)
   */
  router.all('/unsubscribe/:trackingId', async (req: Request, res: Response): Promise<void> => {
    const { trackingId } = req.params;

    try {
      const email = await pool.query(
        `SELECT id, to_email, entity_type, entity_id FROM apexmail_queue WHERE tracking_id = $1`,
        [trackingId]
      );

      if (email.rows.length === 0) {
        res.status(404).send(`
          <!DOCTYPE html>
          <html>
          <head><title>Not Found</title></head>
          <body style="font-family: Arial; text-align: center; padding: 50px;">
            <h1>Email Not Found</h1>
            <p>This unsubscribe link may have expired.</p>
          </body>
          </html>
        `);
        return;
      }

      const { id, to_email, entity_type, entity_id } = email.rows[0];

      // Add to suppression list
      await pool.query(
        `
        INSERT INTO apexmail_suppressions (email, reason, source)
        VALUES ($1, 'unsubscribe', $2)
        ON CONFLICT (email) DO NOTHING
      `,
        [to_email.toLowerCase(), `user:${trackingId}`]
      );

      // If linked to a contact, update contact status
      if (entity_type === 'contact' && entity_id) {
        await pool
          .query(
            `
          UPDATE sa_contacts SET unsubscribed = true, unsubscribed_at = NOW()
          WHERE id = $1
        `,
            [entity_id]
          )
          .catch(() => {
            /* Table might not exist in all environments */
          });
      }

      // Record event
      await apexmail.recordEvent({
        type: 'unsubscribed',
        emailId: id,
        trackingId,
        occurredAt: new Date(),
        metadata: {
          email: to_email,
        },
      });

      // Show confirmation page
      res.send(`
        <!DOCTYPE html>
        <html>
        <head>
          <title>Unsubscribed</title>
          <meta charset="utf-8">
          <meta name="viewport" content="width=device-width, initial-scale=1">
          <style>
            body {
              font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
              display: flex;
              justify-content: center;
              align-items: center;
              min-height: 100vh;
              margin: 0;
              background: #f5f5f5;
            }
            .container {
              text-align: center;
              padding: 40px;
              background: white;
              border-radius: 8px;
              box-shadow: 0 2px 10px rgba(0,0,0,0.1);
              max-width: 400px;
            }
            .checkmark {
              font-size: 64px;
              color: #28a745;
            }
            h1 { color: #333; margin-bottom: 10px; }
            p { color: #666; line-height: 1.6; }
          </style>
        </head>
        <body>
          <div class="container">
            <div class="checkmark">✓</div>
            <h1>Unsubscribed</h1>
            <p>You have been successfully unsubscribed from our mailing list.</p>
            <p>You will no longer receive marketing emails from ApexMediation.</p>
          </div>
        </body>
        </html>
      `);
    } catch (err) {
      console.error('[ApexMail] Unsubscribe error:', err);
      res.status(500).send('Internal server error');
    }
  });

  // ==========================================================================
  // ADMIN/DASHBOARD ENDPOINTS (Should be behind auth in production)
  // ==========================================================================

  /**
   * GET /api/apexmail/stats
   * Email statistics for dashboard
   */
  router.get('/stats', async (req: Request, res: Response) => {
    try {
      const days = parseInt(req.query.days as string) || 7;
      const since = new Date(Date.now() - days * 24 * 60 * 60 * 1000);

      const stats = await apexmail.getStats(since);
      res.json({
        success: true,
        data: stats,
        period: { days, since: since.toISOString() },
      });
    } catch (err) {
      console.error('[ApexMail] Stats error:', err);
      res.status(500).json({ success: false, error: 'Failed to get stats' });
    }
  });

  /**
   * GET /api/apexmail/domain-health
   * Domain health status
   */
  router.get('/domain-health', async (req: Request, res: Response) => {
    try {
      const health = await apexmail.getAllDomainHealth();
      res.json({ success: true, data: health });
    } catch (err) {
      console.error('[ApexMail] Domain health error:', err);
      res.status(500).json({ success: false, error: 'Failed to get domain health' });
    }
  });

  /**
   * POST /api/apexmail/domain-health/:domain/unsuspend
   * Manually unsuspend a domain
   */
  router.post(
    '/domain-health/:domain/unsuspend',
    async (req: Request, res: Response) => {
      try {
        await apexmail.unsuspendDomain(req.params.domain);
        res.json({ success: true, message: 'Domain unsuspended' });
      } catch (err) {
        console.error('[ApexMail] Unsuspend error:', err);
        res.status(500).json({ success: false, error: 'Failed to unsuspend domain' });
      }
    }
  );

  /**
   * GET /api/apexmail/queue
   * View email queue status
   */
  router.get('/queue', async (req: Request, res: Response) => {
    try {
      const status = (req.query.status as string) || 'queued';
      const limit = Math.min(parseInt(req.query.limit as string) || 50, 200);

      const result = await pool.query(
        `
        SELECT id, to_email, subject, status, attempt_count, created_at, next_attempt_at, smtp_response
        FROM apexmail_queue
        WHERE status = $1
        ORDER BY priority, created_at
        LIMIT $2
      `,
        [status, limit]
      );

      const queueStatus = await apexmail.getQueueStatus();

      res.json({
        success: true,
        data: result.rows,
        summary: queueStatus,
      });
    } catch (err) {
      console.error('[ApexMail] Queue error:', err);
      res.status(500).json({ success: false, error: 'Failed to get queue' });
    }
  });

  /**
   * GET /api/apexmail/queue/status
   * Get queue summary counts
   */
  router.get('/queue/status', async (req: Request, res: Response) => {
    try {
      const status = await apexmail.getQueueStatus();
      res.json({ success: true, data: status });
    } catch (err) {
      console.error('[ApexMail] Queue status error:', err);
      res.status(500).json({ success: false, error: 'Failed to get queue status' });
    }
  });

  /**
   * GET /api/apexmail/templates
   * List email templates
   */
  router.get('/templates', async (req: Request, res: Response) => {
    try {
      const templates = await apexmail.getTemplates();
      res.json({ success: true, data: templates });
    } catch (err) {
      console.error('[ApexMail] Templates error:', err);
      res.status(500).json({ success: false, error: 'Failed to get templates' });
    }
  });

  /**
   * POST /api/apexmail/templates
   * Create or update a template
   */
  router.post('/templates', async (req: Request, res: Response): Promise<void> => {
    try {
      const {
        name,
        subjectTemplate,
        bodyHtmlTemplate,
        bodyTextTemplate,
        category,
        variables,
      } = req.body;

      if (!name || !subjectTemplate || !bodyHtmlTemplate) {
        res.status(400).json({
          success: false,
          error: 'Missing required fields: name, subjectTemplate, bodyHtmlTemplate',
        });
        return;
      }

      const id = await apexmail.upsertTemplate({
        name,
        subjectTemplate,
        bodyHtmlTemplate,
        bodyTextTemplate,
        category,
        variables,
      });

      res.json({ success: true, data: { id } });
    } catch (err) {
      console.error('[ApexMail] Template create error:', err);
      res.status(500).json({ success: false, error: 'Failed to create template' });
    }
  });

  /**
   * GET /api/apexmail/suppressions
   * List suppressed emails
   */
  router.get('/suppressions', async (req: Request, res: Response) => {
    try {
      const limit = Math.min(parseInt(req.query.limit as string) || 100, 500);
      const suppressions = await apexmail.getSuppressions(limit);
      res.json({ success: true, data: suppressions });
    } catch (err) {
      console.error('[ApexMail] Suppressions error:', err);
      res.status(500).json({ success: false, error: 'Failed to get suppressions' });
    }
  });

  /**
   * POST /api/apexmail/suppressions
   * Add email to suppression list
   */
  router.post('/suppressions', async (req: Request, res: Response): Promise<void> => {
    try {
      const { email, reason } = req.body;

      if (!email) {
        res.status(400).json({ success: false, error: 'Email is required' });
        return;
      }

      await apexmail.suppress(email, reason || 'manual');
      res.json({ success: true, message: 'Email suppressed' });
    } catch (err) {
      console.error('[ApexMail] Suppress error:', err);
      res.status(500).json({ success: false, error: 'Failed to suppress email' });
    }
  });

  /**
   * DELETE /api/apexmail/suppressions/:email
   * Remove email from suppression list
   */
  router.delete('/suppressions/:email', async (req: Request, res: Response) => {
    try {
      const removed = await apexmail.unsuppress(req.params.email);
      res.json({
        success: true,
        removed,
        message: removed ? 'Email unsuppressed' : 'Email not found in suppression list',
      });
    } catch (err) {
      console.error('[ApexMail] Unsuppress error:', err);
      res.status(500).json({ success: false, error: 'Failed to unsuppress email' });
    }
  });

  /**
   * GET /api/apexmail/check/:email
   * Check if an email is suppressed
   */
  router.get('/check/:email', async (req: Request, res: Response) => {
    try {
      const suppressed = await apexmail.isSuppressed(req.params.email);
      res.json({ success: true, email: req.params.email, suppressed });
    } catch (err) {
      console.error('[ApexMail] Check error:', err);
      res.status(500).json({ success: false, error: 'Failed to check email' });
    }
  });

  /**
   * POST /api/apexmail/send
   * Send an email (for testing/admin)
   */
  router.post('/send', async (req: Request, res: Response): Promise<void> => {
    try {
      const { to, subject, html, text, templateName, templateData, priority, category } =
        req.body;

      if (!to) {
        res.status(400).json({ success: false, error: 'Recipient (to) is required' });
        return;
      }

      if (!templateName && (!subject || !html)) {
        res.status(400).json({
          success: false,
          error: 'Either templateName or both subject and html are required',
        });
        return;
      }

      const result = await apexmail.send({
        to,
        subject: subject || '',
        html: html || '',
        text,
        templateName,
        templateData,
        priority,
        category,
      });

      if (result.success) {
        res.json({ success: true, data: result });
      } else {
        res.status(400).json({ success: false, error: result.error });
      }
    } catch (err) {
      console.error('[ApexMail] Send error:', err);
      res.status(500).json({ success: false, error: 'Failed to send email' });
    }
  });

  /**
   * GET /api/apexmail/health
   * Health check endpoint
   */
  router.get('/health', async (req: Request, res: Response) => {
    try {
      const smtpOk = await apexmail.verifyConnection();
      const queueStatus = await apexmail.getQueueStatus();

      res.json({
        success: true,
        smtp: smtpOk ? 'connected' : 'disconnected',
        queue: queueStatus,
        timestamp: new Date().toISOString(),
      });
    } catch (err) {
      console.error('[ApexMail] Health check error:', err);
      res.status(500).json({
        success: false,
        error: 'Health check failed',
        timestamp: new Date().toISOString(),
      });
    }
  });

  return router;
}

export default createApexMailRouter;
