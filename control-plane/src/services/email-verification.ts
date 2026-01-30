/**
 * Email Verification Service
 * 
 * Provides comprehensive email validation including:
 * - Email format validation (RFC 5322)
 * - MX record verification (domain can receive email)
 * - Disposable email detection
 * - Role-based address detection
 * 
 * USAGE:
 *   import { verifyEmail, validateEmailFormat, verifyEmailDomain } from './email-verification';
 *   
 *   // Full verification (includes MX lookup)
 *   const result = await verifyEmail('user@example.com');
 *   if (!result.valid) {
 *     console.log('Verification failed:', result.errors);
 *   }
 */

import dns from 'dns/promises';
import { logger } from '../logger.js';

// ============================================================================
// DISPOSABLE EMAIL DOMAINS
// Common disposable/temporary email services that should be blocked
// ============================================================================

const DISPOSABLE_DOMAINS = new Set([
  // Popular disposable services
  'mailinator.com',
  'guerrillamail.com',
  'guerrillamail.org',
  'guerrillamail.net',
  'guerrillamail.biz',
  'sharklasers.com',
  'grr.la',
  'tempmail.com',
  'temp-mail.org',
  'temp-mail.io',
  '10minutemail.com',
  '10minutemail.net',
  'throwaway.email',
  'fakeinbox.com',
  'yopmail.com',
  'yopmail.fr',
  'trashmail.com',
  'getairmail.com',
  'dispostable.com',
  'mailnesia.com',
  'mailcatch.com',
  'maildrop.cc',
  'getnada.com',
  'nada.email',
  'tempinbox.com',
  'mohmal.com',
  'burnermail.io',
  'discard.email',
  'dropmail.me',
  'harakirimail.com',
  'spamgourmet.com',
  'mytrashmail.com',
  'spamex.com',
  'emailondeck.com',
  'jetable.org',
  'spambox.us',
  'anonymbox.com',
  'emailfake.com',
  'fakemailgenerator.com',
  'inboxbear.com',
  'instantemailaddress.com',
  'mailexpire.com',
  'mailforspam.com',
  'mailnull.com',
  'nomail.xl.cx',
  'nowmymail.com',
  'spam4.me',
  'spamfree24.org',
  'tempr.email',
  'throwawaymail.com',
  'wegwerfmail.de',
  'wegwerfmail.net',
  'wegwerfmail.org',
  // Add more as needed
]);

// ============================================================================
// ROLE-BASED EMAIL PREFIXES
// Generic addresses that typically don't belong to individuals
// ============================================================================

const ROLE_BASED_PREFIXES = new Set([
  'admin',
  'administrator',
  'info',
  'support',
  'sales',
  'contact',
  'hello',
  'help',
  'helpdesk',
  'webmaster',
  'postmaster',
  'hostmaster',
  'abuse',
  'noreply',
  'no-reply',
  'donotreply',
  'do-not-reply',
  'marketing',
  'press',
  'media',
  'hr',
  'jobs',
  'careers',
  'billing',
  'accounts',
  'accounting',
  'legal',
  'privacy',
  'security',
  'feedback',
  'enquiries',
  'enquiry',
  'team',
  'office',
  'reception',
]);

// ============================================================================
// TYPES
// ============================================================================

export interface EmailValidationResult {
  valid: boolean;
  email: string;
  normalizedEmail: string;
  domain: string;
  isDisposable: boolean;
  isRoleBased: boolean;
  hasMxRecords: boolean;
  mxRecords?: string[];
  errors: string[];
  warnings: string[];
}

export interface DomainVerificationResult {
  valid: boolean;
  domain: string;
  hasMx: boolean;
  mxRecords: string[];
  error?: string;
}

// ============================================================================
// RFC 5322 EMAIL FORMAT VALIDATION
// ============================================================================

/**
 * RFC 5322 compliant email regex
 * More strict than the basic regex currently in use
 */
const RFC5322_EMAIL_REGEX = /^(?:[a-z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-z0-9!#$%&'*+/=?^_`{|}~-]+)*|"(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21\x23-\x5b\x5d-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])*")@(?:(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])?|\[(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?|[a-z0-9-]*[a-z0-9]:(?:[\x01-\x08\x0b\x0c\x0e-\x1f\x21-\x5a\x53-\x7f]|\\[\x01-\x09\x0b\x0c\x0e-\x7f])+)\])$/i;

/**
 * Validate email format according to RFC 5322
 */
export function validateEmailFormat(email: string): {
  valid: boolean;
  normalizedEmail: string;
  localPart: string;
  domain: string;
  errors: string[];
} {
  const errors: string[] = [];
  
  // Trim and lowercase
  const normalizedEmail = email.trim().toLowerCase();
  
  // Basic length check
  if (normalizedEmail.length > 254) {
    errors.push('Email address exceeds maximum length (254 characters)');
    return { valid: false, normalizedEmail, localPart: '', domain: '', errors };
  }
  
  // Check for @ symbol
  const atIndex = normalizedEmail.lastIndexOf('@');
  if (atIndex === -1) {
    errors.push('Email address must contain @ symbol');
    return { valid: false, normalizedEmail, localPart: '', domain: '', errors };
  }
  
  const localPart = normalizedEmail.substring(0, atIndex);
  const domain = normalizedEmail.substring(atIndex + 1);
  
  // Local part length check (max 64 characters)
  if (localPart.length > 64) {
    errors.push('Local part exceeds maximum length (64 characters)');
  }
  
  // Domain length check
  if (domain.length > 253) {
    errors.push('Domain exceeds maximum length (253 characters)');
  }
  
  // Domain must have at least one dot
  if (!domain.includes('.')) {
    errors.push('Domain must contain at least one period');
  }
  
  // Check against RFC 5322 regex
  if (!RFC5322_EMAIL_REGEX.test(normalizedEmail)) {
    errors.push('Email format does not conform to RFC 5322');
  }
  
  return {
    valid: errors.length === 0,
    normalizedEmail,
    localPart,
    domain,
    errors,
  };
}

// ============================================================================
// MX RECORD VERIFICATION
// ============================================================================

/**
 * Verify that a domain has valid MX records (can receive email)
 * 
 * @param domain - Domain to check (e.g., "example.com")
 * @returns Verification result with MX records if found
 */
export async function verifyEmailDomain(domain: string): Promise<DomainVerificationResult> {
  try {
    // Try to resolve MX records
    const mxRecords = await dns.resolveMx(domain);
    
    if (mxRecords.length === 0) {
      return {
        valid: false,
        domain,
        hasMx: false,
        mxRecords: [],
        error: 'Domain has no MX records',
      };
    }
    
    // Sort by priority (lower is higher priority)
    const sortedRecords = mxRecords
      .sort((a, b) => a.priority - b.priority)
      .map(r => r.exchange);
    
    return {
      valid: true,
      domain,
      hasMx: true,
      mxRecords: sortedRecords,
    };
  } catch (err: any) {
    const errorCode = err.code || '';
    
    // Handle specific DNS errors
    if (errorCode === 'ENOTFOUND') {
      return {
        valid: false,
        domain,
        hasMx: false,
        mxRecords: [],
        error: 'Domain does not exist',
      };
    }
    
    if (errorCode === 'ENODATA') {
      return {
        valid: false,
        domain,
        hasMx: false,
        mxRecords: [],
        error: 'Domain has no MX records',
      };
    }
    
    if (errorCode === 'ETIMEOUT' || errorCode === 'EAI_AGAIN') {
      // DNS timeout - don't fail the email, just warn
      logger.warn('[EmailVerification] DNS lookup timeout', { domain, error: errorCode });
      return {
        valid: true, // Don't block on timeout
        domain,
        hasMx: false,
        mxRecords: [],
        error: 'DNS lookup timed out - unable to verify MX records',
      };
    }
    
    // Log unexpected errors
    logger.error('[EmailVerification] Unexpected DNS error', { domain, error: err });
    
    return {
      valid: true, // Don't block on unknown errors
      domain,
      hasMx: false,
      mxRecords: [],
      error: `DNS lookup error: ${err.message}`,
    };
  }
}

// ============================================================================
// DISPOSABLE EMAIL DETECTION
// ============================================================================

/**
 * Check if email domain is a known disposable email service
 */
export function isDisposableEmail(domain: string): boolean {
  return DISPOSABLE_DOMAINS.has(domain.toLowerCase());
}

/**
 * Add a domain to the disposable list at runtime
 * (Useful for learning new disposable domains)
 */
export function addDisposableDomain(domain: string): void {
  DISPOSABLE_DOMAINS.add(domain.toLowerCase());
}

// ============================================================================
// ROLE-BASED EMAIL DETECTION
// ============================================================================

/**
 * Check if email is a role-based address (e.g., info@, support@)
 */
export function isRoleBasedEmail(localPart: string): boolean {
  // Get the base prefix (before any dots or plus signs)
  const basePrefix = localPart.toLowerCase().split(/[.+]/)[0];
  return ROLE_BASED_PREFIXES.has(basePrefix);
}

// ============================================================================
// COMPREHENSIVE EMAIL VERIFICATION
// ============================================================================

/**
 * Perform comprehensive email verification
 * 
 * This combines:
 * - Format validation (RFC 5322)
 * - MX record verification
 * - Disposable email detection
 * - Role-based address detection
 * 
 * @param email - Email address to verify
 * @param options - Verification options
 * @returns Comprehensive verification result
 */
export async function verifyEmail(
  email: string,
  options: {
    checkMx?: boolean;
    checkDisposable?: boolean;
    checkRoleBased?: boolean;
    blockDisposable?: boolean;
    blockRoleBased?: boolean;
  } = {}
): Promise<EmailValidationResult> {
  const {
    checkMx = true,
    checkDisposable = true,
    checkRoleBased = true,
    blockDisposable = true,
    blockRoleBased = false, // Warning only by default
  } = options;
  
  const errors: string[] = [];
  const warnings: string[] = [];
  
  // Step 1: Format validation
  const formatResult = validateEmailFormat(email);
  if (!formatResult.valid) {
    return {
      valid: false,
      email,
      normalizedEmail: formatResult.normalizedEmail,
      domain: formatResult.domain,
      isDisposable: false,
      isRoleBased: false,
      hasMxRecords: false,
      errors: formatResult.errors,
      warnings,
    };
  }
  
  const { normalizedEmail, localPart, domain } = formatResult;
  
  // Step 2: Disposable email check
  const isDisposable = checkDisposable && isDisposableEmail(domain);
  if (isDisposable) {
    if (blockDisposable) {
      errors.push('Disposable email addresses are not allowed');
    } else {
      warnings.push('Email appears to be a disposable/temporary address');
    }
  }
  
  // Step 3: Role-based email check
  const isRoleBased = checkRoleBased && isRoleBasedEmail(localPart);
  if (isRoleBased) {
    if (blockRoleBased) {
      errors.push('Role-based email addresses (info@, support@, etc.) are not allowed');
    } else {
      warnings.push('Email appears to be a role-based address - personal emails have higher engagement');
    }
  }
  
  // Step 4: MX record verification
  let hasMxRecords = false;
  let mxRecords: string[] = [];
  
  if (checkMx) {
    const mxResult = await verifyEmailDomain(domain);
    hasMxRecords = mxResult.hasMx;
    mxRecords = mxResult.mxRecords;
    
    if (!mxResult.valid) {
      errors.push(mxResult.error || 'Domain cannot receive email');
    } else if (!mxResult.hasMx && mxResult.error) {
      warnings.push(mxResult.error);
    }
  }
  
  return {
    valid: errors.length === 0,
    email,
    normalizedEmail,
    domain,
    isDisposable,
    isRoleBased,
    hasMxRecords,
    mxRecords,
    errors,
    warnings,
  };
}

// ============================================================================
// QUICK VALIDATION (NO ASYNC)
// ============================================================================

/**
 * Quick synchronous email validation (no MX lookup)
 * Use this for real-time form validation
 */
export function quickValidateEmail(email: string): {
  valid: boolean;
  isDisposable: boolean;
  isRoleBased: boolean;
  errors: string[];
} {
  const errors: string[] = [];
  
  const formatResult = validateEmailFormat(email);
  if (!formatResult.valid) {
    return {
      valid: false,
      isDisposable: false,
      isRoleBased: false,
      errors: formatResult.errors,
    };
  }
  
  const isDisposable = isDisposableEmail(formatResult.domain);
  if (isDisposable) {
    errors.push('Disposable email addresses are not allowed');
  }
  
  const isRoleBased = isRoleBasedEmail(formatResult.localPart);
  // Role-based is a warning, not an error by default
  
  return {
    valid: errors.length === 0,
    isDisposable,
    isRoleBased,
    errors,
  };
}

// ============================================================================
// BATCH VERIFICATION
// ============================================================================

/**
 * Verify multiple emails in parallel with rate limiting
 * 
 * @param emails - Array of emails to verify
 * @param concurrency - Max concurrent verifications (default: 10)
 * @returns Map of email to verification result
 */
export async function verifyEmailBatch(
  emails: string[],
  concurrency: number = 10
): Promise<Map<string, EmailValidationResult>> {
  const results = new Map<string, EmailValidationResult>();
  
  // Process in batches
  for (let i = 0; i < emails.length; i += concurrency) {
    const batch = emails.slice(i, i + concurrency);
    const batchResults = await Promise.all(
      batch.map(email => verifyEmail(email))
    );
    
    batch.forEach((email, index) => {
      results.set(email, batchResults[index]);
    });
  }
  
  return results;
}
