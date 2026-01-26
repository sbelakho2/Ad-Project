// services/sales-automation/AdaptivePersonalization.ts
// OVERCOMES: "Emails look robotic", "obvious templates", "no personalization at scale"
// OVERCOMES: "same email to everyone", "prospects can tell it's automated"
// Based on Apollo/Instantly complaints: "templates don't work", "no real personalization"
// Zero-cost: Algorithmic personalization, no AI APIs needed

import { Pool } from 'pg';

interface PersonalizationContext {
  contact: {
    firstName: string;
    lastName: string;
    email: string;
    title?: string;
    linkedinUrl?: string;
  };
  company: {
    name: string;
    domain?: string;
    industry?: string;
    size?: string;
    tech?: string[];
    recentNews?: string[];
  };
  engagement: {
    previousEmails: number;
    opens: number;
    clicks: number;
    replies: number;
    lastEngagement?: Date;
  };
  timing: {
    timezone?: string;
    localTime?: string;
    dayOfWeek?: string;
  };
}

export interface PersonalizedEmail {
  subject: string;
  body: string;
  personalizationScore: number;  // 0-100, how personalized this feels
  variationsUsed: string[];
}

interface Template {
  id: string;
  name: string;
  subject: string;
  body: string;
  variables: string[];
}

// Opening line variations based on what we know
const OPENING_VARIATIONS = {
  // When we have title
  withTitle: [
    "I noticed you're the {{title}} at {{company}} — ",
    "As {{company}}'s {{title}}, you probably deal with ",
    "Working as {{title}} at {{company}}, I imagine you're focused on ",
    "Given your role as {{title}}, I thought this might interest you",
  ],
  // When we have industry
  withIndustry: [
    "Working in {{industry}}, you've probably noticed ",
    "The {{industry}} space is evolving fast — ",
    "Most {{industry}} companies we talk to are dealing with ",
  ],
  // When we have tech stack
  withTech: [
    "I saw {{company}} uses {{tech}} — ",
    "Since you're running {{tech}}, ",
    "Given your {{tech}} setup, ",
  ],
  // Fallback
  generic: [
    "I came across {{company}} and ",
    "I was researching companies like {{company}} and ",
    "{{company}} caught my attention because ",
  ],
};

// Closing variations
const CLOSING_VARIATIONS = {
  question: [
    "Would it make sense to chat for 15 minutes this week?",
    "Worth a quick call to see if this could help?",
    "Any interest in exploring this further?",
    "Could this be relevant for {{company}}?",
  ],
  softCta: [
    "Happy to share more details if helpful.",
    "Let me know if you'd like to learn more.",
    "Just reply if this sounds interesting.",
  ],
  valueProp: [
    "Our publishers typically see {{benefit}} within the first month.",
    "Most teams in your position reduce {{pain_point}} by 40%+.",
  ],
};

// Timing-aware greetings
const TIME_GREETINGS: Record<string, string[]> = {
  morning: ['Good morning', 'Morning'],
  afternoon: ['Good afternoon', 'Hi'],
  evening: ['Good evening', 'Hi'],
  monday: ['Hope you had a great weekend', 'Happy Monday'],
  friday: ['Happy Friday', 'TGIF'],
};

// Industry-specific pain points
const INDUSTRY_PAIN_POINTS: Record<string, string[]> = {
  gaming: [
    'ad revenue optimization',
    'user retention vs monetization balance',
    'fill rates across different geos',
  ],
  media: [
    'programmatic yield',
    'first-party data monetization',
    'ad tech stack complexity',
  ],
  utility: [
    'user engagement during low-usage periods',
    'in-app monetization without hurting experience',
  ],
  default: [
    'maximizing ad revenue',
    'finding the right demand partners',
    'transparent reporting',
  ],
};

/**
 * AdaptivePersonalization - Smart email personalization at scale
 * OVERCOMES: "Everyone sends the same template" complaint
 */
export class AdaptivePersonalization {
  private pool: Pool;

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * MAIN: Personalize an email template for a specific contact
   * OVERCOMES: "robotic templates", "obvious automation"
   */
  async personalizeEmail(
    template: Template,
    contactId: string
  ): Promise<PersonalizedEmail> {
    // Gather all context
    const context = await this.gatherContext(contactId);
    
    let subject = template.subject;
    let body = template.body;
    const variationsUsed: string[] = [];

    // 1. Smart opening line
    const opening = this.generateSmartOpening(context);
    if (opening.line && body.includes('{{smart_opening}}')) {
      body = body.replace('{{smart_opening}}', opening.line);
      variationsUsed.push(`opening: ${opening.type}`);
    }

    // 2. Personalize based on industry pain points
    const painPoint = this.selectPainPoint(context);
    if (painPoint && body.includes('{{pain_point}}')) {
      body = body.replace(/\{\{pain_point\}\}/g, painPoint);
      variationsUsed.push('industry pain point');
    }

    // 3. Time-aware greeting
    const greeting = this.getTimeAwareGreeting(context);
    if (greeting) {
      subject = subject.replace(/^(Hi|Hey|Hello)/i, greeting);
      body = body.replace(/^(Hi|Hey|Hello)\b/i, greeting);
      variationsUsed.push(`greeting: ${greeting}`);
    }

    // 4. Smart closing
    const closing = this.generateSmartClosing(context);
    if (closing && body.includes('{{smart_closing}}')) {
      body = body.replace('{{smart_closing}}', closing);
      variationsUsed.push('smart closing');
    }

    // 5. Replace standard variables
    const variables = this.extractVariables(context);
    for (const [key, value] of Object.entries(variables)) {
      const regex = new RegExp(`\\{\\{${key}\\}\\}`, 'g');
      subject = subject.replace(regex, value);
      body = body.replace(regex, value);
    }

    // 6. Add subtle variations to avoid pattern detection
    body = this.addSubtleVariations(body);
    variationsUsed.push('anti-pattern variations');

    // 7. Calculate personalization score
    const personalizationScore = this.calculatePersonalizationScore(context, variationsUsed);

    return {
      subject,
      body,
      personalizationScore,
      variationsUsed,
    };
  }

  /**
   * Gather all context about a contact
   */
  private async gatherContext(contactId: string): Promise<PersonalizationContext> {
    const result = await this.pool.query(`
      SELECT 
        c.first_name, c.last_name, c.email, c.title, c.linkedin_url,
        comp.name as company_name, comp.website as company_domain,
        comp.industry, comp.employee_range as company_size,
        comp.tech_stack as tech,
        (SELECT COUNT(*) FROM outreach_emails WHERE contact_id = c.id AND status = 'sent') as emails_sent,
        (SELECT COUNT(*) FROM outreach_emails WHERE contact_id = c.id AND opened_at IS NOT NULL) as opens,
        (SELECT COUNT(*) FROM outreach_emails WHERE contact_id = c.id AND clicked_at IS NOT NULL) as clicks,
        (SELECT COUNT(*) FROM sales_activities WHERE contact_id = c.id AND activity_type = 'email_replied') as replies,
        (SELECT MAX(created_at) FROM sales_activities WHERE contact_id = c.id) as last_engagement,
        c.timezone
      FROM sales_contacts c
      LEFT JOIN sales_companies comp ON c.company_id = comp.id
      WHERE c.id = $1
    `, [contactId]);

    const row = result.rows[0] || {};
    const now = new Date();

    return {
      contact: {
        firstName: row.first_name || 'there',
        lastName: row.last_name || '',
        email: row.email || '',
        title: row.title,
        linkedinUrl: row.linkedin_url,
      },
      company: {
        name: row.company_name || 'your company',
        domain: row.company_domain,
        industry: row.industry,
        size: row.company_size,
        tech: row.tech ? (Array.isArray(row.tech) ? row.tech : [row.tech]) : [],
      },
      engagement: {
        previousEmails: parseInt(row.emails_sent) || 0,
        opens: parseInt(row.opens) || 0,
        clicks: parseInt(row.clicks) || 0,
        replies: parseInt(row.replies) || 0,
        lastEngagement: row.last_engagement ? new Date(row.last_engagement) : undefined,
      },
      timing: {
        timezone: row.timezone,
        localTime: this.getLocalTimeOfDay(row.timezone),
        dayOfWeek: now.toLocaleDateString('en-US', { weekday: 'long' }).toLowerCase(),
      },
    };
  }

  /**
   * Generate smart opening based on context
   * OVERCOMES: "everyone uses the same opening"
   */
  private generateSmartOpening(context: PersonalizationContext): { line: string; type: string } {
    let variations: string[];
    let type: string;

    // Priority order: tech > title > industry > generic
    if (context.company.tech && context.company.tech.length > 0) {
      variations = OPENING_VARIATIONS.withTech;
      type = 'tech';
    } else if (context.contact.title) {
      variations = OPENING_VARIATIONS.withTitle;
      type = 'title';
    } else if (context.company.industry) {
      variations = OPENING_VARIATIONS.withIndustry;
      type = 'industry';
    } else {
      variations = OPENING_VARIATIONS.generic;
      type = 'generic';
    }

    // Select variation based on contact ID hash (consistent but varied)
    const hash = this.simpleHash(context.contact.email);
    const selected = variations[hash % variations.length];

    // Replace variables
    let line = selected
      .replace('{{title}}', context.contact.title || '')
      .replace('{{company}}', context.company.name)
      .replace('{{industry}}', context.company.industry || '')
      .replace('{{tech}}', context.company.tech?.[0] || '');

    return { line, type };
  }

  /**
   * Select industry-specific pain point
   */
  private selectPainPoint(context: PersonalizationContext): string {
    const industry = (context.company.industry || '').toLowerCase();
    const painPoints = INDUSTRY_PAIN_POINTS[industry] || INDUSTRY_PAIN_POINTS.default;
    
    const hash = this.simpleHash(context.contact.email + 'pain');
    return painPoints[hash % painPoints.length];
  }

  /**
   * Get time-aware greeting
   * OVERCOMES: "emails feel automated" - adds human touch
   */
  private getTimeAwareGreeting(context: PersonalizationContext): string | null {
    const timeOfDay = context.timing.localTime;
    const dayOfWeek = context.timing.dayOfWeek;

    // Special day greetings (less frequently)
    if (dayOfWeek && Math.random() > 0.7) {
      if (dayOfWeek === 'monday' && TIME_GREETINGS.monday) {
        return TIME_GREETINGS.monday[Math.floor(Math.random() * TIME_GREETINGS.monday.length)];
      }
      if (dayOfWeek === 'friday' && TIME_GREETINGS.friday) {
        return TIME_GREETINGS.friday[Math.floor(Math.random() * TIME_GREETINGS.friday.length)];
      }
    }

    // Time-based greetings
    if (timeOfDay && TIME_GREETINGS[timeOfDay]) {
      const options = TIME_GREETINGS[timeOfDay];
      return options[Math.floor(Math.random() * options.length)];
    }

    return null;
  }

  /**
   * Generate smart closing based on engagement
   */
  private generateSmartClosing(context: PersonalizationContext): string {
    let variations: string[];

    // If they've engaged before, be more direct
    if (context.engagement.clicks > 0 || context.engagement.opens > 2) {
      variations = CLOSING_VARIATIONS.question;
    } else if (context.engagement.previousEmails === 0) {
      // First email - soft CTA
      variations = CLOSING_VARIATIONS.softCta;
    } else {
      variations = CLOSING_VARIATIONS.question;
    }

    const hash = this.simpleHash(context.contact.email + 'close');
    let closing = variations[hash % variations.length];

    // Replace variables
    closing = closing
      .replace('{{company}}', context.company.name)
      .replace('{{benefit}}', '20%+ revenue lift')
      .replace('{{pain_point}}', 'ad tech complexity');

    return closing;
  }

  /**
   * Extract standard variables
   */
  private extractVariables(context: PersonalizationContext): Record<string, string> {
    return {
      first_name: context.contact.firstName,
      last_name: context.contact.lastName,
      full_name: `${context.contact.firstName} ${context.contact.lastName}`.trim(),
      email: context.contact.email,
      title: context.contact.title || 'team member',
      company: context.company.name,
      company_name: context.company.name,
      industry: context.company.industry || 'your industry',
    };
  }

  /**
   * Add subtle variations to avoid spam pattern detection
   * OVERCOMES: "pattern detected, domain flagged"
   */
  private addSubtleVariations(text: string): string {
    let result = text;

    // Vary whitespace slightly (invisible but breaks patterns)
    const variations = [
      { find: '.  ', replace: '. ' },  // Normalize double spaces
      { find: '  ', replace: ' ' },
    ];

    for (const v of variations) {
      result = result.split(v.find).join(v.replace);
    }

    // Vary punctuation slightly
    if (Math.random() > 0.5) {
      result = result.replace(/\!$/, '.');  // Sometimes end with period instead of exclamation
    }

    // Vary common phrases
    const phraseVariations = [
      { original: 'Let me know', alternatives: ['Let me know', 'Lmk', 'Tell me'] },
      { original: 'Best regards', alternatives: ['Best', 'Cheers', 'Thanks'] },
      { original: 'I hope this email finds you well', alternatives: ['Hope you\'re doing well', 'Hope all is well', ''] },
    ];

    for (const pv of phraseVariations) {
      if (result.includes(pv.original)) {
        const replacement = pv.alternatives[Math.floor(Math.random() * pv.alternatives.length)];
        result = result.replace(pv.original, replacement);
      }
    }

    return result;
  }

  /**
   * Calculate personalization score
   */
  private calculatePersonalizationScore(context: PersonalizationContext, variationsUsed: string[]): number {
    let score = 40; // Base score

    // Context depth adds points
    if (context.contact.title) score += 10;
    if (context.company.industry) score += 10;
    if (context.company.tech && context.company.tech.length > 0) score += 15;
    if (context.engagement.previousEmails > 0) score += 5;

    // Variations used add points
    score += variationsUsed.length * 5;

    return Math.min(100, score);
  }

  /**
   * Get local time of day
   */
  private getLocalTimeOfDay(timezone?: string): string {
    const now = new Date();
    let hour = now.getHours();

    // Approximate timezone adjustment if provided
    if (timezone) {
      try {
        const localTime = new Date(now.toLocaleString('en-US', { timeZone: timezone }));
        hour = localTime.getHours();
      } catch {
        // Invalid timezone, use default
      }
    }

    if (hour >= 5 && hour < 12) return 'morning';
    if (hour >= 12 && hour < 17) return 'afternoon';
    return 'evening';
  }

  /**
   * Simple hash function for consistent variation selection
   */
  private simpleHash(str: string): number {
    let hash = 0;
    for (let i = 0; i < str.length; i++) {
      const char = str.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash;
    }
    return Math.abs(hash);
  }

  /**
   * Generate A/B test variations
   * OVERCOMES: "no way to test what works"
   */
  async generateABVariations(template: Template, contactId: string, numVariations: number = 2): Promise<PersonalizedEmail[]> {
    const variations: PersonalizedEmail[] = [];
    const context = await this.gatherContext(contactId);

    for (let i = 0; i < numVariations; i++) {
      // Modify context slightly for each variation
      const modifiedContext = { ...context };
      
      // Vary the selection seed
      const originalEmail = context.contact.email;
      modifiedContext.contact.email = originalEmail + i.toString();
      
      const personalized = await this.personalizeEmail(template, contactId);
      
      // Restore original email
      personalized.body = personalized.body.replace(modifiedContext.contact.email, originalEmail);
      
      variations.push(personalized);
    }

    return variations;
  }

  /**
   * Analyze template effectiveness
   */
  async analyzeTemplateEffectiveness(templateId: string): Promise<{
    totalSent: number;
    openRate: number;
    clickRate: number;
    replyRate: number;
    bestPerformingVariation?: string;
  }> {
    const result = await this.pool.query(`
      SELECT 
        COUNT(*) as total,
        COUNT(*) FILTER (WHERE opened_at IS NOT NULL) as opens,
        COUNT(*) FILTER (WHERE clicked_at IS NOT NULL) as clicks,
        COUNT(*) FILTER (WHERE replied_at IS NOT NULL) as replies
      FROM outreach_emails
      WHERE template_id = $1 AND status = 'sent'
    `, [templateId]);

    const stats = result.rows[0];
    const total = parseInt(stats.total) || 1;

    return {
      totalSent: total,
      openRate: (parseInt(stats.opens) || 0) / total * 100,
      clickRate: (parseInt(stats.clicks) || 0) / total * 100,
      replyRate: (parseInt(stats.replies) || 0) / total * 100,
    };
  }
}
