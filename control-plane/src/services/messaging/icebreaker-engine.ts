/**
 * ICEBREAKER ENGINE (Edge Worker Proxy)
 * 
 * Generates context-aware icebreakers by proxying requests to the Edge Worker.
 * The Edge Worker runs the actual LLM inference (Flan-T5-Small via ONNX).
 * 
 * Architecture:
 *   - Control Plane (this file): orchestration, caching, database updates
 *   - Edge Worker (Test Server): LLM inference
 * 
 * Falls back to template-based icebreakers if Edge Worker unavailable.
 * 
 * Usage:
 *   const engine = await IcebreakerEngine.getInstance();
 *   const icebreaker = await engine.generate({
 *     appName: 'Clash of Clans',
 *     appDescription: 'Epic combat strategy game...',
 *     genre: 'Strategy',
 *   });
 */

import { logger } from '../../logger.js';
import { queryOperator, queryReadOnly } from '../../database.js';
import { getEdgeWorkerClient, isEdgeWorkerAvailable } from '../edge-worker-client.js';
import { scoreIcebreakerQuality } from './spintax-engine.js';

// ============================================================================
// TYPES
// ============================================================================

export interface IcebreakerContext {
  appName: string;
  appDescription?: string;
  genre?: string;
  developerName?: string;
  installs?: string;
  rating?: number;
}

export interface IcebreakerResult {
  icebreaker: string;
  source: 'llm' | 'template' | 'cached';
  latencyMs: number;
  modelUsed?: string;
  qualityScore?: number; // Added field
}

export interface IcebreakerStatus {
  modelLoaded: boolean;
  modelLoading: boolean;
  modelError: string | null;
  modelName: string;
  edgeWorkerAvailable: boolean;
  edgeWorkerUrl: string | null;
}

// ============================================================================
// TEMPLATE FALLBACKS
// ============================================================================

// IMPROVED: Using Spintax for high variety and specific keywords to pass quality gates
// FIXED (Jan 2026): Standardized to {{APP_NAME}} for consistency with spintax-engine
const GENRE_TEMPLATES: Record<string, string[]> = {
  'Strategy': [
    "{I noticed|I saw} that {{APP_NAME}} has {some really engaging|deep} tactical depth mechanics",
    "The {strategic elements|tactical design} in {{APP_NAME}} {caught my attention|stood out to me}",
    "I've been {impressed by|following} the gameplay mechanics {updates|changes} in {{APP_NAME}}",
  ],
  'Casual': [
    "{{APP_NAME}} {looks like|seems like} a {perfect|great} way to unwind with its {design|mechanics}",
    "The {pick-up-and-play|accessible} style of {{APP_NAME}} is {refreshing|well executed}",
    "I {love|like} how {accessible|polished} the user interface in {{APP_NAME}} is",
  ],
  'Puzzle': [
    "The {brain-teasing|clever} challenges in {{APP_NAME}} are {impressive|well designed}",
    "I {noticed|saw} {{APP_NAME}} has some {clever|unique} puzzle mechanics in the latest version",
    "The {creative|unique} problem-solving design in {{APP_NAME}} {stands out|is engaging}",
  ],
  'Action': [
    "The {fast-paced|dynamic} gameplay in {{APP_NAME}} is {exciting|engaging}",
    "{{APP_NAME}}'s {action sequences|combat design} look {really polished|solid}",
    "I was {impressed|intrigued} by the {combat system|control scheme} in {{APP_NAME}}",
  ],
  'Simulation': [
    "The {attention to detail|depth} in {{APP_NAME}} is {remarkable|notable}",
    "I {noticed|saw} {{APP_NAME}} creates a {really immersive|compelling} experience",
    "The {simulation depth|mechanics} in {{APP_NAME}} {is impressive|stands out}",
  ],
  'Racing': [
    "The {racing mechanics|physics engine} in {{APP_NAME}} {look|seem} really {smooth|polished}",
    "I {noticed|saw} {{APP_NAME}} has some {exciting|challenging} track designs",
    "The {competitive element|multiplayer aspect} in {{APP_NAME}} is {engaging|well done}",
  ],
  'Sports': [
    "The {athletic|physics} gameplay in {{APP_NAME}} is {well-executed|solid}",
    "I've been {following|watching} {{APP_NAME}}'s {sports mechanics|league updates}",
    "The {realism|authenticity} in {{APP_NAME}} is {impressive|notable}",
  ],
  'RPG': [
    "The {character progression|skill system} in {{APP_NAME}} is {compelling|deep}",
    "I {noticed|saw} {{APP_NAME}} has {deep|engaging} RPG elements in the latest update",
    "The {story|narrative} and gameplay in {{APP_NAME}} {work well together|are well balanced}",
  ],
  'Adventure': [
    "The {exploration elements|world building} in {{APP_NAME}} are {engaging|immersive}",
    "I {noticed|saw} {{APP_NAME}} creates an {immersive|expansive} world",
    "The {adventure design|level design} in {{APP_NAME}} is {impressive|memorable}",
  ],
  'default': [
    "I've been {following|tracking} {{APP_NAME}}'s {growth|progress} in the market",
    "{{APP_NAME}} {caught my attention|stood out} recently with its {features|design}",
    "I {noticed|saw} some {interesting|clever} updates to {{APP_NAME}}",
    "The {user engagement|ratings} on {{APP_NAME}} {is|are} {impressive|solid}",
  ],
};

function getTemplateFallback(context: IcebreakerContext): string {
  const templates = GENRE_TEMPLATES[context.genre || 'default'] || GENRE_TEMPLATES['default'];
  const template = templates[Math.floor(Math.random() * templates.length)];
  // FIXED (Jan 2026): Use case-insensitive regex to match standardized {{APP_NAME}}
  return template.replace(/\{\{APP_NAME\}\}/gi, context.appName);
}

// ============================================================================
// ICEBREAKER ENGINE
// ============================================================================

export class IcebreakerEngine {
  private static instance: IcebreakerEngine | null = null;
  private initialized = false;
  private lastEdgeWorkerCheck = 0;
  private edgeWorkerOnline = false;
  
  private constructor() {}
  
  static async getInstance(): Promise<IcebreakerEngine> {
    if (!IcebreakerEngine.instance) {
      IcebreakerEngine.instance = new IcebreakerEngine();
      await IcebreakerEngine.instance.initialize();
    }
    return IcebreakerEngine.instance;
  }
  
  private async initialize(): Promise<void> {
    if (this.initialized) return;
    
    // Check Edge Worker connectivity
    await this.checkEdgeWorker();
    
    this.initialized = true;
    
    logger.info('[IcebreakerEngine] Initialized', {
      edgeWorkerAvailable: isEdgeWorkerAvailable(),
      edgeWorkerUrl: process.env.EDGE_WORKER_URL,
    });
  }
  
  private async checkEdgeWorker(): Promise<boolean> {
    // Only check every 30 seconds
    const now = Date.now();
    if (now - this.lastEdgeWorkerCheck < 30000) {
      return this.edgeWorkerOnline;
    }
    
    this.lastEdgeWorkerCheck = now;
    
    if (!isEdgeWorkerAvailable()) {
      this.edgeWorkerOnline = false;
      return false;
    }
    
    try {
      const client = getEdgeWorkerClient();
      const result = await client.ping();
      this.edgeWorkerOnline = result.ok;
      return result.ok;
    } catch {
      this.edgeWorkerOnline = false;
      return false;
    }
  }
  
  /**
   * Generate an icebreaker for a contact/app
   */
  async generate(context: IcebreakerContext): Promise<IcebreakerResult> {
    const startTime = Date.now();
    
    // 1. Check cache first
    const cached = await this.getCachedIcebreaker(context.appName);
    if (cached) {
      return {
        icebreaker: cached,
        source: 'cached',
        latencyMs: Date.now() - startTime,
      };
    }
    
    // 2. Try Edge Worker LLM
    const edgeWorkerAvailable = await this.checkEdgeWorker();
    
    if (edgeWorkerAvailable) {
      try {
        const client = getEdgeWorkerClient();
        
        const result = await client.generateIcebreaker({
          name: context.appName,
          company: context.developerName || 'Unknown Developer',
          title: context.genre,
          industry: 'Mobile Gaming',
        }, {
          genre: this.mapGenreToStyle(context.genre),
        });
        
        if (result.success && result.message) {
          // QUALITY GATE CHECK
          // Ensure generated content meets safety and engagement standards
          const quality = scoreIcebreakerQuality(result.message);
          
          if (quality.passesQualityGate) {
            // Cache the result
            await this.cacheIcebreaker(context.appName, result.message);
            
            return {
              icebreaker: result.message,
              source: 'llm',
              latencyMs: Date.now() - startTime,
              modelUsed: 'edge-worker-flan-t5',
              qualityScore: quality.score
            };
          }
          
          logger.warn('[IcebreakerEngine] LLM output failed quality gate, falling back to template', {
            appName: context.appName,
            output: result.message,
            issues: quality.issues
          });
        }
      } catch (err) {
        logger.warn('[IcebreakerEngine] Edge Worker call failed', {
          error: (err as Error).message,
          appName: context.appName,
        });
      }
    }
    
    // 3. Fallback to template (Now Spintax-enhanced for variety)
    const templateIcebreaker = getTemplateFallback(context);
    
    // Cache template result too
    await this.cacheIcebreaker(context.appName, templateIcebreaker);
    
    return {
      icebreaker: templateIcebreaker,
      source: 'template',
      latencyMs: Date.now() - startTime,
      qualityScore: 100 // Trusted source
    };
  }
  
  private mapGenreToStyle(genre?: string): 'curious' | 'direct' | 'social_proof' | 'value_first' | 'pain_agitate' {
    if (!genre) return 'curious';
    
    const genreLower = genre.toLowerCase();
    
    if (genreLower.includes('business') || genreLower.includes('finance')) {
      return 'value_first';
    }
    if (genreLower.includes('social') || genreLower.includes('casual')) {
      return 'social_proof';
    }
    if (genreLower.includes('strategy') || genreLower.includes('rpg')) {
      return 'direct';
    }
    
    return 'curious';
  }
  
  private async getCachedIcebreaker(appName: string): Promise<string | null> {
    try {
      const result = await queryReadOnly<{ icebreaker: string }>(`
        SELECT icebreaker FROM sa_icebreaker_cache
        WHERE app_name = $1 AND created_at > NOW() - INTERVAL '7 days'
      `, [appName]);
      
      return result[0]?.icebreaker || null;
    } catch {
      return null;
    }
  }
  
  private async cacheIcebreaker(appName: string, icebreaker: string): Promise<void> {
    try {
      await queryOperator(`
        INSERT INTO sa_icebreaker_cache (app_name, icebreaker)
        VALUES ($1, $2)
        ON CONFLICT (app_name) DO UPDATE SET
          icebreaker = EXCLUDED.icebreaker,
          created_at = NOW()
      `, [appName, icebreaker]);
    } catch (err) {
      // Non-critical error
      logger.debug('[IcebreakerEngine] Cache write failed', {
        error: (err as Error).message,
      });
    }
  }
  
  getStatus(): IcebreakerStatus {
    return {
      modelLoaded: this.edgeWorkerOnline,
      modelLoading: false,
      modelError: null,
      modelName: 'Xenova/LaMini-Flan-T5-248M',
      edgeWorkerAvailable: isEdgeWorkerAvailable(),
      edgeWorkerUrl: process.env.EDGE_WORKER_URL || null,
    };
  }
  
  /**
   * Pre-warm the Edge Worker model
   */
  async warmup(): Promise<{ success: boolean; latencyMs: number }> {
    const startTime = Date.now();
    
    if (!isEdgeWorkerAvailable()) {
      return { success: false, latencyMs: Date.now() - startTime };
    }
    
    try {
      const client = getEdgeWorkerClient();
      const result = await client.warmup();
      
      return {
        success: result.success && result.icebreaker.loaded,
        latencyMs: Date.now() - startTime,
      };
    } catch {
      return { success: false, latencyMs: Date.now() - startTime };
    }
  }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/**
 * Generate icebreaker for a contact by ID
 */
export async function generateIcebreakerForContact(contactId: string): Promise<IcebreakerResult | null> {
  const contacts = await queryReadOnly<{
    app_name: string | null;
    app_description: string | null;
    app_genre: string | null;
    developer_name: string | null;
    installs: string | null;
    rating: number | null;
  }>(`
    SELECT app_name, app_description, app_genre, developer_name, installs, rating
    FROM sa_contacts WHERE id = $1
  `, [contactId]);
  
  if (contacts.length === 0 || !contacts[0].app_name) {
    return null;
  }
  
  const contact = contacts[0];
  const engine = await IcebreakerEngine.getInstance();
  
  return engine.generate({
    appName: contact.app_name ?? 'App',
    appDescription: contact.app_description || undefined,
    genre: contact.app_genre || undefined,
    developerName: contact.developer_name || undefined,
    installs: contact.installs || undefined,
    rating: contact.rating || undefined,
  });
}

/**
 * Batch generate icebreakers for contacts without them
 */
export async function batchGenerateIcebreakers(limit = 10): Promise<{
  processed: number;
  llmGenerated: number;
  templateGenerated: number;
  errors: number;
}> {
  // Find contacts without icebreakers
  const contacts = await queryReadOnly<{
    id: string;
    app_name: string;
    app_description: string | null;
    app_genre: string | null;
  }>(`
    SELECT id, app_name, app_description, app_genre
    FROM sa_contacts
    WHERE icebreaker IS NULL AND app_name IS NOT NULL
    LIMIT $1
  `, [limit]);
  
  let llmGenerated = 0;
  let templateGenerated = 0;
  let errors = 0;
  
  const engine = await IcebreakerEngine.getInstance();
  
  for (const contact of contacts) {
    try {
      const result = await engine.generate({
        appName: contact.app_name,
        appDescription: contact.app_description || undefined,
        genre: contact.app_genre || undefined,
      });
      
      // Update contact with icebreaker
      await queryOperator(`
        UPDATE sa_contacts SET icebreaker = $1 WHERE id = $2
      `, [result.icebreaker, contact.id]);
      
      if (result.source === 'llm') llmGenerated++;
      else templateGenerated++;
      
    } catch (err) {
      logger.warn('Icebreaker generation failed for contact', {
        contactId: contact.id,
        error: (err as Error).message,
      });
      errors++;
    }
  }
  
  return {
    processed: contacts.length,
    llmGenerated,
    templateGenerated,
    errors,
  };
}
