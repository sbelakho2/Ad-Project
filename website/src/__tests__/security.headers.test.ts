/**
 * TS variant of security.headers.test.ts — implements the test the stub promised.
 * The Jest coverage config enables TS transforms (babel-jest + next/babel), so this
 * file runs in the 100% gate alongside the JS variant; both must stay green.
 */
// eslint-disable-next-line @typescript-eslint/no-var-requires
const nextConfig = require('../../next.config.js') as {
  headers: () => Promise<{ source: string; headers: { key: string; value: string }[] }[]>;
};

function toMap(headers: { key: string; value: string }[]): Record<string, string> {
  const map: Record<string, string> = {};
  for (const h of headers) {
    map[h.key.toLowerCase()] = h.value;
  }
  return map;
}

describe('Website Security Headers (config) — TS', () => {
  it('defines strict security headers on all routes', async () => {
    const rules = await nextConfig.headers();
    const all = rules.find((r) => r.source === '/:path*');
    expect(all).toBeTruthy();
    const map = toMap(all!.headers);

    expect(map['x-dns-prefetch-control']).toBe('on');
    expect(map['strict-transport-security']).toBe('max-age=63072000; includeSubDomains; preload');
    expect(map['x-content-type-options']).toBe('nosniff');
    expect(map['x-frame-options']).toBe('DENY');
    expect(map['referrer-policy']).toBe('strict-origin-when-cross-origin');
    expect(map['permissions-policy']).toContain('camera=()');

    const csp = map['content-security-policy'];
    expect(csp).toContain("default-src 'self'");
    expect(csp).toContain("frame-ancestors 'none'");
    expect(csp).toContain("object-src 'none'");
    expect(csp).toContain("img-src 'self' data: https:");
    expect(csp).toContain("style-src 'self'");
    expect(csp).toContain("script-src 'self'");
    expect(csp).toContain("connect-src 'self' https:");
    expect(csp).toContain("font-src 'self' data:");
  });
});
