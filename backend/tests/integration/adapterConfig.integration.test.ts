import request from 'supertest';
import { Pool } from 'pg';
import { Application } from 'express';
import { createTestApp } from '../helpers/testApp';
import {
  setupTestDatabase,
  teardownTestDatabase,
  cleanDatabase,
} from '../helpers/testDatabase';
import {
  createTestPublisher,
  createTestUser,
  createTestAdapter,
  createTestAdapterConfig,
} from '../helpers/testFixtures';

const describeIfDb = (process.env.SKIP_DB_SETUP === 'true'
  ? describe.skip
  : describe) as typeof describe;

describeIfDb('Adapter Config Integration Tests', () => {
  let pool: Pool;
  let app: Application;
  let authToken: string;
  let publisherId: string;
  let adapterId: string;
  let adapterName: string;

  beforeAll(async () => {
    pool = await setupTestDatabase();
    app = createTestApp();
  });

  afterAll(async () => {
    await teardownTestDatabase();
  });

  beforeEach(async () => {
    await cleanDatabase(pool);

    // Setup test data and authenticate
    const publisher = await createTestPublisher(pool);
    publisherId = publisher.id;

    const user = await createTestUser(pool, publisherId, {
      password: 'password123',
    });

    const adapter = await createTestAdapter(pool);
    adapterId = adapter.id;
    adapterName = adapter.name;

    // Login to get auth token
    const loginResponse = await request(app)
      .post('/api/v1/auth/login')
      .send({
        email: user.email,
        password: user.password,
      });

    authToken = loginResponse.body.data.token;
  });

  describe('GET /api/v1/adapters', () => {
    it('should return empty array when no configs exist', async () => {
      const response = await request(app)
        .get('/api/v1/adapters')
        .set('Authorization', `Bearer ${authToken}`)
        .expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toEqual([]);
    });

    it('should return all adapter configs for publisher', async () => {
      const config = { appId: 'test-app-id', apiKey: 'test-key' };
      await createTestAdapterConfig(pool, publisherId, adapterId, config);

      const response = await request(app)
        .get('/api/v1/adapters')
        .set('Authorization', `Bearer ${authToken}`)
        .expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveLength(1);
      expect(response.body.data[0]).toMatchObject({
        adapterId,
        adapterName,
        enabled: true,
        config,
      });
    });

    it('should return 401 without auth token', async () => {
      await request(app).get('/api/v1/adapters').expect(401);
    });
  });

  describe('POST /api/v1/adapters', () => {
    it('should create a new adapter config', async () => {
      const config = { appId: 'new-app-id', apiKey: 'new-key' };

      const response = await request(app)
        .post('/api/v1/adapters')
        .set('Authorization', `Bearer ${authToken}`)
        .send({
          adapterId,
          config,
        })
        .expect(201);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toMatchObject({
        adapterId,
        adapterName,
        config,
      });
    });

    it('should return 409 when config already exists', async () => {
      const config = { appId: 'test-app-id' };
      await createTestAdapterConfig(pool, publisherId, adapterId, config);

      const response = await request(app)
        .post('/api/v1/adapters')
        .set('Authorization', `Bearer ${authToken}`)
        .send({
          adapterId,
          config: { appId: 'different-app-id' },
        })
        .expect(409);

      expect(response.body.success).toBe(false);
      expect(response.body.error).toContain('already exists');
    });

    it('should return 400 with invalid request data', async () => {
      const response = await request(app)
        .post('/api/v1/adapters')
        .set('Authorization', `Bearer ${authToken}`)
        .send({
          adapterId: 'not-a-uuid',
          config: {},
        })
        .expect(400);

      expect(response.body.success).toBe(false);
    });
  });

  describe('GET /api/v1/adapters/:id', () => {
    it('should return adapter config by ID', async () => {
      const config = { appId: 'test-app-id', apiKey: 'test-key' };
      const configId = await createTestAdapterConfig(pool, publisherId, adapterId, config);

      const response = await request(app)
        .get(`/api/v1/adapters/${configId}`)
        .set('Authorization', `Bearer ${authToken}`)
        .expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toMatchObject({
        id: configId,
        adapterId,
        config,
      });
    });

    it('should return 404 when config not found', async () => {
      const fakeId = '99999999-9999-9999-9999-999999999999';

      const response = await request(app)
        .get(`/api/v1/adapters/${fakeId}`)
        .set('Authorization', `Bearer ${authToken}`)
        .expect(404);

      expect(response.body.success).toBe(false);
    });
  });

  describe('PUT /api/v1/adapters/:id', () => {
    it('should update an existing adapter config', async () => {
      const config = { appId: 'test-app-id', apiKey: 'test-key' };
      const configId = await createTestAdapterConfig(pool, publisherId, adapterId, config);

      const updatedConfig = { appId: 'updated-app-id', apiKey: 'updated-key' };

      const response = await request(app)
        .put(`/api/v1/adapters/${configId}`)
        .set('Authorization', `Bearer ${authToken}`)
        .send({ config: updatedConfig })
        .expect(200);

      expect(response.body.success).toBe(true);
      expect(response.body.data.config).toEqual(updatedConfig);
    });

    it('should return 404 when config not found', async () => {
      const fakeId = '99999999-9999-9999-9999-999999999999';

      const response = await request(app)
        .put(`/api/v1/adapters/${fakeId}`)
        .set('Authorization', `Bearer ${authToken}`)
        .send({ config: { appId: 'test' } })
        .expect(404);

      expect(response.body.success).toBe(false);
    });
  });

  describe('DELETE /api/v1/adapters/:id', () => {
    it('should delete an adapter config', async () => {
      const config = { appId: 'test-app-id' };
      const configId = await createTestAdapterConfig(pool, publisherId, adapterId, config);

      await request(app)
        .delete(`/api/v1/adapters/${configId}`)
        .set('Authorization', `Bearer ${authToken}`)
        .expect(204);

      // Verify deletion
      const getResponse = await request(app)
        .get(`/api/v1/adapters/${configId}`)
        .set('Authorization', `Bearer ${authToken}`)
        .expect(404);

      expect(getResponse.body.success).toBe(false);
    });

    it('should return 404 when config not found', async () => {
      const fakeId = '99999999-9999-9999-9999-999999999999';

      const response = await request(app)
        .delete(`/api/v1/adapters/${fakeId}`)
        .set('Authorization', `Bearer ${authToken}`)
        .expect(404);

      expect(response.body.success).toBe(false);
    });
  });
});
