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
 * Integration tests for A/B Testing routes
 */
describeIfDb('A/B Testing Integration', () => {
  let pool: Pool;
  let app: Application;
  let authToken: string;
  let publisherId: string;

  beforeAll(async () => {
    pool = await setupTestDatabase();
    app = createTestApp();
  });

  afterAll(async () => {
    await teardownTestDatabase();
  });

  beforeEach(async () => {
    await cleanDatabase(pool);

    // Setup publisher and user
    const publisher = await createTestPublisher(pool);
    publisherId = publisher.id;
    const user = await createTestUser(pool, publisherId, { password: 'password123' });

    // Login to get auth token
    const loginResponse = await request(app)
      .post('/api/v1/auth/login')
      .send({ email: user.email, password: user.password });

    authToken = loginResponse.body.data.token;
  });

  it('should create and retrieve an experiment', async () => {
    const createResponse = await request(app)
      .post('/api/v1/ab-testing/experiments')
      .set('Authorization', `Bearer ${authToken}`)
      .send({
        name: 'Banner eCPM optimization',
        description: 'Test different floor prices',
        type: 'floor_price',
        variants: [
          { name: 'Control', trafficAllocation: 50, configuration: { floor: 0.5 } },
          { name: 'Variant A', trafficAllocation: 50, configuration: { floor: 0.7 } },
        ],
        targetSampleSize: 1000,
        confidenceLevel: 0.95,
      })
      .expect(201);

    expect(createResponse.body.success).toBe(true);
    const experimentId = createResponse.body.data.id as string;
    expect(experimentId).toBeTruthy();
    expect(createResponse.body.data.variants.length).toBe(2);

    const getResponse = await request(app)
      .get(`/api/v1/ab-testing/experiments/${experimentId}`)
      .set('Authorization', `Bearer ${authToken}`)
      .expect(200);

    expect(getResponse.body.success).toBe(true);
    expect(getResponse.body.data.id).toBe(experimentId);
    expect(getResponse.body.data.publisherId).toBe(publisherId);
    expect(Array.isArray(getResponse.body.data.variants)).toBe(true);
    expect(getResponse.body.data.variants.length).toBe(2);
  });
});
