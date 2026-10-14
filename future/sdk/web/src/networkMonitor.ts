/**
 * NetworkMonitor - Monitors network connectivity and provides fast-fail behavior.
 *
 * This module provides:
 * - Real-time network state monitoring using Navigator.onLine and Network Information API
 * - Fast-fail for no-network conditions (no UI jank)
 * - Connection quality assessment
 * - Callback-based network state change notifications
 */

/** Fast-fail timeout when offline (ms) */
export const OFFLINE_FAST_FAIL_TIMEOUT_MS = 100;

/** Normal network timeout (ms) */
export const NORMAL_TIMEOUT_MS = 10000;

/** Connection type enumeration */
export type ConnectionType = 
  | 'none'
  | 'wifi'
  | 'cellular'
  | 'ethernet'
  | 'bluetooth'
  | 'wimax'
  | 'other'
  | 'unknown';

/** Effective connection type (from Network Information API) */
export type EffectiveConnectionType = 'slow-2g' | '2g' | '3g' | '4g';

/** Network state information */
export interface NetworkState {
  readonly isConnected: boolean;
  readonly connectionType: ConnectionType;
  readonly effectiveType: EffectiveConnectionType | null;
  readonly downlinkMbps: number | null;
  readonly rtt: number | null;
  readonly saveData: boolean;
  readonly timestamp: number;
}

/** Result of a pre-flight check before network operations */
export type PreflightResult = 
  | { type: 'proceed'; state: NetworkState }
  | { type: 'fastFail'; reason: string; state: NetworkState };

/** Listener callback for network state changes */
export type NetworkStateListener = (state: NetworkState) => void;

/** Network Information API typings */
interface NetworkInformation extends EventTarget {
  readonly type?: ConnectionType;
  readonly effectiveType?: EffectiveConnectionType;
  readonly downlink?: number;
  readonly rtt?: number;
  readonly saveData?: boolean;
  onchange?: ((this: NetworkInformation, ev: Event) => void) | null;
}

interface NavigatorWithConnection extends Navigator {
  readonly connection?: NetworkInformation;
  readonly mozConnection?: NetworkInformation;
  readonly webkitConnection?: NetworkInformation;
}

/** Creates an offline network state */
function offlineState(): NetworkState {
  return {
    isConnected: false,
    connectionType: 'none',
    effectiveType: null,
    downlinkMbps: null,
    rtt: null,
    saveData: false,
    timestamp: Date.now()
  };
}

/** Class-based NetworkMonitor implementation */
export class NetworkMonitor {
  private static instance: NetworkMonitor | null = null;
  
  private currentState: NetworkState = offlineState();
  private listeners = new Map<string, NetworkStateListener>();
  private isMonitoring = false;
  private connection: NetworkInformation | null = null;
  
  private boundOnlineHandler: () => void;
  private boundOfflineHandler: () => void;
  private boundConnectionHandler: () => void;
  
  private constructor() {
    this.boundOnlineHandler = this.handleOnline.bind(this);
    this.boundOfflineHandler = this.handleOffline.bind(this);
    this.boundConnectionHandler = this.handleConnectionChange.bind(this);
    
    // Get Network Information API reference
    if (typeof navigator !== 'undefined') {
      const nav = navigator as NavigatorWithConnection;
      this.connection = nav.connection ?? nav.mozConnection ?? nav.webkitConnection ?? null;
    }
    
    // Initialize state
    this.updateState();
  }
  
  /** Gets the singleton instance */
  static getInstance(): NetworkMonitor {
    if (!NetworkMonitor.instance) {
      NetworkMonitor.instance = new NetworkMonitor();
    }
    return NetworkMonitor.instance;
  }
  
  /** Resets the singleton (for testing) */
  static resetInstance(): void {
    if (NetworkMonitor.instance) {
      NetworkMonitor.instance.stopMonitoring();
      NetworkMonitor.instance = null;
    }
  }
  
  /** Gets the current network state */
  get state(): NetworkState {
    return this.currentState;
  }
  
  /** Checks if network is currently connected */
  get isConnected(): boolean {
    return this.currentState.isConnected;
  }
  
  /** Checks if save-data mode is enabled */
  get isSaveData(): boolean {
    return this.currentState.saveData;
  }
  
  /** Starts monitoring network state changes */
  startMonitoring(): void {
    if (this.isMonitoring) return;
    this.isMonitoring = true;
    
    if (typeof window !== 'undefined') {
      window.addEventListener('online', this.boundOnlineHandler);
      window.addEventListener('offline', this.boundOfflineHandler);
    }
    
    if (this.connection) {
      this.connection.addEventListener('change', this.boundConnectionHandler);
    }
    
    // Update current state
    this.updateState();
  }
  
  /** Stops monitoring network state changes */
  stopMonitoring(): void {
    if (!this.isMonitoring) return;
    this.isMonitoring = false;
    
    if (typeof window !== 'undefined') {
      window.removeEventListener('online', this.boundOnlineHandler);
      window.removeEventListener('offline', this.boundOfflineHandler);
    }
    
    if (this.connection) {
      this.connection.removeEventListener('change', this.boundConnectionHandler);
    }
  }
  
  /**
   * Performs a preflight check before network operations.
   * Returns immediately if offline, allowing fast-fail behavior.
   */
  preflight(): PreflightResult {
    // Ensure we have the latest state
    this.updateState();
    
    if (this.currentState.isConnected) {
      return { type: 'proceed', state: this.currentState };
    }
    
    return {
      type: 'fastFail',
      reason: 'No network connection',
      state: this.currentState
    };
  }
  
  /**
   * Gets the appropriate timeout based on network state.
   * Returns fast-fail timeout when offline.
   */
  effectiveTimeoutMs(): number {
    return this.isConnected ? NORMAL_TIMEOUT_MS : OFFLINE_FAST_FAIL_TIMEOUT_MS;
  }
  
  /**
   * Adds a listener for network state changes.
   * @returns A token to use when removing the listener
   */
  addListener(listener: NetworkStateListener): string {
    const token = crypto.randomUUID();
    this.listeners.set(token, listener);
    
    // Immediately notify with current state
    listener(this.currentState);
    return token;
  }
  
  /**
   * Removes a listener using its token.
   */
  removeListener(token: string): void {
    this.listeners.delete(token);
  }
  
  /**
   * Gets network quality hints for adaptive behavior.
   */
  qualityHints(): Record<string, unknown> {
    const state = this.currentState;
    return {
      isConnected: state.isConnected,
      connectionType: state.connectionType,
      effectiveType: state.effectiveType,
      downlinkMbps: state.downlinkMbps,
      rtt: state.rtt,
      saveData: state.saveData,
      suggestedTimeout: this.effectiveTimeoutMs(),
      shouldReduceQuality: state.saveData || 
        state.effectiveType === 'slow-2g' || 
        state.effectiveType === '2g'
    };
  }
  
  /**
   * Checks if connection is suitable for high-quality media.
   */
  isSuitableForHighQuality(): boolean {
    if (!this.isConnected) return false;
    
    const state = this.currentState;
    
    // Not suitable if save-data is on
    if (state.saveData) return false;
    
    // Not suitable for slow connections
    if (state.effectiveType === 'slow-2g' || state.effectiveType === '2g') {
      return false;
    }
    
    // Not suitable if RTT is too high (>300ms)
    if (state.rtt !== null && state.rtt > 300) {
      return false;
    }
    
    // Not suitable if bandwidth is too low (<1Mbps)
    if (state.downlinkMbps !== null && state.downlinkMbps < 1) {
      return false;
    }
    
    return true;
  }
  
  /** Forces an immediate state update */
  forceUpdate(): void {
    this.updateState();
  }
  
  private handleOnline(): void {
    this.updateState();
    this.notifyListeners();
  }
  
  private handleOffline(): void {
    this.currentState = offlineState();
    this.notifyListeners();
  }
  
  private handleConnectionChange(): void {
    this.updateState();
    this.notifyListeners();
  }
  
  private updateState(): void {
    const isOnline = typeof navigator !== 'undefined' ? navigator.onLine : true;
    
    if (!isOnline) {
      this.currentState = offlineState();
      return;
    }
    
    // Map Network Information API type to our ConnectionType
    let connectionType: ConnectionType = 'unknown';
    if (this.connection?.type) {
      connectionType = this.connection.type as ConnectionType;
    } else if (isOnline) {
      connectionType = 'unknown';
    }
    
    this.currentState = {
      isConnected: isOnline,
      connectionType,
      effectiveType: this.connection?.effectiveType ?? null,
      downlinkMbps: this.connection?.downlink ?? null,
      rtt: this.connection?.rtt ?? null,
      saveData: this.connection?.saveData ?? false,
      timestamp: Date.now()
    };
  }
  
  private notifyListeners(): void {
    const state = this.currentState;
    for (const listener of this.listeners.values()) {
      try {
        listener(state);
      } catch {
        // Ignore listener errors
      }
    }
  }
}

/** Functional API for simpler use cases */

/** Gets the shared NetworkMonitor instance */
export function getNetworkMonitor(): NetworkMonitor {
  return NetworkMonitor.getInstance();
}

/** Starts network monitoring */
export function startNetworkMonitoring(): void {
  getNetworkMonitor().startMonitoring();
}

/** Stops network monitoring */
export function stopNetworkMonitoring(): void {
  getNetworkMonitor().stopMonitoring();
}

/** Gets current network state */
export function getNetworkState(): NetworkState {
  return getNetworkMonitor().state;
}

/** Checks if network is connected */
export function isNetworkConnected(): boolean {
  return getNetworkMonitor().isConnected;
}

/** Performs a preflight check before network operations */
export function preflightNetworkCheck(): PreflightResult {
  return getNetworkMonitor().preflight();
}

/** Gets effective timeout based on network state */
export function getEffectiveTimeout(): number {
  return getNetworkMonitor().effectiveTimeoutMs();
}

/** Gets network quality hints */
export function getNetworkQualityHints(): Record<string, unknown> {
  return getNetworkMonitor().qualityHints();
}

/** Adds a network state change listener */
export function addNetworkListener(listener: NetworkStateListener): string {
  return getNetworkMonitor().addListener(listener);
}

/** Removes a network state change listener */
export function removeNetworkListener(token: string): void {
  getNetworkMonitor().removeListener(token);
}

/** Error class for fast-fail on no-network conditions */
export class NoNetworkError extends Error {
  readonly networkState: NetworkState;
  
  constructor(message: string, networkState: NetworkState) {
    super(message);
    this.name = 'NoNetworkError';
    this.networkState = networkState;
  }
}
