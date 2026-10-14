/**
 * Tier-1 tests for the React Native SDK JS layer.
 * The native module is mocked at the module boundary (jest.mock('react-native')) so the
 * JS-side singleton, event fan-out, and AdSize helpers run deterministically on Node.
 */
import type { InitializationStatus } from '..';

const nativeModule = {
  initialize: jest.fn(async (_appId: string, _testMode: boolean, _verbose: boolean): Promise<InitializationStatus> => ({ isInitialized: true, adapters: [] })),
  getInitializationStatus: jest.fn(async (): Promise<InitializationStatus> => ({ isInitialized: false, adapters: [] })),
  loadBannerAd: jest.fn(async (adUnitId: string, _w: number, _h: number, _adaptive: boolean) => ({ success: true, adUnitId })),
  showBannerAd: jest.fn(async () => undefined),
  hideBannerAd: jest.fn(async () => undefined),
  destroyBannerAd: jest.fn(async () => undefined),
  loadInterstitialAd: jest.fn(async (adUnitId: string) => ({ success: true, adUnitId })),
  showInterstitialAd: jest.fn(async () => undefined),
  isInterstitialAdReady: jest.fn(async () => false),
  collectSignals: jest.fn(async () => []),
  collectSignal: jest.fn(async () => null),
  setExtraData: jest.fn(async () => undefined),
  setUserId: jest.fn(async () => undefined),
  getReadyAdapters: jest.fn(async () => []),
};

type Listener = (data: unknown) => void;
const emitterListeners = new Map<string, Listener>();

jest.mock('react-native', () => ({
  NativeModules: { AdProjectSdk: nativeModule },
  Platform: { select: (opts: Record<string, unknown>) => opts.default },
  NativeEventEmitter: class {
    addListener(event: string, cb: Listener) {
      emitterListeners.set(event, cb);
      return { remove: () => emitterListeners.delete(event) };
    }
  },
}));

import AdProjectDefault, { AdProject, AdSize } from '..';

describe('React Native SDK JS layer', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('exports a singleton under both default and named bindings', () => {
    expect(AdProjectDefault).toBe(AdProject);
    expect(AdProject.isInitialized()).toBe(false);
  });

  it('initialize() forwards to native once and flips isInitialized', async () => {
    const status = await AdProject.initialize('app-1', { testMode: true, verbose: false });
    expect(nativeModule.initialize).toHaveBeenCalledWith('app-1', true, false);
    expect(status.isInitialized).toBe(true);
    expect(AdProject.isInitialized()).toBe(true);

    // Second initialize() must short-circuit to the native status query.
    await AdProject.initialize('app-1');
    expect(nativeModule.getInitializationStatus).toHaveBeenCalledTimes(1);
  });

  it('loadBannerAd() applies default AdSize.BANNER dimensions', async () => {
    await AdProject.loadBannerAd('unit-a');
    expect(nativeModule.loadBannerAd).toHaveBeenCalledWith('unit-a', 320, 50, false);
  });

  it('AdSize.adaptive and AdSize.custom produce the documented shapes', () => {
    expect(AdSize.adaptive(375)).toEqual({ width: 375, height: 0, isAdaptive: true });
    expect(AdSize.custom(300, 250)).toEqual({ width: 300, height: 250, isAdaptive: false });
  });

  it('native events fan out to per-unit and global listeners; unsubscribe stops delivery', async () => {
    const perUnit: unknown[] = [];
    const global: unknown[] = [];
    const offUnit = AdProject.addEventListener('unit-x', 'onAdLoaded', (d) => perUnit.push(d));
    const offGlobal = AdProject.addGlobalEventListener('onAdLoaded', (d) => global.push(d));

    // Simulate a native event through the emitter registered in the constructor.
    const deliver = emitterListeners.get('onAdLoaded');
    expect(deliver).toBeDefined();
    deliver!({ adUnitId: 'unit-x', revenue: 0 });

    expect(perUnit).toHaveLength(1);
    expect(global).toHaveLength(1);

    offUnit();
    offGlobal();
    deliver!({ adUnitId: 'unit-x' });
    expect(perUnit).toHaveLength(1);
    expect(global).toHaveLength(1);
  });

  it('destroy() clears listeners and initialization state, and native subscriptions are removed', async () => {
    await AdProject.initialize('app-1');
    expect(AdProject.isInitialized()).toBe(true);
    AdProject.destroy();
    expect(AdProject.isInitialized()).toBe(false);
    expect(emitterListeners.size).toBe(0);
  });
});
