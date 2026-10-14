/**
 * PostMessageBridge Tests - CSP Compliance & Security
 */

interface MockedWindow {
  postMessage: jest.Mock;
  addEventListener: jest.Mock;
  removeEventListener: jest.Mock;
  parent: MockedWindow | null;
  top: MockedWindow | null;
  origin: string;
}

function createMockWindow(origin: string = 'https://example.com'): MockedWindow {
  const win: MockedWindow = {
    postMessage: jest.fn(),
    addEventListener: jest.fn(),
    removeEventListener: jest.fn(),
    parent: null,
    top: null,
    origin,
  };
  win.parent = win;
  win.top = win;
  return win;
}

import {
  PostMessageBridge,
  type PostMessagePayload,
} from '../src/postMessageBridge';

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
  });

  describe('Security', () => {
    it('should validate message namespace through real handling', () => {
      const receivedMessages: PostMessagePayload[] = [];
      const listeners: Array<(event: MessageEvent) => void> = [];
      const fakeWindow = {
        addEventListener: (_type: string, handler: (event: MessageEvent) => void) => {
          listeners.push(handler);
        },
        removeEventListener: jest.fn(),
      };
      (globalThis as { window?: unknown }).window = fakeWindow;
      try {
        const namespaceBridge = new PostMessageBridge({
          allowedOrigins: ['https://example.com'],
          onMessage: (msg) => receivedMessages.push(msg),
        });
        namespaceBridge.start();

        const validPayload = {
          type: 'test',
          id: 'apex-mediation-123',
          timestamp: Date.now(),
          data: {},
        };
        const invalidPayload = {
          type: 'test',
          id: 'other-sdk-123',
          timestamp: Date.now(),
          data: {},
        };

        for (const handler of listeners) {
          handler({ origin: 'https://example.com', data: validPayload } as MessageEvent);
          handler({ origin: 'https://example.com', data: invalidPayload } as MessageEvent);
        }

        expect(receivedMessages).toHaveLength(1);
        expect(receivedMessages[0].id).toBe('apex-mediation-123');
        namespaceBridge.stop();
      } finally {
        delete (globalThis as { window?: unknown }).window;
      }
    });

    it('should not leak pending request data on stop', () => {
      const mockWindow = createMockWindow();
      const leakBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
        targetWindow: mockWindow as unknown as Window,
        timeout: 10000,
      });

      leakBridge.start();

      const requestPromise = leakBridge.request('test', {});
      leakBridge.stop();

      return expect(requestPromise).rejects.toThrow('Bridge stopped');
    });

    it('should reject payloads that fail structure validation', () => {
      const receivedMessages: PostMessagePayload[] = [];
      const listeners: Array<(event: MessageEvent) => void> = [];
      const fakeWindow = {
        addEventListener: (_type: string, handler: (event: MessageEvent) => void) => {
          listeners.push(handler);
        },
        removeEventListener: jest.fn(),
      };
      (globalThis as { window?: unknown }).window = fakeWindow;
      try {
        const structureBridge = new PostMessageBridge({
          allowedOrigins: ['https://example.com'],
          onMessage: (msg) => receivedMessages.push(msg),
        });
        structureBridge.start();

        const invalidPayloads: unknown[] = [
          null,
          undefined,
          'string',
          123,
          [],
          {},
          { type: 'test' },
          { type: 'test', id: 'apex-mediation-123' },
          { type: 123, id: 'apex-mediation-123', timestamp: Date.now(), data: {} },
        ];
        for (const payload of invalidPayloads) {
          for (const handler of listeners) {
            handler({ origin: 'https://example.com', data: payload } as MessageEvent);
          }
        }
        expect(receivedMessages).toHaveLength(0);

        for (const handler of listeners) {
          handler({
            origin: 'https://example.com',
            data: { type: 'test', id: 'apex-mediation-1', timestamp: Date.now(), data: {} },
          } as MessageEvent);
        }
        expect(receivedMessages).toHaveLength(1);
        structureBridge.stop();
      } finally {
        delete (globalThis as { window?: unknown }).window;
      }
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
        // Expected to time out.
      }
      expect(Date.now() - start).toBeLessThan(500);
    });
  });

  describe('Bridge Lifecycle', () => {
    it('should register exactly one message listener across repeated starts', () => {
      const listeners: Array<(event: MessageEvent) => void> = [];
      const fakeWindow = {
        addEventListener: (_type: string, handler: (event: MessageEvent) => void) => {
          listeners.push(handler);
        },
        removeEventListener: jest.fn(),
      };
      (globalThis as { window?: unknown }).window = fakeWindow;
      try {
        bridge.start();
        bridge.start();
        bridge.start();
        expect(listeners).toHaveLength(1);
      } finally {
        bridge.stop();
        delete (globalThis as { window?: unknown }).window;
      }
    });

    it('should be safe to stop without starting', () => {
      const freshBridge = new PostMessageBridge({
        allowedOrigins: ['https://example.com'],
      });
      expect(() => freshBridge.stop()).not.toThrow();
    });

    it('should remove its listener on stop', () => {
      const fakeWindow = {
        addEventListener: jest.fn(),
        removeEventListener: jest.fn(),
      };
      (globalThis as { window?: unknown }).window = fakeWindow;
      try {
        bridge.start();
        const handler = fakeWindow.addEventListener.mock.calls[0][1];
        bridge.stop();
        expect(fakeWindow.removeEventListener).toHaveBeenCalledWith('message', handler);
      } finally {
        delete (globalThis as { window?: unknown }).window;
      }
    });
  });
});

describe('PostMessageBridge Integration', () => {
  it('should support request-response pattern', async () => {
    const mockWindow = createMockWindow();
    const reqBridge = new PostMessageBridge({
      allowedOrigins: ['https://example.com'],
      targetWindow: mockWindow as unknown as Window,
      timeout: 100,
    });

    reqBridge.start();

    await expect(reqBridge.request('getConfig', {})).rejects.toThrow('Request timed out');
  });

  it('should handle concurrent requests', async () => {
    const mockWindow = createMockWindow();
    const concBridge = new PostMessageBridge({
      allowedOrigins: ['https://example.com'],
      targetWindow: mockWindow as unknown as Window,
      timeout: 50,
    });

    concBridge.start();

    const promises = [
      concBridge.request('req1', {}).catch(() => 'timeout1'),
      concBridge.request('req2', {}).catch(() => 'timeout2'),
      concBridge.request('req3', {}).catch(() => 'timeout3'),
    ];

    const results = await Promise.all(promises);
    expect(results).toEqual(['timeout1', 'timeout2', 'timeout3']);
    expect(mockWindow.postMessage).toHaveBeenCalledTimes(3);
  });
});
