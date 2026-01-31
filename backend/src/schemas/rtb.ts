import { z } from 'zod';
import crypto from 'crypto';

const MigrationSignalSchema = z.object({
  experiment_id: z.string().min(1),
  arm: z.enum(['control', 'test']),
  assignment_ts: z.string().refine(
    (value) => {
      if (!value) return false;
      const time = Date.parse(value);
      return Number.isFinite(time);
    },
    'assignment_ts must be an ISO 8601 timestamp'
  ),
  mirror_percent: z.number().min(0).max(100).optional(),
  mode: z.enum(['shadow', 'mirroring']).optional(),
});

const AuctionSignalSchema = z.object({
  migration: MigrationSignalSchema.optional(),
}).catchall(z.any());

export const ConsentSchema = z.object({
  gdpr: z.number().int().min(0).max(1).optional(),
  gdpr_consent: z.string().min(1).max(2048).optional(),
  us_privacy: z.string().min(1).max(64).optional(),
  coppa: z.boolean().optional(),
}).partial();

export const DeviceSchema = z.object({
  platform: z.enum(['ios', 'android', 'web', 'unity']).optional(),
  osVersion: z.string().optional(),
  model: z.string().optional(),
  idfa: z.string().optional(),
  gaid: z.string().optional(),
}).partial();

export const AppSchema = z.object({
  id: z.string().optional(),
  name: z.string().optional(),
  bundle: z.string().optional(),
  version: z.string().optional(),
}).partial();

export const UserSchema = z.object({
  country: z.string().optional(),
  language: z.string().optional(),
  age: z.number().int().min(0).max(120).optional(),
}).partial();

export const AuctionRequestSchema = z.object({
  body: z.preprocess((val: any) => {
    const body = { ...val };
    
    // Alias placement_id -> placementId
    if (body.placement_id && !body.placementId) {
      body.placementId = body.placement_id;
    }
    
    // Alias ad_type / adType -> adFormat
    if ((body.ad_type || body.adType) && !body.adFormat) {
      body.adFormat = body.ad_type || body.adType;
    }
    
    // Alias app_id -> app.id
    if (body.app_id && !body.app?.id) {
      body.app = { ...body.app, id: body.app_id };
    }

    // Alias device_info -> device
    if (body.device_info && !body.device) {
      body.device = body.device_info;
    }

    // Alias user_info -> user
    if (body.user_info && !body.user) {
      body.user = body.user_info;
    }

    // Normalize device fields
    if (body.device) {
      if (body.device.os_version && !body.device.osVersion) {
        body.device.osVersion = body.device.os_version;
      }
    }

    return body;
  }, z.object({
    requestId: z.string().uuid().optional().default(() => crypto.randomUUID?.() || '00000000-0000-0000-0000-000000000000'),
    placementId: z.string().min(1),
    adFormat: z.enum(['banner', 'interstitial', 'rewarded', 'native']),
    floorCpm: z.number().nonnegative().default(0),
    device: DeviceSchema.optional(),
    user: UserSchema.optional(),
    app: AppSchema.optional(),
    consent: ConsentSchema.optional(),
    signal: AuctionSignalSchema.optional(),
  }))
});

export type AuctionRequestBody = z.infer<typeof AuctionRequestSchema>['body'];
export type MigrationSignal = z.infer<typeof MigrationSignalSchema>;

export const TrackingTokenSchema = z.object({
  bidId: z.string().min(8),
  placementId: z.string().min(1),
  adapter: z.string().min(1),
  cpm: z.number().nonnegative(),
  currency: z.literal('USD'),
  purpose: z.enum(['delivery', 'imp', 'click']),
  // Optional advertiser landing URL for delivery/click redirect flows.
  // When present, controllers must enforce an allow-list or same-origin policy.
  url: z.string().url().optional(),
  nonce: z.string().min(8),
  iat: z.number().optional(),
  exp: z.number().optional(),
});
