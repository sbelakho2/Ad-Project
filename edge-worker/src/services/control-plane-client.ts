/**
 * CONTROL PLANE CLIENT
 * 
 * Client for communicating with the Control Plane API from the Edge Worker.
 * Handles:
 *   - Authentication via shared secret
 *   - Sync results back to Control Plane
 *   - Fetch job configurations
 *   - Report health/status
 *   - Circuit breaker pattern for resilience
 */

import { Logger } from 'winston';
import { CircuitBreaker, createCircuitBreaker, CircuitOpenError } from './circuit-breaker';

// ============================================================================
// TYPES
// ============================================================================

export interface ControlPlaneConfig {
  baseUrl: string;
  secret: string;
  timeoutMs?: number;
  retries?: number;
}

export interface SyncRequest {
  type: 'classification' | 'icebreaker' | 'reputation' | 'scrape';
  data: unknown;
  metadata?: Record<string, unknown>;
}

export interface JobConfig {
  id: string;
  type: string;
  parameters: Record<string, unknown>;
  priority: number;
  createdAt: string;
}

export interface HealthReport {
  workerId: string;
  status: 'healthy' | 'degraded' | 'unhealthy';
  features: string[];
  models: {
    name: string;
    loaded: boolean;
    lastUsed?: string;
  }[];
  metrics: {
    requestsProcessed: number;
    averageLatencyMs: number;
    errorRate: number;
  };
  uptime: number;
}

// ============================================================================
// CONTROL PLANE CLIENT
// ============================================================================

export class ControlPlaneClient {
  private baseUrl: string;
  private secret: string;
  private timeoutMs: number;
  private retries: number;
  private logger?: Logger;
  private circuitBreaker: CircuitBreaker;
  
  constructor(config: ControlPlaneConfig, logger?: Logger) {
    this.baseUrl = config.baseUrl.replace(/\/$/, ''); // Remove trailing slash
    this.secret = config.secret;
    this.timeoutMs = config.timeoutMs ?? 10000;
    this.retries = config.retries ?? 3;
    this.logger = logger;
    
    // Initialize circuit breaker for Control Plane requests
    this.circuitBreaker = createCircuitBreaker('control-plane', logger, {
      failureThreshold: 5,      // Open after 5 failures
      successThreshold: 2,      // Close after 2 successes in half-open
      resetTimeMs: 30000,       // Try half-open after 30 seconds
      windowSizeMs: 60000,      // Count failures in 1 minute window
    });
  }
  
  /**
   * Get circuit breaker stats for health reporting
   */
  getCircuitBreakerStats() {
    return this.circuitBreaker.getStats();
  }
  
  /**
   * Check if circuit is allowing requests
   */
  isCircuitClosed(): boolean {
    return this.circuitBreaker.canExecute();
  }
  
  private async request<T>(
    method: 'GET' | 'POST' | 'PUT' | 'DELETE',
    path: string,
    body?: unknown
  ): Promise<T> {
    // Check circuit breaker before making request
    if (!this.circuitBreaker.canExecute()) {
      this.logger?.warn(`Circuit breaker open - skipping request to ${path}`);
      throw new CircuitOpenError('Control Plane circuit breaker is open - service unavailable');
    }
    
    const url = `${this.baseUrl}${path}`;
    let lastError: Error | null = null;
    
    for (let attempt = 0; attempt < this.retries; attempt++) {
      try {
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), this.timeoutMs);
        
        const response = await fetch(url, {
          method,
          headers: {
            'Content-Type': 'application/json',
            'X-Edge-Worker-Secret': this.secret,
          },
          body: body ? JSON.stringify(body) : undefined,
          signal: controller.signal,
        });
        
        clearTimeout(timeout);
        
        if (!response.ok) {
          const errorText = await response.text();
          throw new Error(`HTTP ${response.status}: ${errorText}`);
        }
        
        // Success - record it and return
        this.circuitBreaker.recordSuccess();
        return response.json() as Promise<T>;
      } catch (err) {
        lastError = err as Error;
        
        if ((err as Error).name === 'AbortError') {
          this.logger?.warn(`Request to ${path} timed out (attempt ${attempt + 1})`);
        } else {
          this.logger?.warn(`Request to ${path} failed (attempt ${attempt + 1}): ${(err as Error).message}`);
        }
        
        // Exponential backoff with jitter to prevent thundering herd
        if (attempt < this.retries - 1) {
          const baseDelay = Math.pow(2, attempt) * 1000;
          const jitter = Math.random() * 500; // 0-500ms jitter
          await new Promise(resolve => setTimeout(resolve, baseDelay + jitter));
        }
      }
    }
    
    // All retries failed - record failure for circuit breaker
    this.circuitBreaker.recordFailure();
    throw lastError || new Error('Request failed');
  }
  
  // ==========================================================================
  // SYNC METHODS
  // ==========================================================================
  
  /**
   * Sync classification result back to Control Plane
   */
  async syncClassification(data: {
    messageId: string;
    category: string;
    confidence: number;
    requiresReview: boolean;
    processingMode: string;
    latencyMs: number;
  }): Promise<void> {
    await this.request('POST', '/api/edge/sync/classification', data);
  }
  
  /**
   * Sync icebreaker generation result
   */
  async syncIcebreaker(data: {
    leadId: string;
    message: string;
    genre: string;
    confidence: number;
    latencyMs: number;
  }): Promise<void> {
    await this.request('POST', '/api/edge/sync/icebreaker', data);
  }
  
  /**
   * Sync reputation analysis result
   */
  async syncReputation(data: {
    domain: string;
    trustScore: number;
    throttleRecommendation: string;
    analysis: string[];
  }): Promise<void> {
    await this.request('POST', '/api/edge/sync/reputation', data);
  }
  
  /**
   * Sync scrape result
   */
  async syncScrape(data: {
    url: string;
    type: 'linkedin' | 'company';
    success: boolean;
    data?: unknown;
    error?: string;
  }): Promise<void> {
    await this.request('POST', '/api/edge/sync/scrape', data);
  }
  
  // ==========================================================================
  // JOB METHODS
  // ==========================================================================
  
  /**
   * Fetch pending jobs from Control Plane
   */
  async fetchJobs(limit = 10): Promise<JobConfig[]> {
    return this.request<JobConfig[]>('GET', `/api/edge/jobs?limit=${limit}`);
  }
  
  /**
   * Mark job as completed
   */
  async completeJob(jobId: string, result: unknown): Promise<void> {
    await this.request('POST', `/api/edge/jobs/${jobId}/complete`, { result });
  }
  
  /**
   * Mark job as failed
   */
  async failJob(jobId: string, error: string): Promise<void> {
    await this.request('POST', `/api/edge/jobs/${jobId}/fail`, { error });
  }
  
  // ==========================================================================
  // HEALTH REPORTING
  // ==========================================================================
  
  /**
   * Report health status to Control Plane
   */
  async reportHealth(report: HealthReport): Promise<void> {
    await this.request('POST', '/api/edge/health', report);
  }
  
  // ==========================================================================
  // CONFIGURATION
  // ==========================================================================
  
  /**
   * Fetch worker configuration from Control Plane
   */
  async fetchConfig(): Promise<{
    confidenceThreshold: number;
    rateLimitRpm: number;
    enabledFeatures: string[];
    modelOverrides?: Record<string, string>;
  }> {
    return this.request('GET', '/api/edge/config');
  }
  
  // ==========================================================================
  // LEAD DATA
  // ==========================================================================
  
  /**
   * Fetch lead data for icebreaker generation
   */
  async fetchLead(leadId: string): Promise<{
    id: string;
    name: string;
    company: string;
    title?: string;
    industry?: string;
    companySize?: string;
    linkedinUrl?: string;
    website?: string;
    recentNews?: string[];
    customSignals?: Record<string, unknown>;
  }> {
    return this.request('GET', `/api/edge/leads/${leadId}`);
  }
  
  /**
   * Fetch message for classification
   */
  async fetchMessage(messageId: string): Promise<{
    id: string;
    subject: string;
    body: string;
    from: string;
    receivedAt: string;
  }> {
    return this.request('GET', `/api/edge/messages/${messageId}`);
  }
  
  // ==========================================================================
  // UTILITY
  // ==========================================================================
  
  /**
   * Check connectivity to Control Plane
   */
  async ping(): Promise<{ ok: boolean; latencyMs: number }> {
    const start = Date.now();
    
    try {
      await this.request('GET', '/api/health');
      return { ok: true, latencyMs: Date.now() - start };
    } catch {
      return { ok: false, latencyMs: Date.now() - start };
    }
  }
}
