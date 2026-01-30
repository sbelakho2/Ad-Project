import { redactString, redactObject, containsSensitiveData, analyzeSensitiveData } from '../redaction';
import redactionService from '../redaction';

describe('VRA redaction utility', () => {
  describe('email redaction', () => {
    it('redacts plain emails', () => {
      const out = redactString('contact alice@example.com now');
      expect(out).toContain('[REDACTED_EMAIL]');
      expect(out).not.toContain('alice@example.com');
    });

    it('redacts URL-encoded emails', () => {
      const out = redactString('user%40example%2Ecom');
      expect(out).toContain('[REDACTED_EMAIL]');
    });

    it('handles multiple emails', () => {
      const out = redactString('From: a@b.com To: c@d.org');
      expect(out.match(/\[REDACTED_EMAIL\]/g)?.length).toBe(2);
    });
  });

  describe('token redaction', () => {
    it('redacts bearer tokens', () => {
      const out = redactString('token: Bearer abc.def.ghi_xyz-123');
      expect(out).toContain('Bearer [REDACTED]');
      expect(out).not.toContain('abc.def.ghi');
    });

    it('redacts JWT tokens', () => {
      const jwt = 'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U';
      const out = redactString(`token: ${jwt}`);
      expect(out).toContain('[REDACTED_JWT]');
      expect(out).not.toContain('eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9');
    });

    it('redacts OAuth tokens in query strings', () => {
      const out = redactString('callback?access_token=abc123xyz&state=ok');
      expect(out).toContain('access_token=[REDACTED]');
      expect(out).not.toContain('abc123xyz');
    });
  });

  describe('API key redaction', () => {
    it('redacts Stripe keys', () => {
      const out = redactString('sk_test_1234567890abcdefghij sk_live_abcdefghij1234567890');
      expect(out).toContain('sk_test_[REDACTED]');
      expect(out).toContain('sk_live_[REDACTED]');
    });

    it('redacts AWS keys', () => {
      const out = redactString('key: AKIAIOSFODNN7EXAMPLE');
      expect(out).toContain('[REDACTED_AWS_KEY]');
    });

    it('redacts Google API keys', () => {
      const out = redactString('AIzaSyDdI0hCZtE6vySjMm-WEfRq3CPzqKqqsHI');
      expect(out).toContain('[REDACTED_GOOGLE_API_KEY]');
    });

    it('redacts GitHub tokens', () => {
      const out = redactString('ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx');
      expect(out).toContain('[REDACTED_GITHUB_TOKEN]');
    });

    it('redacts SendGrid keys', () => {
      const out = redactString('SG.abc123xyz789.abcdefghijklmnopqrstuvwxyz1234567890abcdef');
      expect(out).toContain('[REDACTED_SENDGRID_KEY]');
    });
  });

  describe('PII redaction', () => {
    it('redacts US phone numbers', () => {
      const inputs = [
        '555-123-4567',
        '(555) 123-4567',
        '+1 555 123 4567',
        '5551234567',
      ];
      for (const phone of inputs) {
        const out = redactString(`Call ${phone} now`);
        expect(out).toContain('[REDACTED_PHONE]');
        expect(out).not.toContain(phone.replace(/[-()\s]/g, '').slice(-4));
      }
    });

    it('redacts SSN', () => {
      const out = redactString('SSN: 123-45-6789');
      expect(out).toContain('[REDACTED_SSN]');
      expect(out).not.toContain('123-45-6789');
    });

    it('redacts IPv4 addresses', () => {
      const out = redactString('Server at 192.168.1.100:8080');
      expect(out).toContain('[REDACTED_IP]');
    });
  });

  describe('credit card redaction', () => {
    it('redacts valid card numbers (Luhn pass)', () => {
      // 4242424242424242 passes Luhn check
      const out = redactString('Card: 4242424242424242');
      expect(out).toContain('[REDACTED_CARD]');
    });

    it('redacts card numbers with separators', () => {
      const out = redactString('Card: 4242-4242-4242-4242');
      expect(out).toContain('[REDACTED_CARD]');
    });

    it('does not redact non-Luhn numbers by default', () => {
      // 1234567890123456 fails Luhn check
      const out = redactString('Number: 1234567890123456');
      // Should NOT be redacted as card since it fails Luhn
      expect(out).not.toContain('[REDACTED_CARD]');
    });

    it('redacts all numeric sequences when Luhn disabled', () => {
      const out = redactString('Number: 1234567890123456', { luhnValidate: false });
      expect(out).toContain('[REDACTED_NUMERIC]');
    });
  });

  describe('infrastructure secrets', () => {
    it('redacts database connection strings', () => {
      const out = redactString('postgresql://user:pass@host:5432/db');
      expect(out).toContain('[REDACTED_DB_URL]');
    });

    it('redacts basic auth in URLs', () => {
      const out = redactString('https://admin:secret123@api.example.com/data');
      expect(out).toContain('[REDACTED_CREDENTIALS]');
    });

    it('redacts long hex secrets', () => {
      const out = redactString('secret: a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4');
      expect(out).toContain('[REDACTED_HEX]');
    });

    it('redacts Slack webhook URLs', () => {
      const out = redactString('https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX');
      expect(out).toContain('[REDACTED_WEBHOOK_URL]');
    });
  });

  describe('redactObject', () => {
    it('redacts string values in objects', () => {
      const obj = { email: 'test@example.com', name: 'John' };
      const out = redactObject(obj);
      expect(out.email).toContain('[REDACTED_EMAIL]');
      expect(out.name).toBe('John');
    });

    it('fully redacts sensitive field names', () => {
      const obj = { password: 'secret123', api_key: 'abc', data: 'safe' };
      const out = redactObject(obj);
      expect(out.password).toBe('[REDACTED]');
      expect(out.api_key).toBe('[REDACTED]');
      expect(out.data).toBe('safe');
    });

    it('handles nested objects', () => {
      const obj = {
        user: { email: 'a@b.com' },
        config: { secret: 'xyz' },
      };
      const out = redactObject(obj);
      expect((out.user as any).email).toContain('[REDACTED_EMAIL]');
      expect((out.config as any).secret).toBe('[REDACTED]');
    });

    it('handles arrays', () => {
      const arr = ['test@example.com', 'safe', 'another@email.org'];
      const out = redactObject(arr);
      expect(out[0]).toContain('[REDACTED_EMAIL]');
      expect(out[1]).toBe('safe');
      expect(out[2]).toContain('[REDACTED_EMAIL]');
    });
  });

  describe('utility functions', () => {
    it('containsSensitiveData returns true for sensitive strings', () => {
      expect(containsSensitiveData('Contact test@example.com')).toBe(true);
      expect(containsSensitiveData('sk_live_1234567890abcdefghij')).toBe(true);
      expect(containsSensitiveData('Normal text without secrets')).toBe(false);
    });

    it('analyzeSensitiveData counts sensitive patterns', () => {
      const input = 'Email a@b.com and b@c.org, phone 555-123-4567';
      const analysis = analyzeSensitiveData(input);
      expect(analysis.email).toBe(2);
      expect(analysis.phone).toBe(1);
    });
  });

  describe('Luhn algorithm', () => {
    it('validates known good card numbers', () => {
      // Visa test card
      expect(redactionService.passesLuhnCheck('4242424242424242')).toBe(true);
      // Mastercard test card
      expect(redactionService.passesLuhnCheck('5555555555554444')).toBe(true);
      // Amex test card
      expect(redactionService.passesLuhnCheck('378282246310005')).toBe(true);
    });

    it('rejects invalid card numbers', () => {
      expect(redactionService.passesLuhnCheck('1234567890123456')).toBe(false);
      expect(redactionService.passesLuhnCheck('0000000000000000')).toBe(true); // Actually passes Luhn
      expect(redactionService.passesLuhnCheck('1111111111111111')).toBe(false);
    });
  });

  describe('edge cases', () => {
    it('handles empty strings', () => {
      expect(redactString('')).toBe('');
      expect(redactString(null as any)).toBe('');
      expect(redactString(undefined as any)).toBe('');
    });

    it('handles strings with no sensitive data', () => {
      const input = 'This is a normal message without any secrets.';
      expect(redactString(input)).toBe(input);
    });

    it('handles mixed sensitive data', () => {
      const input = 'User alice@example.com (SSN: 123-45-6789) called from 555-123-4567 using sk_live_abc123xyz890def456';
      const out = redactString(input);
      expect(out).toContain('[REDACTED_EMAIL]');
      expect(out).toContain('[REDACTED_SSN]');
      expect(out).toContain('[REDACTED_PHONE]');
      expect(out).toContain('sk_live_[REDACTED]');
    });

    it('applies custom patterns', () => {
      const out = redactString('Custom ID: CUST-12345', {
        customPatterns: [
          { pattern: /CUST-\d+/g, replacement: '[REDACTED_CUSTOMER_ID]' },
        ],
      });
      expect(out).toContain('[REDACTED_CUSTOMER_ID]');
    });
  });
});
