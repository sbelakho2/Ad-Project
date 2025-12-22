import { defineConfig, devices } from '@playwright/test';

const PLAYWRIGHT_PORT = Number(process.env.PLAYWRIGHT_PORT || 4310)
const PLAYWRIGHT_BASE_URL = process.env.CONSOLE_BASE_URL || `http://localhost:${PLAYWRIGHT_PORT}`

export default defineConfig({
  testDir: './tests',
  timeout: 30 * 1000,
  expect: { timeout: 5000 },
  use: {
    baseURL: PLAYWRIGHT_BASE_URL,
    trace: 'on-first-retry',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'firefox', use: { ...devices['Desktop Firefox'] } },
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
  ],
  // Auto-start the Console app during tests to avoid connection errors
  webServer: {
    command: `npm run dev -- -p ${PLAYWRIGHT_PORT}`,
    url: PLAYWRIGHT_BASE_URL,
    reuseExistingServer: false,
    timeout: 120_000,
    env: {
      NEXT_PUBLIC_USE_MOCK_API: 'true',
      NEXT_PUBLIC_E2E_SESSION: JSON.stringify({
        userId: 'usr-playwright',
        publisherId: 'pub-test-123',
        email: 'playwright@example.com',
        role: 'admin',
      }),
    },
  },
});
