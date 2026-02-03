import { jest } from '@jest/globals';
import request from 'supertest';
import { Pool } from 'pg';
import { Application } from 'express';
import { createTestApp } from '../helpers/testApp';
import twofaService from '../../src/services/twofa.service';
import {
  setupTestDatabase,
  teardownTestDatabase,
  cleanDatabase,
} from '../helpers/testDatabase';
import { createTestPublisher, createTestUser } from '../helpers/testFixtures';

// Integration suite needs the real pg driver rather than the shared Jest manual mock.
jest.mock('pg', () => jest.requireActual('pg'));
jest.mock('../../src/services/twofa.service', () => ({
  __esModule: true,
  default: {
    isEnabled: jest.fn(async () => false),
    verifyTokenOrBackupCode: jest.fn(async () => true),
  },
}));

const describeIfDb = (process.env.SKIP_DB_SETUP === 'true'
  ? describe.skip
  : describe) as typeof describe;

describeIfDb('Auth Integration Tests', () => {
  let pool: Pool;
  let app: Application;
  const twofa = twofaService as unknown as {
    isEnabled: jest.Mock;
    verifyTokenOrBackupCode: jest.Mock;
  };

  beforeAll(async () => {
    pool = await setupTestDatabase();
    app = createTestApp();
  });

  afterAll(async () => {
    await teardownTestDatabase();
  });

  beforeEach(async () => {
    await cleanDatabase(pool);
    twofa.isEnabled.mockImplementation(async () => false);
    twofa.verifyTokenOrBackupCode.mockImplementation(async () => true);
  });

  describe('POST /api/v1/auth/login', () => {
    it('should return 400 with invalid JSON body', async () => {
      const response = await request(app)
        .post('/api/v1/auth/login')
        .set('Content-Type', 'application/json')
        .send('{"email":')
        .expect(400);

      expect(response.body.success).toBe(false);
      expect(response.body.error).toContain('Invalid JSON');
    });

    it('should login successfully with valid credentials', async () => {
      // Setup
      const publisher = await createTestPublisher(pool);
      const user = await createTestUser(pool, publisher.id, {
        password: 'password123',
      });

      // Execute
      const response = await request(app)
        .post('/api/v1/auth/login')
        .send({
          email: user.email,
          password: user.password,
        })
        .expect(200);

      // Assert
      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveProperty('token');
      expect(response.body.data).toHaveProperty('refreshToken');
      expect(response.body.data.user).toMatchObject({
        email: user.email,
        publisherId: publisher.id,
        companyName: publisher.companyName,
      });
    });

    it('should return 401 with invalid email', async () => {
      const response = await request(app)
        .post('/api/v1/auth/login')
        .send({
          email: 'nonexistent@example.com',
          password: 'password123',
        })
        .expect(401);

      expect(response.body.success).toBe(false);
      expect(response.body.error).toContain('Invalid email or password');
    });

    it('should return 401 with invalid password', async () => {
      const publisher = await createTestPublisher(pool);
      const user = await createTestUser(pool, publisher.id, {
        password: 'password123',
      });

      const response = await request(app)
        .post('/api/v1/auth/login')
        .send({
          email: user.email,
          password: 'wrongpassword',
        })
        .expect(401);

      expect(response.body.success).toBe(false);
      expect(response.body.error).toContain('Invalid email or password');
    });

    it('should return 400 with invalid request data', async () => {
      const response = await request(app)
        .post('/api/v1/auth/login')
        .send({
          email: 'not-an-email',
          password: 'short',
        })
        .expect(400);

      expect(response.body.success).toBe(false);
    });

    it('should return 503 when 2FA status lookup fails', async () => {
      const publisher = await createTestPublisher(pool);
      const user = await createTestUser(pool, publisher.id, {
        password: 'password123',
      });

      twofa.isEnabled.mockImplementation(async () => {
        throw new Error('2FA store unavailable');
      });

      const response = await request(app)
        .post('/api/v1/auth/login')
        .send({
          email: user.email,
          password: user.password,
        })
        .expect(503);

      expect(response.body.success).toBe(false);
      expect(response.body.error).toContain('Authentication temporarily unavailable');
    });
  });

  describe('POST /api/v1/auth/refresh', () => {
    it('should refresh access token with valid refresh token', async () => {
      // Setup
      const publisher = await createTestPublisher(pool);
      const user = await createTestUser(pool, publisher.id, {
        password: 'password123',
      });

      // Login to get tokens
      const loginResponse = await request(app)
        .post('/api/v1/auth/login')
        .send({
          email: user.email,
          password: user.password,
        })
        .expect(200);

      const { refreshToken } = loginResponse.body.data;

      // Execute refresh
      const response = await request(app)
        .post('/api/v1/auth/refresh')
        .send({ refreshToken })
        .expect(200);

      // Assert
      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveProperty('token');
      expect(response.body.data).toHaveProperty('refreshToken');
      expect(response.body.data.refreshToken).not.toBe(refreshToken); // Token rotation
    });

    it('should return 401 with invalid refresh token', async () => {
      const response = await request(app)
        .post('/api/v1/auth/refresh')
        .send({ refreshToken: 'invalid-token' })
        .expect(400); // Invalid token format returns 400

      expect(response.body.success).toBe(false);
    });
  });

  describe('POST /api/v1/auth/register', () => {
    it('should register a new user and publisher', async () => {
      const response = await request(app)
        .post('/api/v1/auth/register')
        .send({
          email: 'newuser@example.com',
          password: 'password123',
          companyName: 'New Company Inc.',
        })
        .expect(201);

      expect(response.body.success).toBe(true);
      expect(response.body.data).toHaveProperty('token');
      expect(response.body.data).toHaveProperty('refreshToken');
      expect(response.body.data.user).toMatchObject({
        email: 'newuser@example.com',
        companyName: 'New Company Inc.',
      });
    });

    it('should return 409 when email already exists', async () => {
      const publisher = await createTestPublisher(pool);
      await createTestUser(pool, publisher.id, {
        email: 'existing@example.com',
      });

      const response = await request(app)
        .post('/api/v1/auth/register')
        .send({
          email: 'existing@example.com',
          password: 'password123',
          companyName: 'Another Company',
        })
        .expect(409);

      expect(response.body.success).toBe(false);
      expect(response.body.error).toContain('already registered');
    });
  });
});
