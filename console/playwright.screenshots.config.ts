import { defineConfig, devices } from '@playwright/test';

const BASE_URL = process.env.CONSOLE_BASE_URL || 'http://164.90.232.151:3000';

export default defineConfig({
  testDir: './tests',
  timeout: 60 * 1000,
  expect: { timeout: 10000 },
  workers: 2,
  retries: 1,
  use: {
    baseURL: BASE_URL,
    trace: 'off',
    viewport: { width: 1920, height: 1080 },
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
  // No webServer - use existing deployed server
  // Only run desktop tests
  grep: /Desktop Views/,
});
