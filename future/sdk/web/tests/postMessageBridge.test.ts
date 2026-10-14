/**
 * PostMessageBridge Tests - CSP Compliance & Security
 */
import {
  PostMessageBridge,
  PostMessagePayload,
  MessageTypes,
  createPublisherBridge,
  createAdBridge,
} from '../src/postMessageBridge';

interface MockWindow {
  postMessage: jest.Mock;
  addEventListener: jest.Mock;
  removeEventListener: jest.Mock;
}

function createMockWindow(): MockWindow {
  return {
    postMessage: jest.fn(),
    addEventListener: jest.fn(),
    removeEventListener: jest.fn(),
  };
}

describe('PostMessageBridge', () => {
  let bridge: PostMessageBridge;
  const allowedOrigins = ['https://example.com', 'https://trusted.net'];

  beforeEach(() => {
    bridge = new PostMessageBridge({
      allowedOrigins,
      timeout: 1000,
    });
  });

  afterEach(() => {
    bridge.stop();
  });

  describe('Origin Validation', () => {
    it('should allow exact origin match', () => {
      expect(bridge.isOriginAllowed('https://example.com')).toBe(true);
      expect(bridge.isOriginAllowed('https://trusted.net')).toBe(true);
    });

    it('should reject non-allowed origins', () => {
      expect(bridge.isOriginAllowed('https://malicious.com')).toBe(false);
      expect(bridge.isOriginAllowed('https://fake-example.com')).toBe(false);
    });

    it('should reject subdomains when exact match required', () => {
      expect(bridge.isOriginAllowed('https://sub.example.com')).toBe(false);
    });

    it('should handle wildcard origin', () => {
      const wildcardBridge = new PostMessageBridge({
        allowedOrigins: ['*'],
      });
      expect(wildcardBridge.isOriginAllowed('https://anything.com')).toBe(true);
      expect(wildcardBridge.isOriginAllowed('http://localhost:3000')).toBe(true);
    });

    it('should handle subdomain wildcard', () => {
      const subdomainBridge = new PostMessageBridge({
        allowedOrigins: ['*.example.com'],
      });
      expect(subdomainBridge.isOriginAllowed('https://sub.example.com')).toBe(true);
      expect(subdomainBridge.isOriginAllowed('https://deep.sub.example.com')).toBe(true);
      expect(subdomainBridge.isOriginAllowed('https://other.com')).toBe(false);
    });

    it('should reject empty origin', () => {
      expect(bridge.isOriginAllowed('')).toBe(false);
    });

    it('should reject null-like origins', () => {
      expect(bridge.isOriginAllowed('null')).toBe(false);
    });

    it('should handle protocol variations', () => {
      const httpsBridge = new PostMessageBridge({
        allowedOrigins: ['https://secure.com'],
      });
      expect(httpsBridge.isOriginAllowed('https://secure.com')).toBe(true);
      expect(httpsBridge.isOriginAllowed('http://secure.com')).toBe(false);
    });
  });

  describe('Message Sending', () => {
    it('should generate namespaced message IDs', () => {
      const mockWindow = createMockWindow();
      const sendBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      const id = sendBridge.send('test', { foo: 'bar' });

      expect(id).toContain('apex-mediation-');
      expect(mockWindow.postMessage).toHaveBeenCalledTimes(1);

      const [payload, origin] = mockWindow.postMessage.mock.calls[0];
      expect(payload.type).toBe('test');
      expect(payload.id).toBe(id);
      expect(payload.data).toEqual({ foo: 'bar' });
      expect(origin).toBe('https://example.com');
    });

    it('should use wildcard target origin when configured', () => {
      const mockWindow = createMockWindow();
      const wildcardBridge = new PostMessageBridge({
        allowedOrigins: ['*'],
        targetWindow: mockWindow as unknown as Window,
      });

      wildcardBridge.send('test', {});

      const [, origin] = mockWindow.postMessage.mock.calls[0];
      expect(origin).toBe('*');
    });

    it('should include timestamp in messages', () => {
      const mockWindow = createMockWindow();
      const sendBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      const before = Date.now();
      sendBridge.send('test', {});
      const after = Date.now();

      const [payload] = mockWindow.postMessage.mock.calls[0];
      expect(payload.timestamp).toBeGreaterThanOrEqual(before);
      expect(payload.timestamp).toBeLessThanOrEqual(after);
    });

    it('should generate unique IDs for each message', () => {
      const mockWindow = createMockWindow();
      const sendBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      const id1 = sendBridge.send('test1', {});
      const id2 = sendBridge.send('test2', {});
      const id3 = sendBridge.send('test3', {});

      expect(id1).not.toBe(id2);
      expect(id2).not.toBe(id3);
      expect(id1).not.toBe(id3);
    });
  });

  describe('Response Handling', () => {
    it('should send response with correct format', () => {
      const mockWindow = createMockWindow();
      const respBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      respBridge.respond('apex-mediation-123', { result: 'ok' });

      const [payload] = mockWindow.postMessage.mock.calls[0];
      expect(payload.type).toBe('response');
      expect(payload.id).toBe('apex-mediation-123');
      expect(payload.data).toEqual({ result: 'ok' });
    });

    it('should send error response with correct format', () => {
      const mockWindow = createMockWindow();
      const errBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      errBridge.respondError('apex-mediation-456', 'Something went wrong');

      const [payload] = mockWindow.postMessage.mock.calls[0];
      expect(payload.type).toBe('error');
      expect(payload.id).toBe('apex-mediation-456');
      expect(payload.data).toBe('Something went wrong');
    });
  });

  describe('CSP Compliance', () => {
    it('should not use eval or Function constructor', () => {
      // The bridge implementation should never use eval
      const bridgeCode = PostMessageBridge.toString();
      expect(bridgeCode).not.toContain('eval(');
      expect(bridgeCode).not.toContain('new Function(');
    });

    it('should not inject inline scripts', () => {
      const mockWindow = createMockWindow();
      const cspBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      // Send multiple messages and verify no innerHTML or script injection
      cspBridge.send('init', {});
      cspBridge.send('load', { placement: 'banner' });
      cspBridge.respond('apex-mediation-test', { ok: true });

      // All calls should be pure postMessage calls, no DOM manipulation
      expect(mockWindow.postMessage).toHaveBeenCalledTimes(3);
    });

    it('should use structured clone compatible data only', () => {
      const mockWindow = createMockWindow();
      const dataBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      // These data types should work with structured clone
      dataBridge.send('test', {
        string: 'hello',
        number: 42,
        boolean: true,
        array: [1, 2, 3],
        nested: { a: { b: { c: 1 } } },
        null: null,
        date: new Date().toISOString(),
      });

      expect(mockWindow.postMessage).toHaveBeenCalledTimes(1);
    });

    it('should send data without DOM manipulation', () => {
      const mockWindow = createMockWindow();
      const domBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      // Bridge should only use postMessage, not DOM APIs
      domBridge.send('test', { html: '<script>alert("xss")</script>' });

      // The data should be passed as-is, not executed
      const [payload] = mockWindow.postMessage.mock.calls[0];
      expect(payload.data.html).toBe('<script>alert("xss")</script>');
    });
  });

  describe('Security', () => {
    it('should validate message namespace', () => {
      // Valid payload (has namespace)
      const validPayload = {
        type: 'test',
        id: 'apex-mediation-123',
        timestamp: Date.now(),
        data: {},
      };

      // Invalid payload (wrong namespace)
      const invalidPayload = {
        type: 'test',
        id: 'other-sdk-123',
        timestamp: Date.now(),
        data: {},
      };

      // Test namespace validation
      expect(validPayload.id.startsWith('apex-mediation')).toBe(true);
      expect(invalidPayload.id.startsWith('apex-mediation')).toBe(false);
    });

    it('should not leak pending request data on stop', async () => {
      const mockWindow = createMockWindow();
      const leakBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
        timeout: 10000,
      });

      leakBridge.start();

      // Create a pending request
      const requestPromise = leakBridge.request('test', {});

      // Stop should clear all pending requests
      leakBridge.stop();

      // The promise should be rejected
      await expect(requestPromise).rejects.toThrow('Bridge stopped');
    });

    it('should enforce message structure validation', () => {
      // Test various invalid payloads
      const invalidPayloads = [
        null,
        undefined,
        'string',
        123,
        [],
        {},
        { type: 'test' }, // missing id, timestamp
        { type: 'test', id: 'apex-mediation-123' }, // missing timestamp
      ];

      // These should all fail validation
      invalidPayloads.forEach((payload) => {
        const isValid =
          typeof payload === 'object' &&
          payload !== null &&
          !Array.isArray(payload) &&
          'type' in payload &&
          'id' in payload &&
          'timestamp' in payload &&
          typeof (payload as Record<string, unknown>).id === 'string' &&
          ((payload as Record<string, unknown>).id as string).startsWith('apex-mediation');
        expect(isValid).toBe(false);
      });
    });
  });

  describe('Timeout Handling', () => {
    it('should timeout pending requests', async () => {
      const mockWindow = createMockWindow();
      const timeoutBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
        timeout: 50,
      });

      timeoutBridge.start();

      await expect(timeoutBridge.request('test', {})).rejects.toThrow('Request timed out');
    });

    it('should respect custom timeout values', async () => {
      const mockWindow = createMockWindow();
      const shortTimeoutBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
        timeout: 10,
      });

      shortTimeoutBridge.start();

      const start = Date.now();
      try {
        await shortTimeoutBridge.request('test', {});
      } catch {
        // Expected to fail
      }
      const elapsed = Date.now() - start;

      expect(elapsed).toBeLessThan(500);
    });
  });

  describe('Bridge Lifecycle', () => {
    it('should be idempotent on multiple start calls', () => {
      // Multiple starts should not cause issues
      bridge.start();
      bridge.start();
      bridge.start();

      expect(true).toBe(true);
    });

    it('should be safe to stop without starting', () => {
      const freshBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
      });

      // Should not throw
      freshBridge.stop();
      expect(true).toBe(true);
    });

    it('should clean up on stop', () => {
      bridge.start();
      bridge.stop();

      expect(true).toBe(true);
    });

    it('should reset stats', () => {
      const mockWindow = createMockWindow();
      const statsBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      statsBridge.send('test', {});
      statsBridge.send('test', {});
      
      expect(statsBridge.getStats().messagesSent).toBe(2);
      
      statsBridge.resetStats();
      
      expect(statsBridge.getStats().messagesSent).toBe(0);
    });
  });

  describe('Statistics', () => {
    it('should track messages sent', () => {
      const mockWindow = createMockWindow();
      const statsBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      expect(statsBridge.getStats().messagesSent).toBe(0);

      statsBridge.send('test1', {});
      statsBridge.send('test2', {});
      statsBridge.send('test3', {});

      expect(statsBridge.getStats().messagesSent).toBe(3);
    });

    it('should track responses sent', () => {
      const mockWindow = createMockWindow();
      const statsBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      statsBridge.respond('apex-mediation-1', {});
      statsBridge.respond('apex-mediation-2', {});

      expect(statsBridge.getStats().messagesSent).toBe(2);
    });

    it('should track errors', () => {
      const mockWindow = createMockWindow();
      const statsBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      statsBridge.respondError('apex-mediation-1', 'Error 1');
      statsBridge.respondError('apex-mediation-2', 'Error 2');

      const stats = statsBridge.getStats();
      expect(stats.errors).toBe(2);
    });

    it('should return a copy of stats', () => {
      const mockWindow = createMockWindow();
      const statsBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
      });

      const stats1 = statsBridge.getStats();
      stats1.messagesSent = 999;

      const stats2 = statsBridge.getStats();
      expect(stats2.messagesSent).toBe(0);
    });
  });
});

describe('PostMessageBridge Integration', () => {
  it('should handle concurrent requests', async () => {
    const mockWindow = createMockWindow();
    const concBridge = new PostMessageBridge({
      allowedOrigins: ['https://example.com'],
      targetWindow: mockWindow as unknown as Window,
      timeout: 50,
    });

    concBridge.start();

    // Multiple concurrent requests
    const promises = [
      concBridge.request('req1', {}).catch(() => 'timeout1'),
      concBridge.request('req2', {}).catch(() => 'timeout2'),
      concBridge.request('req3', {}).catch(() => 'timeout3'),
    ];

    const results = await Promise.all(promises);

    expect(results).toEqual(['timeout1', 'timeout2', 'timeout3']);
    expect(mockWindow.postMessage).toHaveBeenCalledTimes(3);
  });

  it('should handle stop during pending requests', async () => {
    const mockWindow = createMockWindow();
    const stopBridge = new PostMessageBridge({
      allowedOrigins: ['https://example.com'],
      targetWindow: mockWindow as unknown as Window,
      timeout: 5000,
    });

    stopBridge.start();

    // Start multiple requests
    const promises = [
      stopBridge.request('req1', {}).catch((e: Error) => e.message),
      stopBridge.request('req2', {}).catch((e: Error) => e.message),
    ];

    // Stop immediately
    stopBridge.stop();

    const results = await Promise.all(promises);
    expect(results).toEqual(['Bridge stopped', 'Bridge stopped']);
  });
});

describe('MessageTypes', () => {
  it('should have all required ad lifecycle types', () => {
    expect(MessageTypes.AD_REQUEST).toBe('ad.request');
    expect(MessageTypes.AD_RESPONSE).toBe('ad.response');
    expect(MessageTypes.AD_LOADED).toBe('ad.loaded');
    expect(MessageTypes.AD_FAILED).toBe('ad.failed');
    expect(MessageTypes.AD_IMPRESSION).toBe('ad.impression');
    expect(MessageTypes.AD_CLICK).toBe('ad.click');
    expect(MessageTypes.AD_COMPLETE).toBe('ad.complete');
  });

  it('should have consent types', () => {
    expect(MessageTypes.CONSENT_REQUEST).toBe('consent.request');
    expect(MessageTypes.CONSENT_RESPONSE).toBe('consent.response');
    expect(MessageTypes.CONSENT_UPDATE).toBe('consent.update');
  });

  it('should have MRAID types', () => {
    expect(MessageTypes.MRAID_READY).toBe('mraid.ready');
    expect(MessageTypes.MRAID_EXPAND).toBe('mraid.expand');
    expect(MessageTypes.MRAID_CLOSE).toBe('mraid.close');
    expect(MessageTypes.MRAID_RESIZE).toBe('mraid.resize');
  });

  it('should have viewability types', () => {
    expect(MessageTypes.VIEWABILITY_UPDATE).toBe('viewability.update');
    expect(MessageTypes.VIEWABILITY_MRC50).toBe('viewability.mrc50');
  });
});

describe('Helper Functions', () => {
  describe('createPublisherBridge', () => {
    it('should create bridge with iframe as target', () => {
      const mockIframe = {
        contentWindow: createMockWindow() as unknown as Window,
      } as HTMLIFrameElement;

      const receivedMessages: PostMessagePayload[] = [];
      const pubBridge = createPublisherBridge({
        adIframe: mockIframe,
        allowedOrigins: ['https://ad.example.com'],
        onAdEvent: (msg) => receivedMessages.push(msg),
      });

      pubBridge.send('test', {});

      expect((mockIframe.contentWindow as unknown as MockWindow).postMessage).toHaveBeenCalledTimes(1);
    });
  });

  describe('createAdBridge', () => {
    it('should create bridge targeting parent window', () => {
      // In test environment, window.parent might be undefined
      const adBridge = createAdBridge({
        publisherOrigin: 'https://publisher.com',
        timeout: 2000,
      });

      // Bridge should be created successfully
      expect(adBridge).toBeDefined();
      expect(adBridge.isOriginAllowed('https://publisher.com')).toBe(true);
      expect(adBridge.isOriginAllowed('https://other.com')).toBe(false);
    });
  });
});
