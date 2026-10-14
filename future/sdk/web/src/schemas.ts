import { z } from 'zod';

export const ConsentSchema = z.object({
  gdprApplies: z.boolean().optional(),
  tcfConsent: z.string().min(1).optional().nullable(),
  usPrivacy: z.string().min(1).optional().nullable(),
  gpp: z.string().min(1).optional().nullable(),
  coppa: z.boolean().optional(),
});

export const AdRequestSchema = z.object({
  placement: z.string().min(1),
  adType: z.enum(['banner', 'interstitial', 'rewarded', 'rewarded_interstitial', 'native', 'app_open']),
  width: z.number().int().positive().optional(),
  height: z.number().int().positive().optional(),
  testMode: z.boolean().optional(),
  extras: z.record(z.any()).optional(),
});

export const AuctionRequestSchema = z.object({
  request: AdRequestSchema,
  consent: ConsentSchema.partial().optional(),
  meta: z.object({
    sdk: z.object({ name: z.string(), version: z.string() }),
    publisherId: z.string().optional(),
    appId: z.string().optional(),
  }),
});

export const AdCreativeSchema = z.object({
  id: z.string(),
  html: z.string().optional(),
  vastTagUrl: z.string().url().optional(),
  tracking: z.record(z.string()).optional(),
});

export const AdResponseSchema = z.object({
  requestId: z.string(),
  fill: z.boolean(),
  price: z.number().optional(),
  currency: z.string().optional(),
  creative: AdCreativeSchema.nullable().optional(),
  ttlSeconds: z.number().int().positive().optional(),
  // Additional fields for parity with native SDKs
  networkName: z.string().optional(),
  adType: z.enum(['banner', 'interstitial', 'rewarded', 'rewarded_interstitial', 'native', 'app_open']).optional(),
  placementId: z.string().optional(),
  expiryTimeMs: z.number().int().positive().optional(),
});

export type ConsentSchemaType = z.infer<typeof ConsentSchema>;
export type AdRequestSchemaType = z.infer<typeof AdRequestSchema>;
export type AuctionRequestSchemaType = z.infer<typeof AuctionRequestSchema>;
export type AdResponseSchemaType = z.infer<typeof AdResponseSchema>;
