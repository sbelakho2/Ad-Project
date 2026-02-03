/**
 * Unit tests for the centralized envLoader utility
 */

import { jest, describe, it, expect, beforeEach, afterEach } from '@jest/globals';
import path from 'node:path';
import fs from 'node:fs';

// Save original env
const originalEnv = { ...process.env };

describe('envLoader', () => {
  beforeEach(() => {
    // Clear the module cache to reset the singleton state
    jest.resetModules();
    // Reset environment
    process.env = { ...originalEnv };
    delete process.env.DATABASE_URL;
    delete process.env.ENV_FILE;
  });

  afterEach(() => {
    process.env = { ...originalEnv };
  });

  describe('ensureEnvLoaded', () => {
    it('should not load .env when DATABASE_URL is already set', () => {
      process.env.DATABASE_URL = 'postgresql://test:test@localhost:5432/test';
      
      const { ensureEnvLoaded } = require('../../../src/utils/envLoader');
      const result = ensureEnvLoaded();
      
      expect(result.loaded).toBe(false);
      expect(result.reason).toBe('already-set');
    });

    it('should return cached result on subsequent calls', () => {
      process.env.DATABASE_URL = 'postgresql://test:test@localhost:5432/test';
      
      const { ensureEnvLoaded } = require('../../../src/utils/envLoader');
      const result1 = ensureEnvLoaded();
      const result2 = ensureEnvLoaded();
      
      // Should return the same object (cached)
      expect(result1).toBe(result2);
    });
  });

  describe('resolveEnvPath', () => {
    it('should respect ENV_FILE environment variable', () => {
      const testEnvPath = '/tmp/test-env-file';
      process.env.ENV_FILE = testEnvPath;
      
      // Create a temporary file
      fs.writeFileSync(testEnvPath, 'TEST=1');
      
      try {
        const { resolveEnvPath } = require('../../../src/utils/envLoader');
        const result = resolveEnvPath();
        
        expect(result).toBe(testEnvPath);
      } finally {
        fs.unlinkSync(testEnvPath);
      }
    });
  });

  describe('getEnvDiagnostics', () => {
    it('should return safe diagnostics without credentials', () => {
      process.env.DATABASE_URL = 'postgresql://user:secret@db.example.com:5432/mydb?sslmode=require';
      
      const { getEnvDiagnostics } = require('../../../src/utils/envLoader');
      const diagnostics = getEnvDiagnostics();
      
      expect(diagnostics.databaseUrl).toBeDefined();
      expect(diagnostics.databaseUrl?.host).toBe('db.example.com');
      expect(diagnostics.databaseUrl?.port).toBe('5432');
      expect(diagnostics.databaseUrl?.database).toBe('mydb');
      expect(diagnostics.databaseUrl?.sslmode).toBe('require');
      
      // Ensure credentials are NOT exposed
      expect(JSON.stringify(diagnostics)).not.toContain('secret');
      expect(JSON.stringify(diagnostics)).not.toContain('user');
    });

    it('should handle invalid DATABASE_URL gracefully', () => {
      process.env.DATABASE_URL = 'not-a-valid-url';
      
      const { getEnvDiagnostics } = require('../../../src/utils/envLoader');
      const diagnostics = getEnvDiagnostics();
      
      expect(diagnostics.databaseUrl).toBeNull();
    });
  });
});
