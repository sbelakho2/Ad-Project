/**
 * Migration Notification Service
 * 
 * Sends notifications for guardrail violations and other migration events.
 * Supports email, Slack, and webhook notifications based on publisher settings.
 */

import { Pool } from 'pg';
import { createHttpClient } from '../utils/httpClient';
import logger from '../utils/logger';

export interface GuardrailViolationPayload {
  experimentId: string;
  experimentName: string;
  publisherId: string;
  violations: string[];
  action: 'paused' | 'killed';
  timestamp: string;
}

export interface NotificationSettings {
  emailAlerts?: {
    enabled: boolean;
    recipients: string[];
    events: string[];
  };
  slackIntegration?: {
    enabled: boolean;
    webhookUrl: string | null;
    channel: string | null;
    events: string[];
  };
  webhooks?: {
    enabled: boolean;
    endpoints: Array<{
      url: string;
      events: string[];
      secret?: string;
    }>;
  };
}

const http = createHttpClient(undefined, { timeout: 5000 });

export class MigrationNotificationService {
  constructor(private pool: Pool) {}

  /**
   * Send guardrail violation notification to all configured channels
   */
  async sendGuardrailViolationAlert(payload: GuardrailViolationPayload): Promise<void> {
    const settings = await this.getPublisherNotificationSettings(payload.publisherId);

    const promises: Promise<void>[] = [];

    // Slack notification
    if (settings.slackIntegration?.enabled && settings.slackIntegration.webhookUrl) {
      if (settings.slackIntegration.events.includes('guardrail_violation') || 
          settings.slackIntegration.events.includes('all')) {
        promises.push(this.sendSlackNotification(settings.slackIntegration.webhookUrl, payload));
      }
    }

    // Webhook notifications
    if (settings.webhooks?.enabled && settings.webhooks.endpoints.length > 0) {
      for (const endpoint of settings.webhooks.endpoints) {
        if (endpoint.events.includes('guardrail_violation') || 
            endpoint.events.includes('migration') ||
            endpoint.events.includes('all')) {
          promises.push(this.sendWebhookNotification(endpoint.url, payload, endpoint.secret));
        }
      }
    }

    // Wait for all notifications but don't fail if some don't work
    const results = await Promise.allSettled(promises);
    const failures = results.filter(r => r.status === 'rejected');
    
    if (failures.length > 0) {
      logger.warn('Some guardrail violation notifications failed', {
        experimentId: payload.experimentId,
        total: promises.length,
        failures: failures.length,
      });
    }
  }

  private async getPublisherNotificationSettings(publisherId: string): Promise<NotificationSettings> {
    try {
      const result = await this.pool.query(
        `SELECT settings FROM publisher_settings 
         WHERE publisher_id = $1 AND category = 'notifications'`,
        [publisherId]
      );

      if (result.rows.length > 0) {
        return result.rows[0].settings as NotificationSettings;
      }
    } catch (error) {
      logger.warn('Failed to get publisher notification settings', { publisherId, error });
    }

    return {};
  }

  private async sendSlackNotification(webhookUrl: string, payload: GuardrailViolationPayload): Promise<void> {
    const emoji = payload.action === 'killed' ? '🚨' : '⚠️';
    const color = payload.action === 'killed' ? '#dc2626' : '#f59e0b';
    const actionText = payload.action === 'killed' ? 'KILLED' : 'PAUSED';

    const slackPayload = {
      attachments: [
        {
          color,
          blocks: [
            {
              type: 'header',
              text: {
                type: 'plain_text',
                text: `${emoji} Migration Experiment ${actionText}`,
                emoji: true,
              },
            },
            {
              type: 'section',
              fields: [
                {
                  type: 'mrkdwn',
                  text: `*Experiment:*\n${payload.experimentName}`,
                },
                {
                  type: 'mrkdwn',
                  text: `*ID:*\n\`${payload.experimentId.slice(0, 8)}...\``,
                },
              ],
            },
            {
              type: 'section',
              text: {
                type: 'mrkdwn',
                text: `*Violations:*\n${payload.violations.map(v => `• ${v}`).join('\n')}`,
              },
            },
            {
              type: 'context',
              elements: [
                {
                  type: 'mrkdwn',
                  text: `Triggered at ${new Date(payload.timestamp).toISOString()}`,
                },
              ],
            },
          ],
        },
      ],
    };

    try {
      await http.post(webhookUrl, slackPayload);
      logger.debug('Slack notification sent', { experimentId: payload.experimentId });
    } catch (error: any) {
      logger.error('Failed to send Slack notification', {
        experimentId: payload.experimentId,
        error: error.message,
      });
      throw error;
    }
  }

  private async sendWebhookNotification(
    url: string,
    payload: GuardrailViolationPayload,
    secret?: string
  ): Promise<void> {
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      'X-Apex-Event': 'migration.guardrail_violation',
    };

    // Add HMAC signature if secret is configured
    if (secret) {
      const crypto = await import('crypto');
      const signature = crypto
        .createHmac('sha256', secret)
        .update(JSON.stringify(payload))
        .digest('hex');
      headers['X-Apex-Signature'] = `sha256=${signature}`;
    }

    try {
      await http.post(url, {
        event: 'migration.guardrail_violation',
        data: payload,
      }, { headers });
      
      logger.debug('Webhook notification sent', { 
        experimentId: payload.experimentId, 
        url: url.replace(/\/\/[^@]+@/, '//***@'), // Mask credentials in URL
      });
    } catch (error: any) {
      logger.error('Failed to send webhook notification', {
        experimentId: payload.experimentId,
        url: url.replace(/\/\/[^@]+@/, '//***@'),
        error: error.message,
      });
      throw error;
    }
  }
}

export default MigrationNotificationService;
