/**
 * SSAI routes tests
 * Focus: route wiring + auth guards (operator session)
 */

import request from 'supertest';
import express, { Application } from 'express';
import ssaiRoutes from '../../routes/ssai';

describe('SSAI Routes', () => {
  let app: Application;

  beforeAll(() => {
    process.env.OPERATOR_SECRET = 'test-operator-secret';
    app = express();
    app.use(express.json());
    app.use('/api/v1/ssai', ssaiRoutes);
  });

  describe('GET /api/v1/ssai/health/live', () => {
    it('responds 200 (liveness)', async () => {
      await request(app).get('/api/v1/ssai/health/live').expect(200);
    });
  });

  describe('GET /api/v1/ssai/admin/dashboard', () => {
    it('denies without operator secret', async () => {
      await request(app).get('/api/v1/ssai/admin/dashboard').expect(403);
    });

    it('allows with operator secret header', async () => {
      const res = await request(app)
        .get('/api/v1/ssai/admin/dashboard')
        .set('x-operator-secret', 'test-operator-secret')
        .expect(200);

      expect(res.body).toHaveProperty('ssai');
      expect(res.body.ssai).toHaveProperty('safeMode');
    });
  });

  describe('POST /api/v1/ssai/sessions', () => {
    it('requires SDK API key', async () => {
      await request(app)
        .post('/api/v1/ssai/sessions')
        .send({ tenantId: 't', assetId: 'a', protocol: 'hls' })
        .expect(401);
    });
  });
});
