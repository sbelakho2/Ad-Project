// jest.coverage.config.cjs — additive 100% coverage gate for the web SDK.
// Scope is single-sourced from tools/coverage/collect-scope.json (stack-integrity parity test).
const base = require('./jest.config.cjs');
const scope = require('../../tools/coverage/collect-scope.json')['web-sdk'];

/** @type {import('jest').Config} */
const config = {
  ...base,
  collectCoverage: true,
  coverageReporters: ['json-summary', 'lcov', 'text'],
  collectCoverageFrom: scope.collectCoverageFrom,
  // Floors are enforced by tools/coverage/coverage-gate.mjs from tools/coverage/coverage-floor.json
  // (ratchet-only, single source of truth). No hardcoded thresholds here by design.

};

module.exports = config;
