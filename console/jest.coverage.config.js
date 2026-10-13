// jest.coverage.config.js — additive 100% coverage gate for console (Next.js).
// Extends the unit config; does not change default `npm test`.
const base = require('./jest.config.js');

/** @type {import('jest').Config} */
const config = {
  ...base,
  collectCoverage: true,
  coverageDirectory: '<rootDir>/coverage',
  coverageReporters: ['json-summary', 'lcov', 'text'],
  collectCoverageFrom: [
    'src/**/*.{ts,tsx}',
    '!src/**/*.d.ts',
    '!src/tests/**',
  ],
  coverageThreshold: {
    global: { branches: 100, functions: 100, lines: 100, statements: 100 },
  },
};

module.exports = config;
