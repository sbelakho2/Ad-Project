import crypto from 'crypto';
import { NextFunction, Request, Response } from 'express';
import { AppError } from './errorHandler';

type OperatorSessionPayload = {
  identifier: string;
  exp: number; // unix seconds
};

function base64urlDecodeToBuffer(input: string): Buffer {
  const normalized = input.replace(/-/g, '+').replace(/_/g, '/');
  const padded = normalized + '==='.slice((normalized.length + 3) % 4);
  return Buffer.from(padded, 'base64');
}

function base64urlEncode(input: Buffer | string): string {
  const buf = typeof input === 'string' ? Buffer.from(input, 'utf8') : input;
  return buf
    .toString('base64')
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/g, '');
}

function timingSafeEqualUtf8(a: string, b: string): boolean {
  try {
    const aa = Buffer.from(a, 'utf8');
    const bb = Buffer.from(b, 'utf8');
    if (aa.length !== bb.length) return false;
    return crypto.timingSafeEqual(aa, bb);
  } catch {
    return false;
  }
}

function sign(payloadPart: string, secret: string): string {
  const signature = crypto.createHmac('sha256', secret).update(payloadPart).digest();
  return base64urlEncode(signature);
}

function parseCookieHeader(cookieHeader: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const part of cookieHeader.split(';')) {
    const idx = part.indexOf('=');
    if (idx <= 0) continue;
    const key = part.slice(0, idx).trim();
    const value = part.slice(idx + 1).trim();
    if (!key) continue;
    out[key] = value;
  }
  return out;
}

export function verifyOperatorSessionToken(token: string, secret: string): OperatorSessionPayload | null {
  const [payloadPart, sigPart] = token.split('.');
  if (!payloadPart || !sigPart) return null;

  const expectedSig = sign(payloadPart, secret);
  if (!timingSafeEqualUtf8(expectedSig, sigPart)) return null;

  try {
    const payloadJson = base64urlDecodeToBuffer(payloadPart).toString('utf8');
    const payload = JSON.parse(payloadJson) as OperatorSessionPayload;
    if (!payload?.identifier || typeof payload.exp !== 'number') return null;

    const nowSeconds = Math.floor(Date.now() / 1000);
    if (payload.exp <= nowSeconds) return null;
    return payload;
  } catch {
    return null;
  }
}

/**
 * Operator-only middleware compatible with the Console operator cockpit.
 *
 * Accepts either:
 * - HMAC-signed operator session cookie (default name: apex_operator_session)
 * - X-Operator-Secret header matching OPERATOR_SECRET (for safe server-to-server tooling)
 */
export function operatorSessionOnly() {
  return (req: Request, _res: Response, next: NextFunction) => {
    const secret = process.env.OPERATOR_SECRET || '';
    if (!secret) {
      return next(new AppError('Operator access not configured', 403));
    }

    const headerSecret = (req.header('x-operator-secret') || '').trim();
    if (headerSecret && timingSafeEqualUtf8(headerSecret, secret)) {
      (req as any).operator = { identifier: 'operator' };
      return next();
    }

    const cookieName = process.env.OPERATOR_COOKIE_NAME || 'apex_operator_session';
    const cookies = (req as Request & { cookies?: Record<string, string | undefined> }).cookies;

    const tokenFromCookie = cookies?.[cookieName]
      ? String(cookies[cookieName])
      : (() => {
          const raw = req.headers.cookie;
          if (!raw) return '';
          const parsed = parseCookieHeader(raw);
          return parsed[cookieName] || '';
        })();

    if (!tokenFromCookie) {
      return next(new AppError('Operator access required', 403));
    }

    const payload = verifyOperatorSessionToken(tokenFromCookie, secret);
    if (!payload) {
      return next(new AppError('Operator access required', 403));
    }

    (req as any).operator = payload;
    return next();
  };
}

export default operatorSessionOnly;
