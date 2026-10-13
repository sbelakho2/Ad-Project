// jest.coverage.config.js — additive 100% coverage gate for control-plane (Express).
const base = require('./jest.config.js');
const scope = require('../tools/coverage/collect-scope.json')['control-plane'];

/** @type {import('jest').Config} */
const config = {
  ...base,
  collectCoverage: true,
  coverageDirectory: '<rootDir>/coverage',
  coverageReporters: ['json-summary', 'lcov', 'text'],
  // Scope is single-sourced from tools/coverage/collect-scope.json; the stack-integrity
  // test asserts it matches the manifest domain, so the artifact cannot silently miss files.
  collectCoverageFrom: scope.collectCoverageFrom,
  // Floors are enforced by tools/coverage/coverage-gate.mjs from tools/coverage/coverage-floor.json
  // (ratchet-only, single source of truth). No hardcoded thresholds here by design.

};

module.exports = config;
