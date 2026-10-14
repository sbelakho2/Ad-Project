/**
 * NetworkMonitor tests
 * Tests the network monitoring and fast-fail functionality
 */

// Declare global for Node.js environment
declare const global: typeof globalThis;

import {
  NetworkMonitor,
  getNetworkMonitor,
  startNetworkMonitoring,
  stopNetworkMonitoring,
  getNetworkState,
  isNetworkConnected,
  preflightNetworkCheck,
  getEffectiveTimeout,
  addNetworkListener,
  removeNetworkListener,
  NoNetworkError,
  OFFLINE_FAST_FAIL_TIMEOUT_MS,
  NORMAL_TIMEOUT_MS,
  NetworkState
} from '../src/networkMonitor';

// Create mock globals before importing
const mockAddEventListener = jest.fn();
const mockRemoveEventListener = jest.fn();

// Mock window if not defined
if (typeof window === 'undefined') {
  (global as any).window = {
    addEventListener: mockAddEventListener,
    removeEventListener: mockRemoveEventListener
  };
} else {
  jest.spyOn(window, 'addEventListener').mockImplementation(mockAddEventListener);
  jest.spyOn(window, 'removeEventListener').mockImplementation(mockRemoveEventListener);
}

// Mock crypto.randomUUID
let uuidCounter = 0;
const mockRandomUUID = jest.fn(() => `test-uuid-${++uuidCounter}`);
(global as any).crypto = { randomUUID: mockRandomUUID };

describe('NetworkMonitor', () => {
  let mockConnection: {
    type?: string;
    effectiveType?: string;
    downlink?: number;
    rtt?: number;
    saveData?: boolean;
    addEventListener: jest.Mock;
    removeEventListener: jest.Mock;
  };
  
  let originalNavigator: any;
  let mockNavigatorOnLine = true;
  
  beforeEach(() => {
    // Reset singleton
    NetworkMonitor.resetInstance();
    
    // Reset counters and mocks
    uuidCounter = 0;
    mockAddEventListener.mockClear();
    mockRemoveEventListener.mockClear();
    mockRandomUUID.mockClear();
    
    // Create mock connection object
    mockConnection = {
      type: 'wifi',
      effectiveType: '4g',
      downlink: 10,
      rtt: 50,
      saveData: false,
      addEventListener: jest.fn(),
      removeEventListener: jest.fn()
    };
    
    // Default to online
    mockNavigatorOnLine = true;
    
    // Store original and mock navigator
    originalNavigator = (global as any).navigator;
    Object.defineProperty(global, 'navigator', {
      value: {
        get onLine() {
          return mockNavigatorOnLine;
        },
        connection: mockConnection
      },
      writable: true,
      configurable: true
    });
  });
  
  afterEach(() => {
    NetworkMonitor.resetInstance();
    if (originalNavigator !== undefined) {
      Object.defineProperty(global, 'navigator', {
        value: originalNavigator,
        writable: true,
        configurable: true
      });
    }
    jest.clearAllMocks();
  });
  
  describe('Constants', () => {
    it('should have correct fast-fail timeout', () => {
      expect(OFFLINE_FAST_FAIL_TIMEOUT_MS).toBe(100);
    });
    
    it('should have correct normal timeout', () => {
      expect(NORMAL_TIMEOUT_MS).toBe(10000);
    });
  });
  
  describe('Singleton pattern', () => {
    it('should return the same instance', () => {
      const instance1 = NetworkMonitor.getInstance();
      const instance2 = NetworkMonitor.getInstance();
      
      expect(instance1).toBe(instance2);
    });
    
    it('should reset instance correctly', () => {
      const instance1 = NetworkMonitor.getInstance();
      NetworkMonitor.resetInstance();
      const instance2 = NetworkMonitor.getInstance();
      
      expect(instance1).not.toBe(instance2);
    });
    
    it('should use getInstance via getNetworkMonitor', () => {
      const instance = getNetworkMonitor();
      expect(instance).toBe(NetworkMonitor.getInstance());
    });
  });
  
  describe('Network state when online', () => {
    it('should report connected state', () => {
      const monitor = getNetworkMonitor();
      
      expect(monitor.isConnected).toBe(true);
      expect(monitor.state.isConnected).toBe(true);
    });
    
    it('should detect connection type from Network Information API', () => {
      mockConnection.type = 'wifi';
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.state.connectionType).toBe('wifi');
    });
    
    it('should detect effective type', () => {
      mockConnection.effectiveType = '4g';
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.state.effectiveType).toBe('4g');
    });
    
    it('should read downlink bandwidth', () => {
      mockConnection.downlink = 25.5;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.state.downlinkMbps).toBe(25.5);
    });
    
    it('should read RTT', () => {
      mockConnection.rtt = 100;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.state.rtt).toBe(100);
    });
    
    it('should detect save-data preference', () => {
      mockConnection.saveData = true;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.state.saveData).toBe(true);
      expect(monitor.isSaveData).toBe(true);
    });
    
    it('should include timestamp', () => {
      const before = Date.now();
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      const after = Date.now();
      
      expect(monitor.state.timestamp).toBeGreaterThanOrEqual(before);
      expect(monitor.state.timestamp).toBeLessThanOrEqual(after);
    });
  });
  
  describe('Network state when offline', () => {
    beforeEach(() => {
      mockNavigatorOnLine = false;
    });
    
    it('should report disconnected state', () => {
      const monitor = getNetworkMonitor();
      
      expect(monitor.isConnected).toBe(false);
      expect(monitor.state.isConnected).toBe(false);
    });
    
    it('should report connection type as none', () => {
      const monitor = getNetworkMonitor();
      
      expect(monitor.state.connectionType).toBe('none');
    });
    
    it('should have null network metrics when offline', () => {
      const monitor = getNetworkMonitor();
      
      expect(monitor.state.effectiveType).toBeNull();
      expect(monitor.state.downlinkMbps).toBeNull();
      expect(monitor.state.rtt).toBeNull();
    });
  });
  
  describe('Monitoring lifecycle', () => {
    it('should register window event listeners when started', () => {
      const monitor = getNetworkMonitor();
      monitor.startMonitoring();
      
      expect(mockAddEventListener).toHaveBeenCalledWith('online', expect.any(Function));
      expect(mockAddEventListener).toHaveBeenCalledWith('offline', expect.any(Function));
    });
    
    it('should register connection listener when started', () => {
      const monitor = getNetworkMonitor();
      monitor.startMonitoring();
      
      expect(mockConnection.addEventListener).toHaveBeenCalledWith('change', expect.any(Function));
    });
    
    it('should remove event listeners when stopped', () => {
      const monitor = getNetworkMonitor();
      monitor.startMonitoring();
      monitor.stopMonitoring();
      
      expect(mockRemoveEventListener).toHaveBeenCalledWith('online', expect.any(Function));
      expect(mockRemoveEventListener).toHaveBeenCalledWith('offline', expect.any(Function));
      expect(mockConnection.removeEventListener).toHaveBeenCalledWith('change', expect.any(Function));
    });
    
    it('should not register listeners twice', () => {
      const monitor = getNetworkMonitor();
      monitor.startMonitoring();
      const callCountBefore = mockAddEventListener.mock.calls.length;
      
      monitor.startMonitoring();
      
      expect(mockAddEventListener.mock.calls.length).toBe(callCountBefore);
    });
    
    it('should not remove listeners if not monitoring', () => {
      const monitor = getNetworkMonitor();
      monitor.stopMonitoring();
      
      expect(mockRemoveEventListener).not.toHaveBeenCalled();
    });
    
    it('should use functional API for start/stop', () => {
      startNetworkMonitoring();
      expect(mockAddEventListener).toHaveBeenCalled();
      
      stopNetworkMonitoring();
      expect(mockRemoveEventListener).toHaveBeenCalled();
    });
  });
  
  describe('Preflight check', () => {
    it('should return proceed result when online', () => {
      const result = preflightNetworkCheck();
      
      expect(result.type).toBe('proceed');
      if (result.type === 'proceed') {
        expect(result.state.isConnected).toBe(true);
      }
    });
    
    it('should return fast-fail result when offline', () => {
      mockNavigatorOnLine = false;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      const result = monitor.preflight();
      
      expect(result.type).toBe('fastFail');
      if (result.type === 'fastFail') {
        expect(result.reason).toBe('No network connection');
        expect(result.state.isConnected).toBe(false);
      }
    });
    
    it('should force update before preflight', () => {
      const monitor = getNetworkMonitor();
      
      // Start online
      expect(monitor.preflight().type).toBe('proceed');
      
      // Go offline
      mockNavigatorOnLine = false;
      
      // Preflight should detect offline state
      expect(monitor.preflight().type).toBe('fastFail');
    });
  });
  
  describe('Effective timeout', () => {
    it('should return normal timeout when online', () => {
      const timeout = getEffectiveTimeout();
      
      expect(timeout).toBe(NORMAL_TIMEOUT_MS);
    });
    
    it('should return fast-fail timeout when offline', () => {
      mockNavigatorOnLine = false;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      const timeout = monitor.effectiveTimeoutMs();
      
      expect(timeout).toBe(OFFLINE_FAST_FAIL_TIMEOUT_MS);
    });
  });
  
  describe('Listeners', () => {
    it('should add listener and return token', () => {
      const listener = jest.fn();
      const token = addNetworkListener(listener);
      
      expect(token).toBe('test-uuid-1');
    });
    
    it('should immediately notify listener with current state', () => {
      const listener = jest.fn();
      addNetworkListener(listener);
      
      expect(listener).toHaveBeenCalledTimes(1);
      expect(listener).toHaveBeenCalledWith(expect.objectContaining({
        isConnected: true
      }));
    });
    
    it('should remove listener with token', () => {
      const listener = jest.fn();
      const token = addNetworkListener(listener);
      listener.mockClear();
      
      removeNetworkListener(token);
      
      // Force an update - listener should not be called again
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(listener).not.toHaveBeenCalled();
    });
    
    it('should handle multiple listeners', () => {
      const listener1 = jest.fn();
      const listener2 = jest.fn();
      
      const token1 = getNetworkMonitor().addListener(listener1);
      const token2 = getNetworkMonitor().addListener(listener2);
      
      expect(token1).not.toBe(token2);
      expect(listener1).toHaveBeenCalled();
      expect(listener2).toHaveBeenCalled();
    });
  });
  
  describe('Quality hints', () => {
    it('should return quality hints object', () => {
      mockConnection.effectiveType = '4g';
      mockConnection.downlink = 10;
      mockConnection.rtt = 50;
      mockConnection.saveData = false;
      
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      const hints = monitor.qualityHints();
      
      expect(hints.isConnected).toBe(true);
      expect(hints.connectionType).toBe('wifi');
      expect(hints.effectiveType).toBe('4g');
      expect(hints.downlinkMbps).toBe(10);
      expect(hints.rtt).toBe(50);
      expect(hints.saveData).toBe(false);
      expect(hints.suggestedTimeout).toBe(NORMAL_TIMEOUT_MS);
      expect(hints.shouldReduceQuality).toBe(false);
    });
    
    it('should suggest reduce quality for slow connections', () => {
      mockConnection.effectiveType = '2g';
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      const hints = monitor.qualityHints();
      
      expect(hints.shouldReduceQuality).toBe(true);
    });
    
    it('should suggest reduce quality for save-data', () => {
      mockConnection.saveData = true;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      const hints = monitor.qualityHints();
      
      expect(hints.shouldReduceQuality).toBe(true);
    });
  });
  
  describe('isSuitableForHighQuality', () => {
    it('should return true for good 4G connection', () => {
      mockConnection.effectiveType = '4g';
      mockConnection.downlink = 10;
      mockConnection.rtt = 50;
      mockConnection.saveData = false;
      
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.isSuitableForHighQuality()).toBe(true);
    });
    
    it('should return false when offline', () => {
      mockNavigatorOnLine = false;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.isSuitableForHighQuality()).toBe(false);
    });
    
    it('should return false for save-data mode', () => {
      mockConnection.saveData = true;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.isSuitableForHighQuality()).toBe(false);
    });
    
    it('should return false for slow-2g', () => {
      mockConnection.effectiveType = 'slow-2g';
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.isSuitableForHighQuality()).toBe(false);
    });
    
    it('should return false for 2g', () => {
      mockConnection.effectiveType = '2g';
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.isSuitableForHighQuality()).toBe(false);
    });
    
    it('should return false for high RTT (>300ms)', () => {
      mockConnection.effectiveType = '4g';
      mockConnection.rtt = 400;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.isSuitableForHighQuality()).toBe(false);
    });
    
    it('should return false for low bandwidth (<1Mbps)', () => {
      mockConnection.effectiveType = '4g';
      mockConnection.downlink = 0.5;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.isSuitableForHighQuality()).toBe(false);
    });
  });
  
  describe('Functional API', () => {
    it('getNetworkState should return current state', () => {
      const state = getNetworkState();
      
      expect(state).toBeDefined();
      expect(state.isConnected).toBe(true);
    });
    
    it('isNetworkConnected should return connection status', () => {
      expect(isNetworkConnected()).toBe(true);
      
      mockNavigatorOnLine = false;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(isNetworkConnected()).toBe(false);
    });
  });
  
  describe('NoNetworkError', () => {
    it('should be an instance of Error', () => {
      const state: NetworkState = {
        isConnected: false,
        connectionType: 'none',
        effectiveType: null,
        downlinkMbps: null,
        rtt: null,
        saveData: false,
        timestamp: Date.now()
      };
      
      const error = new NoNetworkError('No network', state);
      
      expect(error).toBeInstanceOf(Error);
      expect(error.name).toBe('NoNetworkError');
      expect(error.message).toBe('No network');
      expect(error.networkState).toBe(state);
    });
  });
  
  describe('Edge cases', () => {
    it('should handle missing connection API', () => {
      Object.defineProperty(global, 'navigator', {
        value: {
          onLine: true
          // No connection property
        },
        writable: true,
        configurable: true
      });
      
      NetworkMonitor.resetInstance();
      const monitor = getNetworkMonitor();
      
      expect(monitor.isConnected).toBe(true);
      expect(monitor.state.effectiveType).toBeNull();
    });
    
    it('should handle listener errors gracefully', () => {
      const monitor = getNetworkMonitor();
      
      // First add a bad listener that throws (wrapped in try-catch so addListener doesn't fail)
      let badListenerCalled = false;
      const badListener = () => {
        if (badListenerCalled) {
          throw new Error('Listener error');
        }
        badListenerCalled = true;
      };
      const goodListener = jest.fn();
      
      monitor.addListener(badListener);
      monitor.addListener(goodListener);
      
      goodListener.mockClear();
      
      // Now force update - bad listener should throw but not prevent good listener
      expect(() => monitor.forceUpdate()).not.toThrow();
    });
    
    it('should handle unknown connection type', () => {
      mockConnection.type = undefined;
      const monitor = getNetworkMonitor();
      monitor.forceUpdate();
      
      expect(monitor.state.connectionType).toBe('unknown');
    });
  });
});
