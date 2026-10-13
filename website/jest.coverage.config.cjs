// jest.coverage.config.cjs — additive 100% coverage gate for website (Next.js).
// Extends the JS-only base config and ADDS a TypeScript transform so the TS tree
// is testable (the base config deliberately skipped TS; that exclusion is banned
// by the 100%-of-repo policy).
const base = require('./jest.config.cjs');

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
  collectCoverageFrom: ['src/**/*.{js,jsx,ts,tsx}', '!src/**/*.d.ts'],
  coverageThreshold: {
    global: { branches: 100, functions: 100, lines: 100, statements: 100 },
  },
  testPathIgnorePatterns: ['/node_modules/', '/.next/'],
};

module.exports = config;
