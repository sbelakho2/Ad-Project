import { NextResponse } from 'next/server';

const DEFAULT_API_BASE = 'https://api.apexmediation.ee/api/v1';

export async function GET() {
  const timestamp = new Date().toISOString();
  const apiBase = (process.env.NEXT_PUBLIC_API_URL || DEFAULT_API_BASE).replace(/\/+$/, '');
  const url = `${apiBase}/status.json`;

  try {
    const res = await fetch(url, {
      // Cache on the server to reduce backend load.
      next: { revalidate: 30 },
      headers: {
        accept: 'application/json',
      },
    });

    const data = await res.json().catch(() => null);

    if (!data || typeof data !== 'object') {
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

    return NextResponse.json(data, {
      status: 200,
      headers: {
        // Mirror backend-ish caching but keep it modest.
        'Cache-Control': 'public, max-age=15, s-maxage=30, stale-while-revalidate=300',
      },
    });
  } catch {
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
