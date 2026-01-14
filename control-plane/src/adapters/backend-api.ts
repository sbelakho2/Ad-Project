import { config } from '../config';
import { logger } from '../logger';

/**
 * Backend API Adapter
 * Handles communication with the main customer-facing backend
 */

export async function callBackend(path: string, opts: { method?: string; body?: any; operator?: string } = {}) {
  // Use BACKEND_INTERNAL_URL or similar if available
  const baseUrl = process.env.BACKEND_API_URL || 'http://localhost:8080';
  const url = `${baseUrl}${path}`;

  try {
    const response = await fetch(url, {
      method: opts.method || 'GET',
      headers: {
        'Content-Type': 'application/json',
        // In a real system, we'd use a service-to-service token or mTLS
        // For now, we'll assume we can pass the operator identity for auditing
        'X-Operator-Identity': opts.operator || 'system-control-plane',
        'Authorization': `Bearer ${process.env.BACKEND_INTERNAL_TOKEN}` // Shared secret for S2S
      },
      body: opts.body ? JSON.stringify(opts.body) : undefined
    });

    if (!response.ok) {
      const errorText = await response.text();
      throw new Error(`Backend returned ${response.status}: ${errorText}`);
    }

    return await response.json();
  } catch (err) {
    logger.error('Backend API call failed', { url, error: (err as Error).message });
    throw err;
  }
}

export async function getImpersonationLink(userId: string, operator: string) {
  return await callBackend('/api/v1/admin/impersonate/start', {
    method: 'POST',
    body: { userId },
    operator
  });
}
