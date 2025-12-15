/**
 * SSAI Security Handler Unit Tests
 */

import { SecurityHandler } from '../session/SecurityHandler';

describe('SecurityHandler', () => {
  let handler: SecurityHandler;

  beforeEach(() => {
    handler = new SecurityHandler('test-secret-key-for-testing-only');
  });

  describe('generateSessionToken', () => {
    it('should generate valid token', () => {
      const token = handler.generateSessionToken('sess-1', 'tenant-1');

      expect(token).toBeDefined();
      expect(typeof token).toBe('string');
      expect(token.length).toBeGreaterThan(0);
    });

    it('should include session ID in token', () => {
      const token = handler.generateSessionToken('sess-1', 'tenant-1');

      const result = handler.validateSessionToken(token);

      expect(result.valid).toBe(true);
      expect(result.decoded?.sessionId).toBe('sess-1');
    });

    it('should include tenant ID in token', () => {
      const token = handler.generateSessionToken('sess-1', 'tenant-1');

      const result = handler.validateSessionToken(token);

      expect(result.valid).toBe(true);
      expect(result.decoded?.tenantId).toBe('tenant-1');
    });
  });

  describe('validateSessionToken', () => {
    it('should validate correct token', () => {
      const token = handler.generateSessionToken('sess-1', 'tenant-1');

      const result = handler.validateSessionToken(token);

      expect(result.valid).toBe(true);
      expect(result.decoded?.sessionId).toBe('sess-1');
    });

    it('should reject tampered token', () => {
      const token = handler.generateSessionToken('sess-1', 'tenant-1');

      const tamperedToken = token.slice(0, -5) + 'xxxxx';

      const result = handler.validateSessionToken(tamperedToken);

      expect(result.valid).toBe(false);
      expect(result.error).toBeDefined();
    });

    it('should reject expired token', async () => {
      // Create a handler where we'll manually create an already-expired token
      const now = Math.floor(Date.now() / 1000);
      const expiredTokenData = {
        sessionId: 'sess-1',
        tenantId: 'tenant-1',
        issuedAt: now - 3600, // issued 1 hour ago
        expiresAt: now - 1,   // expired 1 second ago
      };
      
      // Create signature with same logic as handler
      const crypto = require('crypto');
      const signature = crypto
        .createHmac('sha256', 'test-secret-expired')
        .update(JSON.stringify(expiredTokenData))
        .digest('hex');
      
      const expiredToken = Buffer.from(
        JSON.stringify({ ...expiredTokenData, signature })
      ).toString('base64url');
      
      const expiredHandler = new SecurityHandler('test-secret-expired');
      const result = expiredHandler.validateSessionToken(expiredToken);

      expect(result.valid).toBe(false);
      expect(result.error).toBe('Token expired');
    });
  });

  describe('refreshSessionToken', () => {
    it('should refresh valid token', () => {
      const token = handler.generateSessionToken('sess-1', 'tenant-1');

      const refreshed = handler.refreshSessionToken(token);

      expect(refreshed).not.toBeNull();
      
      const result = handler.validateSessionToken(refreshed!);
      expect(result.valid).toBe(true);
      expect(result.decoded?.sessionId).toBe('sess-1');
    });

    it('should return null for invalid token', () => {
      const refreshed = handler.refreshSessionToken('invalid-token');

      expect(refreshed).toBeNull();
    });
  });

  describe('rate limiting', () => {
    it('should allow requests within limit', () => {
      const result = handler.checkTenantRateLimit('tenant-1');

      expect(result.allowed).toBe(true);
    });

    it('should track request count', () => {
      handler.checkTenantRateLimit('tenant-1');
      handler.checkTenantRateLimit('tenant-1');
      const result = handler.checkTenantRateLimit('tenant-1');

      expect(result.remaining).toBeLessThan(1000); // Default maxRequests is 1000
    });

    it('should allow session rate limit', () => {
      const result = handler.checkSessionRateLimit('sess-1');

      expect(result.allowed).toBe(true);
    });

    it('should allow IP rate limit', () => {
      const result = handler.checkIpRateLimit('192.168.1.1');

      expect(result.allowed).toBe(true);
    });
  });
});
