import { jest } from '@jest/globals';
import request from 'supertest';
import { createTestApp } from '../../__tests__/helpers/testApp';

jest.mock('../../repositories/userRepository', () => ({
  __esModule: true,
  findUserByEmail: jest.fn(async () => null),
  createUserWithPublisher: jest.fn(),
}));

jest.mock('bcryptjs', () => {
  const api = { compare: jest.fn(async () => false) };
  return {
    __esModule: true,
    default: api,
    ...api,
  };
});

jest.mock('../../services/twofa.service', () => ({
  __esModule: true,
  default: {
    isEnabled: jest.fn(async () => false),
    verifyTokenOrBackupCode: jest.fn(async () => true),
  },
}));

// Imports must occur after jest.mock calls.
import { findUserByEmail } from '../../repositories/userRepository';
import { compare as bcryptCompare } from 'bcryptjs';
import twofaService from '../../services/twofa.service';

type FindUserByEmailFn = (email: string) => Promise<unknown | null>;
type BcryptCompareFn = (plain: string, hash: string) => Promise<boolean>;

const findUserByEmailMock = findUserByEmail as unknown as jest.MockedFunction<FindUserByEmailFn>;
const bcryptCompareMock = bcryptCompare as unknown as jest.MockedFunction<BcryptCompareFn>;
const twofaIsEnabledMock = (twofaService as unknown as { isEnabled: jest.Mock }).isEnabled;

describe('Auth login hardening', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    findUserByEmailMock.mockImplementation(async () => null);
    twofaIsEnabledMock.mockImplementation(async () => false);
    bcryptCompareMock.mockImplementation(async () => false);
  });

  it('returns 400 for invalid JSON on /api/v1/auth/login', async () => {
    const app = createTestApp();

    const response = await request(app)
      .post('/api/v1/auth/login')
      .set('Content-Type', 'application/json')
      .send('{"email":')
      .expect(400);

    expect(response.body.success).toBe(false);
    expect(response.body.error).toContain('Invalid JSON');
  });

  it('returns 503 when 2FA status lookup throws during login', async () => {
    const app = createTestApp();

    findUserByEmailMock.mockResolvedValue({
      id: 'user-1',
      email: 'user@example.com',
      publisher_id: 'pub-1',
      company_name: 'Test Co',
      role: 'publisher',
      password_hash: '$2a$10$7b0p8Y1CwV2uX1b8Q1vM2eJdJQzRzq4q2m9P5QyqvQd7sQZ7cH0dW',
    });

    bcryptCompareMock.mockResolvedValue(true);

    twofaIsEnabledMock.mockImplementation(async () => {
      throw new Error('2FA store unavailable');
    });

    const response = await request(app)
      .post('/api/v1/auth/login')
      .send({
        email: 'user@example.com',
        password: 'password123',
      })
      .expect(503);

    expect(response.body.success).toBe(false);
    expect(response.body.error).toContain('Authentication temporarily unavailable');
  });
});
