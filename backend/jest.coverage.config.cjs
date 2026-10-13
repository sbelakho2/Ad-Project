// jest.coverage.config.cjs — additive coverage gate for backend (does not change default `npm test`).
// Enforces the 100% gate stack: consolidated testMatch (picks up the 157 previously
// unexecuted test files), coverage over all first-party TS, thresholds 100/100/100/100.
const base = require('./jest.config.cjs');

/** @type {import('jest').Config} */
const config = {
  ...base,
  collectCoverage: true,
  coverageProvider: 'v8',
  coverageDirectory: '<rootDir>/coverage',
  coverageReporters: ['json-summary', 'lcov', 'text'],
  collectCoverageFrom: [
    'src/**/*.ts',
    'routes/**/*.ts',
    'services/**/*.ts',
    'scripts/**/*.ts',
    '!src/**/*.d.ts',
    '!src/openapi/**',
    '!src/types/**',
  ],
  coverageThreshold: {
    global: { branches: 100, functions: 100, lines: 100, statements: 100 },
  },
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
