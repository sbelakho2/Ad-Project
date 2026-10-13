// jest.coverage.config.cjs — additive coverage gate for backend (does not change default `npm test`).
// Enforces the 100% gate stack: consolidated testMatch (picks up the 157 previously
// unexecuted test files), coverage over all first-party TS, thresholds 100/100/100/100.
const base = require('./jest.config.cjs');
const scope = require('../tools/coverage/collect-scope.json')['backend'];

/** @type {import('jest').Config} */
const config = {
  ...base,
  collectCoverage: true,
  coverageProvider: 'v8',
  coverageDirectory: '<rootDir>/coverage',
  coverageReporters: ['json-summary', 'lcov', 'text'],
  // Scope is single-sourced from tools/coverage/collect-scope.json; the stack-integrity
  // test asserts it matches the manifest domain, so the artifact cannot silently miss files.
  collectCoverageFrom: scope.collectCoverageFrom,
  // Floors are enforced by tools/coverage/coverage-gate.mjs from tools/coverage/coverage-floor.json
  // (ratchet-only, single source of truth). No hardcoded thresholds here by design.

  testMatch: [
    '<rootDir>/tests/**/*.test.ts',
    '<rootDir>/src/**/__tests__/**/*.test.ts',
    '<rootDir>/scripts/__tests__/**/*.test.ts',
    '<rootDir>/routes/__tests__/**/*.test.ts',
    '<rootDir>/services/sales-automation/tests/**/*.test.ts',
  ],
  // No coveragePathIgnorePatterns: any future ignore must go through
  // tools/coverage/suites.json exclusions with a reason code (meta-suite enforced).
};

module.exports = config;
