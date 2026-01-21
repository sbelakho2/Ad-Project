// services/sales-automation/SendTimingOptimizer.ts
// Optimizes email send times for maximum engagement
// Based on industry research: Tuesday-Thursday, 9-11 AM recipient's local time

import { Pool } from 'pg';

interface SendWindow {
  dayOfWeek: number;  // 0-6 (Sunday-Saturday)
  hourStart: number;  // 0-23
  hourEnd: number;    // 0-23
  weight: number;     // 1-10 priority
}

interface OptimalSendTime {
  timestamp: Date;
  reason: string;
  score: number;  // 0-100
}

// Industry research-backed optimal send windows
// Source: Multiple studies from Hubspot, Mailchimp, Sendinblue
const OPTIMAL_SEND_WINDOWS: SendWindow[] = [
  // Best days: Tuesday, Wednesday, Thursday
  // Best times: 9-11 AM, 1-3 PM
  
  // Tuesday - Highest engagement day
  { dayOfWeek: 2, hourStart: 9, hourEnd: 11, weight: 10 },
  { dayOfWeek: 2, hourStart: 13, hourEnd: 15, weight: 8 },
  
  // Wednesday - Second best
  { dayOfWeek: 3, hourStart: 9, hourEnd: 11, weight: 9 },
  { dayOfWeek: 3, hourStart: 13, hourEnd: 15, weight: 7 },
  
  // Thursday - Third best
  { dayOfWeek: 4, hourStart: 9, hourEnd: 11, weight: 8 },
  { dayOfWeek: 4, hourStart: 13, hourEnd: 15, weight: 6 },
  
  // Monday - Decent, but people are catching up
  { dayOfWeek: 1, hourStart: 10, hourEnd: 12, weight: 6 },
  { dayOfWeek: 1, hourStart: 14, hourEnd: 16, weight: 5 },
  
  // Friday - Lower engagement, people wrapping up
  { dayOfWeek: 5, hourStart: 10, hourEnd: 12, weight: 4 },
];

// Time zones by country (simplified)
const TIMEZONE_MAP: Record<string, string> = {
  'US': 'America/New_York',      // Default to ET
  'CA': 'America/Toronto',
  'GB': 'Europe/London',
  'UK': 'Europe/London',
  'DE': 'Europe/Berlin',
  'FR': 'Europe/Paris',
  'ES': 'Europe/Madrid',
  'IT': 'Europe/Rome',
  'NL': 'Europe/Amsterdam',
  'AU': 'Australia/Sydney',
  'JP': 'Asia/Tokyo',
  'KR': 'Asia/Seoul',
  'CN': 'Asia/Shanghai',
  'IN': 'Asia/Kolkata',
  'SG': 'Asia/Singapore',
  'BR': 'America/Sao_Paulo',
  'MX': 'America/Mexico_City',
  'IL': 'Asia/Jerusalem',
  'EE': 'Europe/Tallinn',
  'FI': 'Europe/Helsinki',
  'SE': 'Europe/Stockholm',
  'NO': 'Europe/Oslo',
  'DK': 'Europe/Copenhagen',
  'PL': 'Europe/Warsaw',
};

export class SendTimingOptimizer {
  private pool: Pool;
  private sendWindows: SendWindow[];

  constructor(databaseUrl: string, customWindows?: SendWindow[]) {
    this.pool = new Pool({ connectionString: databaseUrl });
    this.sendWindows = customWindows || OPTIMAL_SEND_WINDOWS;
  }

  /**
   * Get optimal send time for a contact
   */
  async getOptimalSendTime(
    contactId: string,
    minDelay: number = 0  // Minimum hours from now
  ): Promise<OptimalSendTime> {
    // Get contact's timezone
    const contactTimezone = await this.getContactTimezone(contactId);
    
    // Find next optimal window
    const now = new Date();
    const minSendTime = new Date(now.getTime() + minDelay * 60 * 60 * 1000);
    
    // Check next 14 days for optimal windows
    for (let dayOffset = 0; dayOffset < 14; dayOffset++) {
      const checkDate = new Date(minSendTime);
      checkDate.setDate(checkDate.getDate() + dayOffset);
      
      for (const window of this.sendWindows.sort((a, b) => b.weight - a.weight)) {
        if (checkDate.getDay() !== window.dayOfWeek) continue;
        
        // Convert to contact's timezone
        const sendTime = this.getTimeInTimezone(
          checkDate,
          window.hourStart + Math.random() * (window.hourEnd - window.hourStart),
          contactTimezone
        );
        
        if (sendTime > minSendTime) {
          return {
            timestamp: sendTime,
            reason: `Optimal window: ${this.getDayName(window.dayOfWeek)} ${window.hourStart}-${window.hourEnd}:00 (${contactTimezone})`,
            score: window.weight * 10,
          };
        }
      }
    }
    
    // Fallback: next business day at 10 AM
    const fallbackTime = this.getNextBusinessDay(minSendTime, 10, contactTimezone);
    return {
      timestamp: fallbackTime,
      reason: 'Fallback: Next business day at 10 AM',
      score: 30,
    };
  }

  /**
   * Schedule email for optimal time
   */
  async scheduleEmailOptimally(
    emailId: string,
    contactId: string,
    minDelayHours: number = 0
  ): Promise<Date> {
    const optimal = await this.getOptimalSendTime(contactId, minDelayHours);
    
    await this.pool.query(`
      UPDATE outreach_emails
      SET scheduled_for = $2,
          scheduling_reason = $3,
          scheduling_score = $4,
          updated_at = CURRENT_TIMESTAMP
      WHERE id = $1
    `, [emailId, optimal.timestamp, optimal.reason, optimal.score]);

    return optimal.timestamp;
  }

  /**
   * Batch schedule multiple emails for optimal times
   * Spreads emails to avoid sending all at once
   */
  async batchScheduleOptimally(
    emailIds: string[],
    minDelayHours: number = 0,
    spreadMinutes: number = 15  // Minutes between emails
  ): Promise<Map<string, Date>> {
    const scheduled = new Map<string, Date>();
    let currentDelay = minDelayHours;

    for (let i = 0; i < emailIds.length; i++) {
      const emailId = emailIds[i];
      
      // Get contact for this email
      const result = await this.pool.query(`
        SELECT contact_id FROM outreach_emails WHERE id = $1
      `, [emailId]);
      
      if (result.rows.length === 0) continue;
      
      const contactId = result.rows[0].contact_id;
      const optimal = await this.getOptimalSendTime(contactId, currentDelay);
      
      // Add spread between emails
      const spreadOffset = (i % 4) * spreadMinutes;  // Group emails in sets of 4
      const scheduledTime = new Date(optimal.timestamp.getTime() + spreadOffset * 60 * 1000);
      
      await this.pool.query(`
        UPDATE outreach_emails
        SET scheduled_for = $2,
            scheduling_reason = $3,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = $1
      `, [emailId, scheduledTime, optimal.reason]);

      scheduled.set(emailId, scheduledTime);
      
      // Every 4 emails, add additional delay to next optimal window
      if ((i + 1) % 4 === 0) {
        currentDelay += 2;  // Add 2 hours
      }
    }

    return scheduled;
  }

  /**
   * Get contact's timezone based on company location
   */
  private async getContactTimezone(contactId: string): Promise<string> {
    const result = await this.pool.query(`
      SELECT c.timezone, co.country_code
      FROM sales_contacts c
      LEFT JOIN sales_companies co ON c.company_id = co.id
      WHERE c.id = $1
    `, [contactId]);

    if (result.rows.length === 0) return 'America/New_York';

    const { timezone, country_code } = result.rows[0];
    
    // Use stored timezone if available
    if (timezone) return timezone;
    
    // Otherwise infer from country
    if (country_code && TIMEZONE_MAP[country_code]) {
      return TIMEZONE_MAP[country_code];
    }

    // Default to US Eastern
    return 'America/New_York';
  }

  /**
   * Convert time to specific timezone
   */
  private getTimeInTimezone(date: Date, hour: number, timezone: string): Date {
    try {
      // Create a date string in the target timezone
      const localDate = new Date(date.toLocaleString('en-US', { timeZone: timezone }));
      localDate.setHours(Math.floor(hour), Math.round((hour % 1) * 60), 0, 0);
      
      // Convert back to UTC
      const utcOffset = date.getTime() - localDate.getTime();
      return new Date(localDate.getTime() + utcOffset);
    } catch {
      // Fallback: just set the hour
      const result = new Date(date);
      result.setHours(Math.floor(hour), Math.round((hour % 1) * 60), 0, 0);
      return result;
    }
  }

  /**
   * Get next business day
   */
  private getNextBusinessDay(from: Date, hour: number, timezone: string): Date {
    const date = new Date(from);
    
    // Move to next day if needed
    while (true) {
      const day = date.getDay();
      if (day !== 0 && day !== 6) {  // Not weekend
        break;
      }
      date.setDate(date.getDate() + 1);
    }
    
    return this.getTimeInTimezone(date, hour, timezone);
  }

  /**
   * Get day name
   */
  private getDayName(day: number): string {
    const days = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
    return days[day];
  }

  /**
   * Analyze historical send time performance
   */
  async analyzeSendTimePerformance(): Promise<{
    byDayOfWeek: { day: string; openRate: number; replyRate: number; count: number }[];
    byHour: { hour: number; openRate: number; replyRate: number; count: number }[];
  }> {
    // By day of week
    const byDayResult = await this.pool.query(`
      SELECT 
        EXTRACT(DOW FROM sent_at) as day_of_week,
        COUNT(*) as count,
        COUNT(*) FILTER (WHERE opened_at IS NOT NULL)::float / NULLIF(COUNT(*), 0) * 100 as open_rate,
        COUNT(*) FILTER (WHERE replied_at IS NOT NULL)::float / NULLIF(COUNT(*), 0) * 100 as reply_rate
      FROM outreach_emails
      WHERE sent_at IS NOT NULL
        AND sent_at > CURRENT_TIMESTAMP - INTERVAL '90 days'
      GROUP BY EXTRACT(DOW FROM sent_at)
      ORDER BY day_of_week
    `);

    // By hour
    const byHourResult = await this.pool.query(`
      SELECT 
        EXTRACT(HOUR FROM sent_at) as hour,
        COUNT(*) as count,
        COUNT(*) FILTER (WHERE opened_at IS NOT NULL)::float / NULLIF(COUNT(*), 0) * 100 as open_rate,
        COUNT(*) FILTER (WHERE replied_at IS NOT NULL)::float / NULLIF(COUNT(*), 0) * 100 as reply_rate
      FROM outreach_emails
      WHERE sent_at IS NOT NULL
        AND sent_at > CURRENT_TIMESTAMP - INTERVAL '90 days'
      GROUP BY EXTRACT(HOUR FROM sent_at)
      ORDER BY hour
    `);

    const days = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];

    return {
      byDayOfWeek: byDayResult.rows.map(row => ({
        day: days[parseInt(row.day_of_week)],
        openRate: parseFloat(row.open_rate) || 0,
        replyRate: parseFloat(row.reply_rate) || 0,
        count: parseInt(row.count),
      })),
      byHour: byHourResult.rows.map(row => ({
        hour: parseInt(row.hour),
        openRate: parseFloat(row.open_rate) || 0,
        replyRate: parseFloat(row.reply_rate) || 0,
        count: parseInt(row.count),
      })),
    };
  }

  /**
   * Update send windows based on historical performance
   * Call this weekly to optimize based on actual data
   */
  async optimizeSendWindows(): Promise<void> {
    const performance = await this.analyzeSendTimePerformance();
    
    // Find best performing day/hour combinations
    const newWindows: SendWindow[] = [];
    
    for (const dayData of performance.byDayOfWeek) {
      const dayIndex = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'].indexOf(dayData.day);
      
      // Skip weekends
      if (dayIndex === 0 || dayIndex === 6) continue;
      
      // Calculate weight based on performance
      const weight = Math.min(10, Math.round(
        (dayData.openRate * 0.3 + dayData.replyRate * 0.7) / 5
      ));
      
      // Find best hours for this day
      const bestHours = performance.byHour
        .filter(h => h.count >= 10)  // Minimum sample size
        .sort((a, b) => (b.openRate + b.replyRate * 2) - (a.openRate + a.replyRate * 2))
        .slice(0, 3);
      
      for (const hourData of bestHours) {
        newWindows.push({
          dayOfWeek: dayIndex,
          hourStart: hourData.hour,
          hourEnd: hourData.hour + 2,
          weight: Math.max(1, weight),
        });
      }
    }
    
    // If we have enough data, update windows
    if (newWindows.length >= 5) {
      this.sendWindows = newWindows;
      console.log('[SendTiming] Updated send windows based on historical performance');
    }
  }
}

export default SendTimingOptimizer;
