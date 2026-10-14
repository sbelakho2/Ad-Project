/** @type {import('jest').Config} */
module.exports = {
  preset: 'ts-jest/presets/default-esm',
  testEnvironment: 'node',
  rootDir: '.',
  testMatch: ['**/tests/**/*.test.ts'],
  setupFilesAfterEnv: ['<rootDir>/setupTests.ts'],
  // msw 2 and its ESM-only dependencies (rettime among them) must be
  // transformed; everything else under node_modules stays ignored.
  transformIgnorePatterns: [
    '/node_modules/(?!(msw|@mswjs|@open-draft|rettime|until-async|is-node-process|outvariant|strict-event-emitter|headers-polyfill|tough-cookie|graphql|type-fest)/)'
  ],
  transform: {
    '^.+\\.tsx?$': [
      'ts-jest',
      { tsconfig: '<rootDir>/tsconfig.json', useESM: true },
    ],
    '^.+\\.m?js$': [
      'ts-jest',
      {
        useESM: true,
        diagnostics: false,
        tsconfig: {
          allowJs: true,
          target: 'ES2020',
          module: 'ESNext',
          moduleResolution: 'Bundler',
          esModuleInterop: true,
          skipLibCheck: true,
        },
      },
    ],
  },
  collectCoverage: true,
  collectCoverageFrom: ['src/**/*.{ts,tsx}'],
  coverageDirectory: 'coverage',
  extensionsToTreatAsEsm: ['.ts'],
  moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx', 'json', 'mjs']
};
