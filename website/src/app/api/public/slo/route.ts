import { NextResponse } from 'next/server';

const DEFAULT_API_BASE = 'https://api.apexmediation.ee/api/v1';

export async function GET(req: Request) {
  const timestamp = new Date().toISOString();
  const apiBase = (process.env.NEXT_PUBLIC_API_URL || DEFAULT_API_BASE).replace(/\/+$/, '');

  const { searchParams } = new URL(req.url);
  const days = searchParams.get('days') || '30';

  const url = `${apiBase}/slo.json?days=${encodeURIComponent(days)}`;

  try {
    const res = await fetch(url, {
      next: { revalidate: 300 },
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
            'Cache-Control': 'public, max-age=30, s-maxage=60, stale-while-revalidate=300',
          },
        }
      );
    }

    return NextResponse.json(data, {
      status: 200,
      headers: {
        'Cache-Control': 'public, max-age=60, s-maxage=300, stale-while-revalidate=900',
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
          'Cache-Control': 'public, max-age=30, s-maxage=60, stale-while-revalidate=300',
        },
      }
    );
  }
}
