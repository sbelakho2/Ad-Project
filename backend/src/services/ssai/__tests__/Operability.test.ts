/**
 * SSAI Operability Module Unit Tests
 */

import { SSAIHealthCheck } from '../operability/SSAIHealthCheck';
import { SSAIMetrics } from '../operability/SSAIMetrics';
import { SSAIDebugger } from '../operability/SSAIDebugger';
import { SSAILogger } from '../operability/SSAILogger';

describe('SSAIHealthCheck', () => {
  let healthCheck: SSAIHealthCheck;

  beforeEach(() => {
    healthCheck = new SSAIHealthCheck();
  });

  describe('check', () => {
    it('should return healthy when all components are ok', async () => {
      const result = await healthCheck.check();

      expect(result.healthy).toBeDefined();
      expect(result.components).toBeDefined();
    });

    it('should report component status', async () => {
      const result = await healthCheck.check();

      expect(result.components.session).toBeDefined();
    });
  });

  describe('checkDependency', () => {
    it('should check individual dependency', async () => {
      const result = await healthCheck.checkDependency('redis');

      expect(result.name).toBe('redis');
      expect(result.status).toBeDefined();
    });
  });

  describe('liveness', () => {
    it('should return liveness status', () => {
      const result = healthCheck.liveness();

      expect(result.alive).toBe(true);
    });
  });

  describe('readiness', () => {
    it('should check readiness', async () => {
      const result = await healthCheck.readiness();

      expect(result.ready).toBeDefined();
    });
  });
});

describe('SSAIMetrics', () => {
  let metrics: SSAIMetrics;

  beforeEach(() => {
    metrics = new SSAIMetrics();
  });

  describe('incrementCounter', () => {
    it('should increment counter', () => {
      metrics.incrementCounter('ssai_sessions_created', { protocol: 'hls' });
      metrics.incrementCounter('ssai_sessions_created', { protocol: 'hls' });

      const value = metrics.getCounter('ssai_sessions_created', { protocol: 'hls' });

      expect(value).toBe(2);
    });
  });

  describe('recordHistogram', () => {
    it('should record histogram values', () => {
      metrics.recordHistogram('ssai_manifest_latency_ms', 100);
      metrics.recordHistogram('ssai_manifest_latency_ms', 150);

      const stats = metrics.getHistogramStats('ssai_manifest_latency_ms');

      expect(stats.count).toBe(2);
      expect(stats.avg).toBe(125);
    });
  });

  describe('gauge', () => {
    it('should set gauge value', () => {
      metrics.setGauge('ssai_active_sessions', 42);

      const value = metrics.getGauge('ssai_active_sessions');

      expect(value).toBe(42);
    });
  });

  describe('SLI metrics', () => {
    it('should track fill rate', () => {
      metrics.recordFillRate('partner-1', { filled: 27, requested: 30 });

      const fillRate = metrics.getFillRate('partner-1');

      expect(fillRate).toBe(0.9);
    });

    it('should track decision latency', () => {
      metrics.recordDecisionLatency('partner-1', 250);

      const p99 = metrics.getDecisionLatencyP99('partner-1');

      expect(p99).toBeDefined();
    });
  });

  describe('prometheus export', () => {
    it('should export metrics in prometheus format', () => {
      metrics.incrementCounter('ssai_test_counter', { label: 'value' });

      const output = metrics.exportPrometheus();

      expect(output).toContain('ssai_test_counter');
    });
  });
});

describe('SSAIDebugger', () => {
  let debugger_: SSAIDebugger;

  beforeEach(() => {
    debugger_ = new SSAIDebugger();
  });

  describe('explainPod', () => {
    it('should explain pod decisions', async () => {
      const explanation = await debugger_.explainPod('pod-1');

      expect(explanation).toBeDefined();
      expect(explanation.podId).toBe('pod-1');
    });
  });

  describe('explainNoFill', () => {
    it('should explain no-fill reasons', async () => {
      const explanation = await debugger_.explainNoFill('request-1');

      expect(explanation).toBeDefined();
      expect(explanation.reasons).toBeDefined();
    });
  });

  describe('getDecisionTrace', () => {
    it('should return decision trace', async () => {
      const trace = await debugger_.getDecisionTrace('session-1', 'pod-1');

      expect(trace).toBeDefined();
    });
  });
});

describe('SSAILogger', () => {
  let logger: SSAILogger;
  let logOutput: string[];

  beforeEach(() => {
    logOutput = [];
    logger = new SSAILogger({
      level: 'debug',
      onLog: (msg) => logOutput.push(msg),
    });
  });

  describe('logging levels', () => {
    it('should log at info level', () => {
      logger.info('Test info message', { sessionId: 'sess-1' });

      expect(logOutput.length).toBe(1);
      expect(logOutput[0]).toContain('info');
    });

    it('should log at debug level', () => {
      logger.debug('Test debug message');

      expect(logOutput.length).toBe(1);
    });

    it('should log at error level', () => {
      logger.error('Test error', new Error('test'));

      expect(logOutput.length).toBe(1);
      expect(logOutput[0]).toContain('error');
    });
  });

  describe('context', () => {
    it('should include context in logs', () => {
      logger.info('Message', {
        sessionId: 'sess-1',
        podId: 'pod-1',
      });

      expect(logOutput[0]).toContain('sess-1');
    });
  });

  describe('child logger', () => {
    it('should create child with additional context', () => {
      const child = logger.child({ sessionId: 'sess-1' });
      child.info('Child message');

      expect(logOutput[0]).toContain('sess-1');
    });
  });
});
