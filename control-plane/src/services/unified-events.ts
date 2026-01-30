/**
 * Unified Engagement Events Service
 * 
 * Records engagement events from both V1 and V2 sales systems into
 * the unified_engagement_events table for cross-system learning.
 * 
 * USAGE:
 *   import { recordUnifiedEvent, getUnifiedMetrics } from './unified-events';
 *   
 *   // Record an event
 *   await recordUnifiedEvent({
 *     sourceSystem: 'v1_outbound',
 *     eventType: 'opened',
 *     contactEmail: 'user@example.com',
 *     sourceMessageId: 'uuid-123',
 *   });
 * 
 * EVENT TYPES:
 *   - sent: Email was sent
 *   - delivered: Email was delivered
 *   - opened: Email was opened (tracking pixel)
 *   - clicked: Link in email was clicked
 *   - replied: Recipient replied to email
 *   - bounced: Email bounced
 *   - complained: Recipient marked as spam
 *   - unsubscribed: Recipient unsubscribed
 *   - converted: Lead converted to customer
 */

import { queryOperator, queryReadOnly } from '../database';
import { logger } from '../logger';
import crypto from 'crypto';

// Event types enum
export type EngagementEventType = 
  | 'sent' 
  | 'delivered' 
  | 'opened' 
  | 'clicked' 
  | 'replied' 
  | 'bounced' 
  | 'complained' 
  | 'unsubscribed' 
  | 'converted';

// Source systems enum
export type SourceSystem = 'v1_outbound' | 'v2_autonomous' | 'v1_trial_nurture';

// Event recording parameters
export interface UnifiedEventParams {
  sourceSystem: SourceSystem;
  eventType: EngagementEventType;
  contactEmail: string;
  sourceMessageId?: string;
  sourceContactId?: string;
  subjectLine?: string;
  templateId?: string;
  templateName?: string;
  banditArmId?: string;
  sequenceName?: string;
  stepNumber?: number;
  eventMetadata?: Record<string, any>;
  campaignId?: string;
  campaignName?: string;
}

/**
 * Generate deduplication hash for event
 */
function generateEventHash(email: string, eventType: string, messageId: string | undefined): string {
  const today = new Date().toISOString().split('T')[0];
  const input = `${email || ''}|${eventType || ''}|${messageId || ''}|${today}`;
  return crypto.createHash('sha256').update(input).digest('hex');
}

/**
 * Record a unified engagement event
 * 
 * @param params Event parameters
 * @returns The event ID if inserted, null if deduplicated
 */
export async function recordUnifiedEvent(params: UnifiedEventParams): Promise<string | null> {
  try {
    const contactDomain = params.contactEmail.split('@')[1] || null;
    const eventHash = generateEventHash(params.contactEmail, params.eventType, params.sourceMessageId);
    
    const result = await queryOperator<{ id: string }>(`
      INSERT INTO unified_engagement_events (
        source_system, source_message_id, source_contact_id,
        contact_email, contact_domain, event_type,
        subject_line, template_id, template_name,
        bandit_arm_id, sequence_name, step_number,
        event_metadata, campaign_id, campaign_name,
        event_hash
      ) VALUES (
        $1, $2, $3,
        $4, $5, $6,
        $7, $8, $9,
        $10, $11, $12,
        $13, $14, $15,
        $16
      )
      ON CONFLICT (event_hash) DO NOTHING
      RETURNING id
    `, [
      params.sourceSystem,
      params.sourceMessageId || null,
      params.sourceContactId || null,
      params.contactEmail,
      contactDomain,
      params.eventType,
      params.subjectLine || null,
      params.templateId || null,
      params.templateName || null,
      params.banditArmId || null,
      params.sequenceName || null,
      params.stepNumber || null,
      JSON.stringify(params.eventMetadata || {}),
      params.campaignId || null,
      params.campaignName || null,
      eventHash,
    ], 'unified-events', `Record ${params.eventType} event for ${params.contactEmail}`);
    
    if (result.length > 0) {
      logger.debug('Unified event recorded', { 
        eventId: result[0].id,
        eventType: params.eventType,
        sourceSystem: params.sourceSystem,
        contactEmail: params.contactEmail,
      });
      return result[0].id;
    }
    
    // Event was deduplicated
    return null;
  } catch (error) {
    // Handle missing table gracefully
    if ((error as any)?.code === '42P01') {
      logger.warn('unified_engagement_events table missing, skipping event recording');
      return null;
    }
    logger.error('Failed to record unified event', { error, params });
    throw error;
  }
}

/**
 * Record multiple events in batch
 */
export async function recordUnifiedEventsBatch(events: UnifiedEventParams[]): Promise<{
  recorded: number;
  deduplicated: number;
  errors: number;
}> {
  let recorded = 0;
  let deduplicated = 0;
  let errors = 0;
  
  for (const event of events) {
    try {
      const result = await recordUnifiedEvent(event);
      if (result) {
        recorded++;
      } else {
        deduplicated++;
      }
    } catch {
      errors++;
    }
  }
  
  return { recorded, deduplicated, errors };
}

/**
 * Get unified metrics summary
 */
export async function getUnifiedMetricsSummary(days: number = 30): Promise<{
  totalSent: number;
  totalOpened: number;
  totalClicked: number;
  totalReplied: number;
  totalBounced: number;
  openRate: number;
  clickRate: number;
  replyRate: number;
  bounceRate: number;
  bySystem: Record<string, {
    sent: number;
    opened: number;
    clicked: number;
    replied: number;
    bounced: number;
  }>;
}> {
  try {
    const metrics = await queryReadOnly<{
      source_system: string;
      event_type: string;
      count: string;
    }>(`
      SELECT source_system, event_type, COUNT(*) as count
      FROM unified_engagement_events
      WHERE event_timestamp >= CURRENT_DATE - INTERVAL '${days} days'
      GROUP BY source_system, event_type
    `, []);
    
    // Initialize result
    const bySystem: Record<string, { sent: number; opened: number; clicked: number; replied: number; bounced: number }> = {};
    let totalSent = 0;
    let totalOpened = 0;
    let totalClicked = 0;
    let totalReplied = 0;
    let totalBounced = 0;
    
    for (const row of metrics) {
      const count = parseInt(row.count);
      
      if (!bySystem[row.source_system]) {
        bySystem[row.source_system] = { sent: 0, opened: 0, clicked: 0, replied: 0, bounced: 0 };
      }
      
      switch (row.event_type) {
        case 'sent':
          bySystem[row.source_system].sent += count;
          totalSent += count;
          break;
        case 'opened':
          bySystem[row.source_system].opened += count;
          totalOpened += count;
          break;
        case 'clicked':
          bySystem[row.source_system].clicked += count;
          totalClicked += count;
          break;
        case 'replied':
          bySystem[row.source_system].replied += count;
          totalReplied += count;
          break;
        case 'bounced':
          bySystem[row.source_system].bounced += count;
          totalBounced += count;
          break;
      }
    }
    
    return {
      totalSent,
      totalOpened,
      totalClicked,
      totalReplied,
      totalBounced,
      openRate: totalSent > 0 ? Math.round((totalOpened / totalSent) * 100 * 100) / 100 : 0,
      clickRate: totalSent > 0 ? Math.round((totalClicked / totalSent) * 100 * 100) / 100 : 0,
      replyRate: totalSent > 0 ? Math.round((totalReplied / totalSent) * 100 * 100) / 100 : 0,
      bounceRate: totalSent > 0 ? Math.round((totalBounced / totalSent) * 100 * 100) / 100 : 0,
      bySystem,
    };
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      logger.warn('unified_engagement_events table missing');
      return {
        totalSent: 0,
        totalOpened: 0,
        totalClicked: 0,
        totalReplied: 0,
        totalBounced: 0,
        openRate: 0,
        clickRate: 0,
        replyRate: 0,
        bounceRate: 0,
        bySystem: {},
      };
    }
    throw error;
  }
}

/**
 * Get domain engagement patterns (for cross-system learning)
 */
export async function getDomainEngagementPatterns(domain: string): Promise<{
  totalEmails: number;
  totalOpens: number;
  totalClicks: number;
  totalReplies: number;
  openRate: number;
  bestPerformingTemplates: Array<{ templateName: string; openRate: number; count: number }>;
  bestSendDays: Array<{ dayOfWeek: number; openRate: number; count: number }>;
}> {
  try {
    // Get basic metrics for domain
    const metrics = await queryReadOnly<{
      event_type: string;
      count: string;
    }>(`
      SELECT event_type, COUNT(*) as count
      FROM unified_engagement_events
      WHERE contact_domain = $1
      GROUP BY event_type
    `, [domain]);
    
    let totalEmails = 0;
    let totalOpens = 0;
    let totalClicks = 0;
    let totalReplies = 0;
    
    for (const row of metrics) {
      const count = parseInt(row.count);
      switch (row.event_type) {
        case 'sent': totalEmails += count; break;
        case 'opened': totalOpens += count; break;
        case 'clicked': totalClicks += count; break;
        case 'replied': totalReplies += count; break;
      }
    }
    
    // Get best performing templates for this domain
    const templatePerf = await queryReadOnly<{
      template_name: string;
      sent: string;
      opened: string;
    }>(`
      SELECT 
        COALESCE(template_name, 'Unknown') as template_name,
        COUNT(*) FILTER (WHERE event_type = 'sent') as sent,
        COUNT(*) FILTER (WHERE event_type = 'opened') as opened
      FROM unified_engagement_events
      WHERE contact_domain = $1
        AND template_name IS NOT NULL
      GROUP BY template_name
      HAVING COUNT(*) FILTER (WHERE event_type = 'sent') >= 3
      ORDER BY COUNT(*) FILTER (WHERE event_type = 'opened')::float / NULLIF(COUNT(*) FILTER (WHERE event_type = 'sent'), 0) DESC
      LIMIT 5
    `, [domain]);
    
    const bestPerformingTemplates = templatePerf.map(row => ({
      templateName: row.template_name,
      openRate: parseInt(row.sent) > 0 
        ? Math.round((parseInt(row.opened) / parseInt(row.sent)) * 100 * 100) / 100 
        : 0,
      count: parseInt(row.sent),
    }));
    
    // Get best send days for this domain
    const dayPerf = await queryReadOnly<{
      day_of_week: string;
      sent: string;
      opened: string;
    }>(`
      SELECT 
        EXTRACT(DOW FROM event_timestamp)::integer as day_of_week,
        COUNT(*) FILTER (WHERE event_type = 'sent') as sent,
        COUNT(*) FILTER (WHERE event_type = 'opened') as opened
      FROM unified_engagement_events
      WHERE contact_domain = $1
      GROUP BY day_of_week
      HAVING COUNT(*) FILTER (WHERE event_type = 'sent') >= 3
      ORDER BY COUNT(*) FILTER (WHERE event_type = 'opened')::float / NULLIF(COUNT(*) FILTER (WHERE event_type = 'sent'), 0) DESC
    `, [domain]);
    
    const bestSendDays = dayPerf.map(row => ({
      dayOfWeek: parseInt(row.day_of_week),
      openRate: parseInt(row.sent) > 0 
        ? Math.round((parseInt(row.opened) / parseInt(row.sent)) * 100 * 100) / 100 
        : 0,
      count: parseInt(row.sent),
    }));
    
    return {
      totalEmails,
      totalOpens,
      totalClicks,
      totalReplies,
      openRate: totalEmails > 0 ? Math.round((totalOpens / totalEmails) * 100 * 100) / 100 : 0,
      bestPerformingTemplates,
      bestSendDays,
    };
  } catch (error) {
    if ((error as any)?.code === '42P01') {
      return {
        totalEmails: 0,
        totalOpens: 0,
        totalClicks: 0,
        totalReplies: 0,
        openRate: 0,
        bestPerformingTemplates: [],
        bestSendDays: [],
      };
    }
    throw error;
  }
}

/**
 * Sync existing V1 events to unified table
 */
export async function syncV1EventsToUnified(): Promise<{ synced: number; error?: string }> {
  try {
    const result = await queryOperator<{ sync_v1_outbound_to_unified: string }>(
      'SELECT sync_v1_outbound_to_unified()',
      [],
      'unified-events-sync',
      'Sync V1 outbound events to unified table'
    );
    
    return { synced: parseInt(result[0]?.sync_v1_outbound_to_unified || '0') };
  } catch (error) {
    logger.error('Failed to sync V1 events', { error });
    return { synced: 0, error: (error as Error).message };
  }
}

/**
 * Sync existing V2 events to unified table
 */
export async function syncV2EventsToUnified(): Promise<{ synced: number; error?: string }> {
  try {
    const result = await queryOperator<{ sync_v2_autonomous_to_unified: string }>(
      'SELECT sync_v2_autonomous_to_unified()',
      [],
      'unified-events-sync',
      'Sync V2 autonomous events to unified table'
    );
    
    return { synced: parseInt(result[0]?.sync_v2_autonomous_to_unified || '0') };
  } catch (error) {
    logger.error('Failed to sync V2 events', { error });
    return { synced: 0, error: (error as Error).message };
  }
}
