/**
 * Centralized Environment Loading Utility
 * 
 * This module provides a single source of truth for .env file resolution
 * across the entire backend codebase. It handles:
 * 
 * 1. Scripts executed from /opt/adproject/backend picking up /opt/adproject/.env
 * 2. Local development from various working directories
 * 3. Transpiled dist/ paths (backend/dist/src/utils)
 * 4. Source paths (backend/src/utils)
 * 
 * USAGE:
 *   import { ensureEnvLoaded, getEnvDiagnostics } from './envLoader';
 *   ensureEnvLoaded();
 * 
 * NOTE: This module should be imported BEFORE any module that reads process.env.
 *       Typically, import this at the very top of entry points (index.ts, CLI scripts).
 */

import dotenv from 'dotenv';
import fs from 'node:fs';
import path from 'node:path';

export interface EnvLoadResult {
  loaded: boolean;
  envPath: string | null;
  reason: 'already-set' | 'file-found' | 'default-fallback';
}

export interface EnvDiagnostics {
  cwd: string;
  envPath: string | null;
  databaseUrl: {
    host: string;
    port: string;
    database: string;
    sslmode: string | null;
  } | null;
}

let loadResult: EnvLoadResult | null = null;

/**
 * Resolves the .env file path based on the current working directory and __dirname.
 * 
 * Resolution order:
 * 1. Explicit ENV_FILE environment variable
 * 2. CWD-based candidates (prefers repo-root .env when in backend/)
 * 3. __dirname-based candidates (for transpiled code)
 */
export function resolveEnvPath(): string | null {
  const cwd = process.cwd();
  const cwdBase = path.basename(cwd);

  // CWD-based candidates: prefer repo-root .env when running from backend/
  const cwdCandidates =
    cwdBase === 'backend'
      ? [
          path.resolve(cwd, '../.env'),  // Repo root (e.g., /opt/adproject/.env)
          path.resolve(cwd, '.env'),     // Backend-local (fallback)
        ]
      : [
          path.resolve(cwd, '.env'),
          path.resolve(cwd, '../.env'),
        ];

  // __dirname-based candidates for transpiled dist/ paths
  const dirnameCandidates = [
    // Running from transpiled dist/ paths (backend/dist/src/utils -> ../../../../.env)
    path.resolve(__dirname, '../../../../.env'),
    // Running from source paths (backend/src/utils -> ../../../.env)
    path.resolve(__dirname, '../../../.env'),
    // Fallback: backend/.env from dist
    path.resolve(__dirname, '../../../.env'),
  ];

  const candidates = [
    process.env.ENV_FILE,
    ...cwdCandidates,
    ...dirnameCandidates,
  ].filter((p): p is string => !!p);

  // Find the first existing .env file
  for (const candidate of candidates) {
    try {
      if (fs.existsSync(candidate)) {
        return path.resolve(candidate);
      }
    } catch {
      // Ignore filesystem errors (permission issues, etc.)
    }
  }

  return null;
}

/**
 * Ensures environment variables are loaded from .env file.
 * 
 * This function is idempotent - calling it multiple times is safe and efficient.
 * It will only load the .env file once, and subsequent calls return the cached result.
 * 
 * IMPORTANT: This function does NOT override existing environment variables.
 * If DATABASE_URL is already set (e.g., by PM2), the .env file is NOT loaded.
 */
export function ensureEnvLoaded(): EnvLoadResult {
  // Return cached result if already processed
  if (loadResult !== null) {
    return loadResult;
  }

  // If DATABASE_URL is already set, skip loading (PM2 or Docker provided env)
  if (process.env.DATABASE_URL) {
    loadResult = {
      loaded: false,
      envPath: null,
      reason: 'already-set',
    };
    return loadResult;
  }

  // Resolve and load .env file
  const envPath = resolveEnvPath();

  if (envPath) {
    dotenv.config({ path: envPath });
    loadResult = {
      loaded: true,
      envPath,
      reason: 'file-found',
    };
  } else {
    // Fallback to dotenv's default behavior
    dotenv.config();
    loadResult = {
      loaded: true,
      envPath: null,
      reason: 'default-fallback',
    };
  }

  return loadResult;
}

/**
 * Returns diagnostic information about the loaded environment.
 * 
 * This is useful for logging/debugging in CLI scripts without exposing credentials.
 * The DATABASE_URL is parsed to extract only non-sensitive metadata.
 */
export function getEnvDiagnostics(): EnvDiagnostics {
  const result = ensureEnvLoaded();
  
  let dbInfo: EnvDiagnostics['databaseUrl'] = null;
  
  if (process.env.DATABASE_URL) {
    try {
      const u = new URL(process.env.DATABASE_URL);
      dbInfo = {
        host: u.hostname,
        port: u.port || '5432',
        database: (u.pathname || '').replace(/^\//, ''),
        sslmode: u.searchParams.get('sslmode'),
      };
    } catch {
      // Invalid URL format - leave dbInfo as null
    }
  }

  return {
    cwd: process.cwd(),
    envPath: result.envPath,
    databaseUrl: dbInfo,
  };
}

/**
 * Re-export dotenv for modules that need direct access
 */
export { dotenv };
