/**
 * Migration Studio Service
 * Handles experiment management, assignment logic, and guardrail evaluation
 */

import { Pool, PoolClient } from 'pg';
import { createHash, randomBytes, randomUUID } from 'crypto';
import {
  MigrationExperiment,
  MigrationMapping,
  CreateExperimentRequest,
  UpdateExperimentRequest,
  ActivateExperimentRequest,
  ExperimentArm,
  Guardrails,
  MigrationImport,
  EvaluateGuardrailsResult,
  ExperimentMode,
  MappingStatus,
  MappingConfidence,
  MigrationImportSummary,
  MigrationSignedComparison,
  DryRunSimulationRequest,
  DryRunSimulationResult,
} from '../types/migration';
import { AppError } from '../middleware/errorHandler';
import logger from '../utils/logger';
import { parseMigrationCsv } from '../utils/migrationCsvParser';
import {
  fetchAppLovinSetup,
  fetchIronSourceSetup,
  NormalizedMappingRow,
} from './migrationImportConnectors';
import { generateSignedComparison, generateSignedReportComparison } from './migrationComparisonSigner';
import { migrationGuardrailPausesTotal, migrationKillsTotal } from '../utils/prometheus';
import { MigrationNotificationService } from './migrationNotificationService';

const DEFAULT_GUARDRAILS: Guardrails = {
  latency_budget_ms: 500,
  revenue_floor_percent: -10,
  max_error_rate_percent: 5,
  min_impressions: 1000,
};

const ADAPTER_SUGGESTIONS: Record<string, { id: string; name: string; confidence: MappingConfidence }> = {
  applovin: {
    id: 'apex_adapter_applovin_max_v1',
    name: 'Apex · AppLovin MAX',
    confidence: 'high',
  },
  max: {
    id: 'apex_adapter_applovin_max_v1',
    name: 'Apex · AppLovin MAX',
    confidence: 'high',
  },
  ironsource: {
    id: 'apex_adapter_ironsource_v1',
    name: 'Apex · ironSource',
    confidence: 'high',
  },
  unity: {
    id: 'apex_adapter_unity_ads_v1',
    name: 'Apex · Unity Ads',
    confidence: 'medium',
  },
};

function resolveAdapterSuggestion(network: string | undefined) {
  if (!network) return null;
  const normalized = network.toLowerCase();
  const direct = ADAPTER_SUGGESTIONS[normalized];
  if (direct) return direct;

  const entry = Object.entries(ADAPTER_SUGGESTIONS).find(([alias]) => normalized.includes(alias));
  return entry ? entry[1] : null;
}

export type FeatureFlagSource = 'placement' | 'app' | 'publisher' | 'default';

export interface EffectiveFeatureFlags {
  shadowEnabled: boolean;
  mirroringEnabled: boolean;
  source: FeatureFlagSource;
}

export class MigrationStudioService {
  constructor(private pool: Pool) {}

  private async validatePlacementOwnership(client: PoolClient, placementId: string, publisherId: string): Promise<void> {
    const placementCheck = await client.query(
      `SELECT p.id FROM placements p
         JOIN apps a ON p.app_id = a.id
       WHERE p.id = $1 AND a.publisher_id = $2`,
      [placementId, publisherId]
    );

    if (placementCheck.rows.length === 0) {
      throw new AppError('Placement not found or access denied', 404);
    }
  }

  /**
   * Create a new migration experiment
   */
  async createExperiment(
    publisherId: string,
    userId: string,
    data: CreateExperimentRequest
  ): Promise<MigrationExperiment> {
    const client = await this.pool.connect();
    
    try {
      await client.query('BEGIN');

      // Validate placement exists and belongs to publisher
      if (data.placement_id) {
        await this.validatePlacementOwnership(client, data.placement_id, publisherId);
      }

      const guardrails = { ...DEFAULT_GUARDRAILS, ...(data.guardrails ?? {}) };

      // Create experiment
      const result = await client.query(
        `INSERT INTO migration_experiments (
          publisher_id, name, description, app_id, placement_id, 
          objective, seed, guardrails, created_by
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING *`,
        [
          publisherId,
          data.name,
          data.description || null,
          data.app_id || null,
          data.placement_id || null,
          data.objective || 'revenue_comparison',
          data.seed || randomUUID(),
          JSON.stringify(guardrails),
          userId,
        ]
      );

      const experiment = result.rows[0];

      // Audit log
      await client.query(
        `INSERT INTO migration_audit (
          experiment_id, user_id, action, resource_type, resource_id, new_value
        ) VALUES ($1, $2, $3, $4, $5, $6)`,
        [
          experiment.id,
          userId,
          'create',
          'experiment',
          experiment.id,
          JSON.stringify(experiment),
        ]
      );

      await client.query('COMMIT');
      
      logger.info('Migration experiment created', { experimentId: experiment.id, publisherId });
      return this.mapExperiment(experiment);
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  private parseImportSummary(value: any): MigrationImportSummary {
    const base: MigrationImportSummary = {
      total_mappings: 0,
      status_breakdown: {
        pending: 0,
        confirmed: 0,
        skipped: 0,
        conflict: 0,
      },
      confidence_breakdown: {
        high: 0,
        medium: 0,
        low: 0,
      },
      unique_networks: 0,
    };

    if (!value) {
      return base;
    }

    let parsed = value;
    if (typeof value === 'string') {
      try {
        parsed = JSON.parse(value);
      } catch {
        return base;
      }
    }

    return {
      total_mappings: typeof parsed.total_mappings === 'number' ? parsed.total_mappings : base.total_mappings,
      status_breakdown: {
        pending: parsed.status_breakdown?.pending ?? base.status_breakdown.pending,
        confirmed: parsed.status_breakdown?.confirmed ?? base.status_breakdown.confirmed,
        skipped: parsed.status_breakdown?.skipped ?? base.status_breakdown.skipped,
        conflict: parsed.status_breakdown?.conflict ?? base.status_breakdown.conflict,
      },
      confidence_breakdown: {
        high: parsed.confidence_breakdown?.high ?? base.confidence_breakdown.high,
        medium: parsed.confidence_breakdown?.medium ?? base.confidence_breakdown.medium,
        low: parsed.confidence_breakdown?.low ?? base.confidence_breakdown.low,
      },
      unique_networks: typeof parsed.unique_networks === 'number' ? parsed.unique_networks : base.unique_networks,
    };
  }

  private mapImport(row: any): MigrationImport {
    return {
      id: row.id,
      publisher_id: row.publisher_id,
      experiment_id: row.experiment_id,
      placement_id: row.placement_id,
      source: row.source,
      status: row.status,
      summary: this.parseImportSummary(row.summary),
      error_message: row.error_message,
      created_by: row.created_by,
      created_at: row.created_at,
      updated_at: row.updated_at,
      completed_at: row.completed_at,
    };
  }

  private mapMapping(row: any): MigrationMapping {
    return {
      id: row.id,
      experiment_id: row.experiment_id,
      incumbent_network: row.incumbent_network,
      incumbent_instance_id: row.incumbent_instance_id,
      incumbent_instance_name: row.incumbent_instance_name,
      incumbent_waterfall_position: row.incumbent_waterfall_position,
      incumbent_ecpm_cents: row.incumbent_ecpm_cents,
      our_adapter_id: row.our_adapter_id,
      our_adapter_name: row.our_adapter_name,
      mapping_status: (row.mapping_status || 'pending') as MappingStatus,
      mapping_confidence: (row.mapping_confidence || 'medium') as MappingConfidence,
      conflict_reason: row.conflict_reason,
      resolved_by: row.resolved_by,
      resolved_at: row.resolved_at,
      created_at: row.created_at,
      updated_at: row.updated_at,
    };
  }

  private summarizeMappings(mappings: MigrationMapping[]): MigrationImportSummary {
    const statusBreakdown: Record<MappingStatus, number> = {
      pending: 0,
      confirmed: 0,
      skipped: 0,
      conflict: 0,
    };

    const confidenceBreakdown: Record<MappingConfidence, number> = {
      high: 0,
      medium: 0,
      low: 0,
    };

    const networks = new Set<string>();

    for (const mapping of mappings) {
      const status = (mapping.mapping_status || 'pending') as MappingStatus;
      statusBreakdown[status] = (statusBreakdown[status] ?? 0) + 1;

      const confidence = (mapping.mapping_confidence || 'medium') as MappingConfidence;
      confidenceBreakdown[confidence] = (confidenceBreakdown[confidence] ?? 0) + 1;

      if (mapping.incumbent_network) {
        networks.add(mapping.incumbent_network);
      }
    }

    return {
      total_mappings: mappings.length,
      status_breakdown: statusBreakdown,
      confidence_breakdown: confidenceBreakdown,
      unique_networks: networks.size,
    };
  }

  /**
   * Get experiment by ID (with publisher scope check)
   */
  async getExperiment(experimentId: string, publisherId: string): Promise<MigrationExperiment> {
    const result = await this.pool.query(
      `SELECT * FROM migration_experiments WHERE id = $1 AND publisher_id = $2`,
      [experimentId, publisherId]
    );

    if (result.rows.length === 0) {
      throw new AppError('Experiment not found', 404);
    }

    return this.mapExperiment(result.rows[0]);
  }

  /**
   * List experiments for a publisher
   */
  async listExperiments(
    publisherId: string,
    filters?: { status?: string; placement_id?: string }
  ): Promise<MigrationExperiment[]> {
    let query = 'SELECT * FROM migration_experiments WHERE publisher_id = $1';
    const params: any[] = [publisherId];

    if (filters?.status) {
      params.push(filters.status);
      query += ` AND status = $${params.length}`;
    }

    if (filters?.placement_id) {
      params.push(filters.placement_id);
      query += ` AND placement_id = $${params.length}`;
    }

    query += ' ORDER BY created_at DESC';

    const result = await this.pool.query(query, params);
    return result.rows.map(row => this.mapExperiment(row));
  }

  /**
   * Create or update an import job and draft mappings
   */
  async createImport(options: {
    publisherId: string;
    userId: string;
    placementId: string;
    experimentId?: string;
    source: 'csv' | 'ironSource' | 'applovin';
    fileBuffer?: Buffer;
    credentials?: { api_key: string; account_id: string };
  }): Promise<{
    import: MigrationImport;
    mappings: MigrationMapping[];
    experiment: MigrationExperiment;
    summary: MigrationImportSummary;
    signedComparison: MigrationSignedComparison;
  }> {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      await this.validatePlacementOwnership(client, options.placementId, options.publisherId);

      let experimentId = options.experimentId;
      let experimentRow;

      if (experimentId) {
        const existing = await client.query(
          `SELECT * FROM migration_experiments WHERE id = $1 AND publisher_id = $2`,
          [experimentId, options.publisherId]
        );

        if (existing.rows.length === 0) {
          throw new AppError('Experiment not found', 404);
        }

        experimentRow = existing.rows[0];
      } else {
        const insertExp = await client.query(
          `INSERT INTO migration_experiments (
            publisher_id, name, description, placement_id, objective, seed, mirror_percent, status, guardrails, created_by
          ) VALUES ($1, $2, $3, $4, $5, $6, 0, 'draft', $7, $8)
          RETURNING *`,
          [
            options.publisherId,
            `Migration import ${new Date().toISOString()}`,
            'Draft created from import wizard',
            options.placementId,
            'revenue_comparison',
            randomBytes(8).toString('hex'),
            JSON.stringify(DEFAULT_GUARDRAILS),
            options.userId,
          ]
        );

        experimentRow = insertExp.rows[0];
        experimentId = experimentRow.id;

        await client.query(
          `INSERT INTO migration_audit (
            experiment_id, user_id, action, resource_type, resource_id, new_value
          ) VALUES ($1, $2, $3, $4, $5, $6)`,
          [
            experimentId,
            options.userId,
            'create_import_experiment',
            'experiment',
            experimentId,
            JSON.stringify(experimentRow),
          ]
        );
      }

      let normalized: NormalizedMappingRow[] = [];

      if (options.source === 'csv') {
        if (!options.fileBuffer) {
          throw new AppError('CSV file required for import', 400);
        }
        try {
          normalized = parseMigrationCsv(options.fileBuffer);
        } catch (err) {
          throw new AppError((err as Error).message || 'Unable to parse CSV', 400);
        }
      } else if (options.source === 'ironSource') {
        if (!options.credentials) {
          throw new AppError('ironSource credentials required', 400);
        }
        try {
          normalized = await fetchIronSourceSetup(options.credentials);
        } catch (err) {
          throw new AppError((err as Error).message, 400);
        }
      } else if (options.source === 'applovin') {
        if (!options.credentials) {
          throw new AppError('AppLovin credentials required', 400);
        }
        try {
          normalized = await fetchAppLovinSetup(options.credentials);
        } catch (err) {
          throw new AppError((err as Error).message, 400);
        }
      } else {
        throw new AppError('Unsupported import source', 400);
      }

      if (normalized.length === 0) {
        throw new AppError('No mappings detected in import', 400);
      }

      const importResult = await client.query(
        `INSERT INTO migration_imports (
          publisher_id, experiment_id, placement_id, source, status, summary, created_by
        ) VALUES ($1, $2, $3, $4, 'pending_review', '{}'::jsonb, $5)
        RETURNING *`,
        [options.publisherId, experimentId, options.placementId, options.source, options.userId]
      );

      const insertedMappings: MigrationMapping[] = [];

      for (const row of normalized) {
        const suggestion = resolveAdapterSuggestion(row.network);
        const adapterId = suggestion?.id ?? null;
        const adapterName = suggestion?.name ?? null;
        const mappingStatus: MappingStatus = suggestion ? 'confirmed' : 'pending';
        const mappingConfidence: MappingConfidence = suggestion?.confidence ?? row.confidence ?? 'medium';

        const result = await client.query(
          `INSERT INTO migration_mappings (
            experiment_id,
            incumbent_network,
            incumbent_instance_id,
            incumbent_instance_name,
            incumbent_waterfall_position,
            incumbent_ecpm_cents,
            mapping_status,
            mapping_confidence,
            our_adapter_id,
            our_adapter_name
          ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
          ON CONFLICT (experiment_id, incumbent_instance_id)
          DO UPDATE SET
            incumbent_network = EXCLUDED.incumbent_network,
            incumbent_instance_name = EXCLUDED.incumbent_instance_name,
            incumbent_waterfall_position = EXCLUDED.incumbent_waterfall_position,
            incumbent_ecpm_cents = EXCLUDED.incumbent_ecpm_cents,
            mapping_confidence = EXCLUDED.mapping_confidence,
            our_adapter_id = COALESCE(EXCLUDED.our_adapter_id, migration_mappings.our_adapter_id),
            our_adapter_name = COALESCE(EXCLUDED.our_adapter_name, migration_mappings.our_adapter_name),
            mapping_status = CASE
              WHEN migration_mappings.mapping_status IN ('confirmed','skipped') THEN migration_mappings.mapping_status
              ELSE EXCLUDED.mapping_status
            END,
            updated_at = NOW()
          RETURNING *`,
          [
            experimentId,
            row.network,
            row.instanceId,
            row.instanceName || null,
            row.waterfallPosition ?? null,
            row.ecpmCents ?? null,
            mappingStatus,
            mappingConfidence,
            adapterId,
            adapterName,
          ]
        );

        insertedMappings.push(this.mapMapping(result.rows[0]));
      }

      const summary = this.summarizeMappings(insertedMappings);

      const updatedImport = await client.query(
        `UPDATE migration_imports
         SET summary = $1, updated_at = NOW()
         WHERE id = $2
         RETURNING *`,
        [JSON.stringify(summary), importResult.rows[0].id]
      );

      const experiment = this.mapExperiment(experimentRow);
      const importJob = this.mapImport(updatedImport.rows[0]);

      await client.query('COMMIT');

      const signedComparison = generateSignedComparison(insertedMappings);

      return {
        import: importJob,
        mappings: insertedMappings,
        experiment,
        summary,
        signedComparison,
      };
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  async updateMapping(
    mappingId: string,
    publisherId: string,
    userId: string,
    options: {
      ourAdapterId?: string;
      status?: MappingStatus;
      notes?: string;
    }
  ): Promise<{ mapping: MigrationMapping; summary: MigrationImportSummary }>
  {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      const mappingResult = await client.query(
        `SELECT m.*
         FROM migration_mappings m
         JOIN migration_experiments e ON m.experiment_id = e.id
         WHERE m.id = $1 AND e.publisher_id = $2
         FOR UPDATE`,
        [mappingId, publisherId]
      );

      if (mappingResult.rows.length === 0) {
        throw new AppError('Mapping not found', 404);
      }

      const currentRow = mappingResult.rows[0];
      const experimentId = currentRow.experiment_id;

      const adapterId = options.ourAdapterId?.trim() || null;

      const allowedStatuses: MappingStatus[] = ['pending', 'confirmed', 'skipped', 'conflict'];
      let status: MappingStatus;
      if (options.status) {
        if (!allowedStatuses.includes(options.status)) {
          throw new AppError('Invalid mapping status', 400);
        }
        status = options.status;
      } else {
        status = adapterId ? 'confirmed' : 'pending';
      }

      let conflictReason: string | null = null;
      if (status === 'conflict') {
        conflictReason = options.notes?.trim() || currentRow.conflict_reason || 'Requires review';
      } else if (options.notes !== undefined) {
        conflictReason = options.notes ? options.notes.trim() : null;
      }

      const updated = await client.query(
        `UPDATE migration_mappings
         SET our_adapter_id = $1,
             our_adapter_name = $2,
             mapping_status = $3,
             conflict_reason = $4,
             resolved_by = CASE WHEN $3 IN ('confirmed','skipped') THEN $5 ELSE NULL END,
             resolved_at = CASE WHEN $3 IN ('confirmed','skipped') THEN NOW() ELSE NULL END,
             updated_at = NOW()
         WHERE id = $6
         RETURNING *`,
        [adapterId, adapterId, status, conflictReason, userId, mappingId]
      );

      const { rows: mappingRows } = await client.query(
        `SELECT * FROM migration_mappings WHERE experiment_id = $1`,
        [experimentId]
      );

      const mappedRows = mappingRows.map(row => this.mapMapping(row));
      const summary = this.summarizeMappings(mappedRows);

      const latestImport = await client.query<{ id: string }>(
        `SELECT id FROM migration_imports
         WHERE experiment_id = $1
         ORDER BY created_at DESC
         LIMIT 1`,
        [experimentId]
      );

      if (latestImport.rows[0]) {
        await client.query(
          `UPDATE migration_imports
           SET summary = $1::jsonb,
               updated_at = NOW()
           WHERE id = $2`,
          [JSON.stringify(summary), latestImport.rows[0].id]
        );
      }

      await client.query(
        `INSERT INTO migration_audit (
           experiment_id, user_id, action, resource_type, resource_id, new_value
         ) VALUES ($1, $2, $3, $4, $5, $6)`,
        [
          experimentId,
          userId,
          'update_mapping',
          'mapping',
          mappingId,
          JSON.stringify(updated.rows[0]),
        ]
      );

      await client.query('COMMIT');

      return {
        mapping: this.mapMapping(updated.rows[0]),
        summary,
      };
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  async finalizeImport(
    importId: string,
    publisherId: string,
    userId: string
  ): Promise<{
    import: MigrationImport;
    mappings: MigrationMapping[];
    summary: MigrationImportSummary;
    signedComparison: MigrationSignedComparison;
  }> {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      const importResult = await client.query(
        `SELECT * FROM migration_imports
         WHERE id = $1 AND publisher_id = $2
         FOR UPDATE`,
        [importId, publisherId]
      );

      if (importResult.rows.length === 0) {
        throw new AppError('Import not found', 404);
      }

      const importRow = importResult.rows[0];

      if (!importRow.experiment_id) {
        throw new AppError('Import is not associated with an experiment', 400);
      }

      if (importRow.status !== 'pending_review' && importRow.status !== 'draft') {
        throw new AppError('Import already finalized', 400);
      }

      const { rows: mappingRows } = await client.query(
        `SELECT * FROM migration_mappings WHERE experiment_id = $1`,
        [importRow.experiment_id]
      );

      if (mappingRows.length === 0) {
        throw new AppError('No mappings found for experiment', 400);
      }

      const mappedRows = mappingRows.map(row => this.mapMapping(row));
      const unresolved = mappedRows.filter(mapping => mapping.mapping_status === 'pending');
      if (unresolved.length > 0) {
        throw new AppError(`Resolve ${unresolved.length} pending mappings before finalizing`, 400);
      }

      const conflicts = mappedRows.filter(mapping => mapping.mapping_status === 'conflict');
      if (conflicts.length > 0) {
        throw new AppError('Resolve conflicts before finalizing import', 400);
      }

      const summary = this.summarizeMappings(mappedRows);

      const updatedImport = await client.query(
        `UPDATE migration_imports
         SET status = 'completed',
             summary = $1::jsonb,
             completed_at = NOW(),
             updated_at = NOW()
         WHERE id = $2
         RETURNING *`,
        [JSON.stringify(summary), importId]
      );

      await client.query(
        `UPDATE migration_experiments
         SET mode = 'shadow',
             updated_at = NOW(),
             last_guardrail_check = NOW()
         WHERE id = $1`,
        [importRow.experiment_id]
      );

      await client.query(
        `INSERT INTO migration_audit (
           experiment_id, user_id, action, resource_type, resource_id, new_value
         ) VALUES ($1, $2, $3, $4, $5, $6)`,
        [
          importRow.experiment_id,
          userId,
          'finalize_import',
          'import',
          importId,
          JSON.stringify(updatedImport.rows[0]),
        ]
      );

      await client.query('COMMIT');

      const signedComparison = generateSignedComparison(mappedRows);

      return {
        import: this.mapImport(updatedImport.rows[0]),
        mappings: mappedRows,
        summary,
        signedComparison,
      };
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Update experiment (draft/paused only)
   */
  async updateExperiment(
    experimentId: string,
    publisherId: string,
    userId: string,
    data: UpdateExperimentRequest
  ): Promise<MigrationExperiment> {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      // Get existing experiment
      const existing = await client.query(
        `SELECT * FROM migration_experiments WHERE id = $1 AND publisher_id = $2`,
        [experimentId, publisherId]
      );

      if (existing.rows.length === 0) {
        throw new AppError('Experiment not found', 404);
      }

      const exp = existing.rows[0];

      // Only allow updates to draft or paused experiments
      if (!['draft', 'paused'].includes(exp.status)) {
        throw new AppError('Cannot update active or completed experiment', 400);
      }

      // Build update query
      const updates: string[] = [];
      const params: any[] = [];
      let paramIndex = 1;

      if (data.name !== undefined) {
        updates.push(`name = $${paramIndex++}`);
        params.push(data.name);
      }

      if (data.description !== undefined) {
        updates.push(`description = $${paramIndex++}`);
        params.push(data.description);
      }

      if (data.mirror_percent !== undefined) {
        if (data.mirror_percent < 0 || data.mirror_percent > 20) {
          throw new AppError('mirror_percent must be between 0 and 20', 400);
        }
        updates.push(`mirror_percent = $${paramIndex++}`);
        params.push(data.mirror_percent);
      }

      if (data.guardrails !== undefined) {
        const mergedGuardrails = { ...exp.guardrails, ...data.guardrails };
        updates.push(`guardrails = $${paramIndex++}`);
        params.push(JSON.stringify(mergedGuardrails));
      }

      if (updates.length === 0) {
        throw new AppError('No updates provided', 400);
      }

      updates.push(`updated_at = NOW()`);
      params.push(experimentId, publisherId);

      const result = await client.query(
        `UPDATE migration_experiments 
         SET ${updates.join(', ')}
         WHERE id = $${paramIndex++} AND publisher_id = $${paramIndex++}
         RETURNING *`,
        params
      );

      // Audit log
      await client.query(
        `INSERT INTO migration_audit (
          experiment_id, user_id, action, resource_type, resource_id, old_value, new_value
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)`,
        [
          experimentId,
          userId,
          'update',
          'experiment',
          experimentId,
          JSON.stringify(exp),
          JSON.stringify(result.rows[0]),
        ]
      );

      await client.query('COMMIT');
      
      return this.mapExperiment(result.rows[0]);
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Dry-run simulation: Preview what would happen during activation without making changes
   * 
   * This method performs comprehensive pre-flight checks and simulates the activation
   * to help operators understand the impact before actually starting the experiment.
   */
  async simulateActivation(
    experimentId: string,
    publisherId: string,
    data: DryRunSimulationRequest
  ): Promise<DryRunSimulationResult> {
    const simulationId = randomUUID();
    const simulationHours = data.simulation_hours ?? 24;
    
    // Fetch experiment details
    const expResult = await this.pool.query(
      `SELECT * FROM migration_experiments WHERE id = $1 AND publisher_id = $2`,
      [experimentId, publisherId]
    );
    
    if (expResult.rows.length === 0) {
      throw new AppError('Experiment not found', 404);
    }
    
    const experiment = expResult.rows[0];
    const guardrails = this.parseGuardrails(experiment.guardrails);
    
    // Pre-flight checks
    const preflightChecks: DryRunSimulationResult['preflight']['checks'] = [];
    
    // Check 1: Experiment status
    if (!['draft', 'paused'].includes(experiment.status)) {
      preflightChecks.push({
        name: 'experiment_status',
        status: 'fail',
        message: `Experiment is in '${experiment.status}' status. Only draft or paused experiments can be activated.`,
        severity: 'critical',
      });
    } else {
      preflightChecks.push({
        name: 'experiment_status',
        status: 'pass',
        message: `Experiment is in '${experiment.status}' status and can be activated.`,
        severity: 'info',
      });
    }
    
    // Check 2: Mirror percent validation
    if (data.mirror_percent < 0 || data.mirror_percent > 20) {
      preflightChecks.push({
        name: 'mirror_percent',
        status: 'fail',
        message: `Mirror percent ${data.mirror_percent}% is out of allowed range (0-20%).`,
        severity: 'critical',
      });
    } else if (data.mirror_percent > 10) {
      preflightChecks.push({
        name: 'mirror_percent',
        status: 'warn',
        message: `Mirror percent ${data.mirror_percent}% is aggressive. Consider starting with 5% or less.`,
        severity: 'warning',
      });
    } else {
      preflightChecks.push({
        name: 'mirror_percent',
        status: 'pass',
        message: `Mirror percent ${data.mirror_percent}% is within safe range.`,
        severity: 'info',
      });
    }
    
    // Fetch mapping information
    const mappingsResult = await this.pool.query(
      `SELECT mapping_status, mapping_confidence, incumbent_network, our_adapter_id
       FROM migration_mappings WHERE experiment_id = $1`,
      [experimentId]
    );
    
    const mappings = mappingsResult.rows;
    const confirmedMappings = mappings.filter(m => m.mapping_status === 'confirmed');
    const pendingMappings = mappings.filter(m => m.mapping_status === 'pending');
    const skippedMappings = mappings.filter(m => m.mapping_status === 'skipped');
    const conflictMappings = mappings.filter(m => m.mapping_status === 'conflict');
    
    // Check 3: Mapping coverage
    const coveragePercent = mappings.length > 0 
      ? (confirmedMappings.length / mappings.length) * 100 
      : 0;
    
    if (confirmedMappings.length === 0) {
      preflightChecks.push({
        name: 'mapping_coverage',
        status: 'fail',
        message: 'No confirmed mappings. At least one mapping must be confirmed before activation.',
        severity: 'critical',
      });
    } else if (coveragePercent < 50) {
      preflightChecks.push({
        name: 'mapping_coverage',
        status: 'warn',
        message: `Only ${coveragePercent.toFixed(1)}% of mappings are confirmed. Consider confirming more mappings.`,
        severity: 'warning',
      });
    } else {
      preflightChecks.push({
        name: 'mapping_coverage',
        status: 'pass',
        message: `${confirmedMappings.length} of ${mappings.length} mappings confirmed (${coveragePercent.toFixed(1)}%).`,
        severity: 'info',
      });
    }
    
    // Check 4: Conflicts
    if (conflictMappings.length > 0) {
      preflightChecks.push({
        name: 'mapping_conflicts',
        status: 'warn',
        message: `${conflictMappings.length} mapping conflict(s) need resolution.`,
        severity: 'warning',
      });
    } else {
      preflightChecks.push({
        name: 'mapping_conflicts',
        status: 'pass',
        message: 'No mapping conflicts detected.',
        severity: 'info',
      });
    }
    
    // Fetch traffic estimates from recent data
    const trafficResult = await this.pool.query(
      `SELECT 
         COUNT(DISTINCT placement_id) as placements,
         COUNT(DISTINCT app_id) as apps,
         SUM(impressions) as total_impressions
       FROM analytics_impressions
       WHERE publisher_id = $1
         AND ts >= NOW() - INTERVAL '24 hours'`,
      [publisherId]
    );
    
    const trafficData = trafficResult.rows[0];
    const estimatedDailyRequests = Number(trafficData?.total_impressions || 0);
    const estimatedSimulationRequests = Math.round(estimatedDailyRequests * (simulationHours / 24));
    const estimatedTestRequests = Math.round(estimatedSimulationRequests * (data.mirror_percent / 100));
    
    // Check 5: Traffic volume
    if (estimatedDailyRequests === 0) {
      preflightChecks.push({
        name: 'traffic_volume',
        status: 'warn',
        message: 'No recent traffic data found. Ensure placements are active before activation.',
        severity: 'warning',
      });
    } else {
      preflightChecks.push({
        name: 'traffic_volume',
        status: 'pass',
        message: `Estimated ${estimatedTestRequests.toLocaleString()} test requests over ${simulationHours} hours.`,
        severity: 'info',
      });
    }
    
    // Check 6: Guardrail configuration
    if (guardrails.latency_budget_ms < 200) {
      preflightChecks.push({
        name: 'guardrails_latency',
        status: 'warn',
        message: `Latency budget (${guardrails.latency_budget_ms}ms) is aggressive. May trigger false positives.`,
        severity: 'warning',
      });
    } else {
      preflightChecks.push({
        name: 'guardrails_latency',
        status: 'pass',
        message: `Latency budget set to ${guardrails.latency_budget_ms}ms.`,
        severity: 'info',
      });
    }
    
    // Unmapped networks
    const unmappedNetworks = [...new Set(
      pendingMappings
        .filter(m => !m.our_adapter_id)
        .map(m => m.incumbent_network)
        .filter(Boolean)
    )];
    
    // Guardrail status assessment
    const getLatencyStatus = (ms: number): 'conservative' | 'moderate' | 'aggressive' => {
      if (ms >= 500) return 'conservative';
      if (ms >= 300) return 'moderate';
      return 'aggressive';
    };
    
    const getRevenueFloorStatus = (percent: number): 'conservative' | 'moderate' | 'aggressive' => {
      if (percent >= -5) return 'conservative';
      if (percent >= -15) return 'moderate';
      return 'aggressive';
    };
    
    const getErrorRateStatus = (percent: number): 'conservative' | 'moderate' | 'aggressive' => {
      if (percent <= 1) return 'aggressive';
      if (percent <= 5) return 'moderate';
      return 'conservative';
    };
    
    // Estimate time to minimum impressions
    const hourlyTestRequests = estimatedDailyRequests / 24 * (data.mirror_percent / 100);
    const timeToMinImpressions = hourlyTestRequests > 0 
      ? guardrails.min_impressions / hourlyTestRequests 
      : Infinity;
    
    // Risk factors
    const riskFactors: DryRunSimulationResult['risk_assessment']['risk_factors'] = [];
    
    if (data.mirror_percent > 10) {
      riskFactors.push({
        factor: 'High traffic percentage',
        level: 'medium',
        description: `${data.mirror_percent}% of traffic will be routed through test. Higher risk if issues occur.`,
        mitigation: 'Consider starting with 5% or less.',
      });
    }
    
    if (coveragePercent < 80 && confirmedMappings.length > 0) {
      riskFactors.push({
        factor: 'Incomplete mappings',
        level: 'medium',
        description: `Only ${coveragePercent.toFixed(1)}% of mappings confirmed. Traffic may hit unmapped adapters.`,
        mitigation: 'Review and confirm remaining mappings before activation.',
      });
    }
    
    if (conflictMappings.length > 0) {
      riskFactors.push({
        factor: 'Unresolved conflicts',
        level: 'high',
        description: `${conflictMappings.length} mapping conflicts may cause routing errors.`,
        mitigation: 'Resolve all conflicts before activation.',
      });
    }
    
    if (timeToMinImpressions > 24) {
      riskFactors.push({
        factor: 'Low traffic volume',
        level: 'low',
        description: `May take ${Math.ceil(timeToMinImpressions)} hours to reach minimum impressions for guardrail evaluation.`,
        mitigation: 'Consider increasing mirror percentage or waiting for higher traffic periods.',
      });
    }
    
    // Overall risk
    const highRiskCount = riskFactors.filter(r => r.level === 'high').length;
    const mediumRiskCount = riskFactors.filter(r => r.level === 'medium').length;
    const overallRisk: 'low' | 'medium' | 'high' = 
      highRiskCount > 0 ? 'high' : 
      mediumRiskCount > 1 ? 'high' :
      mediumRiskCount > 0 ? 'medium' : 
      'low';
    
    // Recommendations
    const recommendations: DryRunSimulationResult['recommendations'] = [];
    
    if (preflightChecks.some(c => c.status === 'fail')) {
      recommendations.push({
        type: 'action',
        message: 'Address all critical pre-flight check failures before activation.',
        priority: 'high',
      });
    }
    
    if (coveragePercent < 100 && pendingMappings.length > 0) {
      recommendations.push({
        type: 'action',
        message: `Review and confirm ${pendingMappings.length} pending mapping(s) for better coverage.`,
        priority: 'medium',
      });
    }
    
    if (data.mirror_percent > 5 && experiment.status === 'draft') {
      recommendations.push({
        type: 'warning',
        message: 'Consider starting with 1-5% mirror for initial validation.',
        priority: 'medium',
      });
    }
    
    recommendations.push({
      type: 'info',
      message: 'Guardrails will automatically pause the experiment if thresholds are exceeded.',
      priority: 'low',
    });
    
    // Build result
    const result: DryRunSimulationResult = {
      simulation_id: simulationId,
      experiment_id: experimentId,
      mirror_percent: data.mirror_percent,
      simulation_hours: simulationHours,
      timestamp: new Date(),
      
      preflight: {
        passed: !preflightChecks.some(c => c.status === 'fail'),
        checks: preflightChecks,
      },
      
      traffic_impact: {
        estimated_total_requests: estimatedSimulationRequests,
        estimated_test_requests: estimatedTestRequests,
        estimated_control_requests: estimatedSimulationRequests - estimatedTestRequests,
        test_percentage: data.mirror_percent,
        placements_affected: Number(trafficData?.placements || 0),
        apps_affected: Number(trafficData?.apps || 0),
      },
      
      mapping_readiness: {
        total_mappings: mappings.length,
        confirmed_mappings: confirmedMappings.length,
        pending_mappings: pendingMappings.length,
        skipped_mappings: skippedMappings.length,
        conflict_mappings: conflictMappings.length,
        coverage_percent: coveragePercent,
        unmapped_networks: unmappedNetworks,
      },
      
      guardrail_review: {
        latency_budget_ms: guardrails.latency_budget_ms,
        latency_budget_status: getLatencyStatus(guardrails.latency_budget_ms),
        revenue_floor_percent: guardrails.revenue_floor_percent,
        revenue_floor_status: getRevenueFloorStatus(guardrails.revenue_floor_percent),
        max_error_rate_percent: guardrails.max_error_rate_percent,
        error_rate_status: getErrorRateStatus(guardrails.max_error_rate_percent),
        min_impressions: guardrails.min_impressions,
        estimated_time_to_min_impressions_hours: Math.ceil(timeToMinImpressions),
      },
      
      risk_assessment: {
        overall_risk: overallRisk,
        risk_factors: riskFactors,
      },
      
      rollback_plan: {
        automatic_rollback_enabled: true,
        rollback_triggers: [
          `Latency exceeds ${guardrails.latency_budget_ms}ms`,
          `Revenue drops below ${guardrails.revenue_floor_percent}% floor`,
          `Error rate exceeds ${guardrails.max_error_rate_percent}%`,
        ],
        estimated_rollback_time_seconds: 5,
      },
      
      recommendations,
    };
    
    logger.info('Dry-run simulation completed', {
      simulationId,
      experimentId,
      preflightPassed: result.preflight.passed,
      overallRisk,
    });
    
    return result;
  }

  /**
   * Activate experiment (start mirroring traffic)
   */
  async activateExperiment(
    experimentId: string,
    publisherId: string,
    userId: string,
    data: ActivateExperimentRequest
  ): Promise<MigrationExperiment> {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      const result = await client.query(
        `UPDATE migration_experiments
         SET status = 'active',
             mode = 'mirroring',
             mirror_percent = $1,
             activated_at = NOW(),
             last_guardrail_check = NOW(),
             updated_at = NOW()
         WHERE id = $2 AND publisher_id = $3 AND status IN ('draft', 'paused')
         RETURNING *`,
        [data.mirror_percent, experimentId, publisherId]
      );

      if (result.rows.length === 0) {
        throw new AppError('Experiment not found or cannot be activated', 404);
      }

      // Log activation event
      await client.query(
        `INSERT INTO migration_events (experiment_id, event_type, reason, triggered_by)
         VALUES ($1, $2, $3, $4)`,
        [experimentId, 'activation', `Activated with ${data.mirror_percent}% mirror`, userId]
      );

      // Audit log
      await client.query(
        `INSERT INTO migration_audit (experiment_id, user_id, action, resource_type, resource_id)
         VALUES ($1, $2, $3, $4, $5)`,
        [experimentId, userId, 'activate', 'experiment', experimentId]
      );

      await client.query('COMMIT');

      logger.info('Experiment activated', { experimentId, mirrorPercent: data.mirror_percent });
      return this.mapExperiment(result.rows[0]);
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Pause experiment (stop traffic mirroring)
   */
  async pauseExperiment(
    experimentId: string,
    publisherId: string,
    userId: string,
    reason?: string
  ): Promise<MigrationExperiment> {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      const result = await client.query(
        `UPDATE migration_experiments
         SET status = 'paused',
             mode = 'shadow',
             paused_at = NOW(),
             updated_at = NOW()
         WHERE id = $1 AND publisher_id = $2 AND status = 'active'
         RETURNING *`,
        [experimentId, publisherId]
      );

      if (result.rows.length === 0) {
        throw new AppError('Experiment not found or not active', 404);
      }

      // Log deactivation event
      await client.query(
        `INSERT INTO migration_events (experiment_id, event_type, reason, triggered_by)
         VALUES ($1, $2, $3, $4)`,
        [experimentId, 'deactivation', reason || 'Manual pause', userId]
      );

      await client.query('COMMIT');

      logger.info('Experiment paused', { experimentId, reason });
      return this.mapExperiment(result.rows[0]);
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Deterministic assignment: hash(user, placement, seed) < mirror_percent → test
   */
  async getEffectiveFeatureFlags(
    publisherId: string,
    appId?: string | null,
    placementId?: string | null
  ): Promise<EffectiveFeatureFlags> {
    const { rows } = await this.pool.query(
      `SELECT
         placement_id::text AS placement_id,
         app_id::text AS app_id,
         shadow_enabled,
         mirroring_enabled
       FROM migration_feature_flags
       WHERE publisher_id = $1
         AND (app_id IS NULL OR app_id = $2::uuid)
         AND (placement_id IS NULL OR placement_id = $3::uuid)
       ORDER BY updated_at DESC`,
      [publisherId, appId ?? null, placementId ?? null]
    );

    if (!rows || rows.length === 0) {
      return { shadowEnabled: false, mirroringEnabled: false, source: 'default' };
    }

    const placementRow = placementId
      ? rows.find((row: any) => row.placement_id && row.placement_id === placementId)
      : undefined;

    if (placementRow) {
      return {
        shadowEnabled: Boolean(placementRow.shadow_enabled),
        mirroringEnabled: Boolean(placementRow.mirroring_enabled),
        source: 'placement',
      };
    }

    const appRow = appId
      ? rows.find((row: any) => !row.placement_id && row.app_id && row.app_id === appId)
      : undefined;

    if (appRow) {
      return {
        shadowEnabled: Boolean(appRow.shadow_enabled),
        mirroringEnabled: Boolean(appRow.mirroring_enabled),
        source: 'app',
      };
    }

    const publisherRow = rows.find((row: any) => !row.placement_id && !row.app_id);
    if (publisherRow) {
      return {
        shadowEnabled: Boolean(publisherRow.shadow_enabled),
        mirroringEnabled: Boolean(publisherRow.mirroring_enabled),
        source: 'publisher',
      };
    }

    return { shadowEnabled: false, mirroringEnabled: false, source: 'default' };
  }

  /**
   * Deterministic assignment: hash(user, placement, seed) < mirror_percent → test
   */
  assignArm(
    userIdentifier: string,
    placementId: string,
    seed: string,
    mirrorPercent: number
  ): ExperimentArm {
    const hash = createHash('sha256')
      .update(`${userIdentifier}:${placementId}:${seed}`)
      .digest('hex');

    // Take first 8 hex chars and convert to percentage (0-100)
    const hashInt = parseInt(hash.substring(0, 8), 16);
    const percentage = (hashInt % 100) / 100; // 0.00 to 0.99

    return percentage < (mirrorPercent / 100) ? 'test' : 'control';
  }

  /**
   * Log assignment decision
   */
  async logAssignment(
    experimentId: string,
    arm: ExperimentArm,
    userIdentifier: string,
    placementId: string,
    metadata?: Record<string, any>
  ): Promise<void> {
    // Hash user identifier for privacy
    const hashedUser = createHash('sha256').update(userIdentifier).digest('hex');

    await this.pool.query(
      `INSERT INTO migration_events (
        experiment_id, event_type, arm, user_identifier, placement_id, event_data
      ) VALUES ($1, $2, $3, $4, $5, $6)`,
      [
        experimentId,
        'assignment',
        arm,
        hashedUser,
        placementId,
        JSON.stringify(metadata || {}),
      ]
    );
  }

  /**
   * Evaluate guardrails and pause if violated
   */
  async evaluateGuardrails(
    experimentId: string,
    publisherId: string,
    triggeredBy?: string
  ): Promise<EvaluateGuardrailsResult> {
    // FIX-11-637: Avoid long-lived locks. Do read-only work outside a transaction,
    // and open a short transaction only for state updates.
    const expRead = await this.pool.query(
      `SELECT * FROM migration_experiments
       WHERE id = $1 AND publisher_id = $2`,
      [experimentId, publisherId]
    );

    if (expRead.rows.length === 0) {
      throw new AppError('Experiment not found', 404);
    }

    const experimentRow = expRead.rows[0];
    const guardrails = this.parseGuardrails(experimentRow.guardrails);

    const snapshotsResult = await this.pool.query(
      `SELECT arm, impressions, fills, revenue_micros, latency_p95_ms, error_rate_percent
       FROM migration_guardrail_snapshots
       WHERE experiment_id = $1 AND captured_at >= NOW() - INTERVAL '6 hours'
       ORDER BY captured_at DESC
       LIMIT 50`,
      [experimentId]
    );

    if (snapshotsResult.rows.length === 0) {
      await this.pool.query(
        `UPDATE migration_experiments
         SET last_guardrail_check = NOW(), updated_at = NOW()
         WHERE id = $1`,
        [experimentId]
      );
      return { shouldPause: false, violations: [] };
    }

      type Aggregation = {
        impressions: number;
        revenueMicros: number;
        latencies: number[];
        errors: number[];
      };

      const aggregate: Record<ExperimentArm, Aggregation> = {
        control: { impressions: 0, revenueMicros: 0, latencies: [], errors: [] },
        test: { impressions: 0, revenueMicros: 0, latencies: [], errors: [] },
      };

      for (const snapshot of snapshotsResult.rows) {
        const arm = snapshot.arm as ExperimentArm;
        const bucket = aggregate[arm];
        bucket.impressions += Number(snapshot.impressions || 0);
        bucket.revenueMicros += Number(snapshot.revenue_micros || 0);

        if (typeof snapshot.latency_p95_ms === 'number') {
          bucket.latencies.push(snapshot.latency_p95_ms);
        }

        if (typeof snapshot.error_rate_percent === 'number') {
          bucket.errors.push(Number(snapshot.error_rate_percent));
        }
      }

      const minImpressions = guardrails.min_impressions ?? DEFAULT_GUARDRAILS.min_impressions;
      if (aggregate.test.impressions < minImpressions) {
        await this.pool.query(
          `UPDATE migration_experiments
           SET last_guardrail_check = NOW(), updated_at = NOW()
           WHERE id = $1`,
          [experimentId]
        );
        return { shouldPause: false, violations: [] };
      }

      const average = (values: number[]): number | null => {
        if (values.length === 0) return null;
        return values.reduce((sum, value) => sum + value, 0) / values.length;
      };

      const maxLatency = (values: number[]): number | null => {
        if (values.length === 0) return null;
        return Math.max(...values);
      };

      const testLatency = maxLatency(aggregate.test.latencies);
      const testErrorRate = average(aggregate.test.errors);
      const controlEcpm = aggregate.control.impressions > 0
        ? (aggregate.control.revenueMicros / aggregate.control.impressions) / 1000
        : null;
      const testEcpm = aggregate.test.impressions > 0
        ? (aggregate.test.revenueMicros / aggregate.test.impressions) / 1000
        : null;

      const violations: string[] = [];
      const criticalViolations: string[] = [];

      // Hard stop: latency budget violation (critical)
      if (guardrails.latency_budget_ms && testLatency && testLatency > guardrails.latency_budget_ms) {
        const violation = `Latency ${testLatency}ms exceeds budget ${guardrails.latency_budget_ms}ms`;
        violations.push(violation);
        criticalViolations.push(violation);
      }

      // Error rate threshold (pause)
      if (guardrails.max_error_rate_percent && testErrorRate && testErrorRate > guardrails.max_error_rate_percent) {
        violations.push(`Error rate ${testErrorRate.toFixed(2)}% exceeds max ${guardrails.max_error_rate_percent}%`);
      }

      // Revenue floor protection (critical kill switch)
      if (
        typeof guardrails.revenue_floor_percent === 'number' &&
        controlEcpm !== null &&
        controlEcpm > 0 &&
        testEcpm !== null
      ) {
        const revenueDelta = ((testEcpm - controlEcpm) / controlEcpm) * 100;
        if (revenueDelta < guardrails.revenue_floor_percent) {
          const violation = `Revenue delta ${revenueDelta.toFixed(2)}% below floor ${guardrails.revenue_floor_percent}%`;
          violations.push(violation);
          criticalViolations.push(violation);
        }
      }

      const shouldPause = violations.length > 0;
      const shouldKill = criticalViolations.length > 0;

      // Short transaction for updates only
      const client = await this.pool.connect();
      try {
        await client.query('BEGIN');

        if (shouldPause && experimentRow.status === 'active') {
          const eventType = shouldKill ? 'guardrail_kill' : 'guardrail_pause';
          const newMode: ExperimentMode = 'shadow'; // Always revert to shadow mode on guardrail trigger

          await client.query(
            `UPDATE migration_experiments
             SET status = 'paused',
                 mode = $1,
                 paused_at = COALESCE(paused_at, NOW()),
                 last_guardrail_check = NOW(),
                 updated_at = NOW()
             WHERE id = $2`,
            [newMode, experimentId]
          );

          await client.query(
            `INSERT INTO migration_events (
               experiment_id, event_type, reason, triggered_by, event_data
             ) VALUES ($1, $2, $3, $4, $5)`,
            [
              experimentId,
              eventType,
              violations.join('; '),
              triggeredBy || 'system',
              JSON.stringify({
                violations,
                critical_violations: criticalViolations,
                metrics: {
                  testLatency,
                  testErrorRate,
                  testEcpm,
                  controlEcpm,
                },
              }),
            ]
          );

          // Prometheus instrumentation
          if (shouldKill) {
            migrationKillsTotal.inc({ reason: criticalViolations[0].split(' ')[0].toLowerCase() });
            logger.error('Migration experiment kill switch triggered', {
              experimentId,
              violations: criticalViolations,
            });
          } else {
            migrationGuardrailPausesTotal.inc({ reason: violations[0].split(' ')[0].toLowerCase() });
            logger.warn('Migration experiment auto-paused', {
              experimentId,
              violations,
            });
          }

          // Send guardrail violation notification (non-blocking)
          const notificationService = new MigrationNotificationService(this.pool);
          notificationService.sendGuardrailViolationAlert({
            experimentId,
            experimentName: experimentRow.name,
            publisherId,
            violations,
            action: shouldKill ? 'killed' : 'paused',
            timestamp: new Date().toISOString(),
          }).catch(err => {
            logger.warn('Failed to send guardrail violation notification', {
              experimentId,
              error: err.message,
            });
          });
        } else {
          await client.query(
            `UPDATE migration_experiments
             SET last_guardrail_check = NOW(), updated_at = NOW()
             WHERE id = $1`,
            [experimentId]
          );
        }

        await client.query('COMMIT');
        return { shouldPause, violations };
      } catch (error) {
        await client.query('ROLLBACK');
        throw error;
      } finally {
        client.release();
      }
  }

  /**
   * Delete experiment (soft delete to archived)
   */
  async deleteExperiment(
    experimentId: string,
    publisherId: string,
    userId: string
  ): Promise<void> {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      const result = await client.query(
        `UPDATE migration_experiments
         SET status = 'archived', updated_at = NOW()
         WHERE id = $1 AND publisher_id = $2 AND status IN ('draft', 'paused', 'completed')
         RETURNING id`,
        [experimentId, publisherId]
      );

      if (result.rows.length === 0) {
        throw new AppError('Experiment not found or cannot be deleted', 404);
      }

      // Audit log
      await client.query(
        `INSERT INTO migration_audit (experiment_id, user_id, action, resource_type, resource_id)
         VALUES ($1, $2, $3, $4, $5)`,
        [experimentId, userId, 'delete', 'experiment', experimentId]
      );

      await client.query('COMMIT');
      logger.info('Experiment archived', { experimentId });
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Generate side-by-side comparison report with signed verification
   */
  async generateReport(experimentId: string, publisherId: string): Promise<any> {
    const client = await this.pool.connect();

    try {
      // Get experiment
      const experimentResult = await client.query(
        `SELECT * FROM migration_experiments
         WHERE id = $1 AND publisher_id = $2`,
        [experimentId, publisherId]
      );

      if (experimentResult.rows.length === 0) {
        throw new AppError('Experiment not found', 404);
      }

      const experiment = this.mapExperiment(experimentResult.rows[0]);

      // Get aggregated metrics from snapshots (last 14 days)
      const snapshotsResult = await client.query(
        `SELECT 
           arm,
           MIN(captured_at) AS start_date,
           MAX(captured_at) AS end_date,
           SUM(impressions) AS total_impressions,
           SUM(fills) AS total_fills,
           SUM(revenue_micros) AS total_revenue_micros,
           AVG(latency_p50_ms) AS avg_latency_p50,
           AVG(latency_p95_ms) AS avg_latency_p95,
           AVG(latency_p99_ms) AS avg_latency_p99,
           AVG(error_rate_percent) AS avg_error_rate,
           AVG(ivt_rate_percent) AS avg_ivt_rate
         FROM migration_guardrail_snapshots
         WHERE experiment_id = $1 AND captured_at >= NOW() - INTERVAL '14 days'
         GROUP BY arm`,
        [experimentId]
      );

      if (snapshotsResult.rows.length === 0) {
        throw new AppError('No metrics data available for this experiment', 404);
      }

      // Organize by arm
      const metricsByArm: Record<string, any> = {};
      let start: Date | null = null;
      let end: Date | null = null;

      for (const row of snapshotsResult.rows) {
        metricsByArm[row.arm] = {
          arm: row.arm,
          impressions: Number(row.total_impressions || 0),
          fills: Number(row.total_fills || 0),
          revenue_micros: Number(row.total_revenue_micros || 0),
          latency_p50: Number(row.avg_latency_p50 || 0),
          latency_p95: Number(row.avg_latency_p95 || 0),
          latency_p99: Number(row.avg_latency_p99 || 0),
          error_rate: Number(row.avg_error_rate || 0),
          ivt_rate: Number(row.avg_ivt_rate || 0),
        };

        if (!start || row.start_date < start) start = row.start_date;
        if (!end || row.end_date > end) end = row.end_date;
      }

      const control = metricsByArm['control'];
      const test = metricsByArm['test'];

      if (!control || !test) {
        throw new AppError('Incomplete metrics data (missing control or test arm)', 400);
      }

      // Calculate uplift
      const controlEcpm = control.impressions > 0
        ? control.revenue_micros / control.impressions
        : 0;
      const testEcpm = test.impressions > 0
        ? test.revenue_micros / test.impressions
        : 0;
      const revenueUpliftPercent = controlEcpm > 0
        ? ((testEcpm - controlEcpm) / controlEcpm) * 100
        : 0;

      const controlFillRate = control.impressions > 0
        ? (control.fills / control.impressions) * 100
        : 0;
      const testFillRate = test.impressions > 0
        ? (test.fills / test.impressions) * 100
        : 0;
      const fillUpliftPercent = controlFillRate > 0
        ? ((testFillRate - controlFillRate) / controlFillRate) * 100
        : 0;

      const latencyDeltaMs = test.latency_p95 - control.latency_p95;

      const ivtReductionPercent = control.ivt_rate > 0
        ? ((control.ivt_rate - test.ivt_rate) / control.ivt_rate) * 100
        : 0;

      // Statistical significance (simplified t-test approximation)
      // For production, use proper variance calculation from stratified data
      const controlVariance = control.impressions > 100 ? control.impressions * 0.05 : control.impressions;
      const testVariance = test.impressions > 100 ? test.impressions * 0.05 : test.impressions;
      const pooledStdErr = Math.sqrt((controlVariance + testVariance) / 2);
      const zScore = pooledStdErr > 0 ? Math.abs(testEcpm - controlEcpm) / pooledStdErr : 0;
      const revenuePvalue = zScore > 1.96 ? 0.01 : 0.10; // Simplified
      const fillPvalue = zScore > 1.96 ? 0.01 : 0.10;
      const confident = revenuePvalue < 0.05;

      // Projection: if 100% of traffic routed to test
      const totalControlRevenueMicros = control.revenue_micros;
      const projectedRevenueIfAllTest = totalControlRevenueMicros * (1 + revenueUpliftPercent / 100);
      const revenueUpliftMicros = projectedRevenueIfAllTest - totalControlRevenueMicros;

      // Build report
      const report = {
        experiment,
        period: {
          start: start || new Date(),
          end: end || new Date(),
        },
        control,
        test,
        uplift: {
          revenue_percent: Number(revenueUpliftPercent.toFixed(2)),
          fill_percent: Number(fillUpliftPercent.toFixed(2)),
          latency_p95_ms: Number(latencyDeltaMs.toFixed(0)),
          ivt_reduction_percent: Number(ivtReductionPercent.toFixed(2)),
        },
        statistical_significance: {
          revenue_pvalue: Number(revenuePvalue.toFixed(4)),
          fill_pvalue: Number(fillPvalue.toFixed(4)),
          confident,
        },
        projection: {
          if_100_percent_test: {
            revenue_uplift_micros: Math.round(revenueUpliftMicros),
            revenue_uplift_percent: Number(revenueUpliftPercent.toFixed(2)),
          },
        },
      };

      // Generate signed comparison artifact
      const signedComparison = generateSignedReportComparison({
        experiment_id: experimentId,
        control_ecpm_micros: Math.round(controlEcpm),
        test_ecpm_micros: Math.round(testEcpm),
        control_fill_rate: Number(controlFillRate.toFixed(4)),
        test_fill_rate: Number(testFillRate.toFixed(4)),
        control_latency_p95_ms: control.latency_p95,
        test_latency_p95_ms: test.latency_p95,
        control_ivt_rate: control.ivt_rate,
        test_ivt_rate: test.ivt_rate,
        control_impressions: control.impressions,
        test_impressions: test.impressions,
        start_date: start?.toISOString() || new Date().toISOString(),
        end_date: end?.toISOString() || new Date().toISOString(),
      });

      return {
        report,
        signed_comparison: signedComparison,
      };
    } finally {
      client.release();
    }
  }

  private parseGuardrails(value: any): Guardrails {
    if (!value) {
      return { ...DEFAULT_GUARDRAILS };
    }

    let parsed = value;
    if (typeof value === 'string') {
      try {
        parsed = JSON.parse(value);
      } catch {
        return { ...DEFAULT_GUARDRAILS };
      }
    }

    return { ...DEFAULT_GUARDRAILS, ...parsed };
  }

  private mapExperiment(row: any): MigrationExperiment {
    const guardrails = this.parseGuardrails(row.guardrails);
    const mode = (row.mode as ExperimentMode) || 'shadow';

    return {
      id: row.id,
      publisher_id: row.publisher_id,
      name: row.name,
      description: row.description,
      app_id: row.app_id,
      placement_id: row.placement_id,
      objective: row.objective,
      seed: row.seed,
      mirror_percent: row.mirror_percent,
      mode,
      status: row.status,
      activated_at: row.activated_at,
      paused_at: row.paused_at,
      completed_at: row.completed_at,
      guardrails,
      created_by: row.created_by,
      created_at: row.created_at,
      updated_at: row.updated_at,
      last_guardrail_check: row.last_guardrail_check,
    };
  }

  /**
   * Complete migration - set to 100% and mark as completed
   */
  async completeMigration(
    experimentId: string,
    publisherId: string,
    userId: string
  ): Promise<MigrationExperiment> {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      // Verify experiment exists and is active
      const existing = await client.query(
        `SELECT * FROM migration_experiments WHERE id = $1 AND publisher_id = $2`,
        [experimentId, publisherId]
      );

      if (existing.rows.length === 0) {
        throw new AppError('Experiment not found', 404);
      }

      const exp = existing.rows[0];
      if (exp.status !== 'active') {
        throw new AppError('Experiment must be active to complete migration', 400);
      }

      // Update to 100% and completed status
      const result = await client.query(
        `UPDATE migration_experiments
         SET status = 'completed',
             mode = 'mirroring',
             mirror_percent = 100,
             completed_at = NOW(),
             updated_at = NOW()
         WHERE id = $1
         RETURNING *`,
        [experimentId]
      );

      // Log completion event
      await client.query(
        `INSERT INTO migration_events (experiment_id, event_type, reason, triggered_by)
         VALUES ($1, $2, $3, $4)`,
        [experimentId, 'deactivation', 'Migration completed - 100% traffic on Apex', userId]
      );

      // Audit log
      await client.query(
        `INSERT INTO migration_audit (experiment_id, user_id, action, resource_type, resource_id, new_value)
         VALUES ($1, $2, $3, $4, $5, $6)`,
        [experimentId, userId, 'complete', 'experiment', experimentId, JSON.stringify(result.rows[0])]
      );

      await client.query('COMMIT');

      logger.info('Migration completed', { experimentId, publisherId });
      return this.mapExperiment(result.rows[0]);
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  /**
   * Rollback migration - revert to control (pause and reset)
   */
  async rollbackMigration(
    experimentId: string,
    publisherId: string,
    userId: string,
    reason?: string
  ): Promise<MigrationExperiment> {
    const client = await this.pool.connect();

    try {
      await client.query('BEGIN');

      // Verify experiment exists
      const existing = await client.query(
        `SELECT * FROM migration_experiments WHERE id = $1 AND publisher_id = $2`,
        [experimentId, publisherId]
      );

      if (existing.rows.length === 0) {
        throw new AppError('Experiment not found', 404);
      }

      const exp = existing.rows[0];
      if (!['active', 'completed'].includes(exp.status)) {
        throw new AppError('Experiment must be active or completed to rollback', 400);
      }

      // Rollback to paused with 0% mirror
      const result = await client.query(
        `UPDATE migration_experiments
         SET status = 'paused',
             mode = 'shadow',
             mirror_percent = 0,
             paused_at = NOW(),
             updated_at = NOW()
         WHERE id = $1
         RETURNING *`,
        [experimentId]
      );

      // Log rollback event
      await client.query(
        `INSERT INTO migration_events (experiment_id, event_type, reason, triggered_by)
         VALUES ($1, $2, $3, $4)`,
        [experimentId, 'guardrail_pause', reason || 'Manual rollback requested', userId]
      );

      // Audit log
      await client.query(
        `INSERT INTO migration_audit (experiment_id, user_id, action, resource_type, resource_id, old_value, new_value)
         VALUES ($1, $2, $3, $4, $5, $6, $7)`,
        [experimentId, userId, 'rollback', 'experiment', experimentId, JSON.stringify(exp), JSON.stringify(result.rows[0])]
      );

      await client.query('COMMIT');

      logger.warn('Migration rolled back', { experimentId, publisherId, reason });
      return this.mapExperiment(result.rows[0]);
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }
}
