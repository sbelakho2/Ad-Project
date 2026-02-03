import request from 'supertest';
import { createTestApp } from '../helpers/testApp';
import { twofaEventsTotal } from '../../src/utils/prometheus';

jest.mock('../../src/services/twofa.service', () => ({
  __esModule: true,
  default: {
    isEnabled: jest.fn().mockResolvedValue(false),
    getBackupCodesCount: jest.fn().mockResolvedValue(0),
    enroll: jest.fn().mockResolvedValue({ secret: 'secret', otpauthUrl: 'otpauth://test', qrCode: 'data' }),
    verifyAndEnable: jest.fn().mockResolvedValue({ enabled: true }),
    regenerateBackupCodes: jest.fn().mockResolvedValue({ codes: ['code-1'] }),
    verifyTokenOrBackupCode: jest.fn().mockResolvedValue(true),
    disable: jest.fn().mockResolvedValue({ disabled: true }),
  },
}));

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

  it('increments twofa_events_total on enroll/verify', async () => {
    await request(app)
      .post('/api/v1/auth/2fa/enroll')
      .send({})
      .expect(200);

    await request(app)
      .post('/api/v1/auth/2fa/verify')
      .send({ token: '123456' })
      .expect(200);

    const metric = await twofaEventsTotal.get();
    const values = metric?.values ?? [];
    const enrollSample = values.find((entry) =>
      entry.labels?.event === 'enroll' && entry.labels?.outcome === 'success'
    );
    const verifySample = values.find((entry) =>
      entry.labels?.event === 'verify' && entry.labels?.outcome === 'success'
    );

    expect(enrollSample?.value).toBeGreaterThanOrEqual(1);
    expect(verifySample?.value).toBeGreaterThanOrEqual(1);
  });
});
