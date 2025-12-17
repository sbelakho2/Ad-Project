import { NextResponse } from 'next/server';

const DEFAULT_API_BASE = 'https://api.apexmediation.ee/api/v1';

type CacheEntry = {
  expiresAtMs: number;
  staleUntilMs: number;
  payload: unknown;
};

let cache: CacheEntry | null = null;

export async function GET(req: Request) {
  const timestamp = new Date().toISOString();
  const apiBase = (process.env.NEXT_PUBLIC_API_URL || DEFAULT_API_BASE).replace(/\/+$/, '');

  const { searchParams } = new URL(req.url);
  const days = searchParams.get('days') || '30';

  const url = `${apiBase}/slo.json?days=${encodeURIComponent(days)}`;

  const now = Date.now();
  if (cache && cache.expiresAtMs > now) {
    return NextResponse.json(cache.payload, {
      status: 200,
      headers: {
        'Cache-Control': 'public, max-age=60, s-maxage=300, stale-while-revalidate=900',
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
    // This prevents the page from flipping to "awaiting data" due to transient 429s.
    if (!res.ok || !data || typeof data !== 'object') {
      if (cache && cache.staleUntilMs > now) {
        return NextResponse.json(cache.payload, {
          status: 200,
          headers: {
            'Cache-Control': 'public, max-age=30, s-maxage=60, stale-while-revalidate=300',
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
            'Cache-Control': 'public, max-age=30, s-maxage=60, stale-while-revalidate=300',
          },
        }
      );
    }

    cache = {
      payload: data,
      expiresAtMs: now + 5 * 60_000,
      staleUntilMs: now + 30 * 60_000,
    };

    return NextResponse.json(data, {
      status: 200,
      headers: {
        'Cache-Control': 'public, max-age=60, s-maxage=300, stale-while-revalidate=900',
      },
    });
  } catch {
    if (cache && cache.staleUntilMs > now) {
      return NextResponse.json(cache.payload, {
        status: 200,
        headers: {
          'Cache-Control': 'public, max-age=30, s-maxage=60, stale-while-revalidate=300',
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
          'Cache-Control': 'public, max-age=30, s-maxage=60, stale-while-revalidate=300',
        },
      }
    );
  }
}
