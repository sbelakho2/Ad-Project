import { CircuitBreaker, CircuitOpenError, type CircuitState } from '../src/services/circuit-breaker';

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('CircuitBreaker', () => {
  it('starts closed and allows execution', () => {
    const cb = new CircuitBreaker({ name: 't1' });
    const state: CircuitState = cb.getState();
    expect(state).toBe('closed');
    expect(cb.canExecute()).toBe(true);
  });

  it('opens after failureThreshold failures inside the window', () => {
    const cb = new CircuitBreaker({ name: 't2', failureThreshold: 3, windowSizeMs: 60_000 });
    cb.recordFailure();
    cb.recordFailure();
    expect(cb.getState()).toBe('closed');
    cb.recordFailure();
    expect(cb.getState()).toBe('open');
    expect(cb.canExecute()).toBe(false);
  });

  it('execute() fails fast with CircuitOpenError while open', async () => {
    const cb = new CircuitBreaker({ name: 't3', failureThreshold: 1 });
    cb.recordFailure();
    await expect(cb.execute(async () => 'never')).rejects.toBeInstanceOf(CircuitOpenError);
  });

  it('reopens half-open circuit on any failure', () => {
    const cb = new CircuitBreaker({ name: 't4', failureThreshold: 1 });
    cb.recordFailure();
    expect(cb.getState()).toBe('open');
    cb.forceState('half-open');
    cb.recordFailure();
    expect(cb.getState()).toBe('open');
  });

  it('closes half-open circuit after successThreshold successes', () => {
    const cb = new CircuitBreaker({ name: 't5', failureThreshold: 1, successThreshold: 2 });
    cb.recordFailure();
    cb.forceState('half-open');
    cb.recordSuccess();
    expect(cb.getState()).toBe('half-open');
    cb.recordSuccess();
    expect(cb.getState()).toBe('closed');
  });

  it('transitions open -> half-open after resetTimeMs (real timer)', async () => {
    const cb = new CircuitBreaker({ name: 't6', failureThreshold: 1, resetTimeMs: 40 });
    cb.recordFailure();
    expect(cb.getState()).toBe('open');
    await sleep(70);
    expect(cb.getState()).toBe('half-open');
    expect(cb.canExecute()).toBe(true);
  });

  it('forget window: stale failures outside windowSizeMs do not open the circuit', async () => {
    const cb = new CircuitBreaker({ name: 't7', failureThreshold: 2, windowSizeMs: 30 });
    cb.recordFailure();
    await sleep(60);
    cb.recordFailure();
    expect(cb.getState()).toBe('closed');
  });

  it('execute() records success and propagates the result', async () => {
    const cb = new CircuitBreaker({ name: 't8' });
    const result = await cb.execute(async () => 42);
    expect(result).toBe(42);
    const stats = cb.getStats();
    expect(stats.state).toBe('closed');
    expect(stats.lastSuccess).not.toBeNull();
  });

  it('execute() records failure and rethrows the original error', async () => {
    const cb = new CircuitBreaker({ name: 't9', failureThreshold: 5 });
    const boom = new Error('boom');
    await expect(cb.execute(async () => { throw boom; })).rejects.toBe(boom);
    expect(cb.getStats().failures).toBe(1);
  });

  it('forceState(closed) clears failure counters', () => {
    const cb = new CircuitBreaker({ name: 't10', failureThreshold: 2 });
    cb.recordFailure();
    cb.recordFailure();
    expect(cb.getState()).toBe('open');
    cb.forceState('closed');
    expect(cb.getState()).toBe('closed');
    expect(cb.getStats().failures).toBe(0);
  });
});
