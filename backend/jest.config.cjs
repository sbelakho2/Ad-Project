/** @type {import('jest').Config} */
const config = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  clearMocks: true,
  coverageProvider: 'v8',
  // Consolidated test structure: all tests in tests/ directory
  testMatch: ['<rootDir>/tests/**/*.test.ts'],
  setupFilesAfterEnv: ['<rootDir>/tests/setup.ts'],
  testTimeout: 10000,
  maxWorkers: 1, // Run tests serially to avoid database conflicts
  moduleFileExtensions: ['ts', 'tsx', 'js', 'json'],
  moduleNameMapper: {
    '^@/(.*)$': '<rootDir>/src/$1',
    // Allow CLI script tests under backend/scripts/** to import TS sources via "../src/..."
    '^\\.\\./src/(.*)$': '<rootDir>/src/$1',
  },
  // Ignore built output to prevent duplicate manual mocks (__mocks__) from dist
  // Also ignore legacy src/__tests__ location (deprecated, use tests/)
  modulePathIgnorePatterns: ['<rootDir>/dist/', '<rootDir>/src/__tests__/'],
  testPathIgnorePatterns: [],
  globals: {
    'ts-jest': {
      tsconfig: {
        // Ensure Jest globals are available in TS during test transpilation
        types: ['jest', 'node'],
        typeRoots: ['<rootDir>/src/types', '<rootDir>/node_modules/@types'],
      },
      isolatedModules: true,
    },
  },
};

// Gate DB-backed integration tests behind FORCE_DB_SETUP flag for lightweight CI/local runs
if (process.env.FORCE_DB_SETUP !== 'true') {
  config.testPathIgnorePatterns.push('<rootDir>/tests/integration');
  // Skip observability metrics assertions in lightweight mode to avoid cross-test registry coupling
  config.testPathIgnorePatterns.push('<rootDir>/tests/unit/observability.metrics.test.ts');
}

module.exports = config;
