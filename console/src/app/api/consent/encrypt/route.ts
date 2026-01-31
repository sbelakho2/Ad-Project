/**
 * Server-side consent string encryption API
 * 
 * This endpoint encrypts consent strings using AES-GCM on the server,
 * keeping the encryption key secure and never exposing it to the client.
 * 
 * @security The encryption key is stored in CONSENT_ENCRYPTION_KEY (server-side only)
 *           Never use NEXT_PUBLIC_ prefix for encryption keys.
 */

import { NextRequest, NextResponse } from 'next/server';
import { getServerSession } from 'next-auth';
import { authOptions } from '@/lib/auth/options';
import crypto from 'crypto';

// Server-side only - no NEXT_PUBLIC_ prefix
const CONSENT_ENCRYPTION_KEY = process.env.CONSENT_ENCRYPTION_KEY ?? '';
const CONSENT_ENCRYPTION_KEY_ID = process.env.CONSENT_ENCRYPTION_KEY_ID ?? 'default-consent-key';

interface EncryptedConsentPayload {
  mode: 'encrypted';
  algorithm: 'AES-GCM';
  cipherText: string;
  iv: string;
  keyId: string;
}

interface FallbackPayload {
  mode: 'fallback';
  value: string;
}

type EncryptResponse = EncryptedConsentPayload | FallbackPayload;

function getKeyBuffer(): Buffer | null {
  if (!CONSENT_ENCRYPTION_KEY) return null;
  
  // Accept base64 or hex (fallback to utf8)
  try {
    // Check for 64-char hex string (256-bit key)
    if (/^[A-Fa-f0-9]{64}$/.test(CONSENT_ENCRYPTION_KEY)) {
      return Buffer.from(CONSENT_ENCRYPTION_KEY, 'hex');
    }
    // Try base64
    const buf = Buffer.from(CONSENT_ENCRYPTION_KEY, 'base64');
    if (buf.length === 32) return buf;
  } catch {
    // Fall through to derivation
  }
  
  // Derive 32 bytes via SHA-256 if key is not the right size
  return crypto.createHash('sha256').update(CONSENT_ENCRYPTION_KEY, 'utf8').digest();
}

function encryptValue(value: string): EncryptResponse {
  if (!value) {
    return { mode: 'fallback', value: '' };
  }

  const keyBuffer = getKeyBuffer();
  if (!keyBuffer) {
    // No encryption key configured - return fallback
    console.warn('[consent/encrypt] CONSENT_ENCRYPTION_KEY not configured, using fallback');
    return { mode: 'fallback', value };
  }

  try {
    const iv = crypto.randomBytes(12);
    const cipher = crypto.createCipheriv('aes-256-gcm', keyBuffer, iv);
    
    const encrypted = Buffer.concat([
      cipher.update(value, 'utf8'),
      cipher.final()
    ]);
    
    // Get auth tag and append to ciphertext (standard GCM format)
    const authTag = cipher.getAuthTag();
    const cipherTextWithTag = Buffer.concat([encrypted, authTag]);

    return {
      mode: 'encrypted',
      algorithm: 'AES-GCM',
      cipherText: cipherTextWithTag.toString('base64'),
      iv: iv.toString('base64'),
      keyId: CONSENT_ENCRYPTION_KEY_ID,
    };
  } catch (error) {
    console.error('[consent/encrypt] Encryption failed:', error);
    return { mode: 'fallback', value };
  }
}

export async function POST(request: NextRequest): Promise<NextResponse> {
  // Require authentication
  const session = await getServerSession(authOptions);
  if (!session?.user) {
    return NextResponse.json(
      { error: 'Unauthorized' },
      { status: 401 }
    );
  }

  try {
    const body = await request.json();
    const { value } = body as { value?: string };

    if (typeof value !== 'string') {
      return NextResponse.json(
        { error: 'Invalid request: value must be a string' },
        { status: 400 }
      );
    }

    // Limit input size to prevent abuse (max 10KB for consent strings)
    if (value.length > 10240) {
      return NextResponse.json(
        { error: 'Invalid request: value too large (max 10KB)' },
        { status: 400 }
      );
    }

    const result = encryptValue(value);
    
    return NextResponse.json(result, {
      status: 200,
      headers: {
        'Cache-Control': 'no-store, max-age=0',
      },
    });
  } catch (error) {
    console.error('[consent/encrypt] Error processing request:', error);
    return NextResponse.json(
      { error: 'Internal server error' },
      { status: 500 }
    );
  }
}
