import React, { useEffect, useRef, useState, useCallback } from 'react';
import { View, StyleSheet, requireNativeComponent, Platform, ViewProps } from 'react-native';
import { AdProject, AdSize, AdLoadResult, AdError, AdEventType } from './index';

// MARK: - Types

export interface BannerAdProps extends ViewProps {
  adUnitId: string;
  size?: AdSize;
  onAdLoaded?: (result: AdLoadResult) => void;
  onAdFailedToLoad?: (error: AdError) => void;
  onAdOpened?: () => void;
  onAdClosed?: () => void;
  onAdClicked?: () => void;
  onAdImpression?: (data: { revenue: number; networkName: string }) => void;
  autoRefresh?: boolean;
  refreshInterval?: number; // in seconds
}

export interface UseBannerAdReturn {
  isLoading: boolean;
  isLoaded: boolean;
  error: AdError | null;
  load: () => Promise<void>;
  show: () => Promise<void>;
  hide: () => Promise<void>;
  destroy: () => Promise<void>;
}

export interface UseInterstitialAdReturn {
  isLoading: boolean;
  isLoaded: boolean;
  error: AdError | null;
  load: () => Promise<void>;
  show: () => Promise<void>;
}

export interface UseRewardedAdReturn {
  isLoading: boolean;
  isLoaded: boolean;
  error: AdError | null;
  reward: { type: string; amount: number } | null;
  load: () => Promise<void>;
  show: () => Promise<void>;
}

// MARK: - Native Component

const RNAdProjectBannerView = Platform.select({
  ios: requireNativeComponent<any>('RNAdProjectBannerView'),
  android: requireNativeComponent<any>('RNAdProjectBannerView'),
  default: View,
});

// MARK: - BannerAd Component

export const BannerAd: React.FC<BannerAdProps> = ({
  adUnitId,
  size = AdSize.BANNER,
  onAdLoaded,
  onAdFailedToLoad,
  onAdOpened,
  onAdClosed,
  onAdClicked,
  onAdImpression,
  autoRefresh = false,
  refreshInterval = 60,
  style,
  ...props
}) => {
  const [isLoaded, setIsLoaded] = useState(false);
  const refreshTimerRef = useRef<NodeJS.Timeout | null>(null);

  useEffect(() => {
    const unsubscribers: (() => void)[] = [];

    // Set up event listeners
    unsubscribers.push(
      AdProject.addEventListener(adUnitId, 'onAdLoaded', (data) => {
        setIsLoaded(true);
        onAdLoaded?.(data);
      })
    );

    unsubscribers.push(
      AdProject.addEventListener(adUnitId, 'onAdFailedToLoad', (data) => {
        setIsLoaded(false);
        onAdFailedToLoad?.(data.error);
      })
    );

    if (onAdOpened) {
      unsubscribers.push(AdProject.addEventListener(adUnitId, 'onAdOpened', onAdOpened));
    }

    if (onAdClosed) {
      unsubscribers.push(AdProject.addEventListener(adUnitId, 'onAdClosed', onAdClosed));
    }

    if (onAdClicked) {
      unsubscribers.push(AdProject.addEventListener(adUnitId, 'onAdClicked', onAdClicked));
    }

    if (onAdImpression) {
      unsubscribers.push(AdProject.addEventListener(adUnitId, 'onAdImpression', onAdImpression));
    }

    return () => {
      unsubscribers.forEach((unsub) => unsub());
      if (refreshTimerRef.current) {
        clearInterval(refreshTimerRef.current);
      }
    };
  }, [adUnitId, onAdLoaded, onAdFailedToLoad, onAdOpened, onAdClosed, onAdClicked, onAdImpression]);

  // Auto-refresh logic
  useEffect(() => {
    if (autoRefresh && isLoaded) {
      refreshTimerRef.current = setInterval(() => {
        AdProject.loadBannerAd(adUnitId, size);
      }, refreshInterval * 1000);

      return () => {
        if (refreshTimerRef.current) {
          clearInterval(refreshTimerRef.current);
        }
      };
    }
  }, [autoRefresh, isLoaded, refreshInterval, adUnitId, size]);

  return (
    <View style={[styles.bannerContainer, style]} {...props}>
      <RNAdProjectBannerView
        adUnitId={adUnitId}
        adWidth={size.width}
        adHeight={size.height}
        isAdaptive={size.isAdaptive ?? false}
        style={{
          width: size.width,
          height: size.isAdaptive ? 'auto' : size.height,
        }}
      />
    </View>
  );
};

// MARK: - Hooks

export function useBannerAd(adUnitId: string, size: AdSize = AdSize.BANNER): UseBannerAdReturn {
  const [isLoading, setIsLoading] = useState(false);
  const [isLoaded, setIsLoaded] = useState(false);
  const [error, setError] = useState<AdError | null>(null);

  useEffect(() => {
    const unsubLoaded = AdProject.addEventListener(adUnitId, 'onAdLoaded', () => {
      setIsLoading(false);
      setIsLoaded(true);
      setError(null);
    });

    const unsubFailed = AdProject.addEventListener(adUnitId, 'onAdFailedToLoad', (data) => {
      setIsLoading(false);
      setIsLoaded(false);
      setError(data.error);
    });

    return () => {
      unsubLoaded();
      unsubFailed();
    };
  }, [adUnitId]);

  const load = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      await AdProject.loadBannerAd(adUnitId, size);
    } catch (e) {
      setIsLoading(false);
      setError({ code: 'LOAD_ERROR', message: String(e) });
    }
  }, [adUnitId, size]);

  const show = useCallback(async () => {
    await AdProject.showBannerAd(adUnitId);
  }, [adUnitId]);

  const hide = useCallback(async () => {
    await AdProject.hideBannerAd(adUnitId);
  }, [adUnitId]);

  const destroy = useCallback(async () => {
    await AdProject.destroyBannerAd(adUnitId);
    setIsLoaded(false);
  }, [adUnitId]);

  return { isLoading, isLoaded, error, load, show, hide, destroy };
}

export function useInterstitialAd(adUnitId: string): UseInterstitialAdReturn {
  const [isLoading, setIsLoading] = useState(false);
  const [isLoaded, setIsLoaded] = useState(false);
  const [error, setError] = useState<AdError | null>(null);

  useEffect(() => {
    const unsubLoaded = AdProject.addEventListener(adUnitId, 'onAdLoaded', () => {
      setIsLoading(false);
      setIsLoaded(true);
      setError(null);
    });

    const unsubFailed = AdProject.addEventListener(adUnitId, 'onAdFailedToLoad', (data) => {
      setIsLoading(false);
      setIsLoaded(false);
      setError(data.error);
    });

    const unsubClosed = AdProject.addEventListener(adUnitId, 'onAdClosed', () => {
      setIsLoaded(false);
    });

    return () => {
      unsubLoaded();
      unsubFailed();
      unsubClosed();
    };
  }, [adUnitId]);

  const load = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      await AdProject.loadInterstitialAd(adUnitId);
    } catch (e) {
      setIsLoading(false);
      setError({ code: 'LOAD_ERROR', message: String(e) });
    }
  }, [adUnitId]);

  const show = useCallback(async () => {
    if (isLoaded) {
      await AdProject.showInterstitialAd(adUnitId);
    }
  }, [adUnitId, isLoaded]);

  return { isLoading, isLoaded, error, load, show };
}

export function useRewardedAd(adUnitId: string): UseRewardedAdReturn {
  const [isLoading, setIsLoading] = useState(false);
  const [isLoaded, setIsLoaded] = useState(false);
  const [error, setError] = useState<AdError | null>(null);
  const [reward, setReward] = useState<{ type: string; amount: number } | null>(null);

  useEffect(() => {
    const unsubLoaded = AdProject.addEventListener(adUnitId, 'onAdLoaded', () => {
      setIsLoading(false);
      setIsLoaded(true);
      setError(null);
    });

    const unsubFailed = AdProject.addEventListener(adUnitId, 'onAdFailedToLoad', (data) => {
      setIsLoading(false);
      setIsLoaded(false);
      setError(data.error);
    });

    const unsubClosed = AdProject.addEventListener(adUnitId, 'onAdClosed', () => {
      setIsLoaded(false);
    });

    const unsubReward = AdProject.addEventListener(adUnitId, 'onRewardEarned', (data) => {
      setReward({ type: data.rewardType, amount: data.rewardAmount });
    });

    return () => {
      unsubLoaded();
      unsubFailed();
      unsubClosed();
      unsubReward();
    };
  }, [adUnitId]);

  const load = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    setReward(null);
    try {
      await AdProject.loadRewardedAd(adUnitId);
    } catch (e) {
      setIsLoading(false);
      setError({ code: 'LOAD_ERROR', message: String(e) });
    }
  }, [adUnitId]);

  const show = useCallback(async () => {
    if (isLoaded) {
      await AdProject.showRewardedAd(adUnitId);
    }
  }, [adUnitId, isLoaded]);

  return { isLoading, isLoaded, error, reward, load, show };
}

// MARK: - Styles

const styles = StyleSheet.create({
  bannerContainer: {
    alignItems: 'center',
    justifyContent: 'center',
  },
});

export default BannerAd;
