import { NextResponse } from 'next/server';

const DEFAULT_API_BASE = 'https://api.apexmediation.ee/api/v1';

type CacheEntry = {
  expiresAtMs: number;
  staleUntilMs: number;
  payload: unknown;
};

let cache: CacheEntry | null = null;

export async function GET() {
  const timestamp = new Date().toISOString();
  const apiBase = (process.env.NEXT_PUBLIC_API_URL || DEFAULT_API_BASE).replace(/\/+$/, '');
  const url = `${apiBase}/status.json`;

  const now = Date.now();
  if (cache && cache.expiresAtMs > now) {
    return NextResponse.json(cache.payload, {
      status: 200,
      headers: {
        'Cache-Control': 'public, max-age=15, s-maxage=30, stale-while-revalidate=300',
      },
    });
  }

  try {
    const res = await fetch(url, {
      // Avoid caching upstream 429/HTML responses at the Next layer.
      cache: 'no-store',
      headers: {
        accept: 'application/json',
      },
    });

    const data = await res.json().catch(() => null);

    // If upstream is rate-limited or returned non-JSON, serve last known-good payload.
    // This prevents the SLO page from flipping to "awaiting data" due to transient 429s.
    if (!res.ok || !data || typeof data !== 'object') {
      if (cache && cache.staleUntilMs > now) {
        return NextResponse.json(cache.payload, {
          status: 200,
          headers: {
            'Cache-Control': 'public, max-age=5, s-maxage=10, stale-while-revalidate=60',
          },
        });
      }

      return NextResponse.json(
        {
          status: 'unknown',
          service: 'apexmediation-backend',
          timestamp,
        },
        {
          status: 200,
          headers: {
            'Cache-Control': 'public, max-age=5, s-maxage=10, stale-while-revalidate=60',
          },
        }
      );
    }

    cache = {
      payload: data,
      expiresAtMs: now + 30_000,
      staleUntilMs: now + 10 * 60_000,
    };

    return NextResponse.json(data, {
      status: 200,
      headers: {
        // Mirror backend-ish caching but keep it modest.
        'Cache-Control': 'public, max-age=15, s-maxage=30, stale-while-revalidate=300',
      },
    });
  } catch {
    if (cache && cache.staleUntilMs > now) {
      return NextResponse.json(cache.payload, {
        status: 200,
        headers: {
          'Cache-Control': 'public, max-age=5, s-maxage=10, stale-while-revalidate=60',
        },
      });
    }

    return NextResponse.json(
      {
        status: 'unknown',
        service: 'apexmediation-backend',
        timestamp,
      },
      {
        status: 200,
        headers: {
          'Cache-Control': 'public, max-age=5, s-maxage=10, stale-while-revalidate=60',
        },
      }
    );
  }
}
