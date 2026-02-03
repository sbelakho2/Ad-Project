import request from 'supertest';
import type { Pool } from 'pg';
import type { Application } from 'express';
import { createTestApp as createExpressApp } from '../helpers/testApp';
import {
  setupTestDatabase,
  teardownTestDatabase,
  cleanDatabase,
} from '../helpers/testDatabase';
import {
  createTestPublisher,
  createTestUser,
  createTestApp as createPublisherApp,
  createTestPlacement,
} from '../helpers/testFixtures';

const describeIfDb = (process.env.SKIP_DB_SETUP === 'true'
  ? describe.skip
  : describe) as typeof describe;

describeIfDb('Migration Studio Imports API', () => {
  let pool: Pool;
  let app: Application;
  let authToken: string;
  let placementId: string;

  beforeAll(async () => {
    pool = await setupTestDatabase();
    app = createExpressApp();
  });

  afterAll(async () => {
    await teardownTestDatabase();
  });

  beforeEach(async () => {
    await cleanDatabase(pool);

    const publisher = await createTestPublisher(pool);
    const user = await createTestUser(pool, publisher.id, { password: 'password123' });
    const appId = await createPublisherApp(pool, publisher.id);
    placementId = await createTestPlacement(pool, appId);

    const loginResponse = await request(app)
      .post('/api/v1/auth/login')
      .send({ email: user.email, password: user.password });

    authToken = loginResponse.body.data.token;
  });

  const withAuth = () => ({ Authorization: `Bearer ${authToken}` });

  it('creates import from CSV and finalizes after confirming mappings', async () => {
    const csvContent = `network,instance_id,instance_name,waterfall_position,ecpm_cents\nironSource,is-1,Rewarded,1,250\nAppLovin,max-1,MAX Video,2,300\n`;

    const createResponse = await request(app)
      .post('/api/v1/migration/import')
      .set(withAuth())
      .field('source', 'csv')
      .field('placement_id', placementId)
      .attach('file', Buffer.from(csvContent, 'utf8'), 'import.csv')
      .expect(201);

    expect(createResponse.body.success).toBe(true);
    const importData = createResponse.body.data;
    expect(importData.summary.total_mappings).toBe(2);
    expect(importData.summary.status_breakdown.pending).toBe(2);

    for (const mapping of importData.mappings) {
      const updateResponse = await request(app)
        .put(`/api/v1/migration/mappings/${mapping.id}`)
        .set(withAuth())
        .send({ our_adapter_id: `adapter-${mapping.id.slice(0, 6)}` })
        .expect(200);

      expect(updateResponse.body.data.mapping.mapping_status).toBe('confirmed');
      expect(updateResponse.body.data.summary.status_breakdown.confirmed).toBeGreaterThanOrEqual(1);
    }

    const finalizeResponse = await request(app)
      .post(`/api/v1/migration/import/${importData.import_id}/finalize`)
      .set(withAuth())
      .expect(200);

    expect(finalizeResponse.body.success).toBe(true);
    expect(finalizeResponse.body.data.import.status).toBe('completed');
    expect(finalizeResponse.body.data.summary.status_breakdown.confirmed).toBe(2);
    expect(finalizeResponse.body.data.summary.unique_networks).toBe(2);
  });

  it('creates import via ironSource connector with credentials', async () => {
    const response = await request(app)
      .post('/api/v1/migration/import')
      .set(withAuth())
      .send({
        source: 'ironSource',
        placement_id: placementId,
        credentials: {
          api_key: 'test-api-key',
          account_id: 'acct-12345',
        },
      })
      .expect(201);

    expect(response.body.success).toBe(true);
    const { mappings, summary } = response.body.data;
    expect(Array.isArray(mappings)).toBe(true);
    expect(mappings.length).toBeGreaterThan(0);
    expect(summary.total_mappings).toBe(mappings.length);
    expect(summary.status_breakdown.pending).toBe(mappings.length);
  });
});
