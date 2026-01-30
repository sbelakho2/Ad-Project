/**
 * Shared Spam Prevention Service
 * 
 * Centralized spam checking used by both V1 (sales-automation) and V2 (autonomous-api).
 * This ensures consistent spam prevention across all outbound email systems.
 * 
 * FEATURES:
 * - Spam trigger word detection (critical/high risk words)
 * - Excessive caps detection
 * - Excessive punctuation detection
 * - Link density check
 * - Email length validation
 * - Configurable threshold
 * 
 * USAGE:
 *   import { checkEmailForSpam, SpamCheckResult } from './spam-prevention';
 *   
 *   const check = checkEmailForSpam(subject, body);
 *   if (!check.passesThreshold) {
 *     return { error: `Spam score too high: ${check.score}` };
 *   }
 */

import { logger } from '../logger.js';

// ============================================================================
// SPAM TRIGGER WORDS
// ============================================================================

const SPAM_TRIGGER_WORDS = {
  critical: [
    'act now', 'action required', 'apply now', 'buy now', 'call now',
    'click here', 'click below', 'don\'t delete', 'exclusive deal', 'expire',
    'free', 'free access', 'free gift', 'free trial', 'great offer',
    'guarantee', 'limited offer', 'limited time', 'no cost', 'no obligation',
    'offer expires', 'order now', 'special promotion', 'urgent', 'winner',
    'you have been selected', 'prize', 'congratulations', 'million dollars',
    'earn money', 'make money', 'cash bonus', 'risk free', '100% free',
  ],
  high: [
    'affordable', 'bargain', 'best price', 'big savings', 'bonus',
    'cheap', 'discount', 'fantastic deal', 'final call', 'giveaway',
    'hot deal', 'incredible deal', 'instant', 'new customers only',
    'promotion', 'save big', 'save money', 'special offer', 'unbeatable',
  ],
};

// ============================================================================
// TYPES
// ============================================================================

export interface SpamCheckResult {
  score: number;          // 0-100, higher = cleaner
  issues: string[];       // List of problems found
  passesThreshold: boolean;
  severity: 'pass' | 'warning' | 'fail';
}

export interface SpamCheckOptions {
  threshold?: number;     // Default 70
  strictMode?: boolean;   // Extra checks for important sends
}

// ============================================================================
// SPAM CHECK FUNCTION
// ============================================================================

/**
 * Check email content for spam indicators
 * 
 * @param subject - Email subject line
 * @param body - Email body (HTML or plain text)
 * @param options - Configuration options
 * @returns SpamCheckResult with score and issues
 */
export function checkEmailForSpam(
  subject: string, 
  body: string,
  options: SpamCheckOptions = {}
): SpamCheckResult {
  const threshold = options.threshold ?? 70;
  const issues: string[] = [];
  let score = 100;
  
  // Normalize text for checking
  const fullTextLower = (subject + ' ' + body).toLowerCase();
  const fullTextOriginal = subject + ' ' + body;

  // ====== CRITICAL TRIGGER WORDS (-15 each) ======
  for (const trigger of SPAM_TRIGGER_WORDS.critical) {
    if (fullTextLower.includes(trigger)) {
      issues.push(`Contains spam trigger: "${trigger}"`);
      score -= 15;
    }
  }

  // ====== HIGH-RISK TRIGGER WORDS (-8 each) ======
  for (const trigger of SPAM_TRIGGER_WORDS.high) {
    if (fullTextLower.includes(trigger)) {
      issues.push(`Contains promotional word: "${trigger}"`);
      score -= 8;
    }
  }

  // ====== EXCESSIVE CAPS (-10) ======
  // Must check ORIGINAL text, not lowercased
  const capsMatches = fullTextOriginal.match(/[A-Z]{5,}/g);
  if (capsMatches && capsMatches.length > 0) {
    issues.push(`Contains excessive CAPS: ${capsMatches.slice(0, 3).join(', ')}`);
    score -= 10;
  }

  // ====== EXCESSIVE PUNCTUATION (-10) ======
  if (/[!?]{2,}/.test(fullTextOriginal)) {
    issues.push('Contains excessive punctuation (!! or ??)');
    score -= 10;
  }

  // ====== TOO MANY LINKS (-5 per extra link after 3) ======
  const linkCount = (body.match(/https?:\/\//g) || []).length;
  if (linkCount > 3) {
    issues.push(`Too many links (${linkCount})`);
    score -= (linkCount - 3) * 5;
  }

  // ====== EMAIL LENGTH ======
  const wordCount = body.split(/\s+/).length;
  if (wordCount < 20) {
    issues.push('Email too short (looks like spam)');
    score -= 10;
  }
  if (wordCount > 500) {
    issues.push('Email too long (may be ignored)');
    score -= 5;
  }

  // ====== STRICT MODE CHECKS ======
  if (options.strictMode) {
    // Check for missing personalization
    if (!body.includes('{{') && !fullTextLower.includes('hi ') && !fullTextLower.includes('hello ')) {
      issues.push('No personalization detected (strict mode)');
      score -= 5;
    }
    
    // Check for ALL CAPS subject
    if (subject === subject.toUpperCase() && subject.length > 10) {
      issues.push('Subject line is ALL CAPS (strict mode)');
      score -= 10;
    }
  }

  // Clamp score to 0-100
  score = Math.max(0, Math.min(100, score));
  
  // Determine severity
  let severity: 'pass' | 'warning' | 'fail';
  if (score >= threshold) {
    severity = 'pass';
  } else if (score >= threshold - 15) {
    severity = 'warning';
  } else {
    severity = 'fail';
  }

  return {
    score,
    issues,
    passesThreshold: score >= threshold,
    severity,
  };
}

/**
 * Log spam check result for debugging/monitoring
 */
export function logSpamCheck(
  result: SpamCheckResult,
  context: { system: 'v1' | 'v2'; contactEmail?: string; messageId?: string }
): void {
  if (!result.passesThreshold) {
    logger.warn('Email failed spam check', {
      system: context.system,
      score: result.score,
      issueCount: result.issues.length,
      topIssues: result.issues.slice(0, 3),
      contactEmail: context.contactEmail?.substring(0, 3) + '***',
      messageId: context.messageId,
    });
  } else if (result.severity === 'warning') {
    logger.info('Email passed spam check with warnings', {
      system: context.system,
      score: result.score,
      issueCount: result.issues.length,
    });
  }
}

// ============================================================================
// EXPORT TRIGGER WORDS FOR TESTING
// ============================================================================

export { SPAM_TRIGGER_WORDS };
