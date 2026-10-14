export const SUPPORTED_NETWORKS = [
  'admob',
  'applovin',
  'unity',
  'ironsource',
  'facebook',
  'vungle',
  'chartboost',
  'pangle',
  'mintegral',
  'adcolony',
  'tapjoy',
  'moloco',
  'fyber',
  'smaato',
  'amazon',
] as const;

export type SupportedNetwork = typeof SUPPORTED_NETWORKS[number];

export function getSupportedAdapters(): SupportedNetwork[] {
  return [...SUPPORTED_NETWORKS];
}
