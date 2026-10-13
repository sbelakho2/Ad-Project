// jest.coverage.config.cjs — additive 100% coverage gate for the web SDK
// (base config already collects coverage; this adds the threshold).
const base = require('./jest.config.cjs');

/** @type {import('jest').Config} */
const config = {
  ...base,
  coverageThreshold: {
    global: { branches: 100, functions: 100, lines: 100, statements: 100 },
  },
};

module.exports = config;
