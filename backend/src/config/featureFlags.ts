import { env } from './env';

// Feature flags (in-memory overrides with env defaults)
// Risky features controlled here to enable fast mitigation/rollback in staging/production.

export type FeatureFlags = {
  killSwitch: boolean; // when true, API returns 503 for most routes
  enforce2fa: boolean; // when true, login requires 2FA step-up
  requireEmailVerification: boolean; // when true, email must be verified before login
  disableNewAdapters: boolean; // example flag to disable newly added adapters rollout
};

const envBool = (val: boolean | undefined, def = false) => {
  if (val == null) return def;
  return Boolean(val);
};

let current: FeatureFlags = {
  killSwitch: envBool(env.FEATURE_KILL_SWITCH, false),
  enforce2fa: envBool(env.FEATURE_ENFORCE_2FA, false),
  requireEmailVerification: envBool(env.FEATURE_REQUIRE_EMAIL_VERIFICATION, false),
  disableNewAdapters: envBool(env.FEATURE_DISABLE_NEW_ADAPTERS, false),
};

export function getFeatureFlags(): FeatureFlags {
  return { ...current };
}

export function setFeatureFlags(partial: Partial<FeatureFlags>): FeatureFlags {
  current = { ...current, ...partial };
  return getFeatureFlags();
}

export function resetFeatureFlags(): void {
  current = {
    killSwitch: envBool(env.FEATURE_KILL_SWITCH, false),
    enforce2fa: envBool(env.FEATURE_ENFORCE_2FA, false),
    requireEmailVerification: envBool(env.FEATURE_REQUIRE_EMAIL_VERIFICATION, false),
    disableNewAdapters: envBool(env.FEATURE_DISABLE_NEW_ADAPTERS, false),
  };
}
