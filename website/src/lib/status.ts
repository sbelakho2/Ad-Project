import { getLKG, saveLKG } from '@/lib/lkg';
import type { LiveStatus } from '@/lib/slo';

const KEY = 'status:rollup';

export async function getLiveStatus(): Promise<{ live: LiveStatus | null; usedLkg: boolean }> {
  let live: LiveStatus | null = null;
  let usedLkg = false;

  try {
    const res = await fetch(`${process.env.NEXT_PUBLIC_BASE_URL ?? ''}/api/public/status`, {
      cache: 'no-store',
      headers: { accept: 'application/json' },
    });
    if (res.ok) {
      const data = (await res.json()) as LiveStatus;
      live = data;
      saveLKG(KEY, data);
    } else {
      const snap = getLKG<LiveStatus>(KEY);
      if (snap) {
        live = { ...(snap.payload || null) } as LiveStatus;
        usedLkg = true;
      }
    }
  } catch {
    const snap = getLKG<LiveStatus>(KEY);
    if (snap) {
      live = { ...(snap.payload || null) } as LiveStatus;
      usedLkg = true;
    }
  }

  return { live, usedLkg };
}
