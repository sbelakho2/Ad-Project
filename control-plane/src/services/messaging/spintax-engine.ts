/**
 * SPINTAX ENGINE (The Writer)
 * 
 * Zero-cost message personalization using Spintax syntax.
 * Generates unique, human-like variations for each recipient.
 * 
 * Syntax:
 *   {option1|option2|option3} - Random choice
 *   {option1||option3}        - Empty option allowed
 *   {{nested|choices}|outer}  - Nested spintax
 * 
 * Special Variables:
 *   {{FIRST_NAME}}           - Contact's first name
 *   {{COMPANY}}              - Contact's company
 *   {{APP_NAME}}             - App name if discovered
 *   {{ICEBREAKER}}           - AI-generated icebreaker (from scrape)
 * 
 * Safety:
 *   - Levenshtein distance check between generated variants
 *   - Ensures each email is sufficiently unique to avoid spam filters
 */

import { queryReadOnly, queryOperator } from '../../database';
import { logger } from '../../logger';

// ============================================================================
// TYPES
// ============================================================================

export interface SpintaxTemplate {
  id: string;
  name: string;
  description?: string;
  templateType: string;
  subjectSpintax: string;
  bodySpintax: string;
  availableVariables: string[];
  timesUsed: number;
  totalOpens: number;
  totalClicks: number;
  totalReplies: number;
  openRate: number;
  replyRate: number;
  active: boolean;
}

export interface PersonalizationContext {
  firstName?: string;
  lastName?: string;
  company?: string;
  appName?: string;
  appGenre?: string;
  icebreaker?: string;
  liftPercent?: number;
  liftRange?: string;
  customFields?: Record<string, string>;
}

export interface SpinResult {
  subject: string;
  body: string;
  variationHash: string;
  uniquenessScore: number;
}

// ============================================================================
// ICEBREAKER QUALITY SCORING (NEW - Jan 2026)
// ============================================================================

export interface IcebreakerQualityResult {
  score: number;           // 0-100
  issues: string[];        // List of quality issues found
  passesQualityGate: boolean;  // true if score >= 60
}

/**
 * Score the quality of an icebreaker to prevent generic/low-effort messages.
 * 
 * CIALDINI PRINCIPLE: Liking requires genuine personalization.
 * Generic icebreakers ("great app!") destroy trust instantly.
 * 
 * @param icebreaker - The icebreaker text to score
 * @returns Quality score and list of issues
 */
export function scoreIcebreakerQuality(icebreaker: string): IcebreakerQualityResult {
  const issues: string[] = [];
  let score = 100;
  
  if (!icebreaker || icebreaker.trim() === '') {
    return { score: 0, issues: ['Icebreaker is empty'], passesQualityGate: false };
  }
  
  const trimmed = icebreaker.trim();
  
  // Check for overly generic praise - major red flag
  // IMPROVED: Added check for predicate adjectives ("app is great") and removed specificity falst positives
  const genericPatterns = [
    /\b(great|nice|cool|awesome|amazing|fantastic|wonderful)\s+(app|game|work|job|product)\b/i,
    /\b(app|game|work|job|service)\s+(is|looks|seems)\s+(great|nice|cool|awesome|amazing|fantastic|wonderful)\b/i,
    /\blove\s+(your|the)\s+(app|game|work)\b/i,
    /\b(looks?\s+)?interesting\b/i,
    /\bimpressed\s+by\s+(the|your)\s+(app|work|game)\b/i,
  ];
  
  for (const pattern of genericPatterns) {
    if (pattern.test(trimmed)) {
      score -= 30;
      issues.push('Contains generic praise - not specific enough to feel personal');
      break; // Only penalize once for generic praise
    }
  }
  
  // Check for length - too short is suspicious, too long loses attention
  if (trimmed.length < 30) {
    score -= 25;
    issues.push('Too short - needs more specific detail to feel personal');
  } else if (trimmed.length > 200) {
    score -= 15;
    issues.push('Too long - may lose reader attention, trim to key insight');
  }
  
  // Check for specific detail indicators (positive signals)
  // IMPROVED: Removed "noticed/saw/found" as they are not specific enough on their own
  const specificityIndicators = [
    /\b(feature|update|version|design|growth|revenue|users?|downloads?|ratings?|reviews?|ranking|mechanic|ux|ui|interface)\b/i,
    /\b(specific|particular|especially|notably)\b/i,
    /\b\d+\b/,  // Numbers often indicate specific knowledge
    /\b(recently|last\s+(week|month|year)|this\s+(week|month|year))\b/i,  // Temporal specificity
  ];
  
  let hasSpecificity = false;
  for (const pattern of specificityIndicators) {
    if (pattern.test(trimmed)) {
      hasSpecificity = true;
      break;
    }
  }
  
  if (!hasSpecificity) {
    score -= 20;
    issues.push('No specific details mentioned - add feature/metric/event reference');
  }
  
  // Check for question format (engagement driver)
  const hasQuestion = /\?$/.test(trimmed);
  if (hasQuestion) {
    score += 10; // Bonus for engagement
    // Cap at 100
    score = Math.min(100, score);
  }
  
  // Check for templated patterns that look automated
  const automatedPatterns = [
    /\{\{[^}]+\}\}/,  // Unresolved variables
    /\[insert.*\]/i,  // Insert placeholders
    /^I\s+(noticed|came across|saw|found)\s+(your|the)\s+(app|game|company)\s+and\s*$/i,  // Incomplete sentence
  ];
  
  for (const pattern of automatedPatterns) {
    if (pattern.test(trimmed)) {
      score -= 40;
      issues.push('Contains automation artifacts or incomplete sentences');
      break;
    }
  }
  
  // Ensure score stays in bounds
  score = Math.max(0, Math.min(100, score));
  
  return {
    score,
    issues,
    passesQualityGate: score >= 60,
  };
}

// ============================================================================
// HTML ENCODING FOR SECURITY (NEW - Jan 2026)
// ============================================================================

/**
 * Encode HTML special characters to prevent XSS attacks.
 * 
 * SECURITY: User-provided data (names, company names) could contain malicious HTML.
 * This function ensures all personalization values are safely encoded.
 * 
 * @param str - The string to encode
 * @returns HTML-safe encoded string
 */
export function encodeHtml(str: string): string {
  if (!str) return '';
  
  return str
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/**
 * Encode a personalization context for safe HTML output.
 * 
 * @param context - The context with potentially unsafe values
 * @returns New context with all string values HTML-encoded
 */
export function encodeContextForHtml(context: PersonalizationContext): PersonalizationContext {
  return {
    firstName: context.firstName ? encodeHtml(context.firstName) : undefined,
    lastName: context.lastName ? encodeHtml(context.lastName) : undefined,
    company: context.company ? encodeHtml(context.company) : undefined,
    appName: context.appName ? encodeHtml(context.appName) : undefined,
    appGenre: context.appGenre ? encodeHtml(context.appGenre) : undefined,
    icebreaker: context.icebreaker ? encodeHtml(context.icebreaker) : undefined,
    liftPercent: context.liftPercent, // Numbers don't need HTML encoding
    liftRange: context.liftRange ? encodeHtml(context.liftRange) : undefined,
    customFields: context.customFields 
      ? Object.fromEntries(
          Object.entries(context.customFields).map(([k, v]) => [k, encodeHtml(v)])
        )
      : undefined,
  };
}

// ============================================================================
// SPINTAX SAFETY HELPERS
// ============================================================================

const SPINTAX_ESCAPES = {
  '{': '&#123;',
  '}': '&#125;',
  '|': '&#124;'
};

function escapeSpintax(str: string | undefined): string {
  if (!str) return '';
  return str.replace(/[{}\|]/g, m => SPINTAX_ESCAPES[m as keyof typeof SPINTAX_ESCAPES]);
}

function unescapeSpintax(str: string): string {
  if (!str) return '';
  return str
    .replace(/&#123;/g, '{')
    .replace(/&#125;/g, '}')
    .replace(/&#124;/g, '|');
}

/**
 * Escape context to prevent Spintax Injection (treating user data as spintax choices)
 */
function escapeContextForSpintax(context: PersonalizationContext): PersonalizationContext {
  return {
    firstName: context.firstName ? escapeSpintax(context.firstName) : undefined,
    lastName: context.lastName ? escapeSpintax(context.lastName) : undefined,
    company: context.company ? escapeSpintax(context.company) : undefined,
    appName: context.appName ? escapeSpintax(context.appName) : undefined,
    appGenre: context.appGenre ? escapeSpintax(context.appGenre) : undefined,
    // CRITICAL: See note below. We DO NOT escape icebreaker if it's potentially generated spintax.
    // However, if the icebreaker comes from an untrusted external source, it should be escaped.
    // In our architecture, IcebreakerEngine generates it. If IcebreakerEngine returns spintax, we want to allow it.
    // But if IcebreakerEngine returns "Company | Name", we want to escape it.
    // DECISION: We will NOT escape icebreaker here, assuming the IcebreakerEngine handles its own safety 
    // or intends to provide Spintax.
    icebreaker: context.icebreaker,
    liftPercent: context.liftPercent, // Numbers don't need escaping
    liftRange: context.liftRange ? escapeSpintax(context.liftRange) : undefined,
    customFields: context.customFields 
      ? Object.fromEntries(
          Object.entries(context.customFields).map(([k, v]) => [k, escapeSpintax(v)])
        )
      : undefined,
  };
}


// ============================================================================
// LEVENSHTEIN DISTANCE (Uniqueness Check)
// ============================================================================

/**
 * Calculate Levenshtein distance between two strings.
 * Used to ensure generated emails are sufficiently different.
 */
export function levenshteinDistance(str1: string, str2: string): number {
  const m = str1.length;
  const n = str2.length;
  
  // Create distance matrix
  const dp: number[][] = Array(m + 1).fill(null).map(() => Array(n + 1).fill(0));
  
  // Initialize first row and column
  for (let i = 0; i <= m; i++) dp[i][0] = i;
  for (let j = 0; j <= n; j++) dp[0][j] = j;
  
  // Fill the matrix
  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      if (str1[i - 1] === str2[j - 1]) {
        dp[i][j] = dp[i - 1][j - 1];
      } else {
        dp[i][j] = 1 + Math.min(
          dp[i - 1][j],     // deletion
          dp[i][j - 1],     // insertion
          dp[i - 1][j - 1]  // substitution
        );
      }
    }
  }
  
  return dp[m][n];
}

/**
 * Calculate normalized Levenshtein distance (0 to 1).
 * 0 = identical, 1 = completely different
 */
export function normalizedLevenshtein(str1: string, str2: string): number {
  const maxLen = Math.max(str1.length, str2.length);
  if (maxLen === 0) return 0;
  return levenshteinDistance(str1, str2) / maxLen;
}

// ============================================================================
// SPINTAX PARSER (Recursive)
// ============================================================================

/**
 * Parse and spin a spintax string.
 * Handles nested braces and random selection.
 * 
 * IMPORTANT: This function ONLY processes spintax (choices with |):
 *   - {option1|option2|option3} → Random choice between options
 *   - Single-option braces without | are preserved: {single} → {single}
 *   - Double-brace variables are PRESERVED: {{VAR}} → {{VAR}}
 * 
 * Personalization variables ({{VAR}}) are handled separately by personalize().
 */
export function spin(spintax: string): string {
  // Step 1: Protect double-brace variables by replacing them with placeholders
  const placeholders: Map<string, string> = new Map();
  let placeholderIndex = 0;
  
  let result = spintax.replace(/\{\{([^{}]+)\}\}/g, (match) => {
    const placeholder = `__PLACEHOLDER_${placeholderIndex++}__`;
    placeholders.set(placeholder, match);
    return placeholder;
  });
  
  // Step 2: Process spintax (only braces containing | for choices)
  let iterations = 0;
  const maxIterations = 100; // Prevent infinite loops
  
  while (result.includes('{') && iterations < maxIterations) {
    // Find innermost braces containing a pipe (actual spintax choices)
    const match = result.match(/\{([^{}]*\|[^{}]*)\}/);
    if (!match) break;
    
    const options = match[1].split('|');
    const choice = options[Math.floor(Math.random() * options.length)];
    
    // Replace only the first occurrence
    result = result.replace(match[0], choice);
    iterations++;
  }
  
  // Step 3: Restore protected double-brace variables
  for (const [placeholder, original] of placeholders) {
    result = result.replace(placeholder, original);
  }
  
  return result.trim();
}

/**
 * Apply personalization context to a template.
 * Replaces {{VARIABLE}} placeholders with actual values.
 * 
 * Supported variables:
 *   {{FIRST_NAME}} - Contact's first name (smart fallback based on context)
 *   {{LAST_NAME}}  - Contact's last name (fallback: "")
 *   {{COMPANY}}    - Company name (fallback: extracted from email or "your company")
 *   {{APP_NAME}}   - App name (REQUIRED - if missing, message fails quality gate)
 *   {{APP_GENRE}}  - App genre/category (fallback: "mobile apps")
 *   {{ICEBREAKER}} - AI-generated icebreaker (fallback: generic interest line)
 * 
 * All matches are case-insensitive.
 * 
 * CIALDINI PRINCIPLE: Personalization drives Liking.
 * Bad personalization (obvious fallbacks) destroys trust instantly.
 */
export function personalize(template: string, context: PersonalizationContext): string {
  let result = template;
  
  // Helper to clean and validate values
  const clean = (val: string | undefined, fallback: string): string => {
    if (!val || val.trim() === '' || val === 'null' || val === 'undefined') {
      return fallback;
    }
    return val.trim();
  };
  
  // Smart FIRST_NAME fallback: Use context-aware fallback that doesn't scream automation
  // FIXED: Better handling to avoid awkward "Hi, " with nothing after it
  const smartFirstNameFallback = (): string => {
    // If we have app name, we can skip the name entirely - the email will still feel personal
    // because we're talking about their specific app
    if (context.appName && context.appName.trim() !== '') {
      return ''; // Template: "Hi, I noticed {{APP_NAME}}..." works without a name
    }
    // If we have company but no app, use 'team' - professional and not robotic
    // "Hi team," is common in B2B and doesn't feel automated
    if (context.company && context.company.trim() !== '') {
      return 'team';
    }
    // Last resort: empty string, quality gate will catch if there's no personalization at all
    return '';
  };
  
  // Smart ICEBREAKER fallback: Generic but still personalized using app/genre
  const smartIcebreakerFallback = (): string => {
    if (context.appName && context.appName.trim() !== '') {
      return `I came across ${context.appName} and was impressed by the execution.`;
    }
    if (context.appGenre && context.appGenre.trim() !== '') {
      return `I've been looking at ${context.appGenre} apps and yours caught my attention.`;
    }
    return ''; // No fallback if we have nothing to work with
  };
  
  // Standard replacements (case-insensitive)
  // FIXED: Removed dangerous fallbacks that triggered quality gate rejection
  // The quality gate checks for 'your company' and 'your app' as indicators of missing data
  // Using empty strings allows the quality gate to properly detect missing personalization
  result = result.replace(/\{\{FIRST_NAME\}\}/gi, clean(context.firstName, smartFirstNameFallback()));
  result = result.replace(/\{\{LAST_NAME\}\}/gi, clean(context.lastName, ''));
  // FIXED: Changed from 'your company' to empty string - quality gate will catch missing data
  result = result.replace(/\{\{COMPANY\}\}/gi, clean(context.company, ''));
  result = result.replace(/\{\{APP_NAME\}\}/gi, clean(context.appName, '')); // Empty = fails quality gate
  // FIXED: Changed from 'mobile apps' to empty string - generic fallbacks look robotic
  result = result.replace(/\{\{APP_GENRE\}\}/gi, clean(context.appGenre, ''));
  result = result.replace(/\{\{ICEBREAKER\}\}/gi, clean(context.icebreaker, smartIcebreakerFallback()));
  
  // LIFT_PERCENT: Truthful Puffery - specific lift estimates based on ad setup analysis
  // Falls back to generic "15-30%" range if no specific analysis available
  // This creates Cialdini Authority through specific, defensible numbers
  const liftFallback = context.liftPercent ? `${context.liftPercent}` : '15-30';
  result = result.replace(/\{\{LIFT_PERCENT\}\}/gi, liftFallback);
  result = result.replace(/\{\{LIFT_RANGE\}\}/gi, clean(context.liftRange, '15-30%'));
  
  // Custom fields
  if (context.customFields) {
    for (const [key, value] of Object.entries(context.customFields)) {
      // Escape regex special characters in key to prevent ReDoS
      const escapedKey = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      const regex = new RegExp(`\\{\\{${escapedKey}\\}\\}`, 'gi');
      result = result.replace(regex, clean(value, ''));
    }
  }
  
  // Clean up artifacts:
  // - Multiple consecutive spaces → single space
  // - Space before punctuation → just punctuation  
  // - Empty parenthetical content → removed
  // - Double commas → single comma
  result = result
    .replace(/\s+/g, ' ')
    .replace(/\s+([.,!?;:])/g, '$1')
    .replace(/\(\s*\)/g, '')
    .replace(/,\s*,/g, ',')
    .trim();
  
  return result;
}

/**
 * Generate a unique spin with personalization
 */
export function spinAndPersonalize(
  subjectSpintax: string,
  bodySpintax: string,
  context: PersonalizationContext
): SpinResult {
  // IMPROVED: Personalize FIRST, then Spin.
  // This allows variables (like {{ICEBREAKER}}) to containing Spintax that gets processed.
  // We must escape user data to prevent Spintax Injection, but we allow Icebreaker to have spintax.
  
  const safeContext = escapeContextForSpintax(context);
  
  // 1. Personalize (injects values, potentially with Spintax in icebreaker)
  const personalizedSubject = personalize(subjectSpintax, safeContext);
  const personalizedBody = personalize(bodySpintax, safeContext);
  
  // 2. Spin (resolves choices, including any injected via icebreaker)
  const subject = unescapeSpintax(spin(personalizedSubject));
  const body = unescapeSpintax(spin(personalizedBody));
  
  // Create a hash for tracking this specific variation
  const variationHash = simpleHash(`${subject}|${body}`);
  
  return {
    subject,
    body,
    variationHash,
    uniquenessScore: 1.0, // Will be updated if checking against history
  };
}

/**
 * Generate a unique spin with personalization, HTML-encoded for safe email output.
 * 
 * SECURITY: Use this function when generating HTML emails to prevent XSS attacks
 * from user-provided data in names, company names, etc.
 * 
 * @param subjectSpintax - Subject line with spintax
 * @param bodySpintax - Body with spintax
 * @param context - Personalization context (will be HTML-encoded)
 * @returns SpinResult with HTML-safe subject and body
 */
export function spinAndPersonalizeHtml(
  subjectSpintax: string,
  bodySpintax: string,
  context: PersonalizationContext
): SpinResult {
  // HTML-encode the context first for security
  const safeContext = encodeContextForHtml(context);
  return spinAndPersonalize(subjectSpintax, bodySpintax, safeContext);
}

/**
 * Simple hash function for variation tracking
 */
function simpleHash(str: string): string {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i);
    hash = ((hash << 5) - hash) + char;
    hash = hash & hash; // Convert to 32bit integer
  }
  return Math.abs(hash).toString(16).padStart(8, '0');
}

// ============================================================================
// SPINTAX ENGINE CLASS
// ============================================================================

export class SpintaxEngine {
  private recentGenerations: string[] = [];
  private maxHistorySize: number;
  private minDistance: number;
  
  constructor(options?: { maxHistorySize?: number; minDistance?: number }) {
    this.maxHistorySize = options?.maxHistorySize || 50;
    // FIXED: Increased from 10 to 30 to ensure sufficient uniqueness for spam filters
    // Original value of 10 was too low - emails with only 10 character differences
    // are essentially the same to spam filters
    this.minDistance = options?.minDistance || 30;
  }
  
  /**
   * Generate a unique message, ensuring it's different from recent ones
   */
  generateUnique(
    subjectSpintax: string,
    bodySpintax: string,
    context: PersonalizationContext,
    maxAttempts = 5
  ): SpinResult {
    for (let attempt = 0; attempt < maxAttempts; attempt++) {
      const result = spinAndPersonalize(subjectSpintax, bodySpintax, context);
      const fullMessage = `${result.subject}\n${result.body}`;
      
      // Check uniqueness against recent generations
      let isUnique = true;
      let minFoundDistance = Infinity;
      
      for (const recent of this.recentGenerations) {
        const distance = levenshteinDistance(fullMessage, recent);
        minFoundDistance = Math.min(minFoundDistance, distance);
        
        if (distance < this.minDistance) {
          isUnique = false;
          break;
        }
      }
      
      if (isUnique) {
        // Add to history
        this.recentGenerations.push(fullMessage);
        if (this.recentGenerations.length > this.maxHistorySize) {
          this.recentGenerations.shift();
        }
        
        // Calculate uniqueness score (1.0 = perfectly unique)
        const uniquenessScore = minFoundDistance === Infinity 
          ? 1.0 
          : Math.min(1.0, minFoundDistance / (this.minDistance * 2));
        
        return { ...result, uniquenessScore };
      }
    }
    
    // If we couldn't generate a unique one, return the last attempt anyway
    logger.warn('Could not generate unique spintax variation', {
      attempts: maxAttempts,
      context: { company: context.company },
    });
    
    return spinAndPersonalize(subjectSpintax, bodySpintax, context);
  }
  
  /**
   * Generate a unique message with HTML-encoded context
   * Use this when the output will be rendered as HTML (email body)
   * Prevents XSS and ensures special characters display correctly
   */
  generateUniqueHtml(
    subjectSpintax: string,
    bodySpintax: string,
    context: PersonalizationContext,
    maxAttempts = 5
  ): SpinResult {
    // HTML-encode the context first for security
    const safeContext = encodeContextForHtml(context);
    return this.generateUnique(subjectSpintax, bodySpintax, safeContext, maxAttempts);
  }
  
  /**
   * Clear the history
   */
  clearHistory(): void {
    this.recentGenerations = [];
  }
  
  /**
   * Get history size
   */
  getHistorySize(): number {
    return this.recentGenerations.length;
  }
}

// ============================================================================
// DATABASE-BACKED OPERATIONS
// ============================================================================

/**
 * Get active templates by type
 */
export async function getTemplates(templateType: string): Promise<SpintaxTemplate[]> {
  const rows = await queryReadOnly<{
    id: string;
    name: string;
    description: string | null;
    template_type: string;
    subject_spintax: string;
    body_spintax: string;
    available_variables: string;
    times_used: number;
    total_opens: number;
    total_clicks: number;
    total_replies: number;
    open_rate: string;
    reply_rate: string;
    active: boolean;
  }>(`
    SELECT id, name, description, template_type, subject_spintax, body_spintax,
           available_variables::text, times_used, total_opens, total_clicks, 
           total_replies, open_rate, reply_rate, active
    FROM sa_spintax_templates
    WHERE template_type = $1 AND active = true
    ORDER BY times_used DESC
  `, [templateType]);
  
  return rows.map(row => ({
    id: row.id,
    name: row.name,
    description: row.description || undefined,
    templateType: row.template_type,
    subjectSpintax: row.subject_spintax,
    bodySpintax: row.body_spintax,
    availableVariables: JSON.parse(row.available_variables || '[]'),
    timesUsed: row.times_used,
    totalOpens: row.total_opens,
    totalClicks: row.total_clicks,
    totalReplies: row.total_replies,
    openRate: parseFloat(row.open_rate || '0'),
    replyRate: parseFloat(row.reply_rate || '0'),
    active: row.active,
  }));
}

/**
 * Get a specific template by ID
 */
export async function getTemplate(templateId: string): Promise<SpintaxTemplate | null> {
  const rows = await queryReadOnly<{
    id: string;
    name: string;
    description: string | null;
    template_type: string;
    subject_spintax: string;
    body_spintax: string;
    available_variables: string;
    times_used: number;
    total_opens: number;
    total_clicks: number;
    total_replies: number;
    open_rate: string;
    reply_rate: string;
    active: boolean;
  }>(`
    SELECT id, name, description, template_type, subject_spintax, body_spintax,
           available_variables::text, times_used, total_opens, total_clicks, 
           total_replies, open_rate, reply_rate, active
    FROM sa_spintax_templates
    WHERE id = $1
  `, [templateId]);
  
  if (rows.length === 0) return null;
  
  const row = rows[0];
  return {
    id: row.id,
    name: row.name,
    description: row.description || undefined,
    templateType: row.template_type,
    subjectSpintax: row.subject_spintax,
    bodySpintax: row.body_spintax,
    availableVariables: JSON.parse(row.available_variables || '[]'),
    timesUsed: row.times_used,
    totalOpens: row.total_opens,
    totalClicks: row.total_clicks,
    totalReplies: row.total_replies,
    openRate: parseFloat(row.open_rate || '0'),
    replyRate: parseFloat(row.reply_rate || '0'),
    active: row.active,
  };
}

/**
 * Record a spintax generation for history tracking
 * Schema: sa_spintax_history has template_id, email_id, resolved_subject, resolved_body, content_hash, sent_at
 */
export async function recordGeneration(params: {
  templateId: string;
  emailId?: string;
  resolvedSubject: string;
  resolvedBody: string;
}): Promise<string> {
  // Generate hash of normalized content for similarity detection
  const normalizedContent = `${params.resolvedSubject.toLowerCase().trim()}\n${params.resolvedBody.toLowerCase().trim()}`;
  const contentHash = require('crypto')
    .createHash('sha256')
    .update(normalizedContent)
    .digest('hex');
  
  const result = await queryOperator<{ id: string }>(`
    INSERT INTO sa_spintax_history 
    (template_id, email_id, resolved_subject, resolved_body, content_hash)
    VALUES ($1, $2, $3, $4, $5)
    RETURNING id
  `, [
    params.templateId,
    params.emailId || null,
    params.resolvedSubject,
    params.resolvedBody,
    contentHash,
  ]);
  
  // Increment times_used count
  await queryOperator(`
    UPDATE sa_spintax_templates SET times_used = times_used + 1, updated_at = CURRENT_TIMESTAMP WHERE id = $1
  `, [params.templateId]);
  
  return result[0].id;
}

/**
 * Check if a variation is unique against recent history
 */
export async function checkVariationUniqueness(
  subject: string,
  body: string,
  lookbackDays = 30
): Promise<{ isUnique: boolean; closestMatch?: { distance: number; sentAt: Date } }> {
  const recentHistory = await queryReadOnly<{
    resolved_subject: string;
    resolved_body: string;
    sent_at: string;
  }>(`
    SELECT resolved_subject, resolved_body, sent_at
    FROM sa_spintax_history
    WHERE sent_at > CURRENT_TIMESTAMP - INTERVAL '1 day' * $1
    ORDER BY sent_at DESC
    LIMIT 20
  `, [lookbackDays]);
  
  const fullMessage = `${subject}\n${body}`;
  let closestMatch: { distance: number; sentAt: Date } | undefined;
  
  for (const row of recentHistory) {
    const previous = `${row.resolved_subject}\n${row.resolved_body}`;
    const distance = levenshteinDistance(fullMessage, previous);
    
    if (!closestMatch || distance < closestMatch.distance) {
      closestMatch = {
        distance,
        sentAt: new Date(row.sent_at),
      };
    }
  }
  
  return {
    isUnique: !closestMatch || closestMatch.distance >= 50, // At least 50 character changes
    closestMatch,
  };
}

/**
 * Create a new template
 */
export async function createTemplate(params: {
  name: string;
  description?: string;
  templateType: string;
  subjectSpintax: string;
  bodySpintax: string;
  availableVariables?: string[];
}): Promise<string> {
  const result = await queryOperator<{ id: string }>(`
    INSERT INTO sa_spintax_templates 
    (name, description, template_type, subject_spintax, body_spintax, available_variables)
    VALUES ($1, $2, $3, $4, $5, $6)
    RETURNING id
  `, [
    params.name,
    params.description || null,
    params.templateType,
    params.subjectSpintax,
    params.bodySpintax,
    JSON.stringify(params.availableVariables || ['first_name', 'company_name', 'app_name', 'app_genre', 'icebreaker']),
  ]);
  
  logger.info('Created new spintax template', { 
    templateType: params.templateType, 
    name: params.name 
  });
  
  return result[0].id;
}

// ============================================================================
// UTILITY: Preview spintax variations
// ============================================================================

/**
 * Generate multiple preview variations (for UI display)
 */
export function previewVariations(
  subjectSpintax: string,
  bodySpintax: string,
  context: PersonalizationContext,
  count = 5
): SpinResult[] {
  const engine = new SpintaxEngine({ minDistance: 5 });
  const results: SpinResult[] = [];
  
  for (let i = 0; i < count; i++) {
    results.push(engine.generateUnique(subjectSpintax, bodySpintax, context));
  }
  
  return results;
}

/**
 * Count possible variations in a spintax string
 */
export function countVariations(spintax: string): number {
  let total = 1;
  const regex = /\{([^{}]+)\}/g;
  let match;
  
  while ((match = regex.exec(spintax)) !== null) {
    const options = match[1].split('|');
    total *= options.length;
  }
  
  return total;
}
