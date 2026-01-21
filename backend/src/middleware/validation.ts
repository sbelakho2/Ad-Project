/**
 * Input Validation Middleware
 * 
 * Zod-based request validation for all API endpoints
 * Prevents injection attacks and ensures data integrity
 */

import { Request, Response, NextFunction } from 'express';
import { z, ZodError, ZodSchema } from 'zod';

/**
 * Generic validation middleware factory
 */
export const validate = (schema: ZodSchema) => {
  return async (req: Request, res: Response, next: NextFunction) => {
    try {
      // Validate request body, query params, and route params
      const validated = await schema.parseAsync({
        body: req.body,
        query: req.query,
        params: req.params,
      });

      // Replace with validated data
      req.body = validated.body || req.body;
      req.query = validated.query || req.query;
      req.params = validated.params || req.params;

      next();
    } catch (error) {
      if (error instanceof ZodError) {
        res.status(400).json({
          error: 'Validation failed',
          details: error.errors.map(err => ({
            field: err.path.join('.'),
            message: err.message,
            code: err.code,
          })),
        });
      } else {
        res.status(500).json({
          error: 'Internal validation error',
          message: error instanceof Error ? error.message : 'Unknown error',
        });
      }
    }
  };
};

// =====================================================================
// COMMON VALIDATION SCHEMAS
// =====================================================================

/**
 * UUID validation
 */
export const uuidSchema = z.string().uuid({ message: 'Invalid UUID format' });

/**
 * Email validation
 */
export const emailSchema = z.string().email({ message: 'Invalid email address' });

/**
 * Pagination parameters
 */
export const paginationSchema = z.object({
  page: z.coerce.number().int().positive().default(1),
  limit: z.coerce.number().int().min(1).max(100).default(20),
});

/**
 * Date range validation
 */
export const dateRangeSchema = z.object({
  start_date: z.string().datetime().or(z.string().date()).optional(),
  end_date: z.string().datetime().or(z.string().date()).optional(),
}).refine(
  data => !data.start_date || !data.end_date || new Date(data.start_date) <= new Date(data.end_date),
  { message: 'start_date must be before end_date' }
);

// =====================================================================
// AUTHENTICATION SCHEMAS
// =====================================================================

export const registerSchema = z.object({
  body: z.object({
    email: emailSchema,
    password: z.string()
      .min(8, 'Password must be at least 8 characters')
      .max(128, 'Password too long')
      .regex(/[A-Z]/, 'Password must contain uppercase letter')
      .regex(/[a-z]/, 'Password must contain lowercase letter')
      .regex(/[0-9]/, 'Password must contain number'),
    name: z.string().min(1).max(255),
    company_name: z.string().min(1).max(255).optional(),
    referral_code: z.string().length(12).optional(),
  }),
});

export const loginSchema = z.object({
  body: z.object({
    email: emailSchema,
    password: z.string().min(1, 'Password required'),
  }),
});

export const refreshTokenSchema = z.object({
  body: z.object({
    refresh_token: z.string().min(1, 'Refresh token required'),
  }),
});

// =====================================================================
// FINANCIAL REPORTING SCHEMAS
// =====================================================================

export const transactionLogSchema = z.object({
  body: z.object({
    transaction_type: z.enum([
      'revenue', 'expense', 'payment_received', 'payment_sent', 'refund_issued',
      'refund_received', 'credit_issued', 'credit_applied', 'vat_collected',
      'vat_paid', 'fee_charged', 'commission_earned', 'subscription_charge',
      'usage_charge', 'setup_fee', 'late_fee', 'chargeback', 'payout',
      'adjustment', 'write_off'
    ]),
    category: z.string().min(1).max(100),
    amount_cents: z.number().int().positive(),
    currency_code: z.string().length(3).default('EUR'),
    vat_rate: z.number().min(0).max(100).default(0),
    customer_id: uuidSchema.optional(),
    vendor_name: z.string().max(255).optional(),
    description: z.string().min(1).max(1000),
    payment_method: z.string().max(50).optional(),
    payment_processor_id: z.string().max(255).optional(),
    document_url: z.string().url().max(500).optional(),
  }),
});

export const fiscalYearSchema = z.object({
  params: z.object({
    year: z.coerce.number().int().min(2020).max(2100),
  }),
});

export const vatReportSchema = z.object({
  params: z.object({
    year: z.coerce.number().int().min(2020).max(2100),
    quarter: z.coerce.number().int().min(1).max(4),
  }),
});

// =====================================================================
// REFERRAL SYSTEM SCHEMAS
// =====================================================================

export const referralProcessSchema = z.object({
  body: z.object({
    new_customer_id: uuidSchema,
    referral_code: z.string().length(12),
  }),
});

// =====================================================================
// USAGE METERING SCHEMAS
// =====================================================================

export const usageRecordSchema = z.object({
  body: z.object({
    customer_id: uuidSchema,
    metric_type: z.enum(['impressions', 'api_calls', 'sdk_downloads', 'data_transfer']),
    quantity: z.number().int().positive(),
    timestamp: z.string().datetime().optional(),
    metadata: z.record(z.unknown()).optional(),
  }),
});

export const sdkUsageMetricsSchema = z.object({
  body: z.object({
    metrics: z
      .object({
        adRequests: z.number().int().min(0).optional(),
        adImpressions: z.number().int().min(0).optional(),
        adClicks: z.number().int().min(0).optional(),
        videoStarts: z.number().int().min(0).optional(),
        videoCompletes: z.number().int().min(0).optional(),
        totalRevenue: z.number().min(0).optional(),
        apiCalls: z.number().int().min(0).optional(),
        cacheHits: z.number().int().min(0).optional(),
        cacheMisses: z.number().int().min(0).optional(),
        errors: z.number().int().min(0).optional(),
        periodStart: z.number().optional(),
        periodEnd: z.number().optional(),
        dataTransferGb: z.number().min(0).optional(),
      })
      .passthrough(),
    breakdown: z
      .object({
        byPlacement: z.record(z.unknown()).optional(),
        byAdapter: z.record(z.unknown()).optional(),
        byAdFormat: z.record(z.unknown()).optional(),
      })
      .optional(),
  }),
});

const planTierSchema = z.enum(['starter', 'growth', 'scale', 'enterprise']);

// =====================================================================
// CUSTOMER MANAGEMENT SCHEMAS
// =====================================================================

export const updateCustomerSchema = z.object({
  params: z.object({
    id: uuidSchema,
  }),
  body: z.object({
    email: emailSchema.optional(),
    name: z.string().min(1).max(255).optional(),
    company_name: z.string().min(1).max(255).optional(),
    plan: planTierSchema.optional(),
    status: z.enum(['active', 'suspended', 'cancelled']).optional(),
  }).refine(data => Object.keys(data).length > 0, {
    message: 'At least one field must be provided',
  }),
});

// =====================================================================
// A/B TESTING SCHEMAS
// =====================================================================

export const createExperimentSchema = z.object({
  body: z.object({
    name: z.string().min(1).max(255),
    description: z.string().max(1000).optional(),
    variants: z.array(z.object({
      name: z.string().min(1).max(100),
      weight: z.number().min(0).max(1),
      config: z.record(z.unknown()),
    })).min(2, 'At least 2 variants required'),
    start_date: z.string().datetime().optional(),
    end_date: z.string().datetime().optional(),
  }),
});

export const recordExperimentEventSchema = z.object({
  body: z.object({
    experiment_id: uuidSchema,
    variant_id: uuidSchema,
    event_type: z.enum(['impression', 'click', 'conversion']),
    value: z.number().optional(),
    metadata: z.record(z.unknown()).optional(),
  }),
});

// =====================================================================
// WEBHOOK SCHEMAS
// =====================================================================

export const stripeWebhookSchema = z.object({
  body: z.object({
    id: z.string(),
    type: z.string(),
    data: z.object({
      object: z.record(z.unknown()),
    }),
  }),
  headers: z.object({
    'stripe-signature': z.string().min(1, 'Stripe signature required'),
  }),
});

// =====================================================================
// EXPORT SCHEMAS
// =====================================================================

export const dataExportSchema = z.object({
  body: z.object({
    export_type: z.enum(['customers', 'transactions', 'analytics', 'usage']),
    format: z.enum(['csv', 'json', 'excel']).default('csv'),
    filters: z.record(z.unknown()).optional(),
    date_range: dateRangeSchema.optional(),
  }),
});

// =====================================================================
// ANALYTICS SCHEMAS
// =====================================================================

export const analyticsQuerySchema = z.object({
  query: z.object({
    metric: z.enum(['revenue', 'impressions', 'api_calls', 'active_users']),
    aggregation: z.enum(['sum', 'avg', 'count', 'max', 'min']).default('sum'),
    group_by: z.enum(['day', 'week', 'month', 'customer', 'plan']).optional(),
    start_date: z.string().datetime().or(z.string().date()).optional(),
    end_date: z.string().datetime().or(z.string().date()).optional(),
    page: z.coerce.number().int().positive().default(1),
    limit: z.coerce.number().int().min(1).max(100).default(20),
  }),
});

// =====================================================================
// GEOGRAPHIC DISCOUNT SCHEMAS
// =====================================================================

export const geographicDiscountSchema = z.object({
  body: z.object({
    customer_id: uuidSchema,
    ip_address: z.string().ip(),
    payment_country: z.string().length(2), // ISO country code
    app_store_country: z.string().length(2).optional(),
    requested_discount: z.number().min(0).max(100).optional(),
  }),
});

// =====================================================================
// HEALTH SCORE SCHEMAS
// =====================================================================

export const healthScoreSchema = z.object({
  body: z.object({
    customer_id: uuidSchema,
    usage_score: z.number().min(0).max(100),
    engagement_score: z.number().min(0).max(100),
    payment_score: z.number().min(0).max(100),
    support_score: z.number().min(0).max(100),
  }),
});

// =====================================================================
// SANITIZATION HELPERS
// =====================================================================

/**
 * Sanitize string to prevent XSS
 */
export const sanitizeString = (str: string): string => {
  return str
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#x27;')
    .replace(/\//g, '&#x2F;');
};

/**
 * Sanitize SQL identifiers (table names, column names)
 */
export const sanitizeSQLIdentifier = (identifier: string): string => {
  // Only allow alphanumeric and underscores
  if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(identifier)) {
    throw new Error('Invalid SQL identifier');
  }
  return identifier;
};

/**
 * Validate and sanitize email
 */
export const sanitizeEmail = (email: string): string => {
  const parsed = emailSchema.parse(email);
  return parsed.toLowerCase().trim();
};
