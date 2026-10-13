// jest.coverage.config.js — additive 100% coverage gate for the autopilot service.
/** @type {import('jest').Config} */
const scope = require('../../tools/coverage/collect-scope.json')['autopilot'];

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

  testMatch: ['<rootDir>/src/**/*.test.ts', '<rootDir>/tests/**/*.test.ts'],
  moduleFileExtensions: ['ts', 'js', 'json'],
  transform: {
    '^.+\\.ts$': ['ts-jest', { tsconfig: '<rootDir>/tsconfig.json' }],
  },
};

module.exports = config;
