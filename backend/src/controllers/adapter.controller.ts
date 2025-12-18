import { Request, Response, NextFunction } from 'express';
import { z } from 'zod';
import { AppError } from '../middleware/errorHandler';
import logger from '../utils/logger';
import * as adapterConfigService from '../services/adapterConfigService';
import { getSchemaForAdapter } from '../services/rtb/adapterSchemas';

// Validation schemas
const safeJson = z.union([z.string(), z.number(), z.boolean(), z.null()]);
type Json = z.infer<typeof safeJson> | Json[] | { [k: string]: Json };

const MAX_CONFIG_KEYS = 100;
const MAX_STRING_LEN = 2048;

const configValueSchema: z.ZodType<Json> = z.lazy(() =>
  z.union([
    safeJson,
    z.array(z.lazy(() => configValueSchema)).max(500),
    z.record(z.string().max(128), z.lazy(() => configValueSchema)),
  ])
);

const createAdapterConfigSchema = z.object({
  adapterId: z.string().uuid(),
  config: z
    .record(z.string().max(128), configValueSchema)
    .refine((obj) => Object.keys(obj).length <= MAX_CONFIG_KEYS, {
      message: `Too many config keys (>${MAX_CONFIG_KEYS})`,
    })
    .transform((obj) => maskSecrets(obj)),
});

const updateAdapterConfigSchema = z.object({
  config: z
    .record(z.string().max(128), configValueSchema)
    .refine((obj) => Object.keys(obj).length <= MAX_CONFIG_KEYS, {
      message: `Too many config keys (>${MAX_CONFIG_KEYS})`,
    })
    .transform((obj) => maskSecrets(obj)),
});

// Per‑adapter schemas are maintained in services/rtb/adapterSchemas.ts

function maskSecrets(obj: Record<string, unknown>): Record<string, unknown> {
  const masked: Record<string, unknown> = {};
  const secretKeys = /(secret|password|token|api[-_]?key|private[-_]?key)/i;
  for (const [k, v] of Object.entries(obj)) {
    if (typeof v === 'string') {
      masked[k] = v.length > 8 && secretKeys.test(k) ? `${v.slice(0, 2)}***${v.slice(-2)}` : v;
    } else if (v && typeof v === 'object' && !Array.isArray(v)) {
      masked[k] = maskSecrets(v as Record<string, unknown>);
    } else if (Array.isArray(v)) {
      masked[k] = v.map((x) => (typeof x === 'string' && secretKeys.test(k) ? (x.length > 8 ? `${x.slice(0, 2)}***${x.slice(-2)}` : '***') : x));
    } else {
      masked[k] = v as any;
    }
  }
  return masked;
}

/**
 * List all adapter configs for the authenticated publisher
 */
export const list = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Publisher context required', 401);
    }

    let configs: Awaited<ReturnType<typeof adapterConfigService.getAdapterConfigs>> = [];
    try {
      configs = await adapterConfigService.getAdapterConfigs(publisherId);
    } catch (dbError) {
      // If the adapter_configs table schema doesn't support per-publisher configs yet,
      // return an empty array. This gracefully handles schema mismatches.
      logger.warn('Failed to fetch adapter configs, returning empty array', {
        publisherId,
        error: (dbError as Error).message,
      });
      configs = [];
    }

    res.json({
      success: true,
      data: configs,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * Get single adapter config by ID
 */
export const getById = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const { id } = req.params;
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Publisher context required', 401);
    }

    const config = await adapterConfigService.getAdapterConfigById(id, publisherId);

    if (!config) {
      throw new AppError('Adapter config not found', 404);
    }

    res.json({
      success: true,
      data: config,
    });
  } catch (error) {
    next(error);
  }
};

/**
 * Create a new adapter config
 */
export const create = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Publisher context required', 401);
    }

    const data = createAdapterConfigSchema.parse(req.body);

    // Optional: per-adapter schema validation if we recognize the adapterId
    try {
      const schema = await getSchemaForAdapter({ adapterId: data.adapterId });
      if (schema) {
        schema.parse(data.config);
      }
    } catch (_e) {
      throw new AppError('Invalid adapter config for specified adapter', 400);
    }
    const config = await adapterConfigService.createAdapterConfig(publisherId, data);

    logger.info(`Adapter config created`, { adapterId: data.adapterId, publisherId, keys: Object.keys(data.config) });

    res.status(201).json({
      success: true,
      data: config,
    });
  } catch (error) {
    if (error instanceof z.ZodError) {
      next(new AppError('Invalid request data', 400));
    } else if (error instanceof Error && error.message.includes('already exists')) {
      next(new AppError(error.message, 409));
    } else {
      next(error);
    }
  }
};

/**
 * Update adapter config
 */
export const update = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const { id } = req.params;
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Publisher context required', 401);
    }

    const data = updateAdapterConfigSchema.parse(req.body);

    const existing = await adapterConfigService.getAdapterConfigById(id, publisherId);
    if (!existing) {
      throw new AppError('Adapter config not found', 404);
    }

    try {
      const schema = await getSchemaForAdapter({ adapterId: existing.adapterId });
      if (schema) {
        schema.parse(data.config);
      }
    } catch (_e) {
      throw new AppError('Invalid adapter config for specified adapter', 400);
    }
    const config = await adapterConfigService.updateAdapterConfig(id, publisherId, data);

    if (!config) {
      throw new AppError('Adapter config not found', 404);
    }

    logger.info(`Adapter config updated`, { id, publisherId, keys: Object.keys(data.config) });

    res.json({
      success: true,
      data: config,
    });
  } catch (error) {
    if (error instanceof z.ZodError) {
      next(new AppError('Invalid request data', 400));
    } else {
      next(error);
    }
  }
};

/**
 * Delete adapter config
 */
export const deleteConfig = async (
  req: Request,
  res: Response,
  next: NextFunction
): Promise<void> => {
  try {
    const { id } = req.params;
    const publisherId = req.user?.publisherId;

    if (!publisherId) {
      throw new AppError('Publisher context required', 401);
    }

    const deleted = await adapterConfigService.deleteAdapterConfig(id, publisherId);

    if (!deleted) {
      throw new AppError('Adapter config not found', 404);
    }

    logger.info(`Adapter config deleted: ${id}`);

    res.status(204).send();
  } catch (error) {
    next(error);
  }
};
