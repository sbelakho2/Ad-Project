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
  customFields?: Record<string, string>;
}

export interface SpinResult {
  subject: string;
  body: string;
  variationHash: string;
  uniquenessScore: number;
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
 */
export function spin(spintax: string): string {
  // Find the innermost {options} and replace
  let result = spintax;
  let iterations = 0;
  const maxIterations = 100; // Prevent infinite loops
  
  while (result.includes('{') && iterations < maxIterations) {
    // Find innermost braces (no nested braces inside)
    const match = result.match(/\{([^{}]+)\}/);
    if (!match) break;
    
    const options = match[1].split('|');
    const choice = options[Math.floor(Math.random() * options.length)];
    
    // Replace only the first occurrence
    result = result.replace(match[0], choice);
    iterations++;
  }
  
  return result.trim();
}

/**
 * Apply personalization context to a template.
 * Replaces {{VARIABLE}} placeholders with actual values.
 */
export function personalize(template: string, context: PersonalizationContext): string {
  let result = template;
  
  // Standard replacements
  result = result.replace(/\{\{FIRST_NAME\}\}/gi, context.firstName || 'there');
  result = result.replace(/\{\{LAST_NAME\}\}/gi, context.lastName || '');
  result = result.replace(/\{\{COMPANY\}\}/gi, context.company || 'your company');
  result = result.replace(/\{\{APP_NAME\}\}/gi, context.appName || 'your app');
  result = result.replace(/\{\{APP_GENRE\}\}/gi, context.appGenre || 'mobile');
  result = result.replace(/\{\{ICEBREAKER\}\}/gi, context.icebreaker || '');
  
  // Custom fields
  if (context.customFields) {
    for (const [key, value] of Object.entries(context.customFields)) {
      const regex = new RegExp(`\\{\\{${key.toUpperCase()}\\}\\}`, 'gi');
      result = result.replace(regex, value);
    }
  }
  
  // Clean up double spaces and empty placeholders
  result = result.replace(/\s+/g, ' ').trim();
  
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
  const subject = personalize(spin(subjectSpintax), context);
  const body = personalize(spin(bodySpintax), context);
  
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
    this.minDistance = options?.minDistance || 10;
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
