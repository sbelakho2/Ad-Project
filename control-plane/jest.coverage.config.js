// jest.coverage.config.js — additive 100% coverage gate for control-plane (Express).
const base = require('./jest.config.js');

/** @type {import('jest').Config} */
const config = {
  ...base,
  collectCoverage: true,
  coverageDirectory: '<rootDir>/coverage',
  coverageReporters: ['json-summary', 'lcov', 'text'],
  collectCoverageFrom: ['src/**/*.ts', '!src/**/*.d.ts'],
  coverageThreshold: {
    global: { branches: 100, functions: 100, lines: 100, statements: 100 },
  },
};

module.exports = config;
