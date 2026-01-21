// services/sales-automation/SpamPrevention.ts
// OVERCOMES: "Emails landing in spam" - #1 complaint across ALL tools
// Based on real user complaints: "every email in spam", "<5% open rates", "domain flagged"
// Zero-cost implementation using text analysis and best practices

import { Pool } from 'pg';

interface SpamCheckResult {
  score: number;                    // 0-100 (0 = definitely spam, 100 = clean)
  issues: SpamIssue[];
  suggestions: string[];
  passesThreshold: boolean;         // Must be > 80 to send
}

interface SpamIssue {
  category: 'trigger_words' | 'formatting' | 'links' | 'reputation' | 'technical';
  severity: 'critical' | 'high' | 'medium' | 'low';
  message: string;
  fix?: string;
}

// Comprehensive spam trigger words from industry research
// OVERCOMES: "emails flagged", "domain blacklisted", "content filters blocking"
const SPAM_TRIGGER_WORDS = {
  // Financial urgency (highest risk)
  critical: [
    'act now', 'action required', 'apply now', 'buy now', 'call now',
    'click here', 'click below', 'don\'t delete', 'exclusive deal', 'expire',
    'free', 'free access', 'free gift', 'free trial', 'great offer',
    'guarantee', 'important information', 'incredible', 'limited offer',
    'limited time', 'no cost', 'no obligation', 'offer expires', 'once in lifetime',
    'order now', 'please read', 'special promotion', 'take action', 'urgent',
    'winner', 'you have been selected', 'you\'re a winner', 'prize', 'congratulations',
    'million dollars', 'double your', 'earn money', 'make money', 'cash bonus',
    'risk free', '100% free', '100% guaranteed', 'no catch', 'no strings attached',
    'act immediately', 'don\'t miss out', 'for only', 'just $', 'lowest price',
  ],
  // Promotional language (medium-high risk)
  high: [
    'affordable', 'bargain', 'best price', 'big savings', 'bonus',
    'cheap', 'compare', 'discount', 'fantastic deal', 'final call',
    'for free', 'giveaway', 'hot deal', 'incredible deal', 'instant',
    'new customers only', 'no fees', 'now only', 'one time', 'only today',
    'promotion', 'save big', 'save money', 'save up to', 'special offer',
    'the best', 'unbeatable', 'unlimited', 'while supplies last', 'while stocks last',
  ],
  // Subtle but risky (medium risk)
  medium: [
    'amazing', 'believe', 'breakthrough', 'celebrity', 'certified',
    'copy immediately', 'dear friend', 'drastically', 'exclusive',
    'fantastic', 'freedom', 'hello friend', 'hidden', 'huge',
    'luxury', 'miracle', 'no questions asked', 'outstanding', 'promise you',
    'revolutionary', 'secret', 'shocking', 'solution', 'special',
    'success', 'the truth', 'unbelievable', 'unsolicited', 'verify your account',
  ],
};

// Subject line specific triggers
const SUBJECT_LINE_TRIGGERS = [
  'RE:', 'FW:', 'urgent', 'important', 'final notice', 'attention',
  'verify', 'confirm', 'account', 'security alert', 'action needed',
  '!!!', '$$$', '***', 'FREE', 'GUARANTEED', 'NO COST', 'CLICK HERE',
  'open immediately', 'please respond', 'time sensitive', 'limited time only',
];

// Formatting issues that trigger spam filters
const FORMATTING_ISSUES = {
  excessiveCaps: /[A-Z]{5,}/g,           // 5+ consecutive caps
  excessivePunctuation: /[!?]{2,}/g,     // Multiple !!! or ???
  dollarSigns: /\$+/g,                    // Money mentions
  percentSigns: /%\s*\d+/g,              // Discount percentages
  allCapsWords: /\b[A-Z]{3,}\b/g,        // Words in all caps
  suspiciousUrls: /https?:\/\/[^\s]+/g,  // Links (count them)
  htmlTags: /<[^>]+>/g,                   // HTML in plain text
  multipleNewlines: /\n{3,}/g,           // Excessive spacing
};

export class SpamPrevention {
  private pool: Pool;
  private sendingDomainAge: number = 0;  // Days since domain setup
  private domainReputation: number = 100;

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * MAIN: Check if an email will likely hit spam
   * OVERCOMES: "0% deliverability", "all emails in spam folder"
   */
  checkEmail(subject: string, body: string, fromEmail: string): SpamCheckResult {
    const issues: SpamIssue[] = [];
    let score = 100;

    // 1. Check subject line
    const subjectIssues = this.checkSubjectLine(subject);
    issues.push(...subjectIssues);
    score -= subjectIssues.reduce((sum, i) => sum + this.severityPenalty(i.severity), 0);

    // 2. Check body content
    const bodyIssues = this.checkBody(body);
    issues.push(...bodyIssues);
    score -= bodyIssues.reduce((sum, i) => sum + this.severityPenalty(i.severity), 0);

    // 3. Check formatting
    const formatIssues = this.checkFormatting(subject + ' ' + body);
    issues.push(...formatIssues);
    score -= formatIssues.reduce((sum, i) => sum + this.severityPenalty(i.severity), 0);

    // 4. Check links
    const linkIssues = this.checkLinks(body);
    issues.push(...linkIssues);
    score -= linkIssues.reduce((sum, i) => sum + this.severityPenalty(i.severity), 0);

    // 5. Check email length (too short = spam, too long = also spam)
    const lengthIssues = this.checkLength(body);
    issues.push(...lengthIssues);
    score -= lengthIssues.reduce((sum, i) => sum + this.severityPenalty(i.severity), 0);

    // 6. Check sender reputation factors
    const repIssues = this.checkReputationFactors(fromEmail);
    issues.push(...repIssues);
    score -= repIssues.reduce((sum, i) => sum + this.severityPenalty(i.severity), 0);

    // Ensure score doesn't go negative
    score = Math.max(0, Math.min(100, score));

    return {
      score,
      issues,
      suggestions: this.generateSuggestions(issues),
      passesThreshold: score >= 80,
    };
  }

  /**
   * Check subject line for spam triggers
   * OVERCOMES: "subject lines getting flagged"
   */
  private checkSubjectLine(subject: string): SpamIssue[] {
    const issues: SpamIssue[] = [];
    const lowerSubject = subject.toLowerCase();

    // Check for trigger words
    for (const trigger of SUBJECT_LINE_TRIGGERS) {
      if (lowerSubject.includes(trigger.toLowerCase())) {
        issues.push({
          category: 'trigger_words',
          severity: 'critical',
          message: `Subject contains spam trigger: "${trigger}"`,
          fix: `Remove or rephrase "${trigger}" in subject line`,
        });
      }
    }

    // Check length (ideal: 30-50 chars)
    if (subject.length < 10) {
      issues.push({
        category: 'formatting',
        severity: 'medium',
        message: 'Subject line too short (appears suspicious)',
        fix: 'Add more context to subject line (aim for 30-50 characters)',
      });
    }
    if (subject.length > 60) {
      issues.push({
        category: 'formatting',
        severity: 'low',
        message: 'Subject line too long (may be truncated)',
        fix: 'Shorten subject to under 60 characters',
      });
    }

    // Check for all caps
    if (subject === subject.toUpperCase() && subject.length > 5) {
      issues.push({
        category: 'formatting',
        severity: 'high',
        message: 'Subject line is all uppercase (screaming = spam)',
        fix: 'Use normal capitalization',
      });
    }

    // Check for RE:/FW: faking
    if (/^(RE|FW|Fwd):/i.test(subject)) {
      issues.push({
        category: 'reputation',
        severity: 'high',
        message: 'Fake reply/forward prefix detected (deceptive practice)',
        fix: 'Remove RE:/FW: - be genuine about first contact',
      });
    }

    return issues;
  }

  /**
   * Check body content for spam triggers
   * OVERCOMES: "content filters killing emails"
   */
  private checkBody(body: string): SpamIssue[] {
    const issues: SpamIssue[] = [];
    const lowerBody = body.toLowerCase();

    // Check critical trigger words
    for (const word of SPAM_TRIGGER_WORDS.critical) {
      if (lowerBody.includes(word.toLowerCase())) {
        issues.push({
          category: 'trigger_words',
          severity: 'critical',
          message: `Body contains high-risk spam trigger: "${word}"`,
          fix: `Replace "${word}" with more professional language`,
        });
      }
    }

    // Check high-risk words (limit to 3 issues to avoid overwhelming)
    let highRiskCount = 0;
    for (const word of SPAM_TRIGGER_WORDS.high) {
      if (lowerBody.includes(word.toLowerCase()) && highRiskCount < 3) {
        issues.push({
          category: 'trigger_words',
          severity: 'high',
          message: `Body contains promotional trigger: "${word}"`,
          fix: `Consider rephrasing "${word}"`,
        });
        highRiskCount++;
      }
    }

    // Check medium-risk words
    let mediumRiskCount = 0;
    for (const word of SPAM_TRIGGER_WORDS.medium) {
      if (lowerBody.includes(word.toLowerCase()) && mediumRiskCount < 2) {
        issues.push({
          category: 'trigger_words',
          severity: 'medium',
          message: `Body contains potentially risky word: "${word}"`,
        });
        mediumRiskCount++;
      }
    }

    return issues;
  }

  /**
   * Check formatting issues
   * OVERCOMES: "emails look like spam"
   */
  private checkFormatting(text: string): SpamIssue[] {
    const issues: SpamIssue[] = [];

    // Excessive caps
    const capsMatches = text.match(FORMATTING_ISSUES.excessiveCaps);
    if (capsMatches && capsMatches.length > 2) {
      issues.push({
        category: 'formatting',
        severity: 'high',
        message: 'Too many SHOUTING words (excessive caps)',
        fix: 'Use normal capitalization throughout',
      });
    }

    // Excessive punctuation
    const punctMatches = text.match(FORMATTING_ISSUES.excessivePunctuation);
    if (punctMatches) {
      issues.push({
        category: 'formatting',
        severity: 'medium',
        message: 'Excessive punctuation (!!! or ???)',
        fix: 'Use single punctuation marks',
      });
    }

    // Dollar signs
    const dollarMatches = text.match(FORMATTING_ISSUES.dollarSigns);
    if (dollarMatches && dollarMatches.length > 1) {
      issues.push({
        category: 'trigger_words',
        severity: 'high',
        message: 'Multiple money references ($) trigger spam filters',
        fix: 'Minimize or remove dollar signs',
      });
    }

    return issues;
  }

  /**
   * Check links in email
   * OVERCOMES: "link overload flagging"
   */
  private checkLinks(body: string): SpamIssue[] {
    const issues: SpamIssue[] = [];
    const urlMatches = body.match(FORMATTING_ISSUES.suspiciousUrls) || [];

    if (urlMatches.length > 3) {
      issues.push({
        category: 'links',
        severity: 'critical',
        message: `Too many links (${urlMatches.length}) - max 2 recommended`,
        fix: 'Reduce to 1-2 links maximum',
      });
    } else if (urlMatches.length > 1) {
      issues.push({
        category: 'links',
        severity: 'medium',
        message: 'Multiple links may reduce deliverability',
        fix: 'Consider using single, clear CTA link',
      });
    }

    // Check for URL shorteners (very suspicious)
    const shorteners = ['bit.ly', 'tinyurl', 't.co', 'goo.gl', 'ow.ly', 'rebrand.ly'];
    for (const url of urlMatches) {
      for (const shortener of shorteners) {
        if (url.toLowerCase().includes(shortener)) {
          issues.push({
            category: 'links',
            severity: 'critical',
            message: `URL shortener detected (${shortener}) - heavily flagged by spam filters`,
            fix: 'Use full, direct URLs instead of shorteners',
          });
          break;
        }
      }
    }

    return issues;
  }

  /**
   * Check email length
   * OVERCOMES: "emails look robotic/templated"
   */
  private checkLength(body: string): SpamIssue[] {
    const issues: SpamIssue[] = [];
    const words = body.trim().split(/\s+/).length;

    if (words < 20) {
      issues.push({
        category: 'formatting',
        severity: 'medium',
        message: 'Email too short - may appear as spam or low effort',
        fix: 'Add more context and personalization (aim for 50-150 words)',
      });
    }

    if (words > 300) {
      issues.push({
        category: 'formatting',
        severity: 'low',
        message: 'Email quite long - may reduce engagement',
        fix: 'Consider shortening to under 150 words for better engagement',
      });
    }

    return issues;
  }

  /**
   * Check sender reputation factors
   */
  private checkReputationFactors(fromEmail: string): SpamIssue[] {
    const issues: SpamIssue[] = [];
    
    // Check if using free email provider
    const freeProviders = ['gmail.com', 'yahoo.com', 'hotmail.com', 'outlook.com', 'aol.com'];
    const domain = fromEmail.split('@')[1];
    
    if (freeProviders.includes(domain?.toLowerCase())) {
      issues.push({
        category: 'reputation',
        severity: 'critical',
        message: 'Using free email provider for cold outreach (very suspicious)',
        fix: 'Use a professional domain email (e.g., you@yourcompany.com)',
      });
    }

    return issues;
  }

  /**
   * Calculate penalty for issue severity
   */
  private severityPenalty(severity: SpamIssue['severity']): number {
    switch (severity) {
      case 'critical': return 25;
      case 'high': return 15;
      case 'medium': return 8;
      case 'low': return 3;
      default: return 0;
    }
  }

  /**
   * Generate actionable suggestions
   */
  private generateSuggestions(issues: SpamIssue[]): string[] {
    const suggestions: string[] = [];
    
    // Group by category
    const byCategory = issues.reduce((acc, issue) => {
      acc[issue.category] = acc[issue.category] || [];
      acc[issue.category].push(issue);
      return acc;
    }, {} as Record<string, SpamIssue[]>);

    if (byCategory.trigger_words?.length) {
      suggestions.push('Remove promotional/urgent language - be conversational and genuine');
    }
    if (byCategory.formatting?.length) {
      suggestions.push('Clean up formatting - no ALL CAPS, single punctuation, proper spacing');
    }
    if (byCategory.links?.length) {
      suggestions.push('Reduce links to 1-2 max, never use URL shorteners');
    }
    if (byCategory.reputation?.length) {
      suggestions.push('Use professional custom domain email for cold outreach');
    }

    // General best practices
    if (issues.length > 5) {
      suggestions.push('Consider rewriting this email from scratch using conversational tone');
    }

    return suggestions;
  }

  /**
   * Auto-fix common issues (where safe)
   * OVERCOMES: "having to manually fix every email"
   */
  autoFix(subject: string, body: string): { subject: string; body: string; changes: string[] } {
    const changes: string[] = [];
    let fixedSubject = subject;
    let fixedBody = body;

    // Fix excessive punctuation
    if (FORMATTING_ISSUES.excessivePunctuation.test(fixedSubject)) {
      fixedSubject = fixedSubject.replace(/!+/g, '!').replace(/\?+/g, '?');
      changes.push('Reduced excessive punctuation in subject');
    }
    if (FORMATTING_ISSUES.excessivePunctuation.test(fixedBody)) {
      fixedBody = fixedBody.replace(/!+/g, '!').replace(/\?+/g, '?');
      changes.push('Reduced excessive punctuation in body');
    }

    // Remove RE:/FW: prefix
    if (/^(RE|FW|Fwd):\s*/i.test(fixedSubject)) {
      fixedSubject = fixedSubject.replace(/^(RE|FW|Fwd):\s*/i, '');
      changes.push('Removed fake RE:/FW: prefix');
    }

    // Fix multiple newlines
    if (FORMATTING_ISSUES.multipleNewlines.test(fixedBody)) {
      fixedBody = fixedBody.replace(/\n{3,}/g, '\n\n');
      changes.push('Normalized spacing');
    }

    return { subject: fixedSubject, body: fixedBody, changes };
  }

  /**
   * Generate spam-safe email variation
   * OVERCOMES: "all emails look the same, pattern detected"
   */
  async generateSafeVariation(template: string, personalization: Record<string, string>): Promise<string> {
    let result = template;

    // Replace variables
    for (const [key, value] of Object.entries(personalization)) {
      result = result.replace(new RegExp(`{{${key}}}`, 'g'), value);
    }

    // Add slight variations to avoid pattern detection
    // 1. Vary greeting
    const greetings = ['Hi', 'Hey', 'Hello'];
    result = result.replace(/^(Hi|Hey|Hello)\b/i, greetings[Math.floor(Math.random() * greetings.length)]);

    // 2. Vary sign-off
    const signoffs = ['Best', 'Cheers', 'Thanks', 'Best regards', 'Talk soon'];
    result = result.replace(/(Best|Cheers|Thanks|Best regards|Talk soon),?\s*$/im, 
      signoffs[Math.floor(Math.random() * signoffs.length)] + ',');

    return result;
  }

  /**
   * Score email before sending and block if too risky
   * Integration point for OutreachService
   */
  async validateBeforeSend(subject: string, body: string, fromEmail: string): Promise<{
    allowed: boolean;
    score: number;
    reason?: string;
    autoFixed?: { subject: string; body: string };
  }> {
    // First check
    const initial = this.checkEmail(subject, body, fromEmail);

    if (initial.passesThreshold) {
      return { allowed: true, score: initial.score };
    }

    // Try auto-fix
    const fixed = this.autoFix(subject, body);
    const afterFix = this.checkEmail(fixed.subject, fixed.body, fromEmail);

    if (afterFix.passesThreshold) {
      return {
        allowed: true,
        score: afterFix.score,
        autoFixed: { subject: fixed.subject, body: fixed.body },
      };
    }

    // Still failing - block it
    return {
      allowed: false,
      score: afterFix.score,
      reason: `Email scored ${afterFix.score}/100 (needs 80+). Issues: ${afterFix.issues.slice(0, 3).map(i => i.message).join('; ')}`,
    };
  }
}
