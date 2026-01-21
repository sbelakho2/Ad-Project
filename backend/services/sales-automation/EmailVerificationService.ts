// services/sales-automation/EmailVerificationService.ts
// Proper email verification beyond just MX checks
// Critical for reducing bounces and protecting sender reputation

import { Pool } from 'pg';
import dns from 'dns';
import net from 'net';
import { promisify } from 'util';

const resolveMx = promisify(dns.resolveMx);
const resolveA = promisify(dns.resolve4);

interface VerificationResult {
  email: string;
  isValid: boolean;
  status: 'valid' | 'invalid' | 'risky' | 'unknown' | 'catch_all' | 'disposable';
  reason: string;
  details: {
    formatValid: boolean;
    domainExists: boolean;
    mxRecords: boolean;
    smtpCheck?: boolean;
    isCatchAll?: boolean;
    isDisposable?: boolean;
    isRoleAccount?: boolean;
    isFreeProvider?: boolean;
    riskScore: number;  // 0-100, lower is better
  };
}

// Known disposable email domains
const DISPOSABLE_DOMAINS = new Set([
  'tempmail.com', 'guerrillamail.com', 'mailinator.com', '10minutemail.com',
  'throwaway.email', 'temp-mail.org', 'fakeinbox.com', 'getnada.com',
  'maildrop.cc', 'dispostable.com', 'trashmail.com', 'yopmail.com',
  'sharklasers.com', 'grr.la', 'guerrillamail.info', 'spam4.me',
]);

// Known free email providers (not necessarily invalid, but lower quality for B2B)
const FREE_PROVIDERS = new Set([
  'gmail.com', 'yahoo.com', 'hotmail.com', 'outlook.com', 'aol.com',
  'icloud.com', 'mail.com', 'protonmail.com', 'zoho.com', 'yandex.com',
  'gmx.com', 'live.com', 'msn.com', 'qq.com', '163.com', '126.com',
]);

// Role-based email prefixes (generic, less likely to be read)
const ROLE_PREFIXES = new Set([
  'info', 'contact', 'hello', 'support', 'sales', 'admin', 'webmaster',
  'postmaster', 'hostmaster', 'abuse', 'noreply', 'no-reply', 'team',
  'office', 'marketing', 'press', 'media', 'help', 'service', 'billing',
  'hr', 'jobs', 'careers', 'feedback', 'enquiries', 'inquiries',
]);

// Catch-all detection patterns in SMTP responses
const CATCH_ALL_PATTERNS = [
  /all mail.*accepted/i,
  /catch.?all/i,
  /will accept/i,
];

export class EmailVerificationService {
  private pool: Pool;
  private verificationCache: Map<string, VerificationResult> = new Map();
  private domainMxCache: Map<string, dns.MxRecord[] | null> = new Map();

  constructor(databaseUrl: string) {
    this.pool = new Pool({ connectionString: databaseUrl });
  }

  /**
   * Verify a single email address
   */
  async verifyEmail(email: string): Promise<VerificationResult> {
    // Check cache first
    const cached = this.verificationCache.get(email.toLowerCase());
    if (cached) return cached;

    const result = await this.performVerification(email);
    
    // Cache for 24 hours
    this.verificationCache.set(email.toLowerCase(), result);
    setTimeout(() => this.verificationCache.delete(email.toLowerCase()), 24 * 60 * 60 * 1000);

    // Save to database
    await this.saveVerificationResult(email, result);

    return result;
  }

  /**
   * Verify multiple emails in batch
   */
  async verifyBatch(emails: string[]): Promise<Map<string, VerificationResult>> {
    const results = new Map<string, VerificationResult>();

    // Group by domain for efficiency
    const byDomain = new Map<string, string[]>();
    for (const email of emails) {
      const domain = email.split('@')[1]?.toLowerCase();
      if (domain) {
        const list = byDomain.get(domain) || [];
        list.push(email);
        byDomain.set(domain, list);
      }
    }

    // Verify domain by domain
    for (const [domain, domainEmails] of byDomain) {
      // Check MX once per domain
      const mxRecords = await this.getMxRecords(domain);

      for (const email of domainEmails) {
        const result = await this.performVerification(email, mxRecords);
        results.set(email, result);
        
        // Small delay to avoid rate limiting
        await this.delay(100);
      }
    }

    return results;
  }

  /**
   * Perform actual verification
   */
  private async performVerification(
    email: string, 
    mxRecords?: dns.MxRecord[] | null
  ): Promise<VerificationResult> {
    const details: VerificationResult['details'] = {
      formatValid: false,
      domainExists: false,
      mxRecords: false,
      riskScore: 50,  // Start neutral
    };

    // 1. Format validation
    const formatRegex = /^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/;
    if (!formatRegex.test(email)) {
      return {
        email,
        isValid: false,
        status: 'invalid',
        reason: 'Invalid email format',
        details: { ...details, formatValid: false, riskScore: 100 },
      };
    }
    details.formatValid = true;
    details.riskScore -= 10;

    const [localPart, domain] = email.toLowerCase().split('@');

    // 2. Check for disposable domain
    if (DISPOSABLE_DOMAINS.has(domain)) {
      return {
        email,
        isValid: false,
        status: 'disposable',
        reason: 'Disposable email domain',
        details: { ...details, isDisposable: true, riskScore: 100 },
      };
    }

    // 3. Check for free provider
    details.isFreeProvider = FREE_PROVIDERS.has(domain);
    if (details.isFreeProvider) {
      details.riskScore += 20;  // Higher risk for B2B
    }

    // 4. Check for role-based email
    details.isRoleAccount = ROLE_PREFIXES.has(localPart.split('.')[0]);
    if (details.isRoleAccount) {
      details.riskScore += 15;  // Role accounts less likely to engage
    }

    // 5. Check domain exists (DNS lookup)
    try {
      await resolveA(domain);
      details.domainExists = true;
      details.riskScore -= 10;
    } catch {
      // Try without A record, some domains only have MX
    }

    // 6. Check MX records
    if (!mxRecords) {
      mxRecords = await this.getMxRecords(domain);
    }

    if (!mxRecords || mxRecords.length === 0) {
      return {
        email,
        isValid: false,
        status: 'invalid',
        reason: 'No MX records found',
        details: { ...details, mxRecords: false, riskScore: 100 },
      };
    }
    details.mxRecords = true;
    details.domainExists = true;
    details.riskScore -= 10;

    // 7. SMTP verification (optional, can trigger spam filters)
    // Only do this for high-value prospects
    const smtpResult = await this.smtpVerify(email, mxRecords[0].exchange);
    details.smtpCheck = smtpResult.valid;
    details.isCatchAll = smtpResult.catchAll;

    if (smtpResult.catchAll) {
      details.riskScore += 10;
      return {
        email,
        isValid: true,
        status: 'catch_all',
        reason: 'Domain accepts all emails (catch-all)',
        details,
      };
    }

    if (!smtpResult.valid && smtpResult.definitive) {
      return {
        email,
        isValid: false,
        status: 'invalid',
        reason: smtpResult.reason || 'SMTP verification failed',
        details: { ...details, riskScore: 100 },
      };
    }

    // 8. Calculate final status
    if (details.riskScore <= 30) {
      return {
        email,
        isValid: true,
        status: 'valid',
        reason: 'Email verified',
        details,
      };
    } else if (details.riskScore <= 60) {
      return {
        email,
        isValid: true,
        status: 'risky',
        reason: 'Email may be valid but has risk factors',
        details,
      };
    } else {
      return {
        email,
        isValid: false,
        status: 'unknown',
        reason: 'Could not verify email',
        details,
      };
    }
  }

  /**
   * Get MX records for domain (cached)
   */
  private async getMxRecords(domain: string): Promise<dns.MxRecord[] | null> {
    if (this.domainMxCache.has(domain)) {
      return this.domainMxCache.get(domain) || null;
    }

    try {
      const records = await resolveMx(domain);
      const sorted = records.sort((a, b) => a.priority - b.priority);
      this.domainMxCache.set(domain, sorted);
      
      // Cache for 1 hour
      setTimeout(() => this.domainMxCache.delete(domain), 60 * 60 * 1000);
      
      return sorted;
    } catch {
      this.domainMxCache.set(domain, null);
      return null;
    }
  }

  /**
   * SMTP verification (connect and check if mailbox exists)
   * Note: This can trigger spam filters if done aggressively
   */
  private async smtpVerify(
    email: string, 
    mxHost: string
  ): Promise<{ valid: boolean; catchAll: boolean; definitive: boolean; reason?: string }> {
    return new Promise((resolve) => {
      const timeout = 10000;  // 10 second timeout
      let resolved = false;

      const socket = new net.Socket();
      
      const cleanup = () => {
        if (!resolved) {
          resolved = true;
          socket.destroy();
        }
      };

      socket.setTimeout(timeout);

      socket.on('timeout', () => {
        cleanup();
        resolve({ valid: true, catchAll: false, definitive: false, reason: 'Timeout' });
      });

      socket.on('error', () => {
        cleanup();
        resolve({ valid: true, catchAll: false, definitive: false, reason: 'Connection error' });
      });

      socket.connect(25, mxHost, () => {
        let step = 0;
        let response = '';
        const fakeEmail = `verify-${Date.now()}@apexmediation.ee`;

        socket.on('data', (data) => {
          response += data.toString();

          // Check for catch-all indicators
          for (const pattern of CATCH_ALL_PATTERNS) {
            if (pattern.test(response)) {
              cleanup();
              resolve({ valid: true, catchAll: true, definitive: true });
              return;
            }
          }

          if (step === 0 && response.includes('220')) {
            step = 1;
            socket.write(`HELO apexmediation.ee\r\n`);
            response = '';
          } else if (step === 1 && response.includes('250')) {
            step = 2;
            socket.write(`MAIL FROM:<${fakeEmail}>\r\n`);
            response = '';
          } else if (step === 2 && response.includes('250')) {
            step = 3;
            socket.write(`RCPT TO:<${email}>\r\n`);
            response = '';
          } else if (step === 3) {
            cleanup();
            
            if (response.includes('250')) {
              resolve({ valid: true, catchAll: false, definitive: true });
            } else if (response.includes('550') || response.includes('551') || response.includes('552') || response.includes('553')) {
              // Definitive rejection
              resolve({ valid: false, catchAll: false, definitive: true, reason: 'Mailbox does not exist' });
            } else if (response.includes('450') || response.includes('451') || response.includes('452')) {
              // Temporary issue, treat as unknown
              resolve({ valid: true, catchAll: false, definitive: false, reason: 'Temporary rejection' });
            } else {
              resolve({ valid: true, catchAll: false, definitive: false, reason: 'Unknown response' });
            }
          }
        });
      });

      // Timeout fallback
      setTimeout(() => {
        if (!resolved) {
          cleanup();
          resolve({ valid: true, catchAll: false, definitive: false, reason: 'Timeout' });
        }
      }, timeout);
    });
  }

  /**
   * Save verification result to database
   */
  private async saveVerificationResult(email: string, result: VerificationResult): Promise<void> {
    await this.pool.query(`
      UPDATE sales_contacts
      SET email_verified = $2,
          email_verification_status = $3,
          verification_details = $4,
          updated_at = CURRENT_TIMESTAMP
      WHERE email = $1
    `, [
      email,
      result.isValid,
      result.status,
      JSON.stringify(result.details),
    ]);
  }

  /**
   * Get verification statistics
   */
  async getVerificationStats(): Promise<{
    total: number;
    valid: number;
    invalid: number;
    risky: number;
    catchAll: number;
    unknown: number;
  }> {
    const result = await this.pool.query(`
      SELECT 
        COUNT(*) as total,
        COUNT(*) FILTER (WHERE email_verification_status = 'valid') as valid,
        COUNT(*) FILTER (WHERE email_verification_status = 'invalid') as invalid,
        COUNT(*) FILTER (WHERE email_verification_status = 'risky') as risky,
        COUNT(*) FILTER (WHERE email_verification_status = 'catch_all') as catch_all,
        COUNT(*) FILTER (WHERE email_verification_status = 'unknown' OR email_verification_status IS NULL) as unknown
      FROM sales_contacts
      WHERE email IS NOT NULL
    `);

    const row = result.rows[0];
    return {
      total: parseInt(row.total),
      valid: parseInt(row.valid),
      invalid: parseInt(row.invalid),
      risky: parseInt(row.risky),
      catchAll: parseInt(row.catch_all),
      unknown: parseInt(row.unknown),
    };
  }

  /**
   * Run batch verification on unverified contacts
   */
  async verifyUnverifiedContacts(limit: number = 100): Promise<number> {
    const contacts = await this.pool.query(`
      SELECT email FROM sales_contacts
      WHERE email_verification_status IS NULL
        OR email_verification_status = 'unknown'
      LIMIT $1
    `, [limit]);

    let verified = 0;
    for (const contact of contacts.rows) {
      await this.verifyEmail(contact.email);
      verified++;
      await this.delay(500);  // Rate limit
    }

    return verified;
  }

  private delay(ms: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, ms));
  }
}

export default EmailVerificationService;
