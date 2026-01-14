/**
 * Lightweight verification that VRA pilot‑gate gauges are usable without coupling to the global registry.
 *
 * Skipped by default to keep smoke runs fast and avoid cross‑suite coupling.
 * Enable with: ENABLE_VRA_GAUGES_TEST=1 npm run test:backend
 */

describe('VRA gauges — registration and set()', () => {
  beforeAll(() => {
    // Disable default prom-client collectors for this test to keep isolation
    process.env.PROMETHEUS_COLLECT_DEFAULTS = '0';
  });

  it('allows set() on coverage/variance gauges with labels', async () => {
    jest.isolateModules(() => {
      // Import after setting env so defaults are not collected
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const prom = require('../utils/prometheus') as typeof import('../utils/prometheus');
      expect(prom.vraCoveragePercent).toBeDefined();
      expect(prom.vraVariancePercent).toBeDefined();

      // Best-effort set
      expect(() => prom.vraCoveragePercent.set({ scope: 'pilot' }, 100)).not.toThrow();
      expect(() => prom.vraVariancePercent.set({ scope: 'pilot' }, 0)).not.toThrow();
    });
  });
});
