/**
 * Migration Studio Controller
 * Handles HTTP requests for experiment management
 */

import { Request, Response, NextFunction } from 'express';
import { MigrationStudioService } from '../services/migrationStudioService';
import pool from '../utils/postgres';
import { AppError } from '../middleware/errorHandler';
import { ExperimentMode } from '../types/migration';
import logger from '../utils/logger';

// FIX-11-628: lightweight in-process rate limiting for sensitive endpoints
type Bucket = { count: number; resetAt: number }
const assignmentBuckets: Record<string, Bucket> = {}
const importBuckets: Record<string, Bucket> = {}
const ASSIGN_RPM = Number(process.env.MIGRATION_ASSIGN_RPM ?? 600) // per IP per minute
const IMPORT_RPM = Number(process.env.MIGRATION_IMPORT_RPM ?? 30) // per user per minute
const IMPORT_MAX_BYTES = Number(process.env.MIGRATION_IMPORT_MAX_BYTES ?? 5 * 1024 * 1024) // 5MB default

function checkBucket(map: Record<string, Bucket>, key: string, limit: number) {
  const now = Date.now();
  const bucket = map[key] || { count: 0, resetAt: now + 60_000 };
  if (now > bucket.resetAt) {
    bucket.count = 0;
    bucket.resetAt = now + 60_000;
  }
  bucket.count += 1;
  map[key] = bucket;
  if (bucket.count > limit) {
    const retryIn = Math.max(0, bucket.resetAt - now);
    const err = new AppError('Too many requests', 429);
    (err as any).retryAfterMs = retryIn;
    throw err;
  }
}

const migrationService = new MigrationStudioService(pool);

/**
 * POST /api/v1/migration/experiments
 * Create a new migration experiment
 */
export const createExperiment = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const name = typeof req.body?.name === 'string' ? req.body.name.trim() : '';
    if (!name) {
      throw new AppError('name is required', 400);
    }

    const experiment = await migrationService.createExperiment(
      publisherId,
      userId,
      { ...req.body, name }
    );

    res.status(201).json({
      success: true,
      data: experiment,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * GET /api/v1/migration/experiments/:id
 * Get experiment by ID
 */
export const getExperiment = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const { id } = req.params;

    if (!publisherId) {
      throw new AppError('Unauthorized', 401);
    }

    const experiment = await migrationService.getExperiment(id, publisherId);

    res.json({
      success: true,
      data: experiment,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * GET /api/v1/migration/experiments
 * List all experiments for publisher
 */
export const listExperiments = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Unauthorized', 401);
    }

    const filters = {
      status: req.query.status as string | undefined,
      placement_id: req.query.placement_id as string | undefined,
    };

    const experiments = await migrationService.listExperiments(publisherId, filters);

    res.json({
      success: true,
      data: experiments,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * POST /api/v1/migration/import
 * Create migration import via CSV or connector
 */
export const createImportJob = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const ip = req.ip || req.headers['x-forwarded-for'] as string || 'unknown';
    // Rate limit per IP to protect CSV/connector imports
    checkBucket(importBuckets, String(req.user?.userId ?? ip), IMPORT_RPM);

    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const source = (req.body.source as string | undefined)?.trim();
    const placementId = (req.body.placement_id as string | undefined)?.trim();
    const experimentId = (req.body.experiment_id as string | undefined)?.trim();

    if (!source) {
      throw new AppError('source is required', 400);
    }

    if (!['csv', 'ironSource', 'applovin'].includes(source)) {
      throw new AppError('Unsupported import source', 400);
    }

    if (!placementId) {
      throw new AppError('placement_id is required', 400);
    }

    let credentials: { api_key: string; account_id: string } | undefined;
    if (req.body.credentials) {
      if (typeof req.body.credentials === 'string') {
        try {
          credentials = JSON.parse(req.body.credentials);
        } catch {
          throw new AppError('Invalid credentials payload', 400);
        }
      } else {
        credentials = req.body.credentials;
      }
    } else if (req.body.api_key || req.body.account_id) {
      credentials = {
        api_key: req.body.api_key,
        account_id: req.body.account_id,
      };
    }

    const fileBuffer = (req as any).file?.buffer as Buffer | undefined;
    if (fileBuffer) {
      if (fileBuffer.byteLength > IMPORT_MAX_BYTES) {
        throw new AppError(`CSV too large (>${IMPORT_MAX_BYTES} bytes)`, 413);
      }
      // very lightweight CSV sniff: require at least one comma and newline within first 4KB
      const head = fileBuffer.subarray(0, Math.min(4096, fileBuffer.byteLength)).toString('utf8');
      if (!head.includes(',') || !head.includes('\n')) {
        throw new AppError('Invalid CSV payload', 400);
      }
    }

    const result = await migrationService.createImport({
      publisherId,
      userId,
      placementId,
      experimentId,
      source: source as 'csv' | 'ironSource' | 'applovin',
      fileBuffer,
      credentials,
    });

    res.status(201).json({
      success: true,
      data: {
        import_id: result.import.id,
        experiment_id: result.import.experiment_id,
        placement_id: result.import.placement_id,
        source: result.import.source,
        status: result.import.status,
        created_at: result.import.created_at,
        summary: result.summary,
        mappings: result.mappings,
        signed_comparison: result.signedComparison,
      },
    });
  } catch (error) {
    next(error);
  }
};

/**
 * PUT /api/v1/migration/mappings/:id
 * Update mapping assignment and status
 */
export const updateMapping = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const result = await migrationService.updateMapping(req.params.id, publisherId, userId, {
      ourAdapterId: req.body.our_adapter_id,
      status: req.body.status,
      notes: req.body.notes,
    });

    res.json({
      success: true,
      data: result,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * POST /api/v1/migration/import/:id/finalize
 * Finalize import after mappings resolved
 */
export const finalizeImport = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const result = await migrationService.finalizeImport(req.params.id, publisherId, userId);

    res.json({
      success: true,
      data: {
        import_id: result.import.id,
        experiment_id: result.import.experiment_id,
        placement_id: result.import.placement_id,
        source: result.import.source,
        status: result.import.status,
        created_at: result.import.created_at,
        summary: result.summary,
        mappings: result.mappings,
        signed_comparison: result.signedComparison,
      },
    });
  } catch (error) {
    next(error);
  }
};

/**
 * POST /api/v1/migration/experiments/:id/guardrails/evaluate
 * Evaluate guardrails for an experiment
 */
export const evaluateGuardrails = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const result = await migrationService.evaluateGuardrails(
      req.params.id,
      publisherId,
      userId
    );

    res.json({
      success: true,
      data: result,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * PUT /api/v1/migration/experiments/:id
 * Update experiment
 */
export const updateExperiment = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;
    const { id } = req.params;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const experiment = await migrationService.updateExperiment(
      id,
      publisherId,
      userId,
      req.body
    );

    res.json({
      success: true,
      data: experiment,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * POST /api/v1/migration/experiments/:id/activate
 * Activate experiment (start mirroring)
 */
export const activateExperiment = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;
    const { id } = req.params;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const experiment = await migrationService.activateExperiment(
      id,
      publisherId,
      userId,
      req.body
    );

    res.json({
      success: true,
      data: experiment,
      message: `Experiment activated with ${experiment.mirror_percent}% traffic mirroring`,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * POST /api/v1/migration/experiments/:id/pause
 * Pause experiment (stop mirroring)
 */
export const pauseExperiment = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;
    const { id } = req.params;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const experiment = await migrationService.pauseExperiment(
      id,
      publisherId,
      userId,
      req.body.reason
    );

    res.json({
      success: true,
      data: experiment,
      message: 'Experiment paused',
    });
  } catch (error) {
    next(error);
  }
};

/**
 * DELETE /api/v1/migration/experiments/:id
 * Delete (archive) experiment
 */
export const deleteExperiment = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;
    const { id } = req.params;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    await migrationService.deleteExperiment(id, publisherId, userId);

    res.json({
      success: true,
      message: 'Experiment archived',
    });
  } catch (error) {
    next(error);
  }
};

/**
 * POST /api/v1/migration/assign
 * Get experiment assignment for a user (used by SDKs)
 * Returns adapter configuration for test arm so SDK can actually route traffic
 */
export const getAssignment = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    // Optional shared-secret authentication for SDKs (guards unauthenticated endpoint)
    const configuredSecret = process.env.MIGRATION_SDK_SHARED_SECRET;
    if (configuredSecret) {
      const provided = (req.headers['x-migration-sdk-secret'] as string | undefined)?.trim();
      if (!provided || provided !== configuredSecret) {
        throw new AppError('Unauthorized', 401);
      }
    }

    // Basic per-IP rate limiting
    const ip = req.ip || (req.headers['x-forwarded-for'] as string) || 'unknown';
    try {
      checkBucket(assignmentBuckets, ip, ASSIGN_RPM);
    } catch (e) {
      logger.warn('Assignment rate limited', { ip });
      throw e;
    }

    const { user_identifier, placement_id } = req.body;

    if (!user_identifier || !placement_id) {
      throw new AppError('user_identifier and placement_id required', 400);
    }

    // Find active experiment for this placement
    const result = await pool.query(
      `SELECT * FROM migration_experiments 
       WHERE placement_id = $1 AND status = 'active'
       LIMIT 1`,
      [placement_id]
    );

    if (result.rows.length === 0) {
      // No active experiment for this placement
      res.json({
        success: true,
        data: {
          has_experiment: false,
          arm: 'control',
          use_apex: false,
        },
      });
      return;
    }

    const experiment = result.rows[0];
    const mode: ExperimentMode = (experiment.mode as ExperimentMode) || 'shadow';

    const featureFlags = await migrationService.getEffectiveFeatureFlags(
      experiment.publisher_id,
      experiment.app_id,
      experiment.placement_id
    );

    const modeEnabled = mode === 'shadow'
      ? featureFlags.shadowEnabled
      : featureFlags.mirroringEnabled;

    if (!modeEnabled) {
      res.json({
        success: true,
        data: {
          has_experiment: false,
          arm: 'control',
          use_apex: false,
        },
      });
      return;
    }

    // Deterministic assignment
    const arm = migrationService.assignArm(
      user_identifier,
      placement_id,
      experiment.seed,
      experiment.mirror_percent
    );

    const assignmentTs = new Date().toISOString();
    const assignmentMetadata = {
      assignment_ts: assignmentTs,
      feature_flag_source: featureFlags.source,
      mode,
    };

    // Log assignment (async, don't block response)
    migrationService.logAssignment(
      experiment.id,
      arm,
      user_identifier,
      placement_id,
      assignmentMetadata
    ).catch(err => {
      console.error('Failed to log assignment:', err);
    });

    // For test arm in mirroring mode, fetch adapter configurations
    let adapters: Array<{ adapter_id: string; adapter_name: string; priority: number }> = [];
    const useApex = arm === 'test' && mode === 'mirroring';
    
    if (useApex) {
      const mappingsResult = await pool.query(
        `SELECT our_adapter_id, our_adapter_name, incumbent_waterfall_position
         FROM migration_mappings
         WHERE experiment_id = $1 AND mapping_status = 'confirmed' AND our_adapter_id IS NOT NULL
         ORDER BY incumbent_waterfall_position ASC NULLS LAST`,
        [experiment.id]
      );
      
      adapters = mappingsResult.rows.map((row, idx) => ({
        adapter_id: row.our_adapter_id,
        adapter_name: row.our_adapter_name || row.our_adapter_id,
        priority: row.incumbent_waterfall_position ?? idx + 1,
      }));
    }

    res.json({
      success: true,
      data: {
        has_experiment: true,
        experiment_id: experiment.id,
        arm,
        use_apex: useApex,
        adapters: useApex ? adapters : undefined,
        mirror_percent: experiment.mirror_percent,
        assignment_ts: assignmentTs,
        mode,
        feature_flag_source: featureFlags.source,
      },
    });
  } catch (error) {
    next(error);
  }
};

/**
 * GET /api/v1/migration/reports/:experimentId
 * Generate side-by-side comparison report with signed verification
 */
export const generateReport = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const { experimentId } = req.params;

    if (!publisherId) {
      throw new AppError('Unauthorized', 401);
    }

    const result = await migrationService.generateReport(experimentId, publisherId);

    res.json({
      success: true,
      data: result,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * GET /api/v1/migration/experiments/:id/sdk-integration
 * Generate SDK integration code snippets for the experiment
 */
export const getSdkIntegration = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const { id } = req.params;
    const { platform } = req.query;

    if (!publisherId) {
      throw new AppError('Unauthorized', 401);
    }

    // Get experiment details
    const experiment = await migrationService.getExperiment(id, publisherId);
    
    // Get mappings for adapter list
    const mappingsResult = await pool.query(
      `SELECT our_adapter_id, our_adapter_name
       FROM migration_mappings
       WHERE experiment_id = $1 AND mapping_status = 'confirmed' AND our_adapter_id IS NOT NULL`,
      [id]
    );
    
    const adapters = mappingsResult.rows.map(row => ({
      id: row.our_adapter_id,
      name: row.our_adapter_name || row.our_adapter_id,
    }));

    const baseUrl = process.env.API_BASE_URL || 'https://api.apex-ad.com';
    const placementId = experiment.placement_id || 'YOUR_PLACEMENT_ID';

    // Generate platform-specific code
    const snippets: Record<string, { language: string; code: string; instructions: string }> = {
      ios: {
        language: 'swift',
        code: `// ApexMigration SDK Integration - iOS
import ApexSDK

class AdManager {
    static let shared = AdManager()
    
    func initialize() {
        // Configure migration experiment
        ApexMigration.configure(
            experimentId: "${id}",
            placementId: "${placementId}"
        )
        
        // Check assignment before loading ads
        ApexMigration.getAssignment { [weak self] result in
            switch result {
            case .success(let assignment):
                if assignment.useApex {
                    // Route traffic through Apex adapters
                    self?.loadWithApex(adapters: assignment.adapters)
                } else {
                    // Continue using incumbent mediation
                    self?.loadWithIncumbent()
                }
            case .failure(let error):
                // Fallback to incumbent on error
                print("Migration assignment failed: \\(error)")
                self?.loadWithIncumbent()
            }
        }
    }
    
    private func loadWithApex(adapters: [ApexAdapter]) {
        // Initialize Apex adapters in priority order
        for adapter in adapters {
            ApexAdapterRegistry.register(adapter)
        }
        ApexAdLoader.shared.load(placementId: "${placementId}")
    }
    
    private func loadWithIncumbent() {
        // Your existing mediation code
        // IronSource.loadInterstitial() or MAXInterstitialAd.load()
    }
}`,
        instructions: `1. Add ApexSDK to your Podfile: pod 'ApexSDK', '~> 2.0'
2. Run pod install
3. Replace YOUR_PLACEMENT_ID with your actual placement ID
4. Call AdManager.shared.initialize() in your app delegate`,
      },
      android: {
        language: 'kotlin',
        code: `// ApexMigration SDK Integration - Android
import com.apex.sdk.ApexMigration
import com.apex.sdk.MigrationAssignment

class AdManager private constructor() {
    companion object {
        val instance: AdManager by lazy { AdManager() }
    }
    
    fun initialize(context: Context) {
        // Configure migration experiment
        ApexMigration.configure(
            experimentId = "${id}",
            placementId = "${placementId}"
        )
        
        // Check assignment before loading ads
        ApexMigration.getAssignment { result ->
            result.onSuccess { assignment ->
                if (assignment.useApex) {
                    // Route traffic through Apex adapters
                    loadWithApex(context, assignment.adapters)
                } else {
                    // Continue using incumbent mediation
                    loadWithIncumbent(context)
                }
            }.onFailure { error ->
                // Fallback to incumbent on error
                Log.e("Migration", "Assignment failed: \${error.message}")
                loadWithIncumbent(context)
            }
        }
    }
    
    private fun loadWithApex(context: Context, adapters: List<ApexAdapter>) {
        // Initialize Apex adapters in priority order
        adapters.forEach { adapter ->
            ApexAdapterRegistry.register(adapter)
        }
        ApexAdLoader.getInstance().load(context, "${placementId}")
    }
    
    private fun loadWithIncumbent(context: Context) {
        // Your existing mediation code
        // IronSource.loadInterstitial() or MaxInterstitialAd.load()
    }
}`,
        instructions: `1. Add to build.gradle: implementation 'com.apex.sdk:apex-sdk:2.0.0'
2. Sync Gradle
3. Replace YOUR_PLACEMENT_ID with your actual placement ID
4. Call AdManager.instance.initialize(this) in your Application class`,
      },
      web: {
        language: 'typescript',
        code: `// ApexMigration SDK Integration - Web
import { ApexMigration, MigrationAssignment } from '@apex-ad/web-sdk';

class AdManager {
  private static instance: AdManager;
  
  static getInstance(): AdManager {
    if (!this.instance) {
      this.instance = new AdManager();
    }
    return this.instance;
  }
  
  async initialize(): Promise<void> {
    // Configure migration experiment
    ApexMigration.configure({
      experimentId: '${id}',
      placementId: '${placementId}',
    });
    
    try {
      // Check assignment before loading ads
      const assignment = await ApexMigration.getAssignment();
      
      if (assignment.useApex) {
        // Route traffic through Apex adapters
        await this.loadWithApex(assignment.adapters);
      } else {
        // Continue using incumbent mediation
        await this.loadWithIncumbent();
      }
    } catch (error) {
      // Fallback to incumbent on error
      console.error('Migration assignment failed:', error);
      await this.loadWithIncumbent();
    }
  }
  
  private async loadWithApex(adapters: ApexAdapter[]): Promise<void> {
    // Initialize Apex adapters in priority order
    for (const adapter of adapters) {
      ApexAdapterRegistry.register(adapter);
    }
    await ApexAdLoader.load('${placementId}');
  }
  
  private async loadWithIncumbent(): Promise<void> {
    // Your existing mediation code
    // googletag.cmd.push() or prebid.js
  }
}

export default AdManager;`,
        instructions: `1. Install: npm install @apex-ad/web-sdk
2. Import AdManager in your entry point
3. Replace YOUR_PLACEMENT_ID with your actual placement ID
4. Call AdManager.getInstance().initialize() on page load`,
      },
      unity: {
        language: 'csharp',
        code: `// ApexMigration SDK Integration - Unity
using ApexSDK;
using UnityEngine;

public class AdManager : MonoBehaviour
{
    public static AdManager Instance { get; private set; }
    
    private void Awake()
    {
        if (Instance == null)
        {
            Instance = this;
            DontDestroyOnLoad(gameObject);
        }
        else
        {
            Destroy(gameObject);
        }
    }
    
    public void Initialize()
    {
        // Configure migration experiment
        ApexMigration.Configure(
            experimentId: "${id}",
            placementId: "${placementId}"
        );
        
        // Check assignment before loading ads
        ApexMigration.GetAssignment((assignment, error) =>
        {
            if (error != null)
            {
                Debug.LogError($"Migration assignment failed: {error}");
                LoadWithIncumbent();
                return;
            }
            
            if (assignment.UseApex)
            {
                // Route traffic through Apex adapters
                LoadWithApex(assignment.Adapters);
            }
            else
            {
                // Continue using incumbent mediation
                LoadWithIncumbent();
            }
        });
    }
    
    private void LoadWithApex(ApexAdapter[] adapters)
    {
        // Initialize Apex adapters in priority order
        foreach (var adapter in adapters)
        {
            ApexAdapterRegistry.Register(adapter);
        }
        ApexAdLoader.Instance.Load("${placementId}");
    }
    
    private void LoadWithIncumbent()
    {
        // Your existing mediation code
        // IronSource.Agent.loadInterstitial() or MaxSdk.LoadInterstitial()
    }
}`,
        instructions: `1. Import ApexSDK.unitypackage into your project
2. Add AdManager script to a GameObject in your first scene
3. Replace YOUR_PLACEMENT_ID with your actual placement ID
4. Call AdManager.Instance.Initialize() after scene loads`,
      },
    };

    // Filter by platform if specified
    const platforms = platform 
      ? { [platform as string]: snippets[platform as string] }
      : snippets;

    res.json({
      success: true,
      data: {
        experiment_id: id,
        placement_id: placementId,
        adapters,
        api_endpoint: `${baseUrl}/api/v1/migration/assign`,
        snippets: platforms,
      },
    });
  } catch (error) {
    next(error);
  }
};

/**
 * POST /api/v1/migration/experiments/:id/complete
 * Complete migration - set experiment to 100% and mark complete
 */
export const completeMigration = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;
    const { id } = req.params;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const experiment = await migrationService.completeMigration(id, publisherId, userId);

    res.json({
      success: true,
      data: experiment,
      message: 'Migration completed. 100% traffic now routed through Apex.',
    });
  } catch (error) {
    next(error);
  }
};

/**
 * POST /api/v1/migration/experiments/:id/rollback
 * Rollback migration - pause and revert to control
 */
export const rollbackMigration = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const userId = req.user?.userId;
    const { id } = req.params;
    const { reason } = req.body;

    if (!publisherId || !userId) {
      throw new AppError('Unauthorized', 401);
    }

    const experiment = await migrationService.rollbackMigration(id, publisherId, userId, reason);

    res.json({
      success: true,
      data: experiment,
      message: 'Migration rolled back. Traffic reverted to incumbent mediation.',
    });
  } catch (error) {
    next(error);
  }
};

/**
 * GET /api/v1/migration/experiments/:id/mappings
 * Get all mappings for an experiment
 */
export const getExperimentMappings = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;
    const { id } = req.params;

    if (!publisherId) {
      throw new AppError('Unauthorized', 401);
    }

    // Verify experiment belongs to publisher
    await migrationService.getExperiment(id, publisherId);
    
    const result = await pool.query(
      `SELECT * FROM migration_mappings WHERE experiment_id = $1 ORDER BY incumbent_waterfall_position ASC NULLS LAST`,
      [id]
    );

    res.json({
      success: true,
      data: result.rows,
    });
  } catch (error) {
    next(error);
  }
};
