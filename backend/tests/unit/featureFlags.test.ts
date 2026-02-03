import request from 'supertest';
import { createTestApp } from '../helpers/testApp';

describe('Feature flags and kill switch', () => {
  const app = createTestApp();

  it('returns current flags and allows updating them', async () => {
    const res = await request(app).get('/api/v1/flags').expect(200);
    expect(res.body?.data).toBeDefined();
    expect(typeof res.body.data.killSwitch).toBe('boolean');
    expect(typeof res.body.data.enforce2fa).toBe('boolean');

    // Turn on enforce2fa and verify echo
    const upd = await request(app)
      .post('/api/v1/flags')
      .send({ enforce2fa: true })
      .expect(200);
    expect(upd.body?.data?.enforce2fa).toBe(true);
  });

  it('killSwitch returns 503 for typical API routes when enabled, and recovers when disabled', async () => {
    // Ensure it works when off
    await request(app).post('/api/v1/flags').send({ killSwitch: false }).expect(200);
    const ok = await request(app).get('/api/v1/auth/csrf').expect(200);
    expect(ok.body).toBeTruthy();

    // Enable kill switch
    await request(app).post('/api/v1/flags').send({ killSwitch: true }).expect(200);

    // A non-allowlisted route should now return 503
    await request(app).get('/api/v1/auth/csrf').expect(503);

    // Allowlisted routes still work
    await request(app).get('/health').expect(200);
    await request(app).get('/api/v1/flags').expect(200);

    // Disable kill switch to recover
    await request(app).post('/api/v1/flags').send({ killSwitch: false }).expect(200);
    await request(app).get('/api/v1/auth/csrf').expect(200);
  });
});
