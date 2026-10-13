// jest.coverage.config.cjs — additive 100% coverage gate for edge-worker (first suite ever).
/** @type {import('jest').Config} */
const scope = require('../tools/coverage/collect-scope.json')['edge-worker'];

const config = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  clearMocks: true,
  collectCoverage: true,
  coverageDirectory: '<rootDir>/coverage',
  coverageReporters: ['json-summary', 'lcov', 'text'],
  // Scope is single-sourced from tools/coverage/collect-scope.json; the stack-integrity
  // test asserts it matches the manifest domain, so the artifact cannot silently miss files.
  collectCoverageFrom: scope.collectCoverageFrom,
  // Floors are enforced by tools/coverage/coverage-gate.mjs from tools/coverage/coverage-floor.json
  // (ratchet-only, single source of truth). No hardcoded thresholds here by design.

  testMatch: ['<rootDir>/tests/**/*.test.ts', '<rootDir>/src/**/__tests__/**/*.test.ts'],
  testTimeout: 15000,
  maxWorkers: 1,
  moduleFileExtensions: ['ts', 'js', 'json'],
  moduleNameMapper: {
    '^(\\.{1,2}/.*)\\.js$': '$1',
  },
  transform: {
    '^.+\\.ts$': ['ts-jest', { tsconfig: '<rootDir>/tsconfig.json' }],
  },
};

module.exports = config;
