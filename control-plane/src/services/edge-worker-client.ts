/**
 * EDGE WORKER CLIENT (for Control Plane)
 * 
 * This client runs on the Control Plane (App Server) and proxies requests
 * to the Edge Worker (Test Server).
 * 
 * Usage:
 *   import { EdgeWorkerClient } from './edge-worker-client';
 *   
 *   const client = new EdgeWorkerClient({
 *     baseUrl: process.env.EDGE_WORKER_URL,
 *     secret: process.env.EDGE_WORKER_SECRET,
 *   });
 *   
 *   const icebreaker = await client.generateIcebreaker(leadContext);
 *   const classification = await client.classifyMessage(subject, body);
 */

// ============================================================================
// TYPES
// ============================================================================

export interface EdgeWorkerConfig {
  baseUrl: string;
  secret: string;
  timeoutMs?: number;
  retries?: number;
}

export interface LeadContext {
  name: string;
  company: string;
  title?: string;
  industry?: string;
  companySize?: string;
  recentNews?: string[];
  painPoints?: string[];
  linkedinUrl?: string;
  website?: string;
}

export interface IcebreakerResult {
  success: boolean;
  message?: string;
  genre?: string;
  confidence?: number;
  fallbackUsed?: boolean;
  latencyMs: number;
  error?: string;
}

export type MessageCategory = 
  | 'INTERESTED'
  | 'MEETING'
  | 'NOT_INTERESTED'
  | 'OOO'
  | 'BOUNCE'
  | 'UNSUBSCRIBE'
  | 'SPAM'
  | 'UNKNOWN';

export interface ClassificationResult {
  category: MessageCategory;
  confidence: number;
  requiresReview: boolean;
  processingMode: 'fast' | 'deep' | 'hybrid' | 'rules';
  latencyMs: number;
}

export type ThrottleLevel = 'NONE' | 'LIGHT' | 'MODERATE' | 'HEAVY' | 'PAUSE';

export interface ReputationResult {
  domain: string;
  trustScore: number;
  throttleRecommendation: ThrottleLevel;
  dmarcScore: number;
  spfScore: number;
  bounceScore: number;
  engagementScore: number;
  analysis: string[];
}

export interface LinkedInProfile {
  url: string;
  name: string;
  title?: string;
  company?: string;
  location?: string;
  connections?: string;
  about?: string;
  experience?: Array<{
    title: string;
    company: string;
    duration?: string;
  }>;
}

export interface CompanyInfo {
  url: string;
  name?: string;
  description?: string;
  emails?: string[];
  phones?: string[];
  socialLinks?: {
    linkedin?: string;
    twitter?: string;
    facebook?: string;
  };
}

export interface EdgeWorkerHealth {
  status: 'healthy' | 'degraded' | 'unhealthy';
  uptime: number;
  features: string[];
  models: {
    icebreaker: { loaded: boolean; lastUsed?: string };
    classifier: { loaded: boolean; lastUsed?: string };
  };
}

// ============================================================================
// EDGE WORKER CLIENT
// ============================================================================

export class EdgeWorkerClient {
  private baseUrl: string;
  private secret: string;
  private timeoutMs: number;
  private retries: number;
  private enabled: boolean;
  
  constructor(config: EdgeWorkerConfig) {
    this.baseUrl = config.baseUrl?.replace(/\/$/, '') || '';
    this.secret = config.secret || '';
    this.timeoutMs = config.timeoutMs ?? 30000;
    this.retries = config.retries ?? 2;
    this.enabled = !!(this.baseUrl && this.secret);
  }
  
  isEnabled(): boolean {
    return this.enabled;
  }
  
  private async request<T>(
    method: 'GET' | 'POST',
    path: string,
    body?: unknown
  ): Promise<T> {
    if (!this.enabled) {
      throw new Error('Edge Worker client not configured');
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
            'X-Edge-Secret': this.secret,
          },
          body: body ? JSON.stringify(body) : undefined,
          signal: controller.signal,
        });
        
        clearTimeout(timeout);
        
        if (!response.ok) {
          const errorText = await response.text();
          throw new Error(`HTTP ${response.status}: ${errorText}`);
        }
        
        return response.json() as Promise<T>;
      } catch (err) {
        lastError = err as Error;
        
        // Don't retry on abort or client errors
        if ((err as Error).name === 'AbortError') {
          throw new Error(`Edge Worker request timed out after ${this.timeoutMs}ms`);
        }
        
        // Exponential backoff
        if (attempt < this.retries - 1) {
          await new Promise(resolve => setTimeout(resolve, Math.pow(2, attempt) * 500));
        }
      }
    }
    
    throw lastError || new Error('Edge Worker request failed');
  }
  
  // ==========================================================================
  // ICEBREAKER
  // ==========================================================================
  
  /**
   * Generate an icebreaker message using the Edge Worker's LLM
   */
  async generateIcebreaker(
    lead: LeadContext,
    options?: {
      genre?: 'curious' | 'direct' | 'social_proof' | 'value_first' | 'pain_agitate';
      maxLength?: number;
    }
  ): Promise<IcebreakerResult> {
    const startTime = Date.now();
    
    try {
      const result = await this.request<{
        message: string;
        genre: string;
        confidence: number;
        fallbackUsed: boolean;
        latencyMs: number;
      }>('POST', '/icebreaker/generate', {
        lead,
        genre: options?.genre,
        maxLength: options?.maxLength,
      });
      
      return {
        success: true,
        message: result.message,
        genre: result.genre,
        confidence: result.confidence,
        fallbackUsed: result.fallbackUsed,
        latencyMs: result.latencyMs,
      };
    } catch (err) {
      return {
        success: false,
        error: (err as Error).message,
        latencyMs: Date.now() - startTime,
      };
    }
  }
  
  // ==========================================================================
  // CLASSIFICATION
  // ==========================================================================
  
  /**
   * Classify an email using the Edge Worker's hybrid classifier
   */
  async classifyMessage(
    subject: string,
    body: string
  ): Promise<ClassificationResult> {
    const result = await this.request<ClassificationResult>(
      'POST',
      '/classify/hybrid',
      { subject, body }
    );
    
    return result;
  }
  
  // ==========================================================================
  // REPUTATION
  // ==========================================================================
  
  /**
   * Analyze domain reputation
   */
  async analyzeReputation(
    domain: string,
    metrics?: {
      bounces?: {
        total: number;
        hard: number;
        soft: number;
        sent: number;
        recent: { hard: number; soft: number; sent: number };
      };
      engagement?: {
        sent: number;
        opened: number;
        clicked: number;
        replied: number;
        unsubscribed: number;
        spamComplaints: number;
      };
    }
  ): Promise<ReputationResult> {
    const result = await this.request<ReputationResult>(
      'POST',
      '/reputation/analyze',
      { domain, ...metrics }
    );
    
    return result;
  }
  
  // ==========================================================================
  // SCRAPING
  // ==========================================================================
  
  /**
   * Scrape a LinkedIn profile
   */
  async scrapeLinkedIn(profileUrl: string): Promise<{
    success: boolean;
    data?: LinkedInProfile;
    error?: string;
    cached: boolean;
    latencyMs: number;
  }> {
    return this.request('POST', '/scrape/linkedin', { url: profileUrl });
  }
  
  /**
   * Scrape company website
   */
  async scrapeCompany(websiteUrl: string): Promise<{
    success: boolean;
    data?: CompanyInfo;
    error?: string;
    cached: boolean;
    latencyMs: number;
  }> {
    return this.request('POST', '/scrape/company', { url: websiteUrl });
  }
  
  // ==========================================================================
  // COMPANY ENRICHMENT
  // ==========================================================================
  
  /**
   * Enrich company data using website scraping
   */
  async enrichCompany(company: {
    name: string;
    website?: string;
    linkedinUrl?: string;
  }): Promise<{
    success: boolean;
    data?: {
      description?: string;
      industry?: string;
      employees?: string;
      emails?: string[];
      socialLinks?: Record<string, string>;
    };
    error?: string;
    latencyMs: number;
  }> {
    return this.request('POST', '/enrich/company', company);
  }
  
  // ==========================================================================
  // HEALTH & WARMUP
  // ==========================================================================
  
  /**
   * Check Edge Worker health
   */
  async getHealth(): Promise<EdgeWorkerHealth> {
    return this.request('GET', '/health');
  }
  
  /**
   * Trigger model warmup on Edge Worker
   */
  async warmup(): Promise<{
    success: boolean;
    icebreaker: { loaded: boolean; latencyMs: number };
    classifier: { loaded: boolean; latencyMs: number };
    scraper: { initialized: boolean };
  }> {
    return this.request('POST', '/warmup', {});
  }
  
  /**
   * Ping Edge Worker (simple connectivity check)
   */
  async ping(): Promise<{ ok: boolean; latencyMs: number }> {
    const startTime = Date.now();
    
    try {
      await this.getHealth();
      return { ok: true, latencyMs: Date.now() - startTime };
    } catch {
      return { ok: false, latencyMs: Date.now() - startTime };
    }
  }
}

// ============================================================================
// SINGLETON INSTANCE
// ============================================================================

let _edgeWorkerClient: EdgeWorkerClient | null = null;

/**
 * Get or create the Edge Worker client singleton
 */
export function getEdgeWorkerClient(): EdgeWorkerClient {
  if (!_edgeWorkerClient) {
    _edgeWorkerClient = new EdgeWorkerClient({
      baseUrl: process.env.EDGE_WORKER_URL || '',
      secret: process.env.EDGE_WORKER_SECRET || '',
      timeoutMs: parseInt(process.env.EDGE_WORKER_TIMEOUT_MS || '30000', 10),
    });
  }
  
  return _edgeWorkerClient;
}

/**
 * Check if Edge Worker is configured and available
 */
export function isEdgeWorkerAvailable(): boolean {
  const client = getEdgeWorkerClient();
  return client.isEnabled();
}
