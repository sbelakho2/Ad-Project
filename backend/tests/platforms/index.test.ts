/**
 * Factory + cross-platform tests: the console/autopilot always goes through
 * getPlatformClient, so exhaustiveness and credential validation are contract-level.
 */
import { getPlatformClient, SUPPORTED_PLATFORMS, AdmobClient, LevelPlayClient, MaxClient } from '../../src/services/platforms';
import { PlatformApiError } from '../../src/services/platforms/types';

describe('getPlatformClient', () => {
  it('builds the right client per platform', () => {
    expect(getPlatformClient({ platform: 'applovin_max', credentials: { apiKey: 'k', accountId: 'a' } })).toBeInstanceOf(MaxClient);
    expect(getPlatformClient({ platform: 'levelplay', credentials: { apiKey: 'k', accountId: 'a' } })).toBeInstanceOf(LevelPlayClient);
    expect(getPlatformClient({ platform: 'admob', credentials: { accessToken: 't', accountId: 'pub-1234567890123456' } })).toBeInstanceOf(AdmobClient);
  });

  it('declares exactly the three supported platforms', () => {
    expect([...SUPPORTED_PLATFORMS].sort()).toEqual(['admob', 'applovin_max', 'levelplay']);
  });

  it('fails closed for an unsupported platform value', () => {
    const bad = { platform: 'chartboost' as never, credentials: { apiKey: 'k', accountId: 'a' } as never };
    expect(() => getPlatformClient(bad as never)).toThrow(PlatformApiError);
    expect(() => getPlatformClient(bad as never)).toThrow(/unsupported platform/);
  });

  it('credential validation happens at construction (console onboarding cannot store blanks)', () => {
    expect(() => getPlatformClient({ platform: 'applovin_max', credentials: { apiKey: '   ', accountId: 'a' } })).toThrow(/apiKey and accountId/);
    expect(() => getPlatformClient({ platform: 'levelplay', credentials: { apiKey: 'k', accountId: '' } })).toThrow(/apiKey and accountId/);
    expect(() => getPlatformClient({ platform: 'admob', credentials: { accessToken: '', accountId: 'pub-1234567890123456' } })).toThrow(/accessToken and accountId/);
  });
});
