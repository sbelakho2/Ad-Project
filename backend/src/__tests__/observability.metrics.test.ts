import request from 'supertest';
import { createTestApp } from '../__tests__/helpers/testApp';

describe('Observability metrics', () => {
  const app = createTestApp();

  it('exposes auth_attempts_total in /metrics and increments on failed login', async () => {
    // Trigger a failed login to increment auth_attempts_total{outcome="failure"}
    await request(app)
      .post('/api/v1/auth/login')
      .send({ email: 'nobody@example.com', password: 'wrongpass' })
      .expect(401);

    const res = await request(app).get('/metrics').expect(200);
    const text = res.text;
    expect(text).toMatch(/auth_attempts_total/);
    // Check labeled samples exist
    expect(text).toMatch(/auth_attempts_total\{outcome="failure"}\s+\d+/);
  });

  // NOTE: 2FA metrics tests require a database-backed test environment.
  // These are covered in integration tests.
  it.todo('increments twofa_events_total on enroll/verify (requires db)');
});
