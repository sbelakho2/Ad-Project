/**
 * ICEBREAKER ENGINE (On-Device LLM)
 * 
 * Generates hyper-personalized icebreakers using local Flan-T5-Small model.
 * Runs entirely on CPU via ONNX runtime - no API costs.
 * 
 * Fixed bugs:
 *   - Model loading race condition: explicit warmup with Promise queue
 *   - Template fallback always available
 *   - Graceful degradation on model failure
 */

import type { Pipeline } from '@xenova/transformers';

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
  source: 'llm' | 'template';
  latencyMs: number;
  modelUsed?: string;
}

export interface ModelStatus {
  loaded: boolean;
  loading: boolean;
  error: string | null;
  modelName: string;
}

// ============================================================================
// TEMPLATE FALLBACKS
// ============================================================================

const GENRE_TEMPLATES: Record<string, string[]> = {
  'Strategy': [
    "I noticed {{appName}} has some really engaging tactical depth",
    "The strategic elements in {{appName}} caught my attention",
    "I've been impressed by the gameplay mechanics in {{appName}}",
  ],
  'Casual': [
    "{{appName}} looks like a great way to unwind",
    "The pick-up-and-play style of {{appName}} is refreshing",
    "I love how accessible {{appName}} is for new players",
  ],
  'Puzzle': [
    "The brain-teasing challenges in {{appName}} are impressive",
    "I noticed {{appName}} has some clever puzzle mechanics",
    "The creative problem-solving in {{appName}} stands out",
  ],
  'Action': [
    "The fast-paced gameplay in {{appName}} is exciting",
    "{{appName}}'s action sequences look really polished",
    "I was impressed by the combat system in {{appName}}",
  ],
  'Simulation': [
    "The attention to detail in {{appName}} is remarkable",
    "I noticed {{appName}} creates a really immersive experience",
    "The simulation depth in {{appName}} is impressive",
  ],
  'Racing': [
    "The racing mechanics in {{appName}} feel really tight",
    "I noticed {{appName}} has some impressive track designs",
    "The sense of speed in {{appName}} is exhilarating",
  ],
  'Adventure': [
    "The world-building in {{appName}} really drew me in",
    "I noticed {{appName}} has compelling exploration elements",
    "The narrative depth in {{appName}} stands out",
  ],
  'Role Playing': [
    "The character progression in {{appName}} is well designed",
    "I noticed {{appName}} has deep RPG mechanics",
    "The story elements in {{appName}} caught my attention",
  ],
  'default': [
    "I've been following {{appName}}'s growth in the market",
    "{{appName}} caught my attention recently",
    "I noticed some interesting updates to {{appName}}",
    "I came across {{appName}} and was impressed by what I saw",
  ],
};

function getTemplateFallback(context: IcebreakerContext): string {
  const templates = GENRE_TEMPLATES[context.genre || 'default'] || GENRE_TEMPLATES['default'];
  const template = templates[Math.floor(Math.random() * templates.length)];
  return template.replace(/\{\{appName\}\}/g, context.appName);
}

// ============================================================================
// ICEBREAKER ENGINE CLASS
// ============================================================================

const MODEL_NAME = 'Xenova/LaMini-Flan-T5-248M';

export class IcebreakerEngine {
  private pipeline: Pipeline | null = null;
  private loading = false;
  private loadError: Error | null = null;
  private loadPromise: Promise<void> | null = null;
  private initialized = false;
  
  async initialize(): Promise<void> {
    if (this.initialized) return;
    this.initialized = true;
    // Don't load model here - lazy load on first use or warmup
  }
  
  /**
   * Warmup the model (call on startup to avoid first-request latency)
   */
  async warmup(): Promise<{ success: boolean; latencyMs: number }> {
    const startTime = Date.now();
    
    try {
      await this.loadModel();
      
      // Do a test generation to fully warm the model
      if (this.pipeline) {
        await this.pipeline('Write a short greeting', {
          max_new_tokens: 20,
          do_sample: false,
        });
      }
      
      return { success: true, latencyMs: Date.now() - startTime };
    } catch (err) {
      return { success: false, latencyMs: Date.now() - startTime };
    }
  }
  
  /**
   * Load the model with proper synchronization (fixes race condition)
   */
  private async loadModel(): Promise<boolean> {
    // Already loaded
    if (this.pipeline) return true;
    
    // Failed previously
    if (this.loadError) return false;
    
    // Already loading - wait for it
    if (this.loadPromise) {
      await this.loadPromise;
      return this.pipeline !== null;
    }
    
    // Start loading
    this.loading = true;
    this.loadPromise = this.doLoadModel();
    
    try {
      await this.loadPromise;
      return this.pipeline !== null;
    } finally {
      this.loading = false;
      this.loadPromise = null;
    }
  }
  
  private async doLoadModel(): Promise<void> {
    try {
      console.log(`[IcebreakerEngine] Loading model ${MODEL_NAME}...`);
      
      const { pipeline } = await import('@xenova/transformers');
      
      // Use 'as unknown as Pipeline' to avoid TypeScript mismatch between pipeline types
      this.pipeline = await pipeline('text2text-generation', MODEL_NAME, {
        quantized: true,
      }) as unknown as Pipeline;
      
      console.log('[IcebreakerEngine] Model loaded successfully');
    } catch (err) {
      this.loadError = err as Error;
      console.error('[IcebreakerEngine] Model load failed:', (err as Error).message);
      throw err;
    }
  }
  
  /**
   * Get model status
   */
  getStatus(): ModelStatus {
    return {
      loaded: this.pipeline !== null,
      loading: this.loading,
      error: this.loadError?.message || null,
      modelName: MODEL_NAME,
    };
  }
  
  /**
   * Generate an icebreaker
   */
  async generate(context: IcebreakerContext): Promise<IcebreakerResult> {
    const startTime = Date.now();
    
    // Try LLM generation if model is available and we have a description
    const modelReady = await this.loadModel();
    
    if (modelReady && this.pipeline && context.appDescription) {
      try {
        const prompt = this.buildPrompt(context);
        
        const output = await this.pipeline(prompt, {
          max_new_tokens: 50,
          do_sample: true,
          temperature: 0.7,
          top_p: 0.9,
        });
        
        const generated = Array.isArray(output) 
          ? (output[0] as { generated_text?: string })?.generated_text 
          : (output as { generated_text?: string })?.generated_text;
        
        if (generated && generated.length > 10) {
          // Clean up the output
          const icebreaker = this.cleanOutput(generated, context.appName);
          
          return {
            icebreaker,
            source: 'llm',
            latencyMs: Date.now() - startTime,
            modelUsed: MODEL_NAME,
          };
        }
      } catch (err) {
        console.warn('[IcebreakerEngine] LLM generation failed, falling back to template:', (err as Error).message);
      }
    }
    
    // Fallback to template
    return {
      icebreaker: getTemplateFallback(context),
      source: 'template',
      latencyMs: Date.now() - startTime,
    };
  }
  
  private buildPrompt(context: IcebreakerContext): string {
    const parts = [`App: ${context.appName}`];
    
    if (context.appDescription) {
      // Truncate long descriptions
      const desc = context.appDescription.length > 200 
        ? context.appDescription.substring(0, 200) + '...'
        : context.appDescription;
      parts.push(`Description: ${desc}`);
    }
    
    if (context.genre) parts.push(`Genre: ${context.genre}`);
    if (context.installs) parts.push(`Installs: ${context.installs}`);
    if (context.rating) parts.push(`Rating: ${context.rating}`);
    
    return `Write a brief, professional icebreaker sentence for a sales email about this mobile app. Be specific and genuine.

${parts.join('\n')}

Icebreaker:`;
  }
  
  private cleanOutput(text: string, appName: string): string {
    // Remove common LLM artifacts
    let cleaned = text
      .replace(/^(Icebreaker:|Output:|Response:)/i, '')
      .replace(/^\s*["']|["']\s*$/g, '')
      .trim();
    
    // Ensure it mentions the app name if it doesn't
    if (!cleaned.toLowerCase().includes(appName.toLowerCase())) {
      cleaned = cleaned.replace(/^(I|The|Your)/i, `${appName}'s`);
    }
    
    // Ensure it ends properly
    if (!cleaned.match(/[.!?]$/)) {
      cleaned += '.';
    }
    
    return cleaned;
  }
}
