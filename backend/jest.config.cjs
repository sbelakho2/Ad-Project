/** @type {import('jest').Config} */
const config = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  clearMocks: true,
  coverageProvider: 'v8',
  testMatch: ['**/__tests__/**/*.test.ts', '**/tests/**/*.test.ts'],
  setupFilesAfterEnv: ['<rootDir>/src/__tests__/setup.ts'],
  testTimeout: 10000,
  maxWorkers: 1, // Run tests serially to avoid database conflicts
  moduleFileExtensions: ['ts', 'tsx', 'js', 'json'],
  moduleNameMapper: {
    '^@/(.*)$': '<rootDir>/src/$1',
    // Allow CLI script tests under backend/scripts/** to import TS sources via "../src/..."
    '^\.\./src/(.*)$': '<rootDir>/src/$1',
  },
  // Ignore built output to prevent duplicate manual mocks (__mocks__) from dist
  modulePathIgnorePatterns: ['<rootDir>/dist/'],
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
  config.testPathIgnorePatterns.push('<rootDir>/src/__tests__/integration');
  // Skip observability metrics assertions in lightweight mode to avoid cross-test registry coupling
  config.testPathIgnorePatterns.push('<rootDir>/src/__tests__/observability.metrics.test.ts');
}

module.exports = config;
