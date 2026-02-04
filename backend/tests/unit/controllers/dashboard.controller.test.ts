import { describe, it, expect, beforeAll, beforeEach } from '@jest/globals';
import request from 'supertest';
import { Application } from 'express';
import { createTestApp } from '../../helpers/testApp';

jest.mock('../../../src/utils/postgres', () => {
  const query = jest.fn();
  return {
    __esModule: true,
    default: { query },
    query,
    getClient: jest.fn(),
  };
});

const postgresMock = jest.requireMock('../../../src/utils/postgres');
const mockPoolQuery = postgresMock.default.query as jest.Mock;

describe('Dashboard Controller', () => {
  let app: Application;

  beforeAll(() => {
    process.env.NODE_ENV = 'test';
    app = createTestApp();
  });

  beforeEach(() => {
    mockPoolQuery.mockReset();
    mockPoolQuery.mockResolvedValue({
      rows: [
        {
          total_revenue: '10',
          total_impressions: '20',
          total_clicks: '5',
        },
      ],
    });
  });

  it('returns dashboard metrics with computed fields', async () => {
    const res = await request(app)
      .get('/api/v1/dashboard/overview?page=1&pageSize=20')
      .expect(200);

    expect(res.body?.success).toBe(true);
    expect(res.body?.data?.tiles).toMatchObject({
      revenue: 10,
      impressions: 20,
      clicks: 5,
    });
    expect(res.body?.data?.tiles?.ctr).toBeCloseTo(0.25);
    expect(res.body?.data?.tiles?.rpm).toBeCloseTo(500);
  });
});
