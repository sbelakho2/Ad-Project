/**
 * Platform client factory — the single entry point the audit engine, autopilot,
 * and console use to reach a publisher's mediation platform.
 */
import { AdmobClient, type AdmobCredentials } from './admobClient';
import { LevelPlayClient, type LevelPlayCredentials } from './levelPlayClient';
import { MaxClient, type MaxCredentials } from './maxClient';
import { PlatformApiError, type PlatformId } from './types';

export type PlatformCredentials =
  | { platform: 'applovin_max'; credentials: MaxCredentials }
  | { platform: 'levelplay'; credentials: LevelPlayCredentials }
  | { platform: 'admob'; credentials: AdmobCredentials };

export type PlatformClient = MaxClient | LevelPlayClient | AdmobClient;

export const SUPPORTED_PLATFORMS: readonly PlatformId[] = ['applovin_max', 'levelplay', 'admob'] as const;

export function getPlatformClient(input: PlatformCredentials): PlatformClient {
  switch (input.platform) {
    case 'applovin_max':
      return new MaxClient(input.credentials);
    case 'levelplay':
      return new LevelPlayClient(input.credentials);
    case 'admob':
      return new AdmobClient(input.credentials);
    default: {
      const exhaustive: never = input;
      throw new PlatformApiError(
        `unsupported platform: ${JSON.stringify(exhaustive)}`,
        (input as { platform: PlatformId }).platform,
        'getPlatformClient',
        { code: 'UNSUPPORTED_PLATFORM', retryable: false },
      );
    }
  }
}

export { AdmobClient, LevelPlayClient, MaxClient, PlatformApiError };
export type { AdmobCredentials, LevelPlayCredentials, MaxCredentials };
