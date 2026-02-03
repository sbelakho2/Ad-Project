import request from 'supertest';
import { Pool } from 'pg';
import { Application } from 'express';
import { createTestApp } from '../helpers/testApp';
import { setupTestDatabase, teardownTestDatabase, cleanDatabase } from '../helpers/testDatabase';
import { createTestPublisher, createTestUser } from '../helpers/testFixtures';

const describeIfDb = (process.env.SKIP_DB_SETUP === 'true'
  ? describe.skip
  : describe) as typeof describe;

/**
 * Integration tests for Data Export routes
 */
describeIfDb('Data Export Integration', () => {
  let pool: Pool;
  let app: Application;
  let authToken: string;

  beforeAll(async () => {
    pool = await setupTestDatabase();
    app = createTestApp();
  });

  afterAll(async () => {
    await teardownTestDatabase();
  });

  beforeEach(async () => {
    await cleanDatabase(pool);

    const publisher = await createTestPublisher(pool);
    const user = await createTestUser(pool, publisher.id, { password: 'password123' });

    const loginResponse = await request(app)
      .post('/api/v1/auth/login')
      .send({ email: user.email, password: user.password });

    authToken = loginResponse.body.data.token;
  });

  it('should create a data export job (local JSON)', async () => {
    const now = new Date();
    const start = new Date(now.getTime() - 24 * 60 * 60 * 1000);

    const response = await request(app)
      .post('/api/v1/data-export/jobs')
      .set('Authorization', `Bearer ${authToken}`)
      .send({
        dataType: 'impressions',
        startDate: start.toISOString(),
        endDate: now.toISOString(),
        config: {
          format: 'json',
          compression: 'none',
          destination: { type: 'local' },
        },
      })
      .expect(201);

    expect(response.body.success).toBe(true);
    expect(response.body.data).toBeDefined();
    expect(response.body.data.id).toMatch(/^job-/);
    expect(response.body.meta.statusEndpoint).toContain('/api/data-export/jobs/');
  });
});
