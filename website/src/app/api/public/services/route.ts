import { NextResponse } from 'next/server';

type ServiceCheck = {
  name: string;
  status: 'operational' | 'degraded' | 'down';
  latencyMs: number | null;
  error?: string;
};

const SERVICES = [
  {
    name: 'Website',
    healthUrl: process.env.NEXT_PUBLIC_SITE_URL 
      ? `${process.env.NEXT_PUBLIC_SITE_URL}/api/health`
      : 'https://apexmediation.ee/api/health',
  },
  {
    name: 'API',
    fallbackHealthUrl: process.env.INTERNAL_API_HEALTH_URL || 'http://127.0.0.1:8080/health',
    healthUrl: process.env.NEXT_PUBLIC_API_URL
      ? `${process.env.NEXT_PUBLIC_API_URL.replace('/v1', '').replace('/api', '')}/health`
      : 'https://api.apexmediation.ee/health',
  },
  {
    name: 'Console',
    healthUrl: process.env.NEXT_PUBLIC_CONSOLE_URL
      ? `${process.env.NEXT_PUBLIC_CONSOLE_URL}/api/health`
      : 'https://console.apexmediation.ee/api/health',
  },
];

async function checkService(name: string, healthUrl: string, fallbackHealthUrl?: string): Promise<ServiceCheck> {
  const start = Date.now();
  
  try {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), 5000);

    const res = await fetch(healthUrl, {
      method: 'GET',
      signal: controller.signal,
      cache: 'no-store',
      headers: { accept: 'application/json' },
    });

    clearTimeout(timeoutId);
    const latencyMs = Date.now() - start;

    if (res.ok) {
      const data = await res.json().catch(() => null);
      const status = data?.status === 'degraded' ? 'degraded' : 'operational';
      return { name, status, latencyMs };
    }

    return { name, status: 'down', latencyMs, error: `HTTP ${res.status}` };
  } catch (err) {
    if (fallbackHealthUrl) {
      try {
        const res = await fetch(fallbackHealthUrl, {
          method: 'GET',
          cache: 'no-store',
          headers: { accept: 'application/json' },
        });

        const latencyMs = Date.now() - start;
        if (res.ok) {
          const data = await res.json().catch(() => null);
          const status = data?.status === 'degraded' ? 'degraded' : 'operational';
          return { name, status, latencyMs };
        }

        return { name, status: 'down', latencyMs, error: `HTTP ${res.status}` };
      } catch (fallbackErr) {
        return { 
          name, 
          status: 'down', 
          latencyMs: Date.now() - start,
          error: fallbackErr instanceof Error ? fallbackErr.message : 'Connection failed',
        };
      }
    }

    return {
      name,
      status: 'down',
      latencyMs: Date.now() - start,
      error: err instanceof Error ? err.message : 'Connection failed',
    };
  }
}

export async function GET() {
  const timestamp = new Date().toISOString();

  const checks = await Promise.all(
    SERVICES.map((svc) => checkService(svc.name, svc.healthUrl, (svc as { fallbackHealthUrl?: string }).fallbackHealthUrl))
  );

  const allOperational = checks.every((c) => c.status === 'operational');
  const anyDown = checks.some((c) => c.status === 'down');

  const overallStatus = allOperational 
    ? 'operational' 
    : anyDown 
      ? 'down' 
      : 'degraded';

  return NextResponse.json(
    {
      status: overallStatus,
      timestamp,
      services: checks,
    },
    {
      status: 200,
      headers: {
        'Cache-Control': 'no-store, max-age=0',
      },
    }
  );
}
