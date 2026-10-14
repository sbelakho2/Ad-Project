import { NativeModules, Platform, NativeEventEmitter } from 'react-native';

const LINKING_ERROR =
  `The package '@adproject/react-native-sdk' doesn't seem to be linked. Make sure: \n\n` +
  Platform.select({ ios: "- You have run 'pod install'\n", default: '' }) +
  '- You rebuilt the app after installing the package\n' +
  '- You are not using Expo Go\n';

// Native module with fallback for development
const AdProjectModule = NativeModules.AdProjectSdk
  ? NativeModules.AdProjectSdk
  : new Proxy(
      {},
      {
        get() {
          throw new Error(LINKING_ERROR);
        },
      }
    );

const eventEmitter = new NativeEventEmitter(AdProjectModule);

// MARK: - Types

export type AdType = 'banner' | 'interstitial' | 'rewarded' | 'rewardedInterstitial' | 'native' | 'appOpen';

export interface AdSize {
  width: number;
  height: number;
  isAdaptive?: boolean;
}

export interface AdLoadResult {
  success: boolean;
  adUnitId: string;
  networkName?: string;
  error?: AdError;
  loadTimeMs?: number;
}

export interface AdError {
  code: string;
  message: string;
  networkName?: string;
}

export interface Reward {
  type: string;
  amount: number;
}

export interface ImpressionData {
  adUnitId: string;
  networkName: string;
  revenue: number;
  revenuePrecision: string;
  adType: AdType;
  placement?: string;
}

export interface InitializationStatus {
  isInitialized: boolean;
  adapters: AdapterStatus[];
}

export interface AdapterStatus {
  networkName: string;
  status: 'NOT_INITIALIZED' | 'INITIALIZING' | 'INITIALIZED_SUCCESS' | 'INITIALIZED_FAILURE';
  sdkVersion?: string;
  adapterVersion?: string;
  errorMessage?: string;
}

export interface ConsentStatus {
  hasConsent: boolean;
  consentString?: string;
  vendorConsents?: Record<string, boolean>;
}

export interface BidSignal {
  networkName: string;
  token: string;
  timestamp: number;
  expiresAt: number;
}

// MARK: - Event Types

export type AdEventType =
  | 'onAdLoaded'
  | 'onAdFailedToLoad'
  | 'onAdOpened'
  | 'onAdClosed'
  | 'onAdClicked'
  | 'onAdImpression'
  | 'onRewardEarned'
  | 'onAdRevenuePaid';

export interface AdEventListener {
  type: AdEventType;
  adUnitId: string;
  callback: (data: any) => void;
}

// MARK: - Main SDK Class

class AdProjectSDK {
  private static instance: AdProjectSDK;
  private initialized = false;
  private eventListeners: Map<string, AdEventListener[]> = new Map();
  private subscriptions: any[] = [];

  private constructor() {
    this.setupEventListeners();
  }

  public static getInstance(): AdProjectSDK {
    if (!AdProjectSDK.instance) {
      AdProjectSDK.instance = new AdProjectSDK();
    }
    return AdProjectSDK.instance;
  }

  private setupEventListeners(): void {
    const events: AdEventType[] = [
      'onAdLoaded',
      'onAdFailedToLoad',
      'onAdOpened',
      'onAdClosed',
      'onAdClicked',
      'onAdImpression',
      'onRewardEarned',
      'onAdRevenuePaid',
    ];

    events.forEach((event) => {
      const sub = eventEmitter.addListener(event, (data: any) => {
        this.emitEvent(event, data);
      });
      this.subscriptions.push(sub);
    });
  }

  private emitEvent(type: AdEventType, data: any): void {
    const key = `${type}:${data.adUnitId}`;
    const listeners = this.eventListeners.get(key) || [];
    listeners.forEach((listener) => listener.callback(data));

    // Also emit to global listeners
    const globalListeners = this.eventListeners.get(type) || [];
    globalListeners.forEach((listener) => listener.callback(data));
  }

  // MARK: - Initialization

  async initialize(appId: string, options?: { testMode?: boolean; verbose?: boolean }): Promise<InitializationStatus> {
    if (this.initialized) {
      return AdProjectModule.getInitializationStatus();
    }

    const result = await AdProjectModule.initialize(appId, options?.testMode ?? false, options?.verbose ?? false);
    this.initialized = result.isInitialized;
    return result;
  }

  isInitialized(): boolean {
    return this.initialized;
  }

  async getInitializationStatus(): Promise<InitializationStatus> {
    return AdProjectModule.getInitializationStatus();
  }

  // MARK: - Banner Ads

  async loadBannerAd(adUnitId: string, size?: AdSize): Promise<AdLoadResult> {
    const adSize = size || AdSize.BANNER;
    return AdProjectModule.loadBannerAd(adUnitId, adSize.width, adSize.height, adSize.isAdaptive ?? false);
  }

  async showBannerAd(adUnitId: string): Promise<void> {
    return AdProjectModule.showBannerAd(adUnitId);
  }

  async hideBannerAd(adUnitId: string): Promise<void> {
    return AdProjectModule.hideBannerAd(adUnitId);
  }

  async destroyBannerAd(adUnitId: string): Promise<void> {
    return AdProjectModule.destroyBannerAd(adUnitId);
  }

  // MARK: - Interstitial Ads

  async loadInterstitialAd(adUnitId: string): Promise<AdLoadResult> {
    return AdProjectModule.loadInterstitialAd(adUnitId);
  }

  async showInterstitialAd(adUnitId: string): Promise<void> {
    return AdProjectModule.showInterstitialAd(adUnitId);
  }

  isInterstitialAdReady(adUnitId: string): Promise<boolean> {
    return AdProjectModule.isInterstitialAdReady(adUnitId);
  }

  // MARK: - Rewarded Ads

  async loadRewardedAd(adUnitId: string): Promise<AdLoadResult> {
    return AdProjectModule.loadRewardedAd(adUnitId);
  }

  async showRewardedAd(adUnitId: string): Promise<Reward | null> {
    return AdProjectModule.showRewardedAd(adUnitId);
  }

  isRewardedAdReady(adUnitId: string): Promise<boolean> {
    return AdProjectModule.isRewardedAdReady(adUnitId);
  }

  // MARK: - Rewarded Interstitial Ads

  async loadRewardedInterstitialAd(adUnitId: string): Promise<AdLoadResult> {
    return AdProjectModule.loadRewardedInterstitialAd(adUnitId);
  }

  async showRewardedInterstitialAd(adUnitId: string): Promise<Reward | null> {
    return AdProjectModule.showRewardedInterstitialAd(adUnitId);
  }

  isRewardedInterstitialAdReady(adUnitId: string): Promise<boolean> {
    return AdProjectModule.isRewardedInterstitialAdReady(adUnitId);
  }

  // MARK: - App Open Ads

  async loadAppOpenAd(adUnitId: string): Promise<AdLoadResult> {
    return AdProjectModule.loadAppOpenAd(adUnitId);
  }

  async showAppOpenAd(adUnitId: string): Promise<void> {
    return AdProjectModule.showAppOpenAd(adUnitId);
  }

  isAppOpenAdReady(adUnitId: string): Promise<boolean> {
    return AdProjectModule.isAppOpenAdReady(adUnitId);
  }

  // MARK: - Signal Collection (S2S Bidding)

  async collectSignals(networkNames?: string[]): Promise<BidSignal[]> {
    return AdProjectModule.collectSignals(networkNames ?? []);
  }

  async collectSignal(networkName: string): Promise<BidSignal | null> {
    return AdProjectModule.collectSignal(networkName);
  }

  // MARK: - Consent Management

  async setConsentStatus(status: ConsentStatus): Promise<void> {
    return AdProjectModule.setConsentStatus(status);
  }

  async getConsentStatus(): Promise<ConsentStatus> {
    return AdProjectModule.getConsentStatus();
  }

  async setHasUserConsent(hasConsent: boolean): Promise<void> {
    return AdProjectModule.setHasUserConsent(hasConsent);
  }

  async setIsAgeRestrictedUser(isRestricted: boolean): Promise<void> {
    return AdProjectModule.setIsAgeRestrictedUser(isRestricted);
  }

  async setDoNotSell(doNotSell: boolean): Promise<void> {
    return AdProjectModule.setDoNotSell(doNotSell);
  }

  // MARK: - User Properties

  async setUserId(userId: string): Promise<void> {
    return AdProjectModule.setUserId(userId);
  }

  async setUserSegments(segments: string[]): Promise<void> {
    return AdProjectModule.setUserSegments(segments);
  }

  async setExtraData(key: string, value: string): Promise<void> {
    return AdProjectModule.setExtraData(key, value);
  }

  // MARK: - Adapter Health

  async getAdapterHealth(networkName: string): Promise<AdapterStatus | null> {
    return AdProjectModule.getAdapterHealth(networkName);
  }

  async getAllAdapterHealth(): Promise<Record<string, AdapterStatus>> {
    return AdProjectModule.getAllAdapterHealth();
  }

  async getReadyAdapters(): Promise<string[]> {
    return AdProjectModule.getReadyAdapters();
  }

  // MARK: - Event Listeners

  addEventListener(
    adUnitId: string,
    type: AdEventType,
    callback: (data: any) => void
  ): () => void {
    const key = `${type}:${adUnitId}`;
    const listener: AdEventListener = { type, adUnitId, callback };

    const listeners = this.eventListeners.get(key) || [];
    listeners.push(listener);
    this.eventListeners.set(key, listeners);

    // Return unsubscribe function
    return () => {
      const current = this.eventListeners.get(key) || [];
      const index = current.indexOf(listener);
      if (index > -1) {
        current.splice(index, 1);
        this.eventListeners.set(key, current);
      }
    };
  }

  addGlobalEventListener(type: AdEventType, callback: (data: any) => void): () => void {
    const listener: AdEventListener = { type, adUnitId: '*', callback };
    const listeners = this.eventListeners.get(type) || [];
    listeners.push(listener);
    this.eventListeners.set(type, listeners);

    return () => {
      const current = this.eventListeners.get(type) || [];
      const index = current.indexOf(listener);
      if (index > -1) {
        current.splice(index, 1);
        this.eventListeners.set(type, current);
      }
    };
  }

  removeAllEventListeners(): void {
    this.eventListeners.clear();
  }

  // MARK: - Cleanup

  destroy(): void {
    this.subscriptions.forEach((sub) => sub.remove());
    this.subscriptions = [];
    this.eventListeners.clear();
    this.initialized = false;
  }
}

// MARK: - Standard Ad Sizes

export const AdSize = {
  BANNER: { width: 320, height: 50 } as AdSize,
  LARGE_BANNER: { width: 320, height: 100 } as AdSize,
  MEDIUM_RECTANGLE: { width: 300, height: 250 } as AdSize,
  FULL_BANNER: { width: 468, height: 60 } as AdSize,
  LEADERBOARD: { width: 728, height: 90 } as AdSize,

  adaptive: (width: number): AdSize => ({
    width,
    height: 0, // Will be calculated natively
    isAdaptive: true,
  }),

  custom: (width: number, height: number): AdSize => ({
    width,
    height,
    isAdaptive: false,
  }),
};

// Export singleton instance
export const AdProject = AdProjectSDK.getInstance();

// Default export
export default AdProject;
