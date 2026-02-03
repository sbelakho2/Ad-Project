import { Pool } from 'pg';
import bcrypt from 'bcryptjs';
import { randomUUID } from 'crypto';

export interface TestPublisher {
  id: string;
  companyName: string;
}

export interface TestUser {
  id: string;
  email: string;
  password: string;
  publisherId: string;
}

export interface TestAdapter {
  id: string;
  name: string;
  enabled: boolean;
}

export interface ReportingSeedOptions {
  events?: number;
}

/**
 * Create a test publisher
 */
export const createTestPublisher = async (
  pool: Pool,
  data: Partial<TestPublisher> = {}
): Promise<TestPublisher> => {
  const publisher = {
    id: data.id || randomUUID(),
    companyName: data.companyName || `Test Publisher Co ${randomUUID().slice(0, 8)}`,
  };

  await pool.query(
    `INSERT INTO publishers (id, company_name) VALUES ($1, $2)`,
    [publisher.id, publisher.companyName]
  );

  return publisher;
};

/**
 * Create a test user
 */
export const createTestUser = async (
  pool: Pool,
  publisherId: string,
  data: Partial<TestUser> = {}
): Promise<TestUser> => {
  const password = data.password || 'password123';
  const passwordHash = await bcrypt.hash(password, 10);

  const user = {
    id: data.id || randomUUID(),
    email: data.email || `test-${randomUUID()}@example.com`,
    password,
    publisherId,
  };

  await pool.query(
    `INSERT INTO users (id, publisher_id, email, password_hash) VALUES ($1, $2, $3, $4)`,
    [user.id, publisherId, user.email, passwordHash]
  );

  return user;
};

/**
 * Create a test adapter
 */
export const createTestAdapter = async (
  pool: Pool,
  data: Partial<TestAdapter> = {}
): Promise<TestAdapter> => {
  const adapter = {
    id: data.id || randomUUID(),
    name: data.name || `Test Adapter ${randomUUID().slice(0, 8)}`,
    enabled: data.enabled ?? true,
  };

  await pool.query(
    `INSERT INTO adapters (id, name, enabled) VALUES ($1, $2, $3)`,
    [adapter.id, adapter.name, adapter.enabled]
  );

  return adapter;
};

/**
 * Create a test app
 */
export const createTestApp = async (
  pool: Pool,
  publisherId: string,
  appId?: string
): Promise<string> => {
  const id = appId || randomUUID();
  const bundleId = `com.test.app.${id.slice(0, 8)}`;

  await pool.query(
    `INSERT INTO apps (id, publisher_id, name, bundle_id, platform) VALUES ($1, $2, $3, $4, $5)`,
    [id, publisherId, 'Test App', bundleId, 'ios']
  );

  return id;
};

/**
 * Create a test placement
 */
export const createTestPlacement = async (
  pool: Pool,
  appId: string,
  placementId?: string
): Promise<string> => {
  const id = placementId || randomUUID();

  await pool.query(
    `INSERT INTO placements (id, app_id, name, type) VALUES ($1, $2, $3, $4)`,
    [id, appId, 'Test Placement', 'banner']
  );

  return id;
};

/**
 * Create a test adapter config
 */
export const createTestAdapterConfig = async (
  pool: Pool,
  publisherId: string,
  adapterId: string,
  config: Record<string, unknown> = {}
): Promise<string> => {
  const result = await pool.query<{ id: string }>(
    `INSERT INTO adapter_configs (publisher_id, adapter_id, config) 
     VALUES ($1, $2, $3) 
     RETURNING id`,
    [publisherId, adapterId, JSON.stringify(config)]
  );

  return result.rows[0].id;
};

/**
 * Seed analytics fact tables with deterministic test data so reporting endpoints can query them.
 */
export const seedReportingData = async (
  pool: Pool,
  publisherId: string,
  options: ReportingSeedOptions = {}
): Promise<void> => {
  const events = Math.max(1, options.events ?? 6);
  const appId = await createTestApp(pool, publisherId);
  const placementId = await createTestPlacement(pool, appId);
  const adapter = await createTestAdapter(pool, { name: 'Test Adapter Reporting' });
  const now = Date.now();

  for (let i = 0; i < events; i += 1) {
    const observedAt = new Date(now - i * 60 * 60 * 1000);
    const impressionEventId = randomUUID();
    const metadata = JSON.stringify({ seed: 'reporting-test', index: i });

    await pool.query(
      `INSERT INTO analytics_impressions (
        event_id,
        observed_at,
        publisher_id,
        app_id,
        placement_id,
        adapter_id,
        adapter_name,
        request_id,
        status,
        filled,
        latency_ms,
        revenue_usd,
        is_test_mode,
        meta
      ) VALUES (
        $1,$2,$3,$4,$5,$6,$7,$8,$9,true,$10,$11,false,$12
      )`,
      [
        impressionEventId,
        observedAt,
        publisherId,
        appId,
        placementId,
        adapter.id,
        adapter.name,
        randomUUID(),
        'success',
        40 + i * 5,
        (i + 1) * 0.01,
        metadata,
      ]
    );

    await pool.query(
      `INSERT INTO analytics_revenue_events (
        observed_at,
        publisher_id,
        app_id,
        placement_id,
        adapter_id,
        adapter_name,
        impression_id,
        revenue_type,
        revenue_usd,
        revenue_currency,
        revenue_original,
        exchange_rate,
        ecpm_usd,
        country_code,
        ad_format,
        os,
        is_test_mode,
        metadata
      ) VALUES (
        $1,$2,$3,$4,$5,$6,$7,$8,$9,'USD',$10,$11,$12,$13,$14,$15,false,$16
      )`,
      [
        observedAt,
        publisherId,
        appId,
        placementId,
        adapter.id,
        adapter.name,
        impressionEventId,
        i % 3 === 0 ? 'click' : 'impression',
        (i + 1) * 5,
        (i + 1) * 5,
        1,
        (i + 1) * 5,
        i % 2 === 0 ? 'US' : 'CA',
        'banner',
        i % 2 === 0 ? 'ios' : 'android',
        metadata,
      ]
    );
  }

  await pool.query(
    `INSERT INTO analytics_sdk_telemetry (
      publisher_id,
      adapter_id,
      event_type,
      message,
      metadata
    ) VALUES (
      $1,$2,'info',$3,$4
    )`,
    [publisherId, adapter.id, 'Seeded reporting data', JSON.stringify({ seed: 'reporting-test' })]
  );
};
