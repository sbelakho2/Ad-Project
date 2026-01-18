import { logger } from '../utils/logger';

export interface AlertOptions {
  severity: 'low' | 'medium' | 'high' | 'critical';
  title: string;
  message: string;
  metadata?: Record<string, any>;
  component: string;
}

export async function sendAlert(options: AlertOptions): Promise<void> {
  const { severity, title, message, metadata, component } = options;

  // Always log the alert
  const logMethod = severity === 'critical' || severity === 'high' ? 'error' : 'warn';
  
  if (logger && typeof (logger as any)[logMethod] === 'function') {
    (logger as any)[logMethod](`ALERT [${component}]: ${title}`, { message, severity, metadata });
  } else {
    console.error(`ALERT [${component}]: ${title}`, { message, severity, metadata });
  }

  // Slack Webhook integration
  const slackWebhookUrl = process.env.SLACK_WEBHOOK_URL;
  if (slackWebhookUrl) {
    try {
      await fetch(slackWebhookUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          text: `*${severity.toUpperCase()} ALERT* [${component}]: ${title}\n${message}\n\`\`\`${JSON.stringify(metadata, null, 2)}\`\`\``,
        }),
      });
    } catch (error) {
      logger.warn('Failed to send Slack alert', { error });
    }
  }

  // PagerDuty integration (stub)
  const pdRoutingKey = process.env.PAGERDUTY_ROUTING_KEY;
  if (pdRoutingKey && (severity === 'high' || severity === 'critical')) {
    try {
      const pdSeverity =
        severity === 'critical'
          ? 'critical'
          : severity === 'high'
          ? 'error'
          : severity === 'medium'
          ? 'warning'
          : 'info';

      await fetch('https://events.pagerduty.com/v2/enqueue', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          routing_key: pdRoutingKey,
          event_action: 'trigger',
          payload: {
            summary: `[${component}] ${title}`,
            source: component,
            severity: pdSeverity,
            timestamp: new Date().toISOString(),
            custom_details: {
              message,
              metadata,
            },
          },
        }),
      });
    } catch (error) {
      logger.warn('Failed to send PagerDuty alert', { error });
    }
  }
}

export default {
  sendAlert,
};
