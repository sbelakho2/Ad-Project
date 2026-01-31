import { redactString as redactCsvField } from '../redaction';
import { redactLogInfo } from '../../../utils/logger';

describe('VRA redaction — reason_code safety for CSV and logs', () => {
  it('CSV redaction removes emails, bearer tokens, stripe keys and card-like numbers', () => {
    const before = 'contact jane@example.com, token Bearer super.secret, sk_live_abc123, 4111111111111111';
    const out = redactCsvField(before);
    expect(out).toContain('[REDACTED_EMAIL]');
    expect(out).toContain('Bearer [REDACTED]');
    expect(out).toContain('sk_live_[REDACTED]');
    expect(out).toMatch(/\[REDACTED_(NUMERIC|CARD)\]/);
  });

  it('Logger redaction masks reason_code field and nested structures', () => {
    const before = {
      message: 'emit delta with reason_code containing ops@example.com Bearer abc',
      reason_code: 'ops@example.com Bearer abc',
      nested: { reason_code: 'sk_test_foo 4242424242424242' },
    } as any;
    const out = redactLogInfo({ ...before });
    // message should be scrubbed
    expect(String(out.message)).toContain('[REDACTED_EMAIL]');
    expect(String(out.message)).toContain('Bearer [REDACTED]');
    // structured field should be redacted deeply
    expect(out.reason_code).toBe('[REDACTED]');
    expect((out as any).nested.reason_code).toBe('[REDACTED]');
  });
});
