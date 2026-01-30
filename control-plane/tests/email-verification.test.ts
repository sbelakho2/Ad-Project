import { describe, it, expect, jest, beforeEach } from '@jest/globals';
import dns from 'dns/promises';

// Mock DNS module
jest.mock('dns/promises');

const mockResolveMx = jest.fn();
(dns.resolveMx as jest.Mock) = mockResolveMx;

// Import after mocking
import {
  validateEmailFormat,
  verifyEmailDomain,
  isDisposableEmail,
  isRoleBasedEmail,
  verifyEmail,
  quickValidateEmail,
} from '../src/services/email-verification';

describe('Email Verification Service', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe('validateEmailFormat', () => {
    it('should accept valid email addresses', () => {
      const validEmails = [
        'user@example.com',
        'user.name@example.com',
        'user+tag@example.com',
        'user123@sub.example.com',
        'user_name@example-domain.com',
      ];

      for (const email of validEmails) {
        const result = validateEmailFormat(email);
        expect(result.valid).toBe(true);
        expect(result.errors).toHaveLength(0);
      }
    });

    it('should reject invalid email addresses', () => {
      const invalidEmails = [
        'notanemail',
        '@nodomain.com',
        'user@',
        'user@.com',
        'user@domain',
        'user name@example.com', // space
      ];

      for (const email of invalidEmails) {
        const result = validateEmailFormat(email);
        expect(result.valid).toBe(false);
        expect(result.errors.length).toBeGreaterThan(0);
      }
    });

    it('should normalize email to lowercase', () => {
      const result = validateEmailFormat('USER@EXAMPLE.COM');
      expect(result.normalizedEmail).toBe('user@example.com');
    });

    it('should trim whitespace', () => {
      const result = validateEmailFormat('  user@example.com  ');
      expect(result.normalizedEmail).toBe('user@example.com');
    });

    it('should reject emails exceeding max length', () => {
      const longEmail = 'a'.repeat(65) + '@' + 'b'.repeat(200) + '.com';
      const result = validateEmailFormat(longEmail);
      expect(result.valid).toBe(false);
      expect(result.errors.some(e => e.includes('length'))).toBe(true);
    });
  });

  describe('verifyEmailDomain', () => {
    it('should return valid for domains with MX records', async () => {
      mockResolveMx.mockResolvedValue([
        { priority: 10, exchange: 'mail.example.com' },
        { priority: 20, exchange: 'mail2.example.com' },
      ]);

      const result = await verifyEmailDomain('example.com');
      expect(result.valid).toBe(true);
      expect(result.hasMx).toBe(true);
      expect(result.mxRecords).toContain('mail.example.com');
    });

    it('should return invalid for non-existent domains', async () => {
      const error = new Error('Domain not found');
      (error as any).code = 'ENOTFOUND';
      mockResolveMx.mockRejectedValue(error);

      const result = await verifyEmailDomain('nonexistent.invalid');
      expect(result.valid).toBe(false);
      expect(result.hasMx).toBe(false);
      expect(result.error).toContain('does not exist');
    });

    it('should return invalid for domains without MX records', async () => {
      const error = new Error('No data');
      (error as any).code = 'ENODATA';
      mockResolveMx.mockRejectedValue(error);

      const result = await verifyEmailDomain('nomail.example');
      expect(result.valid).toBe(false);
      expect(result.hasMx).toBe(false);
      expect(result.error).toContain('no MX records');
    });

    it('should not fail on DNS timeout (soft failure)', async () => {
      const error = new Error('Timeout');
      (error as any).code = 'ETIMEOUT';
      mockResolveMx.mockRejectedValue(error);

      const result = await verifyEmailDomain('slow.example.com');
      expect(result.valid).toBe(true); // Don't block on timeout
      expect(result.hasMx).toBe(false);
      expect(result.error).toContain('timed out');
    });
  });

  describe('isDisposableEmail', () => {
    it('should detect known disposable domains', () => {
      const disposableDomains = [
        'mailinator.com',
        'guerrillamail.com',
        'tempmail.com',
        '10minutemail.com',
        'yopmail.com',
      ];

      for (const domain of disposableDomains) {
        expect(isDisposableEmail(domain)).toBe(true);
      }
    });

    it('should not flag legitimate domains', () => {
      const legitimateDomains = [
        'gmail.com',
        'outlook.com',
        'company.com',
        'example.org',
      ];

      for (const domain of legitimateDomains) {
        expect(isDisposableEmail(domain)).toBe(false);
      }
    });
  });

  describe('isRoleBasedEmail', () => {
    it('should detect role-based email prefixes', () => {
      const roleBasedPrefixes = [
        'admin',
        'info',
        'support',
        'sales',
        'contact',
        'noreply',
        'webmaster',
      ];

      for (const prefix of roleBasedPrefixes) {
        expect(isRoleBasedEmail(prefix)).toBe(true);
      }
    });

    it('should not flag personal email prefixes', () => {
      const personalPrefixes = [
        'john',
        'jane.doe',
        'jsmith',
        'alex123',
      ];

      for (const prefix of personalPrefixes) {
        expect(isRoleBasedEmail(prefix)).toBe(false);
      }
    });

    it('should handle prefixes with dots and plus signs', () => {
      expect(isRoleBasedEmail('admin.user')).toBe(true);
      expect(isRoleBasedEmail('info+newsletter')).toBe(true);
      expect(isRoleBasedEmail('john.admin')).toBe(false); // john is the prefix
    });
  });

  describe('verifyEmail (comprehensive)', () => {
    it('should validate a legitimate email', async () => {
      mockResolveMx.mockResolvedValue([
        { priority: 10, exchange: 'mail.example.com' },
      ]);

      const result = await verifyEmail('user@example.com');
      expect(result.valid).toBe(true);
      expect(result.isDisposable).toBe(false);
      expect(result.isRoleBased).toBe(false);
      expect(result.hasMxRecords).toBe(true);
      expect(result.errors).toHaveLength(0);
    });

    it('should reject disposable email when blockDisposable is true', async () => {
      mockResolveMx.mockResolvedValue([
        { priority: 10, exchange: 'mx.mailinator.com' },
      ]);

      const result = await verifyEmail('test@mailinator.com', {
        blockDisposable: true,
      });
      expect(result.valid).toBe(false);
      expect(result.isDisposable).toBe(true);
      expect(result.errors.some(e => e.includes('Disposable'))).toBe(true);
    });

    it('should warn but allow role-based emails by default', async () => {
      mockResolveMx.mockResolvedValue([
        { priority: 10, exchange: 'mail.example.com' },
      ]);

      const result = await verifyEmail('info@example.com', {
        blockRoleBased: false,
      });
      expect(result.valid).toBe(true);
      expect(result.isRoleBased).toBe(true);
      expect(result.warnings.some(w => w.includes('role-based'))).toBe(true);
    });

    it('should reject emails with invalid domains', async () => {
      const error = new Error('No data');
      (error as any).code = 'ENODATA';
      mockResolveMx.mockRejectedValue(error);

      const result = await verifyEmail('user@fakeDomain.invalid');
      expect(result.valid).toBe(false);
      expect(result.hasMxRecords).toBe(false);
    });
  });

  describe('quickValidateEmail', () => {
    it('should quickly validate without MX lookup', () => {
      const result = quickValidateEmail('user@example.com');
      expect(result.valid).toBe(true);
      expect(result.isDisposable).toBe(false);
    });

    it('should detect disposable emails', () => {
      const result = quickValidateEmail('test@mailinator.com');
      expect(result.valid).toBe(false);
      expect(result.isDisposable).toBe(true);
    });
  });
});
