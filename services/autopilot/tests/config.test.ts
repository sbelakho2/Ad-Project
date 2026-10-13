describe('autopilot config', () => {
  const ORIGINAL_ENV = process.env;

  beforeEach(() => {
    jest.resetModules();
    process.env = { ...ORIGINAL_ENV };
  });

  afterAll(() => {
    process.env = ORIGINAL_ENV;
  });

  it('falls back to documented defaults', () => {
    delete process.env.API_PORT;
    delete process.env.ALLOWED_IPS;
    delete process.env.DATABASE_POOL_SIZE;
    const { config } = require('../src/config');
    expect(config.api.port).toBe(8090);
    expect(config.api.host).toBe('0.0.0.0');
    expect(config.api.allowedIps).toEqual(['127.0.0.1']);
    expect(config.database.poolSize).toBe(5);
    expect(config.scheduler.retentionCleanupHour).toBe(3);
    expect(config.alerts.webhookUrl).toBeNull();
    expect(config.logging.format).toBe('json');
  });

  it('honors env overrides, including comma-separated IP trimming', () => {
    process.env.API_PORT = '9001';
    process.env.API_HOST = '127.0.0.1';
    process.env.ALLOWED_IPS = '10.0.0.1, 10.0.0.2 ,10.0.0.3';
    process.env.DATABASE_URL = 'postgresql://u:p@db:5432/x';
    const { config } = require('../src/config');
    expect(config.api.port).toBe(9001);
    expect(config.api.host).toBe('127.0.0.1');
    expect(config.api.allowedIps).toEqual(['10.0.0.1', '10.0.0.2', '10.0.0.3']);
    expect(config.database.url).toBe('postgresql://u:p@db:5432/x');
  });

  it('parses numeric scheduler knobs and treats non-numeric as NaN (documented raw-parse behavior)', () => {
    process.env.HEALTH_CHECK_INTERVAL_SECONDS = '15';
    process.env.RETENTION_CLEANUP_HOUR = 'not-a-number';
    const { config } = require('../src/config');
    expect(config.scheduler.healthCheckIntervalSeconds).toBe(15);
    expect(Number.isNaN(config.scheduler.retentionCleanupHour)).toBe(true);
  });
});
