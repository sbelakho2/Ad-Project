// jest.coverage.config.cjs — additive 100% coverage gate for website (Next.js).
// Extends the JS-only base config and ADDS a TypeScript transform so the TS tree
// is testable (the base config deliberately skipped TS; that exclusion is banned
// by the 100%-of-repo policy).
const base = require('./jest.config.cjs');
const scope = require('../tools/coverage/collect-scope.json')['website'];

/** @type {import('jest').Config} */
const config = {
  ...base,
  collectCoverage: true,
  coverageDirectory: '<rootDir>/coverage',
  coverageReporters: ['json-summary', 'lcov', 'text'],
  transform: {
    '^.+\\.(js|jsx)$': 'babel-jest',
    '^.+\\.(ts|tsx)$': ['babel-jest', { presets: ['next/babel'] }],
  },
  testMatch: ['**/src/**/*.test.{js,ts,tsx}'],
  // Scope is single-sourced from tools/coverage/collect-scope.json; the stack-integrity
  // test asserts it matches the manifest domain, so the artifact cannot silently miss files.
  collectCoverageFrom: scope.collectCoverageFrom,
  // Floors are enforced by tools/coverage/coverage-gate.mjs from tools/coverage/coverage-floor.json
  // (ratchet-only, single source of truth). No hardcoded thresholds here by design.

  testPathIgnorePatterns: ['/node_modules/', '/.next/'],
};

module.exports = config;
