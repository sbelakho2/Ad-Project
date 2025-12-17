type MockPoolOptions = {
  replicaRows?: Array<{ application_name: string; replay_lag_ms?: number; write_lag_ms?: number }>;
  stagingRows?: Array<{ table_name: string; row_estimate: number }>;
  cacheHits?: number;
  cacheReads?: number;
  failReplica?: boolean;
  failSelect1?: boolean;
  failSloQuery?: boolean;
};

const mockRedis = { isReady: jest.fn(() => true) };

const mockQueueManager = { isReady: jest.fn(() => true) };

jest.mock('../../utils/redis', () => ({
  redis: mockRedis,
}));

jest.mock('../../queues/queueManager', () => ({
  queueManager: mockQueueManager,
}));

function makeMockPool(options: MockPoolOptions = {}) {
  return {
    query: jest.fn(async (sql: string) => {
      if (/SELECT 1/.test(sql)) {
        if (options.failSelect1) {
          throw new Error('db down');
        }
        return { rows: [{ '?column?': 1 }] };
      }
      if (sql.includes('WITH impression_metrics')) {
        if (options.failSloQuery) {
          throw new Error('slo query failed');
        }
        return {
          rows: [
            {
              total_requests: '1000',
              successful_requests: '999',
              error_requests: '1',
              latency_p95_ms: '87',
              sessions: '500',
              anrs: '0',
              deadline_samples: '100',
              deadline_hit_rate: '0.99',
              validator_samples: '10000',
              validator_invalid: '0',
              parse_errors: '0',
              tracking_total_events: '1000',
              tracking_unique_events: '1000',
            },
          ],
        };
      }
      if (sql.includes('pg_stat_replication')) {
        if (options.failReplica) {
          throw new Error('replica failure');
        }
        return { rows: options.replicaRows ?? [] };
      }
      if (sql.includes('pg_stat_database')) {
        return {
          rows: [
            {
              hits: options.cacheHits ?? 1000,
              reads: options.cacheReads ?? 0,
            },
          ],
        };
      }
      if (sql.includes('pg_stat_user_tables')) {
        return {
          rows: options.stagingRows ?? [{ table_name: 'analytics_impressions_stage', row_estimate: 100 }],
        };
      }
      return { rows: [{ '?column?': 1 }] };
    }),
  } as any;
}

function createMockRes() {
  const res: any = {};
  res.statusCode = 200;
  res.body = null as any;
  res.headers = {} as Record<string, string>;
  res.status = (code: number) => {
    res.statusCode = code;
    return res;
  };
  res.set = (key: string, value: string) => {
    res.headers[key.toLowerCase()] = value;
    return res;
  };
  res.json = (payload: any) => {
    res.body = payload;
    return res;
  };
  return res;
}

const baseConfig = {
  replicaRequired: false,
  readinessThresholds: {
    dbSlowMs: 1000,
    replicaLagWarnMs: 5000,
    replicaLagCriticalMs: 15000,
    stagingWarnRows: 250000,
    cacheHitWarnRatio: 0.9,
  },
};

describe('HealthCheckController.readiness', () => {
  afterEach(() => {
    jest.resetModules();
    jest.clearAllMocks();
  });

  it('returns 503 when replica health cannot be established while required', async () => {
    jest.doMock('../../config/index', () => ({
      __esModule: true,
      default: { ...baseConfig, replicaRequired: true },
    }));

    const { HealthCheckController: Controller } = await import('../../controllers/HealthCheckController');
    const controller = new Controller(makeMockPool({ replicaRows: [] }));
    const res = createMockRes();
    await controller.readiness({} as any, res as any);

    expect(res.statusCode).toBe(503);
    expect(res.body?.message).toMatch(/Read replica/);
  });

  it('returns 503 when staging backlog exceeds guardrail', async () => {
    jest.doMock('../../config/index', () => ({
      __esModule: true,
      default: { ...baseConfig, readinessThresholds: { ...baseConfig.readinessThresholds, stagingWarnRows: 10 } },
    }));

    const { HealthCheckController: Controller } = await import('../../controllers/HealthCheckController');
    const controller = new Controller(
      makeMockPool({
        replicaRows: [{ application_name: 'replica-1', replay_lag_ms: 100, write_lag_ms: 50 }],
        stagingRows: [{ table_name: 'analytics_impressions_stage', row_estimate: 500 }],
      })
    );

    const res = createMockRes();
    await controller.readiness({} as any, res as any);

    expect(res.statusCode).toBe(503);
    expect(res.body?.message).toMatch(/staging backlog/i);
  });

  it('returns 200 when all readiness signals stay within guardrails', async () => {
    jest.doMock('../../config/index', () => ({ __esModule: true, default: baseConfig }));

    const { HealthCheckController: Controller } = await import('../../controllers/HealthCheckController');
    const controller = new Controller(
      makeMockPool({
        replicaRows: [{ application_name: 'replica-1', replay_lag_ms: 10, write_lag_ms: 5 }],
        stagingRows: [{ table_name: 'analytics_impressions_stage', row_estimate: 5 }],
        cacheHits: 900,
        cacheReads: 100,
      })
    );

    const res = createMockRes();
    await controller.readiness({} as any, res as any);

    expect(res.statusCode).toBe(200);
    expect(res.body?.replica?.hasReplicas).toBe(true);
    expect(res.body?.staging?.totalRows).toBe(5);
    expect(res.body?.cache?.hit_ratio).toBeGreaterThan(0.8);
  });
});

describe('HealthCheckController.publicStatus', () => {
  afterEach(() => {
    jest.resetModules();
    jest.clearAllMocks();
  });

  it('always returns 200 and reports down when DB is unavailable', async () => {
    jest.doMock('../../config/index', () => ({ __esModule: true, default: baseConfig }));

    const { HealthCheckController: Controller } = await import('../../controllers/HealthCheckController');
    const controller = new Controller(makeMockPool({ failSelect1: true }));
    const res = createMockRes();

    await controller.publicStatus({} as any, res as any);

    expect(res.statusCode).toBe(200);
    expect(res.body?.status).toBe('down');
    expect(res.body?.checks?.postgres?.connected).toBe(false);
    expect(res.headers['cache-control']).toMatch(/public/i);
  });

  it('reports operational when core checks are healthy', async () => {
    jest.doMock('../../config/index', () => ({ __esModule: true, default: baseConfig }));

    const { HealthCheckController: Controller } = await import('../../controllers/HealthCheckController');
    const controller = new Controller(makeMockPool());
    const res = createMockRes();

    await controller.publicStatus({} as any, res as any);

    expect(res.statusCode).toBe(200);
    expect(res.body?.status).toBe('operational');
    expect(res.body?.checks?.postgres?.connected).toBe(true);
  });
});

describe('HealthCheckController.publicSLOs', () => {
  afterEach(() => {
    jest.resetModules();
    jest.clearAllMocks();
  });

  it('returns 200 with ok payload when query succeeds', async () => {
    jest.doMock('../../config/index', () => ({ __esModule: true, default: baseConfig }));

    const { HealthCheckController: Controller } = await import('../../controllers/HealthCheckController');
    const controller = new Controller(makeMockPool());
    const res = createMockRes();

    await controller.publicSLOs({ query: { days: '30' } } as any, res as any);

    expect(res.statusCode).toBe(200);
    expect(res.body?.status).toBe('ok');
    expect(res.body?.window?.days).toBe(30);
    expect(res.body?.slis?.availability?.availabilityPct).toBeGreaterThan(99);
    expect(res.body?.slis?.latency?.p95Ms).toBe(87);
    expect(res.body?.slis?.deadlineHitRate?.deadlineHitRatePct).toBeGreaterThan(98);
    expect(res.body?.slis?.invalidManifestRate?.invalidManifestRatePct).toBeLessThan(0.01);
    expect(res.body?.slis?.trackingIntegrity?.integrityPct).toBe(100);
    expect(res.headers['cache-control']).toMatch(/public/i);
  });

  it('always returns 200 even if DB query fails', async () => {
    jest.doMock('../../config/index', () => ({ __esModule: true, default: baseConfig }));

    const { HealthCheckController: Controller } = await import('../../controllers/HealthCheckController');
    const controller = new Controller(makeMockPool({ failSloQuery: true }));
    const res = createMockRes();

    await controller.publicSLOs({ query: { days: '30' } } as any, res as any);

    expect(res.statusCode).toBe(200);
    expect(res.body?.status).toMatch(/unknown|ok/);
    expect(res.body?.window?.days).toBe(30);
  });
});
