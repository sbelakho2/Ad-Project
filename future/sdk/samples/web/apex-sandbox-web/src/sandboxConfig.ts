export const sandboxConfig = {
  apiBase: 'https://api.apexmediation.ee/api/v1',
  placements: {
    interstitialA: '3d1094ab-a85b-4737-a749-d8a153a0f026',
    interstitialB: '7f7f250e-48c0-4a89-9015-5c6cea325b3e',
    rewardedA: '074b2dc7-3173-4da0-aba0-250f3f129df1',
    bannerA: 'f6e7aa9b-09c5-4644-bf56-f8ab781ac62d',
  },
  consent: {
    gdpr: false,
    ccpa: false,
    coppa: false,
    lat: true,
  },
} as const
