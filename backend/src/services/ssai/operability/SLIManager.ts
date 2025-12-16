/**
 * SSAI SLI Metrics Manager (test-facing)
 *
 * The SSAI operability tests expect a lightweight interface:
 * - recordSLI(name, value)
 * - getSLI(name)
 * - getErrorBudget()
 * - onAlert(cb)
 */

export interface SLIManagerConfig {
  /** SLO target as a fraction (e.g. 0.999 for 99.9%). */
  sloTarget: number;

  /** Rolling evaluation window in milliseconds. */
  evaluationWindowMs: number;

  /** Alert when remaining error budget dips below this percent. */
  alertRemainingPercent?: number;
}

type DataPoint = { ts: number; value: number };

export interface SLIStats {
  current: number;
  average: number;
  p99: number;
  samples: number;
}

export interface ErrorBudgetStatus {
  remaining: number;
  percentUsed: number;
  budgetPercent: number;
}

export class SLIManager {
  private readonly config: Required<SLIManagerConfig>;
  private readonly data: Map<string, DataPoint[]> = new Map();
  private alertCallbacks: Array<(budget: ErrorBudgetStatus) => void> = [];
  private alertFired = false;

  constructor(config: SLIManagerConfig) {
    this.config = {
      ...config,
      alertRemainingPercent: config.alertRemainingPercent ?? 20,
    };
  }

  recordSLI(name: string, value: number): void {
    const now = Date.now();
    const list = this.data.get(name) ?? [];
    list.push({ ts: now, value });
    this.data.set(name, list);
    this.prune(now);

    if (name === 'error_rate') {
      const budget = this.getErrorBudget();
      if (!this.alertFired && budget.remaining <= this.config.alertRemainingPercent) {
        this.alertFired = true;
        for (const cb of this.alertCallbacks) cb(budget);
      }
    }
  }

  getSLI(name: string): SLIStats {
    const list = this.getWindowed(name);
    if (list.length === 0) {
      return { current: 0, average: 0, p99: 0, samples: 0 };
    }

    const current = list[list.length - 1].value;
    const average = list.reduce((sum, p) => sum + p.value, 0) / list.length;

    const sorted = list.map((p) => p.value).sort((a, b) => a - b);
    const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * 0.99));
    const p99 = sorted[idx] ?? current;

    return { current, average, p99, samples: list.length };
  }

  getErrorBudget(): ErrorBudgetStatus {
    const budgetPercent = Math.max(0, (1 - this.config.sloTarget) * 100);

    const errorRateSamples = this.getWindowed('error_rate');
    const avgErrorRatePercent =
      errorRateSamples.length === 0
        ? 0
        : errorRateSamples.reduce((sum, p) => sum + p.value, 0) / errorRateSamples.length;

    const percentUsed =
      budgetPercent <= 0 ? 0 : Math.min(100, (avgErrorRatePercent / budgetPercent) * 100);
    const remaining = Math.max(0, 100 - percentUsed);

    return { remaining, percentUsed, budgetPercent };
  }

  onAlert(callback: (budget: ErrorBudgetStatus) => void): void {
    this.alertCallbacks.push(callback);
  }

  private prune(now: number): void {
    const cutoff = now - this.config.evaluationWindowMs;
    for (const [name, list] of this.data) {
      let idx = 0;
      while (idx < list.length && list[idx].ts < cutoff) idx++;
      if (idx > 0) this.data.set(name, list.slice(idx));
    }
  }

  private getWindowed(name: string): DataPoint[] {
    this.prune(Date.now());
    return this.data.get(name) ?? [];
  }
}
