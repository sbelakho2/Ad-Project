import { beforeAll, describe, expect, it } from '@jest/globals';
import type { Application } from 'express';
import request from 'supertest';

import { createTestApp } from '../helpers/testApp';

// These tests exercise the Express-level CORS middleware to ensure
// admin/API consumers get consistent preflight responses. They rely on
// SKIP_DB_SETUP=true to avoid touching Postgres for pure HTTP checks.
describe('CORS preflight handling', () => {
  let app: Application;
  const origin = 'http://localhost:3000';

  const endpoints: Array<{ path: string; method: string }> = [
    { path: '/api/v1/revenue/summary', method: 'GET' },
    { path: '/api/v1/placements', method: 'POST' },
    { path: '/api/v1/data-export/jobs', method: 'GET' },
  ];

  beforeAll(() => {
    process.env.JWT_SECRET = process.env.JWT_SECRET || 'test-secret';
    app = createTestApp();
  });

  endpoints.forEach(({ path, method }) => {
    it(`responds to OPTIONS preflight for ${method} ${path}`, async () => {
      const response = await request(app)
        .options(path)
        .set('Origin', origin)
        .set('Access-Control-Request-Method', method)
        .set('Access-Control-Request-Headers', 'Authorization, Content-Type')
        .expect(204);

      const allowOrigin = response.headers['access-control-allow-origin'];
      expect(allowOrigin).toBe(origin);

      const allowCredentials = response.headers['access-control-allow-credentials'];
      expect(allowCredentials).toBe('true');

      const allowMethods = response.headers['access-control-allow-methods'];
      const methodsString = Array.isArray(allowMethods) ? allowMethods.join(',') : allowMethods;
      expect(typeof methodsString).toBe('string');
      expect((methodsString as string).toUpperCase()).toContain(method);

      const allowHeaders = response.headers['access-control-allow-headers'];
      const headersString = Array.isArray(allowHeaders) ? allowHeaders.join(',') : allowHeaders;
      expect(typeof headersString).toBe('string');
      const normalizedHeaders = (headersString as string).toLowerCase();
      expect(normalizedHeaders).toContain('authorization');
      expect(normalizedHeaders).toContain('content-type');

      const varyHeader = response.headers['vary'];
      if (varyHeader) {
        const varyValues = Array.isArray(varyHeader) ? varyHeader : varyHeader.split(',');
        const normalized = varyValues.map((value: string) => value.trim().toLowerCase());
        expect(normalized).toContain('origin');
      }
    });
  });
});
