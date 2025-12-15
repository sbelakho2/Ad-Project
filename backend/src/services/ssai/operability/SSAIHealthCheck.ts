/**
 * SSAI Health Check
 * Provides health check endpoints for SSAI services
 */

export interface HealthStatus {
  healthy: boolean;
  components: Record<string, ComponentStatus>;
  timestamp: number;
}

export interface ComponentStatus {
  status: 'healthy' | 'degraded' | 'unhealthy';
  latencyMs?: number;
  lastCheck?: number;
  message?: string;
}

export interface LivenessResult {
  alive: boolean;
  uptime: number;
}

export interface ReadinessResult {
  ready: boolean;
  checks: Record<string, boolean>;
}

export interface DependencyCheckResult {
  name: string;
  status: 'healthy' | 'degraded' | 'unhealthy';
  latencyMs?: number;
  error?: string;
}

export class SSAIHealthCheck {
  private startTime: number;
  private componentStatuses: Map<string, ComponentStatus>;

  constructor() {
    this.startTime = Date.now();
    this.componentStatuses = new Map();
  }

  /**
   * Full health check
   */
  async check(): Promise<HealthStatus> {
    const checks = await Promise.all([
      this.checkDependency('redis'),
      this.checkDependency('database'),
      this.checkDependency('partners'),
    ]);

    const components: Record<string, ComponentStatus> = {};
    let allHealthy = true;

    for (const check of checks) {
      components[check.name] = {
        status: check.status,
        latencyMs: check.latencyMs,
        lastCheck: Date.now(),
        message: check.error,
      };
      if (check.status !== 'healthy') {
        allHealthy = false;
      }
    }

    // Add session component
    components.session = {
      status: 'healthy',
      lastCheck: Date.now(),
    };

    return {
      healthy: allHealthy,
      components,
      timestamp: Date.now(),
    };
  }

  /**
   * Check specific dependency
   */
  async checkDependency(name: string): Promise<DependencyCheckResult> {
    const start = Date.now();
    
    // Simulated checks - in production, these would be real health checks
    try {
      switch (name) {
        case 'redis':
          // Check Redis connectivity
          await this.simulateCheck(10);
          break;
        case 'database':
          // Check database connectivity
          await this.simulateCheck(20);
          break;
        case 'partners':
          // Check partner connectivity
          await this.simulateCheck(50);
          break;
        default:
          await this.simulateCheck(5);
      }

      return {
        name,
        status: 'healthy',
        latencyMs: Date.now() - start,
      };
    } catch (error) {
      return {
        name,
        status: 'unhealthy',
        latencyMs: Date.now() - start,
        error: error instanceof Error ? error.message : 'Unknown error',
      };
    }
  }

  /**
   * Liveness probe - is the service running?
   */
  liveness(): LivenessResult {
    return {
      alive: true,
      uptime: Date.now() - this.startTime,
    };
  }

  /**
   * Readiness probe - is the service ready to handle requests?
   */
  async readiness(): Promise<ReadinessResult> {
    const checks: Record<string, boolean> = {};
    
    // Check critical dependencies
    const redis = await this.checkDependency('redis');
    checks.redis = redis.status === 'healthy';

    const allReady = Object.values(checks).every(v => v);

    return {
      ready: allReady,
      checks,
    };
  }

  /**
   * Set component status
   */
  setComponentStatus(name: string, status: ComponentStatus): void {
    this.componentStatuses.set(name, status);
  }

  private simulateCheck(delayMs: number): Promise<void> {
    return new Promise(resolve => setTimeout(resolve, delayMs));
  }
}

export default SSAIHealthCheck;
