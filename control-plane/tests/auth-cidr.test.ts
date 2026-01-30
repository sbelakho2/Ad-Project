/**
 * Auth CIDR Calculation Tests
 *
 * Tests for IP allowlist functionality including CIDR range matching.
 * Critical to verify that high first octets (192.x, 172.x, etc.) work
 * correctly despite JavaScript's signed 32-bit integer arithmetic.
 */

import { _internal } from '../src/auth';

const { ipInCidr, isIpAllowed } = _internal;

describe('CIDR Calculation', () => {
  describe('ipInCidr', () => {
    // Basic matching tests
    it('should match exact IP with /32', () => {
      expect(ipInCidr('10.0.0.1', '10.0.0.1/32')).toBe(true);
      expect(ipInCidr('10.0.0.2', '10.0.0.1/32')).toBe(false);
    });

    it('should match /24 subnet', () => {
      expect(ipInCidr('10.0.0.1', '10.0.0.0/24')).toBe(true);
      expect(ipInCidr('10.0.0.255', '10.0.0.0/24')).toBe(true);
      expect(ipInCidr('10.0.1.0', '10.0.0.0/24')).toBe(false);
    });

    it('should match /16 subnet', () => {
      expect(ipInCidr('10.0.0.1', '10.0.0.0/16')).toBe(true);
      expect(ipInCidr('10.0.255.255', '10.0.0.0/16')).toBe(true);
      expect(ipInCidr('10.1.0.0', '10.0.0.0/16')).toBe(false);
    });

    it('should match /0 (any IP)', () => {
      expect(ipInCidr('1.2.3.4', '0.0.0.0/0')).toBe(true);
      expect(ipInCidr('255.255.255.255', '0.0.0.0/0')).toBe(true);
    });

    // Critical: High first octet tests (the bug we fixed)
    describe('High first octet (> 127) - signed int edge case', () => {
      it('should match 192.168.x.x addresses correctly', () => {
        // 192 << 24 is negative in signed 32-bit arithmetic
        // This is the key test for the bug fix
        expect(ipInCidr('192.168.1.1', '192.168.0.0/16')).toBe(true);
        expect(ipInCidr('192.168.255.255', '192.168.0.0/16')).toBe(true);
        expect(ipInCidr('192.169.0.0', '192.168.0.0/16')).toBe(false);
      });

      it('should match 192.168.1.0/24 subnet correctly', () => {
        expect(ipInCidr('192.168.1.1', '192.168.1.0/24')).toBe(true);
        expect(ipInCidr('192.168.1.254', '192.168.1.0/24')).toBe(true);
        expect(ipInCidr('192.168.2.1', '192.168.1.0/24')).toBe(false);
      });

      it('should match 172.16.x.x addresses correctly', () => {
        // 172 << 24 is also negative in signed 32-bit
        expect(ipInCidr('172.16.0.1', '172.16.0.0/12')).toBe(true);
        expect(ipInCidr('172.31.255.255', '172.16.0.0/12')).toBe(true);
        expect(ipInCidr('172.32.0.0', '172.16.0.0/12')).toBe(false);
      });

      it('should match 255.255.255.255 address', () => {
        expect(ipInCidr('255.255.255.255', '255.255.255.0/24')).toBe(true);
        expect(ipInCidr('255.255.254.255', '255.255.255.0/24')).toBe(false);
      });

      it('should match 128.0.0.0/1 (high-bit set range)', () => {
        // All addresses >= 128.0.0.0
        expect(ipInCidr('128.0.0.0', '128.0.0.0/1')).toBe(true);
        expect(ipInCidr('192.168.1.1', '128.0.0.0/1')).toBe(true);
        expect(ipInCidr('255.255.255.255', '128.0.0.0/1')).toBe(true);
        expect(ipInCidr('127.255.255.255', '128.0.0.0/1')).toBe(false);
        expect(ipInCidr('10.0.0.1', '128.0.0.0/1')).toBe(false);
      });
    });

    // Edge cases
    describe('Edge cases', () => {
      it('should handle missing CIDR prefix as /32', () => {
        expect(ipInCidr('10.0.0.1', '10.0.0.1')).toBe(true);
        expect(ipInCidr('10.0.0.2', '10.0.0.1')).toBe(false);
      });

      it('should reject invalid IPs', () => {
        expect(ipInCidr('invalid', '10.0.0.0/24')).toBe(false);
        expect(ipInCidr('10.0.0.1', 'invalid/24')).toBe(false);
        expect(ipInCidr('', '10.0.0.0/24')).toBe(false);
        expect(ipInCidr('10.0.0.1', '')).toBe(false);
      });

      it('should reject out-of-range octets', () => {
        expect(ipInCidr('256.0.0.1', '10.0.0.0/24')).toBe(false);
        expect(ipInCidr('10.0.0.1', '256.0.0.0/24')).toBe(false);
        expect(ipInCidr('-1.0.0.1', '10.0.0.0/24')).toBe(false);
      });

      it('should reject invalid prefix lengths', () => {
        expect(ipInCidr('10.0.0.1', '10.0.0.0/33')).toBe(false);
        expect(ipInCidr('10.0.0.1', '10.0.0.0/-1')).toBe(false);
      });
    });
  });

  describe('isIpAllowed', () => {
    it('should allow all IPs with empty allowlist', () => {
      expect(isIpAllowed('10.0.0.1', [])).toBe(true);
      expect(isIpAllowed('192.168.1.1', [])).toBe(true);
    });

    it('should match exact IPs', () => {
      expect(isIpAllowed('10.0.0.1', ['10.0.0.1'])).toBe(true);
      expect(isIpAllowed('10.0.0.2', ['10.0.0.1'])).toBe(false);
    });

    it('should match CIDR ranges', () => {
      expect(isIpAllowed('10.0.0.50', ['10.0.0.0/24'])).toBe(true);
      expect(isIpAllowed('10.0.1.50', ['10.0.0.0/24'])).toBe(false);
    });

    it('should match any entry in allowlist', () => {
      const allowlist = ['10.0.0.1', '192.168.0.0/16', '172.16.0.0/12'];
      
      expect(isIpAllowed('10.0.0.1', allowlist)).toBe(true);  // Exact match
      expect(isIpAllowed('192.168.5.10', allowlist)).toBe(true);  // CIDR match
      expect(isIpAllowed('172.20.0.1', allowlist)).toBe(true);  // CIDR match
      expect(isIpAllowed('8.8.8.8', allowlist)).toBe(false);  // No match
    });

    it('should handle IPv6-mapped IPv4 addresses', () => {
      expect(isIpAllowed('::ffff:10.0.0.1', ['10.0.0.0/24'])).toBe(true);
      expect(isIpAllowed('::ffff:192.168.1.1', ['192.168.0.0/16'])).toBe(true);
    });

    it('should handle whitespace in allowlist entries', () => {
      expect(isIpAllowed('10.0.0.1', ['  10.0.0.1  '])).toBe(true);
      expect(isIpAllowed('10.0.0.50', ['  10.0.0.0/24  '])).toBe(true);
    });

    it('should skip empty entries', () => {
      expect(isIpAllowed('10.0.0.1', ['', '  ', '10.0.0.1'])).toBe(true);
      expect(isIpAllowed('10.0.0.1', ['', '  '])).toBe(false);
    });
  });
});
